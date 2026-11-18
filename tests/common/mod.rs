//! Shared fixtures for repository integration tests.
//!
//! Tests using this module require a live PostgreSQL with VICEDEN migrations
//! applied. They are gated behind `#[ignore]` so the default `cargo test` run
//! stays green on machines without a local PG. To run them, ensure
//! `VICEDEN_DATABASE__URL` points at a Postgres with M1+M2 migrations applied
//! and invoke:
//!
//! ```bash
//! cargo test --test repository_users -- --ignored --test-threads=1
//! cargo test --test repository_sessions -- --ignored --test-threads=1
//! ```

use std::{
    fs,
    net::TcpListener,
    path::Path,
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex, Once, OnceLock,
    },
};

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use viceden::{config, forum, models::Board, repository::Repos, AppState};

static PG_ADMIN_URL: OnceLock<String> = OnceLock::new();
static DB_COUNTER: AtomicUsize = AtomicUsize::new(0);
static PG_CLEANUP_REGISTERED: Once = Once::new();
static PG_ROOTS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

const INITDB_BIN: &str = "/opt/homebrew/bin/initdb";
const PG_CTL_BIN: &str = "/opt/homebrew/bin/pg_ctl";

#[allow(dead_code)]
pub async fn shared_pool() -> PgPool {
    let url = std::env::var("VICEDEN_DATABASE__URL").expect(
        "VICEDEN_DATABASE__URL must be set for repository tests \
         (point at a local Postgres with viceden migrations applied)",
    );
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("connect to test PG (set VICEDEN_DATABASE__URL or run with --skip)");
    let applied_schema: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.invite_redemptions')::text")
            .fetch_one(&pool)
            .await
            .expect("check applied schema");
    if applied_schema.is_none() {
        sqlx::migrate!("./migrations").run(&pool).await.expect("migrations apply");
    }
    let boards_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join("boards.toml");
    let boards = viceden::models::Board::load_from_file(&boards_path).expect("boards load");
    viceden::db::sync_boards(&pool, &boards).await.expect("seed boards");
    pool
}

#[allow(dead_code)]
pub async fn app_state() -> AppState {
    app_state_with_secure_cookies(false).await
}

#[allow(dead_code)]
pub async fn app_state_with_secure_cookies(secure_cookies: bool) -> AppState {
    let pool = fresh_pool().await;
    let boards_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config").join("boards.toml");
    let boards = Board::load_from_file(&boards_path).expect("boards config should load");
    viceden::db::sync_boards(&pool, &boards).await.expect("seed boards");
    viceden::db::ensure_bootstrap_users(&pool).await.expect("seed bootstrap users");

    AppState {
        db: viceden::RequiredPgPool::new(pool.clone()),
        boards,
        repos: Repos::new(pool),
        login_throttle: viceden::cache::LoginThrottle::new(5, chrono::Duration::minutes(15)),
        mention_cache: viceden::cache::MentionSuggest::new(std::time::Duration::from_secs(60)),
        moderation: None,
        config: config::Settings {
            server: config::ServerSettings {
                bind: "127.0.0.1:3000".to_string(),
                max_request_body_bytes: 131072,
                request_timeout_seconds: 15,
                secure_cookies,
                proxy: Default::default(),
                rate_limit: Default::default(),
            },
            database: config::DatabaseSettings {
                url: "postgres://127.0.0.1/viceden_test".to_string(),
                require_connection: true,
                max_connections: 5,
            },
            auth: config::AuthSettings::default(),
            classifier: Default::default(),
            ops: Default::default(),
            boards_file: boards_path.to_string_lossy().to_string(),
        },
    }
}

#[allow(dead_code)]
pub async fn compat_store(state: &AppState) -> forum::ForumStore {
    let mut store = forum::ForumStore::try_with_state_path_and_pool(
        &state.boards,
        "/dev/null",
        state.db.as_ref().cloned(),
    )
    .await
    .expect("compatibility forum store should load from postgres");
    store.set_required_postgres(true);
    store
}

