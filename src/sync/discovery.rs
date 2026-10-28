//! Peer discovery: dial targets found from Herdr (spec §11).
//!
//! Responsibility (DiscoveryWorker)
//! - Owns: the 60 s discovery cadence (a `Pacer` on the injected clock) while a Herdr host is attached, the
//!   `herdr machine list --json` call (bounded, killed on drop) and its
//!   tolerant parse, the `herdr --remote <target>` client scan, the choice of
//!   which found destinations are worth upserting, the one-line-per-hour log
//!   of a failing source, and [`DiscoveryStatus`] (what the last pass saw,
//!   including `herdr machine list` being unavailable).
//! - Hides: how `herdr` is spawned, the JSON shape and the process scan.
//! - Must not: write a table directly (rows go through [`DiscoveryStore`]),
//!   dial or judge a target (the Dialer does), gate on a Herdr version (a
//!   missing or unparsable `machine list` degrades only that source), or touch
//!   explicit targets and rows already found (`upsert_discovered` leaves an
//!   existing destination as it is, so `sync.discover = false` and a
//!   forgotten row stay as they are).
//! - Errors: none escape a pass; every failure is logged (rate limited) and the
//!   next pass tries again.
//! - Role: background worker of the elected daemon, composed next to the sync
//!   runtime in `app::run_elected_impl`. After a pass that added rows it asks
//!   the Dialer to reload (`SyncHandle::reload_targets`). A new discovery
//!   source is a new field of [`Found`] and one more branch in the pass.
//!
//! `herdr machine list --json` prints a JSON array of saved SSH machines:
//! `{"id", "label", "target", "session", "enabled", "selected"}`. Only `target`
//! (a string) and `enabled` (a bool, default true when absent) are read;
//! other fields and entries without a usable `target` are ignored.

use std::{
    ffi::OsString,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::Deserialize;

use crate::{
    protocol::{
        results::ApiError,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::pacer::{Pacer, Wake},
    store::sync_dial_targets::{DiscoverySource, MAX_TARGET_TEXT},
};

/// Time between passes.
pub const POLL_PERIOD: Duration = Duration::from_secs(60);
/// Longest a `herdr machine list` call runs.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest the store write of one pass waits.
const STORE_BUDGET_MS: u64 = 5_000;
/// Smallest spacing of the log line for one failing source.
const LOG_EVERY: Duration = Duration::from_secs(3600);

/// A boxed future, for the injectable async source.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// One saved machine of `herdr machine list --json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub target: String,
    pub enabled: bool,
}

/// `herdr machine list --json` printed something that is not a machine list.
#[derive(Debug)]
pub struct MachineListError(String);

impl std::fmt::Display for MachineListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "machine list is not a JSON array: {}", self.0)
    }
}

#[derive(Deserialize)]
struct RawMachine {
    target: Option<String>,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

/// Parses `herdr machine list --json`. Unknown fields are ignored, and so is an
/// entry that is not an object with a non-empty string `target`; only a
/// document that is not a JSON array is an error.
pub fn parse_machine_list(json: &[u8]) -> Result<Vec<Machine>, MachineListError> {
    let entries: Vec<serde_json::Value> =
        serde_json::from_slice(json).map_err(|error| MachineListError(error.to_string()))?;
    Ok(entries
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<RawMachine>(entry).ok())
        .filter_map(|raw| {
            let target = raw.target?;
            (!target.is_empty()).then_some(Machine {
                target,
                enabled: raw.enabled,
            })
        })
        .collect())
}

/// What one `herdr machine list` call came to.
#[derive(Debug, PartialEq, Eq)]
pub enum ListOutcome {
    Machines(Vec<Machine>),
    /// No `herdr` to run, or one without `machine list` (exit 127, unknown
    /// subcommand): shown as not available, retried at the normal cadence.
    NotAvailable,
    /// Any other failure, with a short reason for the log.
    Failed(String),
}

/// The `herdr machine list` source.
pub trait MachineLister: Send + Sync {
    fn list(&self) -> BoxFuture<ListOutcome>;
}

/// Runs `<program> machine list --json`.
pub struct HerdrCli {
    program: OsString,
}

