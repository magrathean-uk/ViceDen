//! Content-classifications audit log.
//!
//! NOTE: Like `users.rs`, `sessions.rs`, `posts.rs`, and `threads.rs`, this
//! module uses runtime SQL via `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`)
//! rather than the compile-time `sqlx::query_as!` / `sqlx::query!` macros. The
//! macro forms require either a live `DATABASE_URL` at build time or prepared
//! offline metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! Enum-typed columns (`target_kind`, `verdict`) are TEXT in Postgres. Selects
//! land in a private `ClassificationRowRaw` (derive-FromRow) and we map the
//! string columns to `TargetKind` / `AuditVerdict` via `From<…Raw> for
//! ClassificationRow`, mirroring the pattern used in `repository/posts.rs` for
//! `mod_state`.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Post,
    Thread,
}

impl TargetKind {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Thread => "thread",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "post" => Self::Post,
            "thread" => Self::Thread,
            // Fail closed-ish: unknown shapes default to Post (logged-only).
            _ => Self::Post,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditVerdict {
    Allow,
    Quarantine,
    Reject,
    Error,
}

impl AuditVerdict {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Quarantine => "quarantine",
            Self::Reject => "reject",
            Self::Error => "error",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "allow" => Self::Allow,
            "quarantine" => Self::Quarantine,
            "reject" => Self::Reject,
            "error" => Self::Error,
            // Fail closed: anything unrecognised becomes Error so callers don't
            // mistakenly treat unknown verdicts as Allow.
            _ => Self::Error,
        }
    }
}

#[derive(Debug)]
pub struct NewClassification {
    pub target_kind: TargetKind,
    pub target_id: Uuid,
    pub verdict: AuditVerdict,
    pub categories: Vec<String>,
    pub confidence: Option<f32>,
    pub model: String,
    pub request_id: Option<String>,
    pub latency_ms: Option<i32>,
    pub raw_response: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct ClassificationRow {
    pub id: i64,
    pub target_kind: TargetKind,
    pub target_id: Uuid,
    pub verdict: AuditVerdict,
    pub categories: Vec<String>,
    pub confidence: Option<f32>,
    pub model: String,
    pub request_id: Option<String>,
    pub latency_ms: Option<i32>,
    pub classified_at: DateTime<Utc>,
    pub raw_response: Option<Value>,
}

/// Internal row shape that mirrors the on-disk columns. Converted to
/// `ClassificationRow` (with the typed enums) via `From` on the way out.
#[derive(Clone, Debug, FromRow)]
struct ClassificationRowRaw {
    id: i64,
    target_kind: String,
    target_id: Uuid,
    verdict: String,
    categories: Vec<String>,
    confidence: Option<f32>,
    model: String,
    request_id: Option<String>,
    latency_ms: Option<i32>,
    classified_at: DateTime<Utc>,
    raw_response: Option<Value>,
}

impl From<ClassificationRowRaw> for ClassificationRow {
    fn from(r: ClassificationRowRaw) -> Self {
        Self {
            id: r.id,
            target_kind: TargetKind::from_db(&r.target_kind),
            target_id: r.target_id,
            verdict: AuditVerdict::from_db(&r.verdict),
            categories: r.categories,
            confidence: r.confidence,
            model: r.model,
            request_id: r.request_id,
            latency_ms: r.latency_ms,
            classified_at: r.classified_at,
            raw_response: r.raw_response,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ContentClassificationRepo;

impl ContentClassificationRepo {
    pub async fn insert<'e, E>(
        &self,
        executor: E,
        row: NewClassification,
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let id: (i64,) = sqlx::query_as(
            "INSERT INTO content_classifications
              (target_kind, target_id, verdict, categories, confidence,
               model, request_id, latency_ms, raw_response)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             RETURNING id",
        )
        .bind(row.target_kind.as_db())
        .bind(row.target_id)
        .bind(row.verdict.as_db())
        .bind(&row.categories)
        .bind(row.confidence)
        .bind(&row.model)
        .bind(row.request_id)
        .bind(row.latency_ms)
        .bind(row.raw_response)
        .fetch_one(executor)
        .await?;
        Ok(id.0)
    }

    pub async fn list_for_target<'e, E>(
        &self,
        executor: E,
        kind: TargetKind,
        target_id: Uuid,
    ) -> Result<Vec<ClassificationRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows = sqlx::query_as::<_, ClassificationRowRaw>(
            "SELECT id, target_kind, target_id, verdict, categories, confidence,
                    model, request_id, latency_ms, classified_at, raw_response
             FROM content_classifications
             WHERE target_kind = $1 AND target_id = $2
             ORDER BY classified_at DESC",
        )
        .bind(kind.as_db())
        .bind(target_id)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(ClassificationRow::from).collect())
    }
}
