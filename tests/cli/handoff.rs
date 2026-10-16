use super::*;
use crate::protocol::{
    authority::{CallerRole, Harness},
    ids::{ExecutionId, HostTargetId, InvitationId, MessageId, NativeSessionId},
    time::{CallBudget, MonoInstant, UtcMillis},
};
use std::sync::Mutex;

// This fixture exercises the parsed omitted-channel request through the actual
// producer, and refuses every standalone Create/Invite/Accept/ACK route.
struct AutomaticClient {
    body: String,
    supported: bool,
    calls: Mutex<Vec<Command>>,
    thread: Mutex<Option<ThreadId>>,
    sent: Mutex<bool>,
    send_count: Mutex<usize>,
    invitations: Mutex<usize>,
    rejected: Mutex<bool>,
    lose_ensure: Mutex<bool>,
    binding_changed: Mutex<bool>,
    completed: Mutex<bool>,
    lose_complete: Mutex<bool>,
}
impl AutomaticClient {
    fn new() -> Self {
        Self {
            body: "secret durable task".into(),
            supported: true,
            calls: Mutex::new(vec![]),
            thread: Mutex::new(None),
            sent: Mutex::new(false),
            send_count: Mutex::new(0),
            invitations: Mutex::new(0),
            rejected: Mutex::new(false),
            lose_ensure: Mutex::new(false),
            binding_changed: Mutex::new(false),
            completed: Mutex::new(false),
            lose_complete: Mutex::new(false),
        }
    }
}
impl LocalClient for AutomaticClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::Capabilities => Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: if self.supported {
                        vec![
                            crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1.into(),
                            crate::protocol::capabilities::ENSURE_CREATION_POLICY_V1.into(),
                            crate::protocol::capabilities::SEND_TIME_RECEIPTS_V1.into(),
                            crate::protocol::capabilities::INVITATION_CONFIRMATION_V1.into(),
                        ]
                    } else {
                        vec![]
                    },
                },
            )),
            Command::AutomaticHandoffDelivery(request) => {
                use crate::protocol::handoff::AutomaticDeliveryAction;
                if *self.binding_changed.lock().unwrap()
                    && !matches!(request.action, AutomaticDeliveryAction::Status)
                {
                    return Err(ApiError::new(
                        ErrorCode::CallerUnverified,
                        "original binding changed; only history may replay",
                    ));
                }
                assert_eq!(request.plan().recipient.as_str(), "recipient");
                assert_eq!(request.plan().payload.body, self.body);
                let keys = &request.plan().payload.keys;
                let status = |state, thread| {
                    CommandResult::Handoff(crate::protocol::handoff::HandoffResult {
                        compound: keys.compound.clone(),
                        thread,
                        state,
                    })
                };
                match &request.action {
                    AutomaticDeliveryAction::Prepare => {
                        Ok(CommandResult::SeatResolved(SeatId::new("recipient")))
                    }
                    AutomaticDeliveryAction::Status | AutomaticDeliveryAction::Begin { .. } => {
                        Ok(status(
                            if *self.completed.lock().unwrap() {
                                crate::protocol::handoff::HandoffState::Completed
                            } else {
                                crate::protocol::handoff::HandoffState::Live
                            },
                            self.thread.lock().unwrap().clone(),
                        ))
                    }
                    AutomaticDeliveryAction::Ensure(ensure) => {
                        assert_eq!(ensure.operation, keys.ensure);
                        let mut thread = self.thread.lock().unwrap();
                        if thread.is_none() {
                            *self.invitations.lock().unwrap() += 1;
                            *thread = Some(ThreadId::new("t-pair"));
                        }
                        drop(thread);
                        if std::mem::take(&mut *self.lose_ensure.lock().unwrap()) {
                            return Err(ApiError::unknown_outcome("Ensure committed; reply lost"));
                        }
                        if let Some(policy) = &ensure.creation_policy {
                            use crate::protocol::results::{
                                DirectThreadEnsureOutcome, DirectThreadEnsured,
                                DirectThreadInvitation,
                            };
                            Ok(CommandResult::DirectThreadEnsured(DirectThreadEnsured {thread:ThreadId::new("t-pair"),recipient:SeatId::new("recipient"),outcome:DirectThreadEnsureOutcome::Created,
                                invitation:Some(DirectThreadInvitation {id:crate::protocol::ids::InvitationId::new("iv-pair"),episode:1,confirmation:policy.confirmation,
                                    effective_deadline_at:if policy.confirmation==crate::protocol::commands::InvitationConfirmation::RequireAck {Some(UtcMillis(11001))} else {None},inviter:Some(ensure.claim.seat.clone())})}))
                        } else {
                            Ok(CommandResult::ThreadCreated(ThreadId::new("t-pair")))
                        }
                    }
                    AutomaticDeliveryAction::Send(send) => {
                        assert_eq!(send.operation, keys.send);
                        assert_eq!(send.thread.as_str(), "t-pair");
                        assert_eq!(
                            send.direct_recipient.as_ref().unwrap().as_str(),
                            "recipient"
                        );
                        if *self.rejected.lock().unwrap() && !*self.sent.lock().unwrap() {
                            return Err(ApiError::new(
                                ErrorCode::MembershipRequired,
                                "recipient rejected; explicitly reinvite",
                            ));
                        }
                        *self.sent.lock().unwrap() = true;
                        *self.send_count.lock().unwrap() += 1;
                        Ok(CommandResult::MessageSent(MessageId::new("m-pair")))
                    }
                    AutomaticDeliveryAction::Complete { operation } => {
                        assert_eq!(operation, &keys.complete);
                        assert!(*self.sent.lock().unwrap());
                        *self.completed.lock().unwrap() = true;
                        if std::mem::take(&mut *self.lose_complete.lock().unwrap()) {
                            return Err(ApiError::unknown_outcome(
                                "Complete committed; reply lost",
                            ));
                        }
                        Ok(status(
                            crate::protocol::handoff::HandoffState::Completed,
                            Some(ThreadId::new("t-pair")),
                        ))
                    }
                }
            }
            other => panic!("automatic producer used forbidden command: {other:?}"),
        }
    }
}

#[derive(Default)]
struct AutomaticLauncher {
    starts: usize,
    preflights: usize,
    lose_start: bool,
    report_padding: usize,
    report_outcome: Option<&'static str>,
    full_report: Option<serde_json::Value>,
}
impl HandoffLauncher for AutomaticLauncher {
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        Ok(crate::harness::adapter::StartupInputTemplate::positional(
            request.argv.len(),
        ))
    }
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
        assert_eq!(seat.as_str(), "recipient");
        assert!(request.argv.iter().any(|arg| arg.contains("t-pair")));
        gate(true)?;
        self.starts += 1;
        if self.lose_start {
            return Err(ApiError::unknown_outcome("native start submitted; reply lost").into());
        }
        let report = serde_json::json!({"outcome":self.report_outcome.unwrap_or("started"),"pane":request.target,"seat":seat,"harness":"codex","argv":request.argv,"padding":"x".repeat(self.report_padding)});
        self.full_report = Some(report.clone());
        Ok(super::super::launch::LaunchReport { report, exit: 0 })
    }
}

#[test]
fn automatic_launched_oversized_original_refuses_before_recipient_allocation() {
    let body = "\0".repeat(65536);
    let parsed = crate::cli::commands::parse_argv([
        "ht", "handoff", "--pane", "w1:p2", "--kind", "codex", "--", &body,
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(mut request) = parsed.action else {
        panic!("route")
    };
    request.launch.argv = std::iter::once("codex".into())
        .chain((0..6).map(|_| format!("--model={}", "\u{1}".repeat(4088))))
        .collect();
    crate::ports::NativeLaunchRequest::validate_argv(&request.launch.argv).unwrap();
    let (_temp, journal) = journal();
    let client = AutomaticClient::new();
    let mut launcher = AutomaticLauncher::default();
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    assert!(
        start(
            &journal,
            request,
            original_claim,
            "recipient",
            &client,
            &mut launcher,
            &TestClock,
            &output,
            &mut vec![]
        )
        .is_err()
    );
    assert_eq!(
        launcher.preflights, 0,
        "oversized origin reached recipient allocation preflight"
    );
    assert_eq!(launcher.starts, 0);
    assert!(journal.resolve_recovery_ref("local:1").is_err());
    assert!(
        client
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| matches!(c, Command::Capabilities))
    );
}

#[test]
fn automatic_launched_maximum_escaped_body_is_frozen_and_retries_without_relaunch() {
    let body = "\0".repeat(65536);
    let parsed = crate::cli::commands::parse_argv([
        "ht", "handoff", "--pane", "w1:p2", "--kind", "codex", "--", &body,
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("route")
    };
    let (_temp, journal) = journal();
    let mut client = AutomaticClient::new();
    client.body = body.clone();
    let mut launcher = AutomaticLauncher::default();
    let mut output = OutputSpec {
        format: crate::protocol::output::OutputFormat::Json,
        ..OutputSpec::default()
    };
    let mut presentation = Vec::new();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let result = start(
        &journal,
        request,
        original_claim,
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut presentation,
    );
    assert!(
        result.is_ok(),
        "maximum escaped launched body failed: {result:?}"
    );
    let report: serde_json::Value = serde_json::from_slice(&presentation).unwrap();
    assert!(
        report["handoff"]["launch"]["argv"].is_null(),
        "ordinary success dumped retained native argv"
    );
    assert_eq!(report["handoff"]["selection"]["outcome"], "created");
    assert_eq!(
        report["handoff"]["local_diagnostics"]["state"],
        "local_file"
    );
    assert_eq!(
        report["handoff"]["local_diagnostics"]["authority"],
        "local_hint"
    );
    let calls = client.calls.lock().unwrap();
    let send = calls
        .iter()
        .find_map(|c| match c {
            Command::AutomaticHandoffDelivery(q) => match &q.action {
                crate::protocol::handoff::AutomaticDeliveryAction::Send(v) => Some(v),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(send.body, body);
    drop(calls);
    let reference =
        super::super::handoff_delivery::resolve_automatic_recovery_ref(&journal, "local:1")
            .unwrap();
    super::super::handoff_delivery::retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &crate::protocol::handoff::HandoffNamespace {
            instance: claim().instance,
            state_dir: "/tmp/automatic-state".into(),
            host_endpoint: "/tmp/automatic-host.sock".into(),
        },
        &client,
        Some(&mut launcher),
        &TestClock,
        &output,
        &mut vec![],
    )
    .unwrap();
    assert_eq!(launcher.starts, 1);
}

#[test]
fn automatic_launched_noncompletion_keeps_exact_artifact_and_bounded_progress() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("route")
    };
    let (_temp, journal) = journal();
    let client = AutomaticClient::new();
    let mut launcher = AutomaticLauncher {
        report_padding: 3 * 1024 * 1024,
        report_outcome: Some("outcome_unknown"),
        ..Default::default()
    };
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    assert!(
        start(
            &journal,
            request,
            original_claim,
            "recipient",
            &client,
            &mut launcher,
            &TestClock,
            &output,
            &mut vec![]
        )
        .is_err()
    );
    let reference = journal.resolve_recovery_ref("local:1").unwrap();
    let path = progress_path(&journal, &reference);
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() < 2 * 1024 * 1024);
    let progress: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        progress["launch"]["format"],
        "automatic_launch_noncompletion_v1"
    );
    assert_eq!(progress["launch"]["observed_started"], false);
    assert_eq!(progress["possible_start"], true);
    let artifact = std::fs::read(automatic_diagnostics_path(&journal, &reference)).unwrap();
    let expected = serde_json::to_vec(launcher.full_report.as_ref().unwrap()).unwrap();
    assert_eq!(artifact, expected);
    use sha2::{Digest, Sha256};
    assert_eq!(
        progress["launch"]["full_report_digest"],
        format!("{:x}", Sha256::digest(&artifact))
    );
    assert!(
        super::super::handoff_delivery::retry_automatic_to_writer(
            &journal,
            &reference,
            super::super::actor_route::InvocationActor::Agent,
            &crate::protocol::handoff::HandoffNamespace {
                instance: claim().instance,
                state_dir: "/tmp/automatic-state".into(),
                host_endpoint: "/tmp/automatic-host.sock".into()
            },
            &client,
            Some(&mut launcher),
            &TestClock,
            &output,
            &mut vec![]
        )
        .is_err()
    );
    assert_eq!(launcher.starts, 1);
    assert!(!*client.completed.lock().unwrap());
}

#[test]
fn automatic_launched_pane_uses_ensure_send_and_one_start() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong parser route")
    };
    assert!(request.automatic);
    let (_temp, journal) = journal();
    let client = AutomaticClient::new();
    let mut launcher = AutomaticLauncher::default();
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut bytes = vec![];
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let result = start(
        &journal,
        request,
        original_claim,
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut bytes,
    );
    assert!(result.is_ok(), "automatic launch failed: {result:?}");
    assert_eq!(launcher.starts, 1);
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|c| matches!(c, Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Ensure(_)))).count(), 1);
    assert_eq!(calls.iter().filter(|c| matches!(c, Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Send(_)))).count(), 1);
    assert!(!bytes.is_empty());
}

#[test]
fn automatic_launched_pane_lost_ensure_then_reject_preserves_original_without_send_or_start() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong parser route")
    };
    assert!(request.automatic);
    let (_temp, journal) = journal();
    let client = AutomaticClient::new();
    *client.lose_ensure.lock().unwrap() = true;
    let mut launcher = AutomaticLauncher::default();
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let mut first_output = vec![];
    let first = start(
        &journal,
        request,
        original_claim.clone(),
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut first_output,
    );
    assert!(
        matches!(first, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{first:?}"
    );
    assert!(!first_output.is_empty(), "lost Ensure needs retry guidance");
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert_eq!(*client.send_count.lock().unwrap(), 0);
    assert_eq!(launcher.starts, 0);

    let reference =
        crate::cli::handoff_delivery::resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    let original =
        crate::cli::handoff_delivery::load_automatic_original(&journal, &reference).unwrap();
    let original_bytes = serde_json::to_vec(&original.semantic).unwrap();
    let original_digest = original.header.semantic_digest.clone();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: original_claim.instance.clone(),
        state_dir: "/tmp/automatic-state".into(),
        host_endpoint: "/tmp/automatic-host.sock".into(),
    };
    *client.rejected.lock().unwrap() = true;
    let mut retry_output = vec![];
    let retry = retry_automatic_live(
        &journal,
        "local:1",
        &original_claim,
        crate::cli::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut retry_output,
    );
    assert!(
        matches!(retry, Err(RunError::Api(ref e)) if e.code == ErrorCode::MembershipRequired && e.detail.contains("reinvite")),
        "{retry:?}"
    );
    assert!(
        !retry_output.is_empty(),
        "rejection needs recovery guidance"
    );
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert_eq!(*client.send_count.lock().unwrap(), 0);
    assert_eq!(launcher.starts, 0);
    let retained =
        crate::cli::handoff_delivery::load_automatic_original(&journal, &reference).unwrap();
    assert_eq!(retained.header.semantic_digest, original_digest);
    assert_eq!(
        serde_json::to_vec(&retained.semantic).unwrap(),
        original_bytes
    );
    assert!(
        String::from_utf8(original_bytes)
            .unwrap()
            .contains("secret durable task")
    );
    assert!(journal.resolve_recovery_ref("local:1").is_ok());
    assert!(client.calls.lock().unwrap().iter().all(|call| !matches!(
        call,
        Command::Invite(_) | Command::Accept(_) | Command::Ack(_)
    )));
}