impl HerdrCli {
    /// `program` is `herdr` looked up on the daemon's PATH, or a path.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// `$HERDR_BIN_PATH` when set, else `herdr` on PATH. A `test-support`
    /// build lets `HT_TEST_MACHINE_LIST_PROGRAM` name the program instead, so a
    /// test daemon never lists the machines of the real `herdr` on PATH and
    /// dials them (`TestIsolation::command` points it at nothing).
    pub fn from_env() -> Self {
        #[cfg(any(test, feature = "test-support"))]
        {
            Self::new(crate::test_support::isolation::discovery_program())
        }
        #[cfg(not(any(test, feature = "test-support")))]
        Self::new(std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into()))
    }
}

impl MachineLister for HerdrCli {
    fn list(&self) -> BoxFuture<ListOutcome> {
        let program = self.program.clone();
        Box::pin(async move {
            let mut command = tokio::process::Command::new(&program);
            command
                .args(["machine", "list", "--json"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            let output = match tokio::time::timeout(LIST_TIMEOUT, command.output()).await {
                Err(_) => return ListOutcome::Failed("timed out".into()),
                Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                    return ListOutcome::NotAvailable;
                }
                Ok(Err(error)) => return ListOutcome::Failed(format!("spawn: {error}")),
                Ok(Ok(output)) => output,
            };
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.code() == Some(127)
                    || stderr.contains("unrecognized subcommand")
                    || stderr.contains("unknown subcommand")
                {
                    return ListOutcome::NotAvailable;
                }
                let first = stderr.lines().next().unwrap_or("").trim();
                return ListOutcome::Failed(format!(
                    "{}: {}",
                    output.status,
                    first.chars().take(120).collect::<String>()
                ));
            }
            match parse_machine_list(&output.stdout) {
                Ok(machines) => ListOutcome::Machines(machines),
                Err(error) => ListOutcome::Failed(error.to_string()),
            }
        })
    }
}

/// What a pass found, by source. A destination found twice keeps its first
/// (machine) source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub machines: Vec<String>,
    pub remote_clients: Vec<String>,
}

impl Found {
    /// The destinations to upsert, deduplicated, each with the source that
    /// found it. A destination the target table would refuse (empty, control
    /// characters, a leading `-`, over the length bound) is dropped here, so
    /// one odd entry never costs the others.
    pub fn upserts(&self) -> Vec<(String, DiscoverySource)> {
        let mut out: Vec<(String, DiscoverySource)> = Vec::new();
        let tagged = self
            .machines
            .iter()
            .map(|dest| (dest, DiscoverySource::HerdrMachine))
            .chain(
                self.remote_clients
                    .iter()
                    .map(|dest| (dest, DiscoverySource::HerdrRemoteProcess)),
            );
        for (dest, source) in tagged {
            let usable = !dest.is_empty()
                && !dest.starts_with('-')
                && dest.chars().count() <= MAX_TARGET_TEXT
                && !dest.chars().any(char::is_control);
            if usable && !out.iter().any(|(known, _)| known == dest) {
                out.push((dest.clone(), source));
            }
        }
        out
    }
}

/// How many rows one upsert pass added and retired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpsertTally {
    pub inserted: usize,
    /// Over the target cap.
    pub skipped: usize,
    /// Never-linked process-scan rows whose client is gone (ht-hvyf).
    pub retired: usize,
}

/// The store side of discovery: one transaction upserting `found` and, with
/// `retire_unseen`, deleting the never-linked `herdr_remote_process` rows
/// whose destination is not in `found`.
pub trait DiscoveryStore: Send + Sync {
    fn upsert_discovered(
        &self,
        found: &[(String, DiscoverySource)],
        retire_unseen: bool,
        budget: &CallBudget,
    ) -> Result<UpsertTally, ApiError>;
}

/// What the last pass saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryState {
    /// No pass has run.
    Starting,
    /// `sync.discover = false`.
    Disabled,
    /// No Herdr host is attached (or none reachable): nothing was polled.
    NoHost,
    /// `herdr machine list` is not available on this machine.
    NotAvailable,
    /// The last pass ran; `machine list` may have failed (logged).
    Running,
}

