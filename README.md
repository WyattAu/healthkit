# healthkit

Health check endpoints for Rust — liveness, readiness, and startup probes with dependency checking for Kubernetes and Docker.

[![Crates.io](https://img.shields.io/crates/v/healthkit.svg)](https://crates.io/crates/healthkit)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](./LICENSE-MIT)

## Purpose

`healthkit` provides a simple, composable API for registering and executing health checks in Rust services. It integrates with Kubernetes probes and includes an optional Axum integration for HTTP endpoints.

## Features

- **Liveness probes** — verify the process is running and not deadlocked
- **Readiness probes** — verify the service can accept traffic
- **Startup probes** — verify initialization is complete
- **Dependency checking** — register custom checks for databases, caches, etc.
- **Axum integration** — ready-to-use route handlers (default feature)
- **Prometheus metrics** — text-exposition rendering of check results, hand-rolled (no `prometheus` crate dependency)
- **Production checkers** — `sqlx` and `redis` probes with timeout and latency-degradation thresholds
- **No unsafe code** — `#![forbid(unsafe_code)]`

## Feature Flags

Default: `axum` only. Every other feature is opt-in and compiles independently.

| Feature | Adds |
|---|---|
| `axum` | `healthkit::axum` route handlers |
| `prometheus` | `render_prometheus`, and (with `axum`) a `/metrics` handler + `metrics_route` |
| `sqlx` | `SqlxCheck` — `SELECT 1` probe for any `sqlx::Pool` |
| `redis` | `RedisCheck` — `PING` probe (no async runtime features pulled) |

### sqlx check

Requires `sqlx` 0.8. The check is generic over the database backend — enable
the driver feature (`sqlite`, `postgres`, `mysql`, ...) on *your* `sqlx`
dependency; `healthkit` itself only pulls `sqlite` for its own tests.

```rust
use std::time::Duration;
use healthkit::{HealthRegistry, SqlxCheck};

let registry = HealthRegistry::new();
SqlxCheck::new(pg_pool, Duration::from_secs(2), 500)
    .register(&registry, "database");
// check_all() now probes with SELECT 1:
// - success under 500 ms  → Healthy
// - success over 500 ms   → Degraded (warn_above_ms)
// - error / timeout (2 s) → Unhealthy
```

### redis check

Requires `redis` 0.32 (`default-features = false` — the check uses a
synchronous connection on a blocking thread, so no async runtime features
are pulled).

```rust
use std::time::Duration;
use healthkit::{HealthRegistry, RedisCheck};

let registry = HealthRegistry::new();
RedisCheck::new("redis://127.0.0.1:6379", Duration::from_secs(2), 500)
    .register(&registry, "cache");
// healthy = PING replies PONG within the timeout and under warn_above_ms
```

### Prometheus metrics

Enable `prometheus` (plus `axum` for the HTTP handler). The text format
(version 0.0.4) is hand-rolled to avoid version coupling with the
`prometheus` crate. Each check produces:

- `healthkit_check_healthiness{check="db"}` — gauge: `1` healthy, `0.5` degraded, `0` unhealthy
- `healthkit_check_duration_seconds{check="db"}` — histogram of probe latency

```rust
use axum::Router;
use healthkit::axum::{liveness_route, metrics_route, readiness_route};

let app = Router::new()
    .route("/healthz", liveness_route())
    .merge(readiness_route(registry.clone()))
    .merge(metrics_route(registry.clone())); // GET /metrics, always 200
```

Prefer a custom pipeline? `healthkit::render_prometheus(&results)` renders
the same exposition for any `Vec<CheckResult>`.

## Kubernetes Probe Configuration

```yaml
apiVersion: v1
kind: Pod
spec:
  containers:
  - name: app
    livenessProbe:
      httpGet:
        path: /healthz
        port: 3000
      initialDelaySeconds: 5
      periodSeconds: 10
    readinessProbe:
      httpGet:
        path: /readyz
        port: 3000
      initialDelaySeconds: 5
      periodSeconds: 5
    startupProbe:
      httpGet:
        path: /startupz
        port: 3000
      failureThreshold: 30
      periodSeconds: 2
```

## Usage

```rust
use axum::Router;
use healthkit::{HealthRegistry, HealthStatus};
use healthkit::axum::{liveness_route, readiness_route, startup_route};

#[tokio::main]
async fn main() {
    let mut registry = HealthRegistry::new();

    // Register a database check
    registry.add_check("database", || async {
        // Perform actual database connectivity check
        HealthStatus::Healthy
    });

    // Register a cache check
    registry.add_check("cache", || async {
        HealthStatus::Healthy
    });

    let app = Router::new()
        .route("/healthz", liveness_route())
        .merge(readiness_route(registry.clone()))
        .merge(startup_route(registry));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

## Security

Threat model: [THREAT-MODEL.md](THREAT-MODEL.md).
