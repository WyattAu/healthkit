# Coverage Notes — healthkit

## Measurement

```
cargo llvm-cov --summary-only --all-features
```

Final: **99.10%** line coverage (3/332 missed) at 1.0.x. Note: the
2026-09-05 audit recorded 76.3% for this crate; that run predates the
same-day coverage-closing commit `d3df6ed` ("test: close coverage gap —
axum route handlers, registry aggregation"), so it measured pre-close
sources. Every re-run of the exact command above reproduces ≥ 99%.

## 1.1.0 measurement (2026-09-09)

Same command with the `sqlx`, `redis`, and `prometheus` modules: **96.87%**
line coverage (27/863 missed).

- `src/checks/sqlx.rs` — 98.33% (2 lines: unmatched-match-arm plumbing).
- `src/checks/redis.rs` — 91.63% (21 lines: the `spawn_blocking` join-error
  arm, plus parts of the `-ERR`/unexpected-reply arms whose hermetic
  reproduction depends on redis client handshake internals). The
  PING-success path is covered hermetically by a fake RESP TCP server;
  live-server tests are `#[ignore]`d.
- `src/metrics.rs` — 99.35% (1 line).
- `src/axum.rs` — 95.00% (same 3 structurally-unreachable lines as below).

## Known exception: `src/axum.rs` lines 68, 82, 96 (3 lines)

These are the `Err(_)` arms of the `readiness_handler`, `startup_handler`, and
`detailed_handler` axum handlers (mapping a registry error to `503`/`500`).
They are **structurally unreachable with the current registry semantics**:
`HealthRegistry::check_readiness` / `check_liveness` return `Ok` on every path —
`check_all` maps any individual check failure to `HealthStatus::Unhealthy`
(`unwrap_or`), so the registry-level `Result` never carries an `Err`.

The arms are kept deliberately as fail-closed handlers: if the registry API
later grows a real error path (e.g. propagating check panics as errors), the
HTTP surface already degrades correctly. Covering them with a test would
require either changing registry semantics (behavior change for coverage) or
mocking an error the type system cannot produce.