#[test]
fn automatic_launched_pane_unsupported_refuses_before_journal_and_start() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let (_temp, journal) = journal();
    let mut client = AutomaticClient::new();
    client.supported = false;
    let mut launcher = AutomaticLauncher::default();
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let result = start(
        &journal,
        request,
        original_claim,
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut vec![],
    );
    assert!(
        matches!(result, Err(RunError::Api(ref e)) if e.code == ErrorCode::Unsupported),
        "{result:?}"
    );
    assert_eq!(launcher.starts, 0);
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert!(journal.resolve_recovery_ref("local:1").is_err());
}

#[test]
fn automatic_launched_pane_lost_start_never_resubmits_on_retry() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let (_temp, journal) = journal();
    let client = AutomaticClient::new();
    let mut launcher = AutomaticLauncher {
        lose_start: true,
        ..Default::default()
    };
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let mut first_bytes = vec![];
    let first = start(
        &journal,
        request,
        original_claim,
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut first_bytes,
    );
    assert!(
        matches!(first, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{first:?}"
    );
    assert_eq!(launcher.starts, 1);
    assert!(!first_bytes.is_empty());
    let reference =
        crate::cli::handoff_delivery::resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    launcher.lose_start = false;
    let retry = crate::cli::handoff_delivery::retry_automatic_to_writer(
        &journal,
        &reference,
        crate::cli::actor_route::InvocationActor::Agent,
        &crate::protocol::handoff::HandoffNamespace {
            instance: claim().instance,
            state_dir: "/tmp/automatic-state".into(),
            host_endpoint: "/tmp/automatic-host.sock".into(),
        },
        &client,
        Some(&mut launcher),
        &TestClock,
        &output,
        &mut vec![],
    );
    assert!(
        matches!(retry, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{retry:?}"
    );
    assert_eq!(launcher.starts, 1);
}

#[test]
fn automatic_launched_pane_lost_complete_retries_terminal_without_relaunch() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let (_temp, journal) = journal();
    let client = AutomaticClient::new();
    *client.lose_complete.lock().unwrap() = true;
    let mut launcher = AutomaticLauncher::default();
    let mut output = OutputSpec::default();
    output.context.state_dir = Some("/tmp/automatic-state".into());
    output.context.host = Some("/tmp/automatic-host.sock".into());
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let first = start(
        &journal,
        request,
        original_claim,
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut vec![],
    );
    assert!(
        matches!(first, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{first:?}"
    );
    assert_eq!(launcher.starts, 1);
    let reference =
        crate::cli::handoff_delivery::resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    let report = crate::cli::handoff_delivery::retry_automatic_to_writer(
        &journal,
        &reference,
        crate::cli::actor_route::InvocationActor::Agent,
        &crate::protocol::handoff::HandoffNamespace {
            instance: claim().instance,
            state_dir: "/tmp/automatic-state".into(),
            host_endpoint: "/tmp/automatic-host.sock".into(),
        },
        &client,
        None,
        &TestClock,
        &output,
        &mut vec![],
    )
    .unwrap();
    assert_eq!(report["outcome"], "started");
    assert_eq!(launcher.starts, 1);
}

#[test]
fn automatic_launched_pane_live_retry_requires_original_current_caller() {
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--",
        "secret durable task",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong parser route")
    };
    let (temp, journal) = journal();
    let selected = crate::daemon::paths::RuntimeContext::explicit(
        temp.0.clone(),
        temp.0.join("host.sock"),
        None,
    )
    .unwrap();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: claim().instance,
        state_dir: selected.state_dir.clone(),
        host_endpoint: selected.host_endpoint.clone(),
    };
    let mut output = OutputSpec::default();
    output.context.state_dir = Some(selected.state_dir.to_string_lossy().into_owned());
    output.context.host = Some(selected.host_endpoint.to_string_lossy().into_owned());
    let client = AutomaticClient::new();
    *client.lose_ensure.lock().unwrap() = true;
    let mut launcher = AutomaticLauncher::default();
    let mut original_claim = claim();
    original_claim.harness = Harness::Codex;
    let first = start(
        &journal,
        request,
        original_claim.clone(),
        "recipient",
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut vec![],
    );
    assert!(
        matches!(first, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{first:?}"
    );
    assert_eq!(*client.send_count.lock().unwrap(), 0);
    assert_eq!(launcher.starts, 0);
    assert!(journal.resolve_recovery_ref("local:1").is_ok());

    let mut other_current = original_claim.clone();
    other_current.seat = SeatId::new("other-current-seat");
    other_current.target = HostTargetId::new("w1:p3");
    let before = client.calls.lock().unwrap().len();
    let mut rejected_output = vec![];
    let rejected = retry_automatic_live(
        &journal,
        "local:1",
        &other_current,
        crate::cli::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut rejected_output,
    );
    assert!(
        matches!(rejected, Err(RunError::Api(ref e)) if e.code == ErrorCode::InvalidRequest),
        "{rejected:?}"
    );
    assert_eq!(client.calls.lock().unwrap().len(), before);
    assert_eq!(*client.send_count.lock().unwrap(), 0);
    assert_eq!(launcher.starts, 0);
    assert!(rejected_output.is_empty());
    assert!(journal.resolve_recovery_ref("local:1").is_ok());

    let mut changed_binding = original_claim.clone();
    changed_binding.binding_generation += 1;
    let rejected_binding = retry_automatic_live(
        &journal,
        "local:1",
        &changed_binding,
        crate::cli::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut vec![],
    );
    assert!(
        matches!(rejected_binding, Err(RunError::Api(ref e)) if e.code == ErrorCode::InvalidRequest),
        "{rejected_binding:?}"
    );
    assert_eq!(client.calls.lock().unwrap().len(), before);
    assert_eq!(*client.send_count.lock().unwrap(), 0);
    assert_eq!(launcher.starts, 0);

    let resumed = retry_automatic_live(
        &journal,
        "local:1",
        &original_claim,
        crate::cli::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        &mut launcher,
        &TestClock,
        &output,
        &mut vec![],
    );
    assert!(matches!(resumed, Ok(true)), "{resumed:?}");
    assert_eq!(*client.send_count.lock().unwrap(), 1);
    assert_eq!(launcher.starts, 1);

    // Public dispatch probes completed history before selecting a current seat.
    *client.binding_changed.lock().unwrap() = true;
    let mut parsed_retry = crate::cli::commands::parse_argv(["ht", "retry", "local:1"]).unwrap();
    parsed_retry.output = output;
    let mut historical = vec![];
    assert!(
        crate::cli::topology_runtime::try_completed(
            &parsed_retry,
            &journal,
            uuid::Uuid::parse_str(&namespace.instance).unwrap(),
            &selected,
            &client,
            &TestClock,
            &mut historical,
        )
        .unwrap()
    );
    assert!(!historical.is_empty());
    assert_eq!(*client.send_count.lock().unwrap(), 1);
    assert_eq!(launcher.starts, 1);
}
struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

#[test]
fn task48_uncertain_retry_never_begins_or_launches() {
    let mut observed = vec![];
    for (label, bytes) in [
        ("malformed", "{"),
        ("possible-start", "{\"possible_start\":true}"),
        (
            "started-without-thread",
            "{\"possible_start\":false,\"launch\":{\"outcome\":\"started\"}}",
        ),
        (
            "partial-success",
            "{\"thread\":\"t1\",\"possible_start\":false,\"launch\":{\"outcome\":\"started\"},\"invitation\":{\"kind\":\"invitation\",\"data\":\"i1\"}}",
        ),
        (
            "nonobject-launch",
            "{\"thread\":\"t1\",\"possible_start\":false,\"launch\":\"started\"}",
        ),
    ] {
        let (_temp, journal) = journal();
        let reference = frozen(&journal);
        std::fs::write(progress_path(&journal, &reference), bytes).unwrap();
        let events = std::sync::Arc::new(Mutex::new(vec![]));
        let client = FencedClient {
            inner: Client::new(None),
            terminal: false.into(),
            events: events.clone(),
        };
        let mut launcher = Launcher::default();
        let before = std::fs::read(progress_path(&journal, &reference)).unwrap();
        let mut writer = Output {
            events: events.clone(),
            fail_flush: false,
        };
        let result = resume(
            &journal,
            &reference,
            &scope(),
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut writer,
        );
        eprintln!(
            "task48 uncertain {label}: result={result:?} events={:?} child_calls={} preflights={} starts={}",
            events.lock().unwrap(),
            client.inner.calls.lock().unwrap().len(),
            launcher.preflights,
            launcher.starts
        );
        observed.push((
            label,
            result,
            events.lock().unwrap().clone(),
            client.inner.calls.lock().unwrap().len(),
            launcher.preflights,
            launcher.starts,
            std::fs::read(progress_path(&journal, &reference)).unwrap() == before,
            journal.load(&reference).is_ok(),
        ));
    }
    for (label, result, events, child_calls, preflights, starts, unchanged, retained) in observed {
        assert!(
            result.is_err(),
            "{label}: uncertainty remains handled pending"
        );
        assert!(
            events.iter().all(|event| *event == "flush"),
            "{label}: no canonical mutation"
        );
        assert_eq!(child_calls, 0, "{label}: no child mutation");
        assert_eq!((preflights, starts), (0, 0), "{label}: no current runtime");
        assert!(unchanged, "{label}: retained progress bytes unchanged");
        assert!(retained, "{label}: intent retained");
    }
}

#[test]
fn task48_progress_unknown_and_oversized_tails_fail_before_begin() {
    for bytes in [
        br#"{"possible_start":false,"unrecognized":true}"#.to_vec(),
        {
            let mut bytes = br#"{"possible_start":false}"#.to_vec();
            bytes.resize(4 * 1024 * 1024, b' ');
            bytes.push(b'{');
            bytes
        },
    ] {
        let (_temp, journal) = journal();
        let reference = frozen(&journal);
        std::fs::write(progress_path(&journal, &reference), &bytes).unwrap();
        assert!(retained_retry_phase(&journal, &reference).is_err());
        let events = std::sync::Arc::new(Mutex::new(vec![]));
        let client = FencedClient {
            inner: Client::new(None),
            terminal: false.into(),
            events: events.clone(),
        };
        let result = resume(
            &journal,
            &reference,
            &scope(),
            &client,
            &mut RetainedOnlyLauncher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        );
        assert!(result.is_err());
        assert!(events.lock().unwrap().is_empty());
        assert!(client.inner.calls.lock().unwrap().is_empty());
        assert_eq!(
            std::fs::read(progress_path(&journal, &reference)).unwrap(),
            bytes
        );
        assert!(journal.load(&reference).is_ok());
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
        agent_id: None,
        liveness: None,
    }
}
fn request() -> HandoffRequest {
    HandoffRequest {
        policy: None,
        automatic: false,
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
    expected_claim: CallerClaim,
    expected_body: String,
    joined: bool,
    already_joined: bool,
    calls: Mutex<Vec<Command>>,
    lost: Mutex<Option<&'static str>>,
    results: Mutex<std::collections::HashMap<String, CommandResult>>,
}
impl Client {
    fn new(lost: Option<&'static str>) -> Self {
        Self {
            expected_claim: claim(),
            expected_body: "secret durable task".into(),
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
        if matches!(command, Command::Capabilities) {
            return Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: crate::protocol::capabilities::ADVERTISED
                        .iter()
                        .copied()
                        .chain([crate::protocol::capabilities::INVITATION_CONFIRMATION_V1])
                        .map(str::to_owned)
                        .collect(),
                },
            ));
        }
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
                assert_eq!(q.claim, self.expected_claim);
                (
                    q.operation.as_str(),
                    "create",
                    CommandResult::ThreadCreated(ThreadId::new("t1")),
                )
            }
            Command::Invite(q) => {
                assert_eq!(q.claim, self.expected_claim);
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
                assert_eq!(q.claim, self.expected_claim);
                assert_eq!(q.body, self.expected_body);
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
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        Ok(crate::harness::adapter::StartupInputTemplate::positional(
            request.argv.len(),
        ))
    }
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
    let temp = Temp(crate::test_support::scratch::temp_path(format!(
        "handoff-test-{}",
        uuid::Uuid::new_v4()
    )));
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
    let fallback = prompt.split_once("use the exact fallback:").unwrap().1;
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
            Command::Capabilities => self.inner.call(c, b),
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
                    startup_input: None,
                    bootstrap_input: None,
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

// Template-free plans retain frozen V1 bytes; newer plans capture today's
// bootstrap at preparation. Tests must not conflate the two render contracts.
fn expected_plan_bootstrap(plan: &HandoffPlan) -> String {
    if plan.startup_input.is_some() {
        bootstrap(&ThreadId::new("t1"), &plan.context, &claim().instance)
    } else {
        bootstrap_v1::prompt("t1", &plan.context, &claim().instance)
    }
}

