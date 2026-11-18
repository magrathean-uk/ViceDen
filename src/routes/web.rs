//! Server-rendered HTML route handlers using Askama.
//!
//! This revision leans hard into a text-first forum flow: subscriptions,
//! personal thread controls, search, denser board listings and deliberate
//! word-labelled navigation.

use askama::Template;
use axum::{
    extract::{Form, Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use uuid::Uuid;

use crate::{
    forum,
    models::Board,
    routes::{
        admin_invites, hidden,
        middleware::rate_limit::{presets, rate_limit_middleware, Bucket, RoleExtractor},
        saved,
    },
    services, AppState,
};

pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(error) => fallback_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Template error",
                "The page could not be rendered.",
                Some(error.to_string()),
                "/login",
                "Back to login",
            ),
        }
    }
}

pub fn web_router(state: AppState) -> Router<AppState> {
    // Rate-limit layers require a `PgPool` for the role/session lookups that
    // power the founder bypass. Required DB sessions
    // skip the layers entirely — the in-memory bucket would still work, but
    // the founder bypass cannot resolve session tokens to user roles, so the
    // pragmatic call is to leave rate-limiting off when running without PG.
    let role_extractor = state
        .db
        .as_ref()
        .map(|pool| RoleExtractor::new(pool.clone(), state.config.auth.token_pepper.clone()));

    let mut protected = Router::new().route("/", get(index));

    protected = if let Some(extractor) = role_extractor.as_ref() {
        protected.route(
            "/search",
            get(search)
                .layer(rate_limit_middleware(Bucket::new(presets::search()), extractor.clone())),
        )
    } else {
        protected.route("/search", get(search))
    };

    protected = if let Some(extractor) = role_extractor.as_ref() {
        protected
            .route(
                "/b/{slug}",
                get(view_board).post(create_thread_web).layer(rate_limit_middleware(
                    Bucket::new(presets::thread_new()),
                    extractor.clone(),
                )),
            )
            .route(
                "/t/{id}",
                get(view_thread).post(create_post_web).layer(rate_limit_middleware(
                    Bucket::new(presets::post_new()),
                    extractor.clone(),
                )),
            )
            .route(
                "/t/{id}/report",
                post(report_thread_web).layer(rate_limit_middleware(
                    Bucket::new(presets::report()),
                    extractor.clone(),
                )),
            )
            .route(
                "/t/{thread_id}/posts/{post_id}/report",
                post(report_post_web).layer(rate_limit_middleware(
                    Bucket::new(presets::report()),
                    extractor.clone(),
                )),
            )
            .route(
                "/t/{thread_id}/vote/{post_id}",
                post(vote_post_web)
                    .layer(rate_limit_middleware(Bucket::new(presets::vote()), extractor.clone())),
            )
            .route(
                "/profile/pats",
                post(create_pat_web).layer(rate_limit_middleware(
                    Bucket::new(presets::pat_create()),
                    extractor.clone(),
                )),
            )
    } else {
        protected
            .route("/b/{slug}", get(view_board).post(create_thread_web))
            .route("/t/{id}", get(view_thread).post(create_post_web))
            .route("/t/{id}/report", post(report_thread_web))
            .route("/t/{thread_id}/posts/{post_id}/report", post(report_post_web))
            .route("/t/{thread_id}/vote/{post_id}", post(vote_post_web))
            .route("/profile/pats", post(create_pat_web))
    };

    let protected = protected
        .route("/b/{slug}/subscription", post(toggle_board_subscription_web))
        .route("/t/{id}/save", post(toggle_thread_save_web))
        .route("/t/{id}/hide", post(toggle_thread_hide_web))
        .route("/notifications", get(view_notifications))
        .route("/notifications/{id}/remove", post(remove_notification_web))
        .route("/notifications/{id}/read", post(mark_single_notification_web))
        .route("/notifications/read-all", post(mark_all_notifications_web))
        .route("/live/state", get(live_state_web))
        .route("/profile", get(profile).post(update_profile_settings))
        .route("/profile/pats/{id}/delete", post(delete_pat_web))
        .route("/saved", get(saved::view_saved))
        .route("/hidden", get(hidden::view_hidden))
        .route("/admin/users", get(view_users))
        .route("/admin/users/{id}", get(view_user_detail))
        .route("/admin/users/{id}/lift-probation", post(lift_probation_web))
        .route("/admin/users/{id}/cauterise", post(cauterise_user_web))
        .route("/admin/users/{id}/wipe", post(wipe_user_web))
        .route("/admin/users/{id}/delete", post(delete_user_web))
        .route("/admin/metrics", get(view_metrics))
        .route("/admin/moderation-events", get(view_moderation_events))
        .route("/admin/moderation-events.csv", get(download_moderation_events_csv))
        .route("/internal/metrics", get(view_internal_metrics))
        .route("/admin/reports", get(view_reports))
        .route("/admin/reports/export.csv", get(download_reports_csv))
        .route("/admin/reports/{id}/triage", post(triage_report_web))
        .route("/admin/reports/{id}/resolve", post(resolve_report_web))
        .route("/admin/reports/{id}/remove-target", post(remove_report_target_web))
        .route("/admin/invites", get(admin_invites::list).post(admin_invites::issue))
        .route("/admin/invites/{code}/revoke", post(admin_invites::revoke))
        .route_layer(middleware::from_fn_with_state(state, require_web_login));

    let mut public = Router::new();
    public = if let Some(extractor) = role_extractor.as_ref() {
        public
            .route(
                "/login",
                get(login_page)
                    .post(login_user)
                    .layer(rate_limit_middleware(Bucket::new(presets::login()), extractor.clone())),
            )
            .route(
                "/register",
                get(register_page).post(register_user).layer(rate_limit_middleware(
                    Bucket::new(presets::register()),
                    extractor.clone(),
                )),
            )
    } else {
        public
            .route("/login", get(login_page).post(login_user))
            .route("/register", get(register_page).post(register_user))
    };

    public
        .route("/logout", post(logout_user))
        .route("/guide", get(guide))
        .route("/rules", get(rules))
        .route("/privacy", get(privacy))
        .merge(protected)
        .route("/robots.txt", get(robots_txt))
        .route("/sitemap.xml", get(sitemap_xml))
        .route("/llms.txt", get(llms_txt))
        .route("/llms-full.txt", get(llms_full_txt))
}

