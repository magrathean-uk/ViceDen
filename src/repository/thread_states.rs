//! Thread-state repository: per-thread admin annotations
//! (`pinned`, `locked`, `answered`, `closed`).
//!
//! Schema (migration 0009): `thread_states(thread_id PK, pinned, locked,
//! answered, closed)`. Threads with no annotation are absent — callers
//! treat the missing row as the all-`false` default.
//!
//! NOTE: Like the rest of the repository layer, this module uses runtime
//! SQL via `sqlx::query_as::<_, T>` rather than the compile-time macros.
//! Migrating to macros later is API-compatible.

use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, FromRow, PartialEq, Eq, Default)]
pub struct ThreadStateRow {
    pub pinned: bool,
    pub locked: bool,
    pub answered: bool,
    pub closed: bool,
}

#[derive(Clone, Debug, FromRow)]
struct ThreadStateRowWithId {
    thread_id: Uuid,
    pinned: bool,
    locked: bool,
    answered: bool,
    closed: bool,
}

#[derive(Clone, Copy)]
pub struct ThreadStateRepo;

impl ThreadStateRepo {
    /// Single-thread state lookup. Returns `None` when the thread has no
    /// annotation row — the caller should treat that as the all-`false`
    /// default. Used by `view_thread` to render pin / lock / answered /
    /// closed badges.
    pub async fn state_for<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
    ) -> Result<Option<ThreadStateRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ThreadStateRow>(
            "SELECT pinned, locked, answered, closed
             FROM thread_states WHERE thread_id = $1",
        )
        .bind(thread_id)
        .fetch_optional(executor)
        .await
    }

    /// Batch-fetch states for many threads. Returns a `Vec<(Uuid, ThreadStateRow)>`
    /// — caller indexes into a HashMap. Threads with no annotation row are
    /// silently absent (default applies). Used by list-view fan-out
    /// (board / index / search) so we don't issue N separate queries.
    pub async fn states_for_batch<'e, E>(
        &self,
        executor: E,
        thread_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, ThreadStateRow)>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<ThreadStateRowWithId> = sqlx::query_as(
            "SELECT thread_id, pinned, locked, answered, closed
             FROM thread_states WHERE thread_id = ANY($1::uuid[])",
        )
        .bind(thread_ids)
        .fetch_all(executor)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.thread_id,
                    ThreadStateRow {
                        pinned: r.pinned,
                        locked: r.locked,
                        answered: r.answered,
                        closed: r.closed,
                    },
                )
            })
            .collect())
    }

    /// Upsert the full annotation row. Used by admin handlers that pin /
    /// lock / mark-answered / close threads. ON CONFLICT replaces every
    /// flag, mirroring `db::sync_thread_states` semantics.
    pub async fn upsert<'e, E>(
        &self,
        executor: E,
        thread_id: Uuid,
        state: ThreadStateRow,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO thread_states (thread_id, pinned, locked, answered, closed)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (thread_id) DO UPDATE
             SET pinned = EXCLUDED.pinned,
                 locked = EXCLUDED.locked,
                 answered = EXCLUDED.answered,
                 closed = EXCLUDED.closed",
        )
        .bind(thread_id)
        .bind(state.pinned)
        .bind(state.locked)
        .bind(state.answered)
        .bind(state.closed)
        .execute(executor)
        .await?;
        Ok(())
    }
}
