use super::*;
use crate::daemon::logs::LaneErrorLog;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::pacer::Wake;
use crate::service::workers::WorkerStatus;
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

fn pacer(lane: &'static str) -> Arc<Pacer> {
    Arc::new(Pacer::new(
        lane,
        Arc::new(FixedClock),
        Cancellation::default(),
    ))
}

#[derive(Default)]
struct CapturingLog(Mutex<Vec<(Lane, ErrorCode)>>);
impl LaneErrorLog for CapturingLog {
    fn record(&self, lane: Lane, error: &ApiError) {
        self.0.lock().unwrap().push((lane, error.code.clone()));
    }
}

#[test]
fn register_then_direct_kick_reaches_the_pacer() {
    let kicks = CommitKicks::default();
    let pacer = pacer("wake");
    kicks.register(Lane::Wakes, pacer.clone());
    assert!(kicks.registered(Lane::Wakes));
    let (tx, rx) = mpsc::channel();
    let waiter = pacer.clone();
    std::thread::spawn(move || {
        let wake = waiter.wait_blocking(Duration::from_secs(5));
        let _ = tx.send((wake, Instant::now()));
    });
    // Let the lane block, then kick only the other lane first: no wake.
    std::thread::sleep(Duration::from_millis(30));
    kicks.kick(LaneSet::EMPTY.with(Lane::Deadlines));
    assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
    let at = Instant::now();
    kicks.kick(LaneSet::EMPTY.with(Lane::Wakes));
    let (wake, woke) = rx.recv_timeout(Duration::from_secs(2)).expect("lane stuck");
    assert_eq!(wake, Wake::Kicked);
    assert!(woke.saturating_duration_since(at) < Duration::from_millis(20));
}

#[test]
fn kick_to_an_unregistered_lane_is_a_noop() {
    let kicks = CommitKicks::default();
    kicks.register(Lane::Wakes, pacer("wake"));
    assert!(!kicks.registered(Lane::Retention));
    kicks.kick(LaneSet::EMPTY.with(Lane::Retention).with(Lane::Observation));
    assert!(!kicks.registered(Lane::Observation));
}

#[test]
fn lane_set_is_a_set_over_all_lanes() {
    let set = LaneSet::EMPTY
        .with(Lane::Wakes)
        .with(Lane::AdmissionObserver);
    assert_eq!(
        set.iter().collect::<Vec<_>>(),
        vec![Lane::Wakes, Lane::AdmissionObserver]
    );
    assert!(!set.without(Lane::Wakes).contains(Lane::Wakes));
    assert!(
        set.union(LaneSet::EMPTY.with(Lane::Retention))
            .contains(Lane::Retention)
    );
    assert_eq!(lanes_for_table("host_instances"), LaneSet::EMPTY);
    let names: Vec<_> = Lane::ALL.iter().map(|lane| lane.name()).collect();
    assert_eq!(
        names,
        [
            "deadline",
            "wake",
            "observation",
            "retention",
            "admission-observer",
            "archival"
        ]
    );
}

#[test]
fn enter_lane_is_scoped_and_restores() {
    assert_eq!(current_origin(), None);
    {
        let _outer = enter_lane(Lane::Deadlines);
        assert_eq!(current_origin(), Some(Lane::Deadlines));
        {
            let _inner = enter_lane(Lane::Retention);
            assert_eq!(current_origin(), Some(Lane::Retention));
        }
        assert_eq!(current_origin(), Some(Lane::Deadlines));
        let child = std::thread::spawn(current_origin).join().unwrap();
        assert_eq!(child, None);
    }
    assert_eq!(current_origin(), None);
}

#[tokio::test]
async fn spawn_blocking_threads_carry_no_origin() {
    let _guard = enter_lane(Lane::Observation);
    assert_eq!(current_origin(), Some(Lane::Observation));
    let blocking = tokio::task::spawn_blocking(current_origin).await.unwrap();
    assert_eq!(blocking, None);
}

/// The Retention row of the classification is empty by decision: no table, in
/// the mapped lists or among the explicitly unmapped ones, kicks the lane.
#[test]
fn no_table_maps_to_the_retention_lane() {
    let tables: Vec<&str> = mapped_tables()
        .chain(KNOWN_UNMAPPED.iter().copied())
        .collect();
    assert!(tables.len() > 30, "classification covers every table");
    for table in tables {
        assert!(
            !lanes_for_table(table).contains(Lane::Retention),
            "{table} must not kick the retention lane"
        );
    }
}

#[test]
fn retention_origin_discards_kicks() {
    assert!(origin_discards_kicks(Some(Lane::Retention)));
    assert!(!origin_discards_kicks(None));
    for lane in Lane::ALL
        .into_iter()
        .filter(|lane| *lane != Lane::Retention)
    {
        assert!(!origin_discards_kicks(Some(lane)), "{lane:?}");
    }
}

#[test]
fn record_failure_reaches_the_lane_error_log() {
    let status = WorkerStatus::default();
    let log = Arc::new(CapturingLog::default());
    status.set_error_log(log.clone());
    status.record_failure(Lane::Wakes, &ApiError::store_busy("x"));
    assert_eq!(
        *log.0.lock().unwrap(),
        vec![(Lane::Wakes, ErrorCode::StoreBusy)]
    );
    assert!(status.health().is_some());
    assert_eq!(status.last_tick(), None);
    status.record_success(UtcMillis(42));
    assert_eq!(status.last_tick(), Some(UtcMillis(42)));
    assert!(status.health().is_none(), "a good pass clears the failure");
}

#[test]
fn lane_spawn_failure_marks_the_lane_failed() {
    let status = WorkerStatus::default();
    let log = Arc::new(CapturingLog::default());
    status.set_error_log(log.clone());
    let io_error = std::io::Error::other("no threads");
    status.record_spawn_failure(Lane::AdmissionObserver, &io_error);
    let health = status.health().expect("degraded");
    assert_eq!(health.summary(), "lane admission-observer failed to start");
    assert_eq!(log.0.lock().unwrap().len(), 1);
    assert_eq!(log.0.lock().unwrap()[0].0, Lane::AdmissionObserver);
    // A later successful pass does not erase a lane that never started.
    status.record_success(UtcMillis(1));
    assert!(status.health().is_some());
}
