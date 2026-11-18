//! JSON API route handlers.
//!
//! The API mirrors the HTML layer and uses the shared forum store.

use axum::{
    extract::{Path, Query, State},
    http::{
        header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
        HeaderMap, StatusCode,
    },
    response::{IntoResponse, Json},
    routing::{delete, get, patch, post},
    Router,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::routes::pagination;
use crate::{forum, AppState};

pub fn api_router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/boards", get(list_boards))
        .route("/boards/{slug}/threads", get(list_threads).post(create_thread))
        .route("/boards/{slug}/subscription", post(toggle_board_subscription))
        .route("/threads/{id}/posts", get(list_posts).post(create_post))
        .route("/threads/{id}/save", post(toggle_thread_save))
        .route("/threads/{id}/hide", post(toggle_thread_hide))
        .route("/threads/{id}/report", post(report_thread))
        .route("/posts/{id}", patch(edit_post))
        .route("/posts/{id}/vote", post(vote_post))
        .route("/posts/{id}/report", post(report_post))
        .route("/me", get(get_my_profile))
        .route("/me/settings", patch(update_my_settings))
        .route("/me/saved", get(list_saved_threads))
        .route("/me/hidden", get(list_hidden_threads))
        .route("/me/mutes/{id}", post(mute_user).delete(unmute_user))
        .route("/me/tags/{tag}/subscription", post(follow_tag).delete(unfollow_tag))
        .route("/me/notifications", get(list_notifications))
        .route("/me/notifications/{id}", patch(mark_notification_read))
        .route("/me/notifications/read-all", post(mark_all_notifications))
        .route("/me/pats", get(list_pats).post(create_pat))
        .route("/me/pats/{id}", delete(delete_pat))
        .route("/search", get(search))
        .route("/admin/users", get(list_admin_users))
        .route("/admin/users/{id}/cauterise", post(cauterise_user))
        .route("/admin/users/{id}/lift-probation", post(lift_user_probation))
        .route("/admin/users/{id}/wipe", post(wipe_user))
        .route("/admin/users/{id}/delete", post(delete_user))
        .route("/admin/metrics", get(admin_metrics))
        .route("/admin/invites", get(list_invites).post(issue_invite))
        .route("/admin/moderation-events", get(list_moderation_events))
        .route("/admin/threads/{id}/state", patch(update_thread_state))
        .route("/admin/reports", get(list_reports))
        .route("/admin/reports/{id}/triage", post(triage_report))
        .route("/admin/reports/{id}", patch(resolve_report))
        .route("/admin/moderation/{target}/{id}", post(handle_moderation))
        .fallback(api_not_found)
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    storage_mode: &'static str,
    database: String,
}

#[derive(Debug, Serialize)]
struct BoardsResponse {
    boards: Vec<crate::models::Board>,
}

