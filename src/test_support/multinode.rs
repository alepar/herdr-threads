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
    test_support::{fake_herdr::FakeHerdr, isolation::TestIsolation, spawn::SpawnOwned},
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
    /// Nodes that may discover peers (`sync.discover`). The others have it
    /// off, so a machine's real `herdr --remote` clients never become targets.
    pub discover: Vec<usize>,
    /// `(node, name, value)` environment of a node's daemon.
    pub env: Vec<(usize, String, String)>,
    /// Route every dial through a relay that can hold a node's frames back
    /// ([`Cluster::hold_frames_from`]). Needs `python3`.
    pub gated: bool,
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
    env: Vec<(String, String)>,
    /// The node's fake Herdr servers, in the order [`Cluster::add_host`] gave them.
    hosts: Vec<Arc<FakeHerdr>>,
}

/// A stand-in agent seat on a node's fake Herdr server: what
/// [`Cluster::seat`] returns and [`Cluster::run_as`] acts as.
#[derive(Clone, Debug)]
pub struct Seat {
    pub id: String,
    pub node: usize,
    pub pane: String,
    /// Index of the seat's server among the node's hosts.
    pub host: usize,
    session: String,
}

/// The relay a gated cluster's dials run through: `relay.py ROOT FROM TO --
/// CMD...` runs the pump `CMD` and forwards both directions frame by frame. The
/// first four frames of each direction (`FREE`) pass at once (hello,
/// auth, accept, the full `have`); after that a frame of node `X` waits while
/// `ROOT/hold/X` exists. A link therefore completes its handshake and both
/// sides learn each other's vectors, but a held node's `want`s and entries do
/// not move.
const RELAY: &str = r#"import os, subprocess, sys, threading, time
root, frm, to = sys.argv[1:4]
cmd = sys.argv[5:]
FREE = 4
child = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)

def read_exact(fd, n):
    buf = b""
    while len(buf) < n:
        chunk = os.read(fd, n - len(buf))
        if not chunk:
            return None
        buf += chunk
    return buf

def write_all(fd, data):
    while data:
        data = data[os.write(fd, data):]

def pump(src, dst, who, skip, close_dst):
    try:
        if skip:
            head = read_exact(src, skip)
            if head is None:
                return
            write_all(dst, head)
        passed = 0
        while True:
            head = read_exact(src, 4)
            if head is None:
                break
            body = read_exact(src, int.from_bytes(head, "big"))
            if body is None:
                break
            if passed >= FREE:
                while os.path.exists(root + "/hold/" + who):
                    time.sleep(0.02)
            passed += 1
            write_all(dst, head + body)
    except OSError:
        pass
    finally:
        if close_dst:
            try:
                os.close(dst)
            except OSError:
                pass

threading.Thread(target=pump, args=(0, child.stdin.fileno(), frm, 0, True), daemon=True).start()
out = threading.Thread(target=pump, args=(child.stdout.fileno(), 1, to, 8, False), daemon=True)
out.start()
child.wait()
out.join(timeout=1)
"#;

/// A copy of one node's `node/` directory (key, store, settings), taken with
/// its daemon stopped: what [`Cluster::backup`] returns and
/// [`Cluster::restore`] puts back.
#[derive(Debug)]
pub struct Backup {
    dir: PathBuf,
}

/// Files of a node directory that belong to a running daemon, not to its
/// state: never copied in a backup and never replaced by a restore.
const RUNTIME_FILES: [&str; 3] = ["endpoint.json", "daemon.sock", "owner.lock"];

fn copy_state(from: &Path, to: &Path) {
    private_dir(to);
    for entry in fs::read_dir(from).expect("read the node directory") {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if RUNTIME_FILES.contains(&name.to_string_lossy().as_ref()) {
            continue;
        }
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            copy_state(&entry.path(), &to.join(&name));
        } else if kind.is_file() {
            // A file that went away since the listing (a WAL sidecar of a store
            // closing) is not state.
            match fs::copy(entry.path(), to.join(&name)) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => panic!("copy a state file: {e}"),
            }
        }
    }
}

