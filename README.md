# healthkit

Health check endpoints for Rust — liveness, readiness, and startup probes with dependency checking for Kubernetes and Docker.

[![Crates.io](https://img.shields.io/crates/v/healthkit.svg)](https://crates.io/crates/healthkit)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](./LICENSE-MIT)

## Purpose

`healthkit` provides a simple, composable API for registering and executing health checks in Rust services. It integrates with Kubernetes probes and includes an optional Axum integration for HTTP endpoints.

## Features

- **Liveness probes** — verify the process is running and not deadlocked
- **Readiness probes** — verify the service can accept traffic
- **Startup probes** — verify initialization is complete
- **Dependency checking** — register custom checks for databases, caches, etc.
- **Axum integration** — ready-to-use route handlers (default feature)
- **No unsafe code** — `#![forbid(unsafe_code)]`

## Kubernetes Probe Configuration

```yaml
apiVersion: v1
kind: Pod
spec:
  containers:
  - name: app
    livenessProbe:
      httpGet:
        path: /healthz
        port: 3000
      initialDelaySeconds: 5
      periodSeconds: 10
    readinessProbe:
      httpGet:
        path: /readyz
        port: 3000
      initialDelaySeconds: 5
      periodSeconds: 5
    startupProbe:
      httpGet:
        path: /startupz
        port: 3000
      failureThreshold: 30
      periodSeconds: 2
```

## Usage

```rust
use axum::Router;
use healthkit::{HealthRegistry, HealthStatus};
use healthkit::axum::{liveness_route, readiness_route, startup_route};

#[tokio::main]
async fn main() {
    let mut registry = HealthRegistry::new();

    // Register a database check
    registry.add_check("database", || async {
        // Perform actual database connectivity check
        HealthStatus::Healthy
    });

    // Register a cache check
    registry.add_check("cache", || async {
        HealthStatus::Healthy
    });

    let app = Router::new()
        .route("/healthz", liveness_route())
        .merge(readiness_route(registry.clone()))
        .merge(startup_route(registry));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

## Security

Threat model: [THREAT-MODEL.md](THREAT-MODEL.md).
