//! Mod watch connection tests (ht-j16.2). Included from
//! `src/daemon/transport/watch_connection.rs`. The first group drives
//! [`serve`] over a socket pair with a scripted service; the second drives the
//! real accept loop (`serve_with_gate`) on a temp socket.
use super::*;
use crate::{
    daemon::ownership::OwnedAsyncListener,
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::Command,
        ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId},
        results::{CommandResult, Health},
        time::UtcMillis,
        watch::WatchRequest,
    },
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_234)
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

type Script = Box<dyn Fn(&WatchRequest) -> Result<u64, WatchRefusalReason> + Send + Sync>;

/// A service whose `watch_register` is scripted. Keeps every sink it was
/// handed and every channel `watch_unregister` was told about.
struct WatchFake {
    script: Script,
    next_id: AtomicUsize,
    registered: Mutex<Vec<(WatchRequest, Arc<dyn ModChannelSink>)>>,
    register_calls: AtomicUsize,
    unregistered: Mutex<Vec<(ModChannelId, UtcMillis)>>,
    instance: String,
    boot: String,
}
impl WatchFake {
    fn new(script: Script) -> Arc<Self> {
        Self::with_ids(
            script,
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        )
    }
    fn with_ids(script: Script, instance: String, boot: String) -> Arc<Self> {
        Arc::new(Self {
            script,
            next_id: AtomicUsize::new(1),
            registered: Mutex::new(Vec::new()),
            register_calls: AtomicUsize::new(0),
            unregistered: Mutex::new(Vec::new()),
            instance,
            boot,
        })
    }
    fn accepting(version: u64) -> Arc<Self> {
        Self::new(Box::new(move |_| Ok(version)))
    }
    fn sink(&self, index: usize) -> Arc<dyn ModChannelSink> {
        Arc::clone(&self.registered.lock().unwrap()[index].1)
    }
}
impl LocalService for WatchFake {
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
        Ok(CommandResult::Health(Health::unknown(
            self.instance.clone(),
            self.boot.clone(),
            peer.effective_uid().to_string(),
            1,
        )))
    }
    fn watch_register(
        &self,
        request: &WatchRequest,
        sink: Arc<dyn ModChannelSink>,
        _budget: &CallBudget,
    ) -> Result<(ModChannelId, u64), WatchRefusalReason> {
        self.register_calls.fetch_add(1, Ordering::SeqCst);
        let version = (self.script)(request)?;
        self.registered
            .lock()
            .unwrap()
            .push((request.clone(), sink));
        Ok((
            ModChannelId(self.next_id.fetch_add(1, Ordering::SeqCst) as u64),
            version,
        ))
    }
    fn watch_unregister(&self, channel: ModChannelId, now: UtcMillis) {
        self.unregistered.lock().unwrap().push((channel, now));
    }
}

