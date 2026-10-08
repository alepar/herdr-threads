//! ht-p03.9.4: deadline and wake lane latency, idleness and outage cost
//! through the daemon request path. Every test runs the production elected
//! daemon (`run_elected_probed`, the same composition `daemon run` uses, with
//! the production `NativeCli` host) in this process against a private named
//! Herdr session from `IsolatedHerdr` (never the shared server). Seats are
//! driven through the built `herdr-threads` executable with the cooperative
//! caller flags, exactly as `sweep.rs` does; panes are plain shells unless a
//! test starts the stand-in `claude` script (no model, no network) in one.
//! The `LaneProbe` reads the daemon's per-origin commit counter, flushed kick
//! log and lane Pacer idle counts without changing its behaviour.
use herdr_threads::{
    app::{KickRecord, LaneProbe, SystemClock, run_elected_probed},
    daemon::paths::{InstancePaths, RuntimeContext},
    host::native::NativeCli,
    ports::HostPort,
    protocol::time::{Cancellation, Clock},
    service::{
        config::ServiceConfig,
        kicks::{Lane, lanes_for_table},
    },
    test_support::isolated_herdr::IsolatedHerdr,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, atomic::AtomicBool, atomic::Ordering},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// A stand-in agent named `claude`, which Herdr classifies as an agent: it
/// echoes every line typed into it, so each wake prompt is visible once.
const STAND_IN: &str = "#!/bin/sh\necho HT-STANDIN-START\n\
    while IFS= read -r line; do echo \"HT-RECEIVED:$line\"; done\n";

#[derive(Clone)]
pub(crate) struct Caller {
    pub(crate) seat: String,
    pub(crate) pane: String,
}

pub(crate) struct Session {
    pub(crate) herdr: IsolatedHerdr,
    pub(crate) state: PathBuf,
    pub(crate) probe: LaneProbe,
    stop: Cancellation,
    daemon: Option<JoinHandle<std::io::Result<bool>>>,
    stand_in: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl Session {
    pub(crate) fn new(case: &str) -> Option<Self> {
        let herdr = IsolatedHerdr::new(case)?;
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        herdr.start();
        let bin = herdr.root().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let stand_in = bin.join("claude");
        fs::write(&stand_in, STAND_IN).unwrap();
        fs::set_permissions(&stand_in, fs::Permissions::from_mode(0o755)).unwrap();
        let state = herdr.root().join("plugin-state");
        fs::create_dir_all(&state).unwrap();
        let context = RuntimeContext::explicit(state.clone(), herdr.socket_path(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let host: Arc<dyn HostPort> =
            Arc::new(NativeCli::new(herdr.socket_path(), Arc::clone(&clock)));
        let probe = LaneProbe::default();
        let stop = Cancellation::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (daemon_stop, daemon_probe) = (stop.clone(), probe.clone());
        let daemon = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected_probed(
                    &paths,
                    clock,
                    daemon_stop,
                    // Measure commit latency and outage freezing independently
                    // of the configurable initial ordinary batching window.
                    ServiceConfig::default().with_wake_batch_delay(0).unwrap(),
                    host,
                    daemon_probe,
                    move |descriptor| {
                        ready_tx
                            .send(descriptor.clone())
                            .map_err(std::io::Error::other)
                    },
                ))
        });
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the elected daemon did not publish its endpoint");
        // The elected daemon installs a quiet panic hook; restore visible failures.
        std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
        let session = Self {
            herdr,
            state,
            probe,
            stop,
            daemon: Some(daemon),
            stand_in,
            _guard: guard,
        };
        wait_until("the probe to attach", Duration::from_secs(5), || {
            session.probe.attached()
        });
        Some(session)
    }

