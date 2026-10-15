use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

fn instance_id(paths: &crate::daemon::paths::NodePaths) -> crate::node::NodeUuid {
    crate::daemon::ownership::read_node_uuid_hint(paths)
        .unwrap()
        .unwrap()
}

fn health_request(instance: crate::node::NodeUuid, request_id: &str) -> Vec<u8> {
    serde_json::to_vec(&WireRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.into(),
        expected_instance: instance.to_string(),
        expected_boot: None,
        output: None,
        command: Command::Health,
    })
    .unwrap()
}

fn guarded_listener() -> (
    OwnedAsyncListener,
    std::path::PathBuf,
    std::path::PathBuf,
    crate::daemon::paths::NodePaths,
) {
    use crate::daemon::{
        ownership::OwnerLock,
        paths::{NodePaths, RuntimeContext},
    };
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("herdr-ipc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = root.join("host-placeholder.sock");
    let context = RuntimeContext::explicit(root.clone(), host, None).unwrap();
    let paths = NodePaths::resolve(&context.state_dir).unwrap();
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    let bound = owner.bind_socket().unwrap();
    let socket = bound.path().to_owned();
    let listener = bound.into_async().unwrap();
    drop(owner);
    (listener, socket, root, paths)
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_body_read() {
    let (mut sender, mut receiver) = tokio::net::UnixStream::pair().unwrap();
    sender
        .write_all(&((MAX_FRAME_BYTES as u32) + 1).to_be_bytes())
        .await
        .unwrap();
    assert!(read_frame(&mut receiver).await.is_err());
}

use crate::protocol::{
    commands::Command,
    results::{CommandResult, Health},
    time::{Clock, UtcMillis},
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct HeldStopService {
    shutdown: Cancellation,
    entered: std::sync::atomic::AtomicBool,
    release: std::sync::atomic::AtomicBool,
    boot: String,
}
impl LocalService for HeldStopService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        command: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        assert!(matches!(command, Command::Stop(_)));
        self.shutdown.cancel();
        self.entered.store(true, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(CommandResult::StopAccepted(
            crate::protocol::results::StopAccepted {
                boot_id: self.boot.clone(),
            },
        ))
    }
}

#[tokio::test]
async fn admitted_stop_returns_correlated_acceptance_after_shutdown_cancellation() {
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let boot = listener.boot_id().to_string();
    let service = Arc::new(HeldStopService {
        shutdown: shutdown.clone(),
        entered: std::sync::atomic::AtomicBool::new(false),
        release: std::sync::atomic::AtomicBool::new(false),
        boot: boot.clone(),
    });
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut stream = UnixStream::connect(&path).await.unwrap();
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "stop-once".into(),
        expected_instance: instance_id(&instance).to_string(),
        expected_boot: None,
        output: None,
        command: Command::Stop(crate::protocol::commands::StopRequest {
            expected_boot: boot.clone(),
        }),
    };
    write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !service.entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(shutdown.is_cancelled());
    service.release.store(true, Ordering::SeqCst);
    let response: WireResponse = serde_json::from_slice(
        &tokio::time::timeout(Duration::from_secs(1), read_frame(&mut stream))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.request_id, "stop-once");
    assert!(
        matches!(response.result, Ok(CommandResult::StopAccepted(value)) if value.boot_id == boot)
    );
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct RejectOtherService;
impl LocalService for RejectOtherService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(api_error(ErrorCode::Unsupported, "other command"))
    }
}

#[tokio::test]
async fn wrong_instance_and_boot_stop_do_not_cancel_real_socket_owner() {
    use crate::daemon::control::{ControlService, StopController};
    use crate::daemon::health::HealthInputs;
    let (listener, path, root, instance) = guarded_listener();
    let instance_id = instance_id(&instance);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let service = ControlService::new(
        StopController::new(instance_id, boot, shutdown.clone()),
        move |_: &CallBudget| HealthInputs::unknown(instance_id, boot),
        RejectOtherService,
    );
    let server = tokio::spawn(serve(
        listener,
        instance_id,
        Arc::new(service),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    for (expected_instance, expected_boot, code) in [
        (
            crate::node::NodeUuid::new_v4(),
            boot,
            ErrorCode::InstanceMismatch,
        ),
        (
            instance_id,
            uuid::Uuid::new_v4(),
            ErrorCode::InstanceMismatch,
        ),
    ] {
        let mut stream = UnixStream::connect(&path).await.unwrap();
        let request = WireRequest {
            version: PROTOCOL_VERSION,
            request_id: "wrong-stop".into(),
            expected_instance: expected_instance.to_string(),
            expected_boot: None,
            output: None,
            command: Command::Stop(crate::protocol::commands::StopRequest {
                expected_boot: expected_boot.to_string(),
            }),
        };
        write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        let response: WireResponse =
            serde_json::from_slice(&read_frame(&mut stream).await.unwrap()).unwrap();
        assert_eq!(response.result.unwrap_err().code, code);
        assert!(!shutdown.is_cancelled());
    }
    let mut stream = UnixStream::connect(&path).await.unwrap();
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "valid-stop".into(),
        expected_instance: instance_id.to_string(),
        expected_boot: None,
        output: None,
        command: Command::Stop(crate::protocol::commands::StopRequest {
            expected_boot: boot.to_string(),
        }),
    };
    write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    let response: WireResponse =
        serde_json::from_slice(&read_frame(&mut stream).await.unwrap()).unwrap();
    assert!(
        matches!(response.result, Ok(CommandResult::StopAccepted(value)) if value.boot_id == boot.to_string())
    );
    assert!(shutdown.is_cancelled());
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn disconnected_stop_keeps_worker_and_owner_lease_until_handler_exits() {
    use crate::daemon::ownership::OwnerLock;
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let boot = listener.boot_id().to_string();
    let service = Arc::new(HeldStopService {
        shutdown: shutdown.clone(),
        entered: std::sync::atomic::AtomicBool::new(false),
        release: std::sync::atomic::AtomicBool::new(false),
        boot: boot.clone(),
    });
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut stream = UnixStream::connect(&path).await.unwrap();
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "disconnected-stop".into(),
        expected_instance: instance_id(&instance).to_string(),
        expected_boot: None,
        output: None,
        command: Command::Stop(crate::protocol::commands::StopRequest {
            expected_boot: boot,
        }),
    };
    write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !service.entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    drop(stream);
    let ServeOutcome::Incomplete(mut pending) = server.await.unwrap().unwrap() else {
        panic!("disconnected Stop released its owner lease while handler was active");
    };
    assert_eq!(pending.remaining(), 1);
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    service.release.store(true, Ordering::SeqCst);
    pending.wait().await.unwrap();
    drop(pending);
    drop(OwnerLock::acquire_with_test_hint(&instance).unwrap());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn timed_out_stop_client_reports_unknown_outcome_without_resubmission() {
    let (listener, path, root, instance) = guarded_listener();
    let instance_id = instance_id(&instance);
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let boot = listener.boot_id();
    let service = Arc::new(HeldStopService {
        shutdown: shutdown.clone(),
        entered: std::sync::atomic::AtomicBool::new(false),
        release: std::sync::atomic::AtomicBool::new(false),
        boot: boot.to_string(),
    });
    let server = tokio::spawn(serve(
        listener,
        instance_id,
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let clock = Arc::new(TestClock);
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock.clone(),
        instance_id,
        Some(boot),
    );
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 100),
        cancellation: Cancellation::default(),
    };
    let error = client
        .call_async(
            Command::Stop(crate::protocol::commands::StopRequest {
                expected_boot: boot.to_string(),
            }),
            &budget,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    assert!(service.entered.load(Ordering::SeqCst));
    let ServeOutcome::Incomplete(mut pending) = server.await.unwrap().unwrap() else {
        panic!("timed-out Stop released active worker early");
    };
    assert_eq!(pending.remaining(), 1);
    service.release.store(true, Ordering::SeqCst);
    pending.wait().await.unwrap();
    drop(pending);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        MonoInstant(
            START
                .get_or_init(std::time::Instant::now)
                .elapsed()
                .as_millis() as u64,
        )
    }
}
struct CountingService(AtomicUsize, String, String);
impl LocalService for CountingService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        peer: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(CommandResult::Health(Health::unknown(
            self.1.clone(),
            self.2.clone(),
            peer.effective_uid().to_string(),
            1,
        )))
    }
}

#[tokio::test]
async fn malformed_requests_do_not_reach_service_and_peer_uid_is_kernel_supplied() {
    let clock = Arc::new(TestClock);
    let shutdown = Cancellation::default();
    let (listener, path, root, instance) = guarded_listener();
    let service = Arc::new(CountingService(
        AtomicUsize::new(0),
        instance_id(&instance).to_string(),
        listener.boot_id().to_string(),
    ));
    let owner_uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        clock,
        owner_uid,
        shutdown.clone(),
    ));
    for bytes in [
        b"not json".to_vec(),
        format!(r#"{{"version":{},"request_id":"r","expected_instance":"{}","operator_actor":"forged","command":{{"kind":"health"}}}}"#, PROTOCOL_VERSION, instance_id(&instance)).into_bytes(),
    ] {
        let mut stream = UnixStream::connect(&path).await.unwrap();
        write_frame(&mut stream, &bytes).await.unwrap();
        assert!(read_frame(&mut stream).await.is_err());
    }
    // A request of another wire version never reaches the service either, but
    // it is answered with a decodable skew error instead of a closed socket
    // (ht-p03.10); the real release skew pair is a protocol-1 client against
    // this protocol-2 daemon. An invalid request id is still just closed.
    let mut other = UnixStream::connect(&path).await.unwrap();
    let skewed = format!(
        r#"{{"version":1,"request_id":"r","expected_instance":"{}","command":{{"kind":"health"}}}}"#,
        instance_id(&instance)
    );
    write_frame(&mut other, skewed.as_bytes()).await.unwrap();
    let reply: WireResponse =
        serde_json::from_slice(&read_frame(&mut other).await.unwrap()).unwrap();
    assert_eq!(reply.version, 1);
    assert_eq!(reply.request_id, "r");
    assert_eq!(
        reply.result.unwrap_err().code,
        ErrorCode::UnknownWireVersion
    );
    let mut bad_id = UnixStream::connect(&path).await.unwrap();
    let skewed = format!(
        r#"{{"version":1,"request_id":"","expected_instance":"{}","command":{{"kind":"health"}}}}"#,
        instance_id(&instance)
    );
    write_frame(&mut bad_id, skewed.as_bytes()).await.unwrap();
    assert!(read_frame(&mut bad_id).await.is_err());
    let mut truncated = UnixStream::connect(&path).await.unwrap();
    truncated.write_all(&4_u32.to_be_bytes()).await.unwrap();
    truncated.write_all(b"ab").await.unwrap();
    drop(truncated);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(service.0.load(Ordering::SeqCst), 0);
    let mut stream = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut stream, &health_request(instance_id(&instance), "r"))
        .await
        .unwrap();
    let response: WireResponse =
        serde_json::from_slice(&read_frame(&mut stream).await.unwrap()).unwrap();
    assert_eq!(response.request_id, "r");
    let CommandResult::Health(health) = response.result.unwrap() else {
        panic!("wrong result")
    };
    assert_eq!(health.software_version, owner_uid.to_string());
    assert_eq!(service.0.load(Ordering::SeqCst), 1);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn lost_response_reports_unknown_outcome_without_retry() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let accepted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _: WireRequest = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
        // Simulate a daemon committing then losing the response.
        drop(stream);
    });
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        crate::node::NodeUuid::new_v4(),
        None,
    );
    let error = client
        .call_async(Command::Health, &budget)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    assert!(error.detail.contains("unknown outcome"));
    accepted.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

