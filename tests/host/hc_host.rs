//! `HcHost`, `HcBoot` and the label vocabulary against `fake_hc`.

use super::labels::{HcAddress, TerminalId};
use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use crate::test_support::fake_hc::{FakeHc, FakeTerminal};

fn hello(core_id: &str) -> HelloReply {
    HelloReply {
        core: wire::CoreInfo {
            id: core_id.into(),
            name: String::new(),
        },
        ..HelloReply::default()
    }
}

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

fn host_with_t1() -> (FakeHc, HcHost) {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let host = HcHost::new(HcClient::new(fake.socket()));
    (fake, host)
}

#[test]
fn hc_boot_from_hello() {
    let boot = HcBoot::from_hello(&hello("core-0001")).unwrap();
    assert_eq!(boot.id().as_str(), "hc-core:core-0001");
    // 8-byte prefix: a 120-byte id fits exactly, 121 does not.
    let fits = "a".repeat(128 - BOOT_PREFIX.len());
    assert_eq!(
        HcBoot::from_hello(&hello(&fits))
            .unwrap()
            .id()
            .as_str()
            .len(),
        128
    );
    assert!(HcBoot::from_hello(&hello(&format!("{fits}a"))).is_none());
    assert!(HcBoot::from_hello(&hello("cöre")).is_none());
    assert!(HcBoot::from_hello(&hello("")).is_none());
    assert!(HcBoot::from_hello(&hello("a b")).is_none());
}

#[test]
fn hc_in_implemented_set() {
    assert!(HostKind::Hc.is_implemented());
    assert_eq!(HostKind::from_stored("hc"), HostKind::Hc);
    assert_eq!(HostKind::Hc.stored_name(), Some("hc"));
    let (_fake, host) = host_with_t1();
    assert_eq!(host.kind(), HostKind::Hc);
}

#[test]
fn target_root_process_is_terminal_get_pid() {
    let (fake, host) = host_with_t1();
    let target = HostTargetId::new("t1");
    assert_eq!(
        host.target_root_process(&target, &context()).unwrap(),
        Some(4242)
    );
    fake.set_pid("t1", Some(777));
    assert_eq!(
        host.target_root_process(&target, &context()).unwrap(),
        Some(777)
    );
    fake.set_pid("t1", None);
    assert_eq!(host.target_root_process(&target, &context()).unwrap(), None);
    assert!(
        host.target_root_process(&HostTargetId::new("t99"), &context())
            .is_err()
    );
    let methods: Vec<String> = fake.calls().into_iter().map(|c| c.method).collect();
    assert!(methods.iter().all(|m| m == "terminal.get"), "{methods:?}");
}

#[test]
fn hc_rpc_honours_the_remaining_budget() {
    use std::os::unix::net::UnixListener;
    let dir = crate::test_support::scratch::temp_path(format!(
        "ht-hc-budget-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("s");
    // Accepts and never answers.
    let listener = UnixListener::bind(&socket).unwrap();
    let held = std::thread::spawn(move || {
        let mut conns = Vec::new();
        while let Ok((conn, _)) = listener.accept() {
            conns.push(conn);
            if conns.len() >= 4 {
                break;
            }
        }
        conns
    });
    let clock = Arc::new(crate::app::SystemClock::new());
    let host = HcHost::with_clock(HcClient::new(&socket), clock.clone());
    let within = |ms: u64| HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + ms),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    };
    let started = std::time::Instant::now();
    let error = host
        .target_root_process(&HostTargetId::new("t1"), &within(200))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the 5 s call deadline did not bound the read: {:?}",
        started.elapsed()
    );
    // An exhausted budget returns without connecting.
    let exhausted = HostCallContext {
        budget: CallBudget {
            deadline: clock.monotonic_now(),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    };
    let started = std::time::Instant::now();
    let error = host
        .target_root_process(&HostTargetId::new("t1"), &exhausted)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::DeadlineExceeded);
    assert!(started.elapsed() < Duration::from_millis(100));
    // Unblock the listener thread (it exits after the connections it expects).
    for _ in 0..3 {
        let _ = std::os::unix::net::UnixStream::connect(&socket);
    }
    let _ = held.join();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn target_root_process_works_inside_a_runtime() {
    let (_fake, host) = host_with_t1();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let pid = rt.block_on(async { host.target_root_process(&HostTargetId::new("t1"), &context()) });
    assert_eq!(pid.unwrap(), Some(4242));
}

#[test]
fn answers_wake_none_closing_no_evidence() {
    let (_fake, host) = host_with_t1();
    assert_eq!(host.wake_capability(), WakeCapability::None);
    assert_eq!(host.closure_authority(), ClosureAuthority::Closing);
    assert_eq!(host.archival_evidence(), ArchivalEvidence::NoEvidence);
    assert_eq!(
        host.native_launch_capability(),
        NativeLaunchCapability::Unsupported
    );
}

