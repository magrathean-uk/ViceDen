//! Application router assembly.
//!
//! Keeping router construction outside `main.rs` makes integration testing much
//! easier and prevents startup wiring from turning into a monolith.

use std::time::Duration;

use axum::{
    error_handling::HandleErrorLayer,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get_service,
    Router,
};
use tower::{timeout::TimeoutLayer, BoxError};
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{routes, AppState};

pub async fn apply_boot_policy(
    pool: &sqlx::PgPool,
    settings: &crate::config::Settings,
) -> Result<(), anyhow::Error> {
    validate_deployed_boot_inputs(settings)?;

    if let Some(password) =
        settings.auth.founder_password.as_deref().map(str::trim).filter(|value| !value.is_empty())
    {
        crate::db::set_founder_password(pool, password)
            .await
            .map_err(|error| boot_policy_error("auth.founder_password", error.to_string()))?;
        return Ok(());
    }

    if settings.database.require_connection
        && crate::db::founder_uses_default_password(pool).await?
    {
        return Err(boot_policy_error(
            "auth.founder_password",
            "deployed mode requires VICEDEN_AUTH__FOUNDER_PASSWORD so founder auth is not root/root",
        ));
    }

    Ok(())
}

fn validate_deployed_boot_inputs(settings: &crate::config::Settings) -> Result<(), anyhow::Error> {
    if settings.database.require_connection {
        crate::auth::tokens::validate_deployed_pepper(settings.auth.token_pepper.as_deref())
            .map_err(|error| boot_policy_error("auth.token_pepper", error.to_string()))?;
        validate_classifier_api_key(settings)?;
    }

    if let Some(password) =
        settings.auth.founder_password.as_deref().map(str::trim).filter(|value| !value.is_empty())
    {
        if password.len() < 8 {
            return Err(boot_policy_error(
                "auth.founder_password",
                "founder password must be at least 8 characters",
            ));
        }
    }

    Ok(())
}

fn validate_classifier_api_key(settings: &crate::config::Settings) -> Result<(), anyhow::Error> {
    let key_env = settings.classifier.api_key_env.trim();
    if key_env.is_empty() {
        return Err(boot_policy_error(
            "classifier.api_key_env",
            "classifier.api_key_env must not be empty in deployed mode",
        ));
    }
    let value = std::env::var(key_env).unwrap_or_default();
    if value.trim().is_empty() {
        return Err(boot_policy_error(
            "classifier.api_key",
            format!("environment variable {key_env} must be set in deployed mode"),
        ));
    }
    Ok(())
}

fn boot_policy_error(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::anyhow!("boot_policy_violation code={code}: {}", message.into())
}

pub fn build_router(state: AppState) -> Router {
    use routes::{api, web};

    let secure_transport = state.config.server.secure_cookies;
    let web_state = state.clone();
    Router::new()
        .nest_service("/assets", get_service(ServeDir::new("static/assets")))
        .nest("/api", api::api_router())
        .merge(web::web_router(web_state))
        .fallback(web::not_found_page)
        .layer(DefaultBodyLimit::max(state.config.server.max_request_body_bytes))
        .layer((
            HandleErrorLayer::new(|_: BoxError| async {
                (StatusCode::REQUEST_TIMEOUT, "request timeout").into_response()
            }),
            TimeoutLayer::new(Duration::from_secs(state.config.server.request_timeout_seconds)),
        ))
        .layer(middleware::from_fn(move |request, next| {
            set_security_headers(request, next, secure_transport)
        }))
        .layer(TraceLayer::new_for_http())
        .layer(routes::middleware::client_ip_layer(routes::middleware::client_ip::ProxyConfig {
            trusted_cidrs: state.config.server.proxy.trusted_cidrs.clone(),
            client_ip_headers: state.config.server.proxy.client_ip_headers.clone(),
        }))
        .with_state(state)
}

