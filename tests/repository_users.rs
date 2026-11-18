//! Integration tests for the users repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_users -- --ignored --test-threads=1
//! ```

mod common;

use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;
use viceden::repository::posts::{ModState, NewPost, PostRepo};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn insert_and_lookup_by_handle() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.expect("begin tx");
    let repo = UserRepo;

    let user_id = Uuid::new_v4();
    let handle = format!("test-{}", &user_id.simple().to_string()[..8]);
    repo.insert(
        &mut *tx,
        NewUser {
            id: user_id,
            handle: handle.clone(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_string(),
            role: "member".to_string(),
            last_known_ip: Some("127.0.0.1".to_string()),
        },
    )
    .await
    .expect("insert");

    let found = repo.find_by_handle(&mut *tx, &handle).await.expect("lookup");
    assert!(found.is_some());
    assert_eq!(found.unwrap().handle, handle);

    tx.rollback().await.expect("rollback");
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn cauterise_marks_user_deactivated() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.expect("begin tx");
    let repo = UserRepo;

    let actor = insert_user(&mut tx, "actor").await;
    let target = insert_user(&mut tx, "target").await;
    repo.cauterise(&mut *tx, target, actor, "spam wave").await.expect("cauterise");

    let row = repo.find_by_id(&mut *tx, target).await.expect("find").unwrap();
    assert!(row.deactivated_at.is_some());
    assert_eq!(row.deactivated_by, Some(actor));
    assert_eq!(row.deactivated_reason.as_deref(), Some("spam wave"));

    tx.rollback().await.expect("rollback");
}

async fn insert_user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, prefix: &str) -> Uuid {
    insert_user_with_role(tx, prefix, "member").await
}

async fn insert_user_with_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    prefix: &str,
    role: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let prefix = prefix.chars().take(7).collect::<String>();
    let handle = format!("{prefix}-{}", &id.simple().to_string()[..8]);
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id,
                handle,
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_string(),
                role: role.to_string(),
                last_known_ip: None,
            },
        )
        .await
        .expect("insert helper");
    id
}

