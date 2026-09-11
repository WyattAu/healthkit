# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [1.2.0] - 2026-09-11

Backward-compatible release: everything below is additive. Existing
`add_check` / `check_all` / route behavior is unchanged except that
`check_all` now runs concurrently and failing checks carry their error
message.

### Added

- **Concurrent checks**: `check_all` (and liveness/readiness/metrics, which
  build on it) now executes all registered checks concurrently instead of
  sequentially — one slow dependency can no longer stall the whole probe.
- **Per-check timeout**: `HealthRegistry::with_default_timeout` (default
  5 s). A check that exceeds the deadline reports `Unhealthy` with the
  message `health check timed out after Ns`; `Duration::ZERO` disables the
  deadline.
- **Error detail capture**: `CheckResult.message` is now populated with the
  failing check's `HealthCheckError` display (previously hardcoded `None`).
- **Check groups**: `HealthRegistry::add_check_to_group(group, name, check)`
  and `HealthRegistry::check_group(group)`. Grouped checks run in
  registry-wide probes *and* when their group is probed explicitly, enabling
  per-route subsets. `startup_route_for_group(registry, "init")` implements
  the Kubernetes startup pattern: startup answers "is init done?" without
  running every dependency check.
- **Readiness config**: `ReadinessConfig` with `readiness_route_with` /
  `detailed_route_with`. Default matches 1.1 behavior (`Degraded` → `200`):
  degraded pods keep receiving traffic while the body, `/healthz/detailed`,
  and `/metrics` report the degradation. Opt into strict draining with
  `degraded_fails_readiness()` (`Degraded` → `503`).
- **LB compatibility**: `ping_route` — constant-body `OK` (`200`, plain
  text) for AWS ALB / GCP target-group health checks. `HEAD` requests are
  served by all `GET` routes (axum strips the body); documented in the new
  README load-balancer recipes section. `ReadinessConfig` also implements
  `Copy`/`Default`/`Debug`.
- **Scrape caching**: `metrics_route_with_cache(registry, ttl)` /
  `MetricsState::with_cache` — serve the Prometheus exposition from a TTL
  cache so scrapes don't execute all checks every time. Default
  (`metrics_route`) remains run-on-every-scrape.
- **`metrics` facade feature** (default off): emits `healthkit_check_result`
  counters (`name`, `status` labels) and `healthkit_check_duration_seconds`
  histograms (`name` label) through the `metrics` crate on every check
  execution, alongside the hand-rolled renderer.
- Example `k8s_service` now demonstrates groups (single registry, `init`
  group for startup), `/ping`, and cached metrics.
- Tests: concurrency (five slow checks complete in ~one check's duration),
  per-check timeout firing, message capture, group subsets, degraded
  readiness in both modes, `/ping` body, HEAD support, cache TTL behavior
  (fresh/expired), metrics-facade emission.
- `examples/k8s_service.rs`: complete Kubernetes-ready service wiring every
  route (liveness, readiness, startup, ping, detailed, Prometheus
  `/metrics`) with `SqlxCheck` and `RedisCheck` — including the production
  patterns the docs recommend: dependency checks only on readiness, an
  `init` group gating the startup probe, and a `draining` check that flips
  `/readyz` to 503 on SIGTERM while axum drains in-flight connections.
- README: expanded "Kubernetes Deployment Guide" (probe-to-route mapping,
  timeout alignment between kubelet and check timeouts, startup-probe
  budgeting, graceful-shutdown/preStop integration) plus load-balancer
  recipes (AWS ALB, GCP) and degraded-readiness guidance.
- COMPARISON.md: positioning against `kube-health-check` and the dormant
  `health` crate, with the ecosystem status as of September 2026.

### Changed

- `futures` (std-only) added as a dependency for concurrent check
  execution.

### Fixed

- A registered check that returned `Err` had its error detail discarded
  (`message: None`); it now appears in `CheckResult.message` and in
  `/readyz` + `/healthz/detailed` JSON payloads.

## [1.1.0] - 2026-09-09

### Added

- `sqlx` feature: `SqlxCheck` — runs `SELECT 1` against any `sqlx::Pool`
  (generic over the backend; only `sqlite` is pulled for this crate's own
  tests) with a configurable timeout and a `warn_above_ms` latency threshold
  (success over the threshold reports `Degraded`).
- `redis` feature: `RedisCheck` — `PING` probe over a synchronous connection
  (works with `default-features = false`; no async runtime features pulled)
  with the same timeout/latency semantics.
- `prometheus` feature: `render_prometheus` renders check results as
  Prometheus text exposition (version 0.0.4), hand-rolled without the
  `prometheus` crate — a per-check `healthkit_check_healthiness` gauge
  (1 / 0.5 / 0) and a `healthkit_check_duration_seconds` histogram. With
  `axum`, adds `metrics_handler` / `metrics_route` serving `GET /metrics`
  (always `200 OK` so scrapes survive unhealthy states).
- Tests: hermetic in-memory SQLite for `sqlx`; a fake RESP TCP server (and
  `#[ignore]`d live-server tests) for `redis`.

### Fixed

- `tokio` dependency now declares the `sync` and `time` features it uses
  (previously satisfied only transitively via `axum`).

## [1.0.0] - 2026-09-05

First stable release. The public API is now covered by the project's
semver guarantees: breaking changes require a major version bump.

### Fixed

- `HealthRegistry::check_liveness` and `check_readiness` aggregated check
  results with `min_by_key`, so a single failing check could still report
  `Healthy`. Aggregation now takes the worst observed status, matching the
  documented contract ("`Unhealthy` if any fail").

## [0.1.0] - 2026-08-31

### Added

- Liveness probes (process running, not deadlocked), readiness probes
  (service can accept traffic), and startup probes (initialization
  complete).
- Dependency checking: register custom checks for databases, caches,
  and other downstream services.
- Ready-to-use Axum route handlers (default feature).
- `#![forbid(unsafe_code)]`.
