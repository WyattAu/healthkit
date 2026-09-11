#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]
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
//! // Run all checks (concurrently — one slow dependency cannot stall the
//! // others; each check is bounded by a per-check timeout, 5 s by default)
//! let results = registry.check_all().await;
//! # }
//! ```
//!
//! ## Check groups (per-route subsets)
//!
//! Checks registered with [`HealthRegistry::add_check`] run on every probe.
//! Checks registered into a named group additionally run when that group is
//! probed — the standard Kubernetes startup pattern probes only an `init`
//! group while readiness keeps checking everything:
//!
//! ```rust,no_run
//! # #[cfg(feature = "axum")]
//! # mod groups_example {
//! use healthkit::{HealthRegistry, HealthStatus, axum::startup_route_for_group};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let registry = HealthRegistry::new();
//! registry.add_check("database", || async { Ok(HealthStatus::Healthy) });
//! registry.add_check_to_group("init", "migrations", || async {
//!     Ok(HealthStatus::Healthy)
//! });
//!
//! // /startupz only answers "is initialization done?"
//! let startup = startup_route_for_group(registry.clone(), "init");
//! # }
//! # }
//! ```
//!
//! ## Axum Integration
//!
//! Enable the default `axum` feature for ready-to-use route handlers:
//!
//! ```rust,no_run
//! # #[cfg(feature = "axum")]
//! # mod axum_example {
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
//! # }
//! ```
//!
//! ## Optional Features
//!
//! - `axum` (default) — ready-to-use route handlers.
//! - `prometheus` — render check results as Prometheus text exposition
//!   ([`metrics::render_prometheus`]) plus a `/metrics` handler and route
//!   (with `axum`), optionally served from a TTL cache.
//! - `metrics` — emit `healthkit_check_result` counters and
//!   `healthkit_check_duration_seconds` histograms through the `metrics`
//!   facade crate, alongside (not replacing) the hand-rolled renderer.
//! - `sqlx` — [`checks::sqlx::SqlxCheck`], a `SELECT 1` probe for a
//!   `sqlx::Pool` with timeout and latency-degradation thresholds.
//! - `redis` — [`checks::redis::RedisCheck`], a `PING` probe with the same
//!   semantics.

mod error;
mod types;

/// Axum integration for health check endpoints.
#[cfg(feature = "axum")]
#[cfg_attr(docsrs, doc(cfg(feature = "axum")))]
pub mod axum;

/// Production dependency checks for common backends.
#[cfg(any(feature = "redis", feature = "sqlx"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "redis", feature = "sqlx"))))]
pub mod checks;

/// Prometheus text-exposition rendering of check results.
#[cfg(feature = "prometheus")]
#[cfg_attr(docsrs, doc(cfg(feature = "prometheus")))]
pub mod metrics;

mod registry;

#[cfg(feature = "redis")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis")))]
pub use checks::redis::RedisCheck;
#[cfg(feature = "sqlx")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlx")))]
pub use checks::sqlx::SqlxCheck;
pub use error::HealthCheckError;
pub use registry::HealthRegistry;
pub use types::{CheckResult, HealthResponse, HealthStatus, LivenessResponse, ReadinessResponse};

#[cfg(feature = "prometheus")]
#[cfg_attr(docsrs, doc(cfg(feature = "prometheus")))]
pub use metrics::render_prometheus;

#[cfg(all(feature = "prometheus", feature = "axum"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "prometheus", feature = "axum"))))]
pub use metrics::{MetricsState, metrics_handler};

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
}
