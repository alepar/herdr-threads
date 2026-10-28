//! Domain service composition: configuration, dispatch and workers.
pub mod archival;
pub mod config;
pub mod delivery_presentation;
pub mod delivery_pull;
pub mod dispatch;
pub mod fair_writer;
pub mod host_evidence;
pub mod host_reachability;
pub mod host_sessions;
pub mod kicks;
pub mod live_gate;
pub mod liveness;
pub mod mod_channels;
pub mod pacer;
pub mod remote_resolver;
pub mod seat_publisher;
pub mod sync_admin;
pub mod workers;

#[cfg(test)]
#[path = "../../tests/service/crash_matrix.rs"]
mod crash_matrix;

#[cfg(test)]
#[path = "../../tests/service/loop_inventory.rs"]
mod loop_inventory;
