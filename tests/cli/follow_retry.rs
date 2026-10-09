//! `read --follow` retry budget, notices and Ctrl-C (root §B10 Decision 3).
//! Mounted from `src/cli/follow.rs`.

use super::*;
use crate::{
    app::SystemClock,
    cli::exit::api_exit_code,
    client::local::LocalSocketClient,
    protocol::{ids::ThreadId, results::ErrorClass},
};
use std::{
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    sync::atomic::AtomicBool,
};

fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError::new(code, detail)
}

/// What the client reports for a connect that timed out.
fn slow_connect() -> ApiError {
    error(ErrorCode::HostUnavailable, "daemon connect timed out")
}

struct Capture {
    out: Vec<u8>,
    err: Vec<u8>,
}

impl Capture {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            err: Vec::new(),
        }
    }
    fn printer(&mut self, form: Form) -> Printer<'_> {
        Printer {
            form,
            style: Style::plain(),
            no_system: false,
            writer: &mut self.out,
            errors: &mut self.err,
        }
    }
    fn text(&self) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&self.out),
            String::from_utf8_lossy(&self.err)
        )
    }
}

fn thread() -> ThreadId {
    ThreadId::new("thread-Ab12Cd34")
}

/// Kills: classifying a timed-out connect by its `host_unavailable` code
/// (class `Unavailable`), which announced "lost the daemon" at once.
#[test]
fn a_slow_connect_is_transient_and_a_refused_one_is_not() {
    assert_eq!(
        crate::client::error_class(&slow_connect()),
        Some(ErrorClass::Transient)
    );
    assert_eq!(classify(&slow_connect()), FailureClass::Transient);
    // The OS-level timeout goes through the shared connect-error mapping.
    let os = crate::client::connect_error(
        &io::Error::from(io::ErrorKind::TimedOut),
        Path::new("/run/x.sock"),
    );
    assert_eq!(classify(&os), FailureClass::Transient, "{}", os.detail);
    let refused = crate::client::connect_error(
        &io::Error::from(io::ErrorKind::ConnectionRefused),
        Path::new("/run/x.sock"),
    );
    assert_eq!(classify(&refused), FailureClass::ConnectionLost);
    assert_eq!(
        classify(&error(
            ErrorCode::HostUnavailable,
            "daemon connection unavailable"
        )),
        FailureClass::ConnectionLost
    );
}

/// Kills: a single timed-out connect printing a notice, or leaving a
/// "reconnected" notice owed.
#[test]
fn one_slow_connect_is_transient_and_retried() {
    let mut capture = Capture::new();
    let mut outage = Outage::default();
    let step = failed_poll(
        &mut outage,
        &slow_connect(),
        Instant::now(),
        &thread(),
        &mut capture.printer(Form::Human),
    )
    .unwrap();
    assert_eq!(step, Step::Retry);
    assert_eq!(capture.text(), "", "no notice for one slow connect");
    assert!(
        !outage.recovered(),
        "nothing was announced, nothing to undo"
    );
}

