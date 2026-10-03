//! ht-p03.51: the root integration sweep for the remaining herdr-threads
//! findings. Cross-bucket paths no per-seam bead owns, driven through the
//! production elected daemon on a private named Herdr session (the
//! `lane_wiring::Session` fixture, never the shared server) and the built
//! `herdr-threads` executable:
//!
//! - a send's commit reaching a wake attempt within 100 ms with all five lanes
//!   registered, with and without a large settled history (B1 x B4);
//! - retention draining a backlog in bounded batches while the other lanes
//!   stay idle (B4 x B1);
//! - every lane failing at once surfacing through `remedy()`/Health within the
//!   Health line budget (B2 x B3);
//! - startup failure, lane failure and version skew each reaching the operator
//!   as their own `remedy()` text, a skewed daemon never showing the lane
//!   pointer (B3 x B6).
//!
//! The existing per-seam tests that cover the remaining flows are listed with
//! their outcomes in `docs/history/remaining-findings-run/integration-sweep-notes.md`.
use crate::lane_wiring::{Session, kicks_since, wait_lanes_settled, wait_until};
use crate::lanes_latency;
use herdr_threads::{
    daemon::{
        health::HEALTH_LINE_BUDGET,
        paths::{InstancePaths, RuntimeContext},
        remedy::{RemedyContext, remedy},
    },
    protocol::{
        commands::{Command, HOOK_PARSE_DETAIL_BYTES, HookParseFailure, bounded_hook_detail},
        results::{ErrorClass, ErrorCode},
        wire::PROTOCOL_VERSION,
    },
    service::kicks::Lane,
    store::retention::RETENTION_BATCH_ROWS,
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// A sender and `recipients` recipients, each in a thread of its own, on the
/// `lanes_latency` fixture (a recipient has had no wake attention yet, so the
/// first wake it receives is the one the test provokes; a second send to the
/// same plain-shell seat would sit on the 30 s ladder, so each measured send
/// goes to a fresh recipient), the lanes quiet.
struct Scene(lanes_latency::Scene);

impl Scene {
    fn new(case: &str, recipients: usize) -> Option<Self> {
        let scene = lanes_latency::Scene::new(case, recipients)?;
        keep_panics_visible();
        wait_lanes_settled_latency(&scene.session);
        Some(Self(scene))
    }

    fn session(&self) -> &lanes_latency::Session {
        &self.0.session
    }

    /// Sends one message and returns the time from the send's own
    /// request-origin commit to the wake lane's first commit after it (the
    /// attempt), which must come through the deadline lane's wake kick and not
    /// the 5 s tick. `recipient` is used once.
    fn send_to_wake_attempt(&self, recipient: usize, body: &str) -> Duration {
        let s = self.session();
        s.wait_commits_quiet("wake", Duration::from_millis(1500));
        let baseline = s.commits("wake");
        let stop = Arc::new(AtomicBool::new(false));
        let attempted_at: Arc<Mutex<Option<Instant>>> = Arc::default();
        let poller = {
            let (probe, stop, slot) = (
                s.probe.clone(),
                Arc::clone(&stop),
                Arc::clone(&attempted_at),
            );
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    if probe.commit_counts().get("wake").copied().unwrap_or(0) > baseline {
                        *slot.lock().unwrap() = Some(Instant::now());
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
        };
        let sent_after = Instant::now();
        self.0.send(recipient, body, &[]);
        let until = Instant::now() + Duration::from_secs(3);
        while attempted_at.lock().unwrap().is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        stop.store(true, Ordering::SeqCst);
        poller.join().unwrap();
        let kicks: Vec<_> = s
            .probe
            .kick_log()
            .into_iter()
            .filter(|(_, _, at)| *at >= sent_after)
            .collect();
        let attempted = attempted_at.lock().unwrap().unwrap_or_else(|| {
            panic!(
                "the wake lane never attempted the send within 3 s: commits {:?}, kicks {kicks:?}",
                s.probe.commit_counts()
            )
        });
        let (_, _, committed_at) = kicks
            .iter()
            .find(|(_, origin, _)| origin.is_none())
            .unwrap_or_else(|| panic!("no request-origin commit after the send: {kicks:?}"));
        assert!(
            kicks
                .iter()
                .any(|(lanes, origin, _)| lanes.contains(Lane::Wakes)
                    && *origin == Some(Lane::Deadlines)),
            "the deadline lane's materialization kicks the wake lane: {kicks:?}"
        );
        attempted.saturating_duration_since(*committed_at)
    }
}

/// Waits until every lane has finished a pass (the registry's Pacers).
fn wait_lanes_settled_latency(s: &lanes_latency::Session) {
    let until = Instant::now() + Duration::from_secs(20);
    while !Lane::ALL
        .iter()
        .all(|lane| s.probe.registered_idle_events(*lane) >= 1)
    {
        assert!(Instant::now() < until, "a lane never finished a pass");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The in-process daemon points the process stderr at its daemon.log, so a
/// failed assertion's message would vanish with the test's scratch directory.
/// With `HT_SWEEP_PANIC_FILE` set, panics are also appended there.
fn keep_panics_visible() {
    let Some(path) = std::env::var_os("HT_SWEEP_PANIC_FILE") else {
        return;
    };
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{info}\n");
        }
    }));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// `count` completed `send_attention` jobs `hist1..histN`, completed at
/// `completed_at` (ms since the epoch).
fn seed_completed_jobs(db: &rusqlite::Connection, count: u64, completed_at: u64) {
    db.execute_batch(&format!(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{count}) \
             INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) \
             SELECT 'hist'||x,'send_attention','hist'||x,0,'complete',{completed_at} FROM n"
    ))
    .unwrap();
}

