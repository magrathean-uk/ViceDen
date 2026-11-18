//! End-to-end rate-limit test for Phase 10.
//!
//! Hits /login six times in quick succession against a router built with
//! required DB pool so the rate-limit middleware is wired (see web.rs:
//! the layers are gated on `state.db.is_some()` until Phase 11 lands and
//! PG becomes mandatory). Inside the burst window the 6th POST must be
//! rejected with `429 TOO_MANY_REQUESTS` per the `presets::login()`
//! bucket (`burst_size: 5, per_second: 1.0, key_kind: Ip`).
//!
//! The test is `#[ignore]`d because it requires a live Postgres at
//! `VICEDEN_DATABASE__URL` with VICEDEN migrations applied — the same
//! contract as every other repository integration test in this crate.
//! Run with:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test route_rate_limit -- --ignored --test-threads=1
//! ```

mod common;

use std::{path::PathBuf, sync::Arc};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

use viceden::{app::build_router, config, models::Board, AppState};

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with viceden migrations applied; run with `cargo test --test route_rate_limit -- --ignored --test-threads=1`"]
async fn sixth_login_attempt_inside_window_returns_429() {
    let pool = common::shared_pool().await;

    let boards_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join("boards.toml");
    let boards = Board::load_from_file(&boards_path).expect("boards config should load");

    let state = AppState {
        db: viceden::RequiredPgPool::new(pool.clone()),
        boards,
        repos: viceden::repository::Repos::new(pool.clone()),
        login_throttle: viceden::cache::LoginThrottle::new(5, chrono::Duration::minutes(15)),
        mention_cache: viceden::cache::MentionSuggest::new(std::time::Duration::from_secs(60)),
        moderation: Some(Arc::new(
            viceden::services::moderation_pipeline::ModerationPipeline::new(
                pool.clone(),
                Arc::new(viceden::services::classifier::stub::StubClassifier::new()),
                viceden::services::moderation_pipeline::PipelineConfig {
                    sync_deadline: std::time::Duration::from_millis(1500),
                    default_model: "stub".into(),
                },
            ),
        )),
        config: config::Settings {
            server: config::ServerSettings {
                bind: "127.0.0.1:3000".to_string(),
                max_request_body_bytes: 131_072,
                request_timeout_seconds: 15,
                secure_cookies: false,
                proxy: Default::default(),
                rate_limit: Default::default(),
            },
            database: config::DatabaseSettings {
                url: "postgres://viceden:viceden@127.0.0.1:5432/viceden".to_string(),
                require_connection: false,
                max_connections: 1,
            },
            auth: config::AuthSettings::default(),
            classifier: Default::default(),
            ops: Default::default(),
            boards_file: boards_path.to_string_lossy().to_string(),
        },
    };

    let app = build_router(state);

    for n in 0..6 {
        let req = Request::builder()
            .method("POST")
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("handle=ghost&password=wrong&csrf=skip"))
            .unwrap();
        let response = app.clone().oneshot(req).await.expect("router should respond");
        if n < 5 {
            assert_ne!(
                response.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "request {n} should not be rate-limited yet"
            );
        } else {
            assert_eq!(
                response.status(),
                StatusCode::TOO_MANY_REQUESTS,
                "request {n} should be rate-limited"
            );
        }
    }
}
