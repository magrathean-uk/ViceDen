//! Invites repository: issuance, redemption, revocation, subtree walk.
//!
//! NOTE: Like `users.rs`, `sessions.rs`, `posts.rs`, and `threads.rs`, this
//! module uses runtime SQL via `sqlx::query_as::<_, T>` (with `#[derive(FromRow)]`)
//! rather than the compile-time `sqlx::query_as!` / `sqlx::query!` macros. The
//! macro forms require either a live `DATABASE_URL` at build time or prepared
//! offline metadata in `.sqlx/`, neither of which the workspace ships today.
//! Migrating to macros later is API-compatible.

use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, Postgres};
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct InviteRow {
    pub code: String,
    pub label: String,
    #[sqlx(rename = "inviter_id")]
    pub inviter_user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_redemptions: i32,
    pub redemption_count: i32,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoke_reason: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug)]
pub struct NewInvite {
    pub code: String,
    pub inviter_user_id: Uuid,
    pub max_redemptions: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
}

#[derive(Debug)]
pub struct RedeemInvite {
    pub code: String,
    pub user_id: Uuid,
    pub redeemed_from_ip: Option<String>,
}

#[derive(Clone, Debug, FromRow)]
pub struct InviteRedemptionRow {
    pub invite_code: String,
    pub redeemed_user_id: Uuid,
    pub redeemed_at: DateTime<Utc>,
}

#[derive(Clone, Copy)]
pub struct InviteRepo;

