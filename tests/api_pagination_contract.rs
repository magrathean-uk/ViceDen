use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use serde_json::Value;
use tower::util::ServiceExt;
use uuid::Uuid;
mod common;
use viceden::{app::build_router, forum, AppState};

async fn test_state() -> AppState {
    common::app_state().await
}

async fn json_body(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.expect("body should read");
    serde_json::from_slice(&body).expect("body should be valid json")
}

#[tokio::test(flavor = "multi_thread")]
async fn api_thread_lists_clamp_limit() {
    let state = test_state().await;
    let (pat, board_slug) = {
        let mut store = common::compat_store(&state).await;
        let pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "pagination".to_string())
            .expect("pat should issue")
            .token;
        let board_slug = state.boards.first().expect("board exists").slug.clone();
        for idx in 0..140 {
            let _ = store.create_thread_with_checks(
                &board_slug,
                format!("pagination-{idx}-{:#x}", Uuid::new_v4().as_u128()),
                "body".to_string(),
                forum::ROOT_USER_ID,
            );
        }
        (pat, board_slug)
    };
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/boards/{board_slug}/threads?limit=10000&page=1"))
                .header(header::AUTHORIZATION, format!("Bearer {pat}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert!(json.as_array().expect("array response").len() <= 100);
}
