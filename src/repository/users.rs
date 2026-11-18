//! Users repository: CRUD, role checks, cauterise, handle blocklist lookup.
//!
//! NOTE: This module uses runtime SQL via `sqlx::query_as::<_, T>` rather than
//! the compile-time `sqlx::query_as!` macro. The macro form requires either a
//! live `DATABASE_URL` at build time or prepared offline metadata in `.sqlx/`,
//! and the workspace currently ships neither. Runtime forms keep `cargo check`
//! and CI build steps DB-free at the cost of moving SQL validation to runtime.
//! When/if offline metadata is added, the queries can be migrated to macros
//! without changing the public API.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, PgPool, Postgres};
use uuid::Uuid;

use crate::repository::invites::InviteRepo;

#[derive(Clone, Debug, FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub handle: String,
    pub password_hash: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub banned: bool,
    pub ban_reason: Option<String>,
    pub probation_until: Option<DateTime<Utc>>,
    pub last_known_ip: Option<String>,
    pub deactivated_at: Option<DateTime<Utc>>,
    pub deactivated_by: Option<Uuid>,
    pub deactivated_reason: Option<String>,
}

#[derive(Debug)]
pub struct NewUser {
    pub id: Uuid,
    pub handle: String,
    pub password_hash: String,
    pub role: String,
    pub last_known_ip: Option<String>,
}

/// Per-user aggregate row used by the admin user list / detail handlers.
/// Mirrors `forum::AdminUserSummary` but uses PG-native counts (`i64`) and
/// stays in the repository layer (no business logic — `can_manage` is derived
/// at the handler boundary).
#[derive(Clone, Debug, FromRow)]
pub struct UserSummary {
    pub id: Uuid,
    pub handle: String,
    pub role: String,
    pub banned: bool,
    pub deactivated_at: Option<DateTime<Utc>>,
    pub probation_until: Option<DateTime<Utc>>,
    pub thread_count: i64,
    pub post_count: i64,
}

#[derive(Clone, Debug)]
pub struct CauteriseSubtreeOutcome {
    pub subtree_user_ids: Vec<Uuid>,
    pub descendant_count: usize,
    pub users_cauterised: usize,
    pub threads_removed: usize,
    pub posts_removed: usize,
    pub invites_revoked: usize,
}

/// One row of a user's recent posting activity, with the parent thread's
/// title and board slug already joined in. Mirrors `forum::UserActivityTeaser`.
///
/// `body_snippet` is truncated to the first 90 characters (PG codepoint
/// semantics via `LEFT()`; differs from Rust's `chars().take(90)` only on
/// surrogate-pair / grapheme-cluster boundaries — acceptable for an admin
/// teaser view).
#[derive(Clone, Debug, FromRow)]
pub struct UserActivityItem {
    pub board_slug: String,
    pub thread_id: Uuid,
    pub post_id: Uuid,
    pub thread_title: String,
    pub body_snippet: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy)]
pub struct UserRepo;

