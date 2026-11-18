//! Reports repository.
//!
//! NOTE: Like `users.rs` and the rest of the repository layer, this module
//! uses runtime SQL via `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`)
//! rather than the compile-time `sqlx::query_as!` macros. The macro forms
//! require either a live `DATABASE_URL` at build time or prepared offline
//! metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! Schema (migration 0005): `target_post_id` and `target_thread_id` are
//! mutually-exclusive nullable columns instead of a `(target_kind, target_id)`
//! polymorphism. `actor_id` plays the role of "reporter_id". This repo mirrors
//! that shape so the SQL is a direct port of `db::insert_report` /
//! `db::list_reports`.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct ReportRow {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub reason: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
    pub priority: i32,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub resolution_note: Option<String>,
    pub queue_state: String,
    pub assigned_to: Option<Uuid>,
    pub assigned_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, FromRow)]
pub struct ReportQueueRow {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub reason: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
    pub priority: i32,
    pub priority_label: String,
    pub reviewer_user_id: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub resolution_note: Option<String>,
    pub queue_state: String,
    pub assigned_user_id: Option<Uuid>,
    pub assigned_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct NewReport {
    pub id: Uuid,
    /// Reporter — mapped to the `actor_id` column.
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub reason: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
}

fn priority_for_reason(reason: &str, details: Option<&str>) -> i32 {
    let reason = reason.to_ascii_lowercase();
    let details = details.unwrap_or_default().to_ascii_lowercase();
    let haystack = format!("{reason} {details}");

    if ["minor", "underage", "doxx", "doxxing", "threat", "violence", "blackmail"]
        .iter()
        .any(|needle| haystack.contains(needle))
    {
        3
    } else if ["spam", "scam", "solicit", "source", "sale", "arrange"]
        .iter()
        .any(|needle| haystack.contains(needle))
    {
        2
    } else {
        1
    }
}

#[derive(Clone, Copy)]
pub struct ReportRepo;

#[derive(Clone, Copy, Debug, Default)]
pub struct ReportQueueFilter<'a> {
    pub status: Option<&'a str>,
    pub queue_state: Option<&'a str>,
    pub assigned: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub event_action: Option<&'a str>,
    pub event_actor_id: Option<Uuid>,
    pub event_created_from: Option<DateTime<Utc>>,
    pub event_created_to: Option<DateTime<Utc>>,
    pub viewer_id: Option<Uuid>,
}

