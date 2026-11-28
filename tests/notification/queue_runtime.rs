use herdr_threads::{
    harness::adapter::TurnEdge,
    notification::{
        queue::{GuardFor, QueueDispatch, QueueWakePort, QueueWakeRuntime},
        queue_recovery::QueueRecovery,
    },
    ports::*,
    protocol::{
        ids::{HostBootId, HostTargetId, SeatId, TerminalId},
        results::{ApiError, ErrorCode},
        time::{Clock, MonoInstant, UtcMillis},
    },
    service::{
        fair_writer::FairWriter,
        turn_state::{TurnKey, TurnStateBook},
    },
    test_support::scratch,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1000)
    }
}
struct MissingProcess {
    calls: AtomicUsize,
    boot: String,
}
impl ProcessInfo for MissingProcess {
    fn process(&self, _pid: u32) -> Option<ProcessRecord> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        None
    }
    fn kernel_boot(&self) -> Option<String> {
        Some(self.boot.clone())
    }
}
#[test]
fn queue_runtime_checks_fences_before_capture_and_never_helpers_a_known_busy_binding() {
    for (enabled, blocked, busy, corrupt_eligibility) in [
        (Some(true), true, false, false),
        (Some(true), false, true, false),
        (Some(true), false, false, false),
        (Some(false), false, false, false),
        (None, false, false, false),
        (Some(false), true, false, false),
        (None, false, false, true),
        (None, true, false, true),
        (Some(false), false, false, true),
    ] {
        let (store, _path, r, p) = super::wake_intents::fixture();
        let budget = super::wake_intents::budget();
        for edge in [TurnEdge::Busy, TurnEdge::Idle] {
            store
                .record_native_turn(
                    &super::wake_intents::native_report(&store, &r, &p, "qualified", edge),
                    &budget,
                )
                .unwrap();
        }
        super::wake_intents::set_programmatic_wake(&store, r.seat.as_str(), enabled == Some(true));
        if corrupt_eligibility {
            store
                .context()
                .open_writer()
                .unwrap()
                .execute("UPDATE native_turn_state SET scope_json=x'00'", [])
                .unwrap();
        }
        if blocked {
            super::wake_intents::prepare_raw(&store, &r, &p);
        }
        let root = scratch::temp_path(format!("ht-queue-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(store);
        let clock = Arc::new(TestClock);
        let writer = Arc::new(FairWriter::new(32));
        let processes = Arc::new(MissingProcess {
            calls: AtomicUsize::new(0),
            boot: p.target.scope.kernel_boot.clone(),
        });
        let book = Arc::new(TurnStateBook::default());
        let key = TurnKey {
            seat: r.seat.clone(),
            generation: p.target.scope.generation,
            harness: p.adapter.as_str().into(),
            native_session: p.target.scope.native_session.clone(),
        };
        book.record(
            &key,
            if busy { TurnEdge::Busy } else { TurnEdge::Idle },
            1000,
        );
        let recovery = Arc::new(QueueRecovery::new(
            store.clone(),
            writer.clone(),
            clock.clone(),
            processes.clone(),
            root.clone(),
            r.daemon_boot,
        ));
        let runtime = QueueWakeRuntime::new(
            store.clone(),
            writer,
            clock,
            processes.clone(),
            root.clone(),
            book,
            recovery,
        );
        let target = SafeWakeTarget {
            seat: r.seat.clone(),
            target: r.target.clone(),
            host_boot: r.host_boot.clone(),
            generation: r.target_generation,
            terminal: TerminalId::new("term-pane"),
            incarnation: "inc".into(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: r.host_epoch,
            observation_sequence: 1,
            bound_harness: Some(p.adapter.as_str().into()),
        };
        let context = HostCallContext {
            budget,
            expected_boot: Some(r.host_boot.clone()),
            expected_epoch: Some(r.host_epoch),
        };
        let modes = Mutex::new(Vec::new());
        let guard = |next| {
            modes.lock().unwrap().push(next);
            Ok(())
        };
        let result = runtime
            .attempt(&r, &target, true, &guard, &|| Ok(true), &context)
            .unwrap();
        // A native-blocked attempt asks the native-fallback guard (its claim is refused); the
        // queue's own admission runs only at arm, after capture and prepare, so none of these
        // rows (capture fails on the missing process) asks it. A known Busy refuses before both.
        let expected = if blocked {
            vec![GuardFor::NativeFallback]
        } else {
            vec![]
        };
        assert_eq!(*modes.lock().unwrap(), expected);
        assert_eq!(
            result,
            if blocked || (enabled == Some(true) && busy) {
                QueueDispatch::Refused
            } else {
                QueueDispatch::Native
            }
        );
        assert_eq!(
            processes.calls.load(Ordering::SeqCst),
            usize::from(enabled == Some(true) && !blocked && !busy),
            "fences and known Busy must precede physical capture"
        );
        assert_eq!(
            store
                .context()
                .open_writer()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM wake_intents WHERE helper_json IS NOT NULL",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// S2 (ht-z41e.6, busy-sibling rule: drop): the queue transport does not serialise a root turn
/// behind a busy `/side` turn, so only the root's own turn state gates the attempt.
#[test]
fn a_busy_side_binding_does_not_refuse_the_root_wake_and_a_busy_root_does() {
    const SIDE: &str = "00000000-0000-0000-0000-0000000000a1";
    for (root_busy, side_busy) in [(false, true), (true, false)] {
        // The side is enrolled by its own hook event (null `transcript_path`), never by a flag;
        // before the wake is reserved, because a check-in advances the decision sequence.
        let (store, _path, r, p) = super::wake_intents::fixture_with(|store| {
            super::wake_intents::enroll_payload(
                store,
                include_bytes!("../fixtures/codex-0.163.0-side/pre-tool-use-side.json"),
                SIDE,
                10,
                "side-first-event",
            )
            .unwrap();
        });
        let budget = super::wake_intents::budget();
        for edge in [TurnEdge::Busy, TurnEdge::Idle] {
            store
                .record_native_turn(
                    &super::wake_intents::native_report(&store, &r, &p, "qualified", edge),
                    &budget,
                )
                .unwrap();
        }
        let root = scratch::temp_path(format!("ht-queue-runtime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = Arc::new(store);
        let clock = Arc::new(TestClock);
        let writer = Arc::new(FairWriter::new(32));
        let processes = Arc::new(MissingProcess {
            calls: AtomicUsize::new(0),
            boot: p.target.scope.kernel_boot.clone(),
        });
        let book = Arc::new(TurnStateBook::default());
        let key = |session: &str| TurnKey {
            seat: r.seat.clone(),
            generation: p.target.scope.generation,
            harness: p.adapter.as_str().into(),
            native_session: herdr_threads::protocol::ids::NativeSessionId::new(session),
        };
        let edge = |busy| if busy { TurnEdge::Busy } else { TurnEdge::Idle };
        book.record(
            &key(p.target.scope.native_session.as_str()),
            edge(root_busy),
            1000,
        );
        book.record(&key(SIDE), edge(side_busy), 1000);
        let recovery = Arc::new(QueueRecovery::new(
            store.clone(),
            writer.clone(),
            clock.clone(),
            processes.clone(),
            root.clone(),
            r.daemon_boot,
        ));
        let runtime = QueueWakeRuntime::new(
            store.clone(),
            writer,
            clock,
            processes.clone(),
            root.clone(),
            book,
            recovery,
        );
        let target = SafeWakeTarget {
            seat: r.seat.clone(),
            target: r.target.clone(),
            host_boot: r.host_boot.clone(),
            generation: r.target_generation,
            terminal: TerminalId::new("term-pane"),
            incarnation: "inc".into(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: r.host_epoch,
            observation_sequence: 1,
            bound_harness: Some(p.adapter.as_str().into()),
        };
        let context = HostCallContext {
            budget,
            expected_boot: Some(r.host_boot.clone()),
            expected_epoch: Some(r.host_epoch),
        };
        let result = runtime
            .attempt(&r, &target, true, &|_| Ok(()), &|| Ok(true), &context)
            .unwrap();
        // A busy root is refused before capture; with only the side busy the attempt reaches
        // physical capture (the missing process then falls back to native).
        assert_eq!(
            result,
            if root_busy {
                QueueDispatch::Refused
            } else {
                QueueDispatch::Native
            }
        );
        assert_eq!(
            processes.calls.load(Ordering::SeqCst),
            usize::from(!root_busy)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// A runtime over the fixture's store whose wake control is `arming`, root hook `Idle` in `book`.
struct Armed {
    runtime: QueueWakeRuntime,
    store: Arc<herdr_threads::store::SqliteStore>,
    target: SafeWakeTarget,
    context: HostCallContext,
    root: std::path::PathBuf,
}
fn armed(
    store: herdr_threads::store::SqliteStore,
    r: &WakeReservation,
    p: &WakeIntentPreparation,
    book: Arc<TurnStateBook>,
    arming: Arc<super::wake_intents::ArmingControl>,
) -> Armed {
    let budget = super::wake_intents::budget();
    for edge in [TurnEdge::Busy, TurnEdge::Idle] {
        store
            .record_native_turn(
                &super::wake_intents::native_report(&store, r, p, "qualified", edge),
                &budget,
            )
            .unwrap();
    }
    let root = scratch::temp_path(format!("ht-queue-runtime-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = Arc::new(store);
    let clock = Arc::new(TestClock);
    let writer = Arc::new(FairWriter::new(32));
    let processes = Arc::new(MissingProcess {
        calls: AtomicUsize::new(0),
        boot: p.target.scope.kernel_boot.clone(),
    });
    let recovery = Arc::new(QueueRecovery::new(
        store.clone(),
        writer.clone(),
        clock.clone(),
        processes.clone(),
        root.clone(),
        r.daemon_boot,
    ));
    let runtime = QueueWakeRuntime::new(
        store.clone(),
        writer,
        clock,
        processes,
        root.clone(),
        book,
        recovery,
    )
    .with_wake_control(arming);
    let target = SafeWakeTarget {
        seat: r.seat.clone(),
        target: r.target.clone(),
        host_boot: r.host_boot.clone(),
        generation: r.target_generation,
        terminal: TerminalId::new("term-pane"),
        incarnation: "inc".into(),
        basis: WakeTargetBasis::CooperativeAgent,
        epoch: r.host_epoch,
        observation_sequence: 1,
        bound_harness: Some(p.adapter.as_str().into()),
    };
    let context = HostCallContext {
        budget,
        expected_boot: Some(r.host_boot.clone()),
        expected_epoch: Some(r.host_epoch),
    };
    Armed {
        runtime,
        store,
        target,
        context,
        root,
    }
}
fn root_key(r: &WakeReservation, p: &WakeIntentPreparation) -> TurnKey {
    TurnKey {
        seat: r.seat.clone(),
        generation: p.target.scope.generation,
        harness: p.adapter.as_str().into(),
        native_session: p.target.scope.native_session.clone(),
    }
}
fn intent_phase(a: &Armed, r: &WakeReservation) -> herdr_threads::ports::WakeIntentPhase {
    a.store
        .wake_intent(&r.attempt, &a.context.budget)
        .unwrap()
        .unwrap()
        .phase
}

/// Catches the host admission running more than once per attempt, before capture, or with the
/// wrong sibling fact: the queue asks it exactly once, inside `authorize`, with `siblings` true
/// only when the root's process has other bindings, and arms on its answer.
#[test]
fn queue_admission_runs_once_at_arm_with_the_sibling_fact() {
    const SIDE: &str = "00000000-0000-0000-0000-0000000000a1";
    for siblings in [false, true] {
        let (store, _path, r, p) = super::wake_intents::fixture_with(|store| {
            if siblings {
                super::wake_intents::enroll_payload(
                    store,
                    include_bytes!("../fixtures/codex-0.163.0-side/pre-tool-use-side.json"),
                    SIDE,
                    10,
                    "side-first-event",
                )
                .unwrap();
            }
        });
        let book = Arc::new(TurnStateBook::default());
        let key = root_key(&r, &p);
        book.record(&key, TurnEdge::Idle, 1000);
        if siblings {
            let side = TurnKey {
                native_session: herdr_threads::protocol::ids::NativeSessionId::new(SIDE),
                ..key.clone()
            };
            book.record(&side, TurnEdge::Busy, 1000);
        }
        let arming = Arc::new(super::wake_intents::ArmingControl::default());
        let a = armed(store, &r, &p, book, arming.clone());
        let modes = Mutex::new(Vec::new());
        let guard = |next| {
            modes.lock().unwrap().push(next);
            Ok(())
        };
        let result = a
            .runtime
            .attempt(&r, &a.target, true, &guard, &|| Ok(true), &a.context)
            .unwrap();
        assert_eq!(result, QueueDispatch::Submitted, "siblings={siblings}");
        assert_eq!(*modes.lock().unwrap(), [GuardFor::Queue { siblings }]);
        assert_eq!(arming.calls.load(Ordering::SeqCst), 1);
        assert_eq!(arming.authorized.load(Ordering::SeqCst), 1);
        assert_eq!(arming.effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            a.runtime.take_root(&r.attempt),
            Some(p.target.scope.native_session.clone()),
            "the receipt awaits the root's session"
        );
        std::fs::remove_dir_all(&a.root).unwrap();
    }
}

/// Catches an admission refusal at arm adding anyway, or a host without the admission arming: a
/// refusal (or the default every host without an implementation answers) settles the same attempt
/// as a native wake with the intent never armed and no root to await.
#[test]
fn queue_admission_refusal_at_arm_falls_back_native_and_never_adds() {
    let default_refusal = HostPort::admit_queue_wake(
        &herdr_threads::test_support::fake_hosts::RefusingHost,
        &SafeWakeTarget {
            seat: SeatId::new("s"),
            target: HostTargetId::new("pane"),
            host_boot: HostBootId::new("host"),
            generation: 1,
            terminal: TerminalId::new("term-pane"),
            incarnation: "inc".into(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: 1,
            observation_sequence: 1,
            bound_harness: Some("codex".into()),
        },
        true,
        &HostCallContext {
            budget: super::wake_intents::budget(),
            expected_boot: None,
            expected_epoch: None,
        },
    )
    .unwrap_err();
    assert_eq!(default_refusal.code, ErrorCode::TargetUnsafe);
    for refusal in [
        ApiError::new(
            ErrorCode::TargetUnsafe,
            "queue wake refused before arm: composer has typed text",
        ),
        default_refusal,
    ] {
        let (store, _path, r, p) = super::wake_intents::fixture();
        let book = Arc::new(TurnStateBook::default());
        book.record(&root_key(&r, &p), TurnEdge::Idle, 1000);
        let arming = Arc::new(super::wake_intents::ArmingControl::default());
        let a = armed(store, &r, &p, book, arming.clone());
        let result = a
            .runtime
            .attempt(
                &r,
                &a.target,
                true,
                &|next| match next {
                    GuardFor::Queue { .. } => Err(refusal.clone()),
                    GuardFor::NativeFallback => Ok(()),
                },
                &|| Ok(true),
                &a.context,
            )
            .unwrap();
        assert_eq!(result, QueueDispatch::Native, "{refusal:?}");
        assert_eq!(arming.authorized.load(Ordering::SeqCst), 1);
        assert_eq!(arming.effects.load(Ordering::SeqCst), 0);
        assert_eq!(
            intent_phase(&a, &r),
            herdr_threads::ports::WakeIntentPhase::NoEffect
        );
        assert_eq!(a.runtime.take_root(&r.attempt), None);
        std::fs::remove_dir_all(&a.root).unwrap();
    }
}

/// Catches a root that turns Busy between selection and arm (rule 4) being typed into by a native
/// fallback in the same attempt: the attempt is refused for the ladder, before any host call.
#[test]
fn root_busy_at_arm_is_refused_through_attempt() {
    let (store, _path, r, p) = super::wake_intents::fixture();
    let book = Arc::new(TurnStateBook::default());
    let key = root_key(&r, &p);
    book.record(&key, TurnEdge::Idle, 1000);
    let arming = Arc::new(super::wake_intents::ArmingControl {
        before_authorize: Some(Box::new({
            let book = book.clone();
            let key = key.clone();
            move || {
                book.record(&key, TurnEdge::Busy, 1001);
            }
        })),
        ..Default::default()
    });
    let a = armed(store, &r, &p, book, arming.clone());
    let modes = Mutex::new(Vec::new());
    let guard = |next| {
        modes.lock().unwrap().push(next);
        Ok(())
    };
    let result = a
        .runtime
        .attempt(&r, &a.target, true, &guard, &|| Ok(true), &a.context)
        .unwrap();
    assert_eq!(result, QueueDispatch::Refused);
    assert!(modes.lock().unwrap().is_empty(), "no host admission");
    assert_eq!(arming.authorized.load(Ordering::SeqCst), 1);
    assert_eq!(arming.effects.load(Ordering::SeqCst), 0);
    assert_eq!(
        intent_phase(&a, &r),
        herdr_threads::ports::WakeIntentPhase::NoEffect
    );
    assert_eq!(a.runtime.take_root(&r.attempt), None);
    std::fs::remove_dir_all(&a.root).unwrap();
}

#[path = "native_fallback_runtime.rs"]
mod native_fallback_runtime;
