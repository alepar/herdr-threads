//! Real CLI, real private daemon, and native PTYs. Regressions measure bytes
//! rather than snapshots: idle must be silent and interaction must not erase
//! the entire screen. Each child has an owner and an isolated configuration.
use herdr_threads::{
    daemon::paths::{InstancePaths, RuntimeContext},
    test_support::{
        isolation::TestIsolation,
        spawn::{OwnedChild, SpawnOwned},
    },
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const WAIT: Duration = Duration::from_secs(10);

struct Host {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Host {
    fn start(socket: &std::path::Path) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                if stream.set_nonblocking(false).is_err()
                    || stream
                        .set_read_timeout(Some(Duration::from_millis(250)))
                        .is_err()
                    || stream
                        .set_write_timeout(Some(Duration::from_millis(250)))
                        .is_err()
                {
                    continue;
                }
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let id = &request["id"];
                let reply = match request["method"].as_str() {
                    Some("ping") => {
                        json!({"id":id,"result":{"type":"pong","version":"0.9.1","protocol":22}})
                    }
                    Some("session.snapshot") => {
                        json!({"id":id,"result":{"type":"session_snapshot","snapshot":{"version":"0.9.1","protocol":22,"panes":[],"agents":[],"tabs":[],"workspaces":[],"layouts":[]}}})
                    }
                    _ => {
                        json!({"id":id,"error":{"code":"pane_not_found","message":"private picker test has no panes"}})
                    }
                };
                let _ = writeln!(stream, "{reply}");
            }
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            let _ = writeln!(
                std::io::stderr(),
                "private picker host worker panicked during cleanup"
            );
        }
    }
}

