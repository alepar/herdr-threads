use super::*;
use crate::protocol::time::{Cancellation, MonoInstant, UtcMillis};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
/// Budgets in these tests are driven only by `InjectedClock`; the wall-clock
/// transport limit is set far beyond any scheduling delay so CPU load cannot
/// turn an expected outcome into `DeadlineExceeded`.
const UNREACHED_WALL_LIMIT: Duration = Duration::from_secs(3600);
/// Safety net only: a fixture read that waits this long indicates a hung
/// client, never an expected outcome.
const FIXTURE_SAFETY_READ_TIMEOUT: Duration = Duration::from_secs(60);
struct Fixture {
    path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    requests: Arc<AtomicUsize>,
    accepted: Arc<AtomicUsize>,
}
impl Fixture {
    /// `late_clock`: before writing any response, move the injected monotonic
    /// clock to the given value (a deterministic "late response").
    fn new(rebind_after_ping: bool, late_clock: Option<(Arc<AtomicU64>, u64)>) -> Self {
        let path = std::env::temp_dir().join(format!("ht-witness-{}", uuid::Uuid::new_v4()));
        eprintln!("owned transport endpoint created: {}", path.display());
        let mut listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let owned_stop = stop.clone();
        let owned_path = path.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let owned_requests = requests.clone();
        let accepted = Arc::new(AtomicUsize::new(0));
        let owned_accepted = accepted.clone();
        let worker = std::thread::spawn(move || {
            let mut index = 0;
            let mut old_listener = None;
            while !owned_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        owned_accepted.fetch_add(1, Ordering::AcqRel);
                        // macOS accept(2) inherits O_NONBLOCK from the listener.
                        // A nonblocking read that races ahead of the client's
                        // write would see WouldBlock and drop the connection.
                        stream.set_nonblocking(false).unwrap();
                        // macOS setsockopt fails with EINVAL once the peer has
                        // closed: that client never wrote a request.
                        if stream
                            .set_read_timeout(Some(FIXTURE_SAFETY_READ_TIMEOUT))
                            .is_err()
                        {
                            continue;
                        }
                        let mut line = String::new();
                        // Zero bytes: the client closed before writing (a
                        // capture error before the request), not a request.
                        if BufReader::new(stream.try_clone().unwrap())
                            .read_line(&mut line)
                            .unwrap()
                            == 0
                        {
                            continue;
                        }
                        let wire: Value = serde_json::from_str(&line).unwrap();
                        owned_requests.fetch_add(1, Ordering::AcqRel);
                        let result = if index == 0 {
                            json!({"type":"pong","protocol":22,"version":"0.9.1"})
                        } else {
                            json!({"type":"pane_info"})
                        };
                        if index == 0 && rebind_after_ping {
                            std::fs::remove_file(&owned_path).unwrap();
                            let replacement = UnixListener::bind(&owned_path).unwrap();
                            replacement.set_nonblocking(true).unwrap();
                            old_listener = Some(std::mem::replace(&mut listener, replacement));
                        }
                        if let Some((clock, value)) = &late_clock {
                            clock.store(*value, Ordering::Release);
                        }
                        let _ = writeln!(stream, "{}", json!({"id":wire["id"],"result":result}));
                        index += 1;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                }
            }
            drop(old_listener);
        });
        Self {
            path,
            stop,
            worker: Some(worker),
            requests,
            accepted,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let joined = self.worker.take().unwrap().join();
        std::fs::remove_file(&self.path).unwrap();
        assert!(!self.path.exists());
        eprintln!(
            "owned transport worker joined; endpoint removed: {}",
            self.path.display()
        );
        if !std::thread::panicking() {
            joined.unwrap();
        }
    }
}
struct ChangingProvider {
    calls: AtomicUsize,
    change_at: usize,
    unavailable: bool,
}
impl ProcessInfoProvider for ChangingProvider {
    fn process_info(
        &self,
        pid: u32,
    ) -> Result<super::super::continuity::ProcessInfo, super::super::continuity::CaptureError> {
        let count = self.calls.fetch_add(1, Ordering::AcqRel);
        if count >= self.change_at && self.unavailable {
            return Err(super::super::continuity::CaptureError::ProcessUnavailable);
        }
        let mut info = KernelProcessInfo.process_info(pid)?;
        if count >= self.change_at {
            info.start_seconds += 1;
        }
        Ok(info)
    }
}
#[test]
fn synthetic_reuse_before_after_and_between_streams_and_exit_reject() {
    // Removing either stream comparison lets one of these synthetic changes pass.
    for (change_at, unavailable) in [(1, false), (2, false), (3, false), (1, true), (3, true)] {
        let fixture = Fixture::new(false, None);
        let clock = InjectedClock::new(0);
        let budget = CallBudget {
            deadline: MonoInstant(2000),
            cancellation: Cancellation::default(),
        };
        let provider = ChangingProvider {
            calls: AtomicUsize::new(0),
            change_at,
            unavailable,
        };
        let result = request_inner(
            &fixture.path,
            "id",
            "pane.get",
            json!({}),
            &clock,
            &budget,
            UNREACHED_WALL_LIMIT,
            Some(&provider),
            &|_| {},
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::StaleHostObservation);
    }
}
#[test]
fn ping_path_rebind_and_late_response_cannot_publish_witness() {
    // The late case advances the injected clock to the deadline before the
    // fixture writes (formerly an 80ms wall sleep against a 30ms wall budget),
    // so the outcome no longer depends on scheduling.
    for (rebind, late, expected) in [
        (true, false, ErrorCode::StaleHostObservation),
        (false, true, ErrorCode::DeadlineExceeded),
    ] {
        let clock = InjectedClock::new(0);
        let fixture = Fixture::new(rebind, late.then(|| (clock.0.clone(), 30)));
        let budget = CallBudget {
            deadline: MonoInstant(30),
            cancellation: Cancellation::default(),
        };
        let result = request_witnessed(
            &fixture.path,
            "id",
            "pane.get",
            json!({}),
            &clock,
            &budget,
            UNREACHED_WALL_LIMIT,
        );
        assert_eq!(result.unwrap_err().code, expected);
    }
}

