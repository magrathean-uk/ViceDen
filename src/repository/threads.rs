//! Threads repository. Mirrors `PostRepo`'s `mod_state` shape.
//!
//! NOTE: Like `users.rs` and `sessions.rs`, this module uses runtime SQL via
//! `sqlx::query_as::<_, T>` rather than the compile-time `sqlx::query_as!` /
//! `sqlx::query!` macros. The macro forms require either a live `DATABASE_URL`
//! at build time or prepared offline metadata in `.sqlx/`, neither of which
//! the workspace ships today. Migrating to macros later is API-compatible.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

use super::posts::ModState;

#[derive(Clone, Debug)]
pub struct ThreadRow {
    pub id: Uuid,
    pub board_slug: String,
    pub user_id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub mod_state: ModState,
    pub mod_state_reason: Option<String>,
    pub mod_state_changed_at: Option<DateTime<Utc>>,
}

/// Internal row shape that mirrors the on-disk columns. Converted to
/// `ThreadRow` (with the `ModState` enum) on the way out.
#[derive(Clone, Debug, FromRow)]
struct ThreadRowRaw {
    id: Uuid,
    board_slug: String,
    user_id: Uuid,
    title: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    mod_state: String,
    mod_state_reason: Option<String>,
    mod_state_changed_at: Option<DateTime<Utc>>,
}

impl From<ThreadRowRaw> for ThreadRow {
    fn from(r: ThreadRowRaw) -> Self {
        Self {
            id: r.id,
            board_slug: r.board_slug,
            user_id: r.user_id,
            title: r.title,
            created_at: r.created_at,
            expires_at: r.expires_at,
            mod_state: ModState::from_db(&r.mod_state),
            mod_state_reason: r.mod_state_reason,
            mod_state_changed_at: r.mod_state_changed_at,
        }
    }
}

#[derive(Debug)]
pub struct NewThread {
    pub id: Uuid,
    pub board_slug: String,
    pub user_id: Uuid,
    pub title: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Copy)]
pub struct ThreadRepo;