struct Fixture {
    daemon: OwnedChild,
    _host: Host,
    iso: TestIsolation,
    host_socket: PathBuf,
    database: PathBuf,
    instance: String,
}
impl Fixture {
    fn new() -> Self {
        let iso = TestIsolation::new("read-picker-pty");
        let host_socket = iso.socket_path("host.sock");
        let host = Host::start(&host_socket);
        let state = iso.path("state");
        let context = RuntimeContext::explicit(state.clone(), host_socket.clone(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        let mut daemon_command = iso.command(BIN);
        daemon_command
            .args(["daemon", "run", "--state-dir"])
            .arg(&state)
            .arg("--host-endpoint")
            .arg(&host_socket)
            .env("CLAUDE_CONFIG_DIR", iso.path("claude"))
            .env("CODEX_HOME", iso.path("codex"))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut daemon = daemon_command.spawn_owned().unwrap();
        let deadline = Instant::now() + WAIT;
        let instance = loop {
            if paths.descriptor_path.exists() && paths.database_path.exists() {
                let db = Connection::open(&paths.database_path).unwrap();
                if let Ok(id) = db.query_row("SELECT id FROM host_instances LIMIT 1", [], |r| {
                    r.get::<_, String>(0)
                }) {
                    break id;
                }
            }
            assert!(
                daemon.try_wait().unwrap().is_none(),
                "private daemon exited during startup"
            );
            assert!(
                Instant::now() < deadline,
                "private daemon did not publish a store"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        Self {
            daemon,
            _host: host,
            iso,
            host_socket,
            database: paths.database_path,
            instance,
        }
    }
    fn command(&self) -> Command {
        let mut command = self.iso.command(BIN);
        command
            .arg("--state-dir")
            .arg(self.iso.path("state"))
            .arg("--host-endpoint")
            .arg(&self.host_socket)
            .env("CLAUDE_CONFIG_DIR", self.iso.path("claude"))
            .env("CODEX_HOME", self.iso.path("codex"))
            .env("TERM", "xterm-256color")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("AISW") {
                command.env_remove(key);
            }
        }
        command
    }
    fn db(&self) -> Connection {
        let db = Connection::open(&self.database).unwrap();
        db.busy_timeout(Duration::from_secs(2)).unwrap();
        db
    }
    fn thread(&self, id: &str, name: &str, at: i64, archived: bool) {
        self.db().execute("INSERT INTO threads(id,instance_id,name,topic,goal,created_at,updated_at,archived) VALUES (?1,?2,?3,'private picker fixture','test',?4,?4,?5)", params![id,self.instance,name,at,archived]).unwrap();
    }
    /// `count` active threads `{prefix}{n:03}` named `channel {n:03}`, created
    /// at `n`, committed in one transaction (one connection, one commit).
    fn threads(&self, prefix: &str, count: i64) {
        let mut db = self.db();
        let tx = db.transaction().unwrap();
        for n in 0..count {
            tx.execute("INSERT INTO threads(id,instance_id,name,topic,goal,created_at,updated_at,archived) VALUES (?1,?2,?3,'private picker fixture','test',?4,?4,0)", params![format!("{prefix}{n:03}"),self.instance,format!("channel {n:03}"),n]).unwrap();
        }
        tx.commit().unwrap();
    }
    fn message(&self, thread: &str, body: &str, seq: i64) {
        let db = self.db();
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,?2,?3,?4,'ordinary',?5,?4,?4)", params![format!("m{thread}{seq}"),self.instance,thread,seq,body]).unwrap();
        db.execute(
            "UPDATE threads SET next_sequence=?2+1 WHERE id=?1",
            params![thread, seq],
        )
        .unwrap();
        db.execute(
            "UPDATE host_instances SET decision_seq=MAX(decision_seq,?2) WHERE id=?1",
            params![self.instance, seq],
        )
        .unwrap();
    }
    fn obligations(&self, thread: &str) {
        let db = self.db();
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('sPty',?1,'resolved','native',1,0)", [&self.instance]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,'sPty','invited')",
            [thread],
        )
        .unwrap();
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('vPty',?1,'sPty',1,'pending',1,0,60000,9223372036854770000)", [thread]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,?2,'sPty','pending',60000,0,9223372036854770000)", params![format!("m{thread}1"),thread]).unwrap();
    }
    fn obligation_states(&self) -> (String, String, String) {
        self.db().query_row("SELECT (SELECT state FROM receipts WHERE seat_id='sPty'),(SELECT state FROM invitations WHERE id='vPty'),(SELECT state FROM memberships WHERE seat_id='sPty')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.daemon.stop();
    }
}

struct PausedDaemon(libc::pid_t);
impl PausedDaemon {
    fn new(fixture: &Fixture) -> Self {
        let pid = fixture.daemon.id() as libc::pid_t;
        // SAFETY: pause only the private daemon owned by this fixture.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
        Self(pid)
    }
}
impl Drop for PausedDaemon {
    fn drop(&mut self) {
        // SAFETY: resume our private daemon before its owned shutdown guard runs.
        unsafe {
            libc::kill(self.0, libc::SIGCONT);
        }
    }
}

/// Forward real daemon frames, holding only refresh page two at a deterministic
/// barrier. Rename only this fixture's socket; restore it and its descriptor
/// before the private daemon's shutdown. No thread data is fabricated here.
struct PageGate {
    reached: Receiver<()>,
    release: Sender<()>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<std::io::Result<()>>>,
    socket: PathBuf,
    upstream: PathBuf,
    descriptor: PathBuf,
    original_descriptor: Vec<u8>,
}
impl PageGate {
    fn new(fixture: &Fixture) -> Self {
        let context =
            RuntimeContext::explicit(fixture.iso.path("state"), fixture.host_socket.clone(), None)
                .unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        let original_descriptor = fs::read(&paths.descriptor_path).unwrap();
        let mut descriptor: Value = serde_json::from_slice(&original_descriptor).unwrap();
        let socket = paths.socket_path;
        let upstream = socket.with_extension("upstream");
        fs::rename(&socket, &upstream).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let meta = fs::metadata(&socket).unwrap();
        descriptor["socket_device"] = json!(meta.dev());
        descriptor["socket_inode"] = json!(meta.ino());
        fs::write(
            &paths.descriptor_path,
            serde_json::to_vec(&descriptor).unwrap(),
        )
        .unwrap();
        let (reached_tx, reached) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let target = upstream.clone();
        let worker = std::thread::spawn(move || -> std::io::Result<()> {
            let mut pages = 0;
            while !stopping.load(Ordering::SeqCst) {
                let Ok((mut client, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                if client.set_nonblocking(false).is_err() {
                    continue;
                }
                client.set_read_timeout(Some(Duration::from_secs(3)))?;
                client.set_write_timeout(Some(Duration::from_secs(3)))?;
                let Ok(request) = read_frame(&mut client) else {
                    continue;
                };
                let parsed: Value =
                    serde_json::from_slice(&request).map_err(std::io::Error::other)?;
                let mut daemon = UnixStream::connect(&target)?;
                daemon.set_read_timeout(Some(Duration::from_secs(3)))?;
                daemon.set_write_timeout(Some(Duration::from_secs(3)))?;
                write_frame(&mut daemon, &request)?;
                let response = read_frame(&mut daemon)?;
                if parsed["command"]["kind"] == "picker_directory" {
                    pages += 1;
                    if pages == 4 {
                        if !parsed["command"]["args"]["page"]["cursor"].is_string() {
                            return Err(std::io::Error::other("held page must be a continuation"));
                        }
                        reached_tx.send(()).map_err(std::io::Error::other)?;
                        let _ = release_rx.recv_timeout(WAIT);
                    }
                }
                let _ = write_frame(&mut client, &response);
            }
            Ok(())
        });
        Self {
            reached,
            release,
            stop,
            worker: Some(worker),
            socket,
            upstream,
            descriptor: paths.descriptor_path,
            original_descriptor,
        }
    }
    fn finish(&mut self) -> Result<(), Vec<String>> {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.release.send(());
        let mut errors = Vec::new();
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("picker forwarder: {error}")),
                Err(_) => errors.push("picker forwarder panicked".into()),
            }
            if let Err(error) = fs::remove_file(&self.socket) {
                errors.push(format!("remove private gate socket: {error}"));
            }
            if let Err(error) = fs::rename(&self.upstream, &self.socket) {
                errors.push(format!("restore private daemon socket: {error}"));
            }
            if let Err(error) = fs::write(&self.descriptor, &self.original_descriptor) {
                errors.push(format!("restore private daemon descriptor: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
    fn wait_until_held(&mut self) -> Result<(), String> {
        self.reached.recv_timeout(WAIT).map_err(|error| {
            let mut detail = format!("refresh page two was not held: {error}");
            if let Err(errors) = self.finish() {
                detail.push_str(&format!("; forwarder/cleanup errors: {errors:?}"));
            }
            detail
        })
    }
}
impl Drop for PageGate {
    fn drop(&mut self) {
        if let Err(errors) = self.finish() {
            for error in errors {
                let _ = writeln!(std::io::stderr(), "private picker gate cleanup: {error}");
            }
        }
    }
}
fn read_frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix)?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len > 1_048_576 {
        return Err(std::io::Error::other("oversized private wire frame"));
    }
    let mut body = vec![0; len];
    stream.read_exact(&mut body)?;
    Ok(body)
}
fn write_frame(stream: &mut UnixStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)
}

struct Pty {
    child: OwnedChild,
    master: File,
    slave: File,
    saved: libc::termios,
    seen: Vec<u8>,
}
impl Pty {
    fn start(mut command: Command, args: &[&str], rows: u16, cols: u16) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: openpty initializes both descriptors, owned by the Files below.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut size,
                )
            },
            0
        );
        // SAFETY: successful openpty returned newly allocated descriptors.
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        // SAFETY: tcgetattr initializes saved using our live slave descriptor.
        let mut saved = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut saved) }, 0);
        command
            .args(args)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()));
        let child = command.spawn_owned().unwrap();
        Self {
            child,
            master,
            slave,
            saved,
            seen: Vec::new(),
        }
    }
    fn read_for(&mut self, duration: Duration) -> Vec<u8> {
        let deadline = Instant::now() + duration;
        let mut out = Vec::new();
        while Instant::now() < deadline {
            let mut fd = libc::pollfd {
                fd: self.master.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let millis = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(100) as i32;
            // SAFETY: fd describes our live master; poll borrows one stack item.
            let ready = unsafe { libc::poll(&mut fd, 1, millis.max(1)) };
            if ready > 0 {
                let mut bytes = [0; 65536];
                match self.master.read(&mut bytes) {
                    Ok(n) if n > 0 => out.extend_from_slice(&bytes[..n]),
                    Ok(_) => break,
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => panic!("read PTY: {e}"),
                }
            }
        }
        self.seen.extend_from_slice(&out);
        out
    }
    fn wait_for(&mut self, needle: &str) -> Vec<u8> {
        self.wait_for_within(needle, WAIT)
    }
    fn wait_for_within(&mut self, needle: &str, duration: Duration) -> Vec<u8> {
        let deadline = Instant::now() + duration;
        let mut out = Vec::new();
        while Instant::now() < deadline {
            out.extend(self.read_for(Duration::from_millis(50)));
            if String::from_utf8_lossy(&out).contains(needle) {
                return out;
            }
            if self.child.try_wait().unwrap().is_some() {
                // The child can print the needle and exit after the read
                // above; its output stays queued on the master (the slave is
                // still open, so there is no EOF). Drain it before deciding.
                let drain_until = Instant::now() + Duration::from_secs(5);
                loop {
                    let more = self.read_for(Duration::from_millis(20));
                    if more.is_empty() {
                        break;
                    }
                    out.extend(more);
                    assert!(Instant::now() < drain_until, "CLI output never drained");
                }
                assert!(
                    String::from_utf8_lossy(&out).contains(needle),
                    "CLI exited before {needle:?}: {}",
                    String::from_utf8_lossy(&self.seen)
                );
                return out;
            }
        }
        panic!(
            "CLI did not print {needle:?}: {}",
            String::from_utf8_lossy(&self.seen)
        );
    }
    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }
    fn resize(&self, rows: u16, cols: u16) {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: ioctl updates only our slave's terminal dimensions.
        assert_eq!(
            unsafe { libc::ioctl(self.slave.as_raw_fd(), libc::TIOCSWINSZ, &size) },
            0
        );
        // No controlling terminal is needed: deliver its normal resize signal explicitly.
        // SAFETY: signal targets only our owned CLI child.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGWINCH);
        }
    }
    fn exit_status(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            self.read_for(Duration::from_millis(20));
            if let Some(status) = self.child.try_wait().unwrap() {
                // The child can write and exit after the last poll above; its
                // output stays queued on the master (the slave is still open,
                // so there is no EOF). Drain it before callers inspect `seen`.
                while !self.read_for(Duration::from_millis(20)).is_empty() {
                    assert!(Instant::now() < deadline, "CLI output never drained");
                }
                return status;
            }
            assert!(Instant::now() < deadline, "CLI ignored cancellation");
        }
    }
    fn exit(&mut self) {
        let status = self.exit_status();
        assert_eq!(
            status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&self.seen)
        );
        self.assert_restored();
    }
    fn assert_restored(&self) {
        // SAFETY: tcgetattr initializes current on our still-open slave.
        let mut current: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(
            unsafe { libc::tcgetattr(self.slave.as_raw_fd(), &mut current) },
            0
        );
        assert_eq!(
            // Returning to canonical mode can set the driver's pending-input
            // reprocessing bit; compare modes, excluding that transient state.
            current.c_lflag & !libc::PENDIN,
            self.saved.c_lflag & !libc::PENDIN,
            "restore canonical/echo/signal flags"
        );
        assert_eq!(current.c_iflag, self.saved.c_iflag, "restore input flags");
        assert_eq!(current.c_oflag, self.saved.c_oflag, "restore output flags");
        assert_eq!(current.c_cflag, self.saved.c_cflag, "restore control flags");
        assert_eq!(current.c_cc, self.saved.c_cc, "restore control characters");
    }
}

