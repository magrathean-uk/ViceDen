//! Sessions repository.
//!
//! Tokens are NEVER stored raw. We store the peppered HMAC-SHA256 digest
//! (or legacy SHA-256 if no pepper is configured) so a DB leak does not
//! expose live sessions.
//!
//! NOTE: Like `users.rs`, this module uses runtime SQL via
//! `sqlx::query_as::<_, T>` rather than the compile-time `sqlx::query_as!`
//! macro. The macro form requires either a live `DATABASE_URL` at build time
//! or prepared offline metadata in `.sqlx/`, neither of which the workspace
//! ships today. Migrating to macros later is API-compatible.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

use crate::auth::tokens;

#[derive(Clone, Debug, FromRow)]
pub struct SessionRow {
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub csrf_token: String,
}

#[derive(Debug)]
pub struct NewSession {
    pub token: String,
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub csrf_token: String,
}

#[derive(Clone, Copy)]
pub struct SessionRepo;

impl SessionRepo {
    pub async fn create<'e, E>(
        &self,
        executor: E,
        session: NewSession,
        pepper: Option<&str>,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let digest = tokens::digest_token(&session.token, pepper).for_storage();
        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, csrf_token)
             VALUES ($1, $2, now(), $3, $4)",
        )
        .bind(digest)
        .bind(session.user_id)
        .bind(session.expires_at)
        .bind(session.csrf_token)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn find_by_token<'e, E>(
        &self,
        executor: E,
        token: &str,
        pepper: Option<&str>,
    ) -> Result<Option<SessionRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let digests = tokens::lookup_digests(token, pepper);
        sqlx::query_as::<_, SessionRow>(
            "SELECT user_id, created_at, expires_at, csrf_token
             FROM sessions
             WHERE token_hash = ANY($1) AND expires_at > now()
             LIMIT 1",
        )
        .bind(&digests)
        .fetch_optional(executor)
        .await
    }

    pub async fn destroy<'e, E>(
        &self,
        executor: E,
        token: &str,
        pepper: Option<&str>,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let digests = tokens::lookup_digests(token, pepper);
        sqlx::query("DELETE FROM sessions WHERE token_hash = ANY($1)")
            .bind(&digests)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn prune_expired<'e, E>(&self, executor: E) -> Result<u64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let result =
            sqlx::query("DELETE FROM sessions WHERE expires_at <= now()").execute(executor).await?;
        Ok(result.rows_affected())
    }
}
