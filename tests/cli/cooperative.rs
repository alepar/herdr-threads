use crate::cli::{
    journal::{IntentScope, Journal, SemanticMutation},
    retry,
};
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness},
    commands::Command,
    ids::*,
    results::{AckResult, CommandResult},
};
use std::io;

#[test]
fn selected_service_mapping_rejects_wrong_target_and_repair_hold() {
    use crate::cli::commands::CooperativeSelection;
    use crate::harness::context::{Harness as ContextHarness, Role};
    use crate::protocol::pagination::{Consistency, Page, StopReason};
    use crate::protocol::results::{
        ContinuityStatus, HoldSummary, MappingStatus, SeatInspection, SeatSummary,
    };
    let selection = CooperativeSelection {
        seat: SeatId::new("seat"),
        target: HostTargetId::new("pane"),
        harness: ContextHarness::Codex,
        role: Role::TopLevel,
    };
    let mut inspection = SeatInspection {
        summary: SeatSummary {
            seat: selection.seat.clone(),
            continuity: ContinuityStatus::Resolved,
            target: Some(selection.target.clone()),
            generation: 7,
            created_at: crate::protocol::time::UtcMillis(0),
            retired_at: None,
        },
        mapping: MappingStatus {
            state: ContinuityStatus::Resolved,
            target: Some(selection.target.clone()),
            detail_argv: None,
        },
        hold: None,
        retirement: None,
        open_binding: None,
        history: Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        },
    };
    assert_eq!(
        crate::cli::selected_generation(&selection, &inspection).unwrap(),
        7
    );
    inspection.summary.target = Some(HostTargetId::new("other"));
    assert!(crate::cli::selected_generation(&selection, &inspection).is_err());
    inspection.summary.target = Some(selection.target.clone());
    inspection.hold = Some(HoldSummary {
        target: selection.target.clone(),
        reason_data: "repair".into(),
        detail_argv: vec![],
    });
    assert!(crate::cli::selected_generation(&selection, &inspection).is_err());
}

#[test]
fn selected_first_lifecycle_uses_service_generation_and_persists_context() {
    selected_first_lifecycle_fixture(false);
}

#[test]
fn actor_boundary_explicit_agent_selection_survives() {
    selected_first_lifecycle_fixture(true);
}

