use super::*;
use crate::{
    client::local::LocalSocketClient,
    daemon::{
        control::{ControlService, StopController},
        health::HealthInputs,
    },
    ports::LocalService,
    protocol::{
        authority::PeerIdentity,
        commands::Command,
        pagination::{Consistency, Page, StopReason},
        results::{ApiError, CapabilityList, CommandResult, ErrorCode, MessageSummary},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
        wire::{PROTOCOL_VERSION, WireRequest, WireResponse},
    },
};
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

/// The top-level JSON keys of `Health` before this task, frozen from the unchanged
/// struct: a pre-change CLI's `deny_unknown_fields` decoder accepts exactly these.
const PRE_CHANGE_HEALTH_KEYS: &[&str] = &[
    "software_version",
    "protocol_version",
    "instance_id",
    "boot_id",
    "state",
    "database",
    "schema",
    "host",
    "last_reconciliation_at",
    "last_scheduler_tick_at",
    "harness",
    "unresolved_seats",
    "held_targets",
    "retirement_pending",
    "retirement_degraded",
    "host_timeouts",
    "queue_rejections",
    "queue_depth",
    "settings",
    "limitations",
    "notes",
];

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(5_000),
        cancellation: Cancellation::default(),
    }
}

struct NoDomain;
impl LocalService for NoDomain {
    crate::unserved_local_service_routes!(
        service_control,
        audit_service_disconnect,
        service_operation,
        handle_with_output
    );
    fn handle(
        &self,
        _command: Command,
        _peer: PeerIdentity,
        _budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(api_error(ErrorCode::NotFound, "no domain"))
    }
}

fn api_error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}

fn daemon_handler(instance: Uuid, boot: Uuid) -> impl LocalService {
    ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        NoDomain,
    )
}

fn short_socket_path() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/htcap-{}.sock",
        &Uuid::new_v4().simple().to_string()[..8]
    ))
}

fn read_request(stream: &mut UnixStream) -> Option<WireRequest> {
    let mut prefix = [0u8; 4];
    stream.read_exact(&mut prefix).ok()?;
    let mut body = vec![0u8; u32::from_be_bytes(prefix) as usize];
    stream.read_exact(&mut body).ok()?;
    WireRequest::decode(&body).ok()
}

fn reply(
    stream: &mut UnixStream,
    request: &WireRequest,
    boot: Uuid,
    result: Result<CommandResult, ApiError>,
) {
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        instance: request.expected_instance.clone(),
        daemon_boot: boot.to_string(),
        result,
    };
    let body = serde_json::to_vec(&response).unwrap();
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&body).unwrap();
}

