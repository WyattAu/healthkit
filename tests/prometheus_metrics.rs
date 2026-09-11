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

#[tokio::test]
async fn cached_metrics_route_does_not_execute_checks_within_ttl() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    let ttl = Duration::from_millis(30);
    let executions = std::sync::Arc::new(AtomicUsize::new(0));
    let registry = HealthRegistry::new();
    let counter = executions.clone();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        r.add_check("db", move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, HealthCheckError>(HealthStatus::Healthy)
            }
        });
    })
    .await
    .unwrap();

    let app = healthkit::axum::metrics_route_with_cache(registry, ttl);

    // First scrape executes the check and populates the cache.
    let res = app
        .clone()
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(executions.load(Ordering::SeqCst), 1);

    // Second scrape within the TTL is served from cache — no executions.
    let res = app
        .clone()
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("healthkit_check_healthiness{check=\"db\"} 1\n"));
    assert_eq!(
        executions.load(Ordering::SeqCst),
        1,
        "scrape within TTL must not execute checks"
    );

    // After the TTL lapses the next scrape executes checks again.
    tokio::time::sleep(ttl * 3).await;
    let res = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        executions.load(Ordering::SeqCst) >= 2,
        "scrape after TTL must execute checks again"
    );
}

#[tokio::test]
async fn uncached_metrics_route_executes_checks_on_every_scrape() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let executions = std::sync::Arc::new(AtomicUsize::new(0));
    let registry = HealthRegistry::new();
    let counter = executions.clone();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        r.add_check("db", move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<_, HealthCheckError>(HealthStatus::Healthy)
            }
        });
    })
    .await
    .unwrap();

    let app = metrics_route(registry);
    for _ in 0..3 {
        let res = app
            .clone()
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
    // Compat: without the cache every scrape runs the checks.
    assert_eq!(executions.load(Ordering::SeqCst), 3);
}
