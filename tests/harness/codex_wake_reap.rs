use super::*;
use std::{
    io::Write,
    os::unix::process::CommandExt,
    process::{ChildStdin, Stdio},
    time::{Duration, Instant},
};

struct Cleanup<'a>(&'a Children);
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for slot in &self.0.slots {
            let mut slot = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(child) = slot.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            *slot = None;
        }
    }
}
struct Gate(Option<ChildStdin>);
impl Gate {
    fn release(&mut self) {
        if let Some(mut input) = self.0.take() {
            let _ = input.write_all(b"exit\n");
        }
    }
}
fn gated_child(pool: &Children) -> (u32, Gate, crate::harness::wake::WakeHelperWitness) {
    let mut slot = pool.reserve().unwrap();
    let mut command = crate::test_support::spawn::command("/bin/sh");
    command
        .args(["-c", "read value"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    // leak-guard: spawn::command tags it; exact pool slot and Cleanup own kill/reap.
    *slot = Some(command.spawn().unwrap());
    let child = slot.as_mut().unwrap();
    let pid = child.id();
    let gate = Gate(child.stdin.take());
    let witness = super::super::helper::witness(pid).unwrap();
    (pid, gate, witness)
}
fn wait_exited(pid: u32) {
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        #[cfg(target_os = "linux")]
        let exited = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|s| {
                s.rsplit_once(')')
                    .map(|(_, tail)| tail.trim_start().starts_with('Z'))
            })
            .is_none_or(|zombie| zombie);
        // libproc answers ESRCH for both a reaped pid and a zombie.
        #[cfg(target_os = "macos")]
        let exited = super::super::macos::birth(pid) == Err(libc::ESRCH);
        if exited {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("owned test child did not exit");
}
fn assert_reaped(pid: u32) {
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
        -1,
        "maintenance did not reap owned child"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn owned_slots_retain_delayed_child_and_recover_capacity_after_reaping() {
    let pool = Children::new();
    let _cleanup = Cleanup(&pool);
    let (pid, mut gate, _) = gated_child(&pool);
    let mut reservations = Vec::new();
    for _ in 1..LIMIT {
        reservations.push(pool.reserve().unwrap());
    }
    assert!(pool.reserve().is_err());
    assert!(pool.pending(), "locked and retained slots are pending");
    let started = Instant::now();
    pool.reap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(pool.pending(), "a live child cannot be discarded");
    gate.release();
    wait_exited(pid);
    pool.reap();
    assert_reaped(pid);
    assert!(pool.reserve().is_ok(), "reaped slot returns capacity");
    drop(reservations);
    assert!(!pool.pending());
}

#[test]
fn externally_reaped_child_releases_bookkeeping_without_quiescence_claim() {
    let pool = Children::new();
    let _cleanup = Cleanup(&pool);
    let (pid, mut gate, _) = gated_child(&pool);
    gate.release();
    wait_exited(pid);
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
        pid as i32
    );
    assert!(pool.pending());
    pool.reap();
    assert!(!pool.pending());
}

mod fixture {
    use crate as herdr_threads;
    include!("../fixtures/wake_intent.rs");
}

#[test]
fn node_stop_retains_process_owner_and_new_node_reaps_before_recovery() {
    use crate::{
        harness::wake::{WakeHelperObserver, WakeHelperPurpose},
        host::process_table::HostProcessTable,
        notification::queue_recovery::QueueRecovery,
        ports::{PrepareWakeIntent, StorePort},
        protocol::time::Cancellation,
        service::{fair_writer::FairWriter, pacer::Pacer},
    };
    use std::sync::{Arc, mpsc};
    if crate::test_support::spawn::ran_in_own_process(
        module_path!(),
        "node_stop_retains_process_owner_and_new_node_reaps_before_recovery",
    ) {
        return;
    }
    let _cleanup = Cleanup(&OWNED);
    let (store, path, reservation, preparation) = fixture::fixture();
    let store = Arc::new(store);
    let clock: Arc<dyn crate::protocol::time::Clock> = Arc::new(fixture::IntentClock);
    let state_dir = path.parent().unwrap().to_owned();
    let recovery = Arc::new(QueueRecovery::new(
        store.clone(),
        Arc::new(FairWriter::new(32)),
        clock.clone(),
        Arc::new(HostProcessTable),
        state_dir.clone(),
        reservation.daemon_boot,
    ));
    recovery.tick(&fixture::budget()).unwrap();
    assert!(!recovery.has_work());
    let (pid, mut gate, witness) = gated_child(&OWNED);
    assert!(
        recovery.has_work(),
        "owned child must count with no witness or intent row"
    );
    let PrepareWakeIntent::Prepared(row) = store
        .prepare_wake_intent(&reservation, &preparation, &fixture::budget())
        .unwrap()
    else {
        panic!("prepared");
    };
    for edge in [
        crate::harness::adapter::TurnEdge::Busy,
        crate::harness::adapter::TurnEdge::Idle,
    ] {
        store
            .record_native_turn(
                &fixture::native_report(
                    &store,
                    &reservation,
                    &preparation,
                    "reap-node-baseline",
                    edge,
                ),
                &fixture::budget(),
            )
            .unwrap();
    }
    let journal = recovery
        .begin_helper(&row, WakeHelperPurpose::Add, None, &fixture::budget())
        .unwrap()
        .unwrap();
    journal.started(&witness).unwrap();
    journal.completed(false).unwrap();
    drop(journal);
    let stop = Cancellation::default();
    stop.cancel();
    let (sent, received) = mpsc::channel();
    let worker_recovery = recovery.clone();
    let worker_clock = clock.clone();
    let worker = std::thread::spawn(move || {
        let pacer = Pacer::new("owned-reap-stop", worker_clock, stop.clone());
        worker_recovery.run(&pacer, &stop, |_| {
            panic!("cancelled run performed metadata work")
        });
        sent.send(()).unwrap();
    });
    let stopped = received.recv_timeout(Duration::from_secs(1)).is_ok();
    if !stopped {
        gate.release();
    }
    worker.join().unwrap();
    assert!(stopped, "shutdown waited for a still-running owned child");
    assert!(OWNED.pending());
    assert!(
        store
            .wake_intent(&reservation.attempt, &fixture::budget())
            .unwrap()
            .unwrap()
            .cleanup_quarantined
    );
    drop(recovery);
    drop(store);
    gate.release();
    wait_exited(pid);
    let boot = uuid::Uuid::new_v4();
    let reopened = Arc::new(fixture::open(path, boot));
    let next = QueueRecovery::new(
        reopened.clone(),
        Arc::new(FairWriter::new(32)),
        clock,
        Arc::new(HostProcessTable),
        state_dir,
        boot,
    );
    next.tick(&fixture::budget()).unwrap();
    assert_reaped(pid);
    let row = reopened
        .wake_intent(&reservation.attempt, &fixture::budget())
        .unwrap()
        .unwrap();
    assert!(
        !row.cleanup_quarantined,
        "ordinary exact witness/group recovery must establish quiescence"
    );
    assert!(!next.has_work());
}
