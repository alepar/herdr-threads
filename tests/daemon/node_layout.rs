//! ht-5fr.1.11: the node layout end to end through the real binary. One
//! detached `daemon run --state-dir` per test, owned by the test
//! (`test_support::spawn_node_daemon`), stopped with `daemon stop` and the
//! owned-child drop; every state, home and socket lives under a
//! `TestIsolation` root. The CLI still needs some claim host to route a call,
//! so each call sets a `HERDR_SOCKET_PATH` that no Herdr server listens on.
use herdr_threads::{
    client::local::LocalSocketClient,
    daemon::{ownership::read_descriptor, paths::NodePaths},
    node::json::{NodeJson, read_hint},
    ports::LocalClient,
    protocol::{
        commands::{Command, ResolveSeat},
        ids::{HostLocator, HostTargetId, OperationId},
        results::{CommandResult, ErrorCode, Health},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    test_support::{isolation::TestIsolation, spawn::OwnedChild, spawn_node_daemon},
};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::Output,
    sync::Arc,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Node {
    iso: TestIsolation,
    state: PathBuf,
    stderr: PathBuf,
    daemon: Option<OwnedChild>,
}

impl Node {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let state = iso.path("state");
        fs::DirBuilder::new().mode(0o700).create(&state).unwrap();
        let stderr = iso.path("daemon.stderr");
        Self {
            iso,
            state,
            stderr,
            daemon: None,
        }
    }

    fn node_dir(&self) -> PathBuf {
        self.state.join("node")
    }

    /// A CLI call routed to this state directory with `socket` as its claim host.
    fn cli_from(&self, socket: &str, args: &[&str]) -> Output {
        self.iso
            .command(BIN)
            .arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .env("HERDR_SOCKET_PATH", self.iso.socket_path(socket))
            .output()
            .unwrap()
    }

    fn cli(&self, args: &[&str]) -> Output {
        self.cli_from("claim.sock", args)
    }

    fn spawn(&mut self, hint: Option<&Path>) {
        self.daemon = Some(
            spawn_node_daemon(&self.iso, BIN, &self.state, hint, &self.stderr)
                .expect("spawn the node daemon"),
        );
    }

    /// Start the daemon and wait until `daemon health` answers.
    fn start(&mut self, hint: Option<&Path>) -> Health {
        self.spawn(hint);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.daemon.as_mut().unwrap().try_wait().unwrap() {
                panic!(
                    "daemon exited early ({status}): {}",
                    fs::read_to_string(&self.stderr).unwrap_or_default()
                );
            }
            if let Some(health) = self.health("claim.sock") {
                return health;
            }
            assert!(Instant::now() < deadline, "daemon never answered health");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn health(&self, socket: &str) -> Option<Health> {
        let out = self.cli_from(socket, &["daemon", "health", "--json"]);
        if !out.status.success() {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        serde_json::from_value(value["result"]["data"].clone()).ok()
    }

    /// `daemon stop`, then wait for the daemon process to exit.
    fn stop(&mut self) {
        let Some(mut daemon) = self.daemon.take() else {
            return;
        };
        let _ = self.cli(&["daemon", "stop"]);
        let deadline = Instant::now() + Duration::from_secs(15);
        while daemon.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "daemon did not stop");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn node_json(&self) -> NodeJson {
        read_hint(&self.node_dir().join("node.json")).expect("node.json")
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if self.daemon.is_some() {
            let _ = self.cli(&["daemon", "stop"]);
        }
        // The owned child's own Drop stops what is left of its group.
        self.daemon.take();
        let _ = fs::remove_dir_all(self.iso.state_root());
    }
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .mode()
        & 0o777
}

/// Kills: a bare `daemon run --state-dir` that fails without a Herdr server,
/// reports a host it does not have, or lays out the node directory with the
/// wrong modes.
#[test]
fn state_dir_only_daemon_starts_store_only() {
    let mut node = Node::new("nl-store-only");
    let health = node.start(None);
    assert!(health.hosts.is_empty(), "{health:?}");
    let dir = node.node_dir();
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("node.key")), 0o600);
    assert!(read_hint(&dir.join("node.json")).is_some());
    assert!(dir.join("herdr-threads.db").is_file());
    // The probe did not end it: still the same boot, still running.
    assert!(node.daemon.as_mut().unwrap().try_wait().unwrap().is_none());
    let again = node.health("claim.sock").expect("second health");
    assert_eq!(again.boot_id, health.boot_id);
    let log = fs::read_to_string(dir.join("daemon.log")).unwrap_or_default();
    assert!(log.contains("store-only"), "{log}");
    node.stop();
}