fn count(db: &rusqlite::Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Every lane is registered with the commit-kick registry and a `WorkerStatus`
/// before any traffic, and then a send's commit is attempted by the wake lane
/// in under 100 ms, for three different recipients (the production worker
/// set, no tick wait).
/// Kills: a lane missing from the production worker set (the registration
/// check names it), and a send chain that falls back to the wake lane's 5 s
/// safety tick on a later send (one send alone would hide a tick that
/// happened to land).
#[test]
fn send_commit_wake_under_100ms_with_all_five_lanes() {
    let Some(scene) = Scene::new("sweep_send_commit_wake", 3) else {
        return;
    };
    for lane in Lane::ALL {
        assert!(
            scene.session().probe.registered(lane) && scene.session().probe.status_has_pacer(lane),
            "{} is not registered with both the kick registry and its status",
            lane.name()
        );
    }
    for round in 0..3 {
        let latency = scene.send_to_wake_attempt(round, &format!("sweep probe {round}"));
        assert!(
            latency < Duration::from_millis(100),
            "send {round}: commit to wake attempt took {latency:?}"
        );
    }
}

/// 6,000 completed jobs younger than the 24 h retention age do not slow a
/// send's wake attempt (discovery stays flat in settled history), retention
/// then deletes them once they age, in more than one bounded batch, keeps the
/// `preparation_cleanup` marker row, and the send is still attempted in under
/// 100 ms afterwards. The flat-cost counters themselves are the store tests
/// `work_discovery_is_flat_in_completed_jobs`,
/// `wake_discovery_is_flat_in_settled_history` and
/// `observation_walk_is_flat_in_retired_seats_and_superseded_generations`
/// (recorded in the sweep notes); this is the end-to-end form through the
/// daemon.
/// Kills: a retention pass that deletes in one unbounded transaction (fewer
/// than ceil(6000/batch) retention commits), one that prunes the kept marker
/// kind, a discovery path that follows settled history (the send under 6,000
/// settled rows would exceed 100 ms), and superseded snapshot generations that
/// accumulate.
#[test]
fn retention_keeps_tables_bounded_while_discovery_stays_flat() {
    let Some(scene) = Scene::new("sweep_retention_bounded", 2) else {
        return;
    };
    let s = scene.session();
    let db = s.db();
    const HISTORY: u64 = 6_000;
    seed_completed_jobs(&db, HISTORY, now_ms());
    db.execute(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) \
             VALUES ('keep-prep','preparation_cleanup','keep-prep',0,'complete',0)",
        [],
    )
    .unwrap();
    let hist = || count(&db, "SELECT count(*) FROM work_jobs WHERE id LIKE 'hist%'");
    assert_eq!(hist(), HISTORY as i64);
    let latency = scene.send_to_wake_attempt(0, "with history");
    assert!(
        latency < Duration::from_millis(100),
        "send under {HISTORY} settled jobs: commit to wake attempt took {latency:?}"
    );
    // Recent completions are not yet prunable: retention leaves them alone.
    s.probe.kick_registered(Lane::Retention);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(hist(), HISTORY as i64, "retention pruned unexpired jobs");

    // Age them, then let retention drain.
    s.db()
        .execute(
            "UPDATE work_jobs SET completed_at = 0 WHERE id LIKE 'hist%'",
            [],
        )
        .unwrap();
    let retention_from = s.commits("retention");
    let deadline_from = s.commits("deadline");
    let until = Instant::now() + Duration::from_secs(60);
    while hist() > 0 {
        assert!(
            Instant::now() < until,
            "retention left {} of {HISTORY} jobs after 60 s",
            hist()
        );
        s.probe.kick_registered(Lane::Retention);
        std::thread::sleep(Duration::from_millis(50));
    }
    let batches = s.commits("retention") - retention_from;
    let least = HISTORY.div_ceil(RETENTION_BATCH_ROWS as u64);
    assert!(
        batches >= least,
        "{HISTORY} rows need at least {least} bounded batches of {RETENTION_BATCH_ROWS}; \
         saw {batches} retention commits"
    );
    assert_eq!(
        s.commits("deadline"),
        deadline_from,
        "retention's prune commits woke the deadline lane into committing"
    );
    assert_eq!(
        count(&db, "SELECT count(*) FROM work_jobs WHERE id = 'keep-prep'"),
        1,
        "preparation_cleanup completions are never pruned"
    );
    let latency = scene.send_to_wake_attempt(1, "after retention");
    assert!(
        latency < Duration::from_millis(100),
        "send after the drain: commit to wake attempt took {latency:?}"
    );
    // Snapshot generations stay bounded: the observation lane publishes a new
    // one every few seconds and retention keeps the active, the previous and
    // the in-flight stage only.
    std::thread::sleep(Duration::from_secs(12));
    s.probe.kick_registered(Lane::Retention);
    std::thread::sleep(Duration::from_secs(1));
    let generations = count(&db, "SELECT count(*) FROM snapshot_generations");
    assert!(
        generations <= 6,
        "{generations} snapshot generations survive retention"
    );
}