fn bounded_output(mut command: Command) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_owned()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        // SAFETY: both descriptors are live owned child pipes; nonblocking
        // reads let the same deadline bound process exit and output draining.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
    }
    let deadline = Instant::now() + WAIT;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    loop {
        drain_pipe(&mut stdout, &mut out);
        drain_pipe(&mut stderr, &mut err);
        if let Some(status) = child.try_wait().unwrap() {
            drain_pipe(&mut stdout, &mut out);
            drain_pipe(&mut stderr, &mut err);
            return Output {
                status,
                stdout: out,
                stderr: err,
            };
        }
        assert!(
            Instant::now() < deadline,
            "child exceeded output/exit deadline: {}",
            String::from_utf8_lossy(&err)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn drain_pipe(pipe: &mut impl Read, output: &mut Vec<u8>) {
    let mut bytes = [0; 4096];
    loop {
        match pipe.read(&mut bytes) {
            Ok(0) => return,
            Ok(n) => {
                output.extend_from_slice(&bytes[..n]);
                assert!(output.len() <= 1_048_576, "bounded captured test output");
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => panic!("read owned child pipe: {error}"),
        }
    }
}

fn no_erase(bytes: &[u8]) {
    assert!(
        !bytes.windows(4).any(|b| b == b"\x1b[2J"),
        "interaction must not erase the whole screen: {}",
        String::from_utf8_lossy(bytes)
    );
}

fn has_sgr(bytes: &[u8]) -> bool {
    bytes.windows(2).enumerate().any(|(i, prefix)| {
        prefix == b"\x1b["
            && bytes[i + 2..]
                .iter()
                .copied()
                .find(|b| (0x40..=0x7e).contains(b))
                == Some(b'm')
    })
}

fn visible_runs_fit(bytes: &[u8], columns: usize) {
    let mut visible = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 27 || bytes[i] == b'\r' || bytes[i] == b'\n' {
            if bytes[i] == 27 && bytes.get(i + 1) == Some(&b'[') {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'm') {
                    i += 1;
                    continue;
                }
            }
            let text = std::str::from_utf8(&visible).expect("never split UTF-8 in a frame");
            assert!(
                unicode_width::UnicodeWidthStr::width(text) <= columns,
                "overwide terminal run: {text:?}"
            );
            visible.clear();
        } else {
            visible.push(bytes[i]);
        }
        i += 1;
    }
    let text = std::str::from_utf8(&visible).expect("never split UTF-8 in a frame");
    assert!(
        unicode_width::UnicodeWidthStr::width(text) <= columns,
        "overwide terminal run: {text:?}"
    );
}

/// Kills unconditional repainting on the 100ms input poll, full-screen clears
/// on navigation/filter/resize, and failure to restore raw mode on Ctrl-C.
#[test]
fn settled_picker_emits_no_idle_bytes_and_updates_without_erase_all() {
    let fixture = Fixture::new();
    fixture.thread("tPtyActive", "界界界 active channel", 1, false);
    fixture.thread("tPtyArchived", "archived channel", 0, true);
    let mut pty = Pty::start(fixture.command(), &["read", "--follow", "--human"], 24, 180);
    let initial = pty.wait_for("complete");
    assert!(
        has_sgr(&initial),
        "supporting terminal gets selection/status color"
    );
    let text = String::from_utf8_lossy(&initial);
    assert!(
        text.contains("[active]") && text.contains("[archived]"),
        "status stays explicit: {text}"
    );
    pty.read_for(Duration::from_millis(150));
    let idle = pty.read_for(Duration::from_millis(550));
    assert!(
        idle.is_empty(),
        "settled idle emitted {} bytes ({} erase-all sequences)",
        idle.len(),
        idle.windows(4).filter(|b| *b == b"\x1b[2J").count()
    );
    pty.send(b"\x1b[B");
    let navigation = pty.wait_for("> [archived]");
    no_erase(&navigation);
    pty.send(b"zzzznomatch");
    let filtered = pty.wait_for("0 matches");
    no_erase(&filtered);
    assert!(String::from_utf8_lossy(&filtered).contains("0 matches"));
    pty.send(&[127; 11]);
    pty.wait_for("2 matches");
    pty.resize(8, 18);
    let unicode_small = pty.wait_for("界");
    no_erase(&unicode_small);
    visible_runs_fit(&unicode_small, 17);
    pty.resize(4, 12);
    let small = pty.wait_for("Channels");
    no_erase(&small);
    visible_runs_fit(&small, 11);
    pty.resize(0, 0);
    no_erase(&pty.read_for(Duration::from_millis(150)));
    pty.send(b"\x03");
    pty.exit();
}

/// Kills omission of explicit status/participants/latest body, terminal control
/// injection in previews, and color output despite an explicit NO_COLOR.
#[test]
fn picker_metadata_is_escaped_and_no_color_keeps_readable_labels() {
    let fixture = Fixture::new();
    fixture.thread("tPtyPreview", "unicode 界🌿", 1, false);
    fixture.message("tPtyPreview", "last\n\x1b[2J\t界🌿", 1);
    let mut plain = fixture.command();
    plain.env("NO_COLOR", "1");
    let mut pty = Pty::start(plain, &["read", "--human"], 24, 220);
    let initial = pty.wait_for("complete");
    let text = String::from_utf8(initial).unwrap();
    assert!(
        text.contains("active") && text.contains("0 participants"),
        "{text}"
    );
    assert!(
        text.contains("last\\n\\u{001b}[2J\\t界🌿"),
        "escaped last preview: {text}"
    );
    assert!(!has_sgr(text.as_bytes()), "NO_COLOR has no SGR: {text}");
    pty.send("界🌿".as_bytes());
    let query = pty.wait_for("> 界🌿");
    assert!(String::from_utf8(query).unwrap().contains("界🌿"));
    pty.send(b"\x1b");
    pty.exit();
    let all = String::from_utf8(pty.seen).unwrap();
    assert!(
        !has_sgr(all.as_bytes()),
        "NO_COLOR includes terminal teardown"
    );
    let mut mono = fixture.command();
    mono.env("TERM", "vt100");
    let mut monochrome = Pty::start(mono, &["read", "--human"], 24, 220);
    let mono_initial = monochrome.wait_for("complete");
    assert!(
        !has_sgr(&mono_initial),
        "unsupported color terminal remains plain"
    );
    assert!(String::from_utf8_lossy(&mono_initial).contains("[active]"));
    monochrome.send(b"\x1b");
    monochrome.exit();
    assert!(!has_sgr(&monochrome.seen));
}

/// Kills blocking socket/capability calls on the UI thread. A paused real
/// private daemon keeps its socket alive but cannot answer; cancellation must
/// interrupt the pending fetch instead of waiting its five-second deadline.
#[test]
fn pending_daemon_response_does_not_delay_escape_or_control_c() {
    let fixture = Fixture::new();
    fixture.thread("tPtyDelayed", "delayed channel", 0, false);
    let _paused = PausedDaemon::new(&fixture);
    for cancel in [b"\x1b".as_slice(), b"\x03".as_slice()] {
        let mut pty = Pty::start(fixture.command(), &["read", "--human"], 24, 160);
        pty.wait_for("loading");
        let started = Instant::now();
        pty.send(cancel);
        pty.exit();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancellation waited for pending daemon response: {:?}",
            started.elapsed()
        );
    }
}

