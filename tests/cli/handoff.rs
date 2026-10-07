use super::*;
use crate::protocol::{
    authority::{CallerRole, Harness},
    ids::{ExecutionId, HostTargetId, InvitationId, MessageId, NativeSessionId},
    time::{CallBudget, MonoInstant, UtcMillis},
};
use std::sync::Mutex;
struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}
fn claim() -> CallerClaim {
    CallerClaim {
        instance: "00000000-0000-0000-0000-0000000000b1".into(),
        seat: SeatId::new("sender"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Human,
        native_session: NativeSessionId::new("human"),
        execution: ExecutionId::new("execution"),
        target: HostTargetId::new("w1:p1"),
    }
}
fn request() -> HandoffRequest {
    HandoffRequest {
        thread: None,
        thread_name: Some("review".into()),
        topic: Some("Review".into()),
        goal: Some("Review".into()),
        body: "secret durable task".into(),
        launch: LaunchRequest {
            target: HostTargetId::new("w1:p2"),
            harness: crate::harness::context::Harness::Codex,
            harness_binary: None,
            argv: vec!["-a".into(), "on-request".into()],
            name: Some("worker".into()),
            pane_label: None,
        },
    }
}
struct Client {
    joined: bool,
    already_joined: bool,
    calls: Mutex<Vec<Command>>,
    lost: Mutex<Option<&'static str>>,
    results: Mutex<std::collections::HashMap<String, CommandResult>>,
}
impl Client {
    fn new(lost: Option<&'static str>) -> Self {
        Self {
            joined: true,
            already_joined: false,
            calls: Mutex::new(vec![]),
            lost: Mutex::new(lost),
            results: Mutex::new(Default::default()),
        }
    }
}
impl LocalClient for Client {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        if let Command::BeginHandoff(q) | Command::CompleteHandoff(q) = &command {
            return Ok(CommandResult::Handoff(
                crate::protocol::handoff::HandoffResult {
                    compound: q.identity.compound.clone(),
                    thread: Some(ThreadId::new("t1")),
                    state: if matches!(command, Command::CompleteHandoff(_)) {
                        crate::protocol::handoff::HandoffState::Completed
                    } else {
                        crate::protocol::handoff::HandoffState::Live
                    },
                },
            ));
        }
        if let Command::Directory(query) = &command {
            assert_eq!(query.membership, Some(claim().seat));
            assert_eq!(
                query.membership_filter,
                crate::protocol::commands::DirectoryMembership::Joined
            );
            let items = if self.joined {
                vec![
                    serde_json::json!({"thread":"t1","name":"renamed elsewhere","topic_data":"topic","topic_omitted":false,"topic_detail_argv":null,"archived":false,"orphaned":false,"message_count":0,"created_at":1,"ordinary_count":0,"system_count":0,"joined_count":1}),
                ]
            } else {
                vec![]
            };
            return Ok(CommandResult::Directory(serde_json::from_value(serde_json::json!({"items":items,"next_cursor":null,"next_argv":null,"high_water_ordinal":1,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"})).unwrap()));
        }
        let (key, phase, result) = match &command {
            Command::CreateThread(q) => {
                assert_eq!(q.claim, claim());
                (
                    q.operation.as_str(),
                    "create",
                    CommandResult::ThreadCreated(ThreadId::new("t1")),
                )
            }
            Command::Invite(q) => {
                assert_eq!(q.claim, claim());
                assert_eq!(q.seat, SeatId::new("recipient"));
                (
                    q.operation.as_str(),
                    "invite",
                    if self.already_joined {
                        CommandResult::AlreadyJoined(crate::protocol::results::AlreadyJoined {
                            thread: q.thread.clone(),
                            seat: q.seat.clone(),
                        })
                    } else {
                        CommandResult::Invitation(InvitationId::new("i1"))
                    },
                )
            }
            Command::SendMessage(q) => {
                assert_eq!(q.claim, claim());
                assert_eq!(q.body, "secret durable task");
                assert_eq!(q.invited_recipients, vec![SeatId::new("recipient")]);
                (
                    q.operation.as_str(),
                    "send",
                    CommandResult::MessageSent(MessageId::new("m1")),
                )
            }
            _ => panic!("unexpected command {command:?}"),
        };
        let result = self
            .results
            .lock()
            .unwrap()
            .entry(key.into())
            .or_insert(result)
            .clone();
        self.calls.lock().unwrap().push(command);
        let mut lost = self.lost.lock().unwrap();
        if *lost == Some(phase) {
            *lost = None;
            return Err(ApiError::new(
                ErrorCode::UnknownOutcome,
                "response lost after commit",
            ));
        }
        Ok(result)
    }
}
#[derive(Default)]
struct Launcher {
    preflights: usize,
    starts: usize,
    submitted_argv: Vec<String>,
    configured_prefix: Vec<String>,
    preflight_argv: Vec<String>,
    refuse: bool,
    confirmed_refusal: bool,
    unknown: bool,
    panic_after_gate: bool,
    panic_after_start: bool,
}
impl HandoffLauncher for Launcher {
    fn preflight(&mut self, request: &LaunchRequest) -> Result<SeatId, RunError> {
        self.preflight_argv = request.argv.clone();
        self.preflights += 1;
        Ok(SeatId::new("recipient"))
    }
    fn launch(
        &mut self,
        request: &LaunchRequest,
        seat: &SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::super::launch::LaunchReport, RunError> {
        assert_eq!(seat, &SeatId::new("recipient"));
        assert!(request.argv.starts_with(&self.configured_prefix));
        let caller_start = self.configured_prefix.len();
        assert_eq!(
            &request.argv[caller_start..caller_start + 2],
            ["-a", "on-request"]
        );
        assert!(
            !request
                .argv
                .iter()
                .any(|s| s.contains("secret durable task"))
        );
        if self.refuse {
            return Err(ApiError::new(ErrorCode::TargetUnsafe, "occupied").into());
        }
        self.submitted_argv = request.argv.clone();
        gate(true)?;
        if self.confirmed_refusal {
            gate(false)?;
            return Err(ApiError::new(
                ErrorCode::TargetUnsafe,
                "confirmed native prestart refusal",
            )
            .into());
        }
        assert!(!self.panic_after_gate, "crash after persisted gate");
        self.starts += 1;
        assert!(
            !self.panic_after_start,
            "crash after native start before result persistence"
        );
        Ok(super::super::launch::LaunchReport {
            report: serde_json::json!({"outcome":if self.unknown {"outcome_unknown"} else {"started"}}),
            exit: if self.unknown { 5 } else { 0 },
        })
    }
}
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn journal() -> (Temp, Journal) {
    let temp = Temp(std::env::temp_dir().join(format!("handoff-test-{}", uuid::Uuid::new_v4())));
    let journal = Journal::open(temp.0.join("intents")).unwrap();
    (temp, journal)
}
fn reference(journal: &Journal) -> IntentRef {
    journal.resolve_recovery_ref("local:1").unwrap()
}
fn scope() -> IntentScope {
    IntentScope::Cooperative {
        instance: "00000000-0000-0000-0000-0000000000b1".into(),
        seat: SeatId::new("sender"),
    }
}
#[test]
fn handoff_exact_keys_replay_lost_responses_each_durable_phase() {
    for phase in ["create", "invite", "send"] {
        let (_temp, journal) = journal();
        let client = Client::new(Some(phase));
        let mut launcher = Launcher::default();
        let mut output = Vec::new();
        assert!(
            start(
                &journal,
                request(),
                claim(),
                "bob",
                &client,
                &mut launcher,
                &TestClock,
                &OutputSpec::default(),
                &mut output
            )
            .is_err()
        );
        assert_eq!(launcher.starts, 0);
        let reference = reference(&journal);
        resume(
            &journal,
            &reference,
            &scope(),
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut output,
        )
        .unwrap();
        assert_eq!(client.results.lock().unwrap().len(), 3);
        assert_eq!(launcher.starts, 1);
        assert!(
            !String::from_utf8(output)
                .unwrap()
                .contains("secret durable task")
        );
    }
}
#[test]
fn handoff_possible_start_crash_and_unknown_never_start_twice() {
    for crash in [false, true] {
        let (_temp, journal) = journal();
        let client = Client::new(None);
        let mut launcher = Launcher {
            unknown: !crash,
            panic_after_gate: crash,
            ..Default::default()
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            start(
                &journal,
                request(),
                claim(),
                "bob",
                &client,
                &mut launcher,
                &TestClock,
                &OutputSpec::default(),
                &mut Vec::new(),
            )
        }));
        let starts = launcher.starts;
        let result = resume(
            &journal,
            &reference(&journal),
            &scope(),
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        );
        assert!(matches!(result, Err(RunError::Exit(5))));
        assert_eq!(launcher.starts, starts);
        assert_eq!(client.calls.lock().unwrap().len(), 3);
    }
}
#[test]
fn handoff_pre_start_refusal_retains_work_and_rechecks_launch() {
    let (_temp, journal) = journal();
    let client = Client::new(None);
    let mut launcher = Launcher {
        refuse: true,
        ..Default::default()
    };
    assert!(
        start(
            &journal,
            request(),
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    launcher.refuse = false;
    resume(
        &journal,
        &reference(&journal),
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(client.calls.lock().unwrap().len(), 3);
    assert_eq!(launcher.starts, 1);
}
struct Broken;
impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn handoff_broken_output_replays_report_without_relaunch_or_wrong_scope() {
    let (_temp, journal) = journal();
    let client = Client::new(None);
    let mut launcher = Launcher::default();
    assert!(
        start(
            &journal,
            request(),
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Broken
        )
        .is_err()
    );
    let reference = reference(&journal);
    let wrong = IntentScope::Cooperative {
        instance: "00000000-0000-0000-0000-0000000000b1".into(),
        seat: SeatId::new("other"),
    };
    assert!(
        resume(
            &journal,
            &reference,
            &wrong,
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    resume(
        &journal,
        &reference,
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(launcher.starts, 1);
    assert_eq!(client.calls.lock().unwrap().len(), 3);
}
#[test]
fn handoff_validation_precedes_native_and_durable_work() {
    let (_temp, journal) = journal();
    let client = Client::new(None);
    let mut launcher = Launcher::default();
    let mut bad = request();
    bad.body.clear();
    assert!(
        start(
            &journal,
            bad,
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    assert!(client.calls.lock().unwrap().is_empty());
    assert_eq!(
        launcher.preflights, 0,
        "invalid body must refuse before host/seat preflight"
    );
}

#[test]
fn handoff_confirmed_native_refusal_resets_fence_for_guarded_retry() {
    let (_temp, journal) = journal();
    let client = Client::new(None);
    let mut launcher = Launcher {
        confirmed_refusal: true,
        ..Default::default()
    };
    let first = start(
        &journal,
        request(),
        claim(),
        "bob",
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    );
    assert!(matches!(first, Err(RunError::Api(_))));
    let reference = reference(&journal);
    assert!(!load(&journal, &reference).unwrap().possible_start);
    launcher.confirmed_refusal = false;
    resume(
        &journal,
        &reference,
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(launcher.starts, 1);
    assert_eq!(client.calls.lock().unwrap().len(), 3);
}

#[test]
fn handoff_pending_page_lists_compound_without_payload_and_uses_private_files() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, journal) = journal();
    let client = Client::new(None);
    let mut launcher = Launcher {
        refuse: true,
        ..Default::default()
    };
    assert!(
        start(
            &journal,
            request(),
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    let page = journal
        .page_with_output(
            &Default::default(),
            &["--json".into(), "pending-ops".into()],
            &OutputSpec::default(),
        )
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].kind,
        crate::protocol::results::IntentKind::Handoff
    );
    assert!(
        !serde_json::to_string(&page)
            .unwrap()
            .contains("secret durable task")
    );
    for entry in fs::read_dir(journal.root()).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().permissions().mode() & 0o077,
            0
        );
    }
}

#[test]
fn handoff_existing_thread_requires_joined_sender_and_freezes_id_on_replay() {
    for joined in [false, true] {
        let (_temp, journal) = journal();
        let mut client = Client::new(Some("send"));
        client.joined = joined;
        client.already_joined = true;
        let mut launcher = Launcher::default();
        let mut req = request();
        req.thread = Some(ThreadId::new("t1"));
        req.thread_name = None;
        req.topic = None;
        req.goal = None;
        let result = start(
            &journal,
            req,
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        );
        if !joined {
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: ErrorCode::MembershipRequired,
                    ..
                }))
            ));
            assert_eq!(launcher.preflights, 0);
            assert!(client.calls.lock().unwrap().is_empty());
            continue;
        }
        assert!(result.is_err());
        resume(
            &journal,
            &reference(&journal),
            &scope(),
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(client.results.lock().unwrap().len(), 2);
        for command in client.calls.lock().unwrap().iter() {
            match command {
                Command::Invite(q) => assert_eq!(q.thread.as_str(), "t1"),
                Command::SendMessage(q) => assert_eq!(q.thread.as_str(), "t1"),
                _ => panic!("existing handoff created a new thread"),
            }
        }
    }
}
#[test]
fn handoff_crash_after_native_start_before_result_never_repeats_start() {
    let (_temp, journal) = journal();
    let client = Client::new(None);
    let mut launcher = Launcher {
        panic_after_start: true,
        ..Default::default()
    };
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        start(
            &journal,
            request(),
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        )
    }));
    assert!(crash.is_err());
    assert_eq!(launcher.starts, 1);
    assert!(matches!(
        resume(
            &journal,
            &reference(&journal),
            &scope(),
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new()
        ),
        Err(RunError::Exit(5))
    ));
    assert_eq!(launcher.starts, 1);
}

// The launched recipient must use its own startup proof, while journal replay
// retains the sender's exact durable target and the frozen task channel.
#[test]
fn handoff_bootstrap_prefers_recipient_hook_commands_and_replays_exact_fallback() {
    let (_temp, journal) = journal();
    let client = Client::new(Some("send"));
    let mut launcher = Launcher::default();
    let pinned = OutputSpec {
        context: crate::protocol::output::ContinuationContext {
            state_dir: Some("/tmp/handoff state".into()),
            host: Some("/tmp/handoff.sock".into()),
        },
        ..OutputSpec::default()
    };
    assert!(
        start(
            &journal,
            request(),
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &pinned,
            &mut Vec::new(),
        )
        .is_err()
    );
    let reference = reference(&journal);
    let before = journal.load(&reference).unwrap();
    resume(
        &journal,
        &reference,
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    let prompt = launcher.submitted_argv.last().unwrap();
    assert!(
        prompt.starts_with("Expected handoff command routing (JSON data): "),
        "{prompt}"
    );
    assert!(
        prompt
            .split_once(" Prefer a startup hook")
            .unwrap()
            .0
            .ends_with("null"),
        "unknown paths require exact fallback"
    );
    let fallback = prompt
        .split_once("Otherwise use the exact fallback:")
        .unwrap()
        .1;
    assert!(
        fallback.contains(
            "herdr-threads --state-dir '/tmp/handoff state' --host-endpoint /tmp/handoff.sock inbox"
        ),
        "{fallback}"
    );
    assert!(!fallback.contains(" read t1"), "{fallback}");
    assert!(prompt.contains("task for thread t1 is stored in inbox"));
    assert!(!prompt.contains("secret durable task"));
    assert!(
        matches!(before.semantic, SemanticMutation::Frozen { mutation, .. }
        if matches!(mutation.as_ref(), SemanticMutation::Handoff(plan) if plan.context == pinned.context))
    );
}

// A handoff's expected routing is derived from its frozen claim and durable
// pair, never from whichever startup hook happens to emit ordinary commands.
#[test]
fn handoff_bootstrap_binds_hook_preference_to_frozen_expected_instance_and_pair() {
    let (_temp, journal) = journal();
    let state = journal.root().parent().unwrap().join("state");
    fs::create_dir(&state).unwrap();
    let host = state.parent().unwrap().join("herdr.sock");
    let context = crate::protocol::output::ContinuationContext {
        state_dir: Some(state.display().to_string()),
        host: Some(host.display().to_string()),
    };
    let client = Client::new(Some("send"));
    let mut launcher = Launcher::default();
    let output = OutputSpec {
        context: context.clone(),
        ..OutputSpec::default()
    };
    assert!(
        start(
            &journal,
            request(),
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &output,
            &mut Vec::new()
        )
        .is_err()
    );
    resume(
        &journal,
        &reference(&journal),
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    let prompt = launcher.submitted_argv.last().unwrap();
    let record = prompt
        .strip_prefix("Expected handoff command routing (JSON data): ")
        .and_then(|record| {
            record
                .split_once(" Prefer a startup hook")
                .map(|(routing, _)| routing)
        })
        .expect("bootstrap must identify its expected hook group");
    let expected: serde_json::Value = serde_json::from_str(record).unwrap();
    assert_eq!(
        expected,
        serde_json::json!({"instance":"00000000-0000-0000-0000-0000000000b1", "state_dir":state.display().to_string(), "host_endpoint":host.display().to_string()})
    );
}
struct FencedClient {
    inner: Client,
    terminal: std::sync::atomic::AtomicBool,
    events: std::sync::Arc<Mutex<Vec<&'static str>>>,
}
impl LocalClient for FencedClient {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
        use crate::protocol::handoff::{HandoffResult, HandoffState};
        match c {
            Command::BeginHandoff(q) => {
                self.events.lock().unwrap().push("begin");
                Ok(CommandResult::Handoff(HandoffResult {
                    compound: q.identity.compound,
                    thread: Some(ThreadId::new("t1")),
                    state: if self.terminal.load(std::sync::atomic::Ordering::Relaxed) {
                        HandoffState::Completed
                    } else {
                        HandoffState::Live
                    },
                }))
            }
            Command::CompleteHandoff(q) => {
                let mut events = self.events.lock().unwrap();
                assert_eq!(events.last(), Some(&"flush"));
                events.push("complete");
                self.terminal
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(CommandResult::Handoff(HandoffResult {
                    compound: q.identity.compound,
                    thread: Some(ThreadId::new("t1")),
                    state: HandoffState::Completed,
                }))
            }
            c => {
                assert_eq!(
                    self.events.lock().unwrap().first(),
                    Some(&"begin"),
                    "durable Begin precedes child effects"
                );
                self.inner.call(c, b)
            }
        }
    }
}
struct Output {
    events: std::sync::Arc<Mutex<Vec<&'static str>>>,
    fail_flush: bool,
}
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("flush failed"));
        }
        self.events.lock().unwrap().push("flush");
        Ok(())
    }
}
fn selected_context(journal: &Journal) -> crate::protocol::output::ContinuationContext {
    let root = journal.root().parent().unwrap();
    crate::protocol::output::ContinuationContext {
        state_dir: Some(root.to_string_lossy().into_owned()),
        host: Some(root.join("host.sock").to_string_lossy().into_owned()),
    }
}
fn frozen(journal: &Journal) -> IntentRef {
    frozen_in(journal, selected_context(journal))
}
fn frozen_in(
    journal: &Journal,
    context: crate::protocol::output::ContinuationContext,
) -> IntentRef {
    journal
        .record(
            scope(),
            SemanticMutation::freeze(
                SemanticMutation::Handoff(Box::new(HandoffPlan {
                    request: request(),
                    context,
                    recipient: SeatId::new("recipient"),
                    create_key: OperationId::new("create"),
                    invite_key: OperationId::new("invite"),
                    send_key: OperationId::new("send"),
                })),
                claim(),
            )
            .unwrap(),
            0,
        )
        .unwrap()
}
#[test]
fn handoff_fenced_begin_precedes_effects_complete_follows_successful_flush() {
    for fail_flush in [true, false] {
        let (_temp, journal) = journal();
        let reference = frozen(&journal);
        let events = std::sync::Arc::new(Mutex::new(vec![]));
        let client = FencedClient {
            inner: Client::new(None),
            terminal: false.into(),
            events: events.clone(),
        };
        let mut launcher = Launcher::default();
        let mut output = Output { events, fail_flush };
        let result = resume(
            &journal,
            &reference,
            &scope(),
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut output,
        );
        if fail_flush {
            assert!(result.is_err());
            assert!(!client.terminal.load(std::sync::atomic::Ordering::Relaxed));
            assert!(journal.load(&reference).is_ok());
        } else {
            result.unwrap();
            assert!(client.terminal.load(std::sync::atomic::Ordering::Relaxed));
            assert!(journal.load(&reference).is_err());
        }
    }
}
#[test]
fn handoff_fenced_terminal_retry_only_reprints_and_cleans_retained_intent() {
    let (_temp, journal) = journal();
    let reference = frozen(&journal);
    let events = std::sync::Arc::new(Mutex::new(vec![]));
    let client = FencedClient {
        inner: Client::new(None),
        terminal: true.into(),
        events: events.clone(),
    };
    let mut launcher = Launcher::default();
    let mut output = Output {
        events,
        fail_flush: false,
    };
    save(
        &journal,
        &reference,
        &Progress {
            thread: Some(ThreadId::new("t1")),
            launch: Some(serde_json::json!({"outcome":"started"})),
            ..Default::default()
        },
    )
    .unwrap();
    resume(
        &journal,
        &reference,
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap();
    assert!(client.inner.calls.lock().unwrap().is_empty());
    assert_eq!(launcher.starts, 0);
    assert_eq!(&*client.events.lock().unwrap(), &["begin", "flush"]);
    assert!(journal.load(&reference).is_err());
}
#[test]
fn handoff_fenced_early_terminal_retry_uses_frozen_selection_without_live_inspection() {
    for pane in ["w1:p1", "w9:p9"] {
        let (_temp, journal) = journal();
        let reference = frozen(&journal);
        let events = std::sync::Arc::new(Mutex::new(vec![]));
        let client = FencedClient {
            inner: Client::new(None),
            terminal: true.into(),
            events: events.clone(),
        };
        let mut output = Output {
            events,
            fail_flush: false,
        };
        save(
            &journal,
            &reference,
            &Progress {
                thread: Some(ThreadId::new("t1")),
                launch: Some(serde_json::json!({"outcome":"started"})),
                ..Default::default()
            },
        )
        .unwrap();
        let parsed = crate::cli::commands::parse_argv(["ht", "retry", "local:1"]).unwrap();
        let result = try_completed_retry(
            &parsed,
            &journal,
            &claim().instance,
            Some(pane),
            &selected_context(&journal),
            &client,
            &TestClock,
            &mut output,
        );
        if pane == "w1:p1" {
            assert!(
                result.unwrap(),
                "terminal cleanup must precede current SeatInspect/generation guards"
            );
            assert!(journal.load(&reference).is_err());
        } else {
            assert!(result.is_err());
            assert!(client.events.lock().unwrap().is_empty());
            assert!(journal.load(&reference).is_ok());
        }
    }
}
#[test]
fn handoff_fenced_early_live_alias_defers_without_calls_or_refusal() {
    let (_temp, journal) = journal();
    let reference = frozen(&journal);
    let events = std::sync::Arc::new(Mutex::new(vec![]));
    let client = FencedClient {
        inner: Client::new(None),
        terminal: false.into(),
        events: events.clone(),
    };
    let mut output = Output {
        events,
        fail_flush: false,
    };
    let mut parsed = crate::cli::commands::parse_argv(["ht", "retry", "local:1"]).unwrap();
    parsed.cooperative = Some(crate::cli::commands::CooperativeSelection {
        seat: claim().seat,
        target: HostTargetId::new("worker-name"),
        harness: crate::harness::context::Harness::Human,
        role: crate::harness::context::Role::TopLevel,
    });
    parsed.cooperative_selector = Some(crate::cli::panes::PaneSelector {
        pane: Some("worker-name".into()),
        ..Default::default()
    });
    assert!(
        !try_completed_retry(
            &parsed,
            &journal,
            &claim().instance,
            None,
            &selected_context(&journal),
            &client,
            &TestClock,
            &mut output
        )
        .expect("unresolved locator must reach normal pane resolver")
    );
    assert!(client.events.lock().unwrap().is_empty());
    assert!(journal.load(&reference).is_ok());
}
#[test]
fn handoff_fenced_early_namespace_alias_is_canonical_but_copied_uuid_is_not() {
    for foreign in [false, true] {
        let (temp, journal) = journal();
        let alias = temp.0.join("alias");
        std::os::unix::fs::symlink(&temp.0, &alias).unwrap();
        let context = crate::protocol::output::ContinuationContext {
            state_dir: Some(alias.to_string_lossy().into_owned()),
            host: Some(alias.join("host.sock").to_string_lossy().into_owned()),
        };
        let reference = frozen_in(&journal, context);
        let mut current = selected_context(&journal);
        if foreign {
            let root = temp.0.join("other");
            fs::create_dir(&root).unwrap();
            current.state_dir = Some(root.to_string_lossy().into_owned());
        }
        let events = std::sync::Arc::new(Mutex::new(vec![]));
        let client = FencedClient {
            inner: Client::new(None),
            terminal: true.into(),
            events: events.clone(),
        };
        let mut output = Output {
            events,
            fail_flush: false,
        };
        save(
            &journal,
            &reference,
            &Progress {
                thread: Some(ThreadId::new("t1")),
                launch: Some(serde_json::json!({"outcome":"started"})),
                ..Default::default()
            },
        )
        .unwrap();
        let parsed = crate::cli::commands::parse_argv(["ht", "retry", "local:1"]).unwrap();
        let result = try_completed_retry(
            &parsed,
            &journal,
            &claim().instance,
            Some("w1:p1"),
            &current,
            &client,
            &TestClock,
            &mut output,
        );
        if foreign {
            assert!(result.is_err());
            assert!(client.events.lock().unwrap().is_empty());
            assert!(journal.load(&reference).is_ok());
        } else {
            assert!(result.expect("canonical alias selects same full instance pair"));
            assert!(journal.load(&reference).is_err());
        }
    }
}
#[test]
fn handoff_fenced_complete_survives_local_removal_failure_then_cleanup_only_retry() {
    use std::os::unix::fs::PermissionsExt;
    struct RemovalFailure {
        inner: Output,
        root: std::path::PathBuf,
    }
    impl Write for RemovalFailure {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.inner.write(b)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()?;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o500))
        }
    }
    impl Drop for RemovalFailure {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700));
        }
    }
    let (_temp, journal) = journal();
    let reference = frozen(&journal);
    let events = std::sync::Arc::new(Mutex::new(vec![]));
    let client = FencedClient {
        inner: Client::new(None),
        terminal: false.into(),
        events: events.clone(),
    };
    let mut launcher = Launcher::default();
    {
        let mut output = RemovalFailure {
            inner: Output {
                events: events.clone(),
                fail_flush: false,
            },
            root: journal.root().to_owned(),
        };
        assert!(
            resume(
                &journal,
                &reference,
                &scope(),
                &client,
                &mut launcher,
                &TestClock,
                &OutputSpec::default(),
                &mut output
            )
            .is_err()
        );
        assert!(
            client.terminal.load(std::sync::atomic::Ordering::Relaxed),
            "Complete commits before failed local removal"
        );
    }
    assert!(journal.load(&reference).is_ok());
    client.inner.calls.lock().unwrap().clear();
    events.lock().unwrap().clear();
    let mut output = Output {
        events: events.clone(),
        fail_flush: false,
    };
    resume(
        &journal,
        &reference,
        &scope(),
        &client,
        &mut launcher,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap();
    assert_eq!(launcher.starts, 1);
    assert!(client.inner.calls.lock().unwrap().is_empty());
    assert_eq!(&*events.lock().unwrap(), &["begin", "flush"]);
    assert!(journal.load(&reference).is_err());
}