async fn set_security_headers(
    request: Request,
    next: middleware::Next,
    secure_transport: bool,
) -> Response {
    let is_asset_request = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(HeaderName::from_static("x-frame-options"), HeaderValue::from_static("DENY"));
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-viceden-version"),
        HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    // F1: tightened CSP. `'unsafe-inline'` is gone from style-src — the
    // app ships zero inline `<style>` blocks and zero `style="..."`
    // attributes (verified by the design_contract test). All styling is
    // /assets/*.css. Adding `img-src 'self' data:` for the small inline
    // SVG / data-URL icons in askama templates and the placeholder
    // avatar fallback.
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'self'; frame-ancestors 'none'; object-src 'none'; form-action 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'",
        ),
    );
    // Permissions-Policy: deny camera, microphone, geolocation, and the
    // interest-cohort (FLoC) feature. Viceden is text-only — no app code
    // ever needs these surfaces.
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=(), payment=(), usb=(), interest-cohort=()",
        ),
    );
    if is_asset_request {
        headers.insert(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("public, max-age=3600"),
        );
    } else {
        headers
            .insert(HeaderName::from_static("cache-control"), HeaderValue::from_static("no-store"));
    }
    if secure_transport {
        // F2: include `preload` so the domain is eligible for the HSTS
        // preload list (browsers ship hardcoded HTTPS-only). Once the
        // domain is in the list, downgrading the policy is hard — only
        // ship `preload` after a deliberate review and submission to
        // https://hstspreload.org/.
        headers.insert(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        config::{AuthSettings, DatabaseSettings, ServerSettings, Settings},
        forum::ROOT_HANDLE,
    };

    #[test]
    fn founder_password_override_is_validated() {
        let settings = settings(true, Some("safe-founder-password-123".to_string()));

        super::validate_deployed_boot_inputs(&settings).expect("override should satisfy policy");
    }

    #[test]
    fn deployed_mode_requires_token_pepper() {
        let mut settings = settings(true, Some("safe-founder-password-123".to_string()));
        settings.auth.token_pepper = None;

        let error = super::validate_deployed_boot_inputs(&settings)
            .expect_err("deployed mode should reject missing token pepper");

        let error = error.to_string();
        assert!(error.contains("boot_policy_violation code=auth.token_pepper"));
        assert!(error.contains("VICEDEN_AUTH__TOKEN_PEPPER"));
    }

    #[test]
    fn deployed_mode_rejects_root_founder_password_override() {
        let settings = settings(true, Some(ROOT_HANDLE.to_string()));

        let error = super::validate_deployed_boot_inputs(&settings)
            .expect_err("root override should be rejected");

        let error = error.to_string();
        assert!(error.contains("boot_policy_violation code=auth.founder_password"));
        assert!(error.contains("founder password"));
    }

    #[test]
    fn deployed_mode_requires_classifier_api_key() {
        let mut settings = settings(true, Some("safe-founder-password-123".to_string()));
        settings.classifier.api_key_env = "VICEDEN_TEST_NONEXISTENT_KEY".to_string();
        std::env::remove_var(&settings.classifier.api_key_env);

        let error = super::validate_deployed_boot_inputs(&settings)
            .expect_err("deployed mode should reject missing classifier API key");

        let error = error.to_string();
        assert!(error.contains("boot_policy_violation code=classifier.api_key"));
        assert!(error.contains(&settings.classifier.api_key_env));
    }

    #[test]
    fn local_mode_does_not_require_classifier_api_key() {
        let mut settings = settings(false, None);
        settings.classifier.api_key_env = "VICEDEN_TEST_NONEXISTENT_KEY".to_string();
        std::env::remove_var(&settings.classifier.api_key_env);

        super::validate_deployed_boot_inputs(&settings)
            .expect("local mode should not require API key");
    }

    fn settings(require_connection: bool, founder_password: Option<String>) -> Settings {
        let api_key_env = format!("VICEDEN_TEST_CLASSIFIER_KEY_{}", Uuid::new_v4().simple());
        std::env::set_var(&api_key_env, "test-classifier-key");
        let classifier = crate::config::ClassifierSettings { api_key_env, ..Default::default() };
        Settings {
            server: ServerSettings {
                bind: "127.0.0.1:3000".to_string(),
                max_request_body_bytes: 131_072,
                request_timeout_seconds: 15,
                secure_cookies: false,
                proxy: Default::default(),
                rate_limit: Default::default(),
            },
            database: DatabaseSettings {
                url: "postgres://viceden:change-me@127.0.0.1:5432/viceden".to_string(),
                require_connection,
                max_connections: 1,
            },
            auth: AuthSettings {
                founder_password,
                token_pepper: Some(
                    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
                ),
            },
            classifier,
            ops: Default::default(),
            boards_file: "config/boards.toml".to_string(),
        }
    }
}