struct RecordingLauncher {
    inner: Launcher,
    requests: Vec<LaunchRequest>,
}
impl RecordingLauncher {
    fn new(inner: Launcher) -> Self {
        Self {
            inner,
            requests: vec![],
        }
    }
    fn check_frozen(&self, plan: &HandoffPlan) {
        let mut expected = plan.request.launch.clone();
        expected.argv.push(expected_plan_bootstrap(plan));
        assert!(!self.requests.is_empty());
        for actual in &self.requests {
            assert_eq!(actual, &expected);
        }
    }
}
impl std::ops::Deref for RecordingLauncher {
    type Target = Launcher;
    fn deref(&self) -> &Launcher {
        &self.inner
    }
}
impl std::ops::DerefMut for RecordingLauncher {
    fn deref_mut(&mut self) -> &mut Launcher {
        &mut self.inner
    }
}
impl HandoffLauncher for RecordingLauncher {
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        self.inner.select_startup_input(request)
    }
    fn preflight(&mut self, request: &LaunchRequest) -> Result<SeatId, RunError> {
        self.inner.preflight(request)
    }
    fn launch(
        &mut self,
        request: &LaunchRequest,
        recipient: &SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::super::launch::LaunchReport, RunError> {
        self.requests.push(request.clone());
        self.inner.launch(request, recipient, gate)
    }
}
// Source-only launch fixtures preserve selected registry identity; these opaque
// native arguments do not qualify a native Hermes invocation.
fn registered_requests() -> Vec<HandoffRequest> {
    let registry = crate::harness::registry::builtins();
    registry
        .registrations()
        .iter()
        .map(|registration| {
            let mut req = request();
            let id = registry.agent(registration.metadata().id).unwrap();
            req.launch.harness = crate::harness::registry::OccupantHarness::Agent(id).into();
            req.launch.harness_binary = Some("/fixture/native binary'quoted".into());
            req.launch.name = Some("registered-worker".into());
            req.launch.argv.extend([
                "literal spaces".into(),
                "apostrophe'quoted".into(),
                String::new(),
            ]);
            req
        })
        .collect()
}
fn registered_output(journal: &Journal) -> OutputSpec {
    OutputSpec {
        format: crate::protocol::output::OutputFormat::Json,
        context: selected_context(journal),
    }
}
fn handoff_json(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["handoff"].clone()
}
fn registered_plan(journal: &Journal, reference: &IntentRef) -> HandoffPlan {
    let pending = journal.load(reference).unwrap();
    let SemanticMutation::Frozen {
        claim: caller,
        mutation,
    } = pending.semantic
    else {
        panic!("unfrozen handoff")
    };
    assert_eq!(caller, claim());
    let SemanticMutation::Handoff(plan) = *mutation else {
        panic!("wrong intent")
    };
    *plan
}
// Return the actual kind so the RED collects every registered refusal case
// before comparing identities. All other launch fields are checked independently.
fn check_registered_report(
    report: &serde_json::Value,
    plan: &HandoffPlan,
    reference: &IntentRef,
) -> String {
    let strings = |field: &str| -> Vec<String> {
        report[field]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    let manual = strings("manual_launch_after_confirming_no_start_argv");
    let launch = manual.iter().position(|v| v == "launch").unwrap();
    let options = &manual[launch + 1..];
    let option = |flag: &str| options[options.iter().position(|v| v == flag).unwrap() + 1].clone();
    assert_eq!(option("--pane"), plan.request.launch.target.as_str());
    assert_eq!(
        option("--name"),
        plan.request.launch.name.as_deref().unwrap()
    );
    assert_eq!(
        option("--harness-binary"),
        plan.request.launch.harness_binary.as_deref().unwrap()
    );
    let suffix = &options[options.iter().position(|v| v == "--").unwrap() + 1..];
    assert_eq!(&suffix[..suffix.len() - 1], plan.request.launch.argv);
    let prompt = suffix.last().unwrap();
    assert_eq!(prompt, &expected_plan_bootstrap(plan));
    let routing = prompt
        .strip_prefix("Expected handoff command routing (JSON data): ")
        .unwrap()
        .split_once(" Prefer a startup hook")
        .unwrap()
        .0;
    let routing: serde_json::Value = serde_json::from_str(routing).unwrap();
    assert_eq!(routing["instance"], claim().instance);
    assert_eq!(
        routing["state_dir"],
        plan.context.state_dir.as_deref().unwrap()
    );
    assert_eq!(
        routing["host_endpoint"],
        plan.context.host.as_deref().unwrap()
    );
    let prefix = super::super::hook::cli_prefix(&plan.context);
    assert!(
        manual[..launch].ends_with(&prefix),
        "manual routing must retain its frozen prefix"
    );
    let mut retry = vec!["env".to_owned(), "HERDR_PANE_ID=w1:p1".to_owned()];
    retry.extend(prefix.clone());
    retry.extend(["retry".into(), reference.recovery_ref()]);
    assert_eq!(strings("retry_argv"), retry);
    let mut inspect = prefix;
    inspect.extend(["seat".into(), "inspect".into(), "recipient".into()]);
    assert_eq!(strings("inspect_argv"), inspect);
    assert_eq!(report["thread"], "t1");
    assert_eq!(report["seat"], "recipient");
    assert_eq!(report["pane"], plan.request.launch.target.as_str());
    assert!(!report.to_string().contains("secret durable task"));
    option("--kind")
}
#[test]
fn registered_handoff_refusal_preserves_canonical_recovery_and_frozen_launch() {
    let mut expected_kinds = Vec::new();
    let mut actual_kinds = Vec::new();
    for req in registered_requests() {
        for confirmed in [false, true] {
            let (_temp, journal) = journal();
            let client = Client::new(None);
            let mut launcher = RecordingLauncher::new(Launcher {
                refuse: !confirmed,
                confirmed_refusal: confirmed,
                ..Default::default()
            });
            let output = registered_output(&journal);
            let mut bytes = Vec::new();
            let result = start(
                &journal,
                req.clone(),
                claim(),
                "worker",
                &client,
                &mut launcher,
                &TestClock,
                &output,
                &mut bytes,
            );
            assert!(matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: ErrorCode::TargetUnsafe,
                    ..
                }))
            ));
            let reference = reference(&journal);
            let plan = registered_plan(&journal, &reference);
            launcher.check_frozen(&plan);
            assert_eq!(plan.request, req);
            assert_eq!(plan.context, output.context);
            assert_eq!(plan.recipient, SeatId::new("recipient"));
            let keys = [&plan.create_key, &plan.invite_key, &plan.send_key];
            assert_eq!(
                keys.iter()
                    .map(|k| k.as_str())
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                3
            );
            assert_eq!(
                client
                    .results
                    .lock()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>(),
                keys.iter().map(|k| k.as_str().to_owned()).collect()
            );
            assert_eq!(launcher.starts, 0);
            assert!(!load(&journal, &reference).unwrap().possible_start);
            let report = handoff_json(&bytes);
            assert_eq!(report["outcome"], "pending");
            assert_eq!(report["phase"], "launch");
            assert_eq!(report["failed"], true);
            let kind = check_registered_report(&report, &plan, &reference);
            eprintln!(
                "registered refusal identity={} confirmed={confirmed} emitted_kind={kind} durable_keys=3 starts=0",
                req.launch.harness.as_str()
            );
            expected_kinds.push(req.launch.harness.as_str().to_owned());
            actual_kinds.push(kind);
            launcher.refuse = false;
            launcher.confirmed_refusal = false;
            bytes.clear();
            let different = OutputSpec {
                context: Default::default(),
                ..output.clone()
            };
            resume(
                &journal,
                &reference,
                &scope(),
                &client,
                &mut launcher,
                &TestClock,
                &different,
                &mut bytes,
            )
            .unwrap();
            let report = handoff_json(&bytes);
            assert_eq!(report["outcome"], "started");
            expected_kinds.push(req.launch.harness.as_str().to_owned());
            actual_kinds.push(check_registered_report(&report, &plan, &reference));
            launcher.check_frozen(&plan);
            assert_eq!(launcher.starts, 1);
            assert_eq!(launcher.preflights, 1);
            assert_eq!(
                launcher.submitted_argv.last().unwrap(),
                &expected_plan_bootstrap(&plan)
            );
            assert_eq!(client.calls.lock().unwrap().len(), 3);
            assert!(journal.load(&reference).is_err());
        }
    }
    assert!(!expected_kinds.is_empty());
    assert_eq!(
        actual_kinds, expected_kinds,
        "actual retained reports must select their registered harness"
    );
}

