//! Posts repository. Owns the moderation read-path filter.
//!
//! NOTE: Like `users.rs` and `sessions.rs`, this module uses runtime SQL via
//! `sqlx::query_as::<_, T>` rather than the compile-time `sqlx::query_as!` /
//! `sqlx::query!` macros. The macro forms require either a live `DATABASE_URL`
//! at build time or prepared offline metadata in `.sqlx/`, neither of which
//! the workspace ships today. Migrating to macros later is API-compatible.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModState {
    Pending,
    Visible,
    Quarantined,
    Removed,
}

impl ModState {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Visible => "visible",
            Self::Quarantined => "quarantined",
            Self::Removed => "removed",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "pending" => Self::Pending,
            "visible" => Self::Visible,
            "quarantined" => Self::Quarantined,
            "removed" => Self::Removed,
            _ => Self::Removed, // fail closed
        }
    }
}

#[derive(Clone, Debug)]
pub struct PostRow {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub user_id: Uuid,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub mod_state: ModState,
    pub mod_state_reason: Option<String>,
    pub mod_state_changed_at: Option<DateTime<Utc>>,
}

/// Internal row shape that mirrors the on-disk columns. We select into this
/// then convert `mod_state: String` into the `ModState` enum on the way out.
#[derive(Clone, Debug, FromRow)]
struct PostRowRaw {
    id: Uuid,
    thread_id: Uuid,
    user_id: Uuid,
    body: String,
    created_at: DateTime<Utc>,
    mod_state: String,
    mod_state_reason: Option<String>,
    mod_state_changed_at: Option<DateTime<Utc>>,
}

impl From<PostRowRaw> for PostRow {
    fn from(r: PostRowRaw) -> Self {
        Self {
            id: r.id,
            thread_id: r.thread_id,
            user_id: r.user_id,
            body: r.body,
            created_at: r.created_at,
            mod_state: ModState::from_db(&r.mod_state),
            mod_state_reason: r.mod_state_reason,
            mod_state_changed_at: r.mod_state_changed_at,
        }
    }
}

#[derive(Debug)]
pub struct NewPost {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub user_id: Uuid,
    pub body: String,
    pub mod_state: ModState,
}

#[derive(Clone, Copy)]
pub struct PostRepo;

