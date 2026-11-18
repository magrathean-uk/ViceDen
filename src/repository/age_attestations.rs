//! Age attestations repository — write-once at the application layer.
//!
//! NOTE: Like `users.rs`, `sessions.rs`, `posts.rs`, and `threads.rs`, this
//! module uses runtime SQL via `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`)
//! rather than the compile-time `sqlx::query_as!` / `sqlx::query!` macros. The
//! macro forms require either a live `DATABASE_URL` at build time or prepared
//! offline metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! The `attested_ip` column is declared `INET` in Postgres. We bind it through
//! a TEXT parameter with `$3::inet` and read it back with the
//! `attested_ip::text` cast, so the Rust side stays a plain `String` and we
//! avoid pulling in `sqlx`'s `ipnetwork` feature.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct AttestationRow {
    pub user_id: Uuid,
    pub dob: NaiveDate,
    pub attested_at: DateTime<Utc>,
    pub attested_ip: String,
    pub attested_ua: String,
    pub invite_code: String,
}

#[derive(Debug)]
pub struct NewAttestation {
    pub user_id: Uuid,
    pub dob: NaiveDate,
    pub attested_ip: String,
    pub attested_ua: String,
    pub invite_code: String,
}

#[derive(Clone, Copy)]
pub struct AgeAttestationRepo;

impl AgeAttestationRepo {
    pub async fn insert<'e, E>(&self, executor: E, row: NewAttestation) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO age_attestations
              (user_id, dob, attested_ip, attested_ua, invite_code)
             VALUES ($1, $2, $3::inet, $4, $5)",
        )
        .bind(row.user_id)
        .bind(row.dob)
        .bind(&row.attested_ip)
        .bind(&row.attested_ua)
        .bind(&row.invite_code)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn find_for_user<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<Option<AttestationRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, AttestationRow>(
            "SELECT user_id, dob, attested_at, attested_ip::text AS attested_ip,
                    attested_ua, invite_code
             FROM age_attestations WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(executor)
        .await
    }
}
