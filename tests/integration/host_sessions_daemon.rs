//! ht-5fr.2.8: the elected daemon composes `HostSessions` from a
//! `HostAttachPlan`: the hint and every remembered server attach at startup, the
//! sweep attaches a server whose socket appears later, and the stop path
//! joins every session thread. The daemon runs in this process (the production
//! `run_elected_probed` composition) over fake-locator hosts; no Herdr server,
//! no child process.
use herdr_threads::{
    app::{LaneProbe, SystemClock, run_elected_probed},
    daemon::paths::NodePaths,
    ports::HostPort,
    protocol::{
        ids::HostLocator,
        time::{Cancellation, Clock},
    },
    service::{config::ServiceConfig, kicks::Lane},
    test_support::{FakeHostWorld, RefusingHost, isolation::TestIsolation},
};
use std::{
    sync::Arc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

fn loc(text: &str) -> HostLocator {
    HostLocator::parse(text).unwrap()
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One in-process elected daemon over `world`, stopped (and joined) on drop.
struct Daemon {
    probe: LaneProbe,
    stop: Cancellation,
    thread: Option<JoinHandle<std::io::Result<bool>>>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Daemon {
    fn start(
        isolation: &TestIsolation,
        world: &Arc<FakeHostWorld>,
        hint: Option<HostLocator>,
        sweep: Duration,
    ) -> Self {
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        let paths: NodePaths = herdr_threads::test_support::resolve_seeded(isolation.state_root());
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let probe = LaneProbe::default();
        probe.set_host_sweep_interval(sweep);
        probe.log_lane_errors_to(isolation.path("lanes.log"));
        let stop = Cancellation::default();
        let plan = world.plan(hint);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (daemon_stop, daemon_probe) = (stop.clone(), probe.clone());
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected_probed(
                    &paths,
                    clock,
                    daemon_stop,
                    ServiceConfig::default(),
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
        // The elected daemon installs a quiet panic hook; restore visible failures.
        std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
        Self {
            probe,
            stop,
            thread: Some(thread),
            _guard: guard,
        }
    }

    fn has_host_lanes(&self, locator: &HostLocator) -> bool {
        self.probe.host_pacer(locator, Lane::Observation).is_some()
            && self.probe.host_pacer(locator, Lane::Wakes).is_some()
    }

    /// Stops the daemon and waits for its drain (which joins every session thread).
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

impl Drop for Daemon {
    fn drop(&mut self) {
        self.shut_down();
    }
}

fn refusing() -> Arc<dyn HostPort> {
    Arc::new(RefusingHost)
}

// Kills: a daemon that attaches nothing at startup, or registers the hint's
// workers without handing their Pacers to the probe.
#[test]
fn hint_host_is_attached_at_startup_with_its_pacers() {
    let isolation = TestIsolation::new("hs-daemon-hint");
    let world = FakeHostWorld::new();
    let hint = loc("/herdr/hint.sock");
    world.add(&hint, refusing());
    let daemon = Daemon::start(
        &isolation,
        &world,
        Some(hint.clone()),
        Duration::from_secs(60),
    );
    assert!(
        daemon.has_host_lanes(&hint),
        "the hint's observation and wake pacers exist at startup"
    );
    assert!(!daemon.has_host_lanes(&loc("/herdr/other.sock")));
    assert!(daemon.probe.registered(Lane::Wakes));
    assert!(daemon.probe.registered(Lane::Observation));
}

// Kills: startup attaching only the hint (or only one remembered row).
#[test]
fn restart_reattaches_two_remembered_rows() {
    let isolation = TestIsolation::new("hs-daemon-restart");
    let world = FakeHostWorld::new();
    let (a, b) = (loc("/herdr/a.sock"), loc("/herdr/b.sock"));
    world.add(&a, refusing());
    world.add(&b, refusing());
    // Two runs, one hint each, remember both rows.
    for hint in [&a, &b] {
        let daemon = Daemon::start(
            &isolation,
            &world,
            Some(hint.clone()),
            Duration::from_secs(60),
        );
        assert!(daemon.has_host_lanes(hint));
    }
    // A restart with no hint attaches both remembered rows.
    let daemon = Daemon::start(&isolation, &world, None, Duration::from_secs(60));
    assert!(daemon.has_host_lanes(&a), "remembered row a re-attached");
    assert!(daemon.has_host_lanes(&b), "remembered row b re-attached");
    assert_eq!(daemon.probe.host_sessions().unwrap().attached_count(), 2);
}

// Kills: a sweep that never runs in the composed daemon.
#[test]
fn sweep_attaches_row_whose_socket_appears() {
    let isolation = TestIsolation::new("hs-daemon-sweep");
    let world = FakeHostWorld::new();
    let (a, b) = (loc("/herdr/a.sock"), loc("/herdr/b.sock"));
    world.add(&b, refusing());
    drop(Daemon::start(
        &isolation,
        &world,
        Some(b.clone()),
        Duration::from_secs(60),
    ));
    // b is remembered now; its socket disappears, a live hint starts the daemon.
    world.set_live(&b, false);
    world.add(&a, refusing());
    let daemon = Daemon::start(
        &isolation,
        &world,
        Some(a.clone()),
        Duration::from_millis(20),
    );
    assert!(daemon.has_host_lanes(&a));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!daemon.has_host_lanes(&b), "no socket, no attach");
    world.set_live(&b, true);
    wait_until("the sweep to attach b", || daemon.has_host_lanes(&b));
}

// Kills: a stop path that leaves session threads running (the host port is
// still held by a wake worker) or never stops the sessions.
#[test]
fn stop_joins_every_session_thread() {
    let isolation = TestIsolation::new("hs-daemon-stop");
    let world = FakeHostWorld::new();
    let hint = loc("/herdr/hint.sock");
    let host = refusing();
    world.add(&hint, Arc::clone(&host));
    let mut daemon = Daemon::start(
        &isolation,
        &world,
        Some(hint.clone()),
        Duration::from_millis(20),
    );
    let sessions = daemon.probe.host_sessions().unwrap();
    let running = Arc::strong_count(&host);
    daemon.shut_down();
    assert!(
        sessions
            .attached()
            .iter()
            .all(|session| session.is_stopped()),
        "every session is stopped"
    );
    assert!(
        Arc::strong_count(&host) < running,
        "the wake worker released its host port by exiting ({running} -> {})",
        Arc::strong_count(&host)
    );
}
