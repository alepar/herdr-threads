//! One test binary for the suites that keep no process-wide state: each
//! test spawns children or calls the library, and none redirects stdio,
//! installs a panic hook, runs an in-process daemon, arms failpoints or
//! changes the umask. Every test target links the whole library, so one
//! binary instead of six saves their links (docs/dev/build-speed.md).
//!
//! The suites stay in their own files; a test is named `<suite>::<test>`
//! (`cargo test --all-features --test combined setup_cli::`). A suite that
//! changes process-wide state gets its own `[[test]]` target instead, like
//! `hook_entrypoint`, `integration`, `package` and `service`.

#[path = "harness/adapter_smoke.rs"]
mod adapter_smoke;
#[path = "store/archival_legacy.rs"]
mod archival_legacy;
#[path = "store/archival_worker.rs"]
mod archival_worker;
#[path = "canary_manifest_writer.rs"]
mod canary_manifest_writer;
#[path = "store/channel_archival.rs"]
mod channel_archival;
#[path = "contracts.rs"]
mod contracts;
#[path = "store/handoff_fences.rs"]
mod handoff_fences;
#[path = "handoff_topology_authority.rs"]
mod handoff_topology_authority;
#[path = "handoff_topology_cli.rs"]
mod handoff_topology_cli;
#[path = "host_adapter.rs"]
mod host_adapter;
#[path = "cli/inbox_continuation_context.rs"]
mod inbox_continuation_context;
#[path = "installer_integrations.rs"]
mod installer_integrations;
#[path = "store/lazy26_prerequisite.rs"]
mod lazy26_prerequisite;
#[path = "protocol/lazy_delivery.rs"]
mod lazy_delivery;
#[path = "cli/lazy_recovery_context.rs"]
mod lazy_recovery_context;
#[path = "store/lazy_schema.rs"]
mod lazy_schema;
#[path = "cli/lazy_send.rs"]
mod lazy_send;
#[path = "lifecycle_ux.rs"]
mod lifecycle_ux;
#[path = "local_endpoint.rs"]
mod local_endpoint;
#[path = "cli/read_picker_pty.rs"]
mod read_picker_pty;
#[path = "setup_cli.rs"]
mod setup_cli;
#[path = "store/topology_handoff.rs"]
mod topology_handoff;
#[path = "view.rs"]
mod view;

/// `autotests = false` (Cargo.toml) means a new top-level `tests/*.rs` file
/// is not built at all until it is listed: each one must be a `[[test]]` root
/// or a suite included above.
#[test]
fn every_top_level_test_file_is_built() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");
    let combined = include_str!("combined.rs");
    let mut unbuilt = Vec::new();
    for entry in std::fs::read_dir(root.join("tests")).expect("tests/") {
        let name = entry.expect("tests/ entry").file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".rs") {
            continue;
        }
        let target = format!("path = \"tests/{name}\"");
        let suite = format!("#[path = \"{name}\"]");
        if !manifest.contains(&target) && !combined.contains(&suite) {
            unbuilt.push(name.into_owned());
        }
    }
    assert!(
        unbuilt.is_empty(),
        "tests/ files built by no target (add a [[test]] entry or a suite in tests/combined.rs): {unbuilt:?}"
    );
}

#[path = "store/lazy_publication.rs"]
mod lazy_publication;

#[path = "store/lazy_metadata.rs"]
mod lazy_metadata;

#[path = "store/lazy_inbox.rs"]
mod lazy_inbox;
#[path = "protocol/lazy_inbox_v2.rs"]
mod lazy_inbox_v2;

#[path = "cli/lazy_markers.rs"]
mod lazy_markers;

#[path = "cli/lazy_display.rs"]
mod lazy_display;

#[path = "cli/lazy_settlement.rs"]
mod lazy_settlement;

#[path = "integration/lazy_attention.rs"]
mod lazy_attention;

#[path = "integration/notice_wake.rs"]
mod notice_wake;

#[path = "integration/lazy_config_smoke.rs"]
mod lazy_config_smoke;

#[path = "integration/lazy_sweep.rs"]
mod lazy_sweep;

#[path = "cli/readme_commands.rs"]
mod readme_commands;
