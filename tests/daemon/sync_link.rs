//! ht-5fr.5.10: two real daemons, composed with the sync runtime, link through
//! `sync.dial_command` (`exec <bin> --state-dir {dest} peer --stdio`, never
//! ssh), exchange genesis logs, converge, and apply what they ingest through
//! the composed Apply lane. Node A dials node B's state directory; B is
//! started by A's own dial (the pump ensures its daemon), so every process is
//! a descendant of a daemon this test stops.
//!
//! Only `node.genesis` and `node.retag` are registered fact kinds today, so
//! the fact authored on A after the link is a retag: the test picks two keys
//! whose tags collide, and the higher node retags once it applies the other's
//! genesis (a fact authored after the link, replicated to the other side).

use herdr_threads::node::{NodeIdentity, NodeUuid, tag_for};
use herdr_threads::store::sync_dial_targets;
use herdr_threads::test_support::{isolation::TestIsolation, spawn::SpawnOwned};
use rusqlite::{Connection, OpenFlags};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const WAIT: Duration = Duration::from_secs(60);

/// A seed for every small integer; distinct integers give distinct nodes.
fn seed(n: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..4].copy_from_slice(&n.to_le_bytes());
    out[31] = 0x5a;
    out
}

fn uuid_of(seed: [u8; 32]) -> NodeUuid {
    NodeIdentity::from_seed(seed).uuid()
}

/// Two seeds whose uuids carry the same tag at `k = 0`, the higher uuid
/// first (the node that retags on the collision).
fn colliding_seeds() -> ([u8; 32], [u8; 32]) {
    let mut seen: BTreeMap<String, [u8; 32]> = BTreeMap::new();
    for n in 100.. {
        let s = seed(n);
        let tag = tag_for(uuid_of(s), 0).to_string();
        if let Some(other) = seen.insert(tag, s) {
            return if uuid_of(s) > uuid_of(other) {
                (s, other)
            } else {
                (other, s)
            };
        }
    }
    unreachable!()
}

fn private_dir(path: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .unwrap();
}

/// A node directory holding `seed` as its key, before any daemon exists.
fn plant_key(state: &Path, seed: [u8; 32]) {
    private_dir(state);
    let node = state.join("node");
    private_dir(&node);
    let mut key = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(node.join("node.key"))
        .unwrap();
    key.write_all(&seed).unwrap();
}

fn db_path(state: &Path) -> PathBuf {
    state.join("node").join("herdr-threads.db")
}

