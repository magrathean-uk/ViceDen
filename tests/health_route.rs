use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use sqlx::postgres::PgPoolOptions;
use tower::util::ServiceExt;

use viceden::{app::build_router, config, models::Board, AppState};

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let response = health_response(false).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
    assert_eq!(response.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(
        response.headers().get("referrer-policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert_eq!(response.headers().get("cross-origin-opener-policy").unwrap(), "same-origin");
    assert_eq!(
        response.headers().get("content-security-policy").unwrap(),
        "default-src 'self'; base-uri 'self'; frame-ancestors 'none'; object-src 'none'; form-action 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'"
    );
    assert_eq!(
        response.headers().get("permissions-policy").unwrap(),
        "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()"
    );
    assert_eq!(response.headers().get("x-viceden-version").unwrap(), env!("CARGO_PKG_VERSION"));
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    assert!(response.headers().get("strict-transport-security").is_none());
}

#[tokio::test]
async fn secure_mode_sets_hsts_header() {
    let response = health_response(true).await;

    assert_eq!(
        response.headers().get("strict-transport-security").unwrap(),
        "max-age=31536000; includeSubDomains; preload"
    );
}

#[tokio::test]
async fn robots_and_sitemap_render_public_metadata() {
    let robots = route_response("/robots.txt", false).await;
    assert_eq!(robots.status(), StatusCode::OK);
    assert_eq!(robots.headers().get("content-type").unwrap(), "text/plain; charset=utf-8");
    let robots_body = read_body(robots).await;
    assert!(robots_body.contains("Sitemap: /sitemap.xml"));
    assert!(robots_body.contains("User-agent: GPTBot"));
    assert!(robots_body.contains("User-agent: ClaudeBot"));
    assert!(robots_body.contains("User-agent: OAI-SearchBot"));
    assert!(robots_body.contains("Disallow: /admin"));
    assert!(robots_body.contains("Disallow: /api"));

    let sitemap = route_response("/sitemap.xml", false).await;
    assert_eq!(sitemap.status(), StatusCode::OK);
    assert_eq!(sitemap.headers().get("content-type").unwrap(), "application/xml; charset=utf-8");
    let sitemap_body = read_body(sitemap).await;
    assert!(sitemap_body.contains("<loc>/guide</loc>"));
    assert!(sitemap_body.contains("<loc>/rules</loc>"));
    assert!(sitemap_body.contains("<loc>/privacy</loc>"));
    assert!(!sitemap_body.contains("<loc>/login</loc>"));
    assert!(!sitemap_body.contains("<loc>/register</loc>"));
    assert!(!sitemap_body.contains("<loc>/b/general</loc>"));

    let llms = route_response("/llms.txt", false).await;
    assert_eq!(llms.status(), StatusCode::OK);
    assert_eq!(llms.headers().get("content-type").unwrap(), "text/plain; charset=utf-8");
    let llms_body = read_body(llms).await;
    assert!(llms_body.contains("https://viceden.example.com/guide"));
}

#[tokio::test]
async fn missing_database_pool_times_out_health_check() {
    let boards = vec![Board {
        slug: "general".to_string(),
        name: "General".to_string(),
        ttl_days: 180,
        description: String::new(),
    }];
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/viceden")
        .expect("lazy pool should construct without network IO");
    let state = AppState {
        db: viceden::RequiredPgPool::new(pool.clone()),
        boards,
        repos: viceden::repository::Repos::new(pool.clone()),
        login_throttle: viceden::cache::LoginThrottle::new(5, chrono::Duration::minutes(15)),
        mention_cache: viceden::cache::MentionSuggest::new(std::time::Duration::from_secs(60)),
        moderation: None,
        config: config::Settings {
            server: config::ServerSettings {
                bind: "127.0.0.1:3000".to_string(),
                max_request_body_bytes: 131072,
                request_timeout_seconds: 1,
                secure_cookies: false,
                proxy: Default::default(),
                rate_limit: Default::default(),
            },
            database: config::DatabaseSettings {
                url: "postgres://postgres:postgres@localhost:5432/viceden".to_string(),
                require_connection: true,
                max_connections: 1,
            },
            auth: config::AuthSettings::default(),
            classifier: Default::default(),
            ops: Default::default(),
            boards_file: "config/boards.toml".to_string(),
        },
    };

    let response = build_router(state)
        .oneshot(Request::builder().uri("/api/healthz").body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
}

async fn health_response(secure_cookies: bool) -> axum::response::Response {
    route_response("/api/healthz", secure_cookies).await
}

async fn route_response(uri: &str, secure_cookies: bool) -> axum::response::Response {
    let settings = config::Settings {
        server: config::ServerSettings {
            bind: "127.0.0.1:3000".to_string(),
            max_request_body_bytes: 131072,
            request_timeout_seconds: 15,
            secure_cookies,
            proxy: Default::default(),
            rate_limit: Default::default(),
        },
        database: config::DatabaseSettings {
            url: "postgres://postgres:postgres@localhost:5432/viceden".to_string(),
            require_connection: false,
            max_connections: 1,
        },
        auth: config::AuthSettings::default(),
        classifier: Default::default(),
        ops: Default::default(),
        boards_file: "config/boards.toml".to_string(),
    };

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy(&settings.database.url)
        .expect("lazy pool should construct without network IO");

    let boards = vec![Board {
        slug: "general".to_string(),
        name: "General".to_string(),
        ttl_days: 180,
        description: String::new(),
    }];

    let state = AppState {
        db: viceden::RequiredPgPool::new(pool.clone()),
        boards,
        repos: viceden::repository::Repos::new(pool.clone()),
        login_throttle: viceden::cache::LoginThrottle::new(5, chrono::Duration::minutes(15)),
        mention_cache: viceden::cache::MentionSuggest::new(std::time::Duration::from_secs(60)),
        moderation: None,
        config: settings,
    };

    let app = build_router(state);
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router should respond");

    response
}

async fn read_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.expect("body should read");
    String::from_utf8(body.to_vec()).expect("body should be utf8")
}