#[test]
fn observation_sequence_is_its_own_counter() {
    let (_fake, a) = host_with_t1();
    let (_fake2, b) = host_with_t1();
    assert_eq!(a.next_observation_sequence(), 1);
    assert_eq!(a.next_observation_sequence(), 2);
    assert_eq!(b.next_observation_sequence(), 1);
}

#[test]
fn resume_after_epoch_reaches_through_the_port() {
    let (_fake, host) = host_with_t1();
    let port: Arc<dyn HostPort> = Arc::new(host);
    port.resume_after_epoch(7);
    assert_eq!(port.enumerate_targets(&context()).unwrap().epoch, 8);
}

#[test]
fn sequence_is_taken_before_the_capture() {
    // A capture that fails still consumed its sequence: it was taken before the request, so
    // an older request can never carry a higher sequence than a newer one.
    let dead = HcHost::new(HcClient::new("/nonexistent/hc.sock"));
    assert!(dead.enumerate_targets(&context()).is_err());
    assert_eq!(dead.next_observation_sequence(), 2, "enumerate_targets");
    assert!(
        dead.observe_current_target(&HostTargetId::new("t1"), &context())
            .is_err()
    );
    assert_eq!(
        dead.next_observation_sequence(),
        4,
        "observe_current_target"
    );
    assert!(
        dead.observe_bootstrap(&HostTargetId::new("t1"), &context())
            .is_err()
    );
    assert_eq!(dead.next_observation_sequence(), 6, "observe_bootstrap");
}

#[test]
fn enumerate_targets_refused_socket_is_host_unavailable() {
    let dead = HcHost::new(HcClient::new("/nonexistent/hc.sock"));
    let error = dead.enumerate_targets(&context()).unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert_eq!(dead.capture().unwrap_err().code, ErrorCode::HostUnavailable);
    // A live core with a terminal enumerates it (the capture rule is
    // tests/host/hc_closure.rs): one connection, no `terminal.get`.
    let (fake, host) = host_with_t1();
    let snapshot = host.enumerate_targets(&context()).unwrap();
    assert_eq!(snapshot.enumeration, EnumerationEvidence::CoherentVerified);
    assert_eq!(snapshot.targets.len(), 1);
    assert_eq!(snapshot.boot.as_str(), "hc-core:core-0001");
    assert!(fake.calls_to("terminal.get").is_empty());
    assert_eq!(fake.calls_to("events.subscribe").len(), 1);
}

#[test]
fn snapshot_without_terminals_is_err_never_coherent() {
    let (fake, host) = host_with_t1();
    fake.set_omit_terminals(true);
    let error = host.enumerate_targets(&context()).unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(format!("{error:?}").contains("no terminals"), "{error:?}");
    assert_eq!(host.capture().unwrap_err().code, ErrorCode::HostUnavailable);
    fake.set_omit_terminals(false);
    let snapshot = host.enumerate_targets(&context()).unwrap();
    assert_eq!(snapshot.enumeration, EnumerationEvidence::CoherentVerified);
}

#[test]
fn unusable_core_id_is_unavailable() {
    let (fake, host) = host_with_t1();
    fake.rotate_core_id("");
    assert!(host.capture().unwrap().is_none());
    let error = host.enumerate_targets(&context()).unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(format!("{error:?}").contains("core id"), "{error:?}");
}

#[test]
fn env_vars_are_the_scrub_set() {
    assert_eq!(
        ENV_VARS,
        [
            "HC_TERMINAL_ID",
            "HC_TERMINAL_SOCKET",
            "HC_SCOPE",
            "HC_SESSION"
        ]
    );
    let list = crate::host::child_env_scrub_list();
    assert!(ENV_VARS.iter().all(|name| list.contains(name)));
}

#[test]
fn address_format() {
    let t = TerminalId::parse("t12").unwrap();
    let a = HcAddress::new("main", t);
    assert_eq!(a.to_string(), "main:t12");
    assert_eq!(HcAddress::parse("main:t12"), Some(a));
    let bare = HcAddress::parse("t3").unwrap();
    assert_eq!(bare.session, None);
    assert_eq!(bare.to_string(), "t3");
    for bad in [
        "",
        "t",
        "tx",
        "t1x",
        ":t1",
        "a:b:t1",
        "main:",
        "main:1",
        "machine/t1",
    ] {
        assert_eq!(HcAddress::parse(bad), None, "{bad}");
    }
}

// ---- bootstrap observation (ht-kqrt.133) ----

fn observer(host: HcHost) -> HcBootstrapObserver {
    HcBootstrapObserver::new(Arc::new(host))
}