impl ReportRepo {
    pub async fn insert<'e, E>(&self, executor: E, report: NewReport) -> Result<Uuid, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let priority = priority_for_reason(&report.reason, report.details.as_deref());
        sqlx::query(
            "INSERT INTO reports (
                id, actor_id, target_post_id, target_thread_id,
                reason, details, created_at, resolved, priority
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8)",
        )
        .bind(report.id)
        .bind(report.actor_id)
        .bind(report.target_post_id)
        .bind(report.target_thread_id)
        .bind(&report.reason)
        .bind(&report.details)
        .bind(report.created_at)
        .bind(priority)
        .execute(executor)
        .await?;
        Ok(report.id)
    }

    /// List every still-open (unresolved) report, newest first.
    pub async fn list_open<'e, E>(&self, executor: E) -> Result<Vec<ReportRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ReportRow>(
            "SELECT id, actor_id, target_post_id, target_thread_id,
                    reason, details, created_at, resolved,
                    priority, reviewed_by, reviewed_at, resolution_note,
                    queue_state, assigned_to, assigned_at
             FROM reports
             WHERE resolved = FALSE
             ORDER BY created_at DESC",
        )
        .fetch_all(executor)
        .await
    }

    /// Mark the report as resolved (or re-open it). The schema has no
    /// "resolved by" column, so `resolver_id` is logged via the moderation
    /// event stream by the caller — kept here for API parity with the spec.
    pub async fn resolve<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        resolver_id: Uuid,
        resolved: bool,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        self.resolve_with_note(executor, id, resolver_id, resolved, None).await
    }

    pub async fn resolve_with_note<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        resolver_id: Uuid,
        resolved: bool,
        note: Option<&str>,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE reports
             SET resolved = $2,
                 reviewed_by = $3,
                 reviewed_at = now(),
                 resolution_note = $4
             WHERE id = $1",
        )
        .bind(id)
        .bind(resolved)
        .bind(resolver_id)
        .bind(note)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn count_open<'e, E>(&self, executor: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reports WHERE resolved = FALSE")
            .fetch_one(executor)
            .await?;
        Ok(count)
    }

    /// Single-report lookup by id. Used by `remove_report_target_web`
    /// to find which thread/post the report points at before applying
    /// moderation.
    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: Uuid,
    ) -> Result<Option<ReportRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ReportRow>(
            "SELECT id, actor_id, target_post_id, target_thread_id,
                    reason, details, created_at, resolved,
                    priority, reviewed_by, reviewed_at, resolution_note,
                    queue_state, assigned_to, assigned_at
             FROM reports WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    pub async fn list_queue<'e, E>(&self, executor: E) -> Result<Vec<ReportQueueRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        self.list_queue_filtered(executor, ReportQueueFilter::default()).await
    }

    pub async fn list_queue_filtered<'e, E>(
        &self,
        executor: E,
        filter: ReportQueueFilter<'_>,
    ) -> Result<Vec<ReportQueueRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let status = if matches!(filter.status, Some("resolved")) { "resolved" } else { "open" };
        let queue_state =
            filter.queue_state.filter(|value| matches!(*value, "new" | "reviewing" | "waiting"));
        let assigned = filter.assigned.filter(|value| matches!(*value, "me" | "unassigned"));
        let priority =
            filter.priority.filter(|value| matches!(*value, "urgent" | "elevated" | "normal"));
        let event_action = filter.event_action.map(str::trim).filter(|value| !value.is_empty());
        let rows = sqlx::query_as::<_, ReportQueueRow>(
            "SELECT r.id,
                    r.actor_id,
                    r.target_post_id,
                    r.target_thread_id,
                    COALESCE(p.user_id, t.user_id) AS target_user_id,
                    r.reason,
                    r.details,
                    r.created_at,
                    r.resolved,
                    r.priority,
                    CASE
                        WHEN r.priority >= 3 THEN 'urgent'
                        WHEN r.priority = 2 THEN 'elevated'
                        ELSE 'normal'
                    END AS priority_label,
                    r.reviewed_by AS reviewer_user_id,
                    r.reviewed_at,
                    r.resolution_note,
                    r.queue_state,
                    r.assigned_to AS assigned_user_id,
                    r.assigned_at
             FROM reports r
             LEFT JOIN posts p ON p.id = r.target_post_id
             LEFT JOIN threads t ON t.id = r.target_thread_id
             WHERE (
                    ($1::text = 'resolved' AND r.resolved = TRUE)
                 OR ($1::text <> 'resolved' AND r.resolved = FALSE)
             )
               AND ($2::text IS NULL OR r.queue_state = $2)
               AND (
                    $3::text IS NULL
                 OR ($3::text = 'me' AND r.assigned_to = $4)
                 OR ($3::text = 'unassigned' AND r.assigned_to IS NULL)
               )
               AND (
                    $5::text IS NULL
                 OR ($5::text = 'urgent' AND r.priority >= 3)
                 OR ($5::text = 'elevated' AND r.priority = 2)
                 OR ($5::text = 'normal' AND r.priority <= 1)
               )
               AND (
                    ($6::text IS NULL AND $7::uuid IS NULL)
                 OR EXISTS (
                        SELECT 1
                        FROM moderation_events me
                        WHERE me.report_id = r.id
                          AND ($6::text IS NULL OR me.action = $6)
                          AND ($7::uuid IS NULL OR me.actor_id = $7)
                          AND ($8::timestamptz IS NULL OR me.created_at >= $8)
                          AND ($9::timestamptz IS NULL OR me.created_at < $9)
                    )
               )
             ORDER BY
                CASE WHEN $1::text = 'resolved' THEN COALESCE(r.reviewed_at, r.created_at) END DESC NULLS LAST,
                CASE WHEN $1::text <> 'resolved' THEN r.priority END DESC NULLS LAST,
                CASE WHEN $1::text <> 'resolved' THEN r.created_at END DESC NULLS LAST,
                r.created_at DESC",
        )
        .bind(status)
        .bind(queue_state)
        .bind(assigned)
        .bind(filter.viewer_id)
        .bind(priority)
        .bind(event_action)
        .bind(filter.event_actor_id)
        .bind(filter.event_created_from)
        .bind(filter.event_created_to)
        .fetch_all(executor)
        .await?;
        Ok(rows)
    }

    pub async fn triage<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        queue_state: Option<&str>,
        assigned_to: Option<Option<Uuid>>,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE reports
             SET queue_state = COALESCE($2, queue_state),
                 assigned_to = CASE
                     WHEN $3::boolean IS TRUE THEN $4
                     ELSE assigned_to
                 END,
                 assigned_at = CASE
                     WHEN $3::boolean IS FALSE THEN assigned_at
                     WHEN $4::uuid IS NULL THEN NULL
                     ELSE now()
                 END
             WHERE id = $1",
        )
        .bind(id)
        .bind(queue_state)
        .bind(assigned_to.is_some())
        .bind(assigned_to.flatten())
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn find_latest_open_for_target<'e, E>(
        &self,
        executor: E,
        target_post_id: Option<Uuid>,
        target_thread_id: Option<Uuid>,
    ) -> Result<Option<ReportRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ReportRow>(
            "SELECT id, actor_id, target_post_id, target_thread_id,
                    reason, details, created_at, resolved,
                    priority, reviewed_by, reviewed_at, resolution_note,
                    queue_state, assigned_to, assigned_at
             FROM reports
             WHERE resolved = FALSE
               AND (
                    ($1::uuid IS NOT NULL AND target_post_id = $1)
                 OR ($2::uuid IS NOT NULL AND target_post_id IS NULL AND target_thread_id = $2)
               )
             ORDER BY priority DESC, created_at DESC
             LIMIT 1",
        )
        .bind(target_post_id)
        .bind(target_thread_id)
        .fetch_optional(executor)
        .await
    }
}
