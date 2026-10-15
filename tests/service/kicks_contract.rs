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

/// True when a kick is already latched on `pacer`. The test clock is fixed, so
/// a wait on an unkicked pacer would never time out: probe from a thread and
/// release it afterwards.
fn observes_kick(pacer: &Arc<Pacer>) -> bool {
    let (tx, rx) = mpsc::channel();
    let waiter = pacer.clone();
    std::thread::spawn(move || {
        let _ = tx.send(waiter.wait_blocking(Duration::from_secs(5)));
    });
    let seen = rx.recv_timeout(Duration::from_millis(200)).ok() == Some(Wake::Kicked);
    if !seen {
        pacer.kick();
        let _ = rx.recv_timeout(Duration::from_secs(2));
    }
    seen
}

#[test]
fn kick_reaches_every_registered_pacer() {
    let kicks = CommitKicks::default();
    let (a, b) = (pacer("a"), pacer("b"));
    kicks.register(Lane::Wakes, a.clone());
    kicks.register(Lane::Wakes, b.clone());
    kicks.kick(LaneSet::EMPTY.with(Lane::Wakes));
    assert!(observes_kick(&a), "first pacer missed the kick");
    assert!(observes_kick(&b), "second pacer missed the kick");
}

#[test]
fn unregister_removes_exactly_one() {
    let kicks = CommitKicks::default();
    let (a, b, c) = (pacer("a"), pacer("b"), pacer("c"));
    for p in [&a, &b, &c] {
        kicks.register(Lane::Wakes, p.clone());
    }
    kicks.unregister(Lane::Wakes, &b);
    kicks.kick(LaneSet::EMPTY.with(Lane::Wakes));
    assert!(observes_kick(&a));
    assert!(observes_kick(&c));
    assert!(!observes_kick(&b), "unregistered pacer was still kicked");
    // Unregistering a pacer that is not registered changes nothing.
    kicks.unregister(Lane::Wakes, &b);
    assert!(kicks.registered(Lane::Wakes));
}

