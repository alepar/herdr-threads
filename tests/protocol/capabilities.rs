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

/// Sends a `RecordManagedLaunch` (ht-5n6) through the daemon handler: it must
/// validate, survive the wire and reach the domain service (which here
/// answers NotFound), not be refused as a control route.
fn probe_seat_managed_launch() {
    use crate::protocol::{
        authority::Harness,
        commands::RecordManagedLaunch,
        ids::{HostBootId, HostTargetId, SeatId, TerminalId},
    };
    let handler = daemon_handler(Uuid::new_v4(), Uuid::new_v4());
    let command = Command::RecordManagedLaunch(RecordManagedLaunch {
        seat: SeatId::new("s"),
        target: HostTargetId::new("w1:p1"),
        harness: Harness::Codex,
        terminal: TerminalId::new("term_1"),
        incarnation: "herdr-server:pid=1".into(),
        host_boot: HostBootId::new("boot"),
        target_generation: 1,
    });
    assert!(command.validate().is_ok());
    let json = serde_json::to_value(&command).unwrap();
    assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    let outcome = handler.handle(command, PeerIdentity::from_kernel(501), &budget());
    assert_eq!(
        outcome.expect_err("NoDomain answers NotFound").code,
        ErrorCode::NotFound
    );
}

fn probe_inbox_batch() {
    use crate::protocol::commands::InboxQuery;
    let handler = daemon_handler(Uuid::new_v4(), Uuid::new_v4());
    let command = Command::InboxBatch(InboxQuery {
        seat: Some(crate::protocol::ids::SeatId::new("seat-probe")),
        page: Default::default(),
    });
    assert!(command.validate().is_ok());
    let json = serde_json::to_value(&command).unwrap();
    assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    let outcome = handler.handle(command, PeerIdentity::from_kernel(501), &budget());
    assert_eq!(
        outcome.expect_err("NoDomain answers NotFound").code,
        ErrorCode::NotFound
    );
}

#[test]
fn capability_constants_are_stable() {
    assert_eq!(HISTORY_FULL_BODIES, "history.full_bodies");
    assert_eq!(HOOK_PARSE_FAILURE_REPORT, "hook.parse_failure_report");
    assert_eq!(SERVICE_SEND_V1, "service.send_v1");
    assert_eq!(HARNESS_EVIDENCE, "hook.harness_evidence");
    assert_eq!(HARNESS_STATES, "harness.states");
    assert_eq!(SEAT_MANAGED_LAUNCH, "seat.managed_launch");
    assert_eq!(INBOX_BATCH, "inbox.batch_v1");
    assert_eq!(
        ADVERTISED,
        &[
            "history.full_bodies",
            "hook.parse_failure_report",
            "service.send_v1",
            "hook.harness_evidence",
            "hook.harness_evidence_v2",
            "harness.states",
            "harness.health_v2",
            "seat.managed_launch",
            "inbox.batch_v1",
            "invitation.reject_v1",
            "participants.locations_v1",
            "picker.directory_v1"
        ]
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
            SERVICE_SEND_V1 => probe_service_send_v1(),
            HARNESS_EVIDENCE => probe_harness_evidence(),
            HARNESS_EVIDENCE_V2 => v2_capability_is_negotiated_only_with_recorder(),
            HARNESS_STATES => probe_harness_states(),
            HARNESS_HEALTH_V2 => health_v2_capability_is_negotiated_only_with_cached_provider(),
            SEAT_MANAGED_LAUNCH => probe_seat_managed_launch(),
            INBOX_BATCH => probe_inbox_batch(),
            INVITATION_REJECT => probe_invitation_reject(),
            PARTICIPANT_LOCATIONS => probe_participant_locations(),
            PICKER_DIRECTORY_V1 => probe_picker_directory(),
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
fn bare_daemon_advertises_legacy_capabilities_without_v2_recorder() {
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
    assert_eq!(
        advertised.capabilities,
        [
            "history.full_bodies",
            "hook.parse_failure_report",
            "service.send_v1",
            "hook.harness_evidence",
            "harness.states",
            "seat.managed_launch",
            "inbox.batch_v1",
            "invitation.reject_v1",
            "participants.locations_v1",
            "picker.directory_v1"
        ]
    );
    let caps = Capabilities::from_list(advertised.capabilities);
    assert!(
        !caps.supports(HOOK_PARSE_FAILURE_REPORT)
            || ADVERTISED.contains(&HOOK_PARSE_FAILURE_REPORT)
    );
    assert!(!caps.supports(HISTORY_FULL_BODIES) || ADVERTISED.contains(&HISTORY_FULL_BODIES));
    assert!(!caps.supports(HARNESS_EVIDENCE_V2));
    assert!(!caps.supports(HARNESS_HEALTH_V2));
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

struct ProbeGate(crate::protocol::ids::ServiceAuthorId);
struct ProbeGuard(crate::protocol::ids::ServiceAuthorId);
impl crate::ports::ServiceDecisionGuard for ProbeGuard {
    fn author(&self) -> &crate::protocol::ids::ServiceAuthorId {
        &self.0
    }
}
impl crate::ports::ServiceAuthorityGate for ProbeGate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: crate::protocol::ids::ServiceAuthorId,
    ) -> Result<crate::ports::ServiceConnectionAuthority, ApiError> {
        Ok(crate::ports::ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            1,
            author,
        ))
    }
    fn decision_guard<'a>(
        &'a self,
        _proof: &crate::ports::ServiceWriteTransactionProof,
        _connection: &crate::ports::ServiceConnectionAuthority,
    ) -> Result<Box<dyn crate::ports::ServiceDecisionGuard + 'a>, ApiError> {
        Ok(Box::new(ProbeGuard(self.0.clone())))
    }
    fn revoke_exact(&self, _connection: &crate::ports::ServiceConnectionAuthority) -> bool {
        true
    }
}

