use super::*;

#[test]
fn hook_context_receipt_capability_is_additive() {
    assert!(ADVERTISED.contains(&"hook.context_receipts_v1"));
    assert!(ADVERTISED.contains(&LAZY_HOOK_DELIVERY));
    assert!(!Capabilities::none().supports("hook.context_receipts_v1"));
}
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

fn daemon_handler(instance: crate::node::NodeUuid, boot: Uuid) -> impl LocalService {
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
            crate::node::NodeUuid::new_v4(),
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
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
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
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let command = Command::RecordManagedLaunch(RecordManagedLaunch {
        seat: SeatId::new("s"),
        target: HostTargetId::new("w1:p1"),
        harness: Harness::Codex,
        terminal: TerminalId::new("term_1"),
        incarnation: "herdr-server:pid=1".into(),
        host_boot: HostBootId::new("boot"),
        target_generation: 1,
        fingerprint: crate::protocol::authority::LivenessFingerprint {
            pid: 4242,
            start: 1,
        },
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
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let command = Command::InboxBatch(InboxQuery {
        pushed: false,
        thread: None,
        claim: None,
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
    assert_eq!(SEAT_BINDING_DETAILS_V1, "seat.binding_details_v1");
    assert_eq!(INBOX_BATCH, "inbox.batch_v1");
    assert_eq!(LAZY_HOOK_DELIVERY, "hook.lazy_delivery_v1");
    assert_eq!(HOOK_CONTEXT_RECEIPTS_V1, "hook.context_receipts_v1");
    assert_eq!(DELIVERY_PRESENTATION_V2, "delivery.presentation_v2");
    assert_eq!(MOD_WATCH_PRESENTATION_V2, "mod.watch_presentation_v2");
    assert_eq!(MOD_ACK_ROW_V1, "mod.ack_row_v1");
    assert_eq!(BINDING_CURSORS, "inbox.binding_cursors_v1");
    assert_eq!(BINDING_DIGEST, "attention.binding_digest_v1");
    assert_eq!(IN_TURN_OFFER, "inbox.in_turn_offer_v1");
    assert_eq!(INBOX_THREAD_SCOPE, "inbox.thread_scope_v1");
    assert_eq!(INBOX_PUSHED_HOLD, "inbox.pushed_hold_v1");
    assert_eq!(CLI_PENDING_HINT, "attention.cli_pending_hint_v1");
    assert_eq!(TURN_END_DUE, "turn_end.due_v1");
    assert_eq!(WAKE_STATUS_V1, "seat.wake_status_v1");
    assert_eq!(PEER_PRUNE_V1, "peer.prune_v1");
    assert_eq!(PEER_EVENTS_V1, "peer.events_v1");
    assert_eq!(
        ADVERTISED,
        &[
            "history.full_bodies",
            "hook.parse_failure_report",
            "service.send_v1",
            "messages.send_time_receipts_v1",
            "hook.harness_evidence",
            "hook.harness_evidence_v2",
            "harness.states",
            "harness.health_v2",
            "seat.managed_launch",
            "seat.binding_details_v1",
            "seat.continuity_same_session_v1",
            "inbox.batch_v1",
            "invitation.reject_v1",
            "thread.join_v1",
            "thread.direct_v1",
            "direct_threads.ensure_creation_policy_v1",
            "participants.locations_v1",
            "seats.directory_v1",
            "picker.directory_v1",
            "attention.notice_delivery_v1",
            "messages.delivery_modes_v1",
            "send.lazy_v1",
            "inbox.batch_v2",
            "mod.watch_v1",
            "mod.watch_presentation_v2",
            "mod.ack_row_v1",
            "seat.wake_on_v1",
            "delivery.pull_v1",
            "hook.lazy_delivery_v1",
            "hook.context_receipts_v1",
            "delivery.presentation_v2",
            "inbox.binding_cursors_v1",
            "attention.binding_digest_v1",
            "inbox.in_turn_offer_v1",
            "inbox.thread_scope_v1",
            "send.priority_v1",
            "seat.nudge_threshold_v1",
            "direct.auto_join_v1",
            "inbox.pushed_hold_v1",
            "attention.cli_pending_hint_v1",
            "turn_end.due_v1",
            "seat.wake_status_v1",
            "peer.prune_v1",
            "peer.events_v1"
        ]
    );
}

/// The sync operator handlers (`service::sync_admin`) are driven by
/// `tests/service/sync_admin.rs` and the multinode suite; here each command
/// must validate and round-trip the wire.
fn probe_sync_commands(commands: &[crate::protocol::sync_admin::SyncCommand]) {
    for sync in commands {
        assert!(sync.validate().is_ok(), "{sync:?}");
        let command = Command::Sync(sync.clone());
        assert_eq!(
            serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
            command
        );
    }
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
            SEAT_BINDING_DETAILS_V1 => {
                let command =
                    Command::SeatInspectBindings(crate::protocol::commands::SeatInspectQuery {
                        seat: crate::protocol::ids::SeatId::new("missing-seat"),
                        page: Default::default(),
                    });
                assert!(command.validate().is_ok());
                assert_eq!(
                    serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap())
                        .unwrap(),
                    command
                );
                assert_eq!(
                    daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4())
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            SAME_SESSION_CONTINUITY_V1 => {
                let command =
                    Command::ContinuityCheckIn(crate::protocol::commands::ContinuityCheckIn {
                        target: crate::protocol::ids::HostTargetId::new("pane"),
                        host: crate::test_support::test_host_locator(),
                        harness: crate::protocol::authority::Harness::Codex,
                        native_session: crate::protocol::ids::NativeSessionId::new("session"),
                        source: "same_session".into(),
                        operation: crate::protocol::ids::OperationId::new("capability-probe"),
                        execution: crate::protocol::ids::ExecutionId::new("execution"),
                        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
                    });
                assert!(command.validate().is_ok());
                assert_eq!(
                    daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4())
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            INBOX_BATCH => probe_inbox_batch(),
            INBOX_BATCH_V2 => {
                let command = Command::InboxBatchV2(crate::protocol::commands::InboxQuery {
                    pushed: false,
                    thread: None,
                    claim: None,
                    seat: Some(crate::protocol::ids::SeatId::new("seat-probe")),
                    page: Default::default(),
                });
                assert!(command.validate().is_ok());
                assert_eq!(
                    serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap())
                        .unwrap(),
                    command
                );
                let isolation =
                    crate::test_support::isolation::TestIsolation::new("lazy-capability");
                let context = crate::store::connection::StoreContext::new(
                    isolation.path("store.db"),
                    Arc::new(FixedClock),
                )
                .with_test_host();
                context.open_writer().unwrap();
                let outcome = crate::store::queries::query(
                    &context,
                    "34750f98bd59fcfc946da45aaabe933b",
                    &command,
                    &budget(),
                );
                assert_eq!(outcome.unwrap_err().code, ErrorCode::NotFound);
            }
            LAZY_SEND | SEND_TIME_RECEIPTS_V1 => {
                let command: Command=serde_json::from_value(serde_json::json!({"kind":"send_message","args":{"delivery_mode":if *name == LAZY_SEND { "lazy" } else { "ordinary" },"thread":"t","body":"quiet","invited_recipients":[],"deadline_millis":null,"operation":"op","claim":{"instance":"34750f98bd59fcfc946da45aaabe933b","host":"/host.sock","seat":"a","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}})).unwrap();
                assert!(command.validate().is_ok());
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            SEND_PRIORITY_V1 => {
                // The explicit priority is on the wire, validates and routes to the domain; the
                // store freezes it (`store::messages::priority_send_tests`).
                let command: Command=serde_json::from_value(serde_json::json!({"kind":"send_message","args":{"priority":3,"thread":"t","body":"quiet","invited_recipients":[],"deadline_millis":null,"operation":"op","claim":{"host":"/host.sock","instance":"34750f98bd59fcfc946da45aaabe933b","seat":"a","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}})).unwrap();
                assert!(command.validate().is_ok());
                let wire = serde_json::to_value(&command).unwrap();
                assert_eq!(wire["args"]["priority"], 3);
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            INVITATION_REJECT => probe_invitation_reject(),
            THREAD_JOIN => probe_thread_join(),
            // `DIRECT_AUTO_JOIN_V1` shares the Ensure wire probe; the handler's auto-join itself is
            // driven through the store in `tests/store/pair_auto_join.rs`.
            DIRECT_THREADS_V1 | ENSURE_CREATION_POLICY_V1 | DIRECT_AUTO_JOIN_V1 => {
                let command =
                    Command::EnsureDirectThread(crate::protocol::commands::EnsureDirectThread {
                        creation_policy: (*name == ENSURE_CREATION_POLICY_V1).then_some(
                            crate::protocol::commands::EnsureCreationPolicy {
                                confirmation:
                                    crate::protocol::commands::InvitationConfirmation::Lazy,
                                deadline_millis: None,
                            },
                        ),
                        recipient: crate::protocol::ids::SeatId::new("peer"),
                        operation: crate::protocol::ids::OperationId::new("direct-probe"),
                        claim: crate::protocol::authority::CallerClaim {
                            host: crate::test_support::test_host_locator(),
                            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                            seat: crate::protocol::ids::SeatId::new("sender"),
                            binding_generation: 1,
                            role: crate::protocol::authority::CallerRole::TopLevel,
                            harness: crate::protocol::authority::Harness::Codex,
                            native_session: crate::protocol::ids::NativeSessionId::new("n"),
                            execution: crate::protocol::ids::ExecutionId::new(
                                "00000000-0000-4000-8000-000000000001",
                            ),
                            target: crate::protocol::ids::HostTargetId::new("p"),
                            agent_id: None,
                            liveness: None,
                        },
                    });
                assert!(command.validate().is_ok());
                assert_eq!(
                    daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4())
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
                assert!(!ADVERTISED.contains(&AUTOMATIC_HANDOFF_V1));
            }
            PARTICIPANT_LOCATIONS => probe_participant_locations(),
            SEATS_DIRECTORY_V1 => probe_seats_directory(),
            WAKE_STATUS_V1 => probe_wake_status(),
            PEER_PRUNE_V1 => probe_sync_commands(&[
                crate::protocol::sync_admin::SyncCommand::PeerPrune {
                    host: None,
                    older_than_ms: 1,
                    apply: false,
                    list: false,
                },
                crate::protocol::sync_admin::SyncCommand::PeerUnforget {
                    target: "Abc".into(),
                    apply: false,
                },
            ]),
            PEER_EVENTS_V1 => {
                probe_sync_commands(&[crate::protocol::sync_admin::SyncCommand::PeerEvents {
                    node: None,
                    before: None,
                    limit: 1,
                }])
            }
            PICKER_DIRECTORY_V1 => probe_picker_directory(),
            MESSAGE_DELIVERY_MODES => probe_message_delivery_modes(),
            ATTENTION_NOTICE_DELIVERY => {
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command = Command::AttentionDigestDelivery(
                    crate::protocol::commands::AttentionDigestQuery {
                        seat: crate::protocol::ids::SeatId::new("s"),
                        lazy: false,
                        claim: None,
                        cli_hint: false,
                    },
                );
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            MOD_WATCH => probe_mod_watch(false),
            MOD_WATCH_PRESENTATION_V2 => probe_mod_watch(true),
            MOD_ACK_ROW_V1 => {
                // The row decodes into `AckModDelivered` and is validated; the store records it
                // in the ACK observation (`tests/store/mod_ack.rs`).
                let ack = |via: &str, row: &str| -> Command {
                    serde_json::from_value(serde_json::json!({"kind":"ack_mod_delivered","args":{"via":via,"messages":["m1"],"operation":"op","row":row,"claim":{"instance":"34750f98bd59fcfc946da45aaabe933b","host":"/host.sock","seat":"nobody","binding_generation":1,"role":"top_level","harness":"claude","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}})).unwrap()
                };
                assert!(ack("append", "3f2a-row_1").validate().is_ok());
                assert!(ack("context", "3f2a-row_1").validate().is_err());
                assert!(ack("append", "no spaces").validate().is_err());
            }
            DELIVERY_PRESENTATION_V2 => presentation_capability_routes_use_configured_domain(),
            DELIVERY_PULL => {
                // The command decodes and routes to the domain service (served by
                // `service::delivery_pull`, tested there); this daemon has none.
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command: Command = serde_json::from_value(serde_json::json!({"kind":"delivery_pull","args":{"claim":{"instance":"34750f98bd59fcfc946da45aaabe933b","host":"/host.sock","seat":"nobody","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}})).unwrap();
                assert!(command.validate().is_ok());
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            TURN_END_DUE => {
                // The command decodes and routes to the domain service (its store decision is
                // `WakeCandidate::owes_turn_end`, the turn-end continuation the hook e2e pins);
                // this daemon has none.
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command: Command = serde_json::from_value(serde_json::json!({"kind":"turn_end_due","args":{"claim":{"instance":"34750f98bd59fcfc946da45aaabe933b","host":"/host.sock","seat":"nobody","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}})).unwrap();
                assert!(command.validate().is_ok());
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            SEAT_WAKE_ON_V1 => {
                // The daemon serves the read: an unknown seat is NotFound (a
                // daemon without the handler would refuse the command itself).
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command = Command::WakeOn(crate::protocol::commands::WakeOnQuery {
                    seat: crate::protocol::ids::SeatId::new("nobody"),
                });
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            SEAT_NUDGE_THRESHOLD_V1 => {
                // The daemon serves the read and routes the set: both are on the wire and
                // round-trip; an unknown seat is NotFound (a daemon without the handlers would
                // refuse the command itself). The store tests cover set-then-read.
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command =
                    Command::NudgeThreshold(crate::protocol::commands::NudgeThresholdQuery {
                        seat: crate::protocol::ids::SeatId::new("nobody"),
                    });
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
                let set: Command = serde_json::from_value(serde_json::json!({"kind":"set_nudge_threshold","args":{"value":3,"operation":"op","claim":{"host":"/host.sock","instance":"34750f98bd59fcfc946da45aaabe933b","seat":"nobody","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}})).unwrap();
                assert!(set.validate().is_ok());
                assert_eq!(
                    handler
                        .handle(set, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            LAZY_HOOK_DELIVERY => {
                // The lazy digest request is routed to the store's
                // seat-scoped read: an unknown seat is NotFound.
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command = Command::AttentionDigestDelivery(
                    crate::protocol::commands::AttentionDigestQuery {
                        seat: crate::protocol::ids::SeatId::new("s"),
                        lazy: true,
                        claim: None,
                        cli_hint: false,
                    },
                );
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            HOOK_CONTEXT_RECEIPTS_V1 => probe_hook_context_receipts(),
            BINDING_CURSORS => probe_binding_cursors(),
            INBOX_THREAD_SCOPE => probe_inbox_thread_scope(),
            INBOX_PUSHED_HOLD => probe_inbox_pushed_hold(),
            IN_TURN_OFFER => {
                // The in-turn flag and its invitations are on the wire only when set and
                // round-trip; the store records them
                // (`lazy_attention::in_turn_display_of_a_nudge_owes_no_idle_wake`).
                let command = Command::AdvanceCursors(crate::protocol::commands::AdvanceCursors {
                    in_turn: true,
                    claim: crate::protocol::authority::CallerClaim {
                        host: crate::test_support::test_host_locator(),
                        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                        seat: crate::protocol::ids::SeatId::new("s"),
                        binding_generation: 1,
                        role: crate::protocol::authority::CallerRole::TopLevel,
                        harness: crate::protocol::authority::Harness::Codex,
                        native_session: crate::protocol::ids::NativeSessionId::new("n"),
                        execution: crate::protocol::ids::ExecutionId::new("e"),
                        target: crate::protocol::ids::HostTargetId::new("p"),
                        agent_id: None,
                        liveness: None,
                    },
                    operation: crate::protocol::ids::OperationId::new("op"),
                    displayed: vec![crate::protocol::ids::MessageId::new("m")],
                    invitations: vec![crate::protocol::ids::InvitationId::new(
                        "34750f98bd59fcfc946da45aaabe933b",
                    )],
                });
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(json.to_string().matches("\"in_turn\":true").count(), 1);
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
            }
            BINDING_DIGEST => {
                // A claimed digest request round-trips and is routed to the store's seat-scoped
                // read: an unknown seat is NotFound, whoever the claim names.
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command =
                    Command::AttentionDigest(crate::protocol::commands::AttentionDigestQuery {
                        seat: crate::protocol::ids::SeatId::new("s"),
                        lazy: false,
                        claim: Some(crate::protocol::authority::CallerClaim {
                            host: crate::test_support::test_host_locator(),
                            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                            seat: crate::protocol::ids::SeatId::new("s"),
                            binding_generation: 1,
                            role: crate::protocol::authority::CallerRole::Subagent,
                            harness: crate::protocol::authority::Harness::Claude,
                            native_session: crate::protocol::ids::NativeSessionId::new("n"),
                            execution: crate::protocol::ids::ExecutionId::new("e"),
                            target: crate::protocol::ids::HostTargetId::new("p"),
                            agent_id: Some(crate::protocol::ids::AgentId::new("ag")),
                            liveness: None,
                        }),
                        cli_hint: false,
                    });
                // A subagent may read its own digest: validation does not refuse the role.
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            CLI_PENDING_HINT => {
                // The opt-in flag is on the wire only when set, round-trips, and reaches the
                // store's digest read (an unknown seat is NotFound). The decision itself:
                // `pending_hint::hookless_caller_sees_pending_count_on_top_of_text_commands`.
                let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
                let command =
                    Command::AttentionDigest(crate::protocol::commands::AttentionDigestQuery {
                        seat: crate::protocol::ids::SeatId::new("s"),
                        lazy: false,
                        claim: None,
                        cli_hint: true,
                    });
                assert!(command.validate().is_ok());
                let json = serde_json::to_value(&command).unwrap();
                assert_eq!(json.to_string().matches("\"cli_hint\":true").count(), 1);
                assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
                assert_eq!(
                    handler
                        .handle(command, PeerIdentity::from_kernel(501), &budget())
                        .unwrap_err()
                        .code,
                    ErrorCode::NotFound
                );
            }
            other => panic!("{other} is advertised but has no handler probe here"),
        }
    }
}

/// Exercise the advertised native-context source through the actual configured daemon:
/// a requested receipt must settle canonically, retaining source provenance across replay.
fn probe_hook_context_receipts() {
    use crate::{
        ports::StorePort,
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            commands::{
                CheckIn, CheckInMode, CompleteInboxDelivery, CreateThread, DeliveryMode, Join,
                LazyCompletionVia, SendMessage,
            },
            ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId, SeatId},
        },
        service::{dispatch::DomainService, fair_writer::FairWriter},
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let isolated = TestIsolation::new("hook-context-capability-route");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let instance = crate::node::NodeUuid::new_v4();
    let context = StoreContext::new(isolated.path("store.db"), clock.clone())
        .with_test_host()
        .with_node_seed(crate::test_support::node_seed_for(instance));
    let db = context.open_writer().unwrap();
    let boot = Uuid::new_v4();
    db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
        .unwrap();
    for seat in ["origin", "recipient"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,?2,'resolved','native',?1,0,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", rusqlite::params![seat, instance.to_string()]).unwrap();
        db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts WHERE node_id=?1),?2,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc-'||?2,'coherent_enumeration',1)", rusqlite::params![instance.to_string(),seat]).unwrap();
    }
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(context, instance.to_string(), StoreSettings::default()).unwrap(),
    );
    let domain = DomainService::new(instance.to_string(), store, clock)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        domain,
    );
    let call = |command| {
        handler
            .handle(command, PeerIdentity::from_kernel(501), &budget())
            .unwrap()
    };
    let CommandResult::Capabilities(caps) = call(Command::Capabilities) else {
        panic!("capabilities")
    };
    assert!(
        caps.capabilities
            .iter()
            .any(|name| name == HOOK_CONTEXT_RECEIPTS_V1),
        "configured actual capability response must advertise the selected routes"
    );
    let check_in = |seat: &str| {
        let session = format!("session-{seat}");
        let claim = CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: instance.to_string(),
            seat: SeatId::new(seat),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(&session),
            execution: ExecutionId::new(Uuid::new_v4().to_string()),
            target: HostTargetId::new(seat),
            agent_id: None,
            liveness: Some(crate::test_support::isolation::stand_in_fingerprint(
                "codex", &session,
            )),
        };
        let CommandResult::CheckedIn(checked) = call(Command::CheckIn(CheckIn {
            lineage_parent: None,
            mode: CheckInMode::Lifecycle {
                expected_binding_generation: 0,
            },
            claim,
            operation: OperationId::new(format!("check-in-{seat}")),
        })) else {
            panic!("canonical binding")
        };
        checked.context
    };
    let origin = check_in("origin");
    let recipient = check_in("recipient");
    let CommandResult::ThreadCreated(thread) = call(Command::CreateThread(CreateThread {
        name: None,
        topic: "hook receipt capability".into(),
        goal: "canonical receipt settlement".into(),
        operation: OperationId::new("receipt-create"),
        claim: origin.clone(),
    })) else {
        panic!("created thread")
    };
    call(Command::Join(Join {
        thread: thread.clone(),
        operation: OperationId::new("receipt-join"),
        claim: recipient.clone(),
    }));
    let CommandResult::MessageSent(message) = call(Command::SendMessage(SendMessage {
        priority: None,
        delivery_mode: DeliveryMode::Ordinary,
        thread,
        body: "complete native context".into(),
        direct_recipient: None,
        invited_recipients: vec![recipient.seat.clone()],
        deadline_millis: Some(300_000),
        operation: OperationId::new("receipt-send"),
        claim: origin,
        relays_user: false,
        user_intent: None,
    })) else {
        panic!("published requested receipt")
    };
    let receipt = || {
        db.query_row(
        "SELECT state,ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id='recipient'",
        [message.as_str()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    ).unwrap()
    };
    assert_eq!(
        crate::store::effective::effective_receipt(&db, message.as_str(), "recipient")
            .unwrap()
            .expect("the published send requests a recipient receipt")
            .state,
        crate::store::effective::EffectiveReceiptState::Pending
    );
    let command = Command::CompleteInboxDelivery(CompleteInboxDelivery {
        messages: vec![],
        receipt_candidates: vec![message.clone()],
        operation: OperationId::new("receipt-complete"),
        claim: recipient,
        via: Some(LazyCompletionVia::HookContext),
    });
    assert!(command.validate().is_ok());
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    let completed = call(command.clone());
    assert_eq!(completed, CommandResult::InboxDeliveryCompleted(vec![]));
    let (state, observation) = receipt();
    assert_eq!(state, "acked");
    let observation: serde_json::Value = serde_json::from_str(&observation.unwrap()).unwrap();
    assert_eq!(observation["action_provenance"], "cooperative_hook_context");
    assert_eq!(observation["via"], "hook_context");
    assert_eq!(observation["session"], "session-recipient");
    let settled = receipt();
    assert_eq!(call(command), completed);
    assert_eq!(
        receipt(),
        settled,
        "exact replay preserves canonical attribution"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack_mod:%'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1,
        "one terminal receipt event across replay"
    );
}

/// The cursor command is served by the store, which decides it against the canonical view: a
/// claim for a seat it does not know is refused (a store without the handler would answer
/// `unsupported`).
fn probe_binding_cursors() {
    use crate::{
        ports::StorePort,
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            commands::AdvanceCursors,
            ids::{ExecutionId, HostTargetId, MessageId, NativeSessionId, OperationId, SeatId},
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    let command = Command::AdvanceCursors(AdvanceCursors {
        in_turn: false,
        invitations: Vec::new(),
        claim: CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new("nobody"),
            binding_generation: 1,
            role: CallerRole::Subagent,
            harness: Harness::Claude,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("p"),
            agent_id: Some(crate::protocol::ids::AgentId::new("ag")),
            liveness: None,
        },
        operation: OperationId::new("op"),
        displayed: vec![MessageId::new("m")],
    });
    // A subagent may send it: validation does not refuse the role.
    assert!(command.validate().is_ok());
    let json = serde_json::to_value(&command).unwrap();
    assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    let Command::AdvanceCursors(request) = command else {
        unreachable!()
    };
    let isolation = crate::test_support::isolation::TestIsolation::new("cursors-capability");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store = SqliteStore::new(
        StoreContext::new(isolation.path("store.db"), clock).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    assert_eq!(
        store.advance_cursors(&request, &budget()).unwrap_err().code,
        ErrorCode::CallerUnverified
    );
}

/// The daemon's domain decides a watch registration against its store: a
/// claim for a seat the store does not know is refused `unresolved` (a
/// service without the handler would answer `disabled`).
fn probe_mod_watch(presentation_v2: bool) {
    use crate::{
        ports::{ModChannelSink, StorePort},
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId},
            watch::{WatchFrame, WatchRefusalReason, WatchRequest},
        },
        service::dispatch::DomainService,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    struct NoSink;
    impl ModChannelSink for NoSink {
        fn push(&self, _frame: WatchFrame) -> bool {
            false
        }
    }
    let isolation = crate::test_support::isolation::TestIsolation::new("mod-watch-capability");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            StoreContext::new(isolation.path("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let domain = DomainService::new("34750f98bd59fcfc946da45aaabe933b".into(), store, clock);
    let request = WatchRequest {
        presentation_v2,
        claim: CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new("nobody"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Claude,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("p"),
            agent_id: None,
            liveness: None,
        },
    };
    assert_eq!(
        serde_json::from_value::<WatchRequest>(serde_json::to_value(&request).unwrap()).unwrap(),
        request,
        "the explicit registration mode must survive the wire"
    );
    let refusal = domain
        .watch_register(&request, Arc::new(NoSink), &budget())
        .unwrap_err();
    assert_eq!(refusal, WatchRefusalReason::Unresolved);
}

#[test]
fn capabilities_reply_round_trips_and_defaults_empty() {
    let instance = crate::node::NodeUuid::new_v4().to_string();
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
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
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
            "messages.send_time_receipts_v1",
            "hook.harness_evidence",
            "harness.states",
            "seat.managed_launch",
            "seat.binding_details_v1",
            "seat.continuity_same_session_v1",
            "inbox.batch_v1",
            "invitation.reject_v1",
            "thread.join_v1",
            "thread.direct_v1",
            "direct_threads.ensure_creation_policy_v1",
            "participants.locations_v1",
            "seats.directory_v1",
            "picker.directory_v1",
            "attention.notice_delivery_v1",
            "messages.delivery_modes_v1",
            "send.lazy_v1",
            "inbox.batch_v2",
            "mod.watch_v1",
            "mod.watch_presentation_v2",
            "mod.ack_row_v1",
            "seat.wake_on_v1",
            "delivery.pull_v1",
            "hook.lazy_delivery_v1",
            "hook.context_receipts_v1",
            "delivery.presentation_v2",
            "inbox.binding_cursors_v1",
            "attention.binding_digest_v1",
            "inbox.in_turn_offer_v1",
            "inbox.thread_scope_v1",
            "send.priority_v1",
            "seat.nudge_threshold_v1",
            "direct.auto_join_v1",
            "inbox.pushed_hold_v1",
            "attention.cli_pending_hint_v1",
            "turn_end.due_v1",
            "seat.wake_status_v1",
            "peer.prune_v1",
            "peer.events_v1",
            "invitation.confirmation_v1"
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
    assert!(!caps.supports(BOOTSTRAP_INSPECTED_RECOVERY_V1));
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
    let instance = crate::node::NodeUuid::new_v4();
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
    let (instance, boot) = (crate::node::NodeUuid::new_v4(), Uuid::new_v4());
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
    let dir = crate::test_support::scratch::temp_path(format!("htcap-send-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let context = StoreContext::new(dir.join("probe.db"), Arc::new(FixedClock)).with_test_host();
    {
        let conn = context.open_writer().unwrap();
        conn.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
    }
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let author = ServiceAuthorId::new("graph:probe");
    let gate = ProbeGate(author.clone());
    let connection = gate
        .register("34750f98bd59fcfc946da45aaabe933b", "boot", author)
        .unwrap();
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
    let (instance, boot) = (crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let iso = TestIsolation::new("cap-harness-evidence");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
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
    let (instance, boot) = (crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let iso = TestIsolation::new("cap-harness-states");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
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
    assert_eq!(&names[..4], ["claude", "codex", "hermes", "agy"]);
    #[cfg(feature = "test-support")]
    assert_eq!(&names[4..], ["synthetic_fourth"]);
    #[cfg(not(feature = "test-support"))]
    assert_eq!(names.len(), 4);
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
        mod_channels: None,
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
    let (instance, boot) = (crate::node::NodeUuid::new_v4(), Uuid::new_v4());
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
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let command = Command::Reject(Reject {
        thread: ThreadId::new("t1"),
        invitation: InvitationId::new("v1"),
        reason: "Outside assigned role".into(),
        operation: OperationId::new("reject-probe"),
        claim: CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: SeatId::new("s1"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("w1:p1"),
            agent_id: None,
            liveness: None,
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
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
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

fn probe_wake_status() {
    use crate::protocol::{commands::WakeStatusQuery, ids::*};
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let command = Command::WakeStatus(WakeStatusQuery {
        all: false,
        seats: vec![SeatId::new("s1")],
        message: Some(MessageId::new("m1")),
        observer: Some(SeatId::new("s2")),
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
    let (instance, boot) = (crate::node::NodeUuid::new_v4(), Uuid::new_v4());
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
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
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
            .chain(std::iter::once(INVITATION_CONFIRMATION_V1))
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
    let instance = crate::node::NodeUuid::new_v4();
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
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
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
    let names = report
        .harnesses
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(&names[..4], ["agy", "claude", "codex", "hermes"]);
    #[cfg(feature = "test-support")]
    assert_eq!(&names[4..], ["synthetic_fourth"]);
    #[cfg(not(feature = "test-support"))]
    assert_eq!(names.len(), 4);
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
    let context =
        StoreContext::new(iso.state_root().join("store.db"), Arc::new(FixedClock)).with_test_host();
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
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
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
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

/// `seats.directory_v1`: a filtered seat list round-trips and reaches the
/// store's seat read (an empty store lists nothing).
fn probe_seats_directory() {
    use crate::{
        ports::{ReadContext, StorePort},
        protocol::{
            commands::{SeatFilter, SeatsQuery},
            output::OutputSpec,
            pagination::PageRequest,
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let iso = TestIsolation::new("cap-seats-directory");
    let context =
        StoreContext::new(iso.state_root().join("store.db"), Arc::new(FixedClock)).with_test_host();
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let command = Command::Seats(SeatsQuery {
        page: PageRequest::default(),
        target: None,
        include_retired: false,
        filter: SeatFilter {
            node: Some("Q0V".into()),
            local: true,
            ..SeatFilter::default()
        },
    });
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    let result = store
        .query(
            &command,
            &ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: OutputSpec::default(),
                operation_scope: None,
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::Seats(page) = result else {
        panic!("wrong seats route")
    };
    assert!(page.items.is_empty());
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
    let mut filtered_command = old_command.clone();
    let Command::Directory(query) = &mut filtered_command else {
        panic!("wrong directory route")
    };
    query.topic_contains = Some("雪's %_.* topic".into());
    let encoded = serde_json::to_value(&filtered_command).unwrap();
    assert_eq!(encoded["args"]["topic_contains"], "雪's %_.* topic");
    assert!(encoded["args"].get("name_contains").is_none());
    assert_eq!(
        serde_json::from_value::<Command>(encoded).unwrap(),
        filtered_command
    );

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

#[test]
fn directory_search_generated_continuation_preserves_literal_scope_order_and_output() {
    use crate::{
        cli::commands::{CliAction, parse_argv},
        ports::{ReadContext, StorePort},
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let iso = TestIsolation::new("directory-search-continuation");
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
    context.open_writer().unwrap().execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES('t1','34750f98bd59fcfc946da45aaabe933b','雪''s %_.* topic','goal',0,0,0),('t2','34750f98bd59fcfc946da45aaabe933b','雪''s %_.* topic','goal',0,0,0);").unwrap();
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let original = parse_argv([
        "ht",
        "--state-dir",
        "/tmp/state with space",
        "--host-endpoint",
        "/tmp/host.sock",
        "--json",
        "thread",
        "list",
        "--all",
        "--recent",
        "--search",
        "雪's %_.* topic",
        "--limit",
        "1",
        "--max-bytes",
        "4096",
    ])
    .unwrap();
    let CliAction::Wire(command) = &original.action else {
        panic!("wrong route")
    };
    let CommandResult::Directory(page) = store
        .query(
            command,
            &ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: original.output.clone(),
                operation_scope: None,
            },
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong result")
    };
    assert!(page.has_more);
    let parsed = parse_argv(page.next_argv.unwrap()).unwrap();
    assert_eq!(parsed.output, original.output);
    let CliAction::Wire(Command::Directory(mut continued)) = parsed.action else {
        panic!("wrong continuation route")
    };
    assert_eq!(continued.page.cursor, page.next_cursor);
    assert!(continued.page.cursor.take().is_some());
    assert_eq!(Command::Directory(continued), *command);
}

fn probe_message_delivery_modes() {
    use crate::{
        ports::{ReadContext, StorePort},
        protocol::{
            commands::{DeliveryMode, MessageDeliveryModesQuery},
            ids::MessageId,
            output::OutputSpec,
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let iso = TestIsolation::new("cap-message-delivery-modes");
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
    context.open_writer().unwrap().execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('m','34750f98bd59fcfc946da45aaabe933b','t',1,'ordinary',1,'ordinary body',0,'lazy'); INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode,priority) VALUES('m3','34750f98bd59fcfc946da45aaabe933b','t',2,'ordinary',2,'urgent body',0,'ordinary',3);").unwrap();
    let store = SqliteStore::new(
        context,
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let command = Command::MessageDeliveryModes(MessageDeliveryModesQuery {
        with_priority: false,
        messages: vec![MessageId::new("m")],
    });
    assert_eq!(
        serde_json::from_value::<Command>(serde_json::to_value(&command).unwrap()).unwrap(),
        command
    );
    let CommandResult::MessageDeliveryModes(modes) = store
        .query(
            &command,
            &ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: OutputSpec::default(),
                operation_scope: None,
            },
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong metadata route")
    };
    assert_eq!(
        modes,
        vec![crate::protocol::results::MessageDeliveryMode {
            priority: None,
            message: MessageId::new("m"),
            delivery_mode: DeliveryMode::Lazy
        }]
    );
    // `with_priority` adds the stored priority and leaves the legacy mode column alone.
    let asked = Command::MessageDeliveryModes(MessageDeliveryModesQuery {
        with_priority: true,
        messages: vec![MessageId::new("m3")],
    });
    let CommandResult::MessageDeliveryModes(modes) = store
        .query(
            &asked,
            &ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: OutputSpec::default(),
                operation_scope: None,
            },
            &budget(),
        )
        .unwrap()
    else {
        panic!("wrong metadata route")
    };
    assert_eq!(
        modes,
        vec![crate::protocol::results::MessageDeliveryMode {
            priority: Some(crate::protocol::priority::DeliveryPriority::new(3).unwrap()),
            message: MessageId::new("m3"),
            delivery_mode: DeliveryMode::Ordinary
        }]
    );
}

fn probe_thread_join() {
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let command = Command::Join(crate::protocol::commands::Join {
        thread: crate::protocol::ids::ThreadId::new("t1"),
        operation: crate::protocol::ids::OperationId::new("join-probe"),
        claim: crate::protocol::authority::CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            seat: crate::protocol::ids::SeatId::new("s1"),
            binding_generation: 1,
            role: crate::protocol::authority::CallerRole::TopLevel,
            harness: crate::protocol::authority::Harness::Codex,
            native_session: crate::protocol::ids::NativeSessionId::new("n"),
            execution: crate::protocol::ids::ExecutionId::new("e"),
            target: crate::protocol::ids::HostTargetId::new("w1:p1"),
            agent_id: None,
            liveness: None,
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

#[test]
fn invitation_confirmation_wire_preserves_legacy_and_distinguishes_modern_requests() {
    let legacy = r#"{"kind":"invite","args":{"thread":"t","seat":"s","deadline_millis":null,"operation":"op","claim":{"instance":"i","host":"/host.sock","seat":"a","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}}"#;
    let original: serde_json::Value = serde_json::from_str(legacy).unwrap();
    let historical: Command = serde_json::from_str(legacy).unwrap();
    assert_eq!(serde_json::to_string(&historical).unwrap(), legacy);
    let digest = crate::store::schema::canonical_digest(&historical).unwrap();
    let digest_hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        digest_hex,
        "f0ad00529a4ca051d582980f98d952bf1f8a33ea1b4d85a4b5964a8509a3cae1"
    );
    for mode in ["lazy", "require_ack"] {
        let mut modern = original.clone();
        modern["args"]["confirmation"] = serde_json::json!(mode);
        let command: Command = serde_json::from_value(modern.clone()).unwrap();
        assert_eq!(serde_json::to_value(&command).unwrap(), modern);
        assert_ne!(
            crate::store::schema::canonical_digest(&command).unwrap(),
            crate::store::schema::canonical_digest(&historical).unwrap()
        );
        assert!(command.validate().is_ok());
        if mode == "require_ack" {
            modern["args"]["deadline_millis"] = serde_json::json!(17);
            let timed: Command = serde_json::from_value(modern.clone()).unwrap();
            assert_eq!(serde_json::to_value(&timed).unwrap(), modern);
            assert!(timed.validate().is_ok());
            modern["args"]["claim"]["harness"] = serde_json::json!("human");
            let human: Command = serde_json::from_value(modern).unwrap();
            // A person's RequireAck invitation is valid since ht-yha.6: every invitation is confirming.
            assert!(human.validate().is_ok());
        } else {
            modern["args"]["deadline_millis"] = serde_json::json!(10);
            let conflict: Command = serde_json::from_value(modern).unwrap();
            assert!(conflict.validate().is_err());
        }
    }
}

#[test]
fn public_operation_status_cannot_bypass_send_time_capability_guard() {
    use crate::{
        ports::StorePort,
        protocol::{commands::OperationStatusQuery, ids::OperationId},
        service::dispatch::DomainService,
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    let isolation = crate::test_support::isolation::TestIsolation::new("ordinary-public-status");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            StoreContext::new(isolation.path("store.db"), clock.clone()).with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let domain = DomainService::new("34750f98bd59fcfc946da45aaabe933b".into(), store, clock);
    let error = domain
        .handle(
            Command::OperationStatus(OperationStatusQuery {
                operation: OperationId::new("historical-key"),
            }),
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Unauthorized);
    assert_eq!(error.detail, "verified operation scope required");
}

#[test]
fn invitation_confirmation_capability_is_advertised_with_active_lifecycle() {
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let CommandResult::Capabilities(reply) = handler
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap()
    else {
        panic!()
    };
    assert!(
        reply
            .capabilities
            .iter()
            .any(|value| value == INVITATION_CONFIRMATION_V1)
    );
}

#[test]
fn automatic_contract_declarations_do_not_advertise_an_unimplemented_runtime() {
    let handler = daemon_handler(crate::node::NodeUuid::new_v4(), Uuid::new_v4());
    let CommandResult::Capabilities(caps) = handler
        .handle(
            Command::Capabilities,
            PeerIdentity::from_kernel(501),
            &budget(),
        )
        .unwrap()
    else {
        panic!("capability reply")
    };
    assert!(
        !caps
            .capabilities
            .iter()
            .any(|cap| cap == AUTOMATIC_HANDOFF_V1)
    );
}

/// Kills: advertising V2 while either deciding route is absent from the
/// configured DomainService, or canonical Confirm loses the captured controls.
/// Unlike the older NoDomain probes this must allocate and retire real output.
#[test]
fn presentation_capability_routes_use_configured_domain() {
    use crate::{
        harness::registry::builtins,
        ports::StorePort,
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            commands::{CheckIn, CheckInMode, CreateThread, InvitationConfirmation, Invite},
            delivery::{
                DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
                PresentationRenderRequest,
            },
            ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId, SeatId},
        },
        service::{dispatch::DomainService, fair_writer::FairWriter},
        store::{SqliteStore, StoreSettings, connection::StoreContext},
        test_support::isolation::TestIsolation,
    };
    let isolated = TestIsolation::new("presentation-capability-route");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let instance = crate::node::NodeUuid::new_v4();
    let context = StoreContext::new(isolated.path("store.db"), clock.clone())
        .with_test_host()
        .with_node_seed(crate::test_support::node_seed_for(instance));
    let db = context.open_writer().unwrap();
    let boot = Uuid::new_v4();
    db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
        .unwrap();
    for seat in ["origin", "recipient"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,?2,'resolved','native',?1,0,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", rusqlite::params![seat, instance.to_string()]).unwrap();
        db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts WHERE node_id=?1),?2,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc-'||?2,'coherent_enumeration',1)", rusqlite::params![instance.to_string(),seat]).unwrap();
    }
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(context, instance.to_string(), StoreSettings::default()).unwrap(),
    );
    let domain = DomainService::new(instance.to_string(), store, clock)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
    let handler = ControlService::new(
        StopController::new(instance, boot, Cancellation::default()),
        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
        domain,
    );
    let call = |command| {
        handler
            .handle(command, PeerIdentity::from_kernel(501), &budget())
            .unwrap()
    };
    let CommandResult::Capabilities(caps) = call(Command::Capabilities) else {
        panic!("capabilities")
    };
    assert!(
        caps.capabilities
            .iter()
            .any(|name| name == DELIVERY_PRESENTATION_V2),
        "configured actual capability response must advertise the selected routes"
    );
    let check_in = |seat: &str| {
        let session = format!("session-{seat}");
        let claim = CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: instance.to_string(),
            seat: SeatId::new(seat),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Agent(builtins().agent("agy").unwrap()),
            native_session: NativeSessionId::new(&session),
            execution: ExecutionId::new(Uuid::new_v4().to_string()),
            target: HostTargetId::new(seat),
            agent_id: None,
            liveness: Some(crate::test_support::isolation::stand_in_fingerprint(
                "agy", &session,
            )),
        };
        let CommandResult::CheckedIn(checked) = call(Command::CheckIn(CheckIn {
            lineage_parent: None,
            mode: CheckInMode::Lifecycle {
                expected_binding_generation: 0,
            },
            claim,
            operation: OperationId::new(format!("check-in-{seat}")),
        })) else {
            panic!("canonical binding")
        };
        checked.context
    };
    let origin = check_in("origin");
    let recipient = check_in("recipient");
    let CommandResult::ThreadCreated(thread) = call(Command::CreateThread(CreateThread {
        name: None,
        topic: "capability route".into(),
        goal: "canonical page retirement".into(),
        operation: OperationId::new("route-create"),
        claim: origin.clone(),
    })) else {
        panic!("created thread")
    };
    call(Command::Invite(Invite {
        confirmation: Some(InvitationConfirmation::RequireAck),
        thread,
        seat: recipient.seat.clone(),
        deadline_millis: Some(300_000),
        operation: OperationId::new("route-invite"),
        claim: origin,
    }));
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) =
        call(Command::DeliveryPullV2(DeliveryPullV2 {
            claim: recipient.clone(),
            consumer: PresentationConsumer::Hook,
            render: PresentationRenderRequest::Hook {
                confirmable_selector: 0,
                passive_selector: 0,
            },
            presentation_lease: None,
            marker_prefix: vec![],
        }))
    else {
        panic!("configured production PullV2 route allocates a canonical page")
    };
    assert!(!page.items.is_empty());
    assert!(
        !page.emitted_controls.is_empty(),
        "exercise canonical control retirement"
    );
    let token = page
        .presentation_token
        .expect("canonical presentation token");
    let confirm = Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
        emitted_controls: page.emitted_controls.clone(),
        claim: recipient,
        consumer: PresentationConsumer::Hook,
        token: token.clone(),
        presentation_lease: None,
    });
    let CommandResult::DeliveryPresentationConfirmedV2(reply) = call(confirm.clone()) else {
        panic!("configured production Confirm route")
    };
    assert_eq!(reply.status, DeliveryPresentationConfirmationV2::Confirmed);
    assert_eq!(reply.acknowledged_controls, page.emitted_controls);
    let CommandResult::DeliveryPresentationConfirmedV2(replay) = call(confirm) else {
        panic!("exact canonical replay")
    };
    assert_eq!(
        replay.status,
        DeliveryPresentationConfirmationV2::AlreadyConfirmed
    );
    assert_eq!(replay.acknowledged_controls, reply.acknowledged_controls);
    assert_eq!(db.query_row("SELECT last_token FROM invitation_presentation_consumers WHERE seat_id='recipient' AND consumer='hook'", [], |row| row.get::<_,String>(0)).unwrap(), token);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM invitations WHERE state='pending'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

/// `inbox.pushed_hold_v1`: `InboxQuery.pushed` is on the wire only when set and round-trips; only
/// a v2 first, unscoped page may carry it. The store applies the catch-up hold to it
/// (`summary_flow::hold_and_bypass`).
fn probe_inbox_pushed_hold() {
    use crate::protocol::{commands::InboxQuery, ids::SeatId, pagination::PageRequest};
    let query = |pushed: bool, cursor: Option<&str>| InboxQuery {
        seat: Some(SeatId::new("s")),
        page: PageRequest {
            cursor: cursor.map(str::to_owned),
            ..PageRequest::default()
        },
        claim: None,
        thread: None,
        pushed,
    };
    let pushed = Command::InboxBatchV2(query(true, None));
    assert!(pushed.validate().is_ok());
    let json = serde_json::to_value(&pushed).unwrap();
    assert_eq!(json.to_string().matches("\"pushed\":true").count(), 1);
    assert_eq!(serde_json::from_value::<Command>(json).unwrap(), pushed);
    let plain = serde_json::to_value(Command::InboxBatchV2(query(false, None))).unwrap();
    assert!(!plain.to_string().contains("\"pushed\""), "{plain}");
    assert!(Command::Inbox(query(true, None)).validate().is_err());
    assert!(Command::InboxBatch(query(true, None)).validate().is_err());
    assert!(
        Command::InboxBatchV2(query(true, Some("v2:x")))
            .validate()
            .is_err()
    );
}

/// `inbox.thread_scope_v1` (ht-br0): `InboxQuery.thread` is on the wire only when set and
/// round-trips; only a v2 first page may carry it. The store serves it
/// (`binding_inbox::thread_scoped_page_lists_only_that_threads_lazy_rows`).
fn probe_inbox_thread_scope() {
    use crate::protocol::{
        commands::InboxQuery,
        ids::{SeatId, ThreadId},
        pagination::PageRequest,
    };
    let query = |thread: Option<&str>, cursor: Option<&str>| InboxQuery {
        pushed: false,
        seat: Some(SeatId::new("s")),
        page: PageRequest {
            cursor: cursor.map(str::to_owned),
            ..PageRequest::default()
        },
        claim: None,
        thread: thread.map(ThreadId::new),
    };
    let scoped = Command::InboxBatchV2(query(Some("t"), None));
    assert!(scoped.validate().is_ok());
    let json = serde_json::to_value(&scoped).unwrap();
    assert_eq!(json.to_string().matches("\"thread\":\"t\"").count(), 1);
    assert_eq!(serde_json::from_value::<Command>(json).unwrap(), scoped);
    let plain = serde_json::to_value(Command::InboxBatchV2(query(None, None))).unwrap();
    assert!(!plain.to_string().contains("\"thread\""), "{plain}");
    assert!(Command::Inbox(query(Some("t"), None)).validate().is_err());
    assert!(
        Command::InboxBatch(query(Some("t"), None))
            .validate()
            .is_err()
    );
    assert!(
        Command::InboxBatchV2(query(Some("t"), Some("v2:x")))
            .validate()
            .is_err()
    );
}
