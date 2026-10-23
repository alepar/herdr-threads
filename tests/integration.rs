//! Cross-tree integration sweep (ht-4is.12): the installed executable and its
//! detached daemon against a private Herdr protocol endpoint. No model runs;
//! stand-in seats use the cooperative caller flags exactly as the isolated
//! host suite (`tests/native/recovery`) does.
/// A spawned binary with every inherited HERDR_/CLAUDE/CODEX variable removed
/// (ht-p03.24); a test sets the variables it needs after this call.
pub(crate) fn scrubbed_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
}

/// One in-process daemon at a time across the whole integration binary: the
/// elected daemon redirects the process-wide stdout/stderr descriptors and
/// installs a quiet panic hook, so two of them running in parallel (lane_wiring
/// and lanes_latency) leave the binary's own output pointing at a daemon log
/// and lose the test summary (ht-p03.26).
#[cfg(feature = "test-support")]
pub(crate) static ONE_DAEMON: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// ht-p03.21: configuration smoke across the Herdr up/down states.
#[cfg(feature = "test-support")]
#[path = "integration/config_smoke.rs"]
mod config_smoke;
/// ht-p03.10: fake other-version daemon, skew detection and the remedy round trip.
#[path = "daemon/skew.rs"]
mod daemon_skew;
/// ht-p03.48: doctor --json Claude admission fields and the canary's t0.admission
/// verdict on the same output.
#[cfg(feature = "test-support")]
#[path = "integration/doctor_admission_seam.rs"]
mod doctor_admission_seam;
/// ht-4is.8.17: `read --follow`, IRC style for a person, JSON lines otherwise.
#[path = "integration/follow.rs"]
mod follow;
/// ht-xoc.7: harness version evidence end to end with a stand-in harness
/// (silent unlisted versions, verified-by-use, manifest-driven broken lines,
/// opt-out and offline paths, unattributable and malformed payloads).
#[cfg(feature = "test-support")]
#[path = "integration/harness_version_evidence.rs"]
mod harness_version_evidence;
/// ht-p03.11: Herdr stopped under a running daemon leaves one first line and
/// capped summaries in daemon.log.
#[cfg(feature = "test-support")]
#[path = "integration/herdr_down_log.rs"]
mod herdr_down_log;
/// End-to-end smoke against one official Herdr release; run per release by
/// `scripts/herdr-release-smoke`.
#[cfg(feature = "test-support")]
#[path = "integration/herdr_release_smoke.rs"]
mod herdr_release_smoke;
/// ht-p03.10: never-started and stale-socket Herdr report "server not running".
#[cfg(feature = "test-support")]
#[path = "integration/herdr_states.rs"]
mod herdr_states;
/// ht-4ne.3: bare Hermes bridge enrollment, the installed bridge driven with
/// synthetic callbacks against an isolated Herdr session and daemon.
#[cfg(feature = "test-support")]
#[path = "integration/hermes_enrollment_smoke.rs"]
mod hermes_enrollment_smoke;
/// ht-4ne.5: Hermes global enrollment main flow end to end through the real
/// binary, plus the registry-wide enrollment policy check.
#[cfg(feature = "test-support")]
#[path = "integration/hermes_enrollment_sweep.rs"]
mod hermes_enrollment_sweep;
/// ht-5fr.2.8: the daemon composes HostSessions: startup attach, sweep, stop.
#[cfg(feature = "test-support")]
#[path = "integration/host_sessions_daemon.rs"]
mod host_sessions_daemon;
/// ht-p03.1: the isolated named Herdr test-session fixture's self-test.
#[cfg(feature = "test-support")]
#[path = "integration/isolated_herdr_fixture.rs"]
mod isolated_herdr_fixture;
/// ht-p03.40: the lane wiring across its seams (registration, commit origin,
/// failure and recovery reporting, idle cost) through the production daemon.
#[cfg(feature = "test-support")]
#[path = "integration/lane_wiring.rs"]
mod lane_wiring;
/// ht-p03.9.4: deadline and wake lane latency, idleness and outage cost
/// through the daemon request path on an isolated Herdr session.
#[cfg(feature = "test-support")]
#[path = "integration/lanes_latency.rs"]
mod lanes_latency;
/// ht-4is.6.7: read-only latency under a multi-agent party with a slow host.
#[path = "integration/latency.rs"]
mod latency;
/// ht-j16.10: the Claude mod delivery flows end to end (real `watch` CLI, scripted
/// mod driver, injected daemon clock, stand-in Herdr).
#[cfg(feature = "test-support")]
#[path = "integration/mod_delivery.rs"]
mod mod_delivery;
/// ht-5fr.2.12: dispatch by claim host over two fake Herdr servers: lazy attach, per-server
/// wake workers and outage freeze, C1 across servers, and the hook in a pane of a second server.
#[cfg(feature = "test-support")]
#[path = "daemon/multi_host.rs"]
mod multi_host;
/// ht-p03.131: no test-spawned process outlives its run (panic, SIGTERM, SIGKILL).
#[cfg(feature = "test-support")]
#[path = "integration/no_leaks.rs"]
mod no_leaks;
/// ht-5fr.1.11: the node layout end to end (store-only start, layout modes, legacy tree
/// untouched, restart identity, key-missing remedy) through the real binary.
#[cfg(feature = "test-support")]
#[path = "daemon/node_layout.rs"]
mod node_layout;
/// ht-p03.9.5: the observation lane's backoff and quiet Herdr-down, against an
/// isolated Herdr stopped and restarted under the running lane.
#[cfg(feature = "test-support")]
#[path = "integration/observation_outage.rs"]
mod observation_outage;
/// ht-p03.46: B3 operator text (remedy() and log-path accessors) across ensure,
/// exit 3, Health and doctor.
#[cfg(feature = "test-support")]
#[path = "integration/operator_text.rs"]
mod operator_text;
/// ht-4is.8.9/ht-4is.8.10: a person's own pane identity (`me init`) and
/// `--pane` names, through the installed executable and the same private
/// Herdr endpoint.
#[path = "integration/operator_ux.rs"]
mod operator_ux;
/// ht-54r.14: pane-close retirement and restart restore holds with several session bindings
/// on one seat, on an isolated Herdr session.
#[cfg(feature = "test-support")]
#[path = "integration/pane_close_retirement.rs"]
mod pane_close_retirement;
/// ht-5fr.5.9: `peer --stdio`, the stateless ssh-side pump, against a daemon it starts.
#[cfg(feature = "test-support")]
#[path = "cli/peer_stdio.rs"]
mod peer_stdio;
/// ht-piggyback: the pending-count line on a hookless caller's text commands, on an isolated
/// Herdr session.
#[cfg(feature = "test-support")]
#[path = "integration/pending_hint.rs"]
mod pending_hint;
/// ht-54r.25: session-binding flows the smoke configurations do not assert (settle once, one-shot).
#[cfg(feature = "test-support")]
#[path = "integration/session_bindings_e2e.rs"]
mod session_bindings_e2e;
/// ht-54r.19: the nine session-binding configurations end to end (real daemon, isolated Herdr).
#[cfg(feature = "test-support")]
#[path = "integration/session_bindings_smoke.rs"]
mod session_bindings_smoke;
#[path = "integration/session_bindings_subagent.rs"]
mod session_bindings_subagent;
/// ht-54r.5: agent callers are attributed from the harness environment, the real
/// binary under a synthetic harness parent.
#[cfg(feature = "test-support")]
#[path = "integration/session_claims.rs"]
mod session_claims;
/// Real versionless setup output/configuration through the live canary consumer.
#[cfg(feature = "test-support")]
#[path = "integration/setup_canary_seam.rs"]
mod setup_canary_seam;
/// ht-1ip.16: the summary flow (job leasing, catch-up, hold, deadline
/// extension, recovery text) on a real daemon, driven through the CLI.
#[path = "integration/summary_flow.rs"]
mod summary_flow;
#[path = "integration/sweep.rs"]
mod sweep;
/// ht-p03.51: the root integration sweep's cross-bucket paths (send-to-wake
/// latency, retention under load, all-lane failure, operator text across
/// startup failure, lane failure and skew).
#[cfg(feature = "test-support")]
#[path = "integration/sweep_remaining.rs"]
mod sweep_remaining;
/// ht-5fr.8: the operator and status CLI for sync (`peer`, `sync ...`) against real daemons.
#[cfg(feature = "test-support")]
#[path = "cli/sync_cli.rs"]
mod sync_cli;
/// ht-5fr.7: peer discovery: a fake `herdr machine list` target is upserted, linked and
/// converged through `sync.dial_command`.
#[cfg(feature = "test-support")]
#[path = "daemon/sync_discovery.rs"]
mod sync_discovery;
/// ht-5fr.5.10: two real daemons linked through `sync.dial_command`: genesis exchange,
/// convergence, the composed Apply lane, backoff and reconnect, dial-child reaping.
#[cfg(feature = "test-support")]
#[path = "daemon/sync_link.rs"]
mod sync_link;
/// ht-4is.8.18: compact agent-facing machine text (short cursors, one line
/// per message, no page blobs), measured through the installed executable.
#[path = "integration/token_diet.rs"]
mod token_diet;
/// ht-rzi.8: B5 trust-policy guards end to end, with a stand-in Herdr that
/// restarts as its own process (a new incarnation).
#[path = "integration/trust_policy.rs"]
mod trust_policy;
/// ht-p03.44: capability-gated `full_bodies` and hook parse-failure report across
/// the CLI/daemon seam: old and new daemon, old CLI frames, skewed protocol_version.
#[path = "integration/wire_compat.rs"]
mod wire_compat;

#[path = "integration/channel_activation.rs"]
mod channel_activation;

/// README named handoffs and conversation on a real private daemon; model-free.
#[cfg(feature = "test-support")]
#[path = "integration/readme_tryout.rs"]
mod readme_tryout;

/// Registry values through real executable/canonical consumers; synthetic host/callbacks.
#[cfg(feature = "test-support")]
#[path = "integration/harness_adapter_wiring.rs"]
mod harness_adapter_wiring;
#[cfg(all(
    feature = "test-support",
    any(target_os = "macos", target_os = "linux")
))]
#[path = "integration/topology_activation.rs"]
mod topology_activation;
