// Prometheus route-handler tests drive the real axum service through
// `oneshot`; unwrap/expect and panicking asserts are the test signal here.
#![cfg(all(feature = "prometheus", feature = "axum"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use healthkit::axum::metrics_route;
use healthkit::{HealthCheckError, HealthRegistry, HealthStatus};
use tower::ServiceExt;

async fn registry_with(outcomes: Vec<(&'static str, bool)>) -> HealthRegistry {
    let registry = HealthRegistry::new();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        for (name, healthy) in outcomes {
            if healthy {
                r.add_check(name, || async { Ok(HealthStatus::Healthy) });
            } else {
                r.add_check(name, || async {
                    Err::<HealthStatus, _>(HealthCheckError::CheckFailed("down".to_string()))
                });
            }
        }
    })
    .await
    .unwrap();
    registry
}

#[tokio::test]
async fn metrics_route_returns_200_with_text_exposition() {
    let registry = registry_with(vec![("db", true)]).await;
    let res = metrics_route(registry)
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers()["content-type"],
        "text/plain; version=0.0.4; charset=utf-8"
    );

    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(body.contains("# TYPE healthkit_check_healthiness gauge\n"));
    assert!(body.contains("healthkit_check_healthiness{check=\"db\"} 1\n"));
    assert!(body.contains("# TYPE healthkit_check_duration_seconds histogram\n"));
    assert!(body.contains("healthkit_check_duration_seconds_count{check=\"db\"} 1\n"));
}

#[tokio::test]
async fn metrics_route_reports_unhealthy_checks_with_200() {
    // A failing check must not fail the scrape — otherwise monitoring goes
    // blind exactly when it matters.
    let registry = registry_with(vec![("db", true), ("cache", false)]).await;
    let res = metrics_route(registry)
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(body.contains("healthkit_check_healthiness{check=\"db\"} 1\n"));
    assert!(body.contains("healthkit_check_healthiness{check=\"cache\"} 0\n"));
}

#[tokio::test]
async fn metrics_route_with_empty_registry_renders_headers_only() {
    let res = metrics_route(HealthRegistry::new())
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();

    assert_eq!(
        body,
        "# HELP healthkit_check_healthiness Health status of the check \
         (1 = healthy, 0.5 = degraded, 0 = unhealthy).\n\
         # TYPE healthkit_check_healthiness gauge\n\
         # HELP healthkit_check_duration_seconds Duration of the check \
         execution in seconds.\n\
         # TYPE healthkit_check_duration_seconds histogram\n"
    );
}
