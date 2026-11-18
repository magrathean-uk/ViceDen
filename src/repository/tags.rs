//! Tags repository: per-thread tag list (ordered) and top-tags aggregation.
//!
//! Schema (migration 0008): `thread_tags(thread_id, tag, position)` with a
//! composite primary key on `(thread_id, tag)` and an index on `(tag, thread_id)`
//! for fast tag-scoped fan-out.
//!
//! NOTE: Like the rest of the repository layer, this module uses runtime SQL
//! via `sqlx::query_as::<_, T>` / `sqlx::query_scalar` rather than the
//! compile-time macros. Migrating to macros later is API-compatible.

use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct TagUsage {
    pub tag: String,
    pub uses: i64,
}

#[derive(Clone, Copy)]
pub struct TagRepo;

impl TagRepo {
    /// Tags attached to a single thread, sorted by `position` ASC then `tag`
    /// ASC for stability (two tags with the same position fall back to
    /// alphabetical). Empty when the thread has no tags. Used by render
    /// paths via `thread_item_from_preview`.
    pub async fn list_for_thread<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
    ) -> Result<Vec<String>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT tag FROM thread_tags
             WHERE thread_id = $1
             ORDER BY position ASC, tag ASC",
        )
        .bind(thread_id)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(|(tag,)| tag).collect())
    }

    /// The `limit` most-used tags across all threads, ordered by usage
    /// count DESC then tag ASC for stability. Used to render the front-page
    /// "popular tags" cluster. Backed by the `idx_thread_tags_tag_thread`
    /// index.
    pub async fn top_tags<'e, E>(
        &self,
        executor: E,
        limit: i64,
    ) -> Result<Vec<TagUsage>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, TagUsage>(
            "SELECT tag, COUNT(*)::bigint AS uses
             FROM thread_tags
             GROUP BY tag
             ORDER BY uses DESC, tag ASC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(executor)
        .await
    }

    /// B5: batch fetch tags for many threads in one query. Returns one
    /// `(thread_id, tag)` pair per row; the caller groups into a
    /// `HashMap<Uuid, Vec<String>>`. Threads with no tags are simply
    /// absent (caller defaults to `vec![]`).
    ///
    /// Replaces the per-thread `list_for_thread` fan-out in
    /// `view_board` / `view_index` / `view_user_detail` — turns N+1
    /// queries into one for any list-view render.
    pub async fn tags_for_threads<'e, E>(
        &self,
        executor: E,
        thread_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String)>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, (Uuid, String)>(
            "SELECT thread_id, tag FROM thread_tags
             WHERE thread_id = ANY($1::uuid[])
             ORDER BY thread_id, position ASC, tag ASC",
        )
        .bind(thread_ids)
        .fetch_all(executor)
        .await
    }

    /// Replace the entire tag set for a thread. DELETE-then-INSERT preserves
    /// the same shape as `db::set_thread_tags`, including position-based
    /// ordering on re-insert. Caller is responsible for opening a transaction
    /// — pass `&mut *tx` so the DELETE and the INSERTs land atomically.
    /// Empty `tags` clears the thread's tag set.
    pub async fn set_for_thread(
        &self,
        executor: &mut sqlx::PgConnection,
        thread_id: Uuid,
        tags: &[&str],
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM thread_tags WHERE thread_id = $1")
            .bind(thread_id)
            .execute(&mut *executor)
            .await?;
        for (position, tag) in tags.iter().enumerate() {
            sqlx::query(
                "INSERT INTO thread_tags (thread_id, tag, position)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (thread_id, tag) DO UPDATE
                 SET position = EXCLUDED.position",
            )
            .bind(thread_id)
            .bind(*tag)
            .bind(position as i32)
            .execute(&mut *executor)
            .await?;
        }
        Ok(())
    }
}