#[test]
fn bootstrap_observer_reads_one_terminal_on_the_hello_peer() {
    use crate::ports::BootstrapObserver;
    let (fake, host) = host_with_t1();
    let hello = HcHost::run(host.client.hello()).unwrap();
    let before = fake.calls_to("terminal.get").len();
    let pane = observer(host)
        .observe_bootstrap_target(&HostTargetId::new("t1"), &context())
        .unwrap();
    assert_eq!(fake.calls_to("terminal.get").len(), before + 1);
    let observation = pane.observation();
    assert_eq!(observation.target.as_str(), "t1");
    assert_eq!(observation.host_boot.as_str(), "hc-core:core-0001");
    assert_eq!(
        observation.provenance,
        ObservationProvenance::FreshCurrentTarget
    );
    assert_eq!(
        observation.terminal,
        Some(crate::protocol::ids::TerminalId::new("t1"))
    );
    assert!(observation.verified_structural_proof().is_some());
    // The read was pinned to the peer `hello` named.
    assert_eq!(pane.witness(), &hello.witness);
}

#[test]
fn bootstrap_observer_refuses_absent_and_finished_terminals() {
    use crate::ports::BootstrapObserver;
    let (fake, host) = host_with_t1();
    fake.add_terminal(FakeTerminal::live("t2"));
    fake.finish("t2", "exited", Some(0), None);
    let observer = observer(host);
    for target in ["t9", "t2"] {
        let error = observer
            .observe_bootstrap_target(&HostTargetId::new(target), &context())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound, "{target}");
    }
    assert!(
        observer
            .observe_bootstrap_target(&HostTargetId::new("t1"), &context())
            .is_ok()
    );
}

#[test]
fn bootstrap_observer_refuses_a_changed_core() {
    use crate::ports::BootstrapObserver;
    let (fake, host) = host_with_t1();
    let observer = observer(host);
    let mut pinned = context();
    pinned.expected_boot = Some(HostBootId::new("hc-core:core-0001"));
    assert!(
        observer
            .observe_bootstrap_target(&HostTargetId::new("t1"), &pinned)
            .is_ok()
    );
    fake.rotate_core_id("core-0002");
    let error = observer
        .observe_bootstrap_target(&HostTargetId::new("t1"), &pinned)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleHostObservation);
}

/// A monotonic origin an hour older than a fresh `SystemClock`'s: a host that read budgets on a
/// private `SystemClock` instead of this one would see them as an hour away.
struct OlderClock(std::time::Instant);
impl crate::protocol::time::Clock for OlderClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(3_600_000 + self.0.elapsed().as_millis() as u64)
    }
}

/// A socket that accepts and never answers, until dropped.
struct SilentServer {
    path: std::path::PathBuf,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl SilentServer {
    fn start() -> Self {
        let path =
            crate::test_support::scratch::temp_path(format!("hc-silent-{}", std::process::id()));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut held = Vec::new();
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => held.push(stream),
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            path,
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
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn production_factory_measures_hc_budgets_on_its_clock() {
    let server = SilentServer::start();
    let clock: Arc<dyn crate::protocol::time::Clock> =
        Arc::new(OlderClock(std::time::Instant::now()));
    let factory = crate::app::production_host_factory(Arc::clone(&clock));
    let locator = crate::protocol::ids::HostLocator::parse(server.path.to_string_lossy()).unwrap();
    let (host, _observer) = (factory.build)(HostKind::Hc, &locator);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 200),
        cancellation: Cancellation::default(),
    };
    let started = std::time::Instant::now();
    let error = host.live_topology(&budget).unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a 200 ms budget on the daemon clock took {:?}",
        started.elapsed()
    );
}

#[test]
fn probe_connect_honours_its_budget_on_a_full_backlog() {
    use crate::host::probe::{connect_until, probe_kind};
    let path =
        crate::test_support::scratch::temp_path(format!("hc-backlog-{}", std::process::id()));
    // Never accepts: the accept backlog fills and a plain `connect` then blocks.
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    // `bind` listens with the system maximum; a backlog of one fills in a few connects.
    // SAFETY: `listen(2)` on the listener's own descriptor takes no pointers.
    assert_eq!(
        unsafe { libc::listen(std::os::fd::AsRawFd::as_raw_fd(&listener), 1) },
        0
    );
    let mut held = Vec::new();
    loop {
        match connect_until(&path, std::time::Instant::now() + Duration::from_millis(50)) {
            Ok(stream) => held.push(stream),
            Err(_) => break,
        }
        assert!(held.len() < 4096, "the backlog never filled");
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let probe_path = path.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let result = probe_kind(&probe_path, Duration::from_millis(300));
        let _ = sender.send((result, started.elapsed()));
    });
    // The watchdog makes a regression (a blocked connect) a failure, not a hang.
    let (result, elapsed) = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("probe_kind blocked on a full backlog");
    assert_eq!(result.unwrap_err().code, ErrorCode::HostUnavailable);
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    drop(held);
    let _ = std::fs::remove_file(&path);
}
