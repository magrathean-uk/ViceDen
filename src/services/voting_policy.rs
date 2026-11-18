use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::{models::Board, repository::votes::VoteRepo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoteOutcome {
    pub post_id: Uuid,
    pub thread_id: Uuid,
    pub score: i64,
    pub my_vote: i16,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VotePolicyError {
    #[error("post not found")]
    PostNotFound,
    #[error("board not found")]
    BoardNotFound,
    #[error("user cannot vote right now")]
    UserCannotVote,
    #[error("downvotes are disabled for this board")]
    DownvotesDisabled,
    #[error("invalid vote value")]
    InvalidValue,
    #[error("database unavailable")]
    Database,
}

#[derive(sqlx::FromRow)]
struct VotePolicyRow {
    thread_id: Uuid,
    board_slug: String,
    downvotes_enabled: bool,
    banned: bool,
    deactivated: bool,
}

pub async fn cast_vote(
    pool: &PgPool,
    boards: &[Board],
    voter_id: Uuid,
    post_id: Uuid,
    value: i8,
) -> Result<VoteOutcome, VotePolicyError> {
    if !matches!(value, -1..=1) {
        return Err(VotePolicyError::InvalidValue);
    }

    let target = load_vote_target(pool, post_id, voter_id).await?;
    if !boards.iter().any(|board| board.slug == target.board_slug) {
        return Err(VotePolicyError::BoardNotFound);
    }
    if target.banned || target.deactivated {
        return Err(VotePolicyError::UserCannotVote);
    }
    if value == -1 && !target.downvotes_enabled {
        return Err(VotePolicyError::DownvotesDisabled);
    }

    let current = VoteRepo
        .user_vote_for_post(pool, voter_id, post_id)
        .await
        .map_err(|_| VotePolicyError::Database)?;
    let next = if value == 0 || current == i16::from(value) { 0 } else { i16::from(value) };

    if next == 0 {
        VoteRepo.delete(pool, voter_id, post_id).await.map_err(|_| VotePolicyError::Database)?;
    } else {
        VoteRepo
            .upsert(pool, voter_id, post_id, next)
            .await
            .map_err(|_| VotePolicyError::Database)?;
    }

    let score =
        VoteRepo.score_for_post(pool, post_id).await.map_err(|_| VotePolicyError::Database)?;
    Ok(VoteOutcome { post_id, thread_id: target.thread_id, score, my_vote: next })
}

async fn load_vote_target(
    pool: &PgPool,
    post_id: Uuid,
    voter_id: Uuid,
) -> Result<VotePolicyRow, VotePolicyError> {
    sqlx::query_as::<_, VotePolicyRow>(
        "SELECT p.thread_id,
                t.board_slug,
                COALESCE(b.downvotes_enabled, FALSE) AS downvotes_enabled,
                COALESCE(u.banned, TRUE) AS banned,
                (u.deactivated_at IS NOT NULL OR u.id IS NULL) AS deactivated
         FROM posts p
         JOIN threads t ON t.id = p.thread_id
         LEFT JOIN boards b ON b.slug = t.board_slug
         LEFT JOIN users u ON u.id = $2
         WHERE p.id = $1",
    )
    .bind(post_id)
    .bind(voter_id)
    .fetch_optional(pool)
    .await
    .map_err(|_| VotePolicyError::Database)?
    .ok_or(VotePolicyError::PostNotFound)
}
