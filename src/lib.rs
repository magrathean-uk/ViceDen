//! Library entry point for VICEDEN Forum.
//!
//! Exposing a library crate alongside the binary makes testing easier and keeps
//! the architecture clearer for future AI-driven development.

pub mod app;
pub mod auth;
pub mod cache;
pub mod config;
pub mod db;
pub mod forum;
pub mod models;
pub mod observability;
pub mod repository;
pub mod routes;
pub mod services;

use std::sync::Arc;

use crate::cache::{LoginThrottle, MentionSuggest};
use crate::repository::Repos;
use crate::services::moderation_pipeline::ModerationPipeline;

#[derive(Clone)]
pub struct RequiredPgPool(sqlx::PgPool);

impl RequiredPgPool {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self(pool)
    }

    pub fn as_ref(&self) -> Option<&sqlx::PgPool> {
        Some(&self.0)
    }

    pub fn is_some(&self) -> bool {
        true
    }
}

impl std::ops::Deref for RequiredPgPool {
    type Target = sqlx::PgPool;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Shared application state injected into request handlers via Axum's
/// `State` extractor.
#[derive(Clone)]
pub struct AppState {
    pub db: RequiredPgPool,
    pub boards: Vec<models::Board>,
    pub config: config::Settings,
    pub repos: Repos,
    pub login_throttle: LoginThrottle,
    pub mention_cache: MentionSuggest,
    pub moderation: Option<Arc<ModerationPipeline>>,
}