/// Kills: announcing before the budget, announcing every poll after it, and
/// printing the fatal notice twice (once as a notice, once as the error).
#[test]
fn lost_daemon_announced_once_after_the_backoff_budget() {
    let mut capture = Capture::new();
    let mut outage = Outage::default();
    let start = Instant::now();
    let thread = thread();
    let mut poll = |at: Duration, error: &ApiError, capture: &mut Capture| {
        failed_poll(
            &mut outage,
            error,
            start + at,
            &thread,
            &mut capture.printer(Form::Human),
        )
    };
    // Inside the budget: quiet.
    for secs in [0, 2, 5, 9] {
        assert_eq!(
            poll(Duration::from_secs(secs), &slow_connect(), &mut capture).unwrap(),
            Step::Retry
        );
    }
    assert_eq!(
        capture.text(),
        "",
        "quiet inside the {QUIET_OUTAGE:?} budget"
    );
    // The budget is exhausted: exactly one announcement, however long it lasts.
    for secs in [10, 12, 20, 40, 80] {
        assert_eq!(
            poll(Duration::from_secs(secs), &slow_connect(), &mut capture).unwrap(),
            Step::Retry
        );
    }
    assert_eq!(
        capture.text().matches("lost the daemon").count(),
        1,
        "{}",
        capture.text()
    );
    // The daemon then keeps refusing: one fatal notice, and an exit status
    // that the top level does not report a second time.
    let refusal = error(ErrorCode::StoreCorrupt, "store is corrupt");
    for _ in 1..DEFINITIVE_LIMIT {
        assert_eq!(
            poll(Duration::from_secs(90), &refusal, &mut capture).unwrap(),
            Step::Retry
        );
    }
    let end = poll(Duration::from_secs(90), &refusal, &mut capture).unwrap_err();
    assert!(
        matches!(end, RunError::Exit(code) if code == api_exit_code(&ErrorCode::StoreCorrupt)),
        "{end:?}"
    );
    let text = capture.text();
    assert_eq!(text.matches("stopped following").count(), 1, "{text}");
    assert_eq!(text.matches("lost the daemon").count(), 1, "{text}");
    assert!(text.contains("store is corrupt"), "{text}");
}

/// Kills: a fatal notice that drops the remedy the top-level error used to
/// print.
#[test]
fn the_fatal_notice_carries_the_restart_remedy() {
    let mut capture = Capture::new();
    let mut outage = Outage::default();
    let mut refusal = error(ErrorCode::Unauthorized, "refused");
    refusal.restart_argv = Some(vec![
        "herdr-threads".into(),
        "daemon".into(),
        "ensure".into(),
    ]);
    let mut result = Ok(Step::Retry);
    for _ in 0..DEFINITIVE_LIMIT {
        result = failed_poll(
            &mut outage,
            &refusal,
            Instant::now(),
            &thread(),
            &mut capture.printer(Form::Human),
        );
    }
    assert!(matches!(result, Err(RunError::Exit(_))));
    assert!(
        capture
            .text()
            .contains("restart: herdr-threads daemon ensure"),
        "{}",
        capture.text()
    );
}

/// Kills: a notice built from daemon text that reaches the terminal raw, in
/// either form (stdout transcript line, stderr line).
#[test]
fn every_notice_goes_through_the_escaper() {
    let hostile = "bad\u{1b}[2J\u{202e}\nforged";
    for form in [Form::Human, Form::Lines] {
        let mut capture = Capture::new();
        let mut printer = capture.printer(form);
        // lost-daemon (announce), fatal, and gone notices all embed text.
        let mut outage = Outage::default();
        failed_poll(
            &mut outage,
            &error(ErrorCode::HostUnavailable, hostile),
            Instant::now(),
            &thread(),
            &mut printer,
        )
        .unwrap();
        let mut fatal = Outage::default();
        let mut last = Ok(Step::Retry);
        for _ in 0..DEFINITIVE_LIMIT {
            last = failed_poll(
                &mut fatal,
                &error(ErrorCode::StoreCorrupt, hostile),
                Instant::now(),
                &thread(),
                &mut printer,
            );
        }
        assert!(last.is_err());
        printer.notice("reconnected to the daemon").unwrap();
        let text = capture.text();
        for raw in ['\u{1b}', '\u{202e}'] {
            assert!(
                !text.contains(raw),
                "{form:?} printed {raw:?} raw: {text:?}"
            );
        }
        assert!(text.contains("\\u{001b}"), "{form:?}: {text:?}");
        assert!(text.contains("\\u{202e}"), "{form:?}: {text:?}");
        // The embedded newline cannot start a forged line.
        assert!(!text.contains("\nforged"), "{form:?}: {text:?}");
        assert_eq!(text.matches("lost the daemon").count(), 1, "{text}");
        assert_eq!(text.matches("stopped following").count(), 1, "{text}");
        assert_eq!(text.matches("reconnected").count(), 1, "{text}");
    }
}

