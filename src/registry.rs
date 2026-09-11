use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::join_all;
use std::sync::RwLock as StdRwLock;
use tokio::time::timeout;

use crate::error::HealthCheckError;
use crate::types::{CheckResult, HealthStatus};

type CheckFn = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<HealthStatus, HealthCheckError>> + Send>>
        + Send
        + Sync,
>;

/// A registered health check with its name and optional group.
#[derive(Clone)]
struct RegisteredCheck {
    name: String,
    /// Group the check belongs to, if any. Checks registered via
    /// [`HealthRegistry::add_check`] have no group and run in every
    /// registry-wide check; checks added to a group also run when that
    /// group is checked explicitly (see [`HealthRegistry::check_group`]).
    group: Option<String>,
    check_fn: Arc<CheckFn>,
}

/// Registry of health checks that can be executed on demand.
///
/// Checks registered against a registry run **concurrently** — one slow or
/// hanging dependency cannot stall the other probes — and each check is
/// bounded by the registry's per-check timeout (default 5 seconds, see
/// [`HealthRegistry::with_default_timeout`]). A check that exceeds the
/// timeout reports [`HealthStatus::Unhealthy`] with a message naming the
/// elapsed deadline.
#[derive(Clone)]
pub struct HealthRegistry {
    checks: Arc<StdRwLock<Vec<RegisteredCheck>>>,
    default_timeout: Duration,
}

impl HealthRegistry {
    /// Create a new empty health registry.
    pub fn new() -> Self {
        Self {
            checks: Arc::new(StdRwLock::new(Vec::new())),
            default_timeout: Duration::from_secs(5),
        }
    }

