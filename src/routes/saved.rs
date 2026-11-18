//! `/saved` page — mobile-friendly fallback for the inspector rail.
//!
//! At <= 1024 px the right inspector is hidden by the responsive shell, so
//! this dedicated page lists the same set of saved threads on its own page.

use askama::Template;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};

use crate::{
    repository::{preferences::PreferencesRepo, threads::ThreadRepo},
    routes::web::{build_shell_context, require_session, templates::ForumChrome},
    AppState,
};

#[derive(Template)]
#[template(path = "saved.html", escape = "html")]
pub struct SavedTemplate {
    pub chrome: ForumChrome,
    pub items: Vec<SavedItem>,
}

pub struct SavedItem {
    pub thread_id: uuid::Uuid,
    pub title: String,
    pub board_slug: String,
}

/// `GET /saved` — list the calling user's saved threads.
pub async fn view_saved(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match require_session(&headers, &state).await {
        Ok(session) => session,
        Err(response) => return response,
    };

    let pool = match state.db.as_ref() {
        Some(pool) => pool,
        None => return (StatusCode::SERVICE_UNAVAILABLE, "no postgres").into_response(),
    };

    let saved_ids =
        PreferencesRepo.saved_threads_for(pool, session.user_id).await.unwrap_or_default();

    let mut items = Vec::with_capacity(saved_ids.len());
    for thread_id in saved_ids {
        if let Ok(Some(row)) = ThreadRepo.find_by_id(pool, thread_id).await {
            items.push(SavedItem { thread_id, title: row.title, board_slug: row.board_slug });
        }
    }

    let chrome = build_shell_context(&state, Some(&session), 0, String::new()).await;
    SavedTemplate { chrome, items }
        .render()
        .map(|html| Html(html).into_response())
        .unwrap_or_else(|_| (StatusCode::INTERNAL_SERVER_ERROR, "render failed").into_response())
}