#[test]
fn registered_handoff_unknown_submission_retains_identity_without_relaunch() {
    for req in registered_requests() {
        for mode in ["unknown", "panic_after_gate", "panic_after_start"] {
            let (_temp, journal) = journal();
            let client = Client::new(None);
            let mut launcher = RecordingLauncher::new(Launcher {
                unknown: mode == "unknown",
                panic_after_gate: mode == "panic_after_gate",
                panic_after_start: mode == "panic_after_start",
                ..Default::default()
            });
            let output = registered_output(&journal);
            let mut bytes = Vec::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                start(
                    &journal,
                    req.clone(),
                    claim(),
                    "worker",
                    &client,
                    &mut launcher,
                    &TestClock,
                    &output,
                    &mut bytes,
                )
            }));
            if mode == "unknown" {
                assert!(matches!(result.unwrap(), Err(RunError::Exit(5))));
            } else {
                assert!(result.is_err(), "fixture must reach its injected panic");
            }
            let reference = reference(&journal);
            let plan = registered_plan(&journal, &reference);
            launcher.check_frozen(&plan);
            assert_eq!(plan.request, req);
            assert_eq!(plan.context, output.context);
            assert!(load(&journal, &reference).unwrap().possible_start);
            let starts = launcher.starts;
            assert_eq!(starts, usize::from(mode != "panic_after_gate"));
            let keys = client.results.lock().unwrap().clone();
            assert_eq!(keys.len(), 3);
            if mode == "unknown" {
                let report = handoff_json(&bytes);
                assert_eq!(report["outcome"], "outcome_unknown");
                assert_eq!(
                    check_registered_report(&report, &plan, &reference),
                    req.launch.harness.as_str()
                );
            }
            bytes.clear();
            let different = OutputSpec {
                context: Default::default(),
                ..output
            };
            assert!(matches!(
                resume(
                    &journal,
                    &reference,
                    &scope(),
                    &client,
                    &mut launcher,
                    &TestClock,
                    &different,
                    &mut bytes
                ),
                Err(RunError::Exit(5))
            ));
            let report = handoff_json(&bytes);
            assert_eq!(report["outcome"], "outcome_unknown");
            assert_eq!(
                check_registered_report(&report, &plan, &reference),
                req.launch.harness.as_str()
            );
            assert_eq!(launcher.starts, starts);
            assert_eq!(client.calls.lock().unwrap().len(), 3);
            assert_eq!(*client.results.lock().unwrap(), keys);
            assert!(journal.load(&reference).is_ok());
            assert!(load(&journal, &reference).unwrap().possible_start);
            eprintln!(
                "registered unknown identity={} mode={mode} starts={starts} retained=true no_relaunch=true",
                req.launch.harness.as_str()
            );
        }
    }
}
struct RegisteredOutput {
    bytes: Vec<u8>,
    events: std::sync::Arc<Mutex<Vec<&'static str>>>,
    fail_write: bool,
    fail_flush: bool,
}
impl Write for RegisteredOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_write {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fixture output closed",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::other("fixture flush failed"));
        }
        self.events.lock().unwrap().push("flush");
        Ok(())
    }
}
#[test]
fn registered_handoff_report_replay_and_completed_cleanup_keep_frozen_identity() {
    use std::sync::atomic::Ordering;
    for (index, req) in registered_requests().into_iter().enumerate() {
        // Both output boundaries retain success, and actual resume never launches again.
        for fail_write in [true, false] {
            let (_temp, journal) = journal();
            let events = std::sync::Arc::new(Mutex::new(vec![]));
            let client = FencedClient {
                inner: Client::new(None),
                terminal: false.into(),
                events: events.clone(),
            };
            let mut writer = RegisteredOutput {
                bytes: vec![],
                events: events.clone(),
                fail_write,
                fail_flush: !fail_write,
            };
            let output = registered_output(&journal);
            let mut launcher = RecordingLauncher::new(Launcher::default());
            assert!(
                start(
                    &journal,
                    req.clone(),
                    claim(),
                    "worker",
                    &client,
                    &mut launcher,
                    &TestClock,
                    &output,
                    &mut writer
                )
                .is_err()
            );
            let reference = reference(&journal);
            let plan = registered_plan(&journal, &reference);
            launcher.check_frozen(&plan);
            assert_eq!(plan.request, req);
            assert_eq!(
                load(&journal, &reference).unwrap().launch.unwrap()["outcome"],
                "started"
            );
            assert!(!client.terminal.load(Ordering::Relaxed));
            assert_eq!(&*events.lock().unwrap(), &["begin"]);
            writer.fail_write = false;
            writer.fail_flush = false;
            writer.bytes.clear();
            let different = OutputSpec {
                context: Default::default(),
                ..output
            };
            resume(
                &journal,
                &reference,
                &scope(),
                &client,
                &mut launcher,
                &TestClock,
                &different,
                &mut writer,
            )
            .unwrap();
            let report = handoff_json(&writer.bytes);
            assert_eq!(report["outcome"], "started");
            assert_eq!(
                check_registered_report(&report, &plan, &reference),
                req.launch.harness.as_str()
            );
            assert_eq!(launcher.starts, 1);
            assert_eq!(client.inner.calls.lock().unwrap().len(), 3);
            assert_eq!(
                &*events.lock().unwrap(),
                &["begin", "begin", "flush", "complete"]
            );
            assert!(client.terminal.load(Ordering::Relaxed));
            assert!(journal.load(&reference).is_err());
            eprintln!(
                "registered replay identity={} fail_write={fail_write} starts=1 flush_before_complete=true",
                req.launch.harness.as_str()
            );
        }
        // Actual resume's canonical Completed fence reaches cleanup without effects.
        {
            let (_temp, journal) = journal();
            let output = registered_output(&journal);
            let plan = registered_plan_fixture(req.clone(), output.context.clone());
            let reference = journal
                .record(
                    scope(),
                    SemanticMutation::freeze(
                        SemanticMutation::Handoff(Box::new(plan.clone())),
                        claim(),
                    )
                    .unwrap(),
                    0,
                )
                .unwrap();
            save_success(&journal, &reference);
            let events = std::sync::Arc::new(Mutex::new(vec![]));
            let client = FencedClient {
                inner: Client::new(None),
                terminal: true.into(),
                events: events.clone(),
            };
            let mut writer = RegisteredOutput {
                bytes: vec![],
                events: events.clone(),
                fail_write: false,
                fail_flush: false,
            };
            let mut launcher = RecordingLauncher::new(Launcher::default());
            resume(
                &journal,
                &reference,
                &scope(),
                &client,
                &mut launcher,
                &TestClock,
                &output,
                &mut writer,
            )
            .unwrap();
            assert_eq!(
                check_registered_report(&handoff_json(&writer.bytes), &plan, &reference),
                req.launch.harness.as_str()
            );
            assert_eq!(launcher.starts, 0);
            assert_eq!(launcher.preflights, 0);
            assert!(launcher.requests.is_empty());
            assert!(client.inner.calls.lock().unwrap().is_empty());
            assert_eq!(&*events.lock().unwrap(), &["begin", "flush"]);
            assert!(journal.load(&reference).is_err());
        }
        // Registered sender selection uses real parse_argv and the completed fast route.
        // The identity-independent mismatches run for the first registration;
        // every registration checks its own match and a harness mismatch.
        let mismatches: &[&str] = if index == 0 {
            &[
                "none",
                "harness",
                "namespace",
                "seat",
                "target",
                "role",
                "host",
                "instance",
            ]
        } else {
            &["none", "harness"]
        };
        // One journal per registration: each mismatch freezes its own intent.
        let (_temp, journal) = journal();
        for &mismatch in mismatches {
            let output = registered_output(&journal);
            let plan = registered_plan_fixture(req.clone(), output.context.clone());
            let mut caller = claim();
            caller.harness = req.launch.harness.occupant();
            let reference = journal
                .record(
                    scope(),
                    SemanticMutation::freeze(
                        SemanticMutation::Handoff(Box::new(plan.clone())),
                        caller.clone(),
                    )
                    .unwrap(),
                    0,
                )
                .unwrap();
            save_success(&journal, &reference);
            let before = serde_json::to_value(journal.load(&reference).unwrap().semantic).unwrap();
            let other = crate::harness::registry::builtins()
                .registrations()
                .iter()
                .find(|r| r.metadata().id != req.launch.harness.as_str())
                .unwrap()
                .metadata()
                .id;
            let harness = if mismatch == "harness" {
                other
            } else {
                req.launch.harness.as_str()
            };
            let seat = if mismatch == "seat" {
                "other"
            } else {
                "sender"
            };
            let target = if mismatch == "target" {
                "w9:p9"
            } else {
                "w1:p1"
            };
            let role = if mismatch == "role" {
                "subagent"
            } else {
                "top-level"
            };
            let parsed = crate::cli::commands::parse_argv([
                "ht",
                "--json",
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                target,
                "--cooperative-harness",
                harness,
                "--cooperative-role",
                role,
                "retry",
                &reference.recovery_ref(),
            ])
            .unwrap();
            let mut context = output.context.clone();
            if mismatch == "namespace" {
                let foreign = journal.root().parent().unwrap().join("foreign");
                fs::create_dir(&foreign).unwrap();
                context.state_dir = Some(foreign.display().to_string());
            }
            if mismatch == "host" {
                context.host = Some(
                    journal
                        .root()
                        .parent()
                        .unwrap()
                        .join("foreign.sock")
                        .display()
                        .to_string(),
                );
            }
            let instance = if mismatch == "instance" {
                "00000000-0000-0000-0000-0000000000b2"
            } else {
                &caller.instance
            };
            let events = std::sync::Arc::new(Mutex::new(vec![]));
            let client = FencedClient {
                inner: Client::new(None),
                terminal: true.into(),
                events: events.clone(),
            };
            let mut writer = RegisteredOutput {
                bytes: vec![],
                events: events.clone(),
                fail_write: false,
                fail_flush: false,
            };
            let result = try_completed_retry(
                &parsed,
                &journal,
                instance,
                None,
                &context,
                &client,
                &TestClock,
                &mut writer,
            );
            if mismatch == "none" {
                assert!(result.unwrap());
                assert_eq!(
                    check_registered_report(&handoff_json(&writer.bytes), &plan, &reference),
                    req.launch.harness.as_str()
                );
                assert_eq!(&*events.lock().unwrap(), &["begin", "flush"]);
                assert!(journal.load(&reference).is_err());
            } else {
                assert!(result.is_err(), "{mismatch}");
                assert!(events.lock().unwrap().is_empty());
                assert!(writer.bytes.is_empty());
                assert_eq!(
                    serde_json::to_value(journal.load(&reference).unwrap().semantic).unwrap(),
                    before
                );
            }
            assert!(client.inner.calls.lock().unwrap().is_empty());
            eprintln!(
                "registered completed identity={} selection={mismatch} cleanup_only={}",
                req.launch.harness.as_str(),
                mismatch == "none"
            );
        }
    }
}
fn registered_plan_fixture(
    req: HandoffRequest,
    context: crate::protocol::output::ContinuationContext,
) -> HandoffPlan {
    HandoffPlan {
        startup_input: None,
        bootstrap_input: None,
        request: req,
        context,
        recipient: SeatId::new("recipient"),
        create_key: OperationId::new("create"),
        invite_key: OperationId::new("invite"),
        send_key: OperationId::new("send"),
    }
}
fn save_success(journal: &Journal, reference: &IntentRef) {
    save(
        journal,
        reference,
        &Progress {
            thread: Some(ThreadId::new("t1")),
            launch: Some(serde_json::json!({"outcome":"started"})),
            ..Default::default()
        },
    )
    .unwrap();
}
#[test]
fn registered_handoff_selectors_refuse_human_and_unknown_without_fallback() {
    let (_temp, journal) = journal();
    let sentinel = frozen(&journal);
    let before = serde_json::to_value(journal.load(&sentinel).unwrap().semantic).unwrap();
    for req in registered_requests() {
        let id = req.launch.harness.as_str();
        let handoff = crate::cli::commands::parse_argv([
            "ht",
            "handoff",
            "--new-thread",
            "--pane",
            "w1:p2",
            "--kind",
            id,
            "--",
            "durable body",
        ])
        .unwrap();
        let crate::cli::commands::CliAction::Handoff(parsed) = handoff.action else {
            panic!("wrong handoff action")
        };
        assert_eq!(parsed.launch.harness, req.launch.harness);
        let launch = crate::cli::commands::parse_argv([
            "ht",
            "launch",
            "--pane",
            "w1:p2",
            "--kind",
            id,
            "--",
            "literal spaces",
            "apostrophe'quoted",
            "",
        ])
        .unwrap();
        let crate::cli::commands::CliAction::Launch(parsed) = launch.action else {
            panic!("wrong launch action")
        };
        assert_eq!(parsed.harness, req.launch.harness);
        assert_eq!(parsed.argv, ["literal spaces", "apostrophe'quoted", ""]);
        eprintln!("registered parser identity={id} handoff_and_launch=true");
    }
    // Pure source grammar control: no executable, profile inspection or native
    // prelaunch admission is invoked. The shared opaque report fixture is distinct.
    let registry = crate::harness::registry::builtins();
    let hermes = registry.agent("hermes").unwrap();
    let policy = registry.by_id(hermes).unwrap().launch_policy().unwrap();
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--new-thread",
        "--pane",
        "w1:p2",
        "--kind",
        "hermes",
        "--agent-arg=--profile",
        "--agent-arg=task43",
        "--agent-arg=--cli",
        "--agent-arg=--query",
        "--agent-arg=literal spaces'apostrophe",
        "--",
        "durable body",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(valid) = parsed.action else {
        panic!("wrong action")
    };
    assert_eq!(
        valid.launch.harness.occupant(),
        crate::harness::registry::OccupantHarness::Agent(hermes)
    );
    assert_eq!(
        valid.launch.argv,
        [
            "--profile",
            "task43",
            "--cli",
            "--query",
            "literal spaces'apostrophe"
        ]
    );
    policy.validate_native_argv(&valid.launch.argv).unwrap();
    let opaque = registered_requests()
        .into_iter()
        .find(|r| r.launch.harness.as_str() == "hermes")
        .unwrap();
    assert!(policy.validate_native_argv(&opaque.launch.argv).is_err());
    eprintln!(
        "registered Hermes pure source grammar=true opaque_report_fixture_native_grammar=false native_acceptance=UNMET"
    );
    for id in ["human", "Human", "unknown_agent"] {
        assert!(
            crate::cli::commands::parse_argv([
                "ht",
                "handoff",
                "--new-thread",
                "--pane",
                "w1:p2",
                "--kind",
                id,
                "--",
                "durable body"
            ])
            .is_err()
        );
        assert!(
            crate::cli::commands::parse_argv(["ht", "launch", "--pane", "w1:p2", "--kind", id])
                .is_err()
        );
    }
    assert_eq!(
        serde_json::to_value(journal.load(&sentinel).unwrap().semantic).unwrap(),
        before
    );
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

/// Execute the reported shell command under new option settings, then pass
/// its captured arguments through the same parser and option resolver as launch.
#[test]
fn handoff_manual_recovery_preserves_frozen_options_without_reentry() {
    const CHILD: &str = "HT_MANUAL_RECOVERY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let captured = fs::read(std::env::var_os("HT_MANUAL_RECOVERY_ARGS").unwrap()).unwrap();
        let mut argv = vec!["herdr-threads".to_owned()];
        argv.extend(
            captured
                .split(|byte| *byte == 0)
                .filter(|word| !word.is_empty())
                .map(|word| String::from_utf8(word.to_vec()).unwrap()),
        );
        let parsed = crate::cli::commands::parse_argv(argv).unwrap();
        let crate::cli::commands::CliAction::Launch(request) = parsed.action else {
            panic!("manual recovery must still use guarded launch");
        };
        let expected: Vec<String> =
            serde_json::from_str(&std::env::var("HT_MANUAL_RECOVERY_EXPECTED").unwrap()).unwrap();
        assert_eq!(request.with_process_options().unwrap().argv, expected);
        return;
    }
    use crate::test_support::spawn::SpawnOwned;
    use std::os::unix::fs::PermissionsExt;
    for harness in [
        crate::harness::context::Harness::Codex,
        crate::harness::context::Harness::Claude,
    ] {
        let (_temp, journal) = journal();
        let root = journal.root().parent().unwrap();
        let bin = root.join("herdr-threads");
        fs::write(
            &bin,
            "#!/bin/sh\nprintf '%s\\000' \"$@\" > \"$HT_MANUAL_RECOVERY_ARGS\"\nexec \"$HT_MANUAL_RECOVERY_EXE\" cli::handoff::tests::handoff_manual_recovery_preserves_frozen_options_without_reentry --exact\n",
        )
        .unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
        let mut request = request();
        request.launch.harness = harness;
        request.launch.argv = vec!["--model".into(), "caller model".into()];
        request.launch = request
            .launch
            .with_configured_options(Some(
                if harness == crate::harness::context::Harness::Codex {
                    "--no-daemon --model 'frozen model'"
                } else {
                    "--model 'frozen model'"
                }
                .into(),
            ))
            .unwrap();
        let plan = HandoffPlan {
            startup_input: None,
            bootstrap_input: None,
            request,
            context: Default::default(),
            recipient: SeatId::new("recipient"),
            create_key: OperationId::new("create"),
            invite_key: OperationId::new("invite"),
            send_key: OperationId::new("send"),
        };
        let progress = Progress {
            thread: Some(ThreadId::new("t1")),
            ..Default::default()
        };
        let reference = frozen(&journal);
        let report = report(
            &reference,
            &plan,
            &progress,
            "launch",
            true,
            false,
            &claim(),
        );
        let manual: Vec<String> =
            serde_json::from_value(report["manual_launch_after_confirming_no_start_argv"].clone())
                .unwrap();
        let mut expected = plan.request.launch.argv.clone();
        expected.push(expected_plan_bootstrap(&plan));
        for options in [
            if harness == crate::harness::context::Harness::Codex {
                "--no-daemon --model 'frozen model'"
            } else {
                "--model 'frozen model'"
            },
            "--model 'changed model'",
            "--model 'unterminated",
        ] {
            let mut command = crate::test_support::spawn::command("/bin/sh");
            command
                .args(["-c", &crate::protocol::output::format_command_argv(&manual)])
                .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
                .env(CHILD, "1")
                .env("HT_MANUAL_RECOVERY_ARGS", root.join("args"))
                .env("HT_MANUAL_RECOVERY_EXE", std::env::current_exe().unwrap())
                .env(
                    "HT_MANUAL_RECOVERY_EXPECTED",
                    serde_json::to_string(&expected).unwrap(),
                )
                .env("HERDR_THREADS_CODEX_OPTS", options)
                .env("HERDR_THREADS_CLAUDE_OPTS", options)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = command.spawn_owned().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while child.try_wait().unwrap().is_none() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "manual recovery test timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{harness:?} {options:?}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        }
    }
}

/// Catches extra-key reentry and shell capture dropping empty frozen native arguments.
#[test]
fn stage_a_manual_recovery_extra_key_lossless_shell_parser_resolver() {
    const CHILD: &str = "HT_STAGE_A_MANUAL_CHILD";
    use crate::cli::launch::tests::fourth_adapter;
    use crate::harness::registry::OccupantHarness;
    if std::env::var_os(CHILD).is_some() {
        let key: &'static str = Box::leak(
            std::env::var("HT_STAGE_A_OPTIONS_KEY")
                .unwrap()
                .into_boxed_str(),
        );
        let (registry, _) = fourth_adapter::registry_with_options(true, false, false, Some(key));
        let captured = fs::read(std::env::var_os("HT_STAGE_A_ARGS").unwrap()).unwrap();
        assert_eq!(captured.last(), Some(&0));
        let mut argv = vec!["herdr-threads".to_owned()];
        argv.extend(
            captured[..captured.len() - 1]
                .split(|byte| *byte == 0)
                .map(|word| String::from_utf8(word.to_vec()).unwrap()),
        );
        let parsed = crate::cli::commands::parse_argv_in_registry(argv, &registry).unwrap();
        let crate::cli::commands::CliAction::Launch(request) = parsed.action else {
            panic!("guarded launch required")
        };
        assert_eq!(request.target.as_str(), "w1:p2");
        assert_eq!(request.name.as_deref(), Some("worker"));
        assert_eq!(request.harness_binary.as_deref(), Some("/fixture/fourth"));
        let expected: Vec<String> =
            serde_json::from_str(&std::env::var("HT_STAGE_A_EXPECTED").unwrap()).unwrap();
        assert_eq!(
            request
                .with_process_options_with_registry(&registry)
                .unwrap()
                .argv,
            expected
        );
        return;
    }
    use crate::test_support::spawn::SpawnOwned;
    use std::os::unix::fs::PermissionsExt;
    let (_temp, journal) = journal();
    let root = journal.root().parent().unwrap();
    let bin = root.join("herdr-threads");
    fs::write(&bin, "#!/bin/sh\nprintf '%s\\000' \"$@\" > \"$HT_STAGE_A_ARGS\"\nexec \"$HT_STAGE_A_EXE\" cli::handoff::tests::stage_a_manual_recovery_extra_key_lossless_shell_parser_resolver --exact\n").unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
    let (original, _) = fourth_adapter::registry_with_options(
        true,
        false,
        false,
        Some("HERDR_THREADS_FOURTH_OPTS"),
    );
    let mut request = request();
    request.launch.harness = crate::harness::context::Harness::from(OccupantHarness::Agent(
        original.agent("synthetic_fourth").unwrap(),
    ));
    request.launch.harness_binary = Some("/fixture/fourth".into());
    request.launch.argv = vec!["--model".into(), "caller model".into(), "".into()];
    request.launch = request
        .launch
        .with_configured_options_with_registry(
            &original,
            Some("--model 'frozen model' '' '$HOME $(id)'".into()),
        )
        .unwrap();
    let plan = HandoffPlan {
        startup_input: None,
        bootstrap_input: None,
        request,
        context: Default::default(),
        recipient: SeatId::new("recipient"),
        create_key: OperationId::new("create"),
        invite_key: OperationId::new("invite"),
        send_key: OperationId::new("send"),
    };
    let progress = Progress {
        thread: Some(ThreadId::new("t1")),
        ..Default::default()
    };
    let reference = frozen(&journal);
    let mut expected = plan.request.launch.argv.clone();
    expected.push(expected_plan_bootstrap(&plan));
    for key in [
        "HERDR_THREADS_FOURTH_OPTS",
        "HERDR_THREADS_SUCCESSOR_OPTS",
        "HERDR_THREADS_CODEX_OPTS",
        "HERDR_THREADS_CLAUDE_OPTS",
    ] {
        let (current, _) = fourth_adapter::registry_with_options(true, false, false, Some(key));
        let report = report_with_registry(
            &reference,
            &plan,
            &progress,
            "launch",
            true,
            false,
            (&claim(), &current),
        );
        let manual: Vec<String> =
            serde_json::from_value(report["manual_launch_after_confirming_no_start_argv"].clone())
                .unwrap();
        assert_eq!(
            &manual[..3],
            [
                "env",
                "HERDR_THREADS_CLAUDE_OPTS=",
                "HERDR_THREADS_CODEX_OPTS="
            ]
        );
        assert_eq!(
            manual
                .iter()
                .filter(|arg| *arg == &format!("{key}="))
                .count(),
            1
        );
        assert_eq!(
            manual
                .iter()
                .filter(|arg| *arg == "HERDR_THREADS_CODEX_OPTS=")
                .count(),
            1
        );
        for value in ["--model 'frozen model'", "--model changed", "'unterminated"] {
            let mut command = crate::test_support::spawn::command("/bin/sh");
            command
                .args(["-c", &crate::protocol::output::format_command_argv(&manual)])
                .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
                .env(CHILD, "1")
                .env("HT_STAGE_A_ARGS", root.join("args"))
                .env("HT_STAGE_A_EXE", std::env::current_exe().unwrap())
                .env("HT_STAGE_A_OPTIONS_KEY", key)
                .env(
                    "HT_STAGE_A_EXPECTED",
                    serde_json::to_string(&expected).unwrap(),
                )
                .env("HERDR_THREADS_CODEX_OPTS", "'malformed")
                .env("HERDR_THREADS_CLAUDE_OPTS", "'malformed")
                .env(key, value)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let output = command.spawn_owned().unwrap().wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        }
    }
    let (invalid, _) =
        fourth_adapter::registry_with_options(true, false, false, Some("HERDR_THREADS_$(ID)_OPTS"));
    let report = report_with_registry(
        &reference,
        &plan,
        &progress,
        "launch",
        true,
        true,
        (&claim(), &invalid),
    );
    assert_eq!(report["outcome"], "outcome_unknown");
    assert_eq!(
        report["manual_launch_after_confirming_no_start_argv"],
        serde_json::json!([])
    );
    assert!(report["manual_recovery_error"].is_string());
}