#[test]
fn handoff_bootstrap_is_one_native_shell_argument() {
    let prompt = bootstrap(
        &ThreadId::new("tReview01"),
        &crate::protocol::output::ContinuationContext::default(),
        "00000000-0000-0000-0000-0000000000b1",
    );
    assert!(
        !prompt.contains(['\n', '\r']),
        "Herdr rejects native launch arguments containing a line break: {prompt:?}"
    );
    assert!(prompt.contains("inbox") && prompt.contains("task for thread tReview01"));
    assert!(prompt.contains("Launch does not accept invitations or ACK messages"));
}

/// Kills rereading configured arguments after preflight, duplicating them on
/// final launch, or losing them when a durable handoff resumes after refusal.
#[test]
fn handoff_freezes_configured_options_for_preflight_launch_and_retry() {
    for retry in [false, true] {
        let (_temp, journal) = journal();
        let client = Client::new(None);
        let mut launcher = Launcher {
            configured_prefix: vec![
                "--no-daemon".into(),
                "--model".into(),
                "model with spaces".into(),
            ],
            confirmed_refusal: retry,
            ..Default::default()
        };
        let mut request = request();
        request.launch = request
            .launch
            .with_configured_options(Some("--no-daemon --model 'model with spaces'".into()))
            .unwrap();
        let result = start(
            &journal,
            request,
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        );
        let expected = [
            "--no-daemon",
            "--model",
            "model with spaces",
            "-a",
            "on-request",
        ];
        assert_eq!(launcher.preflight_argv, expected);
        assert_eq!(&launcher.submitted_argv[..5], expected);
        if retry {
            assert!(result.is_err());
            let reference = reference(&journal);
            let pending = journal.load(&reference).unwrap();
            let SemanticMutation::Frozen { mutation, .. } = pending.semantic else {
                panic!("expected frozen handoff");
            };
            let SemanticMutation::Handoff(plan) = *mutation else {
                panic!("expected handoff plan");
            };
            assert_eq!(plan.request.launch.argv, expected);
            launcher.confirmed_refusal = false;
            launcher.submitted_argv.clear();
            resume(
                &journal,
                &reference,
                &scope(),
                &client,
                &mut launcher,
                &TestClock,
                &OutputSpec::default(),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(&launcher.submitted_argv[..5], expected);
            assert_eq!(launcher.preflights, 1);
        } else {
            result.unwrap();
        }
        assert_eq!(launcher.starts, 1);
        assert_eq!(
            launcher
                .submitted_argv
                .iter()
                .filter(|arg| *arg == "--no-daemon")
                .count(),
            1
        );
    }
}
