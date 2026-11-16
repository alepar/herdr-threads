use herdr_threads::{
    harness::adapter::TurnEdge,
    notification::{
        queue::{QueueDispatch, QueueWakePort, QueueWakeRuntime},
        queue_recovery::QueueRecovery,
    },
    ports::*,
    protocol::{
        ids::TerminalId,
        time::{Clock, MonoInstant, UtcMillis},
    },
    service::{
        fair_writer::FairWriter,
        turn_state::{TurnKey, TurnStateBook},
    },
    test_support::scratch,
};
use std::sync::{
    Arc,
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
        let runtime = match enabled {
            Some(enabled) => runtime.with_programmatic_wake(enabled),
            None => runtime,
        };
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
            .attempt(&r, &target, true, &|| Ok(()), &|| Ok(true), &context)
            .unwrap();
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

#[path = "native_fallback_runtime.rs"]
mod native_fallback_runtime;
