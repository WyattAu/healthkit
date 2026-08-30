use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, MethodRouter};
use axum::Router;

use crate::registry::HealthRegistry;
use crate::types::{HealthResponse, LivenessResponse, ReadinessResponse};

/// Shared state for Axum health check handlers.
#[derive(Clone)]
struct HealthState {
    registry: HealthRegistry,
}

/// Returns a route handler for liveness probes (`/healthz`).
///
/// Liveness checks verify the process is running and not deadlocked.
/// Use this for Kubernetes `livenessProbe`.
pub fn liveness_route() -> MethodRouter {
    get(liveness_handler)
}

/// Returns a route handler for readiness probes (`/readyz`).
///
/// Readiness checks verify the service can accept traffic.
/// Use this for Kubernetes `readinessProbe`.
pub fn readiness_route(registry: HealthRegistry) -> Router {
    Router::new()
        .route("/readyz", get(readiness_handler))
        .with_state(HealthState { registry })
}

/// Returns a route handler for startup probes (`/startupz`).
///
/// Startup checks verify the service has completed initialization.
/// Use this for Kubernetes `startupProbe`.
pub fn startup_route(registry: HealthRegistry) -> Router {
    Router::new()
        .route("/startupz", get(startup_handler))
        .with_state(HealthState { registry })
}

/// Returns a route handler for detailed health status (`/healthz/detailed`).
pub fn detailed_route(registry: HealthRegistry) -> Router {
    Router::new()
        .route("/healthz/detailed", get(detailed_handler))
        .with_state(HealthState { registry })
}

async fn liveness_handler() -> Response {
    let response = LivenessResponse {
        status: crate::types::HealthStatus::Healthy,
    };
    (StatusCode::OK, Json(response)).into_response()
}

async fn readiness_handler(State(state): State<HealthState>) -> Response {
    match state.registry.check_readiness().await {
        Ok((status, checks)) => {
            let response = ReadinessResponse { status, checks };
            if status.is_ready() {
                (StatusCode::OK, Json(response)).into_response()
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn startup_handler(State(state): State<HealthState>) -> Response {
    match state.registry.check_liveness().await {
        Ok(status) => {
            let response = HealthResponse { status };
            if status.is_healthy() {
                (StatusCode::OK, Json(response)).into_response()
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn detailed_handler(State(state): State<HealthState>) -> Response {
    match state.registry.check_readiness().await {
        Ok((status, checks)) => {
            let response = ReadinessResponse { status, checks };
            if status.is_ready() {
                (StatusCode::OK, Json(response)).into_response()
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, Json(response)).into_response()
            }
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
