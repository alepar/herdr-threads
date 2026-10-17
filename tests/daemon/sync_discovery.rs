//! ht-5fr.7: peer discovery end to end. Daemon A runs in this process with a
//! fake attached host (the production `run_elected_probed` composition) and a
//! fake `herdr` whose `machine list --json` names node B's state directory;
//! `sync.dial_command` resolves that target to B's `peer --stdio` pump (never
//! ssh). A must upsert the `herdr_machine` row, link to B and converge with it.
//! B is a spawned daemon, started by A's own dial and stopped by this test.
//!
//! The probe's `set_discovery` points the worker at the fake `herdr` and turns
//! the `herdr --remote` process scan off: the machine's real Herdr clients are
//! not this test's targets.

use herdr_threads::{
    app::{LaneProbe, SystemClock, run_elected_probed},
    daemon::lifecycle::test_owner_env,
    ports::{
        AgentComposerState, ComposerObservation, EnumerationEvidence, EvidenceKind,
        HostCallContext, HostObservation, HostPort, HostSnapshot, IncarnationEvidence,
        NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, PromptOutcome,
        SafeWakeTarget,
    },
    protocol::{
        ids::HostLocator,
        time::{Cancellation, Clock},
    },
    protocol::{
        ids::{HostBootId, HostTargetId, SeatId},
        results::ApiError,
    },
    service::config::ServiceConfig,
    test_support::{
        FakeHostWorld, RefusingHost,
        isolation::{SCRUBBED_PREFIXES, TestIsolation},
    },
};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const WAIT: Duration = Duration::from_secs(60);

fn db_path(state: &Path) -> PathBuf {
    state.join("node").join("herdr-threads.db")
}

