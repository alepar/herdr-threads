use super::*;
use crate::{store::StoreSettings, test_support::isolation::TestIsolation};

fn fixture() -> (TestIsolation, Arc<SqliteStore>, LaneProbe) {
    let iso = TestIsolation::new("lane-commit-observer");
    let context = StoreContext::new(iso.path("store.db"), Arc::new(SystemClock::new()));
    context.open_writer().unwrap().execute_batch(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES('i',0,'b',1,1);
         INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('s','i','unresolved','native',1,0);
         INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) VALUES('old','send_attention','old',0,'complete',0);"
    ).unwrap();
    let store = Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let probe = LaneProbe::default();
    probe.attach_store(&store);
    (iso, store, probe)
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(3000),
        cancellation: Cancellation::default(),
    }
}

#[test]
fn legacy_commit_watchers_preserve_other_origins() {
    let (_iso, store, probe) = fixture();
    let wake = probe.next_commit_instant(Lane::Wakes);
    let deadline = probe.next_commit_instant(Lane::Deadlines);
    let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
    assert_eq!(store.prune_retention(&budget()).unwrap().jobs, 1);
    assert!(
        wake.lock().unwrap().is_some(),
        "a second origin must not overwrite the wake observer"
    );
    assert!(deadline.lock().unwrap().is_none());
}

fn seed_job(iso: &TestIsolation, name: &str) {
    rusqlite::Connection::open(iso.path("store.db")).unwrap().execute(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water,status,completed_at) VALUES(?1,'send_attention',?1,0,'complete',0)",
        [name],
    ).unwrap();
}

#[test]
fn observers_are_exclusive_per_origin_ignore_noops_and_clean_up() {
    let (iso, store, probe) = fixture();
    let samples = Arc::new(Mutex::new(Vec::new()));
    let recorded = samples.clone();
    let path = iso.path("store.db");
    let guard = probe
        .observe_lane_commits(
            Lane::Wakes,
            Box::new(move |at| {
                let db = rusqlite::Connection::open_with_flags(
                    &path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                db.busy_timeout(Duration::from_millis(50)).unwrap();
                assert_eq!(
                    db.query_row("SELECT count(*) FROM work_jobs", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    0
                );
                recorded.lock().unwrap().push(at);
            }),
        )
        .unwrap();
    assert_eq!(
        probe
            .observe_lane_commits(Lane::Wakes, Box::new(|_| panic!("conflicting callback")))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || probe.next_commit_instant(Lane::Wakes)
        ))
        .is_err()
    );
    {
        let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
        assert!(
            store
                .clear_wake_batch_if_empty(&crate::protocol::ids::SeatId::new("s"), &budget())
                .unwrap()
        );
    }
    assert!(
        samples.lock().unwrap().is_empty(),
        "no-op writer turn is not a changed commit"
    );
    {
        let _origin = crate::service::kicks::enter_lane(Lane::Deadlines);
        assert_eq!(store.prune_retention(&budget()).unwrap().jobs, 1);
    }
    assert!(
        samples.lock().unwrap().is_empty(),
        "other origin must not call wake observer"
    );
    for name in ["second", "third"] {
        seed_job(&iso, name);
        let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
        assert_eq!(store.prune_retention(&budget()).unwrap().jobs, 1);
    }
    let before = samples.lock().unwrap().clone();
    assert_eq!(
        before.len(),
        2,
        "conflicts must preserve the original observer"
    );
    assert!(
        before[1] > before[0],
        "one origin's producer instants are monotonic"
    );
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(
        *samples.lock().unwrap(),
        before,
        "consumer delay cannot change producer instants"
    );
    drop(guard);
    let replacement = probe
        .observe_lane_commits(Lane::Wakes, Box::new(|_| {}))
        .unwrap();
    seed_job(&iso, "after-cleanup");
    {
        let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
        assert_eq!(store.prune_retention(&budget()).unwrap().jobs, 1);
    }
    assert_eq!(
        *samples.lock().unwrap(),
        before,
        "dropped observer cannot run after cleanup"
    );
    drop(replacement);
    let weak_store = Arc::downgrade(&store);
    let weak_probe = Arc::downgrade(&probe.inner);
    drop(probe);
    drop(store);
    assert!(weak_probe.upgrade().is_none());
    assert!(
        weak_store.upgrade().is_none(),
        "dispatcher must not retain store/probe cycles"
    );
}

#[test]
fn legacy_owners_release_after_capture_or_abandonment() {
    let (_iso, store, probe) = fixture();
    let abandoned = probe.next_commit_instant(Lane::Wakes);
    drop(abandoned);
    drop(
        probe
            .observe_lane_commits(Lane::Wakes, Box::new(|_| {}))
            .unwrap(),
    );
    let captured = probe.next_commit_instant(Lane::Wakes);
    {
        let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
        store.prune_retention(&budget()).unwrap();
    }
    assert!(captured.lock().unwrap().is_some());
    drop(
        probe
            .observe_lane_commits(Lane::Wakes, Box::new(|_| {}))
            .unwrap(),
    );
}

