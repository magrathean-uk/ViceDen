//! Integration tests for the tags repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_tags -- --ignored --test-threads=1
//! ```

mod common;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::repository::tags::TagRepo;
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
                title: "tag-test".into(),
                expires_at: Utc::now() + Duration::days(7),
            },
        )
        .await
        .expect("seed thread");
    id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn list_for_thread_returns_tags_in_position_order() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = TagRepo;

    let user = seed_user(&mut tx, "tag").await;
    let thread = seed_thread(&mut tx, user).await;

    // Empty thread → no tags.
    assert!(repo.list_for_thread(&mut *tx, thread).await.unwrap().is_empty());

    repo.set_for_thread(&mut tx, thread, &["zeta", "alpha", "beta"]).await.unwrap();

    let tags = repo.list_for_thread(&mut *tx, thread).await.unwrap();
    // Inserted with positions 0..2 — should preserve insert order.
    assert_eq!(tags, vec!["zeta", "alpha", "beta"]);

    // set_for_thread is replace-not-merge: re-setting clears prior tags.
    repo.set_for_thread(&mut tx, thread, &["only"]).await.unwrap();
    assert_eq!(repo.list_for_thread(&mut *tx, thread).await.unwrap(), vec!["only"]);

    // Empty slice clears.
    repo.set_for_thread(&mut tx, thread, &[]).await.unwrap();
    assert!(repo.list_for_thread(&mut *tx, thread).await.unwrap().is_empty());

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn top_tags_orders_by_use_count_desc_then_alpha() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = TagRepo;

    let user = seed_user(&mut tx, "top").await;
    let t1 = seed_thread(&mut tx, user).await;
    let t2 = seed_thread(&mut tx, user).await;
    let t3 = seed_thread(&mut tx, user).await;

    // baseline counts may exist in shared DB; this test rolls back so it's safe.
    // popular: 3 uses. middle: 2 uses. rare: 1 use.
    repo.set_for_thread(&mut tx, t1, &["popular", "middle", "rare"]).await.unwrap();
    repo.set_for_thread(&mut tx, t2, &["popular", "middle"]).await.unwrap();
    repo.set_for_thread(&mut tx, t3, &["popular"]).await.unwrap();

    let top = repo.top_tags(&mut *tx, 10).await.unwrap();
    // Find each tag in the result and assert relative ordering. (Other tags
    // may appear from concurrent fixtures but ours should rank correctly.)
    let pos = |tag: &str| {
        top.iter().position(|u| u.tag == tag).unwrap_or_else(|| panic!("missing {tag}"))
    };
    assert!(pos("popular") < pos("middle"));
    assert!(pos("middle") < pos("rare"));

    let popular_uses = top.iter().find(|u| u.tag == "popular").unwrap().uses;
    let middle_uses = top.iter().find(|u| u.tag == "middle").unwrap().uses;
    let rare_uses = top.iter().find(|u| u.tag == "rare").unwrap().uses;
    assert_eq!(popular_uses, 3);
    assert_eq!(middle_uses, 2);
    assert_eq!(rare_uses, 1);

    // Limit clamps result.
    let top_one = repo.top_tags(&mut *tx, 1).await.unwrap();
    assert_eq!(top_one.len(), 1);
    assert_eq!(top_one[0].tag, "popular");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn set_for_thread_handles_position_updates() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let repo = TagRepo;

    let user = seed_user(&mut tx, "pos").await;
    let thread = seed_thread(&mut tx, user).await;

    // Insert a, b, c at positions 0, 1, 2.
    repo.set_for_thread(&mut tx, thread, &["a", "b", "c"]).await.unwrap();
    assert_eq!(repo.list_for_thread(&mut *tx, thread).await.unwrap(), vec!["a", "b", "c"]);

    // Re-set with reversed order — list reflects new positions.
    repo.set_for_thread(&mut tx, thread, &["c", "b", "a"]).await.unwrap();
    assert_eq!(repo.list_for_thread(&mut *tx, thread).await.unwrap(), vec!["c", "b", "a"]);

    tx.rollback().await.unwrap();
}