#[derive(Debug, Deserialize)]
struct IndexQuery {
    feed: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchPageQuery {
    q: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BoardQuery {
    sort: Option<String>,
    page: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ThreadViewQuery {
    /// Set after a moderation hook finishes — `pending`, `quarantined`,
    /// or `removed`. Anything else is treated as "no banner".
    moderation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsersQuery {
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReportsQuery {
    status: Option<String>,
    queue_state: Option<String>,
    assigned: Option<String>,
    priority: Option<String>,
    action: Option<String>,
    moderator: Option<String>,
    since: Option<String>,
    until: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModerationEventsQuery {
    action: Option<String>,
    moderator: Option<String>,
    target: Option<String>,
    report_id: Option<String>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
}

#[derive(Clone, Debug)]
struct ReportDeskFilters {
    status: String,
    queue_state: String,
    assigned: String,
    priority: String,
    action: String,
    moderator: String,
    since: String,
    until: String,
    moderator_id: Option<Uuid>,
    event_created_from: Option<DateTime<Utc>>,
    event_created_to: Option<DateTime<Utc>>,
}

impl ReportDeskFilters {
    async fn from_query(pool: &sqlx::PgPool, query: ReportsQuery) -> Self {
        use crate::repository::users::UserRepo;

        let status = if matches!(query.status.as_deref(), Some("resolved")) {
            "resolved".to_string()
        } else {
            "open".to_string()
        };
        let queue_state = query
            .queue_state
            .as_deref()
            .filter(|value| matches!(*value, "new" | "reviewing" | "waiting"))
            .unwrap_or_default()
            .to_string();
        let assigned = query
            .assigned
            .as_deref()
            .filter(|value| matches!(*value, "me" | "unassigned"))
            .unwrap_or_default()
            .to_string();
        let priority = query
            .priority
            .as_deref()
            .filter(|value| matches!(*value, "urgent" | "elevated" | "normal"))
            .unwrap_or_default()
            .to_string();
        let action = query.action.unwrap_or_default().trim().to_string();
        let moderator = query.moderator.unwrap_or_default().trim().to_string();
        let since = query.since.unwrap_or_default().trim().to_string();
        let until = query.until.unwrap_or_default().trim().to_string();
        let moderator_id = if moderator.is_empty() {
            None
        } else {
            Some(
                UserRepo
                    .find_by_handle(pool, &moderator)
                    .await
                    .ok()
                    .flatten()
                    .map(|user| user.id)
                    .unwrap_or(Uuid::nil()),
            )
        };
        let event_created_from = parse_query_date_floor(&since);
        let event_created_to = parse_query_date_ceiling(&until);

        Self {
            status,
            queue_state,
            assigned,
            priority,
            action,
            moderator,
            since,
            until,
            moderator_id,
            event_created_from,
            event_created_to,
        }
    }

    fn report_queue_filter(
        &self,
        viewer_id: Uuid,
    ) -> crate::repository::reports::ReportQueueFilter<'_> {
        crate::repository::reports::ReportQueueFilter {
            status: Some(&self.status),
            queue_state: (!self.queue_state.is_empty()).then_some(self.queue_state.as_str()),
            assigned: (!self.assigned.is_empty()).then_some(self.assigned.as_str()),
            priority: (!self.priority.is_empty()).then_some(self.priority.as_str()),
            event_action: (!self.action.is_empty()).then_some(self.action.as_str()),
            event_actor_id: self.moderator_id,
            event_created_from: self.event_created_from,
            event_created_to: self.event_created_to,
            viewer_id: Some(viewer_id),
        }
    }

    fn return_to(&self) -> String {
        self.path_with_base("/admin/reports")
    }

    fn export_url(&self) -> String {
        self.path_with_base("/admin/reports/export.csv")
    }

    fn path_with_base(&self, base: &str) -> String {
        let query = self.query_string();
        if query.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{query}")
        }
    }

    fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if self.status == "resolved" {
            parts.push("status=resolved".to_string());
        }
        if !self.queue_state.is_empty() {
            parts.push(format!("queue_state={}", encode_query_value(&self.queue_state)));
        }
        if !self.assigned.is_empty() {
            parts.push(format!("assigned={}", encode_query_value(&self.assigned)));
        }
        if !self.priority.is_empty() {
            parts.push(format!("priority={}", encode_query_value(&self.priority)));
        }
        if !self.action.is_empty() {
            parts.push(format!("action={}", encode_query_value(&self.action)));
        }
        if !self.moderator.is_empty() {
            parts.push(format!("moderator={}", encode_query_value(&self.moderator)));
        }
        if !self.since.is_empty() {
            parts.push(format!("since={}", encode_query_value(&self.since)));
        }
        if !self.until.is_empty() {
            parts.push(format!("until={}", encode_query_value(&self.until)));
        }
        parts.join("&")
    }
}

#[derive(Clone, Debug)]
struct ModerationEventsFilters {
    action: String,
    moderator: String,
    target: String,
    report_id: String,
    since: String,
    until: String,
    limit: usize,
    moderator_id: Option<Uuid>,
    target_user_id: Option<Uuid>,
    report_uuid: Option<Uuid>,
    created_from: Option<DateTime<Utc>>,
    created_to: Option<DateTime<Utc>>,
}

impl ModerationEventsFilters {
    async fn from_query(pool: &sqlx::PgPool, query: ModerationEventsQuery) -> Self {
        use crate::repository::users::UserRepo;

        let action = query.action.unwrap_or_default().trim().to_string();
        let moderator = query.moderator.unwrap_or_default().trim().to_string();
        let target = query.target.unwrap_or_default().trim().to_string();
        let report_id = query.report_id.unwrap_or_default().trim().to_string();
        let since = query.since.unwrap_or_default().trim().to_string();
        let until = query.until.unwrap_or_default().trim().to_string();
        let limit = query.limit.unwrap_or(100).clamp(1, 500);
        let moderator_id = if moderator.is_empty() {
            None
        } else {
            Some(
                UserRepo
                    .find_by_handle(pool, &moderator)
                    .await
                    .ok()
                    .flatten()
                    .map(|user| user.id)
                    .unwrap_or(Uuid::nil()),
            )
        };
        let target_user_id = if target.is_empty() {
            None
        } else {
            Some(
                UserRepo
                    .find_by_handle(pool, &target)
                    .await
                    .ok()
                    .flatten()
                    .map(|user| user.id)
                    .unwrap_or(Uuid::nil()),
            )
        };
        let report_uuid = Uuid::parse_str(&report_id).ok();
        let created_from = parse_query_date_floor(&since);
        let created_to = parse_query_date_ceiling(&until);

        Self {
            action,
            moderator,
            target,
            report_id,
            since,
            until,
            limit,
            moderator_id,
            target_user_id,
            report_uuid,
            created_from,
            created_to,
        }
    }

    fn csv_url(&self) -> String {
        self.path_with_base("/admin/moderation-events.csv")
    }

    fn path_with_base(&self, base: &str) -> String {
        let query = self.query_string();
        if query.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{query}")
        }
    }

    fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if !self.action.is_empty() {
            parts.push(format!("action={}", encode_query_value(&self.action)));
        }
        if !self.moderator.is_empty() {
            parts.push(format!("moderator={}", encode_query_value(&self.moderator)));
        }
        if !self.target.is_empty() {
            parts.push(format!("target={}", encode_query_value(&self.target)));
        }
        if !self.report_id.is_empty() {
            parts.push(format!("report_id={}", encode_query_value(&self.report_id)));
        }
        if !self.since.is_empty() {
            parts.push(format!("since={}", encode_query_value(&self.since)));
        }
        if !self.until.is_empty() {
            parts.push(format!("until={}", encode_query_value(&self.until)));
        }
        if self.limit != 100 {
            parts.push(format!("limit={}", self.limit));
        }
        parts.join("&")
    }
}

#[derive(Debug, Deserialize)]
struct CreateThreadForm {
    title: String,
    body: String,
    tags: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreatePostForm {
    body: String,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VoteForm {
    value: i8,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NotificationReadForm {
    read: Option<bool>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateProfileForm {
    compactness: Option<i8>,
    #[serde(rename = "theme")]
    _theme: Option<String>,
    notify_mentions: Option<bool>,
    notify_replies: Option<bool>,
    notify_moderation: Option<bool>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    handle: String,
    password: String,
    return_to: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReturnToForm {
    return_to: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LiftProbationForm {
    note: Option<String>,
    return_to: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CauteriseUserForm {
    reason: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReportForm {
    reason: String,
    details: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveReportForm {
    resolved: Option<bool>,
    note: Option<String>,
    return_to: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReportTriageForm {
    queue_state: Option<String>,
    claim: Option<String>,
    release: Option<String>,
    note: Option<String>,
    return_to: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Serialize)]
struct LiveStateResponse {
    unread_notifications: usize,
    open_reports: usize,
}

#[derive(Debug, Deserialize)]
struct CreatePatForm {
    name: String,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterForm {
    handle: String,
    password: String,
    dob: Option<String>,
    confirm_18: Option<String>,
    confirm_rules: Option<String>,
    code: Option<String>,
    return_to: Option<String>,
    csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterQuery {
    code: Option<String>,
    return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GuideQuery {
    welcome: Option<String>,
}

type ShellContext = templates::ForumChrome;
type PublicContext = templates::PublicChrome;

struct ErrorPageSpec<'a> {
    status: StatusCode,
    title: &'a str,
    intro: &'a str,
    details: String,
    next_step: &'a str,
    action_href: &'a str,
    action_label: &'a str,
}

const BOARD_THREADS_PER_PAGE: usize = 10;
const GUEST_USER_ID: Uuid = Uuid::nil();
const LEGACY_SESSION_COOKIE_NAME: &str = "viceden_session";
const HOST_SESSION_COOKIE_NAME: &str = "__Host-viceden_session";

#[derive(Clone)]
pub(crate) struct WebSession {
    pub(crate) user_id: Uuid,
    pub(crate) session_token: String,
    pub(crate) csrf_token: String,
}

async fn require_web_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if session_from_headers(&headers, &state).await.is_some() {
        return next.run(request).await;
    }

    Redirect::to("/login").into_response()
}

async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<IndexQuery>,
) -> Response {
    use crate::repository::{
        posts::PostRepo, preferences::PreferencesRepo, tags::TagRepo,
        thread_states::ThreadStateRepo, threads::ThreadRepo, users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let auth = session_from_headers(&headers, &state).await;
    let user_id = auth.as_ref().map(|session| session.user_id).unwrap_or(GUEST_USER_ID);

    // A6.8 strangle: front-page now reads exclusively from Postgres.
    // required Postgres. Also closes A8 (mod_state overlay) — visibility
    // filter is applied per-board via `visible_threads_for_board`, so
    // quarantined / removed threads no longer leak to the front page.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };

    // Feed selection: subscribed by default if the user has any
    // subscriptions, else all.
    let subscribed_slugs: Vec<String> = if user_id == GUEST_USER_ID {
        Vec::new()
    } else {
        PreferencesRepo.subscribed_slugs_for(pool, user_id).await.unwrap_or_default()
    };
    let has_subscriptions = !subscribed_slugs.is_empty();
    let requested_feed = query
        .feed
        .as_deref()
        .unwrap_or(if has_subscriptions { "subscribed" } else { "all" })
        .trim()
        .to_ascii_lowercase();
    let feed =
        if requested_feed == "subscribed" { "subscribed".to_string() } else { "all".to_string() };

    let subscribed_set: HashSet<&str> = subscribed_slugs.iter().map(String::as_str).collect();

    // Board cards: per-board count + subscribed flag. Board count is small
    // (~5–10) so 1 query per board is fine; B5 will fold into a single
    // grouped query when it lands.
    let mut board_cards = Vec::with_capacity(state.boards.len());
    for board in &state.boards {
        let thread_count =
            crate::db::count_visible_board_threads(pool, user_id, &board.slug).await.unwrap_or(0);
        board_cards.push(templates::BoardCard {
            slug: board.slug.clone(),
            name: board.name.clone(),
            description: board.description.clone(),
            thread_count,
            subscribed: subscribed_set.contains(board.slug.as_str()),
        });
    }

    // Pull recent previews per board, then merge + sort + truncate.
    // ~5–10 boards × 28 = ~150 rows → trim to 28. B5 batches into a single
    // front-page-aggregator query.
    let board_pool: Vec<&str> = if feed == "subscribed" {
        subscribed_slugs.iter().map(String::as_str).collect()
    } else {
        state.boards.iter().map(|b| b.slug.as_str()).collect()
    };
    let mut all_previews: Vec<crate::forum::ThreadPreview> = Vec::new();
    for slug in &board_pool {
        if let Ok(previews) =
            crate::db::list_board_thread_previews(pool, user_id, slug, "new", 28, 0).await
        {
            all_previews.extend(previews);
        }
    }

    // Visibility filter: intersect with `visible_threads_for_board` per
    // board so quarantined / removed threads disappear for everyone but
    // founder + author. Closes the A8 // TODO that the legacy index left
    // open.
    let mut visible_ids: HashSet<Uuid> = HashSet::new();
    for slug in &board_pool {
        if let Ok(rows) = ThreadRepo.visible_threads_for_board(pool, slug, user_id, 28, 0).await {
            visible_ids.extend(rows.into_iter().map(|t| t.id));
        }
    }
    all_previews.retain(|p| visible_ids.contains(&p.thread.id));

    // Sort newest-first across all boards, then truncate.
    all_previews.sort_by_key(|p| std::cmp::Reverse(p.thread.created_at));
    all_previews.truncate(28);

    // Filter out threads the viewer has hidden.
    let hidden_set: HashSet<Uuid> = if user_id == GUEST_USER_ID {
        HashSet::new()
    } else {
        PreferencesRepo
            .hidden_threads_for(pool, user_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect()
    };
    all_previews.retain(|p| !hidden_set.contains(&p.thread.id));

    // Batch lookups for the merged preview set (same pattern as A6.6).
    let mut author_ids: Vec<Uuid> = all_previews.iter().map(|p| p.thread.user_id).collect();
    author_ids.sort();
    author_ids.dedup();
    let handle_map: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &author_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let thread_ids: Vec<Uuid> = all_previews.iter().map(|p| p.thread.id).collect();
    let state_map: HashMap<Uuid, crate::repository::thread_states::ThreadStateRow> =
        ThreadStateRepo
            .states_for_batch(pool, &thread_ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    let saved_set: HashSet<Uuid> = if user_id == GUEST_USER_ID {
        HashSet::new()
    } else {
        PreferencesRepo
            .saved_threads_for(pool, user_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect()
    };
    // B5: batch-fetch tags for all threads in one query.
    let mut tags_map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (tid, tag) in TagRepo.tags_for_threads(pool, &thread_ids).await.unwrap_or_default() {
        tags_map.entry(tid).or_default().push(tag);
    }

    let mut front_page_threads = Vec::with_capacity(all_previews.len());
    for preview in &all_previews {
        let thread_id = preview.thread.id;
        let snippet = PostRepo
            .first_post_for_thread(pool, thread_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.body.chars().take(220).collect::<String>())
            .unwrap_or_default();
        let tags = tags_map.remove(&thread_id).unwrap_or_default();
        let participant_count =
            PostRepo.participant_count(pool, thread_id).await.unwrap_or(0) as usize;
        let state_row = state_map.get(&thread_id).copied().unwrap_or_default();
        let thread_state = forum::ThreadState {
            pinned: state_row.pinned,
            locked: state_row.locked,
            answered: state_row.answered,
            closed: state_row.closed,
        };
        let author = handle_map.get(&preview.thread.user_id).cloned().unwrap_or_else(|| {
            format!("user-{}", &preview.thread.user_id.simple().to_string()[..8])
        });
        let board_slug = preview.thread.board_slug.clone();
        let board_name_text = board_name(&state.boards, &board_slug);
        front_page_threads.push(templates::ThreadListItem {
            thread_id,
            title: preview.thread.title.clone(),
            board_slug,
            board_name: board_name_text,
            author,
            created_at: local_time_text(preview.thread.created_at),
            last_activity: local_time_text(preview.last_activity),
            snippet,
            tags,
            reply_count: preview.reply_count,
            participant_count,
            score: preview.score,
            score_hidden: preview.score_hidden,
            authored_by_current_user: preview.thread.user_id == user_id,
            is_saved: saved_set.contains(&thread_id),
            thread_state,
        });
    }

    let front_page_title = if feed == "subscribed" {
        "Front page".to_string()
    } else {
        "Everything, no feed rigging".to_string()
    };
    let front_page_subtitle = if feed == "subscribed" {
        if has_subscriptions {
            "Only boards you explicitly opted into. No algorithmic sludge.".to_string()
        } else {
            "No subscriptions yet. Pick boards below or flip to all boards.".to_string()
        }
    } else {
        "Newest threads across the whole pit, chronological and blunt.".to_string()
    };

    let shell = build_shell_context(&state, auth.as_ref(), 12, String::new()).await;

    HtmlTemplate(templates::IndexTemplate {
        chrome: shell,
        intro_text: "ViceDen is a text bunker: boards, threads, search, subscriptions, and receipts. No autoplay. No recommended garbage. No icon riddles."
            .to_string(),
        board_cards,
        front_page_threads,
        feed,
        front_page_title,
        front_page_subtitle,
    })
    .into_response()
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchPageQuery>,
) -> Response {
    use crate::repository::{tags::TagRepo, users::UserRepo};
    use std::collections::HashMap;

    let auth = session_from_headers(&headers, &state).await;
    let user_id = auth.as_ref().map(|session| session.user_id).unwrap_or(GUEST_USER_ID);
    let scope = forum::SearchScope::from_query(query.scope.as_deref());
    let search_term = query.q.unwrap_or_default();
    let parsed_query = forum::parse_search_query(&search_term);

    // A6.7 strangle: route directly through `db::search_forum` (already
    // PG-only). required Postgres. db helper migrates to a
    // SearchRepo when db.rs is deleted in A7.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let hits = match crate::db::search_forum(
        pool,
        user_id,
        &search_term,
        None, // board_slug filter — handled by parsed_query inside search_forum
        scope,
        false, // include_hidden
        64,    // limit
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "search: search_forum failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "search unavailable").into_response();
        }
    };
    let return_to = if scope == forum::SearchScope::All {
        format!("/search?q={search_term}")
    } else {
        format!("/search?q={search_term}&scope={}", scope.as_str())
    };

    // Batch handle + tag lookups so render is two queries instead of 2N.
    let mut author_ids: Vec<Uuid> = hits.iter().map(|h| h.author_id).collect();
    author_ids.sort();
    author_ids.dedup();
    let handle_map: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &author_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    // Tag lookup is still per-thread for now (B5 will fold into a single
    // `tags_for_threads` batch query).
    let mut tag_cache: HashMap<Uuid, Vec<String>> = HashMap::new();

    let mut results = Vec::with_capacity(hits.len());
    for hit in &hits {
        let thread_id = hit.thread_id;
        let post_id = hit.post_id;
        let board_slug = hit.board_slug.clone();
        let board_name_text = board_name(&state.boards, &board_slug);
        let url = post_id
            .map(|pid| format!("/t/{thread_id}#post-{pid}"))
            .unwrap_or_else(|| format!("/t/{thread_id}"));
        let tags = if let Some(t) = tag_cache.get(&thread_id) {
            t.clone()
        } else {
            let t = TagRepo.list_for_thread(pool, thread_id).await.unwrap_or_default();
            tag_cache.insert(thread_id, t.clone());
            t
        };
        let author = handle_map
            .get(&hit.author_id)
            .cloned()
            .unwrap_or_else(|| format!("user-{}", &hit.author_id.simple().to_string()[..8]));
        results.push(templates::SearchResultItem {
            thread_id,
            post_id,
            board_slug,
            board_name: board_name_text,
            url,
            open_label: if post_id.is_some() {
                "Open post".to_string()
            } else {
                "Open thread".to_string()
            },
            return_to: return_to.clone(),
            thread_title: hit.thread_title.clone(),
            author,
            preview: hit.body_snippet.clone(),
            tags,
            created_at: local_time_text(hit.created_at),
            score: hit.score,
            kind: match hit.kind {
                forum::SearchHitKind::Thread => "thread".to_string(),
                forum::SearchHitKind::Post => "post".to_string(),
            },
            is_saved: hit.is_saved,
        });
    }

    let result_count = results.len();
    let shell = build_shell_context(&state, auth.as_ref(), 10, String::new()).await;
    let mut active_filters = Vec::new();
    if let Some(tag) = parsed_query.tag {
        active_filters.push(templates::SearchFilterChip {
            label: "tag".to_string(),
            value: format!("#{tag}"),
        });
    }
    if let Some(author_handle) = parsed_query.author_handle {
        active_filters.push(templates::SearchFilterChip {
            label: "author".to_string(),
            value: format!("@{author_handle}"),
        });
    }
    if let Some(board_slug) = parsed_query.board_slug {
        active_filters.push(templates::SearchFilterChip {
            label: "board".to_string(),
            value: format!("/{board_slug}"),
        });
    }
    if scope != forum::SearchScope::All {
        active_filters.push(templates::SearchFilterChip {
            label: "scope".to_string(),
            value: format!("{} only", scope.as_str()),
        });
    }

    HtmlTemplate(templates::SearchTemplate {
        chrome: shell,
        query: search_term,
        scope: scope.as_str().to_string(),
        active_filters,
        results,
        result_count,
    })
    .into_response()
}

async fn guide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<GuideQuery>,
) -> HtmlTemplate<templates::GuideTemplate> {
    use crate::repository::users::UserRepo;

    let auth = session_from_headers(&headers, &state).await;
    let public = build_public_context(&state, auth.as_ref()).await;
    let probation_until = if let (Some(session), Some(pool)) = (auth.as_ref(), state.db.as_ref()) {
        UserRepo
            .find_by_id(pool, session.user_id)
            .await
            .ok()
            .flatten()
            .and_then(|user| active_probation_until(user.probation_until))
            .map(local_time_text)
    } else {
        None
    };

    HtmlTemplate(templates::GuideTemplate {
        public,
        show_welcome: query.welcome.as_deref() == Some("1"),
        probation_until,
    })
}

async fn rules(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> HtmlTemplate<templates::StaticPageTemplate> {
    let auth = session_from_headers(&headers, &state).await;
    let public = build_public_context(&state, auth.as_ref()).await;

    HtmlTemplate(templates::StaticPageTemplate {
        public,
        title: "Rules".to_string(),
        kicker: "Policy".to_string(),
        intro: "Be respectful. Text first. No soliciting."
            .to_string(),
        sections: vec![
            (
                "Age gate".to_string(),
                "ViceDen is for users aged 18 or over. If that is not you, leave.".to_string(),
            ),
            (
                "Legality".to_string(),
                "No harassment, threats, exploitation of minors, or illegal material."
                    .to_string(),
            ),
            (
                "Moderation".to_string(),
                "Report threads or posts when they cross the line. Founder tools can remove content and review reports."
                    .to_string(),
            ),
            (
                "Terms".to_string(),
                "ViceDen is published by MAGRATHEAN UK LTD (\"Magrathean\"). Using ViceDen means accepting these terms, on top of the rules above.".to_string(),
            ),
            (
                "Acceptable use".to_string(),
                "Follow the rules above. Do not attack, overload, or scrape the service; do not attempt to bypass access controls, rate limits, or the invite system; do not use ViceDen for anything unlawful."
                    .to_string(),
            ),
            (
                "Your content".to_string(),
                "You keep ownership of what you post. Posting it gives Magrathean a licence to store, display, and distribute it as part of the forum, including in previews, search, and moderation review."
                    .to_string(),
            ),
            (
                "Moderation and removal".to_string(),
                "Magrathean can remove or hide content, close threads, and use automated and human moderation, including the classifier described in Privacy, to enforce the rules."
                    .to_string(),
            ),
            (
                "Account suspension".to_string(),
                "Magrathean can suspend or close an account, or revoke an invitation, for breaking the rules or these terms, without prior notice for serious or repeated breaches."
                    .to_string(),
            ),
            (
                "No warranty".to_string(),
                "ViceDen is provided as it is, without warranty of any kind. Magrathean does not promise the service will always be available or free of errors."
                    .to_string(),
            ),
            (
                "Law".to_string(),
                "These terms are governed by the law of England and Wales. The Magrathean terms (https://magrathean.uk/terms/) apply alongside them, including their sections on liability and law."
                    .to_string(),
            ),
        ],
    })
}

async fn privacy(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> HtmlTemplate<templates::StaticPageTemplate> {
    let auth = session_from_headers(&headers, &state).await;
    let public = build_public_context(&state, auth.as_ref()).await;

    HtmlTemplate(templates::StaticPageTemplate {
        public,
        title: "Privacy".to_string(),
        kicker: "Policy".to_string(),
        intro: "ViceDen stores the minimum needed to run the forum and moderation queue."
            .to_string(),
        sections: vec![
            (
                "Account data".to_string(),
                "Handle, password hash, sessions, profile settings, PATs, and moderation events are stored."
                    .to_string(),
            ),
            (
                "Forum content".to_string(),
                "Threads, posts, votes, reports, subscriptions, hides, saves, and notifications are persisted."
                    .to_string(),
            ),
            (
                "Operational data".to_string(),
                "IP data may be used for abuse controls and founder moderation actions.".to_string(),
            ),
            (
                "Moderation".to_string(),
                "The text of posts and threads is sent to Anthropic's API for automated moderation classification, to help identify content that breaks the rules. Anthropic processes that text on Magrathean's behalf under its commercial terms, which do not allow it to train models on it. We use the result only for moderation.".to_string(),
            ),
            (
                "Retention".to_string(),
                "Board TTL still applies. Expired root threads and their descendants are removed."
                    .to_string(),
            ),
        ],
    })
}

async fn view_board(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<BoardQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        posts::PostRepo, preferences::PreferencesRepo, tags::TagRepo,
        thread_states::ThreadStateRepo, users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let Some(board) = state.boards.iter().find(|board| board.slug == slug).cloned() else {
        let auth = session_from_headers(&headers, &state).await;
        return error_page_response(
            &state,
            auth.as_ref(),
            ErrorPageSpec {
                status: StatusCode::NOT_FOUND,
                title: "Board not found",
                intro: "That board slug does not exist.",
                details: String::new(),
                next_step: "Pick one of the live boards from the forum index.",
                action_href: "/",
                action_label: "Back to forum",
            },
        )
        .await;
    };

    let auth = session_from_headers(&headers, &state).await;
    let user_id = auth.as_ref().map(|session| session.user_id).unwrap_or(GUEST_USER_ID);
    let sort = query.sort.clone().unwrap_or_else(|| "new".to_string());
    let page = query.page.unwrap_or(1).max(1);

    // A6.6 strangle: read every list entry from Postgres. required Postgres.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };

    // Total visible-thread count for the page-count math.
    let visible_count = match crate::db::count_visible_board_threads(pool, user_id, &slug).await {
        Ok(n) => n,
        Err(error) => {
            tracing::warn!(error = %error, slug = %slug, "view_board: count failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "board unavailable").into_response();
        }
    };
    let page_count = visible_count.max(1).div_ceil(BOARD_THREADS_PER_PAGE);
    let offset = (page.saturating_sub(1)) * BOARD_THREADS_PER_PAGE;

    // Page of previews — `db::list_board_thread_previews` already speaks the
    // sort modes (new / top / hot / active / unanswered) and is PG-only.
    // Will move to a `ThreadRepo.previews_for_board` method when db.rs is
    // deleted in A7.
    let previews = match crate::db::list_board_thread_previews(
        pool,
        user_id,
        &slug,
        &sort,
        BOARD_THREADS_PER_PAGE,
        offset,
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, slug = %slug, "view_board: previews failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "board unavailable").into_response();
        }
    };

    // Apply the same visibility rule used by view_thread:
    // founder + author + Visible. `visible_threads_for_board` returns the
    // viewer-aware set; intersect with `previews` to drop quarantined/removed/
    // pending threads the viewer shouldn't see.
    let visible_ids: HashSet<Uuid> = match crate::repository::threads::ThreadRepo
        .visible_threads_for_board(
            pool,
            &slug,
            user_id,
            BOARD_THREADS_PER_PAGE as i64,
            offset as i64,
        )
        .await
    {
        Ok(rows) => rows.into_iter().map(|t| t.id).collect(),
        Err(error) => {
            tracing::warn!(error = %error, slug = %slug, "view_board: visible_ids failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "board unavailable").into_response();
        }
    };
    let previews: Vec<_> =
        previews.into_iter().filter(|p| visible_ids.contains(&p.thread.id)).collect();

    // Batched lookups — author handles, thread states, and the viewer's full
    // saved-threads set (used per-thread as `is_saved`).
    let mut author_ids: Vec<Uuid> = previews.iter().map(|p| p.thread.user_id).collect();
    author_ids.sort();
    author_ids.dedup();
    let handle_map: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &author_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let thread_ids: Vec<Uuid> = previews.iter().map(|p| p.thread.id).collect();
    let state_map: HashMap<Uuid, crate::repository::thread_states::ThreadStateRow> =
        ThreadStateRepo
            .states_for_batch(pool, &thread_ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    let saved_set: HashSet<Uuid> = if user_id == GUEST_USER_ID {
        HashSet::new()
    } else {
        PreferencesRepo
            .saved_threads_for(pool, user_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect()
    };
    // B5: batch tag fetch.
    let mut tags_map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (tid, tag) in TagRepo.tags_for_threads(pool, &thread_ids).await.unwrap_or_default() {
        tags_map.entry(tid).or_default().push(tag);
    }

    // Per-thread loop (B5 folded tags into the batch above; first_post +
    // participant_count are still per-thread — small constant for a
    // 10-thread page, future B5 follow-up will batch those).
    let mut threads = Vec::with_capacity(previews.len());
    for preview in &previews {
        let thread_id = preview.thread.id;
        let snippet = PostRepo
            .first_post_for_thread(pool, thread_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.body.chars().take(220).collect::<String>())
            .unwrap_or_default();
        let tags = tags_map.remove(&thread_id).unwrap_or_default();
        let participant_count =
            PostRepo.participant_count(pool, thread_id).await.unwrap_or(0) as usize;
        let state_row = state_map.get(&thread_id).copied().unwrap_or_default();
        let thread_state = forum::ThreadState {
            pinned: state_row.pinned,
            locked: state_row.locked,
            answered: state_row.answered,
            closed: state_row.closed,
        };
        let author = handle_map.get(&preview.thread.user_id).cloned().unwrap_or_else(|| {
            format!("user-{}", &preview.thread.user_id.simple().to_string()[..8])
        });
        let board_slug = preview.thread.board_slug.clone();
        let board_name_text = board_name(&state.boards, &board_slug);
        threads.push(templates::ThreadListItem {
            thread_id,
            title: preview.thread.title.clone(),
            board_slug,
            board_name: board_name_text,
            author,
            created_at: local_time_text(preview.thread.created_at),
            last_activity: local_time_text(preview.last_activity),
            snippet,
            tags,
            reply_count: preview.reply_count,
            participant_count,
            score: preview.score,
            score_hidden: preview.score_hidden,
            authored_by_current_user: preview.thread.user_id == user_id,
            is_saved: saved_set.contains(&thread_id),
            thread_state,
        });
    }

    let subscribed = if user_id == GUEST_USER_ID {
        false
    } else {
        PreferencesRepo.is_board_subscribed(pool, user_id, &slug).await.unwrap_or(false)
    };

    let shell = build_shell_context(&state, auth.as_ref(), 12, slug.clone()).await;

    HtmlTemplate(templates::BoardTemplate {
        chrome: shell,
        board,
        board_slug: slug.clone(),
        threads,
        sort,
        page,
        page_count,
        board_thread_count: visible_count,
        subscribed,
    })
    .into_response()
}

async fn create_thread_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(payload): Form<CreateThreadForm>,
) -> impl IntoResponse {
    use crate::models::Thread;
    use crate::repository::{
        posts::{ModState, NewPost, PostRepo},
        tags::TagRepo,
        threads::{NewThread, ThreadRepo},
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let title = payload.title.trim();
    let body = payload.body.trim();

    if title.is_empty() || body.is_empty() {
        return (StatusCode::BAD_REQUEST, "title and body are required").into_response();
    }
    if title.chars().count() > 64 {
        return (StatusCode::BAD_REQUEST, "title cannot exceed 64 characters").into_response();
    }
    if body.chars().count() > 32_000 {
        return (StatusCode::BAD_REQUEST, "body cannot exceed 32,000 characters").into_response();
    }
    if let Some(response) = reject_spam_body_web(body) {
        return response;
    }

    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let Some(board) = state.boards.iter().find(|board| board.slug == slug) else {
        return (StatusCode::NOT_FOUND, "board not found").into_response();
    };
    match UserRepo.find_by_id(pool, user_id).await {
        Ok(Some(user)) if !user.banned => {
            if active_probation_until(user.probation_until).is_some() {
                return (StatusCode::FORBIDDEN, "probation active").into_response();
            }
            match crate::services::limits::posting_limit_violation(
                pool, user_id, &user.role, &slug, true,
            )
            .await
            {
                Ok(Some(limit)) => {
                    return (limit.status_code(), limit.message()).into_response();
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(?error, "thread limit check failed");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "thread create failed")
                        .into_response();
                }
            }
        }
        _ => return (StatusCode::FORBIDDEN, "user cannot post in this board").into_response(),
    }

    let now = Utc::now();
    let thread = Thread {
        id: Uuid::new_v4(),
        board_slug: slug.clone(),
        title: title.to_string(),
        user_id,
        created_at: now,
        expires_at: now + chrono::Duration::days(i64::from(board.ttl_days)),
    };
    let post_id = Uuid::new_v4();
    let normalized_tags = forum::normalize_thread_tags(split_tag_form(payload.tags.as_deref()));
    let tag_refs = normalized_tags.iter().map(String::as_str).collect::<Vec<_>>();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error) => {
            tracing::warn!(?error, "thread tx begin failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "thread create failed").into_response();
        }
    };
    if let Err(error) = ThreadRepo
        .insert(
            &mut *tx,
            NewThread {
                id: thread.id,
                board_slug: thread.board_slug.clone(),
                user_id,
                title: thread.title.clone(),
                expires_at: thread.expires_at,
            },
        )
        .await
    {
        tracing::warn!(?error, "thread insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "thread create failed").into_response();
    }
    if let Err(error) = PostRepo
        .insert(
            &mut *tx,
            NewPost {
                id: post_id,
                thread_id: thread.id,
                user_id,
                body: body.to_string(),
                mod_state: if state.moderation.is_some() {
                    ModState::Pending
                } else {
                    ModState::Visible
                },
            },
        )
        .await
    {
        tracing::warn!(?error, "root post insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "thread create failed").into_response();
    }
    if let Err(error) = TagRepo.set_for_thread(&mut tx, thread.id, &tag_refs).await {
        tracing::warn!(?error, "tag insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "thread create failed").into_response();
    }
    if let Err(error) = tx.commit().await {
        tracing::warn!(?error, "thread tx commit failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "thread create failed").into_response();
    }

    let body_for_classifier = body.to_string();
    let suffix = match state.moderation.as_ref() {
        Some(pipeline) => {
            let outcome = pipeline.classify_thread(thread.id, &body_for_classifier).await;
            moderation_query_suffix(&outcome)
        }
        _ => "",
    };

    Redirect::to(&format!("/t/{}{}", thread.id, suffix)).into_response()
}

async fn toggle_board_subscription_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::preferences::PreferencesRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let return_to = services::redirects::safe_return_to(payload.return_to, format!("/b/{slug}"));
    let user_id = session.user_id;

    // A7 partial: write path now goes straight to PreferencesRepo. The
    // legacy `store.toggle_board_subscription` also did handle-blocked
    // and capacity checks, but for board subscriptions there are no
    // such guards — it's a pure (user_id, slug) toggle.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let already_subbed =
        PreferencesRepo.is_board_subscribed(pool, user_id, &slug).await.unwrap_or(false);
    let outcome = if already_subbed {
        PreferencesRepo.unsubscribe_board(pool, user_id, &slug).await
    } else {
        PreferencesRepo.subscribe_board(pool, user_id, &slug).await
    };
    match outcome {
        Ok(()) => Redirect::to(&return_to).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "toggle_board_subscription_web: PreferencesRepo failed");
            (StatusCode::BAD_REQUEST, "subscription update failed").into_response()
        }
    }
}

async fn view_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<ThreadViewQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        posts::{ModState, PostRepo},
        preferences::PreferencesRepo,
        tags::TagRepo,
        thread_states::ThreadStateRepo,
        threads::ThreadRepo,
        users::UserRepo,
        votes::VoteRepo,
    };
    use std::collections::HashMap;

    let auth = session_from_headers(&headers, &state).await;
    let user_id = auth.as_ref().map(|session| session.user_id).unwrap_or(GUEST_USER_ID);

    // A6.5 strangle: render exclusively from Postgres. Required Postgres (    // pool) returns 503 — the boot policy already requires PG in deployed
    // mode, so this only trips in dev fixtures that intentionally skip PG.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };

    // Step 1: thread row exists?
    let thread_row = match ThreadRepo.find_by_id(pool, id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return error_page_response(
                &state,
                auth.as_ref(),
                ErrorPageSpec {
                    status: StatusCode::NOT_FOUND,
                    title: "Thread not found",
                    intro: "That thread is gone or never existed.",
                    details: String::new(),
                    next_step: "Go back to the board list or front page.",
                    action_href: "/",
                    action_label: "Back to forum",
                },
            )
            .await;
        }
        Err(error) => {
            tracing::warn!(error = %error, %id, "view_thread: ThreadRepo.find_by_id failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "thread unavailable").into_response();
        }
    };

    // Step 2: viewer role for visibility + founder bypass.
    let viewer_is_founder = if user_id == GUEST_USER_ID {
        false
    } else {
        match UserRepo.find_by_id(pool, user_id).await {
            Ok(Some(row)) => row.role == "founder",
            Ok(None) | Err(_) => false,
        }
    };

    // Step 3: visibility — hide quarantined/removed/pending from non-author non-founder.
    let thread_visible = thread_row.mod_state == ModState::Visible
        || thread_row.user_id == user_id
        || viewer_is_founder;
    if !thread_visible {
        return error_page_response(
            &state,
            auth.as_ref(),
            ErrorPageSpec {
                status: StatusCode::NOT_FOUND,
                title: "Thread not found",
                intro: "That thread is gone or never existed.",
                details: String::new(),
                next_step: "Go back to the board list or front page.",
                action_href: "/",
                action_label: "Back to forum",
            },
        )
        .await;
    }

    // Step 4: visible posts (PostRepo applies the viewer-aware filter in SQL).
    let post_rows = match PostRepo.visible_posts_for_thread(pool, id, user_id).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, %id, "view_thread: visible_posts_for_thread failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "thread unavailable").into_response();
        }
    };

    // Step 5: distinct author user-ids for one batched handle lookup.
    let mut author_ids: Vec<Uuid> = post_rows.iter().map(|p| p.user_id).collect();
    author_ids.push(thread_row.user_id);
    author_ids.sort();
    author_ids.dedup();
    let handle_map: HashMap<Uuid, String> =
        match UserRepo.handle_batch_lookup(pool, &author_ids).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(error) => {
                tracing::warn!(error = %error, "view_thread: handle_batch_lookup failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "thread unavailable").into_response();
            }
        };
    let display_name = |uid: Uuid| -> String {
        handle_map
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| format!("user-{}", &uid.simple().to_string()[..8]))
    };

    // Step 6: vote score + viewer-vote per post — two batched queries, not 2N.
    let score_map: HashMap<Uuid, i64> = match VoteRepo.scores_for_thread(pool, id).await {
        Ok(rows) => rows.into_iter().collect(),
        Err(error) => {
            tracing::warn!(error = %error, %id, "view_thread: scores_for_thread failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "thread unavailable").into_response();
        }
    };
    let viewer_vote_map: HashMap<Uuid, i16> = if user_id == GUEST_USER_ID {
        HashMap::new()
    } else {
        match VoteRepo.user_votes_for_thread(pool, user_id, id).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(error) => {
                tracing::warn!(error = %error, "view_thread: user_votes_for_thread failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "thread unavailable").into_response();
            }
        }
    };

    // Step 7: build per-post template entries.
    let reply_count = post_rows.len().saturating_sub(1);
    let thread_posts: Vec<templates::ThreadPost> = post_rows
        .iter()
        .map(|post| templates::ThreadPost {
            id: post.id,
            author: display_name(post.user_id),
            body_html: services::safe_html::render_user_html(&post.body),
            created_at: local_time_text(post.created_at),
            is_current_user: post.user_id == user_id,
            score: score_map.get(&post.id).copied().unwrap_or(0) as i32,
            user_vote: viewer_vote_map.get(&post.id).copied().unwrap_or(0) as i8,
            score_hidden: forum::is_score_hidden(post.created_at),
        })
        .collect();

    // Step 8: thread-level aggregates.
    let thread_score = if forum::is_score_hidden(thread_row.created_at) {
        None
    } else {
        match VoteRepo.score_for_thread(pool, id).await {
            Ok(score) => Some(score as i32),
            Err(error) => {
                tracing::warn!(error = %error, %id, "view_thread: score_for_thread failed");
                Some(0)
            }
        }
    };
    let participant_count = PostRepo.participant_count(pool, id).await.unwrap_or(0) as usize;
    let thread_state_row = match ThreadStateRepo.state_for(pool, id).await {
        Ok(opt) => opt.unwrap_or_default(),
        Err(error) => {
            tracing::warn!(error = %error, %id, "view_thread: state_for failed");
            crate::repository::thread_states::ThreadStateRow::default()
        }
    };
    let thread_state = forum::ThreadState {
        pinned: thread_state_row.pinned,
        locked: thread_state_row.locked,
        answered: thread_state_row.answered,
        closed: thread_state_row.closed,
    };
    let thread_tags = TagRepo.list_for_thread(pool, id).await.unwrap_or_default();

    // Step 9: viewer-specific prefs (skip for guests).
    let is_saved = if user_id == GUEST_USER_ID {
        false
    } else {
        PreferencesRepo.is_thread_saved(pool, user_id, id).await.unwrap_or(false)
    };
    let is_hidden = if user_id == GUEST_USER_ID {
        false
    } else {
        PreferencesRepo.is_thread_hidden(pool, user_id, id).await.unwrap_or(false)
    };

    // Step 10: board-level downvote rule.
    //
    // The legacy `store.downvoting_allowed_for_board` reads
    // `board_downvote_enabled: HashMap<String, bool>` — an in-memory admin
    // toggle with no Postgres home today. Default `true` matches the
    // legacy `.unwrap_or(true)` fallback for boards that haven't been
    // explicitly disabled. A future migration adding a `boards`-table
    // column (or a `board_settings` table) lets us strangle this properly;
    // until then admins toggling downvotes per-board is a known gap.
    let downvote_allowed = true;

    // Convert ThreadRow → models::Thread for the template.
    let thread = crate::models::Thread {
        id: thread_row.id,
        board_slug: thread_row.board_slug.clone(),
        title: thread_row.title.clone(),
        user_id: thread_row.user_id,
        created_at: thread_row.created_at,
        expires_at: thread_row.expires_at,
    };
    let now = Utc::now();
    let remaining = thread.expires_at - now;
    let days_left = remaining.num_days().max(0);

    let thread_board_slug = thread.board_slug.clone();
    let shell = build_shell_context(&state, auth.as_ref(), 8, thread_board_slug.clone()).await;
    let thread_board_name = board_name(&state.boards, &thread_board_slug);

    let participants: Vec<String> = author_ids.iter().map(|uid| display_name(*uid)).collect();
    let participants_json = to_string(&participants).unwrap_or_else(|_| "[]".to_string());

    let moderation_state =
        query.moderation.as_deref().map(normalize_moderation_flag).unwrap_or_default();

    HtmlTemplate(templates::ThreadTemplate {
        chrome: shell,
        thread,
        thread_author: display_name(thread_row.user_id),
        thread_board_name,
        thread_posted_at: local_time_text(thread_row.created_at),
        thread_delete_note: format!(
            "This thread self-destructs in {} day{}.",
            days_left,
            if days_left == 1 { "" } else { "s" }
        ),
        posts: thread_posts,
        posting_notice: "Reply box stays at the bottom on purpose. Read first. Then speak."
            .to_string(),
        thread_participants: participants,
        thread_participants_json: participants_json,
        can_vote_down: downvote_allowed,
        thread_score_text: thread_score
            .map(|value| value.to_string())
            .unwrap_or_else(|| "hidden for first 3 hours".to_string()),
        thread_score,
        is_saved,
        is_hidden,
        thread_tags,
        thread_state,
        participant_count,
        reply_count,
        moderation_state,
    })
    .into_response()
}

/// Map an externally-supplied `?moderation=` value to one of the recognised
/// banner states. Anything not in {`pending`, `quarantined`, `removed`} is
/// dropped to the empty string so the template suppresses the banner.
fn normalize_moderation_flag(value: &str) -> String {
    match value {
        "pending" | "quarantined" | "removed" => value.to_string(),
        _ => String::new(),
    }
}

/// Build the `?moderation=…` redirect suffix for create flows. Returns an
/// empty string for `Allowed`.
fn moderation_query_suffix(
    outcome: &services::moderation_pipeline::PipelineOutcome,
) -> &'static str {
    use services::moderation_pipeline::PipelineOutcome::*;
    match outcome {
        Allowed => "",
        Pending => "?moderation=pending",
        Quarantined => "?moderation=quarantined",
        Rejected => "?moderation=removed",
    }
}

async fn toggle_thread_save_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::preferences::PreferencesRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let return_to = services::redirects::safe_return_to(payload.return_to, format!("/t/{id}"));
    let user_id = session.user_id;

    // A7 partial: write path → PreferencesRepo. Toggle: insert if
    // absent, delete if present. is_thread_saved is the cheap probe.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let already_saved = PreferencesRepo.is_thread_saved(pool, user_id, id).await.unwrap_or(false);
    let outcome = if already_saved {
        PreferencesRepo.unsave_thread(pool, user_id, id).await
    } else {
        PreferencesRepo.save_thread(pool, user_id, id).await
    };
    match outcome {
        Ok(()) => Redirect::to(&return_to).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "toggle_thread_save_web: PreferencesRepo failed");
            (StatusCode::BAD_REQUEST, "save toggle failed").into_response()
        }
    }
}

async fn toggle_thread_hide_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::preferences::PreferencesRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let return_to = services::redirects::safe_return_to(payload.return_to, format!("/t/{id}"));

    // A7 partial: write path → PreferencesRepo. Same toggle pattern.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let already_hidden =
        PreferencesRepo.is_thread_hidden(pool, session.user_id, id).await.unwrap_or(false);
    let outcome = if already_hidden {
        PreferencesRepo.unhide_thread(pool, session.user_id, id).await
    } else {
        PreferencesRepo.hide_thread(pool, session.user_id, id).await
    };
    match outcome {
        Ok(()) => Redirect::to(&return_to).into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "toggle_thread_hide_web: PreferencesRepo failed");
            (StatusCode::BAD_REQUEST, "hide toggle failed").into_response()
        }
    }
}

async fn create_post_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<CreatePostForm>,
) -> impl IntoResponse {
    use crate::models::{Post, PostState};
    use crate::repository::{
        posts::{ModState, NewPost, PostRepo},
        thread_states::ThreadStateRepo,
        threads::ThreadRepo,
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let body = payload.body.trim();
    if body.is_empty() {
        return (StatusCode::BAD_REQUEST, "body is required").into_response();
    }
    if body.chars().count() > 32_000 {
        return (StatusCode::BAD_REQUEST, "body cannot exceed 32,000 characters").into_response();
    }
    if let Some(response) = reject_spam_body_web(body) {
        return response;
    }

    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let Some(_thread) = (match ThreadRepo.find_by_id(pool, id).await {
        Ok(thread) => thread,
        Err(error) => {
            tracing::warn!(?error, "thread lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "post failed").into_response();
        }
    }) else {
        return (StatusCode::NOT_FOUND, "thread not found").into_response();
    };
    let user = match UserRepo.find_by_id(pool, user_id).await {
        Ok(Some(user)) if !user.banned => user,
        _ => return (StatusCode::FORBIDDEN, "user cannot post in this board").into_response(),
    };
    if active_probation_until(user.probation_until).is_some() {
        return (StatusCode::FORBIDDEN, "probation active").into_response();
    }
    match crate::services::limits::posting_limit_violation(
        pool,
        user_id,
        &user.role,
        &_thread.board_slug,
        false,
    )
    .await
    {
        Ok(Some(limit)) => {
            return (limit.status_code(), limit.message()).into_response();
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(?error, "reply limit check failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "post failed").into_response();
        }
    }
    let state_row =
        ThreadStateRepo.state_for(pool, id).await.unwrap_or_default().unwrap_or_default();
    let can_moderate = user.role == "founder" || user.role == "mod";
    if (state_row.locked || state_row.closed) && !can_moderate {
        return (StatusCode::FORBIDDEN, "thread is locked").into_response();
    }

    let now = Utc::now();
    let post = Post {
        id: Uuid::new_v4(),
        thread_id: id,
        user_id,
        body: body.to_string(),
        created_at: now,
        state: PostState::Allow,
    };
    if let Err(error) = PostRepo
        .insert(
            pool,
            NewPost {
                id: post.id,
                thread_id: id,
                user_id,
                body: post.body.clone(),
                mod_state: if state.moderation.is_some() {
                    ModState::Pending
                } else {
                    ModState::Visible
                },
            },
        )
        .await
    {
        tracing::warn!(?error, "post insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "post failed").into_response();
    }

    let body_for_classifier = body.to_string();
    let suffix = match state.moderation.as_ref() {
        Some(pipeline) => {
            let outcome = pipeline.classify_post(post.id, &body_for_classifier).await;
            moderation_query_suffix(&outcome)
        }
        _ => "",
    };

    Redirect::to(&format!("/t/{id}{suffix}#post-{post_id}", post_id = post.id)).into_response()
}

async fn vote_post_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((thread_id, post_id)): Path<(Uuid, Uuid)>,
    Form(payload): Form<VoteForm>,
) -> impl IntoResponse {
    use crate::services::voting_policy::{self, VotePolicyError};

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;

    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    if let Err(error) =
        voting_policy::cast_vote(pool, &state.boards, user_id, post_id, payload.value).await
    {
        return match error {
            VotePolicyError::PostNotFound => {
                (StatusCode::NOT_FOUND, "post not found").into_response()
            }
            VotePolicyError::BoardNotFound => {
                (StatusCode::NOT_FOUND, "board not found").into_response()
            }
            VotePolicyError::UserCannotVote | VotePolicyError::DownvotesDisabled => {
                (StatusCode::FORBIDDEN, error.to_string()).into_response()
            }
            VotePolicyError::InvalidValue => {
                (StatusCode::BAD_REQUEST, "invalid vote value").into_response()
            }
            VotePolicyError::Database => {
                tracing::warn!(%post_id, "vote_post_web: policy check failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "vote unavailable").into_response()
            }
        };
    }
    Redirect::to(&format!("/t/{thread_id}")).into_response()
}

async fn view_notifications(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    use crate::repository::notifications::NotificationRepo;

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;

    // A7 partial: mark-all-read on visit goes through PG. Required Postgres
    // mode silently no-ops (the page renders without the side effect).
    if let Some(pool) = state.db.as_ref() {
        if let Err(error) = NotificationRepo.mark_all_read(pool, user_id).await {
            tracing::warn!(error = %error, "view_notifications: mark_all_read failed");
        }
    }

    let shell = build_shell_context(&state, Some(&session), 8, String::new()).await;
    HtmlTemplate(templates::NotificationsTemplate { chrome: shell }).into_response()
}

async fn live_state_web(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    use crate::repository::{
        notifications::NotificationRepo, reports::ReportRepo, users::UserRepo,
    };

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let unread_notifications =
        NotificationRepo.count_unread(pool, session.user_id).await.unwrap_or(0) as usize;
    // Open-reports surfaced only to founders; non-founders always see 0.
    let viewer_is_founder = matches!(
        UserRepo.find_by_id(pool, session.user_id).await,
        Ok(Some(row)) if row.role == "founder"
    );
    let open_reports =
        if viewer_is_founder { ReportRepo.count_open(pool).await.unwrap_or(0) as usize } else { 0 };
    Json(LiveStateResponse { unread_notifications, open_reports }).into_response()
}

async fn remove_notification_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::notifications::NotificationRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    if let Err(error) = NotificationRepo.delete_for_user(pool, id, user_id).await {
        tracing::warn!(error = %error, %id, "remove_notification_web: delete failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response();
    }
    Redirect::to("/notifications").into_response()
}

async fn mark_single_notification_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<NotificationReadForm>,
) -> impl IntoResponse {
    use crate::repository::notifications::NotificationRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let read = payload.read.unwrap_or(true);
    if let Err(error) = NotificationRepo.set_read(pool, id, user_id, read).await {
        tracing::warn!(error = %error, %id, "mark_single_notification_web: set_read failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "update failed").into_response();
    }
    Redirect::to("/notifications").into_response()
}

async fn mark_all_notifications_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::notifications::NotificationRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    if let Err(error) = NotificationRepo.mark_all_read(pool, user_id).await {
        tracing::warn!(error = %error, "mark_all_notifications_web: mark_all_read failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "update failed").into_response();
    }
    Redirect::to("/notifications").into_response()
}

async fn profile(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    use crate::repository::{
        pats::PatRepo, preferences::PreferencesRepo, threads::ThreadRepo, users::UserRepo,
    };

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };

    let (
        own_threads,
        saved_thread_items,
        subscribed_board_cards,
        total_threads,
        total_posts,
        api_tokens,
        notification_settings,
        probation_until,
    ) = {
        let mut own_previews = Vec::new();
        for board in &state.boards {
            if let Ok(previews) =
                crate::db::list_board_thread_previews(pool, user_id, &board.slug, "new", 50, 0)
                    .await
            {
                own_previews.extend(
                    previews.into_iter().filter(|preview| preview.thread.user_id == user_id),
                );
            }
        }
        own_previews.sort_by_key(|preview| std::cmp::Reverse(preview.thread.created_at));
        own_previews.truncate(12);
        let mut own_threads = Vec::new();
        for preview in own_previews {
            own_threads.push(thread_item_from_preview_db(pool, &state, user_id, preview).await);
        }

        let saved_ids = PreferencesRepo.saved_threads_for(pool, user_id).await.unwrap_or_default();
        let mut saved_thread_items = Vec::new();
        for thread_id in saved_ids.into_iter().take(12) {
            if let Ok(Some(preview)) = ThreadRepo.find_with_preview(pool, thread_id).await {
                saved_thread_items
                    .push(thread_item_from_thread_preview_db(pool, &state, user_id, preview).await);
            }
        }

        let subscribed_slugs =
            PreferencesRepo.subscribed_slugs_for(pool, user_id).await.unwrap_or_default();
        let subscribed_set =
            subscribed_slugs.iter().map(String::as_str).collect::<std::collections::HashSet<_>>();
        let mut subscribed_board_cards = Vec::new();
        for board in
            state.boards.iter().filter(|board| subscribed_set.contains(board.slug.as_str()))
        {
            let thread_count = crate::db::count_visible_board_threads(pool, user_id, &board.slug)
                .await
                .unwrap_or(0);
            subscribed_board_cards.push(templates::BoardCard {
                slug: board.slug.clone(),
                name: board.name.clone(),
                description: board.description.clone(),
                thread_count,
                subscribed: true,
            });
        }

        let api_tokens = PatRepo
            .list_for_user(pool, user_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|item| templates::PatItem {
                id: item.id.to_string(),
                name: item.name,
                preview: item.preview,
                created_at: local_time_text(item.created_at),
                last_used_at: item.last_used_at.map(local_time_text),
            })
            .collect::<Vec<_>>();
        let notification_settings =
            PreferencesRepo.settings_for(pool, user_id).await.unwrap_or_default();
        let detail_user = UserRepo.find_by_id(pool, user_id).await.unwrap_or(None);
        let summary = UserRepo.summary_with_counts(pool, user_id).await.unwrap_or(None);

        (
            own_threads,
            saved_thread_items,
            subscribed_board_cards,
            summary.as_ref().map_or(0, |row| row.thread_count as usize),
            summary.as_ref().map_or(0, |row| row.post_count as usize),
            api_tokens,
            notification_settings,
            detail_user.and_then(|user| active_probation_until(user.probation_until)),
        )
    };

    let shell = build_shell_context(&state, Some(&session), 12, String::new()).await;
    HtmlTemplate(templates::ProfileTemplate {
        chrome: shell,
        own_threads,
        own_thread_count: total_threads,
        own_post_count: total_posts,
        saved_thread_items,
        subscribed_board_cards,
        api_tokens,
        notify_mentions: notification_settings.notify_mentions,
        notify_replies: notification_settings.notify_replies,
        notify_moderation: notification_settings.notify_moderation,
        probation_until: probation_until.map(local_time_text),
    })
    .into_response()
}

async fn view_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<UsersQuery>,
) -> impl IntoResponse {
    use crate::repository::users::UserRepo;

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let search = query.q.unwrap_or_default();
    // A6.2 strangle: read the admin user list from Postgres. Search filter
    // stays in-memory (legacy semantic — handles a few hundred users).
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => {
            return (StatusCode::FORBIDDEN, "founder permission required").into_response();
        }
        Err(error) => {
            tracing::warn!(error = %error, "view_users: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user list unavailable").into_response();
        }
    };
    let summaries = match UserRepo.list_summaries_for_admin(pool).await {
        Ok(summaries) => summaries,
        Err(error) => {
            tracing::warn!(error = %error, "view_users: list_summaries_for_admin failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user list unavailable").into_response();
        }
    };
    let search_lower = search.trim().to_ascii_lowercase();
    let users: Vec<templates::AdminUserItem> = summaries
        .into_iter()
        .filter(|user| {
            search_lower.is_empty() || user.handle.to_ascii_lowercase().contains(&search_lower)
        })
        .map(|user| templates::AdminUserItem {
            id: user.id.to_string(),
            // `can_manage` mirrors the legacy `!user.is_founder()` derivation.
            can_manage: user.role != "founder",
            handle: user.handle,
            status: admin_user_status(
                user.banned,
                user.deactivated_at.is_some(),
                active_probation_until(user.probation_until).is_some(),
            ),
            role: user.role,
            thread_count: user.thread_count as usize,
            post_count: user.post_count as usize,
            profile_url: format!("/admin/users/{}", user.id),
        })
        .collect();

    let shell = build_shell_context(&state, Some(&session), 8, String::new()).await;
    HtmlTemplate(templates::UsersTemplate { chrome: shell, search, users }).into_response()
}

async fn view_user_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    use crate::repository::{
        invites::InviteRepo, moderation_events::ModerationEventRepo, posts::PostRepo,
        preferences::PreferencesRepo, tags::TagRepo, thread_states::ThreadStateRepo,
        users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let root_id = session.user_id;

    // A6.3 strangle: read everything from Postgres. required Postgres.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };

    // Founder gate via UserRepo role check.
    match UserRepo.find_by_id(pool, root_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => {
            return (StatusCode::FORBIDDEN, "founder permission required").into_response();
        }
        Err(error) => {
            tracing::warn!(error = %error, "view_user_detail: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user detail unavailable").into_response();
        }
    };

    // Target user summary. None → 404.
    let summary = match UserRepo.summary_with_counts(pool, id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return error_page_response(
                &state,
                Some(&session),
                ErrorPageSpec {
                    status: StatusCode::NOT_FOUND,
                    title: "User not found",
                    intro: "That account does not exist.",
                    details: String::new(),
                    next_step: "Return to the user index and search again.",
                    action_href: "/admin/users",
                    action_label: "Back to users",
                },
            )
            .await;
        }
        Err(error) => {
            tracing::warn!(error = %error, "view_user_detail: summary lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user detail unavailable").into_response();
        }
    };

    let detail_row = match UserRepo.find_by_id(pool, id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "user detail unavailable").into_response();
        }
        Err(error) => {
            tracing::warn!(error = %error, "view_user_detail: detail row lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "user detail unavailable").into_response();
        }
    };

    // user_posts via the existing latest_activity repo method (already PG).
    let user_posts: Vec<templates::UserPostTeaser> = UserRepo
        .latest_activity(pool, id, 100)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|item| templates::UserPostTeaser {
            board_name: board_name(&state.boards, &item.board_slug),
            board_slug: item.board_slug,
            thread_id: item.thread_id,
            post_id: item.post_id,
            url: format!("/t/{}#post-{}", item.thread_id, item.post_id),
            thread_title: item.thread_title,
            created_at: local_time_text(item.created_at),
        })
        .collect();