fn private_dir(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// Kills: the render path taking `context.lock` (shared or exclusive): with a
/// check-in holding it, the old read waited out its 200 ms lock timeout and
/// gave up, so the harness tag silently vanished from the transcript.
#[test]
fn nick_cache_render_path_takes_no_context_lock() {
    let root = std::env::temp_dir().join(format!("ht-nick-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    // The journal refuses symlinked ancestors (macOS `/var` is one).
    let root = root.canonicalize().unwrap();
    let instance = uuid::Uuid::new_v4();
    let seat = SeatId::new("seat-a");
    let instance_dir = root.join("instance");
    let dir = instance_dir
        .join("contexts")
        .join(format!("{:x}", Sha256::digest(seat.as_str().as_bytes())));
    private_dir(&dir);
    let state = serde_json::json!({
        "version": 1, "instance": instance, "seat": "seat-a",
        "current": {
            "format_version": 1, "instance": instance, "seat": "seat-a",
            "target": "w1:p1", "harness": "Claude", "binding_generation": 3,
            "execution": uuid::Uuid::new_v4(),
            "session": {"PluginContext": uuid::Uuid::new_v4()}, "role": "TopLevel"
        },
        "pending": null, "completed": []
    });
    let json = dir.join("context.json");
    std::fs::write(&json, state.to_string()).unwrap();
    std::fs::set_permissions(&json, std::fs::Permissions::from_mode(0o600)).unwrap();
    let lock_path = dir.join("context.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    lock.lock().unwrap();

    // Control: the locking reader cannot get in while the lock is held.
    let journal =
        ContextJournal::open(&dir, instance, "seat-a", Duration::from_millis(50)).unwrap();
    assert!(
        journal.current().is_err(),
        "the fixture must really hold the lock"
    );

    let started = Instant::now();
    let harness = read_harness(&instance_dir, instance, &seat, 3);
    let took = started.elapsed();
    assert_eq!(harness, Some("claude"));
    assert!(
        took < Duration::from_millis(100),
        "waited {took:?} on the lock"
    );
    assert_eq!(read_harness(&instance_dir, instance, &seat, 4), None);
    drop(lock);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A daemon that accepts and never answers.
fn silent_daemon(path: &Path) -> UnixListener {
    let listener = UnixListener::bind(path).unwrap();
    let accepting = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while let Ok((stream, _)) = accepting.accept() {
            held.push(stream);
        }
    });
    listener
}

/// Kills: an interrupt that is only noticed between calls: with the daemon
/// silent, the call waits out its whole 5 s budget before Ctrl-C is honoured.
#[test]
fn ctrl_c_during_a_call_returns_promptly() {
    let socket =
        std::env::temp_dir().join(format!("ht-{}.s", &uuid::Uuid::new_v4().to_string()[..8]));
    let _daemon = silent_daemon(&socket);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = LocalSocketClient::new(
        socket.clone(),
        Arc::clone(&clock),
        uuid::Uuid::new_v4(),
        Some(uuid::Uuid::new_v4()),
    );
    // The same chain a real signal takes: flag -> watcher -> Cancellation.
    let flag: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    let cancel = Cancellation::default();
    watch_interrupt(flag, cancel.clone());
    let interrupted_at = Arc::new(std::sync::Mutex::new(None));
    let marker = Arc::clone(&interrupted_at);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        *marker.lock().unwrap() = Some(Instant::now());
        flag.store(true, Ordering::SeqCst);
    });
    let result = history(
        &client,
        &thread(),
        Some(HistoryRange::After { sequence: 0 }),
        None,
        10,
        &OutputSpec::default(),
        &budget(clock.as_ref(), 5_000, &cancel),
    );
    let returned = Instant::now();
    let interrupted = interrupted_at
        .lock()
        .unwrap()
        .expect("the call must outlive the 150 ms before the interrupt");
    assert!(result.is_err(), "a silent daemon never answers");
    let lag = returned.duration_since(interrupted);
    assert!(
        lag < Duration::from_millis(100),
        "returned {lag:?} after Ctrl-C"
    );
    assert!(cancel.is_cancelled());
    let _ = std::fs::remove_file(&socket);
}

/// Kills: `pause` sleeping out its interval after an interrupt.
#[test]
fn a_pause_ends_at_once_on_interrupt() {
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        trigger.cancel();
    });
    let started = Instant::now();
    assert!(!pause(Duration::from_secs(5), &cancel));
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(pause(Duration::from_millis(5), &Cancellation::default()));
}