/// A read-only view of a node's store; `None` while it is missing or busy.
fn open_db(state: &Path) -> Option<Connection> {
    let conn =
        Connection::open_with_flags(db_path(state), OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.busy_timeout(Duration::from_secs(5)).ok()?;
    Some(conn)
}

/// `(ssh_dest, source, enabled, state)` of every dial target of `state`.
fn targets(state: &Path) -> Vec<(String, String, i64, String)> {
    let Some(conn) = open_db(state) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT ssh_dest, source, enabled, state FROM sync_dial_targets ORDER BY target_id",
    ) else {
        return Vec::new();
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// `(node, incarnation, contiguous_counter, applied_through)` per held log.
fn logs(state: &Path) -> Vec<(String, i64, i64, i64)> {
    let Some(conn) = open_db(state) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT node_uuid, incarnation, contiguous_counter, applied_through \
         FROM sync_logs ORDER BY node_uuid, incarnation",
    ) else {
        return Vec::new();
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn wait_until<T>(what: &str, mut probe: impl FnMut() -> Option<T>, states: &[&Path]) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        if Instant::now() > deadline {
            let tails: String = states
                .iter()
                .map(|state| {
                    let text = fs::read_to_string(state.join("node").join("daemon.log"))
                        .unwrap_or_default();
                    format!("--- {}\n{text}\n", state.display())
                })
                .collect();
            panic!("timed out after {WAIT:?} waiting for {what}\n{tails}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A Herdr host that answers every enumeration with an empty, coherent
/// snapshot (so the session counts as reachable) and refuses everything else.
struct LiveHost(AtomicU64);

impl HostPort for LiveHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        RefusingHost.native_launch_capability()
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        RefusingHost.observe_current_target(target, context)
    }
    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        RefusingHost.observe_current_target_for_archival(target, context)
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        Ok(HostSnapshot {
            boot: HostBootId::new("boot-discovery"),
            epoch: 1,
            observation_sequence: self.0.fetch_add(1, Ordering::SeqCst) + 1,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "inc-discovery".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: Vec::new(),
        })
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        RefusingHost.safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        RefusingHost.submit_prompt(target, text, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        RefusingHost.launch_native(request, context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        RefusingHost.pane_agent_state(target, context)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        RefusingHost.send_submit_key(target, context)
    }
}

/// A fake `herdr` that answers `machine list --json` with one enabled machine
/// (`target`) and one disabled one, and refuses everything else.
fn write_fake_herdr(path: &Path, target: &Path) {
    let script = format!(
        "#!/bin/sh\n\
         if [ \"$*\" = \"machine list --json\" ]; then\n\
         printf '%s\\n' '[{{\"id\":\"1\",\"label\":\"b\",\"target\":\"{}\",\"session\":\"default\",\"enabled\":true,\"selected\":false,\"future\":1}},{{\"id\":\"2\",\"label\":\"off\",\"target\":\"disabled-box\",\"enabled\":false}}]'\n\
         exit 0\n\
         fi\n\
         exit 2\n",
        target.display()
    );
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// `sync.dial_command`: B's pump with a scrubbed environment. A runs in this
/// process, so its dial children would otherwise inherit this test's real
/// `HERDR_*` variables and HOME; B must see an isolated world and exit when
/// this process does.
fn dial_command(iso: &TestIsolation) -> String {
    let mut unset = String::new();
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if SCRUBBED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
        {
            unset.push_str(&format!("-u {key} "));
        }
    }
    let xdg = |name: &str| iso.path(format!("xdg/{name}")).display().to_string();
    let (owner_key, owner_pid) = test_owner_env();
    for name in ["config", "state", "runtime"] {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(xdg(name))
            .unwrap();
    }
    format!(
        "exec env {unset}HOME={home} XDG_CONFIG_HOME={c} XDG_STATE_HOME={s} \
         XDG_RUNTIME_DIR={r} {owner_key}={owner_pid} {BIN} --state-dir {{dest}} peer --stdio",
        home = iso.home().display(),
        c = xdg("config"),
        s = xdg("state"),
        r = xdg("runtime"),
    )
}

/// One in-process elected daemon (node A), stopped and joined on drop.
struct NodeA {
    stop: Cancellation,
    thread: Option<JoinHandle<std::io::Result<bool>>>,
    probe: LaneProbe,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl NodeA {
    fn start(iso: &TestIsolation, state: &Path, herdr: PathBuf, period: Duration) -> Self {
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(state)
            .unwrap();
        let paths = herdr_threads::test_support::resolve_seeded(state);
        let config = ServiceConfig::default()
            .with_dial_command(Some(&dial_command(iso)))
            .unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let probe = LaneProbe::default();
        probe.set_discovery(herdr, period);
        probe.log_lane_errors_to(iso.path("lanes.log"));
        let world = FakeHostWorld::new();
        let hint = HostLocator::parse("/herdr/discovery.sock").unwrap();
        world.add(&hint, Arc::new(LiveHost(AtomicU64::new(0))));
        let plan = world.plan(Some(hint));
        let stop = Cancellation::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (daemon_stop, daemon_probe) = (stop.clone(), probe.clone());
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected_probed(
                    &paths,
                    clock,
                    daemon_stop,
                    config,
                    plan,
                    daemon_probe,
                    move |descriptor| {
                        ready_tx
                            .send(descriptor.clone())
                            .map_err(std::io::Error::other)
                    },
                ))
        });
        ready_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("the elected daemon did not publish its endpoint");
        std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
        Self {
            stop,
            thread: Some(thread),
            probe,
            _guard: guard,
        }
    }

    fn shut_down(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .expect("daemon thread")
                .expect("daemon drained cleanly");
        }
    }
}

impl Drop for NodeA {
    fn drop(&mut self) {
        self.shut_down();
    }
}

/// `daemon stop` for B, then wait until it withdraws its endpoint.
fn stop_b(iso: &TestIsolation, state: &Path) {
    let _ = iso
        .command(BIN)
        .env("PATH", "/usr/bin:/bin")
        .arg("--state-dir")
        .arg(state)
        .args(["daemon", "stop"])
        .output();
    let endpoint = state.join("node").join("endpoint.json");
    let deadline = Instant::now() + Duration::from_secs(20);
    while endpoint.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn fake_herdr_machine_target_discovered_linked_and_converged() {
    let iso = TestIsolation::new("sync-discovery");
    let (a, b) = (iso.path("a"), iso.path("b"));
    let herdr = iso.path("fake-herdr");
    write_fake_herdr(&herdr, &b);
    let mut node_a = NodeA::start(&iso, &a, herdr, Duration::from_millis(200));
    assert!(
        node_a.probe.host_sessions().unwrap().any_reachable(),
        "the fake host is attached and reachable, so the worker polls: attached={}",
        node_a.probe.host_sessions().unwrap().attached_count()
    );
    // The worker upserts the enabled machine, as a herdr_machine row, and not
    // the disabled one.
    let found = wait_until(
        "A to discover B's target",
        || Some(targets(&a)).filter(|rows| !rows.is_empty()),
        &[&a, &b],
    );
    assert_eq!(found[0].0, b.display().to_string());
    assert_eq!(found[0].1, "herdr_machine");
    assert_eq!(found[0].2, 1);
    assert!(
        found.iter().all(|row| row.0 != "disabled-box"),
        "a disabled machine is not a target: {found:?}"
    );
    // The reload (not a restart) made the Dialer dial it: linked, then both
    // stores hold both genesis logs, fully applied.
    wait_until(
        "the discovered target to link",
        || {
            targets(&a)
                .iter()
                .any(|row| row.3 == "linked")
                .then_some(())
        },
        &[&a, &b],
    );
    let held = wait_until(
        "both stores to converge",
        || {
            let (la, lb) = (logs(&a), logs(&b));
            (la.len() == 2 && la == lb && la.iter().all(|l| l.2 == l.3)).then_some(la)
        },
        &[&a, &b],
    );
    assert_eq!(held.len(), 2, "one log per node: {held:?}");
    // Later passes add no duplicate rows.
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(targets(&a).len(), 1, "{:?}", targets(&a));
    // A first: with the dialer gone B sees EOF and nothing restarts it.
    node_a.shut_down();
    stop_b(&iso, &b);
}
