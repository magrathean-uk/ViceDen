//! Integration tests for the posts repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_posts -- --ignored --test-threads=1
//! ```

mod common;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::repository::posts::{ModState, NewPost, PostRepo};
use viceden::repository::threads::{NewThread, ThreadRepo};
use viceden::repository::users::{NewUser, UserRepo};
use viceden::repository::votes::VoteRepo;

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
    let thread_id = Uuid::new_v4();
    ThreadRepo
        .insert(
            &mut **tx,
            NewThread {
                id: thread_id,
                board_slug: "general".into(),
                user_id,
                title: "test thread".into(),
                expires_at: Utc::now() + Duration::days(30),
            },
        )
        .await
        .expect("seed thread");
    thread_id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn insert_pending_then_promote_to_visible() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let user = seed_user(&mut tx, "p1").await;
    let thread = seed_thread(&mut tx, user).await;

    let post_id = Uuid::new_v4();
    PostRepo
        .insert(
            &mut *tx,
            NewPost {
                id: post_id,
                thread_id: thread,
                user_id: user,
                body: "first post".into(),
                mod_state: ModState::Pending,
            },
        )
        .await
        .unwrap();

    PostRepo.update_mod_state(&mut *tx, post_id, ModState::Visible, None).await.unwrap();
    let row = PostRepo.find_by_id(&mut *tx, post_id).await.unwrap().unwrap();
    assert_eq!(row.mod_state, ModState::Visible);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn visible_posts_for_thread_filters_by_state_and_viewer() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let author = seed_user(&mut tx, "auth").await;
    let bystander = seed_user(&mut tx, "bys").await;
    let founder = seed_user(&mut tx, "fnd").await;
    sqlx::query("UPDATE users SET role='founder' WHERE id=$1")
        .bind(founder)
        .execute(&mut *tx)
        .await
        .unwrap();

    let thread = seed_thread(&mut tx, author).await;

    for (kind, state) in [
        ("vis", ModState::Visible),
        ("pen", ModState::Pending),
        ("qua", ModState::Quarantined),
        ("rem", ModState::Removed),
    ] {
        PostRepo
            .insert(
                &mut *tx,
                NewPost {
                    id: Uuid::new_v4(),
                    thread_id: thread,
                    user_id: author,
                    body: kind.into(),
                    mod_state: state,
                },
            )
            .await
            .unwrap();
    }

    let bystander_view =
        PostRepo.visible_posts_for_thread(&mut *tx, thread, bystander).await.unwrap();
    assert_eq!(bystander_view.len(), 1);
    assert_eq!(bystander_view[0].body, "vis");

    let author_view = PostRepo.visible_posts_for_thread(&mut *tx, thread, author).await.unwrap();
    assert_eq!(author_view.len(), 3, "author sees pending+visible+quarantined, not removed");

    let founder_view = PostRepo.visible_posts_for_thread(&mut *tx, thread, founder).await.unwrap();
    assert_eq!(founder_view.len(), 4, "founder sees everything");

    tx.rollback().await.unwrap();
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
async fn count_tracks_inserts_across_states() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let baseline = PostRepo.count(&mut *tx).await.unwrap();

    let user = seed_user(&mut tx, "ct").await;
    let thread = seed_thread(&mut tx, user).await;
    let _v = insert_post(&mut tx, thread, user, "visible").await;
    PostRepo
        .insert(
            &mut *tx,
            NewPost {
                id: Uuid::new_v4(),
                thread_id: thread,
                user_id: user,
                body: "pending".into(),
                mod_state: ModState::Pending,
            },
        )
        .await
        .unwrap();
    PostRepo
        .insert(
            &mut *tx,
            NewPost {
                id: Uuid::new_v4(),
                thread_id: thread,
                user_id: user,
                body: "removed".into(),
                mod_state: ModState::Removed,
            },
        )
        .await
        .unwrap();

    // count() includes every mod_state — total rows on disk.
    assert_eq!(PostRepo.count(&mut *tx).await.unwrap(), baseline + 3);
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn participant_count_dedups_authors() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let alice = seed_user(&mut tx, "alice").await;
    let bob = seed_user(&mut tx, "bob").await;
    let thread = seed_thread(&mut tx, alice).await;

    // Empty thread → 0.
    assert_eq!(PostRepo.participant_count(&mut *tx, thread).await.unwrap(), 0);

    insert_post(&mut tx, thread, alice, "a1").await;
    insert_post(&mut tx, thread, alice, "a2").await;
    assert_eq!(PostRepo.participant_count(&mut *tx, thread).await.unwrap(), 1);

    insert_post(&mut tx, thread, bob, "b1").await;
    insert_post(&mut tx, thread, bob, "b2").await;
    assert_eq!(PostRepo.participant_count(&mut *tx, thread).await.unwrap(), 2);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn first_post_for_thread_returns_chronologically_earliest() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let user = seed_user(&mut tx, "first").await;
    let thread = seed_thread(&mut tx, user).await;

    // Empty thread → None.
    assert!(PostRepo.first_post_for_thread(&mut *tx, thread).await.unwrap().is_none());

    let root = insert_post(&mut tx, thread, user, "root").await;
    sqlx::query("UPDATE posts SET created_at = now() - interval '2 seconds' WHERE id = $1")
        .bind(root)
        .execute(&mut *tx)
        .await
        .unwrap();
    let _reply1 = insert_post(&mut tx, thread, user, "reply 1").await;
    let _reply2 = insert_post(&mut tx, thread, user, "reply 2").await;

    let first = PostRepo.first_post_for_thread(&mut *tx, thread).await.unwrap().unwrap();
    assert_eq!(first.id, root, "earliest insert wins");
    assert_eq!(first.body, "root");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn user_vote_for_post_round_trips_value() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let voter = seed_user(&mut tx, "vot").await;
    let author = seed_user(&mut tx, "auth").await;
    let thread = seed_thread(&mut tx, author).await;
    let post = insert_post(&mut tx, thread, author, "body").await;

    // No vote yet → 0.
    assert_eq!(VoteRepo.user_vote_for_post(&mut *tx, voter, post).await.unwrap(), 0);

    VoteRepo.upsert(&mut *tx, voter, post, 1).await.unwrap();
    assert_eq!(VoteRepo.user_vote_for_post(&mut *tx, voter, post).await.unwrap(), 1);

    VoteRepo.upsert(&mut *tx, voter, post, -1).await.unwrap();
    assert_eq!(VoteRepo.user_vote_for_post(&mut *tx, voter, post).await.unwrap(), -1);

    // A different user reading the same post sees their own (absent) vote.
    let other = seed_user(&mut tx, "other").await;
    assert_eq!(VoteRepo.user_vote_for_post(&mut *tx, other, post).await.unwrap(), 0);

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn scores_for_thread_is_one_query_per_post_score() {
    use std::collections::HashMap;
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let author = seed_user(&mut tx, "sft").await;
    let voter1 = seed_user(&mut tx, "v1").await;
    let voter2 = seed_user(&mut tx, "v2").await;
    let thread = seed_thread(&mut tx, author).await;
    let p1 = insert_post(&mut tx, thread, author, "p1").await;
    let p2 = insert_post(&mut tx, thread, author, "p2").await;
    let p3 = insert_post(&mut tx, thread, author, "p3").await;

    // Empty before any votes — every post is absent (caller defaults to 0).
    let baseline = VoteRepo.scores_for_thread(&mut *tx, thread).await.unwrap();
    assert!(baseline.iter().all(|(_, v)| *v == 0));

    VoteRepo.upsert(&mut *tx, voter1, p1, 1).await.unwrap();
    VoteRepo.upsert(&mut *tx, voter2, p1, 1).await.unwrap();
    VoteRepo.upsert(&mut *tx, voter1, p2, -1).await.unwrap();
    // p3 has no votes — should be absent.

    let map: HashMap<_, _> =
        VoteRepo.scores_for_thread(&mut *tx, thread).await.unwrap().into_iter().collect();
    assert_eq!(map.get(&p1).copied(), Some(2));
    assert_eq!(map.get(&p2).copied(), Some(-1));
    assert!(!map.contains_key(&p3), "post with no votes is absent");

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn user_votes_for_thread_is_per_post_for_one_viewer() {
    use std::collections::HashMap;
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let author = seed_user(&mut tx, "uvt").await;
    let viewer = seed_user(&mut tx, "view").await;
    let other = seed_user(&mut tx, "oth").await;
    let thread = seed_thread(&mut tx, author).await;
    let p1 = insert_post(&mut tx, thread, author, "p1").await;
    let p2 = insert_post(&mut tx, thread, author, "p2").await;
    let p3 = insert_post(&mut tx, thread, author, "p3").await;

    VoteRepo.upsert(&mut *tx, viewer, p1, 1).await.unwrap();
    VoteRepo.upsert(&mut *tx, viewer, p2, -1).await.unwrap();
    // viewer didn't vote on p3 — should be absent.
    // Other user voted on p3 — must NOT bleed into viewer's map.
    VoteRepo.upsert(&mut *tx, other, p3, 1).await.unwrap();

    let map: HashMap<_, _> = VoteRepo
        .user_votes_for_thread(&mut *tx, viewer, thread)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(map.get(&p1).copied(), Some(1));
    assert_eq!(map.get(&p2).copied(), Some(-1));
    assert!(!map.contains_key(&p3), "viewer's absent vote stays absent");
    assert_eq!(map.len(), 2, "other user's vote does not leak");

    tx.rollback().await.unwrap();
}