/// `service.send_v1` is advertised only once the daemon's store serves all three
/// operations: each of `Send`, `History` and `Receipts` against an unknown
/// subject answers `NotFound`, never `Unsupported` (which is what a daemon
/// without the handler answers).
fn probe_service_send_v1() {
    use crate::{
        ports::{ServiceAuthorityGate, StorePort},
        protocol::{
            ids::{MessageId, OperationId, ServiceAuthorId, ThreadId},
            pagination::PageRequest,
            service::{ServiceHistoryQuery, ServiceOperation, ServiceReceiptsQuery, ServiceSend},
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    let dir = std::env::temp_dir().join(format!("htcap-send-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let context = StoreContext::new(dir.join("probe.db"), Arc::new(FixedClock));
    {
        let conn = context.open_writer().unwrap();
        conn.execute(
            "INSERT INTO host_instances(id, created_at, host_boot, host_epoch) VALUES ('i', 0, 'b', 1)",
            [],
        )
        .unwrap();
    }
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let author = ServiceAuthorId::new("graph:probe");
    let gate = ProbeGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operations = [
        ServiceOperation::Send(ServiceSend {
            thread: ThreadId::new("absent"),
            body: "b".into(),
            recipients: vec![],
            deadline_millis: None,
            operation: OperationId::new("probe-send"),
        }),
        ServiceOperation::History(ServiceHistoryQuery {
            thread: ThreadId::new("absent"),
            page: PageRequest::default(),
            initial: None,
        }),
        ServiceOperation::Receipts(ServiceReceiptsQuery {
            message: MessageId::new("absent"),
            page: PageRequest::default(),
        }),
    ];
    for operation in operations {
        let label = format!("{operation:?}");
        let error = store
            .service_operation(operation, &connection, &gate, &budget(), None)
            .expect_err("an unknown subject is refused");
        assert_eq!(error.code, ErrorCode::NotFound, "{label}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

fn evidence_note(version: Option<&str>) -> crate::protocol::commands::HarnessEvidence {
    use crate::protocol::commands::{HarnessEvidence, HarnessEvidenceOutcome};
    HarnessEvidence {
        harness: "claude".into(),
        version: version.map(str::to_owned),
        unattributed_reason: version.is_none().then(|| "transcript not found".into()),
        contract_id: "0123456789abcdef".into(),
        event: "SessionStart".into(),
        outcome: HarnessEvidenceOutcome::Ok,
        session_id: Some("s1".into()),
    }
}

/// ht-xoc.4. Kills: a control route that drops a harness-evidence note or
/// answers an error, a recorder that is not reached, and a `verified` reply
/// that does not follow the row.
fn probe_harness_evidence() {
    use crate::{
        daemon::harness_evidence::HarnessEvidenceRecorder,
        ports::StorePort,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let (instance, boot) = (Uuid::new_v4(), Uuid::new_v4());
    let iso = TestIsolation::new("cap-harness-evidence");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let recorder = Arc::new(HarnessEvidenceRecorder::new(store.clone(), None, clock));
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        NoDomain,
    )
    .with_harness_evidence(recorder);
    let send = |note| {
        handler
            .handle(
                Command::HarnessEvidence(note),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap()
    };
    assert_eq!(
        send(evidence_note(Some("2.1.286"))),
        CommandResult::HarnessEvidenceRecorded { verified: false }
    );
    let row = store
        .harness_evidence("claude", "2.1.286", "0123456789abcdef", &budget())
        .unwrap();
    assert!(row.is_some_and(|row| row.lifecycle_ok_at.is_some()));
    assert_eq!(
        send(evidence_note(None)),
        CommandResult::HarnessEvidenceRecorded { verified: false }
    );
    assert!(
        store
            .last_unattributed("claude", &budget())
            .unwrap()
            .is_some()
    );
}

/// ht-xoc.5. Kills: a control route that drops `harness.states`, answers an
/// error or another result, a provider that is not reached, and a report that
/// does not follow the evidence rows.
fn probe_harness_states() {
    use crate::{
        daemon::{
            harness_evidence::HarnessEvidenceRecorder,
            harness_states::{HarnessStatesProvider, embedded_source},
        },
        ports::StorePort,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let (instance, boot) = (Uuid::new_v4(), Uuid::new_v4());
    let iso = TestIsolation::new("cap-harness-states");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let recorder = HarnessEvidenceRecorder::new(store.clone(), None, clock.clone());
    recorder
        .record(&evidence_note(Some("2.1.286")), &budget())
        .unwrap();
    let provider = Arc::new(HarnessStatesProvider::new(
        store.clone() as Arc<dyn StorePort>,
        embedded_source(),
        clock,
        Box::new(|harness| (harness == "claude").then(|| "2.1.999".to_owned())),
        None,
    ));
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        NoDomain,
    )
    .with_harness_states(provider);
    let result = handler
        .handle(
            Command::HarnessStates,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap();
    let CommandResult::HarnessStates(report) = result else {
        panic!("expected harness states, got {result:?}");
    };
    let names: Vec<&str> = report
        .harnesses
        .iter()
        .map(|h| h.harness.as_str())
        .collect();
    assert_eq!(names, ["claude", "codex", "hermes"]);
    let claude = &report.harnesses[0];
    assert_eq!(claude.contract_id.as_deref(), Some("0123456789abcdef"));
    assert_eq!(claude.versions.len(), 1);
    assert_eq!(claude.versions[0].version, "2.1.286");
    assert_eq!(claude.versions[0].state, "working");
    // The PATH version has no attributed row: it is evaluated for doctor.
    assert_eq!(claude.detected.as_ref().unwrap().version, "2.1.999");
    assert_eq!(claude.detected.as_ref().unwrap().state, "new");
    assert!(report.harnesses[1].versions.is_empty());
    assert!(report.harnesses[1].detected.is_none());
    let hermes = &report.harnesses[2];
    assert!(hermes.contract_id.is_none());
    assert!(hermes.detected.is_none());
    assert!(hermes.versions.is_empty());
    assert!(hermes.unattributed.is_none());
    assert_eq!(hermes.hook_parse_failures, 0);
}

#[test]
fn harness_states_round_trips_on_the_wire() {
    use crate::protocol::results::{
        DetectedVersion, HarnessStateReport, HarnessStatesReport, UnattributedReport,
        VersionStateReport,
    };
    assert_eq!(
        serde_json::to_value(Command::HarnessStates).unwrap(),
        serde_json::json!({"kind": "harness_states"})
    );
    assert!(Command::HarnessStates.validate().is_ok());
    let result = CommandResult::HarnessStates(HarnessStatesReport {
        harnesses: vec![HarnessStateReport {
            harness: "claude".into(),
            contract_id: Some("0123456789abcdef".into()),
            detected: Some(DetectedVersion {
                version: "2.1.999".into(),
                state: "new".into(),
                line: "new version, not yet seen working; verified on first use".into(),
            }),
            versions: vec![VersionStateReport {
                version: "2.1.286".into(),
                state: "working".into(),
                source: "recipe tables".into(),
                line: "claude 2.1.286: working".into(),
                notes: vec!["n".into()],
                issue_url: None,
                last_seen_at: 7,
                in_health_window: true,
            }],
            unattributed: Some(UnattributedReport {
                reason: "resume".into(),
                at: 9,
            }),
            hook_parse_failures: 3,
        }],
    });
    let encoded = serde_json::to_value(&result).unwrap();
    assert_eq!(encoded["kind"], "harness_states");
    assert_eq!(encoded["data"]["harnesses"][0]["hook_parse_failures"], 3);
    assert_eq!(
        serde_json::from_value::<CommandResult>(encoded).unwrap(),
        result
    );
}

#[test]
fn control_without_a_recorder_accepts_and_drops_evidence() {
    let (instance, boot) = (Uuid::new_v4(), Uuid::new_v4());
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        NoDomain,
    );
    let result = handler
        .handle(
            Command::HarnessEvidence(evidence_note(Some("2.1.286"))),
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        result,
        CommandResult::HarnessEvidenceRecorded { verified: false }
    );
}

#[test]
fn harness_evidence_round_trips_on_the_wire() {
    use crate::protocol::commands::HarnessEvidenceOutcome;
    let mut note = evidence_note(Some("2.1.286"));
    note.outcome = HarnessEvidenceOutcome::Violation {
        field: "tool_input.command".into(),
    };
    let command = Command::HarnessEvidence(note);
    assert!(command.validate().is_ok());
    let json = serde_json::to_value(&command).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "harness_evidence", "args": {
            "harness": "claude", "version": "2.1.286", "unattributed_reason": null,
            "contract_id": "0123456789abcdef", "event": "SessionStart",
            "outcome": {"kind": "violation", "field": "tool_input.command"},
            "session_id": "s1"}})
    );
    assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    let result = CommandResult::HarnessEvidenceRecorded { verified: true };
    let encoded = serde_json::to_value(&result).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({"kind": "harness_evidence_recorded", "data": {"verified": true}})
    );
    assert_eq!(
        serde_json::from_value::<CommandResult>(encoded).unwrap(),
        result
    );
    // deny_unknown_fields: an extra key is refused.
    let extra = serde_json::json!({"kind": "harness_evidence", "args": {
        "harness": "claude", "version": "2.1.286", "unattributed_reason": null,
        "contract_id": "0123456789abcdef", "event": "SessionStart",
        "outcome": {"kind": "ok"}, "session_id": null, "payload": "x"}});
    assert!(serde_json::from_value::<Command>(extra).is_err());
}

#[test]
fn harness_evidence_validation_bounds_every_field() {
    use crate::protocol::commands::HarnessEvidenceOutcome;
    let valid = || evidence_note(Some("2.1.286"));
    assert!(valid().validate().is_ok());
    assert!(evidence_note(None).validate().is_ok());
    let rejected: Vec<(&str, crate::protocol::commands::HarnessEvidence)> = vec![
        ("harness", {
            let mut n = valid();
            n.harness = "human".into();
            n
        }),
        ("version not x.y.z", {
            let mut n = valid();
            n.version = Some("2.1".into());
            n
        }),
        ("version pre-release", {
            let mut n = valid();
            n.version = Some("2.1.286-beta".into());
            n
        }),
        ("both version and reason", {
            let mut n = valid();
            n.unattributed_reason = Some("x".into());
            n
        }),
        ("neither version nor reason", {
            let mut n = valid();
            n.version = None;
            n
        }),
        ("empty reason", {
            let mut n = evidence_note(None);
            n.unattributed_reason = Some(String::new());
            n
        }),
        ("long reason", {
            let mut n = evidence_note(None);
            n.unattributed_reason = Some("r".repeat(129));
            n
        }),
        ("short contract id", {
            let mut n = valid();
            n.contract_id = "0123".into();
            n
        }),
        ("uppercase contract id", {
            let mut n = valid();
            n.contract_id = "0123456789ABCDEF".into();
            n
        }),
        ("empty event", {
            let mut n = valid();
            n.event = String::new();
            n
        }),
        ("long event", {
            let mut n = valid();
            n.event = "a".repeat(64);
            n
        }),
        ("event with punctuation", {
            let mut n = valid();
            n.event = "Pre-Tool".into();
            n
        }),
        ("long field", {
            let mut n = valid();
            n.outcome = HarnessEvidenceOutcome::Violation {
                field: "f".repeat(129),
            };
            n
        }),
        ("empty field", {
            let mut n = valid();
            n.outcome = HarnessEvidenceOutcome::Violation {
                field: String::new(),
            };
            n
        }),
        ("long session id", {
            let mut n = valid();
            n.session_id = Some("s".repeat(257));
            n
        }),
    ];
    for (what, note) in rejected {
        assert!(
            Command::HarnessEvidence(note).validate().is_err(),
            "{what} must be rejected"
        );
    }
    let mut edge = valid();
    edge.event = "a".repeat(63);
    edge.session_id = Some("s".repeat(256));
    edge.outcome = HarnessEvidenceOutcome::Violation {
        field: "f".repeat(128),
    };
    assert!(edge.validate().is_ok(), "the bounds are inclusive");
}

fn probe_invitation_reject() {
    use crate::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::Reject,
        ids::*,
    };
    let handler = daemon_handler(Uuid::new_v4(), Uuid::new_v4());
    let command = Command::Reject(Reject {
        thread: ThreadId::new("t1"),
        invitation: InvitationId::new("v1"),
        reason: "Outside assigned role".into(),
        operation: OperationId::new("reject-probe"),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s1"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("w1:p1"),
        },
    });
    assert!(command.validate().is_ok());
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    assert_eq!(
        handler
            .handle(command, PeerIdentity::from_kernel(501), &budget())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

fn probe_participant_locations() {
    use crate::protocol::{commands::ParticipantLocationsQuery, ids::*};
    let handler = daemon_handler(Uuid::new_v4(), Uuid::new_v4());
    let command = Command::ParticipantLocations(ParticipantLocationsQuery {
        thread: ThreadId::new("t1"),
        seats: vec![SeatId::new("s1")],
    });
    assert!(command.validate().is_ok());
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    assert_eq!(
        handler
            .handle(command, PeerIdentity::from_kernel(501), &budget())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn v2_capability_is_negotiated_only_with_recorder() {
    use crate::{
        daemon::harness_evidence::HarnessEvidenceRecorderV2,
        harness::adapter::HarnessAdapter,
        ports::StorePort,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let (instance, boot) = (Uuid::new_v4(), Uuid::new_v4());
    let make = || {
        ControlService::new(
            StopController::new(instance, boot, Cancellation::default()),
            move |_: &CallBudget| HealthInputs::unknown(instance, boot),
            NoDomain,
        )
    };
    let bare = make();
    let CommandResult::Capabilities(caps) = bare
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap()
    else {
        panic!("capabilities reply")
    };
    assert!(
        !caps
            .capabilities
            .iter()
            .any(|v| v == "hook.harness_evidence_v2")
    );
    let iso = TestIsolation::new("v2-cap-handler");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let handler = make().with_harness_evidence_v2(Arc::new(HarnessEvidenceRecorderV2::new(
        store.clone(),
        None,
        clock,
    )));
    let CommandResult::Capabilities(caps) = handler
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap()
    else {
        panic!("capabilities reply")
    };
    assert!(
        caps.capabilities
            .iter()
            .any(|v| v == "hook.harness_evidence_v2")
    );
    assert_eq!(
        caps.capabilities,
        ADVERTISED
            .iter()
            .filter(|name| **name != HARNESS_HEALTH_V2)
            .copied()
            .collect::<Vec<_>>()
    );
    let d = crate::harness::claude::ClaudeAdapter.contracts()[0];
    let note = crate::protocol::commands::HarnessEvidenceV2 {
        harness: "claude".into(),
        domain: "native_payload".into(),
        origin: d.origin,
        runtime: Some(
            crate::harness::runtime::RuntimeIdentity::stable_release(
                "2.1.286",
                "native_transcript",
            )
            .unwrap(),
        ),
        unavailable_reason: None,
        contract_id: d.contract_id_v2().unwrap(),
        event: "SessionStart".into(),
        outcome: crate::protocol::commands::HarnessEvidenceOutcomeV2::Ok,
        session_id: None,
        qualifications: vec![],
    };
    assert_eq!(
        bare.handle(
            Command::HarnessEvidenceV2(note.clone()),
            PeerIdentity::from_kernel(501),
            &budget()
        )
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        handler
            .handle(
                Command::HarnessEvidenceV2(note),
                PeerIdentity::from_kernel(501),
                &budget()
            )
            .unwrap(),
        CommandResult::HarnessEvidenceV2Recorded(
            crate::protocol::results::HarnessEvidenceV2Recorded { verified: false }
        )
    );
    assert_eq!(
        store
            .harness_evidence_v2_all("claude", 0, &budget())
            .unwrap()
            .len(),
        1
    );
}

// Catches advertising v2 without cached observation/store handling or sending it to domain mutation.
#[test]
fn health_v2_capability_is_negotiated_only_with_cached_provider() {
    use crate::{
        daemon::harness_states::{HarnessStatesProvider, embedded_source},
        ports::StorePort,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let instance = Uuid::new_v4();
    let boot = Uuid::new_v4();
    let bare = daemon_handler(instance, boot);
    let CommandResult::Capabilities(caps) = bare
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong capability response")
    };
    assert!(!caps.capabilities.iter().any(|c| c == "harness.health_v2"));
    assert_eq!(
        bare.handle(
            Command::HarnessHealthV2,
            PeerIdentity::from_kernel(501),
            &budget()
        )
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
    let iso = TestIsolation::new("health-v2-cap");
    let clock = Arc::new(FixedClock);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let provider = Arc::new(
        HarnessStatesProvider::new(
            store as Arc<dyn StorePort>,
            embedded_source(),
            clock,
            Box::new(|_| None),
            None,
        )
        .with_observations(Box::new(|| Ok(Default::default()))),
    );
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        NoDomain,
    )
    .with_harness_health_v2(provider);
    let CommandResult::Capabilities(caps) = handler
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong capability response")
    };
    assert!(
        caps.capabilities.iter().any(|c| c == "harness.health_v2"),
        "implemented handler must advertise capability"
    );
    let result = handler
        .handle(
            Command::HarnessHealthV2,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .expect("v2 must use cached/store handler, never NoDomain");
    let CommandResult::HarnessHealthV2(report) = &result else {
        panic!("wrong v2 response")
    };
    use crate::protocol::results::{
        AdmissionState, CallbackObservationState, EnablementState, HarnessHealthScope, HealthAxis,
        InstallationState,
    };
    assert_eq!(
        report
            .harnesses
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["claude", "codex", "hermes"]
    );
    let hermes = &report.harnesses["hermes"];
    assert_eq!(hermes.scope, HarnessHealthScope::daemon_default());
    assert_eq!(
        hermes.installation,
        HealthAxis {
            state: InstallationState::Unknown,
            detail: None,
        }
    );
    assert_eq!(
        hermes.enablement,
        HealthAxis {
            state: EnablementState::Unknown,
            detail: None,
        }
    );
    assert_eq!(
        hermes.admission,
        HealthAxis {
            state: AdmissionState::Unknown,
            detail: None,
        }
    );
    assert_eq!(
        hermes.callback_observation,
        HealthAxis {
            state: CallbackObservationState::Unknown,
            detail: None,
        }
    );
    assert_eq!(hermes.receipt_basis, "unknown");
    assert!(hermes.runtime_evidence.is_empty());
    assert!(hermes.unattributed.is_empty());
    assert_eq!(hermes.hook_parse_failures, 0);
    assert!(report.validate().is_ok());
    assert_eq!(
        serde_json::from_value::<CommandResult>(serde_json::to_value(&result).unwrap()).unwrap(),
        result
    );
}

fn probe_picker_directory() {
    use crate::{
        ports::{ReadContext, StorePort},
        protocol::{commands::PickerDirectoryQuery, output::OutputSpec, pagination::PageRequest},
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let iso = TestIsolation::new("cap-picker-directory");
    let context = StoreContext::new(iso.state_root().join("store.db"), Arc::new(FixedClock));
    context
        .open_writer()
        .unwrap()
        .execute(
            "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
            [],
        )
        .unwrap();
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = Command::PickerDirectory(PickerDirectoryQuery {
        page: PageRequest::default(),
    });
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    let result = store
        .query(
            &command,
            &ReadContext {
                instance: "i".into(),
                output: OutputSpec::default(),
                operation_scope: None,
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::PickerDirectory(page) = result else {
        panic!("wrong picker route")
    };
    assert!(page.items.is_empty());
    page.validate().unwrap();
}

#[test]
fn picker_directory_wire_keeps_old_directory_shape_and_cursor_only_contract() {
    use crate::protocol::{
        commands::PickerDirectoryQuery, pagination::PageRequest, results::PickerPage,
    };
    let command = Command::PickerDirectory(PickerDirectoryQuery {
        page: PageRequest::default(),
    });
    assert!(command.validate().is_ok());
    let invalid = PageRequest {
        limit: 101,
        ..PageRequest::default()
    };
    assert!(
        Command::PickerDirectory(PickerDirectoryQuery { page: invalid })
            .validate()
            .is_err()
    );
    let old_command:Command=serde_json::from_value(serde_json::json!({"kind":"directory","args":{"membership":null,"membership_filter":"all","topic_contains":null,"page":{"cursor":null,"limit":19,"max_bytes":65536}}})).unwrap();
    assert!(old_command.validate().is_ok());
    let old_result:CommandResult=serde_json::from_value(serde_json::json!({"kind":"directory","data":{"items":[],"next_cursor":null,"next_argv":null,"high_water_ordinal":0,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"}})).unwrap();
    let old_encoded = serde_json::to_value(old_result).unwrap();
    assert_eq!(old_encoded["kind"], "directory");
    assert!(old_encoded["data"].get("next_argv").is_some());
    let mut page = PickerPage {
        items: vec![],
        next_cursor: Some("opaque".into()),
        high_water_ordinal: 1,
        scope_revision: None,
        has_more: true,
        stop_reason: StopReason::Rows,
        consistency: Consistency::BoundedLive,
    };
    assert!(page.validate().is_ok());
    let picker = CommandResult::PickerDirectory(page.clone());
    let encoded = serde_json::to_value(&picker).unwrap();
    assert!(encoded["data"].get("next_argv").is_none());
    assert_eq!(
        serde_json::from_value::<CommandResult>(encoded).unwrap(),
        picker
    );
    page.next_cursor = None;
    assert!(page.validate().is_err());
    page.has_more = false;
    assert!(page.validate().is_err());
    page.stop_reason = StopReason::Complete;
    assert!(page.validate().is_ok());
}
