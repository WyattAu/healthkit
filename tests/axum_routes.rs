// Route-handler tests drive the real axum services through `oneshot`;
// unwrap/expect and panicking asserts are the test signal here.
#![cfg(feature = "axum")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use healthkit::axum::{detailed_route, liveness_route, readiness_route, startup_route};
use healthkit::{HealthCheckError, HealthRegistry, HealthStatus};
use tower::ServiceExt;

/// What a registered check should report when executed.
#[derive(Clone, Copy)]
enum Outcome {
    Healthy,
    Degraded,
    Failing,
}

/// Build a registry with the given checks. `add_check` uses
/// `RwLock::blocking_write`, so registration must happen off the async
/// runtime thread — same pattern as the unit tests in `lib.rs`.
async fn registry_with(outcomes: Vec<(&'static str, Outcome)>) -> HealthRegistry {
    let registry = HealthRegistry::new();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        for (name, outcome) in outcomes {
            match outcome {
                Outcome::Healthy => {
                    r.add_check(name, || async { Ok(HealthStatus::Healthy) });
                }
                Outcome::Degraded => {
                    r.add_check(name, || async { Ok(HealthStatus::Degraded) });
                }
                Outcome::Failing => {
                    r.add_check(name, || async {
                        Err::<HealthStatus, _>(HealthCheckError::CheckFailed(
                            "dependency down".to_string(),
                        ))
                    });
                }
            }
        }
    })
    .await
    .unwrap();
    registry
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn liveness_route_returns_200_with_healthy_status() {
    let res = liveness_route()
        .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await,
        serde_json::json!({"status": "healthy"})
    );
}

#[tokio::test]
async fn readiness_route_returns_200_when_all_checks_pass() {
    let registry = registry_with(vec![("db", Outcome::Healthy)]).await;
    let res = readiness_route(registry)
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let json = body_json(res).await;
    assert_eq!(json["status"], "healthy");
    let checks = json["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["name"], "db");
    assert_eq!(checks[0]["status"], "healthy");
}

#[tokio::test]
async fn readiness_route_returns_503_when_a_check_fails() {
    let registry = registry_with(vec![("db", Outcome::Healthy), ("cache", Outcome::Failing)]).await;
    let res = readiness_route(registry)
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json = body_json(res).await;
    assert_eq!(json["status"], "unhealthy");
    let checks = json["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2);
    // The failed check reports Unhealthy and carries no message (registry
    // folds the error into a status).
    let cache = checks.iter().find(|c| c["name"] == "cache").unwrap();
    assert_eq!(cache["status"], "unhealthy");
    assert!(cache.get("message").is_none());
    // The passing check still reports Healthy in the same payload.
    let db = checks.iter().find(|c| c["name"] == "db").unwrap();
    assert_eq!(db["status"], "healthy");
}

#[tokio::test]
async fn startup_route_returns_200_when_healthy() {
    let registry = registry_with(vec![("init", Outcome::Healthy)]).await;
    let res = startup_route(registry)
        .oneshot(Request::get("/startupz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        body_json(res).await,
        serde_json::json!({"status": "healthy"})
    );
}

#[tokio::test]
async fn startup_route_returns_503_when_unhealthy() {
    let registry = registry_with(vec![("init", Outcome::Failing)]).await;
    let res = startup_route(registry)
        .oneshot(Request::get("/startupz").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body_json(res).await,
        serde_json::json!({"status": "unhealthy"})
    );
}

#[tokio::test]
async fn detailed_route_returns_200_with_check_details_when_ready() {
    let registry = registry_with(vec![("db", Outcome::Healthy), ("queue", Outcome::Degraded)]).await;
    let res = detailed_route(registry)
        .oneshot(
            Request::get("/healthz/detailed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let json = body_json(res).await;
    // Degraded still counts as ready, so the route stays 200 — but the
    // aggregated status honestly reports the worst check outcome.
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["checks"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn detailed_route_returns_503_when_not_ready() {
    let registry = registry_with(vec![("db", Outcome::Failing)]).await;
    let res = detailed_route(registry)
        .oneshot(
            Request::get("/healthz/detailed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_json(res).await["status"], "unhealthy");
}
