use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use tower::util::ServiceExt;
mod common;
use viceden::{app::build_router, forum, AppState};

async fn test_state() -> AppState {
    common::app_state().await
}

async fn test_state_with_secure_cookies(secure_cookies: bool) -> AppState {
    common::app_state_with_secure_cookies(secure_cookies).await
}

async fn read_html(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.expect("body should read");
    String::from_utf8(body.to_vec()).expect("html should be utf8")
}

fn extract_csrf(html: &str) -> String {
    html.split("name=\"csrf\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("csrf token should exist")
        .to_string()
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
        .expect("cookie should have main value")
        .to_string()
}

fn form_body(pairs: &[(&str, &str)]) -> Body {
    Body::from(
        pairs
            .iter()
            .map(|(key, value)| format!("{}={}", urlencoded(key), urlencoded(value)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

fn urlencoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

async fn login_page_csrf(app: &axum::Router) -> (String, String) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let cookie = cookie_header_value(&response);
    let html = read_html(response).await;
    (cookie, extract_csrf(&html))
}

async fn login_root_cookie(app: &axum::Router) -> String {
    let (cookie, csrf) = login_page_csrf(app).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &cookie)
                .body(Body::from(format!("handle=root&password=root&csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    cookie_header_value(&response)
}

#[tokio::test(flavor = "multi_thread")]
async fn rendered_user_fields_neutralise_xss_payloads() {
    let state = test_state().await;
    let board_slug = state.boards.first().expect("board should exist").slug.clone();
    let app = build_router(state);
    let session_cookie = login_root_cookie(&app).await;
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
        .expect("profile should respond");
    let csrf = extract_csrf(&read_html(profile).await);

    let title = "<script>alert(1)</script>";
    let body = r#"<img src=x onerror="alert(1)"> [bad](javascript:alert(1)) @<script>"#;
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/b/{board_slug}"))
                .method("POST")
                .header(header::COOKIE, &session_cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(form_body(&[("csrf", &csrf), ("title", title), ("body", body)]))
                .unwrap(),
        )
        .await
        .expect("create should respond");
    assert_eq!(create.status(), StatusCode::SEE_OTHER);
    let location = create
        .headers()
        .get(header::LOCATION)
        .expect("create should redirect")
        .to_str()
        .expect("location utf8")
        .to_string();

    let page = app
        .oneshot(
            Request::builder()
                .uri(location)
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("thread should render");
    assert_eq!(page.status(), StatusCode::OK);
    let html = read_html(page).await;
    assert!(
        !html.contains("<script>alert"),
        "thread title/body must not render active script tags"
    );
    assert!(!html.contains("javascript:"), "markdown links must not render javascript hrefs");
    assert!(!html.contains("<img"), "html image tags must not survive body sanitization");
}

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_web_reads_redirect_to_login() {
    let app = build_router(test_state().await);
    let response = app
        .clone()
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");

    let board = app
        .oneshot(Request::builder().uri("/b/general").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(board.status(), StatusCode::SEE_OTHER);
    assert_eq!(board.headers().get(header::LOCATION).unwrap(), "/login");
}

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_profile_and_admin_redirect_to_login() {
    let app = build_router(test_state().await);

    let profile = app
        .clone()
        .oneshot(Request::builder().uri("/profile").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(profile.status(), StatusCode::SEE_OTHER);
    assert_eq!(profile.headers().get(header::LOCATION).unwrap(), "/login");

    let admin = app
        .oneshot(Request::builder().uri("/admin/users").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(admin.status(), StatusCode::SEE_OTHER);
    assert_eq!(admin.headers().get(header::LOCATION).unwrap(), "/login");
}

#[tokio::test(flavor = "multi_thread")]
async fn login_page_renders_and_login_logout_roundtrip() {
    let app = build_router(test_state().await);

    let (login_cookie, login_csrf) = login_page_csrf(&app).await;
    let login_html = {
        let response = app
            .clone()
            .oneshot(Request::builder().uri("/login").body(Body::empty()).unwrap())
            .await
            .expect("router should respond");
        assert_eq!(response.status(), StatusCode::OK);
        read_html(response).await
    };
    assert!(login_html.contains(">ID</label>"));
    assert!(login_html.contains(">pass</label>"));
    assert!(login_html.contains(">enter</button>"));
    assert!(login_html.contains("name=\"handle\""));
    assert!(login_html.contains("name=\"password\""));
    assert!(login_html.contains("name=\"csrf\""));
    assert!(login_html.contains("class=\"auth-screen\""));
    assert!(login_html.contains("class=\"identity-card\""));
    assert!(!login_html.contains("Username"));
    assert!(!login_html.contains("Password"));
    assert!(!login_html.contains(">ViceDen<"));
    assert!(!login_html.contains("Sign in to continue"));
    assert!(!login_html.contains("shell-public"));
    assert!(!login_html.contains("shell-app"));
    assert!(!login_html.contains("recent"));
    assert!(!login_html.contains("class=\"board-nav\""));

    let missing_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &login_cookie)
                .body(Body::from("handle=root&password=root"))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);

    let bad_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &login_cookie)
                .body(Body::from("handle=root&password=root&csrf=wrong"))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(bad_csrf.status(), StatusCode::FORBIDDEN);

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, &login_cookie)
                .body(Body::from(format!("handle=root&password=root&csrf={login_csrf}")))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    let session_cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .expect("login should set cookie")
        .to_str()
        .expect("cookie should be utf8");
    assert!(session_cookie.contains(forum::ForumStore::session_cookie_name()));

    let cookie_header = session_cookie.split(';').next().expect("cookie value should exist");
    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, cookie_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(profile.status(), StatusCode::OK);
    let csrf = extract_csrf(&read_html(profile).await);

    let logout = app
        .oneshot(
            Request::builder()
                .uri("/logout")
                .method("POST")
                .header(header::COOKIE, cookie_header)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}&return_to=/login")))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        logout
            .headers()
            .get(header::LOCATION)
            .expect("logout should redirect")
            .to_str()
            .expect("location should be utf8"),
        "/login"
    );
    let cleared_cookies = logout
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("cookie should be utf8").to_string())
        .collect::<Vec<_>>();
    assert!(cleared_cookies.iter().any(|value| {
        value.contains(forum::ForumStore::session_cookie_name()) && value.contains("Max-Age=0")
    }));
    assert!(cleared_cookies
        .iter()
        .any(|value| value.contains("viceden_pre_auth_csrf") && value.contains("Max-Age=0")));
}

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_requests_stay_on_login_wall_and_authenticated_static_pages_render() {
    let app = build_router(test_state().await);

    for path in ["/guide", "/rules", "/privacy"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::OK, "{path} should load");
        let html = read_html(response).await;
        assert!(html.contains("class=\"shell-public\""), "{path} should use public shell");
        assert!(!html.contains("class=\"shell-app\""), "{path} should avoid app shell");
    }

    let response = app
        .clone()
        .oneshot(Request::builder().uri("/missing-page").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER, "missing page should redirect");
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");

    for path in ["/", "/profile", "/saved", "/hidden"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path} should redirect");
        assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/login");
    }

    let register = app
        .clone()
        .oneshot(Request::builder().uri("/register").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(register.status(), StatusCode::OK);

    let cookie = login_root_cookie(&app).await;
    for path in ["/rules", "/privacy"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("router should respond");

        assert_eq!(response.status(), StatusCode::OK, "{path} should load");
        let html = read_html(response).await;
        assert!(html.contains("class=\"shell-public\""), "{path} should use public shell");
        assert!(!html.contains("class=\"shell-app\""), "{path} should avoid app shell");
    }

    let missing = app
        .oneshot(
            Request::builder()
                .uri("/missing-page")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_html = read_html(missing).await;
    assert!(missing_html.contains("Not found"));
}

#[tokio::test(flavor = "multi_thread")]
async fn login_return_to_rejects_protocol_relative_redirects() {
    let app = build_router(test_state().await);
    let (login_cookie, login_csrf) = login_page_csrf(&app).await;

    let login = app
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, login_cookie)
                .body(Body::from(format!(
                    "handle=root&password=root&return_to=//evil.test&csrf={login_csrf}"
                )))
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    assert_eq!(login.headers().get(header::LOCATION).unwrap(), "/profile");
}

#[tokio::test(flavor = "multi_thread")]
async fn login_return_to_rejects_encoded_and_backslash_redirects() {
    let app = build_router(test_state().await);
    let (login_cookie, login_csrf) = login_page_csrf(&app).await;

    for return_to in ["%2f%2fevil.test", "/%5cevil.test"] {
        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/login")
                    .method("POST")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(header::COOKIE, &login_cookie)
                    .body(Body::from(format!(
                        "handle=root&password=root&return_to={return_to}&csrf={login_csrf}"
                    )))
                    .unwrap(),
            )
            .await
            .expect("router should respond");

        assert_eq!(login.status(), StatusCode::SEE_OTHER);
        assert_eq!(login.headers().get(header::LOCATION).unwrap(), "/profile");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn secure_session_cookie_is_cleared_with_secure_attribute() {
    let app = build_router(test_state_with_secure_cookies(true).await);
    let (login_cookie, login_csrf) = login_page_csrf(&app).await;
    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, login_cookie)
                .body(Body::from(format!("handle=root&password=root&csrf={login_csrf}")))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    let session_cookie = login
        .headers()
        .get(header::SET_COOKIE)
        .expect("login should set cookie")
        .to_str()
        .expect("cookie should be utf8");
    assert!(session_cookie.starts_with("__Host-viceden_session="));
    assert!(!session_cookie.starts_with("viceden_session="));
    assert!(session_cookie.contains("; Secure"));
    assert!(session_cookie.contains("Max-Age="));
    assert!(session_cookie.contains("HttpOnly"));
    assert!(session_cookie.contains("SameSite=Lax"));

    let cookie_header = session_cookie.split(';').next().expect("cookie value should exist");
    let legacy_cookie_header =
        cookie_header.replacen("__Host-viceden_session=", "viceden_session=", 1);
    let legacy_profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &legacy_cookie_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(legacy_profile.status(), StatusCode::OK);

    let profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, cookie_header)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router should respond");
    let csrf = extract_csrf(&read_html(profile).await);

    let logout = app
        .oneshot(
            Request::builder()
                .uri("/logout")
                .method("POST")
                .header(header::COOKIE, cookie_header)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("csrf={csrf}")))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    let cleared_cookies = logout
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("cookie should be utf8").to_string())
        .collect::<Vec<_>>();
    assert!(cleared_cookies
        .iter()
        .any(|value| value.starts_with("__Host-viceden_session=deleted")));
    assert!(cleared_cookies.iter().any(|value| value.starts_with("viceden_session=deleted")));
    for cleared in cleared_cookies.iter().filter(|value| value.contains("viceden_session")) {
        assert!(cleared.contains("; Secure"));
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains("HttpOnly"));
        assert!(cleared.contains("SameSite=Lax"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn old_session_csrf_fails_after_relogin() {
    let app = build_router(test_state().await);
    let first_cookie = login_root_cookie(&app).await;
    let first_profile = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/profile")
                .header(header::COOKIE, &first_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("profile should respond");
    let first_csrf = extract_csrf(&read_html(first_profile).await);

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/logout")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, first_cookie)
                .body(Body::from(format!("csrf={first_csrf}")))
                .unwrap(),
        )
        .await
        .expect("logout should respond");
    assert_eq!(logout.status(), StatusCode::SEE_OTHER);

    let second_cookie = login_root_cookie(&app).await;
    let stale_csrf = app
        .oneshot(
            Request::builder()
                .uri("/logout")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, second_cookie)
                .body(Body::from(format!("csrf={first_csrf}")))
                .unwrap(),
        )
        .await
        .expect("stale csrf request should respond");
    assert_eq!(stale_csrf.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_is_invite_only_and_csrf_guarded() {
    let app = build_router(test_state().await);

    let get_response = app
        .clone()
        .oneshot(Request::builder().uri("/register").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");
    assert_eq!(get_response.status(), StatusCode::OK);

    let post_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/register")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("handle=memberx&password=secret123&password_confirm=secret123"))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(post_response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn member_credentials_can_create_regular_session() {
    let state = test_state().await;
    {
        let mut store = common::compat_store(&state).await;
        let _ = store.create_user("memberx", "secret123", None).expect("user should be created");
    }
    let app = build_router(state);
    let (login_cookie, login_csrf) = login_page_csrf(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/login")
                .method("POST")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, login_cookie)
                .body(Body::from(format!("handle=memberx&password=secret123&csrf={login_csrf}")))
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/profile");
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_web_post_requires_csrf() {
    let app = build_router(test_state().await);
    let cookie_header = login_root_cookie(&app).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/profile")
                .method("POST")
                .header(header::COOKIE, &cookie_header)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("compactness=1&theme=dark"))
                .unwrap(),
        )
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread")]
async fn api_write_requires_pat_and_revoked_pat_stops_working() {
    let state = test_state().await;
    let board_slug = state.boards.first().expect("board should exist").slug.clone();
    let founder_pat = {
        let mut store = common::compat_store(&state).await;
        store.issue_api_token(forum::ROOT_USER_ID).expect("pat should be issued")
    };

    let app = build_router(state.clone());

    let anonymous = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/boards/{board_slug}/threads"))
                .method("POST")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"hello","body":"world"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/boards/{board_slug}/threads"))
                .method("POST")
                .header("x-api-token", founder_pat.as_str())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"hello","body":"world"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(create.status(), StatusCode::CREATED);

    {
        let mut store = common::compat_store(&state).await;
        assert!(store.revoke_api_token(forum::ROOT_USER_ID, founder_pat.as_str()));
    }

    let revoked = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/boards/{board_slug}/threads"))
                .method("POST")
                .header("x-api-token", founder_pat.as_str())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"title":"again","body":"blocked"}"#))
                .unwrap(),
        )
        .await
        .expect("router should respond");
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
}
