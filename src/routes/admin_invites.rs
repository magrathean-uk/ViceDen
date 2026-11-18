//! `/admin/invites` page: list, issue, and revoke invite codes.
//!
//! Founder-only. The `require_session` helper in `routes::web` is already
//! founder-gated (it tears down sessions whose user is not a founder), so we
//! reuse it here rather than introducing a parallel helper.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    repository::invites::{InviteRepo, NewInvite},
    repository::users::UserRepo,
    routes::web::{build_shell_context, require_session, templates::ForumChrome},
    AppState,
};

pub(crate) const ACTIVE_INVITE_LIMIT: usize = 5;

#[derive(Template)]
#[template(path = "admin-invites.html", escape = "html")]
pub struct AdminInvitesTemplate {
    pub chrome: ForumChrome,
    pub invites: Vec<InviteListItem>,
    pub flash: String,
    pub active_invites: usize,
    pub active_limit: usize,
    pub high_risk_active_invites: usize,
}

#[derive(Debug, Clone)]
pub struct InviteListItem {
    pub code: String,
    pub redemptions: String,
    pub expires_at: String,
    pub revoked: bool,
    pub note: String,
    pub redeemed_by: String,
    pub downstream_count: usize,
    pub impact_summary: String,
    pub revoke_reason: String,
    pub risk_label: String,
    pub risk_reason: String,
}

#[derive(Debug, Deserialize)]
pub struct IssueForm {
    pub note: Option<String>,
    pub max_redemptions: Option<i32>,
    pub csrf: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeForm {
    pub reason: Option<String>,
    pub csrf: Option<String>,
}

/// `GET /admin/invites` — list invites issued by the calling founder.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };

    render_invites_page(&state, &session, StatusCode::OK, String::new()).await
}

/// `POST /admin/invites` — issue a new invite code.
pub async fn issue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(payload): Form<IssueForm>,
) -> Response {
    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if payload.csrf.as_deref() != Some(session.csrf_token.as_str()) {
        return (StatusCode::FORBIDDEN, "csrf check failed").into_response();
    }

    let pool = match state.db.as_ref() {
        Some(pool) => pool,
        None => return (StatusCode::SERVICE_UNAVAILABLE, "no postgres").into_response(),
    };

    let active_invites =
        InviteRepo.count_unspent_for_inviters(pool, &[session.user_id]).await.unwrap_or(0) as usize;
    if active_invites >= ACTIVE_INVITE_LIMIT {
        return render_invites_page(
            &state,
            &session,
            StatusCode::CONFLICT,
            format!(
                "Active invite limit reached ({ACTIVE_INVITE_LIMIT}). Revoke or spend one first."
            ),
        )
        .await;
    }
    if high_risk_active_invite_count(pool, session.user_id).await.unwrap_or(0) > 0 {
        return render_invites_page(
            &state,
            &session,
            StatusCode::CONFLICT,
            "High-risk invite still active. Revoke or cauterise before issuing more.".into(),
        )
        .await;
    }

    let code = format!("vdn-inv-{}", hex::encode(rand_bytes(16)));
    let max_redemptions = payload.max_redemptions.unwrap_or(1).max(1);
    let _ = InviteRepo
        .issue(
            pool,
            NewInvite {
                code,
                inviter_user_id: session.user_id,
                max_redemptions,
                expires_at: None,
                note: payload.note,
            },
        )
        .await;

    Redirect::to("/admin/invites").into_response()
}

/// `POST /admin/invites/{code}/revoke` — revoke an invite by code.
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(code): Path<String>,
    Form(payload): Form<RevokeForm>,
) -> Response {
    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if payload.csrf.as_deref() != Some(session.csrf_token.as_str()) {
        return (StatusCode::FORBIDDEN, "csrf check failed").into_response();
    }

    let pool = match state.db.as_ref() {
        Some(pool) => pool,
        None => return (StatusCode::SERVICE_UNAVAILABLE, "no postgres").into_response(),
    };

    let reason = payload.reason.unwrap_or_else(|| "founder revoke".into());
    let _ = InviteRepo.revoke(pool, &code, session.user_id, &reason).await;

    Redirect::to("/admin/invites").into_response()
}

fn rand_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("system RNG should be available");
    buf
}

