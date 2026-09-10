# healthkit

Health check endpoints for Rust — liveness, readiness, and startup probes with dependency checking for Kubernetes and Docker.

[![docs.rs](https://docs.rs/healthkit/badge.svg)](https://docs.rs/healthkit)
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

## Kubernetes Deployment Guide

### Probe-to-route mapping

| Probe            | Route                | What it should answer                          |
|------------------|----------------------|------------------------------------------------|
| `livenessProbe`  | `GET /healthz`       | "Is the process alive?" — never checks deps    |
| `readinessProbe` | `GET /readyz`        | "Should traffic be routed here?" — checks deps |
| `startupProbe`   | `GET /startupz`      | "Is initialization done?" — gates the others   |
| (scrape)         | `GET /metrics`       | Prometheus text exposition, always `200 OK`    |
| (debugging)      | `GET /healthz/detailed` | Per-check JSON with status and duration     |

`/healthz` deliberately runs **no** checks: a liveness failure restarts the
container, and killing a pod because Redis hiccuped turns a blip into an
outage. Dependency checks belong in `/readyz`. A complete, runnable service
showing all four routes plus graceful shutdown lives in
[`examples/k8s_service.rs`](examples/k8s_service.rs):

```text
cargo run --example k8s_service --all-features
```

### Recommended manifest

```yaml
apiVersion: v1
kind: Pod
spec:
  terminationGracePeriodSeconds: 40
  containers:
  - name: app
    ports:
    - containerPort: 3000
    startupProbe:
      httpGet:
        path: /startupz
        port: 3000
      periodSeconds: 2
      failureThreshold: 30      # allows up to 60 s for migrations/init
    livenessProbe:
      httpGet:
        path: /healthz
        port: 3000
      periodSeconds: 10
      timeoutSeconds: 2
      failureThreshold: 3
    readinessProbe:
      httpGet:
        path: /readyz
        port: 3000
      periodSeconds: 5
      timeoutSeconds: 3         # must exceed your check timeouts (below)
      failureThreshold: 3
    lifecycle:
      preStop:
        exec:
          command: ["sleep", "5"]
```

### Aligning probe timeouts with check timeouts

Every timeout in the chain must leave headroom for the one below it:

```text
kubelet timeoutSeconds  >  check timeout  >  healthy probe latency
```

If you register `SqlxCheck::new(pool, Duration::from_secs(2), 500)`, a
readiness probe with the Kubernetes default `timeoutSeconds: 1` will report
failures whenever the database round-trip exceeds 1 s — flapping your pod out
of `Service` endpoints even though the check itself would have returned
`Healthy` at 1.2 s. Set `timeoutSeconds` comfortably above the check timeout
(3 s for a 2 s check), and `periodSeconds` large enough that a probe never
overlaps the previous one.

The `warn_above_ms` threshold is your *degradation* signal: probes still
succeed (HTTP 200) but `/metrics` reports `healthkit_check_healthiness = 0.5`
and `/healthz/detailed` shows the elevated duration. Alert on the metric;
don't fail the probe.

### Startup probes

Until the startup probe first succeeds, Kubernetes disables liveness and
readiness — use it to cover slow initialization (schema migrations, cache
warming) without huge `initialDelaySeconds` on the other probes. Budget
`failureThreshold × periodSeconds` above your worst-case startup time. Keep
the startup registry free of dependency checks: if Redis being down at boot
caused `/startupz` to fail, the pod would restart-loop instead of waiting
`NotReady` for the dependency to recover.

### Graceful shutdown

On termination Kubernetes sends `SIGTERM` and removes the pod from Service
endpoints — **concurrently, not sequentially**. Route traffic away before
connections start closing:

1. Register a *draining* check that turns `Unhealthy` when a shutdown signal
   has been observed, so `/readyz` starts returning `503` immediately.
2. Handle `SIGTERM` with `axum::serve(...).with_graceful_shutdown(...)` and
   let in-flight requests finish.
3. Add a `preStop: sleep 5` hook as belt-and-braces for endpoint-propagation
   delay, and size `terminationGracePeriodSeconds` above your longest request.

```yaml
lifecycle:
  preStop:
    exec:
      command: ["sleep", "5"]
```

The example implements steps 1–2: the `draining` check flips on SIGTERM/SIGINT
receipt, readiness returns 503 during the drain window, and the process exits
only after axum's graceful shutdown completes.

## Comparison

See [COMPARISON.md](COMPARISON.md) for how `healthkit` compares with
`kube-health-check` and the `health` crate.

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