/// Catches reparsing the current declared options during a durable registered-provider retry.
#[test]
fn stage_a_registered_options_freeze_once_and_retry_ignores_current_environment() {
    use crate::cli::launch::tests::fourth_adapter;
    use crate::harness::registry::OccupantHarness;
    use crate::test_support::spawn::SpawnOwned;
    const CHILD: &str = "HT_STAGE_A_FREEZE_CHILD";
    let (registry, _) = fourth_adapter::registry_with_options(
        true,
        false,
        false,
        Some("HERDR_THREADS_FOURTH_OPTS"),
    );
    if std::env::var_os(CHILD).is_some() {
        let journal = Journal::open(std::env::var_os("HT_STAGE_A_JOURNAL").unwrap()).unwrap();
        let reference = reference(&journal);
        let plan = registered_plan(&journal, &reference);
        let mut launcher = Launcher {
            configured_prefix: vec!["--model".into(), "frozen model".into(), "".into()],
            ..Default::default()
        };
        let client = Client::new(None);
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
        assert_eq!(
            &launcher.submitted_argv[..plan.request.launch.argv.len()],
            plan.request.launch.argv
        );
        assert_eq!(launcher.preflights, 0);
        assert_eq!(launcher.starts, 1);
        return;
    }
    // A separate child owns process options, with no global environment mutation.
    const FRESH: &str = "HT_STAGE_A_FREEZE_FRESH";
    if std::env::var_os(FRESH).is_none() {
        let output = crate::test_support::spawn::command(std::env::current_exe().unwrap())
            .args(["cli::handoff::tests::stage_a_registered_options_freeze_once_and_retry_ignores_current_environment", "--exact"])
            .env(FRESH, "1").env("HERDR_THREADS_FOURTH_OPTS", "--model 'frozen model' ''")
            .env("HERDR_THREADS_CODEX_OPTS", "'malformed").env("HERDR_THREADS_CLAUDE_OPTS", "'malformed")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let builtin = crate::harness::registry::builtins();
    let hermes = crate::harness::context::Harness::from(OccupantHarness::Agent(
        builtin.agent("hermes").unwrap(),
    ));
    let mut unrelated = request().launch;
    unrelated.harness = hermes;
    assert_eq!(
        unrelated.clone().with_process_options().unwrap().argv,
        unrelated.argv
    );
    let (none_registry, _) = fourth_adapter::registry(true, false);
    unrelated.harness = crate::harness::context::Harness::from(OccupantHarness::Agent(
        none_registry.agent("fourth").unwrap(),
    ));
    assert_eq!(
        unrelated
            .clone()
            .with_process_options_with_registry(&none_registry)
            .unwrap()
            .argv,
        unrelated.argv
    );
    let (_temp, journal) = journal();
    let mut request = request();
    request.launch.harness = crate::harness::context::Harness::from(OccupantHarness::Agent(
        registry.agent("synthetic_fourth").unwrap(),
    ));
    request.launch = request
        .launch
        .with_process_options_with_registry(&registry)
        .unwrap();
    assert_eq!(
        request.launch.argv,
        ["--model", "frozen model", "", "-a", "on-request"]
    );
    let client = Client::new(None);
    let mut launcher = Launcher {
        configured_prefix: vec!["--model".into(), "frozen model".into(), "".into()],
        confirmed_refusal: true,
        ..Default::default()
    };
    assert!(
        start(
            &journal,
            request.clone(),
            claim(),
            "worker",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new()
        )
        .is_err()
    );
    let reference = reference(&journal);
    assert_eq!(
        registered_plan(&journal, &reference).request.launch.argv,
        request.launch.argv
    );
    assert_eq!(launcher.preflight_argv, request.launch.argv);
    let mut retry = crate::test_support::spawn::command(std::env::current_exe().unwrap());
    retry.args(["cli::handoff::tests::stage_a_registered_options_freeze_once_and_retry_ignores_current_environment", "--exact"])
        .env(CHILD, "1").env("HT_STAGE_A_JOURNAL", journal.root())
        .env("HERDR_THREADS_FOURTH_OPTS", "'malformed changed current value")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let output = retry.spawn_owned().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

struct Task48DurableClient {
    events: Mutex<Vec<&'static str>>,
}
impl LocalClient for Task48DurableClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        if matches!(command, Command::Capabilities) {
            return Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: crate::protocol::capabilities::ADVERTISED
                        .iter()
                        .copied()
                        .chain([crate::protocol::capabilities::INVITATION_CONFIRMATION_V1])
                        .map(str::to_owned)
                        .collect(),
                },
            ));
        }
        use crate::protocol::handoff::{HandoffResult, HandoffState};
        let mut events = self.events.lock().unwrap();
        Ok(match command {
            Command::BeginHandoff(q) => {
                events.push("begin");
                CommandResult::Handoff(HandoffResult {
                    compound: q.identity.compound,
                    thread: events.contains(&"create").then(|| ThreadId::new("t1")),
                    state: HandoffState::Live,
                })
            }
            Command::CompleteHandoff(q) => {
                events.push("complete");
                CommandResult::Handoff(HandoffResult {
                    compound: q.identity.compound,
                    thread: Some(ThreadId::new("t1")),
                    state: HandoffState::Completed,
                })
            }
            Command::CreateThread(_) => {
                events.push("create");
                CommandResult::ThreadCreated(ThreadId::new("t1"))
            }
            Command::Invite(q) => {
                assert_eq!(q.seat.as_str(), "seat_launch");
                events.push("invite");
                CommandResult::Invitation(InvitationId::new("i1"))
            }
            Command::SendMessage(q) => {
                assert_eq!(q.invited_recipients, [SeatId::new("seat_launch")]);
                events.push("send");
                CommandResult::MessageSent(MessageId::new("m1"))
            }
            other => panic!("unexpected actual Task48 coordinator command: {other:?}"),
        })
    }
}

#[test]
fn task48_hermes_actual_native_launcher_composes_before_durable_work() {
    let (_temp, journal) = journal();
    let client = Task48DurableClient {
        events: Mutex::new(vec![]),
    };
    let output = OutputSpec {
        context: selected_context(&journal),
        ..Default::default()
    };
    let ((result, bytes), native, seat_calls, records) =
        crate::cli::launch::tests::task48_hermes_native_fixture(|launcher, launch| {
            // BASE original prompt-free preflight must reach real selected Hermes preparation.
            assert_eq!(launcher.preflight(&launch).unwrap().as_str(), "seat_launch");
            let req = HandoffRequest {
                automatic: false,
                launch,
                ..request()
            };
            let mut bytes = vec![];
            let result = start(
                &journal,
                req,
                claim(),
                "worker",
                &client,
                launcher,
                &TestClock,
                &output,
                &mut bytes,
            );
            (result, bytes)
        });
    eprintln!(
        "task48 actual coordinator/Hermes: result={result:?} durable={:?} native_starts={} seat_calls={seat_calls} records={} output={}",
        client.events.lock().unwrap(),
        native.len(),
        records.len(),
        String::from_utf8_lossy(&bytes)
    );
    result.expect("selected Hermes must carry startup through its separate query transport");
    assert_eq!(
        &*client.events.lock().unwrap(),
        &["begin", "create", "invite", "send", "complete"]
    );
    assert_eq!(native.len(), 1);
    assert_eq!(
        native[0].argv[..4],
        ["--profile", "default", "--cli", "chat"]
    );
    let query = native[0]
        .argv
        .iter()
        .position(|arg| arg == "--query")
        .unwrap();
    assert!(native[0].argv[query + 1].starts_with("Expected handoff command routing"));
    assert!(native[0].argv[query + 1].contains("The task for thread t1 is stored in inbox"));
    assert_eq!(records.len(), 1);
}

fn task48_policy(id: &str) -> &'static dyn crate::harness::adapter::LaunchPolicy {
    let registry = crate::harness::registry::builtins();
    registry
        .by_id(registry.agent(id).unwrap())
        .unwrap()
        .launch_policy()
        .unwrap()
}
fn task48_select(
    id: &str,
    argv: &[&str],
) -> Result<crate::harness::adapter::StartupInputTemplate, ApiError> {
    task48_policy(id)
        .prepare_startup_input(
            &argv
                .iter()
                .map(|token| (*token).to_owned())
                .collect::<Vec<_>>(),
            &crate::harness::adapter::StartupInputSpec {
                max_text_bytes: 16384,
            },
        )
        .map(|template| template.unwrap())
}
#[test]
fn task48_finite_grammar_keeps_captured_caller_states_and_refuses_neighbors() {
    let uuid = "01234567-89ab-cdef-0123-456789abcdef";
    let block = [
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
    ];
    for argv in [
        vec![],
        vec!["--model", "sonnet", "--model", "haiku"],
        vec![
            "--effort",
            "low",
            "--settings",
            "path with spaces",
            "--setting-sources",
            "user",
            "--permission-mode",
            "default",
            "--max-budget-usd",
            "1",
        ],
        vec!["-p"],
        vec!["--continue"],
        vec!["--resume", uuid],
        vec!["--session-id", uuid],
        vec!["--"],
        vec!["--output-format", "stream-json", "--verbose", "-p"],
        vec![
            "--setting-sources",
            "project,local",
            "-p",
            "--output-format",
            "json",
        ],
    ] {
        let template = task48_select("claude", &argv).unwrap();
        assert_eq!(template.insertion_index, argv.len());
        for placement in 0..=1 {
            let mut full = argv.clone();
            if full.last() == Some(&"--") {
                full.pop();
            }
            let expected = if placement == 0 {
                let n = full.len();
                full.extend(block);
                n
            } else {
                let mut prefix = block.to_vec();
                prefix.extend(full);
                full = prefix;
                0
            };
            let template = task48_select("claude", &full).unwrap();
            assert_eq!(template.insertion_index, expected);
        }
    }
    let mut multiple = vec!["--model", "sonnet"];
    multiple.extend(block);
    multiple.extend(["-p", "--continue"]);
    multiple.extend(block);
    multiple.push("--");
    assert_eq!(
        task48_select("claude", &multiple).unwrap().insertion_index,
        2
    );
    for argv in [
        vec![""],
        vec!["PROMPT"],
        vec!["-p", "/compact", "--output-format", "json"],
        vec!["--model"],
        vec!["--model", ""],
        vec!["--model", "--continue"],
        vec!["--model=sonnet"],
        vec!["--effort=low"],
        vec!["--setting-sources", "local"],
        vec!["--output-format", "json"],
        vec!["--verbose"],
        vec!["--print"],
        vec!["-p", "-p"],
        vec!["--resume"],
        vec!["--resume", "named"],
        vec!["--resume=01234567-89ab-cdef-0123-456789abcdef"],
        vec!["--continue", "--continue"],
        vec!["--continue", "--resume", uuid],
        vec!["--resume", uuid, "--session-id", uuid],
        vec!["--tools", "tool"],
        vec!["--tools", "", "--mcp-config", "{\"mcpServers\":{}}"],
        vec!["--mcp-config", "{\"mcpServers\":{}}"],
        vec!["--help"],
        vec!["--", "--continue"],
        vec!["--", "--"],
        vec!["-"],
    ] {
        assert!(
            task48_select("claude", &argv).is_err(),
            "accepted Claude {argv:?}"
        );
    }
    for argv in [
        vec![],
        vec!["--model", "exec"],
        vec!["--model=--last"],
        vec!["--image=one.png"],
        vec!["--no-daemon"],
        vec!["--"],
        vec!["exec"],
        vec!["exec", "--json", "--"],
        vec!["exec", "resume", "session"],
        vec!["exec", "resume", "session", "--json", "--"],
        vec!["exec", "resume", "--last"],
        vec!["--model", "resume", "exec", "resume", "--last"],
    ] {
        assert!(
            task48_select("codex", &argv).is_ok(),
            "refused Codex {argv:?}"
        );
    }
    for argv in [
        vec![""],
        vec!["PROMPT"],
        vec!["exec", "PROMPT"],
        vec!["exec", "--", "resume"],
        vec!["resume"],
        vec!["exec", "resume"],
        vec!["exec", "resume", "--"],
        vec!["exec", "resume", "--", "session"],
        vec!["exec", "resume", "session", "PROMPT"],
        vec!["exec", "resume", "--last", "PROMPT"],
        vec!["exec", "resume", "session", "--last"],
        vec!["exec", "resume", "--last", "--last"],
        vec!["--json"],
        vec!["exec", "--no-daemon"],
        vec!["--model"],
        vec!["--model", ""],
        vec!["--model", "--last"],
        vec!["--image", "file"],
        vec!["-ifile"],
        vec!["-msonnet"],
        vec!["--unknown"],
        vec!["--no-daemon=true"],
        vec!["--", "--"],
    ] {
        assert!(
            task48_select("codex", &argv).is_err(),
            "accepted Codex {argv:?}"
        );
    }
    for argv in [
        vec![],
        vec!["--cli"],
        vec!["--profile= DEFAULT "],
        vec!["-p", "default", "-m", "model", "--provider", "provider"],
    ] {
        let template = task48_select("hermes", &argv).unwrap();
        assert_eq!(template.before, ["--query"]);
        assert_eq!(template.max_arg_bytes, 4096);
    }
    for argv in [
        vec!["--query", "occupied"],
        vec!["-q", "occupied"],
        vec!["--query"],
        vec!["--query", ""],
        vec!["--profile", "a", "--profile", "b"],
        vec!["--query=occupied"],
        vec!["--model=sonnet"],
        vec!["--provider=p"],
        vec!["chat"],
        vec!["--"],
        vec![""],
    ] {
        assert!(
            task48_select("hermes", &argv).is_err(),
            "accepted Hermes {argv:?}"
        );
    }
}

