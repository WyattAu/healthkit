// Bench/property tests use fixed and hostile inputs directly; unwrap/expect,
// slicing, and panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use criterion::{Criterion, criterion_group, criterion_main};
use healthkit::{HealthCheckError, HealthRegistry, HealthStatus};

fn bench_registry_new(c: &mut Criterion) {
    c.bench_function("registry_new", |b| {
        b.iter(|| {
            let registry = HealthRegistry::new();
            std::hint::black_box(registry);
        });
    });
}

fn bench_add_check(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    c.bench_function("add_check", |b| {
        b.iter(|| {
            let registry = HealthRegistry::new();
            registry.add_check("check", || async { Ok(HealthStatus::Healthy) });
            std::hint::black_box(registry);
        });
    });
    drop(rt);
}

async fn run_check_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("check_all");

    for num_checks in [1, 5, 20] {
        group.bench_function(format!("{} checks", num_checks), |b| {
            b.iter(|| async {
                let registry = HealthRegistry::new();
                for i in 0..num_checks {
                    let name = format!("check_{}", i);
                    registry.add_check(&name, || async { Ok(HealthStatus::Healthy) });
                }
                let results = registry.check_all().await;
                std::hint::black_box(results);
            });
        });
    }

    group.finish();
}

async fn run_check_all_with_failures(c: &mut Criterion) {
    c.bench_function("check_all_with_failures", |b| {
        b.iter(|| async {
            let registry = HealthRegistry::new();
            registry.add_check("ok", || async { Ok(HealthStatus::Healthy) });
            registry.add_check("degraded", || async { Ok(HealthStatus::Degraded) });
            registry.add_check("failing", || async {
                Err(HealthCheckError::CheckFailed("oops".into()))
            });
            let results = registry.check_all().await;
            std::hint::black_box(results);
        });
    });
}

fn bench_benchmark_health(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        run_check_all(c).await;
        run_check_all_with_failures(c).await;
    });
}

criterion_group!(
    benches,
    bench_registry_new,
    bench_add_check,
    bench_benchmark_health,
);
criterion_main!(benches);
