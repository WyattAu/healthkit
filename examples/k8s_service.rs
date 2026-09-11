//! A complete Kubernetes-ready service wiring up every `healthkit` feature.
//!
//! Endpoints exposed:
//!
//! | Route       | Probe                | Notes                                        |
//! |-------------|----------------------|----------------------------------------------|
//! | `/healthz`  | `livenessProbe`      | Always healthy while the server runs         |
//! | `/ping`     | Load balancer check  | Constant `OK` body, ALB/GCP-friendly         |
//! | `/readyz`   | `readinessProbe`     | sqlx + redis + drain checks; 503 when failing|
//! | `/startupz` | `startupProbe`       | Runs only the `init` check group             |
//! | `/metrics`  | Prometheus scrape    | Text exposition 0.0.4, cached for 5 s        |
//! | `/healthz/detailed` | Debugging    | Per-check JSON with statuses and durations   |
//!
//! Configuration (all optional):
//!
//! - `BIND_ADDR`     — listen address (default `0.0.0.0:3000`)
//! - `DATABASE_URL`  — sqlx connection string (default `sqlite::memory:`)
//! - `REDIS_URL`     — when set, a `RedisCheck` probe is registered
//!
//! Run locally:
//!
//! ```text
//! cargo run --example k8s_service --all-features
//! curl -s localhost:3000/readyz | jq
//! curl -s localhost:3000/metrics
//! ```
//!
//! See the "Kubernetes Deployment Guide" section of the README for the
//! matching probe manifest and shutdown design.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use axum::Router;
use healthkit::HealthRegistry;
use healthkit::axum::{
    ReadinessConfig, detailed_route_with, liveness_route, metrics_route_with_cache, ping_route,
    readiness_route_with, startup_route_for_group,
};
use healthkit::{HealthCheckError, HealthStatus, RedisCheck, SqlxCheck};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
    let redis_url = std::env::var("REDIS_URL").ok();

    let pool = sqlx::sqlite::SqlitePool::connect(&database_url).await?;

    // `HealthRegistry::add_check` takes its lock in a blocking fashion, so
    // registration must happen off the async runtime. All checks live in
    // one registry; per-route subsets come from groups:
    // - every check runs on readiness (a redis outage should stop traffic,
    //   not restart the pod),
    // - only the `init` group gates the startup probe,
    // - the `draining` check flips readiness to 503 on SIGTERM so
    //   Kubernetes stops routing to this pod while connections drain.
    let draining = Arc::new(AtomicBool::new(false));
    let initialized = Arc::new(AtomicBool::new(false));

    let registry = {
        let draining = draining.clone();
        let initialized = initialized.clone();
        tokio::task::spawn_blocking(move || {
            let registry = HealthRegistry::new()
                // One slow dependency can no longer stall the other probes:
                // checks run concurrently and each is bounded by this
                // deadline (kubelet timeoutSeconds must stay above it).
                .with_default_timeout(Duration::from_secs(2));
            SqlxCheck::new(pool, Duration::from_secs(2), 500).register(&registry, "database");
            if let Some(url) = redis_url {
                RedisCheck::new(url, Duration::from_secs(2), 500).register(&registry, "cache");
            }
            // Startup subset: only "is initialization done?" — dependency
            // checks must not gate the startup probe.
            registry.add_check_to_group("init", "initialized", move || {
                let initialized = initialized.clone();
                async move {
                    if initialized.load(Ordering::SeqCst) {
                        Ok(HealthStatus::Healthy)
                    } else {
                        Err(HealthCheckError::DependencyUnavailable(
                            "initialization".into(),
                        ))
                    }
                }
            });
            registry.add_check("draining", move || {
                let draining = draining.clone();
                async move {
                    if draining.load(Ordering::SeqCst) {
                        Err(HealthCheckError::ShuttingDown)
                    } else {
                        Ok(HealthStatus::Healthy)
                    }
                }
            });
            registry
        })
        .await?
    };

    let app = Router::new()
        .route("/healthz", liveness_route())
        .route("/ping", ping_route())
        .merge(readiness_route_with(
            registry.clone(),
            ReadinessConfig::default(),
        ))
        .merge(startup_route_for_group(registry.clone(), "init"))
        .merge(detailed_route_with(
            registry.clone(),
            ReadinessConfig::default(),
        ))
        .merge(metrics_route_with_cache(registry, Duration::from_secs(5)));

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    println!("healthkit example listening on {bind_addr}");

    // Initialization is complete: flip the startup probe to healthy.
    initialized.store(true, Ordering::SeqCst);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(draining))
        .await?;

    println!("shutdown complete");
    Ok(())
}

/// Resolves on SIGINT or SIGTERM (the signal Kubernetes sends), then marks
/// the pod as draining so `/readyz` starts returning 503 while axum drains
/// in-flight connections.
async fn shutdown_signal(draining: Arc<AtomicBool>) {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            eprintln!("failed to listen for ctrl-c");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(err) => eprintln!("failed to install SIGTERM handler: {err}"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("shutdown signal received; marking pod as draining");
    draining.store(true, Ordering::SeqCst);
}
