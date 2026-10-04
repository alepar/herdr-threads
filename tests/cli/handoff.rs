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
        instance: "instance".into(),
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
    refuse: bool,
    confirmed_refusal: bool,
    unknown: bool,
    panic_after_gate: bool,
    panic_after_start: bool,
}
impl HandoffLauncher for Launcher {
    fn preflight(&mut self, _: &LaunchRequest) -> Result<SeatId, RunError> {
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
        assert_eq!(&request.argv[..2], ["-a", "on-request"]);
        assert!(
            !request
                .argv
                .iter()
                .any(|s| s.contains("secret durable task"))
        );
        if self.refuse {
            return Err(ApiError::new(ErrorCode::TargetUnsafe, "occupied").into());
        }
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
        instance: "instance".into(),
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
        instance: "instance".into(),
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