/// Pane handoff places the startup text with each adapter's own template, and the composed
/// argv passes that adapter's native argv validation (agy and Hermes, ht-poh.22 acceptance).
#[test]
fn pane_handoff_agy_and_hermes_produce_valid_argv() {
    let caller: Vec<String> = vec!["--model".into(), "m".into()];
    let agy = task48_select("agy", &["--model", "m"]).unwrap();
    let (argv, slot) = agy.apply(&caller, "TEXT").unwrap();
    assert_eq!(argv, ["--model", "m", "--prompt-interactive", "TEXT"]);
    assert_eq!(slot, 3);
    task48_policy("agy").validate_native_argv(&argv).unwrap();
    // A bare positional is what the V1 bootstrap appended; agy's grammar refuses it.
    let bare: Vec<String> = ["--model", "m", "TEXT"].map(String::from).to_vec();
    assert!(task48_policy("agy").validate_native_argv(&bare).is_err());
    // The prompt flag is occupied by the caller: no template.
    assert!(task48_select("agy", &["--prompt-interactive", "x"]).is_err());

    let hermes = task48_select("hermes", &["--model", "m"]).unwrap();
    let (argv, _) = hermes.apply(&caller, "TEXT").unwrap();
    assert_eq!(argv, ["--model", "m", "--query", "TEXT"]);
    task48_policy("hermes").validate_native_argv(&argv).unwrap();
}