async fn render_invites_page(
    state: &AppState,
    session: &crate::routes::web::WebSession,
    status: StatusCode,
    flash: String,
) -> Response {
    use std::collections::HashMap;

    let pool = match state.db.as_ref() {
        Some(pool) => pool,
        None => return (StatusCode::SERVICE_UNAVAILABLE, "no postgres").into_response(),
    };

    let invite_rows = InviteRepo.list_for_inviter(pool, session.user_id).await.unwrap_or_default();
    let codes = invite_rows.iter().map(|row| row.code.clone()).collect::<Vec<_>>();
    let redemptions = InviteRepo.list_redemptions_for_codes(pool, &codes).await.unwrap_or_default();
    let redeemed_ids = redemptions.iter().map(|item| item.redeemed_user_id).collect::<Vec<_>>();
    let handle_map: HashMap<uuid::Uuid, String> = UserRepo
        .handle_batch_lookup(pool, &redeemed_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut redemptions_by_code: HashMap<String, Vec<uuid::Uuid>> = HashMap::new();
    for row in redemptions {
        redemptions_by_code.entry(row.invite_code).or_default().push(row.redeemed_user_id);
    }
    let mut invites = Vec::with_capacity(invite_rows.len());
    for row in invite_rows {
        let redeemed_users = redemptions_by_code.get(&row.code).cloned().unwrap_or_default();
        let redeemed_by = if redeemed_users.is_empty() {
            "Not redeemed".into()
        } else {
            redeemed_users
                .iter()
                .filter_map(|id| handle_map.get(id).cloned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let redeemed_handles =
            redeemed_users.iter().filter_map(|id| handle_map.get(id).cloned()).collect::<Vec<_>>();
        let mut downstream_count = 0usize;
        for user_id in &redeemed_users {
            downstream_count +=
                InviteRepo.subtree_descendants(pool, *user_id).await.unwrap_or_default().len();
        }
        let risk_label = risk_label(&row, downstream_count).to_string();
        let risk_reason = risk_reason(&row, downstream_count);
        invites.push(InviteListItem {
            code: row.code,
            redemptions: format!("{}/{}", row.redemption_count, row.max_redemptions),
            expires_at: row.expires_at.map(|d| d.to_rfc3339()).unwrap_or_else(|| "—".into()),
            revoked: row.revoked_at.is_some(),
            note: row.note.unwrap_or_default(),
            redeemed_by,
            downstream_count,
            impact_summary: impact_summary(&redeemed_handles, downstream_count),
            revoke_reason: row.revoke_reason.unwrap_or_default(),
            risk_label,
            risk_reason,
        });
    }
    let active_invites =
        InviteRepo.count_unspent_for_inviters(pool, &[session.user_id]).await.unwrap_or(0) as usize;
    let high_risk_active_invites =
        high_risk_active_invite_count(pool, session.user_id).await.unwrap_or(0);
    let chrome = build_shell_context(state, Some(session), 8, String::new()).await;
    let template = AdminInvitesTemplate {
        chrome,
        invites,
        flash,
        active_invites,
        active_limit: ACTIVE_INVITE_LIMIT,
        high_risk_active_invites,
    };
    template
        .render()
        .map(|html| (status, Html(html)).into_response())
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "render failed").into_response())
}

pub(crate) async fn high_risk_active_invite_count(
    pool: &sqlx::PgPool,
    inviter_user_id: uuid::Uuid,
) -> Result<usize, sqlx::Error> {
    let invite_rows = InviteRepo.list_for_inviter(pool, inviter_user_id).await?;
    let codes = invite_rows.iter().map(|row| row.code.clone()).collect::<Vec<_>>();
    let redemptions = InviteRepo.list_redemptions_for_codes(pool, &codes).await?;
    let mut redemptions_by_code: std::collections::HashMap<String, Vec<uuid::Uuid>> =
        std::collections::HashMap::new();
    for row in redemptions {
        redemptions_by_code.entry(row.invite_code).or_default().push(row.redeemed_user_id);
    }
    let mut high_risk = 0usize;
    for row in invite_rows {
        if row.revoked_at.is_some() || row.redemption_count >= row.max_redemptions {
            continue;
        }
        let redeemed_users = redemptions_by_code.get(&row.code).cloned().unwrap_or_default();
        let mut downstream_count = 0usize;
        for user_id in &redeemed_users {
            downstream_count += InviteRepo.subtree_descendants(pool, *user_id).await?.len();
        }
        if risk_label(&row, downstream_count) == "high" {
            high_risk += 1;
        }
    }
    Ok(high_risk)
}

pub(crate) fn impact_summary(redeemed_handles: &[String], downstream_count: usize) -> String {
    if redeemed_handles.is_empty() {
        return "Unused. Revoke blocks future use only.".into();
    }

    let redeemers = redeemed_handles.iter().map(|handle| format!("@{handle}")).collect::<Vec<_>>();
    if downstream_count == 0 {
        return format!(
            "Redeemed by {}. Revoke blocks future use only. Existing account stays active.",
            redeemers.join(", ")
        );
    }

    format!(
        "Redeemed by {}. Revoke blocks future use only. {downstream_count} downstream account(s) stay active.",
        redeemers.join(", ")
    )
}

pub(crate) fn risk_label(
    row: &crate::repository::invites::InviteRow,
    downstream_count: usize,
) -> &'static str {
    if row.revoked_at.is_some() {
        "revoked"
    } else if downstream_count >= 3
        || row.redemption_count >= 3
        || (downstream_count >= 1 && row.redemption_count >= 2)
    {
        "high"
    } else if row.max_redemptions > 1 || row.redemption_count > 0 || invite_age_days(row) >= 14 {
        "watch"
    } else {
        "low"
    }
}

pub(crate) fn risk_reason(
    row: &crate::repository::invites::InviteRow,
    downstream_count: usize,
) -> String {
    if row.revoked_at.is_some() {
        return "Already revoked.".into();
    }
    if downstream_count >= 1 && row.redemption_count >= 2 {
        return format!(
            "Code already redeemed {} times and now reaches {downstream_count} downstream account(s).",
            row.redemption_count
        );
    }
    if downstream_count >= 3 {
        return format!("Invite tree already reaches {downstream_count} downstream accounts.");
    }
    if row.redemption_count >= 3 {
        return format!("Code already redeemed {} times.", row.redemption_count);
    }
    if row.max_redemptions > 1 {
        return format!("Multi-use code allows up to {} redemptions.", row.max_redemptions);
    }
    let age_days = invite_age_days(row);
    if age_days >= 14 && row.redemption_count == 0 {
        return format!("Unused for {age_days} days.");
    }
    if row.redemption_count > 0 {
        return "Redeemed once. Watch downstream growth.".into();
    }
    "Fresh single-use code.".into()
}

fn invite_age_days(row: &crate::repository::invites::InviteRow) -> i64 {
    (chrono::Utc::now() - row.created_at).num_days()
}
