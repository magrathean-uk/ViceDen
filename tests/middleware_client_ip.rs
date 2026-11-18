use std::net::{IpAddr, SocketAddr};

use axum::http::{HeaderMap, HeaderValue};
use ipnet::IpNet;
use viceden::routes::middleware::client_ip::resolve_client_ip;

fn cidrs(values: &[&str]) -> Vec<IpNet> {
    values.iter().map(|v| v.parse().unwrap()).collect()
}

#[test]
fn honours_cf_connecting_ip_when_peer_is_trusted() {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.5"));

    let trusted = cidrs(&["127.0.0.1/32"]);
    let peer: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let resolved = resolve_client_ip(&headers, peer, &trusted, &["Cf-Connecting-Ip".into()]);
    assert_eq!(resolved, "203.0.113.5".parse::<IpAddr>().unwrap());
}

#[test]
fn ignores_header_when_peer_not_trusted() {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.5"));
    let trusted = cidrs(&["10.0.0.0/8"]);
    let peer: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let resolved = resolve_client_ip(&headers, peer, &trusted, &["Cf-Connecting-Ip".into()]);
    assert_eq!(resolved, "127.0.0.1".parse::<IpAddr>().unwrap());
}

#[test]
fn falls_through_x_forwarded_for_first_value() {
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7, 198.51.100.1"));
    let trusted = cidrs(&["127.0.0.1/32"]);
    let peer: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    let resolved = resolve_client_ip(
        &headers,
        peer,
        &trusted,
        &["Cf-Connecting-Ip".into(), "X-Forwarded-For".into()],
    );
    assert_eq!(resolved, "203.0.113.7".parse::<IpAddr>().unwrap());
}

#[test]
fn rejects_spoofed_cf_connecting_ip_from_public_peer() {
    // Mirrors the bug fixed by wiring `into_make_service_with_connect_info`
    // in main.rs: when the real peer is a public address, the resolver must
    // ignore `Cf-Connecting-Ip` regardless of what the client claims.
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", HeaderValue::from_static("198.51.100.42"));
    let trusted = cidrs(&["127.0.0.1/32", "::1/128"]);
    let peer: SocketAddr = "203.0.113.99:48732".parse().unwrap();
    let resolved = resolve_client_ip(&headers, peer, &trusted, &["Cf-Connecting-Ip".into()]);
    assert_eq!(
        resolved,
        "203.0.113.99".parse::<IpAddr>().unwrap(),
        "spoofed Cf-Connecting-Ip from a non-trusted peer must not be honoured"
    );
}
