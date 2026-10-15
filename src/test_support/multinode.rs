//! Multi-node test harness (ht-5fr.10, spec §13): N real daemons in one test,
//! linked through `sync.dial_command`, never ssh.
//!
//! Responsibility
//! - Owns: [`Cluster`], which plants N node directories (one per node, under
//!   one [`TestIsolation`]), runs one daemon per node, wires explicit dial
//!   targets, partitions and heals node pairs, waits for the vectors to
//!   converge, drives the CLI per node, and authors a validly signed entry of
//!   an arbitrary kind on a chosen node ([`Cluster::author_kind`]).
//! - Hides: the dial shim, its partition flag files, how a daemon is started
//!   and stopped, and how a node's store is read without disturbing its daemon.
//! - Must not: be a second spawn site for anything but test children (every
//!   daemon starter and CLI call goes through [`TestIsolation::command`] and
//!   [`SpawnOwned`]; the daemons themselves are detached by their first
//!   `peer --stdio` pump exactly as in production and stopped by
//!   `daemon stop`), open a node's store for writing while its daemon runs,
//!   or know any fact kind's semantics.
//! - Role: test harness. Cross-node tests (ht-5fr.11 and later) build a
//!   cluster, author through the CLI or [`Cluster::author_kind`], and assert on
//!   [`Cluster::vector`], [`Cluster::journal_entry`] and the nodes' tables.
//!
//! Model. Each node is a spawned `herdr-threads` daemon (store-only: no Herdr
//! host), so any number of nodes run in one test process. The fake-locator
//! `HostFactory` (`fake_hosts`) exists only for in-process elected daemons, and
//! an in-process elected daemon redirects the process-wide stdout/stderr and
//! installs a panic hook (see `tests/integration.rs`, `ONE_DAEMON`), so a
//! cluster with a `FakeHost` per node cannot share one process; a cluster test
//! that needs a host on one node runs that node in-process itself and adds the
//! rest here.
//!
//! The dial shim. Every node's `settings.json` carries
//! `sync.dial_command = "exec <root>/shim.sh <root> <from> {dest}"`; a dial of
//! node `to` runs `shim.sh`, which refuses (exit 75) while the pair's cut flag
//! exists, records its pid, and `exec`s `herdr-threads --state-dir <dest> peer
//! --stdio`, so the dialer's child is the pump itself. [`Cluster::partition`]
//! sets both directions' flags first, then kills the recorded pumps; a dial
//! that raced the flag re-checks it after recording its pid, so no link
//! survives a partition. A healed pair relinks at the dialer's next backoff
//! attempt (full jitter, one second doubling per failure), so keep a
//! partition to a few seconds and wait for convergence with a generous bound.
//!
//! Targets live in the node's store and the Dialer loads them at start, so
//! [`Cluster::link`] restarts the dialing node (stop, insert, start), as the
//! two-daemon link tests do. [`Cluster::author_kind`] likewise authors with the
//! daemon down.

use crate::{
    app::SystemClock,
    node::{NodeIdentity, NodeUuid},
    protocol::time::Clock,
    store::{connection::StoreContext, journal, sync_dial_targets},
    sync::facts::{FactKind, FactRegistry},
    test_support::{isolation::TestIsolation, spawn::SpawnOwned},
};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};

/// `(node uuid, incarnation, contiguous counter, applied through)` per held log.
pub type LogRow = (String, i64, i64, i64);

/// What [`Cluster::journal_entry`] reads of one journal row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub kind: String,
    pub hash: Vec<u8>,
    pub apply_state: String,
}

/// Options of [`Cluster::start`].
#[derive(Clone, Debug, Default)]
pub struct ClusterOptions {
    /// Key seeds by node index; missing entries use [`default_seed`].
    pub seeds: Vec<[u8; 32]>,
    /// `(dialer, dest)` pairs given an explicit target before the first start.
    pub links: Vec<(usize, usize)>,
}

/// The seed of node `index` when the options give none: distinct per index,
/// and chosen so the first nodes' tags do not collide.
pub fn default_seed(index: usize) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&(index as u64 + 1).to_le_bytes());
    out[31] = 0x5a;
    out
}

struct Node {
    name: String,
    state: PathBuf,
    seed: [u8; 32],
    uuid: NodeUuid,
    up: bool,
}

