//! AI content classifier — trait + types. Implementations live in
//! `anthropic.rs` (production) and `stub.rs` (tests).

pub mod anthropic;
pub mod prompts;
pub mod stub;

use async_trait::async_trait;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    Post,
    Thread,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Quarantine,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    CsamText,
    NciiText,
    Doxxing,
    TerrorAdvocacy,
    ImminentViolence,
    CommercialSolicitation,
    Spam,
    Allowed,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CsamText => "csam_text",
            Self::NciiText => "ncii_text",
            Self::Doxxing => "doxxing",
            Self::TerrorAdvocacy => "terror_advocacy",
            Self::ImminentViolence => "imminent_violence",
            Self::CommercialSolicitation => "commercial_solicitation",
            Self::Spam => "spam",
            Self::Allowed => "allowed",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "csam_text" => Self::CsamText,
            "ncii_text" => Self::NciiText,
            "doxxing" => Self::Doxxing,
            "terror_advocacy" => Self::TerrorAdvocacy,
            "imminent_violence" => Self::ImminentViolence,
            "commercial_solicitation" => Self::CommercialSolicitation,
            "spam" => Self::Spam,
            "allowed" => Self::Allowed,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Classification {
    pub verdict: Verdict,
    pub categories: Vec<Category>,
    pub confidence: f32,
    pub model: String,
    pub request_id: Option<String>,
    pub latency_ms: u32,
    pub raw_response: Value,
}

#[derive(Clone, Copy, Debug)]
pub enum PromptVariant {
    Default,
    /// Used for user-report-driven reclassifications and the daily audit job.
    Strict,
}

#[derive(Debug, thiserror::Error)]
pub enum ClassifierError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("rate limited")]
    RateLimited,
    #[error("malformed response: {0}")]
    Malformed(String),
    #[error("timeout")]
    Timeout,
    #[error("auth: {0}")]
    Auth(String),
}

#[async_trait]
pub trait ContentClassifier: Send + Sync {
    async fn classify(
        &self,
        kind: TargetKind,
        body: &str,
        variant: PromptVariant,
    ) -> Result<Classification, ClassifierError>;
}

pub type Handle = std::sync::Arc<dyn ContentClassifier>;