/// A read-only view of a node's store; `None` while it is missing or busy.
fn open_db(state: &Path) -> Option<Connection> {
    let conn =
        Connection::open_with_flags(db_path(state), OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.busy_timeout(Duration::from_secs(5)).ok()?;
    Some(conn)
}

type LogRow = (String, i64, i64, i64);

/// `(node, incarnation, contiguous_counter, applied_through)` per held log.
fn logs(state: &Path) -> Vec<LogRow> {
    let Some(conn) = open_db(state) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT node_uuid, incarnation, contiguous_counter, applied_through \
         FROM sync_logs ORDER BY node_uuid, incarnation",
    ) else {
        return Vec::new();
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// The `(tag, tag_k)` the node directory of `state` holds for `node`.
fn directory_row(state: &Path, node: NodeUuid) -> Option<(String, i64)> {
    open_db(state)?
        .query_row(
            "SELECT tag, tag_k FROM node_directory WHERE node_uuid = ?1",
            [node.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
}

fn local_tag_k(state: &Path) -> Option<i64> {
    open_db(state)?
        .query_row("SELECT tag_k FROM local_node", [], |r| r.get(0))
        .ok()
}

/// The dial state of A's only target.
fn target_state(state: &Path) -> Option<String> {
    open_db(state)?
        .query_row("SELECT state FROM sync_dial_targets", [], |r| r.get(0))
        .ok()
}

fn wait_until<T>(what: &str, mut probe: impl FnMut() -> Option<T>, daemons: &[&Path]) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        if Instant::now() > deadline {
            let tails: String = daemons
                .iter()
                .map(|state| {
                    let text = fs::read_to_string(state.join("node").join("daemon.log"))
                        .unwrap_or_default();
                    format!("--- {}\n{text}\n", state.display())
                })
                .collect();
            panic!("timed out after {WAIT:?} waiting for {what}\n{tails}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A `daemon` verb against `state`. The CLI wants a host endpoint for the
/// call; `stop` and `health` never contact it, so a path naming no socket serves.
fn daemon(iso: &TestIsolation, state: &Path, verb: &str) -> std::process::Output {
    iso.command(BIN)
        .arg("--state-dir")
        .arg(state)
        .arg("--host-endpoint")
        .arg(iso.path("no-host.sock"))
        .args(["daemon", verb])
        .output()
        .unwrap()
}

/// Starts the store-only daemon of `state` the way a peer does: a pump
/// ensures it, prints the preamble, and is let go.
fn start_daemon(iso: &TestIsolation, state: &Path) {
    start_daemon_idle(iso, state, None);
}

/// [`start_daemon`] with the daemon's idle-exit period set to `idle_ms`
/// (test-support `HT_TEST_IDLE_EXIT_MS`); the daemon inherits it from the pump.
fn start_daemon_idle(iso: &TestIsolation, state: &Path, idle_ms: Option<u64>) {
    let mut command = iso.command(BIN);
    if let Some(ms) = idle_ms {
        command.env("HT_TEST_IDLE_EXIT_MS", ms.to_string());
    }
    let mut pump = command
        .arg("--state-dir")
        .arg(state)
        .args(["peer", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_owned()
        .unwrap();
    let mut preamble = [0u8; 8];
    if pump
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut preamble)
        .is_err()
    {
        let out = pump.wait_with_output().unwrap();
        panic!("the pump did not start the daemon: {out:?}");
    }
    assert_eq!(&preamble, b"HTPEER1\n");
    drop(pump.stdin.take());
    let _ = pump.wait_with_output();
}

/// `daemon stop`, then wait until the daemon has withdrawn its endpoint.
fn stop_daemon(iso: &TestIsolation, state: &Path) {
    daemon(iso, state, "stop");
    let endpoint = state.join("node").join("endpoint.json");
    wait_until(
        "the daemon to withdraw its endpoint",
        || (!endpoint.exists()).then_some(()),
        &[state],
    );
}

/// Node A (the dialer) and node B (its dial target), both under one isolated
/// root. Dropping the pair stops both daemons.
struct Pair {
    iso: TestIsolation,
    a: PathBuf,
    b: PathBuf,
    a_node: NodeUuid,
    b_node: NodeUuid,
}

impl Pair {
    /// Plants both keys and A's `sync.dial_command`, gives A its explicit
    /// target (B's state directory) while A's daemon is down, and starts A.
    /// B starts when A first dials it.
    fn start(label: &str, a_seed: [u8; 32], b_seed: [u8; 32]) -> Self {
        let iso = TestIsolation::new(label);
        let (a, b) = (iso.path("a"), iso.path("b"));
        plant_key(&a, a_seed);
        plant_key(&b, b_seed);
        let mut settings = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(a.join("node").join("settings.json"))
            .unwrap();
        write!(
            settings,
            r#"{{"sync":{{"dial_command":"exec {BIN} --state-dir {{dest}} peer --stdio"}}}}"#
        )
        .unwrap();
        drop(settings);
        let pair = Self {
            a_node: uuid_of(a_seed),
            b_node: uuid_of(b_seed),
            iso,
            a,
            b,
        };
        // The store exists only once a daemon has run: create it, stop it,
        // insert the target, start again.
        start_daemon(&pair.iso, &pair.a);
        stop_daemon(&pair.iso, &pair.a);
        let mut conn = Connection::open(db_path(&pair.a)).unwrap();
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        let tx = conn.transaction().unwrap();
        sync_dial_targets::add_explicit(&tx, pair.b.to_str().unwrap(), None, 1).unwrap();
        tx.commit().unwrap();
        drop(conn);
        start_daemon(&pair.iso, &pair.a);
        pair
    }

    fn daemons(&self) -> [&Path; 2] {
        [&self.a, &self.b]
    }

    /// Both stores hold the same logs of both nodes, fully applied.
    fn converged(&self) -> Option<Vec<LogRow>> {
        let (a, b) = (logs(&self.a), logs(&self.b));
        let both = [self.a_node.to_string(), self.b_node.to_string()]
            .iter()
            .all(|n| a.iter().any(|l| &l.0 == n));
        (both && a == b && a.iter().all(|l| l.2 == l.3)).then_some(a)
    }

    fn wait_linked(&self) {
        wait_until(
            "the dial target to be linked",
            || (target_state(&self.a).as_deref() == Some("linked")).then_some(()),
            &self.daemons(),
        );
    }
}

impl Drop for Pair {
    fn drop(&mut self) {
        // A first: with the dialer gone B sees EOF and nothing restarts it.
        daemon(&self.iso, &self.a, "stop");
        daemon(&self.iso, &self.b, "stop");
    }
}

#[test]
fn two_daemons_exchange_genesis_and_converge() {
    let pair = Pair::start("sync-link-converge", seed(1), seed(2));
    pair.wait_linked();
    let held = wait_until(
        "both stores to hold both genesis logs, applied",
        || pair.converged(),
        &pair.daemons(),
    );
    assert_eq!(held.len(), 2, "one log per node: {held:?}");
    // Applied as replicated rows, on each side, for the other node.
    for (state, other) in [(&pair.a, pair.b_node), (&pair.b, pair.a_node)] {
        let (tag, tag_k) = wait_until(
            "the other node's directory row",
            || directory_row(state, other),
            &pair.daemons(),
        );
        assert_eq!(tag_k, 0);
        assert_eq!(tag, tag_for(other, 0).to_string());
    }
}

#[test]
fn fact_on_a_applied_on_b_as_replicated_row() {
    // A is the higher uuid, so A retags after applying B's genesis: a fact
    // authored on A after the link, whose row B must apply.
    let (a_seed, b_seed) = colliding_seeds();
    let pair = Pair::start("sync-link-fact", a_seed, b_seed);
    pair.wait_linked();
    // The projection of A's retag on B, not only the vectors.
    let (tag, tag_k) = wait_until(
        "A's retag applied in B's node directory",
        || directory_row(&pair.b, pair.a_node).filter(|row| row.1 == 1),
        &pair.daemons(),
    );
    assert_eq!((tag, tag_k), (tag_for(pair.a_node, 1).to_string(), 1));
    assert_eq!(local_tag_k(&pair.a), Some(1), "A moved to its next tag");
    assert_eq!(
        local_tag_k(&pair.b),
        Some(0),
        "the lower node keeps its tag"
    );
    let held = wait_until(
        "both stores to converge on the retag",
        || pair.converged(),
        &pair.daemons(),
    );
    let a_log = held
        .iter()
        .find(|l| l.0 == pair.a_node.to_string())
        .expect("A's log");
    assert!(a_log.2 >= 2, "genesis and retag: {a_log:?}");
}

/// The `boot_id` line of the daemon's health.
fn boot_id(iso: &TestIsolation, state: &Path) -> String {
    let out = daemon(iso, state, "health");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("boot_id: ").map(str::to_owned))
        .unwrap_or_else(|| panic!("no boot_id in {out:?}"))
}

#[test]
fn killed_remote_gives_backoff_then_reconnects() {
    let pair = Pair::start("sync-link-backoff", seed(1), seed(2));
    pair.wait_linked();
    wait_until(
        "the first convergence",
        || pair.converged(),
        &pair.daemons(),
    );
    let before = boot_id(&pair.iso, &pair.b);
    // Watch A's target state through the whole outage: backoff may be short.
    let seen_backoff = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (a, seen, done) = (pair.a.clone(), seen_backoff.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if target_state(&a).as_deref() == Some("backoff") {
                    seen.store(true, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    // The remote daemon goes away; A's pump child ends and the link drops.
    stop_daemon(&pair.iso, &pair.b);
    wait_until(
        "A's target to enter backoff",
        || seen_backoff.load(Ordering::Acquire).then_some(()),
        &pair.daemons(),
    );
    // The next dial's pump starts B again and the link comes back.
    pair.wait_linked();
    done.store(true, Ordering::Release);
    watcher.join().unwrap();
    let after = boot_id(&pair.iso, &pair.b);
    assert_ne!(before, after, "B runs a new daemon after the restart");
}

/// Pids of the `peer --stdio` pumps whose state directory is `dest`.
fn pumps_for(dest: &Path) -> Vec<i32> {
    let needle = format!("--state-dir {} peer --stdio", dest.display());
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(&needle))
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect()
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn daemon_stop_on_dialer_reaps_children() {
    let pair = Pair::start("sync-link-reap", seed(1), seed(2));
    pair.wait_linked();
    let pumps = pumps_for(&pair.b);
    assert_eq!(pumps.len(), 1, "A's dial child: {pumps:?}");
    stop_daemon(&pair.iso, &pair.a);
    wait_until(
        "A's dial child to be killed and reaped",
        || pumps.iter().all(|pid| !alive(*pid)).then_some(()),
        &pair.daemons(),
    );
    assert!(pumps_for(&pair.b).is_empty());
}

// ---------------------------------------------------------------------------
// ht-5fr.5.11: idle exit of a store-only daemon with nothing to serve.

const IDLE_MS: u64 = 2_000;

/// One node whose dial command always fails (`exit 1`): an enabled target
/// never links, so only the target count can keep the daemon up.
struct Solo {
    iso: TestIsolation,
    state: PathBuf,
}

impl Solo {
    /// Creates the store, optionally inserts an enabled target, and starts the
    /// daemon with a 2 s idle-exit period.
    fn start(label: &str, with_target: bool) -> Self {
        let iso = TestIsolation::new(label);
        let state = iso.path("n");
        plant_key(&state, seed(7));
        let mut settings = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(state.join("node").join("settings.json"))
            .unwrap();
        write!(
            settings,
            r#"{{"sync":{{"dial_command":"exit 1 # {{dest}}"}}}}"#
        )
        .unwrap();
        drop(settings);
        // The store exists only once a daemon has run (idle exit disabled).
        start_daemon_idle(&iso, &state, Some(0));
        stop_daemon(&iso, &state);
        if with_target {
            let mut conn = Connection::open(db_path(&state)).unwrap();
            conn.busy_timeout(Duration::from_secs(5)).unwrap();
            let tx = conn.transaction().unwrap();
            sync_dial_targets::add_explicit(&tx, "ghost", None, 1).unwrap();
            tx.commit().unwrap();
        }
        start_daemon_idle(&iso, &state, Some(IDLE_MS));
        Self { iso, state }
    }

    fn endpoint(&self) -> PathBuf {
        self.state.join("node").join("endpoint.json")
    }

    /// The owner lock can be taken, i.e. no daemon holds it.
    fn lock_free(&self) -> bool {
        use std::os::fd::AsRawFd;
        let file = fs::File::open(self.state.join("node").join("owner.lock")).unwrap();
        // SAFETY: flock on a descriptor this function owns.
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    }
}

impl Drop for Solo {
    fn drop(&mut self) {
        daemon(&self.iso, &self.state, "stop");
    }
}

#[test]
fn store_only_daemon_without_target_idle_exits() {
    let solo = Solo::start("sync-idle-exit", false);
    let before = boot_id(&solo.iso, &solo.state);
    wait_until(
        "the idle daemon to withdraw its endpoint",
        || (!solo.endpoint().exists()).then_some(()),
        &[&solo.state],
    );
    wait_until(
        "the idle daemon to release its lock",
        || solo.lock_free().then_some(()),
        &[&solo.state],
    );
    let log = fs::read_to_string(solo.state.join("node").join("daemon.log")).unwrap();
    assert!(
        log.contains("idle exit: store-only, no peer links, no dial targets, no clients for 2 s"),
        "{log}"
    );
    // The next CLI use restarts it through ensure_running.
    start_daemon_idle(&solo.iso, &solo.state, Some(0));
    assert_ne!(boot_id(&solo.iso, &solo.state), before, "a new daemon boot");
}

#[test]
fn daemon_with_enabled_target_stays() {
    let solo = Solo::start("sync-idle-target", true);
    let before = boot_id(&solo.iso, &solo.state);
    // Three periods: an idle exit would have fired after the first.
    std::thread::sleep(Duration::from_millis(3 * IDLE_MS));
    assert!(solo.endpoint().exists(), "endpoint withdrawn");
    assert_eq!(boot_id(&solo.iso, &solo.state), before, "same daemon boot");
}
