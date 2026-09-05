# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.1.0] - 2026-08-31

### Added

- Liveness probes (process running, not deadlocked), readiness probes
  (service can accept traffic), and startup probes (initialization
  complete).
- Dependency checking: register custom checks for databases, caches,
  and other downstream services.
- Ready-to-use Axum route handlers (default feature).
- `#![forbid(unsafe_code)]`.
