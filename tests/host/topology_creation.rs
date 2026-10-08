use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct Peer {
    path: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    operations: Arc<AtomicUsize>,
}
impl Peer {
    fn new(result: Value) -> Self {
        Self::with_action(result, 0, None)
    }
    fn with_action(
        result: Value,
        mode: usize,
        cancellation: Option<crate::protocol::time::Cancellation>,
    ) -> Self {
        let path = std::env::temp_dir().join(format!("ht-qhz3-{}", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let operations = Arc::new(AtomicUsize::new(0));
        let worker_stop = stop.clone();
        let counts = operations.clone();
        let worker = std::thread::spawn(move || {
            let mut ping_done = false;
            while !worker_stop.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                if stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .is_err()
                {
                    continue;
                }
                if mode == 5 && ping_done {
                    let mut byte = [0];
                    if stream.read(&mut byte).unwrap_or(0) > 0 {
                        counts.fetch_add(1, Ordering::AcqRel);
                    }
                    continue;
                }
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap_or(0)
                    == 0
                {
                    continue;
                }
                let wire: Value = serde_json::from_str(&line).unwrap();
                let response = if wire["method"] == "ping" {
                    ping_done = true;
                    json!({"type":"pong", "version":"0.9.1", "protocol":22})
                } else {
                    counts.fetch_add(1, Ordering::AcqRel);
                    assert_eq!(wire["method"], "tab.create");
                    assert_eq!(
                        wire["params"],
                        json!({"workspace_id":"w4", "cwd":"/private/tmp", "label":"test-tab", "focus":false, "env":{}})
                    );
                    result.clone()
                };
                if wire["method"] != "ping" {
                    if let Some(cancel) = &cancellation {
                        cancel.cancel();
                    }
                    if mode == 1 {
                        continue;
                    }
                    if mode == 3 {
                        let _ = writeln!(stream, "{{");
                        continue;
                    }
                }
                let id = if mode == 2 && wire["method"] != "ping" {
                    json!("wrong-id")
                } else {
                    wire["id"].clone()
                };
                let _ = writeln!(stream, "{}", json!({"id":id, "result":response}));
            }
        });
        Self {
            path,
            stop,
            worker: Some(worker),
            operations,
        }
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let joined = self.worker.take().unwrap().join();
        std::fs::remove_file(&self.path).unwrap();
        if !std::thread::panicking() {
            joined.unwrap();
        }
    }
}
fn result() -> Value {
    json!({"type":"tab_created", "tab":{"workspace_id":"w4", "tab_id":"w4:t9"}, "root_pane":{"workspace_id":"w4", "tab_id":"w4:t9", "pane_id":"w4:p9", "terminal_id":"term9"}})
}
fn witness(path: &Path) -> LocalEndpointWitness {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    runtime.block_on(async {
        let stream = UnixStream::connect(path).await.unwrap();
        super::super::continuity::capture_peer_witness(&stream, path).unwrap()
    })
}
fn request(peer: &Peer) -> crate::ports::CreateTabRequest {
    crate::ports::CreateTabRequest {
        correlation: crate::protocol::ids::HostCallId::parse("creation-id").unwrap(),
        workspace: crate::protocol::ids::HostTargetId::parse("w4").unwrap(),
        cwd: "/private/tmp".into(),
        label: "test-tab".into(),
        focus: false,
        env: Default::default(),
        expected_witness: witness(&peer.path),
    }
}
fn context() -> crate::ports::HostCallContext {
    crate::ports::HostCallContext {
        budget: CallBudget {
            deadline: crate::protocol::time::MonoInstant(10000),
            cancellation: Default::default(),
        },
        expected_boot: None,
        expected_epoch: Some(1),
    }
}
fn call(
    peer: &Peer,
    request: &crate::ports::CreateTabRequest,
    context: &crate::ports::HostCallContext,
) -> crate::ports::CreateTabOutcome {
    use crate::ports::CreateTabPort;
    super::super::native::NativeCli::new(
        peer.path.clone(),
        Arc::new(crate::app::SystemClock::new()),
    )
    .create_tab(request, context)
}
#[test]
fn audited_tab_create_is_admitted() {
    let peer = Peer::new(result());
    let request = request(&peer);
    let response = call(&peer, &request, &context());
    let crate::ports::CreateTabOutcome::Created(created) = response else {
        panic!("audited typed request refused: {response:?}")
    };
    assert_eq!(created.correlation, request.correlation);
    assert_eq!(created.tab.as_str(), "w4:t9");
    assert_eq!(created.root_pane.as_str(), "w4:p9");
    assert_eq!(created.terminal.as_str(), "term9");
    assert_eq!(created.witness, request.expected_witness);
    created.validate().unwrap();
    assert_eq!(peer.operations.load(Ordering::Acquire), 1);
}
#[test]
fn malformed_and_incoherent_replies_are_unknown_without_retry() {
    let mut cases = Vec::new();
    for (pointer, value) in [
        ("/type", json!("pane_info")),
        ("/root_pane/terminal_id", json!("")),
        ("/root_pane/terminal_id", Value::Null),
        ("/tab/workspace_id", json!("w5")),
        ("/root_pane/workspace_id", json!("w5")),
        ("/root_pane/tab_id", json!("w4:t8")),
        ("/tab/tab_id", json!("w5:t9")),
        ("/root_pane/pane_id", json!("w5:p9")),
        ("/root_pane/pane_id", json!("w4:t9")),
    ] {
        let mut value_result = result();
        *value_result.pointer_mut(pointer).unwrap() = value;
        cases.push((value_result, 0));
    }
    let mut missing = result();
    missing["root_pane"]
        .as_object_mut()
        .unwrap()
        .remove("terminal_id");
    cases.push((missing, 0));
    cases.extend([(result(), 1), (result(), 2), (result(), 3)]);
    for (reply, mode) in cases {
        let peer = Peer::with_action(reply, mode, None);
        assert!(matches!(
            call(&peer, &request(&peer), &context()),
            crate::ports::CreateTabOutcome::OutcomeUnknown(_)
        ));
        assert_eq!(peer.operations.load(Ordering::Acquire), 1);
    }
}
#[test]
fn invalid_inputs_and_cancel_before_write_are_proven_not_submitted() {
    let peer = Peer::new(result());
    let original = request(&peer);
    for mode in 0..6 {
        let mut request = original.clone();
        let context = context();
        match mode {
            0 => request.focus = true,
            1 => {
                request.env.insert("HERDR_PANE_ID".into(), "unowned".into());
            }
            2 => request.cwd = "relative".into(),
            3 => request.label.clear(),
            4 => request.expected_witness.start_seconds += 1,
            _ => context.budget.cancellation.cancel(),
        }
        assert!(matches!(
            call(&peer, &request, &context),
            crate::ports::CreateTabOutcome::NotSubmitted(_)
        ));
    }
    assert_eq!(peer.operations.load(Ordering::Acquire), 0);
}
#[test]
fn cancellation_after_create_is_unknown() {
    let context = context();
    let peer = Peer::with_action(result(), 0, Some(context.budget.cancellation.clone()));
    assert!(matches!(
        call(&peer, &request(&peer), &context),
        crate::ports::CreateTabOutcome::OutcomeUnknown(_)
    ));
    assert_eq!(peer.operations.load(Ordering::Acquire), 1);
}
struct FinalChange {
    calls: AtomicUsize,
    mode: usize,
    cancel: crate::protocol::time::Cancellation,
}
impl ProcessInfoProvider for FinalChange {
    fn process_info(
        &self,
        pid: u32,
    ) -> Result<super::super::continuity::ProcessInfo, super::super::continuity::CaptureError> {
        let mut info = KernelProcessInfo.process_info(pid)?;
        // ping capture/recheck, operation capture, then final connected-peer recheck.
        if self.calls.fetch_add(1, Ordering::AcqRel) == 3 {
            if self.mode == 0 {
                info.start_seconds += 1;
            } else {
                self.cancel.cancel();
            }
        }
        Ok(info)
    }
}
#[test]
fn final_write_recheck_and_cancel_refuse_with_zero_operation_bytes() {
    for mode in 0..2 {
        let peer = Peer::new(result());
        let request = request(&peer);
        let context = context();
        let guard = CreationGuard {
            expected: &request.expected_witness,
            possible: AtomicBool::new(false),
        };
        let provider = FinalChange {
            calls: AtomicUsize::new(0),
            mode,
            cancel: context.budget.cancellation.clone(),
        };
        let response = request_inner_guarded(
            &peer.path,
            "creation-id",
            "tab.create",
            json!({}),
            &crate::app::SystemClock::new(),
            &context.budget,
            Duration::from_secs(10),
            Some(&provider),
            Some(&guard),
        );
        assert!(response.is_err());
        assert!(!guard.possible.load(Ordering::Acquire));
        assert_eq!(peer.operations.load(Ordering::Acquire), 0);
    }
}

#[test]
fn over_budget_and_oversized_submitted_responses_are_unknown() {
    let peer = Peer::new(json!({"type":"tab_created", "padding":"x".repeat(MAX_RESPONSE)}));
    let request = request(&peer);
    assert!(matches!(
        call(&peer, &request, &context()),
        crate::ports::CreateTabOutcome::OutcomeUnknown(_)
    ));
    assert_eq!(peer.operations.load(Ordering::Acquire), 1);
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "explicit owned native fixture, model-free"]
fn isolated_native_tab_creation() {
    use crate::{ports::CreateTabPort, test_support::isolated_herdr::IsolatedHerdr};
    let host = IsolatedHerdr::new("ht-qhz3-topology").expect("private Herdr required");
    std::fs::create_dir_all(host.root().join("cfg")).unwrap();
    std::fs::write(
        host.root().join("cfg/herdr.toml"),
        "default_shell = '/bin/sh'\n",
    )
    .unwrap();
    host.start();
    eprintln!(
        "owned native fixture root={} pid={:?}",
        host.root().display(),
        host.pid()
    );
    let workspace = host
        .command("herdr")
        .args([
            "workspace",
            "create",
            "--label",
            "ht-qhz3",
            "--no-focus",
            "--cwd",
        ])
        .arg(host.root().join("home"))
        .output()
        .unwrap();
    assert!(
        workspace.status.success(),
        "{}",
        String::from_utf8_lossy(&workspace.stderr)
    );
    let clock = Arc::new(crate::app::SystemClock::new());
    let context = context();
    let snapshot = request_witnessed(
        &host.socket_path(),
        "snapshot",
        "session.snapshot",
        json!({}),
        clock.as_ref(),
        &context.budget,
        Duration::from_secs(10),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&snapshot.body).unwrap();
    let workspace = value
        .pointer("/result/snapshot/workspaces/0/workspace_id")
        .and_then(Value::as_str)
        .unwrap();
    let request = crate::ports::CreateTabRequest {
        correlation: crate::protocol::ids::HostCallId::new("native-create"),
        workspace: crate::protocol::ids::HostTargetId::new(workspace),
        cwd: host.root().join("home"),
        label: "ht-qhz3-child".into(),
        focus: false,
        env: Default::default(),
        expected_witness: snapshot.witness,
    };
    let cli = super::super::native::NativeCli::new(host.socket_path(), clock);
    let outcome = cli.create_tab(&request, &context);
    let crate::ports::CreateTabOutcome::Created(created) = outcome else {
        panic!("{outcome:?}")
    };
    eprintln!(
        "owned native created tab={} pane={} terminal={} incarnation={}",
        created.tab.as_str(),
        created.root_pane.as_str(),
        created.terminal.as_str(),
        created.host_incarnation.as_str()
    );
    created.validate().unwrap();
    // IsolatedHerdr drops only this fixture, including its created topology.
}

#[test]
fn partial_request_disconnect_is_unknown_with_one_submission() {
    let peer = Peer::with_action(result(), 5, None);
    let mut request = request(&peer);
    request.label = "x".repeat(900_000);
    assert!(matches!(
        call(&peer, &request, &context()),
        crate::ports::CreateTabOutcome::OutcomeUnknown(_)
    ));
    assert_eq!(peer.operations.load(Ordering::Acquire), 1);
}
#[test]
fn raw_tab_create_requires_typed_boundary_before_any_submission() {
    let peer = Peer::new(result());
    let outcome = request_witnessed(
        &peer.path,
        "raw",
        "tab.create",
        json!({}),
        &crate::app::SystemClock::new(),
        &context().budget,
        Duration::from_secs(10),
    );
    assert_eq!(outcome.unwrap_err().code, ErrorCode::InvalidRequest);
    assert_eq!(peer.operations.load(Ordering::Acquire), 0);
}
struct DeadlineClock(std::sync::atomic::AtomicU64);
impl Clock for DeadlineClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(0)
    }
    fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
        crate::protocol::time::MonoInstant(self.0.load(Ordering::Acquire))
    }
}
struct ResponseExpiry<'a> {
    clock: &'a DeadlineClock,
    calls: AtomicUsize,
}
impl ProcessInfoProvider for ResponseExpiry<'_> {
    fn process_info(
        &self,
        pid: u32,
    ) -> Result<super::super::continuity::ProcessInfo, super::super::continuity::CaptureError> {
        let info = KernelProcessInfo.process_info(pid)?;
        // Fifth provider call is the operation response's post-EOF witness.
        if self.calls.fetch_add(1, Ordering::AcqRel) == 4 {
            self.clock.0.store(10000, Ordering::Release);
        }
        Ok(info)
    }
}
#[test]
fn response_finished_over_budget_retains_possible_submission() {
    let peer = Peer::new(result());
    let request = request(&peer);
    let clock = DeadlineClock(std::sync::atomic::AtomicU64::new(0));
    let provider = ResponseExpiry {
        clock: &clock,
        calls: AtomicUsize::new(0),
    };
    let guard = CreationGuard {
        expected: &request.expected_witness,
        possible: AtomicBool::new(false),
    };
    let result = request_inner_guarded(
        &peer.path,
        "creation-id",
        "tab.create",
        json!({"workspace_id":"w4", "cwd":"/private/tmp", "label":"test-tab", "focus":false, "env":{}}),
        &clock,
        &context().budget,
        Duration::from_secs(3600),
        Some(&provider),
        Some(&guard),
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::DeadlineExceeded);
    assert!(guard.possible.load(Ordering::Acquire));
    assert_eq!(peer.operations.load(Ordering::Acquire), 1);
}