/// Kills choosing a different channel after a refresh changes ranking, dropping
/// channels past page one, and incidental receipt ACK/invitation acceptance.
#[test]
fn paged_picker_preserves_canonical_selection_on_refresh_and_read_is_read_only() {
    let fixture = Fixture::new();
    fixture.threads("tPty", 125);
    fixture.thread("tPtyKeeper", "keeper", -1, false);
    fixture.message("tPtyKeeper", "canonical keeper history", 1);
    fixture.obligations("tPtyKeeper");
    let before = fixture.obligation_states();
    assert_eq!(
        before,
        ("pending".into(), "pending".into(), "invited".into())
    );
    let mut pty = Pty::start(fixture.command(), &["read", "--human"], 24, 180);
    let initial = pty.wait_for("complete");
    assert!(String::from_utf8_lossy(&initial).contains("126 loaded"));
    pty.send(b"keeper");
    pty.wait_for("tPtyKeeper");
    fixture.thread("tPtyChallenger", "aaa keeper challenger", 999999, false);
    fixture.message("tPtyKeeper", "refreshed keeper marker", 2);
    let refreshed = pty.wait_for("refreshed keeper marker");
    no_erase(&refreshed);
    pty.send(b"\r");
    pty.wait_for("canonical keeper history");
    pty.exit();
    assert_eq!(fixture.obligation_states(), before);
}

