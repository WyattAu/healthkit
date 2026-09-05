# Threat Model — healthkit

Status: **v1.0** · One-page STRIDE over the public API surface
(`HealthRegistry`, `liveness_route`/`readiness_route`/`startup_route`/
`detailed_route`, `HealthStatus`/`CheckResult` types).

Assets: (A1) orchestration correctness — LB/K8s must see truthful
liveness/readiness; (A2) internal topology confidentiality — dependency
names, error messages, and versions exposed by the detailed endpoint.

| # | Threat | Category | Surface | Mitigation | Verifying test |
|---|--------|----------|---------|------------|----------------|
| T1 | Forged status (check callback panics poison the registry) | Tampering | `HealthRegistry::add_check` | Checks are boxed closures returning `Result`; panics are not caught by the registry (tokio task boundary applies) | `tests/proptest.rs::registry_empty_check_all_returns_empty`, `check_result_*` |
| T2 | Detailed endpoint discloses internals | Info disclosure | `detailed_route` | Status codes follow K8s conventions; *no authentication/authorization is provided by this crate* — network policy is the deployer's | none in-crate (see OPEN-1) |
| T3 | Malformed route state / hostile accept header crashes handler | DoS | axum routes | Handlers return typed responses; `#![forbid(unsafe_code)]` | `tests/proptest.rs::health_status_is_healthy_matches_display`, `health_status_is_ready_consistency` |
| T4 | Stale readiness (check cached too long / too short) | Repudiation | registry evaluation | Checks run per request (no cache) — freshness is maximal; load implications documented | `tests/proptest.rs::registry_empty_check_all_returns_empty` |

**OPEN RISKS**

- **OPEN-1 — `detailed_route` has no built-in access control.** The
  detailed payload (dependency list, per-check messages) is intended for
  operators; exposing it publicly leaks topology. Mitigation must come from
  the deploying service (route behind auth or off the public listener).
- **OPEN-2 — panic propagation from a check is undefined at this layer.**
  A panicking check aborts the evaluation task; no `catch_unwind` wrapper,
  no test for a panicking check.

**Out of scope:** K8s probe configuration (timeouts, periods), TLS, and
metric *content* beyond the typed responses.

**Residual risk:** per-request check execution means a slow dependency
directly slows the readiness endpoint — callers should wrap checks with
their own timeouts.
