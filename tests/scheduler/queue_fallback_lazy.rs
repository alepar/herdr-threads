//! Exercise the retained lazy frontier through real discovery and reservation.
use super::*;
use crate::harness::{
    adapter::TurnEdge,
    wake::{WakeHelperPurpose, WakeHelperWitness},
};
use crate::{
    ports::*,
    protocol::pagination::PageRequest,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
mod fixture {
    use crate as herdr_threads;
    include!("../fixtures/wake_intent.rs");
}
#[derive(Default)]
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100 + self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1000 + self.0.load(Ordering::SeqCst))
    }
}
fn budget(clock: &TestClock) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10000),
        cancellation: Default::default(),
    }
}
fn candidates(store: &SqliteStore, clock: &TestClock) -> Vec<WakeCandidate> {
    store
        .wake_candidates(
            crate::test_support::test_host_id(),
            PageRequest::default(),
            &budget(clock),
        )
        .unwrap()
        .items
}
fn reserve(store: &SqliteStore, clock: &TestClock) -> WakeReservation {
    let candidate = candidates(store, clock).remove(0);
    store
        .reserve_wake(
            crate::test_support::test_host_id(),
            &candidate,
            &budget(clock),
        )
        .unwrap()
        .expect("canonical reservation")
}
fn complete(store: &SqliteStore, clock: &TestClock, r: &WakeReservation) {
    store
        .complete_wake(
            crate::test_support::test_host_id(),
            r.attempt.clone(),
            WakeOutcome::Submitted,
            None,
            &budget(clock),
        )
        .unwrap();
}
fn setup(priority: i64) -> (SqliteStore, Arc<TestClock>, WakeReservation) {
    let (old, path, inv, p) = fixture::fixture();
    let clock = Arc::new(TestClock::default());
    let store = SqliteStore::new(
        StoreContext::new(path, clock.clone()).with_test_host(),
        fixture::INSTANCE,
        StoreSettings {
            daemon_boot: Some(inv.daemon_boot),
            ..Default::default()
        },
    )
    .unwrap();
    drop(old);
    complete(&store, &clock, &inv);
    store
        .context()
        .open_writer()
        .unwrap()
        .execute("UPDATE seats SET nudge_threshold=?1", [priority])
        .unwrap();
    let sql="UPDATE invitations SET state='already_joined';
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES('prep-1','34750f98bd59fcfc946da45aaabe933b','scope','prep-1',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',PRIORITY);
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-1','msg-1','t','s',0);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('msg-1','34750f98bd59fcfc946da45aaabe933b','t',1,'ordinary',2,'passive',0,'lazy',PRIORITY);
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-1','msg-1','34750f98bd59fcfc946da45aaabe933b','t',2,0,2,0,1,0);
        INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES('binding','t',0);
        UPDATE lazy_recipients SET decision_seq=2 WHERE preparation_id='prep-1'; UPDATE local_node SET decision_seq=2;";
    let sql = sql
        .replace("PRIORITY", &priority.to_string())
        .replace("'t','s',0)", &format!("'t','s',{priority})"));
    store
        .context()
        .open_writer()
        .unwrap()
        .execute_batch(&sql)
        .unwrap();
    let r = reserve(&store, &clock);
    let PrepareWakeIntent::Prepared(row) =
        store.prepare_wake_intent(&r, &p, &budget(&clock)).unwrap()
    else {
        panic!("prepare")
    };
    for edge in [TurnEdge::Busy, TurnEdge::Idle] {
        store
            .record_native_turn(
                &fixture::native_report(&store, &r, &p, "baseline", edge),
                &budget(&clock),
            )
            .unwrap();
    }
    let op = WakeHelperOperation {
        id: uuid::Uuid::new_v4(),
        purpose: WakeHelperPurpose::Add,
        subject: None,
        owner_boot: r.daemon_boot,
        kernel_boot: p.target.scope.kernel_boot.clone(),
        witness: None,
    };
    assert!(
        store
            .begin_wake_helper(&r.attempt, row.revision, &op, &budget(&clock))
            .unwrap()
    );
    assert!(
        store
            .record_wake_helper_started(
                &r.attempt,
                op.id,
                &WakeHelperWitness {
                    process: p.target.scope.process,
                    kernel_boot: p.target.scope.kernel_boot.clone(),
                    pgid: p.target.scope.process.pid
                },
                &budget(&clock)
            )
            .unwrap()
    );
    let row = store
        .wake_intent(&r.attempt, &budget(&clock))
        .unwrap()
        .unwrap();
    let arm = store
        .arm_wake_intent(&r, row.revision, uuid::Uuid::new_v4(), &budget(&clock))
        .unwrap()
        .unwrap();
    assert!(
        store
            .record_wake_intent_result(
                &arm,
                &WakeIntentResult {
                    outcome: WakeIntentOutcome::Accepted { id: "owned".into() },
                    cleanup_confirmed: true,
                    diagnostic: String::new()
                },
                &budget(&clock)
            )
            .unwrap()
    );
    complete(&store, &clock, &r);
    (store, clock, r)
}
#[test]
fn unread_reserved_lazy_frontier_becomes_fallback_candidate_without_new_ack_or_message() {
    let (store, clock, original) = setup(0);
    assert!(candidates(&store, &clock).is_empty());
    clock.0.store(29999, Ordering::SeqCst);
    assert!(candidates(&store, &clock).is_empty());
    clock.0.store(30000, Ordering::SeqCst);
    let due = candidates(&store, &clock);
    assert_eq!(
        due.len(),
        1,
        "retained p0 without ACK must reach existing wake lane"
    );
    assert!(!due[0].has_unreserved_lazy());
    let r = reserve(&store, &clock);
    assert_eq!(r.reasons, ["lazy"]);
    assert_eq!(
        r.attention_witness.frontier(),
        original.attention_witness.frontier()
    );
    let ticket = uuid::Uuid::new_v4();
    assert!(
        store
            .claim_native_wake_fallback(&r, ticket, &budget(&clock))
            .unwrap()
    );
    assert!(
        store
            .finish_native_wake_fallback(
                &r,
                ticket,
                NativeFallbackOutcome::Submitted,
                &budget(&clock)
            )
            .unwrap()
    );
    complete(&store, &clock, &r);
    clock.0.store(60000, Ordering::SeqCst);
    assert!(
        candidates(&store, &clock).is_empty(),
        "covered frontier stays suppressed"
    );
}

struct StoreFence<'a>(&'a SqliteStore);
impl ReservationCheck for StoreFence<'_> {
    fn is_current(&self, r: &WakeReservation, b: &CallBudget) -> Result<bool, ApiError> {
        self.0.validate_wake_reservation(r, b)
    }
    fn wake_text_evidence_current(
        &self,
        r: &WakeReservation,
        b: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.0.wake_text_evidence_current(r, b)
    }
    fn owes_wake(
        &self,
        r: &WakeReservation,
        reached: &[Option<&BindingId>],
        b: &CallBudget,
    ) -> Result<bool, ApiError> {
        self.0.wake_owed_to_reached(r, reached, b)
    }
}
struct NoProcess;
impl ProcessInfo for NoProcess {
    fn process(&self, _: u32) -> Option<ProcessRecord> {
        panic!("retained-intent fallback must not capture a process or start a helper")
    }
}
fn dispatch(
    store: Arc<SqliteStore>,
    clock: Arc<TestClock>,
    r: &WakeReservation,
    boundary: NativeBoundary,
    run_lane: bool,
) -> usize {
    use crate::{
        notification::{queue::QueueWakeRuntime, queue_recovery::QueueRecovery},
        service::{fair_writer::FairWriter, turn_state::TurnStateBook},
    };
    let root = crate::test_support::scratch::temp_path(format!(
        "ht-fallback-dispatch-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let writer = Arc::new(FairWriter::new(32));
    let processes = Arc::new(NoProcess);
    let recovery = Arc::new(QueueRecovery::new(
        store.clone(),
        writer.clone(),
        clock.clone(),
        processes.clone(),
        root.clone(),
        r.daemon_boot,
    ));
    // Default-off must still recover retained intents and claim guarded fallback.
    let queue = QueueWakeRuntime::new(
        store.clone(),
        writer.clone(),
        clock.clone(),
        processes,
        root.clone(),
        Arc::new(TurnStateBook::default()),
        recovery,
    );
    let mut host = QueueHost::new();
    host.boundary = boundary;
    let obs = &mut host.inner.observation;
    obs.target = r.target.clone();
    obs.host_boot = r.host_boot.clone();
    obs.generation = r.target_generation;
    obs.epoch = r.host_epoch;
    obs.terminal = Some(TerminalId::new("term-pane"));
    obs.incarnation = IncarnationEvidence::Verified {
        identity: "inc".into(),
        evidence_kind: EvidenceKind::NativeCurrentTarget,
    };
    obs.ui = HostUiState::Idle;
    let later = if run_lane { 60_000 } else { 0 };
    obs.observed_at_mono = MonoInstant(clock.monotonic_now().0 + later);
    obs.observed_at_utc = UtcMillis(clock.utc_now().0 + later as i64);
    obs.started_at_mono = obs.observed_at_mono;
    obs.completed_at_mono = obs.observed_at_mono;
    host.inner.foreground = vec![10];
    let fence = StoreFence(&store);
    let dispatcher = NativeWakeDispatcher::new(&host, &fence, clock.as_ref()).with_queue(&queue);
    let ctx = HostCallContext {
        budget: budget(&clock),
        expected_boot: Some(r.host_boot.clone()),
        expected_epoch: Some(r.host_epoch),
    };
    if run_lane {
        let scheduled = crate::service::workers::ScheduledStore::new(store.clone(), writer)
            .for_host(crate::test_support::test_host_id());
        let runner = crate::scheduler::WakeRunner::new(
            &scheduled,
            &dispatcher,
            crate::notification::policy::RetryConfig::default(),
            r.daemon_boot,
        );
        let selected = candidates(&store, &clock).remove(0);
        assert!(
            runner
                .try_candidate(&selected, &budget(&clock))
                .unwrap()
                .is_none(),
            "restored retry spacing must remain in force"
        );
        assert!(host.guards.lock().unwrap().is_empty());
        clock.0.fetch_add(later, Ordering::SeqCst);
        let selected = candidates(&store, &clock).remove(0);
        assert!(
            runner
                .try_candidate(&selected, &budget(&clock))
                .unwrap()
                .is_some()
        );
        assert!(
            candidates(&store, &clock).is_empty(),
            "spent coverage suppresses another lane attempt"
        );
    } else {
        let _ = dispatcher.attempt_wake(r.clone(), &ctx).unwrap();
    }
    assert_eq!(
        dispatcher.take_transport(&r.attempt),
        WakeTransport::Composer
    );
    assert!(
        !host.guards.lock().unwrap().is_empty(),
        "fresh idle guard must run"
    );
    let typed = host.inner.typed.lock().unwrap().len();
    std::fs::remove_dir_all(root).unwrap();
    typed
}
#[test]
fn lazy_fallback_real_dispatcher_noeffect_retry_then_single_native_effect_when_queue_off() {
    let (store, clock, _) = setup(0);
    let store = Arc::new(store);
    clock.0.store(30000, Ordering::SeqCst);
    let no_effect = reserve(&store, &clock);
    assert_eq!(
        dispatch(
            store.clone(),
            clock.clone(),
            &no_effect,
            NativeBoundary::NoEffect,
            false
        ),
        0
    );
    complete(&store, &clock, &no_effect);
    // Keep ordinary retry spacing/backoff intact; a test clock advances instead
    // of deleting wake_work or inventing a new attention publication.
    clock.0.store(90000, Ordering::SeqCst);
    let retry = reserve(&store, &clock);
    assert_eq!(
        dispatch(
            store.clone(),
            clock.clone(),
            &retry,
            NativeBoundary::Normal,
            false
        ),
        1
    );
    complete(&store, &clock, &retry);
    clock.0.store(210000, Ordering::SeqCst);
    assert!(candidates(&store, &clock).is_empty());
    let row = store.wake_intent(&retry.attempt, &budget(&clock)).unwrap();
    assert!(
        row.is_none(),
        "fallback must not create another queue intent"
    );
}
#[test]
fn lazy_fallback_rechecks_read_threshold_and_uncertain_blockers() {
    for change in ["read", "threshold", "armed", "quarantine", "unknown"] {
        let (store, clock, original) = setup(if change == "threshold" { 3 } else { 0 });
        clock.0.store(30000, Ordering::SeqCst);
        let selected = candidates(&store, &clock).remove(0);
        let sql = match change {
            "read" => "UPDATE binding_cursors SET seq=1",
            "threshold" => "UPDATE seats SET nudge_threshold=0",
            "armed" => "UPDATE wake_intents SET phase='armed'",
            "quarantine" => "UPDATE wake_intents SET cleanup_quarantined=1",
            "unknown" => "UPDATE wake_intents SET phase='unknown'",
            _ => unreachable!(),
        };
        store
            .context()
            .open_writer()
            .unwrap()
            .execute(sql, [])
            .unwrap();
        if change == "unknown" {
            assert_eq!(candidates(&store, &clock).len(), 1);
            let r = reserve(&store, &clock);
            assert!(
                store
                    .claim_native_wake_fallback(&r, uuid::Uuid::new_v4(), &budget(&clock))
                    .unwrap()
            );
        } else {
            assert!(candidates(&store, &clock).is_empty(), "{change}");
            assert!(
                store
                    .reserve_wake(
                        crate::test_support::test_host_id(),
                        &selected,
                        &budget(&clock)
                    )
                    .unwrap()
                    .is_none(),
                "stale {change} selection must not reserve"
            );
        }
        assert!(
            store
                .wake_intent(&original.attempt, &budget(&clock))
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn lazy_fallback_existing_runner_reaches_guarded_native_after_retained_spacing() {
    let (store, clock, queued) = setup(0);
    let store = Arc::new(store);
    clock.0.store(30000, Ordering::SeqCst);
    assert_eq!(
        dispatch(store, clock, &queued, NativeBoundary::Normal, true),
        1
    );
}

#[test]
fn lazy_fallback_new_publication_rearms_once_and_failed_noeffect_release_stays_spent() {
    let (store, clock, _) = setup(0);
    clock.0.store(30000, Ordering::SeqCst);
    let first = reserve(&store, &clock);
    let ticket = uuid::Uuid::new_v4();
    assert!(
        store
            .claim_native_wake_fallback(&first, ticket, &budget(&clock))
            .unwrap()
    );
    // A failed metadata budget cannot restore allowance without a committed
    // NoEffect result. The runtime independently supplies a fresh budget.
    let failed_budget = budget(&clock);
    failed_budget.cancellation.cancel();
    assert!(
        store
            .finish_native_wake_fallback(
                &first,
                ticket,
                NativeFallbackOutcome::NoEffect,
                &failed_budget
            )
            .is_err()
    );
    complete(&store, &clock, &first);
    assert!(candidates(&store, &clock).is_empty());
    store.context().open_writer().unwrap().execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-2','34750f98bd59fcfc946da45aaabe933b','scope','prep-2',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy');
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-2','msg-2','t','s',0);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('msg-2','34750f98bd59fcfc946da45aaabe933b','t',2,'ordinary',3,'new',0,'lazy');
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-2','msg-2','34750f98bd59fcfc946da45aaabe933b','t',3,0,3,0,1,0);
        UPDATE lazy_recipients SET decision_seq=3 WHERE preparation_id='prep-2'; UPDATE local_node SET decision_seq=3;").unwrap();
    clock.0.store(90000, Ordering::SeqCst);
    let next = reserve(&store, &clock);
    let ticket = uuid::Uuid::new_v4();
    assert!(
        next.attention_witness
            .frontier()
            .advanced_beyond(&first.attention_witness.frontier())
    );
    assert!(
        store
            .claim_native_wake_fallback(&next, ticket, &budget(&clock))
            .unwrap()
    );
    assert!(
        store
            .finish_native_wake_fallback(
                &next,
                ticket,
                NativeFallbackOutcome::Unknown,
                &budget(&clock)
            )
            .unwrap()
    );
    complete(&store, &clock, &next);
    assert!(candidates(&store, &clock).is_empty());
}
#[path = "queue_side_dispatch.rs"]
mod queue_side_dispatch;