struct Task48UnavailableLauncher;
impl HandoffLauncher for Task48UnavailableLauncher {
    fn select_startup_input(
        &mut self,
        _: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        panic!("absorbing replay selected current transport")
    }
    fn native_input(&mut self, _: &LaunchRequest) -> Result<Vec<String>, RunError> {
        panic!("absorbing replay prepared current runtime")
    }
    fn preflight(&mut self, _: &LaunchRequest) -> Result<SeatId, RunError> {
        panic!("absorbing replay preflighted runtime")
    }
    fn launch(
        &mut self,
        _: &LaunchRequest,
        _: &SeatId,
        _: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::super::launch::LaunchReport, RunError> {
        panic!("absorbing replay started runtime")
    }
}
struct Task48PhaseClient {
    state: crate::protocol::handoff::HandoffState,
    thread: Option<ThreadId>,
    fail_begin: bool,
    fail_complete: bool,
    events: std::sync::Arc<Mutex<Vec<&'static str>>>,
}
impl LocalClient for Task48PhaseClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        if matches!(command, Command::Capabilities) {
            return Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: crate::protocol::capabilities::ADVERTISED
                        .iter()
                        .copied()
                        .chain([crate::protocol::capabilities::INVITATION_CONFIRMATION_V1])
                        .map(str::to_owned)
                        .collect(),
                },
            ));
        }
        use crate::protocol::handoff::{HandoffResult, HandoffState};
        let (mutation, complete) = match command {
            Command::BeginHandoff(mutation) => (mutation, false),
            Command::CompleteHandoff(mutation) => (mutation, true),
            other => panic!("absorbing replay child effect {other:?}"),
        };
        self.events
            .lock()
            .unwrap()
            .push(if complete { "complete" } else { "begin" });
        if (complete && self.fail_complete) || (!complete && self.fail_begin) {
            return Err(ApiError::new(
                ErrorCode::UnknownOutcome,
                "canonical response lost",
            ));
        }
        Ok(CommandResult::Handoff(HandoffResult {
            compound: mutation.identity.compound,
            thread: self.thread.clone(),
            state: if complete {
                HandoffState::Completed
            } else {
                self.state
            },
        }))
    }
}
#[test]
fn task48_terminal_thin_uncertain_matrix_is_runtime_free_and_canonically_completed() {
    use crate::protocol::handoff::HandoffState;
    // One journal serves every case: each case freezes its own handoff intent
    // (its own reference), so the cases stay independent while the
    // fsync-bound journal setup runs once.
    let (_temp, journal) = journal();
    for (label, progress) in [
        (
            "terminal",
            Progress {
                thread: Some(ThreadId::new("t1")),
                invitation: Some(CommandResult::Invitation(InvitationId::new("i1"))),
                message: Some(CommandResult::MessageSent(MessageId::new("m1"))),
                possible_start: true,
                launch: Some(serde_json::json!({"outcome":"started"})),
            },
        ),
        (
            "thin",
            Progress {
                thread: Some(ThreadId::new("t1")),
                launch: Some(serde_json::json!({"outcome":"started"})),
                ..Default::default()
            },
        ),
        (
            "partial",
            Progress {
                thread: Some(ThreadId::new("t1")),
                invitation: Some(CommandResult::Invitation(InvitationId::new("i1"))),
                launch: Some(serde_json::json!({"outcome":"started"})),
                ..Default::default()
            },
        ),
        (
            "unknown",
            Progress {
                possible_start: true,
                ..Default::default()
            },
        ),
        (
            "already-joined-conflict",
            Progress {
                thread: Some(ThreadId::new("t1")),
                invitation: Some(CommandResult::AlreadyJoined(
                    crate::protocol::results::AlreadyJoined {
                        thread: ThreadId::new("t1"),
                        seat: SeatId::new("foreign"),
                    },
                )),
                message: Some(CommandResult::MessageSent(MessageId::new("m1"))),
                launch: Some(serde_json::json!({"outcome":"started"})),
                ..Default::default()
            },
        ),
    ] {
        let absorbing = matches!(label, "partial" | "unknown" | "already-joined-conflict");
        // An absorbing replay never reaches the canonical client (asserted
        // below), so the canonical answer cannot matter to it: one canonical
        // state covers it under every output/response boundary.
        let canonicals: &[&str] = if absorbing {
            &["live"]
        } else {
            &["completed", "live", "missing-thread"]
        };
        for &canonical in canonicals {
            for boundary in ["success", "write", "flush", "begin-lost", "complete-lost"] {
                let reference = frozen(&journal);
                save(&journal, &reference, &progress).unwrap();
                let events = std::sync::Arc::new(Mutex::new(vec![]));
                let client = Task48PhaseClient {
                    state: if canonical == "completed" {
                        HandoffState::Completed
                    } else {
                        HandoffState::Live
                    },
                    thread: (canonical != "missing-thread").then(|| ThreadId::new("t1")),
                    fail_begin: boundary == "begin-lost",
                    fail_complete: boundary == "complete-lost",
                    events: events.clone(),
                };
                let mut output = RegisteredOutput {
                    bytes: vec![],
                    events: events.clone(),
                    fail_write: boundary == "write",
                    fail_flush: boundary == "flush",
                };
                let result = resume(
                    &journal,
                    &reference,
                    &scope(),
                    &client,
                    &mut Task48UnavailableLauncher,
                    &TestClock,
                    &registered_output(&journal),
                    &mut output,
                );
                let events = events.lock().unwrap().clone();
                if absorbing {
                    assert!(!events.contains(&"begin") && !events.contains(&"complete"));
                    assert!(result.is_err());
                    assert!(journal.load(&reference).is_ok());
                } else {
                    assert_eq!(events.first(), Some(&"begin"));
                    let completed =
                        canonical == "completed" || (canonical == "live" && label == "terminal");
                    let succeeds = completed
                        && !matches!(boundary, "write" | "flush" | "begin-lost")
                        && !(boundary == "complete-lost" && canonical == "live");
                    assert_eq!(
                        result.is_ok(),
                        succeeds,
                        "{label} {canonical} {boundary}: {result:?} {events:?}"
                    );
                    assert_eq!(journal.load(&reference).is_err(), succeeds);
                    if events.contains(&"complete") {
                        assert_eq!(label, "terminal");
                        assert_eq!(canonical, "live");
                        assert!(events.windows(2).any(|pair| pair == ["flush", "complete"]));
                    }
                    if label == "thin" || canonical == "completed" {
                        assert!(!events.contains(&"complete"));
                    }
                }
                if journal.load(&reference).is_ok() {
                    assert_eq!(
                        serde_json::to_value(load(&journal, &reference).unwrap()).unwrap(),
                        serde_json::to_value(&progress).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn task48_old_plan_digest_and_new_template_selection_remain_immutable() {
    let (_temp, journal) = journal();
    let reference = frozen(&journal);
    let pending = journal.load(&reference).unwrap();
    let old = serde_json::to_vec(&pending.semantic).unwrap();
    assert!(!String::from_utf8_lossy(&old).contains("startup_input"));
    assert_eq!(
        serde_json::to_vec(&journal.load(&reference).unwrap().semantic).unwrap(),
        old
    );
    let SemanticMutation::Frozen { mutation, .. } = pending.semantic else {
        panic!("frozen")
    };
    let SemanticMutation::Handoff(mut plan) = *mutation else {
        panic!("handoff")
    };
    plan.startup_input = Some(crate::harness::adapter::StartupInputTemplate::positional(
        plan.request.launch.argv.len() + 1,
    ));
    assert!(plan.validate().is_err());
    assert_eq!(
        serde_json::to_vec(&journal.load(&reference).unwrap().semantic).unwrap(),
        old
    );
}

#[test]
fn task48_envelope_true_reserved_bounds_and_opaque_slot_are_exact() {
    use crate::harness::adapter::StartupInputTemplate;
    let req = request().launch;
    let context = crate::protocol::output::ContinuationContext::default();
    let mut template = StartupInputTemplate::positional(req.argv.len());
    template.prefix = "--initial=".into();
    template.suffix = "suffix".into();
    let envelope = startup_envelope(&req, &context, &claim().instance, None, &template).unwrap();
    assert!(!envelope.slot_token.contains("task for thread t"));
    let measured = envelope.slot_token.len() + 128;
    template.max_arg_bytes = measured;
    let exact = startup_envelope(&req, &context, &claim().instance, None, &template).unwrap();
    validate_whole_input(&exact, &exact.request.argv).unwrap();
    template.max_arg_bytes = measured - 1;
    assert!(startup_envelope(&req, &context, &claim().instance, None, &template).is_err());
    let mut duplicated = exact.request.argv.clone();
    duplicated.push(exact.slot_token.clone());
    assert!(validate_whole_input(&exact, &duplicated).is_err());
    let mut transformed = exact.request.argv.clone();
    transformed.last_mut().unwrap().push('x');
    assert!(validate_whole_input(&exact, &transformed).is_err());
    let thread = ThreadId::new("x".repeat(128));
    template.max_arg_bytes = measured;
    let known =
        startup_envelope(&req, &context, &claim().instance, Some(&thread), &template).unwrap();
    assert_eq!(known.slot_token.len(), exact.slot_token.len() + 128);
    assert_eq!(known.reserve, 0);
    assert!(startup_envelope(&req, &context, &"x".repeat(16385), None, &template).is_err());
    let escaped = crate::protocol::output::ContinuationContext {
        state_dir: Some("/tmp/'quoted\nstate".into()),
        host: Some("/tmp/'quoted\nhost".into()),
    };
    template.max_arg_bytes = 16384;
    let envelope = startup_envelope(&req, &escaped, &claim().instance, None, &template).unwrap();
    assert!(!envelope.slot_token.contains(['\n', '\r']));
    assert!(envelope.slot_token.contains("\\n"));
}

// Independent stated grammar model: native greedy lists consume non-options,
// including an unsafe appended instruction. This is not installed parser proof.
fn task48_claude_instruction_is_positional(argv: &[String]) -> bool {
    let mut index = 0;
    let mut prompts = vec![];
    let mut list_values = vec![];
    while index < argv.len() {
        match argv[index].as_str() {
            "--tools" | "--mcp-config" => {
                index += 1;
                while index < argv.len() && !argv[index].starts_with('-') {
                    list_values.push(argv[index].as_str());
                    index += 1;
                }
            }
            "--model" | "--effort" | "--settings" | "--permission-mode" | "--max-budget-usd"
            | "--setting-sources" | "--output-format" | "--resume" | "--session-id" => {
                index += 2;
            }
            "--strict-mcp-config" | "-p" | "--continue" | "--verbose" | "--" => index += 1,
            other if other.starts_with('-') => return false,
            _ => {
                prompts.push(argv[index].as_str());
                index += 1;
            }
        }
    }
    prompts
        .iter()
        .filter(|token| token.starts_with("Expected handoff command routing"))
        .count()
        == 1
        && !list_values
            .iter()
            .any(|token| token.starts_with("Expected handoff command routing"))
}
struct Task48SelectionSpy<'a, 'b> {
    inner: &'a mut NativeLauncher<'b>,
    selections: usize,
}
impl HandoffLauncher for Task48SelectionSpy<'_, '_> {
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        self.selections += 1;
        self.inner.select_startup_input(request)
    }
    fn native_input(&mut self, request: &LaunchRequest) -> Result<Vec<String>, RunError> {
        self.inner.native_input(request)
    }
    fn preflight_startup(
        &mut self,
        original: &LaunchRequest,
        effective: &LaunchRequest,
    ) -> Result<StartupPreflight, RunError> {
        self.inner.preflight_startup(original, effective)
    }
    fn preflight_saved(
        &mut self,
        original: &LaunchRequest,
        effective: &LaunchRequest,
        recipient: &SeatId,
    ) -> Result<StartupPreflight, RunError> {
        self.inner.preflight_saved(original, effective, recipient)
    }
    fn preflight(&mut self, request: &LaunchRequest) -> Result<SeatId, RunError> {
        self.inner.preflight(request)
    }
    fn launch(
        &mut self,
        request: &LaunchRequest,
        recipient: &SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), ApiError>,
    ) -> Result<super::super::launch::LaunchReport, RunError> {
        self.inner.launch(request, recipient, gate)
    }
}
#[test]
fn task48_claude_actual_native_boundary_covers_fresh_old_new_and_safe_list_mutant() {
    let block = [
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
    ];
    let uuid = "01234567-89ab-cdef-0123-456789abcdef";
    let forms = [
        block.to_vec(),
        [block.to_vec(), vec!["--resume", uuid]].concat(),
        [vec!["-p", "--continue"], block.to_vec()].concat(),
        [block.to_vec(), vec!["-p", "--output-format", "json"]].concat(),
        [block.to_vec(), vec!["--"]].concat(),
        [
            vec!["--model", "one", "--model", "two"],
            block.to_vec(),
            block.to_vec(),
        ]
        .concat(),
    ];
    // Each argv form runs through one lane, rotating fresh/old/new, so every
    // form and every lane is covered (each lane sees two forms) without the
    // full 6x3 product of native launch fixtures.
    for (index, argv) in forms.into_iter().enumerate() {
        {
            let lane = ["fresh", "old", "new"][index % 3];
            let (_temp, journal) = journal();
            let client = Task48DurableClient {
                events: Mutex::new(vec![]),
            };
            let output = registered_output(&journal);
            if lane == "new" {
                let (result, submitted, _, _) =
                    crate::cli::launch::tests::task48_claude_native_fixture_refused(
                        true,
                        |launcher, mut launch| {
                            launch.argv = argv.iter().map(|token| (*token).to_owned()).collect();
                            start(
                                &journal,
                                HandoffRequest {
                                    automatic: false,
                                    launch,
                                    ..request()
                                },
                                claim(),
                                "worker",
                                &client,
                                launcher,
                                &TestClock,
                                &output,
                                &mut Vec::new(),
                            )
                        },
                    );
                assert!(result.is_err());
                assert!(submitted.is_empty());
            }
            let ((result, selections, original), native, _, _) =
                crate::cli::launch::tests::task48_claude_native_fixture(|launcher, mut launch| {
                    launch.argv = argv.iter().map(|token| (*token).to_owned()).collect();
                    let original = launch.argv.clone();
                    let mut spy = Task48SelectionSpy {
                        inner: launcher,
                        selections: 0,
                    };
                    let result = match lane {
                        "fresh" => start(
                            &journal,
                            HandoffRequest {
                                automatic: false,
                                launch,
                                ..request()
                            },
                            claim(),
                            "worker",
                            &client,
                            &mut spy,
                            &TestClock,
                            &output,
                            &mut Vec::new(),
                        ),
                        "new" => {
                            let reference = reference(&journal);
                            resume(
                                &journal,
                                &reference,
                                &scope(),
                                &client,
                                &mut spy,
                                &TestClock,
                                &output,
                                &mut Vec::new(),
                            )
                        }
                        _ => {
                            let plan = HandoffPlan {
                                startup_input: None,
                                bootstrap_input: None,
                                request: HandoffRequest {
                                    automatic: false,
                                    launch,
                                    ..request()
                                },
                                context: output.context.clone(),
                                recipient: SeatId::new("seat_launch"),
                                create_key: OperationId::new("create"),
                                invite_key: OperationId::new("invite"),
                                send_key: OperationId::new("send"),
                            };
                            let reference = journal
                                .record(
                                    scope(),
                                    SemanticMutation::freeze(
                                        SemanticMutation::Handoff(Box::new(plan)),
                                        claim(),
                                    )
                                    .unwrap(),
                                    0,
                                )
                                .unwrap();
                            resume(
                                &journal,
                                &reference,
                                &scope(),
                                &client,
                                &mut spy,
                                &TestClock,
                                &output,
                                &mut Vec::new(),
                            )
                        }
                    };
                    (result, spy.selections, original)
                });
            result.unwrap();
            assert_eq!(selections, usize::from(lane != "new"));
            assert_eq!(native.len(), 1);
            let actual = &native[0].argv;
            assert!(
                task48_claude_instruction_is_positional(actual),
                "{lane} {actual:?}"
            );
            let prompt = actual
                .iter()
                .position(|token| token.starts_with("Expected handoff command routing"))
                .unwrap();
            let mut recovered = actual.clone();
            let instruction = recovered.remove(prompt);
            assert_eq!(recovered, original);
            let mut unsafe_tail = block
                .iter()
                .map(|token| (*token).to_owned())
                .collect::<Vec<_>>();
            unsafe_tail.push(instruction);
            assert!(
                !task48_claude_instruction_is_positional(&unsafe_tail),
                "unsafe MCP-last mutant was accepted"
            );
            assert_eq!(
                actual.iter().filter(|token| token.is_empty()).count(),
                argv.iter().filter(|token| token.is_empty()).count()
            );
        }
    }
    assert!(task48_claude_instruction_is_positional(&[
        "Expected handoff command routing fixture".into(),
        "--model".into(),
        "sonnet".into(),
        "-p".into()
    ]));
}

#[test]
fn task48_claude_lossless_manual_shell_parser_options_and_actual_native_chain() {
    use crate::test_support::spawn::SpawnOwned;
    use std::os::unix::fs::PermissionsExt;
    const CHILD: &str = "HT_TASK48_MANUAL_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let bytes = fs::read(std::env::var_os("HT_TASK48_ARGS").unwrap()).unwrap();
        let mut captured = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
        assert_eq!(captured.pop(), Some(&b""[..]));
        let argv = std::iter::once("ht".to_owned())
            .chain(
                captured
                    .into_iter()
                    .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap()),
            )
            .collect::<Vec<_>>();
        let parsed = crate::cli::commands::parse_argv(argv).unwrap();
        let crate::cli::commands::CliAction::Launch(request) = parsed.action else {
            panic!("manual launch")
        };
        let before = request.argv.clone();
        let request = request.with_process_options().unwrap();
        assert_eq!(request.argv, before);
        let (result, native, _, _) =
            crate::cli::launch::tests::task48_claude_native_fixture(|launcher, _| {
                launcher.launch(&request, &SeatId::new("seat_launch"), &mut |_| Ok(()))
            });
        assert_eq!(result.unwrap().exit, 0);
        assert_eq!(native.len(), 1);
        assert!(task48_claude_instruction_is_positional(&native[0].argv));
        fs::write(
            std::env::var_os("HT_TASK48_RESULT").unwrap(),
            serde_json::to_vec(&native[0].argv).unwrap(),
        )
        .unwrap();
        return;
    }
    let (_temp, journal) = journal();
    let root = journal.root().parent().unwrap();
    let client = Task48DurableClient {
        events: Mutex::new(vec![]),
    };
    let output = registered_output(&journal);
    let argv = vec![
        "--model",
        "frozen model",
        "--model",
        "caller model",
        "--tools",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
    ];
    let ((result, bytes), native, _, _) =
        crate::cli::launch::tests::task48_claude_native_fixture_refused(
            true,
            |launcher, mut launch| {
                launch.argv = argv.iter().map(|token| (*token).to_owned()).collect();
                let mut bytes = vec![];
                let result = start(
                    &journal,
                    HandoffRequest {
                        automatic: false,
                        launch,
                        ..request()
                    },
                    claim(),
                    "worker",
                    &client,
                    launcher,
                    &TestClock,
                    &output,
                    &mut bytes,
                );
                (result, bytes)
            },
        );
    assert!(result.is_err());
    assert!(native.is_empty());
    let report = handoff_json(&bytes);
    assert_eq!(report["outcome"], "pending");
    let manual: Vec<String> =
        serde_json::from_value(report["manual_launch_after_confirming_no_start_argv"].clone())
            .unwrap();
    assert!(!manual.is_empty());
    let command = root.join("herdr-threads");
    fs::write(&command,"#!/bin/sh\nprintf '%s\\000' \"$@\" > \"$HT_TASK48_ARGS\"\nexec \"$HT_TASK48_EXE\" cli::handoff::tests::task48_claude_lossless_manual_shell_parser_options_and_actual_native_chain --exact\n").unwrap();
    fs::set_permissions(&command, fs::Permissions::from_mode(0o700)).unwrap();
    for inherited in ["--model 'frozen model'", "--model changed", "'malformed"] {
        let args = root.join("args");
        let result = root.join("result");
        let mut shell = crate::test_support::spawn::command("/bin/sh");
        shell
            .args(["-c", &crate::protocol::output::format_command_argv(&manual)])
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", root.display()))
            .env("HOME", root)
            .env("CLAUDE_CONFIG_DIR", root.join("claude"))
            .env("CODEX_HOME", root.join("codex"))
            .env("TMPDIR", std::env::temp_dir())
            .env(CHILD, "1")
            .env("HT_TASK48_ARGS", &args)
            .env("HT_TASK48_RESULT", &result)
            .env("HT_TASK48_EXE", std::env::current_exe().unwrap())
            .env("HERDR_THREADS_CLAUDE_OPTS", inherited)
            .env("HERDR_THREADS_CODEX_OPTS", inherited)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(run_id) = std::env::var_os("HT_LEAK_RUN_ID") {
            shell.env("HT_LEAK_RUN_ID", run_id);
        }
        let child = shell.spawn_owned().unwrap().wait_with_output().unwrap();
        assert!(
            child.status.success(),
            "{} {}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        let actual: Vec<String> = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
        assert!(task48_claude_instruction_is_positional(&actual));
        let original = actual
            .iter()
            .filter(|token| !token.starts_with("Expected handoff command routing"))
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(original, argv);
        assert_eq!(actual.iter().filter(|token| token.is_empty()).count(), 1);
    }
}

#[test]
fn task48_nonpositional_optional_provider_reaches_actual_guarded_native() {
    let (_temp, journal) = journal();
    let client = Task48DurableClient {
        events: Mutex::new(vec![]),
    };
    let output = registered_output(&journal);
    let (result, native, _) =
        crate::cli::launch::tests::task48_fourth_native_fixture(|launcher, launch| {
            start(
                &journal,
                HandoffRequest {
                    automatic: false,
                    launch,
                    ..request()
                },
                claim(),
                "worker",
                &client,
                launcher,
                &TestClock,
                &output,
                &mut Vec::new(),
            )
        });
    result.unwrap();
    assert_eq!(native.len(), 1);
    assert_eq!(
        &native[0].argv[..4],
        ["--fourth-owned", "literal", "", "$HOME $(id)"]
    );
    let input = native[0].argv.last().unwrap();
    assert!(input.starts_with("--initial=Expected handoff command routing"));
    let (registry, _) = crate::cli::launch::tests::fourth_adapter::registry(true, false);
    let policy = registry
        .by_id(registry.agent("fourth").unwrap())
        .unwrap()
        .launch_policy()
        .unwrap();
    assert!(
        policy
            .prepare_startup_input(
                &[],
                &crate::harness::adapter::StartupInputSpec {
                    max_text_bytes: 16384
                }
            )
            .unwrap()
            .is_none()
    );
}

// Literal V1 prompt pieces, retained as data independently of production code.
const V1_INSTRUCTION: &str = " Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `";
const V1_SUFFIX: &str = " is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages.";

fn v1_context(state: &str, host: &str) -> crate::protocol::output::ContinuationContext {
    crate::protocol::output::ContinuationContext {
        state_dir: Some(state.into()),
        host: Some(host.into()),
    }
}

/// Frozen V1 bytes: canonical UUID+JSON escaping, null routing, quoted and
/// control-safe fallback tokens, literal thread and no task body.
#[test]
fn frozen_v1_prompt_literals_bind_routing_quoting_and_thread() {
    use sha2::{Digest, Sha256};
    let null = bootstrap_v1::prompt("t", &v1_context("/state", "/host.sock"), "i");
    let literal = format!(
        "Expected handoff command routing (JSON data): null{V1_INSTRUCTION}herdr-threads --state-dir /state --host-endpoint /host.sock inbox`. The task for thread t{V1_SUFFIX}"
    );
    assert_eq!(null, literal);
    assert_eq!(null.len(), 831);
    assert_eq!(
        format!("{:x}", Sha256::digest(null.as_bytes())),
        "67417931a30b16a76f1a93958eba68b9213858e57b7ec425f27b1ef3c07757fb"
    );
    // Paths that do not exist now: validation uses the retained canonical
    // object verbatim, never today's filesystem lookup.
    let context = v1_context(
        "/nonexistent-ht-v1/a \"b\" 'c'",
        "/nonexistent-ht-v1/h\u{1}\u{200b}.sock",
    );
    let instance = "00000000000040008000000000000ABC";
    let prompts = bootstrap_v1::retained_prompts("t9", &context, instance);
    let fallback = r#"herdr-threads --state-dir '/nonexistent-ht-v1/a "b" '\''c'\''' --host-endpoint $'/nonexistent-ht-v1/h\x01\xe2\x80\x8b.sock' inbox"#;
    assert_eq!(
        prompts,
        vec![
            format!(
                "Expected handoff command routing (JSON data): {{\"instance\":\"00000000-0000-4000-8000-000000000abc\",\"state_dir\":\"/nonexistent-ht-v1/a \\\"b\\\" 'c'\",\"host_endpoint\":\"/nonexistent-ht-v1/h\\u0001\u{200b}.sock\"}}{V1_INSTRUCTION}{fallback}`. The task for thread t9{V1_SUFFIX}"
            ),
            format!(
                "Expected handoff command routing (JSON data): null{V1_INSTRUCTION}{fallback}`. The task for thread t9{V1_SUFFIX}"
            ),
        ]
    );
    assert!(prompts.iter().all(|p| !p.contains("secret durable task")));
    // Today's renderer repeats the filesystem lookup: for the same retained
    // namespace it now yields null, so recomposition could not have accepted a
    // genuine object-routing report once these paths disappeared.
    let current = bootstrap(&ThreadId::new("t9"), &context, instance);
    assert!(current.starts_with("Expected handoff command routing (JSON data): null"));
    assert!(current.contains(fallback));
    assert!(
        current.contains(
            "If no matching hook is trusted, or delivery reports incomplete or unavailable"
        )
    );
    assert!(current.contains("including an empty inbox, needs no inbox call"));
    assert_ne!(
        current, prompts[1],
        "current guidance evolves independently of frozen V1"
    );
    assert_eq!(
        bootstrap_v1::retained_prompts("t9", &context, "not-a-uuid"),
        prompts[1..].to_vec(),
        "an invalid UUID admits only the null-routing form"
    );
    // Baseline golden (not a RED): the V1 producer and today's unperturbed
    // renderer agree for an existing canonical pair.
    let (_temp, journal) = journal();
    let state = journal.root().parent().unwrap().canonicalize().unwrap();
    let context = v1_context(
        state.to_str().unwrap(),
        state.join("h.sock").to_str().unwrap(),
    );
    let produced = bootstrap_v1::prompt("t1", &context, &claim().instance);
    assert_ne!(
        produced,
        bootstrap(&ThreadId::new("t1"), &context, &claim().instance)
    );
    assert_eq!(
        bootstrap_v1::retained_prompts("t1", &context, &claim().instance)[0],
        produced
    );
}

/// A simulated later change to today's renderer must not change the frozen
/// V1 prompt or the template-free manual projection built from it.
#[test]
fn frozen_v1_prompt_and_manual_projection_ignore_current_renderer_drift() {
    let (_temp, journal) = journal();
    let plan = HandoffPlan {
        startup_input: None,
        bootstrap_input: None,
        request: request(),
        context: v1_context("/state", "/host.sock"),
        recipient: SeatId::new("recipient"),
        create_key: OperationId::new("create"),
        invite_key: OperationId::new("invite"),
        send_key: OperationId::new("send"),
    };
    let progress = Progress {
        thread: Some(ThreadId::new("t1")),
        ..Default::default()
    };
    let reference = frozen(&journal);
    let v1 = bootstrap_v1::prompt("t1", &plan.context, &claim().instance);
    let current = bootstrap(&ThreadId::new("t1"), &plan.context, &claim().instance);
    let _drift = crate::harness::launch::current_drift::arm(Some(" DRIFTED"), None);
    assert_eq!(
        bootstrap(&ThreadId::new("t1"), &plan.context, &claim().instance),
        format!("{current} DRIFTED"),
        "the current renderer observes the simulated drift"
    );
    assert_eq!(
        bootstrap_v1::prompt("t1", &plan.context, &claim().instance),
        v1
    );
    let report = report(
        &reference,
        &plan,
        &progress,
        "launch",
        true,
        false,
        &claim(),
    );
    let manual = report["manual_launch_after_confirming_no_start_argv"]
        .as_array()
        .unwrap();
    assert_eq!(manual.last().unwrap(), &serde_json::json!(v1));
}

#[test]
fn handoff_instruction_sources_preserve_message_bytes_and_exclude_conflicts() {
    use crate::cli::commands::{CliAction, parse_argv};
    let (temp, _journal) = journal();
    let path = temp.0.join("instructions");
    for body in [
        " \0\n\r\t".to_owned(),
        "x".repeat(65_536),
        "\u{1}".repeat(65_536),
    ] {
        std::fs::write(&path, &body).unwrap();
        for mode in [
            vec!["--pane", "w1:p2", "--kind", "codex"],
            vec!["--new-tab", "peer", "--kind", "codex"],
            vec!["--existing", "--seat", "recipient"],
        ] {
            for source in [
                vec!["--", body.as_str()],
                vec!["--file", path.to_str().unwrap()],
            ] {
                let mut argv = vec!["ht", "handoff", "--thread", "t1"];
                argv.extend(mode.clone());
                argv.extend(source);
                let parsed = parse_argv(argv).expect("valid source must parse");
                let saved = match parsed.action {
                    CliAction::Handoff(request) => request.body,
                    CliAction::TopologyHandoff(request) => request.body,
                    _ => panic!("wrong action"),
                };
                assert_eq!(saved, body);
            }
        }
    }
    for bytes in [vec![], vec![0xff], vec![b'x'; 65_537]] {
        std::fs::write(&path, bytes).unwrap();
        assert!(
            parse_argv([
                "ht",
                "handoff",
                "--existing",
                "--seat",
                "recipient",
                "--thread",
                "t1",
                "--file",
                path.to_str().unwrap()
            ])
            .is_err()
        );
    }
    std::fs::remove_file(&path).unwrap();
    assert!(
        parse_argv([
            "ht",
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--thread",
            "t1",
            "--file",
            path.to_str().unwrap()
        ])
        .is_err()
    );
    for source in [
        vec![],
        vec!["--stdin", "--", "body"],
        vec!["--file", "missing", "--stdin"],
        vec!["--file", "missing", "--", "body"],
    ] {
        let mut argv = vec![
            "ht",
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--thread",
            "t1",
        ];
        argv.extend(source);
        assert!(parse_argv(argv).is_err());
    }
}

#[test]
fn handoff_stdin_acquisition_is_bounded_and_preserves_exact_message_bytes() {
    use crate::cli::input::read_body_from;
    for body in [
        " \0\n\r\t".to_owned(),
        "x".repeat(65_536),
        "\u{1}".repeat(65_536),
    ] {
        let mut input = std::io::Cursor::new(body.as_bytes());
        assert_eq!(read_body_from(None, None, true, &mut input).unwrap(), body);
    }
    for bytes in [vec![], vec![0xff], vec![b'x'; 65_537]] {
        let mut input = std::io::Cursor::new(bytes);
        assert!(read_body_from(None, None, true, &mut input).is_err());
    }
    let mut input = std::io::Cursor::new(vec![b'x'; 1_000_000]);
    assert!(read_body_from(None, None, true, &mut input).is_err());
    assert_eq!(input.position(), 65_537);
    for body in [" \0\n", "x"] {
        let request = HandoffRequest {
            policy: None,
            body: body.into(),
            ..request()
        };
        request.validate().unwrap();
    }
    let mut request = request();
    request.body = "x".repeat(65_536);
    request.validate().unwrap();
    request.body.push('x');
    assert!(request.validate().is_err());
}

#[test]
fn maximum_instruction_file_freezes_through_explicit_launch_retry() {
    for body in ["x".repeat(65_536), "\u{1}".repeat(65_536)] {
        let (temp, journal) = journal();
        let mut agent = claim();
        agent.harness = Harness::Codex;
        agent.native_session = NativeSessionId::new("session");
        agent.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
        let path = temp.0.join("instructions");
        std::fs::write(&path, &body).unwrap();
        let parsed = crate::cli::commands::parse_argv([
            "ht",
            "handoff",
            "--pane",
            "w1:p2",
            "--kind",
            "codex",
            "--thread",
            "t1",
            "--deadline",
            "7",
            "--file",
            path.to_str().unwrap(),
        ])
        .unwrap();
        let crate::cli::commands::CliAction::Handoff(mut r) = parsed.action else {
            panic!("wrong action")
        };
        if body.starts_with('\u{1}') {
            r.policy.as_mut().unwrap().invitation_confirmation =
                crate::protocol::commands::InvitationConfirmation::RequireAck;
            r.policy.as_mut().unwrap().invite_deadline_millis = Some(11000);
        }
        let expected_confirmation = r.policy.as_ref().unwrap().invitation_confirmation;
        let expected_invite_deadline = r.policy.as_ref().unwrap().invite_deadline_millis;
        r.thread = None;
        r.launch.argv = request().launch.argv;
        r.topic = Some("review".into());
        r.goal = Some("review changes".into());
        let mut client = Client::new(Some("send"));
        client.expected_body = body.clone();
        client.expected_claim = agent.clone();
        let mut launcher = Launcher::default();
        assert!(
            start(
                &journal,
                r,
                agent.clone(),
                "bob",
                &client,
                &mut launcher,
                &TestClock,
                &OutputSpec::default(),
                &mut Vec::new()
            )
            .is_err()
        );
        let reference = reference(&journal);
        let original = std::fs::read(journal.root().join(format!(
            "{:020}-{}.intent",
            reference.ordinal,
            reference.operation.as_str()
        )))
        .unwrap();
        std::fs::write(&path, "changed").unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            std::fs::read(journal.root().join(format!(
                "{:020}-{}.intent",
                reference.ordinal,
                reference.operation.as_str()
            )))
            .unwrap(),
            original
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
        assert!(
            launcher
                .submitted_argv
                .iter()
                .all(|arg| !arg.contains(&body) && !arg.contains(path.to_str().unwrap()))
        );
        assert!(
            journal.load(&reference).is_err(),
            "successful retry cleans the original intent"
        );
        let commands = client.calls.lock().unwrap();
        let sends = commands
            .iter()
            .filter_map(|c| {
                if let Command::SendMessage(q) = c {
                    Some(q)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(sends.len(), 2);
        assert_eq!(sends[0], sends[1]);
        assert_eq!(sends[0].body, body);
        assert_eq!(
            sends[0].delivery_mode,
            crate::protocol::commands::DeliveryMode::Ordinary
        );
        let invite = commands
            .iter()
            .find_map(|c| match c {
                Command::Invite(q) => Some(q),
                _ => None,
            })
            .expect("staged invitation");
        assert_eq!(invite.confirmation, Some(expected_confirmation));
        assert_eq!(invite.deadline_millis, expected_invite_deadline);
        assert_eq!(sends[0].deadline_millis, Some(7000));
        assert_eq!(sends[0].invited_recipients, [SeatId::new("recipient")]);
    }
}

#[test]
fn acquired_stdin_survives_retry_after_reader_is_unavailable() {
    let (_temp, journal) = journal();
    let body = "\u{1}".repeat(65_536);
    let acquired = {
        let mut stdin = std::io::Cursor::new(body.as_bytes());
        crate::cli::input::read_body_from(None, None, true, &mut stdin).unwrap()
    };
    let mut r = request();
    r.body = acquired;
    let mut client = Client::new(Some("send"));
    client.expected_body = body.clone();
    let mut launcher = Launcher::default();
    assert!(
        start(
            &journal,
            r,
            claim(),
            "bob",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        )
        .is_err()
    );
    let reference = reference(&journal);
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
    let commands = client.calls.lock().unwrap();
    let sends = commands
        .iter()
        .filter_map(|command| match command {
            Command::SendMessage(send) => Some(send),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sends.len(), 2);
    assert_eq!(sends[0], sends[1]);
    assert_eq!(sends[0].body, body);
    assert_eq!(launcher.starts, 1);
}

#[test]
fn legacy_handoff_origin_overflow_refuses_before_publication_or_effects() {
    let (_temp, journal) = journal();
    let body = "\u{1}".repeat(65_536);
    let model = format!("--model={}", "\u{1}".repeat(12_000));
    let parsed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--new-thread",
        "--topic",
        "review",
        "--goal",
        "review changes",
        "--agent-arg=-a",
        "--agent-arg=on-request",
        &format!("--agent-arg={model}"),
        &format!("--agent-arg={model}"),
        "--",
        &body,
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong action")
    };
    request.validate().unwrap();
    crate::ports::validate_native_argv(&request.launch.argv).unwrap();
    let registry = crate::harness::registry::builtins();
    let codex = registry.agent("codex").unwrap();
    registry
        .by_id(codex)
        .unwrap()
        .launch_policy()
        .unwrap()
        .validate_native_argv(&request.launch.argv)
        .unwrap();
    let semantic = SemanticMutation::freeze(
        SemanticMutation::Handoff(Box::new(HandoffPlan {
            bootstrap_input: None,
            startup_input: Some(crate::harness::adapter::StartupInputTemplate::positional(
                request.launch.argv.len(),
            )),
            request: request.clone(),
            context: OutputSpec::default().context,
            recipient: SeatId::new("recipient"),
            create_key: OperationId::new("create"),
            invite_key: OperationId::new("invite"),
            send_key: OperationId::new("send"),
        })),
        claim(),
    )
    .unwrap();
    let encoded = serde_json::to_vec(&semantic).unwrap();
    eprintln!(
        "legacy handoff semantic={} cap={}",
        encoded.len(),
        super::super::journal::MAX_DELIVERY_ORIGIN_BYTES
    );
    assert!(
        encoded.len() > super::super::journal::MAX_DELIVERY_ORIGIN_BYTES,
        "semantic alone exceeds the origin cap even before its header/newlines"
    );
    let ordinal_before = std::fs::read(journal.root().join("next-ordinal")).unwrap();
    let mut client = Client::new(Some("send"));
    client.expected_body = body;
    let mut launcher = Launcher::default();
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
    assert!(result.is_err());
    assert!(
        journal.page(&Default::default()).unwrap().items.is_empty(),
        "oversized legacy origin must never become a published intent"
    );
    assert_eq!(
        std::fs::read(journal.root().join("next-ordinal")).unwrap(),
        ordinal_before
    );
    assert!(
        client.calls.lock().unwrap().is_empty(),
        "no downstream canonical commands"
    );
    assert_eq!(launcher.starts, 0, "no native launch");
    assert_eq!(
        frozen(&journal).ordinal,
        1,
        "overflow must not consume an ordinal"
    );
}

#[test]
fn handoff_policy_parses_independent_deadlines_before_body_effects() {
    for target in [
        vec!["--pane", "w1:p2", "--kind", "codex"],
        vec!["--new-tab", "peer", "--kind", "codex"],
        vec!["--existing", "--seat", "recipient"],
    ] {
        let mut argv = vec!["ht", "handoff"];
        argv.extend(target);
        argv.extend([
            "--thread",
            "t1",
            "--deadline",
            "7",
            "--invite-deadline",
            "11",
            "--",
            "instructions",
        ]);
        let parsed = crate::cli::commands::parse_argv(argv).expect("independent policy flags");
        let policy = match parsed.action {
            crate::cli::commands::CliAction::Handoff(q) => {
                serde_json::to_value(q).unwrap()["policy"].clone()
            }
            crate::cli::commands::CliAction::TopologyHandoff(q) => {
                serde_json::to_value(q.policy).unwrap()
            }
            _ => panic!("wrong route"),
        };
        assert!(policy.is_object());
        if policy.is_object() {
            assert_eq!(policy["deadline_millis"], 7000);
            assert_eq!(policy["invite_deadline_millis"], 11000);
            assert_eq!(policy["invitation_confirmation"], "require_ack");
        }
    }
}

#[test]
fn handoff_invitations_always_require_ack_and_the_old_flag_is_gone() {
    let parsed = crate::cli::commands::parse_argv([
        "ht", "handoff", "--pane", "w1:p2", "--kind", "codex", "--thread", "t1", "--", "go",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong route")
    };
    assert_eq!(
        serde_json::to_value(request).unwrap()["policy"]["invitation_confirmation"],
        "require_ack"
    );
    // ht-yha.61: the removed flag names its replacement instead of a bare clap usage error.
    let removed = crate::cli::commands::parse_argv([
        "ht",
        "handoff",
        "--pane",
        "w1:p2",
        "--kind",
        "codex",
        "--thread",
        "t1",
        "--require-invite-ack",
        "--",
        "go",
    ])
    .expect_err("--require-invite-ack is refused");
    assert_eq!(
        removed.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    assert_eq!(
        removed.detail,
        "--require-invite-ack was removed: handoff invitations always require acceptance; omit it"
    );
}

#[test]
fn handoff_policy_refuses_unsupported_daemon_before_journal_or_launch() {
    struct Old(Client, Vec<String>);
    impl LocalClient for Old {
        fn call_with_output(
            &self,
            command: Command,
            _: &OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(command, budget)
        }
        fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
            if matches!(command, Command::Capabilities) {
                return Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList {
                        capabilities: self.1.clone(),
                    },
                ));
            }
            self.0.call(command, budget)
        }
    }
    for advertised in [
        vec![],
        vec![crate::protocol::capabilities::SEND_TIME_RECEIPTS_V1.to_owned()],
    ] {
        let (_temp, journal) = journal();
        let mut request = request();
        request.policy = Some(crate::protocol::handoff::HandoffPolicy {
            deadline_millis: Some(7000),
            invitation_confirmation: crate::protocol::commands::InvitationConfirmation::Lazy,
            invite_deadline_millis: None,
        });
        let client = Old(Client::new(None), advertised);
        let mut launcher = Launcher::default();
        let result = start(
            &journal,
            request,
            claim(),
            "peer",
            &client,
            &mut launcher,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        );
        assert!(
            matches!(
                result,
                Err(RunError::Api(ApiError {
                    code: ErrorCode::Unsupported,
                    ..
                }))
            ),
            "{result:?}"
        );
        assert_eq!((launcher.preflights, launcher.starts), (0, 0));
        assert!(client.0.calls.lock().unwrap().is_empty());
        assert!(!std::fs::read_dir(journal.root()).unwrap().any(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "intent")
        }));
    }
}

#[test]
fn automatic_presentation_admission_counts_exact_local_root() {
    let (_temp, journal) = journal();
    let namespace = crate::protocol::handoff::HandoffNamespace {
        instance: claim().instance,
        state_dir: "/tmp/automatic-state".into(),
        host_endpoint: "/tmp/automatic-host.sock".into(),
    };
    let selection = crate::protocol::results::DirectThreadEnsured::admission_witness(
        ThreadId::new("t"),
        SeatId::new("recipient"),
    );
    let summary = crate::protocol::handoff::automatic_launch_summary(&serde_json::json!({"outcome":"started","pane":"w1:p2","seat":"recipient","harness":"codex","argv":["codex"]}),&"f".repeat(64)).unwrap();
    validate_automatic_presentation_admission(
        journal.root(),
        &namespace,
        &summary,
        &selection,
        serde_json::Value::Null,
    )
    .unwrap();
    // Pure admission guard: such a root cannot be opened on the filesystem.
    // Count every path byte rather than assuming provider/local path finiteness.
    let oversized_root = std::path::PathBuf::from(format!("/{}", "x".repeat(1024 * 1024)));
    assert!(
        validate_automatic_presentation_admission(
            &oversized_root,
            &namespace,
            &summary,
            &selection,
            serde_json::Value::Null
        )
        .is_err()
    );
    assert!(journal.resolve_recovery_ref("local:1").is_err());
}

// ht-yha.65 (main review M5): a person's handoff keeps the RequireAck policy the parser builds; no dead Lazy
// request is sent (the daemon ignores it; invitations always confirm).
#[test]
fn a_human_handoff_requests_the_confirming_invitation_policy() {
    let parsed = crate::cli::commands::parse_argv([
        "ht", "human", "handoff", "--pane", "w1:p2", "--kind", "codex", "--thread", "t1", "--",
        "go",
    ])
    .unwrap();
    let crate::cli::commands::CliAction::Handoff(request) = parsed.action else {
        panic!("wrong route")
    };
    assert_eq!(
        serde_json::to_value(request).unwrap()["policy"]["invitation_confirmation"],
        "require_ack"
    );
}
