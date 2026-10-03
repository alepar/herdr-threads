#![cfg(target_os = "macos")]
use herdr_threads::host::continuity::KernelProcessInfo;
use herdr_threads::host::continuity::{
    CaptureError, ProcessInfo, ProcessInfoProvider, capture_peer_witness, capture_peer_witness_with,
};
use herdr_threads::test_support::spawn::SpawnOwned;
use std::{os::unix::net::UnixListener, path::PathBuf};

struct Endpoint(PathBuf);
impl Endpoint {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ht-witness-{}", uuid::Uuid::new_v4()));
        eprintln!("owned endpoint generated: {}", path.display());
        Self(path)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.0) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("owned endpoint cleanup: {e}"),
        }
        assert!(!self.0.exists());
        eprintln!("owned endpoint removed: {}", self.0.display());
    }
}

#[test]
fn actual_peer_start_and_same_process_reconnect_are_captured() {
    let endpoint = Endpoint::new();
    let listener = UnixListener::bind(&endpoint.0).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    runtime.block_on(async {
        let a = tokio::net::UnixStream::connect(&endpoint.0).await.unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let first = capture_peer_witness(&a, &endpoint.0).unwrap();
        assert_eq!(first.peer_pid, std::process::id());
        assert_eq!(first.peer_uid, unsafe { libc::getuid() });
        assert!(first.start_seconds > 0);
        assert!(first.start_microseconds < 1_000_000);
        drop(accepted);
        drop(a);
        let b = tokio::net::UnixStream::connect(&endpoint.0).await.unwrap();
        let (_accepted, _) = listener.accept().unwrap();
        assert_eq!(first, capture_peer_witness(&b, &endpoint.0).unwrap());
        eprintln!("native witness: {first:?}");
    });
    drop(listener);
    std::fs::remove_file(&endpoint.0).unwrap();
    assert!(!endpoint.0.exists());
}

struct Provider(Result<ProcessInfo, CaptureError>);
impl ProcessInfoProvider for Provider {
    fn process_info(&self, _pid: u32) -> Result<ProcessInfo, CaptureError> {
        self.0.clone()
    }
}
#[test]
fn missing_denied_short_and_invalid_process_data_reject() {
    let endpoint = Endpoint::new();
    let listener = UnixListener::bind(&endpoint.0).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    runtime.block_on(async {
        let stream = tokio::net::UnixStream::connect(&endpoint.0).await.unwrap();
        let (_accepted, _) = listener.accept().unwrap();
        let pid = std::process::id();
        let uid = unsafe { libc::getuid() };
        for info in [
            Err(CaptureError::ProcessUnavailable),
            Err(CaptureError::ProcessDenied),
            Err(CaptureError::ShortProcessInfo),
            Ok(ProcessInfo {
                pid: 0,
                uid,
                start_seconds: 1,
                start_microseconds: 0,
            }),
            Ok(ProcessInfo {
                pid,
                uid: uid + 1,
                start_seconds: 1,
                start_microseconds: 0,
            }),
            Ok(ProcessInfo {
                pid,
                uid,
                start_seconds: 0,
                start_microseconds: 0,
            }),
            Ok(ProcessInfo {
                pid,
                uid,
                start_seconds: 1,
                start_microseconds: 1_000_000,
            }),
            Ok(ProcessInfo {
                pid,
                uid,
                start_seconds: u64::MAX,
                start_microseconds: 0,
            }),
        ] {
            assert!(capture_peer_witness_with(&stream, &endpoint.0, &Provider(info)).is_err());
        }
        let first = capture_peer_witness_with(
            &stream,
            &endpoint.0,
            &Provider(Ok(ProcessInfo {
                pid,
                uid,
                start_seconds: 1,
                start_microseconds: 1,
            })),
        )
        .unwrap();
        let reused = capture_peer_witness_with(
            &stream,
            &endpoint.0,
            &Provider(Ok(ProcessInfo {
                pid,
                uid,
                start_seconds: 1,
                start_microseconds: 2,
            })),
        )
        .unwrap();
        assert_ne!(
            first, reused,
            "synthetic same PID different start must differ"
        );
    });
}