    // user_threads: threads STARTED by this user with the preview shape.
    // Strategy mirrors A6.6/A6.8 — iterate boards, call
    // db::list_board_thread_previews per board, filter by author == id,
    // merge + sort + truncate. With ~5–10 boards × 100 = 500 rows pulled
    // and filtered down. B5 will fold into a single
    // `threads_for_author_with_preview` query.
    let mut threads_by_user: Vec<crate::forum::ThreadPreview> = Vec::new();
    for board in &state.boards {
        if let Ok(previews) =
            crate::db::list_board_thread_previews(pool, root_id, &board.slug, "new", 100, 0).await
        {
            threads_by_user.extend(previews.into_iter().filter(|p| p.thread.user_id == id));
        }
    }
    threads_by_user.sort_by_key(|p| std::cmp::Reverse(p.thread.created_at));
    threads_by_user.truncate(100);

    // Same render-bits batching as A6.6/A6.8.
    let thread_ids: Vec<Uuid> = threads_by_user.iter().map(|p| p.thread.id).collect();
    let state_map: HashMap<Uuid, crate::repository::thread_states::ThreadStateRow> =
        ThreadStateRepo
            .states_for_batch(pool, &thread_ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    let saved_set: HashSet<Uuid> = PreferencesRepo
        .saved_threads_for(pool, root_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    // The author of all these threads is the same user (id), so handle
    // lookup is one row.
    let author_handle = UserRepo
        .handle_batch_lookup(pool, &[id])
        .await
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|(_, h)| h)
        .unwrap_or_else(|| format!("user-{}", &id.simple().to_string()[..8]));

    // B5: batch tag fetch for the user_threads list.
    let mut tags_map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (tid, tag) in TagRepo.tags_for_threads(pool, &thread_ids).await.unwrap_or_default() {
        tags_map.entry(tid).or_default().push(tag);
    }

    let mut user_threads = Vec::with_capacity(threads_by_user.len());
    for preview in &threads_by_user {
        let thread_id = preview.thread.id;
        let snippet = PostRepo
            .first_post_for_thread(pool, thread_id)
            .await
            .ok()
            .flatten()
            .map(|p| p.body.chars().take(220).collect::<String>())
            .unwrap_or_default();
        let tags = tags_map.remove(&thread_id).unwrap_or_default();
        let participant_count =
            PostRepo.participant_count(pool, thread_id).await.unwrap_or(0) as usize;
        let state_row = state_map.get(&thread_id).copied().unwrap_or_default();
        let thread_state = forum::ThreadState {
            pinned: state_row.pinned,
            locked: state_row.locked,
            answered: state_row.answered,
            closed: state_row.closed,
        };
        let board_slug = preview.thread.board_slug.clone();
        let board_name_text = board_name(&state.boards, &board_slug);
        user_threads.push(templates::ThreadListItem {
            thread_id,
            title: preview.thread.title.clone(),
            board_slug,
            board_name: board_name_text,
            author: author_handle.clone(),
            created_at: local_time_text(preview.thread.created_at),
            last_activity: local_time_text(preview.last_activity),
            snippet,
            tags,
            reply_count: preview.reply_count,
            participant_count,
            score: preview.score,
            score_hidden: preview.score_hidden,
            authored_by_current_user: preview.thread.user_id == root_id,
            is_saved: saved_set.contains(&thread_id),
            thread_state,
        });
    }

    let mut subtree_user_ids = vec![id];
    subtree_user_ids.extend(InviteRepo.subtree_descendants(pool, id).await.unwrap_or_default());
    let mut subtree_handles = UserRepo
        .handle_batch_lookup(pool, &subtree_user_ids[1..])
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_, handle)| format!("@{handle}"))
        .collect::<Vec<_>>();
    subtree_handles.sort();
    let subtree_active_invites =
        InviteRepo.count_unspent_for_inviters(pool, &subtree_user_ids).await.unwrap_or(0) as usize;
    let subtree_descendant_count = subtree_user_ids.len().saturating_sub(1);
    let moderation_events =
        ModerationEventRepo.list_for_target_user(pool, id, 20).await.unwrap_or_default();
    let moderation_actor_ids =
        moderation_events.iter().map(|item| item.actor_id).collect::<Vec<_>>();
    let moderation_actor_handles: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &moderation_actor_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let moderation_history = moderation_events
        .into_iter()
        .map(|item| templates::ModerationAuditItem {
            actor: moderation_actor_handles
                .get(&item.actor_id)
                .cloned()
                .unwrap_or_else(|| format!("user-{}", &item.actor_id.simple().to_string()[..8])),
            action: item.action,
            reason: item.reason,
            note: item.note.unwrap_or_default(),
            created_at: local_time_text(item.created_at),
        })
        .collect::<Vec<_>>();

