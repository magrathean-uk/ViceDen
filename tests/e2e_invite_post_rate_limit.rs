//! End-to-end path: invite issuance -> registration -> moderated post -> 429.
//!
//! Run with `cargo test --test e2e_invite_post_rate_limit -- --ignored`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;
use uuid::Uuid;
use viceden::repository::invites::{InviteRepo, NewInvite};
use viceden::repository::users::{NewUser, UserRepo};
use viceden::services::classifier::stub::StubClassifier;

#[allow(dead_code)]
fn body_form(pairs: &[(&str, &str)]) -> Body {
    let encoded = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoded(k), urlencoded(v)))
        .collect::<Vec<_>>()
        .join("&");
    Body::from(encoded)
}

#[allow(dead_code)]
fn urlencoded(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn end_to_end_invite_to_moderated_post() {
    let mut state = common::app_state().await;
    let pool = state.db.as_ref().expect("test state has db").clone();

    // Seed a founder + an invite.
    let founder = Uuid::new_v4();
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: founder,
                handle: format!("e2e-f-{}", &founder.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "founder".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();
    let code = format!("vdn-inv-e2e-{}", &Uuid::new_v4().simple().to_string()[..8]);
    InviteRepo
        .issue(
            &pool,
            NewInvite {
                code: code.clone(),
                inviter_user_id: founder,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();

    state.moderation =
        Some(Arc::new(viceden::services::moderation_pipeline::ModerationPipeline::new(
            pool.clone(),
            Arc::new(StubClassifier::new()),
            viceden::services::moderation_pipeline::PipelineConfig {
                sync_deadline: Duration::from_millis(1500),
                default_model: "stub".into(),
            },
        )));
    let board_slug = state.boards.first().expect("board should exist").slug.clone();
    let app = viceden::app::build_router(state);

    // Step 1: GET /register?code=<code>, scrape csrf.
    let response = app
        .clone()
        .oneshot(Request::get(format!("/register?code={code}")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pre_auth_cookie = cookie_header_value(&response);
    let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap().to_string();
    let csrf = csrf_from(&html).expect("csrf in register page");

    // Step 2: POST /register
    let handle = format!("e2e-u-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let register = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/register")
                .method("POST")
                .header(header::COOKIE, &pre_auth_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body_form(&[
                    ("csrf", &csrf),
                    ("code", &code),
                    ("handle", &handle),
                    ("password", "secret123"),
                    ("dob", "1990-01-01"),
                    ("confirm_18", "yes"),
                    ("confirm_rules", "yes"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(register.status(), StatusCode::SEE_OTHER);
    let session_cookie = cookie_header_value(&register);

    // Probation has its own end-to-end coverage. Clear it here so this test
    // isolates moderation and reply-rate limiting after invite redemption.
    let registered_user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE handle = $1")
        .bind(&handle)
        .fetch_one(&pool)
        .await
        .expect("registered user exists");
    UserRepo
        .set_probation_until(&pool, registered_user_id, None)
        .await
        .expect("clear probation for moderation flow");

    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(profile.status(), StatusCode::OK);
    let body = to_bytes(profile.into_body(), 1_000_000).await.unwrap();
    let csrf = csrf_from(std::str::from_utf8(&body).unwrap()).expect("csrf in profile page");

    let create_thread = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/b/{board_slug}"))
                .method("POST")
                .header(header::COOKIE, &session_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body_form(&[
                    ("csrf", &csrf),
                    ("title", "e2e moderation thread"),
                    ("body", "moderated root body"),
                ]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_thread.status(), StatusCode::SEE_OTHER);
    let location =
        create_thread.headers().get(header::LOCATION).unwrap().to_str().unwrap().to_string();
    let thread_path = location.split('?').next().unwrap_or(&location).to_string();

    let mut last_status = StatusCode::OK;
    for index in 0..6 {
        let reply = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&thread_path)
                    .method("POST")
                    .header(header::COOKIE, &session_cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(body_form(&[
                        ("csrf", &csrf),
                        ("body", &format!("moderated reply {index}")),
                    ]))
                    .unwrap(),
            )
            .await
            .unwrap();
        last_status = reply.status();
        if index < 5 {
            assert_eq!(last_status, StatusCode::SEE_OTHER);
        }
    }

    assert_eq!(last_status, StatusCode::TOO_MANY_REQUESTS);
}

fn csrf_from(html: &str) -> Option<String> {
    html.split("name=\"csrf\"")
        .nth(1)
        .and_then(|s| s.split("value=\"").nth(1))
        .and_then(|s| s.split('"').next().map(str::to_string))
}

fn cookie_header_value(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .expect("cookie should be set")
        .to_str()
        .expect("cookie should be utf8")
        .split(';')
        .next()
        .expect("cookie should contain value")
        .to_string()
}
