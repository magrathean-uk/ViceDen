//! Moderation pipeline. Wires classifier → posts/threads repos with the
//! hybrid sync-then-async semantic from spec §2.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::repository::content_classifications::{
    AuditVerdict, ContentClassificationRepo, NewClassification, TargetKind as AuditTargetKind,
};
use crate::repository::posts::{ModState, PostRepo};
use crate::repository::threads::ThreadRepo;
use crate::services::classifier::{
    Category, Classification, ClassifierError, ContentClassifier, PromptVariant, TargetKind,
    Verdict,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineOutcome {
    Allowed,
    Quarantined,
    Rejected,
    Pending,
}

#[derive(Clone)]
pub struct PipelineConfig {
    pub sync_deadline: Duration,
    pub default_model: String,
}

#[derive(Clone)]
pub struct ModerationPipeline {
    pool: PgPool,
    classifier: Arc<dyn ContentClassifier>,
    config: PipelineConfig,
}

impl ModerationPipeline {
    pub fn new(
        pool: PgPool,
        classifier: Arc<dyn ContentClassifier>,
        config: PipelineConfig,
    ) -> Self {
        Self { pool, classifier, config }
    }

    /// Classify a post and persist its new state. Used both at create-time
    /// (variant=Default) and during user-report priority reclassify (Strict).
    pub async fn classify_post(&self, post_id: Uuid, body: &str) -> PipelineOutcome {
        self.classify_target(TargetKind::Post, post_id, body, PromptVariant::Default).await
    }

    pub async fn reclassify_post_strict(&self, post_id: Uuid, body: &str) -> PipelineOutcome {
        self.classify_target(TargetKind::Post, post_id, body, PromptVariant::Strict).await
    }

    pub async fn classify_thread(&self, thread_id: Uuid, body: &str) -> PipelineOutcome {
        self.classify_target(TargetKind::Thread, thread_id, body, PromptVariant::Default).await
    }

    async fn classify_target(
        &self,
        kind: TargetKind,
        target_id: Uuid,
        body: &str,
        variant: PromptVariant,
    ) -> PipelineOutcome {
        let result = tokio::time::timeout(
            self.config.sync_deadline,
            self.classifier.classify(kind, body, variant),
        )
        .await;

        match result {
            Ok(Ok(classification)) => self.apply(kind, target_id, classification).await,
            Ok(Err(error)) => self.apply_error(kind, target_id, error).await,
            Err(_) => self.apply_timeout(kind, target_id).await,
        }
    }

    async fn apply(
        &self,
        kind: TargetKind,
        target_id: Uuid,
        classification: Classification,
    ) -> PipelineOutcome {
        let category = classification.categories.first().copied().unwrap_or(Category::Allowed);
        let (state, audit_verdict, outcome) = action_for(classification.verdict, category);
        tracing::info!(
            target_kind = ?kind,
            target_id = %target_id,
            verdict = ?classification.verdict,
            category = category.as_str(),
            mod_state = state.as_db(),
            "moderation_classified"
        );

        // Persist mod_state.
        match kind {
            TargetKind::Post => {
                PostRepo.update_mod_state(&self.pool, target_id, state, None).await.ok();
            }
            TargetKind::Thread => {
                ThreadRepo.update_mod_state(&self.pool, target_id, state, None).await.ok();
            }
        }

        // Persist audit.
        let _ = ContentClassificationRepo
            .insert(
                &self.pool,
                NewClassification {
                    target_kind: match kind {
                        TargetKind::Post => AuditTargetKind::Post,
                        TargetKind::Thread => AuditTargetKind::Thread,
                    },
                    target_id,
                    verdict: audit_verdict,
                    categories: classification
                        .categories
                        .iter()
                        .map(|c| c.as_str().to_string())
                        .collect(),
                    confidence: Some(classification.confidence),
                    model: classification.model,
                    request_id: classification.request_id,
                    latency_ms: Some(classification.latency_ms as i32),
                    raw_response: Some(classification.raw_response),
                },
            )
            .await;

        outcome
    }

    async fn apply_error(
        &self,
        kind: TargetKind,
        target_id: Uuid,
        error: ClassifierError,
    ) -> PipelineOutcome {
        // Treat malformed as Quarantine (spec §4 failure modes).
        // Treat transport / rate-limited / auth / timeout as: leave pending for the worker.
        let leave_pending = matches!(
            error,
            ClassifierError::Transport(_)
                | ClassifierError::RateLimited
                | ClassifierError::Auth(_)
                | ClassifierError::Timeout
        );
        if leave_pending {
            tracing::warn!(
                target_kind = ?kind,
                target_id = %target_id,
                error = %error,
                "moderation_classifier_error_pending"
            );
            // Audit row only; no state change.
            let _ = ContentClassificationRepo
                .insert(
                    &self.pool,
                    NewClassification {
                        target_kind: match kind {
                            TargetKind::Post => AuditTargetKind::Post,
                            TargetKind::Thread => AuditTargetKind::Thread,
                        },
                        target_id,
                        verdict: AuditVerdict::Error,
                        categories: vec![error_kind_label(&error).to_string()],
                        confidence: None,
                        model: self.config.default_model.clone(),
                        request_id: None,
                        latency_ms: None,
                        raw_response: Some(serde_json::json!({"error": error.to_string()})),
                    },
                )
                .await;
            return PipelineOutcome::Pending;
        }

        // Malformed/timeout: quarantine + audit.
        let state = ModState::Quarantined;
        tracing::warn!(
            target_kind = ?kind,
            target_id = %target_id,
            error = %error,
            mod_state = state.as_db(),
            "moderation_classifier_error_quarantined"
        );
        match kind {
            TargetKind::Post => {
                PostRepo
                    .update_mod_state(&self.pool, target_id, state, Some("classifier_malformed"))
                    .await
                    .ok();
            }
            TargetKind::Thread => {
                ThreadRepo
                    .update_mod_state(&self.pool, target_id, state, Some("classifier_malformed"))
                    .await
                    .ok();
            }
        }
        let _ = ContentClassificationRepo
            .insert(
                &self.pool,
                NewClassification {
                    target_kind: match kind {
                        TargetKind::Post => AuditTargetKind::Post,
                        TargetKind::Thread => AuditTargetKind::Thread,
                    },
                    target_id,
                    verdict: AuditVerdict::Quarantine,
                    categories: vec![error_kind_label(&error).to_string()],
                    confidence: None,
                    model: self.config.default_model.clone(),
                    request_id: None,
                    latency_ms: None,
                    raw_response: Some(serde_json::json!({"error": error.to_string()})),
                },
            )
            .await;
        PipelineOutcome::Quarantined
    }

    async fn apply_timeout(&self, kind: TargetKind, target_id: Uuid) -> PipelineOutcome {
        // Timeout = leave pending. Worker will retry.
        tracing::warn!(target_kind = ?kind, target_id = %target_id, "moderation_timeout_pending");
        let _ = ContentClassificationRepo
            .insert(
                &self.pool,
                NewClassification {
                    target_kind: match kind {
                        TargetKind::Post => AuditTargetKind::Post,
                        TargetKind::Thread => AuditTargetKind::Thread,
                    },
                    target_id,
                    verdict: AuditVerdict::Error,
                    categories: vec!["sync_timeout".into()],
                    confidence: None,
                    model: self.config.default_model.clone(),
                    request_id: None,
                    latency_ms: None,
                    raw_response: Some(serde_json::json!({"error":"sync_timeout"})),
                },
            )
            .await;
        PipelineOutcome::Pending
    }
}

fn action_for(verdict: Verdict, category: Category) -> (ModState, AuditVerdict, PipelineOutcome) {
    use Category::*;
    match (verdict, category) {
        (Verdict::Reject, CsamText | TerrorAdvocacy | Spam | CommercialSolicitation) => {
            (ModState::Removed, AuditVerdict::Reject, PipelineOutcome::Rejected)
        }
        (Verdict::Reject, _) => {
            (ModState::Removed, AuditVerdict::Reject, PipelineOutcome::Rejected)
        }
        (Verdict::Quarantine, _) => {
            (ModState::Quarantined, AuditVerdict::Quarantine, PipelineOutcome::Quarantined)
        }
        (Verdict::Allow, _) => (ModState::Visible, AuditVerdict::Allow, PipelineOutcome::Allowed),
    }
}

fn error_kind_label(error: &ClassifierError) -> &'static str {
    match error {
        ClassifierError::Transport(_) => "transport",
        ClassifierError::RateLimited => "rate_limited",
        ClassifierError::Malformed(_) => "malformed",
        ClassifierError::Timeout => "timeout",
        ClassifierError::Auth(_) => "auth",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn verdict_strategy() -> impl Strategy<Value = Verdict> {
        prop_oneof![Just(Verdict::Allow), Just(Verdict::Quarantine), Just(Verdict::Reject),]
    }

    fn category_strategy() -> impl Strategy<Value = Category> {
        prop_oneof![
            Just(Category::Allowed),
            Just(Category::CommercialSolicitation),
            Just(Category::CsamText),
            Just(Category::TerrorAdvocacy),
            Just(Category::Spam),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        #[test]
        fn moderation_action_is_deterministic_and_total(
            verdict in verdict_strategy(),
            category in category_strategy(),
        ) {
            let (state, audit, outcome) = action_for(verdict, category);

            match verdict {
                Verdict::Allow => {
                    prop_assert_eq!(state, ModState::Visible);
                    prop_assert_eq!(audit, AuditVerdict::Allow);
                    prop_assert_eq!(outcome, PipelineOutcome::Allowed);
                }
                Verdict::Quarantine => {
                    prop_assert_eq!(state, ModState::Quarantined);
                    prop_assert_eq!(audit, AuditVerdict::Quarantine);
                    prop_assert_eq!(outcome, PipelineOutcome::Quarantined);
                }
                Verdict::Reject => {
                    prop_assert_eq!(state, ModState::Removed);
                    prop_assert_eq!(audit, AuditVerdict::Reject);
                    prop_assert_eq!(outcome, PipelineOutcome::Rejected);
                }
            }
        }
    }
}