struct CancellableService {
    entered: AtomicUsize,
    cancelled: AtomicUsize,
}
impl LocalService for CancellableService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        for _ in 0..200 {
            if budget.cancellation.is_cancelled() {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                return Err(api_error(ErrorCode::Cancelled, "disconnected"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(api_error(
            ErrorCode::DeadlineExceeded,
            "work did not cancel",
        ))
    }
}

#[tokio::test]
async fn disconnect_cancels_handler_budget() {
    let service = Arc::new(CancellableService {
        entered: AtomicUsize::new(0),
        cancelled: AtomicUsize::new(0),
    });
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut stream = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut stream, &health_request(instance_id(&instance), "r"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.cancelled.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn client_cancellation_after_send_reports_unknown_outcome_promptly() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let clock = Arc::new(TestClock);
    let cancellation = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 2000),
        cancellation: cancellation.clone(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        crate::node::NodeUuid::new_v4(),
        None,
    );
    let call = tokio::spawn(async move { client.call_async(Command::Health, &budget).await });
    let (mut stream, _) = listener.accept().await.unwrap();
    let _ = read_frame(&mut stream).await.unwrap();
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_millis(300), call)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    assert!(error.detail.contains("unknown outcome"));
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn mismatched_kernel_uid_never_reaches_handler() {
    let (stream, _peer) = UnixStream::pair().unwrap();
    let uid = stream.peer_cred().unwrap().uid();
    let service = Arc::new(CountingService(
        AtomicUsize::new(0),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ));
    let error = serve_connection(
        stream,
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        service.clone(),
        Arc::new(TestClock),
        uid.wrapping_add(1),
        Cancellation::default(),
        Arc::new(Semaphore::new(5)),
        Arc::new(Semaphore::new(1)),
        Arc::new(LiveServiceGate::new()),
        Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
        Arc::new(Semaphore::new(1)),
        None,
        Arc::new(Semaphore::new(MAX_PEER_CONNECTIONS)),
        ConnectionGauge::default().enter(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(service.0.load(Ordering::SeqCst), 0);
}

struct HeldSearchService {
    searches: AtomicUsize,
    instance: String,
    boot: String,
}
impl LocalService for HeldSearchService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        command: Command,
        _: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if matches!(command, Command::Search(_)) {
            self.searches.fetch_add(1, Ordering::SeqCst);
            while !budget.cancellation.is_cancelled() && !budget.is_exhausted(&TestClock) {
                std::thread::sleep(Duration::from_millis(2));
            }
            return Err(api_error(ErrorCode::Cancelled, "search stopped"));
        }
        Ok(CommandResult::Health(Health::unknown(
            self.instance.clone(),
            self.boot.clone(),
            "test".into(),
            1,
        )))
    }
}

#[tokio::test]
async fn held_search_has_four_waiters_and_health_remains_available() {
    use crate::protocol::{commands::SearchQuery, pagination::PageRequest};
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let service = Arc::new(HeldSearchService {
        searches: AtomicUsize::new(0),
        instance: instance_id(&instance).to_string(),
        boot: listener.boot_id().to_string(),
    });
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut held = Vec::new();
    for n in 0..5 {
        let mut stream = UnixStream::connect(&path).await.unwrap();
        let request = WireRequest {
            version: PROTOCOL_VERSION,
            request_id: format!("search-{n}"),
            expected_instance: instance_id(&instance).to_string(),
            expected_boot: None,
            output: None,
            command: Command::Search(SearchQuery {
                literal: "needle".into(),
                thread: None,
                page: PageRequest::default(),
                max_candidates: 100,
            }),
        };
        write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        held.push(stream);
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.searches.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut excess = UnixStream::connect(&path).await.unwrap();
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "excess".into(),
        expected_instance: instance_id(&instance).to_string(),
        expected_boot: None,
        output: None,
        command: Command::Search(SearchQuery {
            literal: "needle".into(),
            thread: None,
            page: PageRequest::default(),
            max_candidates: 100,
        }),
    };
    write_frame(&mut excess, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    let response: WireResponse = serde_json::from_slice(
        &tokio::time::timeout(Duration::from_secs(1), read_frame(&mut excess))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.result.unwrap_err().code, ErrorCode::StoreBusy);
    let mut health = UnixStream::connect(&path).await.unwrap();
    write_frame(
        &mut health,
        &health_request(instance_id(&instance), "health"),
    )
    .await
    .unwrap();
    let response: WireResponse = serde_json::from_slice(
        &tokio::time::timeout(Duration::from_secs(1), read_frame(&mut health))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(response.result, Ok(CommandResult::Health(_))));
    assert_eq!(service.searches.load(Ordering::SeqCst), 1);
    drop(held);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn thirty_third_idle_connection_is_refused_without_handler_work() {
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let service = Arc::new(CountingService(
        AtomicUsize::new(0),
        instance_id(&instance).to_string(),
        listener.boot_id().to_string(),
    ));
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut held = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        held.push(UnixStream::connect(&path).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut excess = UnixStream::connect(&path).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), read_frame(&mut excess))
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(service.0.load(Ordering::SeqCst), 0);
    drop(held);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct PausedService {
    entered: AtomicUsize,
    release: std::sync::atomic::AtomicBool,
}
impl LocalService for PausedService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(2));
        }
        Err(api_error(ErrorCode::Cancelled, "released"))
    }
}

#[tokio::test]
async fn disconnected_search_retains_active_admission_until_worker_exits() {
    use crate::protocol::{commands::SearchQuery, pagination::PageRequest};
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let service = Arc::new(PausedService {
        entered: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let search = |request_id: &str| WireRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.into(),
        expected_instance: instance_id(&instance).to_string(),
        expected_boot: None,
        output: None,
        command: Command::Search(SearchQuery {
            literal: "needle".into(),
            thread: None,
            page: PageRequest::default(),
            max_candidates: 100,
        }),
    };
    let mut first = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut first, &serde_json::to_vec(&search("first")).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    drop(first);
    let mut second = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut second, &serde_json::to_vec(&search("second")).unwrap())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        service.entered.load(Ordering::SeqCst),
        1,
        "second search entered while first worker still ran"
    );
    service.release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    drop(second);
    shutdown.cancel();
    let outcome = server.await.unwrap().unwrap();
    assert!(matches!(outcome, ServeOutcome::Drained));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn shutdown_interrupts_incomplete_frame_and_reports_unfinished_worker() {
    use crate::daemon::ownership::OwnerLock;
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let service = Arc::new(PausedService {
        entered: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut incomplete = UnixStream::connect(&path).await.unwrap();
    incomplete.write_all(&4_u32.to_be_bytes()).await.unwrap();
    incomplete.write_all(b"ab").await.unwrap();
    let mut deciding = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut deciding, &health_request(instance_id(&instance), "r"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ServeOutcome::Incomplete(mut pending) = outcome else {
        panic!("shutdown reported drained with a worker still deciding");
    };
    assert_eq!(pending.remaining(), 1);
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    service.release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), pending.wait())
        .await
        .unwrap()
        .unwrap();
    drop(pending);
    drop(OwnerLock::acquire_with_test_hint(&instance).unwrap());
    drop(incomplete);
    drop(deciding);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct OversizedResultService;
impl LocalService for OversizedResultService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(ApiError::unsupported("x".repeat(MAX_FRAME_BYTES + 1)))
    }
}

#[tokio::test]
async fn oversized_result_returns_bounded_typed_error() {
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        Arc::new(OversizedResultService),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut stream = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut stream, &health_request(instance_id(&instance), "r"))
        .await
        .unwrap();
    let bytes = read_frame(&mut stream).await.unwrap();
    assert!(bytes.len() < 512);
    let response: WireResponse = serde_json::from_slice(&bytes).unwrap();
    let error = response.result.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidBudget);
    assert!(error.required_minimum_bytes.unwrap() as usize > MAX_WIRE_FRAME_BYTES);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn incomplete_real_socket_frame_is_pre_submission_error() {
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    let clock = Arc::new(TestClock);
    let client = crate::client::local::LocalSocketClient::new(
        std::path::PathBuf::new(),
        clock.clone(),
        crate::node::NodeUuid::new_v4(),
        None,
    );
    let cancellation = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 2000),
        cancellation: cancellation.clone(),
    };
    let body = vec![b'x'; MAX_FRAME_BYTES];
    let frame_len = 4 + body.len();
    let writing =
        tokio::spawn(async move { client.write_request(&mut writer, &body, &budget).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !writing.is_finished(),
        "unread Unix socket must block before the complete frame is sent"
    );
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_millis(300), writing)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    let mut received = Vec::new();
    reader.read_to_end(&mut received).await.unwrap();
    assert!(
        received.len() > 4,
        "test must exercise a partial body write"
    );
    assert!(received.len() < frame_len, "complete frame could dispatch");
}

#[tokio::test]
async fn drain_wait_keeps_owner_lease_after_an_earlier_task_error() {
    use crate::daemon::ownership::OwnerLock;
    let (listener, path, root, instance) = guarded_listener();
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let gate = release.clone();
    let mut tasks = JoinSet::new();
    tasks.spawn(async { Err(io::Error::other("earlier client failed")) });
    tasks.spawn(async move {
        while !gate.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        Ok(())
    });
    let mut pending = PendingDrain {
        _listener: listener,
        first_error: None,
        tasks,
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(30), pending.wait())
            .await
            .is_err()
    );
    assert_eq!(
        OwnerLock::acquire_with_test_hint(&instance)
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    release.store(true, Ordering::SeqCst);
    assert!(pending.wait().await.is_err());
    drop(pending);
    drop(OwnerLock::acquire_with_test_hint(&instance).unwrap());
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn client_rejects_oversized_request_before_submission() {
    use crate::protocol::{commands::SearchQuery, pagination::PageRequest};
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        crate::node::NodeUuid::new_v4(),
        None,
    );
    let command = Command::Search(SearchQuery {
        literal: "x".repeat(MAX_FRAME_BYTES + 1),
        thread: None,
        page: PageRequest::default(),
        max_candidates: 100,
    });
    let error = client.call_async(command, &budget).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    drop(listener);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn timed_out_search_keeps_active_permit_until_sync_worker_exits() {
    use crate::protocol::{commands::SearchQuery, pagination::PageRequest};
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let service = Arc::new(PausedService {
        entered: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let search = |request_id: &str| WireRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.into(),
        expected_instance: instance_id(&instance).to_string(),
        expected_boot: None,
        output: None,
        command: Command::Search(SearchQuery {
            literal: "needle".into(),
            thread: None,
            page: PageRequest::default(),
            max_candidates: 100,
        }),
    };
    let mut first = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut first, &serde_json::to_vec(&search("first")).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(ORDINARY_TIMEOUT + Duration::from_millis(100)).await;
    let mut second = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut second, &serde_json::to_vec(&search("second")).unwrap())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        service.entered.load(Ordering::SeqCst),
        1,
        "timeout released the live search worker's permit"
    );
    service.release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    drop(first);
    drop(second);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn another_instance_request_never_reaches_handler() {
    let (listener, path, root, instance) = guarded_listener();
    let expected = instance_id(&instance);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let service = Arc::new(CountingService(
        AtomicUsize::new(0),
        expected.to_string(),
        boot.to_string(),
    ));
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        expected,
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut wrong = UnixStream::connect(&path).await.unwrap();
    write_frame(
        &mut wrong,
        &health_request(crate::node::NodeUuid::new_v4(), "wrong"),
    )
    .await
    .unwrap();
    let mismatch: serde_json::Value =
        serde_json::from_slice(&read_frame(&mut wrong).await.unwrap()).unwrap();
    assert_eq!(mismatch["request_id"], "wrong");
    assert_eq!(mismatch["instance"], expected.to_string());
    assert_eq!(mismatch["daemon_boot"], boot.to_string());
    assert_eq!(mismatch["result"]["Err"]["code"], "instance_mismatch");
    assert_eq!(service.0.load(Ordering::SeqCst), 0);

    let mut right = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut right, &health_request(expected, "right"))
        .await
        .unwrap();
    let response: WireResponse =
        serde_json::from_slice(&read_frame(&mut right).await.unwrap()).unwrap();
    assert_eq!(response.instance, expected.to_string());
    assert_eq!(response.daemon_boot, boot.to_string());
    assert_eq!(service.0.load(Ordering::SeqCst), 1);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn client_rejects_wrong_boot_after_submission_without_retry() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let instance = crate::node::NodeUuid::new_v4();
    let expected_boot = uuid::Uuid::new_v4();
    let actual_boot = uuid::Uuid::new_v4();
    let accepted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance: instance.to_string(),
            daemon_boot: actual_boot.to_string(),
            result: Err(api_error(ErrorCode::Unauthorized, "server rejected")),
        };
        write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
            .await
            .unwrap();
    });
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        instance,
        Some(expected_boot),
    );
    let error = client
        .call_async(Command::Health, &budget)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    accepted.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn client_rejects_wrong_response_instance_after_submission() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let expected_instance = crate::node::NodeUuid::new_v4();
    let other_instance = crate::node::NodeUuid::new_v4();
    let boot = uuid::Uuid::new_v4();
    let accepted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
        assert_eq!(request.expected_instance, expected_instance.to_string());
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance: other_instance.to_string(),
            daemon_boot: boot.to_string(),
            result: Err(api_error(ErrorCode::StoreBusy, "other daemon")),
        };
        write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
            .await
            .unwrap();
    });
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        expected_instance,
        Some(boot),
    );
    let error = client
        .call_async(Command::Health, &budget)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    accepted.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn client_rejects_malformed_boot_even_without_expected_boot() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let instance = crate::node::NodeUuid::new_v4();
    let accepted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance: instance.to_string(),
            daemon_boot: "not-a-uuid".into(),
            result: Err(api_error(ErrorCode::StoreBusy, "forged boot")),
        };
        write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
            .await
            .unwrap();
    });
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client = crate::client::local::LocalSocketClient::new(path.clone(), clock, instance, None);
    let error = client
        .call_async(Command::Health, &budget)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    accepted.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn correlated_server_rejection_preserves_its_error_code() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let instance = crate::node::NodeUuid::new_v4();
    let boot = uuid::Uuid::new_v4();
    let accepted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance: instance.to_string(),
            daemon_boot: boot.to_string(),
            result: Err(api_error(ErrorCode::StoreBusy, "writer queue full")),
        };
        write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
            .await
            .unwrap();
    });
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client =
        crate::client::local::LocalSocketClient::new(path.clone(), clock, instance, Some(boot));
    let error = client
        .call_async(Command::Health, &budget)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreBusy);
    assert_eq!(error.detail, "writer queue full");
    accepted.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

