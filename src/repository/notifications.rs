//! Notifications repository.
//!
//! NOTE: Like `users.rs` and the rest of the repository layer, this module
//! uses runtime SQL via `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`)
//! rather than the compile-time `sqlx::query_as!` macros. The macro forms
//! require either a live `DATABASE_URL` at build time or prepared offline
//! metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! The `kind` column stores a string label (see migration 0005) — this repo
//! treats it as opaque `String`. Callers map between the storage label and
//! their domain enum (`crate::forum::NotificationKind`).

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    pub recipient_user_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub kind: String,
    pub thread_id: Option<Uuid>,
    pub thread_title: String,
    pub post_id: Option<Uuid>,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub is_read: bool,
}

#[derive(Debug)]
pub struct NewNotification {
    pub id: Uuid,
    pub recipient_user_id: Uuid,
    pub actor_id: Option<Uuid>,
    pub kind: String,
    pub thread_id: Option<Uuid>,
    pub thread_title: String,
    pub post_id: Option<Uuid>,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub is_read: bool,
}

#[derive(Clone, Copy)]
pub struct NotificationRepo;

impl NotificationRepo {
    pub async fn insert<'e, E>(
        &self,
        executor: E,
        notification: NewNotification,
    ) -> Result<Uuid, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO notifications (
                id, recipient_user_id, actor_id, kind,
                thread_id, thread_title, post_id, message,
                created_at, is_read
             )
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
             WHERE CASE
                 WHEN $4 = 'mention' THEN COALESCE(
                     (SELECT notify_mentions FROM user_settings WHERE user_id = $2),
                     TRUE
                 )
                 WHEN $4 = 'reply' THEN COALESCE(
                     (SELECT notify_replies FROM user_settings WHERE user_id = $2),
                     TRUE
                 )
                 WHEN $4 IN ('post_removed', 'report', 'mod_action') THEN COALESCE(
                     (SELECT notify_moderation FROM user_settings WHERE user_id = $2),
                     TRUE
                 )
                 ELSE TRUE
             END",
        )
        .bind(notification.id)
        .bind(notification.recipient_user_id)
        .bind(notification.actor_id)
        .bind(&notification.kind)
        .bind(notification.thread_id)
        .bind(&notification.thread_title)
        .bind(notification.post_id)
        .bind(&notification.message)
        .bind(notification.created_at)
        .bind(notification.is_read)
        .execute(executor)
        .await?;
        Ok(notification.id)
    }

    pub async fn list_for_user<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<NotificationRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, NotificationRow>(
            "SELECT id, recipient_user_id, actor_id, kind,
                    thread_id, thread_title, post_id, message,
                    created_at, is_read
             FROM notifications
             WHERE recipient_user_id = $1
             ORDER BY created_at DESC
             LIMIT $2",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(executor)
        .await
    }

    pub async fn mark_read<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE notifications SET is_read = TRUE
             WHERE id = $1 AND recipient_user_id = $2",
        )
        .bind(id)
        .bind(user_id)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn mark_all_read<'e, E>(&self, executor: E, user_id: Uuid) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("UPDATE notifications SET is_read = TRUE WHERE recipient_user_id = $1")
            .bind(user_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    /// Delete a single notification by id, scoped to the recipient so a
    /// stolen notification id from another user can't be used to delete
    /// the wrong row. Used by `remove_notification_web`.
    pub async fn delete_for_user<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        user_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM notifications WHERE id = $1 AND recipient_user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    /// Set is_read on a single notification, scoped to the recipient.
    pub async fn set_read<'e, E>(
        &self,
        executor: E,
        id: Uuid,
        user_id: Uuid,
        read: bool,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE notifications SET is_read = $3
             WHERE id = $1 AND recipient_user_id = $2",
        )
        .bind(id)
        .bind(user_id)
        .bind(read)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn count_unread<'e, E>(&self, executor: E, user_id: Uuid) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications
             WHERE recipient_user_id = $1 AND is_read = FALSE",
        )
        .bind(user_id)
        .fetch_one(executor)
        .await?;
        Ok(count)
    }

    /// Global count of unread notifications across all users. Used by
    /// `view_metrics` to surface "unresolved notifications" on the admin
    /// dashboard. Does not require a per-user filter.
    pub async fn count_unread_global<'e, E>(&self, executor: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE is_read = FALSE")
                .fetch_one(executor)
                .await?;
        Ok(count)
    }
}
