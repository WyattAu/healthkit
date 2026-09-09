# Requirements — healthkit

Numbered, testable requirements. Every requirement maps to at least one named
test; every security-relevant test cites at least one requirement. Doc
comments on the implementing public item carry `REQ-HLK-NNN` tags.

## Functional

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-HLK-001 | `HealthRegistry::new` creates an empty registry; `check_all` on it returns an empty result set | MUST |
| REQ-HLK-002 | `add_check` registers named async checks; multiple checks coexist and all run on `check_all` | MUST |
| REQ-HLK-003 | `check_all` returns one `CheckResult` per registered check, carrying name, status, latency, and message | MUST |
| REQ-HLK-004 | `check_liveness` returns `Healthy` for an empty registry and reflects registered checks without readiness semantics (process is alive) | MUST |
| REQ-HLK-005 | `check_liveness` reports the worst status when checks disagree (mixed statuses never report healthier than reality) | MUST |
| REQ-HLK-006 | `check_readiness` reports `Unhealthy` with details when a check fails, and aggregates mixed results correctly | MUST |
| REQ-HLK-007 | A check returning `Err(HealthCheckError)` folds into an `Unhealthy` readiness result rather than aborting the sweep | MUST |
| REQ-HLK-008 | `HealthStatus::is_healthy` / `is_ready` classify every status consistently with its `Display` form | MUST |
| REQ-HLK-009 | `CheckResult` construction carries name (non-empty), status, and message through serde round-trips | MUST |
| REQ-HLK-010 | `liveness_route()` returns HTTP 200 with a healthy liveness body | MUST |
| REQ-HLK-011 | `readiness_route` returns 200 when all checks pass and 503 when any check fails | MUST |
| REQ-HLK-012 | `startup_route` returns 200 when healthy and 503 when unhealthy | MUST |
| REQ-HLK-013 | `detailed_route` returns 200 with per-check details when ready and 503 when not ready | MUST |
| REQ-HLK-014 | All health routes respond with `application/json` bodies that deserialize into their response types | MUST |
| REQ-HLK-015 | `SqlxCheck` reports healthy when the probe succeeds within the threshold, degraded when latency exceeds `warn_above_ms`, unhealthy when the probe fails | MUST |
| REQ-HLK-016 | `SqlxCheck::with_timeout` aborts a probe exceeding its deadline and folds the timeout into `Unhealthy` readiness | MUST |
| REQ-HLK-017 | `SqlxCheck::register` integrates the check into a `HealthRegistry` so it flows through readiness sweeps | SHOULD |
| REQ-HLK-018 | `RedisCheck` reports healthy on PONG, and folds connection refusal, error replies, unexpected replies, and invalid URLs into `Unhealthy` | MUST |
| REQ-HLK-019 | `render_prometheus` emits a valid Prometheus text exposition: healthy/degraded/unhealthy checks render gauge values 1/0.5/0, headers precede samples, families do not interleave, output ends with newline | MUST |
| REQ-HLK-020 | `render_prometheus` on an empty result set renders only HELP/TYPE headers; latency histograms use cumulative, ordered buckets with slow samples landing in high buckets | MUST |
| REQ-HLK-021 | `metrics_route` / `metrics_handler` return 200 with the text exposition even when checks are unhealthy or the registry is empty | MUST |
| REQ-HLK-022 | Prometheus label values are escaped so metric names/labels cannot be malformed by check names or messages | MUST |

## Security

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-HLK-100 | No public function panics on failure inputs — connection errors, invalid URLs, timeouts, and poisoned state all surface as `HealthStatus::Unhealthy` or `Err(HealthCheckError)` | MUST |
| REQ-HLK-101 | A failing dependency (Redis/SQL down) must never crash the process or the HTTP handler; degradation is reported, not propagated as a panic | MUST |
| REQ-HLK-102 | Redis credentials embedded in the URL are not echoed into check results or error messages | MUST |
| REQ-HLK-103 | The crate forbids `unsafe` code; property-generated arbitrary statuses/messages cannot break response invariants | MUST |
| REQ-HLK-104 | Readiness/liveness endpoints expose no secrets — bodies contain only check names, statuses, latencies, and caller-supplied messages | SHOULD |

## Robustness

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-HLK-200 | Checks run under per-check timeouts: a hung dependency cannot hang the readiness sweep beyond its deadline | MUST |
| REQ-HLK-201 | Registry sweeps are concurrency-safe: checks are stored in `Arc` state and multiple concurrent `check_all`/handler invocations maintain correct results | MUST |
| REQ-HLK-202 | Latency thresholds classify deterministically: at/below `warn_above_ms` → Healthy (or Degraded only above), never flipping with jitter around equality | SHOULD |
| REQ-HLK-203 | Builder-style overrides (`with_timeout`, `with_warn_above_ms`) are applied deterministically over defaults | SHOULD |
| REQ-HLK-204 | Error `Display` messages for `HealthCheckError` and `HealthStatus` are stable and non-empty | SHOULD |

## Traceability Matrix

