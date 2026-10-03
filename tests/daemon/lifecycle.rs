use super::*;
use crate::daemon::paths::{InstancePaths, RuntimeContext};
use crate::ports::LocalService;
use crate::protocol::authority::PeerIdentity;
use crate::protocol::results::{
    CapabilityState, CommandResult, ComponentState, Health, HealthState,
};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

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

struct HealthService {
    version: String,
    instance: Arc<Mutex<String>>,
    boot: Arc<Mutex<String>>,
    calls: AtomicUsize,
}

/// Serves a fixed health state for the published identity.
struct StateHealthService {
    state: HealthState,
    instance: Arc<Mutex<String>>,
    boot: Arc<Mutex<String>>,
}
impl LocalService for StateHealthService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: crate::protocol::commands::Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        let mut health = Health::unknown(
            self.instance.lock().unwrap().clone(),
            self.boot.lock().unwrap().clone(),
            env!("CARGO_PKG_VERSION").into(),
            crate::protocol::wire::PROTOCOL_VERSION,
        );
        health.state = self.state.clone();
        Ok(CommandResult::Health(health))
    }
}

async fn ensure_against_state(state: HealthState) -> Result<EndpointDescriptor, ApiError> {
    let (root, context, paths) = fixture();
    let shutdown = Cancellation::default();
    let boot = Arc::new(Mutex::new(String::new()));
    let instance = Arc::new(Mutex::new(String::new()));
    let service = Arc::new(StateHealthService {
        state,
        instance: instance.clone(),
        boot: boot.clone(),
    });
    let ready = Arc::new(AtomicUsize::new(0));
    let ready_seen = ready.clone();
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        run_owner(
            &paths,
            service,
            Arc::new(TestClock),
            stop,
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |descriptor| {
                *instance.lock().unwrap() = descriptor.instance_uuid.to_string();
                *boot.lock().unwrap() = descriptor.boot_id.to_string();
                ready_seen.store(1, Ordering::SeqCst);
                Ok(())
            },
            || Ok(()),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while ready.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // `/bin/false` proves no second writer is launched: a launch could never
    // become ready, so success means the running owner was reused.
    let result = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_secs(1),
    )
    .await;
    shutdown.cancel();
    assert!(task.await.unwrap().unwrap());
    std::fs::remove_dir_all(root).unwrap();
    result
}

/// Kills: treating a reachable, identity-matching Degraded daemon as not
/// ready (Herdr's startup `ensure` then reported failure on every start).
#[tokio::test]
async fn degraded_matching_owner_is_reused_without_launch() {
    ensure_against_state(HealthState::Degraded).await.unwrap();
}

/// Kills: accepting every reachable health state, including Unavailable.
#[tokio::test]
async fn unavailable_matching_owner_fails_ensure() {
    let error = ensure_against_state(HealthState::Unavailable)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(error.detail.contains("unavailable"), "{}", error.detail);
}
impl LocalService for HealthService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: crate::protocol::commands::Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut health = Health::unknown(
            self.instance.lock().unwrap().clone(),
            self.boot.lock().unwrap().clone(),
            self.version.clone(),
            crate::protocol::wire::PROTOCOL_VERSION,
        );
        health.database.state = ComponentState::Ready;
        health.schema.state = ComponentState::Ready;
        health.host.current_execution = CapabilityState::Supported;
        health.host.coherent_enumeration = CapabilityState::Supported;
        health.state = HealthState::Healthy;
        Ok(CommandResult::Health(health))
    }
}

/// A liveness bound for tests that are not about the ensure wait itself.
const LIVENESS: Duration = Duration::from_secs(60);

