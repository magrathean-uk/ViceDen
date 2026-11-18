//! Background moderation workers — supervised tokio tasks.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::ClassifierSettings;
use crate::repository::posts::{ModState, PostRepo};
use crate::services::moderation_pipeline::{ModerationPipeline, PipelineOutcome};

pub struct WorkerHandles {
    pub pending: JoinHandle<()>,
    pub audit: JoinHandle<()>,
}

pub fn spawn_workers(
    pool: PgPool,
    pipeline: Arc<ModerationPipeline>,
    config: ClassifierSettings,
) -> WorkerHandles {
    let pending =
        tokio::spawn(pending_reclassify_loop(pool.clone(), pipeline.clone(), config.clone()));
    let audit = tokio::spawn(daily_audit_loop(pool, pipeline, config));
    WorkerHandles { pending, audit }
}

async fn pending_reclassify_loop(
    pool: PgPool,
    pipeline: Arc<ModerationPipeline>,
    config: ClassifierSettings,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(config.worker_interval_s));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(config.worker_max_inflight));

    loop {
        ticker.tick().await;

        let cutoff = Utc::now() - chrono::Duration::seconds(config.worker_interval_s as i64);
        let candidates = match PostRepo.find_pending_older_than(&pool, cutoff, 50).await {
            Ok(rows) => rows,
            Err(error) => {
                warn!(?error, "pending_reclassify: query failed");
                continue;
            }
        };

        for row in candidates {
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => continue,
            };
            let pipeline = pipeline.clone();
            let pool_clone = pool.clone();
            let max_retries = config.worker_max_retries;
            tokio::spawn(async move {
                let _permit = permit;
                let outcome = pipeline.reclassify_post_strict(row.id, &row.body).await;
                if let PipelineOutcome::Pending = outcome {
                    // Track attempts via audit count; if too many, force quarantine.
                    let attempts: (i64,) = sqlx::query_as(
                        "SELECT count(*) FROM content_classifications
                         WHERE target_kind='post' AND target_id=$1 AND verdict='error'",
                    )
                    .bind(row.id)
                    .fetch_one(&pool_clone)
                    .await
                    .unwrap_or((0,));
                    if attempts.0 as u32 >= max_retries {
                        PostRepo
                            .update_mod_state(
                                &pool_clone,
                                row.id,
                                ModState::Quarantined,
                                Some("classifier_unreachable"),
                            )
                            .await
                            .ok();
                        warn!(post_id = %row.id, "post auto-quarantined after retries exhausted");
                    }
                }
            });
        }
    }
}

async fn daily_audit_loop(
    pool: PgPool,
    pipeline: Arc<ModerationPipeline>,
    config: ClassifierSettings,
) {
    let mut ticker = tokio::time::interval(Duration::from_secs(60 * 60 * 24));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip the immediate first tick.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        info!("daily_audit: starting sample re-classification");
        let sample = match PostRepo
            .random_visible_sample(
                &pool,
                config.audit_lookback_days as i32,
                config.audit_sample_size as i64,
            )
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                warn!(?error, "daily_audit: sample query failed");
                continue;
            }
        };
        for row in sample {
            let _ = pipeline.reclassify_post_strict(row.id, &row.body).await;
        }
    }
}
