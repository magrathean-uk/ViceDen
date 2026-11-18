//! Posting and registration limits.
//!
//! Planned responsibilities:
//! - thread cap checks (5/day)
//! - reply cap checks (50/day)
//! - probation window checks (24h)
//! - invite issuance caps
//! - failed login throttling

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::repository::{posts::PostRepo, threads::ThreadRepo};

pub const REGISTRATION_PROBATION_HOURS: i64 = 24;
pub const DAILY_THREAD_LIMIT: i64 = 5;
pub const DAILY_REPLY_LIMIT: i64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostingLimitViolation {
    DailyThreadLimit,
    DailyReplyLimit,
    BoardThreadLimit,
}

impl PostingLimitViolation {
    pub fn message(self) -> &'static str {
        match self {
            Self::DailyThreadLimit => "thread limit reached",
            Self::DailyReplyLimit => "reply limit reached",
            Self::BoardThreadLimit => "one thread limit per day for this board",
        }
    }

    pub fn status_code(self) -> StatusCode {
        match self {
            Self::DailyThreadLimit | Self::DailyReplyLimit => StatusCode::TOO_MANY_REQUESTS,
            Self::BoardThreadLimit => StatusCode::CONFLICT,
        }
    }
}

pub fn registration_probation_until(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::hours(REGISTRATION_PROBATION_HOURS)
}

pub fn active_probation_until(
    probation_until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    probation_until.filter(|until| *until > now)
}

pub async fn posting_limit_violation(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    role: &str,
    board_slug: &str,
    is_thread: bool,
) -> Result<Option<PostingLimitViolation>, sqlx::Error> {
    if role == "founder" {
        return Ok(None);
    }

    if is_thread {
        if ThreadRepo.count_for_user_today(pool, user_id).await? >= DAILY_THREAD_LIMIT {
            return Ok(Some(PostingLimitViolation::DailyThreadLimit));
        }
        if board_slug == "anonymous"
            && ThreadRepo.count_for_user_in_board_today(pool, user_id, board_slug).await? > 0
        {
            return Ok(Some(PostingLimitViolation::BoardThreadLimit));
        }
    } else if PostRepo.count_for_user_today(pool, user_id).await? >= DAILY_REPLY_LIMIT {
        return Ok(Some(PostingLimitViolation::DailyReplyLimit));
    }

    Ok(None)
}
