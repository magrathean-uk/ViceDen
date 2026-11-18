//! Retention and cleanup worker.
//!
//! The first implementation should be a single bounded periodic task that:
//! 1. finds expired root threads
//! 2. deletes descendant posts in one transaction
//! 3. deletes the root threads
//! 4. logs counts and duration
//!
//! Do not spawn one task per expired thread.
