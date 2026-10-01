//! Domain service composition: configuration, dispatch and workers.
pub mod config;
pub mod dispatch;
pub mod host_evidence;
pub mod workers;

#[cfg(test)]
#[path = "../../tests/service/crash_matrix.rs"]
mod crash_matrix;