fn service_registration(
    instance: crate::node::NodeUuid,
    request_id: &str,
) -> crate::protocol::service::ServiceWireRequest {
    use crate::protocol::service::*;
    ServiceWireRequest {
        version: PROTOCOL_VERSION,
        request_id: request_id.into(),
        expected_instance: instance.to_string(),
        service: ServiceRequest::Register(ServiceRegister {
            capability: SERVICE_SESSION_CAPABILITY.into(),
        }),
    }
}

async fn service_call(
    stream: &mut UnixStream,
    request: &crate::protocol::service::ServiceWireRequest,
) -> crate::protocol::service::ServiceWireResponse {
    write_frame(stream, &serde_json::to_vec(request).unwrap())
        .await
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(2), read_frame(stream))
        .await
        .unwrap()
        .unwrap();
    let reply: crate::protocol::service::ServiceWireResponse =
        serde_json::from_slice(&body).unwrap();
    assert!(reply.correlates_to(request));
    reply
}

#[tokio::test]
async fn registered_socket_is_exclusive_idle_and_leaves_ordinary_clients_progressing() {
    use crate::protocol::service::ServiceResult;
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id().to_string();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let handler = Arc::new(CountingService(
        AtomicUsize::new(0),
        instance.to_string(),
        boot,
    ));
    let server = tokio::spawn(serve(
        listener,
        instance,
        handler.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut first = UnixStream::connect(&path).await.unwrap();
    let reply = service_call(&mut first, &service_registration(instance, "first")).await;
    let first_generation = match reply.result.unwrap() {
        ServiceResult::Registered(value) => value.connection_generation,
        _ => panic!("wrong registration result"),
    };
    let mut second = UnixStream::connect(&path).await.unwrap();
    let busy = service_call(&mut second, &service_registration(instance, "second")).await;
    assert_eq!(busy.result.unwrap_err().code, ErrorCode::ServiceBusy);
    let mut ordinary = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut ordinary, &health_request(instance, "health"))
        .await
        .unwrap();
    let health: WireResponse =
        serde_json::from_slice(&read_frame(&mut ordinary).await.unwrap()).unwrap();
    assert!(matches!(health.result, Ok(CommandResult::Health(_))));
    assert_eq!(handler.0.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(5100)).await;
    let mut still_busy = UnixStream::connect(&path).await.unwrap();
    assert_eq!(
        service_call(
            &mut still_busy,
            &service_registration(instance, "idle-busy")
        )
        .await
        .result
        .unwrap_err()
        .code,
        ErrorCode::ServiceBusy
    );
    drop(first);
    let mut successor = UnixStream::connect(&path).await.unwrap();
    let mut recovered = None;
    for _ in 0..50 {
        let reply =
            service_call(&mut successor, &service_registration(instance, "successor")).await;
        if let Ok(ServiceResult::Registered(value)) = reply.result {
            recovered = Some(value.connection_generation);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        successor = UnixStream::connect(&path).await.unwrap();
    }
    assert!(recovered.unwrap() > first_generation);
    shutdown.cancel();
    let outcome = server.await.unwrap().unwrap();
    assert!(matches!(outcome, ServeOutcome::Drained));
    drop(successor);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn registered_socket_rejects_malformed_and_oversized_next_frames_and_releases_slot() {
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id().to_string();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let handler = Arc::new(CountingService(
        AtomicUsize::new(0),
        instance.to_string(),
        boot,
    ));
    let server = tokio::spawn(serve(
        listener,
        instance,
        handler,
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut unsupported = UnixStream::connect(&path).await.unwrap();
    let mut request = service_registration(instance, "unsupported-capability");
    request.service = crate::protocol::service::ServiceRequest::Register(
        crate::protocol::service::ServiceRegister {
            capability: "future-service".into(),
        },
    );
    assert_eq!(
        service_call(&mut unsupported, &request)
            .await
            .result
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    for bad in [
        vec![b'x'],
        ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec(),
    ] {
        let mut stream = UnixStream::connect(&path).await.unwrap();
        service_call(&mut stream, &service_registration(instance, "register")).await;
        if bad.len() == 1 {
            write_frame(&mut stream, &bad).await.unwrap();
        } else {
            stream.write_all(&bad).await.unwrap();
        }
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    let mut stream = UnixStream::connect(&path).await.unwrap();
    assert!(
        service_call(&mut stream, &service_registration(instance, "after-bad"))
            .await
            .result
            .is_ok()
    );
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    drop(stream);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn partial_service_frame_expires_without_holding_ordinary_health() {
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let handler = Arc::new(CountingService(
        AtomicUsize::new(0),
        instance.to_string(),
        listener.boot_id().to_string(),
    ));
    let server = tokio::spawn(serve(
        listener,
        instance,
        handler.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut service = UnixStream::connect(&path).await.unwrap();
    service_call(&mut service, &service_registration(instance, "register")).await;
    service.write_all(&[0]).await.unwrap();
    let mut ordinary = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut ordinary, &health_request(instance, "during-partial"))
        .await
        .unwrap();
    let health: WireResponse = serde_json::from_slice(
        &tokio::time::timeout(Duration::from_secs(1), read_frame(&mut ordinary))
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(health.result, Ok(CommandResult::Health(_))));
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(6), service.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let mut replacement = UnixStream::connect(&path).await.unwrap();
    assert!(
        service_call(
            &mut replacement,
            &service_registration(instance, "replacement")
        )
        .await
        .result
        .is_ok()
    );
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    drop(replacement);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

async fn shutdown_during_registered_partial_frame(partial_body: bool) {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::{ids::ServiceAuthorId, service::ServiceResult};
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id().to_string();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let gate = Arc::new(LiveServiceGate::new());
    let server = tokio::spawn(serve_with_gate(
        listener,
        instance,
        Arc::new(RejectOtherService),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
        gate.clone(),
        None,
        ConnectionGauge::default(),
    ));
    let mut registered = UnixStream::connect(&path).await.unwrap();
    let initial = service_call(&mut registered, &service_registration(instance, "register")).await;
    let generation = match initial.result.unwrap() {
        ServiceResult::Registered(registration) => registration.connection_generation,
        _ => panic!("expected registration"),
    };
    if partial_body {
        let body = serde_json::to_vec(&decision_request(instance)).unwrap();
        registered
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        registered.write_all(&body[..body.len() / 2]).await.unwrap();
    } else {
        registered.write_all(&[0]).await.unwrap();
    }
    // The peer has submitted the incomplete frame, and a separate accepted
    // connection proves the registered owner remains live before cancellation.
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut contender = UnixStream::connect(&path).await.unwrap();
    assert_eq!(
        service_call(&mut contender, &service_registration(instance, "contender"))
            .await
            .result
            .unwrap_err()
            .code,
        ErrorCode::ServiceBusy,
    );
    shutdown.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(outcome, ServeOutcome::Drained),
        "partial service frame delayed shutdown drain"
    );
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), registered.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let successor = gate
        .register(&instance.to_string(), &boot, ServiceAuthorId::new("graph"))
        .unwrap();
    assert!(successor.generation() > generation);
    assert!(gate.revoke_exact(&successor));
    drop((registered, contender));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn shutdown_releases_registered_partial_prefix_promptly() {
    shutdown_during_registered_partial_frame(false).await;
}

#[tokio::test]
async fn shutdown_releases_registered_partial_body_promptly() {
    shutdown_during_registered_partial_frame(true).await;
}

#[tokio::test]
async fn separate_instances_accept_independent_service_connections() {
    let (first_listener, first_path, first_root, first_paths) = guarded_listener();
    let (second_listener, second_path, second_root, second_paths) = guarded_listener();
    let first_instance = instance_id(&first_paths);
    let second_instance = instance_id(&second_paths);
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let first_shutdown = Cancellation::default();
    let second_shutdown = Cancellation::default();
    let first = tokio::spawn(serve(
        first_listener,
        first_instance,
        Arc::new(RejectOtherService),
        Arc::new(TestClock),
        uid,
        first_shutdown.clone(),
    ));
    let second = tokio::spawn(serve(
        second_listener,
        second_instance,
        Arc::new(RejectOtherService),
        Arc::new(TestClock),
        uid,
        second_shutdown.clone(),
    ));
    let mut first_client = UnixStream::connect(&first_path).await.unwrap();
    let mut second_client = UnixStream::connect(&second_path).await.unwrap();
    assert!(
        service_call(
            &mut first_client,
            &service_registration(first_instance, "first")
        )
        .await
        .result
        .is_ok()
    );
    assert!(
        service_call(
            &mut second_client,
            &service_registration(second_instance, "second")
        )
        .await
        .result
        .is_ok()
    );
    first_shutdown.cancel();
    second_shutdown.cancel();
    assert!(matches!(
        first.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    assert!(matches!(
        second.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    drop((first_client, second_client));
    std::fs::remove_file(first_path).unwrap();
    std::fs::remove_file(second_path).unwrap();
    std::fs::remove_dir_all(first_root).unwrap();
    std::fs::remove_dir_all(second_root).unwrap();
}

#[tokio::test]
async fn restart_starts_with_empty_live_slot_and_same_reserved_author() {
    use crate::daemon::ownership::OwnerLock;
    use crate::protocol::service::ServiceResult;
    let (listener, old_path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let old_boot = listener.boot_id().to_string();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(RejectOtherService),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let mut old_client = UnixStream::connect(&old_path).await.unwrap();
    let first = service_call(
        &mut old_client,
        &service_registration(instance, "before-restart"),
    )
    .await;
    let first_author = match first.result.unwrap() {
        ServiceResult::Registered(value) => value.author,
        _ => panic!("wrong response"),
    };
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    drop(old_client);
    let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
    let bound = owner.bind_socket().unwrap();
    let new_path = bound.path().to_owned();
    let new_boot = bound.boot_id().to_string();
    let listener = bound.into_async().unwrap();
    drop(owner);
    let new_shutdown = Cancellation::default();
    let new_server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(RejectOtherService),
        Arc::new(TestClock),
        uid,
        new_shutdown.clone(),
    ));
    let mut new_client = UnixStream::connect(&new_path).await.unwrap();
    let second = service_call(
        &mut new_client,
        &service_registration(instance, "after-restart"),
    )
    .await;
    let registration = match second.result.unwrap() {
        ServiceResult::Registered(value) => value,
        _ => panic!("wrong response"),
    };
    assert_ne!(new_boot, old_boot);
    assert_eq!(registration.daemon_boot, new_boot);
    assert_eq!(registration.connection_generation, 1);
    assert_eq!(registration.author, first_author);
    new_shutdown.cancel();
    assert!(matches!(
        new_server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    drop(new_client);
    // The restarted boot reclaimed and rebound the same stable pathname.
    assert_eq!(old_path, new_path);
    std::fs::remove_file(new_path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn late_generation_cleanup_cannot_revoke_successor() {
    use crate::ports::ServiceAuthorityGate;
    let gate = LiveServiceGate::new();
    let author = crate::protocol::ids::ServiceAuthorId::new("graph");
    let old = gate.register("instance", "boot", author.clone()).unwrap();
    assert!(gate.revoke_exact(&old));
    let next = gate.register("instance", "boot", author).unwrap();
    assert!(next.generation() > old.generation());
    assert!(!gate.revoke_exact(&old));
    assert!(gate.revoke_exact(&next));
}

struct DecisionBarrierService {
    database: std::path::PathBuf,
    before_decision: bool,
    stage: AtomicUsize,
    release: std::sync::atomic::AtomicBool,
}
impl LocalService for DecisionBarrierService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        unreachable!("barrier fixture accepts only service operations")
    }

    fn service_operation(
        &self,
        _: crate::protocol::service::ServiceOperation,
        connection: &crate::ports::ServiceConnectionAuthority,
        gate: &dyn crate::ports::ServiceAuthorityGate,
        _: &CallBudget,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        use crate::ports::{ServiceDecisionStartError, ServiceDecisionTransaction};
        let mut db = rusqlite::Connection::open(&self.database).unwrap();
        if self.before_decision {
            self.stage.store(1, Ordering::SeqCst);
        }
        let wait = || {
            while !self.release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        if self.before_decision {
            wait();
        }
        let transaction = match ServiceDecisionTransaction::begin(&mut db, gate, connection) {
            Ok(transaction) => transaction,
            Err(ServiceDecisionStartError::Authority(error)) => {
                self.stage.store(4, Ordering::SeqCst);
                return Err(error);
            }
            Err(ServiceDecisionStartError::Database(error)) => {
                panic!("barrier SQLite error: {error}")
            }
        };
        transaction
            .transaction()
            .execute("INSERT INTO decisions DEFAULT VALUES", [])
            .unwrap();
        if !self.before_decision {
            self.stage.store(2, Ordering::SeqCst);
            wait();
        }
        transaction.commit().unwrap();
        self.stage.store(3, Ordering::SeqCst);
        Err(api_error(ErrorCode::Unsupported, "test decision committed"))
    }
}

fn decision_request(
    instance: crate::node::NodeUuid,
) -> crate::protocol::service::ServiceWireRequest {
    use crate::protocol::{
        ids::{OperationId, ThreadId},
        service::{EnsureManagedThread, ServiceOperation, ServiceRequest},
    };
    crate::protocol::service::ServiceWireRequest {
        version: PROTOCOL_VERSION,
        request_id: "decide".into(),
        expected_instance: instance.to_string(),
        service: ServiceRequest::Operation(ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: ThreadId::new("decision-thread"),
            topic: "test".into(),
            goal: "test".into(),
            operation: OperationId::new("decision-key"),
        })),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_before_guard_rejects_but_disconnect_after_guard_preserves_commit() {
    for before_decision in [true, false] {
        let (listener, path, root, paths) = guarded_listener();
        let instance = instance_id(&paths);
        let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
        let database = root.join("decisions.db");
        rusqlite::Connection::open(&database)
            .unwrap()
            .execute_batch("CREATE TABLE decisions (id INTEGER PRIMARY KEY)")
            .unwrap();
        let service = Arc::new(DecisionBarrierService {
            database: database.clone(),
            before_decision,
            stage: AtomicUsize::new(0),
            release: std::sync::atomic::AtomicBool::new(false),
        });
        let shutdown = Cancellation::default();
        let server = tokio::spawn(serve(
            listener,
            instance,
            service.clone(),
            Arc::new(TestClock),
            uid,
            shutdown.clone(),
        ));
        let mut client = UnixStream::connect(&path).await.unwrap();
        assert!(
            service_call(&mut client, &service_registration(instance, "register"))
                .await
                .result
                .is_ok()
        );
        write_frame(
            &mut client,
            &serde_json::to_vec(&decision_request(instance)).unwrap(),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while service.stage.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        drop(client);
        let mut contender = UnixStream::connect(&path).await.unwrap();
        if before_decision {
            let mut registered = false;
            for attempt in 0..50 {
                let reply = service_call(
                    &mut contender,
                    &service_registration(instance, &format!("contender-{attempt}")),
                )
                .await;
                if reply.result.is_ok() {
                    registered = true;
                    break;
                }
                contender = UnixStream::connect(&path).await.unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(
                registered,
                "EOF must revoke before the paused decision resumes"
            );
        } else {
            let reply =
                service_call(&mut contender, &service_registration(instance, "contender")).await;
            assert_eq!(reply.result.unwrap_err().code, ErrorCode::ServiceBusy);
        }
        service.release.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(2), async {
            while service.stage.load(Ordering::SeqCst) != if before_decision { 4 } else { 3 } {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let count: i64 = rusqlite::Connection::open(&database)
            .unwrap()
            .query_row("SELECT count(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, if before_decision { 0 } else { 1 });
        shutdown.cancel();
        assert!(matches!(
            server.await.unwrap().unwrap(),
            ServeOutcome::Drained
        ));
        drop(contender);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_retains_guarded_service_work_until_commit_and_exact_revoke() {
    use crate::ports::ServiceAuthorityGate;
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id().to_string();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let database = root.join("shutdown-decisions.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE decisions (id INTEGER PRIMARY KEY)")
        .unwrap();
    let service = Arc::new(DecisionBarrierService {
        database: database.clone(),
        before_decision: false,
        stage: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let gate = Arc::new(LiveServiceGate::new());
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve_with_gate(
        listener,
        instance,
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
        gate.clone(),
        None,
        ConnectionGauge::default(),
    ));
    let mut client = UnixStream::connect(&path).await.unwrap();
    assert!(
        service_call(&mut client, &service_registration(instance, "register"))
            .await
            .result
            .is_ok()
    );
    write_frame(
        &mut client,
        &serde_json::to_vec(&decision_request(instance)).unwrap(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.stage.load(Ordering::SeqCst) != 2 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ServeOutcome::Incomplete(mut pending) = outcome else {
        panic!("shutdown must retain the still-deciding service worker");
    };
    assert_eq!(pending.remaining(), 1);
    service.release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(2), pending.wait())
        .await
        .unwrap()
        .unwrap();
    drop(pending);
    assert_eq!(service.stage.load(Ordering::SeqCst), 3);
    let count: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT count(*) FROM decisions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let successor = gate
        .register(
            &instance.to_string(),
            &boot,
            crate::protocol::ids::ServiceAuthorId::new("graph"),
        )
        .unwrap();
    assert!(gate.revoke_exact(&successor));
    drop(client);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct LargeCompletedService {
    database: std::path::PathBuf,
    completed: std::sync::atomic::AtomicBool,
}

impl LargeCompletedService {
    fn response() -> crate::protocol::service::ServiceResult {
        use crate::protocol::{
            ids::{MessageId, ServiceAuthorId, ThreadId},
            results::{MessageKind, MessageSummary},
            service::{ServiceNotification, ServiceResult},
        };
        ServiceResult::Notification(ServiceNotification {
            summary: MessageSummary {
                message: MessageId::new("large-response"),
                thread: ThreadId::new("decision-thread"),
                author: None,
                event_author: None,
                author_role: None,
                relays_user: false,
                user_intent: None,
                author_role_backfilled: false,
                kind: MessageKind::Info,
                sequence: 1,
                created_at: UtcMillis(0),
                actor_label: Some("graph".into()),
                preview_data: "x".repeat(MAX_FRAME_BYTES - 4_096),
                preview_omitted: false,
                preview_detail_argv: None,
            },
            author: ServiceAuthorId::new("graph"),
        })
    }
}

impl LocalService for LargeCompletedService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        unreachable!("large response fixture accepts only service operations")
    }

    fn service_operation(
        &self,
        _: crate::protocol::service::ServiceOperation,
        connection: &crate::ports::ServiceConnectionAuthority,
        gate: &dyn crate::ports::ServiceAuthorityGate,
        _: &CallBudget,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        let mut db = rusqlite::Connection::open(&self.database).unwrap();
        let transaction =
            crate::ports::ServiceDecisionTransaction::begin(&mut db, gate, connection).unwrap();
        transaction
            .transaction()
            .execute("INSERT INTO decisions DEFAULT VALUES", [])
            .unwrap();
        transaction.commit().unwrap();
        self.completed.store(true, Ordering::SeqCst);
        Ok(Self::response())
    }
}

fn queued_peer_bytes(stream: &UnixStream) -> usize {
    use std::os::fd::AsRawFd;
    let mut bytes: libc::c_int = 0;
    let rc = unsafe { libc::ioctl(stream.as_raw_fd(), libc::FIONREAD, &mut bytes) };
    assert_eq!(rc, 0, "FIONREAD failed: {}", io::Error::last_os_error());
    bytes as usize
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_interrupts_blocked_registered_response_after_commit() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::{
        ids::ServiceAuthorId,
        service::{ServiceResult, ServiceWireResponse},
    };
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id().to_string();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let database = root.join("large-response.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE decisions (id INTEGER PRIMARY KEY)")
        .unwrap();
    let service = Arc::new(LargeCompletedService {
        database: database.clone(),
        completed: std::sync::atomic::AtomicBool::new(false),
    });
    let gate = Arc::new(LiveServiceGate::new());
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve_with_gate(
        listener,
        instance,
        service.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
        gate.clone(),
        None,
        ConnectionGauge::default(),
    ));
    let mut client = UnixStream::connect(&path).await.unwrap();
    let registered = service_call(&mut client, &service_registration(instance, "register")).await;
    let generation = match registered.result.unwrap() {
        ServiceResult::Registered(value) => value.connection_generation,
        _ => panic!("expected registration"),
    };
    let request = decision_request(instance);
    let expected_response = ServiceWireResponse {
        version: PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        instance: instance.to_string(),
        daemon_boot: boot.clone(),
        result: Ok(LargeCompletedService::response()),
    };
    let encoded = encode_json(&expected_response).unwrap();
    assert!(encoded.len() > 900_000 && encoded.len() <= MAX_FRAME_BYTES);
    let decoded: ServiceWireResponse = serde_json::from_slice(&encoded).unwrap();
    assert!(decoded.correlates_to(&request));
    write_frame(&mut client, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !service.completed.load(Ordering::SeqCst) || queued_peer_bytes(&client) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let queued = queued_peer_bytes(&client);
    assert!(
        queued > 0 && queued < encoded.len() + 4,
        "response must be partially delivered: {queued} of {} bytes",
        encoded.len() + 4
    );
    shutdown.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(outcome, ServeOutcome::Drained),
        "completed operation must not hold the shutdown drain in a blocked write"
    );
    let mut delivered = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), client.read_to_end(&mut delivered))
        .await
        .unwrap()
        .unwrap();
    assert!(
        delivered.len() < encoded.len() + 4,
        "partial response was unexpectedly completed"
    );
    let count: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT count(*) FROM decisions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        count, 1,
        "response loss cannot roll back or repeat the committed operation"
    );
    let successor = gate
        .register(&instance.to_string(), &boot, ServiceAuthorId::new("graph"))
        .unwrap();
    assert!(successor.generation() > generation);
    assert!(gate.revoke_exact(&successor));
    drop(client);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct RecoveryDomain {
    decision: Arc<DecisionBarrierService>,
    fail_audit: bool,
    audits: AtomicUsize,
}

impl LocalService for RecoveryDomain {
    crate::unserved_local_service_routes!(service_control, handle_with_output);
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        unreachable!("recovery fixture only handles registered operations")
    }

    fn service_operation(
        &self,
        operation: crate::protocol::service::ServiceOperation,
        connection: &crate::ports::ServiceConnectionAuthority,
        gate: &dyn crate::ports::ServiceAuthorityGate,
        budget: &CallBudget,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        self.decision
            .service_operation(operation, connection, gate, budget)
    }

    fn audit_service_disconnect(
        &self,
        _: &str,
        _: u64,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        self.audits.fetch_add(1, Ordering::SeqCst);
        if self.fail_audit {
            Err(api_error(ErrorCode::StoreBusy, "audit writer unavailable"))
        } else {
            Ok(())
        }
    }
}

async fn recovery_call(
    path: &std::path::Path,
    instance: crate::node::NodeUuid,
    command: Command,
) -> WireResponse {
    let mut stream = UnixStream::connect(path).await.unwrap();
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "recovery".into(),
        expected_instance: instance.to_string(),
        expected_boot: None,
        output: None,
        command,
    };
    write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    let response: WireResponse =
        serde_json::from_slice(&read_frame(&mut stream).await.unwrap()).unwrap();
    assert!(response.correlates_to(&request, None));
    response
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_disconnect_is_generation_targeted_and_reports_audit_failure() {
    use crate::daemon::{
        control::{ControlService, StopController},
        health::HealthInputs,
    };
    use crate::protocol::{
        commands::ServiceDisconnectRequest, results::ServiceRecoveryAudit, service::ServiceResult,
    };
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let database = root.join("recovery-decisions.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE decisions (id INTEGER PRIMARY KEY)")
        .unwrap();
    let decision = Arc::new(DecisionBarrierService {
        database: database.clone(),
        before_decision: true,
        stage: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let domain = RecoveryDomain {
        decision: decision.clone(),
        fail_audit: true,
        audits: AtomicUsize::new(0),
    };
    let shutdown = Cancellation::default();
    let handler = Arc::new(ControlService::new(
        StopController::new(instance, boot, shutdown.clone()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        domain,
    ));
    let gate = Arc::new(LiveServiceGate::new());
    let server = tokio::spawn(serve_with_gate(
        listener,
        instance,
        handler.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
        gate.clone(),
        None,
        ConnectionGauge::default(),
    ));
    let mut registered = UnixStream::connect(&path).await.unwrap();
    let initial = service_call(&mut registered, &service_registration(instance, "register")).await;
    let generation = match initial.result.unwrap() {
        ServiceResult::Registered(value) => value.connection_generation,
        _ => panic!("expected registration"),
    };
    let inspected = recovery_call(&path, instance, Command::ServiceInspect)
        .await
        .result
        .unwrap();
    let CommandResult::ServiceInspection(observation) = inspected else {
        panic!("expected inspection")
    };
    assert_eq!(observation.instance, instance.to_string());
    assert_eq!(observation.daemon_boot, boot.to_string());
    assert!(observation.connected);
    assert_eq!(observation.connection_generation, Some(generation));
    assert_eq!(observation.registered_at, Some(UtcMillis(0)));
    assert!(serde_json::to_vec(&observation).unwrap().len() < 512);
    for (expected_boot, expected_generation) in [
        (uuid::Uuid::new_v4().to_string(), generation),
        (boot.to_string(), generation + 1),
    ] {
        let error = recovery_call(
            &path,
            instance,
            Command::ServiceDisconnect(ServiceDisconnectRequest {
                expected_boot,
                expected_generation,
            }),
        )
        .await
        .result
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::StaleServiceGeneration);
        assert!(
            gate.inspect(&instance.to_string(), &boot.to_string())
                .connected
        );
    }
    write_frame(
        &mut registered,
        &serde_json::to_vec(&decision_request(instance)).unwrap(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while decision.stage.load(Ordering::SeqCst) != 1 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let disconnected = recovery_call(
        &path,
        instance,
        Command::ServiceDisconnect(ServiceDisconnectRequest {
            expected_boot: boot.to_string(),
            expected_generation: generation,
        }),
    )
    .await
    .result
    .unwrap();
    let CommandResult::ServiceDisconnected(result) = disconnected else {
        panic!("expected disconnect result")
    };
    assert!(result.disconnected);
    assert_eq!(result.connection_generation, generation);
    assert!(matches!(
        result.audit,
        ServiceRecoveryAudit::Failed(ApiError {
            code: ErrorCode::StoreBusy,
            ..
        })
    ));
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), registered.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0,
        "revocation closes the socket while predecision work is still paused"
    );
    decision.release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), async {
        while decision.stage.load(Ordering::SeqCst) != 4 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let count: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT count(*) FROM decisions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let after = recovery_call(&path, instance, Command::ServiceInspect)
        .await
        .result
        .unwrap();
    let CommandResult::ServiceInspection(after) = after else {
        panic!("expected inspection")
    };
    assert!(!after.connected);
    assert_eq!(after.connection_generation, Some(generation));
    let stale = recovery_call(
        &path,
        instance,
        Command::ServiceDisconnect(ServiceDisconnectRequest {
            expected_boot: boot.to_string(),
            expected_generation: generation,
        }),
    )
    .await
    .result
    .unwrap_err();
    assert_eq!(stale.code, ErrorCode::StaleServiceGeneration);
    let mut successor = UnixStream::connect(&path).await.unwrap();
    let new = service_call(&mut successor, &service_registration(instance, "successor")).await;
    let next_generation = match new.result.unwrap() {
        ServiceResult::Registered(value) => value.connection_generation,
        _ => panic!("expected successor"),
    };
    assert!(next_generation > generation);
    assert!(!gate.disconnect(&instance.to_string(), &boot.to_string(), generation));
    assert!(
        gate.inspect(&instance.to_string(), &boot.to_string())
            .connected
    );
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    drop(successor);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct CommittedWaitDomain {
    database: std::path::PathBuf,
    committed: Arc<std::sync::atomic::AtomicBool>,
    release: Arc<std::sync::atomic::AtomicBool>,
    audits: Arc<AtomicUsize>,
}

impl LocalService for CommittedWaitDomain {
    crate::unserved_local_service_routes!(service_control, handle_with_output);
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        unreachable!("registered operation only")
    }

    fn service_operation(
        &self,
        _: crate::protocol::service::ServiceOperation,
        connection: &crate::ports::ServiceConnectionAuthority,
        gate: &dyn crate::ports::ServiceAuthorityGate,
        _: &CallBudget,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        use crate::ports::ServiceDecisionTransaction;
        let mut db = rusqlite::Connection::open(&self.database).unwrap();
        let transaction = ServiceDecisionTransaction::begin(&mut db, gate, connection).unwrap();
        transaction
            .transaction()
            .execute("INSERT INTO decisions DEFAULT VALUES", [])
            .unwrap();
        transaction.commit().unwrap();
        self.committed.store(true, Ordering::SeqCst);
        while !self.release.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(api_error(
            ErrorCode::Unsupported,
            "committed result withheld",
        ))
    }

    fn audit_service_disconnect(
        &self,
        _: &str,
        _: u64,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        self.audits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_disconnect_after_commit_preserves_history_and_closes_socket() {
    use crate::daemon::{
        control::{ControlService, StopController},
        health::HealthInputs,
    };
    use crate::protocol::{
        commands::ServiceDisconnectRequest, results::ServiceRecoveryAudit, service::ServiceResult,
    };
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let database = root.join("postcommit-recovery.db");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE decisions (id INTEGER PRIMARY KEY)")
        .unwrap();
    let committed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let audits = Arc::new(AtomicUsize::new(0));
    let domain = CommittedWaitDomain {
        database: database.clone(),
        committed: committed.clone(),
        release: release.clone(),
        audits: audits.clone(),
    };
    let shutdown = Cancellation::default();
    let handler = Arc::new(ControlService::new(
        StopController::new(instance, boot, shutdown.clone()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        domain,
    ));
    let gate = Arc::new(LiveServiceGate::new());
    let server = tokio::spawn(serve_with_gate(
        listener,
        instance,
        handler.clone(),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
        gate.clone(),
        None,
        ConnectionGauge::default(),
    ));
    let mut registered = UnixStream::connect(&path).await.unwrap();
    let initial = service_call(&mut registered, &service_registration(instance, "register")).await;
    let generation = match initial.result.unwrap() {
        ServiceResult::Registered(value) => value.connection_generation,
        _ => panic!("expected registration"),
    };
    write_frame(
        &mut registered,
        &serde_json::to_vec(&decision_request(instance)).unwrap(),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !committed.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let disconnected = recovery_call(
        &path,
        instance,
        Command::ServiceDisconnect(ServiceDisconnectRequest {
            expected_boot: boot.to_string(),
            expected_generation: generation,
        }),
    )
    .await
    .result
    .unwrap();
    let CommandResult::ServiceDisconnected(result) = disconnected else {
        panic!("expected disconnect")
    };
    assert!(result.disconnected);
    assert_eq!(result.audit, ServiceRecoveryAudit::Persisted);
    assert_eq!(audits.load(Ordering::SeqCst), 1);
    release.store(true, Ordering::SeqCst);
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), registered.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let count: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row("SELECT count(*) FROM decisions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

struct StoreAuditDomain(Arc<crate::store::SqliteStore>);

impl LocalService for StoreAuditDomain {
    crate::unserved_local_service_routes!(service_control, service_operation, handle_with_output);
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        unreachable!("only recovery commands reach this domain")
    }

    fn audit_service_disconnect(
        &self,
        boot: &str,
        generation: u64,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        crate::ports::StorePort::audit_service_disconnect(
            self.0.as_ref(),
            boot,
            generation,
            peer,
            budget,
        )
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_disconnect_reports_real_writer_failure_after_revocation() {
    use crate::daemon::{
        control::{ControlService, StopController},
        health::HealthInputs,
    };
    use crate::protocol::{
        commands::ServiceDisconnectRequest, results::ServiceRecoveryAudit, service::ServiceResult,
    };
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let database = root.join("recovery-audit.db");
    let store = Arc::new(
        crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(database.clone(), Arc::new(TestClock))
                .with_test_host()
                .with_node_seed(crate::test_support::node_seed_for(instance)),
            instance.to_string(),
            crate::store::StoreSettings {
                daemon_boot: Some(boot),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let shutdown = Cancellation::default();
    let handler = Arc::new(ControlService::new(
        StopController::new(instance, boot, shutdown.clone()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        StoreAuditDomain(store),
    ));
    let gate = Arc::new(LiveServiceGate::new());
    let server = tokio::spawn(serve_with_gate(
        listener,
        instance,
        handler,
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
        gate.clone(),
        None,
        ConnectionGauge::default(),
    ));
    let mut registered = UnixStream::connect(&path).await.unwrap();
    let initial = service_call(&mut registered, &service_registration(instance, "register")).await;
    let generation = match initial.result.unwrap() {
        ServiceResult::Registered(value) => value.connection_generation,
        _ => panic!("expected registration"),
    };
    let stale = recovery_call(
        &path,
        instance,
        Command::ServiceDisconnect(ServiceDisconnectRequest {
            expected_boot: boot.to_string(),
            expected_generation: generation + 1,
        }),
    )
    .await
    .result
    .unwrap_err();
    assert_eq!(stale.code, ErrorCode::StaleServiceGeneration);
    let audit_count = || -> i64 {
        rusqlite::Connection::open(&database)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM operations WHERE actor_scope=?1",
                [format!("service-recovery-audit:{instance}")],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(audit_count(), 0);
    let blocker = rusqlite::Connection::open(&database).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let disconnected = recovery_call(
        &path,
        instance,
        Command::ServiceDisconnect(ServiceDisconnectRequest {
            expected_boot: boot.to_string(),
            expected_generation: generation,
        }),
    )
    .await
    .result
    .unwrap();
    let CommandResult::ServiceDisconnected(result) = disconnected else {
        panic!("expected disconnect")
    };
    assert!(result.disconnected);
    assert!(matches!(
        result.audit,
        ServiceRecoveryAudit::Failed(ApiError {
            code: ErrorCode::StoreBusy | ErrorCode::DeadlineExceeded,
            ..
        })
    ));
    assert!(
        !gate
            .inspect(&instance.to_string(), &boot.to_string())
            .connected
    );
    blocker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(audit_count(), 0);
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), registered.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    shutdown.cancel();
    assert!(matches!(
        server.await.unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

// ---- Production elected Health under the current request budget (fix3) ----

/// Controllable monotonic clock for deterministic request-budget expiry.
struct HealthStepClock(std::sync::atomic::AtomicU64);
impl Clock for HealthStepClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

/// A real `SqliteStore`, exactly as `run_elected` builds one, on a database the
/// store keeps in rollback journaling (test hook), so a competing
/// `BEGIN EXCLUSIVE` blocks the Health retirement read in SQLite's busy
/// handler. Production WAL mode is covered by the elected service test.
/// `busy` fires when that read first waits on the lock; `ended` reports the
/// typed budget outcome when the read's query connection is dropped.
struct HealthLockStore {
    store: Arc<dyn crate::ports::StorePort>,
    writer: rusqlite::Connection,
    busy: std::sync::mpsc::Receiver<()>,
    ended: std::sync::mpsc::Receiver<Option<ErrorCode>>,
    root: std::path::PathBuf,
}

fn health_lock_store(
    instance: crate::node::NodeUuid,
    boot: uuid::Uuid,
    clock: Arc<dyn Clock>,
) -> HealthLockStore {
    let root = std::env::temp_dir().join(format!("herdr-health-lock-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let database = root.join("threads.sqlite3");
    let context =
        crate::store::connection::StoreContext::new(database.clone(), clock).with_test_host();
    context.use_rollback_journal();
    let (busy_tx, busy) = std::sync::mpsc::channel();
    context.signal_next_query_busy(busy_tx);
    let (ended_tx, ended) = std::sync::mpsc::channel();
    context.signal_next_query_end(ended_tx);
    let store: Arc<dyn crate::ports::StorePort> = Arc::new(
        crate::store::SqliteStore::new(
            context,
            instance.to_string(),
            crate::service::config::ServiceConfig::default().store_settings(boot),
        )
        .unwrap(),
    );
    let writer = rusqlite::Connection::open(&database).unwrap();
    let mode: String = writer
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "delete");
    HealthLockStore {
        store,
        writer,
        busy,
        ended,
        root,
    }
}

/// The production elected Health provider (`app::elected_health_provider`,
/// the same builder `run_elected` uses) over a real `SqliteStore`, behind the
/// production `ControlService`.
fn elected_health_control(
    instance: crate::node::NodeUuid,
    boot: uuid::Uuid,
    shutdown: Cancellation,
    store: Arc<dyn crate::ports::StorePort>,
    clock: Arc<dyn Clock>,
) -> crate::daemon::control::ControlService<
    impl Fn(&CallBudget) -> crate::daemon::health::HealthInputs + Send + Sync,
    RejectOtherService,
> {
    use crate::daemon::control::{ControlService, StopController};
    let health = crate::app::elected_health_provider(
        instance,
        boot,
        crate::service::config::ServiceConfig::default().health_settings(),
        clock,
        store,
        <[std::sync::Arc<crate::service::workers::WorkerStatus>; 3]>::default(),
        // No observation lane runs here: no recorded host evidence, under the
        // macOS adapter's witness (Unknown until a capture is observed).
        crate::app::ElectedHostEvidence {
            status: Default::default(),
            incarnation_witness: crate::protocol::results::CapabilityState::Unknown,
            safe_prompt: crate::protocol::results::CapabilityState::Unsupported,
            harnesses: Default::default(),
            release: Default::default(),
        },
    );
    ControlService::new(
        StopController::new(instance, boot, shutdown),
        health,
        RejectOtherService,
    )
}

fn assert_health_degraded(result: Result<CommandResult, ApiError>) {
    let Ok(CommandResult::Health(health)) = result else {
        panic!("missing Health: {result:?}")
    };
    assert_eq!(
        health.database.state,
        crate::protocol::results::ComponentState::Degraded
    );
    assert_eq!(
        health.database.detail.as_deref(),
        Some("retirement status unavailable")
    );
}

#[test]
fn health_summary_budget_shares_request_cancellation_and_caps_deadline() {
    let clock = HealthStepClock(std::sync::atomic::AtomicU64::new(1_000));
    let cancellation = Cancellation::default();
    let long = CallBudget {
        deadline: MonoInstant(6_000),
        cancellation: cancellation.clone(),
    };
    let summary = crate::app::health_summary_budget(&long, &clock);
    assert_eq!(
        summary.deadline,
        MonoInstant(1_000 + crate::app::HEALTH_SUMMARY_LIMIT_MS)
    );
    assert!(!summary.cancellation.is_cancelled());
    cancellation.cancel();
    assert!(
        summary.cancellation.is_cancelled(),
        "request cancellation not shared"
    );
    let short = CallBudget {
        deadline: MonoInstant(1_040),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        crate::app::health_summary_budget(&short, &clock).deadline,
        MonoInstant(1_040)
    );
}

/// Drive `ControlService::handle(Health)` on its own thread while a competing
/// connection holds `BEGIN EXCLUSIVE`; `end_request` ends the request budget
/// once the read is blocked. The handler must return (and its thread be
/// joined) before the lock is released.
fn locked_health_request_ends(
    request_deadline_offset: u64,
    end_request: impl FnOnce(&Cancellation, &HealthStepClock),
) -> Option<ErrorCode> {
    let step = Arc::new(HealthStepClock(std::sync::atomic::AtomicU64::new(1_000)));
    let clock: Arc<dyn Clock> = step.clone();
    let (instance, boot) = (crate::node::NodeUuid::new_v4(), uuid::Uuid::new_v4());
    let HealthLockStore {
        store,
        writer,
        busy,
        ended,
        root,
    } = health_lock_store(instance, boot, Arc::clone(&clock));
    let service = elected_health_control(instance, boot, Cancellation::default(), store, clock);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let cancellation = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(1_000 + request_deadline_offset),
        cancellation: cancellation.clone(),
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = service.handle(Command::Health, PeerIdentity::from_kernel(501), &budget);
        let _ = done_tx.send(());
        result
    });
    busy.recv_timeout(Duration::from_secs(1))
        .expect("Health retirement read reached the SQLite busy handler");
    end_request(&cancellation, &step);
    let returned = done_rx.recv_timeout(Duration::from_secs(1));
    if returned.is_err() {
        // Release the stuck read (clock and lock) so the test fails instead
        // of hanging, even when the read's deadline was detached.
        step.0.fetch_add(10_000, Ordering::SeqCst);
        writer.execute_batch("COMMIT").unwrap();
        let _ = worker.join();
        std::fs::remove_dir_all(root).unwrap();
        panic!("Health did not observe the ended request budget while the lock was held");
    }
    let result = worker.join().expect("Health handler thread panicked");
    // The handler returned and was joined while the competing lock is still held.
    assert!(!writer.is_autocommit(), "competing lock released early");
    assert_health_degraded(result);
    writer.execute_batch("COMMIT").unwrap();
    drop(writer);
    std::fs::remove_dir_all(root).unwrap();
    ended
        .try_recv()
        .expect("Health retirement read connection was dropped")
}

#[test]
fn elected_health_request_cancellation_stops_locked_retirement_read() {
    // Kills: detaching the retirement read's cancellation in the shared
    // builder. The clock is frozen, so only the request cancellation can end
    // the blocked read; a detached read hangs until the fail-safe releases it.
    let observed = locked_health_request_ends(5_000, |cancellation, _| cancellation.cancel());
    assert_eq!(observed, Some(ErrorCode::Cancelled));
}

#[test]
fn elected_health_request_expiry_stops_locked_retirement_read() {
    // Kills: detaching the retirement read's deadline in the shared builder
    // (for example an unbounded deadline); only request expiry can end it.
    // Request deadline (1_050) is earlier than the 100 ms Health cap (1_100).
    let observed = locked_health_request_ends(50, |_, clock| {
        clock.0.store(1_050, Ordering::SeqCst);
    });
    assert_eq!(observed, Some(ErrorCode::DeadlineExceeded));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elected_health_transport_disconnect_cancels_locked_retirement_read() {
    // Kills: detaching the retirement read's cancellation inside the shared
    // production builder (`app::elected_health_provider`), e.g. passing
    // `CallBudget { deadline, cancellation: Cancellation::default() }` to
    // `StorePort::retirement_summary`. The read then ends on its 100 ms cap
    // (`DeadlineExceeded`) instead of the client disconnect (`Cancelled`).
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let clock: Arc<dyn Clock> = Arc::new(TestClock);
    let HealthLockStore {
        store,
        writer,
        busy,
        ended,
        root: store_root,
    } = health_lock_store(instance, boot, Arc::clone(&clock));
    let shutdown = Cancellation::default();
    let service =
        elected_health_control(instance, boot, shutdown.clone(), store, Arc::clone(&clock));
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(service),
        clock,
        uid,
        shutdown.clone(),
    ));
    let mut stream = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut stream, &health_request(instance, "locked-health"))
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || busy.recv_timeout(Duration::from_secs(1)))
        .await
        .unwrap()
        .expect("Health retirement read reached the SQLite busy handler");
    // Client disconnect: the transport cancels this request's budget.
    drop(stream);
    let settled = tokio::task::spawn_blocking(move || ended.recv_timeout(Duration::from_secs(1)))
        .await
        .unwrap();
    // The transport joins the blocking worker before the connection task ends,
    // so a drained shutdown proves the Health worker exited.
    shutdown.cancel();
    let drained = tokio::time::timeout(Duration::from_secs(2), server).await;
    assert!(!writer.is_autocommit(), "competing lock released early");
    writer.execute_batch("COMMIT").unwrap();
    drop(writer);
    assert_eq!(
        settled.expect("Health read did not end while the lock was held"),
        Some(ErrorCode::Cancelled)
    );
    assert!(matches!(
        drained.expect("server did not drain").unwrap().unwrap(),
        ServeOutcome::Drained
    ));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(store_root).unwrap();
}

async fn started_server(
    service: Arc<dyn LocalService>,
) -> (
    tokio::task::JoinHandle<io::Result<ServeOutcome>>,
    Cancellation,
    std::path::PathBuf,
    std::path::PathBuf,
    crate::daemon::paths::NodePaths,
) {
    let (listener, path, root, instance) = guarded_listener();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance_id(&instance),
        service,
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    (server, shutdown, path, root, instance)
}

#[tokio::test]
async fn cancelled_serve_loop_returns_without_polling() {
    let service = Arc::new(PausedService {
        entered: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    // Sample prompt cancellation on up to three fresh loops: one must stop
    // within 20 ms, and every attempt must drain. This does not rule out an
    // intermittently slow implementation or every polling interval (a poll
    // already nearing its deadline can pass). The hosted 5e69f9c7 manual run
    // measured 24.9 ms; host scheduling delay is inferred from local
    // microsecond samples, not measured on that runner.
    let mut samples = Vec::new();
    for _ in 0..3 {
        let (server, shutdown, path, root, _instance) = started_server(service.clone()).await;
        // Let the loop park in accept so the cancel has to wake it.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let started = std::time::Instant::now();
        shutdown.cancel();
        let outcome = server.await.unwrap().unwrap();
        let elapsed = started.elapsed();
        assert!(matches!(outcome, ServeOutcome::Drained));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        samples.push(elapsed);
        if elapsed < Duration::from_millis(20) {
            return;
        }
    }
    panic!("serve took {samples:?} to stop (no attempt within 20 ms)");
}

#[tokio::test]
async fn waiting_client_returns_on_cancel() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let clock = Arc::new(TestClock);
    let cancellation = Cancellation::default();
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 5000),
        cancellation: cancellation.clone(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        crate::node::NodeUuid::new_v4(),
        None,
    );
    let call = tokio::spawn(async move { client.call_async(Command::Health, &budget).await });
    // The fixture server reads the request and never replies.
    let (mut stream, _) = listener.accept().await.unwrap();
    let _ = read_frame(&mut stream).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = std::time::Instant::now();
    cancellation.cancel();
    let error = call.await.unwrap().unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.code, ErrorCode::UnknownOutcome);
    assert!(
        elapsed < Duration::from_millis(20),
        "client took {elapsed:?} to return"
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn accept_drain_returns_at_deadline_when_a_task_never_finishes() {
    use crate::daemon::ownership::OwnerLock;
    let service = Arc::new(PausedService {
        entered: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let (server, shutdown, path, root, instance) = started_server(service.clone()).await;
    let mut deciding = UnixStream::connect(&path).await.unwrap();
    write_frame(&mut deciding, &health_request(instance_id(&instance), "r"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let started = tokio::time::Instant::now();
    shutdown.cancel();
    let outcome = server.await.unwrap().unwrap();
    let elapsed = started.elapsed();
    let ServeOutcome::Incomplete(mut pending) = outcome else {
        panic!("a task that never finishes must leave the drain incomplete");
    };
    assert!(
        elapsed >= SHUTDOWN_DRAIN && elapsed < SHUTDOWN_DRAIN + Duration::from_millis(50),
        "drain returned after {elapsed:?}, expected about {SHUTDOWN_DRAIN:?}"
    );
    assert_eq!(pending.remaining(), 1);
    service.release.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(1), pending.wait())
        .await
        .unwrap()
        .unwrap();
    drop(pending);
    drop(OwnerLock::acquire_with_test_hint(&instance).unwrap());
    drop(deciding);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn accept_drain_returns_promptly_when_tasks_finish() {
    let service = Arc::new(PausedService {
        entered: AtomicUsize::new(0),
        release: std::sync::atomic::AtomicBool::new(false),
    });
    let (server, shutdown, path, root, _instance) = started_server(service).await;
    // Idle connections finish as soon as shutdown is observed.
    let idle: Vec<_> = futures_idle(&path, 3).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = tokio::time::Instant::now();
    shutdown.cancel();
    let outcome = server.await.unwrap().unwrap();
    let elapsed = started.elapsed();
    assert!(matches!(outcome, ServeOutcome::Drained));
    assert!(
        elapsed < SHUTDOWN_DRAIN / 2,
        "drain took {elapsed:?} although every task finished at once"
    );
    drop(idle);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

async fn futures_idle(path: &std::path::Path, count: usize) -> Vec<UnixStream> {
    let mut streams = Vec::new();
    for _ in 0..count {
        streams.push(UnixStream::connect(path).await.unwrap());
    }
    streams
}

struct BootCheckService(Arc<AtomicUsize>);
impl LocalService for BootCheckService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(api_error(ErrorCode::Unsupported, "counted"))
    }
}

async fn send_with_expected_boot(
    path: &std::path::Path,
    instance: crate::node::NodeUuid,
    expected_boot: Option<String>,
) -> WireResponse {
    let mut stream = UnixStream::connect(path).await.unwrap();
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "boot-check".into(),
        expected_instance: instance.to_string(),
        expected_boot,
        output: None,
        command: Command::Health,
    };
    write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    serde_json::from_slice(&read_frame(&mut stream).await.unwrap()).unwrap()
}

#[tokio::test]
async fn stale_expected_boot_is_refused_before_dispatch_and_applies_nothing() {
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(BootCheckService(calls.clone())),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let response =
        send_with_expected_boot(&path, instance, Some(uuid::Uuid::new_v4().to_string())).await;
    assert_eq!(response.daemon_boot, boot.to_string());
    assert_eq!(response.request_id, "boot-check");
    let error = response.result.unwrap_err();
    assert_eq!(error.code, ErrorCode::DaemonBootChanged);
    assert!(error.detail.contains("nothing was applied"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    shutdown.cancel();
    let _ = server.await;
    std::fs::remove_file(path).ok();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn matching_or_absent_expected_boot_dispatches() {
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let boot = listener.boot_id();
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(BootCheckService(calls.clone())),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    for expected in [Some(boot.to_string()), None] {
        let response = send_with_expected_boot(&path, instance, expected).await;
        assert_eq!(response.result.unwrap_err().code, ErrorCode::Unsupported);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    shutdown.cancel();
    let _ = server.await;
    std::fs::remove_file(path).ok();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn client_maps_daemon_boot_changed_to_definite_rejection() {
    let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
    let listener = UnixListener::bind(&path).unwrap();
    let instance = crate::node::NodeUuid::new_v4();
    let expected_boot = uuid::Uuid::new_v4();
    let actual_boot = uuid::Uuid::new_v4();
    let accepted = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: request.request_id,
            instance: instance.to_string(),
            daemon_boot: actual_boot.to_string(),
            result: Err(api_error(ErrorCode::DaemonBootChanged, "boot changed")),
        };
        write_frame(&mut stream, &serde_json::to_vec(&response).unwrap())
            .await
            .unwrap();
    });
    let clock = Arc::new(TestClock);
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1000),
        cancellation: Cancellation::default(),
    };
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        clock,
        instance,
        Some(expected_boot),
    );
    let definite =
        tokio::task::spawn_blocking(move || client.call_definitive(Command::Health, &budget))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(definite.unwrap_err().code, ErrorCode::DaemonBootChanged);
    accepted.await.unwrap();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn local_client_sends_expected_boot_from_descriptor() {
    for boot in [Some(uuid::Uuid::new_v4()), None] {
        let path = std::env::temp_dir().join(format!("herdr-ipc-{}.sock", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&path).unwrap();
        let instance = crate::node::NodeUuid::new_v4();
        let accepted = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let frame = read_frame(&mut stream).await.unwrap();
            let request = WireRequest::decode(&frame).unwrap();
            let raw: serde_json::Value = serde_json::from_slice(&frame).unwrap();
            (request.expected_boot, raw.get("expected_boot").cloned())
        });
        let clock = Arc::new(TestClock);
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 300),
            cancellation: Cancellation::default(),
        };
        let client =
            crate::client::local::LocalSocketClient::new(path.clone(), clock, instance, boot);
        let _ = client.call_async(Command::Health, &budget).await;
        let (seen, raw) = accepted.await.unwrap();
        assert_eq!(seen, boot.map(|b| b.to_string()));
        match boot {
            Some(b) => assert_eq!(raw, Some(serde_json::Value::String(b.to_string()))),
            None => assert_eq!(raw, None),
        }
        std::fs::remove_file(path).unwrap();
    }
}

fn maximum_automatic_recovery_result(
    instance: &str,
) -> crate::protocol::handoff::AutomaticBootstrapResult {
    use crate::protocol::handoff::*;
    use crate::protocol::ids::*;
    let old = topology_contract_tests::payload();
    let mut claim = topology_contract_tests::claim();
    claim.instance = instance.into();
    let mut ns = old.handoff.namespace.clone();
    ns.instance = instance.into();
    let startup_input =
        crate::harness::launch::bootstrap_v2_startup_input(old.launch.harness, &old.launch.argv)
            .unwrap();
    let id = AutomaticBootstrapIdentity::new(
        crate::cli::journal::IntentScope::Cooperative {
            host: crate::test_support::test_host_locator(),
            instance: instance.into(),
            seat: claim.seat.clone(),
        },
        claim,
        AutomaticBootstrapPlan {
            version: 2,
            payload: AutomaticBootstrapPayload {
                handoff: AutomaticHandoffPayload::new(
                    ns,
                    OperationId::new("large-child"),
                    "\0".repeat(65536),
                )
                .unwrap()
                .with_policy(Some(crate::protocol::handoff::HandoffPolicy {
                    deadline_millis: None,
                    invitation_confirmation:
                        crate::protocol::commands::InvitationConfirmation::Lazy,
                    invite_deadline_millis: None,
                }))
                .unwrap(),
                keys: AutomaticBootstrapKeys::new(OperationId::new("large-parent")),
                workspace: old.workspace,
                cwd: old.cwd,
                label: old.label,
                focus: false,
                env: Default::default(),
                launch: old.launch,
                startup_input: Some(startup_input),
            },
        },
    )
    .unwrap();
    id.validate_admission().unwrap();
    let created = topology_contract_tests::created();
    let attachment = AutomaticBootstrapAttachment::derive(
        &id,
        BootstrapAttempt::first(),
        created.clone(),
        SeatId::new("recipient"),
    )
    .unwrap();
    AutomaticBootstrapResult {
        identity: id,
        state: BootstrapState::Cancelled,
        attempt: BootstrapAttempt::first(),
        attempt_state: BootstrapAttemptState::Created,
        administrative_revision: 1,
        creation: Some(created),
        thread: None,
        completed: None,
        recovery: Some(AutomaticBootstrapRecoveryResult {
            operation: OperationId::new("large-recovery"),
            attempt: BootstrapAttempt::first(),
            inspection: BootstrapRecoveryInspection {
                version: 1,
                digest: "a".repeat(64),
            },
            disposition: AutomaticBootstrapRecoveryDisposition::Cancelled {
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                reason: format!("x{}", "\u{1}".repeat(4095)),
                attached_child: Some(Box::new(attachment.child.clone())),
            },
            operator_uid: 501,
            operator_provenance: "operator:local-user:501".into(),
            state: BootstrapState::Cancelled,
            administrative_revision: 1,
        }),
        attachment: Some(attachment),
    }
}
struct AutomaticLargeService(crate::protocol::handoff::AutomaticBootstrapResult);
impl LocalService for AutomaticLargeService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Ok(CommandResult::AutomaticBootstrap(Box::new(self.0.clone())))
    }
}
#[tokio::test]
async fn automatic_recovery_response_larger_than_one_mib_survives_actual_local_client_transport() {
    use crate::protocol::handoff::{AutomaticBootstrapAction, AutomaticBootstrapMutation};
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let result = maximum_automatic_recovery_result(&instance.to_string());
    let command = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
        identity: result.identity.clone(),
        action: AutomaticBootstrapAction::Status,
    }));
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(AutomaticLargeService(result.clone())),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let clock = Arc::new(TestClock);
    let client = crate::client::local::LocalSocketClient::new(path.clone(), clock, instance, None);
    let reply = client
        .call_async(
            command,
            &CallBudget {
                deadline: crate::protocol::time::MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
        )
        .await;
    shutdown.cancel();
    let _ = server.await.unwrap().unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(
        reply.unwrap(),
        CommandResult::AutomaticBootstrap(Box::new(result))
    );
}

#[tokio::test]
async fn automatic_response_embedded_identity_must_match_exact_request() {
    use crate::protocol::handoff::{AutomaticBootstrapAction, AutomaticBootstrapMutation};
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let result = maximum_automatic_recovery_result(&instance.to_string());
    let mut requested = result.identity.clone();
    requested.claim.execution =
        crate::protocol::ids::ExecutionId::new("00000000-0000-4000-8000-000000000099");
    requested.digest = requested.semantic_digest().unwrap();
    requested.validate().unwrap();
    let command = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
        identity: requested,
        action: AutomaticBootstrapAction::Status,
    }));
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(AutomaticLargeService(result)),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        Arc::new(TestClock),
        instance,
        None,
    );
    let reply = client
        .call_async(
            command,
            &CallBudget {
                deadline: crate::protocol::time::MonoInstant(10_000),
                cancellation: Cancellation::default(),
            },
        )
        .await;
    shutdown.cancel();
    let _ = server.await.unwrap().unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        reply.is_err_and(|e| e.code == ErrorCode::UnknownOutcome),
        "different embedded original was accepted"
    );
}

struct AutomaticIdentityService(CommandResult);
impl LocalService for AutomaticIdentityService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Ok(self.0.clone())
    }
}
#[tokio::test]
async fn automatic_response_path_alias_is_not_exact_identity_in_either_family() {
    use crate::protocol::handoff::{
        AutomaticBootstrapAction, AutomaticBootstrapMutation, AutomaticBootstrapReserved,
    };
    for reserved in [false, true] {
        for alias in [false, true] {
            let (listener, path, root, paths) = guarded_listener();
            let instance = instance_id(&paths);
            let mut result = maximum_automatic_recovery_result(&instance.to_string());
            let expected = result.identity.clone();
            if alias {
                let cwd = result.identity.plan.payload.cwd.to_str().unwrap();
                result.identity.plan.payload.cwd = format!("{cwd}/").into();
                let state_dir = result
                    .identity
                    .plan
                    .payload
                    .handoff
                    .namespace
                    .state_dir
                    .to_str()
                    .unwrap();
                result.identity.plan.payload.handoff.namespace.state_dir =
                    format!("{state_dir}/").into();
                assert_eq!(
                    result.identity, expected,
                    "fixture isolates normalized path equality"
                );
                assert_eq!(result.identity.digest, expected.digest);
                assert_ne!(
                    serde_json::to_vec(&result.identity).unwrap(),
                    serde_json::to_vec(&expected).unwrap()
                );
            }
            let reply_result = if reserved {
                CommandResult::AutomaticBootstrapReserved(Box::new(AutomaticBootstrapReserved {
                    status: result,
                    authorization: None,
                }))
            } else {
                CommandResult::AutomaticBootstrap(Box::new(result))
            };
            let command = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
                identity: expected,
                action: AutomaticBootstrapAction::Status,
            }));
            let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
            let shutdown = Cancellation::default();
            let server = tokio::spawn(serve(
                listener,
                instance,
                Arc::new(AutomaticIdentityService(reply_result.clone())),
                Arc::new(TestClock),
                uid,
                shutdown.clone(),
            ));
            let client = crate::client::local::LocalSocketClient::new(
                path.clone(),
                Arc::new(TestClock),
                instance,
                None,
            );
            let reply = client
                .call_async(
                    command,
                    &CallBudget {
                        deadline: MonoInstant(10_000),
                        cancellation: Cancellation::default(),
                    },
                )
                .await;
            shutdown.cancel();
            let _ = server.await.unwrap().unwrap();
            std::fs::remove_file(path).unwrap();
            std::fs::remove_dir_all(root).unwrap();
            if alias {
                assert!(
                    reply.is_err_and(|e| e.code == ErrorCode::UnknownOutcome),
                    "path alias accepted in reserved={reserved}"
                );
            } else {
                assert_eq!(reply.unwrap(), reply_result);
            }
        }
    }
}

fn historical_completed_transport_result(
    instance: &str,
    diagnostics: usize,
) -> crate::protocol::handoff::AutomaticBootstrapResult {
    use crate::protocol::handoff::*;
    use crate::protocol::ids::ThreadId;
    use sha2::{Digest, Sha256};
    let mut result = maximum_automatic_recovery_result(instance);
    result.identity.plan.payload.handoff.body = "work".into();
    result.identity.plan.payload.handoff.policy = None;
    result.identity.digest = result.identity.semantic_digest().unwrap();
    result.identity.validate().unwrap();
    let attachment = AutomaticBootstrapAttachment::derive(
        &result.identity,
        result.attempt,
        result.creation.clone().unwrap(),
        result.attachment.as_ref().unwrap().resolved_seat.clone(),
    )
    .unwrap();
    let thread = ThreadId::new("historical-completed");
    let AutomaticOriginal::Handoff(child) = &attachment.child.original else {
        panic!("handoff")
    };
    let prompt = crate::cli::handoff::bootstrap_v1::retained_prompts(
        thread.as_str(),
        &child.context,
        instance,
    )
    .remove(0);
    let argv = result
        .identity
        .plan
        .payload
        .startup_input
        .as_ref()
        .unwrap()
        .apply(&result.identity.plan.payload.launch.argv, &prompt)
        .unwrap()
        .0;
    let mut report = serde_json::json!({"outcome":"started", "pane":attachment.created.root_pane, "seat":attachment.resolved_seat, "harness":result.identity.plan.payload.launch.harness.as_str(), "argv":argv, "diagnostics":""});
    let diagnostics = if diagnostics == usize::MAX {
        1_048_576 - 512 - serde_json::to_vec(&report).unwrap().len()
    } else {
        diagnostics
    };
    report["diagnostics"] = serde_json::Value::String("x".repeat(diagnostics));
    let retained = LinkedBootstrapReport {
        thread: thread.clone(),
        recipient: attachment.resolved_seat.clone(),
        pane: attachment.created.root_pane.clone(),
        kind: "launch".into(),
        launch: result.identity.plan.payload.launch.clone(),
        report_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&report).unwrap())),
        report,
        terminal: attachment.created.terminal.clone(),
        host_incarnation: attachment.created.host_incarnation.clone(),
    };
    AutomaticBootstrapMutation {
        identity: result.identity.clone(),
        action: AutomaticBootstrapAction::Complete {
            attachment: Box::new(attachment.clone()),
            retained: retained.clone(),
        },
    }
    .validate()
    .unwrap();
    result.state = BootstrapState::Completed;
    result.thread = Some(thread);
    result.recovery = None;
    result.completed = Some(retained);
    result.attachment = Some(attachment);
    result
}
#[tokio::test]
async fn automatic_historical_response_keeps_one_mib_client_allocation_limit() {
    use crate::protocol::handoff::{AutomaticBootstrapAction, AutomaticBootstrapMutation};
    for large in [false, true] {
        let path = std::env::temp_dir().join(format!("ht-h-{}.sock", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&path).unwrap();
        let instance = crate::node::NodeUuid::new_v4();
        let result = historical_completed_transport_result(
            &instance.to_string(),
            if large { usize::MAX } else { 300_000 },
        );
        let command = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
            identity: result.identity.clone(),
            action: AutomaticBootstrapAction::Status,
        }));
        let accepted = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = WireRequest::decode(&read_frame(&mut stream).await.unwrap()).unwrap();
            let response = WireResponse {
                version: PROTOCOL_VERSION,
                request_id: request.request_id,
                instance: instance.to_string(),
                daemon_boot: uuid::Uuid::new_v4().to_string(),
                result: Ok(CommandResult::AutomaticBootstrap(Box::new(result))),
            };
            let bytes = serde_json::to_vec(&response).unwrap();
            assert_eq!(
                bytes.len() + 4 > crate::protocol::wire::MAX_WIRE_FRAME_BYTES,
                large
            );
            assert!(bytes.len() + 4 < crate::protocol::wire::MAX_AUTOMATIC_RESPONSE_FRAME_BYTES);
            stream
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .await
                .unwrap();
            // A corrected client may close immediately after the oversized prefix.
            let _ = stream.write_all(&bytes).await;
        });
        let client = crate::client::local::LocalSocketClient::new(
            path.clone(),
            Arc::new(TestClock),
            instance,
            None,
        );
        let reply = client
            .call_async(
                command,
                &CallBudget {
                    deadline: MonoInstant(10_000),
                    cancellation: Cancellation::default(),
                },
            )
            .await;
        accepted.await.unwrap();
        std::fs::remove_file(path).unwrap();
        if large {
            assert!(
                reply.is_err_and(|e| e.code == ErrorCode::UnknownOutcome),
                "historical >1MiB response was accepted"
            );
        } else {
            assert!(
                matches!(reply, Ok(CommandResult::AutomaticBootstrap(_))),
                "inherited historical256KiB..1MiB transport acceptance changed"
            );
        }
    }
}
#[test]
fn automatic_historical_response_keeps_one_mib_scoped_encoder_limit() {
    use crate::protocol::handoff::{AutomaticBootstrapAction, AutomaticBootstrapMutation};
    let instance = crate::node::NodeUuid::new_v4().to_string();
    let result = historical_completed_transport_result(&instance, usize::MAX);
    let command = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
        identity: result.identity.clone(),
        action: AutomaticBootstrapAction::Status,
    }));
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: "historical".into(),
        instance,
        daemon_boot: uuid::Uuid::new_v4().to_string(),
        result: Ok(CommandResult::AutomaticBootstrap(Box::new(result))),
    };
    let result = crate::protocol::wire::encode_wire_response_scoped(
        &response,
        crate::protocol::wire::automatic_response_request(&command),
    );
    assert!(
        result.is_err_and(|e| e.code == ErrorCode::InvalidBudget),
        "historical >1MiB response was encoded"
    );
}

#[test]
fn unscoped_ordinary_encoder_keeps_one_mib_even_for_automatic_result() {
    let response = crate::protocol::wire::WireResponse {
        version: crate::protocol::wire::PROTOCOL_VERSION,
        request_id: "request".into(),
        instance: crate::node::NodeUuid::from_u128(1).to_string(),
        daemon_boot: uuid::Uuid::from_u128(2).to_string(),
        result: Ok(CommandResult::AutomaticBootstrap(Box::new(
            maximum_automatic_recovery_result(&uuid::Uuid::from_u128(1).to_string()),
        ))),
    };
    assert_eq!(
        crate::protocol::wire::encode_wire_response(&response)
            .unwrap_err()
            .code,
        ErrorCode::InvalidBudget
    );
}
#[tokio::test]
async fn automatic_response_overlimit_length_refuses_before_body_read() {
    struct PrefixOnly {
        prefix: [u8; 4],
        position: usize,
    }
    impl tokio::io::AsyncRead for PrefixOnly {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            buffer: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            assert!(
                self.position < 4,
                "oversized length triggered a body read/allocation"
            );
            let end = (self.position + buffer.remaining()).min(4);
            buffer.put_slice(&self.prefix[self.position..end]);
            self.position = end;
            std::task::Poll::Ready(Ok(()))
        }
    }
    let limit = crate::protocol::wire::MAX_AUTOMATIC_RESPONSE_FRAME_BYTES - 4;
    let mut reader = PrefixOnly {
        prefix: ((limit + 1) as u32).to_be_bytes(),
        position: 0,
    };
    let error = read_frame_with_limit(&mut reader, limit).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(reader.position, 4);
}
struct UnexpectedAutomaticService;
impl LocalService for UnexpectedAutomaticService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Ok(CommandResult::MessageSent(
            crate::protocol::ids::MessageId::new("unrelated"),
        ))
    }
}
#[tokio::test]
async fn automatic_known_request_rejects_unrelated_success_result_family() {
    use crate::protocol::handoff::{AutomaticBootstrapAction, AutomaticBootstrapMutation};
    let (listener, path, root, paths) = guarded_listener();
    let instance = instance_id(&paths);
    let identity = maximum_automatic_recovery_result(&instance.to_string()).identity;
    let command = Command::AutomaticBootstrap(Box::new(AutomaticBootstrapMutation {
        identity,
        action: AutomaticBootstrapAction::Status,
    }));
    let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let shutdown = Cancellation::default();
    let server = tokio::spawn(serve(
        listener,
        instance,
        Arc::new(UnexpectedAutomaticService),
        Arc::new(TestClock),
        uid,
        shutdown.clone(),
    ));
    let client = crate::client::local::LocalSocketClient::new(
        path.clone(),
        Arc::new(TestClock),
        instance,
        None,
    );
    let reply = client
        .call_async(
            command,
            &CallBudget {
                deadline: crate::protocol::time::MonoInstant(10000),
                cancellation: Default::default(),
            },
        )
        .await;
    shutdown.cancel();
    let _ = server.await.unwrap().unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(reply.is_err_and(|e| e.code == ErrorCode::UnknownOutcome));
}

// ---------------------------------------------------------------------------
// Peer sniffing, peer admission and the connection gauge.
// ---------------------------------------------------------------------------

/// A well-formed peer hello frame body.
pub(super) fn hello_bytes() -> Vec<u8> {
    use crate::sync::frame::{BASE_CAP, PeerHello, PeerLimits, Role};
    PeerFrame::Hello(PeerHello {
        envelope: 1,
        node: crate::node::NodeUuid::from_u128(7),
        tag: "aB3".parse().unwrap(),
        pubkey: "11".repeat(32),
        boot: uuid::Uuid::from_u128(9),
        caps: vec![BASE_CAP.into()],
        limits: PeerLimits {
            max_frame: 1 << 20,
            batch_entries: 256,
            batch_bytes: 256 * 1024,
            credit: 1 << 20,
        },
        nonce: "22".repeat(32),
        role: Role::Dialer,
    })
    .encode()
    .unwrap()
}

async fn expect_busy(stream: &mut UnixStream, detail: &str) {
    let body = tokio::time::timeout(Duration::from_secs(2), read_frame(stream))
        .await
        .expect("a busy close arrives")
        .unwrap();
    match PeerFrame::decode(&body).unwrap() {
        PeerFrame::Close {
            reason: crate::sync::frame::CloseReason::Busy,
            detail: Some(got),
        } => assert_eq!(got, detail),
        other => panic!("expected close busy, got {other:?}"),
    }
}

struct PeerDaemon {
    path: std::path::PathBuf,
    root: std::path::PathBuf,
    instance: crate::node::NodeUuid,
    service: Arc<CountingService>,
    shutdown: Cancellation,
    gauge: ConnectionGauge,
    server: tokio::task::JoinHandle<io::Result<ServeOutcome>>,
}
impl PeerDaemon {
    fn start(peers: Option<PeerIntake>) -> Self {
        let (listener, path, root, paths) = guarded_listener();
        let uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
        let service = Arc::new(CountingService(
            AtomicUsize::new(0),
            instance_id(&paths).to_string(),
            listener.boot_id().to_string(),
        ));
        let shutdown = Cancellation::default();
        let gauge = ConnectionGauge::default();
        let server = tokio::spawn(serve_with_gate(
            listener,
            instance_id(&paths),
            service.clone(),
            Arc::new(TestClock),
            uid,
            shutdown.clone(),
            Arc::new(LiveServiceGate::new()),
            peers,
            gauge.clone(),
        ));
        Self {
            path,
            root,
            instance: instance_id(&paths),
            service,
            shutdown,
            gauge,
            server,
        }
    }
    async fn peer(&self) -> UnixStream {
        let mut stream = UnixStream::connect(&self.path).await.unwrap();
        write_frame(&mut stream, &hello_bytes()).await.unwrap();
        stream
    }
    async fn stop(self) {
        self.shutdown.cancel();
        let outcome = tokio::time::timeout(Duration::from_secs(5), self.server)
            .await
            .expect("the daemon drains")
            .unwrap()
            .unwrap();
        assert!(matches!(outcome, ServeOutcome::Drained));
        let _ = std::fs::remove_dir_all(self.root);
    }
}

async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
async fn hello_first_frame_reaches_peer_intake_without_ordinary_permit() {
    let (intake, mut rx) = PeerIntake::with_slots(16);
    let daemon = PeerDaemon::start(Some(intake));
    let _peer = daemon.peer().await;
    let offered = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("the hello reaches the intake")
        .unwrap();
    assert_eq!(offered.first_frame, hello_bytes());
    // The peer is still open and holds a peer permit, so if it also held an
    // ordinary one, 32 ordinary connections plus it would exceed the cap.
    let mut idle = Vec::new();
    for _ in 0..MAX_CONNECTIONS - 1 {
        idle.push(UnixStream::connect(&daemon.path).await.unwrap());
    }
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut last = UnixStream::connect(&daemon.path).await.unwrap();
    write_frame(&mut last, &health_request(daemon.instance, "h"))
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut last))
        .await
        .expect("the 32nd ordinary connection is admitted")
        .unwrap();
    assert!(!reply.is_empty());
    assert_eq!(daemon.service.0.load(Ordering::SeqCst), 1);
    drop((idle, offered));
    daemon.stop().await;
}

