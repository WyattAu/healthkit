// Bench/property tests use fixed and hostile inputs directly; unwrap/expect,
// slicing, and panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use healthkit::CheckResult;
use healthkit::{HealthRegistry, HealthStatus};
use proptest::prelude::*;
use std::time::Duration;

proptest! {
    #[test]
    fn health_status_is_healthy_matches_display(status in arb_health_status()) {
        match status {
            HealthStatus::Healthy => {
                prop_assert!(status.is_healthy());
                prop_assert_eq!(status.to_string(), "healthy");
            }
            HealthStatus::Degraded => {
                prop_assert!(!status.is_healthy());
                prop_assert!(status.is_ready());
                prop_assert_eq!(status.to_string(), "degraded");
            }
            HealthStatus::Unhealthy => {
                prop_assert!(!status.is_healthy());
                prop_assert!(!status.is_ready());
                prop_assert_eq!(status.to_string(), "unhealthy");
            }
        }
    }

    #[test]
    fn health_status_is_ready_consistency(status in arb_health_status()) {
        prop_assert_eq!(status.is_ready(), !matches!(status, HealthStatus::Unhealthy));
    }

    #[test]
    fn check_result_name_non_empty(name in "[a-z]{1,50}") {
        let result = CheckResult {
            name: name.clone(),
            status: HealthStatus::Healthy,
            message: None,
            duration: Duration::from_millis(0),
        };
        prop_assert_eq!(&result.name, &name);
    }

    #[test]
    fn check_result_message_roundtrip(
        name in "[a-z]{1,20}",
        msg in "[a-z ]{0,100}",
    ) {
        let message = if msg.is_empty() { None } else { Some(msg.clone()) };
        let result = CheckResult {
            name,
            status: HealthStatus::Healthy,
            message: message.clone(),
            duration: Duration::from_millis(1),
        };
        prop_assert_eq!(&result.message, &message);
    }

    #[test]
    fn registry_empty_check_all_returns_empty(_dummy in 0..1u32) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: Result<(), TestCaseError> = rt.block_on(async {
            let registry = HealthRegistry::new();
            let results = registry.check_all().await;
            prop_assert!(results.is_empty());
            Ok(())
        });
        result.unwrap();
    }
}

fn arb_health_status() -> impl Strategy<Value = HealthStatus> {
    prop_oneof![
        Just(HealthStatus::Healthy),
        Just(HealthStatus::Degraded),
        Just(HealthStatus::Unhealthy),
    ]
}