/// A fixture daemon on a Unix socket; `answer` gets each decoded request and may
/// return a result to send, or `None` to close without replying.
struct Fixture {
    path: PathBuf,
    accepted: Arc<AtomicUsize>,
}
impl Fixture {
    fn start(
        answer: impl Fn(&WireRequest) -> Option<Result<CommandResult, ApiError>> + Send + 'static,
    ) -> Self {
        let path = short_socket_path();
        let listener = UnixListener::bind(&path).unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        std::thread::spawn(move || {
            let boot = Uuid::new_v4();
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                count.fetch_add(1, Ordering::SeqCst);
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                if let Some(result) = answer(&request) {
                    reply(&mut stream, &request, boot, result);
                }
            }
        });
        Self { path, accepted }
    }
    fn client(&self) -> LocalSocketClient {
        LocalSocketClient::new(
            self.path.clone(),
            Arc::new(FixedClock),
            Uuid::new_v4(),
            None,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn list(names: &[&str]) -> CommandResult {
    CommandResult::Capabilities(CapabilityList {
        capabilities: names.iter().map(|name| (*name).to_owned()).collect(),
    })
}

/// Sends a History request with `full_bodies: true` through the daemon handler:
/// the request must pass validation and reach the domain (which here answers
/// NotFound), so the field is neither ignored at decode nor refused.
fn probe_history_full_bodies() {
    use crate::protocol::{commands::HistoryQuery, ids::ThreadId, pagination::PageRequest};
    let handler = daemon_handler(Uuid::new_v4(), Uuid::new_v4());
    let command = Command::History(HistoryQuery {
        thread: ThreadId::new("t"),
        page: PageRequest::default(),
        initial: None,
        full_bodies: true,
    });
    assert!(command.validate().is_ok());
    let json = serde_json::to_value(&command).unwrap();
    assert_eq!(
        serde_json::from_value::<Command>(json).unwrap(),
        command,
        "full_bodies must survive the wire"
    );
    let outcome = handler.handle(command, PeerIdentity::from_kernel(501), &budget());
    let error = outcome.expect_err("NoDomain answers NotFound");
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn capability_constants_are_stable() {
    assert_eq!(HISTORY_FULL_BODIES, "history.full_bodies");
    assert_eq!(HOOK_PARSE_FAILURE_REPORT, "hook.parse_failure_report");
    assert_eq!(
        ADVERTISED,
        &["history.full_bodies", "hook.parse_failure_report"]
    );
}

/// Every capability this daemon advertises names a feature it serves: a bead
/// that lands a capability adds its arm here with a probe that drives the
/// handler through the daemon. Kills: advertising a capability whose request
/// field the daemon ignores or refuses (ht-p03.105).
#[test]
fn every_advertised_capability_has_a_handler() {
    for name in ADVERTISED {
        match *name {
            HISTORY_FULL_BODIES => probe_history_full_bodies(),
            HOOK_PARSE_FAILURE_REPORT => probe_hook_parse_failure_report(),
            other => panic!("{other} is advertised but has no handler probe here"),
        }
    }
}

#[test]
fn capabilities_reply_round_trips_and_defaults_empty() {
    let instance = Uuid::new_v4().to_string();
    let boot = Uuid::new_v4().to_string();
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: "r1".into(),
        instance: instance.clone(),
        daemon_boot: boot.clone(),
        result: Ok(list(&[HISTORY_FULL_BODIES, "x.y"])),
    };
    let json = serde_json::to_string(&response).unwrap();
    assert_eq!(
        serde_json::from_str::<WireResponse>(&json).unwrap(),
        response
    );

    let bare = format!(
        r#"{{"version":1,"request_id":"r1","instance":"{instance}","daemon_boot":"{boot}","result":{{"Ok":{{"kind":"capabilities","data":{{}}}}}}}}"#
    );
    let decoded: WireResponse = serde_json::from_str(&bare).unwrap();
    assert_eq!(decoded.result, Ok(list(&[])));
    assert_eq!(
        serde_json::to_value(Command::Capabilities).unwrap(),
        serde_json::json!({"kind": "capabilities"})
    );
    assert!(Command::Capabilities.validate().is_ok());
}

#[test]
fn this_daemon_advertises_exactly_advertised() {
    let handler = daemon_handler(Uuid::new_v4(), Uuid::new_v4());
    let result = handler
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap();
    let CommandResult::Capabilities(advertised) = result else {
        panic!("expected a capabilities result, got {result:?}");
    };
    assert_eq!(advertised.capabilities, ADVERTISED);
    let caps = Capabilities::from_list(advertised.capabilities);
    assert!(
        !caps.supports(HOOK_PARSE_FAILURE_REPORT)
            || ADVERTISED.contains(&HOOK_PARSE_FAILURE_REPORT)
    );
    assert!(!caps.supports(HISTORY_FULL_BODIES) || ADVERTISED.contains(&HISTORY_FULL_BODIES));
    assert!(!caps.supports("nonexistent.capability"));
}

#[test]
fn old_daemon_that_closes_reads_as_no_capabilities() {
    let fixture = Fixture::start(|_| None);
    let client = fixture.client();
    let start = Instant::now();
    let caps = client.capabilities(&budget());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(caps.is_empty());
    assert!(!caps.supports(HISTORY_FULL_BODIES));
}

#[test]
fn old_daemon_that_answers_an_error_reads_as_no_capabilities() {
    let fixture = Fixture::start(|_| {
        Some(Err(api_error(
            ErrorCode::InvalidRequest,
            "unknown command kind",
        )))
    });
    let client = fixture.client();
    let caps = client.capabilities(&budget());
    assert!(caps.is_empty());
    assert!(!caps.supports(HISTORY_FULL_BODIES));
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1);
}

#[test]
fn pre_change_cli_decodes_this_daemons_hello_and_history() {
    let instance = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let handler = daemon_handler(instance, boot);
    let CommandResult::Health(health) = handler
        .handle(Command::Health, PeerIdentity::from_kernel(501), &budget())
        .unwrap()
    else {
        panic!("expected Health");
    };
    let value = serde_json::to_value(&health).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut frozen = PRE_CHANGE_HEALTH_KEYS.to_vec();
    frozen.sort_unstable();
    assert_eq!(keys, frozen, "Health must keep its pre-change key set");

    // The deny_unknown_fields decoders accept the hello through a full response, and a
    // History reply decodes too: the pre-change CLI reaches a History call.
    let history = CommandResult::History(Page::<MessageSummary> {
        items: vec![],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    });
    for result in [CommandResult::Health(health), history] {
        let response = WireResponse {
            version: PROTOCOL_VERSION,
            request_id: "r1".into(),
            instance: instance.to_string(),
            daemon_boot: boot.to_string(),
            result: Ok(result),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            serde_json::from_str::<WireResponse>(&json).unwrap(),
            response
        );
    }
}

#[test]
fn client_caches_capabilities_per_session() {
    let fixture = Fixture::start(|_| Some(Ok(list(&[HISTORY_FULL_BODIES]))));
    let client = fixture.client();
    let first = client.capabilities(&budget());
    let second = client.capabilities(&budget());
    assert!(first.supports(HISTORY_FULL_BODIES));
    assert!(!first.supports(HOOK_PARSE_FAILURE_REPORT));
    assert_eq!(first, second);
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1);
}