impl InviteRepo {
    pub async fn issue<'e, E>(&self, executor: E, invite: NewInvite) -> Result<String, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query(
            "INSERT INTO invites (code, label, inviter_id, created_at,
                                  expires_at, max_redemptions, redemption_count, note)
             VALUES ($1, '', $2, now(), $3, $4, 0, $5)",
        )
        .bind(&invite.code)
        .bind(invite.inviter_user_id)
        .bind(invite.expires_at)
        .bind(invite.max_redemptions)
        .bind(invite.note)
        .execute(executor)
        .await?;
        Ok(invite.code)
    }

    pub async fn find<'e, E>(
        &self,
        executor: E,
        code: &str,
    ) -> Result<Option<InviteRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, InviteRow>(
            "SELECT code, label, inviter_id, created_at, expires_at,
                    max_redemptions, redemption_count, revoked_at, revoke_reason, note
             FROM invites WHERE code = $1",
        )
        .bind(code)
        .fetch_optional(executor)
        .await
    }

    /// Redemption mutates two tables and therefore takes a borrowed
    /// `PgConnection` (re-borrow of `&mut *tx` works here) rather than the
    /// generic `Executor`, which would be consumed by the first `execute`.
    ///
    /// Atomic capacity guard: the `UPDATE ... WHERE redemption_count <
    /// max_redemptions` is the source of truth. Two concurrent redeemers
    /// racing on the same single-use code will see exactly one
    /// `rows_affected() == 1`; the loser receives `RowNotFound`. Callers
    /// MUST still pre-validate revoked / expired (this is enforced by the
    /// pre-checks in `auth::invites::redeem_invite`); the SQL guard only
    /// addresses the count race.
    pub async fn redeem(
        &self,
        executor: &mut sqlx::PgConnection,
        redeem: RedeemInvite,
    ) -> Result<(), sqlx::Error> {
        let updated = sqlx::query(
            "UPDATE invites SET redemption_count = redemption_count + 1
             WHERE code = $1 AND redemption_count < max_redemptions",
        )
        .bind(&redeem.code)
        .execute(&mut *executor)
        .await?;

        if updated.rows_affected() == 0 {
            // Capacity exhausted (or row missing). Map to RowNotFound so
            // the redemption-error mapper above can collapse this into
            // `RedemptionError::InviteSpent` without leaking SQL detail.
            return Err(sqlx::Error::RowNotFound);
        }

        sqlx::query(
            "INSERT INTO invite_redemptions (invite_code, redeemed_user_id, redeemed_from_ip)
             VALUES ($1, $2, $3::inet)",
        )
        .bind(&redeem.code)
        .bind(redeem.user_id)
        .bind(redeem.redeemed_from_ip.as_deref())
        .execute(&mut *executor)
        .await?;
        Ok(())
    }

    pub async fn revoke<'e, E>(
        &self,
        executor: E,
        code: &str,
        _actor: Uuid,
        reason: &str,
    ) -> Result<(), sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query("UPDATE invites SET revoked_at = now(), revoke_reason = $2 WHERE code = $1")
            .bind(code)
            .bind(reason)
            .execute(executor)
            .await?;
        Ok(())
    }

    pub async fn list_for_inviter<'e, E>(
        &self,
        executor: E,
        inviter: Uuid,
    ) -> Result<Vec<InviteRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, InviteRow>(
            "SELECT code, label, inviter_id, created_at, expires_at,
                    max_redemptions, redemption_count, revoked_at, revoke_reason, note
             FROM invites WHERE inviter_id = $1
             ORDER BY created_at DESC",
        )
        .bind(inviter)
        .fetch_all(executor)
        .await
    }

    pub async fn list_all<'e, E>(&self, executor: E) -> Result<Vec<InviteRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        sqlx::query_as::<_, InviteRow>(
            "SELECT code, label, inviter_id, created_at, expires_at,
                    max_redemptions, redemption_count, revoked_at, revoke_reason, note
             FROM invites
             ORDER BY created_at DESC",
        )
        .fetch_all(executor)
        .await
    }

    pub async fn list_redemptions_for_codes<'e, E>(
        &self,
        executor: E,
        codes: &[String],
    ) -> Result<Vec<InviteRedemptionRow>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if codes.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, InviteRedemptionRow>(
            "SELECT invite_code, redeemed_user_id, redeemed_at
             FROM invite_redemptions
             WHERE invite_code = ANY($1)
             ORDER BY redeemed_at ASC",
        )
        .bind(codes)
        .fetch_all(executor)
        .await
    }

    pub async fn count_unspent_for_inviters<'e, E>(
        &self,
        executor: E,
        inviter_ids: &[Uuid],
    ) -> Result<i64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if inviter_ids.is_empty() {
            return Ok(0);
        }
        sqlx::query_scalar(
            "SELECT COUNT(*)::bigint
             FROM invites
             WHERE inviter_id = ANY($1::uuid[])
               AND revoked_at IS NULL
               AND redemption_count < max_redemptions",
        )
        .bind(inviter_ids)
        .fetch_one(executor)
        .await
    }

    pub async fn revoke_unspent_for_inviters<'e, E>(
        &self,
        executor: E,
        inviter_ids: &[Uuid],
        reason: &str,
    ) -> Result<u64, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        if inviter_ids.is_empty() {
            return Ok(0);
        }
        let result = sqlx::query(
            "UPDATE invites
             SET revoked_at = now(), revoke_reason = $2
             WHERE inviter_id = ANY($1::uuid[])
               AND revoked_at IS NULL
               AND redemption_count < max_redemptions",
        )
        .bind(inviter_ids)
        .bind(reason)
        .execute(executor)
        .await?;
        Ok(result.rows_affected())
    }

    /// Walk the invite tree downward from `root_user_id`. Returns the user_ids
    /// of every descendant (not the root itself).
    pub async fn subtree_descendants<'e, E>(
        &self,
        executor: E,
        root_user_id: Uuid,
    ) -> Result<Vec<Uuid>, sqlx::Error>
    where
        E: Executor<'e, Database = Postgres>,
    {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            r#"WITH RECURSIVE descendants AS (
                   SELECT r.redeemed_user_id AS user_id
                   FROM invites i
                   JOIN invite_redemptions r ON r.invite_code = i.code
                   WHERE i.inviter_id = $1
                 UNION ALL
                   SELECT r.redeemed_user_id
                   FROM descendants d
                   JOIN invites i        ON i.inviter_id   = d.user_id
                   JOIN invite_redemptions r ON r.invite_code = i.code
               )
               SELECT user_id FROM descendants"#,
        )
        .bind(root_user_id)
        .fetch_all(executor)
        .await?;
        Ok(rows.into_iter().map(|(u,)| u).collect())
    }
}
