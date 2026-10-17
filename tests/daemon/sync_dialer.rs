//! The Dialer's children, end to end through a real coordinator actor: a
//! silent peer times out the handshake and teardown kills and reaps its
//! process group, and cancelling mid-dial leaves no process behind. Children
//! are `sh` commands from `sync.dial_command`, never ssh.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use herdr_threads::node::NodeIdentity;
use herdr_threads::protocol::time::{Cancellation, Clock, MonoInstant, UtcMillis};
use herdr_threads::service::fair_writer::FairWriter;
use herdr_threads::service::pacer::Pacer;
use herdr_threads::service::workers::WorkerStatus;
use herdr_threads::store::connection::OwnHistoryHold;
use herdr_threads::store::sync_dial_targets::DialState;
use herdr_threads::sync::coordinator::{
    CoordinatorDeps, LogOnlyForkSink, SyncCoordinator, SyncHandle,
};
use herdr_threads::sync::dialer::{DialConfig, DialTemplate};
use herdr_threads::sync::memory_journal::MemoryJournal;
use herdr_threads::sync::sync_store::MemorySyncStore;
use herdr_threads::test_support::{TEST_NODE_SEED, isolation::TestIsolation};

struct WallClock(Instant);
impl Clock for WallClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000 + self.0.elapsed().as_millis() as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.elapsed().as_millis() as u64)
    }
}

struct Node {
    store: Arc<MemorySyncStore>,
    handle: SyncHandle,
    cancel: Cancellation,
    join: tokio::task::JoinHandle<()>,
}

/// A coordinator whose Dialer runs `command` (a `sync.dial_command` body) for
/// every target. Each child appends its pid to `pids` and is tagged with this
/// test process as owner.
fn start(command: &str, pids: &Path) -> Node {
    let template = format!(
        ": {{dest}}; export HT_TEST_OWNER={}; echo $$ >> '{}'; {command}",
        std::process::id(),
        pids.display()
    );
    let store = Arc::new(MemorySyncStore::new());
    let clock: Arc<dyn Clock> = Arc::new(WallClock(Instant::now()));
    let cancel = Cancellation::default();
    let log: herdr_threads::sync::coordinator::LogFn = Arc::new(|_| {});
    let (handle, _intake, join) = SyncCoordinator::start(CoordinatorDeps {
        journal: Arc::new(MemoryJournal::new(Arc::new(NodeIdentity::from_seed(
            TEST_NODE_SEED,
        )))),
        sync_store: store.clone(),
        identity: Arc::new(NodeIdentity::from_seed(TEST_NODE_SEED)),
        boot: uuid::Uuid::from_u128(7),
        writer: Arc::new(FairWriter::new(8)),
        clock: clock.clone(),
        pacer: Arc::new(Pacer::new("sync", clock.clone(), cancel.clone())),
        status: Arc::new(WorkerStatus::default()),
        log: log.clone(),
        fork_sink: Arc::new(LogOnlyForkSink::new(log, clock)),
        own_history: Arc::new(OwnHistoryHold::new(Arc::default())),
        dial: DialConfig {
            template: Some(DialTemplate::parse(&template).expect("template")),
            jitter_seed: Some(5),
        },
        resync_pin: None,
        cancel: cancel.clone(),
    });
    Node {
        store,
        handle,
        cancel,
        join,
    }
}

fn pids(file: &Path) -> Vec<i32> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

/// `kill(pid, 0)` fails with ESRCH once the process is gone and reaped; a
/// zombie still answers it.
fn gone(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

async fn until(what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < limit {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("not reached within {limit:?}: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preamble_then_silence_times_out_and_group_reaped() {
    let dir = TestIsolation::new("sync-dialer-timeout");
    let pid_file = dir.path("pids");
    let node = start(
        r#"exec sh -c 'printf "HTPEER1\n"; exec sleep 600'"#,
        &pid_file,
    );
    node.store.add_target("silent-peer");
    node.handle.reload_targets();
    // The 15 s handshake bound passes with no answer; the dial backs off.
    until(
        "backoff after the handshake timeout",
        Duration::from_secs(60),
        || {
            node.store
                .targets()
                .iter()
                .any(|t| t.state == DialState::Backoff)
        },
    )
    .await;
    let first = *pids(&pid_file).first().expect("the child recorded its pid");
    assert!(gone(first), "the timed-out child is killed and reaped");
    assert_eq!(node.handle.link_count(), 0);
    let row = node.store.targets().remove(0);
    assert_eq!(row.learned_node, None, "no handshake, no Linked");
    assert!(
        row.last_error
            .as_deref()
            .unwrap_or("")
            .contains("handshake"),
        "{:?}",
        row.last_error
    );
    node.cancel.cancel();
    node.join.await.unwrap();
    for pid in pids(&pid_file) {
        assert!(gone(pid), "pid {pid} survived the shutdown");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_mid_dial_leaves_no_process() {
    let dir = TestIsolation::new("sync-dialer-cancel");
    let pid_file = dir.path("pids");
    // Never prints the preamble: the dial is stuck reading it.
    let node = start("exec sleep 600", &pid_file);
    node.store.add_target("mute-peer");
    node.handle.reload_targets();
    until("the child started", Duration::from_secs(20), || {
        !pids(&pid_file).is_empty()
    })
    .await;
    let pid = pids(&pid_file)[0];
    assert!(!gone(pid), "the child is running before the cancel");
    assert_eq!(node.handle.enabled_target_count(), 1);
    let started = Instant::now();
    node.cancel.cancel();
    node.join.await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "shutdown stays bounded: {:?}",
        started.elapsed()
    );
    assert!(gone(pid), "cancellation killed and reaped the child");
    assert_eq!(node.handle.enabled_target_count(), 0);
}
