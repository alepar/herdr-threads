use herdr_threads::host::native::{NativeCli, PromptOutcome};
use herdr_threads::host::observation::{normalize_pane, normalize_snapshot};
use herdr_threads::ports::{
    HostCallContext, HostPort, IncarnationEvidence, NativeLaunchCapability,
};
use herdr_threads::protocol::ids::HostTargetId;
use herdr_threads::protocol::results::ErrorCode;
use herdr_threads::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::Arc,
    thread,
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
fn budget(ms: u64) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(ms),
        cancellation: Cancellation::default(),
    }
}
fn socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("ht-host-socket-{}", uuid::Uuid::new_v4()))
}
fn read_request(stream: &mut UnixStream) -> Value {
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    assert!(line.ends_with('\n'));
    serde_json::from_str(&line).unwrap()
}
fn respond(stream: &mut UnixStream, request: &Value, result: Value) {
    let id = request["id"].as_str().unwrap();
    writeln!(stream, "{}", json!({"id":id,"result":result})).unwrap();
}
fn serve<F>(operation: F) -> (PathBuf, thread::JoinHandle<()>)
where
    F: FnOnce(&mut UnixStream, Value) + Send + 'static,
{
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let handle = thread::spawn(move || {
        let (mut ping, _) = listener.accept().unwrap();
        let request = read_request(&mut ping);
        assert_eq!(request["method"], "ping");
        respond(
            &mut ping,
            &request,
            json!({"type":"pong","version":"0.9.1","protocol":22}),
        );
        drop(ping);
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        operation(&mut stream, request);
    });
    (path, handle)
}
fn cleanup(path: PathBuf, handle: thread::JoinHandle<()>) {
    handle.join().unwrap();
    fs::remove_file(path).unwrap();
}
fn pane() -> Value {
    json!({"pane_id":"w4:p1","terminal_id":"term_1","workspace_id":"w4",
        "tab_id":"w4:t1","focused":false,"agent_status":"idle","revision":7,
        "agent_session":{"agent":"codex","kind":"id","source":"herdr:codex","value":"cached"},
        "future_field":{"anything":true}})
}

