//! Anthropic Messages-API classifier using a structured tool-use response.
//!
//! Endpoint: POST https://api.anthropic.com/v1/messages
//! Auth:     x-api-key header, anthropic-version: 2023-06-01

use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    prompts, Category, Classification, ClassifierError, ContentClassifier, PromptVariant,
    TargetKind, Verdict,
};

#[derive(Clone)]
pub struct AnthropicClassifier {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicClassifier {
    pub fn new(api_key: String, model: String) -> Result<Self, ClassifierError> {
        if api_key.trim().is_empty() {
            return Err(ClassifierError::Auth("anthropic api key empty".into()));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ClassifierError::Transport(e.to_string()))?;
        Ok(Self { client, api_key, model, base_url: "https://api.anthropic.com/v1".into() })
    }

    /// Override the base URL. Production only accepts Anthropic HTTPS; debug
    /// builds also accept loopback HTTP for integration tests.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Result<Self, ClassifierError> {
        let base_url = base_url.into();
        validate_base_url(&base_url)?;
        self.base_url = base_url;
        Ok(self)
    }

    fn tool_schema() -> Value {
        json!({
            "name": "classify",
            "description": "Classify content for moderation.",
            "input_schema": {
                "type": "object",
                "required": ["verdict", "category", "confidence"],
                "properties": {
                    "verdict": {
                        "type": "string",
                        "enum": ["allow", "quarantine", "reject"]
                    },
                    "category": {
                        "type": "string",
                        "enum": ["csam_text","ncii_text","doxxing","terror_advocacy",
                                 "imminent_violence","commercial_solicitation",
                                 "spam","allowed"]
                    },
                    "confidence": { "type": "number" },
                    "reason": { "type": "string" }
                }
            }
        })
    }
}

fn validate_base_url(base_url: &str) -> Result<(), ClassifierError> {
    let url = reqwest::Url::parse(base_url)
        .map_err(|_| ClassifierError::Transport("invalid classifier base url".into()))?;
    let host = url
        .host_str()
        .ok_or_else(|| ClassifierError::Transport("classifier base url missing host".into()))?;
    if url.scheme() == "https" && host == "api.anthropic.com" {
        return Ok(());
    }
    if cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(host, "127.0.0.1" | "localhost" | "::1")
    {
        return Ok(());
    }
    Err(ClassifierError::Transport("classifier base url is not allowed".into()))
}

#[async_trait]
impl ContentClassifier for AnthropicClassifier {
    async fn classify(
        &self,
        kind: TargetKind,
        body: &str,
        variant: PromptVariant,
    ) -> Result<Classification, ClassifierError> {
        let started = std::time::Instant::now();
        let system = prompts::system_prompt_for(variant);
        let user_message = format!(
            "Target: {}\n\nBody:\n{}",
            match kind {
                TargetKind::Post => "post",
                TargetKind::Thread => "thread",
            },
            body
        );

        let payload = json!({
            "model": self.model,
            "max_tokens": 256,
            "system": system,
            "tools": [Self::tool_schema()],
            "tool_choice": {"type": "tool", "name": "classify"},
            "messages": [{"role": "user", "content": user_message}],
        });

        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| ClassifierError::Transport(e.to_string()))?;

        let status = response.status();
        if status == 429 {
            return Err(ClassifierError::RateLimited);
        }
        if status == 401 || status == 403 {
            return Err(ClassifierError::Auth(format!("status {status}")));
        }
        if !status.is_success() {
            return Err(ClassifierError::Transport(format!("status {status}")));
        }

        let request_id =
            response.headers().get("request-id").and_then(|v| v.to_str().ok()).map(str::to_string);
        let raw: Value =
            response.json().await.map_err(|e| ClassifierError::Malformed(e.to_string()))?;

        let (verdict, category, confidence) = parse_tool_use(&raw)?;
        let elapsed = started.elapsed().as_millis() as u32;

        Ok(Classification {
            verdict,
            categories: vec![category],
            confidence,
            model: self.model.clone(),
            request_id,
            latency_ms: elapsed,
            raw_response: raw,
        })
    }
}

fn parse_tool_use(raw: &Value) -> Result<(Verdict, Category, f32), ClassifierError> {
    #[derive(Deserialize)]
    struct ToolInput {
        verdict: String,
        category: String,
        confidence: f32,
        #[serde(default)]
        reason: Option<String>,
    }

    let block = raw["content"]
        .as_array()
        .and_then(|arr| arr.iter().find(|b| b["type"] == "tool_use"))
        .ok_or_else(|| ClassifierError::Malformed("no tool_use block".into()))?;
    let input: ToolInput = serde_json::from_value(block["input"].clone())
        .map_err(|e| ClassifierError::Malformed(format!("tool input: {e}")))?;

    let verdict = match input.verdict.as_str() {
        "allow" => Verdict::Allow,
        "quarantine" => Verdict::Quarantine,
        "reject" => Verdict::Reject,
        other => return Err(ClassifierError::Malformed(format!("unknown verdict {other}"))),
    };
    let category = Category::from_str(&input.category).ok_or_else(|| {
        ClassifierError::Malformed(format!("unknown category {}", input.category))
    })?;

    let _ = input.reason; // reason is informational; surface it in audit raw_response if needed.
    Ok((verdict, category, input.confidence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_guard_rejects_private_network_targets() {
        let error = validate_base_url("http://169.254.169.254/latest").unwrap_err();
        assert!(matches!(error, ClassifierError::Transport(_)));
    }
}
