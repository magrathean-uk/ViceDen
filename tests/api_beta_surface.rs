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

async fn text_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.expect("body should read");
    String::from_utf8(body.to_vec()).expect("body should be utf8")
}

#[tokio::test(flavor = "multi_thread")]
async fn me_and_pat_endpoints_require_token_and_work() {
    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "existing".to_string())
            .expect("pat should be issued")
            .token
    };
    let app = build_router(state);

    let anonymous_me = app
        .clone()
        .oneshot(Request::builder().uri("/api/me").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(anonymous_me.status(), StatusCode::UNAUTHORIZED);

    let anonymous_boards = app
        .clone()
        .oneshot(Request::builder().uri("/api/boards").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(anonymous_boards.status(), StatusCode::UNAUTHORIZED);

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(me.status(), StatusCode::OK);
    let me_json = json_body(me).await;
    assert_eq!(me_json["handle"], "root");

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/pats")
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"cli"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_json = json_body(created).await;
    let created_id = created_json["id"].as_str().expect("id should be a uuid string").to_string();
    assert_eq!(created_json["name"], "cli");
    assert!(created_json["token"].as_str().expect("token should exist").starts_with("vdn_"));

    let listed = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/pats")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_json = json_body(listed).await;
    assert_eq!(listed_json.as_array().expect("array response").len(), 2);

    let legacy_thread_create = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/threads")
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"board_slug":"general","title":"legacy route","body":"legacy body"}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(legacy_thread_create.status(), StatusCode::NOT_FOUND);

    let deleted = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/me/pats/{created_id}"))
                .method("DELETE")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(deleted.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_theme_is_dark_only_even_when_light_is_requested() {
    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "theme".to_string())
            .expect("pat should be issued")
            .token
    };
    let app = build_router(state);

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/settings")
                .method("PATCH")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"compactness":1,"theme":"light"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(update.status(), StatusCode::OK);
    let update_json = json_body(update).await;
    assert_eq!(update_json["ok"], true);

    let me = app
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(me.status(), StatusCode::OK);
    let me_json = json_body(me).await;
    assert_eq!(me_json["theme"], "dark");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_probation_state_is_visible_and_blocks_new_threads_and_replies() {
    use chrono::{Duration, Utc};

    let state = test_state().await;
    let (pat, thread_id, user_id) = {
        let mut store = common::compat_store(&state).await;
        let user_id = store.create_user("probapi", "secret123", None).expect("user").id;
        let pat = store.issue_named_api_token(user_id, "probapi".to_string()).expect("pat").token;
        let thread_id = store
            .create_thread_with_checks(
                "general",
                "api probation thread".to_string(),
                "root body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread")
            .id;
        (pat, thread_id, user_id)
    };
    sqlx::query("UPDATE users SET probation_until = $2 WHERE id = $1")
        .bind(user_id)
        .bind(Utc::now() + Duration::hours(24))
        .execute(state.db.as_ref().expect("db"))
        .await
        .expect("probation update");

    let app = build_router(state);
    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("x-api-token", &pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("me");
    assert_eq!(me.status(), StatusCode::OK);
    let me_json = json_body(me).await;
    assert_eq!(me_json["handle"], "probapi");
    assert!(me_json["probation_until"].is_string());

    let thread_attempt = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/boards/general/threads")
                .method("POST")
                .header("x-api-token", &pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"blocked","body":"blocked"}"#))
                .unwrap(),
        )
        .await
        .expect("thread attempt");
    assert_eq!(thread_attempt.status(), StatusCode::FORBIDDEN);
    let thread_json = json_body(thread_attempt).await;
    assert_eq!(thread_json["error"], "probation active");

    let reply_attempt = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/threads/{thread_id}/posts"))
                .method("POST")
                .header("x-api-token", &pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"blocked reply"}"#))
                .unwrap(),
        )
        .await
        .expect("reply attempt");
    assert_eq!(reply_attempt.status(), StatusCode::FORBIDDEN);
    let reply_json = json_body(reply_attempt).await;
    assert_eq!(reply_json["error"], "probation active");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_lift_probation_via_api_and_unlock_posting() {
    use chrono::{Duration, Utc};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let (founder_pat, user_pat, user_id) = {
        let mut store = common::compat_store(&state).await;
        let founder_pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder-lift".to_string())
            .expect("founder pat")
            .token;
        let user_id = store.create_user("probapilift", "secret123", None).expect("user").id;
        let user_pat = store
            .issue_named_api_token(user_id, "probapilift".to_string())
            .expect("user pat")
            .token;
        (founder_pat, user_pat, user_id)
    };
    sqlx::query("UPDATE users SET probation_until = $2 WHERE id = $1")
        .bind(user_id)
        .bind(Utc::now() + Duration::hours(24))
        .execute(&pool)
        .await
        .expect("probation update");

    let app = build_router(state);
    let blocked = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/boards/general/threads")
                .method("POST")
                .header("x-api-token", &user_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"blocked-before-lift","body":"blocked"}"#))
                .unwrap(),
        )
        .await
        .expect("blocked before lift");
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    let lift = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/users/{user_id}/lift-probation"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"note":"manual clear"}"#))
                .unwrap(),
        )
        .await
        .expect("lift");
    assert_eq!(lift.status(), StatusCode::OK);
    let lift_json = json_body(lift).await;
    assert_eq!(lift_json["ok"], true);

    let probation_until: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT probation_until FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .expect("probation row");
    assert!(probation_until.is_none());
    let action: String = sqlx::query_scalar(
        "SELECT action FROM moderation_events WHERE target_user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("audit action");
    assert_eq!(action, "lift_probation");

    let users = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/users")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("admin users");
    assert_eq!(users.status(), StatusCode::OK);
    let users_json = json_body(users).await;
    let lifted = users_json
        .as_array()
        .expect("array")
        .iter()
        .find(|item| item["handle"] == "probapilift")
        .expect("lifted user present");
    assert!(lifted["probation_until"].is_null());

    let unblocked = app
        .oneshot(
            Request::builder()
                .uri("/api/boards/general/threads")
                .method("POST")
                .header("x-api-token", &user_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"open-after-lift","body":"allowed"}"#))
                .unwrap(),
        )
        .await
        .expect("unblocked after lift");
    assert_eq!(unblocked.status(), StatusCode::CREATED);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_create_thread_blocks_daily_thread_limit() {
    use chrono::Utc;
    use viceden::repository::{
        posts::{ModState, NewPost, PostRepo},
        threads::{NewThread, ThreadRepo},
    };

    let state = test_state().await;
    let (pat, user_id) = {
        let mut store = common::compat_store(&state).await;
        let user_id = store.create_user("threadcapapi", "secret123", None).expect("user").id;
        let pat =
            store.issue_named_api_token(user_id, "threadcapapi".to_string()).expect("pat").token;
        (pat, user_id)
    };
    let pool = state.db.as_ref().expect("db").clone();
    for index in 0..5 {
        let thread_id = Uuid::new_v4();
        ThreadRepo
            .insert(
                &pool,
                NewThread {
                    id: thread_id,
                    board_slug: "general".into(),
                    user_id,
                    title: format!("api thread cap {index}"),
                    expires_at: Utc::now() + chrono::Duration::days(30),
                },
            )
            .await
            .expect("thread");
        PostRepo
            .insert(
                &pool,
                NewPost {
                    id: Uuid::new_v4(),
                    thread_id,
                    user_id,
                    body: format!("root body {index}"),
                    mod_state: ModState::Visible,
                },
            )
            .await
            .expect("root post");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/boards/general/threads")
                .method("POST")
                .header("x-api-token", &pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"blocked-thread","body":"blocked-body"}"#))
                .unwrap(),
        )
        .await
        .expect("thread create");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let json = json_body(response).await;
    assert_eq!(json["error"], "thread limit reached");
}

#[tokio::test(flavor = "multi_thread")]
async fn api_create_post_blocks_daily_reply_limit() {
    use viceden::repository::posts::{ModState, NewPost, PostRepo};

    let state = test_state().await;
    let (pat, user_id, thread_id) = {
        let mut store = common::compat_store(&state).await;
        let user_id = store.create_user("replycapapi", "secret123", None).expect("user").id;
        let pat =
            store.issue_named_api_token(user_id, "replycapapi".to_string()).expect("pat").token;
        let thread_id = store
            .create_thread_with_checks(
                "general",
                "api reply cap root".to_string(),
                "root body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread")
            .id;
        (pat, user_id, thread_id)
    };
    let pool = state.db.as_ref().expect("db").clone();
    for index in 0..50 {
        PostRepo
            .insert(
                &pool,
                NewPost {
                    id: Uuid::new_v4(),
                    thread_id,
                    user_id,
                    body: format!("reply body {index}"),
                    mod_state: ModState::Visible,
                },
            )
            .await
            .expect("reply");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/threads/{thread_id}/posts"))
                .method("POST")
                .header("x-api-token", &pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"blocked-reply"}"#))
                .unwrap(),
        )
        .await
        .expect("reply create");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let json = json_body(response).await;
    assert_eq!(json["error"], "reply limit reached");
}

#[tokio::test(flavor = "multi_thread")]
async fn notification_api_returns_exact_target_url_for_post_notifications() {
    use chrono::Utc;
    use viceden::repository::notifications::{NewNotification, NotificationRepo};

    let state = test_state().await;
    let (founder_pat, thread_id, post_id, thread_title, notification_id) = {
        let mut store = common::compat_store(&state).await;
        let thread = store
            .create_thread_with_checks(
                "general",
                "api notify anchor".to_string(),
                "api body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "api reply".to_string(), forum::ROOT_USER_ID)
            .expect("post");
        let pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "notify-api".to_string())
            .expect("pat")
            .token;
        (pat, thread.id, post.id, thread.title, Uuid::new_v4())
    };
    NotificationRepo
        .insert(
            state.db.as_ref().expect("db"),
            NewNotification {
                id: notification_id,
                recipient_user_id: forum::ROOT_USER_ID,
                actor_id: Some(forum::ROOT_USER_ID),
                kind: "mention".to_string(),
                thread_id: Some(thread_id),
                thread_title,
                post_id: Some(post_id),
                message: "api mention".to_string(),
                created_at: Utc::now(),
                is_read: false,
            },
        )
        .await
        .expect("notification");

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/me/notifications")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("notifications");
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let row = json
        .as_array()
        .expect("array")
        .iter()
        .find(|item| item["id"] == notification_id.to_string())
        .expect("notification row");
    assert_eq!(row["target_url"], format!("/t/{thread_id}#post-{post_id}"));
    assert_eq!(row["target_label"], "post");
}

#[tokio::test(flavor = "multi_thread")]
async fn notification_api_returns_richer_reply_message_for_thread_author() {
    let state = test_state().await;
    let (owner_pat, thread_id, post_id) = {
        let mut store = common::compat_store(&state).await;
        let owner = store.create_user("apireplyowner", "secret123", None).expect("owner");
        let replier = store.create_user("apireplyactor", "secret123", None).expect("replier");
        let thread = store
            .create_thread_with_checks(
                "general",
                "api reply thread".to_string(),
                "api reply body".to_string(),
                owner.id,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "reply body".to_string(), replier.id)
            .expect("reply");
        let pat =
            store.issue_named_api_token(owner.id, "reply-owner".to_string()).expect("pat").token;
        (pat, thread.id, post.id)
    };

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/me/notifications")
                .header("x-api-token", &owner_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("notifications");
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let row = json
        .as_array()
        .expect("array")
        .iter()
        .find(|item| item["post_id"] == post_id.to_string())
        .expect("reply row");
    assert_eq!(row["thread_id"], thread_id.to_string());
    assert_eq!(row["message"], "apireplyactor replied to your thread 'api reply thread'.");
    assert_eq!(row["target_url"], format!("/t/{thread_id}#post-{post_id}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_settings_api_round_trips_notification_preferences() {
    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "prefs".to_string())
            .expect("pat should be issued")
            .token
    };
    let app = build_router(state);

    let update = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/settings")
                .method("PATCH")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"notify_mentions":false,"notify_replies":false,"notify_moderation":true}"#,
                ))
                .unwrap(),
        )
        .await
        .expect("settings update");
    assert_eq!(update.status(), StatusCode::OK);

    let me = app
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("me");
    assert_eq!(me.status(), StatusCode::OK);
    let me_json = json_body(me).await;
    assert_eq!(me_json["notify_mentions"], false);
    assert_eq!(me_json["notify_replies"], false);
    assert_eq!(me_json["notify_moderation"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_api_returns_result_url_for_post_hits() {
    let state = test_state().await;
    let unique = format!("api-search-url-{}", Uuid::new_v4().simple());
    let (founder_pat, thread_id, post_id) = {
        let mut store = common::compat_store(&state).await;
        let thread = store
            .create_thread_with_tags(
                "general",
                "search api target".to_string(),
                "thread body".to_string(),
                forum::ROOT_USER_ID,
                vec!["safety".to_string()],
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, unique.clone(), forum::ROOT_USER_ID)
            .expect("post");
        let pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "search-url".to_string())
            .expect("pat")
            .token;
        (pat, thread.id, post.id)
    };
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/search?q={unique}"))
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("search");
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["query"], unique);
    assert_eq!(json["scope"], "all");
    let row = json["results"]
        .as_array()
        .expect("array")
        .iter()
        .find(|item| item["post_id"] == post_id.to_string())
        .expect("post hit");
    assert_eq!(row["thread_id"], thread_id.to_string());
    assert_eq!(row["kind"], "post");
    assert_eq!(row["url"], format!("/t/{thread_id}#post-{post_id}"));
    assert_eq!(row["board_name"], "General");
    assert_eq!(row["tags"][0], "safety");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_api_returns_filter_metadata() {
    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "search-filters".to_string())
            .expect("pat")
            .token
    };
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/search?q=tag:safety%20author:root%20:%20/general&scope=posts")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("search");
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["scope"], "posts");
    let filters = json["filters"].as_array().expect("filters");
    assert!(filters.iter().any(|item| item["label"] == "tag" && item["value"] == "#safety"));
    assert!(filters.iter().any(|item| item["label"] == "author" && item["value"] == "@root"));
    assert!(filters.iter().any(|item| item["label"] == "board" && item["value"] == "/general"));
    assert!(filters.iter().any(|item| item["label"] == "scope" && item["value"] == "posts only"));
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_and_hidden_api_list_thread_manager_items() {
    let state = test_state().await;
    let (founder_pat, saved_thread_id, hidden_thread_id) = {
        let mut store = common::compat_store(&state).await;
        let saved_thread = store
            .create_thread_with_checks(
                "general",
                "saved api list".to_string(),
                "saved body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("saved thread");
        let hidden_thread = store
            .create_thread_with_checks(
                "general",
                "hidden api list".to_string(),
                "hidden body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("hidden thread");
        store.toggle_thread_saved(forum::ROOT_USER_ID, saved_thread.id).expect("save");
        store.toggle_thread_hidden(forum::ROOT_USER_ID, hidden_thread.id).expect("hide");
        let pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "saved-hidden-list".to_string())
            .expect("pat")
            .token;
        (pat, saved_thread.id, hidden_thread.id)
    };
    let app = build_router(state);

    let saved = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/saved")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("saved list");
    assert_eq!(saved.status(), StatusCode::OK);
    let saved_json = json_body(saved).await;
    let saved_row = saved_json
        .as_array()
        .expect("saved array")
        .iter()
        .find(|item| item["thread_id"] == saved_thread_id.to_string())
        .expect("saved row");
    assert_eq!(saved_row["url"], format!("/t/{saved_thread_id}"));

    let hidden = app
        .oneshot(
            Request::builder()
                .uri("/api/me/hidden")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("hidden list");
    assert_eq!(hidden.status(), StatusCode::OK);
    let hidden_json = json_body(hidden).await;
    let hidden_row = hidden_json
        .as_array()
        .expect("hidden array")
        .iter()
        .find(|item| item["thread_id"] == hidden_thread_id.to_string())
        .expect("hidden row");
    assert_eq!(hidden_row["url"], format!("/t/{hidden_thread_id}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn next_feature_api_surface_handles_state_edits_invites_mutes_metrics_and_search_filters() {
    let state = test_state().await;
    let (founder_pat, thread_id, post_id, member_id) = {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("surfaceuser", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_tags(
                "general",
                "surface tagged".to_string(),
                "first body".to_string(),
                forum::ROOT_USER_ID,
                vec!["safety".to_string()],
            )
            .expect("thread");
        let (_, posts) = store.thread_with_posts(&thread.id).expect("posts");
        let pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "next-surface".to_string())
            .expect("pat")
            .token;
        (pat, thread.id, posts[0].id, member.id)
    };
    let app = build_router(state);

    let state_update = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/threads/{thread_id}/state"))
                .method("PATCH")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"locked":true,"pinned":true,"answered":true}"#))
                .unwrap(),
        )
        .await
        .expect("state update");
    assert_eq!(state_update.status(), StatusCode::OK);

    let edit = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/posts/{post_id}"))
                .method("PATCH")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"body":"edited first body"}"#))
                .unwrap(),
        )
        .await
        .expect("edit");
    assert_eq!(edit.status(), StatusCode::OK);

    let mute = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/me/mutes/{member_id}"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("mute");
    assert_eq!(mute.status(), StatusCode::OK);

    let follow_tag = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/tags/safety/subscription")
                .method("POST")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("follow tag");
    assert_eq!(follow_tag.status(), StatusCode::OK);

    let unfollow_tag = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/me/tags/safety/subscription")
                .method("DELETE")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("unfollow tag");
    assert_eq!(unfollow_tag.status(), StatusCode::OK);

    let invite = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/invites")
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"label":"surface"}"#))
                .unwrap(),
        )
        .await
        .expect("invite");
    assert_eq!(invite.status(), StatusCode::CREATED);
    assert!(json_body(invite).await["code"].as_str().unwrap_or_default().starts_with("vdni-"));

    let invite_list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/invites")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invite list");
    assert_eq!(invite_list.status(), StatusCode::OK);
    let invites_json = json_body(invite_list).await;
    assert_eq!(invites_json[0]["impact_summary"], "Unused. Revoke blocks future use only.");
    assert_eq!(invites_json[0]["risk_label"], "low");
    assert_eq!(invites_json[0]["risk_reason"], "Fresh single-use code.");
    assert_eq!(invites_json[0]["redeemed_by"].as_array().map(|items| items.len()), Some(0));

    let metrics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/metrics")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("metrics");
    assert_eq!(metrics.status(), StatusCode::OK);

    let search = app
        .oneshot(
            Request::builder()
                .uri("/api/search?q=tag:safety%20author:root")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("search");
    assert_eq!(search.status(), StatusCode::OK);
    let search_json = json_body(search).await;
    assert!(search_json["results"].as_array().is_some());
    assert!(search_json["filters"].as_array().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_admin_and_report_endpoints_are_locked_down() {
    let state = test_state().await;
    let (member_pat, founder_pat, thread_id) = {
        let mut store = common::compat_store(&state).await;
        let user =
            store.create_user("reporterx", "secret123", None).expect("user should be created");
        let member_pat = store
            .issue_named_api_token(user.id, "member".to_string())
            .expect("member pat should be issued")
            .token;
        let founder_pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token;
        let thread_id =
            store.all_threads().into_iter().next().expect("seeded thread should exist").id;
        (member_pat, founder_pat, thread_id)
    };
    let app = build_router(state);

    let report = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/threads/{thread_id}/report"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason":"needs review","details":"api test"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(report.status(), StatusCode::CREATED);
    let report_json = json_body(report).await;
    let report_id = report_json["id"].as_str().expect("report id should exist").to_string();

    let member_admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports")
                .header("x-api-token", &member_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    // Member is authenticated (the PAT resolves) but lacks the founder
    // role, so `require_founder_api_user` returns FORBIDDEN. UNAUTHORIZED
    // would mean "no token / unknown token" -- a different semantic.
    assert_eq!(member_admin.status(), StatusCode::FORBIDDEN);

    let founder_admin = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(founder_admin.status(), StatusCode::OK);
    let reports_json = json_body(founder_admin).await;
    assert_eq!(reports_json[0]["reason"], "needs review");
    assert_eq!(reports_json[0]["priority_label"], "normal");
    assert_eq!(reports_json[0]["target_kind"], "thread");
    assert_eq!(reports_json[0]["queue_state"], "new");
    assert_eq!(reports_json[0]["audit_history"].as_array().map(|items| items.len()), Some(0));

    let triage = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/reports/{report_id}/triage"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"queue_state":"reviewing","claim":true,"note":"desk owner"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(triage.status(), StatusCode::OK);

    let moderate = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/moderation/thread/{thread_id}"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"action":"remove","reason":"spam","note":"explicit note"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(moderate.status(), StatusCode::OK);

    let founder_admin_after = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(founder_admin_after.status(), StatusCode::OK);
    let reports_after_json = json_body(founder_admin_after).await;
    assert_eq!(reports_after_json[0]["queue_state"], "reviewing");
    assert_eq!(reports_after_json[0]["assigned_to"], "root");
    let audit_history = reports_after_json[0]["audit_history"].as_array().expect("audit array");
    assert_eq!(audit_history.len(), 2);
    assert_eq!(audit_history[0]["action"], "remove_thread");
    assert_eq!(audit_history[0]["note"], "explicit note");
    assert_eq!(audit_history[1]["action"], "claim_report");
    assert_eq!(audit_history[1]["note"], "desk owner");

    let resolve = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/reports/{report_id}"))
                .method("PATCH")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"resolved":true}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(resolve.status(), StatusCode::OK);

    let founder_users = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/users")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(founder_users.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_filter_admin_reports_via_api() {
    let state = test_state().await;
    let (founder_pat, thread_a, thread_b) = {
        let mut store = common::compat_store(&state).await;
        let founder_pat = store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token;
        let member =
            store.create_user("reportfilter", "secret123", None).expect("user should exist");
        let thread_a = store
            .create_thread_with_checks(
                "general",
                "api filter open".to_string(),
                "api filter open body".to_string(),
                member.id,
            )
            .expect("thread a")
            .id;
        let thread_b = store
            .create_thread_with_checks(
                "general",
                "api filter resolved".to_string(),
                "api filter resolved body".to_string(),
                member.id,
            )
            .expect("thread b")
            .id;
        (founder_pat, thread_a, thread_b)
    };
    let app = build_router(state);

    let report_a = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/threads/{thread_a}/report"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason":"open filter","details":"keep open"}"#))
                .unwrap(),
        )
        .await
        .expect("report a");
    assert_eq!(report_a.status(), StatusCode::CREATED);
    let report_a_id = json_body(report_a).await["id"].as_str().expect("report a id").to_string();

    let report_b = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/threads/{thread_b}/report"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason":"resolved filter","details":"history"}"#))
                .unwrap(),
        )
        .await
        .expect("report b");
    assert_eq!(report_b.status(), StatusCode::CREATED);
    let report_b_id = json_body(report_b).await["id"].as_str().expect("report b id").to_string();

    let triage = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/reports/{report_a_id}/triage"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"queue_state":"reviewing","claim":true}"#))
                .unwrap(),
        )
        .await
        .expect("triage");
    assert_eq!(triage.status(), StatusCode::OK);

    let resolve = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/reports/{report_b_id}"))
                .method("PATCH")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"resolved":true,"note":"history note"}"#))
                .unwrap(),
        )
        .await
        .expect("resolve");
    assert_eq!(resolve.status(), StatusCode::OK);

    let open_filtered = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports?assigned=me&queue_state=reviewing")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("open filtered");
    assert_eq!(open_filtered.status(), StatusCode::OK);
    let open_filtered_json = json_body(open_filtered).await;
    assert_eq!(open_filtered_json.as_array().map(|items| items.len()), Some(1));
    assert_eq!(open_filtered_json[0]["reason"], "open filter");
    assert_eq!(open_filtered_json[0]["queue_state"], "reviewing");
    assert_eq!(open_filtered_json[0]["assigned_to"], "root");

    let history = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports?status=resolved")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("history");
    assert_eq!(history.status(), StatusCode::OK);
    let history_json = json_body(history).await;
    assert_eq!(history_json.as_array().map(|items| items.len()), Some(1));
    assert_eq!(history_json[0]["reason"], "resolved filter");
    assert_eq!(history_json[0]["resolution_note"], "history note");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_filter_admin_reports_by_audit_action_and_moderator() {
    use chrono::Utc;
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("reportaudit", "secret123", None).expect("user");
        let thread_a = store
            .create_thread_with_checks(
                "general",
                "api action a".to_string(),
                "api action a body".to_string(),
                member.id,
            )
            .expect("thread a");
        let thread_b = store
            .create_thread_with_checks(
                "general",
                "api action b".to_string(),
                "api action b body".to_string(),
                member.id,
            )
            .expect("thread b");
        let pool = state.db.as_ref().expect("db").clone();

        let report_a = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: report_a,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread_a.id),
                    reason: "api action keep".into(),
                    details: Some("match remove".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("report a");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread_a.id),
                    target_user_id: Some(member.id),
                    report_id: Some(report_a),
                    action: "remove_thread".into(),
                    reason: "spam".into(),
                    note: Some("match".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("event a");

        let report_b = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: report_b,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread_b.id),
                    reason: "api action skip".into(),
                    details: Some("skip claim".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("report b");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread_b.id),
                    target_user_id: Some(member.id),
                    report_id: Some(report_b),
                    action: "claim_report".into(),
                    reason: "desk".into(),
                    note: Some("skip".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("event b");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports?action=remove_thread&moderator=root")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("filtered reports");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json.as_array().map(|items| items.len()), Some(1));
    assert_eq!(json[0]["reason"], "api action keep");
    assert_eq!(json[0]["audit_history"][0]["action"], "remove_thread");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_filter_admin_reports_by_audit_date_window() {
    use chrono::{TimeZone, Utc};
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "report-date-filter".to_string())
            .expect("founder pat should be issued")
            .token
    };
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("reportdateapi", "secret123", None).expect("user");
        let older_thread = store
            .create_thread_with_checks(
                "general",
                "api date old".to_string(),
                "api date old body".to_string(),
                member.id,
            )
            .expect("old thread");
        let newer_thread = store
            .create_thread_with_checks(
                "general",
                "api date new".to_string(),
                "api date new body".to_string(),
                member.id,
            )
            .expect("new thread");
        let pool = state.db.as_ref().expect("db").clone();

        let older_report = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: older_report,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(older_thread.id),
                    reason: "api date old".into(),
                    details: Some("skip old".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 10, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("older report");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(older_thread.id),
                    target_user_id: Some(member.id),
                    report_id: Some(older_report),
                    action: "remove_thread".into(),
                    reason: "old".into(),
                    note: Some("outside".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 11, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("older event");

        let newer_report = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: newer_report,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(newer_thread.id),
                    reason: "api date new".into(),
                    details: Some("keep new".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 10, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("newer report");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(newer_thread.id),
                    target_user_id: Some(member.id),
                    report_id: Some(newer_report),
                    action: "remove_thread".into(),
                    reason: "new".into(),
                    note: Some("inside".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 11, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("newer event");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/reports?action=remove_thread&moderator=root&since=2026-05-15&until=2026-05-31")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("filtered reports");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json.as_array().map(|items| items.len()), Some(1));
    assert_eq!(json[0]["reason"], "api date new");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_export_moderation_events_with_filters() {
    use chrono::{TimeZone, Utc};
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "audit-export".to_string())
            .expect("founder pat should be issued")
            .token
    };
    let report_id = {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("auditexport", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_checks(
                "general",
                "audit export".to_string(),
                "audit export body".to_string(),
                member.id,
            )
            .expect("thread");
        let pool = state.db.as_ref().expect("db").clone();
        let match_report_id = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: match_report_id,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread.id),
                    reason: "audit export report".into(),
                    details: Some("keep".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 8, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("report a");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread.id),
                    target_user_id: Some(member.id),
                    report_id: Some(match_report_id),
                    action: "remove_thread".into(),
                    reason: "audit match".into(),
                    note: Some("keep".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 8, 30, 0).single().unwrap(),
                },
            )
            .await
            .expect("event a");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread.id),
                    target_user_id: Some(member.id),
                    report_id: None,
                    action: "claim_report".into(),
                    reason: "audit skip".into(),
                    note: Some("skip".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 8, 30, 0).single().unwrap(),
                },
            )
            .await
            .expect("event b");
        match_report_id
    };

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/admin/moderation-events?action=remove_thread&moderator=root&report_id={report_id}&since=2026-05-15&until=2026-05-31"
                ))
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("audit export");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json.as_array().map(|items| items.len()), Some(1));
    assert_eq!(json[0]["action"], "remove_thread");
    assert_eq!(json[0]["reason"], "audit match");
    assert_eq!(json[0]["actor"], "root");
    assert_eq!(json[0]["report_id"], report_id.to_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_export_moderation_events_as_csv() {
    use chrono::Utc;
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "audit-csv".to_string())
            .expect("founder pat should be issued")
            .token
    };
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("auditcsv", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_checks(
                "general",
                "audit csv".to_string(),
                "audit csv body".to_string(),
                member.id,
            )
            .expect("thread");
        let pool = state.db.as_ref().expect("db").clone();
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread.id),
                    target_user_id: Some(member.id),
                    report_id: None,
                    action: "remove_thread".into(),
                    reason: "csv match".into(),
                    note: Some("csv note".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("event");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/moderation-events?action=remove_thread&format=csv")
                .header("x-api-token", &founder_pat)
                .header(header::ACCEPT, "text/csv")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("audit csv");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/csv; charset=utf-8"
    );
    let csv = text_body(response).await;
    assert!(csv.starts_with(
        "id,created_at,actor,actor_id,action,reason,note,report_id,target_user_id,target_thread_id,target_post_id\n"
    ));
    assert!(csv.contains("remove_thread"));
    assert!(csv.contains("csv match"));
    assert!(csv.contains("root"));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_admin_metrics_reports_backup_status() {
    use std::fs;
    use viceden::config::ProxySettings;

    let mut state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "metrics-backup".to_string())
            .expect("founder pat should be issued")
            .token
    };
    let backup_dir =
        std::env::temp_dir().join(format!("viceden-metrics-backup-{}", Uuid::new_v4()));
    fs::create_dir_all(&backup_dir).expect("backup dir");
    let backup_file = backup_dir.join("viceden-20260531.dump.gz");
    fs::write(&backup_file, b"backup").expect("backup file");
    fs::write(backup_dir.join("viceden-20260531.dump.gz.verified"), b"verified")
        .expect("marker file");
    state.config.ops.backup_dir = Some(backup_dir.to_string_lossy().to_string());
    state.config.server.secure_cookies = true;
    state.config.server.proxy = ProxySettings::default();
    state.config.server.rate_limit.disabled = false;

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/metrics")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("metrics");
    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["backup"]["status"], "ok");
    assert_eq!(json["backup"]["latest_backup_file"], "viceden-20260531.dump.gz");
    assert_eq!(json["backup"]["latest_verified_file"], "viceden-20260531.dump.gz");
    assert_eq!(json["backup"]["backup_dir"], backup_dir.to_string_lossy().to_string());
    assert_eq!(json["ingress"]["status"], "ok");
    assert_eq!(json["ingress"]["secure_cookies"], true);
    assert_eq!(json["ingress"]["rate_limit_disabled"], false);
    assert_eq!(json["ingress"]["trust_mode"], "loopback_only");
    assert_eq!(json["ingress"]["client_ip_headers"][0], "Cf-Connecting-Ip");

    let _ = fs::remove_dir_all(backup_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_cauterise_user_subtree_via_api() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};
    use viceden::repository::posts::{ModState, NewPost, PostRepo};
    use viceden::repository::threads::{NewThread, ThreadRepo};
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    let invited_a = Uuid::new_v4();
    let invited_b = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    let post_id = Uuid::new_v4();
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: invited_a,
                handle: format!("api-a-{}", &invited_a.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .expect("user a");
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: invited_b,
                handle: format!("api-b-{}", &invited_b.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .expect("user b");
    InviteRepo
        .issue(
            &pool,
            NewInvite {
                code: "vdn-inv-root-api-caut".into(),
                inviter_user_id: forum::ROOT_USER_ID,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .expect("issue root invite");
    let mut conn = pool.acquire().await.expect("conn");
    InviteRepo
        .redeem(
            &mut conn,
            RedeemInvite {
                code: "vdn-inv-root-api-caut".into(),
                user_id: invited_a,
                redeemed_from_ip: None,
            },
        )
        .await
        .expect("redeem root invite");
    InviteRepo
        .issue(
            &pool,
            NewInvite {
                code: "vdn-inv-a-api-caut".into(),
                inviter_user_id: invited_a,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .expect("issue child invite");
    let mut conn = pool.acquire().await.expect("conn");
    InviteRepo
        .redeem(
            &mut conn,
            RedeemInvite {
                code: "vdn-inv-a-api-caut".into(),
                user_id: invited_b,
                redeemed_from_ip: None,
            },
        )
        .await
        .expect("redeem child invite");
    InviteRepo
        .issue(
            &pool,
            NewInvite {
                code: "vdn-inv-b-api-unused".into(),
                inviter_user_id: invited_b,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .expect("issue unused invite");
    ThreadRepo
        .insert(
            &pool,
            NewThread {
                id: thread_id,
                board_slug: "general".into(),
                user_id: invited_a,
                title: "api cauterise thread".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::days(7),
            },
        )
        .await
        .expect("thread");
    PostRepo
        .insert(
            &pool,
            NewPost {
                id: post_id,
                thread_id,
                user_id: invited_b,
                body: "api cauterise post".into(),
                mod_state: ModState::Visible,
            },
        )
        .await
        .expect("post");

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/users/{invited_a}/cauterise"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"reason":"api-spam-wave"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["ok"], true);
    assert_eq!(json["subtree_users"], 2);
    assert_eq!(json["descendants"], 1);
    assert_eq!(json["threads_removed"], 1);
    assert_eq!(json["posts_removed"], 1);
    assert_eq!(json["invites_revoked"], 1);

    let deactivated: bool =
        sqlx::query_scalar("SELECT deactivated_at IS NOT NULL FROM users WHERE id = $1")
            .bind(invited_b)
            .fetch_one(&pool)
            .await
            .expect("user b row");
    let invite_revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM invites WHERE code = 'vdn-inv-b-api-unused'",
    )
    .fetch_one(&pool)
    .await
    .expect("invite row");
    let action: String = sqlx::query_scalar(
        "SELECT action FROM moderation_events WHERE target_user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(invited_a)
    .fetch_one(&pool)
    .await
    .expect("audit row");

    assert!(deactivated);
    assert!(invite_revoked);
    assert_eq!(action, "cauterise_subtree");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_wipe_user_via_api_and_audit_it() {
    use viceden::repository::posts::{ModState, NewPost, PostRepo};
    use viceden::repository::threads::{NewThread, ThreadRepo};
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    let target_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    let post_id = Uuid::new_v4();
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: target_id,
                handle: format!("aw-{}", &target_id.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .expect("user");
    ThreadRepo
        .insert(
            &pool,
            NewThread {
                id: thread_id,
                board_slug: "general".into(),
                user_id: target_id,
                title: "api wipe thread".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::days(7),
            },
        )
        .await
        .expect("thread");
    PostRepo
        .insert(
            &pool,
            NewPost {
                id: post_id,
                thread_id,
                user_id: target_id,
                body: "api wipe post".into(),
                mod_state: ModState::Visible,
            },
        )
        .await
        .expect("post");

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/users/{target_id}/wipe"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("wipe should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["ok"], true);

    let action: String = sqlx::query_scalar(
        "SELECT action FROM moderation_events WHERE target_user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(target_id)
    .fetch_one(&pool)
    .await
    .expect("audit row");
    assert_eq!(action, "wipe_user");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_delete_user_via_api_and_audit_it() {
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    let target_id = Uuid::new_v4();
    let handle = format!("ad-{}", &target_id.simple().to_string()[..8]);
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: target_id,
                handle: handle.clone(),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: Some("127.0.0.10".into()),
            },
        )
        .await
        .expect("user");

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/admin/users/{target_id}/delete"))
                .method("POST")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("delete should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    assert_eq!(json["ok"], true);

    let deleted: bool = sqlx::query_scalar("SELECT COUNT(*) = 0 FROM users WHERE id = $1")
        .bind(target_id)
        .fetch_one(&pool)
        .await
        .expect("user deleted");
    let note: String = sqlx::query_scalar(
        "SELECT note FROM moderation_events WHERE action = 'delete_user' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("audit note");

    assert!(deleted);
    assert!(note.contains(&handle));
    assert!(note.contains(&target_id.to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_invite_api_blocks_when_active_limit_reached() {
    use viceden::repository::invites::{InviteRepo, NewInvite};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    {
        let pool = state.db.as_ref().expect("db").clone();
        for idx in 0..5 {
            InviteRepo
                .issue(
                    &pool,
                    NewInvite {
                        code: format!("vdni-limit-{idx}"),
                        inviter_user_id: forum::ROOT_USER_ID,
                        max_redemptions: 1,
                        expires_at: None,
                        note: None,
                    },
                )
                .await
                .expect("invite issue");
        }
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/invites")
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"label":"overflow"}"#))
                .unwrap(),
        )
        .await
        .expect("issue should respond");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = json_body(response).await;
    assert_eq!(json["error"], "active invite limit reached");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_invite_api_marks_redeemed_tree_as_high_risk() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    {
        let mut store = common::compat_store(&state).await;
        let invited_a = store.create_user("apihriska", "secret123", None).expect("a").id;
        let invited_b = store.create_user("apihriskb", "secret123", None).expect("b").id;
        let invited_c = store.create_user("apihriskc", "secret123", None).expect("c").id;
        let pool = state.db.as_ref().expect("db").clone();

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-api-high-root".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 5,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue root");
        let mut tx = pool.begin().await.expect("tx a");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-api-high-root".into(),
                    user_id: invited_a,
                    redeemed_from_ip: Some("127.0.0.1".into()),
                },
            )
            .await
            .expect("redeem a");
        tx.commit().await.expect("commit a");

        let mut tx = pool.begin().await.expect("tx b");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-api-high-root".into(),
                    user_id: invited_b,
                    redeemed_from_ip: Some("127.0.0.2".into()),
                },
            )
            .await
            .expect("redeem b");
        tx.commit().await.expect("commit b");

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-api-high-child".into(),
                    inviter_user_id: invited_a,
                    max_redemptions: 1,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue child");
        let mut tx = pool.begin().await.expect("tx c");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-api-high-child".into(),
                    user_id: invited_c,
                    redeemed_from_ip: Some("127.0.0.3".into()),
                },
            )
            .await
            .expect("redeem c");
        tx.commit().await.expect("commit c");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/invites")
                .header("x-api-token", &founder_pat)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invite list");

    assert_eq!(response.status(), StatusCode::OK);
    let json = json_body(response).await;
    let row = json
        .as_array()
        .expect("array")
        .iter()
        .find(|item| item["code"] == "vdn-inv-api-high-root")
        .expect("high risk row");
    assert_eq!(row["risk_label"], "high");
    assert_eq!(
        row["risk_reason"],
        "Code already redeemed 2 times and now reaches 1 downstream account(s)."
    );
    assert_eq!(row["downstream_count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_invite_api_blocks_when_high_risk_code_stays_active() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};

    let state = test_state().await;
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store
            .issue_named_api_token(forum::ROOT_USER_ID, "founder".to_string())
            .expect("founder pat should be issued")
            .token
    };
    {
        let mut store = common::compat_store(&state).await;
        let invited_a = store.create_user("apiblocka", "secret123", None).expect("a").id;
        let invited_b = store.create_user("apiblockb", "secret123", None).expect("b").id;
        let invited_c = store.create_user("apiblockc", "secret123", None).expect("c").id;
        let pool = state.db.as_ref().expect("db").clone();

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-api-block-root".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 5,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue root");
        let mut tx = pool.begin().await.expect("tx a");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-api-block-root".into(),
                    user_id: invited_a,
                    redeemed_from_ip: Some("127.0.0.1".into()),
                },
            )
            .await
            .expect("redeem a");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-api-block-root".into(),
                    user_id: invited_b,
                    redeemed_from_ip: Some("127.0.0.2".into()),
                },
            )
            .await
            .expect("redeem b");
        tx.commit().await.expect("commit a");

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-api-block-child".into(),
                    inviter_user_id: invited_a,
                    max_redemptions: 1,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue child");
        let mut tx = pool.begin().await.expect("tx c");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-api-block-child".into(),
                    user_id: invited_c,
                    redeemed_from_ip: Some("127.0.0.3".into()),
                },
            )
            .await
            .expect("redeem c");
        tx.commit().await.expect("commit c");
    }

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/invites")
                .method("POST")
                .header("x-api-token", &founder_pat)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"label":"blocked"}"#))
                .unwrap(),
        )
        .await
        .expect("issue should respond");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let json = json_body(response).await;
    assert_eq!(json["error"], "high-risk invite still active");
}
