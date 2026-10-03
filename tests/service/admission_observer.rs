//! The admission-observer lane (root spec B2, nested spec D2): backoff and
//! reset, cancellation, idle commits and passes per tick, Health retry suffix
//! and the checked spawn. The clock is fake, so a tick is one `advance`.
use super::*;
use crate::{
    app::HarnessObservations,
    daemon::health::HarnessStatus,
    protocol::time::{MonoInstant, UtcMillis},
    service::kicks::CommitKicks,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize};
use std::time::Instant;

const TICK_MS: u64 = ADMISSION_TICK.as_millis() as u64;

struct FakeClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn observed(detail: &str) -> HarnessObservations {
    HarnessObservations {
        claude: HarnessStatus::NotInstalled(detail.into()),
        codex: HarnessStatus::NotInstalled(detail.into()),
    }
}

/// One admission lane around an injected observer.
struct Fx {
    clock: Arc<FakeClock>,
    pacer: Arc<Pacer>,
    status: Arc<WorkerStatus>,
    slot: Arc<Mutex<HarnessObservations>>,
    cancel: Cancellation,
    passes: Arc<AtomicUsize>,
    /// Observer fails while set.
    failing: Arc<AtomicBool>,
    /// The thread-local lane origin the observer saw on its last pass.
    origin: Arc<Mutex<Option<Option<Lane>>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Fx {
    fn start(failing: bool) -> Self {
        let clock = Arc::new(FakeClock {
            mono: AtomicU64::new(1),
            utc: AtomicI64::new(1_000_000),
        });
        let cancel = Cancellation::default();
        let pacer = Arc::new(Pacer::new(
            Lane::AdmissionObserver.name(),
            clock.clone(),
            cancel.clone(),
        ));
        let mut fx = Self {
            clock: clock.clone(),
            pacer: Arc::clone(&pacer),
            status: Arc::new(WorkerStatus::default()),
            slot: Arc::new(Mutex::new(HarnessObservations::default())),
            cancel: cancel.clone(),
            passes: Arc::new(AtomicUsize::new(0)),
            failing: Arc::new(AtomicBool::new(failing)),
            origin: Arc::new(Mutex::new(None)),
            handle: None,
        };
        let (passes, failing, origin) = (
            Arc::clone(&fx.passes),
            Arc::clone(&fx.failing),
            Arc::clone(&fx.origin),
        );
        fx.handle = Some(
            start_admission_observer(
                move |_budget| {
                    passes.fetch_add(1, Ordering::SeqCst);
                    *origin.lock().unwrap() = Some(kicks::current_origin());
                    if failing.load(Ordering::SeqCst) {
                        Err(ApiError::service_busy("injected observation failure"))
                    } else {
                        Ok(observed("fresh"))
                    }
                },
                Arc::clone(&fx.slot),
                pacer,
                clock,
                cancel,
                Arc::clone(&fx.status),
            )
            .unwrap(),
        );
        fx
    }

    fn advance(&self, ms: u64) {
        self.clock.mono.fetch_add(ms, Ordering::SeqCst);
        self.clock.utc.fetch_add(ms as i64, Ordering::SeqCst);
        self.pacer.clock_advanced();
    }

    fn wait_idle(&self, n: u64) {
        wait_until("the lane to wait", || self.pacer.idle_events() >= n);
    }

    fn passes(&self) -> usize {
        self.passes.load(Ordering::SeqCst)
    }

    fn stop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn failing_observation_backs_off_and_resets() {
    let fx = Fx::start(true);
    for attempt in 1..=11u32 {
        wait_until("the failure", || fx.pacer.attempts() == attempt);
        let delay = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
        let nominal = (100u64 << (attempt - 1).min(20)).min(30_000);
        let low = nominal * 8 / 10;
        let high = (nominal * 12 / 10).min(30_000);
        assert!(
            (low..=high).contains(&delay),
            "attempt {attempt}: retry in {delay} ms, expected {low}..={high}"
        );
        assert_eq!(
            fx.passes(),
            attempt as usize,
            "no pass runs before the backoff is due"
        );
        fx.advance(delay);
    }
    assert!(
        fx.slot.lock().unwrap().claude == HarnessStatus::Unknown,
        "a failed pass leaves the slot untouched"
    );
    // A healthy pass resets the schedule and publishes the observation.
    wait_until("the 12th failure", || fx.pacer.attempts() == 12);
    fx.failing.store(false, Ordering::SeqCst);
    let due = fx.pacer.next_retry_at().unwrap().0 - fx.pacer.now().0;
    fx.advance(due);
    // The worker publishes a success in three steps (slot, pacer reset,
    // status), so wait for the last one rather than the pacer's reset.
    wait_until("the retry to succeed and clear the failure", || {
        fx.pacer.attempts() == 0 && fx.status.health().is_none()
    });
    assert!(fx.status.retry().is_none());
    assert_eq!(*fx.slot.lock().unwrap(), observed("fresh"));
    assert!(fx.status.last_tick().is_some());
}

#[test]
fn cancellation_stops_the_blocked_lane_within_20ms() {
    let mut fx = Fx::start(false);
    fx.wait_idle(1);
    let started = Instant::now();
    fx.stop();
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(20),
        "a lane blocked in its tick stopped after {elapsed:?}"
    );
    assert!(
        !fx.status.lane_dead(),
        "a requested stop is not a dead lane"
    );
}

#[test]
fn idle_observer_makes_no_commits_and_one_pass_per_tick() {
    assert!(ADMISSION_TICK >= Duration::from_secs(5), "tick floor");
    let iso = TestIsolation::new("admission-idle");
    let clock_for_store = Arc::new(FakeClock {
        mono: AtomicU64::new(1),
        utc: AtomicI64::new(1_000_000),
    });
    let kick_registry = Arc::new(CommitKicks::default());
    let store = SqliteStore::new(
        StoreContext::new(iso.state_root().join("store.db"), clock_for_store),
        "i",
        StoreSettings::default(),
    )
    .unwrap()
    .with_commit_kicks(kick_registry);
    let before: u64 = store.commit_counts().values().sum();

    let fx = Fx::start(false);
    let idle = Arc::new(AtomicU64::new(0));
    let hook_idle = Arc::clone(&idle);
    fx.pacer.set_idle_hook(Box::new(move |count| {
        hook_idle.store(count, Ordering::SeqCst)
    }));
    fx.wait_idle(1);
    assert_eq!(fx.passes(), 1, "the first pass runs at once");
    assert_eq!(
        *fx.origin.lock().unwrap(),
        Some(Some(Lane::AdmissionObserver)),
        "the pass runs under its own lane origin"
    );

    // Thirty seconds of an unchanged binary: no pass, no wait re-entry.
    for _ in 0..6 {
        fx.advance(5_000);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(fx.passes(), 1, "no second pass inside one safety tick");
    assert_eq!(fx.pacer.idle_events(), 1);

    // Crossing the tick yields exactly one more pass.
    fx.advance(TICK_MS - 30_000);
    fx.wait_idle(2);
    assert_eq!(fx.passes(), 2, "one pass per safety tick");
    assert_eq!(
        idle.load(Ordering::SeqCst),
        2,
        "the idle hook saw both waits"
    );

    let counts = store.commit_counts();
    assert_eq!(
        counts.get("admission-observer").copied().unwrap_or(0),
        0,
        "the admission observer makes no durable commit"
    );
    assert_eq!(counts.values().sum::<u64>(), before, "no commit by anyone");
}

#[test]
fn failing_observation_shows_retry_suffix_without_a_new_health_line() {
    let fx = Fx::start(true);
    // The pacer counts the failure before `record_failure` publishes it.
    wait_until("the failure", || {
        fx.pacer.attempts() == 1 && fx.status.health().is_some()
    });
    let summary = fx
        .status
        .health()
        .expect("a failed pass degrades")
        .summary();
    assert!(
        summary.starts_with("lane admission-observer failed:"),
        "{summary}"
    );
    assert!(summary.contains("; retrying (attempt 1,"), "{summary}");
    assert_eq!(summary.lines().count(), 1, "one Health line: {summary}");
    assert!(
        !summary.contains("injected"),
        "no observer detail in Health: {summary}"
    );
}

#[test]
fn spawn_failure_is_recorded() {
    let status = WorkerStatus::default();
    assert!(status.health().is_none());
    let handle = admission_handle(Err(std::io::Error::other("no threads")), &status);
    assert!(handle.is_none());
    let summary = status.health().expect("a spawn failure degrades").summary();
    assert_eq!(summary, "lane admission-observer failed to start");
    // A spawned lane is passed through untouched.
    let ok = admission_handle(Ok(thread::spawn(|| {})), &status);
    ok.expect("a spawned handle is kept").join().unwrap();
}