use herdr_threads::{
    host::request_witnessed,
    protocol::{
        results::ErrorCode,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
};
use std::{
    io::{BufRead, BufReader, Write},
    time::{Duration, Instant},
};
struct TestClock(Instant);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.elapsed().as_millis() as u64)
    }
}
struct Worker(Option<std::thread::JoinHandle<()>>);
impl Worker {
    fn join(mut self) -> std::thread::Result<()> {
        self.0.take().unwrap().join()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(worker) = self.0.take() {
            let _ = worker.join();
        }
    }
}
fn transport_fixture(rebind: bool) -> (Endpoint, Worker) {
    let endpoint = Endpoint::new();
    let path = endpoint.0.clone();
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let worker = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(2);
        let mut listener = listener;
        for index in 0..2 {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(stream) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until =>
                    {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(_) => return,
                }
            };
            // macOS accept(2) inherits O_NONBLOCK from the listener; a read
            // that races ahead of the client's write would fail WouldBlock.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let request: serde_json::Value = serde_json::from_str(&line).unwrap();
            let body = if index == 0 {
                serde_json::json!({"type":"pong","protocol":22,"version":"0.9.1"})
            } else {
                serde_json::json!({"type":"pane_info"})
            };
            if rebind && index == 1 {
                // Keep the old listener alive so its inode cannot be recycled.
                std::fs::remove_file(&path).unwrap();
                let replacement = UnixListener::bind(&path).unwrap();
                listener = replacement;
            }
            writeln!(
                stream,
                "{}",
                serde_json::json!({"id":request["id"],"result":body})
            )
            .unwrap();
        }
    });
    (endpoint, Worker(Some(worker)))
}
#[test]
fn witnessed_transport_preserves_body_and_rejects_same_process_path_rebind() {
    for rebind in [false, true] {
        let (endpoint, worker) = transport_fixture(rebind);
        let clock = TestClock(Instant::now());
        let budget = CallBudget {
            deadline: MonoInstant(2000),
            cancellation: Cancellation::default(),
        };
        let result = request_witnessed(
            &endpoint.0,
            "private",
            "pane.get",
            serde_json::json!({}),
            &clock,
            &budget,
            Duration::from_secs(2),
        );
        if rebind {
            assert_eq!(result.unwrap_err().code, ErrorCode::StaleHostObservation);
        } else {
            let response = result.unwrap();
            assert_eq!(response.witness.peer_pid, std::process::id());
            let body: serde_json::Value = serde_json::from_str(&response.body).unwrap();
            assert_eq!(body["id"], "private");
            assert_eq!(body["result"]["type"], "pane_info");
        }
        worker.join().unwrap();
    }
}