struct CompletedRetryFixture {
    dir: std::path::PathBuf,
    journal: crate::cli::journal::Journal,
    reference: crate::cli::journal::IntentRef,
    claim: crate::protocol::authority::CallerClaim,
    context: crate::protocol::output::ContinuationContext,
}
impl CompletedRetryFixture {
    fn new(harness: crate::protocol::authority::Harness) -> Self {
        use crate::{
            cli::{
                handoff::{HandoffPlan, HandoffRequest},
                journal::{IntentScope, Journal, SemanticMutation},
                launch::LaunchRequest,
            },
            protocol::{
                authority::{CallerClaim, CallerRole},
                ids::*,
            },
        };
        let dir = std::env::temp_dir().join(format!("ht-completed-retry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let state = dir.join("state");
        let host = dir.join("host.sock");
        let runtime =
            crate::daemon::paths::RuntimeContext::explicit(state.clone(), host.clone(), None)
                .unwrap();
        let paths = crate::daemon::paths::InstancePaths::resolve_read_only(&runtime).unwrap();
        let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
        let context = crate::protocol::output::ContinuationContext {
            state_dir: Some(runtime.state_dir.to_string_lossy().into_owned()),
            host: Some(runtime.host_endpoint.to_string_lossy().into_owned()),
        };
        let claim = CallerClaim {
            instance: "00000000-0000-0000-0000-0000000000b1".into(),
            seat: SeatId::new("sender"),
            target: HostTargetId::new("w1:p1"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness,
            native_session: NativeSessionId::new("original-session"),
            execution: ExecutionId::new("original-execution"),
        };
        let plan = HandoffPlan {
            startup_input: None,
            request: HandoffRequest {
                thread: Some(ThreadId::new("t1")),
                thread_name: None,
                topic: None,
                goal: None,
                body: "frozen body".into(),
                launch: LaunchRequest {
                    target: HostTargetId::new("w1:p2"),
                    harness: crate::harness::context::Harness::Codex,
                    harness_binary: None,
                    argv: vec![],
                    name: None,
                    pane_label: None,
                },
            },
            context: context.clone(),
            recipient: SeatId::new("recipient"),
            create_key: OperationId::new("original-create"),
            invite_key: OperationId::new("original-invite"),
            send_key: OperationId::new("original-send"),
        };
        let reference = journal
            .record(
                IntentScope::Cooperative {
                    instance: claim.instance.clone(),
                    seat: claim.seat.clone(),
                },
                SemanticMutation::freeze(SemanticMutation::Handoff(Box::new(plan)), claim.clone())
                    .unwrap(),
                1,
            )
            .unwrap();
        std::fs::write(
            journal
                .root()
                .join(format!("handoff-{}.progress", reference.operation.as_str())),
            br#"{"thread":"t1","launch":{"outcome":"started"},"possible_start":false}"#,
        )
        .unwrap();
        std::fs::remove_file(journal.root().join("allocator.lock")).unwrap();
        Self {
            dir,
            journal,
            reference,
            claim,
            context,
        }
    }
    fn argv(&self, human: bool) -> Vec<String> {
        let mut argv = vec!["ht".into()];
        if human {
            argv.push("human".into());
        }
        argv.extend([
            "--state-dir".into(),
            self.context.state_dir.clone().unwrap(),
            "--host-endpoint".into(),
            self.context.host.clone().unwrap(),
            "retry".into(),
            self.reference.recovery_ref(),
        ]);
        argv
    }
    fn snapshot(&self) -> std::collections::BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(self.journal.root())
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }
}
impl Drop for CompletedRetryFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).unwrap();
    }
}

