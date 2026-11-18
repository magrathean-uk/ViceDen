//! Personal Access Tokens (PATs) — issuance, lookup, revocation.
//!
//! NOTE: Like `users.rs` and `sessions.rs`, this module uses runtime SQL via
//! `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`) rather than the
//! compile-time `sqlx::query_as!` / `sqlx::query!` macros. The macro forms
//! require either a live `DATABASE_URL` at build time or prepared offline
//! metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! Tokens are NEVER stored raw. We store the peppered HMAC-SHA256 digest
//! (or legacy SHA-256 if no pepper is configured) so a DB leak does not
//! expose live tokens. The dual-form lookup in `authenticate` matches both
//! the new and legacy digest shapes for transitional rows.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

use crate::auth::tokens;

#[derive(Clone, Debug, FromRow)]
pub struct PatRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub preview: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy)]
pub struct PatRepo;

impl PatRepo {
    /// Insert a new PAT row. Returns the row id.
    ///
    /// `raw_token` is hashed (with `pepper` if provided) before being stored;
    /// the raw value never touches the database. The `preview` column gets a
    /// fixed-length prefix of the token so users can identify tokens without
    /// us having to reveal them post-issuance.
    pub async fn issue<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        name: &str,
        raw_token: &str,
        pepper: Option<&str>,
    ) -> Result<Uuid, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let digest = tokens::digest_token(raw_token, pepper);
        let storage = digest.for_storage();
        let version_label = digest.version_label();
        let preview = format!("{}...", raw_token.chars().take(12).collect::<String>());
        let id = Uuid::new_v4();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO api_tokens (
                token_hash, token_digest_version, id, user_id, name,
                preview, created_at, last_used_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, NULL)",
        )
        .bind(storage)
        .bind(version_label)
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(preview)
        .bind(now)
        .execute(executor)
        .await?;
        Ok(id)
    }

    pub async fn list_for_user<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<Vec<PatRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, PatRow>(
            "SELECT id, user_id, name, preview, created_at, last_used_at
             FROM api_tokens
             WHERE user_id = $1
             ORDER BY created_at DESC",
        )
        .bind(user_id)
        .fetch_all(executor)
        .await
    }

    /// Revoke a single PAT, scoped to its owner so a stolen `pat_id` from one
    /// account can't delete another account's token.
    pub async fn revoke<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<bool, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result = sqlx::query("DELETE FROM api_tokens WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(executor)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Look up a token by its raw value, bumping `last_used_at` on success.
    /// Returns `(user_id, pat_id)` for the matching row, or `None` when no
    /// digest matches.
    pub async fn authenticate<'e, E>(
        &self,
        executor: E,
        raw_token: &str,
        pepper: Option<&str>,
    ) -> Result<Option<(Uuid, Uuid)>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let digests = tokens::lookup_digests(raw_token, pepper);
        let row: Option<(Uuid, Uuid)> = sqlx::query_as(
            "UPDATE api_tokens
             SET last_used_at = now()
             WHERE token_hash = ANY($1)
             RETURNING user_id, id",
        )
        .bind(&digests)
        .fetch_optional(executor)
        .await?;
        Ok(row)
    }
}
