//! ht-p03.9.5: the observation lane on the Pacer, against an isolated named
//! Herdr test session (never the shared server). The lane runs exactly as the
//! daemon wires it (real `SqliteStore`, `NativeCli` host, `OrdinaryIdentity`,
//! `start_observation_worker`, one lane Pacer); only the process boundary is
//! dropped so the store's per-origin commit counter and kick sink are
//! readable. Herdr is stopped and restarted under the running lane.
use herdr_threads::{
    app::SystemClock,
    host::native::NativeCli,
    identity::repair::OrdinaryIdentity,
    ports::StorePort,
    protocol::time::{Cancellation, Clock, MonoInstant, UtcMillis},
    service::{
        fair_writer::FairWriter,
        host_evidence::HostEvidenceStatus,
        kicks::{Lane, LaneSet},
        pacer::Pacer,
        workers::{WorkerStatus, start_observation_worker},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolated_herdr::IsolatedHerdr,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

struct StepClock(AtomicU64);
impl Clock for StepClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

type Kicked = Arc<Mutex<Vec<(LaneSet, Option<Lane>)>>>;

struct Lane5 {
    scratch: std::path::PathBuf,
    store: Arc<SqliteStore>,
    pacer: Arc<Pacer>,
    status: Arc<WorkerStatus>,
    cancel: Cancellation,
    kicked: Kicked,
    worker: Option<JoinHandle<()>>,
}

impl Lane5 {
    fn start(herdr: &IsolatedHerdr) -> Self {
        Self::start_with_clock(herdr, Arc::new(SystemClock::new()))
    }

    fn start_with_clock(herdr: &IsolatedHerdr, clock: Arc<dyn Clock>) -> Self {
        let scratch = herdr.root().join("lane-state");
        std::fs::create_dir_all(&scratch).unwrap();
        let instance = uuid::Uuid::new_v4().to_string();
        let store = SqliteStore::new(
            StoreContext::new(scratch.join("store.db"), Arc::clone(&clock)),
            instance.clone(),
            StoreSettings::default(),
        )
        .unwrap();
        let kicked: Kicked = Arc::default();
        let sink = Arc::clone(&kicked);
        store.set_kick_sink(Box::new(move |lanes, origin| {
            sink.lock().unwrap().push((lanes, origin));
        }));
        let store = Arc::new(store);
        let host = Arc::new(NativeCli::new(herdr.socket_path(), Arc::clone(&clock)));
        let cancel = Cancellation::default();
        let pacer = Arc::new(Pacer::new(
            Lane::Observation.name(),
            Arc::clone(&clock),
            cancel.clone(),
        ));
        let writer = Arc::new(FairWriter::new(32));
        let identity = Arc::new(
            OrdinaryIdentity::new(
                instance,
                Arc::clone(&store) as Arc<dyn StorePort>,
                host,
                clock,
                Arc::clone(&writer),
            )
            .with_observation_pacer(Arc::clone(&pacer)),
        );
        let status = Arc::new(WorkerStatus::default());
        let worker = start_observation_worker(
            identity,
            Arc::clone(&store) as Arc<dyn StorePort>,
            writer,
            cancel.clone(),
            Arc::clone(&status),
            Arc::new(HostEvidenceStatus::default()),
            Arc::clone(&pacer),
            Arc::new(Default::default()),
        )
        .unwrap();
        Self {
            scratch,
            store,
            pacer,
            status,
            cancel,
            kicked,
            worker: Some(worker),
        }
    }

    fn commits(&self) -> u64 {
        self.store.commit_counts()["observation"]
    }

    fn invalidation_revision(&self) -> i64 {
        rusqlite::Connection::open(self.scratch.join("store.db"))
            .unwrap()
            .query_row(
                "SELECT invalidation_revision FROM host_instances",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn health(&self) -> String {
        self.status
            .health()
            .map(|health| health.summary())
            .unwrap_or_default()
    }

    fn wait(&self, what: &str, limit: Duration, done: &dyn Fn() -> bool) {
        let until = Instant::now() + limit;
        while !done() {
            assert!(
                Instant::now() < until,
                "timed out waiting for {what}; health: {:?}, attempts {}",
                self.health(),
                self.pacer.attempts()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Lane5 {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn describe(kicks: &[(LaneSet, Option<Lane>)]) -> String {
    let mut lines = Vec::new();
    for (lanes, origin) in kicks {
        let names: Vec<_> = lanes.iter().map(Lane::name).collect();
        lines.push(format!("origin={origin:?} kicked={names:?}"));
    }
    lines.join("; ")
}

/// Herdr stopped under the running lane is unavailability, not evidence
/// (TRUST-POLICY C4, ht-yms): every failed capture, the first included, costs
/// at most its admission commit; no invalidation is written, so nothing is
/// marked and nothing is kicked, and Health shows the frozen state with the
/// lane's capped retry.
/// Kills: a frozen capture writing an invalidation or arming a marking
/// continuation (the first step makes more than one commit, kicks lanes, or
/// moves `invalidation_revision`), and the lane not backing off.
#[test]
fn herdr_stopped_costs_one_commit_per_backoff_step() {
    let Some(herdr) = IsolatedHerdr::new("herdr_stopped_costs_one_commit_per_backoff_step") else {
        return;
    };
    herdr.start();
    let clock = Arc::new(StepClock(AtomicU64::new(10_000)));
    let lane = Lane5::start_with_clock(&herdr, clock.clone());
    lane.wait("the first publication", Duration::from_secs(20), &|| {
        lane.commits() >= 3
            && lane.pacer.attempts() == 0
            && lane.pacer.idle_events() >= 1
            && lane.pacer.idle_events() == lane.pacer.wakes() + 1
    });
    assert_eq!(lane.health(), "", "healthy while Herdr is up");
    let revision = lane.invalidation_revision();
    herdr.stop();
    let kicks_before = lane.kicked.lock().unwrap().len();
    let mut per_step = Vec::new();
    for attempt in 1..=10 {
        let before = lane.commits();
        let idle = lane.pacer.idle_events();
        let due = if attempt == 1 {
            // The healthy observation cadence is 5 s. Time remains frozen
            // throughout each pass so a sample cannot include another retry.
            MonoInstant(clock.monotonic_now().0 + 5_000)
        } else {
            lane.pacer.next_retry_at().expect("retry pending")
        };
        clock.0.store(due.0, Ordering::SeqCst);
        lane.pacer.clock_advanced();
        lane.wait(
            "one completed failed capture",
            Duration::from_secs(30),
            &|| {
                lane.pacer.idle_events() > idle
                    && lane.pacer.idle_events() == lane.pacer.wakes() + 1
            },
        );
        assert_eq!(
            lane.pacer.idle_events(),
            idle + 1,
            "one pass per clock step"
        );
        assert_eq!(lane.pacer.attempts(), attempt);
        let commits = lane.commits() - before;
        per_step.push((attempt, commits));
        assert!(
            commits <= 1,
            "attempt {attempt} made {commits} durable commits: {per_step:?}"
        );
        let kicks = lane.kicked.lock().unwrap()[kicks_before..].to_vec();
        assert!(
            kicks.is_empty(),
            "a frozen capture kicked lanes: [{}]",
            describe(&kicks)
        );
        assert_eq!(
            lane.invalidation_revision(),
            revision,
            "Herdr unavailability wrote an invalidation"
        );
    }
    println!("per-step observation commits (attempt, commits): {per_step:?}");
    let (_, next) = lane.status.retry().expect("retry pending");
    let remaining = next.0.saturating_sub(lane.pacer.now().0);
    assert!(remaining <= 30_000, "capped wait is <= 30 s: {remaining}");
    let health = lane.health();
    assert!(
        health.contains(
            "host unavailable (HostUnavailable): seats and bindings frozen until Herdr answers"
        ) && health.contains("retrying (attempt "),
        "{health}"
    );
    assert_eq!(lane.invalidation_revision(), revision);
}

#[test]
fn herdr_restart_resets_backoff_to_5s_cadence() {
    let Some(herdr) = IsolatedHerdr::new("herdr_restart_resets_backoff_to_5s_cadence") else {
        return;
    };
    herdr.start();
    let lane = Lane5::start(&herdr);
    lane.wait("the first publication", Duration::from_secs(20), &|| {
        lane.commits() >= 3 && lane.pacer.attempts() == 0 && lane.pacer.idle_events() >= 1
    });

    herdr.stop();
    lane.wait("three failed captures", Duration::from_secs(30), &|| {
        lane.pacer.attempts() >= 3
    });
    assert!(lane.health().contains("retrying (attempt "));

    herdr.restart();
    lane.wait(
        "the first publish after restart to reset the backoff",
        Duration::from_secs(40),
        &|| lane.pacer.attempts() == 0 && lane.status.retry().is_none(),
    );
    assert_eq!(lane.health(), "", "a publish clears the retry state");

    // Back on the 5 s cadence: consecutive cycles are about 5 s apart.
    let mut cycles = Vec::new();
    let mut seen = lane.commits();
    let started = Instant::now();
    while cycles.len() < 2 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "no steady cadence after restart: {cycles:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
        let now = lane.commits();
        if now != seen {
            cycles.push(Instant::now());
            // Skip the cycle's own remaining commits.
            std::thread::sleep(Duration::from_millis(500));
            seen = lane.commits();
        }
    }
    let gap = cycles[1].duration_since(cycles[0]);
    assert!(
        (Duration::from_millis(4_000)..=Duration::from_millis(7_000)).contains(&gap),
        "cycle gap after restart is the 5 s cadence, got {gap:?}"
    );
    assert_eq!(lane.pacer.attempts(), 0);
}
