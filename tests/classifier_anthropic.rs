use std::net::SocketAddr;
use std::sync::Arc;

use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use viceden::services::classifier::{
    anthropic::AnthropicClassifier, ContentClassifier, PromptVariant, TargetKind, Verdict,
};

async fn spawn_mock(response: Value) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let response = Arc::new(Mutex::new(response));
    let app = Router::new().route(
        "/messages",
        post(move |_: Json<Value>| {
            let response = Arc::clone(&response);
            async move {
                let resp = response.lock().await.clone();
                Json(resp)
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, handle)
}

#[tokio::test]
async fn parses_allow_verdict_from_tool_use() {
    let response = json!({
        "id": "msg_test", "type": "message", "role": "assistant",
        "model": "claude-haiku-4-5-20251001",
        "content": [{
            "type": "tool_use", "id": "toolu_1", "name": "classify",
            "input": { "verdict": "allow", "category": "allowed", "confidence": 0.97 }
        }],
        "stop_reason": "tool_use"
    });
    let (addr, _h) = spawn_mock(response).await;
    let classifier =
        AnthropicClassifier::new("test-key".into(), "claude-haiku-4-5-20251001".into())
            .unwrap()
            .with_base_url(format!("http://{addr}"))
            .unwrap();

    let result = classifier
        .classify(TargetKind::Post, "ordinary body", PromptVariant::Default)
        .await
        .unwrap();
    assert_eq!(result.verdict, Verdict::Allow);
    assert!((result.confidence - 0.97).abs() < 1e-4);
}

#[tokio::test]
async fn rejects_reject_verdict_with_csam_category() {
    let response = json!({
        "id": "msg_test", "type": "message", "role": "assistant",
        "model": "claude-haiku-4-5-20251001",
        "content": [{
            "type": "tool_use", "id": "toolu_2", "name": "classify",
            "input": { "verdict": "reject", "category": "csam_text", "confidence": 0.99 }
        }],
        "stop_reason": "tool_use"
    });
    let (addr, _h) = spawn_mock(response).await;
    let classifier =
        AnthropicClassifier::new("test-key".into(), "claude-haiku-4-5-20251001".into())
            .unwrap()
            .with_base_url(format!("http://{addr}"))
            .unwrap();
    let result =
        classifier.classify(TargetKind::Post, "...", PromptVariant::Default).await.unwrap();
    assert_eq!(result.verdict, Verdict::Reject);
}

#[test]
fn rejects_non_anthropic_and_private_classifier_base_urls() {
    for url in [
        "http://169.254.169.254/latest",
        "http://10.0.0.1/messages",
        "http://192.168.0.10/messages",
        "http://172.16.0.1/messages",
        "http://[fd00::1]/messages",
        "https://evil.example/messages",
        "http://api.anthropic.com/v1",
    ] {
        let error =
            match AnthropicClassifier::new("test-key".into(), "claude-haiku-4-5-20251001".into())
                .unwrap()
                .with_base_url(url)
            {
                Ok(_) => panic!("classifier base URL should be rejected: {url}"),
                Err(error) => error,
            };
        assert!(format!("{error}").contains("base url"));
    }
}