/// ht-p03.23. Kills: a control route that drops the hook's parse-failure
/// report (no count for Health), one that answers an error, and a request
/// that validates with an unknown harness or an unbounded detail.
#[test]
fn daemon_counts_a_hook_parse_failure_report() {
    probe_hook_parse_failure_report();
}

fn probe_hook_parse_failure_report() {
    use crate::daemon::logs::{HookParseFailures, RateLimitedLaneLog};
    use crate::protocol::commands::HookParseFailure;
    let (instance, boot) = (Uuid::new_v4(), Uuid::new_v4());
    let failures = std::sync::Arc::new(HookParseFailures::new(std::sync::Arc::new(
        RateLimitedLaneLog::new(
            std::sync::Arc::new(FixedClock),
            std::sync::Arc::new(|_: &str| {}),
        ),
    )));
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        NoDomain,
    )
    .with_hook_parse_failures(std::sync::Arc::clone(&failures));
    let report = |harness: &str, detail: &str| {
        Command::HookParseFailure(HookParseFailure {
            harness: harness.into(),
            detail: detail.into(),
        })
    };
    let result = handler
        .handle(
            report("claude", "Invalid"),
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap();
    assert_eq!(result, CommandResult::HookParseFailureRecorded);
    assert_eq!(failures.snapshot(), vec![("claude".to_owned(), 1)]);
    assert!(report("claude", "x").validate().is_ok());
    assert!(report("human", "x").validate().is_err());
    assert!(report("codex", &"x".repeat(257)).validate().is_err());
    assert_eq!(
        serde_json::to_value(report("codex", "Invalid")).unwrap(),
        serde_json::json!({"kind": "hook_parse_failure",
                           "args": {"harness": "codex", "detail": "Invalid"}})
    );
}
