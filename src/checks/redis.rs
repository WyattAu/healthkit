//! Redis health check.
//!
//! Sends `PING` over a synchronous connection (no async runtime features are
//! pulled from the `redis` crate — this module works with
//! `default-features = false`) executed on a blocking thread and bounded by a
//! configurable timeout.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::spawn_blocking;
use tokio::time::timeout;

use crate::error::HealthCheckError;
use crate::registry::HealthRegistry;
use crate::types::HealthStatus;

/// Health check that verifies a Redis server answers `PING`.
///
/// Semantics:
/// - `PING` replies `PONG` within `timeout` and below `warn_above_ms` → [`HealthStatus::Healthy`]
/// - `PING` succeeds but took longer than `warn_above_ms` → [`HealthStatus::Degraded`]
/// - connection / command error or timeout → [`Err`]
///   (`DependencyUnavailable` / `CheckTimedOut`), which the registry folds
///   into [`HealthStatus::Unhealthy`]
///
/// # Example
///
/// ```rust,no_run
/// use std::time::Duration;
/// use healthkit::{HealthRegistry, checks::redis::RedisCheck};
///
/// # fn example() {
/// let registry = HealthRegistry::new();
/// RedisCheck::new("redis://127.0.0.1:6379", Duration::from_secs(2), 500)
///     .register(&registry, "cache");
/// # }
/// ```
#[derive(Clone)]
pub struct RedisCheck {
    url: String,
    timeout: Duration,
    warn_above: Duration,
}

impl RedisCheck {
    /// Create a check for the Redis server at `url` (e.g. `redis://host:6379`).
    ///
    /// `timeout` bounds each probe (connection + `PING`); `warn_above_ms` is
    /// the latency threshold above which a successful probe is reported as
    /// `Degraded` instead of `Healthy`.
    pub fn new(url: impl Into<String>, timeout: Duration, warn_above_ms: u64) -> Self {
        Self {
            url: url.into(),
            timeout,
            warn_above: Duration::from_millis(warn_above_ms),
        }
    }

    /// Override the probe timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the latency threshold (in milliseconds) above which a
    /// successful probe is reported as `Degraded`.
    pub fn with_warn_above_ms(mut self, warn_above_ms: u64) -> Self {
        self.warn_above = Duration::from_millis(warn_above_ms);
        self
    }

    /// Run the probe once and map the outcome to a health status.
    pub async fn check(&self) -> Result<HealthStatus, HealthCheckError> {
        let start = Instant::now();
        let url = self.url.clone();
        let io_deadline = self.timeout;
        let probe = timeout(
            self.timeout,
            spawn_blocking(move || ping(&url, io_deadline)),
        )
        .await;
        let elapsed = start.elapsed();

        match probe {
            Ok(Ok(Ok(reply))) if reply != "PONG" => Err(HealthCheckError::DependencyUnavailable(
                format!("unexpected PING reply: {reply}"),
            )),
            Ok(Ok(Ok(_))) if elapsed > self.warn_above => Ok(HealthStatus::Degraded),
            Ok(Ok(Ok(_))) => Ok(HealthStatus::Healthy),
            Ok(Ok(Err(err))) => Err(HealthCheckError::DependencyUnavailable(format!(
                "redis probe failed: {err}"
            ))),
            Ok(Err(err)) => Err(HealthCheckError::CheckFailed(format!(
                "redis probe task failed: {err}"
            ))),
            Err(_) => Err(HealthCheckError::CheckTimedOut(self.timeout)),
        }
    }

    /// Register this check with `registry` under `name`.
    ///
    /// Convenience wrapper that adapts [`RedisCheck::check`] to the closure
    /// shape `HealthRegistry::add_check` expects.
    pub fn register(self, registry: &HealthRegistry, name: impl Into<String>) {
        let check = Arc::new(self);
        registry.add_check(name, move || {
            let check = Arc::clone(&check);
            async move { check.check().await }
        });
    }
}

/// Perform a blocking `PING` against the server at `url`, returning its reply.
///
/// `io_deadline` bounds each socket read/write so the blocking task can never
/// hang on a silent server — the async caller's own timeout fires first, and
/// this guarantees the blocking thread eventually unwinds.
fn ping(url: &str, io_deadline: Duration) -> Result<String, ::redis::RedisError> {
    let client = ::redis::Client::open(url)?;
    let mut connection = client.get_connection()?;
    // Slack above the async deadline keeps the outer timeout authoritative
    // while guaranteeing the blocking task still terminates.
    let slack = io_deadline + Duration::from_secs(1);
    connection.set_read_timeout(Some(slack))?;
    connection.set_write_timeout(Some(slack))?;
    ::redis::cmd("PING").query::<String>(&mut connection)
}

