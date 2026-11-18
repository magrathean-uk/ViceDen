//! High-level session orchestration. Repos handle SQL; this module owns
//! the application-level rules: token generation, default lifetime,
//! cookie-friendly return shape.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::tokens;
use crate::repository::sessions::{NewSession, SessionRepo};

pub struct IssuedSession {
    pub raw_token: String,
    pub csrf_token: String,
}

pub async fn issue_session(
    pool: &PgPool,
    user_id: Uuid,
    pepper: Option<&str>,
) -> Result<IssuedSession, sqlx::Error> {
    let raw_token = tokens::new_session_token();
    let csrf = hex::encode(rand_bytes(16));

    SessionRepo
        .create(
            pool,
            NewSession {
                token: raw_token.clone(),
                user_id,
                expires_at: Utc::now() + Duration::days(7),
                csrf_token: csrf.clone(),
            },
            pepper,
        )
        .await?;

    Ok(IssuedSession { raw_token, csrf_token: csrf })
}

pub async fn destroy_session(
    pool: &PgPool,
    raw_token: &str,
    pepper: Option<&str>,
) -> Result<(), sqlx::Error> {
    SessionRepo.destroy(pool, raw_token, pepper).await
}

fn rand_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("system RNG should be available");
    buf
}