/// The idle bound with a retention backlog. 30 s with all five lanes running
/// and 3,000 expired jobs to prune: no deadline, wake, request or
/// admission-observer commit; the wake lane makes at most one pass per 5 s
/// window; retention drains the backlog; and its prune commits kick no lane.
/// Kills: a retention prune whose kick wakes the wake or deadline lane (the
/// wake lane would pass more than once per window and commit), and a backlog
/// that makes retention hold the writer past the idle bound instead of
/// finishing in batches.
#[test]
fn retention_runs_alongside_the_wake_idle_bound() {
    let Some(s) = Session::new("sweep_retention_idle_bound") else {
        return;
    };
    keep_panics_visible();
    let pane = s.first_pane();
    s.seat(&pane);
    std::thread::sleep(Duration::from_secs(8));
    for origin in ["wake", "deadline", "request"] {
        s.wait_commits_quiet(origin, Duration::from_secs(2));
    }
    seed_completed_jobs(&s.db(), 3_000, 0);
    let counts = s.probe.commit_counts();
    let kicks_from = s.probe.kick_log().len();
    let wake_from = s.probe.registered_idle_events(Lane::Wakes);
    let window = Duration::from_secs(30);
    let started = Instant::now();
    while started.elapsed() < window {
        s.probe.kick_registered(Lane::Retention);
        std::thread::sleep(Duration::from_secs(5));
    }
    let elapsed = started.elapsed();
    let after = s.probe.commit_counts();
    for origin in ["deadline", "wake", "request", "admission-observer"] {
        assert_eq!(
            after.get(origin),
            counts.get(origin),
            "an idle daemon with a retention backlog committed from the {origin} origin: \
             {counts:?} -> {after:?}"
        );
    }
    assert_eq!(
        count(
            &s.db(),
            "SELECT count(*) FROM work_jobs WHERE id LIKE 'hist%'"
        ),
        0,
        "retention drained the backlog"
    );
    assert!(
        after["retention"] >= counts.get("retention").copied().unwrap_or(0) + 12,
        "3,000 rows are at least 12 batches: {counts:?} -> {after:?}"
    );
    let new_kicks = kicks_since(&s, kicks_from);
    assert!(
        new_kicks.is_empty(),
        "retention pruning kicked a lane: {new_kicks:?}"
    );
    let ticks = elapsed.as_secs() / 5 + 1;
    let wake_passes = s.probe.registered_idle_events(Lane::Wakes) - wake_from;
    assert!(
        (1..=ticks).contains(&wake_passes),
        "the wake lane made {wake_passes} passes in {elapsed:?} (at most {ticks})"
    );
}

