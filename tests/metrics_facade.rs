// Metrics-facade tests need a recorder; installing one is process-global,
// so all assertions live in a single test. unwrap/expect and panicking
// asserts are the test signal here.
#![cfg(feature = "metrics")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::sync::{Arc, Mutex};

use metrics::{
    Counter, CounterFn, Gauge, GaugeFn, Histogram, HistogramFn, Key, KeyName, Metadata, Recorder,
    SharedString, Unit,
};

/// One recorded observation: (metric name, labels, value).
type Observation<V> = (String, Vec<(String, String)>, V);

/// Shared observation storage handed to every registered handle.
#[derive(Default, Clone)]
struct Collected {
    counters: Arc<Mutex<Vec<Observation<u64>>>>,
    histograms: Arc<Mutex<Vec<Observation<f64>>>>,
}

struct CollectingRecorder {
    collected: Collected,
}

struct CounterHandle {
    key: Key,
    collected: Collected,
}

impl CounterFn for CounterHandle {
    fn increment(&self, value: u64) {
        self.collected.counters.lock().unwrap().push((
            self.key.name().to_string(),
            labels(&self.key),
            value,
        ));
    }

    fn absolute(&self, _value: u64) {}
}

struct GaugeHandle;

impl GaugeFn for GaugeHandle {
    fn increment(&self, _value: f64) {}
    fn decrement(&self, _value: f64) {}
    fn set(&self, _value: f64) {}
}

struct HistogramHandle {
    key: Key,
    collected: Collected,
}

impl HistogramFn for HistogramHandle {
    fn record(&self, value: f64) {
        self.collected.histograms.lock().unwrap().push((
            self.key.name().to_string(),
            labels(&self.key),
            value,
        ));
    }
}

fn labels(key: &Key) -> Vec<(String, String)> {
    key.labels()
        .map(|l| (l.key().to_string(), l.value().to_string()))
        .collect()
}

impl Recorder for CollectingRecorder {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}

    fn register_counter(&self, key: &Key, _: &Metadata<'_>) -> Counter {
        Counter::from_arc(Arc::new(CounterHandle {
            key: key.clone(),
            collected: self.collected.clone(),
        }))
    }

    fn register_gauge(&self, _key: &Key, _: &Metadata<'_>) -> Gauge {
        Gauge::from_arc(Arc::new(GaugeHandle))
    }

    fn register_histogram(&self, key: &Key, _: &Metadata<'_>) -> Histogram {
        Histogram::from_arc(Arc::new(HistogramHandle {
            key: key.clone(),
            collected: self.collected.clone(),
        }))
    }
}

#[tokio::test]
async fn facade_emits_counters_and_histograms_for_each_check() {
    let recorder = Arc::new(CollectingRecorder {
        collected: Collected::default(),
    });

    // Each integration test file is its own binary, so the first
    // installation here wins; be tolerant of a pre-existing recorder
    // regardless.
    let _ = metrics::set_global_recorder(Arc::clone(&recorder));
    let collected = recorder.collected.clone();

    let registry = healthkit::HealthRegistry::new();
    let r = registry.clone();
    tokio::task::spawn_blocking(move || {
        r.add_check("db", || async {
            Ok::<_, healthkit::HealthCheckError>(healthkit::HealthStatus::Healthy)
        });
        r.add_check("cache", || async {
            Err::<healthkit::HealthStatus, _>(healthkit::HealthCheckError::CheckFailed(
                "down".to_string(),
            ))
        });
    })
    .await
    .unwrap();

    let results = registry.check_all().await;
    assert_eq!(results.len(), 2);

    let counters = collected.counters.lock().unwrap();
    // One counter increment per check, labeled name + status.
    assert_eq!(counters.len(), 2, "expected one counter per check");
    for (name, labels, value) in counters.iter() {
        assert_eq!(name, "healthkit_check_result");
        assert_eq!(*value, 1);
        assert_eq!(labels[0].0, "name");
        assert_eq!(labels[1].0, "status");
    }
    assert!(
        counters
            .iter()
            .any(|(_, l, _)| l[0].1 == "db" && l[1].1 == "healthy")
    );
    assert!(
        counters
            .iter()
            .any(|(_, l, _)| l[0].1 == "cache" && l[1].1 == "unhealthy")
    );

    let histograms = collected.histograms.lock().unwrap();
    // One histogram observation per check, labeled by name.
    assert_eq!(histograms.len(), 2, "expected one histogram per check");
    for (name, labels, value) in histograms.iter() {
        assert_eq!(name, "healthkit_check_duration_seconds");
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].0, "name");
        assert!(*value >= 0.0);
    }
    assert!(histograms.iter().any(|(_, l, _)| l[0].1 == "db"));
    assert!(histograms.iter().any(|(_, l, _)| l[0].1 == "cache"));
}
