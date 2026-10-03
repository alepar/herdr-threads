use super::*;
use crate::protocol::{ids::ServiceAuthorId, time::UtcMillis};
use std::sync::mpsc;

const INSTANCE: &str = "lease-instance";
const BOOT: &str = "lease-boot";

fn leased() -> (Arc<LiveServiceGate>, SessionLease) {
    let gate = Arc::new(LiveServiceGate::new());
    let (connection, cancellation) = gate
        .register_session(INSTANCE, BOOT, ServiceAuthorId::new("lease"), UtcMillis(0))
        .unwrap();
    let lease = SessionLease {
        gate: gate.clone(),
        connection,
        cancellation,
    };
    (gate, lease)
}

fn connected(gate: &LiveServiceGate) -> bool {
    gate.inspect(INSTANCE, BOOT).connected
}

/// Holds the gate mutex (as a decision guard does) on a std thread until
/// `release` fires; a failsafe timeout keeps a broken drop from hanging the test.
fn hold_gate(gate: &Arc<LiveServiceGate>) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let gate = gate.clone();
    let thread = std::thread::spawn(move || {
        let _guard = gate.state();
        held_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });
    held_rx.recv().unwrap();
    (release_tx, thread)
}

async fn wait_revoked(gate: &LiveServiceGate) {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while connected(gate) {
        assert!(
            std::time::Instant::now() < deadline,
            "session was not revoked within 1 s of the guard release"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn drop_while_decision_guard_held_does_not_block_the_runtime() {
    let (gate, lease) = leased();
    let cancellation = lease.cancellation.clone();
    let (release, holder) = hold_gate(&gate);
    let started = std::time::Instant::now();
    drop(lease);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "drop waited for the decision guard"
    );
    assert!(
        cancellation.is_cancelled(),
        "contended drop must cancel the session"
    );
    let mut counter = 0;
    for _ in 0..10 {
        counter += 1;
        tokio::task::yield_now().await;
    }
    assert_eq!(counter, 10);
    release.send(()).unwrap();
    wait_revoked(&gate).await;
    holder.join().unwrap();
}

#[test]
fn drop_outside_a_runtime_revokes_on_a_std_thread() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let (gate, lease) = leased();
    let (release, holder) = hold_gate(&gate);
    let started = std::time::Instant::now();
    drop(lease);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "drop waited for the decision guard"
    );
    release.send(()).unwrap();
    holder.join().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while connected(&gate) {
        assert!(
            std::time::Instant::now() < deadline,
            "session was not revoked after the guard release"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn drop_without_contention_revokes_inline() {
    let (gate, lease) = leased();
    assert!(connected(&gate));
    drop(lease);
    assert!(
        !connected(&gate),
        "uncontended drop must revoke before returning"
    );
}