// Tests drive a hermetic fake RESP server over TCP plus config/error-mapping
// paths; unwrap/expect and panicking asserts are the test signal here.
// Live-server tests are marked `#[ignore]` (run with `--ignored` and a real
// Redis reachable at $HEALTHKIT_REDIS_URL).
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::thread;

    /// Bind an ephemeral TCP listener and serve one connection that parses
    /// complete RESP array commands from the byte stream and replies with
    /// `reply` to each — a minimal fake RESP server sufficient for `PING`
    /// (the client handshake may issue additional commands such as
    /// `CLIENT SETINFO`, possibly batched into a single write).
    fn spawn_fake_redis(reply: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    while let Some(consumed) = resp_command_len(&buf) {
                        buf.drain(..consumed);
                        if stream
                            .write_all(reply)
                            .and_then(|_| stream.flush())
                            .is_err()
                        {
                            return;
                        }
                    }
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
            }
        });
        addr
    }

    /// Byte length of the first complete RESP array command in `buf`
    /// (`*<n>\r\n` followed by n bulk strings `$<len>\r\n<data>\r\n`), or
    /// `None` if the buffer holds no complete command yet.
    fn resp_command_len(buf: &[u8]) -> Option<usize> {
        if buf.is_empty() || buf[0] != b'*' {
            return None;
        }
        let mut i = 1;
        let mut argc: usize = 0;
        while i < buf.len() && buf[i].is_ascii_digit() {
            argc = argc * 10 + (buf[i] - b'0') as usize;
            i += 1;
        }
        if i + 1 >= buf.len() || buf[i] != b'\r' || buf[i + 1] != b'\n' {
            return None;
        }
        i += 2;
        for _ in 0..argc {
            if i >= buf.len() || buf[i] != b'$' {
                return None;
            }
            i += 1;
            let mut arg_len: usize = 0;
            while i < buf.len() && buf[i].is_ascii_digit() {
                arg_len = arg_len * 10 + (buf[i] - b'0') as usize;
                i += 1;
            }
            if i + 1 >= buf.len() || buf[i] != b'\r' || buf[i + 1] != b'\n' {
                return None;
            }
            i += 2 + arg_len + 2;
            if i > buf.len() {
                return None;
            }
        }
        Some(i)
    }

    /// Bind an ephemeral TCP listener whose connection is accepted but never
    /// answered — for exercising the probe timeout. Stays silent just long
    /// enough to outlive the probe deadlines used in the tests.
    fn spawn_silent_redis() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                thread::sleep(Duration::from_secs(1));
            }
        });
        addr
    }

    /// Bind and immediately drop a listener to obtain an addr that refuses
    /// connections.
    fn refused_addr() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        addr
    }

    #[tokio::test]
    async fn healthy_when_ping_replies_pong() {
        let addr = spawn_fake_redis(b"+PONG\r\n");
        let check = RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500);
        assert_eq!(check.check().await.unwrap(), HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn degraded_when_latency_exceeds_threshold() {
        let addr = spawn_fake_redis(b"+PONG\r\n");
        // A threshold of 0 ms demotes every successful probe to Degraded.
        let check = RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 0);
        assert_eq!(check.check().await.unwrap(), HealthStatus::Degraded);
    }

    #[tokio::test]
    async fn unhealthy_on_error_reply() {
        let addr = spawn_fake_redis(b"-ERR boom\r\n");
        let check = RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500);
        match check.check().await {
            Err(HealthCheckError::DependencyUnavailable(msg)) => {
                assert!(msg.contains("redis probe failed"));
            }
            other => panic!("expected DependencyUnavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unhealthy_on_unexpected_reply() {
        // A bulk string that is not `PONG` must not count as healthy.
        let addr = spawn_fake_redis(b"+PANG\r\n");
        let check = RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500);
        match check.check().await {
            Err(HealthCheckError::DependencyUnavailable(msg)) => {
                assert!(msg.contains("unexpected PING reply"));
            }
            other => panic!("expected DependencyUnavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unhealthy_when_connection_refused() {
        let addr = refused_addr();
        let check = RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500);
        match check.check().await {
            Err(HealthCheckError::DependencyUnavailable(msg)) => {
                assert!(msg.contains("redis probe failed"));
            }
            other => panic!("expected DependencyUnavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unhealthy_on_invalid_url() {
        let check = RedisCheck::new("not-a-url", Duration::from_secs(5), 500);
        match check.check().await {
            Err(HealthCheckError::DependencyUnavailable(msg)) => {
                assert!(msg.contains("redis probe failed"));
            }
            other => panic!("expected DependencyUnavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn times_out_when_probe_exceeds_deadline() {
        // The server accepts but never answers; the deadline must fire while
        // the probe waits for the PING reply.
        let addr = spawn_silent_redis();
        let check = RedisCheck::new(format!("redis://{addr}"), Duration::from_millis(100), 500);
        match check.check().await {
            Err(HealthCheckError::CheckTimedOut(d)) => {
                assert_eq!(d, Duration::from_millis(100));
            }
            other => panic!("expected CheckTimedOut, got {other:?}"),
        }
    }

    #[test]
    fn builder_overrides_are_applied() {
        let check = RedisCheck::new("redis://example.test", Duration::from_secs(30), 0)
            .with_timeout(Duration::from_secs(5))
            .with_warn_above_ms(50);
        assert_eq!(check.timeout, Duration::from_secs(5));
        assert_eq!(check.warn_above, Duration::from_millis(50));
        assert_eq!(check.url, "redis://example.test");
    }

    #[tokio::test]
    async fn register_integrates_with_registry() {
        let addr = spawn_fake_redis(b"+PONG\r\n");
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            RedisCheck::new(format!("redis://{addr}"), Duration::from_secs(5), 500)
                .register(&r, "cache");
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "cache");
        assert_eq!(results[0].status, HealthStatus::Healthy);
    }

    #[tokio::test]
    #[ignore = "requires a live Redis server"]
    async fn live_redis_reports_healthy() {
        let url = std::env::var("HEALTHKIT_REDIS_URL").unwrap_or("redis://127.0.0.1".to_string());
        let check = RedisCheck::new(url, Duration::from_secs(2), 500);
        assert_eq!(check.check().await.unwrap(), HealthStatus::Healthy);
    }
}