    let shell = build_shell_context(&state, Some(&session), 8, String::new()).await;
    HtmlTemplate(templates::AdminUserDetailTemplate {
        chrome: shell,
        detail_user: templates::AdminUserItem {
            id: summary.id.to_string(),
            // can_manage: founders are not manageable.
            can_manage: summary.role != "founder",
            handle: summary.handle,
            status: admin_user_status(
                summary.banned,
                summary.deactivated_at.is_some(),
                active_probation_until(summary.probation_until).is_some(),
            ),
            role: summary.role,
            thread_count: summary.thread_count as usize,
            post_count: summary.post_count as usize,
            profile_url: format!("/admin/users/{id}"),
        },
        subtree_descendant_count,
        subtree_active_invites,
        subtree_handles,
        moderation_history,
        deactivated_reason: detail_row.deactivated_reason.unwrap_or_default(),
        probation_until: active_probation_until(detail_row.probation_until).map(local_time_text),
        user_threads,
        user_posts,
    })
    .into_response()
}

async fn cauterise_user_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<CauteriseUserForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let reason = payload.reason.unwrap_or_else(|| "invite subtree containment".into());
    match UserRepo.cauterise_subtree(pool, session.user_id, id, &reason).await {
        Ok(Some(outcome)) => {
            let note = format!(
                "{} users, {} threads, {} posts, {} invites",
                outcome.subtree_user_ids.len(),
                outcome.threads_removed,
                outcome.posts_removed,
                outcome.invites_revoked
            );
            let _ = ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id: session.user_id,
                        target_post_id: None,
                        target_thread_id: None,
                        target_user_id: Some(id),
                        report_id: None,
                        action: "cauterise_subtree".into(),
                        reason,
                        note: Some(note),
                        created_at: Utc::now(),
                    },
                )
                .await;
            Redirect::to(&format!("/admin/users/{id}")).into_response()
        }
        Ok(None) => (StatusCode::FORBIDDEN, "user cannot be cauterised").into_response(),
        Err(error) => {
            tracing::warn!(?error, "cauterise user failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "cauterise failed").into_response()
        }
    }
}

async fn lift_probation_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<LiftProbationForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let Some(actor) = UserRepo.find_by_id(pool, session.user_id).await.unwrap_or(None) else {
        return (StatusCode::FORBIDDEN, "founder permission required").into_response();
    };
    let Some(target) = UserRepo.find_by_id(pool, id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "user not found").into_response();
    };
    if actor.role != "founder" || target.role == "founder" {
        return (StatusCode::FORBIDDEN, "user cannot be updated").into_response();
    }
    if let Err(error) = UserRepo.set_probation_until(pool, id, None).await {
        tracing::warn!(?error, "lift probation failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "probation update failed").into_response();
    }
    let note = payload
        .note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let _ = ModerationEventRepo
        .insert(
            pool,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id: session.user_id,
                target_post_id: None,
                target_thread_id: None,
                target_user_id: Some(id),
                report_id: None,
                action: "lift_probation".into(),
                reason: "founder probation lift".into(),
                note,
                created_at: Utc::now(),
            },
        )
        .await;
    let return_to =
        services::redirects::safe_return_to(payload.return_to, format!("/admin/users/{id}"));
    Redirect::to(&return_to).into_response()
}

async fn wipe_user_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.wipe_content(pool, user_id, id).await {
        Ok(true) => {
            let _ = ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id: user_id,
                        target_post_id: None,
                        target_thread_id: None,
                        target_user_id: Some(id),
                        report_id: None,
                        action: "wipe_user".into(),
                        reason: "founder content wipe".into(),
                        note: None,
                        created_at: Utc::now(),
                    },
                )
                .await;
            let return_to = services::redirects::safe_return_to(
                payload.return_to,
                format!("/admin/users/{id}"),
            );
            Redirect::to(&return_to).into_response()
        }
        Ok(false) => (StatusCode::FORBIDDEN, "user cannot be wiped").into_response(),
        Err(error) => {
            tracing::warn!(?error, "wipe user failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "wipe failed").into_response()
        }
    }
}