fn selected_first_lifecycle_fixture(replace_human: bool) {
    use crate::cli::commands::CooperativeSelection;
    use crate::harness::context::{Harness as ContextHarness, Role};
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::{
            CheckInContextDisposition, CheckInResult, ContinuityStatus, MappingStatus,
            SeatInspection, SeatSummary,
        },
    };
    let root = std::env::temp_dir().join(format!("cli-selection-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let instance = uuid::Uuid::from_u128(1);
    let selection = CooperativeSelection {
        seat: SeatId::new("seat"),
        target: HostTargetId::new("pane"),
        harness: ContextHarness::Codex,
        role: Role::TopLevel,
    };
    fn empty<T>() -> Page<T> {
        Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }
    }
    struct Client {
        calls: std::sync::Mutex<Vec<Command>>,
        generation: std::sync::atomic::AtomicU64,
    }
    impl crate::ports::LocalClient for Client {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.calls.lock().unwrap().push(command.clone());
            match command {
                Command::SeatInspect(q) => Ok(CommandResult::SeatInspect(SeatInspection {
                    summary: SeatSummary {
                        seat: q.seat,
                        continuity: ContinuityStatus::Resolved,
                        target: Some(HostTargetId::new("pane")),
                        generation: self.generation.load(std::sync::atomic::Ordering::SeqCst),
                        created_at: crate::protocol::time::UtcMillis(0),
                        retired_at: None,
                    },
                    mapping: MappingStatus {
                        state: ContinuityStatus::Resolved,
                        target: Some(HostTargetId::new("pane")),
                        detail_argv: None,
                    },
                    hold: None,
                    retirement: None,
                    open_binding: None,
                    history: empty(),
                })),
                Command::CheckIn(c) => {
                    assert_eq!(
                        c.mode,
                        crate::protocol::commands::CheckInMode::Lifecycle {
                            expected_binding_generation: 1
                        }
                    );
                    let mut context = c.claim;
                    context.binding_generation = 2;
                    Ok(CommandResult::CheckedIn(CheckInResult {
                        seat: context.seat.clone(),
                        context,
                        context_disposition: CheckInContextDisposition::Current,
                        offered_through: None,
                        warning_count: 0,
                        warning_count_has_more: false,
                        warnings: empty(),
                        notices: Default::default(),
                        inbox: empty(),
                    }))
                }
                Command::Ack(c) => Ok(CommandResult::Acknowledged(AckResult {
                    acknowledged: c.messages,
                    already_acknowledged: vec![],
                })),
                _ => panic!("unexpected command"),
            }
        }
    }
    let client = Client {
        calls: Default::default(),
        generation: std::sync::atomic::AtomicU64::new(1),
    };
    let contexts = crate::cli::seat_contexts(&paths, instance, &selection.seat).unwrap();
    if replace_human {
        let execution = uuid::Uuid::new_v4();
        contexts
            .install_reattached(crate::harness::context::OccupantContext {
                format_version: 1,
                instance,
                seat: selection.seat.as_str().into(),
                target: selection.target.as_str().into(),
                harness: ContextHarness::Human,
                binding_generation: 1,
                execution,
                session: crate::harness::context::SessionReference::PluginContext(execution),
                role: Role::TopLevel,
            })
            .unwrap();
    }
    let parsed = crate::cli::commands::parse_argv([
        "herdr-threads",
        "check-in",
        "--lifecycle-event",
        "launch-one",
    ])
    .unwrap();
    let mut output = Vec::new();
    crate::cli::run_selected(
        parsed,
        &selection,
        &paths,
        instance,
        &client,
        &crate::app::SystemClock::new(),
        &mut output,
    )
    .unwrap();
    assert_eq!(
        contexts.current().unwrap().unwrap().harness,
        ContextHarness::Codex
    );
    assert!(!output.is_empty());
    assert_eq!(client.calls.lock().unwrap().len(), 2);
    let ack = crate::cli::commands::parse_argv(["herdr-threads", "ack", "message"]).unwrap();
    assert!(
        crate::cli::run_selected(
            ack.clone(),
            &selection,
            &paths,
            instance,
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        )
        .is_err()
    );
    assert_eq!(
        client.calls.lock().unwrap().len(),
        3,
        "stale mapping cannot dispatch ACK"
    );
    client
        .generation
        .store(2, std::sync::atomic::Ordering::SeqCst);
    crate::cli::run_selected(
        ack,
        &selection,
        &paths,
        instance,
        &client,
        &crate::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(client.calls.lock().unwrap().len(), 5);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_predecessor_replays_after_successor_without_a_new_check_in() {
    use crate::cli::commands::CooperativeSelection;
    use crate::harness::context::{Harness as ContextHarness, Role};
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::{
            CheckInContextDisposition, CheckInResult, ContinuityStatus, MappingStatus,
            SeatInspection, SeatSummary,
        },
    };
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };

    fn empty<T>() -> Page<T> {
        Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }
    }
    struct Client {
        generation: AtomicU64,
        check_ins: AtomicU64,
        replay_calls: AtomicU64,
        lose_response: std::sync::atomic::AtomicBool,
        fail_before_commit: std::sync::atomic::AtomicBool,
        saved: std::sync::Mutex<Option<(OperationId, CommandResult)>>,
    }
    impl crate::ports::LocalClient for Client {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            match command {
                Command::SeatInspect(q) => Ok(CommandResult::SeatInspect(SeatInspection {
                    summary: SeatSummary {
                        seat: q.seat,
                        continuity: ContinuityStatus::Resolved,
                        target: Some(HostTargetId::new("pane")),
                        generation: self.generation.load(Ordering::SeqCst),
                        created_at: crate::protocol::time::UtcMillis(0),
                        retired_at: None,
                    },
                    mapping: MappingStatus {
                        state: ContinuityStatus::Resolved,
                        target: Some(HostTargetId::new("pane")),
                        detail_argv: None,
                    },
                    hold: None,
                    retirement: None,
                    open_binding: None,
                    history: empty(),
                })),
                Command::CheckIn(c) => {
                    if let Some((key, original)) = self.saved.lock().unwrap().as_ref()
                        && key == &c.operation
                    {
                        self.replay_calls.fetch_add(1, Ordering::SeqCst);
                        let mut replay = original.clone();
                        let CommandResult::CheckedIn(result) = &mut replay else {
                            panic!()
                        };
                        result.context_disposition = CheckInContextDisposition::Historical;
                        return Ok(replay);
                    }
                    let crate::protocol::commands::CheckInMode::Lifecycle {
                        expected_binding_generation,
                    } = c.mode
                    else {
                        panic!("expected lifecycle")
                    };
                    if self.fail_before_commit.swap(false, Ordering::SeqCst) {
                        return Err(crate::protocol::results::ApiError::host_unavailable(
                            "request lost before commit",
                        ));
                    }
                    if expected_binding_generation != self.generation.load(Ordering::SeqCst) {
                        return Err(crate::protocol::results::ApiError::caller_unverified(
                            "stale generation",
                        ));
                    }
                    self.check_ins.fetch_add(1, Ordering::SeqCst);
                    let mut context = c.claim;
                    context.binding_generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
                    let result = CommandResult::CheckedIn(CheckInResult {
                        seat: context.seat.clone(),
                        context,
                        context_disposition: CheckInContextDisposition::Current,
                        offered_through: None,
                        warning_count: 0,
                        warning_count_has_more: false,
                        warnings: empty(),
                        notices: Default::default(),
                        inbox: empty(),
                    });
                    *self.saved.lock().unwrap() = Some((c.operation, result.clone()));
                    if self.lose_response.swap(false, Ordering::SeqCst) {
                        return Err(crate::protocol::results::ApiError::host_unavailable(
                            "committed response lost",
                        ));
                    }
                    Ok(result)
                }
                _ => panic!("unexpected dispatch"),
            }
        }
    }
    struct LostOutput;
    impl Write for LostOutput {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "lost output"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let root = std::env::temp_dir().join(format!("cli-replay-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let selection = CooperativeSelection {
        seat: SeatId::new("seat"),
        target: HostTargetId::new("pane"),
        harness: ContextHarness::Codex,
        role: Role::TopLevel,
    };
    let client = Client {
        generation: AtomicU64::new(1),
        check_ins: AtomicU64::new(0),
        replay_calls: AtomicU64::new(0),
        lose_response: std::sync::atomic::AtomicBool::new(false),
        fail_before_commit: std::sync::atomic::AtomicBool::new(false),
        saved: Default::default(),
    };
    fn invoke<W: Write>(
        event: &str,
        writer: &mut W,
        selection: &CooperativeSelection,
        paths: &crate::daemon::paths::InstancePaths,
        client: &Client,
    ) -> Result<(), crate::cli::RunError> {
        let parsed = crate::cli::commands::parse_argv([
            "herdr-threads",
            "--json",
            "check-in",
            "--lifecycle-event",
            event,
        ])
        .unwrap();
        crate::cli::run_selected(
            parsed,
            selection,
            paths,
            uuid::Uuid::from_u128(1),
            client,
            &crate::app::SystemClock::new(),
            writer,
        )
    }
    assert!(invoke("predecessor", &mut LostOutput, &selection, &paths, &client).is_err());
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 1);
    // A different process advanced the durable service generation. This local
    // context journal still has the completed predecessor and its lost output.
    client.generation.store(3, Ordering::SeqCst);
    client.check_ins.store(2, Ordering::SeqCst);
    let changed_payload = crate::cli::commands::parse_argv([
        "herdr-threads",
        "--json",
        "check-in",
        "--lifecycle-event",
        "predecessor",
        "--native-session",
        "edited-session",
    ])
    .unwrap();
    assert!(
        crate::cli::run_selected(
            changed_payload,
            &selection,
            &paths,
            uuid::Uuid::from_u128(1),
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new()
        )
        .is_err()
    );
    let stale_ack = crate::cli::commands::parse_argv(["herdr-threads", "ack", "message"]).unwrap();
    assert!(
        crate::cli::run_selected(
            stale_ack,
            &selection,
            &paths,
            uuid::Uuid::from_u128(1),
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 2);
    assert_eq!(client.generation.load(Ordering::SeqCst), 3);
    let mut original = Vec::new();
    invoke("predecessor", &mut original, &selection, &paths, &client).unwrap();
    let old: serde_json::Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(old["result"]["data"]["context"]["binding_generation"], 2);
    assert_eq!(
        client.check_ins.load(Ordering::SeqCst),
        2,
        "cached replay must not dispatch"
    );
    let added_native = crate::cli::commands::parse_argv([
        "herdr-threads",
        "--json",
        "check-in",
        "--lifecycle-event",
        "predecessor",
        "--native-session",
        "added-after-completion",
    ])
    .unwrap();
    assert!(
        crate::cli::run_selected(
            added_native,
            &selection,
            &paths,
            uuid::Uuid::from_u128(1),
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new()
        )
        .is_err()
    );
    // Root decision (wave-2 fix1 (b)): a fresh lifecycle event is a
    // deliberate new registration. It passes the context gate, CASes the
    // service's current generation (3, not the local 2) and replaces the
    // local context; the stale ACK above stayed refused.
    // Kills: the gate refusing every fresh lifecycle check-in after an
    // out-of-band generation change (fix1 S2), and seeding it from the stale
    // local context instead of the service mapping.
    let mut fresh = Vec::new();
    invoke("changed-key", &mut fresh, &selection, &paths, &client).unwrap();
    let fresh: serde_json::Value = serde_json::from_slice(&fresh).unwrap();
    assert_eq!(fresh["result"]["data"]["context"]["binding_generation"], 4);
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 3);
    std::fs::remove_dir_all(root).unwrap();

    // The transport can lose the response after service commit, leaving a
    // pending local request rather than a completed cached response.
    let root = std::env::temp_dir().join(format!("cli-pending-replay-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let client = Client {
        generation: AtomicU64::new(1),
        check_ins: AtomicU64::new(0),
        replay_calls: AtomicU64::new(0),
        lose_response: std::sync::atomic::AtomicBool::new(false),
        fail_before_commit: std::sync::atomic::AtomicBool::new(false),
        saved: Default::default(),
    };
    invoke("initial", &mut Vec::new(), &selection, &paths, &client).unwrap();
    client.lose_response.store(true, Ordering::SeqCst);
    let native_session = crate::cli::commands::parse_argv([
        "herdr-threads",
        "--json",
        "check-in",
        "--lifecycle-event",
        "lost-response",
        "--native-session",
        "native-original",
    ])
    .unwrap();
    assert!(
        crate::cli::run_selected(
            native_session,
            &selection,
            &paths,
            uuid::Uuid::from_u128(1),
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert_eq!(client.generation.load(Ordering::SeqCst), 3);
    client.generation.store(4, Ordering::SeqCst);
    client.check_ins.store(3, Ordering::SeqCst);
    assert!(
        invoke(
            "lost-response",
            &mut Vec::new(),
            &selection,
            &paths,
            &client
        )
        .is_err(),
        "omitting the original native session cannot use pending replay"
    );
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 3);
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let pending = journal
        .page(&crate::protocol::pagination::PageRequest {
            cursor: None,
            limit: 10,
            max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
        })
        .unwrap();
    assert_eq!(pending.items.len(), 1);
    let reference = pending.items[0].recovery_ref.as_str();
    let retry =
        crate::cli::commands::parse_argv(["herdr-threads", "--json", "retry", reference]).unwrap();
    let mut recovered = Vec::new();
    crate::cli::run_selected(
        retry,
        &selection,
        &paths,
        uuid::Uuid::from_u128(1),
        &client,
        &crate::app::SystemClock::new(),
        &mut recovered,
    )
    .unwrap();
    let old: serde_json::Value = serde_json::from_slice(&recovered).unwrap();
    assert_eq!(old["result"]["data"]["context"]["binding_generation"], 3);
    assert_eq!(client.replay_calls.load(Ordering::SeqCst), 1);
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 3);
    // A fresh event after recovery registers at the current generation (4).
    let mut fresh = Vec::new();
    invoke("changed-key", &mut fresh, &selection, &paths, &client).unwrap();
    let fresh: serde_json::Value = serde_json::from_slice(&fresh).unwrap();
    assert_eq!(fresh["result"]["data"]["context"]["binding_generation"], 5);
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 4);
    std::fs::remove_dir_all(root).unwrap();

    let root = std::env::temp_dir().join(format!("cli-uncommitted-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let client = Client {
        generation: AtomicU64::new(1),
        check_ins: AtomicU64::new(0),
        replay_calls: AtomicU64::new(0),
        lose_response: std::sync::atomic::AtomicBool::new(false),
        fail_before_commit: std::sync::atomic::AtomicBool::new(false),
        saved: Default::default(),
    };
    invoke("initial", &mut Vec::new(), &selection, &paths, &client).unwrap();
    client.fail_before_commit.store(true, Ordering::SeqCst);
    assert!(invoke("uncommitted", &mut Vec::new(), &selection, &paths, &client).is_err());
    client.generation.store(4, Ordering::SeqCst);
    let failure = invoke("uncommitted", &mut Vec::new(), &selection, &paths, &client).unwrap_err();
    assert!(matches!(failure, crate::cli::RunError::Api(error)
        if error.code == crate::protocol::results::ErrorCode::CallerUnverified));
    assert_eq!(client.check_ins.load(Ordering::SeqCst), 1);
    assert_eq!(client.replay_calls.load(Ordering::SeqCst), 0);
    assert_eq!(client.generation.load(Ordering::SeqCst), 4);
    std::fs::remove_dir_all(root).unwrap();
}
fn claim() -> CallerClaim {
    CallerClaim {
        instance: "instance".into(),
        seat: SeatId::new("seat"),
        binding_generation: 7,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("plugin_context:context"),
        execution: ExecutionId::new("execution"),
        target: HostTargetId::new("pane"),
    }
}
#[test]
fn cooperative_ack_retry_retains_claim_key_after_output_loss_and_restart() {
    let dir = std::env::temp_dir().join(format!("cooperative-intent-{}", uuid::Uuid::new_v4()));
    let j = Journal::open(&dir).unwrap();
    let c = claim();
    let scope = IntentScope::Cooperative {
        instance: c.instance.clone(),
        seat: c.seat.clone(),
    };
    let semantic = SemanticMutation::freeze(
        SemanticMutation::Ack {
            messages: vec![MessageId::new("message")],
        },
        c.clone(),
    )
    .unwrap();
    let r = j.record(scope.clone(), semantic, 1).unwrap();
    let result = retry::run_retry(
        &j,
        &r,
        &scope,
        || panic!("must not refresh cooperative claim"),
        |command| {
            let Command::Ack(a) = command else {
                panic!("wrong mutation")
            };
            assert_eq!(a.claim, c);
            assert_eq!(a.operation, r.operation);
            Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![MessageId::new("message")],
                already_acknowledged: vec![],
            }))
        },
        |_| Err(io::Error::new(io::ErrorKind::BrokenPipe, "output lost")),
    );
    assert!(result.is_err());
    drop(j);
    let j = Journal::open(&dir).unwrap();
    retry::run_retry(
        &j,
        &r,
        &scope,
        || panic!("no fresh claim"),
        |command| {
            let Command::Ack(a) = command else { panic!() };
            assert_eq!(a.claim, c);
            assert_eq!(a.operation, r.operation);
            Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![],
                already_acknowledged: vec![MessageId::new("message")],
            }))
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(j.load(&r).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn displayed_ack_keeps_frozen_intent_after_uncertain_submission() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("displayed-ack-intent-{}", uuid::Uuid::new_v4()));
    let journal = Journal::open(&dir).unwrap();
    let claim = claim();
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let semantic = SemanticMutation::freeze(
        SemanticMutation::AckDisplayed {
            messages: vec![MessageId::new("stored-message")],
        },
        claim.clone(),
    )
    .unwrap();
    let reference = journal.record(scope.clone(), semantic, 1).unwrap();
    let mut output = Vec::new();
    let failure = retry::run_retry_api_to_writer(
        &journal,
        &reference,
        &scope,
        || panic!("frozen claim must be reused"),
        |command| {
            let Command::AckDisplayed(ack) = command else {
                panic!("wrong mutation")
            };
            assert_eq!(ack.claim, claim);
            Err(crate::protocol::results::ApiError::unknown_outcome(
                "response lost",
            ))
        },
        &crate::protocol::output::OutputSpec::default(),
        &mut output,
    );
    assert!(failure.is_err());
    assert!(
        journal.load(&reference).is_ok(),
        "uncertain submission retains intent"
    );
    retry::run_retry_api_to_writer(
        &journal,
        &reference,
        &scope,
        || panic!("frozen claim must be reused"),
        |command| {
            let Command::AckDisplayed(ack) = command else {
                panic!("wrong mutation")
            };
            assert_eq!(ack.operation, reference.operation);
            Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![],
                already_acknowledged: ack.messages,
            }))
        },
        &crate::protocol::output::OutputSpec::default(),
        &mut output,
    )
    .unwrap();
    assert!(journal.load(&reference).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cooperative_scope_rejects_child_operator_and_wrong_instance() {
    let dir = std::env::temp_dir().join(format!("cooperative-scope-{}", uuid::Uuid::new_v4()));
    let j = Journal::open(&dir).unwrap();
    let mut c = claim();
    c.role = CallerRole::Subagent;
    assert!(
        SemanticMutation::freeze(
            SemanticMutation::Ack {
                messages: vec![MessageId::new("m")]
            },
            c
        )
        .is_err()
    );
    assert!(
        SemanticMutation::freeze(
            SemanticMutation::OperatorFreshSeat {
                target: HostTargetId::new("p")
            },
            claim()
        )
        .is_err()
    );
    let s = SemanticMutation::freeze(
        SemanticMutation::Accept {
            thread: ThreadId::new("t"),
        },
        claim(),
    )
    .unwrap();
    assert!(
        j.record(
            IntentScope::Cooperative {
                instance: "other".into(),
                seat: SeatId::new("seat")
            },
            s,
            1
        )
        .is_err()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn required_acceptance_journal_retains_revision_across_recovery() {
    let dir = std::env::temp_dir().join(format!("required-accept-intent-{}", uuid::Uuid::new_v4()));
    let journal = Journal::open(&dir).unwrap();
    let c = claim();
    let scope = IntentScope::Cooperative {
        instance: c.instance.clone(),
        seat: c.seat.clone(),
    };
    let semantic = SemanticMutation::freeze(
        SemanticMutation::AcceptRequired {
            thread: ThreadId::new("thread"),
            invitation: InvitationId::new("invite"),
            requirement: RequirementId::new("requirement"),
            expected_revision: 3,
        },
        c.clone(),
    )
    .unwrap();
    let intent = journal.record(scope, semantic, 1).unwrap();
    drop(journal);
    let journal = Journal::open(&dir).unwrap();
    let pending = journal.load(&intent).unwrap();
    let Command::AcceptRequired(accept) =
        pending.semantic.to_command(intent.operation, None).unwrap()
    else {
        panic!("lost required acceptance")
    };
    assert_eq!(accept.claim, c);
    assert_eq!(accept.expected_revision, 3);
    assert_eq!(accept.invitation.as_str(), "invite");
    assert_eq!(accept.requirement.as_str(), "requirement");
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn lifecycle_intent_recovers_original_claim_and_cas_before_context_publication() {
    let dir = std::env::temp_dir().join(format!("cooperative-event-{}", uuid::Uuid::new_v4()));
    let j = Journal::open(&dir).unwrap();
    let c = claim();
    let scope = IntentScope::Cooperative {
        instance: c.instance.clone(),
        seat: c.seat.clone(),
    };
    let r = j
        .record_check_in(scope.clone(), "external-start", 1, || {
            Ok((
                c.clone(),
                crate::protocol::commands::CheckInMode::Lifecycle {
                    expected_binding_generation: 7,
                },
            ))
        })
        .unwrap();
    drop(j);
    let j = Journal::open(&dir).unwrap();
    let recovered = j
        .record_check_in(scope, "external-start", 2, || {
            panic!("must not mint a new execution")
        })
        .unwrap();
    assert_eq!(r, recovered);
    let Command::CheckIn(ci) = j
        .load(&r)
        .unwrap()
        .semantic
        .to_command(r.operation.clone(), None)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(ci.claim, c);
    assert_eq!(
        ci.mode,
        crate::protocol::commands::CheckInMode::Lifecycle {
            expected_binding_generation: 7
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn allocator_lock_wait_is_bounded_before_context_factory() {
    use std::fs::OpenOptions;
    let dir = std::env::temp_dir().join(format!("cooperative-lock-{}", uuid::Uuid::new_v4()));
    let j = Journal::open(&dir).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join("allocator.lock"))
        .unwrap();
    lock.lock().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1400));
        drop(lock);
    });
    let c = claim();
    let result = j.record_check_in(
        IntentScope::Cooperative {
            instance: c.instance.clone(),
            seat: c.seat.clone(),
        },
        "locked",
        1,
        || Ok((c, crate::protocol::commands::CheckInMode::Current)),
    );
    release.join().unwrap();
    assert!(
        result.is_err(),
        "allocation must return before held lock is released"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn explicit_lifecycle_argv_retains_external_identity_and_session_absence() {
    let parsed = crate::cli::commands::parse_argv([
        "herdr-threads",
        "check-in",
        "--lifecycle-event",
        "launch-123",
    ])
    .unwrap();
    assert!(
        matches!(parsed.action,crate::cli::commands::CliAction::Mutation(crate::cli::commands::MutationSpec::CheckInLifecycle{event_id,native_session:None,operator:false}) if event_id=="launch-123")
    );
    assert!(
        crate::cli::commands::parse_argv([
            "herdr-threads",
            "check-in",
            "--native-session",
            "session"
        ])
        .is_err()
    );
}
#[test]
fn model_ack_runner_requires_current_context_and_child_cannot_mutate() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("cooperative-runner-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let j = Journal::open(dir.join("intents")).unwrap();
    let c = crate::harness::context::ContextJournal::open(
        &dir,
        uuid::Uuid::from_u128(1),
        "seat",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    struct Never;
    impl crate::ports::LocalClient for Never {
        crate::default_output_local_client!();
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("missing/child context must not reach transport")
        }
    }
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "ack", "message"]).unwrap();
    let mut out = vec![];
    assert!(
        crate::cli::run_cooperative(
            parsed.clone(),
            &j,
            &c,
            None,
            crate::harness::context::Role::TopLevel,
            &Never,
            &crate::app::SystemClock::new(),
            &mut out
        )
        .is_err()
    );
    assert!(
        crate::cli::run_cooperative(
            parsed,
            &j,
            &c,
            None,
            crate::harness::context::Role::Subagent,
            &Never,
            &crate::app::SystemClock::new(),
            &mut out
        )
        .is_err()
    );
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn cooperative_reader_enforces_selected_page_budget_before_writing() {
    struct Big;
    impl crate::ports::LocalClient for Big {
        crate::default_output_local_client!();
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            Ok(CommandResult::Health(
                crate::protocol::results::Health::unknown(
                    "i".into(),
                    "b".into(),
                    "v".repeat(2000),
                    1,
                ),
            ))
        }
    }
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("cooperative-budget-{}", uuid::Uuid::new_v4()));
    let j = Journal::open(&dir).unwrap();
    let c = crate::harness::context::ContextJournal::open(
        &dir,
        uuid::Uuid::from_u128(1),
        "seat",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let parsed =
        crate::cli::commands::parse_argv(["herdr-threads", "inbox", "--max-bytes", "256"]).unwrap();
    let mut out = vec![];
    assert!(
        crate::cli::run_cooperative(
            parsed,
            &j,
            &c,
            None,
            crate::harness::context::Role::Subagent,
            &Big,
            &crate::app::SystemClock::new(),
            &mut out
        )
        .is_err()
    );
    assert!(out.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn actor_boundary_human_own_inbox_never_display_acks() {
    inbox_empty_work_flush_before_ack(false, false);
}

#[test]
fn inbox_v2_empty_work_flush_before_ack() {
    inbox_empty_work_flush_before_ack(true, false);
}

#[test]
fn inbox_v2_empty_work_flush_before_lazy_completion() {
    inbox_empty_work_flush_before_ack(true, true);
}

fn inbox_empty_work_flush_before_ack(v2: bool, lazy: bool) {
    use crate::harness::context::{
        ContextJournal, Harness as ContextHarness, OccupantContext, Role, SessionReference,
    };
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::InboxBatchItem,
    };
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    struct Client {
        calls: Mutex<Vec<Command>>,
        supports_batch: std::sync::atomic::AtomicBool,
        capability_unavailable: std::sync::atomic::AtomicBool,
        v2: bool,
        lazy: bool,
    }
    impl crate::ports::LocalClient for Client {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.calls.lock().unwrap().push(command.clone());
            match command {
                Command::Capabilities
                    if self
                        .capability_unavailable
                        .load(std::sync::atomic::Ordering::SeqCst) =>
                {
                    Err(crate::protocol::results::ApiError::new(
                        crate::protocol::results::ErrorCode::HostUnavailable,
                        "private daemon unavailable",
                    ))
                }
                Command::Capabilities => Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList {
                        capabilities: if self
                            .supports_batch
                            .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            vec![if self.v2 {
                                crate::protocol::capabilities::INBOX_BATCH_V2.into()
                            } else {
                                crate::protocol::capabilities::INBOX_BATCH.into()
                            }]
                        } else {
                            vec![]
                        },
                    },
                )),
                Command::Inbox(_) => Ok(CommandResult::Inbox(Page {
                    items: vec![],
                    next_cursor: None,
                    next_argv: None,
                    high_water_ordinal: 1,
                    scope_revision: None,
                    has_more: false,
                    stop_reason: StopReason::Complete,
                    consistency: Consistency::BoundedLive,
                })),
                Command::InboxBatch(query) | Command::InboxBatchV2(query) => {
                    let result = |page| {
                        if self.v2 {
                            let mut json = serde_json::to_value(page).unwrap();
                            if self.lazy {
                                for item in json["items"].as_array_mut().unwrap() {
                                    item["kind"] = "lazy_message".into();
                                    item.as_object_mut().unwrap().remove("ack_candidate");
                                }
                            }
                            CommandResult::InboxBatchV2(serde_json::from_value(json).unwrap())
                        } else {
                            CommandResult::InboxBatch(page)
                        }
                    };
                    assert_eq!(query.seat.as_ref().map(SeatId::as_str), Some("seat-test"));
                    if query.page.cursor.as_deref() != Some("empty-two") {
                        return Ok(result(Page {
                            items: vec![],
                            next_cursor: Some(
                                if query.page.cursor.is_none() {
                                    "empty-one"
                                } else {
                                    "empty-two"
                                }
                                .into(),
                            ),
                            next_argv: Some(vec!["hidden-continuation".into()]),
                            high_water_ordinal: 1,
                            scope_revision: None,
                            has_more: true,
                            stop_reason: StopReason::Work,
                            consistency: Consistency::BoundedLive,
                        }));
                    }
                    Ok(result(Page {
                        items: vec![InboxBatchItem::Message {
                            thread: ThreadId::new("thread-original"),
                            message: MessageId::new("message-original"),
                            sequence: 1,
                            topic_data: "topic".into(),
                            sender: Some(SeatId::new("sender-original")),
                            author_role: None,
                            relays_user: false,
                            user_intent: None,
                            author_role_backfilled: false,
                            body: "full body".into(),
                            body_start: 0,
                            body_end: 9,
                            body_len: 9,
                            ack_candidate: Some(MessageId::new("message-original")),
                        }],
                        next_cursor: None,
                        next_argv: None,
                        high_water_ordinal: 1,
                        scope_revision: None,
                        has_more: false,
                        stop_reason: StopReason::Complete,
                        consistency: Consistency::BoundedLive,
                    }))
                }
                Command::CompleteInboxDelivery(completion) => {
                    assert!(self.lazy);
                    assert_eq!(
                        completion.messages,
                        vec![MessageId::new("message-original")]
                    );
                    Ok(CommandResult::InboxDeliveryCompleted(completion.messages))
                }
                Command::AckDisplayed(ack) => {
                    assert!(!self.lazy);
                    assert_eq!(ack.messages, vec![MessageId::new("message-original")]);
                    Ok(CommandResult::Acknowledged(AckResult {
                        acknowledged: ack.messages,
                        already_acknowledged: vec![],
                    }))
                }
                _ => panic!("unexpected command"),
            }
        }
    }
    struct Writer {
        bytes: Vec<u8>,
        fail_write: bool,
        fail_flush: bool,
    }
    impl io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_write {
                self.fail_write = false;
                self.bytes.extend_from_slice(&bytes[..bytes.len().min(4)]);
                return Ok(bytes.len().min(4));
            }
            if !self.bytes.is_empty() {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "partial output"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("inbox-display-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let contexts = ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat-test",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let execution = uuid::Uuid::from_u128(2);
    contexts
        .install_reattached(OccupantContext {
            format_version: 1,
            instance: uuid::Uuid::from_u128(1),
            seat: "seat-test".into(),
            target: "w1:p1".into(),
            harness: ContextHarness::Codex,
            binding_generation: 1,
            execution,
            session: SessionReference::PluginContext(execution),
            role: Role::TopLevel,
        })
        .unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "inbox"]).unwrap();
    let client = Client {
        calls: Mutex::new(Vec::new()),
        supports_batch: std::sync::atomic::AtomicBool::new(false),
        capability_unavailable: std::sync::atomic::AtomicBool::new(false),
        v2,
        lazy,
    };
    let mut writer = Vec::new();
    assert!(
        crate::cli::run_cooperative(
            parsed.clone(),
            &journal,
            &contexts,
            None,
            Role::TopLevel,
            &client,
            &crate::app::SystemClock::new(),
            &mut writer
        )
        .is_err()
    );
    assert!(
        writer.is_empty(),
        "unsupported daemon must not display or ACK"
    );
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    client.calls.lock().unwrap().clear();
    client
        .capability_unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let outcome = crate::cli::run_cooperative(
        parsed.clone(),
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut writer,
    );
    assert!(matches!(outcome, Err(crate::cli::RunError::Api(ref error))
        if error.code == crate::protocol::results::ErrorCode::HostUnavailable));
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    client
        .capability_unavailable
        .store(false, std::sync::atomic::Ordering::SeqCst);
    client
        .supports_batch
        .store(true, std::sync::atomic::Ordering::SeqCst);
    for (fail_write, fail_flush) in [(true, false), (false, true)] {
        client.calls.lock().unwrap().clear();
        let mut writer = Writer {
            bytes: Vec::new(),
            fail_write,
            fail_flush,
        };
        let outcome = crate::cli::run_cooperative(
            parsed.clone(),
            &journal,
            &contexts,
            None,
            Role::TopLevel,
            &client,
            &crate::app::SystemClock::new(),
            &mut writer,
        );
        assert!(outcome.is_err());
        assert_eq!(
            client.calls.lock().unwrap().len(),
            4,
            "no ACK after output failure: {outcome:?}"
        );
        assert!(journal.page(&Default::default()).unwrap().items.is_empty());
        assert!(
            !std::fs::read_dir(root.join("intents")).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("display-")),
            "no progress proof before full write and flush"
        );
    }
    client.calls.lock().unwrap().clear();
    let mut writer = Writer {
        bytes: Vec::new(),
        fail_write: false,
        fail_flush: false,
    };
    crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut writer,
    )
    .unwrap();
    assert_eq!(client.calls.lock().unwrap().len(), 5);
    assert!(
        String::from_utf8(writer.bytes)
            .unwrap()
            .contains("message-original")
    );
    if lazy {
        std::fs::remove_dir_all(root).unwrap();
        return;
    }
    contexts
        .install_reattached(OccupantContext {
            format_version: 1,
            instance: uuid::Uuid::from_u128(1),
            seat: "seat-test".into(),
            target: "w1:p1".into(),
            harness: ContextHarness::Human,
            binding_generation: 2,
            execution,
            session: SessionReference::PluginContext(execution),
            role: Role::TopLevel,
        })
        .unwrap();
    client.calls.lock().unwrap().clear();
    let mut human_output = Vec::new();
    crate::cli::run_cooperative(
        crate::cli::commands::parse_argv(["herdr-threads", "human", "inbox"]).unwrap(),
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut human_output,
    )
    .unwrap();
    assert!(
        String::from_utf8(human_output)
            .unwrap()
            .contains("message-original")
    );
    assert_eq!(
        client.calls.lock().unwrap().len(),
        4,
        "human text inbox is content read-only"
    );
    let before_context = serde_json::to_vec(&contexts.current().unwrap()).unwrap();
    let before_intents: Vec<_> = std::fs::read_dir(root.join("intents"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    for args in [
        vec!["ht", "check-in"],
        vec!["ht", "inbox"],
        vec!["ht", "inbox", "--human"],
    ] {
        client.calls.lock().unwrap().clear();
        let mut output = Vec::new();
        let error = crate::cli::run_cooperative(
            crate::cli::commands::parse_argv(args).unwrap(),
            &journal,
            &contexts,
            None,
            Role::TopLevel,
            &client,
            &crate::app::SystemClock::new(),
            &mut output,
        )
        .unwrap_err();
        assert!(
            matches!(error, crate::cli::RunError::Api(ref e) if e.code == crate::protocol::results::ErrorCode::InvalidRequest),
            "{error:?}"
        );
        assert!(client.calls.lock().unwrap().is_empty());
        assert!(output.is_empty());
        assert_eq!(
            serde_json::to_vec(&contexts.current().unwrap()).unwrap(),
            before_context
        );
        let after_intents: Vec<_> = std::fs::read_dir(root.join("intents"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(after_intents, before_intents);
    }
    for flag in ["--human", "--machine", "--json"] {
        client.calls.lock().unwrap().clear();
        crate::cli::run_cooperative(
            crate::cli::commands::parse_argv(["ht", "human", "inbox", flag]).unwrap(),
            &journal,
            &contexts,
            None,
            Role::TopLevel,
            &client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let calls = client.calls.lock().unwrap();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|call| matches!(
            call,
            Command::Capabilities
                | Command::InboxBatch(_)
                | Command::InboxBatchV2(_)
                | Command::Inbox(_)
        )));
    }
    std::fs::remove_dir_all(root).unwrap();
}
/// Kills the review N1 mutation of the `run_cooperative` `Retry` arm
/// (`src/cli/mod.rs`, "retry context missing"): reverting its
/// `caller_not_located(..)` to `unsupported(..)` (exit 4) or `mapping_error(..)`
/// (exit 1). A frozen cooperative intent retried with no current or pending
/// check-in context and no `initial` seed is a "no caller located" outcome and
/// must be `invalid_request` (exit 2) without reaching the transport or
/// consuming the pending intent.
#[test]
fn cooperative_retry_without_any_context_is_invalid_request() {
    let dir = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("cooperative-retry-ctx-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let j = Journal::open(dir.join("intents")).unwrap();
    let c = claim();
    let scope = IntentScope::Cooperative {
        instance: c.instance.clone(),
        seat: c.seat.clone(),
    };
    let semantic = SemanticMutation::freeze(
        SemanticMutation::Ack {
            messages: vec![MessageId::new("message")],
        },
        c,
    )
    .unwrap();
    let reference = j.record(scope, semantic, 1).unwrap();
    let recovery = j.page(&Default::default()).unwrap().items;
    assert_eq!(recovery.len(), 1);
    let contexts = crate::harness::context::ContextJournal::open(
        &dir,
        uuid::Uuid::from_u128(1),
        "seat",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    assert!(contexts.current().unwrap().is_none());
    assert!(contexts.pending().unwrap().is_none());
    struct Never;
    impl crate::ports::LocalClient for Never {
        crate::default_output_local_client!();
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("a retry without a located caller must not reach transport")
        }
    }
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "retry", "local:1"]).unwrap();
    assert!(
        matches!(parsed.action, crate::cli::commands::CliAction::Retry(_)),
        "`retry local:1` did not parse as a retry"
    );
    let mut out = vec![];
    let failure = crate::cli::run_cooperative(
        parsed,
        &j,
        &contexts,
        None,
        crate::harness::context::Role::TopLevel,
        &Never,
        &crate::app::SystemClock::new(),
        &mut out,
    )
    .expect_err("retry without any context succeeded");
    let crate::cli::RunError::Api(error) = failure else {
        panic!("expected a typed API error, got {failure:?}");
    };
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest,
        "{error:?}"
    );
    assert!(error.detail.contains("retry context missing"), "{error:?}");
    assert_eq!(
        crate::cli::exit::api_exit_code(&error.code),
        2,
        "retry context missing must exit 2"
    );
    assert!(out.is_empty());
    assert!(j.load(&reference).is_ok(), "pending intent was consumed");
    std::fs::remove_dir_all(dir).unwrap();
}

