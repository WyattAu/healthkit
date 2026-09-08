# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

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
