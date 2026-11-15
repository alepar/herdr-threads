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