struct PrivateProcess {
    child: herdr_threads::test_support::spawn::OwnedChild,
    reaped: bool,
    output: Vec<u8>,
    request: Option<
        std::thread::JoinHandle<
            Result<
                herdr_threads::host::WitnessedResponse,
                herdr_threads::protocol::results::ApiError,
            >,
        >,
    >,
}
#[derive(Debug, PartialEq, Eq)]
enum WaitFailure {
    Timeout,
    Eof,
    RequestFinished,
}
impl PrivateProcess {
    fn new(child: herdr_threads::test_support::spawn::OwnedChild) -> Self {
        use std::os::fd::AsRawFd;
        let owned = Self {
            child,
            reaped: false,
            output: Vec::new(),
            request: None,
        };
        let fd = owned.child.stdout.as_ref().unwrap().as_raw_fd();
        // Only the owned fixture's notification pipe is made nonblocking.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        eprintln!("owned fixture PID {} spawned", owned.child.id());
        owned
    }
    fn spawn_code(code: &str, path: &std::path::Path) -> Self {
        let child = std::process::Command::new("python3")
            .args(["-u", "-c", code])
            .arg(path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn_owned()
            .unwrap();
        Self::new(child)
    }
    fn bind(path: &std::path::Path) -> Self {
        let code = "import socket,sys,time\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen(8)\nprint('ready',flush=True)\nwhile True:\n c,_=s.accept()\n print('connected',flush=True)\n time.sleep(30)\n";
        let mut owned = Self::spawn_code(code, path);
        owned
            .wait_line("ready", Instant::now() + Duration::from_secs(2))
            .unwrap();
        owned
    }
    fn wait_line(&mut self, expected: &str, deadline: Instant) -> Result<(), WaitFailure> {
        use std::io::Read;
        loop {
            if self
                .request
                .as_ref()
                .is_some_and(|request| request.is_finished())
            {
                return Err(WaitFailure::RequestFinished);
            }
            if Instant::now() >= deadline {
                return Err(WaitFailure::Timeout);
            }
            if let Some(end) = self.output.iter().position(|byte| *byte == b'\n') {
                let line: Vec<_> = self.output.drain(..=end).collect();
                assert_eq!(std::str::from_utf8(&line).unwrap().trim(), expected);
                return Ok(());
            }
            let mut bytes = [0; 128];
            match self.child.stdout.as_mut().unwrap().read(&mut bytes) {
                Ok(0) => return Err(WaitFailure::Eof),
                Ok(count) => {
                    self.output.extend_from_slice(&bytes[..count]);
                    assert!(self.output.len() <= 1024);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => panic!("owned notification pipe: {e}"),
            }
        }
    }
    fn start_request(&mut self, path: &std::path::Path, cancellation: Cancellation) {
        assert!(self.request.is_none());
        let path = path.to_path_buf();
        self.request = Some(std::thread::spawn(move || {
            let clock = TestClock(Instant::now());
            let budget = CallBudget {
                deadline: MonoInstant(2000),
                cancellation,
            };
            request_witnessed(
                &path,
                "replace",
                "pane.get",
                serde_json::json!({}),
                &clock,
                &budget,
                Duration::from_secs(2),
            )
        }));
    }
    fn request_result(
        &mut self,
    ) -> Result<herdr_threads::host::WitnessedResponse, herdr_threads::protocol::results::ApiError>
    {
        self.request.take().unwrap().join().unwrap()
    }
    fn stop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let status = self.child.wait().unwrap();
            self.reaped = true;
            eprintln!("owned fixture PID {} reaped: {status}", self.child.id());
        }
        // Killing/reaping the pipe/socket owner precedes every dependent join,
        // including unwinding, timeout, early rejection and EOF.
        if let Some(request) = self.request.take() {
            let _ = request.join();
            eprintln!(
                "owned fixture PID {} dependent request joined after reap",
                self.child.id()
            );
        }
    }
}
impl Drop for PrivateProcess {
    fn drop(&mut self) {
        self.stop();
    }
}
#[test]
fn owned_process_exit_and_new_process_rebind_reject_old_identity() {
    let endpoint = Endpoint::new();
    let mut process = PrivateProcess::bind(&endpoint.0);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    runtime.block_on(async {
        let old = tokio::net::UnixStream::connect(&endpoint.0).await.unwrap();
        let witness = capture_peer_witness(&old, &endpoint.0).unwrap();
        assert_eq!(witness.peer_pid, process.child.id());
        assert_ne!(witness.peer_pid, std::process::id());
        process.stop();
        assert!(KernelProcessInfo.process_info(witness.peer_pid).is_err());
        assert!(capture_peer_witness(&old, &endpoint.0).is_err());
        std::fs::remove_file(&endpoint.0).unwrap();
        let mut replacement = PrivateProcess::bind(&endpoint.0);
        let new = tokio::net::UnixStream::connect(&endpoint.0).await.unwrap();
        let next = capture_peer_witness(&new, &endpoint.0).unwrap();
        assert_eq!(next.peer_pid, replacement.child.id());
        assert_ne!(witness, next);
        assert!(capture_peer_witness(&old, &endpoint.0).is_err());
        eprintln!("native old/new process witnesses: {witness:?} / {next:?}");
        replacement.stop();
    });
    std::fs::remove_file(&endpoint.0).unwrap();
    assert!(!endpoint.0.exists());
}

#[test]
fn actual_new_process_replacement_during_ping_cannot_publish_old_witness() {
    let endpoint = Endpoint::new();
    let code = "import socket,sys,json,time\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen()\nprint('ready',flush=True)\nc,_=s.accept()\nr=json.loads(c.makefile().readline())\nprint('accepted',flush=True)\nsys.stdin.readline()\nc.sendall((json.dumps({'id':r['id'],'result':{'type':'pong','version':'0.9.1','protocol':22}})+'\\n').encode())\nc.close()\ntime.sleep(30)\n";
    let child = std::process::Command::new("python3")
        .args(["-u", "-c", code])
        .arg(&endpoint.0)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn_owned()
        .unwrap();
    let mut old = PrivateProcess::new(child);
    old.wait_line("ready", Instant::now() + Duration::from_secs(2))
        .unwrap();
    old.start_request(&endpoint.0, Cancellation::default());
    old.wait_line("accepted", Instant::now() + Duration::from_secs(2))
        .unwrap();
    std::fs::remove_file(&endpoint.0).unwrap();
    let mut replacement = PrivateProcess::bind(&endpoint.0);
    writeln!(old.child.stdin.as_mut().unwrap(), "continue").unwrap();
    assert_eq!(
        old.request_result().unwrap_err().code,
        ErrorCode::StaleHostObservation
    );
    old.stop();
    replacement.stop();
    std::fs::remove_file(&endpoint.0).unwrap();
    assert!(!endpoint.0.exists());
}

