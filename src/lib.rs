/// Test-only crash-boundary hook. In ordinary builds (no `cfg(test)`, no
/// `test-support` feature) this expands to nothing: neither the name nor the
/// scope expression is compiled, and no request field can reach it.
macro_rules! failpoint {
    ($name:literal, $scope:expr) => {{
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::failpoints::hit($name, &$scope)?;
    }};
    ($name:literal, $scope:expr, connection = $conn:expr) => {{
        #[cfg(any(test, feature = "test-support"))]
        crate::test_support::failpoints::hit_connection($name, &$scope, $conn)?;
    }};
}

pub mod app;
pub mod archival_legacy;
pub mod cli;
pub mod client;
pub mod daemon;
pub mod harness;
pub mod host;
pub mod identity;
pub mod notification;
pub mod ports;
pub mod protocol;
pub mod scheduler;
pub mod service;
pub mod store;
pub mod summary;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod view;
