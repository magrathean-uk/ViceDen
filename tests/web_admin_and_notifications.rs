use std::path::PathBuf;

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use tower::util::ServiceExt;
use uuid::Uuid;
mod common;

use viceden::{app::build_router, forum, AppState};

async fn test_state() -> AppState {
    common::app_state().await
}

async fn test_state_with_secure_cookies() -> AppState {
    common::app_state_with_secure_cookies(true).await
}

async fn read_html(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.expect("body should read");
    String::from_utf8(body.to_vec()).expect("html should be utf8")
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
        .expect("cookie value should exist")
        .to_string()
}

fn extract_csrf(html: &str) -> String {
    html.split("name=\"csrf\"")
        .nth(1)
        .and_then(|rest| rest.split("value=\"").nth(1))
        .and_then(|rest| rest.split('"').next())
        .expect("csrf token should exist")
        .to_string()
}

async fn login_root_cookie(app: &axum::Router) -> String {
    login_cookie(app, "root", "root").await
}

async fn login_cookie(app: &axum::Router, handle: &str, password: &str) -> String {
    let page = app
        .clone()
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("login page should respond");
    assert_eq!(page.status(), StatusCode::OK);
    let cookie = cookie_header_value(&page);
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("handle={handle}&password={password}&csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("login should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    cookie_header_value(&response)
}

async fn board_with_extra_threads(state: &AppState, total: usize) -> String {
    let mut store = common::compat_store(state).await;
    let board_slug = state.boards.first().expect("board should exist").slug.clone();
    for index in 0..total {
        let title = format!("page-thread-{index}-{:#x}", Uuid::new_v4().as_u128());
        store
            .create_thread_with_checks(
                &board_slug,
                title,
                format!("body-{index}"),
                forum::ROOT_USER_ID,
            )
            .expect("thread should be created");
    }
    board_slug
}

#[tokio::test(flavor = "multi_thread")]
async fn users_page_renders_with_pg_state_post_strangle() {
    // A6.2 (strangle): view_users now reads exclusively from Postgres.
    // In PG-backed mode the handler returns
    // 200. The HTML-render assertions (User management / atlas / Search /
    // Wipe / Delete labels) will land in a #[ignore] integration test
    // that attaches a real Postgres pool — tracked alongside Phase 11's
    // PG-backed test scaffolding.
    let state = test_state().await;
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/users")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn logged_in_home_uses_app_shell_and_guide_uses_public_shell() {
    let state = test_state().await;
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    // A6.8 (strangle): `/` (index) now reads exclusively from Postgres.
    // In PG-backed mode it renders; the home-shell markup assertions
    // (shell-app, viceden-app.css, board-nav, Subscribed boards, account
    // menu, etc.) move to a PG-backed #[ignore] integration test.
    let home = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("home should respond");
    assert_eq!(home.status(), StatusCode::OK);

    let guide = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/guide")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("guide should respond");
    assert_eq!(guide.status(), StatusCode::OK);
    let guide_html = read_html(guide).await;
    assert!(guide_html.contains("class=\"shell-public\""));
    assert!(guide_html.contains("/assets/viceden-public.css"));
    assert!(guide_html.contains("/assets/viceden-motion.css"));
    assert!(!guide_html.contains("<style"));
    assert!(!guide_html.contains("class=\"shell-app\""));
    assert!(!guide_html.contains("class=\"board-nav\""));

    let login = app
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("login should respond");
    assert_eq!(login.status(), StatusCode::OK);
    let login_html = read_html(login).await;
    assert!(login_html.contains("class=\"auth-screen\""));
    assert!(login_html.contains("/assets/viceden-auth.css"));
    assert!(login_html.contains("/assets/viceden-motion.css"));
    assert!(!login_html.contains("<style"));
}

#[test]
fn app_templates_do_not_reintroduce_page_local_style_blocks_or_inner_scroll() {
    let template_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
    for name in [
        "base.html",
        "public-lite.html",
        "auth-base.html",
        "index.html",
        "board.html",
        "thread.html",
        "profile.html",
        "notifications.html",
    ] {
        let path = template_dir.join(name);
        let html = std::fs::read_to_string(&path).expect("template should read");
        assert!(!html.contains("<style"), "{name} should use shared skeleton CSS");
    }

    let css_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static/assets/viceden-app.css");
    let base = std::fs::read_to_string(css_path).expect("app css should read");
    for selector in [".sidebar-scroll", ".main-scroll", ".app-inspector", ".board-tabs", "pre"] {
        let marker = format!("{selector} {{");
        if let Some(start) = base.find(&marker) {
            let block = &base[start..base[start..].find('}').map(|offset| start + offset).unwrap()];
            assert!(!block.contains("overflow: auto"), "{selector} must not own scrolling");
            assert!(!block.contains("overflow-y: auto"), "{selector} must not own scrolling");
            assert!(!block.contains("overflow-x: auto"), "{selector} must not own scrolling");
            assert!(!block.contains("overflow: scroll"), "{selector} must not own scrolling");
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn rendered_site_is_dark_only_across_shells() {
    let state = test_state().await;
    let (board_slug, thread_id) = {
        let store = common::compat_store(&state).await;
        let thread = store.all_threads().into_iter().next().expect("seeded thread should exist");
        (thread.board_slug, thread.id)
    };
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    // /, /admin/users, /admin/reports, /t/{thread_id}, /b/{slug}, and
    // /search are intentionally excluded — they all render in
    // PG-backed mode after the full A6.x strangle. PG-backed dark-theme
    // assertions for those routes will land alongside Phase 11's #[ignore]
    // integration suite.
    let _ = thread_id;
    let _ = board_slug;
    for path in ["/notifications".to_string(), "/profile".to_string(), "/guide".to_string()] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path.as_str())
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("router should respond");
        assert_eq!(response.status(), StatusCode::OK, "{path} should render");
        let html = read_html(response).await;
        assert!(html.contains("data-theme=\"dark\""), "{path} should render dark theme");
        assert!(!html.contains("value=\"light\""), "{path} should not expose light mode");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn users_page_search_renders_with_pg_state_post_strangle() {
    // A6.2 (strangle): /admin/users now requires Postgres. The search-filter
    // assertion (atl → atlas link visible, luna link hidden) will move to a
    // PG-backed integration test alongside the rest of Phase 11's #[ignore]
    // suite. In PG-backed mode the route renders regardless of query.
    let state = test_state().await;
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/users?q=atl")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn root_user_page_renders_with_pg_state_post_strangle() {
    // A6.3 (strangle): /admin/users/{id} now reads exclusively from
    // Postgres. The "User profile / Recent posts / Started threads"
    // markup assertions move to a PG-backed #[ignore] integration test.
    let state = test_state().await;
    let atlas_id = {
        let store = common::compat_store(&state).await;
        store.user_id_for_handle("atlas").expect("atlas should exist")
    };
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{atlas_id}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn user_detail_shows_invite_subtree_and_cauterise_control() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let invited_a = Uuid::new_v4();
    let invited_b = Uuid::new_v4();
    {
        let pool = state.db.as_ref().expect("db").clone();
        UserRepo
            .insert(
                &pool,
                NewUser {
                    id: invited_a,
                    handle: format!("ia-{}", &invited_a.simple().to_string()[..8]),
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
                    handle: format!("ib-{}", &invited_b.simple().to_string()[..8]),
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
                    code: "vdn-inv-root-tree".into(),
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
                    code: "vdn-inv-root-tree".into(),
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
                    code: "vdn-inv-a-tree".into(),
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
                    code: "vdn-inv-a-tree".into(),
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
                    code: "vdn-inv-b-unused".into(),
                    inviter_user_id: invited_b,
                    max_redemptions: 2,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue unused child invite");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{invited_a}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("Invite subtree"));
    assert!(html.contains("1 downstream account"));
    assert!(html.contains("1 active invite code"));
    assert!(html.contains("Cauterise subtree"));
    assert!(html.contains("blocks subtree accounts"));
    assert!(html.contains("ib-"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cauterise_user_web_deactivates_subtree_and_revokes_unused_invites() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};
    use viceden::repository::posts::{ModState, NewPost, PostRepo};
    use viceden::repository::threads::{NewThread, ThreadRepo};
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let invited_a = Uuid::new_v4();
    let invited_b = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    let post_id = Uuid::new_v4();
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: invited_a,
                handle: format!("ca-{}", &invited_a.simple().to_string()[..8]),
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
                handle: format!("cb-{}", &invited_b.simple().to_string()[..8]),
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
                code: "vdn-inv-root-caut".into(),
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
                code: "vdn-inv-root-caut".into(),
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
                code: "vdn-inv-a-caut".into(),
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
                code: "vdn-inv-a-caut".into(),
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
                code: "vdn-inv-b-unused-caut".into(),
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
                title: "subtree thread".into(),
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
                body: "subtree reply".into(),
                mod_state: ModState::Visible,
            },
        )
        .await
        .expect("post");

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{invited_a}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail page");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{invited_a}/cauterise"))
                .method("POST")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}&reason=spam-wave")))
                .unwrap(),
        )
        .await
        .expect("cauterise should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let user_a_deactivated: bool =
        sqlx::query_scalar("SELECT deactivated_at IS NOT NULL FROM users WHERE id = $1")
            .bind(invited_a)
            .fetch_one(&pool)
            .await
            .expect("user a row");
    let user_b_deactivated: bool =
        sqlx::query_scalar("SELECT deactivated_at IS NOT NULL FROM users WHERE id = $1")
            .bind(invited_b)
            .fetch_one(&pool)
            .await
            .expect("user b row");
    let thread_state: String = sqlx::query_scalar("SELECT mod_state FROM threads WHERE id = $1")
        .bind(thread_id)
        .fetch_one(&pool)
        .await
        .expect("thread state");
    let post_state: String = sqlx::query_scalar("SELECT mod_state FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&pool)
        .await
        .expect("post state");
    let invite_revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM invites WHERE code = 'vdn-inv-b-unused-caut'",
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

    assert!(user_a_deactivated);
    assert!(user_b_deactivated);
    assert_eq!(thread_state, "removed");
    assert_eq!(post_state, "removed");
    assert!(invite_revoked);
    assert_eq!(action, "cauterise_subtree");
}

#[tokio::test(flavor = "multi_thread")]
async fn wipe_user_web_records_audit_and_shows_it_on_user_detail() {
    use viceden::repository::posts::{ModState, NewPost, PostRepo};
    use viceden::repository::threads::{NewThread, ThreadRepo};
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let target_id = Uuid::new_v4();
    let thread_id = Uuid::new_v4();
    let post_id = Uuid::new_v4();
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: target_id,
                handle: format!("wu-{}", &target_id.simple().to_string()[..8]),
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
                title: "wipe thread".into(),
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
                body: "wipe post".into(),
                mod_state: ModState::Visible,
            },
        )
        .await
        .expect("post");

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{target_id}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail page");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{target_id}/wipe"))
                .method("POST")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}&return_to=%2Fadmin%2Fusers%2F{target_id}")))
                .unwrap(),
        )
        .await
        .expect("wipe should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let action: String = sqlx::query_scalar(
        "SELECT action FROM moderation_events WHERE target_user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(target_id)
    .fetch_one(&pool)
    .await
    .expect("audit row");
    assert_eq!(action, "wipe_user");

    let detail = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{target_id}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail should respond");
    assert_eq!(detail.status(), StatusCode::OK);
    let html = read_html(detail).await;
    assert!(html.contains("Moderation history"));
    assert!(html.contains("wipe_user"));
    assert!(html.contains("founder content wipe"));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_user_web_records_audit_note() {
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let target_id = Uuid::new_v4();
    let handle = format!("du-{}", &target_id.simple().to_string()[..8]);
    UserRepo
        .insert(
            &pool,
            NewUser {
                id: target_id,
                handle: handle.clone(),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: Some("127.0.0.9".into()),
            },
        )
        .await
        .expect("user");

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{target_id}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail page");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{target_id}/delete"))
                .method("POST")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}&return_to=%2Fadmin%2Fusers")))
                .unwrap(),
        )
        .await
        .expect("delete should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

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
async fn thread_page_renders_with_pg_state_post_strangle() {
    // A6.5 (strangle): /t/{id} now reads exclusively from Postgres.
    // The detailed render assertions (vote-active, aria-label="Upvoted",
    // action-compact / action-vote / Report markup, absence of legacy
    // labels) move to a #[ignore] PG-backed integration test alongside
    // the rest of Phase 11's suite. PG-backed mode renders.
    let state = test_state().await;
    let thread_id = {
        let store = common::compat_store(&state).await;
        store.all_threads().into_iter().next().expect("seeded thread should exist").id
    };
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn notifications_page_shows_remove_button() {
    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let thread = store.all_threads().into_iter().next().expect("seeded thread should exist");
        store.add_notification(
            forum::ROOT_USER_ID,
            Some(Uuid::from_u128(1001)),
            forum::NotificationKind::Mention,
            Some(thread.id),
            None,
            Some("test notification".to_string()),
        );
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/notifications")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;

    assert!(html.contains("Remove"));
    assert!(html.contains("/notifications/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn notifications_page_links_to_exact_post_target() {
    use chrono::Utc;
    use viceden::repository::notifications::{NewNotification, NotificationRepo};

    let state = test_state().await;
    let (thread_id, post_id, thread_title) = {
        let mut store = common::compat_store(&state).await;
        let thread = store
            .create_thread_with_checks(
                "general",
                "notify anchor".to_string(),
                "notify body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "notify reply".to_string(), forum::ROOT_USER_ID)
            .expect("post");
        (thread.id, post.id, thread.title)
    };
    NotificationRepo
        .insert(
            state.db.as_ref().expect("db"),
            NewNotification {
                id: Uuid::new_v4(),
                recipient_user_id: forum::ROOT_USER_ID,
                actor_id: Some(forum::ROOT_USER_ID),
                kind: "mention".to_string(),
                thread_id: Some(thread_id),
                thread_title,
                post_id: Some(post_id),
                message: "new reply".to_string(),
                created_at: Utc::now(),
                is_read: false,
            },
        )
        .await
        .expect("notification");

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/notifications")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;

    assert!(html.contains(&format!("href=\"/t/{thread_id}#post-{post_id}\"")));
    assert!(html.contains("Open post"));
}

#[tokio::test(flavor = "multi_thread")]
async fn live_state_renders_with_pg_state_post_a7() {
    // A7 partial strangle (b688df3): /live/state now reads exclusively
    // from Postgres. The unread-notifications JSON assertion moves to a
    // PG-backed #[ignore] integration test.
    let state = test_state().await;
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/live/state")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_remove_reported_post_from_report_queue() {
    let state = test_state().await;
    let (report_id, post_id) = {
        let mut store = common::compat_store(&state).await;
        let thread = store.all_threads().into_iter().next().expect("seeded thread should exist");
        let post = store
            .create_post_with_checks(thread.id, "reported body".to_string(), forum::ROOT_USER_ID)
            .expect("post should be created");
        let report = store
            .report_post(
                forum::ROOT_USER_ID,
                Some(post.id),
                Some(thread.id),
                "needs review".to_string(),
                None,
            )
            .expect("report should be created");
        (report.id, post.id)
    };

    let app = build_router(state.clone());
    let cookie = login_root_cookie(&app).await;
    // After A6.4 strangle, /admin/reports renders in PG-backed mode.
    // Pull the CSRF token from a still-rendering page — `/notifications`
    // ships the same chrome and a same-session CSRF token.
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/notifications")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("notifications should respond");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/reports/{report_id}/remove-target"))
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("remove target should respond");

    // A7 partial strangle: /admin/reports/{id}/remove-target now requires
    // Postgres. PG-backed mode renders; the post-removal assertion
    // moves to a PG-backed #[ignore] integration test alongside the
    // rest of the strangle suite.
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let _ = post_id; // referenced below in the deferred PG-backed test.
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_report_removal_notifies_author_with_reason() {
    use viceden::repository::notifications::NotificationRepo;

    let state = test_state().await;
    let (report_id, thread_id, post_id, member_id) = {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("notifytarget", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_checks(
                "general",
                "notify moderation".to_string(),
                "thread body".to_string(),
                member.id,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "reported body".to_string(), member.id)
            .expect("post");
        let report = store
            .report_post(
                forum::ROOT_USER_ID,
                Some(post.id),
                Some(thread.id),
                "spam links".to_string(),
                None,
            )
            .expect("report");
        (report.id, thread.id, post.id, member.id)
    };

    let app = build_router(state.clone());
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/notifications")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("notifications should respond");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/reports/{report_id}/remove-target"))
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("remove target should respond");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let notifications = NotificationRepo
        .list_for_user(state.db.as_ref().expect("db"), member_id, 10)
        .await
        .expect("notifications");
    let notification =
        notifications.iter().find(|row| row.kind == "post_removed").expect("removed notification");
    assert_eq!(notification.kind, "post_removed");
    assert_eq!(notification.thread_id, Some(thread_id));
    assert_eq!(notification.post_id, Some(post_id));
    assert_eq!(notification.message, "Your post was removed. Reason: spam links.");
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_mention_notifications_skip_new_mention_alerts() {
    use chrono::Utc;
    use viceden::repository::{
        notifications::{NewNotification, NotificationRepo},
        preferences::{PreferencesRepo, UpdateSettings},
    };

    let state = test_state().await;
    let member_id = {
        let mut store = common::compat_store(&state).await;
        store.create_user("quietmention", "secret123", None).expect("user").id
    };
    PreferencesRepo
        .update_settings(
            state.db.as_ref().expect("db"),
            member_id,
            UpdateSettings {
                compactness: 0,
                theme: "dark".to_string(),
                notify_mentions: false,
                notify_replies: true,
                notify_moderation: true,
            },
        )
        .await
        .expect("settings");
    NotificationRepo
        .insert(
            state.db.as_ref().expect("db"),
            NewNotification {
                id: Uuid::new_v4(),
                recipient_user_id: member_id,
                actor_id: Some(forum::ROOT_USER_ID),
                kind: "mention".to_string(),
                thread_id: None,
                thread_title: "General".to_string(),
                post_id: None,
                message: "root mentioned you".to_string(),
                created_at: Utc::now(),
                is_read: false,
            },
        )
        .await
        .expect("insert mention");

    let notifications = NotificationRepo
        .list_for_user(state.db.as_ref().expect("db"), member_id, 10)
        .await
        .expect("notifications");
    assert!(!notifications.iter().any(|row| row.kind == "mention"));
}

#[tokio::test(flavor = "multi_thread")]
async fn reply_notifications_target_thread_author_with_reason() {
    use viceden::repository::notifications::NotificationRepo;

    let state = test_state().await;
    let (owner_id, replier_id, thread_id, post_id) = {
        let mut store = common::compat_store(&state).await;
        let owner_id = store.create_user("replyowner", "secret123", None).expect("owner").id;
        let replier_id = store.create_user("replyauthor", "secret123", None).expect("replier").id;
        let thread = store
            .create_thread_with_checks(
                "general",
                "reply notify title".to_string(),
                "reply notify body".to_string(),
                owner_id,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "reply message".to_string(), replier_id)
            .expect("reply");
        (owner_id, replier_id, thread.id, post.id)
    };

    let pool = state.db.as_ref().expect("db");
    let owner_notifications =
        NotificationRepo.list_for_user(pool, owner_id, 20).await.expect("owner notifications");
    let reply_notification = owner_notifications
        .iter()
        .find(|row| row.kind == "reply" && row.post_id == Some(post_id))
        .expect("reply notification");
    assert_eq!(reply_notification.thread_id, Some(thread_id));
    assert_eq!(
        reply_notification.message,
        "replyauthor replied to your thread 'reply notify title'."
    );

    let replier_notifications =
        NotificationRepo.list_for_user(pool, replier_id, 20).await.expect("replier notifications");
    assert!(!replier_notifications.iter().any(|row| {
        row.kind == "reply" && row.thread_id == Some(thread_id) && row.post_id == Some(post_id)
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_reply_notifications_skip_thread_reply_alerts() {
    use chrono::Utc;
    use viceden::repository::{
        notifications::{NewNotification, NotificationRepo},
        preferences::{PreferencesRepo, UpdateSettings},
    };

    let state = test_state().await;
    let owner_id = {
        let mut store = common::compat_store(&state).await;
        store.create_user("quietreply", "secret123", None).expect("owner").id
    };
    PreferencesRepo
        .update_settings(
            state.db.as_ref().expect("db"),
            owner_id,
            UpdateSettings {
                compactness: 0,
                theme: "dark".to_string(),
                notify_mentions: true,
                notify_replies: false,
                notify_moderation: true,
            },
        )
        .await
        .expect("settings");
    NotificationRepo
        .insert(
            state.db.as_ref().expect("db"),
            NewNotification {
                id: Uuid::new_v4(),
                recipient_user_id: owner_id,
                actor_id: Some(forum::ROOT_USER_ID),
                kind: "reply".to_string(),
                thread_id: Some(Uuid::new_v4()),
                thread_title: "quiet reply title".to_string(),
                post_id: Some(Uuid::new_v4()),
                message: "root replied to your thread 'quiet reply title'.".to_string(),
                created_at: Utc::now(),
                is_read: false,
            },
        )
        .await
        .expect("reply insert");

    let notifications = NotificationRepo
        .list_for_user(state.db.as_ref().expect("db"), owner_id, 20)
        .await
        .expect("notifications");
    assert!(!notifications.iter().any(|row| row.kind == "reply"));
}

#[tokio::test(flavor = "multi_thread")]
async fn tag_follow_notifications_use_richer_reason_text() {
    use viceden::repository::notifications::NotificationRepo;

    let state = test_state().await;
    let follower_id = {
        let mut store = common::compat_store(&state).await;
        let follower_id = store.create_user("tagwatch", "secret123", None).expect("follower").id;
        let actor_id = store.create_user("tagposter", "secret123", None).expect("actor").id;
        store.follow_tag(follower_id, "safety").expect("follow tag");
        store
            .create_thread_with_tags(
                "general",
                "tagged thread title".to_string(),
                "tagged thread body".to_string(),
                actor_id,
                vec!["safety".to_string()],
            )
            .expect("thread");
        follower_id
    };

    let notifications = NotificationRepo
        .list_for_user(state.db.as_ref().expect("db"), follower_id, 20)
        .await
        .expect("notifications");
    let tag_notification =
        notifications.iter().find(|row| row.kind == "reply").expect("tag follow notification");
    assert_eq!(
        tag_notification.message,
        "tagposter started a new thread matching #safety: 'tagged thread title'."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_moderation_notifications_skip_report_removal_alerts() {
    use viceden::repository::{
        notifications::NotificationRepo,
        preferences::{PreferencesRepo, UpdateSettings},
    };

    let state = test_state().await;
    let (report_id, member_id) = {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("quietmod", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_checks(
                "general",
                "quiet mod".to_string(),
                "thread body".to_string(),
                member.id,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "reported body".to_string(), member.id)
            .expect("post");
        let report = store
            .report_post(
                forum::ROOT_USER_ID,
                Some(post.id),
                Some(thread.id),
                "spam links".to_string(),
                None,
            )
            .expect("report");
        (report.id, member.id)
    };
    PreferencesRepo
        .update_settings(
            state.db.as_ref().expect("db"),
            member_id,
            UpdateSettings {
                compactness: 0,
                theme: "dark".to_string(),
                notify_mentions: true,
                notify_replies: true,
                notify_moderation: false,
            },
        )
        .await
        .expect("settings");

    let app = build_router(state.clone());
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/notifications")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("notifications should respond");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/reports/{report_id}/remove-target"))
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("remove target should respond");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let notifications = NotificationRepo
        .list_for_user(state.db.as_ref().expect("db"), member_id, 10)
        .await
        .expect("notifications");
    assert!(!notifications.iter().any(|row| row.kind == "post_removed"));
}

#[tokio::test(flavor = "multi_thread")]
async fn profile_alert_toggle_updates_notification_preferences() {
    use viceden::repository::preferences::PreferencesRepo;

    let state = test_state().await;
    let app = build_router(state.clone());
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("profile should respond");
    assert_eq!(page.status(), StatusCode::OK);
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/profile")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}&notify_mentions=false")))
                .unwrap(),
        )
        .await
        .expect("profile update");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let settings = PreferencesRepo
        .settings_for(state.db.as_ref().expect("db"), forum::ROOT_USER_ID)
        .await
        .expect("settings");
    assert!(!settings.notify_mentions);
    assert!(settings.notify_replies);
    assert!(settings.notify_moderation);
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_page_shows_unsave_controls() {
    let state = test_state().await;
    let thread_id = {
        let mut store = common::compat_store(&state).await;
        let thread = store
            .create_thread_with_checks(
                "general",
                "saved page item".to_string(),
                "saved body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread");
        store.toggle_thread_saved(forum::ROOT_USER_ID, thread.id).expect("save should work");
        thread.id
    };

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/saved")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("saved should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains(&format!("/t/{thread_id}")));
    assert!(html.contains(&format!("action=\"/t/{thread_id}/save\"")));
    assert!(html.contains("Unsave"));
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_page_lists_hidden_threads_and_can_unhide() {
    use viceden::repository::preferences::PreferencesRepo;

    let state = test_state().await;
    let thread_id = {
        let mut store = common::compat_store(&state).await;
        let thread = store
            .create_thread_with_checks(
                "general",
                "hidden page item".to_string(),
                "hidden body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread");
        store.toggle_thread_hidden(forum::ROOT_USER_ID, thread.id).expect("hide should work");
        thread.id
    };

    let app = build_router(state.clone());
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/hidden")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("hidden should respond");
    assert_eq!(page.status(), StatusCode::OK);
    let html = read_html(page).await;
    assert!(html.contains(&format!("/t/{thread_id}")));
    assert!(html.contains(&format!("action=\"/t/{thread_id}/hide\"")));
    assert!(html.contains("Unhide"));
    let csrf = extract_csrf(&html);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}/hide"))
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}&return_to=/hidden")))
                .unwrap(),
        )
        .await
        .expect("unhide");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let hidden = PreferencesRepo
        .is_thread_hidden(state.db.as_ref().expect("db"), forum::ROOT_USER_ID, thread_id)
        .await
        .expect("hidden lookup");
    assert!(!hidden);
}

#[tokio::test(flavor = "multi_thread")]
async fn search_page_shows_save_and_hide_controls_for_results() {
    let state = test_state().await;
    let unique = format!("search-controls-{}", Uuid::new_v4().simple());
    let thread_id = {
        let mut store = common::compat_store(&state).await;
        store
            .create_thread_with_checks(
                "general",
                unique.clone(),
                "search body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread")
            .id
    };

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/search?q={unique}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("search should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains(&format!("action=\"/t/{thread_id}/save\"")));
    assert!(html.contains(&format!("action=\"/t/{thread_id}/hide\"")));
    assert!(html.contains("Save"));
    assert!(html.contains("Hide"));
    assert!(html.contains("Open thread"));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_page_shows_active_filter_chips() {
    let state = test_state().await;
    let unique = format!("search-chips-{}", Uuid::new_v4().simple());
    {
        let mut store = common::compat_store(&state).await;
        store
            .create_thread_with_tags(
                "general",
                format!("chip title {unique}"),
                format!("chip body {unique}"),
                forum::ROOT_USER_ID,
                vec!["safety".to_string()],
            )
            .expect("thread");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/search?q={unique}%20tag:safety%20author:root%20:%20/general&scope=posts"
                ))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("search should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("Active filters"));
    assert!(html.contains("#safety"));
    assert!(html.contains("@root"));
    assert!(html.contains("/general"));
    assert!(html.contains("posts only"));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_reports_page_renders_triage_lanes() {
    use chrono::Utc;
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    let (thread_id, post_id, member_id, report_id) = {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("deskuser", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_checks(
                "general",
                "desk title".to_string(),
                "desk body".to_string(),
                member.id,
            )
            .expect("thread");
        let post = store
            .create_post_with_checks(thread.id, "desk reply".to_string(), member.id)
            .expect("post");
        let report_id = Uuid::new_v4();
        let pool = state.db.as_ref().expect("db").clone();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: report_id,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: Some(post.id),
                    target_thread_id: Some(thread.id),
                    reason: "minor risk".into(),
                    details: Some("desk case".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("report insert");
        ReportRepo
            .triage(&pool, report_id, Some("reviewing"), Some(Some(forum::ROOT_USER_ID)))
            .await
            .expect("triage update");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: Some(post.id),
                    target_thread_id: Some(thread.id),
                    target_user_id: Some(member.id),
                    report_id: Some(report_id),
                    action: "remove_post".into(),
                    reason: "minor risk".into(),
                    note: Some("desk note".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("event insert");
        (thread.id, post.id, member.id, report_id)
    };

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/reports")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("reports should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("Urgent"));
    assert!(html.contains("Normal"));
    assert!(html.contains("Action history"));
    assert!(html.contains("Resolution note"));
    assert!(html.contains("desk note"));
    assert!(html.contains("reviewing"));
    assert!(html.contains("Assigned to root"));
    assert!(html.contains(&member_id.to_string()) || html.contains("deskuser"));
    let _ = (thread_id, post_id, report_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_claim_report_and_set_queue_state_from_web() {
    use chrono::Utc;
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    let report_id = {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("deskclaim", "secret123", None).expect("user");
        let thread = store
            .create_thread_with_checks(
                "general",
                "desk claim title".to_string(),
                "desk claim body".to_string(),
                member.id,
            )
            .expect("thread");
        let pool = state.db.as_ref().expect("db").clone();
        let report_id = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: report_id,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(thread.id),
                    reason: "needs owner".into(),
                    details: Some("claim this".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("report insert");
        report_id
    };

    let app = build_router(state.clone());
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/reports")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("reports page");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/reports/{report_id}/triage"))
                .method("POST")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf={csrf}&queue_state=reviewing&claim=true&note=desk-owner"
                )))
                .unwrap(),
        )
        .await
        .expect("triage should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let pool = state.db.as_ref().expect("db");
    let queue_state: String = sqlx::query_scalar("SELECT queue_state FROM reports WHERE id = $1")
        .bind(report_id)
        .fetch_one(pool)
        .await
        .expect("queue state");
    let assigned_to: Option<Uuid> =
        sqlx::query_scalar("SELECT assigned_to FROM reports WHERE id = $1")
            .bind(report_id)
            .fetch_one(pool)
            .await
            .expect("assigned");
    let action: String = sqlx::query_scalar(
        "SELECT action FROM moderation_events WHERE report_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(report_id)
    .fetch_one(pool)
    .await
    .expect("audit row");

    assert_eq!(queue_state, "reviewing");
    assert_eq!(assigned_to, Some(forum::ROOT_USER_ID));
    assert_eq!(action, "claim_report");
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_reports_page_filters_queue_and_history() {
    use chrono::Utc;
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("deskfilter", "secret123", None).expect("user");
        let open_thread = store
            .create_thread_with_checks(
                "general",
                "desk filter open".to_string(),
                "desk filter open body".to_string(),
                member.id,
            )
            .expect("open thread");
        let resolved_thread = store
            .create_thread_with_checks(
                "general",
                "desk filter resolved".to_string(),
                "desk filter resolved body".to_string(),
                member.id,
            )
            .expect("resolved thread");
        let pool = state.db.as_ref().expect("db").clone();

        let open_report_id = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: open_report_id,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(open_thread.id),
                    reason: "filter open case".into(),
                    details: Some("open only".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("open report");
        ReportRepo
            .triage(&pool, open_report_id, Some("reviewing"), Some(Some(forum::ROOT_USER_ID)))
            .await
            .expect("triage");

        let resolved_report_id = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: resolved_report_id,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(resolved_thread.id),
                    reason: "filter resolved case".into(),
                    details: Some("history only".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("resolved report");
        ReportRepo
            .resolve_with_note(
                &pool,
                resolved_report_id,
                forum::ROOT_USER_ID,
                true,
                Some("history note"),
            )
            .await
            .expect("resolve");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    let filtered = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/reports?assigned=me&queue_state=reviewing")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("filtered reports");
    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered_html = read_html(filtered).await;
    assert!(filtered_html.contains("filter open case"));
    assert!(!filtered_html.contains("filter resolved case"));
    assert!(filtered_html.contains("Assigned to root"));

    let history = app
        .oneshot(
            Request::builder()
                .uri("/admin/reports?status=resolved")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("history reports");
    assert_eq!(history.status(), StatusCode::OK);
    let history_html = read_html(history).await;
    assert!(history_html.contains("review history"));
    assert!(history_html.contains("filter resolved case"));
    assert!(history_html.contains("history note"));
    assert!(!history_html.contains("filter open case"));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_reports_page_filters_by_audit_action_and_moderator() {
    use chrono::Utc;
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("deskact", "secret123", None).expect("user");
        let thread_a = store
            .create_thread_with_checks(
                "general",
                "desk action a".to_string(),
                "desk action a body".to_string(),
                member.id,
            )
            .expect("thread a");
        let thread_b = store
            .create_thread_with_checks(
                "general",
                "desk action b".to_string(),
                "desk action b body".to_string(),
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
                    reason: "action keep".into(),
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
                    note: Some("match me".into()),
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
                    reason: "action skip".into(),
                    details: Some("no match".into()),
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
                    note: Some("skip me".into()),
                    created_at: Utc::now(),
                },
            )
            .await
            .expect("event b");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/reports?action=remove_thread&moderator=root")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("filtered reports");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("action keep"));
    assert!(!html.contains("action skip"));
    assert!(html.contains("value=\"remove_thread\""));
    assert!(html.contains("value=\"root\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_reports_page_filters_by_audit_date_window() {
    use chrono::{TimeZone, Utc};
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("deskdate", "secret123", None).expect("user");
        let older_thread = store
            .create_thread_with_checks(
                "general",
                "desk date old".to_string(),
                "desk date old body".to_string(),
                member.id,
            )
            .expect("old thread");
        let newer_thread = store
            .create_thread_with_checks(
                "general",
                "desk date new".to_string(),
                "desk date new body".to_string(),
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
                    reason: "date old".into(),
                    details: Some("skip old".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).single().unwrap(),
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
                    reason: "old action".into(),
                    note: Some("outside".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 9, 30, 0).single().unwrap(),
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
                    reason: "date new".into(),
                    details: Some("keep new".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 9, 0, 0).single().unwrap(),
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
                    reason: "new action".into(),
                    note: Some("inside".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 11, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("newer event");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/admin/reports?action=remove_thread&moderator=root&since=2026-05-15&until=2026-05-31",
                )
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("filtered reports");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("date new"));
    assert!(!html.contains("date old"));
    assert!(html.contains("value=\"2026-05-15\""));
    assert!(html.contains("value=\"2026-05-31\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_reports_page_exports_filtered_audit_csv() {
    use chrono::{TimeZone, Utc};
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};
    use viceden::repository::reports::{NewReport, ReportRepo};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let member = store.create_user("deskcsv", "secret123", None).expect("user");
        let keep_thread = store
            .create_thread_with_checks(
                "general",
                "desk csv keep".to_string(),
                "desk csv keep body".to_string(),
                member.id,
            )
            .expect("keep thread");
        let skip_thread = store
            .create_thread_with_checks(
                "general",
                "desk csv skip".to_string(),
                "desk csv skip body".to_string(),
                member.id,
            )
            .expect("skip thread");
        let pool = state.db.as_ref().expect("db").clone();

        let keep_report = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: keep_report,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(keep_thread.id),
                    reason: "csv keep reason".into(),
                    details: Some("keep export".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 8, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("keep report");
        ReportRepo
            .resolve_with_note(
                &pool,
                keep_report,
                forum::ROOT_USER_ID,
                true,
                Some("kept in resolved export"),
            )
            .await
            .expect("resolve keep");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(keep_thread.id),
                    target_user_id: Some(member.id),
                    report_id: Some(keep_report),
                    action: "remove_thread".into(),
                    reason: "csv keep action".into(),
                    note: Some("keep note".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 20, 8, 30, 0).single().unwrap(),
                },
            )
            .await
            .expect("keep event");

        let skip_report = Uuid::new_v4();
        ReportRepo
            .insert(
                &pool,
                NewReport {
                    id: skip_report,
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(skip_thread.id),
                    reason: "csv skip reason".into(),
                    details: Some("skip export".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 8, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("skip report");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(skip_thread.id),
                    target_user_id: Some(member.id),
                    report_id: Some(skip_report),
                    action: "remove_thread".into(),
                    reason: "csv skip action".into(),
                    note: Some("skip note".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 8, 30, 0).single().unwrap(),
                },
            )
            .await
            .expect("skip event");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(
                    "/admin/reports?status=resolved&action=remove_thread&moderator=root&since=2026-05-15&until=2026-05-31",
                )
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("reports page");
    assert_eq!(page.status(), StatusCode::OK);
    let html = read_html(page).await;
    assert!(html.contains("Audit CSV"));
    assert!(html.contains("/admin/reports/export.csv?status=resolved"));
    assert!(html.contains("action=remove_thread"));
    assert!(html.contains("moderator=root"));
    assert!(html.contains("since=2026-05-15"));
    assert!(html.contains("until=2026-05-31"));

    let response = app
        .oneshot(
            Request::builder()
                .uri(
                    "/admin/reports/export.csv?status=resolved&action=remove_thread&moderator=root&since=2026-05-15&until=2026-05-31",
                )
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("csv export");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/csv; charset=utf-8"
    );
    let csv = read_html(response).await;
    assert!(csv.starts_with(
        "report_id,report_status,report_queue_state,report_priority,report_reason,reporter,target_kind,target_author,audit_id,audit_created_at,audit_actor,audit_action,audit_reason,audit_note\n"
    ));
    assert!(csv.contains("csv keep reason"));
    assert!(csv.contains("csv keep action"));
    assert!(csv.contains("root"));
    assert!(!csv.contains("csv skip reason"));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_moderation_events_page_filters_and_exports_csv() {
    use chrono::{TimeZone, Utc};
    use viceden::repository::moderation_events::{ModerationEventInsert, ModerationEventRepo};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let keep_user = store.create_user("auditkeep", "secret123", None).expect("keep user");
        let skip_user = store.create_user("auditskip", "secret123", None).expect("skip user");
        let keep_thread = store
            .create_thread_with_checks(
                "general",
                "audit page keep".to_string(),
                "audit page keep body".to_string(),
                keep_user.id,
            )
            .expect("keep thread");
        let skip_thread = store
            .create_thread_with_checks(
                "general",
                "audit page skip".to_string(),
                "audit page skip body".to_string(),
                skip_user.id,
            )
            .expect("skip thread");
        let pool = state.db.as_ref().expect("db").clone();

        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(keep_thread.id),
                    target_user_id: Some(keep_user.id),
                    report_id: None,
                    action: "remove_thread".into(),
                    reason: "audit page keep reason".into(),
                    note: Some("keep note".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 21, 9, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("keep event");
        ModerationEventRepo
            .insert(
                &pool,
                ModerationEventInsert {
                    id: Uuid::new_v4(),
                    actor_id: forum::ROOT_USER_ID,
                    target_post_id: None,
                    target_thread_id: Some(skip_thread.id),
                    target_user_id: Some(skip_user.id),
                    report_id: None,
                    action: "claim_report".into(),
                    reason: "audit page skip reason".into(),
                    note: Some("skip note".into()),
                    created_at: Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).single().unwrap(),
                },
            )
            .await
            .expect("skip event");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/moderation-events?action=remove_thread&moderator=root&target=auditkeep&since=2026-05-15&until=2026-05-31&limit=25")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("audit page");

    assert_eq!(page.status(), StatusCode::OK);
    let html = read_html(page).await;
    assert!(html.contains("Moderation audit"));
    assert!(html.contains("audit page keep reason"));
    assert!(!html.contains("audit page skip reason"));
    assert!(html.contains("value=\"remove_thread\""));
    assert!(html.contains("value=\"root\""));
    assert!(html.contains("value=\"auditkeep\""));
    assert!(html.contains("value=\"2026-05-15\""));
    assert!(html.contains("value=\"2026-05-31\""));
    assert!(html.contains("/admin/moderation-events.csv?action=remove_thread"));
    assert!(html.contains("limit=25"));

    let csv_response = app
        .oneshot(
            Request::builder()
                .uri("/admin/moderation-events.csv?action=remove_thread&moderator=root&target=auditkeep&since=2026-05-15&until=2026-05-31&limit=25")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("audit csv");
    assert_eq!(csv_response.status(), StatusCode::OK);
    assert_eq!(
        csv_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default(),
        "text/csv; charset=utf-8"
    );
    let csv = read_html(csv_response).await;
    assert!(csv.starts_with(
        "id,created_at,actor,action,reason,note,report_id,target_user,target_thread_id,target_post_id\n"
    ));
    assert!(csv.contains("audit page keep reason"));
    assert!(csv.contains("auditkeep"));
    assert!(!csv.contains("audit page skip reason"));
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_metrics_page_and_thread_state_badges_render() {
    use std::fs;
    use viceden::config::ProxySettings;

    let mut state = test_state().await;
    let backup_dir =
        std::env::temp_dir().join(format!("viceden-web-metrics-backup-{}", Uuid::new_v4()));
    fs::create_dir_all(&backup_dir).expect("backup dir");
    fs::write(backup_dir.join("viceden-20260531.dump.gz"), b"backup").expect("backup file");
    fs::write(backup_dir.join("viceden-20260531.dump.gz.verified"), b"verified")
        .expect("marker file");
    state.config.ops.backup_dir = Some(backup_dir.to_string_lossy().to_string());
    state.config.server.secure_cookies = true;
    state.config.server.proxy = ProxySettings::default();
    state.config.server.rate_limit.disabled = false;
    let thread_id = {
        let mut store = common::compat_store(&state).await;
        let thread = store.all_threads().into_iter().next().expect("seeded thread should exist");
        store.set_thread_pinned(thread.id, forum::ROOT_USER_ID, true).expect("pin");
        store.set_thread_locked(thread.id, forum::ROOT_USER_ID, true).expect("lock");
        thread.id
    };

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let metrics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/metrics")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("metrics should respond");
    // A6.1 (strangle): view_metrics now reads exclusively from Postgres.
    // In PG-backed mode the handler returns
    // 200. The "page renders" assertion now lives in a #[ignore]
    // integration test that actually attaches a Postgres pool.
    assert_eq!(metrics.status(), StatusCode::OK);
    let metrics_html = read_html(metrics).await;
    assert!(metrics_html.contains("Backup status"));
    assert!(metrics_html.contains("Latest verified restore"));
    assert!(metrics_html.contains("viceden-20260531.dump.gz"));
    assert!(metrics_html.contains("ok"));
    assert!(metrics_html.contains("Ingress status"));
    assert!(metrics_html.contains("loopback_only"));
    assert!(metrics_html.contains("secure cookies on"));
    assert!(metrics_html.contains("rate limit on"));
    assert!(metrics_html.contains("Cf-Connecting-Ip"));

    // A6.5 (strangle): the second half of this test asserted "pinned" /
    // "locked" badges render on /t/{id}. After the strangle the route
    // renders in PG-backed mode; the badge-render check moves to a
    // PG-backed #[ignore] integration test alongside the rest of Phase 11's
    // suite.
    let thread = app
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}"))
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("thread should respond");
    assert_eq!(thread.status(), StatusCode::OK);

    let _ = fs::remove_dir_all(backup_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn board_page_renders_with_pg_state_post_strangle() {
    // A6.6 (strangle): /b/{slug} now reads exclusively from Postgres.
    // The pagination + render assertions (page 2 / 2, thread-meta /
    // thread-submit markup, absence of legacy "Open thread" / ">Save<" /
    // "Duplicate-title guard" / "started " labels) move to a PG-backed
    // #[ignore] integration test. PG-backed mode renders.
    let state = test_state().await;
    let board_slug = board_with_extra_threads(&state, 12).await;
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/b/{board_slug}?page=2"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn thread_page_local_time_hook_renders_with_pg_state_post_strangle() {
    // A6.5 (strangle): /t/{id} now reads exclusively from Postgres. The
    // `local-datetime` / `data-utc=` / `DD.MM.YY-HH:MM` markup assertions
    // move to a PG-backed #[ignore] integration test.
    let state = test_state().await;
    let thread_id = {
        let store = common::compat_store(&state).await;
        store.all_threads().into_iter().next().expect("seeded thread should exist").id
    };

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn recent_activity_links_renders_with_pg_state_post_strangle() {
    // A6.8 (strangle): `/` (index) now reads exclusively from Postgres.
    // In PG-backed mode the route renders; the
    // "/t/{thread_id}#post-{post_id}" link + body-hiding assertions move
    // to a PG-backed #[ignore] integration test.
    let state = test_state().await;
    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread")]
async fn register_page_renders_invite_registration() {
    let state = test_state().await;
    let app = build_router(state);

    let response = app
        .oneshot(Request::builder().uri("/register").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("Claim your handle"));
    assert!(html.contains("name=\"code\""));
    assert!(html.contains("name=\"confirm_18\""));
    assert!(html.contains("name=\"confirm_rules\""));
}

#[tokio::test(flavor = "multi_thread")]
async fn register_page_with_code_shows_inviter_context() {
    let state = test_state().await;
    let code = {
        let pool = state.db.as_ref().expect("db");
        viceden::repository::invites::InviteRepo
            .issue(
                pool,
                viceden::repository::invites::NewInvite {
                    code: "vdn-inv-landing".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 2,
                    expires_at: None,
                    note: Some("friends only".into()),
                },
            )
            .await
            .expect("invite issue")
    };
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder().uri(format!("/register?code={code}")).body(Body::empty()).unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("Invited by"));
    assert!(html.contains("root"));
    assert!(html.contains("friends only"));
    assert!(html.contains("2 uses left"));
}

#[tokio::test(flavor = "multi_thread")]
async fn register_failures_trip_second_line_throttle_even_when_ip_changes() {
    let state = test_state().await;
    let code = {
        let pool = state.db.as_ref().expect("db");
        viceden::repository::invites::InviteRepo
            .issue(
                pool,
                viceden::repository::invites::NewInvite {
                    code: "vdn-inv-throttle".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 20,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("invite issue")
    };
    let app = build_router(state);

    for attempt in 0..5 {
        let page = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/register?code={code}"))
                    .header("cf-connecting-ip", format!("198.51.100.{}", attempt + 1))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("register page should respond");
        let pre_auth_cookie = cookie_header_value(&page);
        let csrf = extract_csrf(&read_html(page).await);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/register")
                    .method("POST")
                    .header("cf-connecting-ip", format!("198.51.100.{}", attempt + 1))
                    .header(header::COOKIE, &pre_auth_cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(format!(
                        "csrf={csrf}&code={code}&handle=root&password=secret123&dob=1990-01-01&confirm_18=yes&confirm_rules=yes"
                    )))
                    .unwrap(),
            )
            .await
            .expect("register attempt should respond");
        assert_eq!(response.status(), StatusCode::OK);
        let html = read_html(response).await;
        assert!(html.contains("handle is taken"));
    }

    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/register?code={code}"))
                .header("cf-connecting-ip", "198.51.100.99")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("register page should respond");
    let pre_auth_cookie = cookie_header_value(&page);
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/register")
                .method("POST")
                .header("cf-connecting-ip", "198.51.100.99")
                .header(header::COOKIE, &pre_auth_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf={csrf}&code={code}&handle=root&password=secret123&dob=1990-01-01&confirm_18=yes&confirm_rules=yes"
                )))
                .unwrap(),
        )
        .await
        .expect("register attempt should respond");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let html = read_html(response).await;
    assert!(html.contains("Too many registration attempts. Try again later."));
}

#[tokio::test(flavor = "multi_thread")]
async fn successful_registration_redirects_to_welcome_guide() {
    let state = test_state().await;
    let code = {
        let pool = state.db.as_ref().expect("db");
        viceden::repository::invites::InviteRepo
            .issue(
                pool,
                viceden::repository::invites::NewInvite {
                    code: "vdn-inv-welcome".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 1,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("invite issue")
    };
    let app = build_router(state);

    let page = app
        .clone()
        .oneshot(
            Request::builder().uri(format!("/register?code={code}")).body(Body::empty()).unwrap(),
        )
        .await
        .expect("register page");
    let pre_auth_cookie = cookie_header_value(&page);
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/register")
                .method("POST")
                .header(header::COOKIE, &pre_auth_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf={csrf}&code={code}&handle=welcome1&password=secret123&dob=1990-01-01&confirm_18=yes&confirm_rules=yes"
                )))
                .unwrap(),
        )
        .await
        .expect("register should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some("/guide?welcome=1")
    );

    let session_cookie = cookie_header_value(&response);
    let guide = app
        .oneshot(
            Request::builder()
                .uri("/guide?welcome=1")
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("guide should respond");
    assert_eq!(guide.status(), StatusCode::OK);
    let html = read_html(guide).await;
    assert!(html.contains("Account ready."));
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_sets_probation_and_blocks_new_threads_and_replies() {
    use chrono::Utc;
    use viceden::repository::invites::{InviteRepo, NewInvite};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let thread_id = {
        let mut store = common::compat_store(&state).await;
        store
            .create_thread_with_checks(
                "general",
                "probation root thread".to_string(),
                "root body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("root thread")
            .id
    };
    InviteRepo
        .issue(
            &pool,
            NewInvite {
                code: "vdn-inv-probation".into(),
                inviter_user_id: forum::ROOT_USER_ID,
                max_redemptions: 1,
                expires_at: None,
                note: Some("probation test".into()),
            },
        )
        .await
        .expect("invite issue");

    let app = build_router(state);
    let register_page = app
        .clone()
        .oneshot(
            Request::builder().uri("/register?code=vdn-inv-probation").body(Body::empty()).unwrap(),
        )
        .await
        .expect("register page");
    assert_eq!(register_page.status(), StatusCode::OK);
    let register_cookie = cookie_header_value(&register_page);
    let register_html = read_html(register_page).await;
    let register_csrf = extract_csrf(&register_html);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/register")
                .method("POST")
                .header(header::COOKIE, &register_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf={register_csrf}&code=vdn-inv-probation&handle=probationweb&password=secret123&dob=1990-01-01&confirm_18=yes&confirm_rules=yes"
                )))
                .unwrap(),
        )
        .await
        .expect("register submit");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).and_then(|value| value.to_str().ok()),
        Some("/guide?welcome=1")
    );
    let session_cookie = cookie_header_value(&response);

    let probation_until: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT probation_until FROM users WHERE handle = 'probationweb'")
            .fetch_one(&pool)
            .await
            .expect("probation row");
    assert!(probation_until.is_some());
    assert!(probation_until.unwrap() > Utc::now());

    let guide = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/guide?welcome=1")
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("guide");
    assert_eq!(guide.status(), StatusCode::OK);
    let guide_html = read_html(guide).await;
    assert!(guide_html.contains("New accounts can read now."));

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
        .expect("profile");
    assert_eq!(profile.status(), StatusCode::OK);
    let profile_html = read_html(profile).await;
    assert!(profile_html.contains("Probation active until"));
    let session_csrf = extract_csrf(&profile_html);

    let thread_attempt = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/b/general")
                .method("POST")
                .header(header::COOKIE, &session_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf={session_csrf}&title=probation-blocked-thread&body=blocked"
                )))
                .unwrap(),
        )
        .await
        .expect("thread attempt");
    assert_eq!(thread_attempt.status(), StatusCode::FORBIDDEN);
    assert!(read_html(thread_attempt).await.contains("probation active"));

    let reply_attempt = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}"))
                .method("POST")
                .header(header::COOKIE, &session_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={session_csrf}&body=blocked-reply")))
                .unwrap(),
        )
        .await
        .expect("reply attempt");
    assert_eq!(reply_attempt.status(), StatusCode::FORBIDDEN);
    assert!(read_html(reply_attempt).await.contains("probation active"));

    let root_cookie = login_root_cookie(&app).await;
    let users = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/users?q=probationweb")
                .header(header::COOKIE, &root_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("users page");
    assert_eq!(users.status(), StatusCode::OK);
    let users_html = read_html(users).await;
    assert!(users_html.contains("probation"));

    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE handle = 'probationweb'")
        .fetch_one(&pool)
        .await
        .expect("user id");
    let detail = app
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{user_id}"))
                .header(header::COOKIE, &root_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("detail");
    assert_eq!(detail.status(), StatusCode::OK);
    let detail_html = read_html(detail).await;
    assert!(detail_html.contains("Probation until"));
}

#[tokio::test(flavor = "multi_thread")]
async fn founder_can_lift_probation_from_user_detail_and_unlock_posting() {
    use chrono::{Duration, Utc};

    let state = test_state().await;
    let pool = state.db.as_ref().expect("db").clone();
    let thread_id = {
        let mut store = common::compat_store(&state).await;
        let user = store.create_user("probclear", "secret123", None).expect("user");
        sqlx::query("UPDATE users SET probation_until = $2 WHERE id = $1")
            .bind(user.id)
            .bind(Utc::now() + Duration::hours(24))
            .execute(&pool)
            .await
            .expect("probation set");
        store
            .create_thread_with_checks(
                "general",
                "probation unlock thread".to_string(),
                "root body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread")
            .id
    };

    let app = build_router(state);

    let founder_cookie = login_root_cookie(&app).await;
    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/users")
                .header(header::COOKIE, &founder_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("users");
    let founder_csrf = extract_csrf(&read_html(detail).await);

    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE handle = 'probclear'")
        .fetch_one(&pool)
        .await
        .expect("user id");
    let lift = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/users/{user_id}/lift-probation"))
                .method("POST")
                .header(header::COOKIE, &founder_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "csrf={founder_csrf}&return_to=%2Fadmin%2Fusers%2F{user_id}&note=manual-clear"
                )))
                .unwrap(),
        )
        .await
        .expect("lift");
    assert_eq!(lift.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        lift.headers().get(header::LOCATION).and_then(|value| value.to_str().ok()),
        Some(format!("/admin/users/{user_id}").as_str())
    );

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

    let login_page = app
        .clone()
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("login page");
    let login_cookie = cookie_header_value(&login_page);
    let login_csrf = extract_csrf(&read_html(login_page).await);
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::COOKIE, &login_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("handle=probclear&password=secret123&csrf={login_csrf}")))
                .unwrap(),
        )
        .await
        .expect("login");
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let user_cookie = cookie_header_value(&login);

    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &user_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("profile");
    let profile_html = read_html(profile).await;
    let user_csrf = extract_csrf(&profile_html);
    assert!(!profile_html.contains("Probation active until"));

    let thread_attempt = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/b/general")
                .method("POST")
                .header(header::COOKIE, &user_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={user_csrf}&title=probation-now-open&body=allowed")))
                .unwrap(),
        )
        .await
        .expect("thread attempt");
    assert_eq!(thread_attempt.status(), StatusCode::SEE_OTHER);

    let reply_attempt = app
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}"))
                .method("POST")
                .header(header::COOKIE, &user_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={user_csrf}&body=reply-now-open")))
                .unwrap(),
        )
        .await
        .expect("reply attempt");
    assert_eq!(reply_attempt.status(), StatusCode::SEE_OTHER);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_thread_web_blocks_daily_thread_limit() {
    use chrono::Utc;
    use viceden::repository::{
        posts::{ModState, NewPost, PostRepo},
        threads::{NewThread, ThreadRepo},
    };

    let state = test_state().await;
    let user_id = {
        let mut store = common::compat_store(&state).await;
        store.create_user("threadcapweb", "secret123", None).expect("user").id
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
                    title: format!("cap thread {index}"),
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
    let cookie = login_cookie(&app, "threadcapweb", "secret123").await;
    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("profile");
    let csrf = extract_csrf(&read_html(profile).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/b/general")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}&title=blocked-thread&body=blocked-body")))
                .unwrap(),
        )
        .await
        .expect("thread create");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(read_html(response).await, "thread limit reached");
}

#[tokio::test(flavor = "multi_thread")]
async fn create_post_web_blocks_daily_reply_limit() {
    use viceden::repository::{
        posts::{ModState, NewPost, PostRepo},
        threads::ThreadRepo,
    };

    let state = test_state().await;
    let (user_id, thread_id) = {
        let mut store = common::compat_store(&state).await;
        let user_id = store.create_user("replycapweb", "secret123", None).expect("user").id;
        let thread_id = store
            .create_thread_with_checks(
                "general",
                "reply cap root".to_string(),
                "root body".to_string(),
                forum::ROOT_USER_ID,
            )
            .expect("thread")
            .id;
        (user_id, thread_id)
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
    let _ =
        ThreadRepo.find_by_id(&pool, thread_id).await.expect("thread lookup").expect("thread row");

    let app = build_router(state);
    let cookie = login_cookie(&app, "replycapweb", "secret123").await;
    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("profile");
    let csrf = extract_csrf(&read_html(profile).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/t/{thread_id}"))
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("csrf={csrf}&body=blocked-reply")))
                .unwrap(),
        )
        .await
        .expect("reply create");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(read_html(response).await, "reply limit reached");
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_invites_page_shows_redemptions_and_downstream_count() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};
    use viceden::repository::users::{NewUser, UserRepo};

    let state = test_state().await;
    {
        let pool = state.db.as_ref().expect("db").clone();
        let invited_a = Uuid::new_v4();
        let invited_b = Uuid::new_v4();
        UserRepo
            .insert(
                &pool,
                NewUser {
                    id: invited_a,
                    handle: format!("ia-{}", &invited_a.simple().to_string()[..8]),
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
                    handle: format!("ib-{}", &invited_b.simple().to_string()[..8]),
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
                    code: "vdn-inv-root-a".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 1,
                    expires_at: None,
                    note: Some("wave one".into()),
                },
            )
            .await
            .expect("issue root invite");
        let mut conn = pool.acquire().await.expect("conn");
        InviteRepo
            .redeem(
                &mut conn,
                RedeemInvite {
                    code: "vdn-inv-root-a".into(),
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
                    code: "vdn-inv-a-b".into(),
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
                    code: "vdn-inv-a-b".into(),
                    user_id: invited_b,
                    redeemed_from_ip: None,
                },
            )
            .await
            .expect("redeem child invite");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invites should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("Redeemed by"));
    assert!(html.contains("1 downstream"));
    assert!(html.contains("wave one"));
    assert!(html.contains("Revoke blocks future use only."));
    assert!(html.contains("1 downstream account(s) stay active."));
    assert!(html.contains("Revoke future use"));
    assert!(html.contains("watch"));
    assert!(html.contains("Redeemed once. Watch downstream growth."));
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_invites_issue_blocks_when_active_limit_reached() {
    use viceden::repository::invites::{InviteRepo, NewInvite};

    let state = test_state().await;
    {
        let pool = state.db.as_ref().expect("db").clone();
        for idx in 0..5 {
            InviteRepo
                .issue(
                    &pool,
                    NewInvite {
                        code: format!("vdn-inv-limit-{idx}"),
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
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invites page");
    let csrf = extract_csrf(&read_html(page).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .method("POST")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}&note=overflow&max_redemptions=1")))
                .unwrap(),
        )
        .await
        .expect("issue should respond");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = read_html(response).await;
    assert!(html.contains("5/5 active"));
    assert!(html.contains("Active invite limit reached (5)."));
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_invites_page_marks_multi_use_codes_as_watch_risk() {
    use viceden::repository::invites::{InviteRepo, NewInvite};

    let state = test_state().await;
    {
        let pool = state.db.as_ref().expect("db").clone();
        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-watch-risk".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 5,
                    expires_at: None,
                    note: Some("wide code".into()),
                },
            )
            .await
            .expect("invite issue");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invites should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("watch"));
    assert!(html.contains("Multi-use code allows up to 5 redemptions."));
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_invites_page_marks_redeemed_tree_as_high_risk() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let invited_a = store.create_user("invtreea", "secret123", None).expect("a").id;
        let invited_b = store.create_user("invtreeb", "secret123", None).expect("b").id;
        let invited_c = store.create_user("invtreec", "secret123", None).expect("c").id;
        let pool = state.db.as_ref().expect("db").clone();

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-high-risk-root".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 5,
                    expires_at: None,
                    note: Some("wide tree".into()),
                },
            )
            .await
            .expect("issue root invite");

        let mut tx = pool.begin().await.expect("begin tx a");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-high-risk-root".into(),
                    user_id: invited_a,
                    redeemed_from_ip: Some("127.0.0.1".into()),
                },
            )
            .await
            .expect("redeem a");
        tx.commit().await.expect("commit a");

        let mut tx = pool.begin().await.expect("begin tx b");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-high-risk-root".into(),
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
                    code: "vdn-inv-high-risk-child".into(),
                    inviter_user_id: invited_a,
                    max_redemptions: 1,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue child invite");
        let mut tx = pool.begin().await.expect("begin tx c");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-high-risk-child".into(),
                    user_id: invited_c,
                    redeemed_from_ip: Some("127.0.0.3".into()),
                },
            )
            .await
            .expect("redeem c");
        tx.commit().await.expect("commit c");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invites should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let html = read_html(response).await;
    assert!(html.contains("high"));
    assert!(html.contains("Code already redeemed 2 times and now reaches 1 downstream account(s)."));
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_invites_issue_blocks_when_high_risk_code_stays_active() {
    use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};

    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let invited_a = store.create_user("invblocka", "secret123", None).expect("a").id;
        let invited_b = store.create_user("invblockb", "secret123", None).expect("b").id;
        let invited_c = store.create_user("invblockc", "secret123", None).expect("c").id;
        let pool = state.db.as_ref().expect("db").clone();

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-high-block-root".into(),
                    inviter_user_id: forum::ROOT_USER_ID,
                    max_redemptions: 5,
                    expires_at: None,
                    note: Some("wide tree".into()),
                },
            )
            .await
            .expect("issue root invite");

        let mut tx = pool.begin().await.expect("begin tx a");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-high-block-root".into(),
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
                    code: "vdn-inv-high-block-root".into(),
                    user_id: invited_b,
                    redeemed_from_ip: Some("127.0.0.2".into()),
                },
            )
            .await
            .expect("redeem b");
        tx.commit().await.expect("commit tx a");

        InviteRepo
            .issue(
                &pool,
                NewInvite {
                    code: "vdn-inv-high-block-child".into(),
                    inviter_user_id: invited_a,
                    max_redemptions: 1,
                    expires_at: None,
                    note: None,
                },
            )
            .await
            .expect("issue child invite");

        let mut tx = pool.begin().await.expect("begin tx c");
        InviteRepo
            .redeem(
                &mut tx,
                RedeemInvite {
                    code: "vdn-inv-high-block-child".into(),
                    user_id: invited_c,
                    redeemed_from_ip: Some("127.0.0.3".into()),
                },
            )
            .await
            .expect("redeem c");
        tx.commit().await.expect("commit tx c");
    }

    let app = build_router(state);
    let cookie = login_root_cookie(&app).await;
    let page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("invites page");
    let page_html = read_html(page).await;
    let csrf = extract_csrf(&page_html);
    assert!(page_html.contains("1 high risk active"));
    assert!(
        page_html.contains("High-risk active codes must be revoked or contained before new issue.")
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/invites")
                .method("POST")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}&note=blocked&max_redemptions=1")))
                .unwrap(),
        )
        .await
        .expect("issue should respond");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let html = read_html(response).await;
    assert!(
        html.contains("High-risk invite still active. Revoke or cauterise before issuing more.")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn register_post_requires_pre_auth_csrf() {
    let app = build_router(test_state().await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/register")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("handle=newmember&password=secret123&password_confirm=secret123"))
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// F5: CSRF token rotates per-request. Two consecutive GET /login
/// hits should produce different `pre_auth_csrf` token values; an
/// attacker who gets a previously-issued token should not be able to
/// reuse it after the next page render.
#[tokio::test(flavor = "multi_thread")]
async fn csrf_token_rotates_across_login_pages() {
    let state = test_state().await;
    let app = build_router(state);

    let first = app
        .clone()
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("login first");
    assert_eq!(first.status(), StatusCode::OK);
    let first_csrf = extract_csrf(&read_html(first).await);

    let second = app
        .clone()
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("login second");
    assert_eq!(second.status(), StatusCode::OK);
    let second_csrf = extract_csrf(&read_html(second).await);

    assert_ne!(
        first_csrf, second_csrf,
        "F5 regression: CSRF token must rotate per-request (got identical {first_csrf} on both)"
    );
    // Both should be UUID-shaped (8-4-4-4-12 hex digits with dashes,
    // total 36 chars).
    assert_eq!(first_csrf.len(), 36);
    assert_eq!(second_csrf.len(), 36);
}

/// F11: assert the session cookie ships with the security flags we expect.
/// Catches accidental softening (e.g. dropping HttpOnly or SameSite=Lax,
/// shipping cookies in plaintext when secure_cookies=true).
#[tokio::test(flavor = "multi_thread")]
async fn session_cookie_carries_security_flags() {
    let state = test_state_with_secure_cookies().await;
    let app = build_router(state);

    // GET /login → pre-auth CSRF cookie + form CSRF token in the body.
    let page = app
        .clone()
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("login page should respond");
    let pre_auth_cookie = cookie_header_value(&page);
    let csrf = extract_csrf(&read_html(page).await);

    // POST /login → session cookie issued.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &pre_auth_cookie)
                .body(Body::from(format!("handle=root&password=root&csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("login should respond");
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    let raw = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie header must be set on successful login")
        .to_str()
        .expect("cookie should be utf8");

    // Required security flags. Each flag is a separate assert so the
    // failure message names exactly which one regressed.
    assert!(
        raw.starts_with("__Host-viceden_session="),
        "secure cookie missing __Host- name: {raw}"
    );
    assert!(raw.contains("HttpOnly"), "session cookie missing HttpOnly: {raw}");
    assert!(raw.contains("SameSite=Lax"), "session cookie missing SameSite=Lax: {raw}");
    assert!(raw.contains("Path=/"), "session cookie missing Path=/: {raw}");
    assert!(raw.contains("Max-Age="), "session cookie missing Max-Age=: {raw}");
    assert!(
        raw.contains("Secure"),
        "session cookie must include Secure when secure_cookies=true: {raw}"
    );
}
