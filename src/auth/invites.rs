//! High-level invite redemption.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::age_attestation::{validate_dob, AgeError};
use crate::repository::age_attestations::{AgeAttestationRepo, NewAttestation};
use crate::repository::invites::{InviteRepo, RedeemInvite};
use crate::repository::users::{NewUser, UserRepo};

#[derive(Debug)]
pub struct RedemptionContext<'a> {
    pub code: &'a str,
    pub handle: &'a str,
    pub password_hash: &'a str,
    pub dob: NaiveDate,
    pub today: NaiveDate,
    pub attested_ip: &'a str,
    pub attested_ua: &'a str,
}

#[derive(Debug)]
pub struct RedemptionOutcome {
    pub user_id: Uuid,
}

#[derive(Debug, thiserror::Error)]
pub enum RedemptionError {
    #[error("invite not found")]
    InviteNotFound,
    #[error("invite revoked")]
    InviteRevoked,
    #[error("invite expired")]
    InviteExpired,
    #[error("invite spent")]
    InviteSpent,
    #[error("handle taken")]
    HandleTaken,
    #[error("handle blocked")]
    HandleBlocked,
    #[error("handle invalid")]
    HandleInvalid,
    #[error("future dob")]
    FutureDob,
    #[error("underage")]
    Underage,
    #[error("db: {0}")]
    Db(String),
}

impl From<AgeError> for RedemptionError {
    fn from(error: AgeError) -> Self {
        match error {
            AgeError::FutureDob => Self::FutureDob,
            AgeError::Underage => Self::Underage,
        }
    }
}

pub async fn redeem_invite(
    pool: &PgPool,
    ctx: RedemptionContext<'_>,
) -> Result<RedemptionOutcome, RedemptionError> {
    let mut tx = pool.begin().await.map_err(|e| RedemptionError::Db(e.to_string()))?;

    // 1. Validate invite first — no DOB-only oracle for valid codes.
    let invite = InviteRepo
        .find(&mut *tx, ctx.code)
        .await
        .map_err(|e| RedemptionError::Db(e.to_string()))?
        .ok_or(RedemptionError::InviteNotFound)?;
    if invite.revoked_at.is_some() {
        return Err(RedemptionError::InviteRevoked);
    }
    if let Some(exp) = invite.expires_at {
        if exp <= chrono::Utc::now() {
            return Err(RedemptionError::InviteExpired);
        }
    }
    if invite.redemption_count >= invite.max_redemptions {
        return Err(RedemptionError::InviteSpent);
    }

    // 2. Handle.
    if !is_valid_handle(ctx.handle) {
        return Err(RedemptionError::HandleInvalid);
    }
    if UserRepo
        .handle_blocked(&mut *tx, ctx.handle)
        .await
        .map_err(|e| RedemptionError::Db(e.to_string()))?
    {
        return Err(RedemptionError::HandleBlocked);
    }
    let existing = UserRepo
        .find_by_handle(&mut *tx, ctx.handle)
        .await
        .map_err(|e| RedemptionError::Db(e.to_string()))?;
    if existing.is_some() {
        return Err(RedemptionError::HandleTaken);
    }

    // 3. DOB.
    validate_dob(ctx.dob, ctx.today)?;

    // 4. Insert.
    let user_id = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: user_id,
                handle: ctx.handle.to_string(),
                password_hash: ctx.password_hash.to_string(),
                role: "member".to_string(),
                last_known_ip: Some(ctx.attested_ip.to_string()),
            },
        )
        .await
        .map_err(|e| RedemptionError::Db(e.to_string()))?;
    UserRepo
        .set_probation_until(
            &mut *tx,
            user_id,
            Some(crate::services::limits::registration_probation_until(chrono::Utc::now())),
        )
        .await
        .map_err(|e| RedemptionError::Db(e.to_string()))?;

    AgeAttestationRepo
        .insert(
            &mut *tx,
            NewAttestation {
                user_id,
                dob: ctx.dob,
                attested_ip: ctx.attested_ip.to_string(),
                attested_ua: ctx.attested_ua.to_string(),
                invite_code: ctx.code.to_string(),
            },
        )
        .await
        .map_err(|e| RedemptionError::Db(e.to_string()))?;

    InviteRepo
        .redeem(
            &mut tx,
            RedeemInvite {
                code: ctx.code.to_string(),
                user_id,
                redeemed_from_ip: Some(ctx.attested_ip.to_string()),
            },
        )
        .await
        .map_err(|e| match e {
            // The atomic UPDATE in `InviteRepo::redeem` returns `RowNotFound`
            // when capacity has been exhausted (or the code is gone). This
            // is the loser's path of a double-spend race -- collapse it to
            // the same surface as the pre-check above.
            sqlx::Error::RowNotFound => RedemptionError::InviteSpent,
            other => RedemptionError::Db(other.to_string()),
        })?;

    tx.commit().await.map_err(|e| RedemptionError::Db(e.to_string()))?;

    Ok(RedemptionOutcome { user_id })
}

fn is_valid_handle(handle: &str) -> bool {
    let len_ok = (3..=24).contains(&handle.chars().count());
    let chars_ok = handle.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    len_ok
        && chars_ok
        && handle
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}