async fn delete_user_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let target_handle = UserRepo
        .find_by_id(pool, id)
        .await
        .ok()
        .flatten()
        .map(|user| user.handle)
        .unwrap_or_else(|| format!("user-{}", &id.simple().to_string()[..8]));
    match UserRepo.delete_user(pool, user_id, id).await {
        Ok(true) => {
            let _ = ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id: user_id,
                        target_post_id: None,
                        target_thread_id: None,
                        target_user_id: None,
                        report_id: None,
                        action: "delete_user".into(),
                        reason: "founder account delete".into(),
                        note: Some(format!("@{target_handle} ({id})")),
                        created_at: Utc::now(),
                    },
                )
                .await;
            let return_to =
                services::redirects::safe_return_to(payload.return_to, "/admin/users".to_string());
            Redirect::to(&return_to).into_response()
        }
        Ok(false) => (StatusCode::FORBIDDEN, "user cannot be deleted").into_response(),
        Err(error) => {
            tracing::warn!(?error, "delete user failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}

async fn update_profile_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<UpdateProfileForm>,
) -> impl IntoResponse {
    use crate::repository::preferences::{PreferencesRepo, UpdateSettings};

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let user_id = session.user_id;
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let current = match PreferencesRepo.settings_for(pool, user_id).await {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(?error, "profile settings load failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "settings update failed").into_response();
        }
    };
    let settings = UpdateSettings {
        compactness: payload.compactness.unwrap_or(current.compactness).clamp(-2, 2),
        theme: "dark".to_string(),
        notify_mentions: payload.notify_mentions.unwrap_or(current.notify_mentions),
        notify_replies: payload.notify_replies.unwrap_or(current.notify_replies),
        notify_moderation: payload.notify_moderation.unwrap_or(current.notify_moderation),
    };
    if let Err(error) = PreferencesRepo.update_settings(pool, user_id, settings).await {
        tracing::warn!(?error, "profile settings update failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "settings update failed").into_response();
    }

    Redirect::to("/profile").into_response()
}

async fn create_pat_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<CreatePatForm>,
) -> impl IntoResponse {
    use crate::{
        auth::tokens,
        repository::{
            notifications::{NewNotification, NotificationRepo},
            pats::PatRepo,
        },
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let name = payload.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "token could not be created").into_response();
    }
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let raw_token = tokens::new_pat_token();
    let token_id = match PatRepo
        .issue(pool, session.user_id, name, &raw_token, state.config.auth.token_pepper.as_deref())
        .await
    {
        Ok(id) => id,
        Err(error) => {
            tracing::warn!(?error, "PAT create failed");
            return (StatusCode::BAD_REQUEST, "token could not be created").into_response();
        }
    };
    let preview = format!("{}...", raw_token.chars().take(12).collect::<String>());
    let notice = format!("PAT created: {name} ({preview}) save the raw token now: {raw_token}");
    let _ = NotificationRepo
        .insert(
            pool,
            NewNotification {
                id: Uuid::new_v4(),
                recipient_user_id: session.user_id,
                actor_id: None,
                kind: "invite".to_string(),
                thread_id: None,
                thread_title: "General".to_string(),
                post_id: None,
                message: notice,
                created_at: Utc::now(),
                is_read: false,
            },
        )
        .await
        .map_err(|error| tracing::warn!(?error, %token_id, "PAT notification insert failed"));

    Redirect::to("/profile").into_response()
}

async fn delete_pat_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    use crate::repository::pats::PatRepo;

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match PatRepo.revoke(pool, id, session.user_id).await {
        Ok(true) => Redirect::to("/profile").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "token not found").into_response(),
        Err(error) => {
            tracing::warn!(?error, "PAT delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "token delete failed").into_response()
        }
    }
}

async fn report_thread_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReportForm>,
) -> impl IntoResponse {
    use crate::repository::{reports::NewReport, reports::ReportRepo, threads::ThreadRepo};

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match ThreadRepo.find_by_id(pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "thread not found").into_response(),
        Err(error) => {
            tracing::warn!(?error, "report thread lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "report failed").into_response();
        }
    }
    let reason = payload.reason.trim();
    if reason.is_empty() {
        return (StatusCode::BAD_REQUEST, "reason is required").into_response();
    }
    if let Err(error) = ReportRepo
        .insert(
            pool,
            NewReport {
                id: Uuid::new_v4(),
                actor_id: session.user_id,
                target_post_id: None,
                target_thread_id: Some(id),
                reason: reason.to_string(),
                details: payload.details.filter(|value| !value.trim().is_empty()),
                created_at: Utc::now(),
            },
        )
        .await
    {
        tracing::warn!(?error, "report thread insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "report failed").into_response();
    }
    Redirect::to(&format!("/t/{id}")).into_response()
}

async fn report_post_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((thread_id, post_id)): Path<(Uuid, Uuid)>,
    Form(payload): Form<ReportForm>,
) -> impl IntoResponse {
    use crate::repository::{posts::PostRepo, reports::NewReport, reports::ReportRepo};

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let post = match PostRepo.find_by_id(pool, post_id).await {
        Ok(Some(post)) if post.thread_id == thread_id => post,
        Ok(Some(_)) | Ok(None) => return (StatusCode::NOT_FOUND, "post not found").into_response(),
        Err(error) => {
            tracing::warn!(?error, "report post lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "report failed").into_response();
        }
    };
    let reason = payload.reason.trim();
    if reason.is_empty() {
        return (StatusCode::BAD_REQUEST, "reason is required").into_response();
    }
    if let Err(error) = ReportRepo
        .insert(
            pool,
            NewReport {
                id: Uuid::new_v4(),
                actor_id: session.user_id,
                target_post_id: Some(post_id),
                target_thread_id: Some(thread_id),
                reason: reason.to_string(),
                details: payload.details.filter(|value| !value.trim().is_empty()),
                created_at: Utc::now(),
            },
        )
        .await
    {
        tracing::warn!(?error, "report post insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "report failed").into_response();
    }
    if let Some(pipeline) = state.moderation.as_ref() {
        let pipeline = pipeline.clone();
        let body = post.body.clone();
        tokio::spawn(async move {
            pipeline.reclassify_post_strict(post_id, &body).await;
        });
    }

    Redirect::to(&format!("/t/{thread_id}#post-{post_id}")).into_response()
}

async fn view_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReportsQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::ModerationEventRepo, reports::ReportRepo, users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    // A6.4 strangle: read open reports + reporter handles directly from
    // Postgres. required Postgres.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => {
            return (StatusCode::FORBIDDEN, "founder permission required").into_response();
        }
        Err(error) => {
            tracing::warn!(error = %error, "view_reports: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
        }
    };
    let filters = ReportDeskFilters::from_query(pool, query).await;
    let reports = match ReportRepo
        .list_queue_filtered(pool, filters.report_queue_filter(session.user_id))
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "view_reports: list_queue failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
        }
    };
    let actor_ids: Vec<_> = reports
        .iter()
        .flat_map(|r| [Some(r.actor_id), r.target_user_id, r.assigned_user_id])
        .collect::<HashSet<_>>()
        .into_iter()
        .flatten()
        .collect();
    let handle_map: HashMap<uuid::Uuid, String> =
        match UserRepo.handle_batch_lookup(pool, &actor_ids).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(error) => {
                tracing::warn!(error = %error, "view_reports: handle_batch_lookup failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
            }
        };
    let mut urgent_reports = Vec::new();
    let mut normal_reports = Vec::new();
    for report in reports {
        let audit_history = match ModerationEventRepo.list_for_report(pool, report.id).await {
            Ok(rows) => rows
                .into_iter()
                .map(|item| templates::ReportAuditItem {
                    actor: handle_map.get(&item.actor_id).cloned().unwrap_or_else(|| {
                        format!("user-{}", &item.actor_id.simple().to_string()[..8])
                    }),
                    action: item.action,
                    note: item.note.unwrap_or_default(),
                    created_at: local_time_text(item.created_at),
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(error = %error, "view_reports: audit lookup failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
            }
        };
        let item = templates::ReportItem {
            id: report.id.to_string(),
            actor: handle_map
                .get(&report.actor_id)
                .cloned()
                .unwrap_or_else(|| format!("user-{}", &report.actor_id.simple().to_string()[..8])),
            target_author: report.target_user_id.and_then(|id| handle_map.get(&id).cloned()),
            target_kind: if report.target_post_id.is_some() {
                "post".into()
            } else {
                "thread".into()
            },
            reason: report.reason,
            details: report.details.unwrap_or_default(),
            thread_id: report.target_thread_id.map(|value| value.to_string()),
            post_id: report.target_post_id.map(|value| value.to_string()),
            created_at: local_time_text(report.created_at),
            resolved: report.resolved,
            priority_label: report.priority_label,
            resolution_note: report.resolution_note.unwrap_or_default(),
            queue_state: report.queue_state,
            assigned_to: report.assigned_user_id.and_then(|id| handle_map.get(&id).cloned()),
            audit_history,
        };
        if item.priority_label == "urgent" {
            urgent_reports.push(item);
        } else {
            normal_reports.push(item);
        }
    }

    let shell = build_shell_context(&state, Some(&session), 8, String::new()).await;
    HtmlTemplate(templates::ReportsTemplate {
        chrome: shell,
        current_status: filters.status.clone(),
        current_queue_state: filters.queue_state.clone(),
        current_assigned: filters.assigned.clone(),
        current_priority: filters.priority.clone(),
        current_action: filters.action.clone(),
        current_moderator: filters.moderator.clone(),
        current_since: filters.since.clone(),
        current_until: filters.until.clone(),
        return_to: filters.return_to(),
        csv_export_url: filters.export_url(),
        urgent_count: urgent_reports.len(),
        normal_count: normal_reports.len(),
        urgent_reports,
        normal_reports,
    })
    .into_response()
}

async fn download_reports_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReportsQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::ModerationEventRepo, reports::ReportRepo, users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => {
            return (StatusCode::FORBIDDEN, "founder permission required").into_response();
        }
        Err(error) => {
            tracing::warn!(error = %error, "download_reports_csv: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
        }
    };

    let filters = ReportDeskFilters::from_query(pool, query).await;
    let reports = match ReportRepo
        .list_queue_filtered(pool, filters.report_queue_filter(session.user_id))
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "download_reports_csv: list_queue failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
        }
    };

    let mut actor_ids = HashSet::new();
    for report in &reports {
        actor_ids.insert(report.actor_id);
        if let Some(target_user_id) = report.target_user_id {
            actor_ids.insert(target_user_id);
        }
    }

    let mut audit_rows = Vec::new();
    for report in &reports {
        let events = match ModerationEventRepo.list_for_report(pool, report.id).await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "download_reports_csv: audit lookup failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
            }
        };
        let filtered_events = events
            .into_iter()
            .filter(|event| filters.action.is_empty() || event.action == filters.action)
            .filter(|event| filters.moderator_id.is_none_or(|actor_id| event.actor_id == actor_id))
            .filter(|event| {
                filters
                    .event_created_from
                    .is_none_or(|created_from| event.created_at >= created_from)
            })
            .filter(|event| {
                filters.event_created_to.is_none_or(|created_to| event.created_at < created_to)
            })
            .collect::<Vec<_>>();
        for event in &filtered_events {
            actor_ids.insert(event.actor_id);
        }
        audit_rows.push((report, filtered_events));
    }

    let actor_ids: Vec<_> = actor_ids.into_iter().collect();
    let handle_map: HashMap<uuid::Uuid, String> =
        match UserRepo.handle_batch_lookup(pool, &actor_ids).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(error) => {
                tracing::warn!(error = %error, "download_reports_csv: handle lookup failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "reports unavailable").into_response();
            }
        };

    let mut csv = String::from(
        "report_id,report_status,report_queue_state,report_priority,report_reason,reporter,target_kind,target_author,audit_id,audit_created_at,audit_actor,audit_action,audit_reason,audit_note\n",
    );
    for (report, events) in audit_rows {
        for event in events {
            let reporter = handle_map
                .get(&report.actor_id)
                .cloned()
                .unwrap_or_else(|| format!("user-{}", &report.actor_id.simple().to_string()[..8]));
            let target_author = report
                .target_user_id
                .and_then(|id| handle_map.get(&id).cloned())
                .unwrap_or_default();
            let audit_actor = handle_map
                .get(&event.actor_id)
                .cloned()
                .unwrap_or_else(|| format!("user-{}", &event.actor_id.simple().to_string()[..8]));
            let report_status = if report.resolved { "resolved" } else { "open" };
            let target_kind = if report.target_post_id.is_some() { "post" } else { "thread" };
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                report.id,
                report_status,
                csv_escape(&report.queue_state),
                csv_escape(&report.priority_label),
                csv_escape(&report.reason),
                csv_escape(&reporter),
                target_kind,
                csv_escape(&target_author),
                event.id,
                event.created_at.to_rfc3339(),
                csv_escape(&audit_actor),
                csv_escape(&event.action),
                csv_escape(&event.reason),
                csv_escape(event.note.as_deref().unwrap_or_default()),
            ));
        }
    }

    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], csv).into_response()
}

async fn view_metrics(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    use crate::repository::{
        notifications::NotificationRepo, posts::PostRepo, reports::ReportRepo, threads::ThreadRepo,
        users::UserRepo,
    };

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    // A6.1 strangle: read every count from Postgres rather than ForumStore.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    let viewer = match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => row,
        Ok(_) => {
            return (StatusCode::FORBIDDEN, "founder permission required").into_response();
        }
        Err(error) => {
            tracing::warn!(error = %error, "view_metrics: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "metrics unavailable").into_response();
        }
    };
    // Six independent count queries. Sequential for now — small absolute
    // cost (each is a single COUNT(*) on an indexed column). If load ever
    // makes this matter, fold into a single `WITH ... SELECT` aggregator.
    let metrics = match async {
        Ok::<_, sqlx::Error>(crate::forum::AdminMetrics {
            total_users: UserRepo.count(pool).await? as usize,
            total_threads: ThreadRepo.count(pool).await? as usize,
            total_posts: PostRepo.count(pool).await? as usize,
            open_reports: ReportRepo.count_open(pool).await? as usize,
            unresolved_notifications: NotificationRepo.count_unread_global(pool).await? as usize,
        })
    }
    .await
    {
        Ok(m) => m,
        Err(error) => {
            tracing::warn!(error = %error, "view_metrics: count fan-out failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "metrics unavailable").into_response();
        }
    };
    // `count_banned` is captured but not surfaced — `AdminMetrics` doesn't
    // expose it today. Kept here as a comment so the next person extending
    // the dashboard knows it's already in the repo: `UserRepo.count_banned(pool)`.
    let _ = viewer; // suppress unused — kept for symmetry with future role-aware fields
    let backup = crate::services::ops_status::backup_status(&state.config.ops);
    let ingress = crate::services::ops_status::ingress_status(&state.config.server);
    let shell = build_shell_context(&state, Some(&session), 8, String::new()).await;
    HtmlTemplate(templates::MetricsTemplate {
        chrome: shell,
        metrics,
        backup: templates::BackupStatusCard {
            status: backup.status.to_string(),
            backup_dir: backup.backup_dir.unwrap_or_default(),
            latest_backup_file: backup.latest_backup_file.unwrap_or_default(),
            latest_backup_at: backup.latest_backup_at.map(local_time_text),
            latest_backup_age_hours: backup.latest_backup_age_hours,
            latest_verified_file: backup.latest_verified_file.unwrap_or_default(),
            latest_verified_at: backup.latest_verified_at.map(local_time_text),
        },
        ingress: templates::IngressStatusCard {
            status: ingress.status.to_string(),
            secure_cookies: ingress.secure_cookies,
            rate_limit_disabled: ingress.rate_limit_disabled,
            trust_mode: ingress.trust_mode.to_string(),
            trusted_cidrs: ingress.trusted_cidrs.join(", "),
            client_ip_headers: ingress.client_ip_headers.join(", "),
        },
    })
    .into_response()
}

async fn view_moderation_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ModerationEventsQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventFilter, ModerationEventRepo},
        users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => return (StatusCode::FORBIDDEN, "founder permission required").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "view_moderation_events: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "moderation audit unavailable")
                .into_response();
        }
    };

    let filters = ModerationEventsFilters::from_query(pool, query).await;
    let rows = match ModerationEventRepo
        .list_filtered(
            pool,
            ModerationEventFilter {
                action: (!filters.action.is_empty()).then_some(filters.action.as_str()),
                actor_id: filters.moderator_id,
                report_id: filters.report_uuid,
                target_user_id: filters.target_user_id,
                created_from: filters.created_from,
                created_to: filters.created_to,
                limit: filters.limit as i64,
            },
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "view_moderation_events: list_filtered failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "moderation audit unavailable")
                .into_response();
        }
    };

    let user_ids = rows
        .iter()
        .flat_map(|row| [Some(row.actor_id), row.target_user_id])
        .collect::<HashSet<_>>()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let handle_map: HashMap<Uuid, String> =
        match UserRepo.handle_batch_lookup(pool, &user_ids).await {
            Ok(rows) => rows.into_iter().collect(),
            Err(error) => {
                tracing::warn!(error = %error, "view_moderation_events: handle lookup failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "moderation audit unavailable")
                    .into_response();
            }
        };

    let items = rows
        .into_iter()
        .map(|row| templates::ModerationEventListItem {
            id: row.id.to_string(),
            actor: handle_map
                .get(&row.actor_id)
                .cloned()
                .unwrap_or_else(|| format!("user-{}", &row.actor_id.simple().to_string()[..8])),
            action: row.action,
            reason: row.reason,
            note: row.note.unwrap_or_default(),
            report_id: row.report_id.map(|value| value.to_string()).unwrap_or_default(),
            target_user: row
                .target_user_id
                .and_then(|id| handle_map.get(&id).cloned())
                .unwrap_or_default(),
            target_thread_id: row
                .target_thread_id
                .map(|value| value.to_string())
                .unwrap_or_default(),
            target_post_id: row.target_post_id.map(|value| value.to_string()).unwrap_or_default(),
            created_at: local_time_text(row.created_at),
        })
        .collect::<Vec<_>>();

    let shell = build_shell_context(&state, Some(&session), 8, String::new()).await;
    let csv_export_url = filters.csv_url();
    HtmlTemplate(templates::ModerationEventsTemplate {
        chrome: shell,
        current_action: filters.action,
        current_moderator: filters.moderator,
        current_target: filters.target,
        current_report_id: filters.report_id,
        current_since: filters.since,
        current_until: filters.until,
        current_limit: filters.limit,
        csv_export_url,
        event_count: items.len(),
        items,
    })
    .into_response()
}

