//! SQLx database health check.
//!
//! Runs `SELECT 1` against a `sqlx::Pool` with a configurable timeout and a
//! latency warning threshold. The check is generic over the database backend:
//! it works with any pool (`SqlitePool`, `PgPool`, ...) as long as the
//! corresponding sqlx driver feature is enabled by the *consumer* — this
//! crate only pulls `sqlite` for its own tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ::sqlx::Database;
use tokio::time::timeout;

use crate::error::HealthCheckError;
use crate::registry::HealthRegistry;
use crate::types::HealthStatus;

/// Query used to verify database responsiveness.
const PROBE_QUERY: &str = "SELECT 1";

/// Health check that verifies a sqlx database pool is responsive.
///
/// Semantics:
/// - `SELECT 1` succeeds within `timeout` and below `warn_above_ms` → [`HealthStatus::Healthy`]
/// - `SELECT 1` succeeds but took longer than `warn_above_ms` → [`HealthStatus::Degraded`]
/// - query error or timeout → [`Err`] (`DependencyUnavailable` / `CheckTimedOut`),
///   which the registry folds into [`HealthStatus::Unhealthy`]
///
/// # Example
///
/// ```rust,no_run
/// use std::time::Duration;
/// use healthkit::{HealthRegistry, checks::sqlx::SqlxCheck};
///
/// # async fn example(pool: sqlx::SqlitePool) {
/// let registry = HealthRegistry::new();
/// SqlxCheck::new(pool, Duration::from_secs(2), 500)
///     .register(&registry, "database");
/// # }
/// ```
pub struct SqlxCheck<DB: Database> {
    pool: ::sqlx::Pool<DB>,
    timeout: Duration,
    warn_above: Duration,
}

impl<DB> SqlxCheck<DB>
where
    DB: Database,
    for<'c> &'c mut <DB as Database>::Connection: ::sqlx::Executor<'c, Database = DB>,
    for<'a> <DB as Database>::Arguments<'a>: ::sqlx::IntoArguments<'a, DB>,
{
    /// Create a check for the given pool.
    ///
    /// `timeout` bounds each probe; `warn_above_ms` is the latency threshold
    /// above which a successful probe is reported as `Degraded` instead of
    /// `Healthy`.
    pub fn new(pool: ::sqlx::Pool<DB>, timeout: Duration, warn_above_ms: u64) -> Self {
        Self {
            pool,
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
        let probe = timeout(self.timeout, ::sqlx::query(PROBE_QUERY).execute(&self.pool)).await;
        let elapsed = start.elapsed();

        match probe {
            Ok(Ok(_)) if elapsed > self.warn_above => Ok(HealthStatus::Degraded),
            Ok(Ok(_)) => Ok(HealthStatus::Healthy),
            Ok(Err(err)) => Err(HealthCheckError::DependencyUnavailable(format!(
                "database probe failed: {err}"
            ))),
            Err(_) => Err(HealthCheckError::CheckTimedOut(self.timeout)),
        }
    }

    /// Register this check with `registry` under `name`.
    ///
    /// Convenience wrapper that adapts [`SqlxCheck::check`] to the closure
    /// shape `HealthRegistry::add_check` expects.
    pub fn register(self, registry: &HealthRegistry, name: impl Into<String>) {
        let check = Arc::new(self);
        registry.add_check(name, move || {
            let check = Arc::clone(&check);
            async move { check.check().await }
        });
    }
}

// Tests exercise failure paths against a real in-memory SQLite database;
// unwrap/expect and panicking asserts are the test signal here.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;

    async fn memory_pool() -> ::sqlx::SqlitePool {
        ::sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn healthy_when_probe_succeeds_within_threshold() {
        let check = SqlxCheck::new(memory_pool().await, Duration::from_secs(2), 500);
        assert_eq!(check.check().await.unwrap(), HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn degraded_when_latency_exceeds_threshold() {
        // A threshold of 0 ms demotes every successful probe to Degraded.
        let check = SqlxCheck::new(memory_pool().await, Duration::from_secs(2), 0);
        assert_eq!(check.check().await.unwrap(), HealthStatus::Degraded);
    }

    #[tokio::test]
    async fn builder_overrides_are_applied() {
        let pool = memory_pool().await;
        let check = SqlxCheck::new(pool.clone(), Duration::from_secs(30), 0)
            .with_timeout(Duration::from_secs(5))
            .with_warn_above_ms(50);

        // Healthy: 50 ms threshold is not exceeded by a trivial query.
        assert_eq!(check.check().await.unwrap(), HealthStatus::Healthy);
        drop(check);
        pool.close().await;
    }

    #[tokio::test]
    async fn unhealthy_when_probe_fails() {
        let pool = memory_pool().await;
        pool.close().await;
        let check = SqlxCheck::new(pool, Duration::from_secs(2), 500);
        match check.check().await {
            Err(HealthCheckError::DependencyUnavailable(msg)) => {
                assert!(msg.contains("database probe failed"));
            }
            other => panic!("expected DependencyUnavailable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn times_out_when_probe_exceeds_deadline() {
        // Exhaust the pool's single connection so the probe blocks waiting
        // for a connection; the deadline must fire while it waits.
        let pool = ::sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let _held = pool.acquire().await.unwrap();
        let check = SqlxCheck::new(pool, Duration::from_millis(100), 500);
        match check.check().await {
            Err(HealthCheckError::CheckTimedOut(d)) => {
                assert_eq!(d, Duration::from_millis(100));
            }
            other => panic!("expected CheckTimedOut, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn register_integrates_with_registry() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        let pool = memory_pool().await;
        tokio::task::spawn_blocking(move || {
            SqlxCheck::new(pool, Duration::from_secs(2), 500).register(&r, "database");
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "database");
        assert_eq!(results[0].status, HealthStatus::Healthy);
    }
}
