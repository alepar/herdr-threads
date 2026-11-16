use super::*;
#[test]
fn disabled_queue_retains_fallback_fences_and_completion_uses_fresh_budget() {
    let (store, _path, _original, p) = super::super::wake_native_fallback::accepted();
    let r = super::super::wake_intents::reserve_again(&store);
    let store = Arc::new(store);
    let root = scratch::temp_path(format!(
        "ht-native-fallback-runtime-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
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
    // Constructor default is off; retained possible effects still require a
    // fallback ticket and do not grant a fresh programmatic attempt.
    let runtime = QueueWakeRuntime::new(
        store.clone(),
        writer,
        clock,
        processes.clone(),
        root.clone(),
        Arc::new(TurnStateBook::default()),
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
        budget: super::super::wake_intents::budget(),
        expected_boot: Some(r.host_boot.clone()),
        expected_epoch: Some(r.host_epoch),
    };
    let attempt = |r: &WakeReservation, ctx: &HostCallContext| {
        runtime
            .attempt(r, &target, true, &|| Ok(()), &|| Ok(true), ctx)
            .unwrap()
    };
    let QueueDispatch::NativeFallback(ticket) = attempt(&r, &context) else {
        panic!("missing native fallback authority")
    };
    assert_eq!(
        attempt(&r, &context),
        QueueDispatch::Refused,
        "same ticket/reservation must not submit twice"
    );
    context.budget.cancellation.cancel();
    runtime
        .finish_native_fallback(&r, ticket, NativeFallbackOutcome::NoEffect)
        .unwrap();
    let next = super::super::wake_intents::reserve_again(&store);
    let QueueDispatch::NativeFallback(next_ticket) = attempt(
        &next,
        &HostCallContext {
            budget: super::super::wake_intents::budget(),
            expected_boot: Some(next.host_boot.clone()),
            expected_epoch: Some(next.host_epoch),
        },
    ) else {
        panic!("NoEffect failed to restore allowance")
    };
    runtime
        .finish_native_fallback(&next, next_ticket, NativeFallbackOutcome::Unknown)
        .unwrap();
    let repeated = super::super::wake_intents::reserve_again(&store);
    assert_eq!(
        attempt(
            &repeated,
            &HostCallContext {
                budget: super::super::wake_intents::budget(),
                expected_boot: Some(repeated.host_boot.clone()),
                expected_epoch: Some(repeated.host_epoch)
            }
        ),
        QueueDispatch::Refused
    );
    assert_eq!(
        processes.calls.load(Ordering::SeqCst),
        0,
        "disabled path started/captured helper"
    );
    std::fs::remove_dir_all(root).unwrap();
}