impl DiscoveryState {
    /// The compact token the read surfaces print.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Disabled => "disabled",
            Self::NoHost => "no_host",
            Self::NotAvailable => "not_available",
            Self::Running => "running",
        }
    }
}

/// Shared view of the worker's state, for status surfaces.
#[derive(Debug)]
pub struct DiscoveryStatus(Mutex<DiscoveryState>);

impl Default for DiscoveryStatus {
    fn default() -> Self {
        Self(Mutex::new(DiscoveryState::Starting))
    }
}

impl DiscoveryStatus {
    pub fn get(&self) -> DiscoveryState {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_for_test(&self, state: DiscoveryState) {
        self.set(state);
    }

    fn set(&self, state: DiscoveryState) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }
}

/// What one pass did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassOutcome {
    Disabled,
    NoHost,
    /// The pass ran; `changed` is whether it added a row (and so reloaded the
    /// Dialer).
    Ran {
        changed: bool,
    },
}

/// Everything the worker needs, each piece replaceable in tests.
pub struct DiscoveryDeps {
    /// `sync.discover`.
    pub enabled: bool,
    /// Whether any Herdr host is attached and reachable.
    pub host_attached: Arc<dyn Fn() -> bool + Send + Sync>,
    pub machines: Arc<dyn MachineLister>,
    /// The `herdr --remote` targets of the process table (blocking).
    pub remote_clients: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    pub store: Arc<dyn DiscoveryStore>,
    /// `SyncHandle::reload_targets`.
    pub reload: Arc<dyn Fn() + Send + Sync>,
    pub clock: Arc<dyn Clock>,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
    pub status: Arc<DiscoveryStatus>,
    pub period: Duration,
    pub cancel: Cancellation,
}

/// The process-table scan of the running daemon.
pub fn scan_remote_clients() -> Vec<String> {
    #[cfg(any(test, feature = "test-support"))]
    panic!("isolated test daemon attempted real sync discovery process scan; use a fixture source");
    #[cfg(not(any(test, feature = "test-support")))]
    {
        use crate::host::process_table::{HostSource, my_uid};
        remote_clients_from(&HostSource, my_uid())
    }
}

/// The `herdr --remote` targets among the processes `my_uid` owns; other users' processes are
/// skipped before their argv is read.
#[cfg(any(test, not(feature = "test-support")))]
pub(crate) fn remote_clients_from(
    source: &dyn crate::host::process_table::ProcessSource,
    my_uid: u32,
) -> Vec<String> {
    crate::host::process_table::remote_targets(&crate::host::process_table::scan_from(
        source, my_uid,
    ))
}

