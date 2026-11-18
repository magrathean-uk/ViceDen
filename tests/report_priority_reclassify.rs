//! Integration test: a strict reclassify driven by a user report can flip a
//! visible post into quarantined when the classifier returns a Quarantine
//! verdict.
//!
//! Requires `VICEDEN_DATABASE__URL` to point at a Postgres with the viceden
//! migrations applied. Marked `#[ignore]` so the default `cargo test` run
//! stays green when no local PG is available.
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test report_priority_reclassify -- --ignored --test-threads=1
//! ```

mod common;

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;
use viceden::repository::posts::{ModState, NewPost, PostRepo};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};
use viceden::services::classifier::stub::StubClassifier;
use viceden::services::moderation_pipeline::{ModerationPipeline, PipelineConfig};

#[tokio::test]
#[ignore]
async fn strict_reclassify_can_flip_visible_to_quarantined() {
    let pool = common::shared_pool().await;
    let user = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: user,
                handle: format!("rp-{}", &user.simple().to_string()[..6]),
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
            &mut *tx,
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
            &mut *tx,
            NewPost {
                id: post,
                thread_id: thread,
                user_id: user,
                body: "trigger:quarantine please reconsider".into(),
                mod_state: ModState::Visible,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let pipeline = ModerationPipeline::new(
        pool.clone(),
        Arc::new(StubClassifier::new()),
        PipelineConfig { sync_deadline: Duration::from_millis(1500), default_model: "stub".into() },
    );

    pipeline.reclassify_post_strict(post, "trigger:quarantine please reconsider").await;

    let row = PostRepo.find_by_id(&pool, post).await.unwrap().unwrap();
    assert_eq!(row.mod_state, ModState::Quarantined);
}