#[test]
fn fix1_withheld_readiness_returns_finite_timeout_and_cleans_child() {
    let endpoint = Endpoint::new();
    let code = "import socket,sys,time\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen()\ntime.sleep(.25)\nprint('ready',flush=True)\ntime.sleep(30)\n";
    let child = std::process::Command::new("python3")
        .args(["-u", "-c", code])
        .arg(&endpoint.0)
        .stdout(std::process::Stdio::piped())
        .spawn_owned()
        .unwrap();
    let mut owned = PrivateProcess::new(child);
    let started = Instant::now();
    let result = owned.wait_line("ready", started + Duration::from_millis(80));
    assert_eq!(result, Err(WaitFailure::Timeout));
    assert!(started.elapsed() < Duration::from_millis(200));
    let pid = owned.child.id();
    drop(owned);
    assert!(KernelProcessInfo.process_info(pid).is_err());
}

#[test]
fn fix1_unwind_stops_child_before_dependent_request_join() {
    let endpoint = Endpoint::new();
    let code = "import socket,sys,json,time\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen()\nprint('ready',flush=True)\nc,_=s.accept()\nc.makefile().readline()\nprint('accepted',flush=True)\nsys.stdin.readline()\ntime.sleep(30)\n";
    let started = Instant::now();
    let mut pid = 0;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let child = std::process::Command::new("python3")
            .args(["-u", "-c", code])
            .arg(&endpoint.0)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn_owned()
            .unwrap();
        let mut old = PrivateProcess::new(child);
        pid = old.child.id();
        old.wait_line("ready", Instant::now() + Duration::from_secs(2))
            .unwrap();
        old.start_request(&endpoint.0, Cancellation::default());
        old.wait_line("accepted", Instant::now() + Duration::from_secs(2))
            .unwrap();
        panic!("controlled fixture assertion unwind");
    }));
    assert_eq!(
        panic.unwrap_err().downcast_ref::<&str>(),
        Some(&"controlled fixture assertion unwind")
    );
    assert!(KernelProcessInfo.process_info(pid).is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "child must stop before request join, elapsed {:?}",
        started.elapsed()
    );
}

#[test]
fn fix1_withheld_accepted_early_request_and_eof_reach_cleanup() {
    for mode in [0, 1, 2] {
        let endpoint = Endpoint::new();
        let code = if mode == 2 {
            "import socket,sys\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen()\nprint('ready',flush=True)\n"
        } else {
            "import socket,sys,time\ns=socket.socket(socket.AF_UNIX)\ns.bind(sys.argv[1])\ns.listen()\nprint('ready',flush=True)\nc,_=s.accept()\nc.makefile().readline()\ntime.sleep(30)\n"
        };
        let mut owned = PrivateProcess::spawn_code(code, &endpoint.0);
        owned
            .wait_line("ready", Instant::now() + Duration::from_secs(2))
            .unwrap();
        let pid = owned.child.id();
        let started = Instant::now();
        if mode != 2 {
            let cancellation = Cancellation::default();
            if mode == 1 {
                cancellation.cancel();
            }
            owned.start_request(&endpoint.0, cancellation);
        }
        let waited = owned.wait_line("accepted", Instant::now() + Duration::from_millis(100));
        assert_eq!(
            waited,
            Err(if mode == 0 {
                WaitFailure::Timeout
            } else if mode == 1 {
                WaitFailure::RequestFinished
            } else {
                WaitFailure::Eof
            })
        );
        if mode == 1 {
            assert_eq!(
                owned.request_result().unwrap_err().code,
                ErrorCode::Cancelled
            );
        }
        drop(owned);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(KernelProcessInfo.process_info(pid).is_err());
        eprintln!(
            "fix1 failure cleanup mode={mode}, elapsed={:?}",
            started.elapsed()
        );
    }
}
