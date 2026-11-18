//! Moderation event repository.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct ModerationEventRow {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub report_id: Option<Uuid>,
    pub action: String,
    pub reason: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct ModerationEventInsert {
    pub id: Uuid,
    pub actor_id: Uuid,
    pub target_post_id: Option<Uuid>,
    pub target_thread_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub report_id: Option<Uuid>,
    pub action: String,
    pub reason: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModerationEventFilter<'a> {
    pub action: Option<&'a str>,
    pub actor_id: Option<Uuid>,
    pub report_id: Option<Uuid>,
    pub target_user_id: Option<Uuid>,
    pub created_from: Option<DateTime<Utc>>,
    pub created_to: Option<DateTime<Utc>>,
    pub limit: i64,
}

#[derive(Clone, Copy)]
pub struct ModerationEventRepo;

impl ModerationEventRepo {
    pub async fn insert<'e, E>(
        &self,
        executor: E,
        event: ModerationEventInsert,
    ) -> Result<Uuid, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO moderation_events (
                id, actor_id, target_post_id, target_thread_id, target_user_id,
                report_id, action, reason, note, created_at
             )
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(event.id)
        .bind(event.actor_id)
        .bind(event.target_post_id)
        .bind(event.target_thread_id)
        .bind(event.target_user_id)
        .bind(event.report_id)
        .bind(&event.action)
        .bind(&event.reason)
        .bind(&event.note)
        .bind(event.created_at)
        .execute(executor)
        .await?;
        Ok(event.id)
    }

    pub async fn list_for_report<'e, E>(
        &self,
        executor: E,
        report_id: Uuid,
    ) -> Result<Vec<ModerationEventRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ModerationEventRow>(
            "SELECT id, actor_id, target_post_id, target_thread_id, target_user_id,
                    report_id, action, reason, note, created_at
             FROM moderation_events
             WHERE report_id = $1
             ORDER BY created_at DESC",
        )
        .bind(report_id)
        .fetch_all(executor)
        .await
    }

    pub async fn list_for_target_user<'e, E>(
        &self,
        executor: E,
        target_user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<ModerationEventRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ModerationEventRow>(
            "SELECT id, actor_id, target_post_id, target_thread_id, target_user_id,
                    report_id, action, reason, note, created_at
             FROM moderation_events
             WHERE target_user_id = $1
             ORDER BY created_at DESC
             LIMIT $2",
        )
        .bind(target_user_id)
        .bind(limit)
        .fetch_all(executor)
        .await
    }

    pub async fn list_filtered<'e, E>(
        &self,
        executor: E,
        filter: ModerationEventFilter<'_>,
    ) -> Result<Vec<ModerationEventRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, ModerationEventRow>(
            "SELECT id, actor_id, target_post_id, target_thread_id, target_user_id,
                    report_id, action, reason, note, created_at
             FROM moderation_events
             WHERE ($1::text IS NULL OR action = $1)
               AND ($2::uuid IS NULL OR actor_id = $2)
               AND ($3::uuid IS NULL OR report_id = $3)
               AND ($4::uuid IS NULL OR target_user_id = $4)
               AND ($5::timestamptz IS NULL OR created_at >= $5)
               AND ($6::timestamptz IS NULL OR created_at < $6)
             ORDER BY created_at DESC
             LIMIT $7",
        )
        .bind(filter.action)
        .bind(filter.actor_id)
        .bind(filter.report_id)
        .bind(filter.target_user_id)
        .bind(filter.created_from)
        .bind(filter.created_to)
        .bind(filter.limit)
        .fetch_all(executor)
        .await
    }
}