async fn download_moderation_events_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ModerationEventsQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventFilter, ModerationEventRepo},
        users::UserRepo,
    };
    use std::collections::HashMap;

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => return (StatusCode::FORBIDDEN, "founder permission required").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "download_moderation_events_csv: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "moderation audit unavailable")
                .into_response();
        }
    };

    let filters = ModerationEventsFilters::from_query(pool, query).await;
    let rows = match ModerationEventRepo
        .list_filtered(
            pool,
            ModerationEventFilter {
                action: (!filters.action.is_empty()).then_some(filters.action.as_str()),
                actor_id: filters.moderator_id,
                report_id: filters.report_uuid,
                target_user_id: filters.target_user_id,
                created_from: filters.created_from,
                created_to: filters.created_to,
                limit: filters.limit as i64,
            },
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "download_moderation_events_csv: list_filtered failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "moderation audit unavailable")
                .into_response();
        }
    };

    let user_ids = rows
        .iter()
        .flat_map(|row| [Some(row.actor_id), row.target_user_id])
        .flatten()
        .collect::<Vec<_>>();
    let handle_map: HashMap<Uuid, String> = match UserRepo
        .handle_batch_lookup(pool, &user_ids)
        .await
    {
        Ok(rows) => rows.into_iter().collect(),
        Err(error) => {
            tracing::warn!(error = %error, "download_moderation_events_csv: handle lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "moderation audit unavailable")
                .into_response();
        }
    };

    let mut csv = String::from(
        "id,created_at,actor,action,reason,note,report_id,target_user,target_thread_id,target_post_id\n",
    );
    for row in rows {
        let actor = handle_map
            .get(&row.actor_id)
            .cloned()
            .unwrap_or_else(|| format!("user-{}", &row.actor_id.simple().to_string()[..8]));
        let target_user =
            row.target_user_id.and_then(|id| handle_map.get(&id).cloned()).unwrap_or_default();
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            row.id,
            row.created_at.to_rfc3339(),
            csv_escape(&actor),
            csv_escape(&row.action),
            csv_escape(&row.reason),
            csv_escape(row.note.as_deref().unwrap_or_default()),
            row.report_id.map(|value| value.to_string()).unwrap_or_default(),
            csv_escape(&target_user),
            row.target_thread_id.map(|value| value.to_string()).unwrap_or_default(),
            row.target_post_id.map(|value| value.to_string()).unwrap_or_default(),
        ));
    }

    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], csv).into_response()
}

async fn triage_report_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ReportTriageForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        reports::ReportRepo,
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => return (StatusCode::FORBIDDEN, "founder permission required").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "triage_report_web: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "report unavailable").into_response();
        }
    }
    let Some(report) = ReportRepo.find_by_id(pool, id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "report not found").into_response();
    };
    let queue_state = payload
        .queue_state
        .as_deref()
        .map(str::trim)
        .filter(|value| matches!(*value, "new" | "reviewing" | "waiting"))
        .map(str::to_string);
    let assigned_to = if payload.release.as_deref() == Some("true") {
        Some(None)
    } else if payload.claim.as_deref() == Some("true") {
        Some(Some(session.user_id))
    } else {
        None
    };
    if let Err(error) = ReportRepo.triage(pool, id, queue_state.as_deref(), assigned_to).await {
        tracing::warn!(error = %error, %id, "triage_report_web: triage failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "report update failed").into_response();
    }
    let action = if payload.release.as_deref() == Some("true") {
        "release_report"
    } else if payload.claim.as_deref() == Some("true") {
        "claim_report"
    } else {
        "triage_report"
    };
    let note = payload
        .note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| queue_state.clone());
    let _ = ModerationEventRepo
        .insert(
            pool,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id: session.user_id,
                target_post_id: report.target_post_id,
                target_thread_id: report.target_thread_id,
                target_user_id: None,
                report_id: Some(id),
                action: action.into(),
                reason: report.reason,
                note,
                created_at: Utc::now(),
            },
        )
        .await;
    let return_to =
        services::redirects::safe_return_to(payload.return_to, "/admin/reports".to_string());
    Redirect::to(&return_to).into_response()
}

/// I3: machine-readable metrics for ops + alerting. Founder-gated so
/// the surface doesn't leak counts to the world. Plain-text format,
/// each line `viceden_<metric>{labels} <value>`. Compatible with
/// Prometheus scrape via a node_exporter-style textfile collector.
///
/// Counts come from PG only — no in-process counters yet (rate-limit
/// drops / login throttle hits / classifier latency need state machinery
/// not wired today).
async fn view_internal_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    use crate::repository::{
        notifications::NotificationRepo,
        posts::{ModState, PostRepo},
        reports::ReportRepo,
        threads::ThreadRepo,
        users::UserRepo,
    };

    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => return (StatusCode::FORBIDDEN, "founder permission required").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "view_internal_metrics: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "metrics unavailable").into_response();
        }
    }

    // Gather in parallel-ish — sequential here because each is one
    // small COUNT(*). Total budget is ~10ms locally.
    let users_total = UserRepo.count(pool).await.unwrap_or(0);
    let users_banned_total = UserRepo.count_banned(pool).await.unwrap_or(0);
    let threads_total = ThreadRepo.count(pool).await.unwrap_or(0);
    let posts_total = PostRepo.count(pool).await.unwrap_or(0);
    let posts_pending_total = PostRepo.count_with_state(pool, ModState::Pending).await.unwrap_or(0);
    let posts_quarantined_total =
        PostRepo.count_with_state(pool, ModState::Quarantined).await.unwrap_or(0);
    let posts_removed_total = PostRepo.count_with_state(pool, ModState::Removed).await.unwrap_or(0);
    let reports_open_total = ReportRepo.count_open(pool).await.unwrap_or(0);
    let notifications_unread_total = NotificationRepo.count_unread_global(pool).await.unwrap_or(0);

    let body = format!(
        "# HELP viceden_users_total Total user rows in the database.\n\
         # TYPE viceden_users_total gauge\n\
         viceden_users_total {users_total}\n\
         # HELP viceden_users_banned_total Currently banned users.\n\
         # TYPE viceden_users_banned_total gauge\n\
         viceden_users_banned_total {users_banned_total}\n\
         # HELP viceden_threads_total Total thread rows (any mod_state).\n\
         # TYPE viceden_threads_total gauge\n\
         viceden_threads_total {threads_total}\n\
         # HELP viceden_posts_total Total post rows (any mod_state).\n\
         # TYPE viceden_posts_total gauge\n\
         viceden_posts_total {posts_total}\n\
         # HELP viceden_posts_by_state Posts by moderation state.\n\
         # TYPE viceden_posts_by_state gauge\n\
         viceden_posts_by_state{{state=\"pending\"}} {posts_pending_total}\n\
         viceden_posts_by_state{{state=\"quarantined\"}} {posts_quarantined_total}\n\
         viceden_posts_by_state{{state=\"removed\"}} {posts_removed_total}\n\
         # HELP viceden_reports_open_total Unresolved reports awaiting moderation.\n\
         # TYPE viceden_reports_open_total gauge\n\
         viceden_reports_open_total {reports_open_total}\n\
         # HELP viceden_notifications_unread_total Unread notifications across all users.\n\
         # TYPE viceden_notifications_unread_total gauge\n\
         viceden_notifications_unread_total {notifications_unread_total}\n",
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; version=0.0.4"))],
        body,
    )
        .into_response()
}

async fn resolve_report_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ResolveReportForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        posts::PostRepo,
        reports::ReportRepo,
        threads::ThreadRepo,
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };

    // A7 partial: write path → ReportRepo. Founder gate via UserRepo.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => return (StatusCode::FORBIDDEN, "founder permission required").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "resolve_report_web: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "report unavailable").into_response();
        }
    }
    let Some(report) = ReportRepo.find_by_id(pool, id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "report not found").into_response();
    };
    let note = payload
        .note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Err(error) = ReportRepo
        .resolve_with_note(
            pool,
            id,
            session.user_id,
            payload.resolved.unwrap_or(true),
            note.as_deref(),
        )
        .await
    {
        tracing::warn!(error = %error, %id, "resolve_report_web: ReportRepo.resolve failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "report update failed").into_response();
    }
    let target_user_id = if let Some(post_id) = report.target_post_id {
        PostRepo.find_by_id(pool, post_id).await.unwrap_or(None).map(|post| post.user_id)
    } else if let Some(thread_id) = report.target_thread_id {
        ThreadRepo.find_by_id(pool, thread_id).await.unwrap_or(None).map(|thread| thread.user_id)
    } else {
        None
    };
    if let Err(error) = ModerationEventRepo
        .insert(
            pool,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id: session.user_id,
                target_post_id: report.target_post_id,
                target_thread_id: report.target_thread_id,
                target_user_id,
                report_id: Some(id),
                action: if payload.resolved.unwrap_or(true) {
                    "resolve_report".into()
                } else {
                    "reopen_report".into()
                },
                reason: report.reason,
                note,
                created_at: Utc::now(),
            },
        )
        .await
    {
        tracing::warn!(error = %error, %id, "resolve_report_web: audit insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "report update failed").into_response();
    }
    let return_to =
        services::redirects::safe_return_to(payload.return_to, "/admin/reports".to_string());
    Redirect::to(&return_to).into_response()
}

async fn remove_report_target_web(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Form(payload): Form<ResolveReportForm>,
) -> impl IntoResponse {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        notifications::{NewNotification, NotificationRepo},
        posts::{ModState, PostRepo},
        reports::ReportRepo,
        threads::ThreadRepo,
        users::UserRepo,
    };

    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };

    // A7 partial: write path → ReportRepo + PostRepo/ThreadRepo
    // mod_state mutation. Founder gate via UserRepo.
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    match UserRepo.find_by_id(pool, session.user_id).await {
        Ok(Some(row)) if row.role == "founder" => {}
        Ok(_) => return StatusCode::FORBIDDEN.into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "remove_report_target_web: founder lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "report unavailable").into_response();
        }
    }

    let Some(report) = ReportRepo.find_by_id(pool, id).await.unwrap_or(None) else {
        return (StatusCode::NOT_FOUND, "report not found").into_response();
    };
    let reason = format!("report {}", report.reason);
    let note = payload
        .note
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let mut moderation_result: Result<(), &'static str> = Ok(());
    let mut notification = None;
    let target_user_id = if let Some(post_id) = report.target_post_id {
        let post = PostRepo.find_by_id(pool, post_id).await.unwrap_or(None);
        let target_user_id = post.as_ref().map(|post| post.user_id);
        if let Some(post) = post {
            let thread_title = ThreadRepo
                .find_by_id(pool, post.thread_id)
                .await
                .unwrap_or(None)
                .map(|thread| thread.title)
                .unwrap_or_else(|| "Thread".to_string());
            notification = Some(NewNotification {
                id: Uuid::new_v4(),
                recipient_user_id: post.user_id,
                actor_id: Some(session.user_id),
                kind: "post_removed".to_string(),
                thread_id: Some(post.thread_id),
                thread_title,
                post_id: Some(post_id),
                message: moderation_notification_message("post", &report.reason),
                created_at: Utc::now(),
                is_read: false,
            });
        }
        if let Err(error) =
            PostRepo.update_mod_state(pool, post_id, ModState::Removed, Some(&reason)).await
        {
            tracing::warn!(error = %error, %post_id, "remove_report_target_web: post update failed");
            moderation_result = Err("post update failed");
        }
        target_user_id
    } else if let Some(thread_id) = report.target_thread_id {
        let thread = ThreadRepo.find_by_id(pool, thread_id).await.unwrap_or(None);
        let target_user_id = thread.as_ref().map(|thread| thread.user_id);
        if let Some(thread) = thread {
            notification = Some(NewNotification {
                id: Uuid::new_v4(),
                recipient_user_id: thread.user_id,
                actor_id: Some(session.user_id),
                kind: "post_removed".to_string(),
                thread_id: Some(thread_id),
                thread_title: thread.title,
                post_id: None,
                message: moderation_notification_message("thread", &report.reason),
                created_at: Utc::now(),
                is_read: false,
            });
        }
        if let Err(error) =
            ThreadRepo.update_mod_state(pool, thread_id, ModState::Removed, Some(&reason)).await
        {
            tracing::warn!(error = %error, %thread_id, "remove_report_target_web: thread update failed");
            moderation_result = Err("thread update failed");
        }
        target_user_id
    } else {
        return (StatusCode::NOT_FOUND, "report target not found").into_response();
    };

    if let Err(msg) = moderation_result {
        return (StatusCode::INTERNAL_SERVER_ERROR, msg).into_response();
    }
    if let Err(error) =
        ReportRepo.resolve_with_note(pool, id, session.user_id, true, note.as_deref()).await
    {
        tracing::warn!(error = %error, "remove_report_target_web: resolve failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "resolve failed").into_response();
    }
    if let Err(error) = ModerationEventRepo
        .insert(
            pool,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id: session.user_id,
                target_post_id: report.target_post_id,
                target_thread_id: report.target_thread_id,
                target_user_id,
                report_id: Some(id),
                action: if report.target_post_id.is_some() {
                    "remove_post".into()
                } else {
                    "remove_thread".into()
                },
                reason: report.reason,
                note,
                created_at: Utc::now(),
            },
        )
        .await
    {
        tracing::warn!(error = %error, "remove_report_target_web: audit insert failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "resolve failed").into_response();
    }
    if let Some(notification) = notification {
        if let Err(error) = NotificationRepo.insert(pool, notification).await {
            tracing::warn!(error = %error, "remove_report_target_web: notification insert failed");
        }
    }
    let return_to =
        services::redirects::safe_return_to(payload.return_to, "/admin/reports".to_string());
    Redirect::to(&return_to).into_response()
}

async fn login_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LoginQuery>,
) -> impl IntoResponse {
    if session_from_headers(&headers, &state).await.is_some() {
        return Redirect::to("/profile").into_response();
    }

    render_login_form(
        StatusCode::OK,
        &state,
        services::redirects::safe_return_to(query.return_to, "/profile".to_string()),
        String::new(),
    )
}

async fn login_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<LoginForm>,
) -> impl IntoResponse {
    use crate::repository::users::UserRepo;

    let handle = payload.handle.trim();
    let password = payload.password.trim();
    if !pre_auth_csrf_valid(&headers, payload.csrf.as_deref()) {
        return (StatusCode::FORBIDDEN, "csrf check failed").into_response();
    }
    let throttle_key = login_throttle_key(handle, &headers);

    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database required").into_response();
    };
    if !state.login_throttle.allowed(&throttle_key).await {
        return render_login_form(
            StatusCode::TOO_MANY_REQUESTS,
            &state,
            services::redirects::safe_return_to(payload.return_to, "/profile".to_string()),
            "Too many failed logins. Try again later.".to_string(),
        );
    }

    let user = match UserRepo.find_by_handle(pool, handle).await {
        Ok(Some(user))
            if !user.banned
                && crate::auth::verify_password(&user.handle, password, &user.password_hash) =>
        {
            user
        }
        _ => {
            state.login_throttle.record_failure(&throttle_key).await;
            return render_login_form(
                StatusCode::UNAUTHORIZED,
                &state,
                services::redirects::safe_return_to(payload.return_to, "/profile".to_string()),
                "Invalid credentials.".to_string(),
            );
        }
    };

    state.login_throttle.clear(&throttle_key).await;
    let pepper = state.config.auth.token_pepper.as_deref();
    let session = match crate::auth::sessions::issue_session(pool, user.id, pepper).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(?error, "session create failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "session could not be created")
                .into_response();
        }
    };
    let return_to = services::redirects::safe_return_to(payload.return_to, "/profile".to_string());
    let cookie = session_cookie_header(&state, &session.raw_token);

    ([(header::SET_COOKIE, cookie)], Redirect::to(&return_to)).into_response()
}

async fn logout_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<ReturnToForm>,
) -> impl IntoResponse {
    let session = match require_session_and_csrf(&headers, &state, payload.csrf.as_deref()).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if let Some(pool) = state.db.as_ref() {
        let pepper = state.config.auth.token_pepper.as_deref();
        if let Err(error) =
            crate::auth::sessions::destroy_session(pool, &session.session_token, pepper).await
        {
            tracing::warn!(?error, "session destroy failed");
        }
    }
    let return_to = services::redirects::safe_return_to(payload.return_to, "/".to_string());
    let mut response = Redirect::to(&return_to).into_response();
    for cookie in cleared_session_cookie_headers(&state)
        .into_iter()
        .chain(std::iter::once(cleared_pre_auth_csrf_cookie_header(&state)))
    {
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).expect("valid cookie header"),
        );
    }
    response
}

async fn register_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RegisterQuery>,
) -> impl IntoResponse {
    if session_from_headers(&headers, &state).await.is_some() {
        return Redirect::to("/").into_response();
    }
    let (csrf_token, csrf_cookie) = issue_pre_auth_csrf_cookie(&state);
    let return_to = services::redirects::safe_return_to(query.return_to, "/".to_string());
    let code = query.code.unwrap_or_default();
    let (invite_title, invite_copy) = register_invite_copy(&state, &code).await;
    (
        [(header::SET_COOKIE, csrf_cookie)],
        HtmlTemplate(templates::RegisterTemplate {
            auth: templates::AuthChrome { theme: "dark".to_string() },
            csrf_token,
            return_to,
            code,
            error: String::new(),
            invite_title,
            invite_copy,
        }),
    )
        .into_response()
}

async fn robots_txt() -> impl IntoResponse {
    const CRAWLERS: &[&str] = &[
        "GPTBot",
        "OAI-SearchBot",
        "ChatGPT-User",
        "ClaudeBot",
        "Claude-User",
        "Claude-SearchBot",
        "PerplexityBot",
        "Googlebot",
        "Google-Extended",
        "bingbot",
        "Applebot",
        "Applebot-Extended",
        "CCBot",
        "Twitterbot",
    ];
    const PRIVATE_PATHS: &[&str] = &[
        "/admin",
        "/api",
        "/internal",
        "/live",
        "/login",
        "/notifications",
        "/profile",
        "/register",
        "/saved",
        "/search",
    ];

    let mut body = String::new();
    for crawler in CRAWLERS.iter().copied().chain(std::iter::once("*")) {
        body.push_str("User-agent: ");
        body.push_str(crawler);
        body.push('\n');
        body.push_str("Allow: /\n");
        for path in PRIVATE_PATHS {
            body.push_str("Disallow: ");
            body.push_str(path);
            body.push('\n');
        }
        body.push('\n');
    }
    body.push_str("Sitemap: /sitemap.xml\n");

    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

async fn llms_txt() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "# ViceDen\n\nViceDen is an invite-only text forum for community discussion.\n\nAI systems may use public policy and orientation pages from this site as input for search, retrieval, summaries, and answers. Login-only forum content, account pages, admin pages, internal metrics, and API endpoints are not public indexing surfaces.\n\nPublic pages:\n\n- https://viceden.example.com/guide\n- https://viceden.example.com/rules\n- https://viceden.example.com/privacy\n\nSitemap:\n\n- https://viceden.example.com/sitemap.xml\n",
    )
}

async fn llms_full_txt() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "# ViceDen full LLM index\n\nViceDen is a closed community forum. The public indexing surface is intentionally limited to orientation and policy pages.\n\nCrawler policy:\n\n- Search and AI retrieval crawlers are welcome on public orientation and policy pages.\n- Login-only forum content, profiles, saved threads, notifications, admin tools, internal metrics, and API endpoints are not public indexing surfaces.\n- Public pages may be used for search, retrieval, summaries, and answers.\n\nCanonical public entry points:\n\n- https://viceden.example.com/guide\n- https://viceden.example.com/rules\n- https://viceden.example.com/privacy\n\nMachine-readable discovery:\n\n- https://viceden.example.com/robots.txt\n- https://viceden.example.com/sitemap.xml\n- https://viceden.example.com/llms.txt\n",
    )
}

