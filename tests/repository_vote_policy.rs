//! Vote policy tests require a live Postgres and are ignored by default.

mod common;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::{
    models::Board,
    services::voting_policy::{cast_vote, VotePolicyError},
};

fn board(slug: &str) -> Board {
    Board {
        slug: slug.to_string(),
        name: slug.to_string(),
        ttl_days: 30,
        description: String::new(),
    }
}

async fn seed_target(pool: &sqlx::PgPool, slug: &str) -> (Uuid, Uuid, Uuid) {
    let author = Uuid::new_v4();
    let voter = Uuid::new_v4();
    let thread = Uuid::new_v4();
    let post = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO boards (slug, name, ttl_days, description, downvotes_enabled)
         VALUES ($1, $2, 30, '', TRUE)",
    )
    .bind(slug)
    .bind(slug)
    .execute(pool)
    .await
    .expect("board");
    for (id, handle) in [(author, "author"), (voter, "voter")] {
        sqlx::query(
            "INSERT INTO users (id, handle, password_hash, role)
             VALUES ($1, $2, 'hash', 'member')",
        )
        .bind(id)
        .bind(format!("{handle}-{}", &id.simple().to_string()[..8]))
        .execute(pool)
        .await
        .expect("user");
    }
    sqlx::query(
        "INSERT INTO threads (id, board_slug, user_id, title, expires_at)
         VALUES ($1, $2, $3, 'thread', $4)",
    )
    .bind(thread)
    .bind(slug)
    .bind(author)
    .bind(Utc::now() + Duration::days(30))
    .execute(pool)
    .await
    .expect("thread");
    sqlx::query(
        "INSERT INTO posts (id, thread_id, user_id, body, mod_state, mod_state_changed_at)
         VALUES ($1, $2, $3, 'body', 'visible', now())",
    )
    .bind(post)
    .bind(thread)
    .bind(author)
    .execute(pool)
    .await
    .expect("post");
    (voter, thread, post)
}

async fn cleanup(pool: &sqlx::PgPool, slug: &str) {
    let _ = sqlx::query(
        "DELETE FROM votes WHERE post_id IN (
             SELECT p.id FROM posts p JOIN threads t ON t.id = p.thread_id WHERE t.board_slug = $1
         )",
    )
    .bind(slug)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM boards WHERE slug = $1").bind(slug).execute(pool).await;
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at throwaway Postgres"]
async fn same_vote_clears_and_missing_config_board_fails() {
    let pool = common::shared_pool().await;
    let slug = format!("vote-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (voter, thread, post) = seed_target(&pool, &slug).await;

    let first = cast_vote(&pool, &[board(&slug)], voter, post, 1).await.expect("upvote");
    assert_eq!(first.thread_id, thread);
    assert_eq!(first.my_vote, 1);
    assert_eq!(first.score, 1);

    let second = cast_vote(&pool, &[board(&slug)], voter, post, 1).await.expect("clear");
    assert_eq!(second.my_vote, 0);
    assert_eq!(second.score, 0);

    let error = cast_vote(&pool, &[], voter, post, 1).await.expect_err("missing board");
    assert_eq!(error, VotePolicyError::BoardNotFound);

    cleanup(&pool, &slug).await;
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at throwaway Postgres"]
async fn banned_users_and_disabled_downvotes_are_rejected() {
    let pool = common::shared_pool().await;
    let slug = format!("vote-{}", &Uuid::new_v4().simple().to_string()[..8]);
    let (voter, _thread, post) = seed_target(&pool, &slug).await;

    sqlx::query("UPDATE users SET banned = TRUE WHERE id = $1")
        .bind(voter)
        .execute(&pool)
        .await
        .expect("ban voter");
    let banned = cast_vote(&pool, &[board(&slug)], voter, post, 1).await.expect_err("banned");
    assert_eq!(banned, VotePolicyError::UserCannotVote);

    sqlx::query("UPDATE users SET banned = FALSE WHERE id = $1")
        .bind(voter)
        .execute(&pool)
        .await
        .expect("unban voter");
    sqlx::query("UPDATE boards SET downvotes_enabled = FALSE WHERE slug = $1")
        .bind(&slug)
        .execute(&pool)
        .await
        .expect("disable downvotes");
    let downvote =
        cast_vote(&pool, &[board(&slug)], voter, post, -1).await.expect_err("downvote disabled");
    assert_eq!(downvote, VotePolicyError::DownvotesDisabled);

    cleanup(&pool, &slug).await;
}
