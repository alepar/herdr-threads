//! Observe owned real proc descriptors without running a harness or model.
use herdr_threads::{
    app::SystemClock,
    harness::{
        registry::builtins,
        wake::{WakeControlContext, WakeScope},
    },
    host::process_table::HostProcessTable,
    ports::ProcessInfo,
    protocol::{
        authority::LivenessFingerprint,
        ids::{BindingId, ExecutionId, NativeSessionId},
        time::{CallBudget, MonoInstant},
    },
    test_support::scratch,
};
use std::{fs, os::fd::AsRawFd};
#[test]
fn current_scope_qualifies_but_boot_pid_and_inode_changes_refuse() {
    let root = scratch::temp_path(format!("ht-wake-observation-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("thread-writer-locks")).unwrap();
    let database = root.join("queue_1.sqlite");
    fs::write(&database, b"fixture").unwrap();
    let _db = fs::File::open(&database).unwrap();
    let _db_again = fs::File::open(&database).unwrap();
    let lock = root.join("thread-writer-locks/00000000-0000-0000-0000-000000000001.lock");
    fs::write(&lock, b"").unwrap();
    let lock = fs::File::open(lock).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let clock = SystemClock::new();
    let processes = HostProcessTable;
    let budget = CallBudget {
        deadline: MonoInstant(2000),
        cancellation: Default::default(),
    };
    let context = WakeControlContext {
        clock: &clock,
        budget: &budget,
        processes: &processes,
        state_dir: &root,
    };
    let scope = WakeScope {
        binding: BindingId::new("binding"),
        native_session: NativeSessionId::new("00000000-0000-0000-0000-000000000001"),
        execution: ExecutionId::new("execution"),
        generation: 1,
        process: LivenessFingerprint::of(&processes.process(std::process::id()).unwrap()),
        kernel_boot: processes.kernel_boot().unwrap(),
    };
    let registry = builtins();
    let control = registry
        .by_id(registry.agent("codex").unwrap())
        .unwrap()
        .wake_control()
        .expect("registered queue control");
    assert!(control.observe(&scope, &context).unwrap().is_some());
    let mut wrong = scope.clone();
    wrong.kernel_boot = "another-boot".into();
    assert!(control.observe(&wrong, &context).unwrap().is_none());
    let mut wrong = scope.clone();
    wrong.process.start += 1;
    assert!(control.observe(&wrong, &context).unwrap().is_none());
    fs::rename(&database, root.join("old-database")).unwrap();
    fs::write(&database, b"replacement").unwrap();
    assert!(control.observe(&scope, &context).unwrap().is_none());
    drop(lock);
    drop(_db_again);
    drop(_db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replay_cohort_survives_reconnect_and_storage_relocation() {
    use herdr_threads::harness::wake::WakeControlTarget;
    let registry = builtins();
    let control = registry
        .by_id(registry.agent("codex").unwrap())
        .unwrap()
        .wake_control()
        .unwrap();
    let mut target = WakeControlTarget {
        scope: WakeScope {
            binding: BindingId::new("old-binding"),
            native_session: NativeSessionId::new("00000000-0000-0000-0000-000000000001"),
            execution: ExecutionId::new("old-execution"),
            generation: 1,
            process: LivenessFingerprint { pid: 10, start: 1 },
            kernel_boot: "old-boot".into(),
        },
        descriptor: b"old storage".to_vec(),
    };
    assert_eq!(
        control.replay_scope(&target).unwrap().as_str(),
        "session:00000000-0000-0000-0000-000000000001"
    );
    target.scope.binding = BindingId::new("new-binding");
    target.scope.execution = ExecutionId::new("new-execution");
    target.scope.generation = 4;
    target.scope.process.pid = 20;
    target.scope.kernel_boot = "new-boot".into();
    target.descriptor = b"relocated storage".to_vec();
    assert_eq!(
        control.replay_scope(&target).unwrap().as_str(),
        "session:00000000-0000-0000-0000-000000000001"
    );
    target.scope.native_session = NativeSessionId::new("not-a-native-uuid");
    assert!(control.replay_scope(&target).is_err());
}

#[test]
fn decoded_native_turn_identity_survives_fresh_decoder_event_ids() {
    use herdr_threads::harness::adapter::{
        AdmissionRequest, HookInput, InstallObservation, TurnEdge,
    };
    let registry = builtins();
    let registration = registry.by_id(registry.agent("codex").unwrap()).unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(30000),
        cancellation: Default::default(),
    };
    let admitted = registration
        .admit(
            &AdmissionRequest {
                installed: InstallObservation::NotRequested,
                input: None,
                runtime_candidate: None,
                admitted_at: None,
            },
            &budget,
        )
        .unwrap();
    let payload=serde_json::to_vec(&serde_json::json!({"hook_event_name":"UserPromptSubmit","session_id":"session","turn_id":"native-turn","prompt":"normal user input"})).unwrap();
    let mut event_ids = Vec::new();
    for _ in 0..2 {
        let decoded = registration
            .decode(
                &admitted,
                &HookInput {
                    registered_event: Some("UserPromptSubmit".into()),
                    bytes: payload.clone(),
                },
            )
            .unwrap();
        assert_eq!(
            decoded
                .metadata
                .native_turn_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("native-turn")
        );
        assert_eq!(registration.turn_state_edge(&decoded), Some(TurnEdge::Busy));
        event_ids.push(decoded.event_id);
    }
    assert_ne!(
        event_ids[0], event_ids[1],
        "decoder keys are not native identity"
    );
    for id in ["x".repeat(129), "bad id".into()] {
        let decoded = registration
            .decode(
                &admitted,
                &HookInput {
                    registered_event: Some("UserPromptSubmit".into()),
                    bytes: serde_json::to_vec(&serde_json::json!({
                        "hook_event_name":"UserPromptSubmit", "session_id":"session",
                        "turn_id":id, "prompt":"normal user input"
                    }))
                    .unwrap(),
                },
            )
            .unwrap();
        assert!(decoded.metadata.native_turn_id.is_none());
        assert_eq!(
            registration.turn_state_edge(&decoded),
            Some(TurnEdge::Busy),
            "invalid optional identity does not remove the legacy edge"
        );
    }
}
