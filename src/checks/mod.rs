//! Production dependency checks for common backends.
//!
//! Each check integrates with [`HealthRegistry`](crate::HealthRegistry) via
//! its `register` helper, which wraps the check into the closure shape that
//! `add_check` expects:
//!
//! ```rust,ignore
//! let check = SqlxCheck::new(pool, Duration::from_secs(2), 500);
//! check.register(&registry, "database");
//! ```

#[cfg(feature = "redis")]
pub mod redis;

#[cfg(feature = "sqlx")]
pub mod sqlx;