/// Everything under `root`, relative path to (kind, bytes, mtime ns).
fn snapshot(root: &Path) -> Vec<(PathBuf, bool, Vec<u8>, i128)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, bool, Vec<u8>, i128)>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).unwrap();
            let mtime = i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec());
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if meta.is_dir() {
                out.push((relative, true, Vec::new(), mtime));
                walk(&path, root, out);
            } else {
                out.push((relative, false, fs::read(&path).unwrap(), mtime));
            }
        }
    }
    let mut out = Vec::new();
    out.push((PathBuf::new(), true, Vec::new(), {
        let meta = fs::metadata(root).unwrap();
        i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec())
    }));
    walk(root, root, &mut out);
    out
}

/// Kills: the node daemon opening, migrating, creating in or deleting from a
/// legacy `instances/` tree (bytes, mtimes and the directory listing compared).
#[test]
fn legacy_instances_tree_untouched() {
    let mut node = Node::new("nl-legacy");
    let legacy = node.state.join("instances").join("abc");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join("threads.sqlite3"),
        b"not a database, never opened",
    )
    .unwrap();
    fs::write(legacy.join("settings.json"), b"{\"legacy\":true}\n").unwrap();
    let before = snapshot(&node.state.join("instances"));
    assert_eq!(before.len(), 4, "instances, abc and two files: {before:?}");
    // Mtime granularity: a write by the daemon must land in a later tick.
    std::thread::sleep(Duration::from_millis(20));
    let health = node.start(None);
    assert!(health.hosts.is_empty());
    node.stop();
    assert_eq!(snapshot(&node.state.join("instances")), before);
}

/// Kills: a node whose address depends on the claim host, so that two callers
/// in different Herdr servers reach different daemons.
#[test]
fn different_herdr_socket_paths_reach_the_same_descriptor() {
    let mut node = Node::new("nl-same-descriptor");
    let first = node.start(None);
    let from_a = node.health("claim-a.sock").expect("health from claim a");
    let from_b = node.health("claim-b.sock").expect("health from claim b");
    assert_eq!(from_a.boot_id, first.boot_id);
    assert_eq!(from_b.boot_id, first.boot_id);
    assert_eq!(from_a.instance_id, from_b.instance_id);
    assert!(
        node.node_dir().join("endpoint.json").is_file(),
        "one descriptor in the node directory"
    );
    node.stop();
}