#[derive(Debug, Deserialize)]
struct ListThreadsQuery {
    sort: Option<String>,
    page: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ListPostsQuery {
    page: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
    scope: Option<String>,
    page: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AdminReportsQuery {
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
struct AdminModerationEventsQuery {
    action: Option<String>,
    moderator: Option<String>,
    report_id: Option<Uuid>,
    target_user_id: Option<Uuid>,
    since: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
    format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateThreadFromBoardPayload {
    title: String,
    body: String,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct CreatePostPayload {
    body: String,
}

#[derive(Debug, Deserialize)]
struct EditPostPayload {
    body: String,
}

#[derive(Debug, Deserialize)]
struct VotePayload {
    value: i8,
}

#[derive(Debug, Deserialize)]
struct NotificationReadPayload {
    read: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ProfileSettingsPayload {
    compactness: Option<i8>,
    #[serde(rename = "theme")]
    _theme: Option<String>,
    notify_mentions: Option<bool>,
    notify_replies: Option<bool>,
    notify_moderation: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreatePatPayload {
    name: String,
}

#[derive(Debug, Deserialize)]
struct IssueInvitePayload {
    label: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CauteriseUserPayload {
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LiftProbationPayload {
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadStatePayload {
    pinned: Option<bool>,
    locked: Option<bool>,
    answered: Option<bool>,
    closed: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ReportTriagePayload {
    queue_state: Option<String>,
    claim: Option<bool>,
    release: Option<bool>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModerationPayload {
    action: String,
    reason: Option<String>,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReportPayload {
    reason: String,
    details: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveReportPayload {
    resolved: bool,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Serialize)]
struct ThreadItem {
    id: Uuid,
    board_slug: String,
    title: String,
    tags: Vec<String>,
    author_id: Uuid,
    author: String,
    created_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
    score: i32,
    reply_count: usize,
    score_hidden: bool,
    trust_level: u8,
    state: crate::forum::ThreadState,
}

#[derive(Debug, Serialize)]
struct PostItem {
    id: Uuid,
    thread_id: Uuid,
    author_id: Uuid,
    author: String,
    body: String,
    created_at: chrono::DateTime<Utc>,
    score: i32,
    my_vote: i8,
    edited: bool,
}

#[derive(Debug, Serialize)]
struct NotificationItem {
    id: Uuid,
    kind: String,
    actor: Option<String>,
    thread_id: Option<Uuid>,
    thread_title: String,
    post_id: Option<Uuid>,
    target_url: Option<String>,
    target_label: Option<String>,
    message: String,
    created_at: chrono::DateTime<Utc>,
    is_read: bool,
}

#[derive(Debug, Serialize)]
struct SearchItem {
    thread_id: Uuid,
    post_id: Option<Uuid>,
    board_slug: String,
    board_name: String,
    thread_title: String,
    author: String,
    preview: String,
    tags: Vec<String>,
    url: String,
    created_at: chrono::DateTime<Utc>,
    score: i32,
    kind: String,
    is_saved: bool,
}

#[derive(Debug, Serialize)]
struct SearchFilterItem {
    label: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    scope: String,
    filters: Vec<SearchFilterItem>,
    results: Vec<SearchItem>,
}

#[derive(Debug, Serialize)]
struct MeThreadItem {
    thread_id: Uuid,
    board_slug: String,
    title: String,
    url: String,
    reply_count: usize,
}

#[derive(Debug, Serialize)]
struct UserProfileResponse {
    id: Uuid,
    handle: String,
    role: String,
    compactness: i8,
    theme: String,
    notify_mentions: bool,
    notify_replies: bool,
    notify_moderation: bool,
    probation_until: Option<chrono::DateTime<Utc>>,
    started_threads: usize,
    total_posts: usize,
    recent_posts: Vec<PostItem>,
}

#[derive(Debug, Serialize)]
struct PatItem {
    id: Uuid,
    name: String,
    preview: String,
    created_at: chrono::DateTime<Utc>,
    last_used_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct CreatedPatResponse {
    id: Uuid,
    name: String,
    preview: String,
    token: String,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct VoteResponse {
    post_id: Uuid,
    score: i32,
    my_vote: i8,
    thread_id: Uuid,
}

#[derive(Debug, Serialize)]
struct EmptyResponse {
    ok: bool,
}

#[derive(Debug, Serialize)]
struct AdminMetricsResponse {
    total_users: usize,
    total_threads: usize,
    total_posts: usize,
    open_reports: usize,
    unresolved_notifications: usize,
    backup: BackupStatusResponse,
    ingress: IngressStatusResponse,
}

#[derive(Debug, Serialize)]
struct BackupStatusResponse {
    status: String,
    backup_dir: Option<String>,
    latest_backup_file: Option<String>,
    latest_backup_at: Option<chrono::DateTime<Utc>>,
    latest_backup_age_hours: Option<i64>,
    latest_verified_file: Option<String>,
    latest_verified_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct IngressStatusResponse {
    status: String,
    secure_cookies: bool,
    rate_limit_disabled: bool,
    trust_mode: String,
    trusted_cidrs: Vec<String>,
    client_ip_headers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PreferenceToggleResponse {
    ok: bool,
    active: bool,
}

#[derive(Debug, Serialize)]
struct AdminUserResponse {
    id: Uuid,
    handle: String,
    role: String,
    is_banned: bool,
    deactivated: bool,
    probation_until: Option<chrono::DateTime<Utc>>,
    thread_count: usize,
    post_count: usize,
    can_manage: bool,
}

#[derive(Debug, Serialize)]
struct CauteriseUserResponse {
    ok: bool,
    subtree_users: usize,
    descendants: usize,
    users_cauterised: usize,
    threads_removed: usize,
    posts_removed: usize,
    invites_revoked: usize,
}

#[derive(Debug, Serialize)]
struct ReportItem {
    id: Uuid,
    actor: String,
    target_kind: String,
    target_author: Option<String>,
    reason: String,
    details: Option<String>,
    thread_id: Option<Uuid>,
    post_id: Option<Uuid>,
    created_at: chrono::DateTime<Utc>,
    resolved: bool,
    priority: i32,
    priority_label: String,
    queue_state: String,
    assigned_user_id: Option<Uuid>,
    assigned_to: Option<String>,
    reviewer_user_id: Option<Uuid>,
    reviewed_at: Option<chrono::DateTime<Utc>>,
    resolution_note: Option<String>,
    audit_history: Vec<ModerationAuditItem>,
}

#[derive(Debug, Serialize)]
struct ModerationAuditItem {
    id: Uuid,
    actor_id: Uuid,
    target_user_id: Option<Uuid>,
    action: String,
    reason: String,
    note: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct AdminModerationEventItem {
    id: Uuid,
    actor_id: Uuid,
    actor: String,
    target_post_id: Option<Uuid>,
    target_thread_id: Option<Uuid>,
    target_user_id: Option<Uuid>,
    report_id: Option<Uuid>,
    action: String,
    reason: String,
    note: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct AdminInviteItem {
    code: String,
    label: String,
    inviter_id: Uuid,
    created_at: chrono::DateTime<Utc>,
    expires_at: Option<chrono::DateTime<Utc>>,
    max_redemptions: i32,
    redemption_count: i32,
    revoked_at: Option<chrono::DateTime<Utc>>,
    revoke_reason: Option<String>,
    redeemed_by: Vec<String>,
    downstream_count: usize,
    impact_summary: String,
    risk_label: String,
    risk_reason: String,
}

type ResponseResult = Result<axum::response::Response, axum::response::Response>;

async fn api_not_found() -> axum::response::Response {
    error_response(StatusCode::NOT_FOUND, "not found")
}

fn parse_query_date_floor(value: Option<&str>) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value?.trim(), "%Y-%m-%d").ok()?;
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc))
}

fn parse_query_date_ceiling(value: Option<&str>) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(value?.trim(), "%Y-%m-%d").ok()?;
    let next_day = date.succ_opt()?;
    let midnight = next_day.and_hms_opt(0, 0, 0)?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(midnight, Utc))
}

async fn healthz(State(state): State<AppState>) -> axum::response::Response {
    let storage_mode = if state.db.is_some() {
        if state.config.database.require_connection {
            "required_postgres"
        } else {
            "optional_postgres"
        }
    } else {
        "local_json"
    };
    let mut database = if state.db.is_some() { "ok".to_string() } else { "missing".to_string() };
    let mut status = "ok";
    let mut code = StatusCode::OK;

    if state.config.database.require_connection {
        match state.db.as_ref() {
            Some(pool) => {
                if let Err(error) = crate::db::ping(pool).await {
                    database = format!("error: {error}");
                    status = "unhealthy";
                    code = StatusCode::SERVICE_UNAVAILABLE;
                } else if database == "missing" || database == "error" {
                    status = "unhealthy";
                    code = StatusCode::SERVICE_UNAVAILABLE;
                }
            }
            None => {
                database = "missing".to_string();
                status = "unhealthy";
                code = StatusCode::SERVICE_UNAVAILABLE;
            }
        }
    }

    (code, Json(HealthResponse { status, storage_mode, database })).into_response()
}

async fn list_boards(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(response) = require_api_user(&headers, &state).await {
        return response;
    }
    Json(BoardsResponse { boards: state.boards.clone() }).into_response()
}

async fn list_threads(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Query(query): Query<ListThreadsQuery>,
) -> impl IntoResponse {
    use crate::repository::{tags::TagRepo, thread_states::ThreadStateRepo, users::UserRepo};

    let user_id = match require_api_user(&headers, &state).await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "database required");
    };
    if !state.boards.iter().any(|board| board.slug == slug) {
        return error_response(StatusCode::NOT_FOUND, "board not found");
    }
    let bounds = pagination::PageBounds::from_query(pagination::PageQuery {
        page: query.page,
        limit: query.limit,
    });

    let previews = match crate::db::list_board_thread_previews(
        pool,
        user_id,
        &slug,
        query.sort.as_deref().unwrap_or("new"),
        bounds.limit,
        bounds.offset,
    )
    .await
    {
        Ok(items) => items,
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"));
        }
    };
    let thread_ids = previews.iter().map(|item| item.thread.id).collect::<Vec<_>>();
    let author_ids = previews.iter().map(|item| item.thread.user_id).collect::<HashSet<_>>();
    let handles = UserRepo
        .handle_batch_lookup(pool, &author_ids.iter().copied().collect::<Vec<_>>())
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let mut tags = HashMap::<Uuid, Vec<String>>::new();
    for (thread_id, tag) in TagRepo.tags_for_threads(pool, &thread_ids).await.unwrap_or_default() {
        tags.entry(thread_id).or_default().push(tag);
    }
    let states = ThreadStateRepo
        .states_for_batch(pool, &thread_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let mut trust = HashMap::new();
    for author_id in author_ids {
        if let Ok(Some(summary)) = UserRepo.summary_with_counts(pool, author_id).await {
            trust.insert(
                author_id,
                trust_level_from_summary(&summary.role, summary.thread_count, summary.post_count),
            );
        }
    }

    let mut items = Vec::with_capacity(previews.len());
    for item in previews {
        let state = states.get(&item.thread.id).copied().unwrap_or_default();
        items.push(ThreadItem {
            id: item.thread.id,
            board_slug: item.thread.board_slug.clone(),
            title: item.thread.title.clone(),
            tags: tags.remove(&item.thread.id).unwrap_or_default(),
            author_id: item.thread.user_id,
            author: handles
                .get(&item.thread.user_id)
                .cloned()
                .unwrap_or_else(|| user_fallback(item.thread.user_id)),
            created_at: item.thread.created_at,
            expires_at: item.thread.expires_at,
            score: item.score,
            reply_count: item.reply_count,
            score_hidden: item.score_hidden,
            trust_level: trust.get(&item.thread.user_id).copied().unwrap_or(0),
            state: forum::ThreadState {
                pinned: state.pinned,
                locked: state.locked,
                answered: state.answered,
                closed: state.closed,
            },
        });
    }

    Json(items).into_response()
}

async fn create_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<CreateThreadFromBoardPayload>,
) -> ResponseResult {
    create_thread_inner(state, headers, slug, payload.title, payload.body, payload.tags).await
}

async fn create_thread_inner(
    state: AppState,
    headers: HeaderMap,
    slug: String,
    title: String,
    body: String,
    tags: Option<Vec<String>>,
) -> ResponseResult {
    use crate::models::{Post, PostState, Thread};
    use crate::repository::{
        posts::{ModState, NewPost, PostRepo},
        tags::TagRepo,
        threads::{NewThread, ThreadRepo},
        users::UserRepo,
    };

    let user_id = require_api_user(&headers, &state).await?;
    let title = title.trim();
    let body = body.trim();

    if title.is_empty() || body.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "title and body are required"));
    }
    if title.chars().count() > 64 {
        return Err(api_error(StatusCode::BAD_REQUEST, "title cannot exceed 64 characters"));
    }
    if body.chars().count() > 32_000 {
        return Err(api_error(StatusCode::BAD_REQUEST, "body cannot exceed 32,000 characters"));
    }
    if let Some(response) = reject_spam_body_api(body) {
        return Err(response);
    }
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let board = state
        .boards
        .iter()
        .find(|board| board.slug == slug)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "board not found"))?;
    let user = UserRepo
        .find_by_id(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "user cannot post in this board"))?;
    if user.banned {
        return Err(api_error(StatusCode::FORBIDDEN, "user cannot post in this board"));
    }
    if crate::services::limits::active_probation_until(user.probation_until, Utc::now()).is_some() {
        return Err(api_error(StatusCode::FORBIDDEN, "probation active"));
    }
    if let Some(limit) =
        crate::services::limits::posting_limit_violation(pool, user_id, &user.role, &slug, true)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    {
        return Err(api_error(limit.status_code(), limit.message()));
    }

    let body_for_classifier = body.to_string();
    let now = Utc::now();
    let thread = Thread {
        id: Uuid::new_v4(),
        board_slug: slug.clone(),
        title: title.to_string(),
        user_id,
        created_at: now,
        expires_at: now + chrono::Duration::days(i64::from(board.ttl_days)),
    };
    let post = Post {
        id: Uuid::new_v4(),
        thread_id: thread.id,
        user_id,
        body: body.to_string(),
        created_at: now,
        state: PostState::Allow,
    };
    let normalized_tags = forum::normalize_thread_tags(tags.unwrap_or_default());
    let tag_refs = normalized_tags.iter().map(String::as_str).collect::<Vec<_>>();
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    ThreadRepo
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
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    PostRepo
        .insert(
            &mut *tx,
            NewPost {
                id: post.id,
                thread_id: thread.id,
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
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    TagRepo
        .set_for_thread(&mut tx, thread.id, &tag_refs)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    tx.commit()
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;

    if let Some(pipeline) = state.moderation.as_ref() {
        let _ = pipeline.classify_thread(thread.id, &body_for_classifier).await;
    }

    Ok((StatusCode::CREATED, Json(thread)).into_response())
}

async fn list_posts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(query): Query<ListPostsQuery>,
) -> impl IntoResponse {
    use crate::repository::{
        posts::PostRepo, threads::ThreadRepo, users::UserRepo, votes::VoteRepo,
    };

    let user_id = match require_api_user(&headers, &state).await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "database required");
    };
    let bounds = pagination::PageBounds::from_query(pagination::PageQuery {
        page: query.page,
        limit: query.limit,
    });
    if !matches!(ThreadRepo.find_by_id(pool, id).await, Ok(Some(_))) {
        return error_response(StatusCode::NOT_FOUND, "thread not found");
    }
    let posts = match PostRepo.visible_posts_for_thread(pool, id, user_id).await {
        Ok(posts) => posts,
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"))
        }
    };
    let visible = posts.into_iter().skip(bounds.offset).take(bounds.limit).collect::<Vec<_>>();
    let post_ids = visible.iter().map(|post| post.id).collect::<Vec<_>>();
    let author_ids = visible.iter().map(|post| post.user_id).collect::<HashSet<_>>();
    let handles = UserRepo
        .handle_batch_lookup(pool, &author_ids.iter().copied().collect::<Vec<_>>())
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let scores = VoteRepo
        .scores_for_thread(pool, id)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let votes = VoteRepo
        .user_votes_for_thread(pool, user_id, id)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let edited = PostRepo
        .edited_post_ids(pool, &post_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let items = visible
        .into_iter()
        .map(|post| PostItem {
            id: post.id,
            thread_id: post.thread_id,
            author_id: post.user_id,
            author: handles
                .get(&post.user_id)
                .cloned()
                .unwrap_or_else(|| user_fallback(post.user_id)),
            body: post.body,
            created_at: post.created_at,
            score: scores.get(&post.id).copied().unwrap_or(0) as i32,
            my_vote: votes.get(&post.id).copied().unwrap_or(0) as i8,
            edited: edited.contains(&post.id),
        })
        .collect::<Vec<_>>();
    Json(items).into_response()
}

async fn create_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<CreatePostPayload>,
) -> ResponseResult {
    use crate::models::{Post, PostState};
    use crate::repository::{
        posts::{ModState, NewPost, PostRepo},
        thread_states::ThreadStateRepo,
        threads::ThreadRepo,
        users::UserRepo,
    };

    let user_id = require_api_user(&headers, &state).await?;
    let body = payload.body.trim();
    if body.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "body is required"));
    }
    if body.chars().count() > 32_000 {
        return Err(api_error(StatusCode::BAD_REQUEST, "body cannot exceed 32,000 characters"));
    }
    if let Some(response) = reject_spam_body_api(body) {
        return Err(response);
    }
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let Some(thread) = ThreadRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    else {
        return Err(api_error(StatusCode::NOT_FOUND, "thread not found"));
    };
    let user = UserRepo
        .find_by_id(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "user cannot post in this board"))?;
    if user.banned || !state.boards.iter().any(|board| board.slug == thread.board_slug) {
        return Err(api_error(StatusCode::FORBIDDEN, "user cannot post in this board"));
    }
    if crate::services::limits::active_probation_until(user.probation_until, Utc::now()).is_some() {
        return Err(api_error(StatusCode::FORBIDDEN, "probation active"));
    }
    if let Some(limit) = crate::services::limits::posting_limit_violation(
        pool,
        user_id,
        &user.role,
        &thread.board_slug,
        false,
    )
    .await
    .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    {
        return Err(api_error(limit.status_code(), limit.message()));
    }
    let state_row =
        ThreadStateRepo.state_for(pool, id).await.unwrap_or_default().unwrap_or_default();
    let can_moderate = user.role == "founder" || user.role == "mod";
    if (state_row.locked || state_row.closed) && !can_moderate {
        return Err(api_error(StatusCode::FORBIDDEN, "thread is locked"));
    }

    let body_for_classifier = body.to_string();
    let now = Utc::now();
    let post = Post {
        id: Uuid::new_v4(),
        thread_id: id,
        user_id,
        body: body.to_string(),
        created_at: now,
        state: PostState::Allow,
    };
    PostRepo
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
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if let Some(pipeline) = state.moderation.as_ref() {
        let _ = pipeline.classify_post(post.id, &body_for_classifier).await;
    }

    Ok((StatusCode::CREATED, Json(post)).into_response())
}