/// Kills clearing or partially replacing the visible snapshot while refreshing:
/// Enter during a held page-two response must keep the selected late-page ID.
#[test]
fn enter_during_partial_refresh_keeps_the_selected_late_page_thread() {
    let fixture = Fixture::new();
    fixture.threads("tPtyPage", 125);
    fixture.thread("tPtyLate", "late keeper", -1, false);
    fixture.message("tPtyLate", "late canonical history", 1);
    let mut gate = PageGate::new(&fixture);
    let mut pty = Pty::start(fixture.command(), &["read", "--human"], 24, 180);
    pty.wait_for("126 loaded — complete");
    pty.send(b"late keeper");
    pty.wait_for("tPtyLate");
    fixture
        .db()
        .execute("UPDATE threads SET archived=1 WHERE id='tPtyLate'", [])
        .unwrap();
    gate.wait_until_held().expect("refresh reached page two");
    pty.send(b"\r");
    // Leave the picker and restore termios while page two is still held.
    // Releasing beforehand would let a blocked UI pass this regression.
    pty.wait_for_within("\x1b[?1049l", Duration::from_millis(750));
    pty.assert_restored();
    gate.release.send(()).unwrap();
    pty.wait_for("late canonical history");
    pty.exit();
    gate.finish()
        .expect("forwarder and socket restoration succeeded");
}