/// N linked daemons under one isolated root. Dropping it stops them all.
pub struct Cluster {
    iso: TestIsolation,
    bin: PathBuf,
    nodes: Vec<Node>,
}

fn private_dir(path: &Path) {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
}

fn write_private(path: &Path, mode: u32, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap_or_else(|e| panic!("create {}: {e}", path.display()));
    file.write_all(bytes).unwrap();
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

fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

impl Cluster {
    /// Plants `n` nodes and starts their daemons. `bin` is the
    /// `herdr-threads` executable under test (`env!("CARGO_BIN_EXE_herdr-threads")`).
    /// Every `options.links` pair is a dial target from before the first
    /// start. Panics when a daemon does not come up.
    pub fn start(label: &str, bin: impl Into<PathBuf>, n: usize, options: ClusterOptions) -> Self {
        let bin = bin.into();
        let iso = TestIsolation::new(label);
        let root = iso.path("cluster");
        private_dir(&root.join("cut"));
        private_dir(&root.join("pids"));
        let shim = root.join("shim.sh");
        write_private(
            &shim,
            0o700,
            format!(
                "#!/bin/sh\n\
                 root=\"$1\"; from=\"$2\"; dest=\"$3\"; to=$(basename \"$dest\")\n\
                 [ -e \"$root/cut/$from-$to\" ] && {{ echo \"partitioned $from-$to\" >&2; exit 75; }}\n\
                 echo $$ > \"$root/pids/$from-$to\"\n\
                 [ -e \"$root/cut/$from-$to\" ] && {{ echo \"partitioned $from-$to\" >&2; exit 75; }}\n\
                 exec {} --state-dir \"$dest\" peer --stdio\n",
                sh_quote(&bin.to_string_lossy()),
            )
            .as_bytes(),
        );
        let mut nodes = Vec::new();
        for index in 0..n {
            let name = format!("n{index}");
            let state = iso.path(&name);
            let seed = options
                .seeds
                .get(index)
                .copied()
                .unwrap_or_else(|| default_seed(index));
            private_dir(&state.join("node"));
            write_private(&state.join("node").join("node.key"), 0o600, &seed);
            let dial = format!(
                "exec {} {} {} {{dest}}",
                sh_quote(&shim.to_string_lossy()),
                sh_quote(&root.to_string_lossy()),
                name
            );
            write_private(
                &state.join("node").join("settings.json"),
                0o600,
                format!(r#"{{"sync":{{"dial_command":{}}}}}"#, json_string(&dial)).as_bytes(),
            );
            nodes.push(Node {
                name,
                state,
                seed,
                uuid: NodeIdentity::from_seed(seed).uuid(),
                up: false,
            });
        }
        let mut cluster = Self { iso, bin, nodes };
        for (dialer, dest) in &options.links {
            // The store exists only once a daemon has run: create it, stop it,
            // insert the target; the final start picks it up.
            cluster.start_node(*dialer);
            cluster.stop_node(*dialer);
            cluster.insert_target(*dialer, *dest);
        }
        for index in 0..n {
            cluster.start_node(index);
        }
        cluster
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn state_dir(&self, node: usize) -> &Path {
        &self.nodes[node].state
    }

    pub fn node_uuid(&self, node: usize) -> NodeUuid {
        self.nodes[node].uuid
    }

    /// The isolation that owns the cluster's root (HOME, XDG, sockets).
    pub fn isolation(&self) -> &TestIsolation {
        &self.iso
    }

    fn cut_flag(&self, from: usize, to: usize) -> PathBuf {
        self.iso
            .path("cluster")
            .join("cut")
            .join(format!("{}-{}", self.nodes[from].name, self.nodes[to].name))
    }

    fn pid_file(&self, from: usize, to: usize) -> PathBuf {
        self.iso
            .path("cluster")
            .join("pids")
            .join(format!("{}-{}", self.nodes[from].name, self.nodes[to].name))
    }

    /// Starts `node`'s daemon the way a peer does: a pump ensures it, prints
    /// the preamble and is let go.
    pub fn start_node(&mut self, node: usize) {
        if self.nodes[node].up {
            return;
        }
        let mut pump = self
            .iso
            .command(&self.bin)
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
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
            panic!(
                "the pump did not start node {}'s daemon: {out:?}",
                self.nodes[node].name
            );
        }
        assert_eq!(&preamble, b"HTPEER1\n");
        drop(pump.stdin.take());
        let _ = pump.wait_with_output();
        self.nodes[node].up = true;
    }

    /// `daemon stop` on `node`, then waits until it has withdrawn its endpoint.
    pub fn stop_node(&mut self, node: usize) {
        if !self.nodes[node].up {
            return;
        }
        let _ = self.daemon_verb(node, "stop");
        let endpoint = self.nodes[node].state.join("node").join("endpoint.json");
        self.wait_until(
            &format!("node {} to withdraw its endpoint", self.nodes[node].name),
            Duration::from_secs(60),
            || (!endpoint.exists()).then_some(()),
        );
        self.nodes[node].up = false;
    }

    /// A `daemon` verb against `node`. The CLI wants a host endpoint for the
    /// call; `stop` and `health` never contact it, so a path naming no socket serves.
    fn daemon_verb(&self, node: usize, verb: &str) -> Output {
        self.iso
            .command(&self.bin)
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
            .arg("--host-endpoint")
            .arg(self.iso.path("no-host.sock"))
            .args(["daemon", verb])
            .output()
            .unwrap()
    }

    /// The CLI against `node`: its state dir, a host endpoint naming no
    /// socket, the scrubbed environment. Returns the whole output.
    pub fn cli(&self, node: usize, args: &[&str]) -> Output {
        self.iso
            .command(&self.bin)
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
            .arg("--host-endpoint")
            .arg(self.iso.path("no-host.sock"))
            .args(args)
            .output()
            .unwrap()
    }

    fn insert_target(&self, dialer: usize, dest: usize) {
        let mut conn = Connection::open(db_path(&self.nodes[dialer].state))
            .expect("open the dialer's store (it has run once)");
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        let tx = conn.transaction().unwrap();
        sync_dial_targets::add_explicit(&tx, self.nodes[dest].state.to_str().unwrap(), None, 1)
            .unwrap();
        tx.commit().unwrap();
    }

    /// Gives `dialer` an explicit target for `dest` (restarting the dialer).
    pub fn link(&mut self, dialer: usize, dest: usize) {
        self.stop_node(dialer);
        self.insert_target(dialer, dest);
        self.start_node(dialer);
    }

    /// Cuts every link between `a` and `b` and refuses new dials until
    /// [`Cluster::heal`].
    pub fn partition(&self, a: usize, b: usize) {
        for (from, to) in [(a, b), (b, a)] {
            let flag = self.cut_flag(from, to);
            let _ = fs::write(&flag, b"");
        }
        for (from, to) in [(a, b), (b, a)] {
            self.kill_pump(from, to);
        }
    }

    /// Lifts [`Cluster::partition`]; the dialers reconnect on their next attempt.
    pub fn heal(&self, a: usize, b: usize) {
        for (from, to) in [(a, b), (b, a)] {
            let _ = fs::remove_file(self.cut_flag(from, to));
        }
    }

    /// Kills the pump `from` last recorded for `to`, after checking the pid
    /// still names that pump (a stale file may hold a reused pid).
    fn kill_pump(&self, from: usize, to: usize) {
        let Some(pid) = fs::read_to_string(self.pid_file(from, to))
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
        else {
            return;
        };
        let out = std::process::Command::new("ps")
            .args(["-o", "args=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let args = String::from_utf8_lossy(&out.stdout);
        let needle = format!(
            "--state-dir {} peer --stdio",
            self.nodes[to].state.display()
        );
        if args.contains(&needle) {
            // SAFETY: the pid names the pump this cluster's shim exec'd.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }

    /// `(node, incarnation, contiguous, applied_through)` of every log `node`
    /// holds; empty while its store is missing or busy.
    pub fn vector(&self, node: usize) -> Vec<LogRow> {
        let Some(conn) = open_db(&self.nodes[node].state) else {
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

    /// Every node holds a log of every node and all vectors are equal. The
    /// vector is returned when so.
    pub fn converged(&self) -> Option<Vec<LogRow>> {
        let first = self.vector(0);
        let all_nodes = self
            .nodes
            .iter()
            .all(|n| first.iter().any(|l| l.0 == n.uuid.to_string()));
        (all_nodes && (1..self.nodes.len()).all(|i| self.vector(i) == first)).then_some(first)
    }

    /// Waits until [`Cluster::converged`]; panics with every daemon log on timeout.
    pub fn wait_converged(&self, timeout: Duration) -> Vec<LogRow> {
        self.wait_until("the vectors to converge", timeout, || self.converged())
    }

    /// Waits for `probe`; panics with the daemon logs after `timeout`.
    pub fn wait_until<T>(
        &self,
        what: &str,
        timeout: Duration,
        mut probe: impl FnMut() -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(found) = probe() {
                return found;
            }
            if Instant::now() > deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {what}\n{}",
                    self.logs()
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn logs(&self) -> String {
        self.nodes
            .iter()
            .map(|n| {
                let text =
                    fs::read_to_string(n.state.join("node").join("daemon.log")).unwrap_or_default();
                format!("--- {} ({})\n{text}\n", n.name, n.state.display())
            })
            .collect()
    }

    /// The state of `dialer`'s only dial target (`linked`, `backoff`, ...).
    pub fn target_state(&self, dialer: usize) -> Option<String> {
        open_db(&self.nodes[dialer].state)?
            .query_row("SELECT state FROM sync_dial_targets", [], |r| r.get(0))
            .ok()
    }

    /// The journal row of `node` for the log `(author, incarnation)` at `counter`.
    pub fn journal_entry(&self, node: usize, author: usize, counter: u64) -> Option<JournalEntry> {
        open_db(&self.nodes[node].state)?
            .query_row(
                "SELECT j.kind, j.hash, j.apply_state FROM journal j \
                 JOIN sync_logs l ON l.log_ix = j.log_ix \
                 WHERE l.node_uuid = ?1 AND j.counter = ?2 \
                 ORDER BY l.incarnation DESC LIMIT 1",
                rusqlite::params![self.nodes[author].uuid.to_string(), counter as i64],
                |r| {
                    Ok(JournalEntry {
                        kind: r.get(0)?,
                        hash: r.get(1)?,
                        apply_state: r.get(2)?,
                    })
                },
            )
            .ok()
    }

    /// Authors one entry of kind `K` on `node`, signed with the node's real
    /// key, and returns its counter and hash. `K` is registered only in the
    /// authoring store (the builtins plus `K`), so the running daemons, which
    /// know the builtins only, see an unknown kind: the mixed-versions
    /// configuration, an upgraded node authoring what an older one parks. The
    /// node is restarted around the write.
    pub fn author_kind<K: FactKind>(
        &mut self,
        node: usize,
        payload: &K::Payload,
    ) -> (u64, [u8; 32]) {
        let was_up = self.nodes[node].up;
        self.stop_node(node);
        let seed = self.nodes[node].seed;
        let ctx = StoreContext::new(
            db_path(&self.nodes[node].state),
            Arc::new(SystemClock::new()) as Arc<dyn Clock>,
        )
        .with_journal_seed(seed);
        let mut conn = ctx.open_writer().expect("open the node's store writer");
        let registry = FactRegistry::builtins_plus(|b| b.with::<K>());
        journal::open(
            &ctx,
            &mut conn,
            Arc::new(NodeIdentity::from_seed(seed)),
            "multinode",
            registry,
        )
        .expect("open the node's journal under the extended registry");
        let authored = ctx
            .execute_decision(
                &mut conn,
                |_| Ok(()),
                |tx, decision, ()| journal::author::<K>(tx, &ctx, decision, payload),
            )
            .expect("author the entry");
        drop(conn);
        drop(ctx);
        if was_up {
            self.start_node(node);
        }
        (authored.counter, authored.hash)
    }

    /// Stops every daemon: dialers first, so no pump is left to restart one.
    /// Idempotent; also runs on drop.
    pub fn stop(&mut self) {
        for index in 0..self.nodes.len() {
            if self.nodes[index].up {
                let _ = self.daemon_verb(index, "stop");
            }
        }
        for index in 0..self.nodes.len() {
            if self.nodes[index].up {
                let endpoint = self.nodes[index].state.join("node").join("endpoint.json");
                let deadline = Instant::now() + Duration::from_secs(30);
                while endpoint.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(25));
                }
                self.nodes[index].up = false;
            }
        }
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `text` as a JSON string literal.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string encodes")
}
