#![forbid(unsafe_code)]

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
//! let mut registry = HealthRegistry::new();
//!
//! // Register a custom check
//! registry.add_check("database", || async {
//!     // Check database connectivity
//!     HealthStatus::Healthy
//! });
//!
//! // Run all checks
//! let results = registry.check_all().await;
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
//!     .route("/readyz", readiness_route(registry.clone()))
//!     .route("/startup", startup_route(registry.clone()));
//! # }
//! ```

mod error;
mod types;

#[cfg(feature = "axum")]
pub mod axum;

mod registry;

pub use error::HealthCheckError;
pub use registry::HealthRegistry;
pub use types::{CheckResult, HealthResponse, HealthStatus, LivenessResponse, ReadinessResponse};
