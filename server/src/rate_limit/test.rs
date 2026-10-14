//! Tests for per-client rate limiting.

use super::{RateLimitConfig, RateLimiter, client_key, middleware};
use crate::AppState;
use actix_web::{
    App, HttpResponse,
    dev::{Service, ServiceResponse},
    http::{StatusCode, header::RETRY_AFTER},
    middleware::from_fn,
    test::{self, TestRequest},
    web,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

/// Builds a limiter allowing `burst` immediate requests and one per second after that.
fn limiter(burst: u32, max_wait: Duration) -> Arc<RateLimiter> {
    RateLimiter::new(&RateLimitConfig {
        enabled: true,
        requests_per_minute: 60,
        burst,
        max_wait,
    })
    .unwrap()
    .unwrap()
}

/// Parses an IP address.
fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// Sends a request and returns its status and `Retry-After` header.
async fn send<S, R>(app: &S, req: R) -> (StatusCode, Option<String>)
where
    S: Service<R, Response = ServiceResponse, Error = actix_web::Error>,
{
    let resp = test::call_service(app, req).await;
    let retry_after = resp
        .headers()
        .get(RETRY_AFTER)
        .map(|v| v.to_str().unwrap().to_string());
    (resp.status(), retry_after)
}

#[tokio::test]
async fn rejects_when_delay_would_exceed_max_wait() {
    let limiter = limiter(2, Duration::ZERO);
    let client = ip("192.0.2.1");
    assert!(limiter.acquire(client).await.is_ok());
    assert!(limiter.acquire(client).await.is_ok());
    let retry_after = limiter.acquire(client).await.unwrap_err();
    assert!(retry_after > Duration::ZERO && retry_after <= Duration::from_secs(1));

    // Other clients have their own budget.
    assert!(limiter.acquire(ip("192.0.2.2")).await.is_ok());
}

#[tokio::test]
async fn delays_requests_within_max_wait() {
    let limiter = limiter(1, Duration::from_secs(5));
    let client = ip("192.0.2.1");
    assert!(limiter.acquire(client).await.is_ok());

    let start = Instant::now();
    assert!(limiter.acquire(client).await.is_ok());
    let waited = start.elapsed();
    assert!(waited >= Duration::from_millis(800), "waited {waited:?}");
}

#[test]
fn disabled_or_invalid_settings() {
    let disabled = RateLimitConfig {
        enabled: false,
        ..Default::default()
    };
    assert!(RateLimiter::new(&disabled).unwrap().is_none());

    let zero_rate = RateLimitConfig {
        requests_per_minute: 0,
        ..Default::default()
    };
    assert!(RateLimiter::new(&zero_rate).is_err());
}

#[test]
fn identifies_clients() {
    let key = |peer: &str, forwarded: Option<&str>| {
        let mut req = TestRequest::default().peer_addr(peer.parse::<SocketAddr>().unwrap());
        if let Some(forwarded) = forwarded {
            req = req.insert_header(("x-forwarded-for", forwarded));
        }
        client_key(&req.to_srv_request())
    };

    // Behind the local proxy, the entry it appended identifies the client.
    assert_eq!(
        key("127.0.0.1:1234", Some("203.0.113.9")),
        ip("203.0.113.9")
    );
    assert_eq!(
        key("127.0.0.1:1234", Some("198.51.100.1, 203.0.113.9")),
        ip("203.0.113.9")
    );
    assert_eq!(key("127.0.0.1:1234", None), ip("127.0.0.1"));
    // Direct clients can't choose their key through the header.
    assert_eq!(
        key("203.0.113.9:1234", Some("198.51.100.1")),
        ip("203.0.113.9")
    );
    // IPv6 clients share a budget per /64; IPv4-mapped addresses count as IPv4.
    assert_eq!(
        key("127.0.0.1:1234", Some("2001:db8:1:2:3:4:5:6")),
        ip("2001:db8:1:2::")
    );
    assert_eq!(key("[::ffff:203.0.113.9]:1234", None), ip("203.0.113.9"));
}

#[actix_web::test]
async fn limits_api_but_not_assets_webhooks_or_operator() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::for_test(dir.path());
    Arc::get_mut(&mut state).unwrap().api_token = Some("secret".into());

    let limiter = Some(limiter(1, Duration::ZERO));
    let app = test::init_service(
        App::new()
            .app_data(web::Data::from(state))
            .wrap(from_fn(move |req, next| {
                middleware(limiter.clone(), req, next)
            }))
            .default_service(web::to(HttpResponse::Ok)),
    )
    .await;
    let req = |path: &str| {
        TestRequest::get()
            .uri(path)
            .peer_addr("203.0.113.9:1234".parse().unwrap())
    };

    assert_eq!(
        send(&app, req("/api/overview").to_request()).await.0,
        StatusCode::OK
    );
    let (status, retry_after) = send(&app, req("/mcp").to_request()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(retry_after.as_deref(), Some("1"));

    for path in ["/", "/index.html", "/api/webhook/github"] {
        assert_eq!(
            send(&app, req(path).to_request()).await.0,
            StatusCode::OK,
            "{path}"
        );
    }
    let operator = req("/api/overview")
        .insert_header(("authorization", "Bearer secret"))
        .to_request();
    assert_eq!(send(&app, operator).await.0, StatusCode::OK);
    let wrong_token = req("/api/overview")
        .insert_header(("authorization", "Bearer wrong"))
        .to_request();
    assert_eq!(
        send(&app, wrong_token).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}