/// A Unix socket standing in for a Herdr server that exists but answers
/// nothing: it accepts and drops every connection until dropped.
struct SilentServer {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SilentServer {
    fn bind(path: &Path) -> Self {
        let listener = std::os::unix::net::UnixListener::bind(path).expect("bind the fake server");
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((connection, _)) => drop(connection),
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for SilentServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Kills: a daemon (or a CLI in front of it) that serves one Herdr server and
/// refuses a call from another, or one that attaches a second server at
/// startup instead of when a call first names it.
#[test]
fn second_claim_host_attaches_lazily() {
    let mut node = Node::new("nl-second-claim");
    let hint = node.iso.socket_path("herdr-a.sock");
    let health = node.start(Some(&hint));
    assert_eq!(
        health.hosts.len(),
        1,
        "the hinted host is attached: {health:?}"
    );
    // The attached host's own claim and a second server's claim both pass.
    let own = node
        .iso
        .command(BIN)
        .arg("--state-dir")
        .arg(&node.state)
        .args(["daemon", "ensure"])
        .env("HERDR_SOCKET_PATH", &hint)
        .output()
        .unwrap();
    assert!(
        own.status.success(),
        "{}",
        String::from_utf8_lossy(&own.stderr)
    );
    let second = node.iso.socket_path("herdr-b.sock");
    let other = node.cli_from("herdr-b.sock", &["daemon", "ensure"]);
    assert!(
        other.status.success(),
        "a second claim host is not refused: {}",
        String::from_utf8_lossy(&other.stderr)
    );
    assert_eq!(
        node.health("herdr-b.sock").expect("health").hosts.len(),
        1,
        "a call that names no pane attaches nothing"
    );
    // The second server comes up; the first call that names it attaches it.
    let _server = SilentServer::bind(&second);
    let paths = NodePaths::resolve(&fs::canonicalize(&node.state).unwrap()).unwrap();
    let descriptor = read_descriptor(&paths, node.node_json().node_uuid).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(herdr_threads::app::SystemClock::new());
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.node_uuid,
        Some(descriptor.boot_id),
    );
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    // The silent server answers no observation, so the call itself fails; the
    // attach before it is what this test measures.
    let _ = client.call(
        Command::ResolveSeat(ResolveSeat {
            host: HostLocator::parse(second.to_string_lossy().into_owned()).unwrap(),
            target: HostTargetId::new("p"),
            operation: OperationId::new("op"),
        }),
        &budget,
    );
    let after = node.health("herdr-b.sock").expect("health after");
    assert_eq!(after.hosts.len(), 2, "{after:?}");
    assert_eq!(after.boot_id, health.boot_id, "the same daemon served both");
    node.stop();
}

/// Kills: a restart that mints a new node uuid or tag from the existing key and
/// store, or rewrites `node.json` with other identity.
#[test]
fn restart_keeps_node_uuid_and_tag() {
    let mut node = Node::new("nl-restart");
    let first = node.start(None);
    let before = node.node_json();
    node.stop();
    let second = node.start(None);
    let after = node.node_json();
    assert_ne!(first.boot_id, second.boot_id, "a new daemon boot");
    assert_eq!(before.node_uuid, after.node_uuid);
    assert_eq!(before.tag, after.tag);
    assert_eq!(before.tag_k, after.tag_k);
    assert_eq!(first.instance_id, second.instance_id);
    assert_eq!(first.instance_id, after.node_uuid.to_string());
    node.stop();
}

/// Kills: a daemon that quietly mints a fresh key beside an existing store, or
/// fails without naming the variant and the remedy.
#[test]
fn missing_key_with_store_fails_with_remedy() {
    let mut node = Node::new("nl-missing-key");
    node.start(None);
    node.stop();
    fs::remove_file(node.node_dir().join("node.key")).unwrap();
    assert!(node.node_dir().join("herdr-threads.db").is_file());
    node.spawn(None);
    let mut daemon = node.daemon.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "daemon kept running without its key"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!status.success(), "{status}");
    let stderr = fs::read_to_string(&node.stderr).unwrap();
    assert!(stderr.contains("KeyMissingForStore"), "{stderr}");
    assert!(stderr.contains("restore node.key"), "{stderr}");
    assert!(
        !node.node_dir().join("node.key").exists(),
        "the key is never recreated beside a store"
    );
}

/// Kills: a store-only daemon answering a host-bound request for a server
/// that is not there with anything but `HostUnavailable` naming that server
/// (this speaks the protocol to the daemon directly).
#[test]
fn host_needing_request_on_store_only_is_host_unavailable() {
    let mut node = Node::new("nl-host-needed");
    node.start(None);
    // The daemon publishes its descriptor under the canonical state path.
    let paths = NodePaths::resolve(&fs::canonicalize(&node.state).unwrap()).unwrap();
    let uuid = node.node_json().node_uuid;
    let descriptor = read_descriptor(&paths, uuid).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(herdr_threads::app::SystemClock::new());
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.node_uuid,
        Some(descriptor.boot_id),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    };
    assert!(matches!(
        client.call(Command::Health, &budget()),
        Ok(CommandResult::Health(_))
    ));
    let refused = client
        .call(
            Command::ResolveSeat(ResolveSeat {
                host: herdr_threads::test_support::test_host_locator(),
                target: HostTargetId::new("p"),
                operation: OperationId::new("op"),
            }),
            &budget(),
        )
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::HostUnavailable, "{refused:?}");
    assert_eq!(
        refused.detail,
        format!(
            "Herdr server {} not reachable",
            herdr_threads::test_support::test_host_locator().as_str()
        )
    );
    node.stop();
}
