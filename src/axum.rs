use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{MethodRouter, get};

use crate::registry::HealthRegistry;
use crate::types::{HealthResponse, HealthStatus, LivenessResponse, ReadinessResponse};

/// Configuration for readiness-style routes (`/readyz`, `/healthz/detailed`).
///
/// By default a `Degraded` aggregate responds `200 OK` (matches 1.1
/// behavior: Kubernetes and load balancers keep routing to a pod whose
/// dependencies are merely slow). Enable
/// [`degraded_fails_readiness`](ReadinessConfig::degraded_fails_readiness)
/// for the strict alternative — degraded pods are drained from the pool
/// while the response body continues to report the degraded detail.
/// `Unhealthy` aggregates respond `503` either way.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReadinessConfig {
    /// When `true`, a `Degraded` aggregate responds `503 Service
    /// Unavailable` instead of `200 OK`.
    ///
    /// Tradeoff: Kubernetes and most load balancers treat any non-2xx probe
    /// as "stop routing here", so a single degraded dependency (say, a slow
    /// cache above its `warn_above_ms` threshold) drains *every* affected
    /// replica of its traffic. Keep the default (`false`) and alert on
    /// `/metrics` or `/healthz/detailed` instead, unless a degraded
    /// dependency genuinely makes your service unable to serve.
    pub degraded_fails_readiness: bool,
}

impl ReadinessConfig {
    /// Create the default configuration (`Degraded` responds `200 OK`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Map `Degraded` aggregates to `503 Service Unavailable` (strict
    /// readiness — degraded pods stop receiving traffic).
    #[must_use]
    pub fn degraded_fails_readiness(mut self) -> Self {
        self.degraded_fails_readiness = true;
        self
    }
}

/// Shared state for Axum health check handlers.
#[derive(Clone)]
struct HealthState {
    registry: HealthRegistry,
    config: ReadinessConfig,
}

/// Returns a route handler for liveness probes (`/healthz`).
///
/// Liveness checks verify the process is running and not deadlocked.
/// Use this for Kubernetes `livenessProbe`.
pub fn liveness_route() -> MethodRouter {
    get(liveness_handler)
}

/// Returns a constant-body route for load-balancer health checks (`/ping`).
///
/// Responds `200 OK` with the constant body `OK` (`text/plain`) for as long
/// as the process can serve HTTP — the same contract as
/// [`liveness_route`], in the plain-text shape AWS ALB, GCP, and other
/// load balancers expect. HEAD requests are served automatically (axum
/// routes `HEAD` to `GET` handlers and strips the body).
pub fn ping_route() -> MethodRouter {
    get(ping_handler)
}

/// Returns a route handler for readiness probes (`/readyz`).
///
/// Readiness checks verify the service can accept traffic.
/// Use this for Kubernetes `readinessProbe`.
///
/// Uses the default [`ReadinessConfig`] — a `Degraded` aggregate responds
/// `503`. See [`readiness_route_with`] to customize.
pub fn readiness_route(registry: HealthRegistry) -> Router {
    readiness_route_with(registry, ReadinessConfig::default())
}

/// Like [`readiness_route`], with explicit [`ReadinessConfig`].
pub fn readiness_route_with(registry: HealthRegistry, config: ReadinessConfig) -> Router {
    Router::new()
        .route("/readyz", get(readiness_handler))
        .with_state(HealthState { registry, config })
}

/// Returns a route handler for startup probes (`/startupz`).
///
/// Startup checks verify the service has completed initialization.
/// Use this for Kubernetes `startupProbe`. Runs **all** registered checks —
/// for the usual k8s pattern (startup = init-only subset) see
/// [`startup_route_for_group`].
pub fn startup_route(registry: HealthRegistry) -> Router {
    startup_route_for_group_inner(registry, None)
}

/// Returns a route handler for startup probes restricted to a check group
/// (`/startupz`).
///
/// Only checks registered via
/// [`HealthRegistry::add_check_to_group`](crate::HealthRegistry::add_check_to_group)
/// under `group` run for this route. This is the standard Kubernetes
/// startup-probe pattern: `/startupz` answers "is initialization done?"
/// using a small `init` group (migrations, cache warming), while
/// `/readyz` keeps checking every dependency.
///
/// Unregistered/empty groups respond `200` (no checks = nothing failed).
pub fn startup_route_for_group(registry: HealthRegistry, group: impl Into<String>) -> Router {
    startup_route_for_group_inner(registry, Some(group.into()))
}

fn startup_route_for_group_inner(registry: HealthRegistry, group: Option<String>) -> Router {
    Router::new()
        .route("/startupz", get(startup_handler))
        .with_state(StartupState { registry, group })
}

/// Returns a route handler for detailed health status (`/healthz/detailed`).
///
/// Uses the default [`ReadinessConfig`] — see [`detailed_route_with`].
pub fn detailed_route(registry: HealthRegistry) -> Router {
    detailed_route_with(registry, ReadinessConfig::default())
}

/// Like [`detailed_route`], with explicit [`ReadinessConfig`].
pub fn detailed_route_with(registry: HealthRegistry, config: ReadinessConfig) -> Router {
    Router::new()
        .route("/healthz/detailed", get(detailed_handler))
        .with_state(HealthState { registry, config })
}