/// Kills a cleanup panic that aborts the combined process while an assertion is
/// already unwinding. Re-exec isolates the deliberate double-panic regression.
#[test]
fn gate_worker_panic_during_unwind_restores_private_socket_and_daemon_cleanup() {
    const FAULT_ENV: &str = "HT_PICKER_GATE_UNWIND_PROBE";
    if std::env::var_os(FAULT_ENV).is_none() {
        let iso = TestIsolation::new("picker-gate-unwind");
        let mut command = iso.command(std::env::current_exe().unwrap());
        command.args(["read_picker_pty::gate_worker_panic_during_unwind_restores_private_socket_and_daemon_cleanup", "--exact", "--nocapture"])
            .env("CLAUDE_CONFIG_DIR", iso.path("claude"))
            .env("CODEX_HOME", iso.path("codex"))
            .env(FAULT_ENV, "1");
        let output = bounded_output(command);
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed"),
            "cleanup probe aborted or failed: {:?}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let fixture = Fixture::new();
    let context =
        RuntimeContext::explicit(fixture.iso.path("state"), fixture.host_socket.clone(), None)
            .unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let descriptor = fs::read(&paths.descriptor_path).unwrap();
    let pid = fixture.daemon.id() as libc::pid_t;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut gate = PageGate::new(&fixture);
        // Replace the stopped real forwarder with a worker that fails exactly
        // when Drop joins it; no extra fault path enters production code.
        gate.stop.store(true, Ordering::SeqCst);
        gate.release.send(()).unwrap();
        gate.worker.take().unwrap().join().unwrap().unwrap();
        gate.worker = Some(std::thread::spawn(|| -> std::io::Result<()> {
            panic!("injected forwarding worker panic");
        }));
        panic!("injected assertion unwind with live PageGate");
    }));
    assert!(outcome.is_err(), "the assertion fault must unwind");
    assert_eq!(fs::read(&paths.descriptor_path).unwrap(), descriptor);
    herdr_threads::daemon::ownership::read_descriptor(&paths, fixture.instance.parse().unwrap())
        .expect("private socket identity restored after worker panic");
    let mut command = fixture.command();
    command.args(["daemon", "health"]);
    assert!(
        bounded_output(command).status.success(),
        "restored daemon endpoint is usable"
    );
    drop(fixture);
    // SAFETY: inspect only the pid of the private daemon whose owner was dropped.
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "owned daemon stopped after failed worker cleanup"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

