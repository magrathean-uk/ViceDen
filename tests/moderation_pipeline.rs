//! Integration tests for the moderation pipeline.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test moderation_pipeline -- --ignored --test-threads=1
//! ```

mod common;

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;
use viceden::repository::posts::{ModState, NewPost, PostRepo};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};
use viceden::services::classifier::stub::StubClassifier;
use viceden::services::moderation_pipeline::{ModerationPipeline, PipelineConfig, PipelineOutcome};

async fn seed(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> (Uuid, Uuid, Uuid) {
    let user = Uuid::new_v4();
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id: user,
                handle: format!("p-{}", &user.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();

    let thread = Uuid::new_v4();
    ThreadRepo
        .insert(
            &mut **tx,
            NewThread {
                id: thread,
                board_slug: "general".into(),
                user_id: user,
                title: "x".into(),
                expires_at: chrono::Utc::now() + chrono::Duration::days(7),
            },
        )
        .await
        .unwrap();

    let post = Uuid::new_v4();
    PostRepo
        .insert(
            &mut **tx,
            NewPost {
                id: post,
                thread_id: thread,
                user_id: user,
                body: "ordinary body".into(),
                mod_state: ModState::Pending,
            },
        )
        .await
        .unwrap();
    (user, thread, post)
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn allow_promotes_pending_to_visible() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let (_user, _thread, post) = seed(&mut tx).await;
    tx.commit().await.unwrap();

    let pipeline = ModerationPipeline::new(
        pool.clone(),
        Arc::new(StubClassifier::new()),
        PipelineConfig { sync_deadline: Duration::from_millis(1500), default_model: "stub".into() },
    );

    let outcome = pipeline.classify_post(post, "ordinary body").await;
    assert_eq!(outcome, PipelineOutcome::Allowed);

    let row = PostRepo.find_by_id(&pool, post).await.unwrap().unwrap();
    assert_eq!(row.mod_state, ModState::Visible);
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn reject_marks_post_removed() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let (_user, _thread, post) = seed(&mut tx).await;
    tx.commit().await.unwrap();

    let pipeline = ModerationPipeline::new(
        pool.clone(),
        Arc::new(StubClassifier::new()),
        PipelineConfig { sync_deadline: Duration::from_millis(1500), default_model: "stub".into() },
    );

    let outcome = pipeline.classify_post(post, "trigger:reject").await;
    assert_eq!(outcome, PipelineOutcome::Rejected);

    let row = PostRepo.find_by_id(&pool, post).await.unwrap().unwrap();
    assert_eq!(row.mod_state, ModState::Removed);
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn timeout_leaves_post_pending() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let (_user, _thread, post) = seed(&mut tx).await;
    tx.commit().await.unwrap();

    let pipeline = ModerationPipeline::new(
        pool.clone(),
        Arc::new(StubClassifier::new()),
        PipelineConfig { sync_deadline: Duration::from_millis(50), default_model: "stub".into() },
    );

    let outcome = pipeline.classify_post(post, "trigger:timeout body").await;
    assert!(matches!(outcome, PipelineOutcome::Pending));

    let row = PostRepo.find_by_id(&pool, post).await.unwrap().unwrap();
    assert_eq!(row.mod_state, ModState::Pending);
}
