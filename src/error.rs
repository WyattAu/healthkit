/// Errors that can occur during health checks.
#[derive(Debug, thiserror::Error)]
pub enum HealthCheckError {
    /// A health check failed with a descriptive message.
    #[error("health check failed: {0}")]
    CheckFailed(String),

    /// A health check panicked during execution.
    #[error("health check panicked: {0}")]
    CheckPanicked(String),

    /// A health check timed out.
    #[error("health check timed out after {0:?}")]
    CheckTimedOut(std::time::Duration),

    /// A required dependency is unavailable.
    #[error("dependency unavailable: {0}")]
    DependencyUnavailable(String),

    /// The service is shutting down.
    #[error("service is shutting down")]
    ShuttingDown,
}