struct InjectedClock(Arc<AtomicU64>);
impl InjectedClock {
    fn new(now: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now)))
    }
}
impl Clock for InjectedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::Acquire))
    }
}
struct ErrorProvider<'a> {
    calls: AtomicUsize,
    error_at: usize,
    mode: usize,
    cancellation: &'a Cancellation,
    clock: &'a InjectedClock,
}
impl ProcessInfoProvider for ErrorProvider<'_> {
    fn process_info(
        &self,
        pid: u32,
    ) -> Result<super::super::continuity::ProcessInfo, super::super::continuity::CaptureError> {
        if self.calls.fetch_add(1, Ordering::AcqRel) == self.error_at {
            if self.mode == 0 {
                self.cancellation.cancel();
            }
            if self.mode == 1 {
                self.clock.0.store(2000, Ordering::Release);
            }
            return Err(super::super::continuity::CaptureError::ProcessDenied);
        }
        KernelProcessInfo.process_info(pid)
    }
}
#[test]
fn fix1_error_capture_observes_cancel_expiry_and_live_budget_before_write_and_after_response() {
    for error_at in [0, 1, 3] {
        for (mode, expected) in [
            (0, ErrorCode::Cancelled),
            (1, ErrorCode::DeadlineExceeded),
            (2, ErrorCode::StaleHostObservation),
        ] {
            let fixture = Fixture::new(false, None);
            let clock = InjectedClock::new(0);
            let budget = CallBudget {
                deadline: MonoInstant(2000),
                cancellation: Cancellation::default(),
            };
            let provider = ErrorProvider {
                calls: AtomicUsize::new(0),
                error_at,
                mode,
                cancellation: &budget.cancellation,
                clock: &clock,
            };
            let result = request_inner(
                &fixture.path,
                "id",
                "pane.get",
                json!({}),
                &clock,
                &budget,
                UNREACHED_WALL_LIMIT,
                Some(&provider),
                &|_| {},
            );
            assert_eq!(
                result.unwrap_err().code,
                expected,
                "error_at={error_at}, mode={mode}"
            );
            assert_eq!(
                fixture.requests.load(Ordering::Acquire),
                if error_at == 0 {
                    0
                } else if error_at == 1 {
                    1
                } else {
                    2
                }
            );
        }
    }
}

#[test]
fn fix1_path_capture_error_seam_has_post_call_budget_precedence() {
    for (mode, expected) in [
        (0, ErrorCode::Cancelled),
        (1, ErrorCode::DeadlineExceeded),
        (2, ErrorCode::Unsupported),
    ] {
        let clock = InjectedClock::new(0);
        let budget = CallBudget {
            deadline: MonoInstant(2000),
            cancellation: Cancellation::default(),
        };
        let result: Result<super::super::continuity::SocketIdentity, ApiError> = budgeted_capture(
            || {
                if mode == 0 {
                    budget.cancellation.cancel();
                }
                if mode == 1 {
                    clock.0.store(2000, Ordering::Release);
                }
                Err(super::super::continuity::CaptureError::Unsupported)
            },
            &clock,
            &budget,
            Instant::now(),
            UNREACHED_WALL_LIMIT,
        );
        assert_eq!(result.unwrap_err().code, expected);
    }
}

#[test]
fn fixture_serves_request_written_after_its_accept_returned() {
    // Kills: dropping `stream.set_nonblocking(false)` in `Fixture` (macOS
    // accepted sockets inherit O_NONBLOCK). The server then reads before this
    // client writes, sees WouldBlock and drops the connection, which made
    // `fix1_error_capture_...` observe HostUnavailable or a peer-less witness
    // under CPU load.
    let fixture = Fixture::new(false, None);
    let mut stream = std::os::unix::net::UnixStream::connect(&fixture.path).unwrap();
    while fixture.accepted.load(Ordering::Acquire) == 0 {
        std::thread::yield_now();
    }
    // Let the worker reach its read before the request exists.
    std::thread::sleep(Duration::from_millis(50));
    writeln!(stream, "{}", json!({"id":"late-writer","method":"ping"})).unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], "late-writer");
    assert_eq!(response["result"]["type"], "pong");
    assert_eq!(fixture.requests.load(Ordering::Acquire), 1);
}
