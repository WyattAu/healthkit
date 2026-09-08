//! Prometheus text-exposition rendering of health check results.
//!
//! The output implements the [Prometheus text-based format, version
//! 0.0.4](https://prometheus.io/docs/instrumenting/exposition_formats/)
//! by hand — deliberately without depending on the `prometheus` crate to
//! avoid version coupling. Two metric families are emitted:
//!
//! - `healthkit_check_healthiness` (gauge): `1` = healthy, `0.5` = degraded,
//!   `0` = unhealthy, labeled by check name.
//! - `healthkit_check_duration_seconds` (histogram): probe latency in
//!   seconds with the standard bucket set plus `_sum` and `_count`.
//!
//! Example scrape output for a single healthy check named `db` that took
//! 3 ms:
//!
//! ```text
//! # HELP healthkit_check_healthiness Health status of the check (1 = healthy, 0.5 = degraded, 0 = unhealthy).
//! # TYPE healthkit_check_healthiness gauge
//! healthkit_check_healthiness{check="db"} 1
//! # HELP healthkit_check_duration_seconds Duration of the check execution in seconds.
//! # TYPE healthkit_check_duration_seconds histogram
//! healthkit_check_duration_seconds_bucket{check="db",le="0.005"} 1
//! healthkit_check_duration_seconds_bucket{check="db",le="0.01"} 1
//! ...
//! healthkit_check_duration_seconds_bucket{check="db",le="+Inf"} 1
//! healthkit_check_duration_seconds_sum{check="db"} 0.003
//! healthkit_check_duration_seconds_count{check="db"} 1
//! ```

#[cfg(all(feature = "prometheus", feature = "axum"))]
use crate::registry::HealthRegistry;
use crate::types::{CheckResult, HealthStatus};

/// Upper bounds (in seconds) of the latency histogram buckets. Mirrors the
/// bucket set Prometheus uses for its own client libraries.
const LATENCY_BUCKETS_SECS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Content type mandated by the text exposition format, version 0.0.4.
#[cfg(all(feature = "prometheus", feature = "axum"))]
const EXPOSITION_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Map a [`HealthStatus`] to its Prometheus gauge value.
fn healthiness_value(status: HealthStatus) -> f64 {
    match status {
        HealthStatus::Healthy => 1.0,
        HealthStatus::Degraded => 0.5,
        HealthStatus::Unhealthy => 0.0,
    }
}

/// Escape a label value per the exposition format (`\`, `"`, newline).
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render a float in the bare format the exposition parser accepts
/// (`1.0` → `1`, `0.5` → `0.5`, `0.0` → `0`).
fn fmt_float(value: f64) -> String {
    format!("{value}")
}

/// Render check results as Prometheus text exposition (version 0.0.4).
///
/// Each entry of `results` produces one `healthkit_check_healthiness` sample
/// and one `healthkit_check_duration_seconds` histogram (single observation:
/// cumulative bucket counts are `0` or `1`). An empty slice renders only the
/// `HELP`/`TYPE` header lines.
pub fn render_prometheus(results: &[CheckResult]) -> String {
    let mut out = String::with_capacity(results.len() * 512 + 256);

    out.push_str(
        "# HELP healthkit_check_healthiness Health status of the check \
         (1 = healthy, 0.5 = degraded, 0 = unhealthy).\n",
    );
    out.push_str("# TYPE healthkit_check_healthiness gauge\n");
    for result in results {
        out.push_str(&format!(
            "healthkit_check_healthiness{{check=\"{}\"}} {}\n",
            escape_label(&result.name),
            fmt_float(healthiness_value(result.status)),
        ));
    }

    out.push_str(
        "# HELP healthkit_check_duration_seconds Duration of the check \
         execution in seconds.\n",
    );
    out.push_str("# TYPE healthkit_check_duration_seconds histogram\n");
    for result in results {
        let name = escape_label(&result.name);
        let secs = result.duration.as_secs_f64();
        for bucket in LATENCY_BUCKETS_SECS {
            let count = u64::from(secs <= bucket);
            out.push_str(&format!(
                "healthkit_check_duration_seconds_bucket{{check=\"{name}\",le=\"{}\"}} {count}\n",
                fmt_float(bucket),
            ));
        }
        out.push_str(&format!(
            "healthkit_check_duration_seconds_bucket{{check=\"{name}\",le=\"+Inf\"}} 1\n"
        ));
        out.push_str(&format!(
            "healthkit_check_duration_seconds_sum{{check=\"{name}\"}} {}\n",
            fmt_float(secs),
        ));
        out.push_str(&format!(
            "healthkit_check_duration_seconds_count{{check=\"{name}\"}} 1\n"
        ));
    }

    out
}

/// Shared state for the Prometheus metrics handler.
///
/// Every scrape executes all registered checks and renders the results.
#[cfg(all(feature = "prometheus", feature = "axum"))]
#[derive(Clone)]
pub struct MetricsState {
    /// Registry whose checks run on every scrape.
    pub registry: HealthRegistry,
}

#[cfg(all(feature = "prometheus", feature = "axum"))]
impl MetricsState {
    /// Create handler state around a registry.
    pub fn new(registry: HealthRegistry) -> Self {
        Self { registry }
    }
}