/// Returns a route handler exposing check results as Prometheus metrics
/// (`/metrics`).
///
/// Available with the `prometheus` feature. Every scrape runs all checks and
/// renders the text exposition format; the response is always `200 OK`.
/// See [`crate::metrics::metrics_route_with_cache`] to serve cached results
/// instead of running checks on every scrape.
#[cfg(feature = "prometheus")]
pub fn metrics_route(registry: HealthRegistry) -> Router {
    Router::new()
        .route("/metrics", get(crate::metrics::metrics_handler))
        .with_state(crate::metrics::MetricsState::new(registry))
}

#[cfg(all(feature = "prometheus", feature = "axum"))]
pub use crate::metrics::metrics_route_with_cache;

/// Shared state for the startup handler, which may be scoped to a group.
#[derive(Clone)]
struct StartupState {
    registry: HealthRegistry,
    group: Option<String>,
}

async fn liveness_handler() -> Response {
    let response = LivenessResponse {
        status: HealthStatus::Healthy,
    };
    (StatusCode::OK, Json(response)).into_response()
}

async fn ping_handler() -> Response {
    (
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        "OK",
    )
        .into_response()
}

/// Decide the HTTP status for a readiness aggregate under `config`.
fn readiness_status_code(status: HealthStatus, config: ReadinessConfig) -> StatusCode {
    let degraded_ok = status == HealthStatus::Degraded && !config.degraded_fails_readiness;
    if status.is_healthy() || degraded_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn readiness_handler(State(state): State<HealthState>) -> Response {
    match state.registry.check_readiness().await {
        Ok((status, checks)) => {
            let response = ReadinessResponse { status, checks };
            (readiness_status_code(status, state.config), Json(response)).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn startup_handler(State(state): State<StartupState>) -> Response {
    let results = match &state.group {
        Some(group) => state.registry.check_group(group).await,
        None => state.registry.check_all().await,
    };
    let status = aggregate_status(&results);
    let response = HealthResponse { status };
    if status.is_healthy() {
        (StatusCode::OK, Json(response)).into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
    }
}

async fn detailed_handler(State(state): State<HealthState>) -> Response {
    match state.registry.check_readiness().await {
        Ok((status, checks)) => {
            let response = ReadinessResponse { status, checks };
            (readiness_status_code(status, state.config), Json(response)).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Aggregate individual check results into the worst observed status.
fn aggregate_status(results: &[crate::types::CheckResult]) -> HealthStatus {
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

// Route-handler tests assert exact statuses and bodies; unwrap/expect and
// panicking asserts are the test signal here.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    async fn registry_with(outcomes: Vec<(&'static str, HealthStatus)>) -> HealthRegistry {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            for (name, status) in outcomes {
                r.add_check(name, move || async move { Ok(status) });
            }
        })
        .await
        .unwrap();
        registry
    }

    #[tokio::test]
    async fn ping_route_returns_constant_ok_body() {
        let res = ping_route()
            .oneshot(
                axum::http::Request::get("/ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"OK");
    }

    #[tokio::test]
    async fn head_requests_fall_back_to_get_handlers() {
        // axum routes HEAD to GET handlers and strips the body; load
        // balancers (ALB, GCP) rely on this for cheap probes.
        let res = liveness_route()
            .oneshot(
                axum::http::Request::builder()
                    .method("HEAD")
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.is_empty(), "HEAD body must be stripped");
    }

    #[tokio::test]
    async fn readiness_degraded_stays_200_by_default() {
        let registry = registry_with(vec![("db", HealthStatus::Degraded)]).await;
        let res = readiness_route(registry)
            .oneshot(
                axum::http::Request::get("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // 1.1 behavior: a degraded pod keeps receiving traffic.
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_degraded_is_503_when_strict() {
        let registry = registry_with(vec![("db", HealthStatus::Degraded)]).await;
        let res = readiness_route_with(registry, ReadinessConfig::new().degraded_fails_readiness())
            .oneshot(
                axum::http::Request::get("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readiness_unhealthy_is_503() {
        let registry = registry_with(vec![("db", HealthStatus::Unhealthy)]).await;
        let res = readiness_route(registry)
            .oneshot(
                axum::http::Request::get("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn startup_route_for_group_runs_only_that_group() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            // Dependency check that would fail startup if it ran here.
            r.add_check("database", || async { Ok(HealthStatus::Unhealthy) });
            r.add_check_to_group("init", "migrations", || async { Ok(HealthStatus::Healthy) });
        })
        .await
        .unwrap();

        let res = startup_route_for_group(registry, "init")
            .oneshot(
                axum::http::Request::get("/startupz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // Only the init group ran — the failing dependency check didn't.
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn startup_route_for_group_still_reports_failures() {
        let registry = HealthRegistry::new();
        let r = registry.clone();
        tokio::task::spawn_blocking(move || {
            r.add_check_to_group("init", "migrations", || async {
                Ok(HealthStatus::Unhealthy)
            });
        })
        .await
        .unwrap();

        let res = startup_route_for_group(registry, "init")
            .oneshot(
                axum::http::Request::get("/startupz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn startup_route_for_unknown_group_is_200() {
        let registry = registry_with(vec![("db", HealthStatus::Unhealthy)]).await;
        let res = startup_route_for_group(registry, "nope")
            .oneshot(
                axum::http::Request::get("/startupz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
