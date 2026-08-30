use serde::Serialize;
use std::fmt;
use std::time::Duration;

/// The status of a health check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// The component is fully operational.
    Healthy,
    /// The component is operational but degraded.
    Degraded,
    /// The component is not operational.
    Unhealthy,
}

impl HealthStatus {
    /// Returns `true` if the status is `Healthy`.
    pub fn is_healthy(self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }

    /// Returns `true` if the component is considered ready (healthy or degraded).
    pub fn is_ready(self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// The result of a single health check.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Name of the component that was checked.
    pub name: String,
    /// The health status.
    pub status: HealthStatus,
    /// Optional message providing more detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// How long the check took.
    pub duration: Duration,
}

/// Response body for liveness probes.
#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
}

/// Response body for readiness probes.
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessResponse {
    pub status: HealthStatus,
    pub checks: Vec<CheckResult>,
}

/// Response body for liveness probes.
#[derive(Debug, Clone, Serialize)]
pub struct LivenessResponse {
    pub status: HealthStatus,
}
