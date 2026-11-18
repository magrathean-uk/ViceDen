//! Preferences repository: per-user `user_settings`, `board_subscriptions`,
//! `saved_threads`, and `hidden_threads`.
//!
//! NOTE: Like `users.rs` and the rest of the repository layer, this module
//! uses runtime SQL via `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`)
//! rather than the compile-time `sqlx::query_as!` macros. The macro forms
//! require either a live `DATABASE_URL` at build time or prepared offline
//! metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.
//!
//! `user_settings.compactness` is `SMALLINT` on disk (see migration 0004) so
//! the row maps it as `i16`. `UserSettings` exposes it as `i8` to match the
//! existing `crate::forum::UserProfileSettings` shape.

use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct UserSettings {
    pub compactness: i8,
    pub theme: String,
    pub notify_mentions: bool,
    pub notify_replies: bool,
    pub notify_moderation: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        // Mirrors `forum::UserProfileSettings::default()` so newly-seen users
        // get the same "dark + 0" preset whether they came in via the old
        // forum store or this repo.
        Self {
            compactness: 0,
            theme: "dark".to_string(),
            notify_mentions: true,
            notify_replies: true,
            notify_moderation: true,
        }
    }
}

#[derive(Debug)]
pub struct UpdateSettings {
    pub compactness: i8,
    pub theme: String,
    pub notify_mentions: bool,
    pub notify_replies: bool,
    pub notify_moderation: bool,
}

#[derive(Clone, Debug, FromRow)]
struct UserSettingsRow {
    compactness: i16,
    theme: String,
    notify_mentions: bool,
    notify_replies: bool,
    notify_moderation: bool,
}

#[derive(Clone, Copy)]
pub struct PreferencesRepo;

impl PreferencesRepo {
    /// Fetch the user's settings, falling back to the default when no row
    /// exists yet. The `user_settings` row is created lazily on first update.
    pub async fn settings_for<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<UserSettings, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let row = sqlx::query_as::<_, UserSettingsRow>(
            "SELECT compactness, theme, notify_mentions, notify_replies, notify_moderation
             FROM user_settings WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(executor)
        .await?;
        Ok(match row {
            Some(row) => UserSettings {
                compactness: row.compactness as i8,
                theme: row.theme,
                notify_mentions: row.notify_mentions,
                notify_replies: row.notify_replies,
                notify_moderation: row.notify_moderation,
            },
            None => UserSettings::default(),
        })
    }

    pub async fn update_settings<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        settings: UpdateSettings,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO user_settings (
                user_id, compactness, theme, notify_mentions, notify_replies, notify_moderation
             )
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (user_id) DO UPDATE
             SET compactness = EXCLUDED.compactness,
                 theme = EXCLUDED.theme,
                 notify_mentions = EXCLUDED.notify_mentions,
                 notify_replies = EXCLUDED.notify_replies,
                 notify_moderation = EXCLUDED.notify_moderation",
        )
        .bind(user_id)
        .bind(settings.compactness as i16)
        .bind(&settings.theme)
        .bind(settings.notify_mentions)
        .bind(settings.notify_replies)
        .bind(settings.notify_moderation)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn subscribe_board<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        slug: &str,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO board_subscriptions (user_id, board_slug)
             VALUES ($1, $2)
             ON CONFLICT (user_id, board_slug) DO NOTHING",
        )
        .bind(user_id)
        .bind(slug)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn unsubscribe_board<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        slug: &str,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM board_subscriptions WHERE user_id = $1 AND board_slug = $2")
            .bind(user_id)
            .bind(slug)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn subscribed_slugs_for<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<Vec<String>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT board_slug FROM board_subscriptions WHERE user_id = $1
             ORDER BY board_slug",
        )
        .bind(user_id)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(|(slug,)| slug).collect())
    }

    pub async fn save_thread<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO saved_threads (user_id, thread_id)
             VALUES ($1, $2)
             ON CONFLICT (user_id, thread_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(thread_id)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn unsave_thread<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM saved_threads WHERE user_id = $1 AND thread_id = $2")
            .bind(user_id)
            .bind(thread_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn hide_thread<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO hidden_threads (user_id, thread_id)
             VALUES ($1, $2)
             ON CONFLICT (user_id, thread_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(thread_id)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn unhide_thread<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM hidden_threads WHERE user_id = $1 AND thread_id = $2")
            .bind(user_id)
            .bind(thread_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn saved_threads_for<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<Vec<Uuid>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(Uuid,)> =
            sqlx::query_as("SELECT thread_id FROM saved_threads WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(executor)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn hidden_threads_for<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
    ) -> Result<Vec<Uuid>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(Uuid,)> =
            sqlx::query_as("SELECT thread_id FROM hidden_threads WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(executor)
                .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    /// Single-thread "is saved" check. Index-fast (PK on
    /// `saved_threads(user_id, thread_id)`). Used by `view_thread` and the
    /// thread-preview render path so they don't have to fan out
    /// `saved_threads_for(user)` and re-scan it per row.
    pub async fn is_thread_saved<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<bool, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exists: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM saved_threads WHERE user_id = $1 AND thread_id = $2")
                .bind(user_id)
                .bind(thread_id)
                .fetch_optional(executor)
                .await?;
        Ok(exists.is_some())
    }

    /// Single-thread "is hidden" check. Index-fast (PK on
    /// `hidden_threads(user_id, thread_id)`). Used by `view_thread` to
    /// surface the un-hide control and by `view_board` / `index` to filter
    /// per-row.
    pub async fn is_thread_hidden<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        thread_id: Uuid,
    ) -> Result<bool, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM hidden_threads WHERE user_id = $1 AND thread_id = $2",
        )
        .bind(user_id)
        .bind(thread_id)
        .fetch_optional(executor)
        .await?;
        Ok(exists.is_some())
    }

    /// "Is the user subscribed to this board" check. Index-fast (PK on
    /// `board_subscriptions(user_id, board_slug)`). Used by `view_board` to
    /// render the subscribe / unsubscribe toggle.
    pub async fn is_board_subscribed<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        slug: &str,
    ) -> Result<bool, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM board_subscriptions WHERE user_id = $1 AND board_slug = $2",
        )
        .bind(user_id)
        .bind(slug)
        .fetch_optional(executor)
        .await?;
        Ok(exists.is_some())
    }

    pub async fn mute_user<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        muted_user_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO muted_users (user_id, muted_user_id)
             VALUES ($1, $2)
             ON CONFLICT (user_id, muted_user_id) DO NOTHING",
        )
        .bind(user_id)
        .bind(muted_user_id)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn unmute_user<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        muted_user_id: Uuid,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM muted_users WHERE user_id = $1 AND muted_user_id = $2")
            .bind(user_id)
            .bind(muted_user_id)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn follow_tag<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        tag: &str,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO tag_subscriptions (user_id, tag)
             VALUES ($1, $2)
             ON CONFLICT (user_id, tag) DO NOTHING",
        )
        .bind(user_id)
        .bind(tag)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn unfollow_tag<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        tag: &str,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("DELETE FROM tag_subscriptions WHERE user_id = $1 AND tag = $2")
            .bind(user_id)
            .bind(tag)
            .execute(executor)
            .await?;
        Ok(())
    }
}
