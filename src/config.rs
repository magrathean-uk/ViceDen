//! Application configuration loader.
//!
//! This module defines the `Settings` struct which aggregates all
//! configuration sections for the VICEDEN forum application.  It uses the
//! `config` crate to merge values from multiple sources: a default TOML
//! file (`config/default.toml`), optional environment variables prefixed
//! with `VICEDEN_`, and future command line flags.  The configuration is
//! intentionally explicit so later work can extend it without
//! reverse engineering implicit defaults.

use ipnet::IpNet;
use serde::Deserialize;

/// Root application settings.
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub server: ServerSettings,
    pub database: DatabaseSettings,
    #[serde(default)]
    pub auth: AuthSettings,
    #[serde(default)]
    pub classifier: ClassifierSettings,
    #[serde(default)]
    pub ops: OpsSettings,
    pub boards_file: String,
}

impl Settings {
    /// Load configuration from `config/default.toml` plus environment.
    pub fn load() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .add_source(config::File::with_name("config/default"))
            .add_source(
                config::Environment::with_prefix("VICEDEN").prefix_separator("_").separator("__"),
            )
            .build()?
            .try_deserialize()
    }
}

/// HTTP server configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerSettings {
    /// Bind address like `127.0.0.1:3000`.
    pub bind: String,
    /// Maximum accepted request body size in bytes.  This limits memory
    /// pressure from large request bodies and is one of the simplest leak /
    /// abuse mitigations for a text-only service.
    #[serde(default = "default_max_request_body_bytes")]
    pub max_request_body_bytes: usize,
    /// Per-request timeout in seconds.
    #[serde(default = "default_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
    #[serde(default)]
    pub secure_cookies: bool,
    #[serde(default)]
    pub proxy: ProxySettings,
    #[serde(default)]
    pub rate_limit: RateLimitSettings,
}

/// Trusted-proxy / real-client-IP extraction settings.
///
/// `trusted_cidrs` lists the CIDR ranges from which we will trust client-IP
/// headers (e.g. Cloudflare edge ranges).  `client_ip_headers` lists, in
/// priority order, which headers to consult when extracting the original
/// client IP from a trusted proxy.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxySettings {
    #[serde(default = "default_trusted_cidrs", deserialize_with = "deserialize_cidrs")]
    pub trusted_cidrs: Vec<IpNet>,
    #[serde(default = "default_client_ip_headers", deserialize_with = "deserialize_string_list")]
    pub client_ip_headers: Vec<String>,
}

impl Default for ProxySettings {
    fn default() -> Self {
        Self {
            trusted_cidrs: default_trusted_cidrs(),
            client_ip_headers: default_client_ip_headers(),
        }
    }
}

fn default_trusted_cidrs() -> Vec<IpNet> {
    vec!["127.0.0.1/32".parse().unwrap(), "::1/128".parse().unwrap()]
}

fn default_client_ip_headers() -> Vec<String> {
    vec!["Cf-Connecting-Ip".to_string(), "X-Forwarded-For".to_string()]
}

/// Custom deserializer that accepts either:
/// - a TOML array of CIDR strings (`["127.0.0.1/32", "::1/128"]`), or
/// - a comma-separated single string (`"127.0.0.1/32,::1/128"`) — used when
///   values arrive via the `config::Environment` source, which exposes env
///   vars as plain strings without list-splitting (`config 0.13` only
///   list-splits when `try_parsing(true)` is set, which we avoid because it
///   eagerly coerces every env var to bool/i64/f64 first).
///
/// Returns `Vec<IpNet>` on success; we spell out the turbofish on the inner
/// `<StringOrVec as Deserialize>::deserialize` call so rust-analyzer's
/// expansion of the surrounding `#[derive(Deserialize)]` doesn't trip E0282.
fn deserialize_cidrs<'de, D>(deserializer: D) -> Result<Vec<IpNet>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let raw: StringOrVec = <StringOrVec as Deserialize<'de>>::deserialize(deserializer)?;
    raw.into_iter().map(|item| item.trim().parse::<IpNet>().map_err(D::Error::custom)).collect()
}

