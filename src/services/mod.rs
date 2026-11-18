//! Service layer placeholders.
//!
//! Route handlers should call services instead of accumulating business logic.
//! These modules are intentionally stubbed so follow-up work can implement
//! them without first deciding where logic belongs.

pub mod classifier;
pub mod limits;
pub mod moderation_pipeline;
pub mod moderation_workers;
pub mod ops_status;
pub mod policy;
pub mod redirects;
pub mod retention;
pub mod safe_html;
pub mod voting_policy;