async fn sitemap_xml(State(state): State<AppState>) -> impl IntoResponse {
    let urls = vec!["/guide".to_string(), "/rules".to_string(), "/privacy".to_string()];
    let _ = state;

    let mut body = String::from(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    body.push_str(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#);
    for path in urls {
        body.push_str("<url><loc>");
        body.push_str(&escape_html(&path));
        body.push_str("</loc></url>");
    }
    body.push_str("</urlset>");

    ([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], body)
}

async fn register_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<RegisterForm>,
) -> impl IntoResponse {
    use chrono::NaiveDate;

    if !pre_auth_csrf_valid(&headers, payload.csrf.as_deref()) {
        return (StatusCode::FORBIDDEN, "csrf check failed").into_response();
    }

    let code = payload.code.as_deref().unwrap_or("").trim();
    if code.is_empty() {
        return render_register_error(&state, &payload, "invite code is required").await;
    }
    if payload.confirm_18.as_deref() != Some("yes") {
        return render_register_error(&state, &payload, "you must confirm you are 18+").await;
    }
    if payload.confirm_rules.as_deref() != Some("yes") {
        return render_register_error(&state, &payload, "you must accept the rules").await;
    }

    let dob =
        match payload.dob.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
            Some(d) => d,
            None => {
                return render_register_error(&state, &payload, "date of birth is required").await
            }
        };

    let real_ip = headers
        .get("x-real-ip-resolved")
        .or_else(|| headers.get("cf-connecting-ip"))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0.0.0.0")
        .to_string();
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "registration unavailable").into_response();
    };
    let handle = payload.handle.trim();
    let code_throttle_key = register_code_throttle_key(code);
    let handle_throttle_key = register_handle_throttle_key(handle);
    if !state.login_throttle.allowed(&code_throttle_key).await
        || !state.login_throttle.allowed(&handle_throttle_key).await
    {
        return render_register_form(
            StatusCode::TOO_MANY_REQUESTS,
            &state,
            &payload,
            "Too many registration attempts. Try again later.",
        )
        .await;
    }

    let password_hash = crate::auth::hash_password(payload.password.trim());
    let outcome = crate::auth::invites::redeem_invite(
        pool,
        crate::auth::invites::RedemptionContext {
            code,
            handle,
            password_hash: &password_hash,
            dob,
            today: chrono::Utc::now().date_naive(),
            attested_ip: &real_ip,
            attested_ua: &ua,
        },
    )
    .await;

    let user_id = match outcome {
        Ok(o) => o.user_id,
        Err(e) => {
            state.login_throttle.record_failure(&code_throttle_key).await;
            state.login_throttle.record_failure(&handle_throttle_key).await;
            return render_register_error(&state, &payload, &friendly_error(&e)).await;
        }
    };
    state.login_throttle.clear(&code_throttle_key).await;
    state.login_throttle.clear(&handle_throttle_key).await;

    // Issue session.
    let pepper = state.config.auth.token_pepper.clone();
    let session = match crate::auth::sessions::issue_session(pool, user_id, pepper.as_deref()).await
    {
        Ok(s) => s,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "session create failed").into_response()
        }
    };

    let default_return_to = if payload.return_to.as_deref().unwrap_or("").trim().is_empty() {
        "/guide?welcome=1".to_string()
    } else {
        "/".to_string()
    };
    let return_to = services::redirects::safe_return_to(payload.return_to, default_return_to);
    let cookie = session_cookie_header(&state, &session.raw_token);
    ([(header::SET_COOKIE, cookie)], Redirect::to(&return_to)).into_response()
}

async fn render_register_error(
    state: &AppState,
    payload: &RegisterForm,
    message: &str,
) -> Response {
    render_register_form(StatusCode::OK, state, payload, message).await
}

async fn render_register_form(
    status: StatusCode,
    state: &AppState,
    payload: &RegisterForm,
    message: &str,
) -> Response {
    let (invite_title, invite_copy) =
        register_invite_copy(state, payload.code.as_deref().unwrap_or_default()).await;
    let (csrf_token, csrf_cookie) = issue_pre_auth_csrf_cookie(state);
    (
        status,
        [(header::SET_COOKIE, csrf_cookie)],
        HtmlTemplate(templates::RegisterTemplate {
            auth: templates::AuthChrome { theme: "dark".to_string() },
            csrf_token,
            return_to: services::redirects::safe_return_to(
                payload.return_to.clone(),
                "/".to_string(),
            ),
            code: payload.code.clone().unwrap_or_default(),
            error: message.to_string(),
            invite_title,
            invite_copy,
        }),
    )
        .into_response()
}

async fn register_invite_copy(state: &AppState, code: &str) -> (String, String) {
    use crate::repository::{invites::InviteRepo, users::UserRepo};

    let code = code.trim();
    if code.is_empty() {
        return ("Invite code".into(), "Enter a code from the person who invited you.".into());
    }
    let Some(pool) = state.db.as_ref() else {
        return ("Invite code".into(), "Invite lookup unavailable right now.".into());
    };
    let Some(invite) = InviteRepo.find(pool, code).await.unwrap_or(None) else {
        return ("Invite code".into(), "This code does not exist or is no longer valid.".into());
    };
    let inviter = UserRepo
        .find_by_id(pool, invite.inviter_user_id)
        .await
        .ok()
        .flatten()
        .map(|user| user.handle)
        .unwrap_or_else(|| "unknown".into());
    let remaining = (invite.max_redemptions - invite.redemption_count).max(0);
    let status = if invite.revoked_at.is_some() {
        "This invite was revoked.".into()
    } else if invite.expires_at.is_some_and(|value| value <= Utc::now()) {
        "This invite expired.".into()
    } else if remaining == 0 {
        "This invite is already spent.".into()
    } else {
        format!("{remaining} uses left.")
    };
    let note = invite.note.filter(|value| !value.trim().is_empty());
    let copy = match note {
        Some(note) => format!("Invited by @{inviter}. {status} Note: {note}"),
        None => format!("Invited by @{inviter}. {status}"),
    };
    ("Invited by".into(), copy)
}

fn friendly_error(err: &crate::auth::invites::RedemptionError) -> String {
    use crate::auth::invites::RedemptionError as E;
    match err {
        E::InviteNotFound | E::InviteRevoked | E::InviteExpired | E::InviteSpent => {
            "invite not valid".into()
        }
        E::HandleTaken => "handle is taken".into(),
        E::HandleBlocked => "handle not allowed".into(),
        E::HandleInvalid => "handle must be 3-24 chars, lowercase letters/digits/_/-".into(),
        E::Underage => "minimum age is 18".into(),
        E::FutureDob => "date of birth must be in the past".into(),
        E::Db(_) => "registration failed, try again later".into(),
    }
}

pub async fn not_found_page(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let auth = session_from_headers(&headers, &state).await;
    if auth.is_none() {
        return Redirect::to("/login").into_response();
    }
    error_page_response(
        &state,
        auth.as_ref(),
        ErrorPageSpec {
            status: StatusCode::NOT_FOUND,
            title: "Not found",
            intro: "That page does not exist here.",
            details: String::new(),
            next_step: "Use one of the live forum routes or go back to a known entry point.",
            action_href: if auth.is_some() { "/" } else { "/login" },
            action_label: if auth.is_some() { "Back to forum" } else { "Back to login" },
        },
    )
    .await
}

fn render_login_form(
    status: StatusCode,
    state: &AppState,
    return_to: String,
    error: String,
) -> Response {
    let (csrf_token, csrf_cookie) = issue_pre_auth_csrf_cookie(state);
    (
        status,
        [(header::SET_COOKIE, csrf_cookie)],
        HtmlTemplate(templates::LoginTemplate {
            auth: templates::AuthChrome { theme: "dark".to_string() },
            csrf_token,
            return_to,
            error,
        }),
    )
        .into_response()
}

pub(crate) async fn build_shell_context(
    state: &AppState,
    auth: Option<&WebSession>,
    recent_limit: usize,
    active_board_slug: String,
) -> ShellContext {
    let Some(auth) = auth else {
        return ShellContext {
            boards: state.boards.clone(),
            current_user: guest_user(),
            recent_posts: Vec::new(),
            active_board_slug,
            notifications: Vec::new(),
            notification_count: 0,
            compactness: 0,
            compactness_label: compactness_label(0),
            next_compactness: 1,
            theme: "dark".to_string(),
            saved_threads: Vec::new(),
            subscribed_boards: Vec::new(),
            csrf_token: String::new(),
        };
    };

    let user_id = auth.user_id;
    let Some(pool) = state.db.as_ref() else {
        return ShellContext {
            boards: state.boards.clone(),
            current_user: guest_user(),
            recent_posts: Vec::new(),
            active_board_slug,
            notifications: Vec::new(),
            notification_count: 0,
            compactness: 0,
            compactness_label: compactness_label(0),
            next_compactness: 1,
            theme: "dark".to_string(),
            saved_threads: Vec::new(),
            subscribed_boards: Vec::new(),
            csrf_token: auth.csrf_token.clone(),
        };
    };

    use crate::repository::{
        notifications::NotificationRepo, preferences::PreferencesRepo, threads::ThreadRepo,
        users::UserRepo,
    };
    use std::collections::HashMap;

    let user = UserRepo.find_by_id(pool, user_id).await.ok().flatten();
    let settings = PreferencesRepo.settings_for(pool, user_id).await.unwrap_or_default();
    let current_user = templates::ForumUser {
        id: user_id.to_string(),
        handle: user
            .as_ref()
            .map(|row| row.handle.clone())
            .unwrap_or_else(|| user_fallback(user_id)),
        role: user.as_ref().map(|row| row.role.clone()).unwrap_or_else(|| "member".to_string()),
        theme: settings.theme.clone(),
        is_authenticated: true,
    };

    let recent_posts = UserRepo
        .latest_activity(pool, user_id, recent_limit as i64)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|item| templates::UserPostTeaser {
            board_name: board_name(&state.boards, &item.board_slug),
            board_slug: item.board_slug,
            thread_id: item.thread_id,
            post_id: item.post_id,
            url: format!("/t/{}#post-{}", item.thread_id, item.post_id),
            thread_title: item.thread_title,
            created_at: local_time_text(item.created_at),
        })
        .collect::<Vec<_>>();

    let notification_rows =
        NotificationRepo.list_for_user(pool, user_id, 10).await.unwrap_or_default();
    let actor_ids = notification_rows.iter().filter_map(|row| row.actor_id).collect::<Vec<_>>();
    let actor_handles: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &actor_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let notifications = notification_rows
        .into_iter()
        .map(|item| {
            let (thread_url, has_thread_link, target_label) =
                notification_target_link(item.thread_id, item.post_id);
            templates::NotificationItem {
                id: item.id.to_string(),
                kind: notification_kind_display(&item.kind).to_string(),
                actor: item
                    .actor_id
                    .and_then(|actor| actor_handles.get(&actor).cloned())
                    .unwrap_or_else(|| "system".to_string()),
                thread_title: item.thread_title,
                thread_id: item.thread_id.map(|value| value.to_string()),
                thread_url,
                target_label,
                has_thread_link,
                created_at: local_time_text(item.created_at),
                is_read: item.is_read,
                message: item.message,
            }
        })
        .collect::<Vec<_>>();
    let notification_count =
        NotificationRepo.count_unread(pool, user_id).await.unwrap_or(0) as usize;
    let compactness = settings.compactness;
    let theme = settings.theme;

    let mut saved_threads = Vec::new();
    for thread_id in PreferencesRepo
        .saved_threads_for(pool, user_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .take(16)
    {
        if let Ok(Some(thread)) = ThreadRepo.find_by_id(pool, thread_id).await {
            saved_threads.push(templates::SavedThreadPreview {
                thread_id: thread.id,
                title: thread.title,
                board_name: board_name(&state.boards, &thread.board_slug),
            });
        }
    }

    let subscribed_slugs =
        PreferencesRepo.subscribed_slugs_for(pool, user_id).await.unwrap_or_default();
    let subscribed_set =
        subscribed_slugs.iter().map(String::as_str).collect::<std::collections::HashSet<_>>();
    let mut subscribed_boards = Vec::new();
    for board in state.boards.iter().filter(|board| subscribed_set.contains(board.slug.as_str())) {
        let thread_count =
            crate::db::count_visible_board_threads(pool, user_id, &board.slug).await.unwrap_or(0);
        subscribed_boards.push(templates::SubscribedBoardPreview {
            slug: board.slug.clone(),
            name: board.name.clone(),
            thread_count,
        });
    }

    let next_compactness = (compactness + 1).rem_euclid(3);

    ShellContext {
        boards: state.boards.clone(),
        current_user,
        recent_posts,
        active_board_slug,
        notifications,
        notification_count,
        compactness,
        compactness_label: compactness_label(compactness),
        next_compactness,
        theme,
        saved_threads,
        subscribed_boards,
        csrf_token: auth.csrf_token.clone(),
    }
}

async fn build_public_context(state: &AppState, auth: Option<&WebSession>) -> PublicContext {
    let Some(auth) = auth else {
        return PublicContext { current_user: guest_user(), theme: "dark".to_string() };
    };

    let Some(pool) = state.db.as_ref() else {
        return PublicContext { current_user: guest_user(), theme: "dark".to_string() };
    };
    use crate::repository::{preferences::PreferencesRepo, users::UserRepo};
    let user = UserRepo.find_by_id(pool, auth.user_id).await.ok().flatten();
    let settings = PreferencesRepo.settings_for(pool, auth.user_id).await.unwrap_or_default();
    PublicContext {
        current_user: templates::ForumUser {
            id: auth.user_id.to_string(),
            handle: user
                .as_ref()
                .map(|row| row.handle.clone())
                .unwrap_or_else(|| user_fallback(auth.user_id)),
            role: user.as_ref().map(|row| row.role.clone()).unwrap_or_else(|| "member".to_string()),
            theme: settings.theme.clone(),
            is_authenticated: true,
        },
        theme: settings.theme,
    }
}

async fn thread_item_from_thread_preview_db(
    pool: &sqlx::PgPool,
    state: &AppState,
    user_id: Uuid,
    item: crate::repository::threads::ThreadWithPreview,
) -> templates::ThreadListItem {
    let thread = crate::models::Thread {
        id: item.thread.id,
        board_slug: item.thread.board_slug,
        title: item.thread.title,
        user_id: item.thread.user_id,
        created_at: item.thread.created_at,
        expires_at: item.thread.expires_at,
    };
    thread_item_from_preview_db(
        pool,
        state,
        user_id,
        forum::ThreadPreview {
            score_hidden: forum::is_score_hidden(thread.created_at),
            thread,
            score: item.score as i32,
            reply_count: item.reply_count as usize,
            last_activity: item.last_activity,
        },
    )
    .await
}

async fn thread_item_from_preview_db(
    pool: &sqlx::PgPool,
    state: &AppState,
    user_id: Uuid,
    item: forum::ThreadPreview,
) -> templates::ThreadListItem {
    use crate::repository::{
        posts::PostRepo, preferences::PreferencesRepo, tags::TagRepo,
        thread_states::ThreadStateRepo, users::UserRepo,
    };

    let thread = item.thread;
    let thread_id = thread.id;
    let board_slug = thread.board_slug.clone();
    let board_name_text = board_name(&state.boards, &board_slug);
    let author = UserRepo
        .handle_batch_lookup(pool, &[thread.user_id])
        .await
        .unwrap_or_default()
        .into_iter()
        .next()
        .map(|(_, handle)| handle)
        .unwrap_or_else(|| user_fallback(thread.user_id));
    let snippet = PostRepo
        .first_post_for_thread(pool, thread_id)
        .await
        .ok()
        .flatten()
        .map(|post| post.body.chars().take(220).collect::<String>())
        .unwrap_or_default();
    let tags = TagRepo.list_for_thread(pool, thread_id).await.unwrap_or_default();
    let participant_count = PostRepo.participant_count(pool, thread_id).await.unwrap_or(0) as usize;
    let is_saved = PreferencesRepo.is_thread_saved(pool, user_id, thread_id).await.unwrap_or(false);
    let state_row =
        ThreadStateRepo.state_for(pool, thread_id).await.unwrap_or_default().unwrap_or_default();
    templates::ThreadListItem {
        thread_id,
        title: thread.title,
        board_slug,
        board_name: board_name_text,
        author,
        created_at: local_time_text(thread.created_at),
        last_activity: local_time_text(item.last_activity),
        snippet,
        tags,
        reply_count: item.reply_count,
        participant_count,
        score: item.score,
        score_hidden: item.score_hidden,
        authored_by_current_user: thread.user_id == user_id,
        is_saved,
        thread_state: forum::ThreadState {
            pinned: state_row.pinned,
            locked: state_row.locked,
            answered: state_row.answered,
            closed: state_row.closed,
        },
    }
}

fn split_tag_form(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn fallback_error_response(
    status: StatusCode,
    title: &str,
    intro: &str,
    details: Option<String>,
    action_href: &str,
    action_label: &str,
) -> Response {
    let details_block = details
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("<p>{}</p>", escape_html(&value)))
        .unwrap_or_default();
    let html = format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\" /><meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" /><title>{title}</title></head><body class=\"shell-public\"><main><h1>{title}</h1><p>{intro}</p>{details_block}<p><a href=\"{action_href}\">{action_label}</a></p></main></body></html>",
        title = escape_html(title),
        intro = escape_html(intro),
        details_block = details_block,
        action_href = escape_html(action_href),
        action_label = escape_html(action_label),
    );
    (status, Html(html)).into_response()
}

fn escape_html(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

async fn error_page_response(
    state: &AppState,
    auth: Option<&WebSession>,
    spec: ErrorPageSpec<'_>,
) -> Response {
    let public = build_public_context(state, auth).await;
    (
        spec.status,
        HtmlTemplate(templates::ErrorPageTemplate {
            public,
            title: spec.title.to_string(),
            kicker: spec.status.as_u16().to_string(),
            intro: spec.intro.to_string(),
            details: spec.details,
            next_step: spec.next_step.to_string(),
            action_href: spec.action_href.to_string(),
            action_label: spec.action_label.to_string(),
        }),
    )
        .into_response()
}

fn session_cookie_header(state: &AppState, token: &str) -> String {
    let mut cookie = format!(
        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
        session_cookie_name(state),
        token,
        60 * 60 * 24 * 7
    );
    if state.config.server.secure_cookies {
        cookie.push_str("; Secure");
    }
    cookie
}

fn cleared_session_cookie_header(state: &AppState, cookie_name: &str) -> String {
    let mut cookie = format!("{}=deleted; Path=/; Max-Age=0; HttpOnly; SameSite=Lax", cookie_name);
    if state.config.server.secure_cookies {
        cookie.push_str("; Secure");
    }
    cookie
}

fn cleared_session_cookie_headers(state: &AppState) -> Vec<String> {
    let current = session_cookie_name(state);
    let mut names = vec![current];
    if current != LEGACY_SESSION_COOKIE_NAME {
        names.push(LEGACY_SESSION_COOKIE_NAME);
    }
    names.into_iter().map(|name| cleared_session_cookie_header(state, name)).collect()
}

fn session_cookie_name(state: &AppState) -> &'static str {
    if state.config.server.secure_cookies {
        HOST_SESSION_COOKIE_NAME
    } else {
        LEGACY_SESSION_COOKIE_NAME
    }
}

const PRE_AUTH_CSRF_COOKIE_NAME: &str = "viceden_pre_auth_csrf";