#[test]
fn explicit_pane_and_snapshot_use_separate_ping_and_operation_connections() {
    let (path, handle) = serve(|stream, request| {
        assert_eq!(request["method"], "pane.get");
        assert_eq!(request["params"], json!({"pane_id":"w4:p1"}));
        respond(stream, &request, json!({"type":"pane_info","pane":pane()}));
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let context = HostCallContext {
        budget: budget(5000),
        expected_boot: None,
        expected_epoch: None,
    };
    let observed = cli
        .observe_current_target(&HostTargetId::new("w4:p1"), &context)
        .unwrap();
    assert_eq!(observed.target.as_str(), "w4:p1");
    // macOS binds the response to the serving process; elsewhere it stays Unknown.
    assert_eq!(
        matches!(observed.incarnation, IncarnationEvidence::Verified { .. }),
        cfg!(target_os = "macos")
    );
    assert!(!observed.has_verified_execution());
    cleanup(path, handle);

    let (path, handle) = serve(|stream, request| {
        assert_eq!(request["method"], "session.snapshot");
        assert_eq!(request["params"], json!({}));
        respond(
            stream,
            &request,
            json!({"type":"session_snapshot","snapshot":{
            "version":"0.9.1","protocol":22,"panes":[pane()],"agents":[],
            "tabs":[],"workspaces":[],"layouts":[]}}),
        );
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let snapshot = cli.enumerate_targets(&context).unwrap();
    assert_eq!(snapshot.targets.len(), 1);
    assert_eq!(
        snapshot.authorizes_absence_closure(),
        cfg!(target_os = "macos")
    );
    cleanup(path, handle);
}

#[test]
fn connected_no_reply_obeys_whole_call_deadline_and_closes_owned_socket() {
    let (path, handle) = serve(|stream, _request| {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(std::io::Read::read(stream, &mut byte).unwrap(), 0);
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let started = Instant::now();
    let result = cli.run(
        &["pane", "get", "w4:p1"],
        &budget(5000),
        Duration::from_millis(500),
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::DeadlineExceeded);
    assert!(started.elapsed() < Duration::from_millis(750));
    cleanup(path, handle);
}

#[test]
fn cancellation_closes_socket_and_prompt_result_is_unknown() {
    let (path, handle) = serve(|stream, request| {
        assert_eq!(request["method"], "agent.prompt");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(std::io::Read::read(stream, &mut byte).unwrap(), 0);
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let call_budget = budget(5000);
    let cancel = call_budget.cancellation.clone();
    let notifier = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        cancel.cancel();
    });
    assert_eq!(
        cli.prompt("w4:p1", "hint", &call_budget),
        PromptOutcome::Unknown
    );
    notifier.join().unwrap();
    cleanup(path, handle);
}

#[test]
fn malformed_foreign_id_and_partial_frames_fail_closed() {
    for mode in [
        "foreign",
        "malformed",
        "partial",
        "two_frames",
        "both_branches",
    ] {
        let (path, handle) = serve(move |stream, request| match mode {
            "foreign" => writeln!(
                stream,
                "{}",
                json!({"id":"other","result":{"type":"pane_info","pane":pane()}})
            )
            .unwrap(),
            "malformed" => stream.write_all(b"{bad}\n").unwrap(),
            "two_frames" => {
                let id = request["id"].as_str().unwrap();
                let frames = format!(
                    "{}\n{}\n",
                    json!({"id":id,"result":{"type":"pane_info","pane":pane()}}),
                    json!({"id":id,"result":{"type":"pane_info","pane":pane()}})
                );
                stream.write_all(frames.as_bytes()).unwrap();
            }
            "both_branches" => {
                let id = request["id"].as_str().unwrap();
                writeln!(
                    stream,
                    "{}",
                    json!({"id":id,"result":{"type":"pane_info","pane":pane()},
                        "error":{"code":"internal_error","message":"ambiguous"}})
                )
                .unwrap();
            }
            _ => {
                let id = request["id"].as_str().unwrap();
                write!(
                    stream,
                    "{}",
                    json!({"id":id,"result":{"type":"pane_info","pane":pane()}})
                )
                .unwrap();
            }
        });
        let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
        assert!(cli.pane("w4:p1", &budget(5000)).is_err());
        cleanup(path, handle);
    }
}

#[test]
fn failed_ping_never_opens_operation_connection() {
    for protocol in [21, 23] {
        let path = socket_path();
        let listener = UnixListener::bind(&path).unwrap();
        let worker = thread::spawn(move || {
            let (mut ping, _) = listener.accept().unwrap();
            let request = read_request(&mut ping);
            respond(
                &mut ping,
                &request,
                json!({"type":"pong","version":"0.9.1","protocol":protocol}),
            );
            listener.set_nonblocking(true).unwrap();
            thread::sleep(Duration::from_millis(30));
            assert!(
                listener.accept().is_err(),
                "operation sent after failed ping"
            );
        });
        let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
        assert_eq!(
            cli.run(
                &["pane", "get", "w4:p1"],
                &budget(5000),
                Duration::from_secs(2)
            )
            .unwrap_err()
            .code,
            ErrorCode::Unsupported
        );
        cleanup(path, worker);
    }
}

#[test]
fn oversized_and_continuous_responses_stop_without_unbounded_memory_or_time() {
    let (path, handle) = serve(|stream, _| {
        let chunk = vec![b'x'; 8192];
        for _ in 0..600 {
            if stream.write_all(&chunk).is_err() {
                break;
            }
        }
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    assert_eq!(
        cli.run(&["api", "snapshot"], &budget(5000), Duration::from_secs(2))
            .unwrap_err()
            .code,
        ErrorCode::StaleHostObservation
    );
    cleanup(path, handle);

    let (path, handle) = serve(|stream, _| {
        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
        let chunk = [b'x'; 1024];
        for _ in 0..10000 {
            if stream.write_all(&chunk).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let started = Instant::now();
    assert_eq!(
        cli.run(
            &["api", "snapshot"],
            &budget(5000),
            Duration::from_millis(150)
        )
        .unwrap_err()
        .code,
        ErrorCode::DeadlineExceeded
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    cleanup(path, handle);
}

#[test]
fn prompt_submission_is_transport_only_and_public_native_capabilities_stay_closed() {
    let (path, handle) = serve(|stream, request| {
        assert_eq!(request["method"], "agent.prompt");
        assert_eq!(request["params"], json!({"target":"w4:p1","text":"hint"}));
        respond(
            stream,
            &request,
            json!({"type":"agent_prompted","agent":{"pane_id":"w4:p1"}}),
        );
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    assert_eq!(
        cli.prompt("w4:p1", "hint", &budget(5000)),
        PromptOutcome::Submitted
    );
    // Guarded start is the adapter's launch capability wherever the kernel
    // peer witness exists (macOS); it grants no prompt or receipt authority.
    assert_eq!(
        cli.native_launch_capability(),
        if cfg!(target_os = "macos") {
            NativeLaunchCapability::HostGuardedStart
        } else {
            NativeLaunchCapability::Unsupported
        }
    );
    cleanup(path, handle);
}

#[test]
fn parser_rejects_wrong_explicit_target_and_missing_snapshot_identity() {
    let response = json!({"id":"x","result":{"type":"pane_info","pane":pane()}});
    assert!(normalize_pane(&response.to_string(), "w4:p2").is_err());
    let mut malformed = pane();
    malformed.as_object_mut().unwrap().remove("terminal_id");
    let snapshot = json!({"id":"x","result":{"type":"session_snapshot","snapshot":{
        "version":"0.9.1","protocol":22,"panes":[malformed],"agents":[],
        "tabs":[],"workspaces":[],"layouts":[]}}});
    assert!(normalize_snapshot(&snapshot.to_string()).is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn synchronous_host_port_is_safe_inside_service_runtime() {
    let (path, handle) = serve(|stream, request| {
        respond(stream, &request, json!({"type":"pane_info","pane":pane()}));
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    assert_eq!(
        cli.pane("w4:p1", &budget(5000)).unwrap().terminal_id,
        "term_1"
    );
    cleanup(path, handle);
}

// Returning after the first LF would accept a delayed second frame.
#[test]
fn delayed_extra_response_frame_is_rejected() {
    let (path, handle) = serve(|stream, request| {
        respond(stream, &request, json!({"type":"pane_info","pane":pane()}));
        thread::sleep(Duration::from_millis(30));
        let extra = stream.write_all(b"{}\n");
        // Early rejection may close the peer; successful delivery is also valid.
        if let Err(error) = extra {
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ));
        }
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let result = cli.pane("w4:p1", &budget(5000));
    cleanup(path, handle);
    assert_eq!(result.unwrap_err().code, ErrorCode::StaleHostObservation);
}

#[test]
fn ping_errors_preserve_permission_and_protocol_details_before_dispatch() {
    for (body, expected) in [
        (
            json!({"error":{"code":"permission_denied","message":"private socket denied"}}),
            ErrorCode::Unauthorized,
        ),
        (
            json!({"result":{"type":"pong","version":"0.9.2","protocol":22}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"result":{"type":"not_pong","version":"0.9.1","protocol":22}}),
            ErrorCode::Unsupported,
        ),
        (
            json!({"result":{"type":"pong","version":"0.9.1","protocol":"22"}}),
            ErrorCode::Unsupported,
        ),
    ] {
        let path = socket_path();
        let listener = UnixListener::bind(&path).unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let mut response = body;
            response["id"] = request["id"].clone();
            stream
                .write_all(format!("{response}\n").as_bytes())
                .unwrap();
            drop(stream);
            listener.set_nonblocking(true).unwrap();
            thread::sleep(Duration::from_millis(30));
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        });
        let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
        let error = cli.pane("w4:p1", &budget(5000)).unwrap_err();
        assert_eq!(error.code, expected);
        if expected == ErrorCode::Unauthorized {
            assert!(error.detail.contains("private socket denied"));
        }
        cleanup(path, worker);
    }
}

#[test]
fn response_limit_counts_lf_and_accepts_exact_boundary() {
    for extra in [0, 1] {
        let (path, worker) = serve(move |stream, request| {
            let mut frame = json!({"id":request["id"],"result":{"type":"pane_info","pane":pane()}})
                .to_string()
                .into_bytes();
            frame.resize(4 * 1024 * 1024 - 1 + extra, b' ');
            frame.push(b'\n');
            let result = stream.write_all(&frame);
            if extra == 0 {
                result.unwrap();
            }
        });
        let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
        let result = cli.pane("w4:p1", &budget(5000));
        cleanup(path, worker);
        if extra == 0 {
            assert_eq!(result.unwrap().target.as_str(), "w4:p1");
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::StaleHostObservation);
        }
    }
}

#[test]
fn ping_and_operation_share_absolute_deadline() {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        thread::sleep(Duration::from_millis(80));
        respond(
            &mut stream,
            &request,
            json!({"type":"pong","version":"0.9.1","protocol":22}),
        );
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(std::io::Read::read(&mut stream, &mut byte).unwrap(), 0);
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let started = Instant::now();
    assert_eq!(
        cli.run(
            &["pane", "get", "w4:p1"],
            &budget(130),
            Duration::from_secs(2)
        )
        .unwrap_err()
        .code,
        ErrorCode::DeadlineExceeded
    );
    assert!(started.elapsed() < Duration::from_millis(190));
    cleanup(path, worker);
}

#[test]
fn cancellation_during_ping_closes_socket_without_operation() {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let call_budget = budget(5000);
    let cancel = call_budget.cancellation.clone();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        cancel.cancel();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(std::io::Read::read(&mut stream, &mut byte).unwrap(), 0);
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let epoch = cli.epoch();
    assert_eq!(
        cli.run(
            &["pane", "get", "w4:p1"],
            &call_budget,
            Duration::from_secs(2)
        )
        .unwrap_err()
        .code,
        ErrorCode::Cancelled
    );
    assert!(cli.epoch() > epoch);
    cleanup(path, worker);
}

#[test]
fn stalled_request_write_is_cancellable_and_never_reports_submission() {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let call_budget = budget(5000);
    let cancel = call_budget.cancellation.clone();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        respond(
            &mut stream,
            &request,
            json!({"type":"pong","version":"0.9.1","protocol":22}),
        );
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        // Leave request unread so the socket write buffer fills.
        thread::sleep(Duration::from_millis(40));
        cancel.cancel();
        thread::sleep(Duration::from_millis(30));
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            let count = std::io::Read::read(&mut stream, &mut chunk).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        assert!(!bytes.is_empty());
        assert!(
            !bytes.ends_with(b"\n"),
            "fixture did not exercise partial write"
        );
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    assert_eq!(
        cli.prompt("w4:p1", &"x".repeat(900_000), &call_budget),
        PromptOutcome::Unknown
    );
    cleanup(path, worker);
}

#[test]
fn duplicate_envelope_keys_and_non_object_branches_fail_closed() {
    for mode in [
        "duplicate_id",
        "duplicate_result",
        "neither",
        "null",
        "array",
        "null_error",
    ] {
        let (path, worker) = serve(move |stream, request| {
            let id = request["id"].as_str().unwrap();
            let result = json!({"type":"pane_info","pane":pane()});
            let body = match mode {
                "duplicate_id" => format!(r#"{{"id":"other","id":"{id}","result":{result}}}"#),
                "duplicate_result" => format!(r#"{{"id":"{id}","result":{{}},"result":{result}}}"#),
                "neither" => json!({"id":id}).to_string(),
                "null" => json!({"id":id,"result":null}).to_string(),
                "null_error" => json!({"id":id,"result":result,"error":null}).to_string(),
                _ => json!({"id":id,"result":[]}).to_string(),
            };
            stream.write_all(format!("{body}\n").as_bytes()).unwrap();
        });
        let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
        let result = cli.pane("w4:p1", &budget(5000));
        cleanup(path, worker);
        assert_eq!(
            result.unwrap_err().code,
            ErrorCode::StaleHostObservation,
            "{mode}"
        );
    }
}

#[test]
fn cancelled_before_connect_sends_nothing() {
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let call_budget = budget(5000);
    call_budget.cancellation.cancel();
    assert_eq!(
        cli.run(
            &["pane", "get", "w4:p1"],
            &call_budget,
            Duration::from_secs(2)
        )
        .unwrap_err()
        .code,
        ErrorCode::Cancelled
    );
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(listener);
    fs::remove_file(path).unwrap();
}

#[test]
fn possible_prompt_submission_with_late_reply_stays_unknown_and_is_not_replayed() {
    let (path, worker) = serve(|stream, request| {
        assert_eq!(request["method"], "agent.prompt");
        thread::sleep(Duration::from_millis(100));
        let response = json!({"id":request["id"],"result":{"type":"agent_prompted","agent":{"pane_id":"w4:p1"}}});
        let _ = stream.write_all(format!("{response}\n").as_bytes());
    });
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    assert_eq!(
        cli.prompt_with_limit("w4:p1", "hint", &budget(5000), Duration::from_millis(40)),
        PromptOutcome::Unknown
    );
    cleanup(path, worker);
}

/// Explicit opt-in only: the runner must create and own an isolated Herdr server.
#[test]
#[ignore = "requires explicitly authorized isolated Herdr 0.9.1 server"]
fn isolated_pinned_herdr_readonly_compatibility() {
    let path = PathBuf::from(
        std::env::var("TASK11_HERDR_COMPAT_SOCKET").expect("explicit isolated endpoint"),
    );
    assert!(
        path.to_str()
            .unwrap()
            .starts_with("/private/tmp/task11-herdr-compat-")
    );
    let cli = NativeCli::new(path, Arc::new(TestClock(Instant::now())));
    let snapshot = cli.snapshot(&budget(5000)).unwrap();
    assert!(!snapshot.panes.is_empty());
    assert!(!snapshot.current_execution_proven);
    assert!(!snapshot.incarnation_proven);
    assert!(!snapshot.coherent_enumeration_proven);
    for pane in &snapshot.panes {
        let fetched = cli.pane(pane.target.as_str(), &budget(5000)).unwrap();
        assert_eq!(fetched.target, pane.target);
        assert_eq!(fetched.terminal_id, pane.terminal_id);
    }
    assert_ne!(
        cli.native_launch_capability(),
        NativeLaunchCapability::ProvenEmptyShell
    );
    println!(
        "read-only pinned compatibility: {} pane(s); version/protocol/id/framing/EOF validated; native authority unavailable",
        snapshot.panes.len()
    );
}

#[test]
fn operation_shapes_are_checked_and_permission_errors_are_typed() {
    for (body, expected) in [
        (
            json!({"result":{"type":"wrong"}}),
            ErrorCode::StaleHostObservation,
        ),
        (
            json!({"error":{"code":99,"message":"bad code type"}}),
            ErrorCode::StaleHostObservation,
        ),
        (
            json!({"error":{"code":"permission_denied","message":"private deny"}}),
            ErrorCode::Unauthorized,
        ),
    ] {
        let (path, worker) = serve(move |stream, request| {
            let mut body = body;
            body["id"] = request["id"].clone();
            stream.write_all(format!("{body}\n").as_bytes()).unwrap();
        });
        let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
        let result = cli.run(
            &["pane", "get", "w4:p1"],
            &budget(5000),
            Duration::from_secs(2),
        );
        cleanup(path, worker);
        assert_eq!(result.unwrap_err().code, expected);
    }
}

#[test]
fn relative_endpoint_and_oversized_request_are_rejected_before_connect() {
    let cli = NativeCli::new(
        PathBuf::from("herdr.sock"),
        Arc::new(TestClock(Instant::now())),
    );
    assert_eq!(
        cli.pane("w4:p1", &budget(5000)).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    match cli.prompt("w4:p1", &"x".repeat(1024 * 1024), &budget(5000)) {
        PromptOutcome::Rejected(error) => assert_eq!(error.code, ErrorCode::InvalidRequest),
        result => panic!("oversized request was not rejected: {result:?}"),
    }
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    drop(listener);
    fs::remove_file(path).unwrap();
}

/// Serves `calls` complete ping+operation exchanges on one endpoint, each on
/// its own connection, from this process (so the kernel peer is this process).
#[cfg(target_os = "macos")]
fn serve_calls<F>(calls: usize, operation: F) -> (PathBuf, thread::JoinHandle<()>)
where
    F: Fn(&mut UnixStream, Value) + Send + 'static,
{
    let path = socket_path();
    let listener = UnixListener::bind(&path).unwrap();
    let handle = thread::spawn(move || {
        for _ in 0..calls {
            let (mut ping, _) = listener.accept().unwrap();
            let request = read_request(&mut ping);
            assert_eq!(request["method"], "ping");
            respond(
                &mut ping,
                &request,
                json!({"type":"pong","version":"0.9.1","protocol":22}),
            );
            drop(ping);
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            operation(&mut stream, request);
        }
    });
    (path, handle)
}

#[cfg(target_os = "macos")]
fn snapshot_or_pane(stream: &mut UnixStream, request: Value) {
    match request["method"].as_str().unwrap() {
        "session.snapshot" => respond(
            stream,
            &request,
            json!({"type":"session_snapshot","snapshot":{
                "version":"0.9.1","protocol":22,"panes":[pane()],"agents":[],
                "tabs":[],"workspaces":[],"layouts":[]}}),
        ),
        "pane.get" if request["params"]["pane_id"] == "w4:p1" => {
            respond(stream, &request, json!({"type":"pane_info","pane":pane()}))
        }
        "pane.get" => writeln!(
            stream,
            "{}",
            json!({"id":request["id"],"error":{"code":"pane_not_found","message":"pane not found"}})
        )
        .unwrap(),
        other => panic!("unexpected method {other}"),
    }
}

/// Kills: `enumerate_targets` reporting `IncarnationEvidence::Unknown` /
/// `CompleteUnverified` with sequence 0 (composition probe B8: every capture
/// was invalidated and no seat could exist on a real host).
#[cfg(target_os = "macos")]
#[test]
fn witnessed_snapshot_is_a_verified_coherent_server_incarnation() {
    use herdr_threads::ports::{EnumerationEvidence, EvidenceKind, ObservationProvenance};
    let (path, handle) = serve_calls(2, snapshot_or_pane);
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let unfenced = HostCallContext {
        budget: budget(5000),
        expected_boot: None,
        expected_epoch: None,
    };
    let first = cli.enumerate_targets(&unfenced).unwrap();
    let second = cli.enumerate_targets(&unfenced).unwrap();
    cleanup(path, handle);
    let identity = format!("herdr-server:pid={}:start=", std::process::id());
    let IncarnationEvidence::Verified {
        identity: observed,
        evidence_kind,
    } = &first.incarnation
    else {
        panic!(
            "snapshot incarnation stayed unverified: {:?}",
            first.incarnation
        );
    };
    assert!(observed.starts_with(&identity), "{observed}");
    assert_eq!(*evidence_kind, EvidenceKind::CoherentEnumeration);
    assert_eq!(first.boot.as_str(), observed);
    assert_eq!(first.enumeration, EnumerationEvidence::CoherentVerified);
    assert!(first.authorizes_absence_closure());
    let target = &first.targets[0];
    assert_eq!(target.host_boot, first.boot);
    assert_eq!(
        target.provenance,
        ObservationProvenance::CoherentEnumeration
    );
    assert_eq!(target.generation, 1);
    assert!(!target.has_verified_execution());
    // Same server process: same boot and incarnation, strictly newer order.
    assert_eq!(second.boot, first.boot);
    assert_eq!(second.incarnation, first.incarnation);
    assert_eq!(second.epoch, first.epoch);
    assert!(second.observation_sequence > first.observation_sequence);
}

/// Counting fake Herdr endpoint that stays alive until stopped: every accepted
/// connection is counted, each ping is answered with a pong and each operation
/// is served by `snapshot_or_pane`.
#[cfg(target_os = "macos")]
struct CountingServer {
    path: PathBuf,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

#[cfg(target_os = "macos")]
impl CountingServer {
    fn start() -> Self {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let path = socket_path();
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, halt) = (connections.clone(), stop.clone());
        let handle = thread::spawn(move || {
            while !halt.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        count.fetch_add(1, Ordering::SeqCst);
                        stream.set_nonblocking(false).unwrap();
                        let request = read_request(&mut stream);
                        if request["method"] == "ping" {
                            respond(
                                &mut stream,
                                &request,
                                json!({"type":"pong","version":"0.9.1","protocol":22}),
                            );
                        } else {
                            snapshot_or_pane(&mut stream, request);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            }
        });
        Self {
            path,
            connections,
            stop,
            handle: Some(handle),
        }
    }
    fn connections(&self) -> usize {
        self.connections.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn finish(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.handle.take().unwrap().join().unwrap();
        fs::remove_file(&self.path).unwrap();
    }
}

/// Kills: `check_context` rejecting every `expected_boot` / comparing the
/// store's epoch against an unrelated counter (composition probe B7:
/// `seat resolve --pane` was always StaleHostObservation), a mutation that
/// drops the post-read server-boot comparison, and Mutation A (review S1):
/// removing the pre-I/O epoch fence in `check_context`
/// (`if false && context.expected_boot.is_some() && ...`). The stale-epoch
/// case runs while the counting server is still alive, so without the fence
/// the call connects (and the read even succeeds against the same boot).
#[cfg(target_os = "macos")]
#[test]
fn current_target_is_fenced_by_published_server_boot_and_epoch() {
    use herdr_threads::protocol::ids::HostBootId;
    let server = CountingServer::start();
    let cli = NativeCli::new(server.path.clone(), Arc::new(TestClock(Instant::now())));
    let snapshot = cli
        .enumerate_targets(&HostCallContext {
            budget: budget(5000),
            expected_boot: None,
            expected_epoch: None,
        })
        .unwrap();
    let fenced = HostCallContext {
        budget: budget(5000),
        expected_boot: Some(snapshot.boot.clone()),
        expected_epoch: Some(snapshot.epoch),
    };
    let observed = cli
        .observe_current_target(&HostTargetId::new("w4:p1"), &fenced)
        .unwrap();
    assert_eq!(observed.host_boot, snapshot.boot);
    assert_eq!(observed.epoch, snapshot.epoch);
    assert!(observed.observation_sequence > snapshot.observation_sequence);
    let proof = observed
        .verified_structural_proof()
        .expect("fresh witnessed pane read is qualified structural proof");
    assert_eq!(proof.terminal().as_str(), "term_1");
    assert!(!observed.has_verified_execution());
    // A different server incarnation is rejected after the read.
    let changed = HostCallContext {
        budget: budget(5000),
        expected_boot: Some(HostBootId::new("herdr-server:pid=1:start=1.000000:uid=0")),
        expected_epoch: Some(snapshot.epoch),
    };
    let error = cli
        .observe_current_target(&HostTargetId::new("w4:p1"), &changed)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleHostObservation);
    assert_eq!(error.detail, "host server incarnation changed");
    // Three complete ping+operation exchanges so far: the server is live.
    assert_eq!(server.connections(), 6);
    // A stale epoch against the live server is rejected before any
    // connection is opened.
    let stale = HostCallContext {
        budget: budget(5000),
        expected_boot: Some(snapshot.boot.clone()),
        expected_epoch: Some(snapshot.epoch + 1),
    };
    let result = cli.observe_current_target(&HostTargetId::new("w4:p1"), &stale);
    let connections_after_stale = server.connections();
    server.finish();
    let error = result.expect_err("stale epoch must be fenced");
    assert_eq!(error.code, ErrorCode::StaleHostObservation);
    assert_eq!(error.detail, "host context changed");
    assert_eq!(
        connections_after_stale, 6,
        "stale-epoch fence opened a connection"
    );
}

/// Kills: a no-op epoch resume. Daemon restart must start above every epoch
/// a previous daemon boot persisted, and never lower a live epoch.
#[test]
fn resume_after_persisted_epoch_starts_a_newer_epoch_only() {
    let cli = NativeCli::new(socket_path(), Arc::new(TestClock(Instant::now())));
    assert_eq!(cli.epoch(), 1);
    HostPort::resume_after_epoch(&cli, 0);
    assert_eq!(cli.epoch(), 1);
    HostPort::resume_after_epoch(&cli, 41);
    assert_eq!(cli.epoch(), 42);
    HostPort::resume_after_epoch(&cli, 7);
    assert_eq!(cli.epoch(), 42);
}

/// Kills: mapping Herdr `pane_not_found` to HostUnavailable, which advanced
/// the connection epoch and invalidated the whole observation lane for a
/// mistyped pane.
#[cfg(target_os = "macos")]
#[test]
fn missing_pane_is_typed_not_found_without_epoch_advance() {
    let (path, handle) = serve_calls(1, snapshot_or_pane);
    let cli = NativeCli::new(path.clone(), Arc::new(TestClock(Instant::now())));
    let epoch = cli.epoch();
    let error = cli
        .observe_current_target(
            &HostTargetId::new("w4:p404"),
            &HostCallContext {
                budget: budget(5000),
                expected_boot: None,
                expected_epoch: None,
            },
        )
        .unwrap_err();
    cleanup(path, handle);
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(cli.epoch(), epoch);
}
