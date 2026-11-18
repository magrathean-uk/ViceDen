//! Domain models for the VICEDEN forum.
//!
//! This module defines the core data structures used throughout the
//! application: boards, users, threads and posts.  Each model
//! represents a record stored in PostgreSQL but is intentionally
//! decoupled from the underlying database schema.  Queries and
//! modifications are implemented as associated functions on these
//! structs, keeping the controllers (route handlers) slim.

use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
use uuid::Uuid;

/// A forum board is a top-level category under which threads are
/// organised.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Board {
    /// Unique slug used in URLs, e.g. `general`, `hobbies`.
    pub slug: String,
    /// Display name shown to the user.
    pub name: String,
    /// Number of days after which the root thread (and its posts) are
    /// automatically expired and deleted.
    pub ttl_days: u32,
    /// Description shown on the board index page.
    #[serde(default)]
    pub description: String,
}

impl Board {
    /// Load boards from a TOML file.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Vec<Board>, anyhow::Error> {
        let text = fs::read_to_string(&path)?;
        #[derive(Deserialize)]
        struct Wrapper {
            boards: Vec<Board>,
        }
        let wrapper: Wrapper = toml::from_str(&text)?;
        Ok(wrapper.boards)
    }

    /// Convenience helper for templates.
    pub fn has_description(&self) -> bool {
        !self.description.trim().is_empty()
    }
}

/// User role used for permission checks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Founder,
    Member,
    Mod,
}

/// Representation of a registered user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub handle: String,
    pub password_hash: String,
    #[serde(default = "default_handle_role")]
    pub role: UserRole,
    #[serde(default)]
    pub is_banned: bool,
    #[serde(default)]
    pub probation_until: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passkey_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn default_handle_role() -> UserRole {
    UserRole::Member
}

/// A thread (topic) within a board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: Uuid,
    pub board_slug: String,
    pub title: String,
    pub user_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl Thread {
    pub fn created_at_display(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M UTC").to_string()
    }

    pub fn expires_at_display(&self) -> String {
        self.expires_at.format("%Y-%m-%d %H:%M UTC").to_string()
    }
}

/// A single post (reply) within a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub user_id: Uuid,
    pub body: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub state: PostState,
}

/// Vote values available on posts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoteValue {
    Up = 1,
    Down = -1,
}

impl TryFrom<i8> for VoteValue {
    type Error = ();

    fn try_from(value: i8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Up),
            -1 => Ok(Self::Down),
            _ => Err(()),
        }
    }
}

impl Post {
    pub fn created_at_display(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M UTC").to_string()
    }
}

/// The moderation state assigned to a post before publication.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type)]
#[repr(i32)]
pub enum PostState {
    Allow = 0,
    Quarantine = 1,
    Reject = 2,
}