#[test]
fn observer_drop_waits_for_inflight_callback_and_disables_stale_snapshot() {
    let (_iso, store, probe) = fixture();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release_rx = Mutex::new(release_rx);
    let calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let recorded = calls.clone();
    let guard = probe
        .observe_lane_commits(
            Lane::Wakes,
            Box::new(move |_| {
                recorded.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap();
            }),
        )
        .unwrap();
    let stale_entry = Arc::clone(&guard.entry);
    let producer = std::thread::spawn(move || {
        let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
        store.prune_retention(&budget()).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let (dropping_tx, dropping_rx) = std::sync::mpsc::sync_channel(1);
    let (dropped_tx, dropped_rx) = std::sync::mpsc::sync_channel(1);
    let cleanup = std::thread::spawn(move || {
        dropping_tx.send(()).unwrap();
        drop(guard);
        dropped_tx.send(()).unwrap();
    });
    dropping_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        matches!(
            dropped_rx.recv_timeout(Duration::from_millis(20)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "Drop must wait for an executing callback"
    );
    release_tx.send(()).unwrap();
    producer.join().unwrap();
    dropped_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    cleanup.join().unwrap();
    assert!(
        !stale_entry.execution.lock().unwrap().active,
        "a previously snapshotted entry must be inactive after Drop returns"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    drop(
        probe
            .observe_lane_commits(Lane::Wakes, Box::new(|_| {}))
            .unwrap(),
    );
}

#[test]
fn reattaching_probe_cannot_observe_old_stores_commits() {
    let (_old_iso, old_store, probe) = fixture();
    drop(
        probe
            .observe_lane_commits(Lane::Wakes, Box::new(|_| {}))
            .unwrap(),
    );
    let (_new_iso, new_store, _new_probe) = fixture();
    probe.attach_store(&new_store);
    let calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observed = calls.clone();
    let _guard = probe
        .observe_lane_commits(
            Lane::Wakes,
            Box::new(move |_| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .unwrap();
    let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
    assert_eq!(old_store.prune_retention(&budget()).unwrap().jobs, 1);
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "an old store dispatcher must not invoke a new store's observer"
    );
    assert_eq!(new_store.prune_retention(&budget()).unwrap().jobs, 1);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn refused_reattachment_preserves_both_stores_observers_and_kick_sinks() {
    let (_old_iso, old_store, old_probe) = fixture();
    let (_new_iso, new_store, new_probe) = fixture();
    let old_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let new_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observed = old_calls.clone();
    let _old_guard = old_probe
        .observe_lane_commits(
            Lane::Wakes,
            Box::new(move |_| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .unwrap();
    let observed = new_calls.clone();
    let _new_guard = new_probe
        .observe_lane_commits(
            Lane::Wakes,
            Box::new(move |_| {
                observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        )
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || old_probe.attach_store(&new_store)
        ))
        .is_err()
    );
    let _origin = crate::service::kicks::enter_lane(Lane::Wakes);
    old_store.prune_retention(&budget()).unwrap();
    new_store.prune_retention(&budget()).unwrap();
    assert_eq!(old_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(new_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(old_probe.kick_log().len(), 1);
    assert_eq!(
        new_probe.kick_log().len(),
        1,
        "refused attachment must not mutate the new store's sink"
    );
}

#[test]
fn observer_registration_holds_attachment_ownership_until_inserted() {
    let (_iso, _store, probe) = fixture();
    let registry = Arc::clone(&probe.state().commit_observers);
    let held = registry.lock().unwrap();
    let baseline = Arc::strong_count(&registry);
    let registering_probe = probe.clone();
    let registration = std::thread::spawn(move || {
        registering_probe
            .observe_lane_commits(Lane::Wakes, Box::new(|_| {}))
            .unwrap()
    });
    let until = Instant::now() + Duration::from_secs(3);
    while Arc::strong_count(&registry) == baseline && Instant::now() < until {
        std::thread::yield_now();
    }
    let captured = Arc::strong_count(&registry) > baseline;
    // Give the old implementation a bounded opportunity to expose its
    // unlocked state while registry insertion is deliberately blocked.
    let mut attachment_unlocked = false;
    let until = Instant::now() + Duration::from_millis(100);
    while captured && Instant::now() < until {
        if probe.inner.try_lock().is_ok() {
            attachment_unlocked = true;
            break;
        }
        std::thread::yield_now();
    }
    drop(held);
    let guard = registration.join().unwrap();
    drop(guard);
    assert!(
        captured,
        "registration must capture the held registry within 3 s"
    );
    assert!(
        !attachment_unlocked,
        "reattachment must remain excluded until observer insertion completes"
    );
}
