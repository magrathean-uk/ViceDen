//! Integration tests for the high-level invite redemption flow.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test auth_invites_redeem -- --ignored --test-threads=1
//! ```

mod common;

use chrono::NaiveDate;
use uuid::Uuid;
use viceden::auth::invites::{redeem_invite, RedemptionContext, RedemptionError};
use viceden::repository::invites::{InviteRepo, NewInvite};
use viceden::repository::users::{NewUser, UserRepo};

async fn user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, p: &str) -> Uuid {
    let id = Uuid::new_v4();
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id,
                handle: format!("{p}-{}", &id.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "founder".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn happy_path_redeems_invite_and_inserts_attestation() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();
    let founder = user(&mut tx, "fou").await;
    InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "vdn-inv-redeem1".into(),
                inviter_user_id: founder,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let outcome = redeem_invite(
        &pool,
        RedemptionContext {
            code: "vdn-inv-redeem1",
            handle: "newbie01",
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def",
            dob: NaiveDate::from_ymd_opt(1990, 1, 1).unwrap(),
            today: NaiveDate::from_ymd_opt(2026, 5, 8).unwrap(),
            attested_ip: "1.2.3.4",
            attested_ua: "ua/1.0",
        },
    )
    .await
    .unwrap();

    assert!(outcome.user_id != Uuid::nil());

    // Cleanup.
    sqlx::query("DELETE FROM age_attestations WHERE user_id = $1")
        .bind(outcome.user_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM invite_redemptions WHERE redeemed_user_id = $1")
        .bind(outcome.user_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE id = $1").bind(outcome.user_id).execute(&pool).await.ok();
    sqlx::query("DELETE FROM invites WHERE code = $1")
        .bind("vdn-inv-redeem1")
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn invite_validation_runs_before_dob_check() {
    let pool = common::shared_pool().await;
    let err = redeem_invite(
        &pool,
        RedemptionContext {
            code: "vdn-inv-does-not-exist",
            handle: "newbie02",
            password_hash: "x",
            dob: NaiveDate::from_ymd_opt(2050, 1, 1).unwrap(), // future DOB; would fail age check
            today: NaiveDate::from_ymd_opt(2026, 5, 8).unwrap(),
            attested_ip: "1.2.3.4",
            attested_ua: "ua/1.0",
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, RedemptionError::InviteNotFound),
        "invite check must come before DOB check (no oracle), got {err:?}"
    );
}
