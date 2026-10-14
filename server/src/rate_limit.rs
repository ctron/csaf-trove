//! Per-client rate limiting for the API, MCP and aggregator endpoints.
//!
//! Requests over the rate are delayed until the client's budget allows them, and only rejected
//! with `429 Too Many Requests` once the delay would exceed the configured maximum. This keeps
//! interactive use (dashboard, MCP clients) unaffected while slowing down bulk scraping.
#[cfg(test)]
mod test;

use crate::{AppState, api::auth::verify_bearer_token};
use actix_web::{
    Error, HttpResponse,
    body::{BoxBody, MessageBody},
    dev::{ServiceRequest, ServiceResponse},
    http::header::RETRY_AFTER,
    middleware::Next,
    web,
};
use anyhow::{Context, Result};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter as Governor, clock::Clock};
use serde::Deserialize;
use std::{
    net::{IpAddr, Ipv6Addr},
    num::NonZeroU32,
    sync::Arc,
    time::Duration,
};

/// Path prefixes subject to rate limiting; dashboard assets are served without limits.
const LIMITED_PREFIXES: &[&str] = &["/api/", "/mcp", "/.well-known/"];

/// Path prefixes exempt from rate limiting; webhooks authenticate themselves and the startup
/// status is cheap and polled by the dashboard while the server starts.
const EXEMPT_PREFIXES: &[&str] = &["/api/webhook/", "/api/startup"];

/// Interval for dropping the state of clients that have been idle long enough to be refilled.
const CLEANUP_INTERVAL: Duration = Duration::from_secs(60);

/// Rate limit settings, applied per client IP address (IPv6 per /64 network).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    /// Whether requests are rate limited at all.
    pub enabled: bool,
    /// Sustained number of requests per minute.
    pub requests_per_minute: u32,
    /// Number of requests allowed in a burst on top of the sustained rate.
    pub burst: u32,
    /// Longest time a request over the rate is delayed before it is rejected.
    #[serde(with = "humantime_serde")]
    pub max_wait: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            requests_per_minute: 60,
            burst: 60,
            max_wait: Duration::from_secs(10),
        }
    }
}

/// Per-client rate limiter that delays requests before rejecting them.
pub struct RateLimiter {
    /// Request budgets keyed by client address.
    limiter: DefaultKeyedRateLimiter<IpAddr>,
    /// Longest time a request is delayed before it is rejected.
    max_wait: Duration,
}

impl RateLimiter {
    /// Creates a limiter from its settings, or `None` if rate limiting is disabled.
    pub fn new(config: &RateLimitConfig) -> Result<Option<Arc<Self>>> {
        if !config.enabled {
            return Ok(None);
        }
        let rate = NonZeroU32::new(config.requests_per_minute)
            .context("rate_limit.requests_per_minute must be greater than zero")?;
        let burst =
            NonZeroU32::new(config.burst).context("rate_limit.burst must be greater than zero")?;
        Ok(Some(Arc::new(Self {
            limiter: Governor::keyed(Quota::per_minute(rate).allow_burst(burst)),
            max_wait: config.max_wait,
        })))
    }

    /// Waits until the client may proceed, or returns how long it should back off.
    pub async fn acquire(&self, client: IpAddr) -> Result<(), Duration> {
        let Err(not_until) = self.limiter.check_key(&client) else {
            return Ok(());
        };
        let wait = not_until.wait_time_from(self.limiter.clock().now());
        if wait > self.max_wait {
            return Err(wait);
        }
        // Concurrent requests of the same client queue up, so the wait can grow beyond the estimate.
        tokio::time::timeout(self.max_wait, self.limiter.until_key_ready(&client))
            .await
            .map_err(|_| self.max_wait)
    }

    /// Periodically drops the state of idle clients so memory stays bounded.
    pub fn spawn_cleanup(self: &Arc<Self>) {
        let limiter = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CLEANUP_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(limiter) = limiter.upgrade() else {
                    break;
                };
                limiter.limiter.retain_recent();
                limiter.limiter.shrink_to_fit();
            }
        });
    }
}

/// Returns whether requests to the path count against the rate limit.
fn is_limited(path: &str) -> bool {
    LIMITED_PREFIXES.iter().any(|p| path.starts_with(p))
        && !EXEMPT_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// Determines the rate limit key of the requesting client.
///
/// Behind the local reverse proxy, the client is the last `X-Forwarded-For` entry, which the
/// proxy sets itself. Direct connections use the peer address, so the header can't be spoofed.
/// IPv6 clients are grouped by /64, since a single host can easily rotate within its network.
fn client_key(req: &ServiceRequest) -> IpAddr {
    let peer = req
        .peer_addr()
        .map_or(IpAddr::from(Ipv6Addr::UNSPECIFIED), |a| a.ip());
    let client = if peer.is_loopback() {
        req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|ip| ip.trim().parse().ok())
            .unwrap_or(peer)
    } else {
        peer
    };
    match client {
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let mut segments = ip.segments();
                segments[4..].fill(0);
                IpAddr::V6(Ipv6Addr::from(segments))
            }
        },
        v4 => v4,
    }
}

/// Middleware that rate limits API, MCP and aggregator requests per client.
///
/// Requests carrying the operator's API token are not limited, and neither is anything when
/// rate limiting is disabled.
pub async fn middleware(
    limiter: Option<Arc<RateLimiter>>,
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<BoxBody>, Error> {
    let Some(limiter) = limiter else {
        return Ok(next.call(req).await?.map_into_boxed_body());
    };
    let exempt = !is_limited(req.path())
        || req
            .app_data::<web::Data<AppState>>()
            .and_then(|state| state.api_token.as_deref())
            .is_some_and(|token| verify_bearer_token(req.request(), token));

    if !exempt && let Err(retry_after) = limiter.acquire(client_key(&req)).await {
        tracing::debug!(client = %client_key(&req), path = req.path(), "Rate limited request");
        let response = HttpResponse::TooManyRequests()
            .insert_header((RETRY_AFTER, retry_after.as_secs().max(1)))
            .body("Too many requests, please slow down");
        return Ok(req.into_response(response));
    }

    Ok(next.call(req).await?.map_into_boxed_body())
}