/// Every lane fails at once, as a store outage would make them. Health stays
/// within its line budget (the docs' fold: more than two degraded lanes become
/// one summary line, not a line per lane), that one line names every lane and
/// the daemon log path, each lane wrote exactly one first-occurrence line to
/// daemon.log, and a later good pass clears Health back to its healthy lines.
/// Kills: a fold that is not applied through the daemon (five scheduler lines
/// or a pointer per lane), a summary that drops a lane or the log path, Health
/// lines beyond `HEALTH_LINE_BUDGET`, and a lane whose failure never reaches
/// the shared rate-limited log.
#[test]
fn lane_failure_surfaces_through_remedy_text_within_the_health_line_budget() {
    let Some(s) = Session::new("sweep_all_lanes_fail") else {
        return;
    };
    keep_panics_visible();
    let pane = s.first_pane();
    s.seat(&pane);
    wait_lanes_settled(&s, Duration::from_millis(1500));
    let (state, healthy) = s.health();
    assert_eq!(state, "healthy", "{healthy:?}");
    let log_before: Vec<usize> = Lane::ALL
        .iter()
        .map(|lane| s.lane_log_lines(*lane).len())
        .collect();
    for lane in Lane::ALL {
        s.probe.fail_lane(lane, ErrorCode::StoreBusy);
    }
    let until = Instant::now() + Duration::from_secs(40);
    while !Lane::ALL
        .iter()
        .all(|lane| s.probe.lane_health(*lane).is_some())
    {
        assert!(
            Instant::now() < until,
            "not every lane reported its failure: {:?}",
            Lane::ALL
                .iter()
                .map(|lane| (lane.name(), s.probe.lane_health(*lane)))
                .collect::<Vec<_>>()
        );
        for lane in Lane::ALL {
            s.probe.kick_registered(lane);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let (state, lines) = s.health();
    assert_eq!(state, "degraded", "{lines:?}");
    assert!(
        lines.len() <= HEALTH_LINE_BUDGET,
        "{} Health lines exceed the budget of {HEALTH_LINE_BUDGET}: {lines:?}",
        lines.len()
    );
    let scheduler: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("scheduler degraded"))
        .collect();
    assert_eq!(scheduler.len(), 1, "one folded summary line: {lines:?}");
    assert!(
        scheduler[0].contains("5 lanes degraded")
            && Lane::ALL
                .iter()
                .all(|lane| scheduler[0].contains(lane.name()))
            && scheduler[0].contains(&format!("see {}", s.log.display())),
        "{scheduler:?}"
    );
    assert!(
        lines.iter().all(|line| !line.starts_with("degraded: ")),
        "a folded summary carries the log path itself, not a pointer per lane: {lines:?}"
    );
    for (lane, before) in Lane::ALL.iter().zip(&log_before) {
        assert_eq!(
            s.lane_log_lines(*lane).len(),
            before + 1,
            "{}: one first-occurrence daemon.log line however many passes failed: {:?}; log: {}",
            lane.name(),
            s.lane_log_lines(*lane),
            s.daemon_log()
        );
    }
    for lane in Lane::ALL {
        s.probe.heal_lane(lane);
    }
    wait_until("every lane to clear", Duration::from_secs(60), || {
        for lane in Lane::ALL {
            s.probe.kick_registered(lane);
        }
        Lane::ALL
            .iter()
            .all(|lane| s.probe.lane_health(*lane).is_none())
    });
    let (state, cleared) = s.health();
    assert_eq!(state, "healthy", "{cleared:?}");
    assert_eq!(cleared, healthy, "Health returns to its healthy lines");
}

/// One operator, three failure classes, each its own `remedy()` text: a start
/// attempt that cannot open its database prints that attempt's startup-log
/// remedy (exit 3), a degraded lane prints the `degraded: <remedy naming daemon.log>`
/// pointer, and a daemon whose descriptor is one protocol version behind
/// prints the stop-then-ensure skew remedy and does not print the lane pointer
/// (a skewed daemon is not read at all) -- then, with the skew removed, the
/// lane pointer is back, so neither class hid the other.
/// Kills: a skew path that decodes or reads Health from the old daemon, a
/// lane pointer that survives into the skew report, a skew that makes the
/// degraded state forget itself, and a startup failure that prints the
/// daemon.log path instead of its own attempt log.
#[test]
fn startup_failure_lane_failure_and_skew_reach_the_operator() {
    let Some(s) = Session::new("sweep_three_classes") else {
        return;
    };
    keep_panics_visible();
    // 1. Startup failure, in its own state directory.
    {
        let root = std::path::PathBuf::from(format!(
            "/private/tmp/hosw-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        let state = root.join("st");
        fs::create_dir_all(&state).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let host = root.join("no-host.sock");
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(state.clone(), host.clone(), None).unwrap(),
        )
        .unwrap();
        paths.prepare_instance_dir().unwrap();
        fs::write(&paths.database_path, b"this is not a sqlite database").unwrap();
        let ensure = crate::scrubbed_command(env!("CARGO_BIN_EXE_herdr-threads"))
            .arg("--state-dir")
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host)
            .args(["daemon", "ensure"])
            .env("HOME", root.join("home"))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&ensure.stderr).into_owned();
        assert_eq!(ensure.status.code(), Some(3), "{stderr}");
        let logs: Vec<_> = fs::read_dir(paths.instance_dir.join("logs"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(logs.len(), 1, "one per-attempt startup log: {logs:?}");
        let expected = remedy(
            None,
            &RemedyContext::StartupFailure {
                log: logs[0].path(),
            },
        );
        assert!(
            stderr.lines().any(|line| line.contains(&expected)),
            "no line has {expected:?}: {stderr}"
        );
        assert!(
            !stderr.contains(&s.log.display().to_string()),
            "a startup failure names its own attempt log, not another daemon's: {stderr}"
        );
    }

    // 2. A degraded lane, on the running daemon.
    let pane = s.first_pane();
    s.seat(&pane);
    wait_lanes_settled(&s, Duration::from_millis(1500));
    let lane = Lane::Wakes;
    s.probe.fail_lane(lane, ErrorCode::StoreBusy);
    wait_until(
        "the wake lane to report its failure",
        Duration::from_secs(20),
        || {
            s.probe.kick_registered(lane);
            s.probe.lane_health(lane).is_some()
        },
    );
    let pointer = format!(
        "degraded: {}",
        remedy(
            Some(ErrorClass::Transient),
            &RemedyContext::LaneDegraded { log: s.log.clone() }
        )
    );
    let (state, lines) = s.health();
    assert_eq!(state, "degraded", "{lines:?}");
    assert!(lines.contains(&pointer), "{pointer:?} not in {lines:?}");

    // 3. Skew: the descriptor says the daemon speaks the previous protocol.
    let paths = InstancePaths::resolve(
        &RuntimeContext::explicit(s.state.clone(), s.herdr.socket_path(), None).unwrap(),
    )
    .unwrap();
    let original = fs::read(&paths.descriptor_path).unwrap();
    let published: Value = serde_json::from_slice(&original).unwrap();
    let software = published["software_version"].as_str().unwrap().to_owned();
    let mut skewed = published.clone();
    skewed["protocol_version"] = json!(PROTOCOL_VERSION - 1);
    fs::write(&paths.descriptor_path, serde_json::to_vec(&skewed).unwrap()).unwrap();
    let restore = |bytes: &[u8]| fs::write(&paths.descriptor_path, bytes).unwrap();
    let (code, _, stderr) = s.cli(None, &["daemon", "health"]);
    restore(&original);
    assert_eq!(code, 3, "{stderr}");
    let skew = remedy(
        Some(ErrorClass::VersionSkew),
        &RemedyContext::VersionSkew {
            daemon: format!("{software} (protocol {})", PROTOCOL_VERSION - 1),
            cli: format!(
                "{} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            ),
        },
    );
    assert!(stderr.contains(&skew), "{skew:?} not in {stderr}");
    assert!(
        !stderr.contains(&pointer),
        "the skew report is not the lane pointer: {stderr}"
    );

    // With the skew gone the degraded state, which lives in the daemon, is
    // still reported.
    let (state, lines) = s.health();
    assert_eq!(state, "degraded", "{lines:?}");
    assert!(lines.contains(&pointer), "{pointer:?} not in {lines:?}");
    s.probe.heal_lane(lane);
    wait_until("the wake lane to clear", Duration::from_secs(40), || {
        s.probe.lane_health(lane).is_none()
    });
}

/// Unwired-value sweep finding (ht-p03.51): `HOOK_PARSE_DETAIL_BYTES` was a
/// constant nothing read, and the CLI cut the detail to 256 characters while
/// the wire rejects more than 256 bytes, so a multi-byte diagnostic (the
/// `{error:?}` of a payload that carried non-ASCII text) was dropped by
/// validation instead of reported. The CLI's cut now is the wire's byte bound.
/// Kills: a character-count truncation (300 two-byte characters would be 600
/// bytes and fail `validate`), a cut inside a character (a panic or invalid
/// text), and a bound that drifts from the validation limit.
#[test]
fn hook_parse_detail_cut_is_the_wire_bound_for_multibyte_text() {
    let long = "\u{e9}".repeat(300);
    let detail = bounded_hook_detail(&long);
    assert!(
        detail.len() <= HOOK_PARSE_DETAIL_BYTES,
        "{} bytes",
        detail.len()
    );
    assert_eq!(
        detail.len(),
        HOOK_PARSE_DETAIL_BYTES,
        "two-byte characters fill the bound exactly"
    );
    let report = |detail: String| {
        Command::HookParseFailure(HookParseFailure {
            harness: "claude".into(),
            detail,
        })
        .validate()
    };
    report(detail).expect("the CLI's own cut passes the wire validation");
    assert!(
        report("\u{e9}".repeat(HOOK_PARSE_DETAIL_BYTES / 2 + 1)).is_err(),
        "one byte over the bound is rejected"
    );
    // A three-byte character straddling the bound is cut whole.
    let straddle = format!("{}{}", "a".repeat(HOOK_PARSE_DETAIL_BYTES - 1), "\u{20ac}");
    assert_eq!(
        bounded_hook_detail(&straddle),
        "a".repeat(HOOK_PARSE_DETAIL_BYTES - 1)
    );
    assert_eq!(bounded_hook_detail("short"), "short");
}