async fn ban_user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, user_id: Uuid) {
    sqlx::query("UPDATE users SET banned = TRUE, ban_reason = 'test' WHERE id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .expect("ban update");
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
async fn handle_batch_lookup_returns_present_handles_only() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = UserRepo;

    let alice = insert_user(&mut tx, "alice").await;
    let bob = insert_user(&mut tx, "bob").await;
    let missing = Uuid::new_v4();

    let rows = repo.handle_batch_lookup(&mut *tx, &[alice, bob, missing]).await.unwrap();

    let map: HashMap<Uuid, String> = rows.into_iter().collect();
    assert_eq!(map.len(), 2);
    assert!(map.contains_key(&alice));
    assert!(map.contains_key(&bob));
    assert!(!map.contains_key(&missing));

    // Empty input short-circuits without a SQL round-trip.
    let empty = repo.handle_batch_lookup(&mut *tx, &[]).await.unwrap();
    assert!(empty.is_empty());

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn count_and_count_banned_track_inserts() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = UserRepo;

    let baseline_total = repo.count(&mut *tx).await.unwrap();
    let baseline_banned = repo.count_banned(&mut *tx).await.unwrap();

    let _ok = insert_user(&mut tx, "ok").await;
    let bad = insert_user(&mut tx, "bad").await;
    ban_user(&mut tx, bad).await;

    assert_eq!(repo.count(&mut *tx).await.unwrap(), baseline_total + 2);
    assert_eq!(repo.count_banned(&mut *tx).await.unwrap(), baseline_banned + 1);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn list_paged_orders_newest_first() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = UserRepo;

    let baseline = repo.count(&mut *tx).await.unwrap();
    let first = insert_user(&mut tx, "first").await;
    let second = insert_user(&mut tx, "second").await;
    let third = insert_user(&mut tx, "third").await;
    sqlx::query(
        "UPDATE users
         SET created_at = CASE id
             WHEN $1 THEN now() + interval '1 second'
             WHEN $2 THEN now() + interval '2 seconds'
             WHEN $3 THEN now() + interval '3 seconds'
         END
         WHERE id IN ($1, $2, $3)",
    )
    .bind(first)
    .bind(second)
    .bind(third)
    .execute(&mut *tx)
    .await
    .unwrap();

    let page = repo.list_paged(&mut *tx, 3, 0).await.unwrap();
    assert_eq!(page.len(), 3, "limit=3 returns 3 rows");
    // newest first: third → second → first
    assert_eq!(page[0].id, third);
    assert_eq!(page[1].id, second);
    assert_eq!(page[2].id, first);

    let offset_one = repo.list_paged(&mut *tx, 1, 1).await.unwrap();
    assert_eq!(offset_one.len(), 1);
    assert_eq!(offset_one[0].id, second);

    let total = repo.count(&mut *tx).await.unwrap();
    assert_eq!(total, baseline + 3);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn summary_with_counts_aggregates_threads_and_posts() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = UserRepo;

    let user = insert_user_with_role(&mut tx, "summary", "founder").await;
    let _t1 = insert_thread(&mut tx, user, "first thread").await;
    let t2 = insert_thread(&mut tx, user, "second thread").await;
    let _p1 = insert_post(&mut tx, t2, user, "reply one").await;
    let _p2 = insert_post(&mut tx, t2, user, "reply two").await;

    let summary = repo.summary_with_counts(&mut *tx, user).await.unwrap().unwrap();
    assert_eq!(summary.id, user);
    assert_eq!(summary.role, "founder");
    assert!(!summary.banned);
    assert_eq!(summary.thread_count, 2);
    assert_eq!(summary.post_count, 2);

    // Unknown id → None.
    let missing = repo.summary_with_counts(&mut *tx, Uuid::new_v4()).await.unwrap();
    assert!(missing.is_none());

    // User with no activity → counts default to 0 (LEFT JOIN COALESCE).
    let bystander = insert_user(&mut tx, "bystander").await;
    let zeros = repo.summary_with_counts(&mut *tx, bystander).await.unwrap().unwrap();
    assert_eq!(zeros.thread_count, 0);
    assert_eq!(zeros.post_count, 0);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn latest_activity_returns_recent_posts_with_thread_meta() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = UserRepo;

    let user = insert_user(&mut tx, "active").await;
    let other = insert_user(&mut tx, "other").await;
    let thread = insert_thread(&mut tx, user, "topic title").await;
    let p_old = insert_post(&mut tx, thread, user, "old body").await;
    let p_new = insert_post(&mut tx, thread, user, "newer body").await;
    sqlx::query(
        "UPDATE posts
         SET created_at = CASE id
             WHEN $1 THEN now() - interval '2 seconds'
             WHEN $2 THEN now() - interval '1 second'
         END
         WHERE id IN ($1, $2)",
    )
    .bind(p_old)
    .bind(p_new)
    .execute(&mut *tx)
    .await
    .unwrap();
    // Post by another user — should be excluded.
    let _foreign = insert_post(&mut tx, thread, other, "noise").await;

    let activity = repo.latest_activity(&mut *tx, user, 10).await.unwrap();
    assert_eq!(activity.len(), 2);
    // Newest first.
    assert_eq!(activity[0].post_id, p_new);
    assert_eq!(activity[1].post_id, p_old);
    assert_eq!(activity[0].thread_id, thread);
    assert_eq!(activity[0].thread_title, "topic title");
    assert_eq!(activity[0].board_slug, "general");
    assert_eq!(activity[0].body_snippet, "newer body");

    // Limit clamps result set.
    let limited = repo.latest_activity(&mut *tx, user, 1).await.unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].post_id, p_new);

    // Body snippet truncates to 90 chars.
    let long_body = "x".repeat(200);
    let long = insert_post(&mut tx, thread, user, &long_body).await;
    sqlx::query("UPDATE posts SET created_at = now() WHERE id = $1")
        .bind(long)
        .execute(&mut *tx)
        .await
        .unwrap();
    let with_long = repo.latest_activity(&mut *tx, user, 1).await.unwrap();
    assert_eq!(with_long[0].body_snippet.chars().count(), 90);

    tx.rollback().await.unwrap();
}
