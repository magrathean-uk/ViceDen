//! Application entry point.
//!
//! This file wires together configuration loading, database setup,
//! application state and the Axum router.  It exposes two main route
//! trees: `/api` for JSON endpoints and the root (`/`) for server-rendered
//! HTML pages.  The code is intentionally explicit and verbose so a
//! later work can extend it safely.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::Duration;
use tokio::{net::TcpListener, signal};
use tracing::{error, info, warn};

use viceden::{
    app,
    cache::{LoginThrottle, MentionSuggest},
    config, db, models,
    repository::Repos,
    services::{
        classifier::{anthropic::AnthropicClassifier, stub::StubClassifier, ContentClassifier},
        moderation_pipeline::{ModerationPipeline, PipelineConfig},
        moderation_workers::spawn_workers,
    },
    AppState, RequiredPgPool,
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    init_tracing();

    let settings = config::Settings::load()?;
    let pool = db::connect_pool(&settings.database).await?;

    let boards = models::Board::load_from_file(&settings.boards_file)?;
    db::sync_boards(&pool, &boards).await?;
    db::ensure_bootstrap_users(&pool).await?;
    app::apply_boot_policy(&pool, &settings).await.map_err(|error| {
        error!(error = %error, "boot_policy_violation");
        error
    })?;

    let classifier: Arc<dyn ContentClassifier> = if settings.classifier.provider == "stub" {
        Arc::new(StubClassifier::new())
    } else {
        let api_key = std::env::var(&settings.classifier.api_key_env).unwrap_or_default();
        Arc::new(AnthropicClassifier::new(api_key, settings.classifier.model.clone())?)
    };

    let moderation = Arc::new(ModerationPipeline::new(
        pool.clone(),
        classifier,
        PipelineConfig {
            sync_deadline: StdDuration::from_millis(settings.classifier.sync_deadline_ms),
            default_model: settings.classifier.model.clone(),
        },
    ));

    // Drop the worker join handles intentionally — the tasks are supervised by
    // tokio for the process lifetime.
    let _workers = spawn_workers(pool.clone(), moderation.clone(), settings.classifier.clone());

    let state = AppState {
        db: RequiredPgPool::new(pool.clone()),
        boards,
        config: settings.clone(),
        repos: Repos::new(pool),
        login_throttle: LoginThrottle::new(5, Duration::minutes(15)),
        mention_cache: MentionSuggest::new(StdDuration::from_secs(60)),
        moderation: Some(moderation),
    };

    let app = app::build_router(state);
    let addr: SocketAddr = settings.server.bind.parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!(bind = %addr, "starting viceden server");

    // Surface `ConnectInfo<SocketAddr>` to extractors so the client-IP
    // middleware can see the real peer address. Without this, the middleware
    // falls back to the `127.0.0.1:0` placeholder, which IS in the default
    // trusted-CIDR set, and any client can spoof `Cf-Connecting-Ip`.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("viceden=info,tower_http=info"));

    let fmt_layer = fmt::layer().with_target(false).compact();
    tracing_subscriber::registry().with(filter).with(fmt_layer).init();
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let ctrl_c = async {
            let _ = signal::ctrl_c().await;
        };

        let terminate = async {
            match signal(SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    sigterm.recv().await;
                }
                Err(error) => {
                    warn!(error = %error, "failed to install SIGTERM handler; falling back to CTRL+C only");
                    std::future::pending::<()>().await;
                }
            }
        };

        tokio::select! {
            _ = ctrl_c => {},
            _ = terminate => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = signal::ctrl_c().await;
    }

    info!("shutdown signal received");
}