fn fixture() -> (PathBuf, RuntimeContext, InstancePaths) {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("herdr-lifecycle-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    (root, context, paths)
}

#[tokio::test]
async fn live_unresponsive_owner_cannot_be_replaced() {
    let (root, context, paths) = fixture();
    let lock = OwnerLock::acquire(&paths).unwrap();
    let listener = lock.bind_socket().unwrap();
    let descriptor = lock
        .publish_endpoint(
            &listener,
            env!("CARGO_PKG_VERSION"),
            crate::protocol::wire::PROTOCOL_VERSION,
        )
        .unwrap();
    let result = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_millis(150),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(
        crate::daemon::ownership::read_descriptor(&paths, lock.instance_uuid()).unwrap(),
        descriptor
    );
    drop(listener);
    drop(lock);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn incompatible_protocol_owner_is_reported_before_health_dispatch() {
    let (root, context, paths) = fixture();
    let lock = OwnerLock::acquire(&paths).unwrap();
    let listener = lock.bind_socket().unwrap();
    let descriptor = lock
        .publish_endpoint(&listener, "0.0.1", PROTOCOL_VERSION - 1)
        .unwrap();
    let started = std::time::Instant::now();
    let error = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownWireVersion);
    assert_eq!(
        error.detail,
        crate::daemon::remedy::remedy(
            Some(ErrorClass::VersionSkew),
            &RemedyContext::VersionSkew {
                daemon: format!("0.0.1 (protocol {})", PROTOCOL_VERSION - 1),
                cli: format!(
                    "{} (protocol {PROTOCOL_VERSION})",
                    env!("CARGO_PKG_VERSION")
                ),
            }
        )
    );
    assert!(error.detail.contains("`herdr-threads daemon stop`"));
    assert!(error.detail.contains("`herdr-threads daemon ensure`"));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        crate::daemon::ownership::read_descriptor(&paths, lock.instance_uuid()).unwrap(),
        descriptor
    );
    drop(listener);
    drop(lock);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn upgraded_client_against_previous_protocol_daemon_reports_mismatch() {
    let (root, context, paths) = fixture();
    let lock = OwnerLock::acquire(&paths).unwrap();
    let listener = lock.bind_socket().unwrap();
    let descriptor = lock
        .publish_endpoint(&listener, "0.0.1", PROTOCOL_VERSION - 1)
        .unwrap();
    // The previous daemon would drop the upgraded request without replying;
    // the live listener never answers, so only the descriptor check can report.
    let error = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_secs(2),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::UnknownWireVersion);
    // The one VersionSkew remedy line (B3 taxonomy) naming both protocols.
    assert!(
        error.detail.contains(&format!(
            "daemon is version 0.0.1 (protocol {}), CLI is {} (protocol {})",
            PROTOCOL_VERSION - 1,
            env!("CARGO_PKG_VERSION"),
            PROTOCOL_VERSION
        )),
        "{}",
        error.detail
    );
    assert!(!error.detail.contains("did not become ready"));
    assert_eq!(
        crate::daemon::ownership::read_descriptor(&paths, lock.instance_uuid()).unwrap(),
        descriptor
    );
    drop(listener);
    drop(lock);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn crashed_incompatible_protocol_owner_recovers_to_new_healthy_boot() {
    let (root, context, paths) = fixture();
    let launcher = launcher(&root);
    let lock = OwnerLock::acquire(&paths).unwrap();
    let instance = lock.instance_uuid();
    let listener = lock.bind_socket().unwrap();
    let stale = lock
        .publish_endpoint(&listener, "0.0.1", PROTOCOL_VERSION - 1)
        .unwrap();
    drop(listener);
    drop(lock);
    assert!(stale.endpoint.exists());
    let recovered = ensure_running_with_timeout(
        &context,
        &launcher,
        Arc::new(TestClock),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(recovered.instance_uuid, instance);
    assert_ne!(recovered.boot_id, stale.boot_id);
    // The stale boot's socket was replaced at the same stable pathname.
    assert_eq!(recovered.endpoint, stale.endpoint);
    assert_eq!(recovered.protocol_version, PROTOCOL_VERSION);
    std::fs::write(root.join("stop"), b"stop").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while recovered.endpoint.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn oversized_namespace_fails_without_spawn_or_rewrite() {
    let (root, context, paths) = fixture();
    let owner = OwnerLock::acquire(&paths).unwrap();
    drop(owner);
    let bytes = vec![b'a'; 8193];
    std::fs::write(&paths.namespace_path, &bytes).unwrap();
    let error = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_millis(200),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert_eq!(std::fs::read(&paths.namespace_path).unwrap(), bytes);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn elected_owner_drains_before_releasing_lock() {
    let (root, _, paths) = fixture();
    let shutdown = Cancellation::default();
    let owner_shutdown = shutdown.clone();
    let service = Arc::new(HealthService {
        version: env!("CARGO_PKG_VERSION").into(),
        instance: Arc::new(Mutex::new(String::new())),
        boot: Arc::new(Mutex::new(String::new())),
        calls: AtomicUsize::new(0),
    });
    let ready = Arc::new(AtomicUsize::new(0));
    let ready_seen = ready.clone();
    let boot = service.boot.clone();
    let instance = service.instance.clone();
    let task = tokio::spawn(async move {
        run_owner(
            &paths,
            service,
            Arc::new(TestClock),
            owner_shutdown,
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |descriptor| {
                *instance.lock().unwrap() = descriptor.instance_uuid.to_string();
                *boot.lock().unwrap() = descriptor.boot_id.to_string();
                ready_seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || Ok(()),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while ready.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    assert!(task.await.unwrap().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn compatible_protocol_old_software_owner_requires_explicit_stop() {
    let (root, context, paths) = fixture();
    let shutdown = Cancellation::default();
    let boot = Arc::new(Mutex::new(String::new()));
    let instance = Arc::new(Mutex::new(String::new()));
    let service = Arc::new(HealthService {
        version: "0.0.1".into(),
        instance: instance.clone(),
        boot: boot.clone(),
        calls: AtomicUsize::new(0),
    });
    let ready = Arc::new(AtomicUsize::new(0));
    let ready_seen = ready.clone();
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        run_owner(
            &paths,
            service,
            Arc::new(TestClock),
            stop,
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |descriptor| {
                *instance.lock().unwrap() = descriptor.instance_uuid.to_string();
                *boot.lock().unwrap() = descriptor.boot_id.to_string();
                ready_seen.store(1, Ordering::SeqCst);
                Ok(())
            },
            || Ok(()),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while ready.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let error = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::DaemonVersionMismatch);
    assert!(error.detail.contains("daemon stop"));
    assert!(error.detail.contains("daemon ensure"));
    shutdown.cancel();
    assert!(task.await.unwrap().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn healthy_matching_owner_is_reused_without_launch() {
    let (root, context, paths) = fixture();
    let shutdown = Cancellation::default();
    let boot = Arc::new(Mutex::new(String::new()));
    let instance = Arc::new(Mutex::new(String::new()));
    let service = Arc::new(HealthService {
        version: env!("CARGO_PKG_VERSION").into(),
        instance: instance.clone(),
        boot: boot.clone(),
        calls: AtomicUsize::new(0),
    });
    let ready = Arc::new(AtomicUsize::new(0));
    let ready_seen = ready.clone();
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        run_owner(
            &paths,
            service,
            Arc::new(TestClock),
            stop,
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |descriptor| {
                *instance.lock().unwrap() = descriptor.instance_uuid.to_string();
                *boot.lock().unwrap() = descriptor.boot_id.to_string();
                ready_seen.store(1, Ordering::SeqCst);
                Ok(())
            },
            || Ok(()),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while ready.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let first = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let second = ensure_running_with_timeout(
        &context,
        PathBuf::from("/bin/false").as_path(),
        Arc::new(TestClock),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(first.boot_id, second.boot_id);
    shutdown.cancel();
    assert!(task.await.unwrap().unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

struct BlockingService {
    entered: Arc<AtomicUsize>,
    release: Arc<AtomicUsize>,
}

struct CapturingSink(Arc<Mutex<Vec<u8>>>);
impl DiagnosticSink for CapturingSink {
    fn install(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn write(&mut self, _: DiagnosticSource, fragment: &[u8]) -> io::Result<()> {
        self.0.lock().unwrap().extend_from_slice(fragment);
        Ok(())
    }
    fn drain(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn close(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn readiness_failure_is_written_to_elected_child_sink_and_endpoint_is_removed() {
    let (root, _, paths) = fixture();
    let output = Arc::new(Mutex::new(Vec::new()));
    let result = run_owner(
        &paths,
        Arc::new(HealthService {
            version: env!("CARGO_PKG_VERSION").into(),
            instance: Arc::new(Mutex::new(String::new())),
            boot: Arc::new(Mutex::new(String::new())),
            calls: AtomicUsize::new(0),
        }),
        Arc::new(TestClock),
        Cancellation::default(),
        CapturingSink(output.clone()),
        BufferLimits::new(1024, 256).unwrap(),
        |_| Err(io::Error::other("database open failed")),
        || Ok(()),
    )
    .await;
    assert!(result.is_err());
    assert!(String::from_utf8_lossy(&output.lock().unwrap()).contains("database open failed"));
    assert!(!paths.descriptor_path.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn factory_failure_logs_and_removes_only_unpublished_bound_socket() {
    let (root, _, paths) = fixture();
    let output = Arc::new(Mutex::new(Vec::new()));
    let boot = Arc::new(Mutex::new(None));
    let seen_boot = boot.clone();
    let drained = Arc::new(AtomicUsize::new(0));
    let drained_seen = drained.clone();
    let drain_paths = paths.clone();
    let result = run_owner_with_factory(
        &paths,
        Arc::new(TestClock),
        Cancellation::default(),
        CapturingSink(output.clone()),
        BufferLimits::new(1024, 256).unwrap(),
        move |_, bound_boot, _| {
            *seen_boot.lock().unwrap() = Some(bound_boot);
            Err(io::Error::other("factory database failed"))
        },
        |_| panic!("failed factory must not publish readiness"),
        move || async move {
            assert_eq!(
                OwnerLock::acquire(&drain_paths).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            drained_seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(drained.load(Ordering::SeqCst), 1);
    assert!(!paths.descriptor_path.exists());
    assert!(boot.lock().unwrap().is_some());
    assert!(!paths.socket_path.exists());
    assert!(String::from_utf8_lossy(&output.lock().unwrap()).contains("factory database failed"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn factory_receives_bound_identity_and_postfactory_failure_tears_down_once() {
    let (root, _, paths) = fixture();
    let observed = Arc::new(Mutex::new(None));
    let observed_factory = observed.clone();
    let ready_paths = paths.clone();
    let drain_paths = paths.clone();
    let drained = Arc::new(AtomicUsize::new(0));
    let drained_seen = drained.clone();
    let output = Arc::new(Mutex::new(Vec::new()));
    let result = run_owner_with_factory(
        &paths,
        Arc::new(TestClock),
        Cancellation::default(),
        CapturingSink(output.clone()),
        BufferLimits::new(1024, 256).unwrap(),
        move |instance, boot, token| {
            assert!(!ready_paths.descriptor_path.exists());
            assert!(!token.is_cancelled());
            *observed_factory.lock().unwrap() = Some((instance, boot));
            Ok(Arc::new(HealthService {
                version: env!("CARGO_PKG_VERSION").into(),
                instance: Arc::new(Mutex::new(instance.to_string())),
                boot: Arc::new(Mutex::new(boot.to_string())),
                calls: AtomicUsize::new(0),
            }) as Arc<dyn LocalService>)
        },
        |descriptor| {
            assert_eq!(
                Some((descriptor.instance_uuid, descriptor.boot_id)),
                *observed.lock().unwrap()
            );
            Err(io::Error::other("post-factory readiness failed"))
        },
        move || async move {
            assert_eq!(
                OwnerLock::acquire(&drain_paths).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            drained_seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(drained.load(Ordering::SeqCst), 1);
    assert!(!paths.descriptor_path.exists());
    assert!(
        String::from_utf8_lossy(&output.lock().unwrap()).contains("post-factory readiness failed")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn losing_owner_never_invokes_factory_or_installs_sink() {
    let (root, _, paths) = fixture();
    let incumbent = OwnerLock::acquire(&paths).unwrap();
    let installed = Arc::new(AtomicUsize::new(0));
    struct CountingSink(Arc<AtomicUsize>);
    impl DiagnosticSink for CountingSink {
        fn install(&mut self) -> io::Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn write(&mut self, _: DiagnosticSource, _: &[u8]) -> io::Result<()> {
            Ok(())
        }
        fn drain(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn close(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let result = run_owner_with_factory(
        &paths,
        Arc::new(TestClock),
        Cancellation::default(),
        CountingSink(installed.clone()),
        BufferLimits::new(1024, 256).unwrap(),
        |_, _, _| panic!("losing starter called factory"),
        |_| panic!("losing starter published endpoint"),
        || async { panic!("losing starter drained") },
    )
    .await
    .unwrap();
    assert!(!result);
    assert_eq!(installed.load(Ordering::SeqCst), 0);
    drop(incumbent);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_factory_hides_descriptor_and_admits_no_handler() {
    let (root, _, paths) = fixture();
    let task_paths = paths.clone();
    let entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicUsize::new(0));
    let seen_boot = Arc::new(Mutex::new(None));
    let entered_factory = entered.clone();
    let release_factory = release.clone();
    let boot_factory = seen_boot.clone();
    let task = tokio::spawn(async move {
        run_owner_with_factory(
            &task_paths,
            Arc::new(TestClock),
            Cancellation::default(),
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |_, boot, _| {
                *boot_factory.lock().unwrap() = Some(boot);
                entered_factory.store(1, Ordering::SeqCst);
                while release_factory.load(Ordering::SeqCst) == 0 {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(io::Error::other("construction released"))
            },
            |_| panic!("blocked factory advertised readiness"),
            || async { Ok(()) },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    assert!(!paths.descriptor_path.exists());
    assert!(seen_boot.lock().unwrap().is_some());
    let socket = paths.socket_path.clone();
    assert!(socket.exists());
    let mut peer = tokio::net::UnixStream::connect(&socket).await.unwrap();
    use tokio::io::AsyncReadExt;
    let mut byte = [0_u8; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(30), peer.read_exact(&mut byte))
            .await
            .is_err()
    );
    release.store(1, Ordering::SeqCst);
    assert!(task.await.unwrap().is_err());
    assert!(!socket.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn factory_cancellation_token_stops_the_elected_owner() {
    let (root, _, paths) = fixture();
    let task_paths = paths.clone();
    let captured = Arc::new(Mutex::new(None));
    let captured_factory = captured.clone();
    let ready = Arc::new(Mutex::new(None));
    let ready_seen = ready.clone();
    let drained = Arc::new(AtomicUsize::new(0));
    let drained_seen = drained.clone();
    let task = tokio::spawn(async move {
        run_owner_with_factory(
            &task_paths,
            Arc::new(TestClock),
            Cancellation::default(),
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |instance, boot, token| {
                *captured_factory.lock().unwrap() = Some(token);
                Ok(Arc::new(HealthService {
                    version: env!("CARGO_PKG_VERSION").into(),
                    instance: Arc::new(Mutex::new(instance.to_string())),
                    boot: Arc::new(Mutex::new(boot.to_string())),
                    calls: AtomicUsize::new(0),
                }) as Arc<dyn LocalService>)
            },
            move |descriptor| {
                *ready_seen.lock().unwrap() = Some(descriptor.clone());
                Ok(())
            },
            move || async move {
                drained_seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    });
    let descriptor = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(descriptor) = ready.lock().unwrap().clone() {
                break descriptor;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    captured.lock().unwrap().as_ref().unwrap().cancel();
    assert!(task.await.unwrap().unwrap());
    assert_eq!(drained.load(Ordering::SeqCst), 1);
    assert!(!descriptor.endpoint.exists());
    assert!(!paths.descriptor_path.exists());
    std::fs::remove_dir_all(root).unwrap();
}
impl LocalService for BlockingService {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _: crate::protocol::commands::Command,
        _: PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.entered.store(1, Ordering::SeqCst);
        while self.release.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(ApiError::cancelled("released"))
    }
}

#[tokio::test]
async fn shutdown_waits_for_synchronous_handler_before_releasing_lock() {
    let (root, _, paths) = fixture();
    let path_for_probe = paths.clone();
    let shutdown = Cancellation::default();
    let entered = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(Mutex::new(None));
    let drained = Arc::new(AtomicUsize::new(0));
    let ready_seen = ready.clone();
    let drained_seen = drained.clone();
    let service = Arc::new(BlockingService {
        entered: entered.clone(),
        release: release.clone(),
    });
    let stop = shutdown.clone();
    let task = tokio::spawn(async move {
        run_owner(
            &paths,
            service,
            Arc::new(TestClock),
            stop,
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |descriptor| {
                *ready_seen.lock().unwrap() = Some(descriptor.clone());
                Ok(())
            },
            move || {
                drained_seen.store(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    });
    let descriptor = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(value) = ready.lock().unwrap().clone() {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::new(TestClock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
    );
    let clock = TestClock;
    let call = tokio::spawn(async move {
        client
            .call_async(
                Command::Health,
                &CallBudget {
                    deadline: MonoInstant(clock.monotonic_now().0 + 2000),
                    cancellation: Cancellation::default(),
                },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while entered.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    shutdown.cancel();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(OwnerLock::acquire(&path_for_probe).is_err());
    assert_eq!(drained.load(Ordering::SeqCst), 0);
    release.store(1, Ordering::SeqCst);
    let _ = call.await.unwrap();
    assert!(task.await.unwrap().unwrap());
    assert_eq!(drained.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "subprocess fixture invoked explicitly by lifecycle tests"]
fn subprocess_owner_fixture() {
    // Exit once the test that launched this owner is gone, however it ended
    // (an untagged fixture outlived a killed test run, ht-zo4).
    #[cfg(feature = "test-support")]
    crate::test_support::owner_watch::watch_from_env();
    let root = PathBuf::from(std::env::var_os("HERDR_LIFECYCLE_ROOT").unwrap());
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let shutdown = Cancellation::default();
        let stop = shutdown.clone();
        let stop_path = root.join("stop");
        let crash_path = root.join("crash");
        tokio::spawn(async move {
            loop {
                if crash_path.exists() {
                    std::process::exit(9);
                }
                if stop_path.exists() {
                    stop.cancel();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let boot = Arc::new(Mutex::new(String::new()));
        let instance = Arc::new(Mutex::new(String::new()));
        let service = Arc::new(HealthService {
            version: env!("CARGO_PKG_VERSION").into(),
            instance: instance.clone(),
            boot: boot.clone(),
            calls: AtomicUsize::new(0),
        });
        run_owner(
            &paths,
            service,
            Arc::new(TestClock),
            shutdown,
            crate::daemon::diagnostics::WriterSink(Vec::<u8>::new()),
            BufferLimits::new(1024, 256).unwrap(),
            move |descriptor| {
                *instance.lock().unwrap() = descriptor.instance_uuid.to_string();
                *boot.lock().unwrap() = descriptor.boot_id.to_string();
                Ok(())
            },
            || Ok(()),
        )
        .await
        .unwrap();
    });
}

fn launcher(root: &std::path::Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("launch.sh");
    let executable = std::env::current_exe().unwrap();
    // The fixture is started by the production detached spawn, which does not
    // tag test children: the launcher names this test process as its owner.
    let owner = std::process::id();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{}/args-$$.tmp\"\nmv \"{}/args-$$.tmp\" \"{}/args-$$\"\nHERDR_LIFECYCLE_ROOT='{}' {}={owner} {}={owner} exec '{}' --ignored --exact daemon::lifecycle::tests::subprocess_owner_fixture\n",
        root.display(),
        root.display(),
        root.display(),
        root.display(),
        crate::daemon::lifecycle::TEST_OWNER_PID_ENV,
        crate::test_support::spawn::OWNER_ENV,
        executable.display()
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[test]
#[ignore = "subprocess fixture invoked explicitly by environment test"]
fn subprocess_resolved_environment_fixture() {
    let (root, _, _) = fixture();
    let selected_binary = root.join("selected-herdr");
    for (name, selected) in [("none", None), ("some", Some(selected_binary.clone()))] {
        let output = root.join(format!("{name}.txt"));
        let script = root.join(format!("{name}.sh"));
        std::fs::write(&script, format!("#!/bin/sh\nprintf '%s\\n' \"${{HERDR_BIN_PATH-<unset>}}\" > '{}.tmp'\nprintf '%s\\n' \"$@\" >> '{}.tmp'\nmv '{}.tmp' '{}'\n", output.display(), output.display(), output.display(), output.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context =
            RuntimeContext::explicit(root.clone(), root.join("host.sock"), selected).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        paths.prepare_instance_dir().unwrap();
        spawn_detached(&script, &context, &paths, &StartAttempt::new()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let observed = loop {
            if let Ok(value) = std::fs::read_to_string(&output) {
                break value;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "detached child did not write environment probe"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            observed.lines().next().unwrap(),
            if name == "none" {
                "<unset>"
            } else {
                selected_binary.to_str().unwrap()
            }
        );
        assert!(observed.contains(&format!(
            "--state-dir\n{}\n--host-endpoint\n{}\n",
            context.state_dir.display(),
            context.host_endpoint.display()
        )));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn detached_child_uses_resolved_herdr_binary_environment() {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    crate::test_support::isolation::scrub_env(&mut command);
    let status = command
        .env("HERDR_BIN_PATH", "/nonexistent/conflicting-herdr")
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon::lifecycle::tests::subprocess_resolved_environment_fixture")
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
async fn racing_ensure_callers_converge_and_crash_releases_owner() {
    let (root, context, paths) = fixture();
    let launcher = launcher(&root);
    // A liveness bound: this test is about convergence, and a daemon start in
    // a loaded parallel suite can outrun the in-process (unscaled) 5 s wait.
    let results = tokio::join!(
        ensure_running_with_timeout(&context, &launcher, Arc::new(TestClock), LIVENESS),
        ensure_running_with_timeout(&context, &launcher, Arc::new(TestClock), LIVENESS),
    );
    let first = results.0.unwrap();
    let second = results.1.unwrap();
    assert_eq!(first.boot_id, second.boot_id);
    assert_eq!(first.pid, second.pid);
    let launch_args = std::fs::read_to_string(root.join(format!("args-{}", first.pid))).unwrap();
    assert!(launch_args.contains("daemon\nrun\n--state-dir\n"));
    assert!(launch_args.contains(&format!(
        "{}\n--host-endpoint\n{}\n",
        context.state_dir.display(),
        context.host_endpoint.display()
    )));
    std::fs::write(root.join("crash"), b"crash").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while OwnerLock::acquire(&paths).is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    std::fs::remove_file(root.join("crash")).unwrap();
    let recovered = ensure_running_with_timeout(&context, &launcher, Arc::new(TestClock), LIVENESS)
        .await
        .unwrap();
    assert_ne!(first.boot_id, recovered.boot_id);
    // Same stable pathname, new boot: an allowlist naming it stays valid.
    assert_eq!(first.endpoint, recovered.endpoint);
    let owner_lock_identity = crate::daemon::ownership::owner_lock_identity(&paths).unwrap();
    std::fs::write(root.join("stop"), b"stop").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !crate::daemon::ownership::previous_owner_released(&paths, owner_lock_identity)
            .unwrap()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!recovered.endpoint.exists());
    assert!(!paths.descriptor_path.exists());
    std::fs::remove_dir_all(root).unwrap();
}