#[allow(dead_code)]
pub async fn fresh_pool() -> PgPool {
    let admin_url = pg_admin_url();
    let db_name = format!(
        "viceden_test_{}_{}",
        std::process::id(),
        DB_COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    assert!(
        db_name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
        "generated DB name should be identifier-safe"
    );
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin_url)
        .await
        .expect("connect to postgres admin database");
    sqlx::query(sqlx::AssertSqlSafe(format!(r#"CREATE DATABASE "{db_name}""#)))
        .execute(&admin_pool)
        .await
        .expect("create fresh test database");
    admin_pool.close().await;

    let url = database_url_with_db(&admin_url, &db_name);
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("connect to fresh test PG");
    sqlx::migrate!("./migrations").run(&pool).await.expect("migrations apply");
    pool
}

fn pg_admin_url() -> String {
    PG_ADMIN_URL
        .get_or_init(|| {
            std::env::var("VICEDEN_DATABASE__URL")
                .map(|url| database_url_with_db(&url, "postgres"))
                .unwrap_or_else(|_| start_test_postgres())
        })
        .clone()
}

fn start_test_postgres() -> String {
    cleanup_stale_test_postgres_roots();
    register_test_postgres_cleanup();

    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve postgres test port");
    let port = listener.local_addr().expect("test port addr").port();
    drop(listener);

    let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_string());
    let root = std::env::temp_dir().join(format!("viceden-pg-{}-{port}", Uuid::new_v4()));
    let data = root.join("data");
    fs::create_dir_all(&root).expect("create postgres test root");
    fs::write(root.join("owner.pid"), std::process::id().to_string()).expect("write owner pid");

    let init = Command::new(INITDB_BIN)
        .args(["-D", data.to_str().expect("data dir utf8"), "-A", "trust", "-U", &user])
        .status()
        .expect("run initdb");
    assert!(init.success(), "initdb failed");

    let log = root.join("postgres.log");
    let options = format!("-h 127.0.0.1 -p {port}");
    let start = Command::new(PG_CTL_BIN)
        .args([
            "-D",
            data.to_str().expect("data dir utf8"),
            "-l",
            log.to_str().expect("log path utf8"),
            "-o",
            &options,
            "-w",
            "start",
        ])
        .status()
        .expect("start test postgres");
    assert!(start.success(), "pg_ctl start failed");
    tracked_test_postgres_roots().lock().expect("pg roots lock").push(root);

    format!("postgres://{user}@127.0.0.1:{port}/postgres")
}

fn register_test_postgres_cleanup() {
    PG_CLEANUP_REGISTERED.call_once(|| unsafe {
        libc::atexit(stop_registered_test_postgres);
    });
}

extern "C" fn stop_registered_test_postgres() {
    if let Some(roots) = PG_ROOTS.get() {
        if let Ok(mut roots) = roots.lock() {
            for root in roots.drain(..) {
                stop_test_postgres_root(&root);
            }
        }
    }
}

fn tracked_test_postgres_roots() -> &'static Mutex<Vec<PathBuf>> {
    PG_ROOTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn cleanup_stale_test_postgres_roots() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("viceden-pg-") {
            continue;
        }
        if stale_test_postgres_root(&path) {
            stop_test_postgres_root(&path);
        }
    }
}

fn stale_test_postgres_root(root: &Path) -> bool {
    let pid_path = root.join("owner.pid");
    let Ok(pid_text) = fs::read_to_string(pid_path) else {
        return true;
    };
    let Ok(pid) = pid_text.trim().parse::<i32>() else {
        return true;
    };
    if pid <= 0 {
        return true;
    }
    let alive = unsafe { libc::kill(pid, 0) == 0 };
    if alive {
        return false;
    }
    !matches!(std::io::Error::last_os_error().raw_os_error(), Some(libc::EPERM))
}

fn stop_test_postgres_root(root: &Path) {
    let data = root.join("data");
    if data.join("postmaster.pid").exists() {
        let _ = Command::new(PG_CTL_BIN)
            .args(["-D", data.to_str().unwrap_or_default(), "-m", "immediate", "-w", "stop"])
            .status();
    }
    let _ = fs::remove_dir_all(root);
}

fn database_url_with_db(url: &str, db_name: &str) -> String {
    let (base, query) = url.split_once('?').map_or((url, ""), |(base, query)| (base, query));
    let (prefix, _) = base.rsplit_once('/').expect("postgres URL should contain database name");
    if query.is_empty() {
        format!("{prefix}/{db_name}")
    } else {
        format!("{prefix}/{db_name}?{query}")
    }
}
