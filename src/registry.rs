use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;

use crate::error::HealthCheckError;
use crate::types::{CheckResult, HealthStatus};

type CheckFn = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<HealthStatus, HealthCheckError>> + Send>>
        + Send
        + Sync,
>;

/// Registry of health checks that can be executed on demand.
#[derive(Clone)]
pub struct HealthRegistry {
    checks: Arc<RwLock<Vec<(String, CheckFn)>>>,
}

impl HealthRegistry {
    /// Create a new empty health registry.
    pub fn new() -> Self {
        Self {
            checks: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Register a health check with a given name and async check function.
    pub fn add_check<F, Fut>(&self, name: impl Into<String>, check: F)
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<HealthStatus, HealthCheckError>> + Send + 'static,
    {
        let check_fn: CheckFn = Box::new(move || Box::pin(check()));
        let mut checks = self.checks.blocking_write();
        checks.push((name.into(), check_fn));
    }

    /// Run all registered health checks and return the results.
    pub async fn check_all(&self) -> Vec<CheckResult> {
        let checks = self.checks.read().await;
        let mut results = Vec::with_capacity(checks.len());

        for (name, check_fn) in checks.iter() {
            let start = Instant::now();
            let status = check_fn().await.unwrap_or(HealthStatus::Unhealthy);
            let duration = start.elapsed();

            results.push(CheckResult {
                name: name.clone(),
                status,
                message: None,
                duration,
            });
        }

        results
    }

    /// Check liveness — returns `Healthy` if all checks pass, `Unhealthy` otherwise.
    pub async fn check_liveness(&self) -> Result<HealthStatus, HealthCheckError> {
        let results = self.check_all().await;
        let overall = results
            .iter()
            .map(|r| r.status)
            .min_by_key(|s| match s {
                HealthStatus::Healthy => 0,
                HealthStatus::Degraded => 1,
                HealthStatus::Unhealthy => 2,
            })
            .unwrap_or(HealthStatus::Healthy);

        Ok(overall)
    }

    /// Check readiness — returns `Healthy` if all checks pass, `Unhealthy` if any fail.
    pub async fn check_readiness(&self) -> Result<(HealthStatus, Vec<CheckResult>), HealthCheckError>
    {
        let results = self.check_all().await;
        let overall = results
            .iter()
            .map(|r| r.status)
            .min_by_key(|s| match s {
                HealthStatus::Healthy => 0,
                HealthStatus::Degraded => 1,
                HealthStatus::Unhealthy => 2,
            })
            .unwrap_or(HealthStatus::Healthy);

        Ok((overall, results))
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}
