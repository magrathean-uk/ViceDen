//! Integration tests for the thread-states repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_thread_states -- --ignored --test-threads=1
//! ```

mod common;

use std::collections::HashMap;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::repository::thread_states::{ThreadStateRepo, ThreadStateRow};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};

async fn seed_user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, prefix: &str) -> Uuid {
    let id = Uuid::new_v4();
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id,
                handle: format!("{prefix}-{}", &id.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .expect("seed user");
    id
}

async fn seed_thread(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    ThreadRepo
        .insert(
            &mut **tx,
            NewThread {
                id,
                board_slug: "general".into(),
                user_id,
                title: "ts-test".into(),
                expires_at: Utc::now() + Duration::days(7),
            },
        )
        .await
        .expect("seed thread");
    id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn state_for_returns_none_for_unannotated_thread() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = ThreadStateRepo;

    let user = seed_user(&mut tx, "ts").await;
    let thread = seed_thread(&mut tx, user).await;

    // Fresh thread: no annotation row → caller treats as default.
    assert!(repo.state_for(&mut *tx, thread).await.unwrap().is_none());

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn upsert_round_trips_all_flags() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = ThreadStateRepo;

    let user = seed_user(&mut tx, "up").await;
    let thread = seed_thread(&mut tx, user).await;

    let pinned_locked =
        ThreadStateRow { pinned: true, locked: true, answered: false, closed: false };
    repo.upsert(&mut *tx, thread, pinned_locked).await.unwrap();
    let row = repo.state_for(&mut *tx, thread).await.unwrap().unwrap();
    assert!(row.pinned);
    assert!(row.locked);
    assert!(!row.answered);
    assert!(!row.closed);

    // Re-upsert flips all flags — ON CONFLICT replaces, not merges.
    let answered_closed =
        ThreadStateRow { pinned: false, locked: false, answered: true, closed: true };
    repo.upsert(&mut *tx, thread, answered_closed).await.unwrap();
    let row = repo.state_for(&mut *tx, thread).await.unwrap().unwrap();
    assert_eq!(row, answered_closed);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn states_for_batch_returns_only_annotated_threads() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = ThreadStateRepo;

    let user = seed_user(&mut tx, "batch").await;
    let pinned_thread = seed_thread(&mut tx, user).await;
    let plain_thread = seed_thread(&mut tx, user).await;
    let missing = Uuid::new_v4();

    repo.upsert(
        &mut *tx,
        pinned_thread,
        ThreadStateRow { pinned: true, locked: false, answered: false, closed: false },
    )
    .await
    .unwrap();

    let batch =
        repo.states_for_batch(&mut *tx, &[pinned_thread, plain_thread, missing]).await.unwrap();
    let map: HashMap<Uuid, ThreadStateRow> = batch.into_iter().collect();

    // Only pinned_thread has an annotation row.
    assert_eq!(map.len(), 1);
    assert!(map.get(&pinned_thread).unwrap().pinned);
    assert!(!map.contains_key(&plain_thread));
    assert!(!map.contains_key(&missing));

    // Empty input short-circuits.
    let empty = repo.states_for_batch(&mut *tx, &[]).await.unwrap();
    assert!(empty.is_empty());

    tx.rollback().await.unwrap();
}