/// Kills routing bare follow around the picker, selecting a display label
/// instead of an exact ID, and follow implicitly acknowledging obligations.
#[test]
fn bare_follow_selects_canonical_thread_and_cancels_cleanly() {
    let fixture = Fixture::new();
    fixture.thread("tPtyFollow", "follow channel", 0, false);
    fixture.message("tPtyFollow", "followed canonical body", 1);
    fixture.obligations("tPtyFollow");
    let before = fixture.obligation_states();
    let mut cancelled = Pty::start(fixture.command(), &["follow", "--human"], 24, 160);
    cancelled.wait_for("complete");
    cancelled.send(b"\x1b");
    cancelled.exit();
    let mut selected = Pty::start(fixture.command(), &["follow", "--human"], 24, 160);
    selected.wait_for("complete");
    selected.send(b"\r");
    selected.wait_for("followed canonical body");
    // Follow restores termios before streaming; SIGINT is its normal exit path.
    // SAFETY: signal targets only this owned follower.
    unsafe {
        libc::kill(selected.child.id() as libc::pid_t, libc::SIGINT);
    }
    selected.exit();
    assert_eq!(fixture.obligation_states(), before);
}

/// Kills terminal eligibility regressions and selecting a picker for machine
/// requests; empty stores exit successfully without leaving terminal raw.
#[test]
fn empty_picker_and_terminal_machine_boundaries_exit_without_raw_mode() {
    let fixture = Fixture::new();
    let mut empty = Pty::start(fixture.command(), &["read", "--human"], 24, 120);
    empty.exit();
    assert!(String::from_utf8_lossy(&empty.seen).contains("No channels"));
    for args in [&["read", "--human"][..], &["follow", "--human"][..]] {
        let mut command = fixture.command();
        command.args(args);
        let out = bounded_output(command);
        assert!(
            !out.status.success(),
            "redirected human request must refuse: {args:?}"
        );
        assert!(
            !out.stdout
                .windows(8)
                .chain(out.stderr.windows(8))
                .any(|b| b == b"\x1b[?1049h")
        );
    }
    for args in [
        &["read", "--machine"][..],
        &["--json", "read"][..],
        &["follow", "--machine"][..],
        &["--json", "follow"][..],
    ] {
        let mut machine = Pty::start(fixture.command(), args, 24, 120);
        assert!(
            !machine.exit_status().success(),
            "bare machine request must refuse on a PTY: {args:?}"
        );
        assert!(!machine.seen.windows(8).any(|b| b == b"\x1b[?1049h"));
        machine.assert_restored();
    }
    let mut dumb = fixture.command();
    dumb.env("TERM", "dumb");
    let mut unsupported = Pty::start(dumb, &["read", "--human"], 24, 120);
    assert!(!unsupported.exit_status().success());
    assert!(!unsupported.seen.windows(8).any(|b| b == b"\x1b[?1049h"));
}
