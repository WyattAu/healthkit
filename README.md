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
- **Concurrent execution** — all checks run in parallel, each bounded by a per-check timeout (default 5 s), so one hanging dependency can never stall a probe
- **Check groups** — per-route check subsets; the startup probe answers "is init done?" without running every dependency
- **Error detail** — failing checks carry the error message in the result payload
- **Axum integration** — ready-to-use route handlers (default feature)
- **Prometheus metrics** — text-exposition rendering of check results, hand-rolled (no `prometheus` crate dependency), with an optional TTL cache so scrapes don't execute checks every time
- **`metrics` facade** — optional counters + histograms through the de-facto `metrics` crate
- **Load-balancer friendly** — constant-body `/ping`, HEAD support on every route, documented ALB/GCP recipes
- **Production checkers** — `sqlx` and `redis` probes with timeout and latency-degradation thresholds
- **No unsafe code** — `#![forbid(unsafe_code)]`

## Feature Flags

Default: `axum` only. Every other feature is opt-in and compiles independently.

| Feature | Adds |
|---|---|
| `axum` | `healthkit::axum` route handlers |
| `prometheus` | `render_prometheus`, and (with `axum`) a `/metrics` handler + `metrics_route` |
| `metrics` | `healthkit_check_result` counters and `healthkit_check_duration_seconds` histograms via the `metrics` facade crate |
| `sqlx` | `SqlxCheck` — `SELECT 1` probe for any `sqlx::Pool` |
| `redis` | `RedisCheck` — `PING` probe (no async runtime features pulled) |

### Concurrent execution and timeouts

`check_all` (and therefore liveness/readiness/metrics) runs **all checks
concurrently** via `join_all`. A check that does not finish within the
registry's per-check timeout reports `Unhealthy` with the message
`health check timed out after Ns` instead of hanging the probe — the exact
failure mode probes exist to prevent.

```rust
use std::time::Duration;
use healthkit::HealthRegistry;

let registry = HealthRegistry::new()
    .with_default_timeout(Duration::from_secs(2)); // default is 5 s
```

Timeouts nest: keep the kubelet's `timeoutSeconds` above this check timeout
(see the guide below). `Duration::ZERO` disables the deadline.

### Check groups (per-route subsets)

Checks registered with `add_check` run on every probe. Checks registered
into a group additionally run when that group is probed — the standard
Kubernetes startup pattern probes only an `init` group while readiness keeps
checking everything:

```rust
use healthkit::HealthRegistry;
use healthkit::axum::startup_route_for_group;

let registry = HealthRegistry::new();
registry.add_check("database", || async { Ok(HealthStatus::Healthy) });
registry.add_check_to_group("init", "migrations", || async {
    Ok(HealthStatus::Healthy)
});

// /startupz runs only the `init` group (migrations) — a database outage
// cannot crash-loop the pod during startup. /readyz still runs everything.
let startup = startup_route_for_group(registry.clone(), "init");
```

`HealthRegistry::check_group("init")` is available for custom routes.

### Degraded readiness semantics

Readiness aggregates to the worst check status. `Degraded` (e.g. a database
probe above its `warn_above_ms` threshold) responds **`200 OK`** — the pod
stays in the load-balancer pool, and the degraded detail remains visible in
the body, `/healthz/detailed`, and `/metrics`. Alert on the metrics; don't
drain traffic for a slow cache.

If a degraded dependency genuinely makes your service unable to serve, opt
into strict readiness so degraded pods are drained:

```rust
use healthkit::axum::{ReadinessConfig, readiness_route_with};

let app = readiness_route_with(registry, ReadinessConfig::new().degraded_fails_readiness());
```

`Unhealthy` aggregates respond `503` in both modes.

### Load balancers (AWS ALB / GCP)

Load balancers want cheap, stable probes — not dependency checks:

- **`GET /ping`** — constant body `OK`, `200` for as long as the process can
  serve HTTP. Point target-group health checks here.
- **HEAD** — every route serves `HEAD` automatically (axum routes `HEAD` to
  the `GET` handler and strips the body), so ALB/GCP HEAD probes work
  out of the box.
- **Body-vs-status** — ALB matches on status code + optional body; GCP
  health checks match on status. `/ping` gives you both deterministically.