#[test]
fn registered_while_at_least_one_remains() {
    let kicks = CommitKicks::default();
    let (a, b) = (pacer("a"), pacer("b"));
    kicks.register(Lane::Wakes, a.clone());
    kicks.register(Lane::Wakes, b.clone());
    kicks.unregister(Lane::Wakes, &a);
    assert!(kicks.registered(Lane::Wakes));
    kicks.unregister(Lane::Wakes, &b);
    assert!(!kicks.registered(Lane::Wakes));
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
    assert_eq!(lanes_for_table("herdr_hosts"), LaneSet::EMPTY);
    let names: Vec<_> = Lane::ALL.iter().map(|lane| lane.name()).collect();
    assert_eq!(
        names,
        [
            "deadline",
            "wake",
            "observation",
            "retention",
            "admission-observer",
            "archival",
            "apply",
            "sync"
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
fn retention_origin_discards_all() {
    let all = Lane::ALL
        .into_iter()
        .fold(LaneSet::EMPTY, |set, lane| set.with(lane));
    assert_eq!(kicks_for_commit(Some(Lane::Retention), all), LaneSet::EMPTY);
    assert_eq!(kicks_for_commit(None, all), all);
    for lane in Lane::ALL
        .into_iter()
        .filter(|lane| !matches!(lane, Lane::Retention | Lane::Apply))
    {
        assert_eq!(kicks_for_commit(Some(lane), all), all, "{lane:?}");
    }
}

#[test]
fn journal_kicks_apply_and_sync() {
    assert_eq!(
        lanes_for_table("journal"),
        LaneSet::EMPTY.with(Lane::Apply).with(Lane::Sync)
    );
    assert!(!KNOWN_UNMAPPED.contains(&"journal"));
    assert!(mapped_tables().any(|table| table == "journal"));
    assert_eq!(Lane::Apply.name(), "apply");
    assert_eq!(Lane::Sync.name(), "sync");
}

#[test]
fn sync_logs_kicks_sync() {
    assert_eq!(
        lanes_for_table("sync_logs"),
        LaneSet::EMPTY.with(Lane::Sync)
    );
    assert!(!KNOWN_UNMAPPED.contains(&"sync_logs"));
    assert!(mapped_tables().any(|table| table == "sync_logs"));
}

#[test]
fn sync_dial_targets_kick_sync_and_node_contact_kicks_nothing() {
    assert_eq!(
        lanes_for_table("sync_dial_targets"),
        LaneSet::EMPTY.with(Lane::Sync)
    );
    assert!(!KNOWN_UNMAPPED.contains(&"sync_dial_targets"));
    assert!(mapped_tables().any(|table| table == "sync_dial_targets"));
    assert_eq!(lanes_for_table("sync_node_contact"), LaneSet::EMPTY);
    assert!(KNOWN_UNMAPPED.contains(&"sync_node_contact"));
    assert!(!mapped_tables().any(|table| table == "sync_node_contact"));
}

#[test]
fn sync_and_apply_origin_keep_sync() {
    let sealed = LaneSet::EMPTY.with(Lane::Apply).with(Lane::Sync);
    assert_eq!(
        kicks_for_commit(Some(Lane::Apply), sealed),
        LaneSet::EMPTY.with(Lane::Sync)
    );
    assert_eq!(kicks_for_commit(Some(Lane::Sync), sealed), sealed);
    assert_eq!(
        kicks_for_commit(Some(Lane::Retention), sealed),
        LaneSet::EMPTY
    );
}

#[test]
fn apply_origin_drops_apply() {
    let sealed = LaneSet::EMPTY.with(Lane::Apply).with(Lane::Wakes);
    assert_eq!(
        kicks_for_commit(Some(Lane::Apply), sealed),
        LaneSet::EMPTY.with(Lane::Wakes)
    );
    assert_eq!(kicks_for_commit(None, sealed), sealed);
}

#[test]
fn lane_set_holds_eight_lanes_without_overflow() {
    assert_eq!(Lane::COUNT, 8);
    assert_eq!(Lane::ALL.len(), Lane::COUNT);
    // Every defined lane survives a bits round trip, including bits 6 and 7.
    let all = Lane::ALL
        .into_iter()
        .fold(LaneSet::EMPTY, |set, lane| set.with(lane));
    assert_eq!(all.to_bits(), 0b1111_1111);
    assert_eq!(LaneSet::from_bits(all.to_bits()), all);
    // Undefined high bits are masked off, never carried.
    assert_eq!(LaneSet::from_bits(0xFFFF).to_bits(), 0b1111_1111);
    assert_eq!(LaneSet::from_bits(1 << 8).to_bits(), 0);
    assert_eq!(
        LaneSet::from_bits(1 << 7).iter().collect::<Vec<_>>(),
        [Lane::Sync]
    );
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

/// Lazy progress must be classified without waking any daemon lane. Removing
/// its classification trips the same assertion used by the SQLite update hook.
#[test]
fn lazy_recipient_publication_and_completion_kick_no_lane() {
    assert_eq!(lanes_for_table("lazy_recipients"), LaneSet::EMPTY);
    // A meaningful ordinary control still activates both attention consumers.
    assert!(lanes_for_table("receipts").contains(Lane::Deadlines));
    assert!(lanes_for_table("wake_work").contains(Lane::Wakes));
}

#[test]
fn invitation_presentation_bookkeeping_is_passive_but_context_producers_refresh_mod() {
    for table in [
        "invitation_presentation_consumers",
        "invitation_presentation_items",
    ] {
        assert_eq!(lanes_for_table(table), LaneSet::EMPTY);
        assert!(
            !notifies_mod(table),
            "presentation must not reschedule itself"
        );
    }
    assert_eq!(lanes_for_table("invitation_context_latest"), LaneSet::EMPTY);
    assert!(notifies_mod("invitation_context_latest"));
    assert!(notifies_mod("invitations"));
    assert!(lanes_for_table("invitations").contains(Lane::Deadlines));
}
