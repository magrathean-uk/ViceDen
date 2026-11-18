//! Boards repository: upsert from TOML config into the `boards` table.
//!
//! NOTE: Like the rest of the repository layer, this module uses runtime SQL
//! via `sqlx::query` rather than the compile-time `sqlx::query!` macros. The
//! macro forms require either a live `DATABASE_URL` at build time or prepared
//! offline metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! Boards are still defined in TOML (see `crate::models::Board`); this repo
//! exists so the runtime store reflects what the config says without bouncing
//! through the legacy `db.rs` helpers.

use crate::models::Board;

#[derive(Clone, Copy)]
pub struct BoardRepo;

impl BoardRepo {
    /// Upsert each board into Postgres. Mirrors `db::sync_boards`. Takes a
    /// borrowed `PgConnection` because each row is a separate `execute` call;
    /// callers typically pass `&mut *tx` from a wrapping transaction.
    pub async fn upsert_from_config(
        &self,
        executor: &mut sqlx::PgConnection,
        boards: &[Board],
    ) -> Result<(), sqlx::Error> {
        for board in boards {
            sqlx::query(
                "INSERT INTO boards (slug, name, ttl_days, description, downvotes_enabled)
                 VALUES ($1, $2, $3, $4, TRUE)
                 ON CONFLICT (slug) DO UPDATE
                 SET name = EXCLUDED.name,
                     ttl_days = EXCLUDED.ttl_days,
                     description = EXCLUDED.description",
            )
            .bind(&board.slug)
            .bind(&board.name)
            .bind(board.ttl_days as i32)
            .bind(&board.description)
            .execute(&mut *executor)
            .await?;
        }
        Ok(())
    }
}
