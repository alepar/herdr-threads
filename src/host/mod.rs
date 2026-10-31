//! Herdr host boundary: the production `NativeCli` HostPort (Herdr 0.9.1 or
//! newer) and its observation parsing.

/// Variables Herdr sets for one pane, or for one plugin action or startup
/// command, that locate that caller: Herdr gives a plugin command the
/// *focused* pane as `HERDR_PANE_ID`, and the `herdr` CLI takes its caller
/// pane from it. A process that outlives its caller (the daemon) is spawned
/// without them. Instance routing (`HERDR_SOCKET_PATH`, `HERDR_SESSION`,
/// `HERDR_CONFIG_PATH`, `HERDR_PLUGIN_STATE_DIR`) is not caller-scoped and is
/// kept.
pub const PANE_SCOPED_ENV_VARS: &[&str] = &[
    "HERDR_ENV",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_PLUGIN_ACTION_ID",
    "HERDR_PLUGIN_EVENT",
    "HERDR_PLUGIN_EVENT_JSON",
    "HERDR_PLUGIN_CONTEXT_JSON",
    "HERDR_PLUGIN_CLICKED_URL",
    "HERDR_PLUGIN_LINK_HANDLER_ID",
];

pub mod compatibility;
pub mod native;
pub mod observation;
pub mod process_table;
mod transport;

pub mod continuity;
pub use transport::{WitnessedResponse, request_witnessed};