/// Axum handler serving `GET /metrics` in Prometheus text format.
///
/// Always responds `200 OK` with `Content-Type:
/// text/plain; version=0.0.4; charset=utf-8` — a scrape must succeed even
/// when checks report unhealthy, otherwise monitoring blind-spots appear
/// exactly when they matter most.
#[cfg(all(feature = "prometheus", feature = "axum"))]
pub async fn metrics_handler(
    axum::extract::State(state): axum::extract::State<MetricsState>,
) -> impl axum::response::IntoResponse {
    let results = state.registry.check_all().await;
    (
        [(axum::http::header::CONTENT_TYPE, EXPOSITION_CONTENT_TYPE)],
        render_prometheus(&results),
    )
}

// Tests assert exact exposition bytes; unwrap/expect and panicking asserts
// are the test signal here.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn result(name: &str, status: HealthStatus, millis: u64) -> CheckResult {
        CheckResult {
            name: name.to_string(),
            status,
            message: None,
            duration: Duration::from_millis(millis),
        }
    }

    #[test]
    fn healthy_check_renders_gauge_value_one() {
        let out = render_prometheus(&[result("db", HealthStatus::Healthy, 3)]);
        assert!(out.contains("healthkit_check_healthiness{check=\"db\"} 1\n"));
    }

    #[test]
    fn degraded_check_renders_gauge_value_half() {
        let out = render_prometheus(&[result("db", HealthStatus::Degraded, 3)]);
        assert!(out.contains("healthkit_check_healthiness{check=\"db\"} 0.5\n"));
    }

    #[test]
    fn unhealthy_check_renders_gauge_value_zero() {
        let out = render_prometheus(&[result("db", HealthStatus::Unhealthy, 3)]);
        assert!(out.contains("healthkit_check_healthiness{check=\"db\"} 0\n"));
    }

    #[test]
    fn histogram_buckets_are_cumulative_and_ordered() {
        let out = render_prometheus(&[result("db", HealthStatus::Healthy, 3)]);

        // 3 ms falls into the 0.005 bucket and every bucket above it.
        assert!(
            out.contains("healthkit_check_duration_seconds_bucket{check=\"db\",le=\"0.005\"} 1\n")
        );
        assert!(
            out.contains("healthkit_check_duration_seconds_bucket{check=\"db\",le=\"0.01\"} 1\n")
        );
        assert!(
            out.contains("healthkit_check_duration_seconds_bucket{check=\"db\",le=\"+Inf\"} 1\n")
        );
        assert!(out.contains("healthkit_check_duration_seconds_sum{check=\"db\"} 0.003\n"));
        assert!(out.contains("healthkit_check_duration_seconds_count{check=\"db\"} 1\n"));

        // Every standard bucket line is present, in order.
        let buckets = [
            "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "10",
        ];
        let mut last = 0;
        for le in buckets {
            let marker = format!("le=\"{le}\"}}");
            let pos = out.find(&marker).unwrap();
            assert!(pos > last, "bucket {le} out of order");
            last = pos;
        }
    }

    #[test]
    fn slow_check_lands_above_low_buckets() {
        let out = render_prometheus(&[result("db", HealthStatus::Healthy, 3_000)]);
        assert!(
            out.contains("healthkit_check_duration_seconds_bucket{check=\"db\",le=\"0.005\"} 0\n")
        );
        assert!(
            out.contains("healthkit_check_duration_seconds_bucket{check=\"db\",le=\"2.5\"} 0\n")
        );
        assert!(out.contains("healthkit_check_duration_seconds_bucket{check=\"db\",le=\"5\"} 1\n"));
        assert!(out.contains("healthkit_check_duration_seconds_sum{check=\"db\"} 3\n"));
    }

    #[test]
    fn header_lines_precede_samples_and_families_do_not_interleave() {
        let out = render_prometheus(&[
            result("db", HealthStatus::Healthy, 1),
            result("cache", HealthStatus::Unhealthy, 2),
        ]);

        let help_gauge = out.find("# HELP healthkit_check_healthiness").unwrap();
        let type_gauge = out
            .find("# TYPE healthkit_check_healthiness gauge\n")
            .unwrap();
        let first_gauge = out
            .find("healthkit_check_healthiness{check=\"db\"} 1\n")
            .unwrap();
        assert!(help_gauge < type_gauge && type_gauge < first_gauge);

        let help_hist = out.find("# HELP healthkit_check_duration_seconds").unwrap();
        assert!(help_hist > first_gauge);
        // Both gauge samples precede the histogram family.
        assert!(
            out.find("healthkit_check_healthiness{check=\"cache\"} 0\n")
                .unwrap()
                < help_hist
        );

        assert_eq!(out.matches("# TYPE").count(), 2);
        assert_eq!(out.matches("# HELP").count(), 2);
    }

    #[test]
    fn empty_results_render_only_headers() {
        let out = render_prometheus(&[]);
        assert_eq!(
            out,
            "# HELP healthkit_check_healthiness Health status of the check \
             (1 = healthy, 0.5 = degraded, 0 = unhealthy).\n\
             # TYPE healthkit_check_healthiness gauge\n\
             # HELP healthkit_check_duration_seconds Duration of the check \
             execution in seconds.\n\
             # TYPE healthkit_check_duration_seconds histogram\n"
        );
    }

    #[test]
    fn label_values_are_escaped() {
        let out = render_prometheus(&[result("we\"ird\\na#me", HealthStatus::Healthy, 0)]);
        assert!(out.contains("healthkit_check_healthiness{check=\"we\\\"ird\\\\na#me\"} 1\n"));
    }

    #[test]
    fn output_ends_with_newline() {
        let out = render_prometheus(&[result("db", HealthStatus::Healthy, 1)]);
        assert!(out.ends_with('\n'));
        assert!(!out.ends_with("\n\n"));
    }
}