impl UserRepo {
    pub async fn insert<'e, E>(&self, executor: E, user: NewUser) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO users (id, handle, password_hash, role, last_known_ip)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user.id)
        .bind(&user.handle)
        .bind(&user.password_hash)
        .bind(&user.role)
        .bind(user.last_known_ip)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn find_by_handle<'e, E>(
        &self,
        executor: E,
        handle: &str,
    ) -> Result<Option<UserRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, handle, password_hash, role, created_at, banned, ban_reason,
                    probation_until, last_known_ip,
                    deactivated_at, deactivated_by, deactivated_reason
             FROM users WHERE handle = $1",
        )
        .bind(handle)
        .fetch_optional(executor)
        .await
    }

    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: Uuid,
    ) -> Result<Option<UserRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, handle, password_hash, role, created_at, banned, ban_reason,
                    probation_until, last_known_ip,
                    deactivated_at, deactivated_by, deactivated_reason
             FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    pub async fn cauterise<'e, E>(
        &self,
        executor: E,
        target: Uuid,
        actor: Uuid,
        reason: &str,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "UPDATE users
             SET deactivated_at = now(), deactivated_by = $2, deactivated_reason = $3
             WHERE id = $1",
        )
        .bind(target)
        .bind(actor)
        .bind(reason)
        .execute(executor)
        .await?;
        Ok(())
    }

    pub async fn set_probation_until<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        probation_until: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("UPDATE users SET probation_until = $2 WHERE id = $1")
            .bind(user_id)
            .bind(probation_until)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn cauterise_subtree(
        &self,
        pool: &PgPool,
        actor_id: Uuid,
        target_id: Uuid,
        reason: &str,
    ) -> Result<Option<CauteriseSubtreeOutcome>, sqlx::Error> {
        let Some(actor) = self.find_by_id(pool, actor_id).await? else {
            return Ok(None);
        };
        let Some(target) = self.find_by_id(pool, target_id).await? else {
            return Ok(None);
        };
        if actor.role != "founder" || target.role == "founder" {
            return Ok(None);
        }

        let mut tx = pool.begin().await?;
        let mut subtree_user_ids = vec![target_id];
        for user_id in InviteRepo.subtree_descendants(&mut *tx, target_id).await? {
            if !subtree_user_ids.contains(&user_id) {
                subtree_user_ids.push(user_id);
            }
        }

        let founder_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::bigint
             FROM users
             WHERE id = ANY($1::uuid[]) AND role = 'founder'",
        )
        .bind(&subtree_user_ids)
        .fetch_one(&mut *tx)
        .await?;
        if founder_count > 0 {
            return Ok(None);
        }

        let users_cauterised = sqlx::query(
            "UPDATE users
             SET deactivated_at = now(), deactivated_by = $2, deactivated_reason = $3
             WHERE id = ANY($1::uuid[])
               AND deactivated_at IS NULL",
        )
        .bind(&subtree_user_ids)
        .bind(actor_id)
        .bind(reason)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;

        let threads_removed = sqlx::query(
            "UPDATE threads
             SET mod_state = 'removed',
                 mod_state_reason = CONCAT('cauterised_from_', mod_state),
                 mod_state_changed_at = now()
             WHERE user_id = ANY($1::uuid[])
               AND mod_state <> 'removed'",
        )
        .bind(&subtree_user_ids)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;

        let posts_removed = sqlx::query(
            "UPDATE posts
             SET mod_state = 'removed',
                 mod_state_reason = CONCAT('cauterised_from_', mod_state),
                 mod_state_changed_at = now()
             WHERE user_id = ANY($1::uuid[])
               AND mod_state <> 'removed'",
        )
        .bind(&subtree_user_ids)
        .execute(&mut *tx)
        .await?
        .rows_affected() as usize;

        let invites_revoked = InviteRepo
            .revoke_unspent_for_inviters(&mut *tx, &subtree_user_ids, reason)
            .await? as usize;

        tx.commit().await?;

        Ok(Some(CauteriseSubtreeOutcome {
            descendant_count: subtree_user_ids.len().saturating_sub(1),
            subtree_user_ids,
            users_cauterised,
            threads_removed,
            posts_removed,
            invites_revoked,
        }))
    }

    pub async fn handle_blocked<'e, E>(
        &self,
        executor: E,
        handle: &str,
    ) -> Result<bool, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let lc = handle.to_ascii_lowercase();
        let row: Option<(String, bool)> = sqlx::query_as(
            "SELECT handle_pattern, is_regex FROM handle_blocklist
             WHERE (NOT is_regex AND $1 = handle_pattern)
                OR (is_regex AND $1 ~ handle_pattern)
             LIMIT 1",
        )
        .bind(&lc)
        .fetch_optional(executor)
        .await?;
        Ok(row.is_some())
    }

    /// Batch-resolve `(id, handle)` for the given user ids. Used by render
    /// paths that today fan out to `ForumStore::user_display_name`. Order of
    /// the returned `Vec` is undefined; callers should index into a HashMap.
    /// Unknown ids are silently skipped (not present in the result).
    pub async fn handle_batch_lookup<'e, E>(
        &self,
        executor: E,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String)>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, handle FROM users WHERE id = ANY($1::uuid[])")
                .bind(ids)
                .fetch_all(executor)
                .await?;
        Ok(rows)
    }

    /// Total number of user rows. Used by `view_metrics`.
    pub async fn count<'e, E>(&self, executor: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(executor).await?;
        Ok(count)
    }

    /// Number of currently-banned users. Used by `view_metrics`.
    pub async fn count_banned<'e, E>(&self, executor: E) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE banned = TRUE")
            .fetch_one(executor)
            .await?;
        Ok(count)
    }

    /// Paged user list, newest first. Used by `view_users` (admin list).
    /// `limit` is clamped to 500 by the handler — this method does not enforce.
    pub async fn list_paged<'e, E>(
        &self,
        executor: E,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UserRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, handle, password_hash, role, created_at, banned, ban_reason,
                    probation_until, last_known_ip,
                    deactivated_at, deactivated_by, deactivated_reason
             FROM users
             ORDER BY created_at DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(executor)
        .await
    }

    /// Single-user summary: id, handle, role, banned, plus thread/post counts.
    /// Used by `view_user_detail`. Returns `None` if the user does not exist.
    /// SQL shape mirrors `db::list_admin_user_summaries` (LEFT JOIN over
    /// per-user count subqueries) so behavior matches today's render path.
    pub async fn summary_with_counts<'e, E>(
        &self,
        executor: E,
        id: Uuid,
    ) -> Result<Option<UserSummary>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, UserSummary>(
            r#"
            SELECT
                u.id,
                u.handle,
                u.role,
                u.banned,
                u.deactivated_at,
                u.probation_until,
                COALESCE(tc.thread_count, 0) AS thread_count,
                COALESCE(pc.post_count,   0) AS post_count
            FROM users u
            LEFT JOIN (
                SELECT user_id, COUNT(*)::bigint AS thread_count
                FROM threads GROUP BY user_id
            ) tc ON tc.user_id = u.id
            LEFT JOIN (
                SELECT user_id, COUNT(*)::bigint AS post_count
                FROM posts GROUP BY user_id
            ) pc ON pc.user_id = u.id
            WHERE u.id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(executor)
        .await
    }

    /// Full user-summary list for the admin /admin/users page. Ordering
    /// matches the legacy `db::list_admin_user_summaries`: founders first,
    /// then handle ASC. Used by `view_users` to render the admin user table.
    /// The handler filters by search term in memory; a paged variant will
    /// land if/when the user table grows past the in-memory budget.
    pub async fn list_summaries_for_admin<'e, E>(
        &self,
        executor: E,
    ) -> Result<Vec<UserSummary>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, UserSummary>(
            r#"
            SELECT
                u.id,
                u.handle,
                u.role,
                u.banned,
                u.deactivated_at,
                u.probation_until,
                COALESCE(tc.thread_count, 0) AS thread_count,
                COALESCE(pc.post_count,   0) AS post_count
            FROM users u
            LEFT JOIN (
                SELECT user_id, COUNT(*)::bigint AS thread_count
                FROM threads GROUP BY user_id
            ) tc ON tc.user_id = u.id
            LEFT JOIN (
                SELECT user_id, COUNT(*)::bigint AS post_count
                FROM posts GROUP BY user_id
            ) pc ON pc.user_id = u.id
            ORDER BY CASE WHEN u.role = 'founder' THEN 0 ELSE 1 END, u.handle ASC
            "#,
        )
        .fetch_all(executor)
        .await
    }

    /// Most-recent posts by `user_id`, joined with the parent thread's title
    /// and board slug. Used by `view_user_detail`. Limit is the upper bound
    /// on returned rows; the handler typically passes 10. No moderation-state
    /// filter — admins are expected to see hidden / pending content too,
    /// matching the legacy in-memory behavior.
    pub async fn latest_activity<'e, E>(
        &self,
        executor: E,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<UserActivityItem>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, UserActivityItem>(
            r#"
            SELECT
                t.board_slug         AS board_slug,
                t.id                 AS thread_id,
                p.id                 AS post_id,
                t.title              AS thread_title,
                LEFT(p.body, 90)     AS body_snippet,
                p.created_at         AS created_at
            FROM posts p
            JOIN threads t ON t.id = p.thread_id
            WHERE p.user_id = $1
            ORDER BY p.created_at DESC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(executor)
        .await
    }

    pub async fn wipe_content(
        &self,
        pool: &PgPool,
        actor_id: Uuid,
        target_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        let Some(actor) = self.find_by_id(pool, actor_id).await? else {
            return Ok(false);
        };
        let Some(target) = self.find_by_id(pool, target_id).await? else {
            return Ok(false);
        };
        if actor.role != "founder" || target.role == "founder" {
            return Ok(false);
        }
        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM threads WHERE user_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM posts WHERE user_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM votes WHERE user_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM notifications WHERE recipient_user_id = $1 OR actor_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM reports WHERE actor_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM moderation_events WHERE actor_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn delete_user(
        &self,
        pool: &PgPool,
        actor_id: Uuid,
        target_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        let Some(actor) = self.find_by_id(pool, actor_id).await? else {
            return Ok(false);
        };
        let Some(target) = self.find_by_id(pool, target_id).await? else {
            return Ok(false);
        };
        if actor.role != "founder" || target.role == "founder" {
            return Ok(false);
        }
        let mut tx = pool.begin().await?;
        if let Some(ip) = target.last_known_ip {
            sqlx::query("INSERT INTO banned_ips (ip) VALUES ($1) ON CONFLICT (ip) DO NOTHING")
                .bind(ip)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM notifications WHERE actor_id = $1")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM reports WHERE target_post_id IN (SELECT id FROM posts WHERE user_id = $1)
                    OR target_thread_id IN (SELECT id FROM threads WHERE user_id = $1)",
        )
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM moderation_events WHERE target_post_id IN (SELECT id FROM posts WHERE user_id = $1)
                    OR target_thread_id IN (SELECT id FROM threads WHERE user_id = $1)")
            .bind(target_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM users WHERE id = $1").bind(target_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}