impl ThreadRepo {
    pub async fn insert<'e, E>(&self, executor: E, thread: NewThread) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO threads (id, board_slug, user_id, title, expires_at, mod_state_changed_at)
             VALUES ($1, $2, $3, $4, $5, now())",
        )
        .bind(thread.id)
        .bind(&thread.board_slug)
        .bind(thread.user_id)
        .bind(&thread.title)
        .bind(thread.expires_at)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: Uuid,
    ) -> Result<Option<ThreadRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, ThreadRowRaw>(
            "SELECT id, board_slug, user_id, title, created_at, expires_at,
                    mod_state, mod_state_reason, mod_state_changed_at
             FROM threads WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(executor)
        .await?;
        Ok(row.map(ThreadRow::from))
    }

    pub async fn update_mod_state<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        state: ModState,
        reason: Option<&str>,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE threads
             SET mod_state = $2, mod_state_reason = $3, mod_state_changed_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(state.as_db())
        .bind(reason)
        .execute(executor)
        .await?;
        Ok(())
    }

    /// Read with viewer-aware visibility. Same rule shape as `PostRepo`:
    /// - founder sees everything,
    /// - the thread's author sees their own pending/quarantined,
    /// - everyone else sees only `mod_state = 'visible'`.
    pub async fn visible_threads_for_board<'e, E>(
        &self,
        executor: E,
        board_slug: &str,
        viewer_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ThreadRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, ThreadRowRaw>(
            "SELECT t.id, t.board_slug, t.user_id, t.title, t.created_at, t.expires_at,
                    t.mod_state, t.mod_state_reason, t.mod_state_changed_at
             FROM threads t
             LEFT JOIN users v ON v.id = $2
             WHERE t.board_slug = $1
               AND ( v.role = 'founder'
                  OR (t.user_id = $2 AND t.mod_state IN ('pending','visible','quarantined'))
                  OR  t.mod_state = 'visible')
             ORDER BY t.created_at DESC
             LIMIT $3 OFFSET $4",
        )
        .bind(board_slug)
        .bind(viewer_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(ThreadRow::from).collect())
    }

    /// Total number of thread rows. Used by `view_metrics`.
    pub async fn count<'e, E>(&self, executor: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM threads").fetch_one(executor).await?;
        Ok(count)
    }

    pub async fn count_for_user_today<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM threads
             WHERE user_id = $1
               AND (created_at AT TIME ZONE 'UTC')::date
                   = (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date",
        )
        .bind(user_id)
        .fetch_one(executor)
        .await?;
        Ok(count)
    }

    pub async fn count_for_user_in_board_today<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        board_slug: &str,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM threads
             WHERE user_id = $1
               AND board_slug = $2
               AND (created_at AT TIME ZONE 'UTC')::date
                   = (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date",
        )
        .bind(user_id)
        .bind(board_slug)
        .fetch_one(executor)
        .await?;
        Ok(count)
    }

    /// Single-thread read with the derived bits that the index / board /
    /// search fan-out needs: vote score, reply count (posts minus the root),
    /// last-activity timestamp. Returns `None` when the thread does not
    /// exist. `score_hidden` is intentionally NOT included — the handler
    /// derives it from `thread.created_at` via `forum::is_score_hidden`.
    ///
    /// One round-trip via correlated subqueries. The schema indexes on
    /// `posts(thread_id)` (added in 0007) keep this cheap.
    pub async fn find_with_preview<'e, E>(
        &self,
        executor: E,
        id: Uuid,
    ) -> Result<Option<ThreadWithPreview>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, ThreadWithPreviewRaw>(
            r#"
            SELECT
                t.id, t.board_slug, t.user_id, t.title, t.created_at, t.expires_at,
                t.mod_state, t.mod_state_reason, t.mod_state_changed_at,
                COALESCE(
                    (SELECT SUM(v.value)::bigint
                     FROM votes v JOIN posts p ON p.id = v.post_id
                     WHERE p.thread_id = t.id),
                    0
                ) AS score,
                GREATEST(
                    COALESCE((SELECT COUNT(*)::bigint FROM posts WHERE thread_id = t.id), 0) - 1,
                    0
                ) AS reply_count,
                COALESCE(
                    (SELECT MAX(created_at) FROM posts WHERE thread_id = t.id),
                    t.created_at
                ) AS last_activity
            FROM threads t
            WHERE t.id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(executor)
        .await?;
        Ok(row.map(ThreadWithPreview::from))
    }
}

/// Public preview shape returned by `ThreadRepo::find_with_preview`. Mirrors
/// `forum::ThreadPreview` minus the `score_hidden` flag (handler-derived).
#[derive(Clone, Debug)]
pub struct ThreadWithPreview {
    pub thread: ThreadRow,
    pub score: i64,
    pub reply_count: i64,
    pub last_activity: DateTime<Utc>,
}

#[derive(Clone, Debug, FromRow)]
struct ThreadWithPreviewRaw {
    id: Uuid,
    board_slug: String,
    user_id: Uuid,
    title: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    mod_state: String,
    mod_state_reason: Option<String>,
    mod_state_changed_at: Option<DateTime<Utc>>,
    score: i64,
    reply_count: i64,
    last_activity: DateTime<Utc>,
}

impl From<ThreadWithPreviewRaw> for ThreadWithPreview {
    fn from(r: ThreadWithPreviewRaw) -> Self {
        Self {
            thread: ThreadRow {
                id: r.id,
                board_slug: r.board_slug,
                user_id: r.user_id,
                title: r.title,
                created_at: r.created_at,
                expires_at: r.expires_at,
                mod_state: ModState::from_db(&r.mod_state),
                mod_state_reason: r.mod_state_reason,
                mod_state_changed_at: r.mod_state_changed_at,
            },
            score: r.score,
            reply_count: r.reply_count,
            last_activity: r.last_activity,
        }
    }
}
