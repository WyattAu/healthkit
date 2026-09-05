#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! # healthkit
//!
//! Health check endpoints for Rust services — liveness, readiness, and startup
//! probes with dependency checking for Kubernetes and Docker.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use healthkit::{HealthRegistry, HealthStatus};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let registry = HealthRegistry::new();
//!
//! // Register a custom check
//! registry.add_check("database", || async {
//!     // Check database connectivity
//!     Ok(HealthStatus::Healthy)
//! });
//!
//! // Run all checks
//! let results = registry.check_all().await;
//! # }
//! ```
//!
//! ## Axum Integration
//!
//! Enable the default `axum` feature for ready-to-use route handlers:
//!
//! ```rust,no_run
//! use axum::Router;
//! use healthkit::{HealthRegistry, axum::{liveness_route, readiness_route, startup_route}};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let mut registry = HealthRegistry::new();
//!
//! let app = Router::new()
//!     .route("/healthz", liveness_route())
//!     .merge(readiness_route(registry.clone()))
//!     .merge(startup_route(registry.clone()));
//! # }
//! ```

mod error;
mod types;

/// Axum integration for health check endpoints.
#[cfg(feature = "axum")]
pub mod axum;

mod registry;

pub use error::HealthCheckError;
pub use registry::HealthRegistry;
pub use types::{CheckResult, HealthResponse, HealthStatus, LivenessResponse, ReadinessResponse};

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

    #[test]
    fn health_status_is_healthy() {
        assert!(HealthStatus::Healthy.is_healthy());
        assert!(!HealthStatus::Degraded.is_healthy());
        assert!(!HealthStatus::Unhealthy.is_healthy());
    }

    #[test]
    fn health_status_is_ready() {
        assert!(HealthStatus::Healthy.is_ready());
        assert!(HealthStatus::Degraded.is_ready());
        assert!(!HealthStatus::Unhealthy.is_ready());
    }

    #[test]
    fn health_status_display() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
        assert_eq!(HealthStatus::Unhealthy.to_string(), "unhealthy");
    }

    #[test]
    fn check_result_creation_and_is_healthy() {
        let result = CheckResult {
            name: "db".to_string(),
            status: HealthStatus::Healthy,
            message: None,
            duration: std::time::Duration::from_millis(5),
        };
        assert!(result.status.is_healthy());
        assert_eq!(result.name, "db");
        assert!(result.message.is_none());
    }

    #[test]
    fn check_result_unhealthy() {
        let result = CheckResult {
            name: "redis".to_string(),
            status: HealthStatus::Unhealthy,
            message: Some("connection refused".to_string()),
            duration: std::time::Duration::from_millis(1),
        };
        assert!(!result.status.is_healthy());
        assert_eq!(result.message.as_deref(), Some("connection refused"));
    }

    #[test]
    fn health_check_error_display() {
        let err = HealthCheckError::CheckFailed("disk full".to_string());
        assert_eq!(err.to_string(), "health check failed: disk full");

        let err = HealthCheckError::CheckPanicked("thread panic".to_string());
        assert_eq!(err.to_string(), "health check panicked: thread panic");

        let err = HealthCheckError::CheckTimedOut(std::time::Duration::from_secs(5));
        assert_eq!(err.to_string(), "health check timed out after 5s");

        let err = HealthCheckError::DependencyUnavailable("postgres".to_string());
        assert_eq!(err.to_string(), "dependency unavailable: postgres");

        let err = HealthCheckError::ShuttingDown;
        assert_eq!(err.to_string(), "service is shutting down");
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
                Err(HealthCheckError::CheckTimedOut(std::time::Duration::from_secs(2)))
            });
        })
        .await
        .unwrap();

        let (status, results) = registry.check_readiness().await.unwrap();
        assert_eq!(status, HealthStatus::Unhealthy);
        let failing = results.iter().find(|r| r.name == "failing").unwrap();
        assert_eq!(failing.status, HealthStatus::Unhealthy);
        assert!(failing.message.is_none());
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
    }
}
