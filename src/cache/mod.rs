//! In-memory caches that survive M2's Mutex teardown.
//!
//! Only two types live here, both backed by `Arc<RwLock<…>>`:
//! - `LoginThrottle`: per-(ip+handle) sliding window of failed login attempts.
//! - `MentionSuggest`: per-thread TTL'd participant list for @-mention dropdown.

pub mod login_throttle;
pub mod mention_suggest;

pub use login_throttle::LoginThrottle;
pub use mention_suggest::MentionSuggest;