fn clear_state(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read the node directory") {
        let entry = entry.unwrap();
        if RUNTIME_FILES.contains(&entry.file_name().to_string_lossy().as_ref()) {
            continue;
        }
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            fs::remove_dir_all(&path).expect("remove a state directory");
        } else {
            fs::remove_file(&path).expect("remove a state file");
        }
    }
}

impl Seat {
    /// The same seat acting through another copy of its node
    /// ([`Cluster::clone_node`]): same id, pane and server, other daemon.
    pub fn on_node(&self, node: usize) -> Seat {
        Seat {
            node,
            ..self.clone()
        }
    }
}

/// Whether nobody holds the owner lease at `path` (a missing file is released).
fn lease_released(path: &Path) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(file) = fs::File::open(path) else {
        return true;
    };
    // SAFETY: the descriptor is open for the call; the lock is dropped with it.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
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
        private_dir(&root.join("down"));
        let shim = root.join("shim.sh");
        write_private(
            &shim,
            0o700,
            format!(
                "#!/bin/sh\n\
                 base={base}\n\
                 root=\"$1\"; from=\"$2\"; dest=\"$3\"; to=$(basename \"$dest\")\n\
                 case \"$dest\" in \"$base\"/n[0-9]*) ;; *) echo \"not a cluster node: $dest\" >&2; exit 75;; esac\n\
                 [ -e \"$root/cut/$from-$to\" ] && {{ echo \"partitioned $from-$to\" >&2; exit 75; }}\n\
                 [ -e \"$root/down/$to\" ] && {{ echo \"down $to\" >&2; exit 75; }}\n\
                 echo $$ > \"$root/pids/$from-$to\"\n\
                 [ -e \"$root/cut/$from-$to\" ] && {{ echo \"partitioned $from-$to\" >&2; exit 75; }}\n\
                 [ -e \"$root/relay.py\" ] && exec python3 \"$root/relay.py\" \"$root\" \"$from\" \"$to\" -- {bin} --state-dir \"$dest\" peer --stdio\n\
                 exec {bin} --state-dir \"$dest\" peer --stdio\n",
                base = sh_quote(&iso.state_root().to_string_lossy()),
                bin = sh_quote(&bin.to_string_lossy()),
            )
            .as_bytes(),
        );
        if options.gated {
            private_dir(&root.join("hold"));
            write_private(&root.join("relay.py"), 0o600, RELAY.as_bytes());
        }
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
                format!(
                    r#"{{"sync":{{"dial_command":{},"discover":{}}}}}"#,
                    json_string(&dial),
                    options.discover.contains(&index)
                )
                .as_bytes(),
            );
            nodes.push(Node {
                name,
                state,
                seed,
                uuid: NodeIdentity::from_seed(seed).uuid(),
                up: false,
                env: options
                    .env
                    .iter()
                    .filter(|(n, _, _)| *n == index)
                    .map(|(_, k, v)| (k.clone(), v.clone()))
                    .collect(),
                hosts: Vec::new(),
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

    fn down_flag(&self, node: usize) -> PathBuf {
        self.iso
            .path("cluster")
            .join("down")
            .join(&self.nodes[node].name)
    }

    fn pid_file(&self, from: usize, to: usize) -> PathBuf {
        self.iso
            .path("cluster")
            .join("pids")
            .join(format!("{}-{}", self.nodes[from].name, self.nodes[to].name))
    }

    /// Starts `node`'s daemon. A node with a fake Herdr server starts it as a
    /// user's command does (`daemon ensure`, then a host-scoped request that
    /// also attaches to an already-running daemon); any other node the way a
    /// peer does (store-only). A wiped store
    /// restarts store-only either way, see [`Cluster::wipe_store`].
    pub fn start_node(&mut self, node: usize) {
        if self.nodes[node].up {
            return;
        }
        if let Some(host) = self.nodes[node].hosts.first() {
            let mut command = self.iso.command(&self.bin);
            let output = command
                .envs(self.nodes[node].env.iter().map(|(k, v)| (k, v)))
                .arg("--state-dir")
                .arg(&self.nodes[node].state)
                .arg("--host-endpoint")
                .arg(host.socket())
                .args(["daemon", "ensure"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "daemon ensure of node {}: {output:?}",
                self.nodes[node].name
            );
            self.nodes[node].up = true;
            // A pump already past the shutdown gate may have won the
            // election store-only. Ensure only checks that daemon's health;
            // attach through a host-scoped request before allowing new dials.
            self.attach_node_host(node);
            let _ = fs::remove_file(self.down_flag(node));
            return;
        }
        self.start_store_only(node);
    }

    /// Starts `node`'s daemon the way a peer does: a `peer --stdio` pump
    /// ensures it (store-only, no Herdr server attached), prints the preamble
    /// and is let go.
    fn start_store_only(&mut self, node: usize) {
        let mut command = self.iso.command(&self.bin);
        command.envs(self.nodes[node].env.iter().map(|(k, v)| (k, v)));
        let mut pump = command
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
        let _ = fs::remove_file(self.down_flag(node));
    }

    /// `daemon stop` on `node`, then waits until it has withdrawn its endpoint.
    pub fn stop_node(&mut self, node: usize) {
        if !self.nodes[node].up {
            return;
        }
        // A stopped node is a machine that is off: no peer's dial may start its
        // daemon again (a pump ensures the remote daemon, store-only) until the
        // test starts it.
        let _ = fs::write(self.down_flag(node), b"");
        let _ = self.daemon_verb(node, "stop");
        let endpoint = self.nodes[node].state.join("node").join("endpoint.json");
        self.wait_until(
            &format!("node {} to withdraw its endpoint", self.nodes[node].name),
            Duration::from_secs(60),
            || (!endpoint.exists()).then_some(()),
        );
        // The endpoint goes first; the daemon is gone when it has let go of its
        // owner lease (its WAL is then checkpointed and closed).
        let lease = self.nodes[node].state.join("node").join("owner.lock");
        self.wait_until(
            &format!("node {} to release its owner lease", self.nodes[node].name),
            Duration::from_secs(60),
            || lease_released(&lease).then_some(()),
        );
        self.nodes[node].up = false;
    }

    /// A `daemon` verb against `node`, by state dir alone: a store-only node
    /// needs no host.
    fn daemon_verb(&self, node: usize, verb: &str) -> Output {
        self.iso
            .command(&self.bin)
            .env("PATH", "/usr/bin:/bin")
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
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

    /// Gives `node` another fake Herdr server with idle Claude agents in
    /// `panes` and returns its index. A CLI call naming its socket attaches it
    /// to the node's daemon lazily, so the daemon keeps running as started.
    pub fn add_host(&mut self, node: usize, panes: &[&str]) -> usize {
        let index = self.nodes[node].hosts.len();
        let socket = self.iso.socket_path(&format!("h{node}{index}"));
        let label = format!("{}h{index}", self.nodes[node].name);
        self.nodes[node]
            .hosts
            .push(Arc::new(FakeHerdr::start(&label, &socket, panes)));
        index
    }

    /// `node`'s first fake Herdr server. Panics when it has none.
    pub fn host(&self, node: usize) -> &FakeHerdr {
        self.host_at(node, 0)
    }

    /// `node`'s `index`th fake Herdr server.
    pub fn host_at(&self, node: usize, index: usize) -> &FakeHerdr {
        self.nodes[node]
            .hosts
            .get(index)
            .unwrap_or_else(|| panic!("node {node} has no fake host {index}"))
    }

    /// The CLI of `node` as `seat` (a stand-in agent on its pane through the
    /// cooperative caller flags), or as nobody when `seat` is `None`, against
    /// the node's fake host when it has one. Returns exit code, the parsed
    /// `--json` stdout and stderr.
    pub fn run_as(
        &self,
        seat: Option<&Seat>,
        node: usize,
        args: &[&str],
    ) -> (i32, serde_json::Value, String) {
        self.run_on(seat, seat.map_or(0, |s| s.host), node, args)
    }

    /// [`Cluster::run_as`] naming the node's `host`th server as the endpoint.
    pub fn run_on(
        &self,
        seat: Option<&Seat>,
        host: usize,
        node: usize,
        args: &[&str],
    ) -> (i32, serde_json::Value, String) {
        let output = self
            .seat_command(seat, host, node, true)
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let value = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
        (output.status.code().unwrap_or(-1), value, stderr)
    }

    /// The compact text form (stdout is not a terminal) of `args` as `seat`:
    /// exit code, stdout, stderr.
    pub fn text_as(
        &self,
        seat: Option<&Seat>,
        node: usize,
        args: &[&str],
    ) -> (i32, String, String) {
        let output = self
            .seat_command(seat, seat.map_or(0, |s| s.host), node, false)
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    /// The CLI of `node` against its `host`th server, as `seat` when given.
    fn seat_command(
        &self,
        seat: Option<&Seat>,
        host: usize,
        node: usize,
        json: bool,
    ) -> std::process::Command {
        let hosts = &self.nodes[node].hosts;
        let endpoint = match hosts.get(host) {
            Some(host) => host.socket().to_path_buf(),
            None => self.iso.path("no-host.sock"),
        };
        let mut command = self.iso.command(&self.bin);
        if json {
            command.arg("--json");
        }
        command
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
            .arg("--host-endpoint")
            .arg(&endpoint);
        if let Some(seat) = seat {
            crate::test_support::isolation::as_agent(&mut command, "claude", &seat.session);
            command.args([
                "--cooperative-seat",
                &seat.id,
                "--cooperative-target",
                &seat.pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
            ]);
        }
        command
    }

    /// A `--json` command of `seat`'s node as the seat's pane shell: the
    /// caller flags of [`Cluster::run_as`] plus the pane route (`HERDR_ENV`,
    /// `HERDR_PANE_ID`, `HERDR_SOCKET_PATH`) that the mod's `watch` child
    /// runs with. The caller appends the subcommand and spawns it through
    /// `test_support::spawn`.
    pub fn pane_command(&self, seat: &Seat) -> std::process::Command {
        let mut command = self.seat_command(Some(seat), seat.host, seat.node, true);
        let socket = self.nodes[seat.node].hosts[seat.host]
            .socket()
            .to_path_buf();
        command
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", &seat.pane)
            .env("HERDR_SOCKET_PATH", socket);
        command
    }

    /// The session id of `seat`'s stand-in agent (what `watch --session` takes).
    pub fn session_of(&self, seat: &Seat) -> String {
        seat.session.clone()
    }

    /// A read-only view of `node`'s store; `None` while it is missing or busy.
    pub fn open_store(&self, node: usize) -> Option<Connection> {
        open_db(&self.nodes[node].state)
    }

    /// Success of `args` as `seat`; returns `result.data`.
    pub fn ok_as(&self, seat: Option<&Seat>, node: usize, args: &[&str]) -> serde_json::Value {
        let (code, value, stderr) = self.run_as(seat, node, args);
        assert_eq!(code, 0, "{args:?} failed on node {node}: {stderr}{value}");
        value["result"]["data"].clone()
    }

    /// Resolves `pane` of `node`'s first host to its seat and checks the
    /// stand-in agent in (availability and the binding a wake needs).
    pub fn seat(&self, node: usize, pane: &str) -> Seat {
        self.seat_on(node, 0, pane)
    }

    /// [`Cluster::seat`] on the node's `host`th server.
    pub fn seat_on(&self, node: usize, host: usize, pane: &str) -> Seat {
        let probe = Seat {
            id: String::new(),
            node,
            pane: pane.to_owned(),
            host,
            session: String::new(),
        };
        let (code, value, stderr) =
            self.run_on(None, host, node, &["seat", "resolve", "--pane", pane]);
        assert_eq!(
            code, 0,
            "seat resolve {pane} on node {node}: {stderr}{value}"
        );
        let id = value["result"]["data"].clone();
        let seat = Seat {
            id: id.as_str().expect("a seat id").to_owned(),
            session: crate::test_support::fake_herdr::session_of(
                self.host_at(node, host).label(),
                pane,
            ),
            ..probe
        };
        let (code, _, stderr) = self.run_as(
            Some(&seat),
            node,
            &["check-in", "--lifecycle-event", "agent-start"],
        );
        assert_eq!(code, 0, "check-in of {}: {stderr}", seat.id);
        seat
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

    /// The operator form of the CLI (`human ...`) against `node`, naming its
    /// first fake server when it has one.
    pub fn human(&self, node: usize, args: &[&str]) -> Output {
        let endpoint = match self.nodes[node].hosts.first() {
            Some(host) => host.socket().to_path_buf(),
            None => self.iso.path("no-host.sock"),
        };
        self.iso
            .command(&self.bin)
            .arg("human")
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
            .arg("--host-endpoint")
            .arg(endpoint)
            .args(args)
            .output()
            .unwrap()
    }

    /// CLI in a person's isolated pane, without agent caller claims.
    pub fn human_pane_command(&self, seat: &Seat) -> std::process::Command {
        let mut command = self.iso.command(&self.bin);
        command
            .arg("human")
            .arg("--state-dir")
            .arg(&self.nodes[seat.node].state)
            .arg("--host-endpoint")
            .arg(self.nodes[seat.node].hosts[seat.host].socket())
            .env("HERDR_PANE_ID", &seat.pane);
        command
    }

    /// Records `pane` of `node`'s first fake server as a person's seat
    /// (`human me init --operator` with that pane as the caller's context)
    /// and returns it. The pane must not also host an agent seat of the
    /// test: one pane is one seat. The returned [`Seat`] names the person's
    /// seat as a recipient; it carries no agent session, so it is not a
    /// caller for [`Cluster::run_as`].
    pub fn human_seat(&self, node: usize, pane: &str) -> Seat {
        let host = &self.nodes[node].hosts[0];
        let output = self
            .iso
            .command(&self.bin)
            .arg("human")
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.nodes[node].state)
            .arg("--host-endpoint")
            .arg(host.socket())
            .args(["me", "init", "--operator"])
            .env("HERDR_PANE_ID", pane)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "human me init on {pane}: {output:?}"
        );
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("me init prints JSON");
        Seat {
            id: value["result"]["data"]["seat"]
                .as_str()
                .expect("a person's seat id")
                .to_owned(),
            node,
            pane: pane.to_owned(),
            host: 0,
            session: String::new(),
        }
    }

    /// Copies `node`'s state directory (daemon stopped around the copy).
    pub fn backup(&mut self, node: usize) -> Backup {
        let was_up = self.nodes[node].up;
        self.stop_node(node);
        let dir = self.iso.path(format!(
            "backup-{}-{}",
            self.nodes[node].name,
            uuid::Uuid::new_v4().simple()
        ));
        copy_state(&self.nodes[node].state.join("node"), &dir);
        if was_up {
            self.start_node(node);
        }
        Backup { dir }
    }

    /// Replaces `node`'s state with `backup` and starts its daemon: the store
    /// of the past, under the same key.
    pub fn restore(&mut self, node: usize, backup: &Backup) {
        self.stop_node(node);
        let state = self.nodes[node].state.join("node");
        clear_state(&state);
        copy_state(&backup.dir, &state);
        self.start_node(node);
    }

    /// A second node whose state is a copy of `node`'s (key, store, settings): the
    /// clone of a cloned VM or a restored image. Both nodes are left stopped, so
    /// the test chooses which daemon starts first; the copy shares `node`'s fake
    /// Herdr servers and answers to the same node uuid. Returns its index.
    pub fn clone_node(&mut self, node: usize) -> usize {
        self.stop_node(node);
        let index = self.nodes.len();
        let name = format!("n{index}");
        let state = self.iso.path(&name);
        copy_state(&self.nodes[node].state.join("node"), &state.join("node"));
        let copy = Node {
            name,
            state,
            seed: self.nodes[node].seed,
            uuid: self.nodes[node].uuid,
            up: false,
            env: self.nodes[node].env.clone(),
            hosts: self.nodes[node].hosts.clone(),
        };
        self.nodes.push(copy);
        index
    }

    /// Stops `node`, moves its whole node directory (key and store) aside and
    /// starts it again: a fresh node with a new key under the same state
    /// directory. Its settings are put back (a dial command), as an operator's
    /// would be. Returns the new node uuid.
    pub fn restart_as_new_node(&mut self, node: usize) -> NodeUuid {
        self.stop_node(node);
        let state = self.nodes[node].state.join("node");
        let aside = self.iso.path(format!("aside-{}", self.nodes[node].name));
        fs::rename(&state, &aside).expect("move the node directory aside");
        private_dir(&state);
        let settings = fs::read(aside.join("settings.json")).expect("the node's settings");
        write_private(&state.join("settings.json"), 0o600, &settings);
        self.start_node(node);
        let uuid = self
            .wait_until("the new node's identity", Duration::from_secs(30), || {
                self.scalar(node, "SELECT id FROM local_node", &[])
            })
            .parse::<NodeUuid>()
            .expect("a node uuid");
        self.nodes[node].uuid = uuid;
        self.nodes[node].seed = [0; 32];
        uuid
    }

    /// Stops `node` and moves its store aside, key kept: a wiped store under the
    /// same node identity. The daemon is started again store-only, as a peer
    /// brings a wiped node back, even when the node has a fake Herdr server:
    /// its own seats wait for a host ([`Cluster::attach_host`]). The old store
    /// stays in the cluster's root.
    pub fn wipe_store(&mut self, node: usize) {
        self.stop_node(node);
        let aside = self.iso.path(format!("wiped-{}", self.nodes[node].name));
        private_dir(&aside);
        let state = self.nodes[node].state.join("node");
        for entry in fs::read_dir(&state).expect("read the node directory") {
            let entry = entry.unwrap();
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("herdr-threads.db") {
                fs::rename(entry.path(), aside.join(&name)).expect("move the store aside");
            }
        }
        self.start_store_only(node);
    }

    /// Attaches `seat`'s node to its fake Herdr server, then waits until its
    /// `herdr_hosts` holds a row. The attach is what any call naming a server
    /// causes in the daemon (here the ordinary `seat resolve` of a pane the
    /// server does not have, which fails after the attach); a claimed command
    /// of `seat` cannot do it, because the node does not know the seat until
    /// it is restored. No operator command.
    pub fn attach_host(&self, seat: &Seat) {
        self.attach_node_host(seat.node);
    }

    fn attach_node_host(&self, node: usize) {
        let _ = self.run_as(None, node, &["seat", "resolve", "--pane", "w1:p9"]);
        self.wait_until(
            &format!("node {} to attach its host", self.nodes[node].name),
            Duration::from_secs(60),
            || {
                self.scalar(node, "SELECT count(*) FROM herdr_hosts", &[])
                    .filter(|n| n != "0")
            },
        );
    }

    /// `human sync resync --from <from>` on `node`: the command stops its
    /// daemon, and the next start (here, at once) backs the store up and pulls
    /// from `from`. Returns the command's output.
    pub fn resync(&mut self, node: usize, from: &str) -> Output {
        // Refuse new peer dials while the command stops and restarts the
        // daemon. A pump already past this gate can still win the election;
        // start_node attaches the host to that store-only winner too.
        let _ = fs::write(self.down_flag(node), b"");
        let out = self.human(node, &["sync", "resync", "--from", from]);
        if out.status.success() {
            let endpoint = self.nodes[node].state.join("node").join("endpoint.json");
            self.wait_until(
                &format!("node {} to stop for the resync", self.nodes[node].name),
                Duration::from_secs(60),
                || (!endpoint.exists()).then_some(()),
            );
            let lease = self.nodes[node].state.join("node").join("owner.lock");
            self.wait_until(
                &format!("node {} to release its owner lease", self.nodes[node].name),
                Duration::from_secs(60),
                || lease_released(&lease).then_some(()),
            );
            self.nodes[node].up = false;
            self.start_node(node);
        } else {
            let _ = fs::remove_file(self.down_flag(node));
        }
        out
    }

    /// Pretends `node` last heard from `about` `ago` ago: restarts `node` around
    /// a write of that contact time (the daemon is down for it). Under a
    /// partition nothing refreshes it, so after the reach threshold the node
    /// reads as unreachable without waiting real minutes.
    pub fn age_contact(&mut self, node: usize, about: usize, ago: Duration) {
        let was_up = self.nodes[node].up;
        self.stop_node(node);
        let conn = Connection::open(db_path(&self.nodes[node].state)).expect("open the store");
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .saturating_sub(ago)
            .as_millis() as i64;
        conn.execute(
            "INSERT INTO sync_node_contact(node_uuid,last_contact_at,via,updated_at) \
             VALUES (?1,?2,'direct',?2) \
             ON CONFLICT(node_uuid) DO UPDATE SET last_contact_at = ?2, updated_at = ?2",
            rusqlite::params![self.nodes[about].uuid.to_string(), at],
        )
        .expect("age the contact");
        drop(conn);
        if was_up {
            self.start_node(node);
        }
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

    /// Holds back `node`'s frames on every link (after each link's handshake
    /// and first `have`) until [`Cluster::release_frames_from`]. Needs
    /// [`ClusterOptions::gated`] and applies to links dialed afterwards or
    /// already up alike.
    pub fn hold_frames_from(&self, node: usize) {
        let flag = self
            .iso
            .path("cluster")
            .join("hold")
            .join(&self.nodes[node].name);
        assert!(
            flag.parent().is_some_and(Path::exists),
            "hold_frames_from needs ClusterOptions::gated"
        );
        fs::write(flag, b"").expect("set the hold flag");
    }

    /// Lifts [`Cluster::hold_frames_from`]; the held frames flow.
    pub fn release_frames_from(&self, node: usize) {
        let _ = fs::remove_file(
            self.iso
                .path("cluster")
                .join("hold")
                .join(&self.nodes[node].name),
        );
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
            .enumerate()
            .map(|(index, n)| {
                let text = fs::read_to_string(
                    n.state
                        .join("node")
                        .join(crate::daemon::logs::DAEMON_LOG_FILE),
                )
                .unwrap_or_default();
                let status = if n.up {
                    let out = self.cli(index, &["sync", "status"]);
                    String::from_utf8_lossy(&out.stdout).into_owned()
                } else {
                    "(down)".into()
                };
                format!(
                    "--- {} ({})\nvector {:?}\n{status}\n{text}\n",
                    n.name,
                    n.state.display(),
                    self.vector(index)
                )
            })
            .collect()
    }

    /// The first column of the first row of `sql` on `node`'s store, as text
    /// (`None` for no row or a busy store). Read-only.
    pub fn scalar(
        &self,
        node: usize,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Option<String> {
        use rusqlite::types::Value;
        let value: Value = open_db(&self.nodes[node].state)?
            .query_row(sql, params, |r| r.get(0))
            .ok()?;
        Some(match value {
            Value::Null => "NULL".to_owned(),
            Value::Integer(i) => i.to_string(),
            Value::Real(f) => f.to_string(),
            Value::Text(t) => t,
            Value::Blob(b) => format!("{b:?}"),
        })
    }

    /// Every row of `sql` on `node`'s store as text columns (empty for a busy
    /// store). Read-only; for assertions over a few rows and for diagnostics.
    pub fn rows(&self, node: usize, sql: &str) -> Vec<Vec<String>> {
        use rusqlite::types::Value;
        let Some(conn) = open_db(&self.nodes[node].state) else {
            return Vec::new();
        };
        let Ok(mut stmt) = conn.prepare(sql) else {
            return Vec::new();
        };
        let width = stmt.column_count();
        stmt.query_map([], |row| {
            (0..width)
                .map(|i| {
                    Ok(match row.get::<_, Value>(i)? {
                        Value::Null => "NULL".to_owned(),
                        Value::Integer(i) => i.to_string(),
                        Value::Real(f) => f.to_string(),
                        Value::Text(t) => t,
                        Value::Blob(b) => b.iter().take(4).map(|x| format!("{x:02x}")).collect(),
                    })
                })
                .collect()
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    /// `(kind, count)` of every journal row `node` holds.
    pub fn kinds(&self, node: usize) -> std::collections::BTreeMap<String, i64> {
        let Some(conn) = open_db(&self.nodes[node].state) else {
            return Default::default();
        };
        let Ok(mut stmt) = conn.prepare("SELECT kind, count(*) FROM journal GROUP BY kind") else {
            return Default::default();
        };
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
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
