//! Repository layer over Postgres via SQLx.
//!
//! Each module owns one table or one tightly-related cluster of tables.
//! Public methods take an executor (`&PgPool` or `&mut PgConnection`).
//! No business logic lives here — only SQL + serde mapping.
//!
//! The `Repos` struct aggregates the pool and lives inside `AppState`.

pub mod age_attestations;
pub mod boards;
pub mod content_classifications;
pub mod invites;
pub mod moderation_events;
pub mod notifications;
pub mod pats;
pub mod posts;
pub mod preferences;
pub mod reports;
pub mod sessions;
pub mod tags;
pub mod thread_states;
pub mod threads;
pub mod users;
pub mod votes;

use sqlx::PgPool;

#[derive(Clone)]
pub struct Repos {
    pub pool: PgPool,
}

impl Repos {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
