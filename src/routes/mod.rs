//! Route table definitions.
//!
//! This module exports two functions, `api_router` and `web_router`,
//! which construct nested `axum::Router`s for JSON API endpoints and
//! server‑rendered HTML pages respectively.  Keeping the route
//! definitions in a separate module allows the main entry point to
//! compose them cleanly.

pub mod admin_invites;
pub mod api;
pub mod hidden;
pub mod middleware;
pub mod pagination;
pub mod saved;
pub mod web;
