//! Votes repository: per-post upvote/downvote storage and aggregate scoring.
//!
//! NOTE: Like `users.rs` and the rest of the repository layer, this module
//! uses runtime SQL via `sqlx::query_scalar` / `sqlx::query` rather than the
//! compile-time `sqlx::query!` macros. The macro forms require either a live
//! `DATABASE_URL` at build time or prepared offline metadata in `.sqlx/`,
//! neither of which the workspace ships today. Migrating to macros later is
//! API-compatible.

use sqlx::{Executor, Postgres};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct VoteRepo;

impl VoteRepo {
    /// Upsert a vote for `(post_id, voter_id)`. `value` should be `-1`, `0`,
    /// or `+1`; the caller is responsible for normalising user input. The
    /// schema CHECK accepts `-1..=1` (see migration 0004).
    pub async fn upsert<'e, E>(
        &self,
        executor: E,
        voter_id: Uuid,
        post_id: Uuid,
        value: i16,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO votes (post_id, user_id, value)
             VALUES ($1, $2, $3)
             ON CONFLICT (post_id, user_id) DO UPDATE
             SET value = EXCLUDED.value",
        )
        .bind(post_id)
        .bind(voter_id)
        .bind(value)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn delete<'e, E>(
        &self,
        executor: E,
        voter_id: Uuid,
        post_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM votes WHERE post_id = $1 AND user_id = $2")
            .bind(post_id)
            .bind(voter_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    /// Sum of all vote values cast on a single post. Missing rows count as 0.
    pub async fn score_for_post<'e, E>(
        &self,
        executor: E,
        post_id: Uuid,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let score: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(SUM(value)::bigint, 0) FROM votes WHERE post_id = $1",
        )
        .bind(post_id)
        .fetch_one(executor)
        .await?;
        Ok(score.unwrap_or(0))
    }

    /// Sum of votes across every post in a thread. Threads with no posts /
    /// votes return 0.
    pub async fn score_for_thread<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let score: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(SUM(v.value)::bigint, 0)
             FROM votes v
             JOIN posts p ON p.id = v.post_id
             WHERE p.thread_id = $1",
        )
        .bind(thread_id)
        .fetch_one(executor)
        .await?;
        Ok(score.unwrap_or(0))
    }

    /// The vote a single user has cast on a single post: `-1`, `0` (no vote
    /// row), or `+1`. Used by `view_thread` to highlight the viewer's own
    /// vote arrow.
    ///
    /// This is a pure votes-table read, so it lives on `VoteRepo` for
    /// repository cohesion. Handlers call `VoteRepo` directly.
    pub async fn user_vote_for_post<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        post_id: Uuid,
    ) -> Result<i16, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let value: Option<i16> =
            sqlx::query_scalar("SELECT value FROM votes WHERE user_id = $1 AND post_id = $2")
                .bind(user_id)
                .bind(post_id)
                .fetch_optional(executor)
                .await?;
        Ok(value.unwrap_or(0))
    }

    /// Per-post score map for every post in a thread, in a single query.
    /// Posts with no votes are absent from the result — caller treats the
    /// missing key as `0`. Avoids N+1 fan-out in `view_thread` rendering.
    pub async fn scores_for_thread<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
    ) -> Result<Vec<(Uuid, i64)>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT v.post_id, SUM(v.value)::bigint AS score
             FROM votes v
             JOIN posts p ON p.id = v.post_id
             WHERE p.thread_id = $1
             GROUP BY v.post_id",
        )
        .bind(thread_id)
        .fetch_all(executor)
        .await?;
        Ok(rows)
    }

    /// Per-post vote value cast by a single viewer across every post in a
    /// thread. Returns one row per post the viewer has voted on; absent
    /// posts mean "no vote (0)". One query, used by `view_thread` to
    /// highlight the viewer's own vote on each post without N+1 fan-out.
    pub async fn user_votes_for_thread<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<Vec<(Uuid, i16)>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(Uuid, i16)> = sqlx::query_as(
            "SELECT v.post_id, v.value
             FROM votes v
             JOIN posts p ON p.id = v.post_id
             WHERE v.user_id = $1 AND p.thread_id = $2",
        )
        .bind(user_id)
        .bind(thread_id)
        .fetch_all(executor)
        .await?;
        Ok(rows)
    }
}