/// Runs the passes until `deps.cancel`. The first pass runs at once.
pub struct DiscoveryWorker {
    deps: DiscoveryDeps,
    pacer: Arc<Pacer>,
    last_logged: Mutex<Vec<(&'static str, std::time::Instant)>>,
    /// The remote clients the last pass with a completed process scan found
    /// (`None` before the first): retiring runs on the first such pass and
    /// whenever a client found then is gone, so a steady pass writes nothing.
    last_found: Mutex<Option<Vec<String>>>,
}

impl DiscoveryWorker {
    pub fn new(deps: DiscoveryDeps) -> Self {
        let pacer = Arc::new(Pacer::new(
            "discovery",
            Arc::clone(&deps.clock),
            deps.cancel.clone(),
        ));
        Self {
            deps,
            pacer,
            last_logged: Mutex::new(Vec::new()),
            last_found: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn pacer(&self) -> &Arc<Pacer> {
        &self.pacer
    }

    #[cfg(test)]
    fn into_deps(self) -> DiscoveryDeps {
        self.deps
    }

    /// Spawns the worker on the current tokio runtime.
    pub fn spawn(deps: DiscoveryDeps) -> tokio::task::JoinHandle<()> {
        let worker = Self::new(deps);
        tokio::spawn(async move { worker.run().await })
    }

    async fn run(self) {
        let cancel = self.deps.cancel.clone();
        loop {
            tokio::select! {
                () = cancel.cancelled() => return,
                () = async { self.pass().await; } => {}
            }
            if let Wake::Cancelled = self.pacer.wait(self.deps.period).await {
                return;
            }
        }
    }

    /// Logs `line` unless the same `key` was logged within [`LOG_EVERY`].
    fn log_limited(&self, key: &'static str, line: &str) {
        let now = std::time::Instant::now();
        let mut last = self.last_logged.lock().unwrap_or_else(|e| e.into_inner());
        match last.iter_mut().find(|(known, _)| *known == key) {
            Some((_, at)) if now.duration_since(*at) < LOG_EVERY => return,
            Some((_, at)) => *at = now,
            None => last.push((key, now)),
        }
        drop(last);
        (self.deps.log)(line);
    }

    /// One pass: nothing when disabled or without an attached host; else the
    /// found destinations are upserted, never-linked process-scan rows whose
    /// client is gone are retired, and when a row was added or retired the
    /// Dialer reloads.
    pub async fn pass(&self) -> PassOutcome {
        let deps = &self.deps;
        if !deps.enabled {
            deps.status.set(DiscoveryState::Disabled);
            return PassOutcome::Disabled;
        }
        if !(deps.host_attached)() {
            deps.status.set(DiscoveryState::NoHost);
            return PassOutcome::NoHost;
        }
        let mut found = Found::default();
        let mut state = DiscoveryState::Running;
        match deps.machines.list().await {
            ListOutcome::Machines(machines) => {
                found.machines = machines
                    .into_iter()
                    .filter(|machine| machine.enabled)
                    .map(|machine| machine.target)
                    .collect();
            }
            ListOutcome::NotAvailable => state = DiscoveryState::NotAvailable,
            ListOutcome::Failed(reason) => self.log_limited(
                "machine_list",
                &format!("sync discovery: herdr machine list failed: {reason}"),
            ),
        }
        let scan = Arc::clone(&deps.remote_clients);
        let scanned = match tokio::task::spawn_blocking(move || scan()).await {
            Ok(clients) => {
                found.remote_clients = clients;
                true
            }
            Err(error) => {
                self.log_limited(
                    "process_scan",
                    &format!("sync discovery: process scan failed: {error}"),
                );
                false
            }
        };
        deps.status.set(state);
        let upserts = found.upserts();
        // Only a client leaving the process scan triggers retirement, not a
        // flapping machine list.
        let retire = scanned && {
            let last = self.last_found.lock().unwrap_or_else(|e| e.into_inner());
            last.as_ref().is_none_or(|last| {
                last.iter()
                    .any(|dest| !found.remote_clients.iter().any(|client| client == dest))
            })
        };
        if upserts.is_empty() && !retire {
            return PassOutcome::Ran { changed: false };
        }
        let budget = CallBudget {
            deadline: MonoInstant(deps.clock.monotonic_now().0.saturating_add(STORE_BUDGET_MS)),
            cancellation: deps.cancel.clone(),
        };
        let store = Arc::clone(&deps.store);
        let written = {
            let upserts = upserts.clone();
            tokio::task::spawn_blocking(move || store.upsert_discovered(&upserts, retire, &budget))
                .await
        };
        let tally = match written {
            Ok(Ok(tally)) => tally,
            Ok(Err(error)) => {
                self.log_limited(
                    "upsert",
                    &format!("sync discovery: could not store targets: {}", error.detail),
                );
                return PassOutcome::Ran { changed: false };
            }
            Err(error) => {
                self.log_limited(
                    "upsert",
                    &format!("sync discovery: store task failed: {error}"),
                );
                return PassOutcome::Ran { changed: false };
            }
        };
        if tally.skipped > 0 {
            self.log_limited(
                "cap",
                &format!(
                    "sync discovery: {} target(s) not added: dial target cap reached",
                    tally.skipped
                ),
            );
        }
        if scanned {
            *self.last_found.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(found.remote_clients.clone());
        }
        if tally.inserted > 0 {
            (deps.log)(&format!(
                "sync discovery: added {} dial target(s)",
                tally.inserted
            ));
        }
        if tally.retired > 0 {
            (deps.log)(&format!(
                "sync discovery: retired {} never-linked dial target(s) whose herdr --remote client is gone",
                tally.retired
            ));
        }
        let changed = tally.inserted > 0 || tally.retired > 0;
        if changed {
            (deps.reload)();
        }
        PassOutcome::Ran { changed }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SystemClock;
    use crate::ports::ProcessRecord;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    #[should_panic(expected = "isolated test daemon attempted real sync discovery process scan")]
    fn isolated_discovery_rejects_real_process_scan() {
        scan_remote_clients();
    }

    #[test]
    fn parse_machine_list_enabled_targets() {
        let json = br#"[
            {"id":"a","label":"blacky","target":"blacky.dayton","session":"default","enabled":true,"selected":false},
            {"id":"b","label":"off","target":"off.example","enabled":false}
        ]"#;
        let machines = parse_machine_list(json).unwrap();
        assert_eq!(
            machines,
            [
                Machine {
                    target: "blacky.dayton".into(),
                    enabled: true
                },
                Machine {
                    target: "off.example".into(),
                    enabled: false
                },
            ]
        );
        assert!(parse_machine_list(b"[]").unwrap().is_empty());
        assert!(parse_machine_list(b"{\"target\":\"x\"}").is_err());
        assert!(parse_machine_list(b"not json").is_err());
    }

    #[test]
    fn parse_machine_list_ignores_unknown_fields() {
        let json = br#"[
            {"target":"h1","enabled":true,"future":{"nested":[1,2]},"id":7},
            {"target":"h2"},
            {"label":"no target","enabled":true},
            {"target":"","enabled":true},
            {"target":5},
            "not an object",
            null
        ]"#;
        let machines = parse_machine_list(json).unwrap();
        let targets: Vec<_> = machines
            .iter()
            .map(|m| (m.target.as_str(), m.enabled))
            .collect();
        // A missing `enabled` reads as enabled; unusable entries are skipped.
        assert_eq!(targets, [("h1", true), ("h2", true)]);
    }

    #[test]
    fn found_drops_unusable_and_duplicate_destinations() {
        let found = Found {
            machines: vec![
                "a".into(),
                "-bad".into(),
                "x\ty".into(),
                String::new(),
                "a".into(),
            ],
            remote_clients: vec!["a".into(), "c".into()],
        };
        assert_eq!(
            found.upserts(),
            [
                ("a".to_owned(), DiscoverySource::HerdrMachine),
                ("c".to_owned(), DiscoverySource::HerdrRemoteProcess),
            ]
        );
    }

    struct FixedLister(Mutex<Option<ListOutcome>>, AtomicUsize);

    impl MachineLister for FixedLister {
        fn list(&self) -> BoxFuture<ListOutcome> {
            self.1.fetch_add(1, Ordering::SeqCst);
            let outcome = self
                .0
                .lock()
                .unwrap()
                .take()
                .unwrap_or(ListOutcome::NotAvailable);
            Box::pin(async move { outcome })
        }
    }

    #[derive(Default)]
    struct RecordingStore {
        calls: Mutex<Vec<Vec<(String, DiscoverySource)>>>,
        retires: Mutex<Vec<bool>>,
        inserted: AtomicUsize,
        retired: AtomicUsize,
    }

    impl DiscoveryStore for RecordingStore {
        fn upsert_discovered(
            &self,
            found: &[(String, DiscoverySource)],
            retire_unseen: bool,
            _: &CallBudget,
        ) -> Result<UpsertTally, ApiError> {
            self.calls.lock().unwrap().push(found.to_vec());
            self.retires.lock().unwrap().push(retire_unseen);
            Ok(UpsertTally {
                inserted: self.inserted.load(Ordering::SeqCst),
                skipped: 0,
                retired: if retire_unseen {
                    self.retired.load(Ordering::SeqCst)
                } else {
                    0
                },
            })
        }
    }

    struct Rig {
        worker: DiscoveryWorker,
        store: Arc<RecordingStore>,
        lister: Arc<FixedLister>,
        reloads: Arc<AtomicUsize>,
        attached: Arc<AtomicBool>,
        logs: Arc<Mutex<Vec<String>>>,
    }

    fn rig(enabled: bool, outcome: ListOutcome, clients: Vec<String>) -> Rig {
        rig_with(enabled, outcome, Arc::new(move || clients.clone()))
    }

    fn rig_with(
        enabled: bool,
        outcome: ListOutcome,
        remote_clients: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    ) -> Rig {
        let store = Arc::new(RecordingStore::default());
        store.inserted.store(1, Ordering::SeqCst);
        let lister = Arc::new(FixedLister(Mutex::new(Some(outcome)), AtomicUsize::new(0)));
        let reloads = Arc::new(AtomicUsize::new(0));
        let attached = Arc::new(AtomicBool::new(true));
        let logs = Arc::new(Mutex::new(Vec::new()));
        let worker = DiscoveryWorker::new(DiscoveryDeps {
            enabled,
            host_attached: {
                let attached = Arc::clone(&attached);
                Arc::new(move || attached.load(Ordering::SeqCst))
            },
            machines: Arc::clone(&lister) as Arc<dyn MachineLister>,
            remote_clients,
            store: Arc::clone(&store) as Arc<dyn DiscoveryStore>,
            reload: {
                let reloads = Arc::clone(&reloads);
                Arc::new(move || {
                    reloads.fetch_add(1, Ordering::SeqCst);
                })
            },
            clock: Arc::new(SystemClock::new()),
            log: {
                let logs = Arc::clone(&logs);
                Arc::new(move |line: &str| logs.lock().unwrap().push(line.to_owned()))
            },
            status: Arc::default(),
            period: Duration::from_secs(60),
            cancel: Cancellation::default(),
        });
        Rig {
            worker,
            store,
            lister,
            reloads,
            attached,
            logs,
        }
    }

    fn machines(targets: &[(&str, bool)]) -> ListOutcome {
        ListOutcome::Machines(
            targets
                .iter()
                .map(|(target, enabled)| Machine {
                    target: (*target).into(),
                    enabled: *enabled,
                })
                .collect(),
        )
    }

    struct FakeProcs(Mutex<Vec<u32>>);

    impl crate::host::process_table::ProcessSource for FakeProcs {
        fn pids(&self) -> Vec<u32> {
            vec![20, 21]
        }
        fn owner(&self, pid: u32) -> Option<u32> {
            Some(if pid == 20 { 501 } else { 777 })
        }
        fn record(&self, pid: u32) -> Option<ProcessRecord> {
            self.0.lock().unwrap().push(pid);
            let argv: &[&str] = if pid == 20 {
                &["herdr", "--remote", "mine.example"]
            } else {
                &["/usr/local/bin/herdr", "--remote=theirs.example"]
            };
            Some(ProcessRecord {
                pid,
                parent: None,
                start: 1,
                argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            })
        }
    }

    #[tokio::test]
    async fn foreign_uid_remote_client_is_not_upserted() {
        let fake = Arc::new(FakeProcs(Mutex::new(Vec::new())));
        let source = Arc::clone(&fake);
        let rig = rig_with(
            true,
            ListOutcome::Machines(vec![]),
            Arc::new(move || remote_clients_from(&*source, 501)),
        );
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: true });
        assert_eq!(
            rig.store.calls.lock().unwrap().as_slice(),
            [vec![(
                "mine.example".to_owned(),
                DiscoverySource::HerdrRemoteProcess
            )]]
        );
        assert_eq!(*fake.0.lock().unwrap(), [20], "pid 21 never read");
    }