- Prefer `/ping` for load balancers and `/readyz` for Kubernetes —
  routing-vs-lifecycle decisions belong to different layers. If you must
  point an ALB at `/readyz`, note the `Degraded` semantics above: by default
  degraded pods still answer `200` and stay in the pool, which is usually
  what you want in front of an ALB.

**AWS ALB** (target group → Health checks):

```text
Protocol  HTTP
Path      /ping
Port      traffic-port
Matcher   200
Interval  15 s   Timeout 5 s   Healthy 2   Unhealthy 3
```

**GCP** (HTTP health check):

```text
Path           /ping        (or /readyz for dependency-aware checks)
Check interval 15 s
Timeout        5 s
Healthy / Unhealthy thresholds 2 / 3
```

Keep the LB timeout above the registry's per-check timeout, and the check
timeout above your slowest healthy probe — the same nesting rule as the
kubelet section below.

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

By default every scrape executes all checks. With 15 s Prometheus scrapes
and expensive checks, serve a cached rendering instead — the first scrape
runs the checks, scrapes within the TTL are served instantly:

```rust
use std::time::Duration;
use healthkit::axum::metrics_route_with_cache;

let app = Router::new().merge(metrics_route_with_cache(registry, Duration::from_secs(5)));
```

Keep the TTL well below the scrape interval (5 s for 15 s scrapes): the
tradeoff is staleness — a check that flips status right after a scrape stays
invisible in `/metrics` for up to `ttl`. Unhealthy scrapes are unaffected;
the exposition always returns `200 OK`.

Prefer a custom pipeline? `healthkit::render_prometheus(&results)` renders
the same exposition for any `Vec<CheckResult>`.

### `metrics` facade

Enable `metrics` to emit through the de-facto `metrics` facade crate —
alongside, not replacing, the hand-rolled renderer above:

- `healthkit_check_result{name, status}` — counter, incremented per check execution
- `healthkit_check_duration_seconds{name}` — histogram of probe latency

With no recorder installed the calls are no-ops, so the feature is always
safe to enable.

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

Both probes are `Send + Sync` and safe under concurrent execution: each
probe opens its own connection/uses the pool, so parallel `check_all`
executions never share connection state.

## Kubernetes Deployment Guide

### Probe-to-route mapping

| Probe            | Route                | What it should answer                          |
|------------------|----------------------|------------------------------------------------|
| `livenessProbe`  | `GET /healthz`       | "Is the process alive?" — never checks deps    |
| `readinessProbe` | `GET /readyz`        | "Should traffic be routed here?" — checks deps |
| `startupProbe`   | `GET /startupz`      | "Is initialization done?" — gates the others   |
| (load balancer)  | `GET /ping`          | Constant `OK` body — ALB/GCP target checks     |
| (scrape)         | `GET /metrics`       | Prometheus text exposition, always `200 OK`    |
| (debugging)      | `GET /healthz/detailed` | Per-check JSON with status and duration     |

`/healthz` deliberately runs **no** checks: a liveness failure restarts the
container, and killing a pod because Redis hiccuped turns a blip into an
outage. Dependency checks belong in `/readyz`. The startup probe should run
only an `init` group (`startup_route_for_group(registry, "init")`) — see
[Startup probes](#startup-probes-1). A complete, runnable service showing
all six routes plus graceful shutdown lives in
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
kubelet timeoutSeconds  >  check timeout (registry default)  >  healthy probe latency
```

Since 1.2 the registry itself bounds every check
(`HealthRegistry::with_default_timeout`, 5 s by default) — a hanging
dependency reports `Unhealthy` with `health check timed out after Ns`
instead of stalling the probe until the kubelet gives up.

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
`failureThreshold × periodSeconds` above your worst-case startup time.

Keep the startup probe free of dependency checks: if Redis being down at
boot caused `/startupz` to fail, the pod would restart-loop instead of
waiting `NotReady` for the dependency to recover. Register init checks in a
group and point the startup route at it:

```rust
use healthkit::axum::startup_route_for_group;

registry.add_check_to_group("init", "migrations", || async {
    Ok(HealthStatus::Healthy)
});
let startup = startup_route_for_group(registry.clone(), "init");
```

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
