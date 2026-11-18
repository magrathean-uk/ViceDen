//! Integration tests for the sessions repository.
//!
//! These tests require `VICEDEN_DATABASE__URL` to point at a Postgres instance
//! with the viceden migrations applied. They are marked `#[ignore]` so the
//! default `cargo test` run stays green when no local PG is available.
//!
//! To run them locally:
//!
//! ```bash
//! export VICEDEN_DATABASE__URL="postgres://viceden:viceden@127.0.0.1:5432/viceden_test"
//! cargo test --test repository_sessions -- --ignored --test-threads=1
//! ```

mod common;

use chrono::{Duration, Utc};
use uuid::Uuid;
use viceden::repository::sessions::{NewSession, SessionRepo};
use viceden::repository::users::{NewUser, UserRepo};

#[tokio::test]
#[ignore = "requires VICEDEN_DATABASE__URL pointing at a Postgres with M1+M2 migrations applied; run with `cargo test -- --ignored`"]
async fn create_lookup_destroy_roundtrip() {
    let pool = common::shared_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let user_id = Uuid::new_v4();
    UserRepo
        .insert(
            &mut *tx,
            NewUser {
                id: user_id,
                handle: format!("sess-{}", &user_id.simple().to_string()[..8]),
                password_hash: "$argon2id$v=19$m=19456,t=2,p=1$abc$def".into(),
                role: "member".into(),
                last_known_ip: None,
            },
        )
        .await
        .unwrap();

    let pepper = Some("a-long-random-pepper-for-tests-1234567890");
    let session = NewSession {
        token: "sometoken123".into(),
        user_id,
        expires_at: Utc::now() + Duration::hours(24),
        csrf_token: "csrfdeadbeef".into(),
    };
    SessionRepo.create(&mut *tx, session, pepper).await.unwrap();

    let found = SessionRepo.find_by_token(&mut *tx, "sometoken123", pepper).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().user_id, user_id);

    SessionRepo.destroy(&mut *tx, "sometoken123", pepper).await.unwrap();
    let gone = SessionRepo.find_by_token(&mut *tx, "sometoken123", pepper).await.unwrap();
    assert!(gone.is_none());

    tx.rollback().await.unwrap();
}