fn cleared_pre_auth_csrf_cookie_header(state: &AppState) -> String {
    let mut cookie =
        format!("{}=deleted; Path=/; Max-Age=0; HttpOnly; SameSite=Lax", PRE_AUTH_CSRF_COOKIE_NAME);
    if state.config.server.secure_cookies {
        cookie.push_str("; Secure");
    }
    cookie
}

fn issue_pre_auth_csrf_cookie(state: &AppState) -> (String, String) {
    let csrf_token = Uuid::new_v4().to_string();
    let mut cookie =
        format!("{}={}; Path=/; HttpOnly; SameSite=Lax", PRE_AUTH_CSRF_COOKIE_NAME, csrf_token);
    if state.config.server.secure_cookies {
        cookie.push_str("; Secure");
    }
    (csrf_token, cookie)
}

fn login_throttle_key(handle: &str, headers: &HeaderMap) -> String {
    let normalized = handle.trim().trim_start_matches('@').to_ascii_lowercase();
    let ip = client_ip_from_headers(headers).unwrap_or_else(|| "no-ip".to_string());
    format!("{normalized}|{ip}")
}

fn register_code_throttle_key(code: &str) -> String {
    format!("register-code|{}", code.trim().to_ascii_lowercase())
}

fn register_handle_throttle_key(handle: &str) -> String {
    format!("register-handle|{}", handle.trim().trim_start_matches('@').to_ascii_lowercase())
}

fn board_name(boards: &[Board], slug: &str) -> String {
    boards
        .iter()
        .find(|board| board.slug == slug)
        .map(|board| board.name.clone())
        .unwrap_or_else(|| slug.to_string())
}

fn user_fallback(user_id: Uuid) -> String {
    format!("user-{}", &user_id.simple().to_string()[..8])
}

fn active_probation_until(probation_until: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    crate::services::limits::active_probation_until(probation_until, Utc::now())
}

fn admin_user_status(banned: bool, deactivated: bool, probation_active: bool) -> String {
    if deactivated {
        "deactivated".into()
    } else if banned {
        "banned".into()
    } else if probation_active {
        "probation".into()
    } else {
        "active".into()
    }
}

fn notification_kind_display(kind: &str) -> &str {
    match kind {
        "post_removed" => "post removed",
        "mod_action" => "moderation",
        other => other,
    }
}

fn notification_target_link(
    thread_id: Option<Uuid>,
    post_id: Option<Uuid>,
) -> (String, bool, String) {
    match (thread_id, post_id) {
        (Some(thread_id), Some(post_id)) => {
            (format!("/t/{thread_id}#post-{post_id}"), true, "Open post".to_string())
        }
        (Some(thread_id), None) => (format!("/t/{thread_id}"), true, "Open thread".to_string()),
        (None, _) => (String::new(), false, String::new()),
    }
}

fn moderation_notification_message(target: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        format!("Your {target} was removed.")
    } else {
        format!("Your {target} was removed. Reason: {reason}.")
    }
}

fn reject_spam_body_web(value: &str) -> Option<Response> {
    let link_count = value
        .split_whitespace()
        .filter(|part| part.starts_with("http://") || part.starts_with("https://"))
        .count();
    if link_count > 8 {
        return Some((StatusCode::TOO_MANY_REQUESTS, "too many links").into_response());
    }
    let repeated_run = value
        .chars()
        .fold((None, 0usize, 0usize), |(last, run, max_run), ch| {
            if Some(ch) == last {
                (last, run + 1, max_run.max(run + 1))
            } else {
                (Some(ch), 1, max_run.max(1))
            }
        })
        .2;
    if repeated_run > 80 {
        return Some(
            (StatusCode::TOO_MANY_REQUESTS, "repeated text limit reached").into_response(),
        );
    }
    None
}

fn compactness_label(value: i8) -> String {
    match value {
        -2 => "Ultra compact".to_string(),
        -1 => "Compact".to_string(),
        1 => "Comfortable".to_string(),
        2 => "Spacious".to_string(),
        _ => "Default".to_string(),
    }
}

fn encode_query_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push_str("%20"),
            _ => {
                use std::fmt::Write;

                let _ = write!(&mut encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

fn parse_query_date_floor(value: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()?;
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc))
}

fn parse_query_date_ceiling(value: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d").ok()?;
    let next_day = date.succ_opt()?;
    let midnight = next_day.and_hms_opt(0, 0, 0)?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc))
}

fn csv_escape(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn guest_user() -> templates::ForumUser {
    templates::ForumUser {
        id: "guest".to_string(),
        handle: "guest".to_string(),
        role: "guest".to_string(),
        theme: "dark".to_string(),
        is_authenticated: false,
    }
}

async fn session_from_headers(headers: &HeaderMap, state: &AppState) -> Option<WebSession> {
    use crate::repository::sessions::SessionRepo;

    let token = session_cookie_from_headers(headers, state)?;
    if let Some(pool) = state.db.as_ref() {
        let pepper = state.config.auth.token_pepper.as_deref();
        let row = SessionRepo.find_by_token(pool, &token, pepper).await.ok().flatten()?;
        return Some(WebSession {
            user_id: row.user_id,
            session_token: token,
            csrf_token: row.csrf_token,
        });
    }

    None
}

pub(crate) async fn require_session(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<WebSession, Response> {
    session_from_headers(headers, state).await.ok_or_else(|| Redirect::to("/login").into_response())
}

fn pre_auth_csrf_valid(headers: &HeaderMap, submitted_csrf: Option<&str>) -> bool {
    matches!(
        (pre_auth_csrf_from_headers(headers), submitted_csrf),
        (Some(expected), Some(submitted)) if !submitted.is_empty() && submitted == expected
    )
}

async fn require_session_and_csrf(
    headers: &HeaderMap,
    state: &AppState,
    submitted_csrf: Option<&str>,
) -> Result<WebSession, Response> {
    let session = require_session(headers, state).await?;
    if submitted_csrf != Some(session.csrf_token.as_str()) {
        return Err((StatusCode::FORBIDDEN, "csrf check failed").into_response());
    }
    Ok(session)
}

fn session_cookie_from_headers(headers: &HeaderMap, state: &AppState) -> Option<String> {
    let current = session_cookie_name(state);
    cookie_value_from_headers(headers, current).or_else(|| {
        if current == LEGACY_SESSION_COOKIE_NAME {
            None
        } else {
            cookie_value_from_headers(headers, LEGACY_SESSION_COOKIE_NAME)
        }
    })
}

fn pre_auth_csrf_from_headers(headers: &HeaderMap) -> Option<String> {
    cookie_value_from_headers(headers, PRE_AUTH_CSRF_COOKIE_NAME)
}

fn cookie_value_from_headers(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let trimmed = part.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            if name == cookie_name {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn client_ip_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn local_time_text(value: chrono::DateTime<Utc>) -> templates::LocalTimeText {
    templates::LocalTimeText {
        iso: value.to_rfc3339(),
        display: value.format("%d.%m.%y-%H:%M").to_string(),
    }
}

pub mod templates {
    use askama::Template;

    use crate::models::Thread;

    #[derive(Clone)]
    pub struct LocalTimeText {
        pub iso: String,
        pub display: String,
    }

    #[derive(Clone)]
    pub struct ForumUser {
        pub id: String,
        pub handle: String,
        pub role: String,
        pub theme: String,
        pub is_authenticated: bool,
    }

    #[derive(Clone)]
    pub struct UserPostTeaser {
        pub board_name: String,
        pub board_slug: String,
        pub thread_id: uuid::Uuid,
        pub post_id: uuid::Uuid,
        pub url: String,
        pub thread_title: String,
        pub created_at: LocalTimeText,
    }

    #[derive(Clone)]
    pub struct SavedThreadPreview {
        pub thread_id: uuid::Uuid,
        pub title: String,
        pub board_name: String,
    }

    #[derive(Clone)]
    pub struct SubscribedBoardPreview {
        pub slug: String,
        pub name: String,
        pub thread_count: usize,
    }

    #[derive(Clone)]
    pub struct NotificationItem {
        pub id: String,
        pub kind: String,
        pub actor: String,
        pub thread_title: String,
        pub thread_id: Option<String>,
        pub thread_url: String,
        pub target_label: String,
        pub has_thread_link: bool,
        pub created_at: LocalTimeText,
        pub is_read: bool,
        pub message: String,
    }

    #[derive(Clone)]
    pub struct AdminUserItem {
        pub id: String,
        pub handle: String,
        pub role: String,
        pub status: String,
        pub thread_count: usize,
        pub post_count: usize,
        pub can_manage: bool,
        pub profile_url: String,
    }

    #[derive(Clone)]
    pub struct BoardCard {
        pub slug: String,
        pub name: String,
        pub description: String,
        pub thread_count: usize,
        pub subscribed: bool,
    }

    #[derive(Clone)]
    pub struct ThreadListItem {
        pub thread_id: uuid::Uuid,
        pub title: String,
        pub board_slug: String,
        pub board_name: String,
        pub author: String,
        pub created_at: LocalTimeText,
        pub last_activity: LocalTimeText,
        pub snippet: String,
        pub tags: Vec<String>,
        pub reply_count: usize,
        pub participant_count: usize,
        pub score: i32,
        pub score_hidden: bool,
        pub authored_by_current_user: bool,
        pub is_saved: bool,
        pub thread_state: crate::forum::ThreadState,
    }

    #[derive(Clone)]
    pub struct ThreadPost {
        pub id: uuid::Uuid,
        pub author: String,
        pub body_html: String,
        pub created_at: LocalTimeText,
        pub is_current_user: bool,
        pub score: i32,
        pub user_vote: i8,
        pub score_hidden: bool,
    }

    #[derive(Clone)]
    pub struct SearchResultItem {
        pub thread_id: uuid::Uuid,
        pub post_id: Option<uuid::Uuid>,
        pub board_slug: String,
        pub board_name: String,
        pub url: String,
        pub open_label: String,
        pub return_to: String,
        pub thread_title: String,
        pub author: String,
        pub preview: String,
        pub tags: Vec<String>,
        pub created_at: LocalTimeText,
        pub score: i32,
        pub kind: String,
        pub is_saved: bool,
    }

    #[derive(Clone)]
    pub struct SearchFilterChip {
        pub label: String,
        pub value: String,
    }

    #[derive(Clone)]
    pub struct ReportItem {
        pub id: String,
        pub actor: String,
        pub target_author: Option<String>,
        pub target_kind: String,
        pub reason: String,
        pub details: String,
        pub thread_id: Option<String>,
        pub post_id: Option<String>,
        pub created_at: LocalTimeText,
        pub resolved: bool,
        pub priority_label: String,
        pub queue_state: String,
        pub assigned_to: Option<String>,
        pub resolution_note: String,
        pub audit_history: Vec<ReportAuditItem>,
    }

    #[derive(Clone)]
    pub struct ReportAuditItem {
        pub actor: String,
        pub action: String,
        pub note: String,
        pub created_at: LocalTimeText,
    }

    #[derive(Clone)]
    pub struct ModerationAuditItem {
        pub actor: String,
        pub action: String,
        pub reason: String,
        pub note: String,
        pub created_at: LocalTimeText,
    }

    #[derive(Clone)]
    pub struct PatItem {
        pub id: String,
        pub name: String,
        pub preview: String,
        pub created_at: LocalTimeText,
        pub last_used_at: Option<LocalTimeText>,
    }

    #[derive(Clone)]
    pub struct ForumChrome {
        pub boards: Vec<crate::models::Board>,
        pub current_user: ForumUser,
        pub recent_posts: Vec<UserPostTeaser>,
        pub active_board_slug: String,
        pub notifications: Vec<NotificationItem>,
        pub notification_count: usize,
        pub compactness: i8,
        pub compactness_label: String,
        pub next_compactness: i8,
        pub theme: String,
        pub saved_threads: Vec<SavedThreadPreview>,
        pub subscribed_boards: Vec<SubscribedBoardPreview>,
        pub csrf_token: String,
    }

    #[derive(Clone)]
    pub struct PublicChrome {
        pub current_user: ForumUser,
        pub theme: String,
    }

    #[derive(Clone)]
    pub struct AuthChrome {
        pub theme: String,
    }

    #[derive(Template)]
    #[template(path = "index.html", escape = "html")]
    pub struct IndexTemplate {
        pub chrome: ForumChrome,
        pub intro_text: String,
        pub board_cards: Vec<BoardCard>,
        pub front_page_threads: Vec<ThreadListItem>,
        pub feed: String,
        pub front_page_title: String,
        pub front_page_subtitle: String,
    }

    #[derive(Template)]
    #[template(path = "board.html", escape = "html")]
    pub struct BoardTemplate {
        pub chrome: ForumChrome,
        pub board: crate::models::Board,
        pub board_slug: String,
        pub threads: Vec<ThreadListItem>,
        pub sort: String,
        pub page: usize,
        pub page_count: usize,
        pub board_thread_count: usize,
        pub subscribed: bool,
    }

    #[derive(Template)]
    #[template(path = "thread.html", escape = "html")]
    pub struct ThreadTemplate {
        pub chrome: ForumChrome,
        pub thread: Thread,
        pub thread_author: String,
        pub thread_board_name: String,
        pub thread_posted_at: LocalTimeText,
        pub thread_delete_note: String,
        pub posts: Vec<ThreadPost>,
        pub posting_notice: String,
        pub thread_participants: Vec<String>,
        pub thread_participants_json: String,
        pub can_vote_down: bool,
        pub thread_score_text: String,
        pub thread_score: Option<i32>,
        pub is_saved: bool,
        pub is_hidden: bool,
        pub thread_tags: Vec<String>,
        pub thread_state: crate::forum::ThreadState,
        pub participant_count: usize,
        pub reply_count: usize,
        /// `?moderation=pending|quarantined|removed` flag set when the page is
        /// reached straight from a thread/post create. Empty string means
        /// "no banner".
        pub moderation_state: String,
    }

    #[derive(Template)]
    #[template(path = "profile.html", escape = "html")]
    pub struct ProfileTemplate {
        pub chrome: ForumChrome,
        pub own_threads: Vec<ThreadListItem>,
        pub own_thread_count: usize,
        pub own_post_count: usize,
        pub saved_thread_items: Vec<ThreadListItem>,
        pub subscribed_board_cards: Vec<BoardCard>,
        pub api_tokens: Vec<PatItem>,
        pub notify_mentions: bool,
        pub notify_replies: bool,
        pub notify_moderation: bool,
        pub probation_until: Option<LocalTimeText>,
    }

    #[derive(Template)]
    #[template(path = "notifications.html", escape = "html")]
    pub struct NotificationsTemplate {
        pub chrome: ForumChrome,
    }

    #[derive(Template)]
    #[template(path = "search.html", escape = "html")]
    pub struct SearchTemplate {
        pub chrome: ForumChrome,
        pub query: String,
        pub scope: String,
        pub active_filters: Vec<SearchFilterChip>,
        pub results: Vec<SearchResultItem>,
        pub result_count: usize,
    }

    #[derive(Template)]
    #[template(path = "guide.html", escape = "html")]
    pub struct GuideTemplate {
        pub public: PublicChrome,
        pub show_welcome: bool,
        pub probation_until: Option<LocalTimeText>,
    }

    #[derive(Template)]
    #[template(path = "login.html", escape = "html")]
    pub struct LoginTemplate {
        pub auth: AuthChrome,
        pub csrf_token: String,
        pub return_to: String,
        pub error: String,
    }

    #[derive(Template)]
    #[template(path = "register.html", escape = "html")]
    pub struct RegisterTemplate {
        pub auth: AuthChrome,
        pub csrf_token: String,
        pub return_to: String,
        pub code: String,
        pub error: String,
        pub invite_title: String,
        pub invite_copy: String,
    }

    #[derive(Template)]
    #[template(path = "users.html", escape = "html")]
    pub struct UsersTemplate {
        pub chrome: ForumChrome,
        pub search: String,
        pub users: Vec<AdminUserItem>,
    }

    #[derive(Template)]
    #[template(path = "user-detail.html", escape = "html")]
    pub struct AdminUserDetailTemplate {
        pub chrome: ForumChrome,
        pub detail_user: AdminUserItem,
        pub subtree_descendant_count: usize,
        pub subtree_active_invites: usize,
        pub subtree_handles: Vec<String>,
        pub moderation_history: Vec<ModerationAuditItem>,
        pub deactivated_reason: String,
        pub probation_until: Option<LocalTimeText>,
        pub user_threads: Vec<ThreadListItem>,
        pub user_posts: Vec<UserPostTeaser>,
    }

    #[derive(Template)]
    #[template(path = "reports.html", escape = "html")]
    pub struct ReportsTemplate {
        pub chrome: ForumChrome,
        pub current_status: String,
        pub current_queue_state: String,
        pub current_assigned: String,
        pub current_priority: String,
        pub current_action: String,
        pub current_moderator: String,
        pub current_since: String,
        pub current_until: String,
        pub return_to: String,
        pub csv_export_url: String,
        pub urgent_count: usize,
        pub normal_count: usize,
        pub urgent_reports: Vec<ReportItem>,
        pub normal_reports: Vec<ReportItem>,
    }

    #[derive(Template)]
    #[template(path = "metrics.html", escape = "html")]
    pub struct MetricsTemplate {
        pub chrome: ForumChrome,
        pub metrics: crate::forum::AdminMetrics,
        pub backup: BackupStatusCard,
        pub ingress: IngressStatusCard,
    }

    pub struct ModerationEventListItem {
        pub id: String,
        pub actor: String,
        pub action: String,
        pub reason: String,
        pub note: String,
        pub report_id: String,
        pub target_user: String,
        pub target_thread_id: String,
        pub target_post_id: String,
        pub created_at: LocalTimeText,
    }

    #[derive(Template)]
    #[template(path = "moderation-events.html", escape = "html")]
    pub struct ModerationEventsTemplate {
        pub chrome: ForumChrome,
        pub current_action: String,
        pub current_moderator: String,
        pub current_target: String,
        pub current_report_id: String,
        pub current_since: String,
        pub current_until: String,
        pub current_limit: usize,
        pub csv_export_url: String,
        pub event_count: usize,
        pub items: Vec<ModerationEventListItem>,
    }

    pub struct BackupStatusCard {
        pub status: String,
        pub backup_dir: String,
        pub latest_backup_file: String,
        pub latest_backup_at: Option<LocalTimeText>,
        pub latest_backup_age_hours: Option<i64>,
        pub latest_verified_file: String,
        pub latest_verified_at: Option<LocalTimeText>,
    }

    pub struct IngressStatusCard {
        pub status: String,
        pub secure_cookies: bool,
        pub rate_limit_disabled: bool,
        pub trust_mode: String,
        pub trusted_cidrs: String,
        pub client_ip_headers: String,
    }

    #[derive(Template)]
    #[template(path = "static-page.html", escape = "html")]
    pub struct StaticPageTemplate {
        pub public: PublicChrome,
        pub title: String,
        pub kicker: String,
        pub intro: String,
        pub sections: Vec<(String, String)>,
    }

    #[derive(Template)]
    #[template(path = "error-page.html", escape = "html")]
    pub struct ErrorPageTemplate {
        pub public: PublicChrome,
        pub title: String,
        pub kicker: String,
        pub intro: String,
        pub details: String,
        pub next_step: String,
        pub action_href: String,
        pub action_label: String,
    }
}