fn claim() -> CallerClaim {
    CallerClaim {
        instance: "ignored".into(),
        seat: SeatId::new("seat-w"),
        binding_generation: 3,
        role: CallerRole::TopLevel,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("native-w"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("pane-w"),
    }
}

fn watch_request(instance: &str, boot: Option<&str>) -> WatchWireRequest {
    WatchWireRequest {
        version: PROTOCOL_VERSION,
        request_id: "w1".into(),
        expected_instance: instance.into(),
        expected_boot: boot.map(str::to_owned),
        watch: WatchRequest { claim: claim() },
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> T {
    let frame = tokio::time::timeout(Duration::from_secs(5), read_frame(stream))
        .await
        .expect("a frame within five seconds")
        .expect("a readable frame");
    serde_json::from_slice(&frame).unwrap()
}

async fn send_watch(stream: &mut UnixStream, request: &WatchWireRequest) {
    write_frame(stream, &serde_json::to_vec(request).unwrap())
        .await
        .unwrap();
}

/// Starts `serve` on one end of a pair. Returns the client end, the task and
/// the watch slot semaphore.
struct Served {
    client: UnixStream,
    task: tokio::task::JoinHandle<io::Result<()>>,
    shutdown: Cancellation,
    slots: Arc<Semaphore>,
}
fn start(fake: Arc<WatchFake>, request: &WatchWireRequest) -> Served {
    let (client, server) = UnixStream::pair().unwrap();
    let shutdown = Cancellation::default();
    let slots = Arc::new(Semaphore::new(1));
    let permit = slots.clone().try_acquire_owned().unwrap();
    let task = tokio::spawn(serve(
        server,
        request.clone(),
        permit,
        fake.instance.clone(),
        fake.boot.clone(),
        fake.clone(),
        Arc::new(TestClock),
        shutdown.clone(),
    ));
    Served {
        client,
        task,
        shutdown,
        slots,
    }
}

async fn wait_until(what: &str, condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn accepted_reply_carries_attention_version_and_the_claim() {
    let fake = WatchFake::accepting(7);
    let request = watch_request(&fake.instance, Some(&fake.boot));
    let mut served = start(fake.clone(), &request);
    let reply: WatchReply = read_json(&mut served.client).await;
    assert_eq!(reply.version, PROTOCOL_VERSION);
    assert_eq!(reply.request_id, "w1");
    assert_eq!(reply.instance, fake.instance);
    assert_eq!(reply.daemon_boot, fake.boot);
    assert_eq!(
        reply.outcome,
        WatchOutcome::Accepted(WatchAccepted {
            attention_version: 7
        })
    );
    {
        let registered = fake.registered.lock().unwrap();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].0.claim, claim());
    }
    assert!(fake.unregistered.lock().unwrap().is_empty());
    drop(served.client);
    served.task.await.unwrap().unwrap();
}

#[tokio::test]
async fn each_refusal_reason_round_trips_in_the_reply() {
    for reason in [
        WatchRefusalReason::NoBinding,
        WatchRefusalReason::SessionMismatch,
        WatchRefusalReason::Held,
        WatchRefusalReason::Unresolved,
        WatchRefusalReason::Cooldown,
        WatchRefusalReason::Busy,
        WatchRefusalReason::Stopping,
        WatchRefusalReason::NotClaude,
        WatchRefusalReason::Disabled,
    ] {
        let fake = WatchFake::new(Box::new(move |_| Err(reason)));
        let request = watch_request(&fake.instance, None);
        let mut served = start(fake.clone(), &request);
        let reply: WatchReply = read_json(&mut served.client).await;
        assert_eq!(
            reply.outcome,
            WatchOutcome::Refused(WatchRefusal {
                reason,
                detail: None
            }),
            "{reason:?}"
        );
        served.task.await.unwrap().unwrap();
        // A refused registration leaves nothing to unregister and frees the slot.
        assert!(fake.unregistered.lock().unwrap().is_empty());
        assert_eq!(served.slots.available_permits(), 1, "{reason:?}");
    }
}

#[tokio::test]
async fn instance_and_boot_mismatch_are_session_mismatch_without_registering() {
    let fake = WatchFake::accepting(1);
    let other = uuid::Uuid::new_v4().to_string();
    for (request, expect_detail) in [
        (watch_request(&other, None), "instance"),
        (watch_request(&fake.instance, Some(&other)), "boot"),
    ] {
        let mut served = start(fake.clone(), &request);
        let reply: WatchReply = read_json(&mut served.client).await;
        let WatchOutcome::Refused(refusal) = reply.outcome else {
            panic!("expected a refusal");
        };
        assert_eq!(refusal.reason, WatchRefusalReason::SessionMismatch);
        assert!(
            refusal.detail.as_deref().unwrap().contains(expect_detail),
            "{refusal:?}"
        );
        served.task.await.unwrap().unwrap();
    }
    assert_eq!(fake.register_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pushed_attention_and_close_frames_reach_the_client_in_order() {
    let fake = WatchFake::accepting(0);
    let request = watch_request(&fake.instance, None);
    let mut served = start(fake.clone(), &request);
    let _: WatchReply = read_json(&mut served.client).await;
    let sink = fake.sink(0);
    assert!(sink.push(WatchFrame::Attention { version: 1 }));
    assert!(sink.push(WatchFrame::Attention { version: 2 }));
    assert!(sink.push(WatchFrame::Close {
        reason: WatchCloseReason::Replaced
    }));
    for expected in [
        WatchFrame::Attention { version: 1 },
        WatchFrame::Attention { version: 2 },
        WatchFrame::Close {
            reason: WatchCloseReason::Replaced,
        },
    ] {
        let frame: WatchFrame = read_json(&mut served.client).await;
        assert_eq!(frame, expected);
    }
    // After a Close the task ends and the stream reaches EOF; the registry
    // already settled the entry, so nothing is unregistered.
    served.task.await.unwrap().unwrap();
    assert!(read_frame(&mut served.client).await.is_err());
    assert!(fake.unregistered.lock().unwrap().is_empty());
    assert_eq!(served.slots.available_permits(), 1);
    // The sink reports the connection gone.
    assert!(!sink.push(WatchFrame::Attention { version: 3 }));
}

#[tokio::test]
async fn client_eof_unregisters_the_channel() {
    let fake = WatchFake::accepting(0);
    let request = watch_request(&fake.instance, None);
    let mut served = start(fake.clone(), &request);
    let _: WatchReply = read_json(&mut served.client).await;
    drop(served.client);
    served.task.await.unwrap().unwrap();
    assert_eq!(
        *fake.unregistered.lock().unwrap(),
        vec![(ModChannelId(1), UtcMillis(1_234))]
    );
    assert_eq!(served.slots.available_permits(), 1);
}

#[tokio::test]
async fn bytes_after_the_first_frame_are_a_protocol_error_that_unregisters() {
    let fake = WatchFake::accepting(0);
    let request = watch_request(&fake.instance, None);
    let mut served = start(fake.clone(), &request);
    let _: WatchReply = read_json(&mut served.client).await;
    served.client.write_all(b"x").await.unwrap();
    served.task.await.unwrap().unwrap();
    assert_eq!(fake.unregistered.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn shutdown_sends_close_stopping_and_ends_task() {
    let fake = WatchFake::accepting(0);
    let request = watch_request(&fake.instance, None);
    let mut served = start(fake.clone(), &request);
    let _: WatchReply = read_json(&mut served.client).await;
    served.shutdown.cancel();
    let frame: WatchFrame = read_json(&mut served.client).await;
    assert_eq!(
        frame,
        WatchFrame::Close {
            reason: WatchCloseReason::Stopping
        }
    );
    tokio::time::timeout(Duration::from_secs(2), served.task)
        .await
        .expect("the task ends promptly on shutdown")
        .unwrap()
        .unwrap();
    // The registry's close_all removed the entry: the connection must not
    // unregister it again.
    assert!(fake.unregistered.lock().unwrap().is_empty());
}

#[tokio::test]
async fn shutdown_forwards_a_queued_registry_close_instead_of_a_second_one() {
    let fake = WatchFake::accepting(0);
    let request = watch_request(&fake.instance, None);
    let mut served = start(fake.clone(), &request);
    let _: WatchReply = read_json(&mut served.client).await;
    fake.sink(0).push(WatchFrame::Close {
        reason: WatchCloseReason::Disabled,
    });
    // Whichever branch the select takes first, exactly one Close arrives and
    // it is the registry's when that was queued before the shutdown was seen.
    let frame: WatchFrame = read_json(&mut served.client).await;
    assert!(matches!(frame, WatchFrame::Close { .. }));
    served.shutdown.cancel();
    served.task.await.unwrap().unwrap();
}

// ---------------------------------------------------------------------------
// The real accept loop.
// ---------------------------------------------------------------------------

fn guarded_listener() -> (
    OwnedAsyncListener,
    std::path::PathBuf,
    std::path::PathBuf,
    uuid::Uuid,
) {
    use crate::daemon::{
        ownership::OwnerLock,
        paths::{InstancePaths, RuntimeContext},
    };
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("herdr-watch-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let host = root.join("host-placeholder.sock");
    let context = RuntimeContext::explicit(root.clone(), host, None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let bound = owner.bind_socket().unwrap();
    let socket = bound.path().to_owned();
    let listener = bound.into_async().unwrap();
    drop(owner);
    let instance = uuid::Uuid::parse_str(
        std::fs::read_to_string(&paths.namespace_path)
            .unwrap()
            .trim(),
    )
    .unwrap();
    (listener, socket, root, instance)
}

struct Daemon {
    path: std::path::PathBuf,
    root: std::path::PathBuf,
    instance: uuid::Uuid,
    boot: uuid::Uuid,
    shutdown: Cancellation,
    fake: Arc<WatchFake>,
    server: tokio::task::JoinHandle<io::Result<ServeOutcome>>,
}
fn daemon(script: Script) -> Daemon {
    let (listener, path, root, instance) = guarded_listener();
    let boot = listener.boot_id();
    let owner_uid = UnixStream::pair().unwrap().0.peer_cred().unwrap().uid();
    let fake = WatchFake::with_ids(script, instance.to_string(), boot.to_string());
    let shutdown = Cancellation::default();
    let server = tokio::spawn(crate::daemon::transport::serve(
        listener,
        instance,
        fake.clone(),
        Arc::new(TestClock),
        owner_uid,
        shutdown.clone(),
    ));
    Daemon {
        path,
        root,
        instance,
        boot,
        shutdown,
        fake,
        server,
    }
}
impl Daemon {
    async fn watch(&self) -> (UnixStream, WatchReply) {
        let mut stream = UnixStream::connect(&self.path).await.unwrap();
        let request = watch_request(&self.instance.to_string(), Some(&self.boot.to_string()));
        send_watch(&mut stream, &request).await;
        let reply = read_json(&mut stream).await;
        (stream, reply)
    }
    async fn health(&self) -> WireResponse {
        let mut stream = UnixStream::connect(&self.path).await.unwrap();
        let request = WireRequest {
            version: PROTOCOL_VERSION,
            request_id: "h".into(),
            expected_instance: self.instance.to_string(),
            expected_boot: None,
            output: None,
            command: Command::Health,
        };
        write_frame(&mut stream, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        read_json(&mut stream).await
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

#[tokio::test]
async fn watch_frame_is_sniffed_by_the_accept_loop() {
    let daemon = daemon(Box::new(|_| Ok(42)));
    let (stream, reply) = daemon.watch().await;
    assert_eq!(reply.instance, daemon.instance.to_string());
    assert_eq!(
        reply.outcome,
        WatchOutcome::Accepted(WatchAccepted {
            attention_version: 42
        })
    );
    assert_eq!(daemon.fake.register_calls.load(Ordering::SeqCst), 1);
    drop(stream);
    daemon.stop().await;
}

#[tokio::test]
async fn watch_connections_do_not_consume_ordinary_slots() {
    let daemon = daemon(Box::new(|_| Ok(0)));
    let mut open = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        let (stream, reply) = daemon.watch().await;
        assert!(matches!(reply.outcome, WatchOutcome::Accepted(_)));
        open.push(stream);
    }
    // MAX_CONNECTIONS watch streams are held open; an ordinary request still
    // gets an ordinary slot.
    for _ in 0..3 {
        let response = daemon.health().await;
        assert!(response.result.is_ok(), "{response:?}");
    }
    drop(open);
    daemon.stop().await;
}

#[tokio::test]
async fn watch_admission_over_64_refuses_busy_and_frees_on_close() {
    let daemon = daemon(Box::new(|_| Ok(0)));
    let mut open = Vec::new();
    for _ in 0..MAX_WATCH_CONNECTIONS {
        let (stream, reply) = daemon.watch().await;
        assert!(matches!(reply.outcome, WatchOutcome::Accepted(_)));
        open.push(stream);
    }
    let register_calls = daemon.fake.register_calls.load(Ordering::SeqCst);
    let (_extra, reply) = daemon.watch().await;
    assert_eq!(
        reply.outcome,
        WatchOutcome::Refused(WatchRefusal {
            reason: WatchRefusalReason::Busy,
            detail: None
        })
    );
    assert_eq!(
        daemon.fake.register_calls.load(Ordering::SeqCst),
        register_calls,
        "a busy refusal never reaches the registry"
    );
    // Closing one frees its slot.
    drop(open.pop());
    let fake = daemon.fake.clone();
    wait_until("the closed watch to unregister", || {
        !fake.unregistered.lock().unwrap().is_empty()
    })
    .await;
    let (_again, reply) = daemon.watch().await;
    assert!(
        matches!(reply.outcome, WatchOutcome::Accepted(_)),
        "{reply:?}"
    );
    drop(open);
    daemon.stop().await;
}

#[tokio::test]
async fn old_version_watch_frame_gets_skew_reply() {
    let daemon = daemon(Box::new(|_| Ok(0)));
    let mut stream = UnixStream::connect(&daemon.path).await.unwrap();
    let frame = format!(
        r#"{{"version":5,"request_id":"old","expected_instance":"{}","watch":{{"claim":{}}}}}"#,
        daemon.instance,
        serde_json::to_string(&claim()).unwrap()
    );
    write_frame(&mut stream, frame.as_bytes()).await.unwrap();
    let reply: WireResponse = read_json(&mut stream).await;
    assert_eq!(reply.version, 5);
    assert_eq!(reply.request_id, "old");
    assert_eq!(
        reply.result.unwrap_err().code,
        ErrorCode::UnknownWireVersion
    );
    assert_eq!(daemon.fake.register_calls.load(Ordering::SeqCst), 0);
    daemon.stop().await;
}

#[tokio::test]
async fn daemon_shutdown_closes_open_watches_with_stopping_and_drains() {
    let daemon = daemon(Box::new(|_| Ok(0)));
    let (mut stream, _) = daemon.watch().await;
    daemon.shutdown.cancel();
    let frame: WatchFrame = read_json(&mut stream).await;
    assert_eq!(
        frame,
        WatchFrame::Close {
            reason: WatchCloseReason::Stopping
        }
    );
    daemon.stop().await;
}
