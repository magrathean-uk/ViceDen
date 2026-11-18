use viceden::services::classifier::{
    stub::StubClassifier, ContentClassifier, PromptVariant, TargetKind, Verdict,
};

#[tokio::test]
async fn stub_returns_allow_by_default() {
    let stub = StubClassifier::new();
    let result = stub
        .classify(TargetKind::Post, "ordinary post body", PromptVariant::Default)
        .await
        .unwrap();
    assert_eq!(result.verdict, Verdict::Allow);
}

#[tokio::test]
async fn stub_recognises_trigger_reject() {
    let stub = StubClassifier::new();
    let result = stub
        .classify(TargetKind::Post, "trigger:reject please", PromptVariant::Default)
        .await
        .unwrap();
    assert_eq!(result.verdict, Verdict::Reject);
}

#[tokio::test]
async fn stub_recognises_trigger_quarantine() {
    let stub = StubClassifier::new();
    let result = stub
        .classify(TargetKind::Post, "trigger:quarantine review", PromptVariant::Default)
        .await
        .unwrap();
    assert_eq!(result.verdict, Verdict::Quarantine);
}

#[tokio::test]
async fn stub_recognises_trigger_timeout() {
    let stub = StubClassifier::new();
    let err = stub
        .classify(TargetKind::Post, "trigger:timeout x", PromptVariant::Default)
        .await
        .unwrap_err();
    assert!(matches!(err, viceden::services::classifier::ClassifierError::Timeout));
}