impl PostRepo {
    pub async fn insert<'e, E>(&self, executor: E, post: NewPost) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO posts (id, thread_id, user_id, body, mod_state, mod_state_changed_at)
             VALUES ($1, $2, $3, $4, $5, now())",
        )
        .bind(post.id)
        .bind(post.thread_id)
        .bind(post.user_id)
        .bind(&post.body)
        .bind(post.mod_state.as_db())
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: Uuid,
    ) -> Result<Option<PostRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, PostRowRaw>(
            "SELECT id, thread_id, user_id, body, created_at,
                    mod_state, mod_state_reason, mod_state_changed_at
             FROM posts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(executor)
        .await?;
        Ok(row.map(PostRow::from))
    }

    pub async fn update_body_with_edit<'e, E>(
        &self,
        executor: E,
        post_id: Uuid,
        editor_id: Uuid,
        new_body: &str,
    ) -> Result<bool, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query(
            r#"
            WITH old AS (
                SELECT body FROM posts WHERE id = $1
            ), updated AS (
                UPDATE posts
                SET body = $3
                WHERE id = $1
                  AND EXISTS (SELECT 1 FROM old WHERE old.body <> $3)
                RETURNING id
            )
            INSERT INTO post_edits (post_id, editor_id, previous_body, new_body, edited_at)
            SELECT $1, $2, old.body, $3, now()
            FROM old
            WHERE old.body <> $3
            "#,
        )
        .bind(post_id)
        .bind(editor_id)
        .bind(new_body)
        .execute(executor)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn edited_post_ids<'e, E>(
        &self,
        executor: E,
        post_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if post_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(Uuid,)> =
            sqlx::query_as("SELECT DISTINCT post_id FROM post_edits WHERE post_id = ANY($1)")
                .bind(post_ids)
                .fetch_all(executor)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
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
            "UPDATE posts
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

    /// Public read with the spec §7 visibility rule:
    /// - founder sees everything,
    /// - the post's author sees their own pending/quarantined,
    /// - everyone else sees only `mod_state = 'visible'`.
    pub async fn visible_posts_for_thread<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
        viewer_id: Uuid,
    ) -> Result<Vec<PostRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, PostRowRaw>(
            "SELECT p.id, p.thread_id, p.user_id, p.body, p.created_at,
                    p.mod_state, p.mod_state_reason, p.mod_state_changed_at
             FROM posts p
             LEFT JOIN users v ON v.id = $2
             WHERE p.thread_id = $1
               AND ( v.role = 'founder'
                  OR (p.user_id = $2 AND p.mod_state IN ('pending','visible','quarantined'))
                  OR  p.mod_state = 'visible')
             ORDER BY p.created_at ASC",
        )
        .bind(thread_id)
        .bind(viewer_id)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(PostRow::from).collect())
    }

    pub async fn find_pending_older_than<'e, E>(
        &self,
        executor: E,
        cutoff: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<PostRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, PostRowRaw>(
            "SELECT id, thread_id, user_id, body, created_at,
                    mod_state, mod_state_reason, mod_state_changed_at
             FROM posts
             WHERE mod_state = 'pending' AND created_at < $1
             ORDER BY created_at ASC LIMIT $2",
        )
        .bind(cutoff)
        .bind(limit)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(PostRow::from).collect())
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
             FROM posts
             WHERE user_id = $1
               AND (created_at AT TIME ZONE 'UTC')::date
                   = (CURRENT_TIMESTAMP AT TIME ZONE 'UTC')::date",
        )
        .bind(user_id)
        .fetch_one(executor)
        .await?;
        Ok(count)
    }

    pub async fn random_visible_sample<'e, E>(
        &self,
        executor: E,
        lookback_days: i32,
        sample: i64,
    ) -> Result<Vec<PostRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, PostRowRaw>(
            "SELECT id, thread_id, user_id, body, created_at,
                    mod_state, mod_state_reason, mod_state_changed_at
             FROM posts
             WHERE mod_state = 'visible'
               AND created_at > now() - ($1 || ' days')::interval
             ORDER BY random() LIMIT $2",
        )
        .bind(lookback_days.to_string())
        .bind(sample)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(PostRow::from).collect())
    }

    /// Total number of post rows. Used by `view_metrics`. Includes every
    /// `mod_state` (the metric is "rows on disk", not "visible posts").
    pub async fn count<'e, E>(&self, executor: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM posts").fetch_one(executor).await?;
        Ok(count)
    }

    /// Count posts in a given moderation state. Used by
    /// `/internal/metrics` (pending-queue depth gauge) and by the
    /// daily-audit worker to estimate sample size.
    pub async fn count_with_state<'e, E>(
        &self,
        executor: E,
        state: ModState,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE mod_state = $1")
            .bind(state.as_db())
            .fetch_one(executor)
            .await?;
        Ok(count)
    }

    /// Number of distinct users who posted in the given thread. The thread
    /// author counts only if they posted (they always post the root, so this
    /// is normally `≥ 1` for any non-empty thread). Returns `0` if the thread
    /// has no posts. Used by `view_thread` to render the "N participants"
    /// chip.
    pub async fn participant_count<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(DISTINCT user_id) FROM posts WHERE thread_id = $1")
                .bind(thread_id)
                .fetch_one(executor)
                .await?;
        Ok(count)
    }

    /// First (chronologically earliest) post in a thread. Used to render the
    /// thread-preview snippet on `index` / `view_board` / `search`. Returns
    /// `None` when the thread has no posts.
    ///
    /// Includes every `mod_state` — the caller is expected to apply visibility
    /// rules. For preview rendering, the caller usually wants the visible
    /// root post, which it can get via `visible_posts_for_thread().first()`
    /// or by checking `row.mod_state` here.
    pub async fn first_post_for_thread<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
    ) -> Result<Option<PostRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, PostRowRaw>(
            "SELECT id, thread_id, user_id, body, created_at,
                    mod_state, mod_state_reason, mod_state_changed_at
             FROM posts
             WHERE thread_id = $1
             ORDER BY created_at ASC, id ASC
             LIMIT 1",
        )
        .bind(thread_id)
        .fetch_optional(executor)
        .await?;
        Ok(row.map(PostRow::from))
    }
}