| Requirement | Test (fn, file) | Property class |
|-------------|-----------------|----------------|
| REQ-HLK-001 | `registry_default_is_empty_and_healthy` (`src/registry.rs`), `registry_empty_check_all_returns_empty` (`tests/proptest.rs`) | unit/property |
| REQ-HLK-002 | `registry_new_and_add_check`, `registry_add_multiple_checks` (`src/registry.rs`) | unit |
| REQ-HLK-003 | `check_result_creation_and_is_healthy` (`src/types.rs`), `check_result_name_non_empty`, `check_result_message_roundtrip` (`tests/proptest.rs`) | unit/property |
| REQ-HLK-004 | `registry_check_liveness_no_checks` (`src/registry.rs`) | unit |
| REQ-HLK-005 | `registry_check_liveness_mixed_reports_worst_status` (`src/registry.rs`) | unit |
| REQ-HLK-006 | `registry_check_readiness_failing_reports_unhealthy_with_details`, `registry_check_readiness_mixed` (`src/registry.rs`) | unit |
| REQ-HLK-007 | `registry_check_with_error_returns_unhealthy` (`src/registry.rs`) | unit |
| REQ-HLK-008 | `health_status_is_healthy`, `health_status_is_ready` (`src/types.rs`), `health_status_is_healthy_matches_display`, `health_status_is_ready_consistency` (`tests/proptest.rs`) | unit/property |
| REQ-HLK-009 | `check_result_creation_and_is_healthy`, `check_result_unhealthy` (`src/types.rs`), `check_result_message_roundtrip` (`tests/proptest.rs`) | unit/property |
| REQ-HLK-010 | `liveness_route_returns_200_with_healthy_status` (`tests/axum_routes.rs`) | integration |
| REQ-HLK-011 | `readiness_route_returns_200_when_all_checks_pass`, `readiness_route_returns_503_when_a_check_fails` (`tests/axum_routes.rs`) | integration |
| REQ-HLK-012 | `startup_route_returns_200_when_healthy`, `startup_route_returns_503_when_unhealthy` (`tests/axum_routes.rs`) | integration |
| REQ-HLK-013 | `detailed_route_returns_200_with_check_details_when_ready`, `detailed_route_returns_503_when_not_ready` (`tests/axum_routes.rs`) | integration |
| REQ-HLK-014 | `body_json` (`tests/axum_routes.rs`) | integration |
| REQ-HLK-015 | `healthy_when_probe_succeeds_within_threshold`, `probe_over_threshold_reports_degraded_readiness`, `unhealthy_when_probe_fails` (`tests/sqlx_check.rs`) | integration |
| REQ-HLK-016 | `times_out_when_probe_exceeds_deadline` (`src/checks/sqlx.rs`) | unit |
| REQ-HLK-017 | `registered_check_flows_through_readiness` (`tests/sqlx_check.rs`), `register_integrates_with_registry` (`src/checks/redis.rs`) | integration |
| REQ-HLK-018 | `healthy_when_ping_replies_pong` (`src/checks/redis.rs`), `connection_refused_folds_into_unhealthy_readiness`, `error_reply_folds_into_unhealthy_readiness` (`tests/redis_check.rs`), `unhealthy_on_invalid_url`, `unhealthy_on_unexpected_reply` (`src/checks/redis.rs`) | unit/integration |
| REQ-HLK-019 | `healthy_check_renders_gauge_value_one`, `degraded_check_renders_gauge_value_half`, `unhealthy_check_renders_gauge_value_zero`, `header_lines_precede_samples_and_families_do_not_interleave`, `output_ends_with_newline` (`src/metrics.rs`) | unit |
| REQ-HLK-020 | `empty_results_render_only_headers` (`src/metrics.rs`), `histogram_buckets_are_cumulative_and_ordered`, `slow_check_lands_above_low_buckets` (`src/metrics.rs`) | unit |
| REQ-HLK-021 | `metrics_route_returns_200_with_text_exposition`, `metrics_route_reports_unhealthy_checks_with_200`, `metrics_route_with_empty_registry_renders_headers_only` (`tests/prometheus_metrics.rs`) | integration |
| REQ-HLK-022 | `label_values_are_escaped` (`src/metrics.rs`) | unit |
| REQ-HLK-100 | `unhealthy_when_connection_refused`, `unhealthy_on_invalid_url`, `times_out_when_probe_exceeds_deadline` (`src/checks/*.rs`); property sweeps in `tests/proptest.rs` | unit/property |
| REQ-HLK-101 | `connection_refused_folds_into_unhealthy_readiness` (`tests/redis_check.rs`), `failed_probe_folds_into_unhealthy_readiness` (`tests/sqlx_check.rs`) | integration |
| REQ-HLK-102 | `unhealthy_on_invalid_url` (`src/checks/redis.rs`), `error_reply_folds_into_unhealthy_readiness` (`tests/redis_check.rs`) | unit/integration |
| REQ-HLK-103 | `#![forbid(unsafe_code)]` (`src/lib.rs`); `tests/proptest.rs` (`arb_health_status` properties) | property/build |
| REQ-HLK-104 | `body_json` (`tests/axum_routes.rs`), `detailed_route_returns_200_with_check_details_when_ready` (`tests/axum_routes.rs`) | integration |
| REQ-HLK-200 | `times_out_when_probe_exceeds_deadline` (`src/checks/sqlx.rs`), `healthy_ping_flows_through_readiness` with timeout builder (`tests/redis_check.rs`) | unit/integration |
| REQ-HLK-201 | shared-registry use across `tests/axum_routes.rs` and `tests/prometheus_metrics.rs` (`registry_with` helpers) | integration |
| REQ-HLK-202 | `degraded_when_latency_exceeds_threshold` (`src/checks/sqlx.rs`), `probe_over_threshold_reports_degraded_readiness` (`tests/sqlx_check.rs`) | unit/integration |
| REQ-HLK-203 | `builder_overrides_are_applied` (`src/checks/sqlx.rs`) | unit |
| REQ-HLK-204 | `health_check_error_display`, `health_status_display` (`src/error.rs`, `src/types.rs`) | unit |
