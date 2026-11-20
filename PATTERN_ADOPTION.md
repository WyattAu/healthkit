# Pattern adoption

Health check registry with sqlx and redis probes, metrics, and axum
routes.

| Pattern | Status | Evidence |
|---|---|---|
| T13 | partial | src/axum.rs:6 (HTTP surface is a shell importing HealthRegistry and the response types; check execution lives in src/registry.rs) |
| P64 | adopted | tests/proptest.rs:15 (proptest suite) |
| P94 | n/a | probe registry consumed by orchestrators (src/registry.rs); the supervision loop itself is out of domain for this crate |
