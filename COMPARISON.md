# healthkit vs. the alternatives

Status of the Rust health-check-for-Kubernetes ecosystem as of **September
2026**. Numbers are crates.io figures at the time of writing; check the
linked crates for current data.

| | **healthkit** | `kube-health-check` | `health` |
|---|---|---|---|
| Version | 1.1.0 | 0.1.0 | 0.2.0 |
| Last release | Sep 2026 | Aug 2026 | **May 2022** |
| Maintenance | active | new, single release | dormant since 2022 |
| Downloads | 67 | 61 total | ~120k total, ~4k recent |
| Liveness route | ✔ (no dependency checks by design) | ✔ | ✔ (cached state) |
| Readiness route | ✔ + per-check JSON detail | ✔ | ✔ (cached state) |
| Startup probe | ✔ dedicated route + guidance | ✔ | ✘ (pre-dates the pattern) |
| Dependency checkers | `sqlx`, `redis` built-in + any custom async closure | sqlx backends, redis, mongo, kafka, … behind features | none bundled |
| Latency-degradation signal | ✔ `Degraded` status + `warn_above_ms` thresholds | — | — |
| Prometheus exposition | ✔ hand-rolled text format, zero extra deps | ✔ via the `prometheus` crate | — |
| Axum integration | axum 0.8 (default feature) | ✔ | tower-era (0.2.0 predates axum 0.7+) |
| HTTP status semantics | 200 / 503 / 500 derived from aggregated status | ✔ | framework-dependent |
| `unsafe` | forbidden (`#![forbid(unsafe_code)]`) | — | — |

## `health` (crates.io/health)

The most-adopted crate in this space (~120k lifetime downloads) — and the
most abandoned. Its last release, 0.2.0, shipped in **May 2022**; recent
downloads are carried by long-lived dependents, not new adoption.

Architecturally it takes a different approach: checks are polled *in the
background* on a fixed interval and HTTP handlers serve the cached result.
That keeps probe latency flat but means the endpoint reports state that can
be arbitrarily stale — a database that just went down still shows healthy
until the next tick. `healthkit` executes checks on-demand at probe time, so
a readiness probe reflects reality within its configured timeout, at the cost
of paying the check cost per probe (which is exactly what the `timeoutSeconds`
guidance in the README addresses).

`health` also predates the startup-probe pattern and axum 0.7+; integrating
it with a current axum stack means writing your own adapter layer.

## `kube-health-check` (crates.io/kube-health-check)

A single 0.1.0 release published **August 2026** (61 downloads at the time of
writing). The broad backend feature list (postgres, mysql, mssql, mongodb,
kafka, rabbitmq, nats, elasticsearch…) is wider than `healthkit`'s built-in
set, and the Prometheus support builds on the `prometheus` crate.

Trade-offs to weigh:

- **Maturity** — one pre-1.0 release from a single maintainer, no release
  history, no semver track record. `healthkit` is at 1.1.0 with a
  changelogged, semver-covered API.
- **Compile-time weight** — the `full` feature pulls sysinfo plus a dozen
  client libraries. `healthkit`'s features compile independently; the core
  has three dependencies, and the Prometheus format is hand-rolled so
  scrapes don't couple you to the `prometheus` crate's API.
- **Degradation signal** — `kube-health-check` reports binary up/down;
  `healthkit`'s `Degraded` status (successful check above a latency
  threshold) gives you a leading indicator between "fine" and "down".

## When to choose which

- **Choose `health`** only if you already depend on it and need its exact
  background-poll semantics — and accept unmaintained-dependency risk.
- **Choose `kube-health-check`** if you need one of its exotic backend
  checkers (kafka, nats, mssql) out of the box and accept pre-1.0 risk.
- **Choose `healthkit`** for a current, minimal-dependency axum service
  where you want correct probe semantics (deps on readiness only), startup
  gating, a degradation signal, and Prometheus metrics without pulling the
  `prometheus` crate.

Missing a checker you need? `HealthRegistry::add_check` accepts any async
closure returning `Result<HealthStatus, HealthCheckError>` — a custom check
is three lines, and PRs for new built-ins are welcome.