async fn edit_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<EditPostPayload>,
) -> ResponseResult {
    use crate::repository::{posts::PostRepo, users::UserRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let body = payload.body.trim();
    if body.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "body is required"));
    }
    if let Some(response) = reject_spam_body_api(body) {
        return Err(response);
    }
    let Some(post) = PostRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    else {
        return Err(api_error(StatusCode::NOT_FOUND, "post not found"));
    };
    let user = UserRepo
        .find_by_id(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let can_moderate =
        user.as_ref().is_some_and(|user| user.role == "founder" || user.role == "mod");
    if post.user_id != user_id && !can_moderate {
        return Err(api_error(StatusCode::FORBIDDEN, "cannot edit this post"));
    }
    PostRepo
        .update_body_with_edit(pool, id, user_id, body)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn vote_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<VotePayload>,
) -> ResponseResult {
    use crate::services::voting_policy::{self, VotePolicyError};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    match voting_policy::cast_vote(pool, &state.boards, user_id, id, payload.value).await {
        Ok(outcome) => Ok(Json(VoteResponse {
            post_id: outcome.post_id,
            score: outcome.score as i32,
            my_vote: outcome.my_vote as i8,
            thread_id: outcome.thread_id,
        })
        .into_response()),
        Err(VotePolicyError::PostNotFound) => {
            Err(api_error(StatusCode::NOT_FOUND, "post not found"))
        }
        Err(VotePolicyError::BoardNotFound) => {
            Err(api_error(StatusCode::NOT_FOUND, "board not found"))
        }
        Err(VotePolicyError::UserCannotVote) => {
            Err(api_error(StatusCode::FORBIDDEN, "user cannot vote right now"))
        }
        Err(VotePolicyError::DownvotesDisabled) => {
            Err(api_error(StatusCode::FORBIDDEN, "downvotes are disabled for this board"))
        }
        Err(VotePolicyError::InvalidValue) => {
            Err(api_error(StatusCode::BAD_REQUEST, "invalid vote value"))
        }
        Err(VotePolicyError::Database) => {
            Err(api_error(StatusCode::INTERNAL_SERVER_ERROR, "vote unavailable"))
        }
    }
}

