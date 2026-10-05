//! Domain service composition: configuration, dispatch and workers.
pub mod archival;
pub mod config;
pub mod dispatch;
pub mod fair_writer;
pub mod host_evidence;
pub mod host_reachability;
pub mod kicks;
pub mod live_gate;
pub mod pacer;
pub mod workers;

#[cfg(test)]
#[path = "../../tests/service/crash_matrix.rs"]
mod crash_matrix;

#[cfg(test)]
#[path = "../../tests/service/loop_inventory.rs"]
mod loop_inventory;
