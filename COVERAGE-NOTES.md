# Coverage Notes — healthkit

## Measurement

```
cargo llvm-cov --summary-only --all-features
```

Final: **99.10%** line coverage (3/332 missed). Note: the 2026-09-05 audit
recorded 76.3% for this crate; that run predates the same-day coverage-closing
commit `d3df6ed` ("test: close coverage gap — axum route handlers, registry
aggregation"), so it measured pre-close sources. Every re-run of the exact
command above reproduces ≥ 99%.

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