    #[tokio::test]
    async fn discover_false_upserts_nothing() {
        let rig = rig(false, machines(&[("h", true)]), vec!["r".into()]);
        assert_eq!(rig.worker.pass().await, PassOutcome::Disabled);
        assert!(rig.store.calls.lock().unwrap().is_empty());
        assert_eq!(rig.lister.1.load(Ordering::SeqCst), 0, "not even polled");
        assert_eq!(rig.reloads.load(Ordering::SeqCst), 0);
        assert_eq!(rig.worker.deps.status.get(), DiscoveryState::Disabled);
    }

    #[tokio::test]
    async fn no_attached_host_no_poll() {
        let rig = rig(true, machines(&[("h", true)]), vec!["r".into()]);
        rig.attached.store(false, Ordering::SeqCst);
        assert_eq!(rig.worker.pass().await, PassOutcome::NoHost);
        assert_eq!(rig.lister.1.load(Ordering::SeqCst), 0);
        assert!(rig.store.calls.lock().unwrap().is_empty());
        assert_eq!(rig.worker.deps.status.get(), DiscoveryState::NoHost);
    }

    #[tokio::test]
    async fn changed_pass_calls_reload_targets() {
        let rig = rig(
            true,
            machines(&[("h1", true), ("off", false)]),
            vec!["r1".into(), "h1".into()],
        );
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: true });
        assert_eq!(rig.reloads.load(Ordering::SeqCst), 1);
        {
            let calls = rig.store.calls.lock().unwrap();
            assert_eq!(
                calls.as_slice(),
                [vec![
                    ("h1".to_owned(), DiscoverySource::HerdrMachine),
                    ("r1".to_owned(), DiscoverySource::HerdrRemoteProcess),
                ]],
                "enabled machines and remote clients, the disabled machine left out"
            );
        }
        // Nothing new: the store reports no insert, so no reload.
        rig.store.inserted.store(0, Ordering::SeqCst);
        *rig.lister.0.lock().unwrap() = Some(machines(&[("h1", true)]));
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: false });
        assert_eq!(rig.reloads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unavailable_machine_list_is_shown_and_remote_clients_still_found() {
        let rig = rig(true, ListOutcome::NotAvailable, vec!["r".into()]);
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: true });
        assert_eq!(rig.worker.deps.status.get(), DiscoveryState::NotAvailable);
        assert_eq!(
            rig.store.calls.lock().unwrap().as_slice(),
            [vec![("r".to_owned(), DiscoverySource::HerdrRemoteProcess)]]
        );
        // Not a failure: not a log line.
        assert!(
            rig.logs
                .lock()
                .unwrap()
                .iter()
                .all(|l| !l.contains("failed"))
        );
    }

    #[tokio::test]
    async fn failing_machine_list_logs_once_per_hour() {
        let rig = rig(true, ListOutcome::Failed("boom".into()), Vec::new());
        rig.store.inserted.store(0, Ordering::SeqCst);
        for _ in 0..3 {
            *rig.lister.0.lock().unwrap() = Some(ListOutcome::Failed("boom".into()));
            assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: false });
        }
        let failures = rig
            .logs
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("machine list failed"))
            .count();
        assert_eq!(failures, 1);
        // Only the first pass's retirement reaches the store; nothing was found.
        assert_eq!(rig.store.calls.lock().unwrap().as_slice(), [vec![]]);
        assert_eq!(rig.store.retires.lock().unwrap().as_slice(), [true]);
    }

    #[tokio::test]
    async fn retires_on_the_first_scan_and_when_a_client_is_gone() {
        let clients = Arc::new(Mutex::new(vec!["r1".to_owned(), "r2".to_owned()]));
        let scan = Arc::clone(&clients);
        let rig = rig_with(
            true,
            machines(&[]),
            Arc::new(move || scan.lock().unwrap().clone()),
        );
        let next_pass = |rig: &Rig| *rig.lister.0.lock().unwrap() = Some(machines(&[]));
        // The first scan retires rows left by clients of an earlier run.
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: true });
        // The same clients again: upserted as before, nothing retired.
        rig.store.inserted.store(0, Ordering::SeqCst);
        next_pass(&rig);
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: false });
        // A new client alone upserts without retiring.
        clients.lock().unwrap().push("r3".into());
        next_pass(&rig);
        rig.worker.pass().await;
        // r1's client is gone: retired, logged, and the Dialer reloads.
        clients.lock().unwrap().remove(0);
        rig.store.retired.store(1, Ordering::SeqCst);
        let reloads = rig.reloads.load(Ordering::SeqCst);
        next_pass(&rig);
        assert_eq!(rig.worker.pass().await, PassOutcome::Ran { changed: true });
        assert_eq!(rig.reloads.load(Ordering::SeqCst), reloads + 1);
        assert_eq!(
            rig.store.retires.lock().unwrap().as_slice(),
            [true, false, false, true]
        );
        assert_eq!(
            rig.store.calls.lock().unwrap().last().unwrap(),
            &vec![
                ("r2".to_owned(), DiscoverySource::HerdrRemoteProcess),
                ("r3".to_owned(), DiscoverySource::HerdrRemoteProcess),
            ]
        );
        // A machine-list entry that comes and goes never triggers retirement.
        rig.store.retired.store(0, Ordering::SeqCst);
        *rig.lister.0.lock().unwrap() = Some(machines(&[("m", true)]));
        rig.worker.pass().await;
        next_pass(&rig);
        rig.worker.pass().await;
        assert_eq!(
            rig.store.retires.lock().unwrap()[4..],
            [false, false],
            "a machine-list flap does not retire"
        );
        assert!(rig.logs.lock().unwrap().iter().any(|line| line
            == "sync discovery: retired 1 never-linked dial target(s) whose herdr --remote client is gone"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_maps_exit_127_unknown_subcommand_and_missing_binary() {
        use std::os::unix::fs::PermissionsExt;
        let isolation = crate::test_support::isolation::TestIsolation::new("discovery-cli");
        let dir = isolation.path("bin");
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            HerdrCli::new(path)
        };
        let ok = script("ok", r#"echo '[{"target":"t","enabled":true,"x":1}]'"#);
        assert_eq!(ok.list().await, machines(&[("t", true)]));
        let code127 = script("c127", "exit 127");
        assert_eq!(code127.list().await, ListOutcome::NotAvailable);
        let unknown = script(
            "unk",
            "echo \"error: unrecognized subcommand 'machine'\" >&2; exit 2",
        );
        assert_eq!(unknown.list().await, ListOutcome::NotAvailable);
        let missing = HerdrCli::new(dir.join("absent"));
        assert_eq!(missing.list().await, ListOutcome::NotAvailable);
        let broken = script("bad", "echo 'oops' >&2; exit 1");
        assert!(matches!(broken.list().await, ListOutcome::Failed(_)));
        let garbage = script("garbage", "echo 'not json'");
        assert!(matches!(garbage.list().await, ListOutcome::Failed(_)));
    }

    /// Polls `cond` every 5 ms for at most 5 s.
    async fn poll_until(what: &str, cond: impl Fn() -> bool) {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !cond() {
            assert!(std::time::Instant::now() < until, "{what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn worker_stops_on_cancel() {
        let rig = rig(true, machines(&[("h", true)]), Vec::new());
        let cancel = rig.worker.deps.cancel.clone();
        let reloads = Arc::clone(&rig.reloads);
        let task = tokio::spawn(async move { rig.worker.run().await });
        poll_until("first pass runs at once", || {
            reloads.load(Ordering::SeqCst) > 0
        })
        .await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the worker stops promptly")
            .unwrap();
    }

    struct SteppedClock(std::sync::atomic::AtomicU64);
    impl Clock for SteppedClock {
        fn utc_now(&self) -> crate::protocol::time::UtcMillis {
            crate::protocol::time::UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
    }

    #[tokio::test]
    async fn worker_waits_its_period_on_the_injected_clock() {
        let clock = Arc::new(SteppedClock(std::sync::atomic::AtomicU64::new(0)));
        let mut rig = rig(true, machines(&[("h", true)]), Vec::new());
        rig.worker = DiscoveryWorker::new(DiscoveryDeps {
            clock: Arc::clone(&clock) as Arc<dyn Clock>,
            ..rig.worker.into_deps()
        });
        let cancel = rig.worker.deps.cancel.clone();
        let lister = Arc::clone(&rig.lister);
        let pacer = Arc::clone(rig.worker.pacer());
        let task = tokio::spawn(async move { rig.worker.run().await });
        poll_until("first pass", || lister.1.load(Ordering::SeqCst) == 1).await;
        // Real time passes; the injected clock does not: no second pass.
        poll_until("pacer idle", || pacer.idle_events() >= 1).await;
        tokio::time::timeout(Duration::from_millis(100), std::future::pending::<()>())
            .await
            .unwrap_err();
        assert_eq!(lister.1.load(Ordering::SeqCst), 1);
        clock.0.store(60_000, Ordering::SeqCst);
        pacer.clock_advanced();
        poll_until("second pass after the clock moved", || {
            lister.1.load(Ordering::SeqCst) == 2
        })
        .await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the worker stops promptly")
            .unwrap();
    }
}
