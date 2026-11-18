//! Integration tests for the age attestations repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_age_attestations -- --ignored --test-threads=1
//! ```

mod common;

use chrono::NaiveDate;
use uuid::Uuid;
use viceden::repository::age_attestations::{AgeAttestationRepo, NewAttestation};
use viceden::repository::invites::{InviteRepo, NewInvite};
use viceden::repository::users::{NewUser, UserRepo};

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn insert_attestation_then_fetch() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let founder = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: founder,
                handle: format!("at-f-{}", &founder.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "founder".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();
    let invitee = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: invitee,
                handle: format!("at-i-{}", &invitee.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();

    InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "vdn-inv-att1".into(),
                inviter_user_id: founder,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();

    AgeAttestationRepo
        .insert(
            &mut *tx,
            NewAttestation {
                user_id: invitee,
                dob: NaiveDate::from_ymd_opt(1990, 5, 1).unwrap(),
                attested_ip: "1.2.3.4".into(),
                attested_ua: "ua/1.0".into(),
                invite_code: "vdn-inv-att1".into(),
            },
        )
        .await
        .unwrap();

    let row = AgeAttestationRepo.find_for_user(&mut *tx, invitee).await.unwrap().unwrap();
    assert_eq!(row.dob, NaiveDate::from_ymd_opt(1990, 5, 1).unwrap());
    assert_eq!(row.invite_code, "vdn-inv-att1");

    tx.rollback().await.unwrap();
}

/// F16: age-attestation rows are write-once — once a user has attested,
/// the row is immutable and a re-insert MUST fail. Two parts:
///
/// 1. Source-level check: the codebase must never UPDATE the
///    `age_attestations` table. We grep src/ at test-time so a future
///    refactor that adds an UPDATE statement is caught immediately.
/// 2. Database-level check: re-inserting an attestation for the same
///    user_id violates the UNIQUE / PK constraint. The `expect_err`
///    call asserts the second insert returns a sqlx error rather than
///    silently overwriting.
#[test]
fn no_update_against_age_attestations_in_source() {
    use std::path::PathBuf;
    let src_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    walk_rust_files(&src_root, &mut |path, contents| {
        // Match `UPDATE age_attestations` (case-insensitive) and any
        // `age_attestations SET` form. Exclude comment-only mentions
        // by requiring it on a non-`//` line.
        for (line_no, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if lower.contains("update age_attestations") || lower.contains("age_attestations set ")
            {
                hits.push(format!("{}:{}: {}", path.display(), line_no + 1, line.trim()));
            }
        }
    });
    assert!(
        hits.is_empty(),
        "F16 regression: age_attestations is write-once; found UPDATE statements:\n  {}",
        hits.join("\n  ")
    );
}

fn walk_rust_files(dir: &std::path::Path, visit: &mut impl FnMut(&std::path::Path, &str)) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_rust_files(&path, visit);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    visit(&path, &contents);
                }
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn re_insert_attestation_fails_unique_constraint() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let founder = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: founder,
                handle: format!("aim-f-{}", &founder.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "founder".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();
    let invitee = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: invitee,
                handle: format!("aim-i-{}", &invitee.simple().to_string()[..6]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();
    InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "vdn-inv-im1".into(),
                inviter_user_id: founder,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();

    AgeAttestationRepo
        .insert(
            &mut *tx,
            NewAttestation {
                user_id: invitee,
                dob: NaiveDate::from_ymd_opt(1990, 5, 1).unwrap(),
                attested_ip: "1.2.3.4".into(),
                attested_ua: "ua/1.0".into(),
                invite_code: "vdn-inv-im1".into(),
            },
        )
        .await
        .unwrap();

    // Second insert for the same user_id with a DIFFERENT DOB MUST
    // fail. If it silently overwrites, the audit row is no longer
    // trustworthy as a write-once record.
    sqlx::query("SAVEPOINT age_attestation_unique_check").execute(&mut *tx).await.unwrap();
    let err = AgeAttestationRepo
        .insert(
            &mut *tx,
            NewAttestation {
                user_id: invitee,
                dob: NaiveDate::from_ymd_opt(1985, 1, 1).unwrap(),
                attested_ip: "5.6.7.8".into(),
                attested_ua: "ua/2.0".into(),
                invite_code: "vdn-inv-im1".into(),
            },
        )
        .await
        .expect_err("F16: re-insert for same user_id must fail UNIQUE constraint");
    let msg = format!("{err:?}");
    assert!(
        msg.to_lowercase().contains("unique") || msg.to_lowercase().contains("duplicate"),
        "expected unique-constraint violation, got: {msg}"
    );
    sqlx::query("ROLLBACK TO SAVEPOINT age_attestation_unique_check")
        .execute(&mut *tx)
        .await
        .unwrap();

    // Original DOB still in place — proves the second insert didn't
    // partially apply or replace the row.
    let row = AgeAttestationRepo.find_for_user(&mut *tx, invitee).await.unwrap().unwrap();
    assert_eq!(row.dob, NaiveDate::from_ymd_opt(1990, 5, 1).unwrap());

    tx.rollback().await.unwrap();
}