struct CompletedRetryClient {
    identity: crate::protocol::handoff::HandoffIdentity,
    calls: std::sync::atomic::AtomicUsize,
}
impl CompletedRetryClient {
    fn new(fixture: &CompletedRetryFixture) -> Self {
        use crate::cli::journal::SemanticMutation;
        let pending = fixture.journal.load(&fixture.reference).unwrap();
        let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
            panic!("expected frozen")
        };
        let SemanticMutation::Handoff(plan) = *mutation else {
            panic!("expected handoff")
        };
        Self {
            identity: crate::protocol::handoff::HandoffIdentity {
                compound: fixture.reference.operation.clone(),
                digest: pending.header.semantic_digest,
                claim,
                thread: plan.request.thread,
                recipient: plan.recipient,
                create_key: plan.create_key,
                invite_key: plan.invite_key,
                send_key: plan.send_key,
            },
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}
impl crate::ports::LocalClient for CompletedRetryClient {
    fn call(
        &self,
        command: crate::protocol::commands::Command,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, ApiError> {
        let crate::protocol::commands::Command::BeginHandoff(query) = command else {
            panic!("terminal replay must not inspect a live binding or mutate: {command:?}")
        };
        assert_eq!(
            query.identity, self.identity,
            "replay must send the exact original claim, digest and operation keys"
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(crate::protocol::results::CommandResult::Handoff(
            crate::protocol::handoff::HandoffResult {
                compound: query.identity.compound,
                thread: Some(ThreadId::new("t1")),
                state: crate::protocol::handoff::HandoffState::Completed,
            },
        ))
    }
    fn call_with_output(
        &self,
        command: crate::protocol::commands::Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, ApiError> {
        self.call(command, budget)
    }
}

#[test]
fn completed_human_retry_root_refuses_without_output_or_cleanup() {
    let fixture = CompletedRetryFixture::new(crate::protocol::authority::Harness::Human);
    let before = fixture.snapshot();
    let mut output = Vec::new();
    let result = crate::cli::run_in_pane(fixture.argv(false), Some("w1:p1"), &mut output);
    assert!(
        format!("{result:?}").contains("person/operator retry requires immediate human namespace; use herdr-threads human --state-dir"),
        "{result:?}"
    );
    assert!(output.is_empty());
    assert_eq!(
        fixture.snapshot(),
        before,
        "refusal must retain intent, progress, allocator and format bytes without adding locks"
    );
    assert_eq!(
        crate::cli::retry::preflight_original_actor(
            fixture.journal.root(),
            &fixture.reference.recovery_ref(),
            crate::cli::actor_route::InvocationActor::Human,
            &crate::protocol::output::ContinuationContext::default()
        )
        .unwrap(),
        crate::cli::journal::OriginalActor::HumanOrOperator
    );
    assert_eq!(fixture.snapshot(), before);
    let client = CompletedRetryClient::new(&fixture);
    let parsed = crate::cli::commands::parse_argv(fixture.argv(true)).unwrap();
    assert!(
        crate::cli::handoff::try_completed_retry(
            &parsed,
            &fixture.journal,
            &fixture.claim.instance,
            Some("w1:p1"),
            &fixture.context,
            &client,
            &SystemClock::new(),
            &mut output
        )
        .unwrap()
    );
    assert!(!output.is_empty());
    assert!(fixture.journal.load(&fixture.reference).is_err());
    assert_eq!(client.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn completed_agent_retry_survives_live_human_binding() {
    use crate::harness::context::{Harness, OccupantContext, Role, SessionReference};
    let fixture = CompletedRetryFixture::new(crate::protocol::authority::Harness::Codex);
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        fixture.context.state_dir.as_ref().unwrap().into(),
        fixture.context.host.as_ref().unwrap().into(),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve_read_only(&runtime).unwrap();
    let instance = uuid::Uuid::parse_str(&fixture.claim.instance).unwrap();
    let live = crate::cli::seat_contexts(&paths, instance, &fixture.claim.seat).unwrap();
    live.install_reattached(OccupantContext {
        format_version: 1,
        instance,
        seat: fixture.claim.seat.as_str().into(),
        target: "w1:p1".into(),
        harness: Harness::Human,
        binding_generation: 2,
        execution: uuid::Uuid::new_v4(),
        session: SessionReference::PluginContext(uuid::Uuid::new_v4()),
        role: Role::TopLevel,
    })
    .unwrap();
    assert_eq!(live.current().unwrap().unwrap().harness, Harness::Human);
    let before = fixture.snapshot();
    assert_eq!(
        crate::cli::retry::preflight_original_actor(
            fixture.journal.root(),
            &fixture.reference.recovery_ref(),
            crate::cli::actor_route::InvocationActor::Agent,
            &crate::protocol::output::ContinuationContext::default()
        )
        .unwrap(),
        crate::cli::journal::OriginalActor::Agent
    );
    assert_eq!(fixture.snapshot(), before);
    let client = CompletedRetryClient::new(&fixture);
    let parsed = crate::cli::commands::parse_argv(fixture.argv(false)).unwrap();
    struct FailedFlush;
    impl io::Write for FailedFlush {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("injected output failure"))
        }
    }
    assert!(
        crate::cli::handoff::try_completed_retry(
            &parsed,
            &fixture.journal,
            &fixture.claim.instance,
            Some("w1:p1"),
            &fixture.context,
            &client,
            &SystemClock::new(),
            &mut FailedFlush
        )
        .is_err()
    );
    let after = fixture.snapshot();
    for (name, bytes) in &before {
        assert_eq!(
            after.get(name),
            Some(bytes),
            "output failure must preserve {name}"
        );
    }
    let mut output = Vec::new();
    assert!(
        crate::cli::handoff::try_completed_retry(
            &parsed,
            &fixture.journal,
            &fixture.claim.instance,
            Some("w1:p1"),
            &fixture.context,
            &client,
            &SystemClock::new(),
            &mut output
        )
        .unwrap()
    );
    assert!(!output.is_empty());
    assert_eq!(client.calls.load(Ordering::SeqCst), 2);
    assert!(fixture.journal.load(&fixture.reference).is_err());
    assert_eq!(live.current().unwrap().unwrap().harness, Harness::Human);
}

#[test]
fn completed_human_retry_malformed_record_retained_without_output() {
    let fixture = CompletedRetryFixture::new(crate::protocol::authority::Harness::Human);
    let intent = fixture.journal.root().join(format!(
        "{:020}-{}.intent",
        fixture.reference.ordinal,
        fixture.reference.operation.as_str()
    ));
    let bytes = std::fs::read_to_string(&intent).unwrap();
    std::fs::write(&intent, bytes.replace("frozen body", "corrupted body")).unwrap();
    let before = fixture.snapshot();
    for human in [false, true] {
        let mut output = Vec::new();
        let result = crate::cli::run_in_pane(fixture.argv(human), Some("w1:p1"), &mut output);
        assert!(
            format!("{result:?}").contains("intent semantic mismatch"),
            "{result:?}"
        );
        assert!(output.is_empty());
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn frozen_human_ordinary_retry_root_refuses() {
    let fx = crate::cli::cooperative_tests::OrdinaryHumanRetryFixture::new(true);
    let before = fx.snapshot();
    let mut output = vec![];
    let failure = crate::cli::run_in_pane(fx.argv(false), Some("w:p1"), &mut output).unwrap_err();
    assert!(
        matches!(failure, crate::cli::RunError::Io(ref error) if error.to_string().contains("person/operator retry requires immediate human namespace")),
        "{failure:?}"
    );
    assert!(output.is_empty());
    assert_eq!(fx.snapshot(), before);
    assert!(
        fx.client.calls.lock().unwrap().is_empty(),
        "root refusal precedes transport and effects"
    );
}
