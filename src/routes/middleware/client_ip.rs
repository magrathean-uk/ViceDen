//! Trusted-proxy client-IP resolution middleware.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;

use axum::{
    extract::ConnectInfo,
    http::{HeaderMap, Request},
    middleware::{FromFnLayer, Next},
    response::Response,
};
use ipnet::IpNet;

type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[derive(Clone, Copy)]
pub struct RealClientIp(pub IpAddr);

#[derive(Clone)]
pub struct ProxyConfig {
    pub trusted_cidrs: Vec<IpNet>,
    pub client_ip_headers: Vec<String>,
}

pub fn resolve_client_ip(
    headers: &HeaderMap,
    peer: SocketAddr,
    trusted_cidrs: &[IpNet],
    client_ip_headers: &[String],
) -> IpAddr {
    let peer_ip = peer.ip();
    let peer_trusted = trusted_cidrs.iter().any(|cidr| cidr.contains(&peer_ip));
    if !peer_trusted {
        return peer_ip;
    }

    for header_name in client_ip_headers {
        if let Some(value) =
            headers.get(header_name.to_ascii_lowercase().as_str()).and_then(|v| v.to_str().ok())
        {
            // Take the first comma-separated entry.
            let first = value.split(',').next().unwrap_or(value).trim();
            if let Ok(parsed) = first.parse::<IpAddr>() {
                return parsed;
            }
        }
    }
    peer_ip
}

/// Build the client-IP resolution layer. Once installed, every request gets a
/// `RealClientIp` extension after honouring trusted-proxy headers.
#[allow(clippy::type_complexity)]
pub fn client_ip_layer(
    config: ProxyConfig,
) -> FromFnLayer<
    impl Fn(Request<axum::body::Body>, Next) -> BoxedFuture<Response> + Clone + Send + 'static,
    (),
    (Request<axum::body::Body>,),
> {
    use axum::middleware;
    middleware::from_fn(move |request: Request<axum::body::Body>, next: Next| {
        let cfg = config.clone();
        Box::pin(attach_client_ip(cfg, request, next)) as BoxedFuture<Response>
    })
}

async fn attach_client_ip(
    cfg: ProxyConfig,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0)
        .unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
    let resolved =
        resolve_client_ip(request.headers(), peer, &cfg.trusted_cidrs, &cfg.client_ip_headers);
    request.extensions_mut().insert(RealClientIp(resolved));
    next.run(request).await
}