async fn toggle_board_subscription(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ResponseResult {
    use crate::repository::preferences::PreferencesRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    if !state.boards.iter().any(|board| board.slug == slug) {
        return Err(api_error(StatusCode::NOT_FOUND, "board not found"));
    }
    let active = !PreferencesRepo
        .is_board_subscribed(pool, user_id, &slug)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if active {
        PreferencesRepo
            .subscribe_board(pool, user_id, &slug)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    } else {
        PreferencesRepo
            .unsubscribe_board(pool, user_id, &slug)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    }
    Ok(Json(PreferenceToggleResponse { ok: true, active }).into_response())
}

async fn toggle_thread_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::{preferences::PreferencesRepo, threads::ThreadRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    if ThreadRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .is_none()
    {
        return Err(api_error(StatusCode::NOT_FOUND, "thread not found"));
    }
    let active = !PreferencesRepo
        .is_thread_saved(pool, user_id, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if active {
        PreferencesRepo
            .save_thread(pool, user_id, id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    } else {
        PreferencesRepo
            .unsave_thread(pool, user_id, id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    }
    Ok(Json(PreferenceToggleResponse { ok: true, active }).into_response())
}

async fn toggle_thread_hide(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::{preferences::PreferencesRepo, threads::ThreadRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    if ThreadRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .is_none()
    {
        return Err(api_error(StatusCode::NOT_FOUND, "thread not found"));
    }
    let active = !PreferencesRepo
        .is_thread_hidden(pool, user_id, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if active {
        PreferencesRepo
            .hide_thread(pool, user_id, id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    } else {
        PreferencesRepo
            .unhide_thread(pool, user_id, id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    }
    Ok(Json(PreferenceToggleResponse { ok: true, active }).into_response())
}

async fn list_saved_threads(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::{preferences::PreferencesRepo, threads::ThreadRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let ids = PreferencesRepo
        .saved_threads_for(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let mut items = Vec::with_capacity(ids.len());
    for thread_id in ids {
        if let Some(preview) = ThreadRepo
            .find_with_preview(pool, thread_id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        {
            items.push(MeThreadItem {
                thread_id,
                board_slug: preview.thread.board_slug,
                title: preview.thread.title,
                url: format!("/t/{thread_id}"),
                reply_count: preview.reply_count as usize,
            });
        }
    }
    Ok(Json(items).into_response())
}

async fn list_hidden_threads(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::{preferences::PreferencesRepo, threads::ThreadRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let ids = PreferencesRepo
        .hidden_threads_for(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let mut items = Vec::with_capacity(ids.len());
    for thread_id in ids {
        if let Some(preview) = ThreadRepo
            .find_with_preview(pool, thread_id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        {
            items.push(MeThreadItem {
                thread_id,
                board_slug: preview.thread.board_slug,
                title: preview.thread.title,
                url: format!("/t/{thread_id}"),
                reply_count: preview.reply_count as usize,
            });
        }
    }
    Ok(Json(items).into_response())
}

async fn report_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReportPayload>,
) -> ResponseResult {
    use crate::repository::{reports::NewReport, reports::ReportRepo, threads::ThreadRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let reason = payload.reason.trim();
    if reason.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "reason is required"));
    }

    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    if ThreadRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .is_none()
    {
        return Err(api_error(StatusCode::NOT_FOUND, "thread not found"));
    }
    let report = forum::ReportRecord {
        id: Uuid::new_v4(),
        actor_id: user_id,
        target_post_id: None,
        target_thread_id: Some(id),
        reason: reason.to_string(),
        details: payload.details,
        created_at: Utc::now(),
        resolved: false,
    };
    ReportRepo
        .insert(
            pool,
            NewReport {
                id: report.id,
                actor_id: report.actor_id,
                target_post_id: report.target_post_id,
                target_thread_id: report.target_thread_id,
                reason: report.reason.clone(),
                details: report.details.clone(),
                created_at: report.created_at,
            },
        )
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok((StatusCode::CREATED, Json(report)).into_response())
}

async fn report_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReportPayload>,
) -> ResponseResult {
    use crate::repository::{posts::PostRepo, reports::NewReport, reports::ReportRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let reason = payload.reason.trim();
    if reason.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "reason is required"));
    }

    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let Some(post) = PostRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    else {
        return Err(api_error(StatusCode::NOT_FOUND, "post not found"));
    };
    let report = forum::ReportRecord {
        id: Uuid::new_v4(),
        actor_id: user_id,
        target_post_id: Some(id),
        target_thread_id: Some(post.thread_id),
        reason: reason.to_string(),
        details: payload.details,
        created_at: Utc::now(),
        resolved: false,
    };
    ReportRepo
        .insert(
            pool,
            NewReport {
                id: report.id,
                actor_id: report.actor_id,
                target_post_id: report.target_post_id,
                target_thread_id: report.target_thread_id,
                reason: report.reason.clone(),
                details: report.details.clone(),
                created_at: report.created_at,
            },
        )
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok((StatusCode::CREATED, Json(report)).into_response())
}

async fn get_my_profile(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::{
        posts::PostRepo, preferences::PreferencesRepo, users::UserRepo, votes::VoteRepo,
    };

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let user = UserRepo
        .find_by_id(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "user not found"))?;
    let settings = PreferencesRepo
        .settings_for(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let summary = UserRepo
        .summary_with_counts(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let activity = UserRepo
        .latest_activity(pool, user_id, 20)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let post_ids = activity.iter().map(|item| item.post_id).collect::<Vec<_>>();
    let edited = PostRepo
        .edited_post_ids(pool, &post_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let mut recent_posts = Vec::with_capacity(activity.len());
    for item in activity {
        let score = VoteRepo.score_for_post(pool, item.post_id).await.unwrap_or(0) as i32;
        let my_vote =
            VoteRepo.user_vote_for_post(pool, user_id, item.post_id).await.unwrap_or(0) as i8;
        recent_posts.push(PostItem {
            id: item.post_id,
            thread_id: item.thread_id,
            author_id: user_id,
            author: user.handle.clone(),
            body: item.body_snippet,
            created_at: item.created_at,
            score,
            my_vote,
            edited: edited.contains(&item.post_id),
        });
    }

    Ok(Json(UserProfileResponse {
        id: user_id,
        handle: user.handle,
        role: user.role,
        compactness: settings.compactness,
        theme: settings.theme,
        notify_mentions: settings.notify_mentions,
        notify_replies: settings.notify_replies,
        notify_moderation: settings.notify_moderation,
        probation_until: crate::services::limits::active_probation_until(
            user.probation_until,
            Utc::now(),
        ),
        started_threads: summary.as_ref().map_or(0, |row| row.thread_count as usize),
        total_posts: summary.as_ref().map_or(0, |row| row.post_count as usize),
        recent_posts,
    })
    .into_response())
}

async fn update_my_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ProfileSettingsPayload>,
) -> ResponseResult {
    use crate::repository::preferences::{PreferencesRepo, UpdateSettings};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let current = PreferencesRepo
        .settings_for(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let settings = UpdateSettings {
        compactness: payload.compactness.unwrap_or(current.compactness).clamp(-2, 2),
        theme: "dark".to_string(),
        notify_mentions: payload.notify_mentions.unwrap_or(current.notify_mentions),
        notify_replies: payload.notify_replies.unwrap_or(current.notify_replies),
        notify_moderation: payload.notify_moderation.unwrap_or(current.notify_moderation),
    };
    PreferencesRepo
        .update_settings(pool, user_id, settings)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn mute_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::{preferences::PreferencesRepo, users::UserRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    if UserRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .is_none()
    {
        return Err(api_error(StatusCode::NOT_FOUND, "user not found"));
    }
    PreferencesRepo
        .mute_user(pool, user_id, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn unmute_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::preferences::PreferencesRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    PreferencesRepo
        .unmute_user(pool, user_id, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn follow_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tag): Path<String>,
) -> ResponseResult {
    use crate::repository::preferences::PreferencesRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let Some(tag) = forum::normalize_thread_tags(vec![tag]).into_iter().next() else {
        return Err(api_error(StatusCode::BAD_REQUEST, "tag is required"));
    };
    PreferencesRepo
        .follow_tag(pool, user_id, &tag)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn unfollow_tag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(tag): Path<String>,
) -> ResponseResult {
    use crate::repository::preferences::PreferencesRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let Some(tag) = forum::normalize_thread_tags(vec![tag]).into_iter().next() else {
        return Err(api_error(StatusCode::BAD_REQUEST, "tag is required"));
    };
    PreferencesRepo
        .unfollow_tag(pool, user_id, &tag)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn list_notifications(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::{notifications::NotificationRepo, users::UserRepo};
    use std::collections::{HashMap, HashSet};

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let rows = NotificationRepo
        .list_for_user(pool, user_id, 50)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let actor_ids = rows
        .iter()
        .filter_map(|row| row.actor_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let handles: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &actor_ids)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .into_iter()
        .collect();
    let items = rows
        .into_iter()
        .map(|row| {
            let (target_url, target_label) = notification_target(row.thread_id, row.post_id);
            NotificationItem {
                id: row.id,
                kind: notification_kind_label(&row.kind).to_string(),
                actor: row.actor_id.and_then(|id| handles.get(&id).cloned()),
                thread_id: row.thread_id,
                thread_title: row.thread_title,
                post_id: row.post_id,
                target_url,
                target_label,
                message: row.message,
                created_at: row.created_at,
                is_read: row.is_read,
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(items).into_response())
}

async fn mark_notification_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<NotificationReadPayload>,
) -> ResponseResult {
    use crate::repository::notifications::NotificationRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    NotificationRepo
        .set_read(pool, id, user_id, payload.read.unwrap_or(true))
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn mark_all_notifications(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ResponseResult {
    use crate::repository::notifications::NotificationRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    NotificationRepo
        .mark_all_read(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn list_pats(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::pats::PatRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let items = PatRepo
        .list_for_user(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .into_iter()
        .map(|item| PatItem {
            id: item.id,
            name: item.name,
            preview: item.preview,
            created_at: item.created_at,
            last_used_at: item.last_used_at,
        })
        .collect::<Vec<_>>();
    Ok(Json(items).into_response())
}

async fn create_pat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<CreatePatPayload>,
) -> ResponseResult {
    use crate::{auth::tokens, repository::pats::PatRepo};

    let user_id = require_api_user(&headers, &state).await?;
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "name is required"));
    }
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let raw_token = tokens::new_pat_token();
    let id = PatRepo
        .issue(pool, user_id, name, &raw_token, state.config.auth.token_pepper.as_deref())
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedPatResponse {
            id,
            name: name.to_string(),
            preview: format!("{}...", raw_token.chars().take(12).collect::<String>()),
            token: raw_token,
            created_at: Utc::now(),
        }),
    )
        .into_response())
}

async fn delete_pat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::pats::PatRepo;

    let user_id = require_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let deleted = PatRepo
        .revoke(pool, id, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if !deleted {
        return Err(api_error(StatusCode::NOT_FOUND, "api token not found"));
    }
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> impl IntoResponse {
    use crate::repository::{tags::TagRepo, users::UserRepo};

    let user_id = match require_api_user(&headers, &state).await {
        Ok(user_id) => user_id,
        Err(response) => return response,
    };
    let Some(pool) = state.db.as_ref() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "database required");
    };
    let scope = forum::SearchScope::from_query(query.scope.as_deref());
    let raw_query = query.q.unwrap_or_default();
    let parsed_query = forum::parse_search_query(&raw_query);
    let bounds = pagination::PageBounds::from_query(pagination::PageQuery {
        page: query.page,
        limit: query.limit,
    });
    let search_query = pagination::bounded_search_query(Some(raw_query.as_str()));
    let rows = match crate::db::search_forum(
        pool,
        user_id,
        &search_query,
        None,
        scope,
        false,
        bounds.offset + bounds.limit,
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"));
        }
    };
    let author_ids = rows.iter().map(|item| item.author_id).collect::<HashSet<_>>();
    let thread_ids = rows.iter().map(|item| item.thread_id).collect::<Vec<_>>();
    let handles = UserRepo
        .handle_batch_lookup(pool, &author_ids.iter().copied().collect::<Vec<_>>())
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<_, _>>();
    let mut tags = HashMap::<Uuid, Vec<String>>::new();
    for (thread_id, tag) in TagRepo.tags_for_threads(pool, &thread_ids).await.unwrap_or_default() {
        tags.entry(thread_id).or_default().push(tag);
    }
    let results = rows
        .into_iter()
        .skip(bounds.offset)
        .take(bounds.limit)
        .map(|item| SearchItem {
            thread_id: item.thread_id,
            post_id: item.post_id,
            board_name: state
                .boards
                .iter()
                .find(|board| board.slug == item.board_slug)
                .map(|board| board.name.clone())
                .unwrap_or_else(|| item.board_slug.clone()),
            board_slug: item.board_slug,
            thread_title: item.thread_title,
            author: handles
                .get(&item.author_id)
                .cloned()
                .unwrap_or_else(|| user_fallback(item.author_id)),
            preview: item.body_snippet,
            tags: tags.remove(&item.thread_id).unwrap_or_default(),
            url: item
                .post_id
                .map(|post_id| format!("/t/{}#post-{post_id}", item.thread_id))
                .unwrap_or_else(|| format!("/t/{}", item.thread_id)),
            created_at: item.created_at,
            score: item.score,
            kind: match item.kind {
                forum::SearchHitKind::Thread => "thread".to_string(),
                forum::SearchHitKind::Post => "post".to_string(),
            },
            is_saved: item.is_saved,
        })
        .collect::<Vec<_>>();

    let mut filters = Vec::new();
    if let Some(tag) = parsed_query.tag {
        filters.push(SearchFilterItem { label: "tag".into(), value: format!("#{tag}") });
    }
    if let Some(author_handle) = parsed_query.author_handle {
        filters
            .push(SearchFilterItem { label: "author".into(), value: format!("@{author_handle}") });
    }
    if let Some(board_slug) = parsed_query.board_slug {
        filters.push(SearchFilterItem { label: "board".into(), value: format!("/{board_slug}") });
    }
    if scope != forum::SearchScope::All {
        filters.push(SearchFilterItem {
            label: "scope".into(),
            value: format!("{} only", scope.as_str()),
        });
    }

    Json(SearchResponse { query: raw_query, scope: scope.as_str().to_string(), filters, results })
        .into_response()
}

async fn list_admin_users(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::users::UserRepo;

    require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let summaries = UserRepo
        .list_summaries_for_admin(pool)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let items: Vec<AdminUserResponse> = summaries
        .into_iter()
        .map(|user| AdminUserResponse {
            id: user.id,
            // Founders are not manageable — same legacy semantic.
            can_manage: user.role != "founder",
            handle: user.handle,
            is_banned: user.banned,
            deactivated: user.deactivated_at.is_some(),
            probation_until: crate::services::limits::active_probation_until(
                user.probation_until,
                Utc::now(),
            ),
            thread_count: user.thread_count as usize,
            post_count: user.post_count as usize,
            role: user.role,
        })
        .collect();
    Ok(Json(items).into_response())
}

async fn wipe_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    match UserRepo.wipe_content(pool, actor_id, id).await {
        Ok(true) => {
            let _ = ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id,
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
            Ok(Json(EmptyResponse { ok: true }).into_response())
        }
        Ok(false) => Err(api_error(StatusCode::FORBIDDEN, "user cannot be wiped")),
        Err(error) => Err(api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"))),
    }
}

async fn lift_user_probation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<LiftProbationPayload>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let actor = UserRepo
        .find_by_id(pool, actor_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .ok_or_else(|| api_error(StatusCode::FORBIDDEN, "founder permission required"))?;
    let target = UserRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "user not found"))?;
    if actor.role != "founder" || target.role == "founder" {
        return Err(api_error(StatusCode::FORBIDDEN, "user cannot be updated"));
    }
    UserRepo
        .set_probation_until(pool, id, None)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
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
                actor_id,
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
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn cauterise_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<CauteriseUserPayload>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let reason = payload.reason.unwrap_or_else(|| "invite subtree containment".into());
    match UserRepo.cauterise_subtree(pool, actor_id, id, &reason).await {
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
                        actor_id,
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
            Ok(Json(CauteriseUserResponse {
                ok: true,
                subtree_users: outcome.subtree_user_ids.len(),
                descendants: outcome.descendant_count,
                users_cauterised: outcome.users_cauterised,
                threads_removed: outcome.threads_removed,
                posts_removed: outcome.posts_removed,
                invites_revoked: outcome.invites_revoked,
            })
            .into_response())
        }
        Ok(None) => Err(api_error(StatusCode::FORBIDDEN, "user cannot be cauterised")),
        Err(error) => Err(api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"))),
    }
}

async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        users::UserRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let target_handle = UserRepo
        .find_by_id(pool, id)
        .await
        .ok()
        .flatten()
        .map(|user| user.handle)
        .unwrap_or_else(|| format!("user-{}", &id.simple().to_string()[..8]));
    match UserRepo.delete_user(pool, actor_id, id).await {
        Ok(true) => {
            let _ = ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id,
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
            Ok(Json(EmptyResponse { ok: true }).into_response())
        }
        Ok(false) => Err(api_error(StatusCode::FORBIDDEN, "user cannot be deleted")),
        Err(error) => Err(api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{error}"))),
    }
}

async fn admin_metrics(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use crate::repository::{
        notifications::NotificationRepo, posts::PostRepo, reports::ReportRepo, threads::ThreadRepo,
        users::UserRepo,
    };

    require_founder_api_user(&headers, &state).await?;
    // A7 partial: read counts straight from PG (mirrors view_metrics).
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let total_users = UserRepo.count(pool).await.unwrap_or(0) as usize;
    let total_threads = ThreadRepo.count(pool).await.unwrap_or(0) as usize;
    let total_posts = PostRepo.count(pool).await.unwrap_or(0) as usize;
    let open_reports = ReportRepo.count_open(pool).await.unwrap_or(0) as usize;
    let unresolved_notifications =
        NotificationRepo.count_unread_global(pool).await.unwrap_or(0) as usize;
    let backup = crate::services::ops_status::backup_status(&state.config.ops);
    let ingress = crate::services::ops_status::ingress_status(&state.config.server);
    Ok(Json(AdminMetricsResponse {
        total_users,
        total_threads,
        total_posts,
        open_reports,
        unresolved_notifications,
        backup: BackupStatusResponse {
            status: backup.status.to_string(),
            backup_dir: backup.backup_dir,
            latest_backup_file: backup.latest_backup_file,
            latest_backup_at: backup.latest_backup_at,
            latest_backup_age_hours: backup.latest_backup_age_hours,
            latest_verified_file: backup.latest_verified_file,
            latest_verified_at: backup.latest_verified_at,
        },
        ingress: IngressStatusResponse {
            status: ingress.status.to_string(),
            secure_cookies: ingress.secure_cookies,
            rate_limit_disabled: ingress.rate_limit_disabled,
            trust_mode: ingress.trust_mode.to_string(),
            trusted_cidrs: ingress.trusted_cidrs,
            client_ip_headers: ingress.client_ip_headers,
        },
    })
    .into_response())
}

async fn list_invites(State(state): State<AppState>, headers: HeaderMap) -> ResponseResult {
    use std::collections::HashMap;

    use crate::repository::{invites::InviteRepo, users::UserRepo};

    require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let rows = InviteRepo
        .list_all(pool)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let codes = rows.iter().map(|row| row.code.clone()).collect::<Vec<_>>();
    let redemptions = InviteRepo
        .list_redemptions_for_codes(pool, &codes)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let redeemed_ids = redemptions.iter().map(|item| item.redeemed_user_id).collect::<Vec<_>>();
    let handle_map: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &redeemed_ids)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .into_iter()
        .collect();
    let mut redemptions_by_code: HashMap<String, Vec<Uuid>> = HashMap::new();
    for row in redemptions {
        redemptions_by_code.entry(row.invite_code).or_default().push(row.redeemed_user_id);
    }
    let mut invites = Vec::with_capacity(rows.len());
    for row in rows {
        let redeemed_users = redemptions_by_code.get(&row.code).cloned().unwrap_or_default();
        let redeemed_by =
            redeemed_users.iter().filter_map(|id| handle_map.get(id).cloned()).collect::<Vec<_>>();
        let mut downstream_count = 0usize;
        for user_id in &redeemed_users {
            downstream_count +=
                InviteRepo.subtree_descendants(pool, *user_id).await.unwrap_or_default().len();
        }
        let risk_label = crate::routes::admin_invites::risk_label(&row, downstream_count).into();
        let risk_reason = crate::routes::admin_invites::risk_reason(&row, downstream_count);
        invites.push(AdminInviteItem {
            code: row.code,
            label: row.note.unwrap_or(row.label),
            inviter_id: row.inviter_user_id,
            created_at: row.created_at,
            expires_at: row.expires_at,
            max_redemptions: row.max_redemptions,
            redemption_count: row.redemption_count,
            revoked_at: row.revoked_at,
            revoke_reason: row.revoke_reason,
            impact_summary: crate::routes::admin_invites::impact_summary(
                &redeemed_by,
                downstream_count,
            ),
            risk_label,
            risk_reason,
            redeemed_by,
            downstream_count,
        });
    }
    Ok(Json(invites).into_response())
}

async fn issue_invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<IssueInvitePayload>,
) -> ResponseResult {
    use crate::repository::invites::{InviteRepo, NewInvite};

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let active_invites = InviteRepo
        .count_unspent_for_inviters(pool, &[actor_id])
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        as usize;
    if active_invites >= crate::routes::admin_invites::ACTIVE_INVITE_LIMIT {
        return Err(api_error(StatusCode::CONFLICT, "active invite limit reached"));
    }
    let high_risk_active =
        crate::routes::admin_invites::high_risk_active_invite_count(pool, actor_id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if high_risk_active > 0 {
        return Err(api_error(StatusCode::CONFLICT, "high-risk invite still active"));
    }
    let label =
        payload.label.as_deref().unwrap_or("api").trim().chars().take(48).collect::<String>();
    let code = format!("vdni-{}", Uuid::new_v4().simple());
    InviteRepo
        .issue(
            pool,
            NewInvite {
                code: code.clone(),
                inviter_user_id: actor_id,
                max_redemptions: 1,
                expires_at: None,
                note: Some(label.clone()),
            },
        )
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let invite = forum::InviteRecord {
        code,
        label,
        inviter_id: actor_id,
        created_at: Utc::now(),
        redeemed_by: None,
        redeemed_at: None,
    };
    Ok((StatusCode::CREATED, Json(invite)).into_response())
}

async fn list_moderation_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminModerationEventsQuery>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventFilter, ModerationEventRepo},
        users::UserRepo,
    };

    require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let moderator_id = if let Some(handle) = query.moderator.as_deref().map(str::trim) {
        if handle.is_empty() {
            None
        } else {
            Some(
                UserRepo
                    .find_by_handle(pool, handle)
                    .await
                    .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
                    .map(|user| user.id)
                    .unwrap_or(Uuid::nil()),
            )
        }
    } else {
        None
    };
    let created_from = parse_query_date_floor(query.since.as_deref());
    let created_to = parse_query_date_ceiling(query.until.as_deref());
    let limit = query.limit.unwrap_or(100).clamp(1, 500) as i64;
    let rows = ModerationEventRepo
        .list_filtered(
            pool,
            ModerationEventFilter {
                action: query.action.as_deref().map(str::trim).filter(|value| !value.is_empty()),
                actor_id: moderator_id,
                report_id: query.report_id,
                target_user_id: query.target_user_id,
                created_from,
                created_to,
                limit,
            },
        )
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let actor_ids = rows.iter().map(|row| row.actor_id).collect::<HashSet<_>>();
    let handles = UserRepo
        .handle_batch_lookup(pool, &actor_ids.iter().copied().collect::<Vec<_>>())
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .into_iter()
        .collect::<HashMap<_, _>>();
    let items = rows
        .into_iter()
        .map(|row| AdminModerationEventItem {
            id: row.id,
            actor_id: row.actor_id,
            actor: handles
                .get(&row.actor_id)
                .cloned()
                .unwrap_or_else(|| user_fallback(row.actor_id)),
            target_post_id: row.target_post_id,
            target_thread_id: row.target_thread_id,
            target_user_id: row.target_user_id,
            report_id: row.report_id,
            action: row.action,
            reason: row.reason,
            note: row.note,
            created_at: row.created_at,
        })
        .collect::<Vec<_>>();
    let wants_csv = query.format.as_deref().is_some_and(|value| value.eq_ignore_ascii_case("csv"))
        || headers
            .get(ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/csv"));
    if wants_csv {
        let mut csv = String::from(
            "id,created_at,actor,actor_id,action,reason,note,report_id,target_user_id,target_thread_id,target_post_id\n",
        );
        for item in &items {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
                item.id,
                item.created_at.to_rfc3339(),
                csv_escape(&item.actor),
                item.actor_id,
                csv_escape(&item.action),
                csv_escape(&item.reason),
                csv_escape(item.note.as_deref().unwrap_or_default()),
                item.report_id.map(|value| value.to_string()).unwrap_or_default(),
                item.target_user_id.map(|value| value.to_string()).unwrap_or_default(),
                item.target_thread_id.map(|value| value.to_string()).unwrap_or_default(),
                item.target_post_id.map(|value| value.to_string()).unwrap_or_default(),
            ));
        }
        return Ok(([(CONTENT_TYPE, "text/csv; charset=utf-8")], csv).into_response());
    }
    Ok(Json(items).into_response())
}

async fn update_thread_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ThreadStatePayload>,
) -> ResponseResult {
    use crate::repository::thread_states::{ThreadStateRepo, ThreadStateRow};

    let _actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;

    // A7 partial: read-modify-write the full ThreadState row via PG.
    // Read current state (default = all-false if absent), apply the
    // payload's `Some(_)` overrides, upsert.
    let current = ThreadStateRepo
        .state_for(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .unwrap_or_default();
    let next = ThreadStateRow {
        pinned: payload.pinned.unwrap_or(current.pinned),
        locked: payload.locked.unwrap_or(current.locked),
        answered: payload.answered.unwrap_or(current.answered),
        closed: payload.closed.unwrap_or(current.closed),
    };
    ThreadStateRepo
        .upsert(pool, id, next)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let view = forum::ThreadState {
        pinned: next.pinned,
        locked: next.locked,
        answered: next.answered,
        closed: next.closed,
    };
    Ok(Json(view).into_response())
}

async fn list_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AdminReportsQuery>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::ModerationEventRepo,
        reports::{ReportQueueFilter, ReportRepo},
        users::UserRepo,
    };
    use std::collections::{HashMap, HashSet};

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let moderator_id = if let Some(handle) = query.moderator.as_deref().map(str::trim) {
        if handle.is_empty() {
            None
        } else {
            Some(
                UserRepo
                    .find_by_handle(pool, handle)
                    .await
                    .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
                    .map(|user| user.id)
                    .unwrap_or(Uuid::nil()),
            )
        }
    } else {
        None
    };
    let event_created_from = parse_query_date_floor(query.since.as_deref());
    let event_created_to = parse_query_date_ceiling(query.until.as_deref());
    let rows = ReportRepo
        .list_queue_filtered(
            pool,
            ReportQueueFilter {
                status: query.status.as_deref(),
                queue_state: query.queue_state.as_deref(),
                assigned: query.assigned.as_deref(),
                priority: query.priority.as_deref(),
                event_action: query.action.as_deref(),
                event_actor_id: moderator_id,
                event_created_from,
                event_created_to,
                viewer_id: Some(actor_id),
            },
        )
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let actor_ids = rows
        .iter()
        .flat_map(|row| [Some(row.actor_id), row.target_user_id, row.assigned_user_id])
        .collect::<HashSet<_>>()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let handles: HashMap<Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &actor_ids)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
        .into_iter()
        .collect();
    let mut items = Vec::with_capacity(rows.len());
    for item in rows {
        let audit_history = ModerationEventRepo
            .list_for_report(pool, item.id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
            .into_iter()
            .map(|event| ModerationAuditItem {
                id: event.id,
                actor_id: event.actor_id,
                target_user_id: event.target_user_id,
                action: event.action,
                reason: event.reason,
                note: event.note,
                created_at: event.created_at,
            })
            .collect::<Vec<_>>();
        items.push(ReportItem {
            id: item.id,
            actor: handles.get(&item.actor_id).cloned().unwrap_or_else(|| "unknown".into()),
            target_kind: if item.target_post_id.is_some() {
                "post".into()
            } else {
                "thread".into()
            },
            target_author: item.target_user_id.and_then(|id| handles.get(&id).cloned()),
            reason: item.reason,
            details: item.details,
            thread_id: item.target_thread_id,
            post_id: item.target_post_id,
            created_at: item.created_at,
            resolved: item.resolved,
            priority: item.priority,
            priority_label: item.priority_label,
            queue_state: item.queue_state,
            assigned_user_id: item.assigned_user_id,
            assigned_to: item.assigned_user_id.and_then(|id| handles.get(&id).cloned()),
            reviewer_user_id: item.reviewer_user_id,
            reviewed_at: item.reviewed_at,
            resolution_note: item.resolution_note,
            audit_history,
        });
    }
    Ok(Json(items).into_response())
}

async fn triage_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ReportTriagePayload>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        reports::ReportRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let Some(report) = ReportRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    else {
        return Err(api_error(StatusCode::NOT_FOUND, "report not found"));
    };
    let queue_state = payload
        .queue_state
        .as_deref()
        .map(str::trim)
        .filter(|value| matches!(*value, "new" | "reviewing" | "waiting"));
    let assigned_to = if payload.release.unwrap_or(false) {
        Some(None)
    } else if payload.claim.unwrap_or(false) {
        Some(Some(actor_id))
    } else {
        None
    };
    ReportRepo
        .triage(pool, id, queue_state, assigned_to)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let action = if payload.release.unwrap_or(false) {
        "release_report"
    } else if payload.claim.unwrap_or(false) {
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
        .or_else(|| queue_state.map(str::to_string));
    let _ = ModerationEventRepo
        .insert(
            pool,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id,
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
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn resolve_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ResolveReportPayload>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        posts::PostRepo,
        reports::ReportRepo,
        threads::ThreadRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let Some(report) = ReportRepo
        .find_by_id(pool, id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
    else {
        return Err(api_error(StatusCode::NOT_FOUND, "report not found"));
    };
    ReportRepo
        .resolve_with_note(pool, id, actor_id, payload.resolved, payload.note.as_deref())
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    let target_user_id = if let Some(post_id) = report.target_post_id {
        PostRepo
            .find_by_id(pool, post_id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
            .map(|post| post.user_id)
    } else if let Some(thread_id) = report.target_thread_id {
        ThreadRepo
            .find_by_id(pool, thread_id)
            .await
            .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
            .map(|thread| thread.user_id)
    } else {
        None
    };
    ModerationEventRepo
        .insert(
            pool,
            ModerationEventInsert {
                id: Uuid::new_v4(),
                actor_id,
                target_post_id: report.target_post_id,
                target_thread_id: report.target_thread_id,
                target_user_id,
                report_id: Some(id),
                action: if payload.resolved { "resolve_report" } else { "reopen_report" }.into(),
                reason: report.reason,
                note: payload.note,
                created_at: Utc::now(),
            },
        )
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    Ok(Json(EmptyResponse { ok: true }).into_response())
}

async fn handle_moderation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((target, id)): Path<(String, Uuid)>,
    Json(payload): Json<ModerationPayload>,
) -> ResponseResult {
    use crate::repository::{
        moderation_events::{ModerationEventInsert, ModerationEventRepo},
        notifications::{NewNotification, NotificationRepo},
        posts::{ModState, PostRepo},
        reports::ReportRepo,
        threads::ThreadRepo,
    };

    let actor_id = require_founder_api_user(&headers, &state).await?;
    let action = payload.action.trim().to_ascii_lowercase();
    if action != "remove" {
        return Err(api_error(StatusCode::BAD_REQUEST, "only remove action is supported"));
    }

    let reason = payload.reason.unwrap_or_else(|| "moderation action".to_string());
    let note = payload.note;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    match target.as_str() {
        "post" => {
            let Some(post) = PostRepo
                .find_by_id(pool, id)
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
            else {
                return Err(api_error(StatusCode::NOT_FOUND, "post not found"));
            };
            let thread = ThreadRepo
                .find_by_id(pool, post.thread_id)
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            PostRepo
                .update_mod_state(pool, id, ModState::Removed, Some(&reason))
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            let linked_report = ReportRepo
                .find_latest_open_for_target(pool, Some(id), Some(post.thread_id))
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id,
                        target_post_id: Some(id),
                        target_thread_id: Some(post.thread_id),
                        target_user_id: Some(post.user_id),
                        report_id: linked_report.as_ref().map(|report| report.id),
                        action: "remove_post".into(),
                        reason: reason.clone(),
                        note: note.clone(),
                        created_at: Utc::now(),
                    },
                )
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            if let Some(thread) = thread {
                let _ = NotificationRepo
                    .insert(
                        pool,
                        NewNotification {
                            id: Uuid::new_v4(),
                            recipient_user_id: post.user_id,
                            actor_id: Some(actor_id),
                            kind: "post_removed".to_string(),
                            thread_id: Some(post.thread_id),
                            thread_title: thread.title,
                            post_id: Some(id),
                            message: moderation_notification_message("post", &reason),
                            created_at: Utc::now(),
                            is_read: false,
                        },
                    )
                    .await;
            }
            Ok(Json(EmptyResponse { ok: true }).into_response())
        }
        "thread" => {
            let Some(thread) = ThreadRepo
                .find_by_id(pool, id)
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?
            else {
                return Err(api_error(StatusCode::NOT_FOUND, "thread not found"));
            };
            ThreadRepo
                .update_mod_state(pool, id, ModState::Removed, Some(&reason))
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            let linked_report = ReportRepo
                .find_latest_open_for_target(pool, None, Some(id))
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            ModerationEventRepo
                .insert(
                    pool,
                    ModerationEventInsert {
                        id: Uuid::new_v4(),
                        actor_id,
                        target_post_id: None,
                        target_thread_id: Some(id),
                        target_user_id: Some(thread.user_id),
                        report_id: linked_report.as_ref().map(|report| report.id),
                        action: "remove_thread".into(),
                        reason: reason.clone(),
                        note,
                        created_at: Utc::now(),
                    },
                )
                .await
                .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
            let _ = NotificationRepo
                .insert(
                    pool,
                    NewNotification {
                        id: Uuid::new_v4(),
                        recipient_user_id: thread.user_id,
                        actor_id: Some(actor_id),
                        kind: "post_removed".to_string(),
                        thread_id: Some(id),
                        thread_title: thread.title,
                        post_id: None,
                        message: moderation_notification_message("thread", &reason),
                        created_at: Utc::now(),
                        is_read: false,
                    },
                )
                .await;
            Ok(Json(EmptyResponse { ok: true }).into_response())
        }
        _ => Err(api_error(StatusCode::BAD_REQUEST, "target must be post or thread")),
    }
}

fn notification_kind_label(kind: &str) -> &str {
    match kind {
        "post_removed" => "post removed",
        "mod_action" => "moderation",
        other => other,
    }
}

fn notification_target(
    thread_id: Option<Uuid>,
    post_id: Option<Uuid>,
) -> (Option<String>, Option<String>) {
    match (thread_id, post_id) {
        (Some(thread_id), Some(post_id)) => {
            (Some(format!("/t/{thread_id}#post-{post_id}")), Some("post".to_string()))
        }
        (Some(thread_id), None) => (Some(format!("/t/{thread_id}")), Some("thread".to_string())),
        (None, _) => (None, None),
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

fn user_fallback(user_id: Uuid) -> String {
    format!("user-{}", &user_id.simple().to_string()[..8])
}

fn csv_escape(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn trust_level_from_summary(role: &str, thread_count: i64, post_count: i64) -> u8 {
    if role == "founder" {
        4
    } else if role == "mod" {
        3
    } else if post_count >= 50 || thread_count >= 10 {
        2
    } else if post_count >= 5 || thread_count >= 2 {
        1
    } else {
        0
    }
}

fn reject_spam_body_api(value: &str) -> Option<axum::response::Response> {
    let link_count = value
        .split_whitespace()
        .filter(|part| part.starts_with("http://") || part.starts_with("https://"))
        .count();
    if link_count > 8 {
        return Some(api_error(StatusCode::TOO_MANY_REQUESTS, "too many links"));
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
        return Some(api_error(StatusCode::TOO_MANY_REQUESTS, "repeated text limit reached"));
    }
    None
}

fn api_error(code: StatusCode, msg: &str) -> axum::response::Response {
    (code, Json(ErrorResponse { error: msg.to_string() })).into_response()
}

fn error_response(code: StatusCode, msg: &str) -> axum::response::Response {
    (code, Json(ErrorResponse { error: msg.to_string() })).into_response()
}

async fn optional_api_user_id(headers: &HeaderMap, state: &AppState) -> Option<Uuid> {
    let token = headers.get("x-api-token").and_then(|token| token.to_str().ok()).or_else(|| {
        headers
            .get(AUTHORIZATION)
            .and_then(|token| token.to_str().ok())
            .and_then(|token| {
                token
                    .strip_prefix("Bearer ")
                    .or_else(|| token.strip_prefix("bearer "))
                    .or_else(|| token.split_once(' ').map(|(_, token)| token))
            })
            .or_else(|| headers.get(AUTHORIZATION).and_then(|token| token.to_str().ok()))
    });

    if let Some(raw_token) = token {
        if let Some(pool) = state.db.as_ref() {
            let pepper = state.config.auth.token_pepper.as_deref();
            if let Ok(Some((user_id, _pat_id))) =
                crate::repository::pats::PatRepo.authenticate(pool, raw_token.trim(), pepper).await
            {
                // Founder-only gates live in `require_founder_api_user`, which is
                // applied per-route. Restricting this resolver to founders alone
                // would lock every non-founder API client out at the door.
                return Some(user_id);
            }
        }
    }

    None
}

async fn require_api_user(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Uuid, axum::response::Response> {
    optional_api_user_id(headers, state)
        .await
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "api token required"))
}

async fn require_founder_api_user(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<Uuid, axum::response::Response> {
    use crate::repository::users::UserRepo;

    let user_id = require_api_user(headers, state).await?;
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::SERVICE_UNAVAILABLE, "database required"))?;
    let user = UserRepo
        .find_by_id(pool, user_id)
        .await
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, &format!("{err}")))?;
    if user.is_none_or(|user| user.role != "founder") {
        return Err(api_error(StatusCode::FORBIDDEN, "founder permission required"));
    }
    Ok(user_id)
}