// Native codex matrix P2: a thread read names the located caller seat so the
// service marks its own participant row; a pane-less, selection-less read
// stays unmarked and never fails for it. Kills: never filling `caller`, or
// requiring a caller for a plain thread read.
#[test]
fn thread_reads_carry_the_located_caller_for_the_self_marker() {
    let root = std::path::PathBuf::from("/nonexistent-herdr-threads-test");
    let paths = crate::daemon::paths::InstancePaths {
        instance_dir: root.clone(),
        socket_path: root.join("s"),
        lock_path: root.join("l"),
        descriptor_path: root.join("d"),
        locator_path: root.join("loc"),
        namespace_path: root.join("n"),
        database_path: root.join("db"),
        locator: "test".into(),
    };
    let clock: std::sync::Arc<dyn crate::protocol::time::Clock> =
        std::sync::Arc::new(crate::app::SystemClock::new());
    let runtime = crate::daemon::paths::RuntimeContext {
        state_dir: root.clone(),
        host_endpoint: root.join("host"),
        herdr_bin: None,
    };
    let selected = [
        "herdr-threads",
        "--cooperative-seat",
        "seat-1",
        "--cooperative-target",
        "w:p1",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
    ];
    for tail in [
        &["thread", "participants", "t1"][..],
        &["participants", "t1"][..],
        &["thread", "show", "t1"][..],
    ] {
        let mut parsed =
            crate::cli::commands::parse_argv(selected.iter().chain(tail).copied()).unwrap();
        crate::cli::derive_caller(
            &mut parsed,
            None,
            &runtime,
            &paths,
            &no_connection(),
            &clock,
        )
        .unwrap();
        let caller = match &parsed.action {
            crate::cli::commands::CliAction::Wire(Command::Participants(q)) => q.caller.clone(),
            crate::cli::commands::CliAction::Wire(Command::Thread(q)) => q.caller.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(caller, Some(SeatId::new("seat-1")), "{tail:?}");
        let mut bare =
            crate::cli::commands::parse_argv(["herdr-threads"].iter().chain(tail).copied())
                .unwrap();
        assert!(
            crate::cli::derive_caller(&mut bare, None, &runtime, &paths, &no_connection(), &clock)
                .unwrap()
                .is_none()
        );
        let caller = match &bare.action {
            crate::cli::commands::CliAction::Wire(Command::Participants(q)) => q.caller.clone(),
            crate::cli::commands::CliAction::Wire(Command::Thread(q)) => q.caller.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(caller, None, "{tail:?}");
    }
}

/// A connection these pane-less cases must never open.
fn no_connection() -> crate::cli::LazyConnection<
    crate::client::local::LocalSocketClient,
    impl Fn() -> Result<(uuid::Uuid, crate::client::local::LocalSocketClient), crate::cli::RunError>,
> {
    crate::cli::LazyConnection::new(|| panic!("a pane-less caller must not connect"))
}

#[test]
fn codex_home_alone_is_not_agent_evidence() {
    use crate::cli::agent_env_marker;
    assert_eq!(
        agent_env_marker([("CLAUDECODE", "1")]),
        Some("CLAUDECODE".to_owned())
    );
    assert_eq!(
        agent_env_marker([("PATH", "/bin"), ("CODEX_HOME", "/x")]),
        None
    );
    assert_eq!(
        agent_env_marker([("CODEX_API_KEY", "k"), ("CODEXX", "1")]),
        None
    );
    assert_eq!(
        agent_env_marker([("XCLAUDECODE", "1"), ("CLAUDE_CODE", "1")]),
        None
    );
    // The selection proceeds with CODEX_HOME alone.
    assert!(select(&[("CODEX_HOME", "/x")], Ok(None), false).is_ok());
}

#[test]
fn codex_sandbox_marker_is_agent_evidence() {
    use crate::cli::agent_env_marker;
    for name in ["CODEX_SANDBOX", "CODEX_SANDBOX_NETWORK_DISABLED"] {
        assert_eq!(agent_env_marker([(name, "1")]), Some(name.to_owned()));
        let detail = refusal_detail(select(&[(name, "1")], Ok(None), false).unwrap_err());
        assert!(detail.contains(name), "{detail}");
    }
}

#[test]
fn herdr_read_error_is_not_agent_evidence() {
    let failed = crate::protocol::results::ApiError::unsupported("herdr down");
    assert_eq!(
        crate::cli::agent_evidence::<&str, &str>([], || Err(failed)),
        None
    );
    assert_eq!(
        crate::cli::agent_evidence::<&str, &str>([], || Ok(Some(observed(Some("claude"))))),
        Some("Herdr reports a `claude` agent in this pane".to_owned())
    );
    assert_eq!(
        crate::cli::agent_evidence::<&str, &str>([], || Ok(Some(observed(Some("shell"))))),
        None
    );
}

#[test]
fn operator_marked_human_context_is_selected_despite_agent_evidence() {
    let marker = [("CLAUDECODE", "1")];
    assert!(select(&marker, Ok(None), true).is_ok());
    assert!(select(&[], Ok(Some(observed(Some("codex")))), true).is_ok());
    // Negative: without the recorded mark the same evidence refuses.
    assert!(select(&marker, Ok(None), false).is_err());
    assert!(select(&[], Ok(Some(observed(Some("codex")))), false).is_err());
}

fn human_context(pane: &str) -> crate::harness::context::OccupantContext {
    use crate::harness::context::{Harness as H, OccupantContext, Role, SessionReference};
    OccupantContext {
        format_version: 1,
        instance: uuid::Uuid::from_u128(1),
        seat: "seat-1".into(),
        target: pane.into(),
        harness: H::Human,
        binding_generation: 1,
        execution: uuid::Uuid::from_u128(2),
        session: SessionReference::PluginContext(uuid::Uuid::from_u128(2)),
        role: Role::TopLevel,
    }
}
fn observed(kind: Option<&str>) -> crate::ports::PaneAgentObservation {
    crate::ports::PaneAgentObservation {
        kind: kind.map(str::to_owned),
        agent_session: None,
    }
}
fn select(
    env: &[(&str, &str)],
    agent: Result<Option<crate::ports::PaneAgentObservation>, crate::protocol::results::ApiError>,
    operator_override: bool,
) -> Result<crate::cli::commands::CooperativeSelection, crate::cli::RunError> {
    crate::cli::selection_from_context(
        HostTargetId::new("w:p1"),
        SeatId::new("seat-1"),
        &human_context("w:p1"),
        operator_override,
        env.iter().copied(),
        |_| agent,
    )
}
fn refusal_detail(error: crate::cli::RunError) -> String {
    match error {
        crate::cli::RunError::Api(api) => {
            assert_eq!(
                api.code,
                crate::protocol::results::ErrorCode::InvalidRequest
            );
            api.detail
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn derive_selection_refuses_human_context_with_agent_marker() {
    let detail = refusal_detail(select(&[("CLAUDECODE", "1")], Ok(None), false).unwrap_err());
    assert!(detail.contains("CLAUDECODE"), "{detail}");
    assert!(
        detail.contains("herdr-threads human me init --operator"),
        "{detail}"
    );
    assert!(select(&[("PATH", "/bin")], Ok(None), false).is_ok());
}

#[test]
fn derive_selection_refuses_human_context_when_herdr_reports_agent() {
    for kind in ["claude", "codex"] {
        let detail =
            refusal_detail(select(&[], Ok(Some(observed(Some(kind)))), false).unwrap_err());
        assert!(detail.contains(kind), "{detail}");
        assert!(
            detail.contains("herdr-threads human me init --operator"),
            "{detail}"
        );
    }
    // No agent, an unrelated detection, or a failed read never refuses.
    assert!(select(&[], Ok(Some(observed(None))), false).is_ok());
    assert!(select(&[], Ok(Some(observed(Some("shell")))), false).is_ok());
    let failed = crate::protocol::results::ApiError::unsupported("herdr down");
    assert!(select(&[], Err(failed), false).is_ok());
}

#[test]
fn agent_context_selection_ignores_agent_markers() {
    let mut context = human_context("w:p1");
    context.harness = crate::harness::context::Harness::Claude;
    let selected = crate::cli::selection_from_context(
        HostTargetId::new("w:p1"),
        SeatId::new("seat-1"),
        &context,
        false,
        [("CLAUDECODE", "1")],
        |_| panic!("an agent context never reads the pane"),
    )
    .unwrap();
    assert_eq!(selected.harness, crate::harness::context::Harness::Claude);
}

#[test]
fn cli_command_against_previous_protocol_descriptor_is_refused_before_send() {
    use crate::daemon::{
        ownership::OwnerLock,
        paths::{InstancePaths, RuntimeContext},
    };
    use crate::protocol::{results::ErrorCode, wire::PROTOCOL_VERSION};
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("herdr-cli-skew-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let context = RuntimeContext::explicit(root.clone(), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let lock = OwnerLock::acquire(&paths).unwrap();
    let listener = lock.bind_socket().unwrap();
    lock.publish_endpoint(&listener, "0.0.1", PROTOCOL_VERSION - 1)
        .unwrap();
    let clock: std::sync::Arc<dyn crate::protocol::time::Clock> =
        std::sync::Arc::new(crate::app::SystemClock::new());
    let error = match crate::cli::connect(&paths, &clock) {
        Err(crate::cli::RunError::Api(error)) => error,
        Err(_) => panic!("expected an API error"),
        Ok(_) => panic!("a previous-protocol descriptor must refuse before any client exists"),
    };
    assert_eq!(error.code, ErrorCode::UnknownWireVersion);
    // One VersionSkew remedy line (B3), naming both protocols.
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
    assert!(error.detail.contains("daemon stop"), "{}", error.detail);
    drop(listener);
    drop(lock);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn foreign_pane_inbox_never_requests_display_ack_caller() {
    let root = std::env::temp_dir().join(format!("ht-foreign-pane-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let parsed =
        crate::cli::commands::parse_argv(["ht", "inbox", "--human", "--pane", "w1:p2"]).unwrap();
    assert!(matches!(
        crate::cli::caller_need(&parsed, &paths).unwrap(),
        crate::cli::CallerNeed::None
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn recipient_panes_reads_do_not_allocate_and_invite_uses_guarded_resolution() {
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::{ApiError, ContinuityStatus, SeatSummary},
        time::Clock,
    };
    use std::sync::{Arc, Mutex};
    struct Client {
        calls: Mutex<Vec<Command>>,
        empty: bool,
    }
    impl crate::ports::LocalClient for Client {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.calls.lock().unwrap().push(command.clone());
            match command {
                Command::Seats(query) => {
                    assert_eq!(query.target, Some(HostTargetId::new("w1:p2")));
                    Ok(CommandResult::Seats(Page {
                        items: if self.empty {
                            vec![]
                        } else {
                            vec![SeatSummary {
                                seat: SeatId::new("recipient"),
                                continuity: ContinuityStatus::Resolved,
                                target: query.target,
                                generation: 1,
                                created_at: crate::protocol::time::UtcMillis(0),
                                retired_at: None,
                            }]
                        },
                        next_cursor: None,
                        next_argv: None,
                        high_water_ordinal: 0,
                        scope_revision: None,
                        has_more: false,
                        stop_reason: StopReason::Complete,
                        consistency: Consistency::BoundedLive,
                    }))
                }
                Command::ResolveSeat(query) => {
                    assert_eq!(query.target, HostTargetId::new("w1:p2"));
                    Ok(CommandResult::SeatResolved(SeatId::new("recipient")))
                }
                _ => panic!("unexpected recipient command: {command:?}"),
            }
        }
    }
    let root = std::env::temp_dir().join(format!("ht-pane-recipient-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(crate::cli::SystemClock::new());
    for argv in [
        vec!["ht", "inbox", "--pane", "w1:p2"],
        vec!["ht", "invite", "t123", "--pane", "w1:p2"],
        vec![
            "ht",
            "send",
            "t123",
            "--body",
            "work",
            "--require-ack",
            "recipient",
            "--require-ack-pane",
            "w1:p2",
            "--require-ack-pane",
            "w1:p2",
        ],
    ] {
        let is_read = argv[1] == "inbox";
        let connection = crate::cli::LazyConnection::new(|| {
            Ok((
                uuid::Uuid::from_u128(1),
                Client {
                    calls: Mutex::new(vec![]),
                    empty: !is_read,
                },
            ))
        });
        let mut parsed = crate::cli::commands::parse_argv(argv).unwrap();
        crate::cli::panes::resolve_cli_targets(
            &mut parsed,
            || panic!("direct IDs"),
            || panic!("direct IDs"),
        )
        .unwrap();
        crate::cli::resolve_recipient_seats(&mut parsed, &paths, &connection, &clock).unwrap();
        assert!(
            parsed.cooperative.is_none(),
            "recipient must not become caller"
        );
        let calls = connection.get().unwrap().1.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        if is_read {
            assert!(matches!(calls[0], Command::Seats(_)));
            assert!(
                matches!(parsed.action, crate::cli::commands::CliAction::Wire(Command::Inbox(ref q)) if q.seat == Some(SeatId::new("recipient")))
            );
            assert!(
                !paths.instance_dir.join("intents").exists(),
                "reads must not journal allocations"
            );
        } else {
            assert!(matches!(calls[0], Command::ResolveSeat(_)));
            match parsed.action {
                crate::cli::commands::CliAction::Mutation(
                    crate::cli::commands::MutationSpec::Invite { seat, .. },
                ) => assert_eq!(seat.as_str(), "recipient"),
                crate::cli::commands::CliAction::Mutation(
                    crate::cli::commands::MutationSpec::Send { require_ack, .. },
                ) => assert_eq!(require_ack, vec![SeatId::new("recipient")]),
                other => panic!("unexpected recipient action: {other:?}"),
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

/// Exercise public runtime composition against isolated host/daemon socket fixtures.
mod scoped_runtime {
    use super::*;
    use crate::{
        daemon::{
            ownership::OwnerLock,
            paths::{InstancePaths, RuntimeContext},
        },
        protocol::{
            pagination::{Consistency, Page, StopReason},
            results::{ApiError, ContinuityStatus, ErrorCode, SeatSummary},
            wire::{PROTOCOL_VERSION, WireRequest, WireResponse},
        },
    };
    use serde_json::{Value, json};
    use std::{
        fs,
        io::{BufRead, BufReader, Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::PathBuf,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    fn empty<T>() -> Page<T> {
        Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }
    }
    fn snapshot() -> Value {
        let p = |id: &str, tab: &str, label: &str| json!({"pane_id":id,"terminal_id":id,"workspace_id":"w1","tab_id":tab,"label":label,"focused":false,"agent_status":"idle","revision":1});
        json!({"version":"0.9.1","protocol":22,"layouts":[],"workspaces":[{"workspace_id":"w1","label":"project"}],
            "tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"main"},{"tab_id":"w1:t2","workspace_id":"w1","label":"tryout"}],
            "panes":[p("w1:p1","w1:t1","caller"),p("w1:p2","w1:t2","alice")],"agents":[]})
    }
    struct Runtime {
        root: PathBuf,
        paths: InstancePaths,
        _lock: OwnerLock,
        calls: Arc<Mutex<Vec<Command>>>,
        host_calls: Arc<Mutex<Vec<Value>>>,
        journals: Arc<Mutex<Vec<Value>>>,
        stop: Arc<AtomicBool>,
        workers: Vec<thread::JoinHandle<()>>,
    }
    impl Runtime {
        fn new(snapshot: Value) -> Self {
            let root = PathBuf::from("/tmp")
                .canonicalize()
                .unwrap()
                .join(format!("htsr-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let host = UnixListener::bind(root.join("host.sock")).unwrap();
            let runtime =
                RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
            let paths = InstancePaths::resolve(&runtime).unwrap();
            let lock = OwnerLock::acquire(&paths).unwrap();
            let daemon = lock.bind_socket().unwrap();
            let descriptor = lock
                .publish_endpoint(&daemon, env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION)
                .unwrap();
            let calls = Arc::new(Mutex::new(vec![]));
            let host_calls = Arc::new(Mutex::new(vec![]));
            let journals = Arc::new(Mutex::new(vec![]));
            let stop = Arc::new(AtomicBool::new(false));
            let host_stop = Arc::clone(&stop);
            let host_seen = Arc::clone(&host_calls);
            let host_worker = thread::spawn(move || {
                loop {
                    let (mut stream, _) = host.accept().unwrap();
                    if host_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut line = String::new();
                    BufReader::new(&mut stream).read_line(&mut line).unwrap();
                    let request: Value = serde_json::from_str(&line).unwrap();
                    host_seen.lock().unwrap().push(request.clone());
                    let result = match request["method"].as_str().unwrap() {
                        "ping" => json!({"type":"pong","version":"0.9.1","protocol":22}),
                        "session.snapshot" => {
                            json!({"type":"session_snapshot","snapshot":snapshot})
                        }
                        "pane.current" => {
                            assert!(request["params"]["caller_pane_id"].is_string());
                            json!({"type":"pane_current","pane":{"pane_id":"w1:p2","focused":false}})
                        }
                        other => panic!("unexpected host method {other}"),
                    };
                    writeln!(stream, "{}", json!({"id":request["id"],"result":result})).unwrap();
                }
            });
            let daemon_stop = Arc::clone(&stop);
            let seen = Arc::clone(&calls);
            let journal_seen = Arc::clone(&journals);
            let intents = paths.instance_dir.join("intents");
            let daemon_worker = thread::spawn(move || {
                loop {
                    let (mut stream, _) = daemon.accept().unwrap();
                    if daemon_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut length = [0; 4];
                    stream.read_exact(&mut length).unwrap();
                    let mut body = vec![0; u32::from_be_bytes(length) as usize];
                    stream.read_exact(&mut body).unwrap();
                    let request: WireRequest = serde_json::from_slice(&body).unwrap();
                    seen.lock().unwrap().push(request.command.clone());
                    for entry in fs::read_dir(&intents).into_iter().flatten().flatten() {
                        if entry.path().extension().is_some_and(|ext| ext == "intent") {
                            let text = fs::read_to_string(entry.path()).unwrap();
                            let (header, semantic) = text.split_once('\n').unwrap();
                            journal_seen.lock().unwrap().push(json!({"header":serde_json::from_str::<Value>(header).unwrap(),"semantic":serde_json::from_str::<Value>(semantic).unwrap()}));
                        }
                    }
                    let result = match &request.command {
                        Command::ResolveThread(query)
                            if matches!(
                                query.selector.as_str(),
                                "t123" | "review" | "team café"
                            ) =>
                        {
                            Ok(CommandResult::ThreadResolved(ThreadId::new("t123")))
                        }
                        Command::ThreadName(query) => Ok(CommandResult::ThreadName(
                            crate::protocol::results::ThreadNameResult {
                                thread: query.thread.clone(),
                                name: Some("team café".into()),
                            },
                        )),
                        Command::ResolveSeat(_) => {
                            Ok(CommandResult::SeatResolved(SeatId::new("recipient")))
                        }
                        Command::OperatorOrphanInvite(_) => {
                            Ok(CommandResult::OperatorInvited(InvitationId::new("i123")))
                        }
                        Command::OperatorRebind(_) | Command::OperatorFreshSeat(_) => {
                            Err(ApiError::not_found("fixture refuses repair"))
                        }
                        Command::Seats(query) => Ok(CommandResult::Seats(Page {
                            items: vec![SeatSummary {
                                seat: SeatId::new("recipient"),
                                target: Some(HostTargetId::new("w1:p2")),
                                continuity: ContinuityStatus::Resolved,
                                generation: 1,
                                created_at: crate::protocol::time::UtcMillis(0),
                                retired_at: None,
                            }]
                            .into_iter()
                            .filter(|seat| {
                                query
                                    .target
                                    .as_ref()
                                    .is_none_or(|target| seat.target.as_ref() == Some(target))
                            })
                            .collect(),
                            ..empty()
                        })),
                        Command::SeatInspect(_) => Ok(CommandResult::SeatInspect(
                            crate::protocol::results::SeatInspection {
                                summary: SeatSummary {
                                    seat: SeatId::new("recipient"),
                                    target: Some(HostTargetId::new("w1:p2")),
                                    continuity: ContinuityStatus::Resolved,
                                    generation: 1,
                                    created_at: crate::protocol::time::UtcMillis(0),
                                    retired_at: None,
                                },
                                mapping: crate::protocol::results::MappingStatus {
                                    state: ContinuityStatus::Resolved,
                                    target: Some(HostTargetId::new("w1:p2")),
                                    detail_argv: None,
                                },
                                hold: None,
                                retirement: None,
                                open_binding: None,
                                history: empty(),
                            },
                        )),
                        Command::Capabilities => Ok(CommandResult::Capabilities(
                            crate::protocol::results::CapabilityList {
                                capabilities: vec![
                                    crate::protocol::capabilities::INBOX_BATCH.into(),
                                ],
                            },
                        )),
                        Command::InboxBatch(_) => Ok(CommandResult::InboxBatch(Page {
                            items: vec![crate::protocol::results::InboxBatchItem::Message {
                                thread: ThreadId::new("t123"),
                                topic_data: "topic".into(),
                                message: MessageId::new("m123"),
                                sequence: 1,
                                sender: Some(SeatId::new("sender")),
                                author_role: None,
                                relays_user: false,
                                user_intent: None,
                                author_role_backfilled: false,
                                body: "hello".into(),
                                body_start: 0,
                                body_end: 5,
                                body_len: 5,
                                ack_candidate: Some(MessageId::new("m123")),
                            }],
                            ..empty()
                        })),
                        Command::Ack(q) | Command::AckDisplayed(q) => Ok(
                            CommandResult::Acknowledged(crate::protocol::results::AckResult {
                                acknowledged: q.messages.clone(),
                                already_acknowledged: vec![],
                            }),
                        ),
                        Command::Directory(_) => Ok(CommandResult::Directory(empty())),
                        Command::Warnings(_) => Ok(CommandResult::Warnings(empty())),
                        Command::Diagnostics(_) => Ok(CommandResult::Diagnostics(empty())),
                        Command::ActiveWarnings(_) => Ok(CommandResult::ActiveWarnings(empty())),
                        Command::Inbox(_) => Ok(CommandResult::Inbox(empty())),
                        Command::PendingReceipts(_) => Ok(CommandResult::PendingReceipts(empty())),
                        other => Err(ApiError::invalid_request(format!(
                            "unexpected fixture daemon command {other:?}"
                        ))),
                    };
                    let response = WireResponse {
                        version: PROTOCOL_VERSION,
                        request_id: request.request_id,
                        instance: descriptor.instance_uuid.to_string(),
                        daemon_boot: descriptor.boot_id.to_string(),
                        result,
                    };
                    let body = serde_json::to_vec(&response).unwrap();
                    stream
                        .write_all(&(body.len() as u32).to_be_bytes())
                        .unwrap();
                    stream.write_all(&body).unwrap();
                }
            });
            Self {
                root,
                paths,
                _lock: lock,
                calls,
                host_calls,
                journals,
                stop,
                workers: vec![host_worker, daemon_worker],
            }
        }
        fn install_context(&self, target: &str) {
            self.install_context_harness(target, crate::harness::context::Harness::Codex);
        }
        fn install_context_harness(&self, target: &str, harness: crate::harness::context::Harness) {
            let execution = uuid::Uuid::new_v4();
            crate::cli::seat_contexts(
                &self.paths,
                self._lock.instance_uuid(),
                &SeatId::new("recipient"),
            )
            .unwrap()
            .install_reattached(crate::harness::context::OccupantContext {
                format_version: 1,
                instance: self._lock.instance_uuid(),
                seat: "recipient".into(),
                target: target.into(),
                harness,
                binding_generation: 1,
                execution,
                session: crate::harness::context::SessionReference::PluginContext(execution),
                role: crate::harness::context::Role::TopLevel,
            })
            .unwrap();
        }
        fn run(&self, args: &[&str]) -> Result<(), crate::cli::RunError> {
            self.run_mode(args, true)
        }
        fn run_mode(&self, args: &[&str], json: bool) -> Result<(), crate::cli::RunError> {
            self.run_mode_in_pane(args, json, "w1:p99")
        }
        fn run_mode_in_pane(
            &self,
            args: &[&str],
            json: bool,
            caller: &str,
        ) -> Result<(), crate::cli::RunError> {
            let state = self.root.join("state");
            let host = self.root.join("host.sock");
            let (human, args) = if args.first() == Some(&"human") {
                (true, &args[1..])
            } else {
                (false, args)
            };
            let mut argv = vec!["ht".to_owned()];
            if human {
                argv.push("human".into());
            }
            argv.extend([
                "--state-dir".into(),
                state.to_str().unwrap().into(),
                "--host-endpoint".into(),
                host.to_str().unwrap().into(),
            ]);
            if json {
                argv.push("--json".into());
            }
            let args = if args.first() == Some(&"human") {
                argv.insert(1, "human".into());
                &args[1..]
            } else {
                args
            };
            argv.extend(args.iter().map(|value| value.to_string()));
            crate::cli::run_in_pane(argv, Some(caller), &mut Vec::new())
        }
    }
    #[test]
    fn actor_boundary_root_human_refuses_before_retirement() {
        let runtime = Runtime::new(snapshot());
        runtime.install_context_harness("w1:p2", crate::harness::context::Harness::Human);
        let contexts = crate::cli::seat_contexts(
            &runtime.paths,
            runtime._lock.instance_uuid(),
            &SeatId::new("recipient"),
        )
        .unwrap();
        let before = serde_json::to_vec(&contexts.current().unwrap()).unwrap();
        let error = runtime
            .run_mode_in_pane(
                &["check-in", "--lifecycle-event", "human-root"],
                false,
                "w1:p2",
            )
            .unwrap_err();
        assert!(
            matches!(error, crate::cli::RunError::Api(ref e) if e.code == crate::protocol::results::ErrorCode::InvalidRequest && e.detail.contains("herdr-threads human")),
            "{error:?}"
        );
        assert_eq!(
            serde_json::to_vec(&contexts.current().unwrap()).unwrap(),
            before
        );
        assert!(!runtime.paths.instance_dir.join("intents").exists());
        assert!(
            runtime
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|call| matches!(call, Command::Seats(_)))
        );
        assert!(runtime.host_calls.lock().unwrap().is_empty());
    }

    impl Drop for Runtime {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = UnixStream::connect(self.root.join("host.sock"));
            let _ = UnixStream::connect(&self.paths.socket_path);
            let joined = self
                .workers
                .drain(..)
                .map(|worker| worker.join().is_ok())
                .collect::<Vec<_>>();
            fs::remove_dir_all(&self.root).unwrap();
            assert!(
                joined.iter().all(|success| *success) || thread::panicking(),
                "runtime fixture worker failed"
            );
        }
    }

    #[test]
    fn operator_pane_invite_runtime_freezes_guarded_seat_and_operator_journal() {
        let runtime = Runtime::new(snapshot());
        runtime
            .run(&[
                "human",
                "invite",
                "t123",
                "--tab",
                "tryout",
                "--pane",
                "alice",
                "--operator",
            ])
            .unwrap();
        let calls = runtime.calls.lock().unwrap();
        assert!(matches!(calls.first(),Some(Command::ResolveThread(q)) if q.selector == "t123"));
        assert!(
            matches!(calls.get(1),Some(Command::ResolveSeat(q)) if q.target.as_str() == "w1:p2")
        );
        assert!(
            matches!(calls.last(), Some(Command::OperatorOrphanInvite(q)) if q.seat.as_str() == "recipient")
        );
        let journals = runtime.journals.lock().unwrap();
        assert!(
            journals
                .iter()
                .any(|entry| entry["semantic"]["kind"] == "resolve_seat"
                    && entry["header"]["scope"]["kind"] == "service_allocation"
                    && entry["semantic"]["target"] == "w1:p2")
        );
        assert!(journals.iter().any(
            |entry| entry["semantic"]["kind"] == "operator_orphan_invite"
                && entry["header"]["scope"]["kind"] == "operator"
                && entry["header"]["scope"]["local_user_uid"]
                    == json!(crate::daemon::paths::effective_uid())
        ));
        assert!(journals.iter().any(
            |entry| entry["semantic"]["kind"] == "operator_orphan_invite"
                && entry["semantic"]["seat"] == "recipient"
        ));
        assert!(
            !journals
                .iter()
                .any(|entry| entry.to_string().contains("pending-pane-selector"))
        );
    }

    #[test]
    fn omitted_runtime_reads_use_canonical_caller_mapping_without_host_or_allocation() {
        for args in [
            vec!["thread", "list"],
            vec!["warnings"],
            vec!["diagnostics"],
            vec!["inbox"],
            vec!["pending-receipts"],
        ] {
            let runtime = Runtime::new(snapshot());
            runtime.run_mode_in_pane(&args, true, "w1:p2").unwrap();
            let calls = runtime.calls.lock().unwrap();
            assert!(
                matches!(calls.first(), Some(Command::Seats(q)) if q.target.as_ref().is_some_and(|target| target.as_str() == "w1:p2")),
                "{args:?}: {calls:?}"
            );
            assert!(
                matches!(calls.last(), Some(Command::Directory(q)) if q.membership.as_ref().is_some_and(|seat| seat.as_str() == "recipient"))
                    || matches!(calls.last(), Some(Command::Warnings(q)) if q.seat.as_str() == "recipient")
                    || matches!(calls.last(), Some(Command::Diagnostics(q)) if q.seat.as_ref().is_some_and(|seat| seat.as_str() == "recipient"))
                    || matches!(calls.last(), Some(Command::Inbox(q)) if q.seat.as_ref().is_some_and(|seat| seat.as_str() == "recipient"))
                    || matches!(calls.last(), Some(Command::PendingReceipts(q)) if q.seat.as_ref().is_some_and(|seat| seat.as_str() == "recipient")),
                "{args:?}: {calls:?}"
            );
            assert!(
                !calls
                    .iter()
                    .any(|command| matches!(command, Command::ResolveSeat(_)))
            );
            assert!(!runtime.paths.instance_dir.join("intents").exists());
            assert!(runtime.host_calls.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn absent_qualified_runtime_ids_never_retarget_labels_or_agents() {
        for level in ["pane", "agent", "tab"] {
            let mut host = snapshot();
            let args = match level {
                "pane" => {
                    host["panes"][1]["label"] = json!("w1:p999");
                    vec![
                        "human",
                        "seat",
                        "rebind",
                        "s123",
                        "--tab",
                        "w1:t2",
                        "--pane",
                        "w1:p999",
                        "--operator",
                    ]
                }
                "agent" => {
                    host["agents"] = json!([{"pane_id":"w1:p2","name":"w1:p999"}]);
                    vec![
                        "human",
                        "invite",
                        "t123",
                        "--tab",
                        "w1:t2",
                        "--pane",
                        "w1:p999",
                        "--operator",
                    ]
                }
                _ => {
                    host["tabs"][1]["label"] = json!("w1:t999");
                    vec!["inbox", "--tab", "w1:t999", "--pane", "alice"]
                }
            };
            let runtime = Runtime::new(host);
            assert!(
                matches!(runtime.run(&args), Err(crate::cli::RunError::Api(error)) if error.code == ErrorCode::NotFound),
                "{level}"
            );
            assert!(
                runtime.calls.lock().unwrap().is_empty(),
                "{level} retargeted a daemon command"
            );
        }
    }
    #[test]
    fn bounded_runtime_conflict_reports_total_and_omitted_candidates() {
        let mut host = snapshot();
        let template = host["panes"][1].clone();
        for index in 3..=11 {
            let mut pane = template.clone();
            pane["pane_id"] = json!(format!("w1:p{index}"));
            pane["terminal_id"] = json!(format!("term{index}"));
            host["panes"].as_array_mut().unwrap().push(pane);
        }
        let runtime = Runtime::new(host);
        match runtime.run(&["inbox", "--tab", "w1:t2", "--pane", "alice"]) {
            Err(crate::cli::RunError::Api(error)) => {
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.detail.contains("10 matches"), "{}", error.detail);
                assert!(error.detail.contains("2 omitted"), "{}", error.detail);
            }
            other => panic!("{other:?}"),
        }
        assert!(runtime.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn thread_names_runtime_freezes_utf8_before_read_and_operator_intent() {
        let runtime = Runtime::new(snapshot());
        runtime.run(&["thread", "name", "team café"]).unwrap();
        {
            let calls = runtime.calls.lock().unwrap();
            assert!(
                matches!(calls.first(),Some(Command::ResolveThread(q)) if q.selector=="team café")
            );
            assert!(
                matches!(calls.last(),Some(Command::ThreadName(q)) if q.thread.as_str()=="t123")
            );
            assert!(runtime.journals.lock().unwrap().is_empty());
        }
        runtime
            .run(&[
                "human",
                "invite",
                "review",
                "--tab",
                "tryout",
                "--pane",
                "alice",
                "--operator",
            ])
            .unwrap();
        let calls = runtime.calls.lock().unwrap();
        assert!(
            matches!(calls.last(),Some(Command::OperatorOrphanInvite(q)) if q.thread.as_str()=="t123" && q.seat.as_str()=="recipient")
        );
        let journals = runtime.journals.lock().unwrap();
        assert!(journals.iter().any(
            |entry| entry["semantic"]["kind"] == "operator_orphan_invite"
                && entry["semantic"]["thread"] == "t123"
        ));
        assert!(
            !journals
                .iter()
                .any(|entry| entry["semantic"]["thread"] == "review")
        );
    }

    // Kills composing name lookup with an inherited pane instead of the explicit caller.
    #[test]
    fn thread_names_runtime_cooperative_caller_overrides_inherited_pane() {
        let runtime = Runtime::new(snapshot());
        runtime
            .run(&[
                "--cooperative-seat",
                "recipient",
                "--cooperative-target",
                "w1:p2",
                "--cooperative-harness",
                "codex",
                "--cooperative-role",
                "top-level",
                "thread",
                "name",
                "team café",
            ])
            .unwrap();
        let calls = runtime.calls.lock().unwrap();
        assert!(
            matches!(calls.first(), Some(Command::ResolveThread(q))
            if q.caller == Some(SeatId::new("recipient"))
                && q.caller_target == Some(HostTargetId::new("w1:p2"))),
            "{calls:?}"
        );
        assert!(
            matches!(calls.last(), Some(Command::ThreadName(q)) if q.thread == ThreadId::new("t123"))
        );
    }

    // Kills treating a foreign read scope as the joined-name caller.
    #[test]
    fn thread_names_runtime_foreign_read_scope_keeps_actual_caller() {
        for args in [
            vec![
                "pending-receipts",
                "--thread",
                "team café",
                "--seat",
                "foreign",
            ],
            vec![
                "pending-receipts",
                "--thread",
                "team café",
                "--pane",
                "w1:p2",
            ],
        ] {
            let runtime = Runtime::new(snapshot());
            runtime.run_mode_in_pane(&args, true, "w1:p99").unwrap();
            let calls = runtime.calls.lock().unwrap();
            assert!(
                matches!(calls.first(), Some(Command::ResolveThread(q))
                if q.caller.is_none() && q.caller_target == Some(HostTargetId::new("w1:p99"))),
                "{calls:?}"
            );
            assert!(
                matches!(calls.last(), Some(Command::PendingReceipts(q))
                if q.thread == Some(ThreadId::new("t123"))
                    && q.seat.as_ref().is_some_and(|seat| matches!(seat.as_str(), "foreign" | "recipient"))),
                "{calls:?}"
            );
        }
    }

    #[test]
    fn deliberately_scoped_runtime_reads_preserve_their_filters() {
        for args in [
            vec!["thread", "list", "--all"],
            vec!["warnings", "--active", "t123"],
            vec!["diagnostics", "--thread", "t123"],
            vec!["pending-receipts", "--thread", "t123"],
            vec!["inbox", "--seat", "recipient"],
        ] {
            let runtime = Runtime::new(snapshot());
            runtime.run(&args).unwrap();
            let calls = runtime.calls.lock().unwrap();
            let has_thread = args.contains(&"--thread") || args.contains(&"--active");
            assert_eq!(
                calls.len(),
                if has_thread || args[0] == "inbox" {
                    2
                } else {
                    1
                },
                "{args:?}: {calls:?}"
            );
            if args[0] == "inbox" {
                assert!(matches!(calls.first(), Some(Command::Capabilities)));
                assert!(
                    matches!(calls.last(), Some(Command::Inbox(q)) if q.seat == Some(SeatId::new("recipient")) && q.page.cursor.is_none() && q.page.limit == 20)
                );
            }
            if has_thread {
                assert!(
                    matches!(calls.first(),Some(Command::ResolveThread(q)) if q.selector=="t123")
                );
            }
            assert!(
                runtime.host_calls.lock().unwrap().is_empty(),
                "{args:?} unnecessarily selects caller"
            );
        }
    }

    #[test]
    fn runtime_workspace_ids_win_and_ordinary_workspace_labels_remain_valid() {
        for name in ["w1", "work"] {
            let mut host = snapshot();
            host["workspaces"][0]["label"] = json!("work");
            host["workspaces"]
                .as_array_mut()
                .unwrap()
                .push(json!({"workspace_id":"w2","label":"w1"}));
            let runtime = Runtime::new(host);
            runtime
                .run(&[
                    "inbox", "--space", name, "--tab", "tryout", "--pane", "alice",
                ])
                .unwrap();
            assert!(
                matches!(runtime.calls.lock().unwrap().first(), Some(Command::Seats(q)) if q.target == Some(HostTargetId::new("w1:p2")))
            );
        }
    }
    #[test]
    fn own_canonical_inbox_runtime_remains_display_ack_eligible_and_foreign_inbox_does_not() {
        for explicit in [false, true] {
            let runtime = Runtime::new(snapshot());
            runtime.install_context("w1:p2");
            let args = if explicit {
                vec!["inbox", "--human", "--pane", "w1:p2"]
            } else {
                vec!["inbox", "--human"]
            };
            let caller = if explicit { "w1:p1" } else { "w1:p2" };
            runtime.run_mode_in_pane(&args, false, caller).unwrap();
            assert!(runtime.host_calls.lock().unwrap().is_empty());
            let calls = runtime.calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .any(|command| matches!(command, Command::InboxBatch(_))),
                !explicit
            );
            assert_eq!(calls.iter().any(|command| matches!(command, Command::AckDisplayed(q) if q.claim.seat.as_str() == "recipient" && q.messages == vec![MessageId::new("m123")])), !explicit);
            assert!(
                !calls
                    .iter()
                    .any(|command| matches!(command, Command::ResolveSeat(_)))
            );
        }
    }

    #[test]
    fn stale_local_context_is_refused_after_canonical_inbox_mapping_and_never_acks() {
        let runtime = Runtime::new(snapshot());
        runtime.install_context("w1:p99");
        let error = runtime
            .run_mode_in_pane(&["inbox", "--human"], false, "w1:p2")
            .unwrap_err();
        assert!(matches!(error, crate::cli::RunError::Api(ref error)
            if error.code == ErrorCode::TargetUnresolved && error.detail == "local context differs from current service mapping"));
        assert!(runtime.host_calls.lock().unwrap().is_empty());
        let calls = runtime.calls.lock().unwrap();
        assert!(
            matches!(calls.first(), Some(Command::Seats(q)) if q.target == Some(HostTargetId::new("w1:p2")))
        );
        assert!(!calls.iter().any(|command| matches!(
            command,
            Command::Ack(_)
                | Command::AckDisplayed(_)
                | Command::ResolveSeat(_)
                | Command::InboxBatch(_)
        )));
    }

    #[test]
    fn unmapped_canonical_caller_is_refused_without_host_retarget_or_ack() {
        let runtime = Runtime::new(snapshot());
        runtime.install_context("w1:p2");
        let error = runtime.run_mode(&["inbox", "--human"], false).unwrap_err();
        assert!(
            matches!(error, crate::cli::RunError::Api(ref error) if error.code == ErrorCode::InvalidRequest)
        );
        let calls = runtime.calls.lock().unwrap();
        assert!(
            matches!(calls.as_slice(), [Command::Seats(q)] if q.target == Some(HostTargetId::new("w1:p99")))
        );
        assert!(runtime.host_calls.lock().unwrap().is_empty());
        assert!(!runtime.paths.instance_dir.join("intents").exists());
    }

    #[test]
    fn explicit_cooperative_runtime_read_default_uses_claimed_seat_without_host_scope() {
        for args in [
            vec!["thread", "list"],
            vec!["warnings"],
            vec!["diagnostics"],
            vec!["inbox"],
            vec!["pending-receipts"],
        ] {
            let runtime = Runtime::new(snapshot());
            runtime.install_context("w1:p2");
            let mut selected = vec![
                "--cooperative-seat",
                "recipient",
                "--cooperative-target",
                "w1:p2",
                "--cooperative-harness",
                "codex",
                "--cooperative-role",
                "top-level",
            ];
            let inbox = args[0] == "inbox";
            selected.extend(args);
            runtime.run(&selected).unwrap();
            assert!(runtime.host_calls.lock().unwrap().is_empty());
            let calls = runtime.calls.lock().unwrap();
            assert_eq!(calls.len(), if inbox { 3 } else { 2 }, "{calls:?}");
            if inbox {
                assert!(matches!(calls.get(1), Some(Command::Capabilities)));
                assert!(
                    matches!(calls.last(), Some(Command::Inbox(q)) if q.seat == Some(SeatId::new("recipient")) && q.page.cursor.is_none() && q.page.limit == 20)
                );
            }
            assert!(matches!(calls.first(), Some(Command::SeatInspect(_))));
            assert!(
                !calls
                    .iter()
                    .any(|command| matches!(command, Command::ResolveSeat(_) | Command::Seats(_)))
            );
        }
    }

    #[test]
    fn direct_runtime_pane_ids_stay_direct_and_qualified_parent_mismatch_is_refused() {
        let runtime = Runtime::new(snapshot());
        runtime
            .run(&["seat", "resolve", "--pane", "w9:p999"])
            .unwrap();
        assert!(runtime.host_calls.lock().unwrap().is_empty());
        assert!(
            matches!(runtime.calls.lock().unwrap().last(), Some(Command::ResolveSeat(q)) if q.target.as_str() == "w9:p999")
        );
        runtime.calls.lock().unwrap().clear();
        assert!(
            runtime
                .run(&[
                    "seat",
                    "rebind",
                    "s123",
                    "--tab",
                    "w1:t2",
                    "--pane",
                    "w1:p1",
                    "--operator"
                ])
                .is_err()
        );
        assert!(runtime.calls.lock().unwrap().is_empty());
    }
}

// Missing original-actor preflight reaches context selection instead of refusing
// the honest frozen Human/operator origin, and may consume completed intents.
#[test]
fn actor_prerequisite_human_and_operator_retry_refuses_before_context_or_transport() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("actor-preflight-{}", uuid::Uuid::new_v4()));
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let contexts = crate::harness::context::ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    struct Never;
    impl crate::ports::LocalClient for Never {
        crate::default_output_local_client!();
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("Human/operator retry reached transport");
        }
    }
    let mut human = claim();
    human.harness = Harness::Human;
    let cases = [
        (
            IntentScope::Cooperative {
                instance: human.instance.clone(),
                seat: human.seat.clone(),
            },
            SemanticMutation::freeze(
                SemanticMutation::Ack {
                    messages: vec![MessageId::new("message")],
                },
                human,
            )
            .unwrap(),
        ),
        (
            IntentScope::Operator {
                instance: "instance".into(),
                local_user_uid: 1,
            },
            SemanticMutation::OperatorRetire {
                seat: SeatId::new("seat"),
            },
        ),
    ];
    for (scope, semantic) in cases {
        let reference = journal.record(scope, semantic, 1).unwrap();
        let before = journal.load(&reference).unwrap();
        let parsed = crate::cli::commands::parse_argv([
            "ht",
            "--state-dir",
            "state space",
            "--host-endpoint",
            "host space",
            "retry",
            &reference.recovery_ref(),
        ])
        .unwrap();
        let mut output = Vec::new();
        let failure = crate::cli::run_cooperative(
            parsed,
            &journal,
            &contexts,
            None,
            crate::harness::context::Role::TopLevel,
            &Never,
            &crate::app::SystemClock::new(),
            &mut output,
        )
        .unwrap_err();
        assert!(
            format!("{failure:?}")
                .contains("person/operator retry requires immediate human namespace"),
            "{failure:?}"
        );
        assert!(
            format!("{failure:?}")
                .contains("human --state-dir 'state space' --host-endpoint 'host space' retry"),
            "{failure:?}"
        );
        assert!(output.is_empty());
        assert_eq!(journal.load(&reference).unwrap().semantic, before.semantic);
        assert_eq!(
            journal.load(&reference).unwrap().header.semantic_digest,
            before.header.semantic_digest
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_prerequisite_agent_fresh_action_refuses_selected_human_context() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("actor-fresh-{}", uuid::Uuid::new_v4()));
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let contexts = crate::harness::context::ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat-1",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    struct Never;
    impl crate::ports::LocalClient for Never {
        crate::default_output_local_client!();
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("fresh Agent action reached transport through Human context");
        }
    }
    let parsed = crate::cli::commands::parse_argv(["ht", "ack", "message"]).unwrap();
    let mut output = Vec::new();
    let failure = crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        Some(&human_context("pane")),
        crate::harness::context::Role::TopLevel,
        &Never,
        &crate::app::SystemClock::new(),
        &mut output,
    )
    .unwrap_err();
    assert!(
        format!("{failure:?}").contains("this command selects a Human context"),
        "{failure:?}"
    );
    assert!(output.is_empty());
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_prerequisite_all_unfrozen_handoffs_refuse_before_dispatch() {
    use crate::protocol::handoff::topology_contract_tests as fixture;
    let legacy: SemanticMutation = serde_json::from_value(serde_json::json!({"kind":"handoff","request":{"thread":"thread-1","thread_name":null,"topic":null,"goal":null,"body":"work","launch":{"target":"target-2","harness":"Codex","harness_binary":"/bin/codex","argv":[],"name":"peer","pane_label":null}},"context":{"state_dir":"/state","host":"/host.sock"},"recipient":"recipient","create_key":"create","invite_key":"invite","send_key":"send"})).unwrap();
    for semantic in [
        legacy,
        SemanticMutation::HandoffBootstrap(Box::new(crate::cli::journal::BootstrapPlan {
            version: 1,
            payload: fixture::payload(),
        })),
        SemanticMutation::HandoffDelivery(Box::new(crate::cli::journal::DeliveryPlan {
            version: 1,
            payload: fixture::payload().handoff,
            recipient: SeatId::new("peer"),
        })),
    ] {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("actor-unfrozen-{}", uuid::Uuid::new_v4()));
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let journal = Journal::open(root.join("intents")).unwrap();
        let reference = journal
            .record(
                IntentScope::Native {
                    instance: "i".into(),
                    seat: SeatId::new("sender"),
                },
                semantic,
                1,
            )
            .unwrap();
        let contexts = crate::harness::context::ContextJournal::open(
            &root,
            uuid::Uuid::from_u128(1),
            "sender",
            std::time::Duration::from_millis(20),
        )
        .unwrap();
        struct Never;
        impl crate::ports::LocalClient for Never {
            crate::default_output_local_client!();
            fn call(
                &self,
                _: Command,
                _: &crate::protocol::time::CallBudget,
            ) -> Result<CommandResult, crate::protocol::results::ApiError> {
                panic!("unfrozen handoff reached transport")
            }
        }
        let parsed =
            crate::cli::commands::parse_argv(["ht", "retry", &reference.recovery_ref()]).unwrap();
        let mut output = Vec::new();
        let error = crate::cli::run_cooperative(
            parsed,
            &journal,
            &contexts,
            None,
            crate::harness::context::Role::TopLevel,
            &Never,
            &crate::app::SystemClock::new(),
            &mut output,
        )
        .unwrap_err();
        assert!(
            format!("{error:?}").contains("handoff needs frozen caller"),
            "{error:?}"
        );
        assert!(output.is_empty());
        assert!(journal.load(&reference).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn actor_prerequisite_public_retry_refuses_before_daemon_state_or_connect() {
    use crate::daemon::paths::{InstancePaths, RuntimeContext};
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("actor-public-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let runtime =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&runtime).unwrap();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let mut human = claim();
    human.harness = Harness::Human;
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: human.instance.clone(),
                seat: human.seat.clone(),
            },
            SemanticMutation::freeze(
                SemanticMutation::Ack {
                    messages: vec![MessageId::new("message")],
                },
                human,
            )
            .unwrap(),
            1,
        )
        .unwrap();
    let snapshot = || {
        let mut rows: Vec<_> = std::fs::read_dir(journal.root())
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    let before = snapshot();
    let argv = vec![
        "ht".to_owned(),
        "--state-dir".into(),
        runtime.state_dir.display().to_string(),
        "--host-endpoint".into(),
        runtime.host_endpoint.display().to_string(),
        "retry".into(),
        reference.recovery_ref(),
    ];
    let mut output = Vec::new();
    let error = crate::cli::run_in_pane(argv, None, &mut output).unwrap_err();
    assert!(
        format!("{error:?}").contains("person/operator retry requires immediate human namespace"),
        "{error:?}"
    );
    assert_eq!(snapshot(), before);
    assert!(output.is_empty());
    assert!(!paths.descriptor_path.exists());
    assert!(!paths.database_path.exists());
    assert!(!paths.instance_dir.join("contexts").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_prerequisite_fresh_mutation_validates_consumed_current_not_initial_seed() {
    actor_current_ack_fixture(false, true);
}

#[test]
fn actor_prerequisite_human_ack_refuses_current_agent_despite_human_seed() {
    actor_current_ack_fixture(true, false);
}

#[test]
fn actor_prerequisite_agent_ack_uses_current_despite_unused_human_seed() {
    actor_current_ack_fixture(false, false);
}

#[test]
fn actor_prerequisite_human_ack_uses_current_despite_unused_agent_seed() {
    actor_current_ack_fixture(true, true);
}

fn actor_fixture_snapshot(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, (bool, Vec<u8>)> {
    fn visit(
        root: &std::path::Path,
        dir: &std::path::Path,
        saved: &mut std::collections::BTreeMap<std::path::PathBuf, (bool, Vec<u8>)>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let directory = path.is_dir();
            saved.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                (
                    directory,
                    if directory {
                        vec![]
                    } else {
                        std::fs::read(&path).unwrap()
                    },
                ),
            );
            if directory {
                visit(root, &path, saved);
            }
        }
    }
    let mut saved = Default::default();
    visit(root, root, &mut saved);
    saved
}

fn actor_current_ack_fixture(human_route: bool, human_current: bool) {
    use crate::harness::context::{Harness as ContextHarness, Role, SessionReference};
    let isolation = crate::test_support::isolation::TestIsolation::new("actor-current-ack");
    let root = isolation.state_root().canonicalize().unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let contexts = crate::harness::context::ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat-1",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let mut current = human_context("current-pane");
    if !human_current {
        current.harness = ContextHarness::Codex;
    }
    contexts.install_reattached(current.clone()).unwrap();
    let mut seed = current.clone();
    seed.instance = uuid::Uuid::from_u128(9);
    seed.seat = "unused-seat".into();
    seed.target = "unused-pane".into();
    seed.binding_generation = 99;
    seed.execution = uuid::Uuid::from_u128(3);
    seed.session = SessionReference::Native("unused-session".into());
    seed.harness = if human_current {
        ContextHarness::Codex
    } else {
        ContextHarness::Human
    };
    // Literal full claim for current, independent of bridge::caller_claim.
    let expected = CallerClaim {
        instance: "00000000-0000-0000-0000-000000000001".into(),
        seat: SeatId::new("seat-1"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: if human_current {
            Harness::Human
        } else {
            Harness::Codex
        },
        native_session: NativeSessionId::new("plugin_context:00000000-0000-0000-0000-000000000002"),
        execution: ExecutionId::new("00000000-0000-0000-0000-000000000002"),
        target: HostTargetId::new("current-pane"),
    };
    struct Client<'a> {
        journal: &'a Journal,
        expected: CallerClaim,
        calls: std::sync::Mutex<Vec<Command>>,
    }
    impl crate::ports::LocalClient for Client<'_> {
        crate::default_output_local_client!();
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            let Command::Ack(ack) = &command else {
                panic!("fresh ACK attempted an unrelated effect: {command:?}")
            };
            assert_eq!(ack.claim, self.expected);
            assert_eq!(ack.messages, vec![MessageId::new("message")]);
            let page = self.journal.page(&Default::default()).unwrap();
            assert_eq!(page.items.len(), 1, "submit only the one published ACK");
            let reference = self
                .journal
                .resolve_recovery_ref(page.items[0].recovery_ref.as_str())
                .unwrap();
            let pending = self.journal.load(&reference).unwrap();
            assert_eq!(reference.operation, ack.operation);
            assert_eq!(
                pending.header.scope,
                IntentScope::Cooperative {
                    instance: self.expected.instance.clone(),
                    seat: SeatId::new("seat-1"),
                }
            );
            let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
                panic!("ACK original was not frozen")
            };
            assert_eq!(claim, self.expected);
            assert!(
                matches!(*mutation, SemanticMutation::Ack { messages } if messages == vec![MessageId::new("message")])
            );
            self.calls.lock().unwrap().push(command);
            Ok(CommandResult::Acknowledged(AckResult {
                acknowledged: vec![MessageId::new("message")],
                already_acknowledged: vec![],
            }))
        }
    }
    let client = Client {
        journal: &journal,
        expected,
        calls: Default::default(),
    };
    let parsed = if human_route {
        crate::cli::commands::parse_argv(["ht", "human", "ack", "message"]).unwrap()
    } else {
        crate::cli::commands::parse_argv(["ht", "ack", "message"]).unwrap()
    };
    let before = actor_fixture_snapshot(&root);
    let mut output = Vec::new();
    let outcome = crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        Some(&seed),
        Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut output,
    );
    if human_route == human_current {
        assert!(
            outcome.is_ok(),
            "unused opposite-harness initial vetoed current: {outcome:?}"
        );
        assert_eq!(
            client.calls.lock().unwrap().len(),
            1,
            "one typed submission"
        );
        assert!(String::from_utf8(output).unwrap().contains("message"));
    } else {
        let error = outcome.unwrap_err();
        let expected_detail = if human_route {
            "human namespace cannot act through an agent cooperative selection"
        } else {
            "this command selects a Human context"
        };
        assert!(format!("{error:?}").contains(expected_detail), "{error:?}");
        assert!(client.calls.lock().unwrap().is_empty());
        assert!(output.is_empty());
        assert_eq!(actor_fixture_snapshot(&root), before);
    }
    assert_eq!(contexts.current().unwrap().unwrap(), current);
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
}

#[test]
fn actor_prerequisite_false_default_plain_inbox_reads_without_acknowledging_human_context() {
    use crate::protocol::pagination::{Consistency, Page, StopReason};
    let isolation = crate::test_support::isolation::TestIsolation::new("actor-plain-inbox");
    let root = isolation.state_root().canonicalize().unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let contexts = crate::harness::context::ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat-1",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let human = human_context("current-pane");
    contexts.install_reattached(human.clone()).unwrap();
    struct ReadClient(std::sync::Mutex<Vec<Command>>);
    impl crate::ports::LocalClient for ReadClient {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            assert!(
                matches!(&command, Command::Inbox(query) if query.seat.is_none()),
                "plain read attempted an accountable/display action: {command:?}"
            );
            self.0.lock().unwrap().push(command);
            Ok(CommandResult::Inbox(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: StopReason::Complete,
                consistency: Consistency::BoundedLive,
            }))
        }
    }
    // This parsed public seam shape is non-default and read-only. Normal CLI
    // resolves its explicit recipient to Some(seat) before run_cooperative;
    // this control does not claim to test pane-selector resolution.
    let parsed = crate::cli::commands::parse_argv(["ht", "inbox", "--pane", "w1:p2"]).unwrap();
    assert!(!parsed.caller_read_default);
    assert!(
        matches!(&parsed.action, crate::cli::commands::CliAction::Wire(Command::Inbox(query)) if query.seat.is_none())
    );
    let client = ReadClient(Default::default());
    let before = actor_fixture_snapshot(&root);
    let mut output = Vec::new();
    let outcome = crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        None,
        crate::harness::context::Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut output,
    );
    assert!(
        outcome.is_ok(),
        "plain non-default inbox was incorrectly accountable: {outcome:?}"
    );
    assert_eq!(client.0.lock().unwrap().len(), 1);
    assert_eq!(contexts.current().unwrap().unwrap(), human);
    assert_eq!(actor_fixture_snapshot(&root), before);
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
}

#[test]
fn actor_prerequisite_lifecycle_guard_uses_its_selected_initial_seed() {
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::{CheckInContextDisposition, CheckInResult},
    };
    fn empty<T>() -> Page<T> {
        Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }
    }
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("actor-lifecycle-{}", uuid::Uuid::new_v4()));
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let contexts = crate::harness::context::ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat-1",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let human = human_context("pane");
    contexts.install_reattached(human.clone()).unwrap();
    let mut seed = human.clone();
    seed.harness = crate::harness::context::Harness::Codex;
    struct Client(std::sync::atomic::AtomicUsize);
    impl crate::ports::LocalClient for Client {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            let Command::CheckIn(check) = command else {
                panic!("lifecycle attempted an unrelated command: {command:?}")
            };
            assert_eq!(check.claim.harness, Harness::Codex);
            assert_eq!(
                check.mode,
                crate::protocol::commands::CheckInMode::Lifecycle {
                    expected_binding_generation: 1
                }
            );
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut context = check.claim;
            context.binding_generation = 2;
            Ok(CommandResult::CheckedIn(CheckInResult {
                seat: context.seat.clone(),
                context,
                context_disposition: CheckInContextDisposition::Current,
                offered_through: None,
                warning_count: 0,
                warning_count_has_more: false,
                warnings: empty(),
                notices: Default::default(),
                inbox: empty(),
            }))
        }
    }
    let client = Client(std::sync::atomic::AtomicUsize::new(0));
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "human",
        "check-in",
        "--lifecycle-event",
        "human-through-agent",
    ])
    .unwrap();
    let error = crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        Some(&seed),
        crate::harness::context::Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        format!("{error:?}")
            .contains("human namespace cannot act through an agent cooperative selection"),
        "{error:?}"
    );
    assert_eq!(client.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(contexts.current().unwrap().unwrap(), human);
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
    // The real run_selected path retires this exact Human current context
    // after service mapping selection before seeding an Agent lifecycle. The
    // bridge's unchanged local harness guard refuses that transition otherwise.
    assert!(contexts.retire_current(&human).unwrap());
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "check-in",
        "--lifecycle-event",
        "agent-lifecycle",
    ])
    .unwrap();
    crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        Some(&seed),
        crate::harness::context::Role::TopLevel,
        &client,
        &crate::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(client.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        contexts.current().unwrap().unwrap().harness,
        crate::harness::context::Harness::Codex
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn actor_boundary_root_human_refuses_before_journal() {
    actor_boundary_selected(false);
}

#[test]
fn actor_boundary_human_explicit_agent_mismatch() {
    actor_boundary_selected(true);
}

fn actor_boundary_selected(human_route: bool) {
    use crate::cli::commands::CooperativeSelection;
    use crate::harness::context::{Harness, Role};
    struct NoCalls(std::sync::atomic::AtomicUsize);
    impl crate::ports::LocalClient for NoCalls {
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(command, budget)
        }
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(crate::protocol::results::ApiError::not_found(
                "boundary fixture",
            ))
        }
    }
    let root = std::env::temp_dir().join(format!("actor-boundary-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        root.join("state"),
        root.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
    let argv = if human_route {
        vec!["ht", "human", "check-in"]
    } else {
        vec!["ht", "check-in"]
    };
    let parsed = crate::cli::commands::parse_argv(argv).unwrap();
    let selection = CooperativeSelection {
        seat: SeatId::new("seat"),
        target: HostTargetId::new("w1:p1"),
        harness: if human_route {
            Harness::Codex
        } else {
            Harness::Human
        },
        role: Role::TopLevel,
    };
    let client = NoCalls(std::sync::atomic::AtomicUsize::new(0));
    let mut output = Vec::new();
    let error = crate::cli::run_selected(
        parsed,
        &selection,
        &paths,
        uuid::Uuid::from_u128(1),
        &client,
        &crate::app::SystemClock::new(),
        &mut output,
    )
    .unwrap_err();
    assert_eq!(
        client.0.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "actor refusal must precede service proof and mutation calls"
    );
    assert!(
        matches!(error, crate::cli::RunError::Api(ref e) if e.code == crate::protocol::results::ErrorCode::InvalidRequest),
        "{error:?}"
    );
    assert!(!paths.instance_dir.join("intents").exists());
    assert!(!paths.instance_dir.join("contexts").exists());
    assert!(output.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

// Kills: selected retry reaching SeatInspect or cleanup before original-actor
// preflight. A misleading current selection cannot turn a saved Human into Agent.
#[test]
fn lazy_selected_human_retry_root_refuses_before_client() {
    use crate::{
        cli::commands::CooperativeSelection,
        daemon::paths::{InstancePaths, RuntimeContext},
        harness::context::{Harness as ContextHarness, Role},
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(".tmp/ht-big.5.2")
        .join(format!("lazy-selected-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let runtime = RuntimeContext::explicit(root.join("state"), root.join("host"), None).unwrap();
    let paths = InstancePaths::resolve_read_only(&runtime).unwrap();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let mut saved = claim();
    saved.harness = Harness::Human;
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: saved.instance.clone(),
                seat: saved.seat.clone(),
            },
            SemanticMutation::freeze(
                SemanticMutation::CompleteInboxDelivery {
                    messages: vec![MessageId::new("lazy")],
                },
                saved.clone(),
            )
            .unwrap(),
            1,
        )
        .unwrap();
    struct NoClient;
    impl crate::ports::LocalClient for NoClient {
        fn call(
            &self,
            c: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("must refuse before client {c:?}")
        }
        fn call_with_output(
            &self,
            c: Command,
            _: &crate::protocol::output::OutputSpec,
            b: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.call(c, b)
        }
    }
    let selection = CooperativeSelection {
        seat: saved.seat,
        target: saved.target,
        harness: ContextHarness::Codex,
        role: Role::TopLevel,
    };
    let parsed = crate::cli::commands::parse_argv([
        "ht".to_owned(),
        "retry".into(),
        reference.recovery_ref(),
    ])
    .unwrap();
    let mut output = vec![];
    let error = crate::cli::run_selected(
        parsed,
        &selection,
        &paths,
        uuid::Uuid::from_u128(1),
        &NoClient,
        &crate::app::SystemClock::new(),
        &mut output,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("person/operator retry requires immediate human namespace")
    );
    assert!(output.is_empty());
    assert!(journal.load(&reference).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn inbox_empty_work_selection_preserves_bounds_and_continuations() {
    use crate::protocol::{
        commands::InboxQuery,
        output::OutputSpec,
        pagination::{Consistency, Page, PageRequest, StopReason},
        results::{ApiError, InboxBatchItem},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    };
    struct Time(AtomicU64);
    impl Clock for Time {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
    }
    struct Client<'a> {
        clock: &'a Time,
        step: u64,
        pages: Mutex<std::collections::VecDeque<CommandResult>>,
        requests: Mutex<Vec<InboxQuery>>,
        v2: bool,
    }
    impl crate::ports::LocalClient for Client<'_> {
        fn call(&self, _: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            panic!("selection must only read inbox pages")
        }
        fn call_with_output(
            &self,
            command: Command,
            _: &OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            let query = match command {
                Command::InboxBatch(query) if !self.v2 => query,
                Command::InboxBatchV2(query) if self.v2 => query,
                _ => panic!("selection changed protocol or attempted mutation"),
            };
            assert_eq!(
                budget.deadline,
                MonoInstant(5000),
                "all reads share one deadline"
            );
            assert_eq!(query.seat.as_ref().map(SeatId::as_str), Some("seat"));
            assert_eq!((query.page.limit, query.page.max_bytes), (1, 1024));
            self.requests.lock().unwrap().push(query.clone());
            self.clock.0.fetch_add(self.step, Ordering::SeqCst);
            if query.page.cursor.as_deref() == Some("read-error") {
                return Err(ApiError::new(
                    crate::protocol::results::ErrorCode::HostUnavailable,
                    "selected read failed",
                ));
            }
            Ok(self
                .pages
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra read"))
        }
    }
    let work = |cursor: &str| Page {
        items: vec![],
        next_cursor: Some(cursor.into()),
        next_argv: Some(vec!["exact-route".into(), cursor.into()]),
        high_water_ordinal: 123,
        scope_revision: None,
        has_more: true,
        stop_reason: StopReason::Work,
        consistency: Consistency::BoundedLive,
    };
    let mut partial = work("partial");
    partial.stop_reason = StopReason::Bytes;
    partial.items.push(InboxBatchItem::Message {
        thread: ThreadId::new("thread"),
        topic_data: "topic".into(),
        message: MessageId::new("message"),
        sequence: 1,
        sender: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: "first chunk".into(),
        body_start: 0,
        body_end: 11,
        body_len: 100,
        ack_candidate: None,
    });
    let mut useful_work = partial.clone();
    useful_work.stop_reason = StopReason::Work;
    let mut warning = work("warning-next");
    warning.items.push(InboxBatchItem::Warning {
        thread: ThreadId::new("thread"),
        topic_data: "topic".into(),
        warning: MessageId::new("warning"),
        sequence: 1,
    });
    let mut malformed = work("missing");
    malformed.next_cursor = None;
    let mut complete = work("unused");
    complete.has_more = false;
    complete.next_cursor = None;
    complete.next_argv = None;
    complete.stop_reason = StopReason::Complete;
    let mut rows = work("rows");
    rows.stop_reason = StopReason::Rows;
    let mut bytes = work("bytes");
    bytes.stop_reason = StopReason::Bytes;
    // Each expected selected page and count is independent of the helper.
    for version in 0..3 {
        for (pages, step, count, expected, error) in [
            (
                vec![work("one"), complete.clone()],
                0,
                2,
                complete.clone(),
                false,
            ),
            (
                vec![work("one"), partial.clone()],
                0,
                2,
                partial.clone(),
                false,
            ),
            (
                vec![work("one"), useful_work.clone()],
                0,
                2,
                useful_work.clone(),
                false,
            ),
            (
                vec![warning.clone(), complete.clone()],
                0,
                1,
                warning.clone(),
                false,
            ),
            (vec![malformed.clone()], 0, 1, malformed.clone(), true),
            (vec![rows.clone()], 0, 1, rows.clone(), false),
            (
                vec![work("read-error"), rows.clone()],
                0,
                2,
                rows.clone(),
                true,
            ),
            (vec![bytes.clone()], 0, 1, bytes.clone(), false),
            (
                (1..=9).map(|n| work(&n.to_string())).collect(),
                0,
                8,
                work("8"),
                false,
            ),
            (
                vec![work("one"), work("two"), work("three")],
                2000,
                3,
                work("three"),
                false,
            ),
            (vec![work("one"), work("one")], 0, 2, work("one"), true),
            (vec![malformed.clone()], 5000, 1, malformed.clone(), false),
            (
                (1..8)
                    .map(|n| work(&n.to_string()))
                    .chain([malformed.clone()])
                    .collect(),
                0,
                8,
                malformed.clone(),
                false,
            ),
        ] {
            let first_cursor = pages[0].next_cursor.clone();
            let convert = |page: Page<InboxBatchItem>| {
                if version == 0 {
                    return CommandResult::InboxBatch(page);
                }
                let mut json = serde_json::to_value(page).unwrap();
                if version == 2 {
                    for item in json["items"].as_array_mut().unwrap() {
                        if item["kind"] == "message" {
                            item["kind"] = "lazy_message".into();
                            item.as_object_mut().unwrap().remove("ack_candidate");
                        }
                    }
                }
                CommandResult::InboxBatchV2(serde_json::from_value(json).unwrap())
            };
            let expected = convert(expected);
            let time = Time(AtomicU64::new(0));
            let client = Client {
                clock: &time,
                step,
                pages: Mutex::new(pages.into_iter().map(convert).collect()),
                v2: version != 0,
                requests: Mutex::new(vec![]),
            };
            let request = InboxQuery {
                seat: Some(SeatId::new("seat")),
                page: PageRequest {
                    cursor: None,
                    limit: 1,
                    max_bytes: 1024,
                },
            };
            let result = super::select_display_inbox_page(
                if version == 0 {
                    Command::InboxBatch(request)
                } else {
                    Command::InboxBatchV2(request)
                },
                &OutputSpec::default(),
                &client,
                &time,
            );
            assert_eq!(client.requests.lock().unwrap().len(), count);
            if error {
                assert!(
                    matches!(result, Err(super::RunError::Api(ref e)) if matches!(e.code, crate::protocol::results::ErrorCode::StoreCorrupt | crate::protocol::results::ErrorCode::HostUnavailable))
                );
            } else {
                let (selected_command, actual) = result.unwrap();
                assert_eq!(actual, expected);
                let requests = client.requests.lock().unwrap();
                assert_eq!(
                    selected_command.page().unwrap(),
                    &requests.last().unwrap().page
                );
                if requests.len() > 1 {
                    assert_eq!(requests[1].page.cursor, first_cursor);
                }
            }
        }
    }
}

#[test]
fn inbox_drained_page_refit_retains_selected_cursor_and_body_offset() {
    use crate::protocol::{
        commands::InboxQuery,
        output::OutputSpec,
        pagination::{Consistency, Page, PageRequest, StopReason},
        results::{ApiError, InboxBatchV2Item},
        time::CallBudget,
    };
    use std::sync::Mutex;
    struct Client(Mutex<Vec<InboxQuery>>);
    impl crate::ports::LocalClient for Client {
        fn call(&self, _: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            panic!("selection/refit must not mutate")
        }
        fn call_with_output(
            &self,
            command: Command,
            _: &OutputSpec,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            let Command::InboxBatchV2(query) = command else {
                panic!("must remain v2")
            };
            let first = self.0.lock().unwrap().is_empty();
            self.0.lock().unwrap().push(query.clone());
            if first {
                return Ok(CommandResult::InboxBatchV2(Page {
                    items: vec![],
                    next_cursor: Some("frozen-selected-body".into()),
                    next_argv: Some(vec!["initial-route".into()]),
                    high_water_ordinal: 123,
                    scope_revision: None,
                    has_more: true,
                    stop_reason: StopReason::Work,
                    consistency: Consistency::BoundedLive,
                }));
            }
            assert_eq!(
                query.page.cursor.as_deref(),
                Some("frozen-selected-body"),
                "byte refit must not replay drained history"
            );
            let len = if query.page.max_bytes == 1024 {
                900
            } else {
                20
            };
            Ok(CommandResult::InboxBatchV2(Page {
                items: vec![InboxBatchV2Item::LazyMessage {
                    thread: ThreadId::new("thread"),
                    topic_data: "topic".into(),
                    message: MessageId::new("lazy"),
                    sequence: 1,
                    sender: None,
                    author_role: None,
                    relays_user: false,
                    user_intent: None,
                    author_role_backfilled: false,
                    body: "x".repeat(len),
                    body_start: 7,
                    body_end: 7 + len as u64,
                    body_len: 2000,
                }],
                next_cursor: Some("frozen-next-body".into()),
                // Final CLI routing can make a wire-selected body exceed its bound.
                next_argv: Some(vec![
                    "herdr-threads".into(),
                    "--state-dir".into(),
                    "r".repeat(250),
                    "inbox".into(),
                ]),
                high_water_ordinal: 123,
                scope_revision: None,
                has_more: true,
                stop_reason: StopReason::Bytes,
                consistency: Consistency::BoundedLive,
            }))
        }
    }
    let client = Client(Mutex::new(vec![]));
    let command = Command::InboxBatchV2(InboxQuery {
        seat: Some(SeatId::new("seat")),
        page: PageRequest {
            cursor: None,
            limit: 1,
            max_bytes: 1024,
        },
    });
    let clock = crate::app::SystemClock::new();
    let spec = OutputSpec::default();
    let (selected, page) =
        super::select_display_inbox_page(command, &spec, &client, &clock).unwrap();
    let result = super::fit_inbox_read(selected, page, &spec, &client, &|| {
        super::cooperative_budget(&clock)
    })
    .unwrap();
    assert_eq!(
        client.0.lock().unwrap().len(),
        3,
        "two selection reads then one body refit"
    );
    assert!(super::output::emitted_bytes(&result, &spec).unwrap().len() <= 1024);
    let CommandResult::InboxBatchV2(page) = result else {
        panic!("wrong protocol")
    };
    assert_eq!(page.high_water_ordinal, 123);
    assert_eq!(page.next_cursor.as_deref(), Some("frozen-next-body"));
    assert!(matches!(
        &page.items[0],
        InboxBatchV2Item::LazyMessage {
            body_start: 7,
            body_end: 27,
            body_len: 2000,
            ..
        }
    ));
}

// Catches new/retried join being dispatched to an incompatible daemon, or a
// fresh refused invocation creating an intent before capability discovery.
#[test]
fn public_join_cli_capability_refusal_precedes_journal_submission() {
    use crate::harness::context::{
        ContextJournal, Harness as ContextHarness, OccupantContext, Role, SessionReference,
    };
    use crate::protocol::{
        output::OutputSpec,
        results::{ApiError, CapabilityList, ErrorCode},
        time::CallBudget,
    };
    struct Client {
        available: bool,
    }
    impl crate::ports::LocalClient for Client {
        fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            assert_eq!(command, Command::Capabilities, "must not submit join");
            if self.available {
                Ok(CommandResult::Capabilities(CapabilityList {
                    capabilities: vec![],
                }))
            } else {
                Err(ApiError::new(ErrorCode::HostUnavailable, "unavailable"))
            }
        }
        fn call_with_output(
            &self,
            command: Command,
            _: &OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(command, budget)
        }
    }
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("public-join-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let instance = uuid::Uuid::from_u128(1);
    let contexts = ContextJournal::open(
        &root,
        instance,
        "seat-test",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let execution = uuid::Uuid::from_u128(2);
    contexts
        .install_reattached(OccupantContext {
            format_version: 1,
            instance,
            seat: "seat-test".into(),
            target: "w1:p1".into(),
            harness: ContextHarness::Codex,
            binding_generation: 1,
            execution,
            session: SessionReference::PluginContext(execution),
            role: Role::TopLevel,
        })
        .unwrap();
    let intents = root.join("intents");
    let journal = Journal::open(&intents).unwrap();
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "join", "thread"]).unwrap();
    let before: Vec<_> = std::fs::read_dir(&intents)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    for available in [true, false] {
        let mut output = Vec::new();
        let error = crate::cli::run_cooperative(
            parsed.clone(),
            &journal,
            &contexts,
            None,
            Role::TopLevel,
            &Client { available },
            &crate::app::SystemClock::new(),
            &mut output,
        )
        .unwrap_err();
        let text = error.to_string();
        assert!(
            text.contains(if available {
                "thread.join_v1"
            } else {
                "unavailable"
            }),
            "{text}"
        );
        assert!(output.is_empty());
        let after: Vec<_> = std::fs::read_dir(&intents)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(after, before);
    }
    let claim =
        crate::harness::bridge::caller_claim(&contexts.current().unwrap().unwrap()).unwrap();
    let scope = IntentScope::Cooperative {
        instance: instance.to_string(),
        seat: SeatId::new("seat-test"),
    };
    let reference = journal
        .record(
            scope,
            SemanticMutation::freeze(
                SemanticMutation::Join {
                    thread: ThreadId::new("thread"),
                },
                claim,
            )
            .unwrap(),
            0,
        )
        .unwrap();
    let recovery = reference.recovery_ref();
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "retry", &recovery]).unwrap();
    let error = crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        &Client { available: true },
        &crate::app::SystemClock::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("thread.join_v1"));
    assert!(
        journal.load(&reference).is_ok(),
        "incompatible retry must retain its intent"
    );
    std::fs::remove_dir_all(root).unwrap();
}

mod main539_composition {
    use super::*;
    use crate::{
        ports::{LocalClient, LocalService},
        protocol::{
            authority::PeerIdentity,
            time::{CallBudget, Clock},
        },
    };
    use std::sync::Arc;

    struct Client {
        domain: Box<dyn LocalService>,
        join_capability: std::sync::atomic::AtomicBool,
        calls: std::sync::Mutex<Vec<Command>>,
    }
    impl LocalClient for Client {
        crate::default_output_local_client!();
        fn call(
            &self,
            command: Command,
            budget: &CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.calls.lock().unwrap().push(command.clone());
            let result = self
                .domain
                .handle(command, PeerIdentity::from_kernel(501), budget)?;
            if let CommandResult::Capabilities(mut capabilities) = result {
                assert!(
                    capabilities
                        .capabilities
                        .iter()
                        .any(|s| s == crate::protocol::capabilities::THREAD_JOIN)
                );
                assert!(
                    !capabilities.capabilities.iter().any(
                        |s| s == crate::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1
                    )
                );
                if !self
                    .join_capability
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    capabilities
                        .capabilities
                        .retain(|s| s != crate::protocol::capabilities::THREAD_JOIN);
                }
                return Ok(CommandResult::Capabilities(capabilities));
            }
            Ok(result)
        }
    }
    struct Fixture {
        client: Client,
        journal: Journal,
        contexts: crate::harness::context::ContextJournal,
        claim: CallerClaim,
        root: std::path::PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        fn new(human: bool) -> Self {
            use crate::harness::context::{
                ContextJournal, Harness, OccupantContext, Role, SessionReference,
            };
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("ht-main539-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let instance = uuid::Uuid::new_v4();
            let contexts = ContextJournal::open(
                &root,
                instance,
                "seat-test",
                std::time::Duration::from_millis(20),
            )
            .unwrap();
            contexts
                .install_reattached(OccupantContext {
                    format_version: 1,
                    instance,
                    seat: "seat-test".into(),
                    target: "w1:p1".into(),
                    harness: if human {
                        Harness::Human
                    } else {
                        Harness::Codex
                    },
                    binding_generation: 1,
                    execution: uuid::Uuid::new_v4(),
                    session: SessionReference::Native("n".into()),
                    role: Role::TopLevel,
                })
                .unwrap();
            let claim = crate::harness::bridge::caller_claim(&contexts.current().unwrap().unwrap())
                .unwrap();
            let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::new());
            let context =
                crate::store::connection::StoreContext::new(root.join("state.db"), clock.clone());
            let db = context.open_writer().unwrap();
            db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES (?1,0,'b',1)", [&claim.instance]).unwrap();
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native',?3,1,1,0)", rusqlite::params![claim.seat.as_str(),claim.instance,claim.target.as_str()]).unwrap();
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,?2,'b',1,1,?3,'fresh','term','inc','coherent_enumeration',1)", rusqlite::params![claim.instance,claim.target.as_str(),clock.utc_now().0]).unwrap();
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,1,?2,'b',1,?3,?4,?5,?6,?7,'term','inc')", rusqlite::params![claim.seat.as_str(),claim.target.as_str(),if human {"human"} else {"codex"},claim.native_session.as_str(),claim.execution.as_str(),if human {"operator_human"} else {"cooperative_top_level"},clock.utc_now().0]).unwrap();
            db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t',?1,'topic','goal',0,0)", [&claim.instance]).unwrap();
            drop(db);
            let store = Arc::new(
                crate::store::SqliteStore::new(context, claim.instance.clone(), Default::default())
                    .unwrap(),
            );
            let domain =
                crate::service::dispatch::DomainService::new(claim.instance.clone(), store, clock)
                    .with_cooperative_owner(
                        501,
                        Arc::new(crate::service::fair_writer::FairWriter::new(8)),
                    );
            let boot = uuid::Uuid::new_v4();
            let domain = crate::daemon::control::ControlService::new(
                crate::daemon::control::StopController::new(instance, boot, Default::default()),
                move |_: &CallBudget| crate::daemon::health::HealthInputs::unknown(instance, boot),
                domain,
            );
            let journal = Journal::open(root.join("intents")).unwrap();
            Self {
                client: Client {
                    domain: Box::new(domain),
                    join_capability: std::sync::atomic::AtomicBool::new(true),
                    calls: Default::default(),
                },
                journal,
                contexts,
                claim,
                root,
            }
        }
        fn record_join(&self) -> crate::cli::journal::IntentRef {
            self.journal
                .record(
                    IntentScope::Cooperative {
                        instance: self.claim.instance.clone(),
                        seat: self.claim.seat.clone(),
                    },
                    SemanticMutation::freeze(
                        SemanticMutation::Join {
                            thread: ThreadId::new("t"),
                        },
                        self.claim.clone(),
                    )
                    .unwrap(),
                    0,
                )
                .unwrap()
        }
        fn run(&self, argv: Vec<String>, output: &mut Vec<u8>) -> Result<(), crate::cli::RunError> {
            crate::cli::run_cooperative(
                crate::cli::commands::parse_argv(argv).unwrap(),
                &self.journal,
                &self.contexts,
                None,
                crate::harness::context::Role::TopLevel,
                &self.client,
                &crate::app::SystemClock::new(),
                output,
            )
        }
    }
    #[test]
    fn fresh_agent_and_human_join_capability_refusal_precedes_publication() {
        for human in [false, true] {
            let f = Fixture::new(human);
            f.client
                .join_capability
                .store(false, std::sync::atomic::Ordering::Relaxed);
            let before: Vec<_> = std::fs::read_dir(f.journal.root())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            let mut argv = vec!["ht".to_owned()];
            if human {
                argv.push("human".into());
            }
            argv.extend(["join".into(), "t".into()]);
            let mut output = Vec::new();
            assert!(
                f.run(argv, &mut output)
                    .unwrap_err()
                    .to_string()
                    .contains("thread.join_v1")
            );
            assert!(output.is_empty());
            assert_eq!(*f.client.calls.lock().unwrap(), vec![Command::Capabilities]);
            let after: Vec<_> = std::fs::read_dir(f.journal.root())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(before, after);
        }
    }
    #[test]
    fn original_human_join_refuses_root_then_exact_human_and_agent_replays_complete() {
        for human in [true, false] {
            let f = Fixture::new(human);
            let reference = f.record_join();
            let path = f.journal.root().join(format!(
                "{:020}-{}.intent",
                reference.ordinal,
                reference.operation.as_str()
            ));
            let before = std::fs::read(&path).unwrap();
            if human {
                let mut output = Vec::new();
                let error = f
                    .run(
                        vec!["ht".into(), "retry".into(), reference.recovery_ref()],
                        &mut output,
                    )
                    .unwrap_err();
                assert!(
                    error
                        .to_string()
                        .contains("person/operator retry requires immediate human namespace")
                );
                assert!(f.client.calls.lock().unwrap().is_empty());
                assert!(output.is_empty());
                assert_eq!(std::fs::read(&path).unwrap(), before);
            }
            let mut argv = vec!["ht".into()];
            if human {
                argv.push("human".into());
            }
            argv.extend(["retry".into(), reference.recovery_ref()]);
            let mut output = Vec::new();
            f.run(argv, &mut output).unwrap();
            assert!(!output.is_empty());
            assert!(f.journal.load(&reference).is_err());
            assert!(
                matches!(&f.client.calls.lock().unwrap()[..],[Command::Capabilities,Command::Join(v)] if v.claim==f.claim && v.operation==reference.operation)
            );
        }
    }
    #[test]
    fn public_root_human_join_retry_refuses_before_context_transport_and_cleanup() {
        use crate::daemon::paths::{InstancePaths, RuntimeContext};
        let f = Fixture::new(true);
        let runtime = RuntimeContext::explicit(
            f.root.join("isolated-state"),
            f.root.join("absent-host.sock"),
            None,
        )
        .unwrap();
        let paths = InstancePaths::resolve(&runtime).unwrap();
        let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
        let reference = journal
            .record(
                IntentScope::Cooperative {
                    instance: f.claim.instance.clone(),
                    seat: f.claim.seat.clone(),
                },
                SemanticMutation::freeze(
                    SemanticMutation::Join {
                        thread: ThreadId::new("t"),
                    },
                    f.claim.clone(),
                )
                .unwrap(),
                0,
            )
            .unwrap();
        let snapshot = || {
            let mut rows: Vec<_> = std::fs::read_dir(journal.root())
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    (
                        path.file_name().unwrap().to_owned(),
                        std::fs::read(path).unwrap(),
                    )
                })
                .collect();
            rows.sort();
            rows
        };
        let before = snapshot();
        let mut output = Vec::new();
        let error = crate::cli::run_in_pane(
            vec![
                "ht".into(),
                "--state-dir".into(),
                runtime.state_dir.display().to_string(),
                "--host-endpoint".into(),
                runtime.host_endpoint.display().to_string(),
                "retry".into(),
                reference.recovery_ref(),
            ],
            None,
            &mut output,
        )
        .unwrap_err();
        assert!(
            format!("{error:?}")
                .contains("person/operator retry requires immediate human namespace"),
            "{error:?}"
        );
        assert!(output.is_empty());
        assert_eq!(snapshot(), before);
        assert!(!paths.descriptor_path.exists());
        assert!(!paths.database_path.exists());
        assert!(!paths.instance_dir.join("contexts").exists());
    }

    #[test]
    fn joined_result_is_refused_by_handoff_invitation_stage_and_durable_retry() {
        let f = Fixture::new(false);
        let invite = SemanticMutation::Invite {
            thread: ThreadId::new("t"),
            seat: SeatId::new("recipient"),
            deadline_millis: None,
        };
        let reference = f
            .journal
            .record(
                IntentScope::Cooperative {
                    instance: f.claim.instance.clone(),
                    seat: f.claim.seat.clone(),
                },
                SemanticMutation::freeze(invite.clone(), f.claim.clone()).unwrap(),
                0,
            )
            .unwrap();
        let mut output = Vec::new();
        let failure = retry::run_retry_api_to_writer(
            &f.journal,
            &reference,
            &IntentScope::Cooperative {
                instance: f.claim.instance.clone(),
                seat: f.claim.seat.clone(),
            },
            || Ok(f.claim.clone()),
            |_| Ok(CommandResult::Joined(ThreadId::new("t"))),
            &Default::default(),
            &mut output,
        )
        .unwrap_err();
        assert!(
            matches!(failure, retry::RetryFailure::Local(ref e) if e.to_string().contains("unexpected mutation result"))
        );
        assert!(output.is_empty());
        assert!(f.journal.load(&reference).is_ok());
        let channel = crate::protocol::handoff::HandoffChannel::Existing {
            thread: ThreadId::new("t"),
        };
        let create = OperationId::new("create");
        let send = OperationId::new("send");
        let recipient = SeatId::new("recipient");
        let mut progress = crate::cli::handoff::StagedWork::default();
        let mut phase = "unset";
        let error = crate::cli::handoff::stage_work(
            crate::cli::handoff::Staging {
                channel: &channel,
                body: "work",
                recipient: &recipient,
                create_key: &create,
                invite_key: &reference.operation,
                send_key: &send,
                skip_joined: false,
            },
            &mut progress,
            &mut phase,
            &|semantic, key| {
                assert_eq!(semantic, invite);
                assert_eq!(key, &reference.operation);
                Ok(CommandResult::Joined(ThreadId::new("t")))
            },
            &mut |_| Ok(()),
            &f.client,
            &crate::app::SystemClock::new(),
        )
        .unwrap_err();
        assert_eq!(phase, "invite");
        assert!(error.to_string().contains("unexpected invite result"));
        assert!(progress.invitation.is_none());
        assert!(progress.message.is_none());
        assert!(f.client.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn current_main_human_selected_registered_agent_refuses_before_effects() {
    use crate::harness::context::{Harness as ContextHarness, Role};
    let isolation = crate::test_support::isolation::TestIsolation::new("human-selected-hermes");
    let root = isolation.state_root().canonicalize().unwrap();
    let journal = Journal::open(root.join("intents")).unwrap();
    let contexts = crate::harness::context::ContextJournal::open(
        &root,
        uuid::Uuid::from_u128(1),
        "seat-1",
        std::time::Duration::from_millis(20),
    )
    .unwrap();
    let mut current = human_context("current-pane");
    current.harness = ContextHarness::from(crate::harness::registry::OccupantHarness::Agent(
        crate::harness::registry::builtins()
            .agent("hermes")
            .unwrap(),
    ));
    contexts.install_reattached(current).unwrap();
    struct Never;
    impl crate::ports::LocalClient for Never {
        crate::default_output_local_client!();
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("Human selected registered Agent reached accountable submission: {command:?}")
        }
    }
    let before = actor_fixture_snapshot(&root);
    let parsed = crate::cli::commands::parse_argv(["ht", "human", "ack", "message"]).unwrap();
    let mut output = Vec::new();
    let error = crate::cli::run_cooperative(
        parsed,
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        &Never,
        &crate::app::SystemClock::new(),
        &mut output,
    )
    .unwrap_err();
    assert!(
        format!("{error:?}")
            .contains("human namespace cannot act through an agent cooperative selection"),
        "{error:?}"
    );
    assert_eq!(actor_fixture_snapshot(&root), before);
    assert!(output.is_empty());
}

/// Ordinary replay uses the production canonical store and domain dispatcher.
pub(crate) struct OrdinaryHumanRetryFixture {
    pub(crate) client: OrdinaryReplayClient,
    pub(crate) dir: std::path::PathBuf,
    pub(crate) runtime: crate::daemon::paths::RuntimeContext,
    pub(crate) paths: crate::daemon::paths::InstancePaths,
    pub(crate) journal: Journal,
    pub(crate) reference: crate::cli::journal::IntentRef,
    pub(crate) original: CallerClaim,
    pub(crate) clock: std::sync::Arc<dyn crate::protocol::time::Clock>,
}
pub(crate) struct OrdinaryReplayClient {
    service: crate::service::dispatch::DomainService,
    pub(crate) calls: std::sync::Mutex<Vec<Command>>,
}
impl crate::ports::LocalClient for OrdinaryReplayClient {
    fn call(
        &self,
        command: Command,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.call_with_output(command, &Default::default(), budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &crate::protocol::output::OutputSpec,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        use crate::ports::LocalService;
        self.calls.lock().unwrap().push(command.clone());
        self.service.handle_with_output(
            command,
            crate::protocol::authority::PeerIdentity::from_kernel(501),
            budget,
            output,
        )
    }
}
impl crate::ports::LocalClient for &OrdinaryReplayClient {
    fn call(
        &self,
        command: Command,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        crate::ports::LocalClient::call(*self, command, budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &crate::protocol::output::OutputSpec,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        crate::ports::LocalClient::call_with_output(*self, command, output, budget)
    }
}
impl OrdinaryHumanRetryFixture {
    pub(crate) fn new(committed: bool) -> Self {
        use crate::{
            harness::context::{OccupantContext, Role, SessionReference},
            ports::LocalClient,
            protocol::commands::{CheckIn, CheckInMode},
        };
        let dir =
            std::env::temp_dir().join(format!("ht-ordinary-human-retry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            dir.join("state"),
            dir.join("host.sock"),
            None,
        )
        .unwrap();
        let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
        let instance = uuid::Uuid::new_v4();
        crate::daemon::paths::ensure_owned_state_root(&runtime.state_dir).unwrap();
        crate::daemon::paths::ensure_private_dir(paths.instance_dir.parent().unwrap()).unwrap();
        crate::daemon::paths::ensure_private_dir(&paths.instance_dir).unwrap();
        std::fs::write(&paths.namespace_path, instance.to_string()).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &paths.namespace_path,
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let clock: std::sync::Arc<dyn crate::protocol::time::Clock> =
            std::sync::Arc::new(crate::app::SystemClock::new());
        let context =
            crate::store::connection::StoreContext::new(dir.join("store.db"), clock.clone());
        let conn = context.open_writer().unwrap();
        conn.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES (?1,0,'b',1)",
            [&instance.to_string()],
        )
        .unwrap();
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat',?1,'resolved','native','w:p1',0,0,0)", [&instance.to_string()]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,'w:p1','b',1,0,1,'fresh','unknown','unknown',0,0,'term','inc','coherent_enumeration',1)", [&instance.to_string()]).unwrap();
        drop(conn);
        let store = std::sync::Arc::new(
            crate::store::SqliteStore::new(
                context,
                instance.to_string(),
                crate::store::StoreSettings::default(),
            )
            .unwrap(),
        );
        let client = OrdinaryReplayClient {
            service: crate::service::dispatch::DomainService::new(
                instance.to_string(),
                store,
                clock.clone(),
            )
            .with_cooperative_owner(
                501,
                std::sync::Arc::new(crate::service::fair_writer::FairWriter::new(4)),
            ),
            calls: Default::default(),
        };
        let budget = crate::cli::cooperative_budget(clock.as_ref());
        let mut original = claim();
        original.instance = instance.to_string();
        original.target = HostTargetId::new("w:p1");
        original.binding_generation = 0;
        original.harness = Harness::Human;
        original.execution = ExecutionId::new(uuid::Uuid::new_v4().to_string());
        original.native_session =
            NativeSessionId::new(format!("plugin_context:{}", uuid::Uuid::new_v4()));
        let result = client
            .call(
                Command::CheckIn(CheckIn {
                    claim: original,
                    operation: OperationId::new("human-initial"),
                    mode: CheckInMode::Lifecycle {
                        expected_binding_generation: 0,
                    },
                }),
                &budget,
            )
            .unwrap();
        let CommandResult::CheckedIn(result) = result else {
            panic!("check-in result")
        };
        let original = result.context;
        let contexts = crate::cli::seat_contexts(&paths, instance, &original.seat).unwrap();
        let context_for = |claim: &CallerClaim, harness| OccupantContext {
            format_version: 1,
            instance,
            seat: claim.seat.as_str().into(),
            target: claim.target.as_str().into(),
            harness,
            binding_generation: claim.binding_generation,
            execution: uuid::Uuid::parse_str(claim.execution.as_str()).unwrap(),
            session: SessionReference::PluginContext(
                uuid::Uuid::parse_str(
                    claim
                        .native_session
                        .as_str()
                        .strip_prefix("plugin_context:")
                        .unwrap(),
                )
                .unwrap(),
            ),
            role: Role::TopLevel,
        };
        let human = context_for(&original, crate::harness::context::Harness::Human);
        contexts.install_reattached(human).unwrap();
        let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
        let semantic = SemanticMutation::freeze(
            SemanticMutation::CreateThread {
                name: None,
                topic: "historical human topic".into(),
                goal: "historical goal".into(),
            },
            original.clone(),
        )
        .unwrap();
        let reference = journal
            .record(
                IntentScope::Cooperative {
                    instance: original.instance.clone(),
                    seat: original.seat.clone(),
                },
                semantic,
                1,
            )
            .unwrap();
        if committed {
            let failure = retry::run_retry_api_to_writer(
                &journal,
                &reference,
                &journal.load(&reference).unwrap().header.scope,
                || panic!("frozen claim"),
                |command| client.call(command, &budget),
                &json_output(),
                &mut OrdinaryFailedOutput::flush(),
            );
            assert!(
                matches!(failure, Err(retry::RetryFailure::Local(_))),
                "{failure:?}"
            );
        }
        let mut agent = original.clone();
        agent.harness = Harness::Codex;
        agent.execution = ExecutionId::new(uuid::Uuid::new_v4().to_string());
        agent.native_session =
            NativeSessionId::new(format!("plugin_context:{}", uuid::Uuid::new_v4()));
        let result = client
            .call(
                Command::CheckIn(CheckIn {
                    claim: agent,
                    operation: OperationId::new("agent-successor"),
                    mode: CheckInMode::Lifecycle {
                        expected_binding_generation: original.binding_generation,
                    },
                }),
                &budget,
            )
            .unwrap();
        let CommandResult::CheckedIn(result) = result else {
            panic!("agent check-in result")
        };
        contexts
            .install_reattached(context_for(
                &result.context,
                crate::harness::context::Harness::Codex,
            ))
            .unwrap();
        client.calls.lock().unwrap().clear();
        Self {
            client,
            dir,
            runtime,
            paths,
            journal,
            reference,
            original,
            clock,
        }
    }
    pub(crate) fn argv(&self, human: bool) -> Vec<String> {
        let mut argv = vec!["ht".into()];
        if human {
            argv.push("human".into());
        }
        argv.extend([
            "--state-dir".into(),
            self.runtime.state_dir.to_string_lossy().into_owned(),
            "--host-endpoint".into(),
            self.runtime.host_endpoint.to_string_lossy().into_owned(),
            "--json".into(),
            "retry".into(),
            self.reference.recovery_ref(),
        ]);
        argv
    }
    fn invoke(&self, writer: &mut impl io::Write) -> Result<(), crate::cli::RunError> {
        let parsed = crate::cli::commands::parse_argv(self.argv(true)).unwrap();
        let connection = crate::cli::LazyConnection::new(|| {
            Ok((
                uuid::Uuid::parse_str(&self.original.instance).unwrap(),
                &self.client,
            ))
        });
        let selection = crate::cli::derive_selection(
            &parsed,
            Some("w:p1"),
            &self.runtime,
            &self.paths,
            &connection,
            &self.clock,
        )?;
        crate::cli::run_selected(
            parsed,
            &selection,
            &self.paths,
            uuid::Uuid::parse_str(&self.original.instance).unwrap(),
            &self.client,
            self.clock.as_ref(),
            writer,
        )
    }
    pub(crate) fn snapshot(&self) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn visit(
            root: &std::path::Path,
            bytes: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        ) {
            for entry in std::fs::read_dir(root).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, bytes);
                } else {
                    bytes.insert(path.clone(), std::fs::read(path).unwrap());
                }
            }
        }
        let mut bytes = Default::default();
        visit(&self.paths.instance_dir, &mut bytes);
        bytes
    }
    fn operation_bytes(&self) -> Vec<(String, Vec<u8>, String)> {
        let conn = rusqlite::Connection::open(self.dir.join("store.db")).unwrap();
        let mut statement = conn
            .prepare(
                "SELECT operation_key,digest,result_json FROM operations ORDER BY operation_key",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}
impl Drop for OrdinaryHumanRetryFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}
fn json_output() -> crate::protocol::output::OutputSpec {
    crate::protocol::output::OutputSpec {
        format: crate::protocol::output::OutputFormat::Json,
        ..Default::default()
    }
}
struct OrdinaryFailedOutput {
    bytes: Vec<u8>,
    fail_write: bool,
}
impl OrdinaryFailedOutput {
    fn flush() -> Self {
        Self {
            bytes: vec![],
            fail_write: false,
        }
    }
}
impl io::Write for OrdinaryFailedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_write {
            return Err(io::Error::other("ordinary output write interrupted"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("ordinary output flush interrupted"))
    }
}
#[test]
fn frozen_human_ordinary_retry_after_agent_binding() {
    let fx = OrdinaryHumanRetryFixture::new(true);
    let frozen = fx.snapshot();
    let saved = fx.journal.load(&fx.reference).unwrap();
    let operations = fx.operation_bytes();
    let request = saved
        .semantic
        .to_command(fx.reference.operation.clone(), Some(fx.original.clone()))
        .unwrap();
    let mut flushed = vec![];
    for fail_write in [true, false] {
        let mut writer = OrdinaryFailedOutput {
            bytes: vec![],
            fail_write,
        };
        let failure = fx.invoke(&mut writer).unwrap_err();
        assert!(
            matches!(failure, crate::cli::RunError::Io(ref error) if error.to_string().contains("ordinary output")),
            "{failure:?}"
        );
        assert_eq!(
            fx.snapshot(),
            frozen,
            "output failure must retain exact intent and context bytes"
        );
        assert_eq!(
            fx.operation_bytes(),
            operations,
            "canonical stored response/digest/key must remain exact"
        );
        if !fail_write {
            flushed = writer.bytes;
            assert!(!flushed.is_empty());
        }
    }
    let mut output = vec![];
    fx.invoke(&mut output).unwrap();
    assert_eq!(output, flushed);
    assert_eq!(fx.operation_bytes(), operations);
    assert!(
        fx.journal.load(&fx.reference).is_err(),
        "successful flush completes only the retained intent"
    );
    let remaining = fx.snapshot();
    let removed: Vec<_> = frozen
        .keys()
        .filter(|path| !remaining.contains_key(*path))
        .collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].extension().unwrap(), "intent");
    for (path, bytes) in remaining {
        assert_eq!(Some(&bytes), frozen.get(&path));
    }
    let calls = fx.client.calls.lock().unwrap();
    let replay: Vec<_> = calls
        .iter()
        .filter(|command| matches!(command, Command::CreateThread(_)))
        .collect();
    assert_eq!(replay.len(), 3);
    for command in replay {
        assert_eq!(
            serde_json::to_vec(command).unwrap(),
            serde_json::to_vec(&request).unwrap()
        );
    }
}
#[test]
fn frozen_human_ordinary_retry_pending_canonical_refuses() {
    let fx = OrdinaryHumanRetryFixture::new(false);
    let frozen = fx.snapshot();
    let operations = fx.operation_bytes();
    let mut output = vec![];
    let failure = fx.invoke(&mut output).unwrap_err();
    assert!(
        matches!(failure, crate::cli::RunError::Api(ref error) if error.code == crate::protocol::results::ErrorCode::CallerUnverified),
        "{failure:?}"
    );
    assert!(output.is_empty());
    assert_eq!(fx.snapshot(), frozen);
    assert_eq!(fx.operation_bytes(), operations);
    let conn = rusqlite::Connection::open(fx.dir.join("store.db")).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM threads", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(
        fx.client
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|command| matches!(command, Command::CreateThread(_))),
        "refusal must come from canonical replay, after frozen dispatch"
    );
}
