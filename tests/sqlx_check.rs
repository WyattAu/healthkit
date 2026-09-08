// Integration tests drive SqlxCheck against a real in-memory SQLite
// database through the public API; unwrap/expect and panicking asserts are
// the test signal here.
#![cfg(feature = "sqlx")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use healthkit::{HealthRegistry, HealthStatus, SqlxCheck};
use sqlx::sqlite::SqlitePoolOptions;
use std::time::Duration;

async fn memory_pool() -> sqlx::SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

#[tokio::test]
async fn registered_check_flows_through_readiness() {
    let registry = HealthRegistry::new();
    let r = registry.clone();
    let pool = memory_pool().await;
    tokio::task::spawn_blocking(move || {
        SqlxCheck::new(pool, Duration::from_secs(2), 500).register(&r, "database");
    })
    .await
    .unwrap();

    let (status, results) = registry.check_readiness().await.unwrap();
    assert_eq!(status, HealthStatus::Healthy);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "database");
    assert_eq!(results[0].status, HealthStatus::Healthy);
}

#[tokio::test]
async fn failed_probe_folds_into_unhealthy_readiness() {
    let registry = HealthRegistry::new();
    let r = registry.clone();
    let pool = memory_pool().await;
    pool.close().await;
    tokio::task::spawn_blocking(move || {
        SqlxCheck::new(pool, Duration::from_secs(2), 500).register(&r, "database");
    })
    .await
    .unwrap();

    let (status, results) = registry.check_readiness().await.unwrap();
    assert_eq!(status, HealthStatus::Unhealthy);
    assert_eq!(results[0].status, HealthStatus::Unhealthy);
}

#[tokio::test]
async fn probe_over_threshold_reports_degraded_readiness() {
    let registry = HealthRegistry::new();
    let r = registry.clone();
    let pool = memory_pool().await;
    tokio::task::spawn_blocking(move || {
        SqlxCheck::new(pool, Duration::from_secs(2), 0).register(&r, "database");
    })
    .await
    .unwrap();

    let (status, results) = registry.check_readiness().await.unwrap();
    assert_eq!(status, HealthStatus::Degraded);
    assert_eq!(results[0].status, HealthStatus::Degraded);
}