    /// Set the per-check timeout applied to every registered check.
    ///
    /// The default is 5 seconds. A check that does not finish within the
    /// deadline reports `Unhealthy` with the message
    /// `health check timed out after Ns`. Passing [`Duration::ZERO`]
    /// disables the timeout entirely (checks may then hang a probe — the
    /// kubelet's own `timeoutSeconds` remains the backstop).
    #[must_use]
    pub fn with_default_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// The per-check timeout currently configured for this registry.
    pub fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    /// Register a health check with a given name and async check function.
    ///
    /// The check runs on every registry-wide probe (`check_all`,
    /// `check_liveness`, `check_readiness`). To register a check that only
    /// runs for a named subset (e.g. an init check for the startup probe),
    /// see [`HealthRegistry::add_check_to_group`].
    pub fn add_check<F, Fut>(&self, name: impl Into<String>, check: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HealthStatus, HealthCheckError>> + Send + 'static,
    {
        self.add_check_inner(None, name, check);
    }

    /// Register a health check that belongs to a named group.
    ///
    /// Grouped checks run on every registry-wide probe (`check_all`,
    /// `check_liveness`, `check_readiness`) *and* when their group is
    /// checked explicitly via [`HealthRegistry::check_group`]. This enables
    /// per-route check subsets — the Kubernetes startup probe, for example,
    /// should only answer "is initialization done?" and run a small
    /// `init` group instead of every dependency check.
    ///
    /// Group names are arbitrary strings; the same check name may appear in
    /// different groups, and a check never joins more than one group.
    pub fn add_check_to_group<F, Fut>(
        &self,
        group: impl Into<String>,
        name: impl Into<String>,
        check: F,
    ) where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HealthStatus, HealthCheckError>> + Send + 'static,
    {
        self.add_check_inner(Some(group.into()), name, check);
    }

    fn add_check_inner<F, Fut>(&self, group: Option<String>, name: impl Into<String>, check: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HealthStatus, HealthCheckError>> + Send + 'static,
    {
        let check_fn: Arc<CheckFn> = Arc::new(Box::new(move || Box::pin(check())));
        let mut checks = self
            .checks
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        checks.push(RegisteredCheck {
            name: name.into(),
            group,
            check_fn,
        });
    }

    /// Run all registered health checks and return the results.
    ///
    /// All checks execute concurrently and are bounded by the registry's
    /// per-check timeout (see [`HealthRegistry::with_default_timeout`]). A
    /// failing check's error is preserved as the result's `message`; a
    /// check that exceeds the timeout reports `Unhealthy` with a
    /// `health check timed out after Ns` message.
    ///
    /// Results are returned in registration order.
    pub async fn check_all(&self) -> Vec<CheckResult> {
        let timeout = self.default_timeout;
        let snapshot: Vec<RegisteredCheck> = self
            .checks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let results = join_all(snapshot.iter().map(|check| run_one(check, timeout))).await;

        #[cfg(feature = "metrics")]
        for result in &results {
            emit_facade_metrics(result);
        }

        results
    }

    /// Run only the checks registered in `group` (see
    /// [`HealthRegistry::add_check_to_group`]) and return their results.
    ///
    /// Like [`HealthRegistry::check_all`], the group's checks run
    /// concurrently and are bounded by the per-check timeout. Checks that
    /// were not added to the group do not run. An unknown group yields an
    /// empty `Vec`.
    pub async fn check_group(&self, group: &str) -> Vec<CheckResult> {
        let timeout = self.default_timeout;
        let snapshot: Vec<RegisteredCheck> = self
            .checks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        join_all(
            snapshot
                .iter()
                .filter(|check| check.group.as_deref() == Some(group))
                .map(|check| run_one(check, timeout)),
        )
        .await
    }

    /// Check liveness — returns `Healthy` if all checks pass, `Unhealthy` otherwise.
    pub async fn check_liveness(&self) -> Result<HealthStatus, HealthCheckError> {
        let results = self.check_all().await;
        Ok(aggregate(&results))
    }

    /// Check readiness — returns `Healthy` if all checks pass, `Unhealthy` if any fail.
    pub async fn check_readiness(
        &self,
    ) -> Result<(HealthStatus, Vec<CheckResult>), HealthCheckError> {
        let results = self.check_all().await;
        let overall = aggregate(&results);
        Ok((overall, results))
    }
}

/// Run a single check, measuring its duration and folding failures into the
/// result — including the per-check timeout deadline.
async fn run_one(check: &RegisteredCheck, deadline: Duration) -> CheckResult {
    let start = Instant::now();
    let outcome = if deadline.is_zero() {
        (check.check_fn)().await
    } else {
        match timeout(deadline, (check.check_fn)()).await {
            Ok(outcome) => outcome,
            Err(_) => Err(HealthCheckError::CheckTimedOut(deadline)),
        }
    };
    let duration = start.elapsed();

    let (status, message) = match outcome {
        Ok(status) => (status, None),
        Err(err) => (HealthStatus::Unhealthy, Some(err.to_string())),
    };

    CheckResult {
        name: check.name.clone(),
        status,
        message,
        duration,
    }
}

/// Aggregate individual check results into the worst observed status.
fn aggregate(results: &[CheckResult]) -> HealthStatus {
    results
        .iter()
        .map(|r| r.status)
        .max_by_key(|s| match s {
            HealthStatus::Healthy => 0,
            HealthStatus::Degraded => 1,
            HealthStatus::Unhealthy => 2,
        })
        .unwrap_or(HealthStatus::Healthy)
}

/// Emit one check outcome through the `metrics` facade crate (de-facto
/// observability facade in the Rust ecosystem): a `healthkit_check_result`
/// counter labeled by check name and status, plus a
/// `healthkit_check_duration_seconds` histogram labeled by check name.
///
/// Emits alongside — not instead of — the hand-rolled Prometheus renderer
/// (see [`crate::metrics`]). With no recorder installed the calls are
/// no-ops, so this is always safe.
#[cfg(feature = "metrics")]
fn emit_facade_metrics(result: &CheckResult) {
    use metrics::{counter, histogram};

    counter!(
        "healthkit_check_result",
        "name" => result.name.clone(),
        "status" => result.status.to_string(),
    )
    .increment(1);
    histogram!(
        "healthkit_check_duration_seconds",
        "name" => result.name.clone(),
    )
    .record(result.duration.as_secs_f64());
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Tests exercise failure paths and invariants directly; unwrap/expect,
// slicing, and panicking asserts are acceptable here — violations
// surface as test failures, not production panics.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn checks_run_concurrently_slow_check_does_not_delay_others() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            for i in 0..5 {
                r.add_check(format!("slow_{i}"), || async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Ok(HealthStatus::Healthy)
                });
            }
            r.add_check("fast", || async { Ok(HealthStatus::Healthy) });
        })
        .await
        .unwrap();

        let start = Instant::now();
        let results = registry.check_all().await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 6);
        // Sequential execution would take >= 5 × 200 ms = 1 s; concurrent
        // execution must finish in roughly one check's duration.
        assert!(
            elapsed < Duration::from_millis(600),
            "check_all took {elapsed:?} — checks are not running concurrently"
        );
        for result in &results {
            assert_eq!(result.status, HealthStatus::Healthy);
        }
    }

    #[tokio::test]
    async fn per_check_timeout_fires_and_captures_message() {
        let registry = HealthRegistry::new().with_default_timeout(Duration::from_millis(100));
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("hangs", || async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(HealthStatus::Healthy)
            });
            r.add_check("quick", || async { Ok(HealthStatus::Healthy) });
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 2);

        let hangs = results.iter().find(|r| r.name == "hangs").unwrap();
        assert_eq!(hangs.status, HealthStatus::Unhealthy);
        assert_eq!(
            hangs.message.as_deref(),
            Some("health check timed out after 100ms")
        );
        // The duration must reflect the deadline, not the check's own runtime.
        assert!(hangs.duration >= Duration::from_millis(100));
        assert!(hangs.duration < Duration::from_secs(2));

        // The healthy check alongside the hanging one is unaffected.
        let quick = results.iter().find(|r| r.name == "quick").unwrap();
        assert_eq!(quick.status, HealthStatus::Healthy);
        assert!(quick.message.is_none());
    }

    #[tokio::test]
    async fn zero_timeout_disables_deadline() {
        let registry = HealthRegistry::new().with_default_timeout(Duration::ZERO);
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("slow_but_ok", || async {
                tokio::time::sleep(Duration::from_millis(150)).await;
                Ok(HealthStatus::Healthy)
            });
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, HealthStatus::Healthy);
        assert!(results[0].message.is_none());
    }

    #[tokio::test]
    async fn failing_check_message_captures_error_detail() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("db", || async { Ok(HealthStatus::Healthy) });
            r.add_check("cache", || async {
                Err(HealthCheckError::DependencyUnavailable(
                    "connection refused".to_string(),
                ))
            });
        })
        .await
        .unwrap();

        let (status, results) = registry.check_readiness().await.unwrap();
        assert_eq!(status, HealthStatus::Unhealthy);
        let cache = results.iter().find(|r| r.name == "cache").unwrap();
        assert_eq!(cache.status, HealthStatus::Unhealthy);
        assert_eq!(
            cache.message.as_deref(),
            Some("dependency unavailable: connection refused")
        );
        // Healthy checks carry no message.
        let db = results.iter().find(|r| r.name == "db").unwrap();
        assert!(db.message.is_none());
    }

    #[tokio::test]
    async fn check_group_runs_only_grouped_checks() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("ungrouped", || async { Ok(HealthStatus::Healthy) });
            r.add_check_to_group("init", "migrations", || async { Ok(HealthStatus::Healthy) });
            r.add_check_to_group("init", "cache_warm", || async {
                Ok(HealthStatus::Degraded)
            });
            r.add_check_to_group("deps", "database", || async { Ok(HealthStatus::Healthy) });
        })
        .await
        .unwrap();

        let init = registry.check_group("init").await;
        assert_eq!(init.len(), 2);
        assert!(init.iter().all(|r| r.name != "ungrouped"));
        assert!(init.iter().all(|r| r.name != "database"));

        // Grouped checks also run in registry-wide probes...
        let all = registry.check_all().await;
        assert_eq!(all.len(), 4);

        // ...and aggregate into check_liveness / check_readiness.
        assert_eq!(
            registry.check_liveness().await.unwrap(),
            HealthStatus::Degraded
        );

        // Unknown groups are empty.
        assert!(registry.check_group("nope").await.is_empty());
    }

    #[tokio::test]
    async fn group_checks_respect_timeout_and_message_capture() {
        let registry = HealthRegistry::new().with_default_timeout(Duration::from_millis(100));
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check_to_group("init", "hangs", || async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok(HealthStatus::Healthy)
            });
        })
        .await
        .unwrap();

        let results = registry.check_group("init").await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, HealthStatus::Unhealthy);
        assert_eq!(
            results[0].message.as_deref(),
            Some("health check timed out after 100ms")
        );
    }

    #[test]
    fn default_timeout_is_five_seconds() {
        assert_eq!(
            HealthRegistry::new().default_timeout(),
            Duration::from_secs(5)
        );
    }

    #[tokio::test]
    async fn registry_new_and_add_check() {
        let registry = HealthRegistry::new();
        let results = registry.check_all().await;
        assert!(results.is_empty());

        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("always_healthy", || async { Ok(HealthStatus::Healthy) });
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "always_healthy");
        assert!(results[0].status.is_healthy());
    }

    #[tokio::test]
    async fn registry_add_multiple_checks() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("ok", || async { Ok(HealthStatus::Healthy) });
            r.add_check("degraded", || async { Ok(HealthStatus::Degraded) });
            r.add_check("failing", || async {
                Err(HealthCheckError::CheckFailed("oops".to_string()))
            });
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn registry_check_liveness_no_checks() {
        let registry = HealthRegistry::new();
        let status = registry.check_liveness().await.unwrap();
        assert!(status.is_healthy());
    }

    #[tokio::test]
    async fn registry_check_readiness_mixed() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("ok", || async { Ok(HealthStatus::Healthy) });
            r.add_check("degraded", || async { Ok(HealthStatus::Degraded) });
        })
        .await
        .unwrap();

        let (status, results) = registry.check_readiness().await.unwrap();
        // Aggregate takes the worst status (Degraded=1 > Healthy=0)
        assert_eq!(status, HealthStatus::Degraded);
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn registry_check_liveness_mixed_reports_worst_status() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("ok", || async { Ok(HealthStatus::Healthy) });
            r.add_check("degraded", || async { Ok(HealthStatus::Degraded) });
            r.add_check("failing", || async {
                Err(HealthCheckError::DependencyUnavailable("db".to_string()))
            });
        })
        .await
        .unwrap();

        // Liveness aggregates to the worst observed status.
        let status = registry.check_liveness().await.unwrap();
        assert_eq!(status, HealthStatus::Unhealthy);
    }

    #[tokio::test]
    async fn registry_check_readiness_failing_reports_unhealthy_with_details() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("ok", || async { Ok(HealthStatus::Healthy) });
            r.add_check("failing", || async {
                Err(HealthCheckError::CheckTimedOut(
                    std::time::Duration::from_secs(2),
                ))
            });
        })
        .await
        .unwrap();

        let (status, results) = registry.check_readiness().await.unwrap();
        assert_eq!(status, HealthStatus::Unhealthy);
        let failing = results.iter().find(|r| r.name == "failing").unwrap();
        assert_eq!(failing.status, HealthStatus::Unhealthy);
        assert_eq!(
            failing.message.as_deref(),
            Some("health check timed out after 2s")
        );
    }

    #[tokio::test]
    async fn registry_default_is_empty_and_healthy() {
        let registry = HealthRegistry::default();
        assert!(registry.check_all().await.is_empty());
        assert_eq!(
            registry.check_liveness().await.unwrap(),
            HealthStatus::Healthy
        );
        let (status, results) = registry.check_readiness().await.unwrap();
        assert_eq!(status, HealthStatus::Healthy);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn registry_check_with_error_returns_unhealthy() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check("failing", || async {
                Err(HealthCheckError::CheckFailed("oops".to_string()))
            });
        })
        .await
        .unwrap();

        let results = registry.check_all().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, HealthStatus::Unhealthy);
        assert_eq!(
            results[0].message.as_deref(),
            Some("health check failed: oops")
        );
    }
}
