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

/// Named-seat caller env (moves to `host::named::ENV_VARS` in N). Scrub-only in F.
pub(crate) const NAMED_SCRUB_ENV_VARS: &[&str] =
    &["HERDR_THREADS_SEAT_NAME", "HERDR_THREADS_SEAT_READONLY"];

pub(crate) use hc::{HC_TERMINAL_ID_ENV, HC_TERMINAL_SOCKET_ENV};

/// Basenames of herdr-core terminal processes (the `hc` client, the core). `LocateCaller`
/// treats one on the walk as a wall: a Herdr pane root above it does not verify. Moves to
/// `host::hc` in H1 (ht-kqrt.46).
pub fn foreign_terminal_process_names() -> &'static [&'static str] {
    &["herdr-core", "hc"]
}

/// The single env scrub list for every child the daemon side spawns (K5): the
/// union of each host family's names. `spawn_detached` and handoff spawns
/// remove all of it; the H3 gate shim removes it minus `host::hc::ENV_VARS`
/// (ht-kqrt.54). Never copy these names elsewhere.
pub fn child_env_scrub_list() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for name in PANE_SCOPED_ENV_VARS
        .iter()
        .chain(NAMED_SCRUB_ENV_VARS)
        .chain(hc::ENV_VARS)
    {
        if !names.contains(name) {
            names.push(name);
        }
    }
    names
}

pub mod compatibility;
pub mod hc;
pub mod native;
pub mod observation;
pub mod probe;
pub mod process_table;
pub mod target_creator;
mod transport;

pub mod continuity;
pub use transport::{WitnessedResponse, request_witnessed};