/// Same shape as `deserialize_cidrs` but for plain `Vec<String>` fields,
/// so env-supplied `client_ip_headers="A,B"` works alongside TOML arrays.
///
/// Returns `Vec<String>`; turbofish on the inner `StringOrVec::deserialize`
/// is spelled out for the same rust-analyzer / E0282 reason as
/// [`deserialize_cidrs`].
fn deserialize_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: StringOrVec = <StringOrVec as Deserialize<'de>>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|item| item.trim().to_string()).collect())
}

/// Helper that deserializes either `Vec<String>` or a comma-separated `String`
/// into `Vec<String>`.  Used by `deserialize_cidrs` and `deserialize_string_list`.
struct StringOrVec(Vec<String>);

impl StringOrVec {
    fn into_iter(self) -> std::vec::IntoIter<String> {
        self.0.into_iter()
    }
}

impl<'de> Deserialize<'de> for StringOrVec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StringOrVec;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string or a sequence of strings")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                let parts: Vec<String> =
                    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                Ok(StringOrVec(parts))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out: Vec<String> = Vec::new();
                while let Some(item) = seq.next_element::<String>()? {
                    out.push(item);
                }
                Ok(StringOrVec(out))
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// Per-route rate-limit knobs.  In M1+M2 we only expose a master `disabled`
/// toggle (used by integration tests); concrete per-route limits are wired
/// up directly in the server module.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RateLimitSettings {
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSettings {
    pub url: String,
    #[serde(default)]
    pub require_connection: bool,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthSettings {
    #[serde(default)]
    pub founder_password: Option<String>,
    #[serde(default)]
    pub token_pepper: Option<String>,
}

/// AI moderation classifier settings (M1+M2 spec §4).
#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierSettings {
    #[serde(default = "default_classifier_provider")]
    pub provider: String,
    #[serde(default = "default_classifier_model")]
    pub model: String,
    #[serde(default = "default_classifier_api_key_env")]
    pub api_key_env: String,
    #[serde(default = "default_sync_deadline_ms")]
    pub sync_deadline_ms: u64,
    #[serde(default = "default_worker_interval_s")]
    pub worker_interval_s: u64,
    #[serde(default = "default_worker_max_inflight")]
    pub worker_max_inflight: usize,
    #[serde(default = "default_worker_max_retries")]
    pub worker_max_retries: u32,
    #[serde(default = "default_audit_sample_size")]
    pub audit_sample_size: usize,
    #[serde(default = "default_audit_lookback_days")]
    pub audit_lookback_days: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpsSettings {
    #[serde(default)]
    pub backup_dir: Option<String>,
    #[serde(default = "default_backup_max_age_hours")]
    pub backup_max_age_hours: u64,
}

impl Default for OpsSettings {
    fn default() -> Self {
        Self { backup_dir: None, backup_max_age_hours: default_backup_max_age_hours() }
    }
}

impl Default for ClassifierSettings {
    fn default() -> Self {
        Self {
            provider: default_classifier_provider(),
            model: default_classifier_model(),
            api_key_env: default_classifier_api_key_env(),
            sync_deadline_ms: default_sync_deadline_ms(),
            worker_interval_s: default_worker_interval_s(),
            worker_max_inflight: default_worker_max_inflight(),
            worker_max_retries: default_worker_max_retries(),
            audit_sample_size: default_audit_sample_size(),
            audit_lookback_days: default_audit_lookback_days(),
        }
    }
}

fn default_classifier_provider() -> String {
    "anthropic".to_string()
}

fn default_classifier_model() -> String {
    "claude-haiku-4-5-20251001".to_string()
}

fn default_classifier_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

fn default_sync_deadline_ms() -> u64 {
    1500
}

fn default_worker_interval_s() -> u64 {
    30
}

fn default_worker_max_inflight() -> usize {
    4
}

fn default_worker_max_retries() -> u32 {
    10
}

fn default_backup_max_age_hours() -> u64 {
    36
}

fn default_audit_sample_size() -> usize {
    50
}

fn default_audit_lookback_days() -> u32 {
    7
}

fn default_max_connections() -> u32 {
    10
}

fn default_max_request_body_bytes() -> usize {
    128 * 1024
}

fn default_request_timeout_seconds() -> u64 {
    15
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn double_underscore_env_overrides_nested_config() {
        std::env::set_var("VICEDEN_SERVER__BIND", "127.0.0.1:40123");
        std::env::set_var("VICEDEN_DATABASE__REQUIRE_CONNECTION", "true");
        std::env::set_var("VICEDEN_AUTH__FOUNDER_PASSWORD", "safe-founder-password-123");
        std::env::set_var(
            "VICEDEN_AUTH__TOKEN_PEPPER",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        );

        let settings = Settings::load().expect("settings should load");

        assert_eq!(settings.server.bind, "127.0.0.1:40123");
        assert!(settings.database.require_connection);
        assert_eq!(settings.auth.founder_password.as_deref(), Some("safe-founder-password-123"));
        assert_eq!(
            settings.auth.token_pepper.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );

        std::env::remove_var("VICEDEN_SERVER__BIND");
        std::env::remove_var("VICEDEN_DATABASE__REQUIRE_CONNECTION");
        std::env::remove_var("VICEDEN_AUTH__FOUNDER_PASSWORD");
        std::env::remove_var("VICEDEN_AUTH__TOKEN_PEPPER");
    }

    #[test]
    fn loads_classifier_and_proxy_and_rate_limit_sections() {
        std::env::set_var("VICEDEN_CLASSIFIER__API_KEY_ENV", "ANTHROPIC_API_KEY");
        std::env::set_var("VICEDEN_CLASSIFIER__MODEL", "claude-haiku-4-5-20251001");
        std::env::set_var("VICEDEN_SERVER__PROXY__TRUSTED_CIDRS", "127.0.0.1/32,::1/128");
        std::env::set_var("VICEDEN_SERVER__PROXY__CLIENT_IP_HEADERS", "Foo,Bar");

        let settings = Settings::load().expect("settings should load");

        assert_eq!(settings.classifier.model, "claude-haiku-4-5-20251001");
        assert_eq!(settings.classifier.sync_deadline_ms, 1500);

        // Verify that the env-CSV path of the StringOrVec custom deserializer
        // actually splits multi-element lists rather than silently falling
        // back to the TOML defaults.  Both ipv4 and ipv6 entries must round
        // trip from the comma-separated env value.
        assert_eq!(
            settings.server.proxy.trusted_cidrs.len(),
            2,
            "expected env-CSV to split into exactly two CIDR entries, got {:?}",
            settings.server.proxy.trusted_cidrs
        );
        assert!(settings
            .server
            .proxy
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains(&"127.0.0.1".parse::<std::net::IpAddr>().unwrap())));
        assert!(settings
            .server
            .proxy
            .trusted_cidrs
            .iter()
            .any(|cidr| cidr.contains(&"::1".parse::<std::net::IpAddr>().unwrap())));
        assert_eq!(settings.server.proxy.client_ip_headers, vec!["Foo", "Bar"]);

        std::env::remove_var("VICEDEN_CLASSIFIER__API_KEY_ENV");
        std::env::remove_var("VICEDEN_CLASSIFIER__MODEL");
        std::env::remove_var("VICEDEN_SERVER__PROXY__TRUSTED_CIDRS");
        std::env::remove_var("VICEDEN_SERVER__PROXY__CLIENT_IP_HEADERS");
    }
}
