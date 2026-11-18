//! Integration tests for the preferences repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_preferences -- --ignored --test-threads=1
//! ```

mod common;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::repository::preferences::PreferencesRepo;
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
                title: "prefs-test".into(),
                expires_at: Utc::now() + Duration::days(7),
            },
        )
        .await
        .expect("seed thread");
    id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn is_thread_saved_round_trips() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = PreferencesRepo;

    let user = seed_user(&mut tx, "save").await;
    let thread = seed_thread(&mut tx, user).await;

    assert!(!repo.is_thread_saved(&mut *tx, user, thread).await.unwrap());

    repo.save_thread(&mut *tx, user, thread).await.unwrap();
    assert!(repo.is_thread_saved(&mut *tx, user, thread).await.unwrap());

    repo.unsave_thread(&mut *tx, user, thread).await.unwrap();
    assert!(!repo.is_thread_saved(&mut *tx, user, thread).await.unwrap());

    // A different user reading the same thread sees their own (absent) save.
    let other = seed_user(&mut tx, "other").await;
    repo.save_thread(&mut *tx, user, thread).await.unwrap();
    assert!(!repo.is_thread_saved(&mut *tx, other, thread).await.unwrap());

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn is_thread_hidden_round_trips() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = PreferencesRepo;

    let user = seed_user(&mut tx, "hide").await;
    let thread = seed_thread(&mut tx, user).await;

    assert!(!repo.is_thread_hidden(&mut *tx, user, thread).await.unwrap());

    repo.hide_thread(&mut *tx, user, thread).await.unwrap();
    assert!(repo.is_thread_hidden(&mut *tx, user, thread).await.unwrap());

    repo.unhide_thread(&mut *tx, user, thread).await.unwrap();
    assert!(!repo.is_thread_hidden(&mut *tx, user, thread).await.unwrap());

    // Save and hide are independent dimensions.
    repo.hide_thread(&mut *tx, user, thread).await.unwrap();
    repo.save_thread(&mut *tx, user, thread).await.unwrap();
    assert!(repo.is_thread_hidden(&mut *tx, user, thread).await.unwrap());
    assert!(repo.is_thread_saved(&mut *tx, user, thread).await.unwrap());

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn is_board_subscribed_round_trips() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = PreferencesRepo;

    let user = seed_user(&mut tx, "sub").await;

    assert!(!repo.is_board_subscribed(&mut *tx, user, "general").await.unwrap());

    repo.subscribe_board(&mut *tx, user, "general").await.unwrap();
    assert!(repo.is_board_subscribed(&mut *tx, user, "general").await.unwrap());

    // Idempotent re-subscribe.
    repo.subscribe_board(&mut *tx, user, "general").await.unwrap();
    assert!(repo.is_board_subscribed(&mut *tx, user, "general").await.unwrap());

    repo.unsubscribe_board(&mut *tx, user, "general").await.unwrap();
    assert!(!repo.is_board_subscribed(&mut *tx, user, "general").await.unwrap());

    // Different slug → independent.
    repo.subscribe_board(&mut *tx, user, "general").await.unwrap();
    assert!(!repo.is_board_subscribed(&mut *tx, user, "drugs").await.unwrap());

    tx.rollback().await.unwrap();
}
