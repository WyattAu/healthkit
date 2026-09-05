# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

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
