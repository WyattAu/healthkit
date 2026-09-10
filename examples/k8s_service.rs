//! A complete Kubernetes-ready service wiring up every `healthkit` feature.
//!
//! Endpoints exposed:
//!
//! | Route       | Probe                | Notes                                        |
//! |-------------|----------------------|----------------------------------------------|
//! | `/healthz`  | `livenessProbe`      | Always healthy while the server runs         |
//! | `/readyz`   | `readinessProbe`     | sqlx + redis + drain checks; 503 when failing|
//! | `/startupz` | `startupProbe`       | Gated by a dedicated `initialized` check     |
//! | `/metrics`  | Prometheus scrape    | Text exposition 0.0.4, always `200 OK`       |
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
    detailed_route, liveness_route, metrics_route, readiness_route, startup_route,
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
    // registration must happen off the async runtime. Probe dependency
    // checks (database, cache) belong in the *readiness* registry: if redis
    // is down the pod should stop receiving traffic, not be restarted.
    let draining = Arc::new(AtomicBool::new(false));
    let initialized = Arc::new(AtomicBool::new(false));

    let readiness_registry = {
        let draining = draining.clone();
        tokio::task::spawn_blocking(move || {
            let registry = HealthRegistry::new();
            SqlxCheck::new(pool, Duration::from_secs(2), 500).register(&registry, "database");
            if let Some(url) = redis_url {
                RedisCheck::new(url, Duration::from_secs(2), 500).register(&registry, "cache");
            }
            // Flips unhealthy while shutting down so Kubernetes and any
            // load balancers stop routing to this pod during drain.
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

    // The startup registry only answers "is this process ready to serve?".
    // It must not depend on downstream services, or a redis outage would
    // keep the pod in `CrashLoopBackOff` instead of merely `NotReady`.
    let startup_registry = {
        let initialized = initialized.clone();
        tokio::task::spawn_blocking(move || {
            let registry = HealthRegistry::new();
            registry.add_check("initialized", move || {
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
            registry
        })
        .await?
    };

    let app = Router::new()
        .route("/healthz", liveness_route())
        .merge(readiness_route(readiness_registry.clone()))
        .merge(startup_route(startup_registry))
        .merge(detailed_route(readiness_registry.clone()))
        .merge(metrics_route(readiness_registry));

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
