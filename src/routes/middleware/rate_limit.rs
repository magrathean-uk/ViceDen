//! Per-route rate-limit layers using the local token-bucket middleware.
//!
//! Three keying strategies map onto our buckets:
//!
//! - `KeyKind::Ip`: keyed off `RealClientIp` from the client_ip middleware.
//! - `KeyKind::User`: keyed off the authenticated session's user_id.
//! - `KeyKind::IpHandle`: special-case for /login, key = `ip:lc(handle)`.
//!
//! Founder bypass: every layer checks the request session first; if role is
//! 'founder' the layer short-circuits before the GCRA check.

use axum::{
    body::Body,
    http::{header, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};

#[derive(Clone, Copy, Debug)]
pub enum KeyKind {
    Ip,
    User,
    IpHandle,
    Pat,
}

#[derive(Clone, Debug)]
pub struct BucketSpec {
    pub burst_size: u32,
    /// Token refill rate, in tokens per second.
    ///
    /// `f32` so we can express ratios like "5 per hour" as `5.0 / 3600.0`
    /// or "1 per 30s" as `1.0 / 30.0` without losing the intent in integer
    /// rounding.
    pub per_second: f32,
    pub key_kind: KeyKind,
}

/// Bucket presets, one per logical action surface, sourced from spec §5.
///
/// `per_second` values follow the spec's ratio formulation. For instance,
/// `5/h` becomes `5.0 / 3600.0` so the bucket refills at exactly that rate.
pub mod presets {
    use super::{BucketSpec, KeyKind};

    pub fn login() -> BucketSpec {
        BucketSpec { burst_size: 5, per_second: 1.0, key_kind: KeyKind::Ip }
    }

    pub fn register() -> BucketSpec {
        BucketSpec { burst_size: 3, per_second: 5.0 / 3600.0, key_kind: KeyKind::Ip }
    }

    pub fn thread_new() -> BucketSpec {
        BucketSpec { burst_size: 2, per_second: 0.5, key_kind: KeyKind::User }
    }

    pub fn post_new() -> BucketSpec {
        BucketSpec { burst_size: 5, per_second: 1.0 / 30.0, key_kind: KeyKind::User }
    }

    pub fn vote() -> BucketSpec {
        BucketSpec { burst_size: 30, per_second: 0.5, key_kind: KeyKind::User }
    }

    pub fn report() -> BucketSpec {
        BucketSpec { burst_size: 5, per_second: 5.0 / 3600.0, key_kind: KeyKind::User }
    }

    pub fn search() -> BucketSpec {
        BucketSpec { burst_size: 30, per_second: 1.0 / 3.0, key_kind: KeyKind::User }
    }

    pub fn pat_create() -> BucketSpec {
        BucketSpec { burst_size: 5, per_second: 5.0 / 3600.0, key_kind: KeyKind::User }
    }

    pub fn api_pat() -> BucketSpec {
        BucketSpec { burst_size: 60, per_second: 1.0, key_kind: KeyKind::Pat }
    }
}

/// Returns the routing layer placeholder for the local bucket middleware.
pub fn layer(spec: BucketSpec) -> tower::layer::util::Identity {
    let _ = spec;
    tower::layer::util::Identity::new()
}

/// Per-process token bucket with per-key, per-route, founder-bypass, and
/// application-specific 429 behavior.
pub use bucket_driver::*;

mod bucket_driver {
    use super::*;
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::Mutex;

    use crate::routes::middleware::client_ip::RealClientIp;

    type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

    #[derive(Clone)]
    pub struct Bucket {
        pub(crate) spec: BucketSpec,
        state: Arc<Mutex<HashMap<String, BucketState>>>,
    }

    struct BucketState {
        tokens: f32,
        last_refill: Instant,
    }

    impl Bucket {
        pub fn new(spec: BucketSpec) -> Self {
            Self { spec, state: Arc::new(Mutex::new(HashMap::new())) }
        }

        pub async fn check(&self, key: &str) -> bool {
            let now = Instant::now();
            let mut guard = self.state.lock().await;
            let entry = guard
                .entry(key.to_string())
                .or_insert(BucketState { tokens: self.spec.burst_size as f32, last_refill: now });
            let elapsed = now.duration_since(entry.last_refill).as_secs_f32();
            entry.tokens =
                (entry.tokens + elapsed * self.spec.per_second).min(self.spec.burst_size as f32);
            entry.last_refill = now;
            if entry.tokens >= 1.0 {
                entry.tokens -= 1.0;
                true
            } else {
                false
            }
        }
    }

    /// Axum middleware factory for one bucket.
    #[allow(clippy::type_complexity)]
    pub fn rate_limit_middleware(
        bucket: Bucket,
        role_extractor: RoleExtractor,
    ) -> middleware::FromFnLayer<
        impl Fn(Request<Body>, Next) -> BoxedFuture<Response> + Clone + Send + 'static,
        (),
        (Request<Body>,),
    > {
        middleware::from_fn(move |req: Request<Body>, next: Next| {
            let bucket = bucket.clone();
            let extractor = role_extractor.clone();
            Box::pin(handle_request(bucket, extractor, req, next)) as BoxedFuture<Response>
        })
    }

    async fn handle_request(
        bucket: Bucket,
        extractor: RoleExtractor,
        req: Request<Body>,
        next: Next,
    ) -> Response {
        // Pull request-scoped values up front so the future stays `Send`
        // (a borrowed `Request<Body>` is `!Sync` and can't cross awaits).
        let session_token = session_token_from_cookie(&req);
        let bearer_token = bearer_token_from_authorization(&req);
        let real_ip = req.extensions().get::<RealClientIp>().map(|ip| ip.0.to_string());
        let path = req.uri().path().to_string();

        // Founder bypass.
        if let Some(token) = session_token.as_deref() {
            if let Some(role) = extractor.role_for_token(token).await {
                if role == "founder" {
                    return next.run(req).await;
                }
            }
        }

        let key = match bucket.spec.key_kind {
            KeyKind::Ip => real_ip.clone().unwrap_or_else(|| "unknown".into()),
            KeyKind::User => match session_token.as_deref() {
                Some(token) => {
                    extractor.user_id_for_token(token).await.unwrap_or_else(|| "anon".into())
                }
                None => "anon".into(),
            },
            KeyKind::Pat => match bearer_token.as_deref() {
                Some(token) => {
                    extractor.pat_id_for_token(token).await.unwrap_or_else(|| "anon".into())
                }
                None => "anon".into(),
            },
            KeyKind::IpHandle => {
                let ip = real_ip.clone().unwrap_or_else(|| "unknown".into());
                let handle = peek_handle(&req).unwrap_or_default();
                format!("{ip}|{}", handle.to_ascii_lowercase())
            }
        };

        if bucket.check(&key).await {
            next.run(req).await
        } else {
            too_many(&path)
        }
    }

    fn too_many(path: &str) -> Response {
        tracing::warn!(path = %path, "rate_limit_breach");
        if path.starts_with("/api/") {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))],
                r#"{"error":"rate_limited","retry_after_s":60}"#,
            )
                .into_response()
        } else {
            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
                    (header::RETRY_AFTER, HeaderValue::from_static("60")),
                ],
                "<!DOCTYPE html><html><body><h1>Slow down.</h1></body></html>".to_string(),
            )
                .into_response()
        }
    }

    /// Reading the form body to peek the handle for /login key extraction
    /// requires consuming and replacing the body — gnarly. The simpler
    /// alternative is to apply only an IP-based bucket on /login at the
    /// HTTP layer and let the existing app-layer throttle (forum.rs)
    /// handle the (ip, handle) key. We choose the latter; this stub
    /// returns None so /login uses Ip keying at the HTTP layer.
    ///
    /// Kept synchronous so the surrounding middleware future stays `Send`
    /// (a borrowed `Request<Body>` is `!Sync` and can't cross awaits).
    fn peek_handle(_req: &Request<Body>) -> Option<String> {
        None
    }

    #[derive(Clone)]
    pub struct RoleExtractor {
        pool: sqlx::PgPool,
        pepper: Option<String>,
    }

    impl RoleExtractor {
        pub fn new(pool: sqlx::PgPool, pepper: Option<String>) -> Self {
            Self { pool, pepper }
        }

        async fn role_for_token(&self, token: &str) -> Option<String> {
            let session = crate::repository::sessions::SessionRepo
                .find_by_token(&self.pool, token, self.pepper.as_deref())
                .await
                .ok()??;
            let user = crate::repository::users::UserRepo
                .find_by_id(&self.pool, session.user_id)
                .await
                .ok()??;
            Some(user.role)
        }

        async fn user_id_for_token(&self, token: &str) -> Option<String> {
            let session = crate::repository::sessions::SessionRepo
                .find_by_token(&self.pool, token, self.pepper.as_deref())
                .await
                .ok()??;
            Some(session.user_id.to_string())
        }

        async fn pat_id_for_token(&self, token: &str) -> Option<String> {
            let auth = crate::repository::pats::PatRepo
                .authenticate(&self.pool, token, self.pepper.as_deref())
                .await
                .ok()
                .flatten()?;
            Some(auth.1.to_string())
        }
    }

    fn session_token_from_cookie(req: &Request<Body>) -> Option<String> {
        let cookie_header = req.headers().get(header::COOKIE)?.to_str().ok()?;
        for part in cookie_header.split(';') {
            let part = part.trim();
            if let Some(rest) = part
                .strip_prefix("__Host-viceden_session=")
                .or_else(|| part.strip_prefix("viceden_session="))
            {
                return Some(rest.to_string());
            }
        }
        None
    }

    fn bearer_token_from_authorization(req: &Request<Body>) -> Option<String> {
        let raw = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
        raw.strip_prefix("Bearer ").map(|s| s.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[tokio::test]
        async fn concurrent_bucket_checks_consume_exact_burst() {
            let bucket =
                Bucket::new(BucketSpec { burst_size: 5, per_second: 0.0, key_kind: KeyKind::Ip });
            let mut handles = Vec::new();
            for _ in 0..32 {
                let bucket = bucket.clone();
                handles.push(tokio::spawn(async move { bucket.check("same-client").await }));
            }

            let mut allowed = 0;
            for handle in handles {
                if handle.await.expect("bucket task should join") {
                    allowed += 1;
                }
            }

            assert_eq!(allowed, 5);
            assert!(!bucket.check("same-client").await);
        }
    }
}
