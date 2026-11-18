//! Integration tests for the invites repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_invites -- --ignored --test-threads=1
//! ```

mod common;

use std::collections::{HashSet, VecDeque};

use proptest::prelude::*;
use uuid::Uuid;
use viceden::repository::invites::{InviteRepo, NewInvite, RedeemInvite};
use viceden::repository::users::{NewUser, UserRepo};

async fn user(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, p: &str) -> Uuid {
    let id = Uuid::new_v4();
    let prefix = p.chars().take(7).collect::<String>();
    UserRepo
        .insert(
            &mut **tx,
            NewUser {
                id,
                handle: format!("{prefix}-{}", &id.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn issue_redeem_revoke_full_cycle() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let founder = user(&mut tx, "fou").await;
    let invitee = user(&mut tx, "inv").await;

    let code = InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "vdn-inv-test1".into(),
                inviter_user_id: founder,
                max_redemptions: 1,
                expires_at: None,
                note: Some("first one".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(code, "vdn-inv-test1");

    let lookup = InviteRepo.find(&mut *tx, "vdn-inv-test1").await.unwrap().unwrap();
    assert_eq!(lookup.redemption_count, 0);

    InviteRepo
        .redeem(
            &mut tx,
            RedeemInvite {
                code: "vdn-inv-test1".into(),
                user_id: invitee,
                redeemed_from_ip: Some("1.2.3.4".into()),
            },
        )
        .await
        .unwrap();

    let after = InviteRepo.find(&mut *tx, "vdn-inv-test1").await.unwrap().unwrap();
    assert_eq!(after.redemption_count, 1);

    InviteRepo.revoke(&mut *tx, "vdn-inv-test1", founder, "no longer needed").await.unwrap();
    let revoked = InviteRepo.find(&mut *tx, "vdn-inv-test1").await.unwrap().unwrap();
    assert!(revoked.revoked_at.is_some());

    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn subtree_walks_descendants() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let founder = user(&mut tx, "fdr").await;
    let a = user(&mut tx, "a").await;
    let b = user(&mut tx, "b").await;
    let c = user(&mut tx, "c").await;

    InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "fa".into(),
                inviter_user_id: founder,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    InviteRepo
        .redeem(&mut tx, RedeemInvite { code: "fa".into(), user_id: a, redeemed_from_ip: None })
        .await
        .unwrap();
    InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "ab".into(),
                inviter_user_id: a,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    InviteRepo
        .redeem(&mut tx, RedeemInvite { code: "ab".into(), user_id: b, redeemed_from_ip: None })
        .await
        .unwrap();
    InviteRepo
        .issue(
            &mut *tx,
            NewInvite {
                code: "bc".into(),
                inviter_user_id: b,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    InviteRepo
        .redeem(&mut tx, RedeemInvite { code: "bc".into(), user_id: c, redeemed_from_ip: None })
        .await
        .unwrap();

    let descendants = InviteRepo.subtree_descendants(&mut *tx, a).await.unwrap();
    assert_eq!(descendants.len(), 2);
    assert!(descendants.contains(&b));
    assert!(descendants.contains(&c));

    tx.rollback().await.unwrap();
}

/// F12: race-regression test. N concurrent redemptions on the same
/// single-use invite code. Exactly ONE redemption must succeed; the
/// rest must observe `sqlx::Error::RowNotFound` (the atomic capacity
/// guard mapping `rows_affected() == 0` → `RowNotFound`).
///
/// This test runs against a shared pool (not a transaction) because
/// every connection needs to commit independently to race for real.
/// We tear down the invite + spawned users at the end so re-runs
/// stay deterministic.
#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn redeem_under_concurrency_admits_exactly_one() {
    use std::sync::Arc;
    let pool = common::shared_pool().await;

    // Stand-alone setup tx so the invite + inviter exist in the shared
    // pool (no rollback isolation).
    let mut setup = pool.begin().await.unwrap();
    let inviter = user(&mut setup, "race-inviter").await;
    let code = format!("race-{}", Uuid::new_v4().simple());
    InviteRepo
        .issue(
            &mut *setup,
            NewInvite {
                code: code.clone(),
                inviter_user_id: inviter,
                max_redemptions: 1,
                expires_at: None,
                note: None,
            },
        )
        .await
        .unwrap();
    setup.commit().await.unwrap();

    // Spawn N voters and N concurrent redemption attempts.
    const RACERS: usize = 20;
    let mut redeemer_ids = Vec::with_capacity(RACERS);
    {
        let mut tx = pool.begin().await.unwrap();
        for n in 0..RACERS {
            redeemer_ids.push(user(&mut tx, &format!("racer-{n}")).await);
        }
        tx.commit().await.unwrap();
    }

    let pool_arc = Arc::new(pool.clone());
    let code_arc = Arc::new(code.clone());
    let mut handles = Vec::with_capacity(RACERS);
    for user_id in &redeemer_ids {
        let pool = pool_arc.clone();
        let code = code_arc.clone();
        let uid = *user_id;
        handles.push(tokio::spawn(async move {
            let mut conn = pool.acquire().await.unwrap();
            InviteRepo
                .redeem(
                    &mut conn,
                    RedeemInvite { code: (*code).clone(), user_id: uid, redeemed_from_ip: None },
                )
                .await
        }));
    }

    let mut wins = 0usize;
    let mut row_not_found_losses = 0usize;
    let mut other_errors: Vec<String> = Vec::new();
    for handle in handles {
        match handle.await.unwrap() {
            Ok(()) => wins += 1,
            Err(sqlx::Error::RowNotFound) => row_not_found_losses += 1,
            Err(e) => other_errors.push(format!("{e:?}")),
        }
    }
    assert_eq!(wins, 1, "exactly one race winner expected, got {wins}");
    assert_eq!(
        row_not_found_losses,
        RACERS - 1,
        "every loser must observe RowNotFound; got {row_not_found_losses} (others: {other_errors:?})"
    );
    assert!(other_errors.is_empty(), "no unexpected errors expected; got {other_errors:?}");

    // Cleanup so re-runs stay deterministic.
    let mut cleanup = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM invite_redemptions WHERE invite_code = $1")
        .bind(&code)
        .execute(&mut *cleanup)
        .await
        .unwrap();
    sqlx::query("DELETE FROM invites WHERE code = $1")
        .bind(&code)
        .execute(&mut *cleanup)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE id = ANY($1::uuid[])")
        .bind(&redeemer_ids)
        .execute(&mut *cleanup)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(inviter)
        .execute(&mut *cleanup)
        .await
        .unwrap();
    cleanup.commit().await.unwrap();
}

fn expected_subtree(root: usize, parent_choices: &[Option<usize>]) -> Vec<usize> {
    let mut children = vec![Vec::new(); parent_choices.len() + 1];
    for (offset, choice) in parent_choices.iter().enumerate() {
        let child = offset + 1;
        if let Some(raw_parent) = choice {
            children[*raw_parent % child].push(child);
        }
    }

    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([root]);
    while let Some(parent) = queue.pop_front() {
        for child in &children[parent] {
            if seen.insert(*child) {
                queue.push_back(*child);
            }
        }
    }
    let mut descendants = seen.into_iter().collect::<Vec<_>>();
    descendants.sort_unstable();
    descendants
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn subtree_descendant_oracle_never_returns_roots_or_siblings(
        parent_choices in prop::collection::vec(prop::option::of(0usize..32), 0..32),
    ) {
        let descendants = expected_subtree(0, &parent_choices);
        prop_assert!(descendants.iter().all(|id| *id > 0));

        for child in 1..=parent_choices.len() {
            let mut cursor = child;
            let mut reaches_root = false;
            while cursor > 0 {
                let Some(raw_parent) = parent_choices[cursor - 1] else {
                    break;
                };
                cursor = raw_parent % cursor;
                if cursor == 0 {
                    reaches_root = true;
                    break;
                }
            }
            prop_assert_eq!(descendants.contains(&child), reaches_root);
        }
    }
}
