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
                        return Err(crate::protocol::results::ApiError {
                            code: crate::protocol::results::ErrorCode::HostUnavailable,
                            detail: "request lost before commit".into(),
                            restart_argv: None,
                            required_minimum_bytes: None,
                        });
                    }
                    if expected_binding_generation != self.generation.load(Ordering::SeqCst) {
                        return Err(crate::protocol::results::ApiError {
                            code: crate::protocol::results::ErrorCode::CallerUnverified,
                            detail: "stale generation".into(),
                            restart_argv: None,
                            required_minimum_bytes: None,
                        });
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
                        return Err(crate::protocol::results::ApiError {
                            code: crate::protocol::results::ErrorCode::HostUnavailable,
                            detail: "committed response lost".into(),
                            restart_argv: None,
                            required_minimum_bytes: None,
                        });
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
        crate::cli::derive_caller(&mut parsed, None, &runtime, &paths, &clock).unwrap();
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
            crate::cli::derive_caller(&mut bare, None, &runtime, &paths, &clock)
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
    let failed = crate::protocol::results::ApiError {
        code: crate::protocol::results::ErrorCode::Unsupported,
        detail: "herdr down".into(),
        restart_argv: None,
        required_minimum_bytes: None,
    };
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
        detail.contains("herdr-threads me init --operator"),
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
            detail.contains("herdr-threads me init --operator"),
            "{detail}"
        );
    }
    // No agent, an unrelated detection, or a failed read never refuses.
    assert!(select(&[], Ok(Some(observed(None))), false).is_ok());
    assert!(select(&[], Ok(Some(observed(Some("shell")))), false).is_ok());
    let failed = crate::protocol::results::ApiError {
        code: crate::protocol::results::ErrorCode::Unsupported,
        detail: "herdr down".into(),
        restart_argv: None,
        required_minimum_bytes: None,
    };
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