    /// A `herdr` CLI call in the private session; `None` when it fails (an
    /// agent that is not there yet, for one), else its `result`.
    pub(crate) fn herdr_try(&self, args: &[&str]) -> Option<Value> {
        let output = self.herdr.command("herdr").args(args).output().unwrap();
        if !output.status.success() {
            return None;
        }
        // Some verbs (`pane run`) print nothing on success.
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            return Some(Value::Null);
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("herdr {args:?}: {e}: {output:?}"));
        Some(value["result"].clone())
    }

    pub(crate) fn herdr_api(&self, args: &[&str]) -> Value {
        self.herdr_try(args)
            .unwrap_or_else(|| panic!("herdr {args:?} failed"))
    }

    pub(crate) fn first_pane(&self) -> String {
        let created = self.herdr_api(&["workspace", "create"]);
        created["root_pane"]["pane_id"].as_str().unwrap().to_owned()
    }

    pub(crate) fn split_pane(&self, from: &str) -> String {
        let pane = self.herdr_api(&["pane", "split", from, "--direction", "right"]);
        pane["pane"]["pane_id"].as_str().unwrap().to_owned()
    }

    pub(crate) fn run_stand_in(&self, pane: &str) {
        let script = self.stand_in.to_string_lossy().into_owned();
        self.herdr_api(&["pane", "run", pane, &script]);
        wait_until(
            "Herdr to report the stand-in agent idle",
            Duration::from_secs(30),
            || {
                self.herdr_try(&["agent", "get", pane])
                    .is_some_and(|agent| {
                        agent["agent"]["agent"] == "claude"
                            && agent["agent"]["agent_status"] == "idle"
                    })
            },
        );
    }

    pub(crate) fn pane_text(&self, pane: &str) -> String {
        let output = self
            .herdr
            .command("herdr")
            .args(["pane", "read", pane, "--source", "recent-unwrapped"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    pub(crate) fn cli(&self, caller: Option<&Caller>, args: &[&str]) -> (i32, Value, String) {
        let mut command = self.herdr.command(BIN);
        let args = if args.first() == Some(&"human") {
            command.arg("human");
            &args[1..]
        } else {
            args
        };
        command
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(self.herdr.socket_path());
        if let Some(caller) = caller {
            command.args([
                "--cooperative-seat",
                &caller.seat,
                "--cooperative-target",
                &caller.pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
            ]);
        }
        let output = command
            .args(args)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
        (output.status.code().unwrap_or(-1), value, stderr)
    }

    pub(crate) fn ok(&self, caller: Option<&Caller>, args: &[&str]) -> Value {
        let (code, value, stderr) = self.cli(caller, args);
        assert_eq!(code, 0, "herdr-threads {args:?} failed: {stderr}{value}");
        value["result"]["data"].clone()
    }

    /// A seat for `pane`, checked in as a top-level stand-in cooperative caller.
    pub(crate) fn seat(&self, pane: &str) -> Caller {
        let seat = self.ok(None, &["seat", "resolve", "--pane", pane]);
        let caller = Caller {
            seat: seat.as_str().unwrap().to_owned(),
            pane: pane.to_owned(),
        };
        self.ok(
            Some(&caller),
            &["check-in", "--lifecycle-event", &format!("start-{pane}")],
        );
        caller
    }

    pub(crate) fn db(&self) -> rusqlite::Connection {
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(self.state.clone(), self.herdr.socket_path(), None).unwrap(),
        )
        .unwrap();
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    pub(crate) fn commits(&self, origin: &str) -> u64 {
        self.probe.commit_counts().get(origin).copied().unwrap_or(0)
    }

    /// Waits until the `origin` commit counter has stood still for `window`.
    pub(crate) fn wait_commits_quiet(&self, origin: &str, window: Duration) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut last = self.commits(origin);
        let mut since = Instant::now();
        while since.elapsed() < window {
            assert!(
                Instant::now() < deadline,
                "{origin} commits never went quiet"
            );
            std::thread::sleep(Duration::from_millis(20));
            let now = self.commits(origin);
            if now != last {
                last = now;
                since = Instant::now();
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.join();
        }
        // The elected daemon redirects process stderr into its private log.
        // Restore the captured panic diagnostic after it has restored stderr,
        // before IsolatedHerdr removes the fixture directory.
        if std::thread::panicking()
            && let Ok(context) =
                RuntimeContext::explicit(self.state.clone(), self.herdr.socket_path(), None)
            && let Ok(paths) = InstancePaths::resolve(&context)
            && let Ok(bytes) = fs::read(herdr_threads::daemon::logs::daemon_log_path(&paths))
        {
            let tail = &bytes[bytes.len().saturating_sub(16 * 1024)..];
            eprintln!(
                "private daemon failure log:\n{}",
                String::from_utf8_lossy(tail)
            );
        }
    }
}

fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Waits for the first commit, retaining its producer-side timestamp rather
/// than charging observer-thread scheduling delay to the worker's latency.
struct FirstCommit {
    at: Arc<Mutex<Option<Instant>>>,
    stop: Arc<AtomicBool>,
    poller: Option<JoinHandle<()>>,
}
impl FirstCommit {
    fn watch(read: impl Fn() -> Option<Instant> + Send + 'static) -> Self {
        let at = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (slot, halt) = (Arc::clone(&at), Arc::clone(&stop));
        let poller = std::thread::spawn(move || {
            while !halt.load(Ordering::SeqCst) {
                if let Some(committed_at) = read() {
                    *slot.lock().unwrap() = Some(committed_at);
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            at,
            stop,
            poller: Some(poller),
        }
    }
    fn wait(&mut self, timeout: Duration) -> Option<Instant> {
        let until = Instant::now() + timeout;
        while Instant::now() < until {
            if let Some(at) = *self.at.lock().unwrap() {
                return Some(at);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }
}
impl Drop for FirstCommit {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(poller) = self.poller.take() {
            let _ = poller.join();
        }
    }
}

#[test]
fn commit_latency_excludes_delayed_observation() {
    // The producer completed before the observer was scheduled. Reading the
    // stored timestamp must not relabel that commit as happening now.
    let committed_at = Instant::now() - Duration::from_millis(250);
    let mut observation = FirstCommit::watch(move || Some(committed_at));
    assert_eq!(
        observation.wait(Duration::from_secs(5)),
        Some(committed_at),
        "observer scheduling must not inflate commit latency"
    );
}

/// A content digest per table, so a send's writes are the tables whose digest
/// changed (a count alone would miss an upsert). Rows are ordered by their own
/// content: a WITHOUT ROWID table (harness_version_evidence) has no rowid.
fn table_digests(db: &rusqlite::Connection) -> BTreeMap<String, String> {
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let columns: Vec<String> = db
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get(1))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let row = columns
                .iter()
                .map(|column| format!("quote(\"{column}\")"))
                .collect::<Vec<_>>()
                .join("||'|'||");
            let digest: Option<String> = db
                .query_row(
                    &format!("SELECT group_concat(r, char(10)) FROM (SELECT {row} AS r FROM \"{table}\" ORDER BY r)"),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            (table, digest.unwrap_or_default())
        })
        .collect()
}

/// A sender and `recipients` recipients, each recipient in a thread of its
/// own (it created the thread, so it joined without an invitation). A
/// recipient therefore has had no wake attention yet, and the first wake it
/// receives is the one the test provokes: a wake that reaches a plain shell
/// pane is `Unsafe` and holds the seat on the 30 s ladder, which would hide
/// the latency under test. The sender is invited and accepts in each thread
/// (its own wakes at setup are of no interest).
pub(crate) struct Scene {
    pub(crate) session: Session,
    pub(crate) sender: Caller,
    pub(crate) recipients: Vec<(Caller, String)>,
}

impl Scene {
    pub(crate) fn new(case: &str, recipients: usize) -> Option<Self> {
        let session = Session::new(case)?;
        Some(Self::from_session(session, case, recipients))
    }

    fn from_session(session: Session, case: &str, recipients: usize) -> Self {
        let sender_pane = session.first_pane();
        let sender = session.seat(&sender_pane);
        let mut joined = Vec::new();
        let mut from = sender_pane;
        for index in 0..recipients {
            from = session.split_pane(&from);
            let caller = session.seat(&from);
            let thread = session
                .ok(
                    Some(&caller),
                    &["thread", "create", "--topic", &format!("{case}-{index}")],
                )
                .as_str()
                .unwrap()
                .to_owned();
            session.ok(Some(&caller), &["invite", &thread, "--seat", &sender.seat]);
            session.ok(Some(&sender), &["accept", &thread]);
            joined.push((caller, thread));
        }
        Self {
            session,
            sender,
            recipients: joined,
        }
    }

    pub(crate) fn send(&self, index: usize, body: &str, extra: &[&str]) -> Value {
        let (to, thread) = &self.recipients[index];
        let mut args = vec!["send", thread, "--body", body, "--require-ack", &to.seat];
        args.extend_from_slice(extra);
        self.session.ok(Some(&self.sender), &args)
    }

    fn recipients_have_wake_outcomes(&self) -> bool {
        let db = self.session.db();
        self.recipients.iter().all(|(caller, _)| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM wake_work WHERE seat_id=?1 AND last_outcome IS NOT NULL)",
                [&caller.seat],
                |r| r.get::<_, bool>(0),
            )
            .unwrap()
        })
    }
}

fn kicks_after(log: Vec<KickRecord>, since: Instant) -> Vec<KickRecord> {
    log.into_iter().filter(|(_, _, at)| *at >= since).collect()
}

/// A send committed through client, transport and store on the full
/// production worker set is attempted by the wake lane in under 100 ms, with
/// no safety-tick wait; every table the send writes maps to a lane, and
/// `wake_work` maps to the wake lane.
/// Kills: a wake lane that waits for its 5 s tick (or a 1 s gate) after a
/// commit, and a commit-to-lane map that misses the send path's wake table.
#[test]
fn send_is_attempted_within_100ms_without_a_tick_wait() {
    let Some(scene) = Scene::new("send_is_attempted_within_100ms", 1) else {
        return;
    };
    let s = &scene.session;
    // The sender's setup wake (its invitation, `Unsafe` on a plain shell) is
    // done; the recipient has had no attention at all. Wait until the wake
    // lane has nothing left to do before measuring.
    s.wait_commits_quiet("wake", Duration::from_millis(1500));
    let before = table_digests(&s.db());
    let committed_at = s.probe.next_commit_instant(Lane::Wakes);
    let mut attempt = FirstCommit::watch(move || *committed_at.lock().unwrap());
    let sent_after = Instant::now();
    scene.send(0, "latency probe", &[]);
    let attempted_at = attempt
        .wait(Duration::from_secs(3))
        .expect("the wake lane never attempted the send within 3 s");
    // Read after the attempt, so the deadline lane's follow-up commit is in.
    let after = table_digests(&s.db());
    let kicks = kicks_after(s.probe.kick_log(), sent_after);
    // The send's own commit runs on a request thread and kicks the deadline
    // lane (the new `send_attention` job); that lane's commit writes the
    // seat's `wake_work` attention and kicks the wake lane. The whole chain
    // must reach an attempt inside the budget.
    let (_, _, committed_at) = kicks
        .iter()
        .find(|(_, origin, _)| origin.is_none())
        .unwrap_or_else(|| panic!("no request-origin commit after the send: {kicks:?}"));
    let (_, wake_kick_origin, _) = kicks
        .iter()
        .find(|(lanes, _, _)| lanes.contains(Lane::Wakes))
        .unwrap_or_else(|| panic!("nothing kicked the wake lane: {kicks:?}"));
    assert_eq!(
        *wake_kick_origin,
        Some(Lane::Deadlines),
        "the wake kick comes from the deadline lane's materialization: {kicks:?}"
    );
    let latency = attempted_at.saturating_duration_since(*committed_at);
    assert!(
        latency < Duration::from_millis(100),
        "send commit to wake attempt took {latency:?}"
    );
    let changed: Vec<&String> = before
        .iter()
        .filter(|(table, digest)| after.get(*table) != Some(*digest))
        .map(|(table, _)| table)
        .collect();
    assert!(
        changed.iter().any(|table| *table == "wake_work"),
        "the send path writes wake_work: {changed:?}"
    );
    for table in &changed {
        // lanes_for_table debug-asserts that every table is classified; the
        // tables that carry a send into an attempt must each map to a lane.
        let lanes = lanes_for_table(table);
        if ["work_jobs", "wake_work", "receipt_state"].contains(&table.as_str()) {
            assert!(!lanes.is_empty(), "{table} kicks no lane");
        }
    }
    assert!(lanes_for_table("wake_work").contains(Lane::Wakes));
    assert!(lanes_for_table("work_jobs").contains(Lane::Deadlines));
}

/// An idle daemon commits nothing from the deadline, wake or request origins
/// and each of those two lanes makes at most one pass per 5 s safety tick,
/// with the observation lane running at its own 5 s cadence. Observation
/// commits (admission, snapshot stage, seal, publish) kick no lane.
/// Kills: a lane that polls on a short turn (many passes), a tick pass that
/// commits (an empty scan writing), and an observation commit that is mapped
/// to a lane and so wakes the wake lane every cycle.
#[test]
fn idle_daemon_commits_nothing_from_deadline_wake_request_for_30s() {
    // Long real-time waits: runs beside, not behind, ONE_DAEMON.
    if herdr_threads::test_support::spawn::ran_in_own_process(
        module_path!(),
        "idle_daemon_commits_nothing_from_deadline_wake_request_for_30s",
    ) {
        return;
    }
    let Some(scene) = Scene::new("idle_daemon_30s", 1) else {
        return;
    };
    let s = &scene.session;
    // Let setup's own work (invitation wake, receipts, first observation
    // publication) settle, then take the baseline.
    std::thread::sleep(Duration::from_secs(8));
    s.wait_commits_quiet("wake", Duration::from_secs(2));
    s.wait_commits_quiet("deadline", Duration::from_secs(2));
    // On a loaded host setup's own wake work can outlast the fixed 8 s: take
    // the baseline once neither lane has passed for 3 s. A lane that polls on
    // a short turn never gets there, so that failure still fails here.
    for lane in [Lane::Wakes, Lane::Deadlines] {
        let give_up = Instant::now() + Duration::from_secs(90);
        let mut last = (s.probe.idle_events(lane), Instant::now());
        while last.1.elapsed() < Duration::from_secs(3) {
            assert!(
                Instant::now() < give_up,
                "the {lane:?} lane never went quiet after setup"
            );
            std::thread::sleep(Duration::from_millis(100));
            let now = s.probe.idle_events(lane);
            if now != last.0 {
                last = (now, Instant::now());
            }
        }
    }
    let counts = s.probe.commit_counts();
    let kicks = s.probe.kick_log().len();
    let (wake_idle, deadline_idle) = (
        s.probe.idle_events(Lane::Wakes),
        s.probe.idle_events(Lane::Deadlines),
    );
    let window = Duration::from_secs(30);
    let started = Instant::now();
    std::thread::sleep(window);
    let elapsed = started.elapsed();
    let after = s.probe.commit_counts();
    for origin in ["deadline", "wake", "request"] {
        assert_eq!(
            after.get(origin),
            counts.get(origin),
            "an idle daemon committed from the {origin} origin: {counts:?} -> {after:?}"
        );
    }
    // The observation lane really ran (its commits are the only ones), and
    // none of them kicked anything.
    assert!(
        after["observation"] > counts["observation"],
        "the observation lane never committed: {counts:?} -> {after:?}"
    );
    let new_kicks: Vec<KickRecord> = s.probe.kick_log().split_off(kicks);
    assert_eq!(
        new_kicks.len(),
        0,
        "no commit at idle kicks any lane: {new_kicks:?}"
    );
    // One pass per 5 s tick, plus one for the window's edges.
    let ticks = elapsed.as_secs() / 5 + 1;
    let wake_passes = s.probe.idle_events(Lane::Wakes) - wake_idle;
    let deadline_passes = s.probe.idle_events(Lane::Deadlines) - deadline_idle;
    assert!(
        (1..=ticks).contains(&wake_passes),
        "wake lane made {wake_passes} passes in {elapsed:?}"
    );
    assert!(
        (1..=ticks).contains(&deadline_passes),
        "deadline lane made {deadline_passes} passes in {elapsed:?}"
    );
}

/// A deadline-lane commit that creates a warning wake (an overdue invitation
/// writes `warning_jobs`, then `warning_recipients` and `wake_work`) is
/// attempted by the wake lane in under 100 ms, with no tick wait for the wake
/// lane: the commit's kick, not its 5 s safety tick, starts the pass.
/// Kills: a deadline-origin commit that does not kick the wake lane (the
/// warning would wait up to 5 s), and a wake lane that ignores the kick.
#[test]
fn deadline_commit_creating_a_warning_wake_is_attempted_within_100ms() {
    let Some(session) = Session::new("warning_wake_within_100ms") else {
        return;
    };
    let inviter_pane = session.first_pane();
    let guest_pane = session.split_pane(&inviter_pane);
    let inviter = session.seat(&inviter_pane);
    // A prelaunch guest: a seat for its pane that never checks in, so its
    // invitation stays pending past its 1 s deadline.
    let guest = session
        .ok(None, &["seat", "resolve", "--pane", &guest_pane])
        .as_str()
        .unwrap()
        .to_owned();
    let thread = session
        .ok(Some(&inviter), &["thread", "create", "--topic", "overdue"])
        .as_str()
        .unwrap()
        .to_owned();
    // The inviter has had no attention yet, so the warning is the first wake it
    // is owed; the guest's own invitation wake is not the measured one.
    session.ok(
        Some(&inviter),
        &["invite", &thread, "--seat", &guest, "--deadline", "1"],
    );
    session.wait_commits_quiet("wake", Duration::from_millis(1500));
    let kicks_before = session.probe.kick_log().len();
    let committed_at = session.probe.next_commit_instant(Lane::Wakes);
    let mut attempt = FirstCommit::watch(move || *committed_at.lock().unwrap());
    // The deadline passes after 1 s; the deadline lane notices at its next
    // 5 s safety tick (documented as up to 5 s late).
    let attempted_at = attempt
        .wait(Duration::from_secs(15))
        .expect("the overdue invitation never produced a wake attempt");
    let kicks = session.probe.kick_log().split_off(kicks_before);
    let (_, _, kicked_at) = kicks
        .iter()
        .find(|(lanes, origin, _)| lanes.contains(Lane::Wakes) && *origin == Some(Lane::Deadlines))
        .unwrap_or_else(|| panic!("no deadline-origin Wakes kick: {kicks:?}"));
    let latency = attempted_at.saturating_duration_since(*kicked_at);
    assert!(
        latency < Duration::from_millis(100),
        "deadline commit to wake attempt took {latency:?}: {kicks:?}"
    );
    assert!(
        session
            .db()
            .query_row(
                "SELECT count(*) FROM warning_recipients WHERE seat_id=?1",
                [&inviter.seat],
                |row| row.get::<_, i64>(0)
            )
            .unwrap()
            > 0,
        "the warning's recipients include the inviter"
    );
    assert!(lanes_for_table("warning_recipients").contains(Lane::Wakes));
    assert!(lanes_for_table("warning_jobs").contains(Lane::Deadlines));
}

/// The most attempts one seat can make in `elapsed` seconds after its first
/// refused attempt: the n-th retry comes `100 ms x 2^(n-1)` (-20 %, capped at
/// 30 s) after the previous one.
fn most_attempts(elapsed: f64) -> u64 {
    let (mut most, mut cumulative) = (1, 0.0);
    for n in 1..200 {
        cumulative += (100.0 * 2f64.powi(n - 1)).min(30_000.0) * 0.8 / 1000.0;
        if cumulative <= elapsed {
            most += 1;
        }
    }
    most
}

#[test]
fn settled_setup_invitations_do_not_require_a_sender_wake() {
    let Some(session) = Session::new("settled_setup_invitations") else {
        return;
    };
    let (parked_tx, parked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let stop = session.stop.clone();
    let first = AtomicBool::new(true);
    session.probe.set_registered_idle_hook(
        Lane::Wakes,
        Box::new(move |_| {
            if !first.swap(false, Ordering::SeqCst) {
                return;
            }
            parked_tx.send(()).unwrap();
            // Session teardown cancels this wait even if fixture setup panics.
            while !stop.is_cancelled() {
                match release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_millis(20))
                {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        }),
    );
    session.probe.kick_registered(Lane::Wakes);
    parked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let scene = Scene::from_session(session, "settled_setup_invitations", 3);
    for index in 0..2 {
        scene.send(index, "owed", &[]);
    }
    release_tx.send(()).unwrap();
    wait_until(
        "exact recipients first wakes",
        Duration::from_secs(15),
        || {
            scene.recipients[..2].iter().all(|(caller, _)| {
                scene.session.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM wake_work WHERE seat_id=?1 AND last_outcome IS NOT NULL)",
            [&caller.seat], |r| r.get::<_, bool>(0)).unwrap()
            })
        },
    );
    assert!(
        !scene.recipients_have_wake_outcomes(),
        "the unsent third recipient cannot satisfy the wait"
    );
    scene.send(2, "owed", &[]);
    wait_until(
        "all exact recipients first wakes",
        Duration::from_secs(15),
        || scene.recipients_have_wake_outcomes(),
    );
    let completed: i64 = scene
        .session
        .db()
        .query_row(
            "SELECT count(*) FROM wake_work WHERE last_outcome IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(completed, 3, "settled setup attention needs no sender wake");
    assert!(!scene.session.db().query_row(
        "SELECT EXISTS(SELECT 1 FROM wake_work WHERE seat_id=?1 AND last_outcome IS NOT NULL)",
        [&scene.sender.seat], |r| r.get::<_, bool>(0)).unwrap());
}

/// Herdr stopped with `SEATS` seats holding pending wake work: once the
/// observation lane's first capture of the dead host is frozen (TRUST-POLICY
/// C4), the wake lane is frozen too (ht-72q): for the rest of the outage it
/// makes no wake reservation, records no refusal and calls no Herdr method, so
/// it commits nothing at all, while still waking on its safety tick. Only the
/// detection window (Herdr stopped, no frozen capture yet; at most one 5 s
/// observation cadence) may see refused attempts, within the per-seat
/// refusal backoff bound (`2 x SEATS x steps`). After Herdr returns and the
/// operator repairs the seats each one is submitted exactly once.
///
/// A real stop is Herdr unavailability, which freezes state (ht-yms): no
/// invalidation is written and every seat stays resolved, so without the
/// freeze the wake lane would keep attempting each recipient against the
/// dead host. The restarted Herdr is a new incarnation: the lane's first
/// capture of it ends the freeze and unresolves every seat (C2), so nothing
/// is prompted until the repair.
/// Kills: a wake lane that keeps reserving or refusing against a down host,
/// a frozen capture that unresolves seats or writes an invalidation, a
/// frozen lane that stops ticking, and a refusal path that climbs the 30 s
/// ladder (the seats would not be submitted promptly after the restore).
#[test]
fn herdr_stopped_freezes_wake_lane() {
    // Long real-time waits: runs beside, not behind, ONE_DAEMON.
    if herdr_threads::test_support::spawn::ran_in_own_process(
        module_path!(),
        "herdr_stopped_freezes_wake_lane",
    ) {
        return;
    }
    const SEATS: u64 = 3;
    const OUTAGE: Duration = Duration::from_secs(30);
    let Some(scene) = Scene::new("herdr_stopped_freezes_wake_lane", SEATS as usize) else {
        return;
    };
    let s = &scene.session;
    for index in 0..SEATS as usize {
        scene.send(index, &format!("owed {index}"), &[]);
    }
    // Each recipient's first wake runs with Herdr up and reaches a plain
    // shell (`Unsafe`). Setup accepts the sender's invitations, so that
    // settled attention need not have produced any sender wake.
    wait_until(
        "each recipient's first wake",
        Duration::from_secs(15),
        || scene.recipients_have_wake_outcomes(),
    );
    s.wait_commits_quiet("wake", Duration::from_secs(1));
    let before = s.commits("wake");
    let invalidation_revision = || -> i64 {
        s.db()
            .query_row(
                "SELECT invalidation_revision FROM host_instances",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    let revision = invalidation_revision();
    assert!(
        !s.probe.host_down(),
        "the host is reachable before the stop"
    );
    s.herdr.stop();
    let stopped = Instant::now();
    // The observation lane's next capture (within its 5 s cadence) is frozen
    // and marks the host down; a wake pass already running finishes.
    wait_until(
        "the frozen capture to freeze the wake lane",
        Duration::from_secs(15),
        || s.probe.host_down(),
    );
    s.wait_commits_quiet("wake", Duration::from_millis(500));
    let detection = s.commits("wake") - before;
    let bound = 2 * SEATS * most_attempts(stopped.elapsed().as_secs_f64());
    assert!(
        detection <= bound,
        "{detection} wake commits in the {:?} detection window exceed 2 x {SEATS} seats x steps ({bound})",
        stopped.elapsed()
    );
    let frozen = s.commits("wake");
    let frozen_idle = s.probe.idle_events(Lane::Wakes);
    std::thread::sleep(OUTAGE);
    assert_eq!(
        s.commits("wake"),
        frozen,
        "the wake lane committed while Herdr was down"
    );
    assert!(s.probe.host_down(), "the host came back while stopped");
    assert!(
        s.probe.idle_events(Lane::Wakes) > frozen_idle,
        "the frozen wake lane stopped ticking"
    );
    // Herdr unavailability is not evidence (TRUST-POLICY C4, ht-yms): no
    // seat was unresolved and no invalidation was written.
    let unresolved = || -> i64 {
        s.db()
            .query_row(
                "SELECT count(*) FROM seats WHERE state='unresolved'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(unresolved(), 0, "the host going away unresolved seats");
    assert_eq!(
        invalidation_revision(),
        revision,
        "Herdr unavailability wrote an invalidation"
    );
    // Herdr returns as a new incarnation with fresh terminals: once the lane
    // captures it, C2 unresolves every saved seat (restore holds) and they
    // stay unresolved until the operator repairs them, as in the
    // host-recovery validation.
    s.herdr.start();
    wait_until(
        "the new Herdr incarnation to unresolve every seat",
        Duration::from_secs(60),
        || unresolved() == SEATS as i64 + 1,
    );
    assert!(!s.probe.host_down(), "an answered capture ends the freeze");
    for (caller, _) in &scene.recipients {
        s.run_stand_in(&caller.pane);
        // First contact after the Herdr restart: the published boot is the
        // old one, so the rebind waits for the lane capture it asks for.
        s.ok(
            None,
            &[
                "human",
                "seat",
                "rebind",
                &caller.seat,
                "--pane",
                &caller.pane,
                "--operator",
            ],
        );
        s.ok(Some(caller), &["check-in", "--lifecycle-event", "restored"]);
    }
    wait_until(
        "every recipient submitted after the restore",
        Duration::from_secs(90),
        || {
            let submitted: i64 = s
                .db()
                .query_row(
                    "SELECT count(*) FROM wake_work WHERE last_outcome='submitted'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            submitted == SEATS as i64
        },
    );
    std::thread::sleep(Duration::from_secs(3));
    for (caller, _) in &scene.recipients {
        let text = s.pane_text(&caller.pane);
        // The stand-in echoes each submitted line; an empty echo is only the
        // dispatcher's single submit-key retry, not another prompt.
        let received = text
            .matches("HT-RECEIVED:herdr-threads: attention pending")
            .count();
        assert_eq!(
            received, 1,
            "{} was prompted {received} times: {text}",
            caller.seat
        );
    }
}
