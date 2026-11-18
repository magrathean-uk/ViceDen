//! Integration tests for the threads repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_threads -- --ignored --test-threads=1
//! ```

mod common;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::repository::posts::{ModState, NewPost, PostRepo};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};
use viceden::repository::votes::VoteRepo;

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn insert_thread_defaults_to_visible() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let user = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: user,
                handle: format!("th-{}", &user.simple().to_string()[..8]),
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
                title: "hi".into(),
                expires_at: Utc::now() + Duration::days(7),
            },
        )
        .await
        .unwrap();

    let row = ThreadRepo.find_by_id(&mut *tx, thread).await.unwrap().unwrap();
    assert_eq!(row.mod_state, ModState::Visible);
    tx.rollback().await.unwrap();
}

async fn insert_user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, prefix: &str) -> Uuid {
    let id = Uuid::new_v4();
    let handle = format!("{prefix}-{}", &id.simple().to_string()[..8]);
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id,
                handle,
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .expect("user insert");
    id
}

async fn insert_thread(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    title: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    ThreadRepo
        .insert(
            &mut **tx,
            NewThread {
                id,
                board_slug: "general".into(),
                user_id,
                title: title.into(),
                expires_at: Utc::now() + Duration::days(7),
            },
        )
        .await
        .expect("thread insert");
    id
}

async fn insert_post(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: Uuid,
    user_id: Uuid,
    body: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    PostRepo
        .insert(
            &mut **tx,
            NewPost { id, thread_id, user_id, body: body.into(), mod_state: ModState::Visible },
        )
        .await
        .expect("post insert");
    id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn count_tracks_inserts() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let baseline = ThreadRepo.count(&mut *tx).await.unwrap();

    let author = insert_user(&mut tx, "ct").await;
    let _t1 = insert_thread(&mut tx, author, "first").await;
    let _t2 = insert_thread(&mut tx, author, "second").await;

    assert_eq!(ThreadRepo.count(&mut *tx).await.unwrap(), baseline + 2);
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn find_with_preview_returns_score_replies_and_last_activity() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let author = insert_user(&mut tx, "auth").await;
    let voter1 = insert_user(&mut tx, "v1").await;
    let voter2 = insert_user(&mut tx, "v2").await;
    let thread = insert_thread(&mut tx, author, "preview-target").await;
    let root_post = insert_post(&mut tx, thread, author, "root body").await;
    let reply_a = insert_post(&mut tx, thread, voter1, "reply a").await;
    let reply_b = insert_post(&mut tx, thread, voter2, "reply b").await;

    // Score = +1 (root) +1 (reply_a) -1 (reply_b) = +1.
    VoteRepo.upsert(&mut *tx, voter1, root_post, 1).await.unwrap();
    VoteRepo.upsert(&mut *tx, voter2, reply_a, 1).await.unwrap();
    VoteRepo.upsert(&mut *tx, author, reply_b, -1).await.unwrap();

    let preview =
        ThreadRepo.find_with_preview(&mut *tx, thread).await.unwrap().expect("preview row exists");

    assert_eq!(preview.thread.id, thread);
    assert_eq!(preview.thread.title, "preview-target");
    assert_eq!(preview.thread.mod_state, ModState::Visible);
    assert_eq!(preview.score, 1, "vote sum +1+1-1 = 1");
    assert_eq!(preview.reply_count, 2, "3 posts minus root = 2");
    // last_activity is the most-recent post timestamp, not the thread's.
    assert!(preview.last_activity >= preview.thread.created_at);

    // Unknown id → None.
    let missing = ThreadRepo.find_with_preview(&mut *tx, Uuid::new_v4()).await.unwrap();
    assert!(missing.is_none());

    // Empty thread (no posts, no votes) → score=0, reply_count=0,
    // last_activity falls back to thread.created_at.
    let empty = insert_thread(&mut tx, author, "empty").await;
    let empty_preview = ThreadRepo.find_with_preview(&mut *tx, empty).await.unwrap().unwrap();
    assert_eq!(empty_preview.score, 0);
    assert_eq!(empty_preview.reply_count, 0);
    assert_eq!(empty_preview.last_activity, empty_preview.thread.created_at);

    tx.rollback().await.unwrap();
}