#[tokio::test]
async fn seventeenth_peer_gets_busy() {
    let (intake, mut rx) = PeerIntake::with_slots(MAX_PEER_CONNECTIONS);
    let daemon = PeerDaemon::start(Some(intake));
    let mut admitted = Vec::new();
    let mut held = Vec::new();
    for _ in 0..MAX_PEER_CONNECTIONS {
        held.push(daemon.peer().await);
        admitted.push(
            tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    let mut seventeenth = daemon.peer().await;
    expect_busy(&mut seventeenth, "peer connections full (16)").await;
    // Releasing one admitted peer's permit frees a slot for the next hello.
    drop(admitted.pop());
    let _again = daemon.peer().await;
    let offered = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("a freed slot admits the next peer")
        .unwrap();
    drop((offered, admitted, held));
    daemon.stop().await;
}

#[tokio::test]
async fn none_intake_gives_busy() {
    let daemon = PeerDaemon::start(None);
    let mut peer = daemon.peer().await;
    expect_busy(&mut peer, "sync not running").await;
    assert_eq!(daemon.service.0.load(Ordering::SeqCst), 0);
    daemon.stop().await;
}

#[tokio::test]
async fn refused_offer_gives_busy() {
    let (intake, rx) = PeerIntake::with_slots(1);
    drop(rx);
    let daemon = PeerDaemon::start(Some(intake));
    let mut peer = daemon.peer().await;
    expect_busy(&mut peer, "sync busy").await;
    daemon.stop().await;
}

#[tokio::test]
async fn uid_check_runs_first() {
    let (intake, mut rx) = PeerIntake::with_slots(1);
    let (stream, mut remote) = UnixStream::pair().unwrap();
    let uid = stream.peer_cred().unwrap().uid();
    write_frame(&mut remote, &hello_bytes()).await.unwrap();
    let service = Arc::new(CountingService(
        AtomicUsize::new(0),
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ));
    let error = serve_connection(
        stream,
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        service,
        Arc::new(TestClock),
        uid.wrapping_add(1),
        Cancellation::default(),
        Arc::new(Semaphore::new(5)),
        Arc::new(Semaphore::new(1)),
        Arc::new(LiveServiceGate::new()),
        Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap(),
        Arc::new(Semaphore::new(1)),
        Some(intake),
        Arc::new(Semaphore::new(MAX_PEER_CONNECTIONS)),
        ConnectionGauge::default().enter(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        rx.try_recv().is_err(),
        "a foreign uid never reaches the intake"
    );
}

#[tokio::test]
async fn gauge_counts_ordinary_not_peer_and_guard_decrements() {
    let (intake, mut rx) = PeerIntake::with_slots(16);
    let daemon = PeerDaemon::start(Some(intake));
    assert_eq!(daemon.gauge.count(), 0);
    let ordinary = UnixStream::connect(&daemon.path).await.unwrap();
    eventually("the idle ordinary connection to count", || {
        daemon.gauge.count() == 1
    })
    .await;
    let _peer = daemon.peer().await;
    let offered = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(daemon.gauge.count(), 1, "the peer is not counted");
    drop(ordinary);
    eventually("the closed ordinary connection to leave the count", || {
        daemon.gauge.count() == 0
    })
    .await;
    drop(offered);
    daemon.stop().await;
}
