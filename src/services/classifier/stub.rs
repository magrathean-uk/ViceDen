//! Deterministic stub classifier for tests. Triggers via body markers:
//!
//! - `trigger:reject`     → Verdict::Reject + Category::Spam
//! - `trigger:quarantine` → Verdict::Quarantine + Category::Doxxing
//! - `trigger:timeout`    → ClassifierError::Timeout
//! - `trigger:rate`       → ClassifierError::RateLimited
//! - `trigger:malformed`  → ClassifierError::Malformed("stub")
//! - everything else      → Verdict::Allow + Category::Allowed

use async_trait::async_trait;
use serde_json::json;

use super::{
    Category, Classification, ClassifierError, ContentClassifier, PromptVariant, TargetKind,
    Verdict,
};

#[derive(Clone, Default)]
pub struct StubClassifier;

impl StubClassifier {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ContentClassifier for StubClassifier {
    async fn classify(
        &self,
        _kind: TargetKind,
        body: &str,
        _variant: PromptVariant,
    ) -> Result<Classification, ClassifierError> {
        if body.contains("trigger:timeout") {
            return Err(ClassifierError::Timeout);
        }
        if body.contains("trigger:rate") {
            return Err(ClassifierError::RateLimited);
        }
        if body.contains("trigger:malformed") {
            return Err(ClassifierError::Malformed("stub".into()));
        }

        let (verdict, category) = if body.contains("trigger:reject") {
            (Verdict::Reject, Category::Spam)
        } else if body.contains("trigger:quarantine") {
            (Verdict::Quarantine, Category::Doxxing)
        } else {
            (Verdict::Allow, Category::Allowed)
        };

        Ok(Classification {
            verdict,
            categories: vec![category],
            confidence: 0.99,
            model: "stub".into(),
            request_id: Some("stub-req-id".into()),
            latency_ms: 1,
            raw_response: json!({"stub": true, "body_len": body.len()}),
        })
    }
}
