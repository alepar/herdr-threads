//! ht-p03.40: the daemon lane wiring across its seams, through the production
//! elected daemon (`run_elected_probed`, the composition `daemon run` uses, with
//! the production `NativeCli` host) in this process against a private named
//! Herdr session from `IsolatedHerdr` (never the shared server). The
//! `LaneProbe` reads the daemon's per-origin commit counter, flushed kick log,
//! commit-kick registry and lane statuses, and injects lane failures; it
//! changes no behaviour while nothing is injected. Seats are driven through the
//! built `herdr-threads` executable with the cooperative caller flags, as
//! `lanes_latency.rs` does.
use herdr_threads::{
    app::{KickRecord, LaneProbe, SystemClock, run_elected_probed},
    client::{
        local::LocalSocketClient,
        service::{PersistentServiceClient, ServiceIntentJournal},
    },
    daemon::{
        logs::daemon_log_path,
        ownership::EndpointDescriptor,
        paths::{InstancePaths, RuntimeContext},
        remedy::{RemedyContext, remedy},
    },
    host::native::NativeCli,
    ports::{HostPort, LocalClient},
    protocol::{
        commands::{Command, ServiceDisconnectRequest},
        ids::{OperationId, ThreadId},
        results::{CommandResult, ErrorClass, ErrorCode},
        service::{EnsureManagedThread, ServiceOperation, ServiceResult},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{config::ServiceConfig, kicks::Lane},
    test_support::isolated_herdr::IsolatedHerdr,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

#[derive(Clone)]
pub(crate) struct Caller {
    pub(crate) seat: String,
    pub(crate) pane: String,
}

pub(crate) struct Session {
    pub(crate) herdr: IsolatedHerdr,
    pub(crate) state: PathBuf,
    pub(crate) log: PathBuf,
    pub(crate) descriptor: EndpointDescriptor,
    pub(crate) probe: LaneProbe,
    stop: Cancellation,
    daemon: Option<JoinHandle<std::io::Result<bool>>>,
    _guard: MutexGuard<'static, ()>,
}

impl Session {
    pub(crate) fn new(case: &str) -> Option<Self> {
        let herdr = IsolatedHerdr::new(case)?;
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        herdr.start();
        let state = herdr.root().join("plugin-state");
        fs::create_dir_all(&state).unwrap();
        let context = RuntimeContext::explicit(state.clone(), herdr.socket_path(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        let log = daemon_log_path(&paths);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let host: Arc<dyn HostPort> =
            Arc::new(NativeCli::new(herdr.socket_path(), Arc::clone(&clock)));
        let probe = LaneProbe::default();
        // The detached daemon's stderr is its daemon.log; in this process the
        // lane error log is pointed at the same file.
        probe.log_lane_errors_to(log.clone());
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
                    ServiceConfig::default(),
                    host,
                    daemon_probe,
                    move |descriptor| {
                        ready_tx
                            .send(descriptor.clone())
                            .map_err(std::io::Error::other)
                    },
                ))
        });
        let descriptor = ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the elected daemon did not publish its endpoint");
        // The elected daemon installs a quiet panic hook; restore visible failures.
        std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
        let session = Self {
            herdr,
            state,
            log,
            descriptor,
            probe,
            stop,
            daemon: Some(daemon),
            _guard: guard,
        };
        wait_until("the probe to attach", Duration::from_secs(5), || {
            session.probe.attached()
        });
        Some(session)
    }

    pub(crate) fn herdr_api(&self, args: &[&str]) -> Value {
        let output = self.herdr.command("herdr").args(args).output().unwrap();
        assert!(output.status.success(), "herdr {args:?} failed: {output:?}");
        let value: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("herdr {args:?}: {e}: {output:?}"));
        value["result"].clone()
    }

    pub(crate) fn first_pane(&self) -> String {
        let created = self.herdr_api(&["workspace", "create"]);
        created["root_pane"]["pane_id"].as_str().unwrap().to_owned()
    }

    pub(crate) fn split_pane(&self, from: &str) -> String {
        let pane = self.herdr_api(&["pane", "split", from, "--direction", "right"]);
        pane["pane"]["pane_id"].as_str().unwrap().to_owned()
    }

    pub(crate) fn cli(&self, caller: Option<&Caller>, args: &[&str]) -> (i32, Value, String) {
        let mut command = self.herdr.command(BIN);
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

    /// Waits until none of the `origins` commit counters has moved for
    /// `window` (one shared window, not one per origin in turn).
    pub(crate) fn wait_all_commits_quiet(&self, origins: &[&str], window: Duration) {
        let deadline = Instant::now() + Duration::from_secs(60);
        let read = || -> Vec<u64> { origins.iter().map(|o| self.commits(o)).collect() };
        let mut last = read();
        let mut since = Instant::now();
        while since.elapsed() < window {
            assert!(
                Instant::now() < deadline,
                "{origins:?} commits never went quiet"
            );
            std::thread::sleep(Duration::from_millis(20));
            let now = read();
            if now != last {
                last = now;
                since = Instant::now();
            }
        }
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

    /// Every row of `table`, for a failure message.
    pub(crate) fn dump_table(&self, table: &str) -> String {
        let db = self.db();
        let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns: Vec<String> = statement
            .column_names()
            .iter()
            .map(|c| c.to_string())
            .collect();
        let rows = statement
            .query_map([], |row| {
                Ok((0..columns.len())
                    .map(|i| format!("{}={:?}", columns[i], row.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join(" "))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect::<Vec<_>>();
        rows.join(" | ")
    }

    pub(crate) fn daemon_log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Daemon-log lines the `lane` error log wrote.
    pub(crate) fn lane_log_lines(&self, lane: Lane) -> Vec<String> {
        let prefix = format!("lane {}: ", lane.name());
        self.daemon_log()
            .lines()
            .filter(|line| line.starts_with(&prefix))
            .map(str::to_owned)
            .collect()
    }

    /// Health as `herdr-threads daemon health` reports it: overall state and
    /// the limitation lines.
    pub(crate) fn health(&self) -> (String, Vec<String>) {
        let data = self.ok(None, &["daemon", "health"]);
        let lines = data["limitations"]
            .as_array()
            .expect("health limitations")
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        (data["state"].as_str().unwrap().to_owned(), lines)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.join();
        }
        // Daemon stderr is redirected into this private file. Replay failures
        // after joining restores stderr, before IsolatedHerdr removes the root.
        if std::thread::panicking()
            && let Ok(bytes) = fs::read(&self.log)
        {
            let tail = &bytes[bytes.len().saturating_sub(16 * 1024)..];
            eprintln!(
                "private daemon failure log:\n{}",
                String::from_utf8_lossy(tail)
            );
        }
    }
}

pub(crate) fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Limitation lines a degraded lane adds: the scheduler line and the pointer.
pub(crate) fn is_degraded_line(line: &str) -> bool {
    line.starts_with("scheduler degraded") || line.starts_with("degraded: ")
}

pub(crate) fn without_degraded(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|line| !is_degraded_line(line))
        .cloned()
        .collect()
}

/// Waits until every lane has finished a pass and none has run one for
/// `quiet`, so a kick's pass is the only one that can complete after it.
pub(crate) fn wait_lanes_settled(s: &Session, quiet: Duration) {
    wait_until(
        "every lane to finish its first pass",
        Duration::from_secs(20),
        || {
            Lane::ALL
                .iter()
                .all(|lane| s.probe.registered_idle_events(*lane) >= 1)
        },
    );
    let mut last: Vec<u64> = Lane::ALL
        .iter()
        .map(|l| s.probe.registered_idle_events(*l))
        .collect();
    let mut since = Instant::now();
    let give_up = Instant::now() + Duration::from_secs(60);
    while since.elapsed() < quiet {
        assert!(
            Instant::now() < give_up,
            "the lanes never went quiet (idle events {last:?}); wake_work: {}",
            s.dump_table("wake_work")
        );
        std::thread::sleep(Duration::from_millis(20));
        let now: Vec<u64> = Lane::ALL
            .iter()
            .map(|l| s.probe.registered_idle_events(*l))
            .collect();
        if now != last {
            last = now;
            since = Instant::now();
        }
    }
}

pub(crate) fn kicks_since(s: &Session, from: usize) -> Vec<KickRecord> {
    s.probe.kick_log().split_off(from)
}

/// Lets setup's own work (the seat's check-in wake) settle before an idle
/// window: every lane has finished a pass, and the idle origins go quiet.
pub(crate) fn settle_setup(s: &Session) {
    wait_lanes_settled(s, Duration::from_millis(500));
    s.wait_all_commits_quiet(
        &["wake", "deadline", "request"],
        Duration::from_millis(1500),
    );
}

/// The shortest idle window that spans one wake safety tick
/// (`WAKE_SAFETY_TICK_MILLIS`, 5 s) and one observation cadence (5 s) with
/// margin. The window is extended (never past `IDLE_WINDOW_LIMIT`) until both
/// the wake lane and the observation lane have made a pass in it, so a loaded
/// machine that delays a tick does not fail the lower bounds; the upper bound
/// on wake passes is computed from the window's actual length.
pub(crate) const IDLE_WINDOW: Duration = Duration::from_millis(5_500);
const IDLE_WINDOW_LIMIT: Duration = Duration::from_secs(30);

/// Runs an idle window (see [`IDLE_WINDOW`]), kicking retention every 500 ms
/// (it has no commit-driven kick), and returns its length.
pub(crate) fn run_idle_window(s: &Session, wake_from: u64, observation_from: u64) -> Duration {
    let started = Instant::now();
    loop {
        s.probe.kick_registered(Lane::Retention);
        std::thread::sleep(Duration::from_millis(500));
        let elapsed = started.elapsed();
        let passes_seen = s.probe.registered_idle_events(Lane::Wakes) > wake_from
            && s.commits("observation") > observation_from;
        if (elapsed >= IDLE_WINDOW && passes_seen) || elapsed >= IDLE_WINDOW_LIMIT {
            return elapsed;
        }
    }
}

/// The production factory registers all registered lanes with the commit-kick
/// registry the store's hooks kick through, and attaches each lane's Pacer to
/// the `WorkerStatus` Health reads. A kick sent through the registry makes the
/// lane thread itself finish a pass, so the registered Pacer is the one the
/// lane waits on, not a stand-in.
/// Kills: a lane started without `kicks.register` (commits would never wake
/// it), a status whose Pacer was never attached (no retry suffix in Health),
/// and a registry holding a different Pacer from the lane's.
#[test]
fn every_lane_registers() {
    let Some(s) = Session::new("every_lane_registers") else {
        return;
    };
    for lane in Lane::ALL {
        wait_until(
            &format!("{} to register and attach its Pacer", lane.name()),
            Duration::from_secs(10),
            || s.probe.registered(lane) && s.probe.status_has_pacer(lane),
        );
    }
    wait_lanes_settled(&s, Duration::from_millis(1500));
    for lane in Lane::ALL {
        let before = s.probe.registered_idle_events(lane);
        s.probe.kick_registered(lane);
        wait_until(
            &format!("a kick through the registry to run a {} pass", lane.name()),
            Duration::from_secs(3),
            || s.probe.registered_idle_events(lane) > before,
        );
    }
}

/// A committed write to a mapped table, made on a request thread, kicks the
/// lane the table maps to through the registry: a seat check-in (`seats`,
/// `occupant_bindings`) is a request-origin commit that kicks the wake lane,
/// and its Pacer, not a tick, starts the pass. A send's deadline-lane
/// materialization then commits `wake_work` and kicks the wake lane from the
/// deadline origin, and the wake lane attempts the seat at once. Neither
/// kick is made by the test: it only observes.
/// Kills: a store whose commit hook is not wired to the daemon's registry
/// (no kick, so a 5 s tick wait), a kick that reaches a different Pacer from
/// the wake lane's, and a materialization commit that kicks nothing.
#[test]
fn committed_wake_work_insert_kicks_the_wake_pacer() {
    let Some(s) = Session::new("committed_wake_work_kicks_wake") else {
        return;
    };
    let sender_pane = s.first_pane();
    let recipient_pane = s.split_pane(&sender_pane);
    let sender = s.seat(&sender_pane);
    let recipient = s.seat(&recipient_pane);
    let thread = s
        .ok(Some(&recipient), &["thread", "create", "--topic", "wiring"])
        .as_str()
        .unwrap()
        .to_owned();
    s.ok(
        Some(&recipient),
        &["invite", &thread, "--seat", &sender.seat],
    );
    s.ok(Some(&sender), &["accept", &thread]);
    wait_lanes_settled(&s, Duration::from_millis(1000));
    s.wait_all_commits_quiet(&["wake", "deadline"], Duration::from_millis(1000));

    // 1. A request-origin commit to a wake-mapped table.
    let kicks_from = s.probe.kick_log().len();
    let passes_before = s.probe.registered_idle_events(Lane::Wakes);
    let third_pane = s.split_pane(&recipient_pane);
    s.seat(&third_pane);
    let request_wake_kick = kicks_since(&s, kicks_from)
        .into_iter()
        .find(|(lanes, origin, _)| lanes.contains(Lane::Wakes) && origin.is_none())
        .unwrap_or_else(|| panic!("no request-origin commit kicked the wake lane"));
    wait_until(
        "the wake lane to run a pass after the commit's kick",
        Duration::from_secs(1),
        || s.probe.registered_idle_events(Lane::Wakes) > passes_before,
    );
    let latency = Instant::now().saturating_duration_since(request_wake_kick.2);
    assert!(
        latency < Duration::from_secs(1),
        "a commit's kick to the wake pass took {latency:?}, a tick wait is 5 s"
    );

    // 2. A send: the deadline lane materializes the seat's `wake_work`.
    s.wait_all_commits_quiet(&["wake", "deadline"], Duration::from_millis(1000));
    let kicks_from = s.probe.kick_log().len();
    let wake_commits = s.commits("wake");
    s.ok(
        Some(&sender),
        &[
            "send",
            &thread,
            "--body",
            "wiring probe",
            "--require-ack",
            &recipient.seat,
        ],
    );
    wait_until(
        "the wake lane to attempt the send",
        Duration::from_secs(3),
        || s.commits("wake") > wake_commits,
    );
    let kicks = kicks_since(&s, kicks_from);
    assert!(
        kicks
            .iter()
            .any(|(lanes, origin, _)| lanes.contains(Lane::Deadlines) && origin.is_none()),
        "the send's request commit kicks the deadline lane: {kicks:?}"
    );
    assert!(
        kicks
            .iter()
            .any(|(lanes, origin, _)| lanes.contains(Lane::Wakes)
                && *origin == Some(Lane::Deadlines)),
        "the deadline lane's wake_work commit kicks the wake lane: {kicks:?}"
    );
}

/// An injected failure in each lane (the store lanes fail on their next store
/// access, the admission observer fails its observation) lands in daemon.log
/// as one first-occurrence line however many passes fail, Health says
/// the `remedy()` degraded pointer naming daemon.log with exactly the scheduler line and that
/// pointer added (the other lines are the healthy ones, unchanged, and the
/// lane's retry suffix rides on the scheduler line), and a later good pass
/// clears all of it without another log line.
/// Kills: a lane whose `WorkerStatus` is not the one Health reads, a lane
/// whose failures bypass the shared rate-limited log (no line, or a line per
/// pass), a degraded rule that adds a Health line per lane or per failure,
/// and a status that stays degraded after a success.
#[test]
fn injected_lane_failure_logs_and_degrades_then_clears() {
    // Long real-time waits: runs beside, not behind, ONE_DAEMON.
    if herdr_threads::test_support::spawn::ran_in_own_process(
        module_path!(),
        "injected_lane_failure_logs_and_degrades_then_clears",
    ) {
        return;
    }
    let Some(s) = Session::new("injected_lane_failure_logs") else {
        return;
    };
    let pane = s.first_pane();
    s.seat(&pane);
    wait_lanes_settled(&s, Duration::from_millis(1500));
    let (state, healthy) = s.health();
    assert_eq!(state, "healthy", "{healthy:?}");
    assert!(
        healthy.iter().all(|line| !is_degraded_line(line)),
        "{healthy:?}"
    );
    let pointer = format!(
        "degraded: {}",
        remedy(
            Some(ErrorClass::Transient),
            &RemedyContext::LaneDegraded { log: s.log.clone() }
        )
    );
    for lane in Lane::ALL {
        let name = lane.name();
        assert_eq!(s.probe.lane_health(lane), None, "{name} starts healthy");
        let log_before = s.lane_log_lines(lane).len();
        s.probe.fail_lane(lane, ErrorCode::StoreBusy);
        // Enough failed passes that an unlimited log would have written
        // several lines: kick the lane a few times and let its backoff run.
        let hits_from = s.probe.lane_fault_hits(lane);
        let until = Instant::now() + Duration::from_secs(20);
        while s.probe.lane_fault_hits(lane) < hits_from + 3 {
            assert!(
                Instant::now() < until,
                "{name}: the injected failure fired {} times in 20 s",
                s.probe.lane_fault_hits(lane) - hits_from
            );
            s.probe.kick_registered(lane);
            std::thread::sleep(Duration::from_millis(25));
        }
        wait_until(
            &format!("{name} to report its failure"),
            Duration::from_secs(5),
            || s.probe.lane_health(lane).is_some(),
        );
        let lines = s.lane_log_lines(lane);
        assert_eq!(
            lines.len(),
            log_before + 1,
            "{name}: one first-occurrence line in daemon.log, got {lines:?}"
        );
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with(&format!("lane {name}: StoreBusy: ")),
            "{name}: {lines:?}"
        );
        let (state, degraded) = s.health();
        assert_eq!(state, "degraded", "{name}: {degraded:?}");
        let mut others = without_degraded(&degraded);
        if lane == Lane::Observation {
            // A failed capture also leaves the host evidence unverified, which
            // Health states on the host line it already has for that case (it
            // is not a line per lane or per failure).
            let host: Vec<String> = others
                .iter()
                .filter(|line| line.starts_with("host degraded: "))
                .cloned()
                .collect();
            assert_eq!(host.len(), 1, "{name}: {degraded:?}");
            others.retain(|line| !line.starts_with("host degraded: "));
        }
        assert_eq!(
            others, healthy,
            "{name}: Health gained or lost a line other than the degraded ones"
        );
        let added: Vec<&String> = degraded.iter().filter(|l| is_degraded_line(l)).collect();
        assert_eq!(
            added.len(),
            2,
            "{name}: the scheduler line and the pointer: {degraded:?}"
        );
        assert!(
            added[0].starts_with("scheduler degraded: ") && added[0].contains(name),
            "{name}: {added:?}"
        );
        assert_eq!(added[1], &pointer, "{name}");

        s.probe.heal_lane(lane);
        // Kicked, so the deadline and wake lanes' next good pass need not wait
        // for their 5 s safety tick (a kick never shortens a backoff).
        wait_until(
            &format!("a later success to clear {name}"),
            Duration::from_secs(40),
            || {
                s.probe.kick_registered(lane);
                s.probe.lane_health(lane).is_none()
            },
        );
        let (state, cleared) = s.health();
        assert_eq!(state, "healthy", "{name}: {cleared:?}");
        assert_eq!(
            cleared, healthy,
            "{name}: Health returns to its healthy lines"
        );
        assert_eq!(
            s.lane_log_lines(lane).len(),
            log_before + 1,
            "{name}: recovery writes no further line"
        );
    }
}

/// Commits made on a tokio blocking thread are request-origin: a programmatic
/// service session's write (`EnsureThread`, run by the transport on
/// `spawn_blocking` under the live gate) and the operator's revoke of that
/// session (`service disconnect`: the gate revoke, then the audit commit)
/// both land under the `request` counter key, never under a lane's, and
/// neither is dropped as a lane self-kick. The deadline, wake, retention and
/// admission-observer counters do not move while the daemon is otherwise
/// idle. (A lease dropped while a decision guard is held defers its revoke to
/// a blocking thread; the guard-held ordering is pinned by
/// `tests/daemon/session_lease.rs`, since the gate is private to the daemon.)
/// Kills: a blocking-pool thread that carries a lane origin (its commit
/// would count under that lane and its kick would be discarded as a
/// self-kick), and a revoke whose audit commit never runs.
#[test]
fn spawn_blocking_revoke_counts_as_request_origin() {
    let Some(s) = Session::new("spawn_blocking_revoke_origin") else {
        return;
    };
    let pane = s.first_pane();
    s.seat(&pane);
    wait_lanes_settled(&s, Duration::from_millis(1000));
    s.wait_all_commits_quiet(
        &["deadline", "wake", "retention"],
        Duration::from_millis(1000),
    );
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = PersistentServiceClient::new(
        s.descriptor.endpoint.clone(),
        Arc::clone(&clock),
        s.descriptor.instance_uuid,
        Some(s.descriptor.boot_id),
        ServiceIntentJournal::open(s.herdr.root().join("intents")).unwrap(),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let registration = runtime.block_on(client.register(&budget())).unwrap();
    s.wait_commits_quiet("request", Duration::from_millis(500));
    let (admission_from, retention_from) =
        (s.commits("admission-observer"), s.commits("retention"));

    // The service write, run by the transport on a blocking thread.
    let request_from = s.commits("request");
    let (_, ensured) = runtime
        .block_on(client.submit(
            ServiceOperation::EnsureThread(EnsureManagedThread {
                thread: ThreadId::new("wiring-service"),
                topic: "wiring".into(),
                goal: "origin".into(),
                operation: OperationId::new("wiring-ensure"),
            }),
            &budget(),
        ))
        .unwrap();
    assert!(matches!(ensured, ServiceResult::ThreadEnsured(_)));
    assert!(
        s.commits("request") > request_from,
        "the service write committed under the request key: {:?}",
        s.probe.commit_counts()
    );

    // The operator revoke.
    let request_from = s.commits("request");
    let ordinary = LocalSocketClient::new(
        s.descriptor.endpoint.clone(),
        Arc::clone(&clock),
        s.descriptor.instance_uuid,
        Some(s.descriptor.boot_id),
    );
    let CommandResult::ServiceDisconnected(revoked) = ordinary
        .call(
            Command::ServiceDisconnect(ServiceDisconnectRequest {
                expected_boot: registration.daemon_boot.clone(),
                expected_generation: registration.connection_generation,
            }),
            &budget(),
        )
        .unwrap()
    else {
        panic!("missing service disconnect result");
    };
    assert!(revoked.disconnected, "{revoked:?}");
    assert_eq!(
        s.commits("request"),
        request_from + 1,
        "the revoke's one audit commit counted under the request key: {:?}",
        s.probe.commit_counts()
    );
    let CommandResult::ServiceInspection(inspected) =
        ordinary.call(Command::ServiceInspect, &budget()).unwrap()
    else {
        panic!("missing service inspection");
    };
    assert!(!inspected.connected, "the session is revoked");
    assert_eq!(
        (s.commits("admission-observer"), s.commits("retention")),
        (admission_from, retention_from),
        "neither the service write nor the revoke counted under a lane that holds no such work"
    );
    runtime.block_on(client.disconnect());
}

/// With all registered lanes running on the production worker set and nothing to do
/// for an idle window spanning a wake safety tick and an observation cadence
/// ([`IDLE_WINDOW`], 5.5 s or more; the name keeps its historical `30s`):
/// the per-origin counter shows no deadline, wake, request or
/// admission-observer commit; the wake lane makes at most one pass per 5 s
/// window; and retention (kicked every 500 ms, since it has no commit-driven
/// kick) prunes the superseded snapshot generations the observation lane
/// keeps publishing, with those Retention-origin commits kicking no lane.
/// Over the short window the pass bound (`elapsed/5 + 1`) only catches a
/// lane that polls faster than ~2.7 s; slower short-turn polling is caught
/// by `lanes_latency::idle_daemon_commits_nothing_from_deadline_wake_request_for_30s`
/// (12 s window, under ~4 s).
/// Kills: a lane that polls on a very short turn, an idle pass that commits, a
/// retention prune whose kick wakes the wake or deadline lane (it would then
/// pass more than once per window and commit), and a lane (retention for
/// one) missing from the production worker set.
#[test]
fn idle_five_lanes_30s() {
    // Long real-time waits: runs beside, not behind, ONE_DAEMON.
    if herdr_threads::test_support::spawn::ran_in_own_process(module_path!(), "idle_five_lanes_30s")
    {
        return;
    }
    let Some(s) = Session::new("idle_five_lanes_30s") else {
        return;
    };
    let pane = s.first_pane();
    s.seat(&pane);
    // Let setup's own work settle, then take the baseline.
    settle_setup(&s);
    let counts = s.probe.commit_counts();
    let kicks_from = s.probe.kick_log().len();
    let wake_from = s.probe.registered_idle_events(Lane::Wakes);
    // A completed `send_attention` job from long ago: retention deletes it, and
    // `work_jobs` is a deadline-lane table, so only the Retention origin's
    // discarded kick set keeps this commit from waking the deadline lane. (A
    // second connection's write fires no commit hook.)
    let stale_jobs = || -> i64 {
        s.db()
            .query_row(
                "SELECT count(*) FROM work_jobs WHERE id = 'stale-job'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    s.db()
        .execute(
            "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) \
             VALUES ('stale-job','send_attention','stale-job',0,'complete',0)",
            [],
        )
        .unwrap();
    assert_eq!(stale_jobs(), 1);
    let observation_from = counts.get("observation").copied().unwrap_or(0);
    let elapsed = run_idle_window(&s, wake_from, observation_from);
    let after = s.probe.commit_counts();
    for origin in ["deadline", "wake", "request", "admission-observer"] {
        assert_eq!(
            after.get(origin),
            counts.get(origin),
            "an idle daemon committed from the {origin} origin: {counts:?} -> {after:?}"
        );
    }
    assert!(
        after.get("observation").copied().unwrap_or(0) > observation_from,
        "the observation lane never committed: {counts:?} -> {after:?}"
    );
    assert!(
        after.get("retention").copied().unwrap_or(0)
            > counts.get("retention").copied().unwrap_or(0),
        "retention pruned nothing in {elapsed:?}: {counts:?} -> {after:?}"
    );
    assert_eq!(stale_jobs(), 0, "retention deleted the stale completed job");
    let new_kicks = kicks_since(&s, kicks_from);
    assert!(
        new_kicks.is_empty(),
        "no commit at idle (observation publishes, retention prunes) kicks a lane: {new_kicks:?}"
    );
    let ticks = elapsed.as_secs() / 5 + 1;
    let wake_passes = s.probe.registered_idle_events(Lane::Wakes) - wake_from;
    assert!(
        (1..=ticks).contains(&wake_passes),
        "the wake lane made {wake_passes} passes in {elapsed:?} (at most {ticks})"
    );
}
