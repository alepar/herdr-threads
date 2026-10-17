//! The runner side of the neutral hook seams (agy-adapter D4 and D6): `decode_refined` folds an
//! adapter's local evidence before any budget or branch uses the event, and a declared
//! `hook_step` runs for top-level `Observer` events only. The adapter is test-local, wrapped
//! around the synthetic fourth fixture, in a test `Registry`; the daemon is a scripted
//! in-process service.
use super::*;
use crate::protocol::{
    commands::SeatInspectQuery,
    results::{
        CheckInContextDisposition, CheckInResult, ContinuityStatus, MappingStatus, SeatInspection,
        SeatSummary,
    },
};
use crate::test_support::{isolation::TestIsolation, synthetic_fourth::ADAPTER};
use std::ffi::OsString;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

/// One `local_evidence` call as the adapter saw it.
struct Seen {
    deadline: Instant,
    state_dir: Option<PathBuf>,
    home: Option<OsString>,
    pane_session: Option<Option<String>>,
}

/// One `hook_step` run as the step saw it.
struct Ran {
    event_id: String,
    claim: Option<crate::protocol::authority::CallerClaim>,
    state_dir: Option<PathBuf>,
    command_prefix: Vec<String>,
    daemon_answer: Option<bool>,
    deadline: Instant,
}

struct Step {
    runs: Mutex<Vec<Ran>>,
    output: Vec<u8>,
}
impl HookStep for Step {
    fn run(&self, context: &HookStepContext<'_>) -> Vec<u8> {
        let daemon_answer = context.daemon.map(|daemon| {
            daemon
                .call(Command::SeatInspect(SeatInspectQuery {
                    seat: SeatId::new("callback-seat"),
                    page: PageRequest {
                        cursor: None,
                        limit: 1,
                        max_bytes: MAX_PAGE_BYTES,
                    },
                }))
                .is_ok()
        });
        self.runs.lock().unwrap().push(Ran {
            event_id: context.decoded.event_id.clone(),
            claim: context.claim.clone(),
            state_dir: context.state_dir.clone(),
            command_prefix: context.command_prefix.clone(),
            daemon_answer,
            deadline: context.deadline,
        });
        self.output.clone()
    }
}

/// The synthetic fixture with scripted local evidence and an optional step.
struct Evidenced {
    evidence: LocalEvidence,
    call_pane_session: bool,
    seen: Mutex<Vec<Seen>>,
    step: Option<Step>,
}
impl HarnessAdapter for Evidenced {
    type Admission = ();
    fn metadata(&self) -> &'static AdapterMetadata {
        ADAPTER.metadata()
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        ADAPTER.contracts()
    }
    fn hook_admission_policy(&self) -> HookAdmissionPolicy {
        HookAdmissionPolicy::RegisteredContract
    }
    fn observe_install(&self, e: &InstallEnvironment, b: &CallBudget) -> InstallObservation {
        ADAPTER.observe_install(e, b)
    }
    fn admit(&self, r: &AdmissionRequest, b: &CallBudget) -> AdmissionDecision<()> {
        ADAPTER.admit(r, b)
    }
    fn version_ladder(&self, r: &RuntimeIdentity) -> Ladder {
        ADAPTER.version_ladder(r)
    }
    fn classify(&self, i: &HookInput) -> ContractObservation {
        ADAPTER.classify(i)
    }
    fn decode(&self, a: &(), i: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
        ADAPTER.decode(a, i)
    }
    fn encode(
        &self,
        a: &(),
        e: &DecodedEvent,
        o: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        ADAPTER.encode(a, e, o)
    }
    fn attribute_runtime(&self, i: &HookInput, b: &CallBudget) -> RuntimeAttribution {
        ADAPTER.attribute_runtime(i, b)
    }
    fn setup(&self, r: &SetupRequest, b: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
        ADAPTER.setup(r, b)
    }
    fn status(&self, r: &StatusRequest, b: &CallBudget) -> SetupStatus {
        ADAPTER.status(r, b)
    }
    fn unsetup(&self, r: &UnsetupRequest, b: &CallBudget) -> Result<RemovalOutcome, SetupFailure> {
        ADAPTER.unsetup(r, b)
    }
    fn local_evidence(&self, request: &LocalEvidenceRequest<'_>) -> LocalEvidence {
        self.seen.lock().unwrap().push(Seen {
            deadline: request.deadline,
            state_dir: request.state_dir.map(Path::to_path_buf),
            home: request.home.map(OsString::from),
            pane_session: self.call_pane_session.then(|| (request.pane_session)()),
        });
        self.evidence.clone()
    }
    fn hook_step(&self) -> Option<&dyn HookStep> {
        self.step.as_ref().map(|step| step as &dyn HookStep)
    }
}

fn adapter(
    evidence: LocalEvidence,
    call_pane_session: bool,
    step: Option<&[u8]>,
) -> &'static Evidenced {
    Box::leak(Box::new(Evidenced {
        evidence,
        call_pane_session,
        seen: Mutex::default(),
        step: step.map(|output| Step {
            runs: Mutex::default(),
            output: output.to_vec(),
        }),
    }))
}

fn registration(adapter: &'static Evidenced) -> &'static Registration {
    let registrations = Box::leak(Box::new([Registration::new(adapter)]));
    let registry = Box::leak(Box::new(
        crate::harness::registry::Registry::new(registrations).unwrap(),
    ));
    registry
        .by_id(registry.agent(ADAPTER.metadata().id).unwrap())
        .unwrap()
}

const PRESENT_ABSENT: RoleWitness = RoleWitness {
    top_level_marker: Witness::Present,
    child_listing: Witness::Absent,
};
const ABSENT_UNCHECKED: RoleWitness = RoleWitness {
    top_level_marker: Witness::Absent,
    child_listing: Witness::Unchecked,
};

fn herdr() -> HookEnv {
    HookEnv {
        herdr_env: true,
        pane: Some("w9:p1".into()),
    }
}

fn payload(event: &str, event_id: &str, session: &str, role: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "event": event, "event_id": event_id, "session_id": session, "role": role
    }))
    .unwrap()
}

fn empty<T>() -> crate::protocol::pagination::Page<T> {
    crate::protocol::pagination::Page {
        items: vec![],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: crate::protocol::pagination::StopReason::Complete,
        consistency: crate::protocol::pagination::Consistency::BoundedLive,
    }
}

/// A scripted daemon service that records every command it is sent.
#[derive(Default)]
struct Service(Mutex<Vec<Command>>);
impl Service {
    fn count(&self, matching: impl Fn(&Command) -> bool) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matching(c))
            .count()
    }
}
impl crate::ports::LocalService for Service {
    fn service_control(
        &self,
        _: Command,
        _: crate::protocol::authority::PeerIdentity,
        _: &str,
        _: &str,
        _: &crate::service::live_gate::LiveServiceGate,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(crate::test_support::unserved("no service control"))
    }
    fn audit_service_disconnect(
        &self,
        _: &str,
        _: u64,
        _: crate::protocol::authority::PeerIdentity,
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        Err(crate::test_support::unserved("no service disconnect"))
    }
    fn service_operation(
        &self,
        _: crate::protocol::service::ServiceOperation,
        _: &crate::ports::ServiceConnectionAuthority,
        _: &dyn crate::ports::ServiceAuthorityGate,
        _: &CallBudget,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        Err(crate::test_support::unserved("no service operation"))
    }
    fn handle_with_output(
        &self,
        c: Command,
        p: crate::protocol::authority::PeerIdentity,
        b: &CallBudget,
        _: &OutputSpec,
    ) -> Result<CommandResult, ApiError> {
        self.handle(c, p, b)
    }
    fn handle(
        &self,
        c: Command,
        _: crate::protocol::authority::PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.lock().unwrap().push(c.clone());
        let summary = || SeatSummary {
            seat: SeatId::new("callback-seat"),
            continuity: ContinuityStatus::Resolved,
            target: Some(HostTargetId::new("w9:p1")),
            generation: 1,
            created_at: crate::protocol::time::UtcMillis(0),
            retired_at: None,
            active: true,
            availability: None,
            bindings: Vec::new(),
        };
        match c {
            Command::Seats(_) => {
                let mut page = empty();
                page.items.push(summary());
                Ok(CommandResult::Seats(page))
            }
            Command::ResolveSeat(_) => {
                Ok(CommandResult::SeatResolved(SeatId::new("callback-seat")))
            }
            Command::SeatInspect(_) => Ok(CommandResult::SeatInspect(SeatInspection {
                summary: summary(),
                mapping: MappingStatus {
                    state: ContinuityStatus::Resolved,
                    target: Some(HostTargetId::new("w9:p1")),
                    detail_argv: None,
                },
                hold: None,
                retirement: None,
                open_binding: None,
                history: empty(),
            })),
            Command::Directory(_) => Ok(CommandResult::Directory(empty())),
            Command::AttentionDigest(_) => {
                use crate::protocol::attention::{AttentionClass, AttentionDigest};
                let none = || AttentionClass {
                    count: 0,
                    items: vec![],
                    has_more: false,
                    count_has_more: false,
                };
                Ok(CommandResult::AttentionDigest(AttentionDigest {
                    version: 1,
                    seat: SeatId::new("callback-seat"),
                    token: Default::default(),
                    invitations: none(),
                    receipts: none(),
                    warnings: none(),
                    unavailability_open: false,
                    mod_channel_live: false,
                    lazy: None,
                    mail: None,
                }))
            }
            Command::CheckIn(check_in) => {
                assert_eq!(check_in.claim.harness.as_str(), ADAPTER.metadata().id);
                // The check-in opens the successor of the seat's generation 1.
                let mut claim = check_in.claim;
                claim.binding_generation = 2;
                Ok(CommandResult::CheckedIn(CheckInResult {
                    seat: claim.seat.clone(),
                    context: claim,
                    context_disposition: CheckInContextDisposition::Current,
                    offered_through: None,
                    warning_count: 0,
                    warning_count_has_more: false,
                    warnings: empty(),
                    notices: Default::default(),
                    inbox: empty(),
                    fast_forward: None,
                    first_contact: None,
                }))
            }
            _ => Err(crate::test_support::unserved("unused fixture operation")),
        }
    }
}

struct Server {
    shutdown: Cancellation,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.worker.take().unwrap().join().unwrap();
    }
}

/// An isolated state root; with `serve`, an in-process daemon answers on its endpoint.
struct Fixture {
    iso: TestIsolation,
    args: HookArgs,
    service: Arc<Service>,
    _server: Option<Server>,
}
fn fixture(label: &str, serve: bool) -> Fixture {
    let iso = TestIsolation::new(label);
    let root = iso.state_root();
    let mut args = HookArgs {
        state_dir: Some(root.join("state")),
        host_endpoint: Some(root.join("host.sock")),
        harness: Harness::Claude,
        event: None,
    };
    args.harness = crate::harness::registry::OccupantHarness::Agent(
        crate::harness::registry::builtins()
            .agent(ADAPTER.metadata().id)
            .unwrap(),
    )
    .into();
    let service = Arc::new(Service::default());
    let server = serve.then(|| {
        use crate::daemon::ownership::OwnerLock;
        let context = RuntimeContext::explicit(
            args.state_dir.clone().unwrap(),
            args.host_endpoint.clone().unwrap(),
            None,
        )
        .unwrap();
        let paths = NodePaths::resolve(&context.state_dir).unwrap();
        let owner = OwnerLock::acquire_with_test_hint(&paths).unwrap();
        let instance = owner.node_uuid().unwrap();
        let listener = owner.bind_socket().unwrap();
        owner
            .publish_endpoint(
                &listener,
                env!("CARGO_PKG_VERSION"),
                crate::protocol::wire::PROTOCOL_VERSION,
            )
            .unwrap();
        let handler = Arc::clone(&service);
        let shutdown = Cancellation::default();
        let stopped = shutdown.clone();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let _ = crate::daemon::transport::serve(
                    listener.into_async().unwrap(),
                    instance,
                    handler,
                    Arc::new(SystemClock::new()),
                    crate::daemon::paths::effective_uid(),
                    stopped,
                )
                .await
                .unwrap();
            });
            drop(owner);
        });
        Server {
            shutdown,
            worker: Some(worker),
        }
    });
    if !serve {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(args.state_dir.as_ref().unwrap())
            .unwrap();
    }
    Fixture {
        iso,
        args,
        service,
        _server: server,
    }
}

fn run(
    fixture: &Fixture,
    registration: &'static Registration,
    event: &str,
    bytes: &[u8],
) -> HookOutcome {
    let mut args = fixture.args.clone();
    args.event = Some(event.into());
    run_hook_registered(
        registration,
        &args,
        &InstalledHarness::Claude("unused".into()),
        bytes,
        &herdr(),
        Instant::now() + LIFECYCLE_BUDGET,
        Arc::new(SystemClock::new()),
        None,
    )
}

fn context_text(outcome: &HookOutcome) -> String {
    let value: serde_json::Value = serde_json::from_slice(&outcome.stdout).unwrap();
    value["synthetic_context"].as_str().unwrap_or("").to_owned()
}

fn check_ins(fixture: &Fixture) -> usize {
    fixture
        .service
        .count(|command| matches!(command, Command::CheckIn(_)))
}

fn inspects(fixture: &Fixture) -> usize {
    fixture
        .service
        .count(|command| matches!(command, Command::SeatInspect(_)))
}

fn only_evidence(role: RoleWitness) -> LocalEvidence {
    LocalEvidence {
        role: Some(role),
        lifecycle: None,
    }
}

/// Kills: an `Unknown` event dropped before the evidence fold (no check-in ever happens), and a
/// witness folded after the branch that already returned.
#[test]
fn unknown_with_positive_witnesses_becomes_top_level_and_checks_in() {
    let fixture = fixture("local-evidence-top", true);
    let adapter = adapter(only_evidence(PRESENT_ABSENT), false, None);
    let outcome = run(
        &fixture,
        registration(adapter),
        "SyntheticStart",
        &payload("SyntheticStart", "e1", "s1", "unknown"),
    );
    assert_eq!(outcome.diagnostic, None);
    assert_eq!(check_ins(&fixture), 1, "one check-in is sent");
    assert!(
        context_text(&outcome).contains("Before using threads"),
        "the top-level context is rendered: {:?}",
        String::from_utf8_lossy(&outcome.stdout)
    );
    assert_eq!(adapter.seen.lock().unwrap().len(), 1);
}

/// Kills: an unwitnessed `Unknown` that checks in (a child would then act as the seat), one that
/// renders the top-level context, and one that sends any daemon request at all.
#[test]
fn unknown_without_witnesses_is_a_child_with_a_local_restriction_and_no_daemon_request() {
    let fixture = fixture("local-evidence-child", true);
    let adapter = adapter(only_evidence(ABSENT_UNCHECKED), false, None);
    let outcome = run(
        &fixture,
        registration(adapter),
        "SyntheticStart",
        &payload("SyntheticStart", "e1", "s1", "unknown"),
    );
    assert_eq!(outcome.diagnostic, None);
    assert!(
        context_text(&outcome).contains(crate::harness::CHILD_RESTRICTION),
        "the subagent restriction is rendered locally: {:?}",
        String::from_utf8_lossy(&outcome.stdout)
    );
    assert!(!context_text(&outcome).contains("Before using threads"));
    assert!(
        fixture.service.0.lock().unwrap().is_empty(),
        "a child sends no daemon request: {:?}",
        fixture.service.0.lock().unwrap()
    );
}

/// Kills: no evidence call for an adapter that declares none (a default adapter must see the
/// old `Unknown` handling), and an `Unknown` that is not silent without evidence.
#[test]
fn unknown_without_evidence_stays_silent() {
    let fixture = fixture("local-evidence-none", true);
    let adapter = adapter(LocalEvidence::default(), false, None);
    let outcome = run(
        &fixture,
        registration(adapter),
        "SyntheticStart",
        &payload("SyntheticStart", "e1", "s1", "unknown"),
    );
    assert_eq!(outcome, HookOutcome::default());
    assert!(fixture.service.0.lock().unwrap().is_empty());
}

/// Kills: a lifecycle reading applied after the event bound is computed (a resume found at a
/// turn start would keep the observer budget), or an Observer event that stays an Observer.
#[test]
fn lifecycle_evidence_raises_the_event_bound_to_the_lifecycle_budget() {
    let fixture = fixture("local-evidence-bound", false);
    let input = HookInput {
        bytes: payload("SyntheticObserver", "e1", "s1", "unknown"),
        registered_event: Some("SyntheticObserver".into()),
    };
    let mut args = fixture.args.clone();
    args.event = Some("SyntheticObserver".into());
    let bound = |evidence: LocalEvidence| {
        let adapter = adapter(evidence, false, None);
        let registration = registration(adapter);
        let admitted = registration
            .admit(
                &hook_admission_request(
                    registration,
                    InstallObservation::NotRequested,
                    &input,
                    &SystemClock::new(),
                ),
                &budget(Instant::now() + TOOL_BUDGET, &SystemClock::new()),
            )
            .unwrap();
        let decoded = decode_refined(
            registration,
            &admitted,
            &input,
            &args,
            &herdr(),
            Instant::now(),
            &SystemClock::new(),
        )
        .unwrap();
        (event_bound(registration, &decoded), decoded)
    };
    let metadata = ADAPTER.metadata();
    let (observer, decoded) = bound(LocalEvidence::default());
    assert!(matches!(decoded.intent, EventIntent::Observer));
    assert_eq!(
        observer,
        crate::protocol::time::external_bound(Duration::from_millis(metadata.budget.observer_ms))
    );
    let (lifecycle, decoded) = bound(LocalEvidence {
        role: Some(PRESENT_ABSENT),
        lifecycle: Some(EventKind::Resume),
    });
    assert!(matches!(
        decoded.intent,
        EventIntent::Lifecycle(EventKind::Resume)
    ));
    assert_eq!(decoded.role, EventRole::TopLevel);
    assert_eq!(
        lifecycle,
        crate::protocol::time::external_bound(Duration::from_millis(metadata.budget.lifecycle_ms))
    );
    assert!(lifecycle > observer);
}

/// Kills: an evidence deadline that is not half of the provisional (pre-refinement) bound, a
/// missing or wrong HOME, and a state dir that is not `<node_dir>/adapter-state/<id>/`.
#[test]
fn local_evidence_request_carries_a_half_bound_deadline_home_and_state_dir() {
    let fixture = fixture("local-evidence-request", false);
    let input = HookInput {
        bytes: payload("SyntheticObserver", "e1", "s1", "top_level"),
        registered_event: Some("SyntheticObserver".into()),
    };
    let mut args = fixture.args.clone();
    args.event = Some("SyntheticObserver".into());
    // The evidence lifts the intent to a lifecycle, but its own deadline came from the
    // observer bound of the event as decoded.
    let adapter = adapter(
        LocalEvidence {
            role: None,
            lifecycle: Some(EventKind::Resume),
        },
        false,
        None,
    );
    let registration = registration(adapter);
    let admitted = registration
        .admit(
            &hook_admission_request(
                registration,
                InstallObservation::NotRequested,
                &input,
                &SystemClock::new(),
            ),
            &budget(Instant::now() + TOOL_BUDGET, &SystemClock::new()),
        )
        .unwrap();
    let started = Instant::now();
    decode_refined(
        registration,
        &admitted,
        &input,
        &args,
        &herdr(),
        started,
        &SystemClock::new(),
    )
    .unwrap();
    let seen = adapter.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let observer = crate::protocol::time::external_bound(Duration::from_millis(
        ADAPTER.metadata().budget.observer_ms,
    ));
    assert_eq!(seen[0].deadline, started + observer / 2);
    assert_eq!(seen[0].home, std::env::var_os("HOME"));
    let context = RuntimeContext::explicit(
        args.state_dir.clone().unwrap(),
        args.host_endpoint.clone().unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(
        seen[0].state_dir,
        Some(
            crate::daemon::paths::node_dir(&context.state_dir)
                .join("adapter-state")
                .join(ADAPTER.metadata().id)
        )
    );
    assert!(
        !seen[0].state_dir.as_ref().unwrap().exists(),
        "the evidence request only names the state dir: it creates nothing"
    );
}

/// Kills: a `pane_session` that is eager (a scan before the adapter asks), one that returns a
/// foreign pane's or another harness's session, and one that does not read the pane's current
/// context at all.
#[test]
fn pane_session_is_lazy_and_reads_the_pane_context_native_session() {
    let fixture = fixture("local-evidence-pane", true);
    // Establish the pane's current context through a real top-level check-in.
    let setup = adapter(LocalEvidence::default(), false, None);
    let outcome = run(
        &fixture,
        registration(setup),
        "SyntheticStart",
        &payload("SyntheticStart", "e1", "native-1", "top_level"),
    );
    assert_eq!(outcome.diagnostic, None);
    assert_eq!(check_ins(&fixture), 1);
    assert!(
        setup.seen.lock().unwrap()[0].pane_session.is_none(),
        "the adapter did not ask for the pane session"
    );

    let asking = adapter(LocalEvidence::default(), true, None);
    run(
        &fixture,
        registration(asking),
        "SyntheticObserver",
        &payload("SyntheticObserver", "e2", "native-2", "top_level"),
    );
    assert_eq!(
        asking.seen.lock().unwrap()[0].pane_session,
        Some(Some("native-1".to_owned())),
        "the pane context's native session, whatever the event's own"
    );
}

/// Kills: a hook step that runs for a child, for a non-Observer event, or without a declaring
/// adapter; one that claims a session the pane context does not hold; one that starts a daemon
/// or drops the state dir, prefix or daemon handle; and an output that is not the step's.
#[test]
fn hook_step_runs_for_top_level_observer_events_with_claim_state_dir_and_daemon() {
    let fixture = fixture("local-evidence-step", true);
    let adapter = adapter(LocalEvidence::default(), false, Some(b"step-output"));
    let registration = registration(adapter);
    let step = adapter.step.as_ref().unwrap();
    // The pane's current context comes from a real check-in.
    run(
        &fixture,
        registration,
        "SyntheticStart",
        &payload("SyntheticStart", "start", "native-1", "top_level"),
    );
    assert_eq!(check_ins(&fixture), 1);
    assert!(step.runs.lock().unwrap().is_empty(), "not a step event");

    let inspects_before = inspects(&fixture);
    let before = Instant::now();
    // Two observer events (the PreInvocation-like and the Stop-like shape): both run the step.
    for event_id in ["pre", "stop"] {
        let outcome = run(
            &fixture,
            registration,
            "SyntheticObserver",
            &payload("SyntheticObserver", event_id, "native-1", "top_level"),
        );
        assert_eq!(outcome.stdout, b"step-output", "{event_id}");
        assert_eq!(outcome.attention, None);
    }
    let after = Instant::now();
    let observer = crate::protocol::time::external_bound(Duration::from_millis(
        ADAPTER.metadata().budget.observer_ms,
    ));
    {
        let runs = step.runs.lock().unwrap();
        assert_eq!(
            runs.iter().map(|r| r.event_id.as_str()).collect::<Vec<_>>(),
            ["pre", "stop"]
        );
        for ran in runs.iter() {
            let claim = ran.claim.as_ref().expect("the pane claim");
            assert_eq!(claim.native_session.as_str(), "native-1");
            assert_eq!(claim.harness.as_str(), ADAPTER.metadata().id);
            assert_eq!(claim.seat.as_str(), "callback-seat");
            assert_eq!(
                claim.liveness,
                Some(crate::protocol::authority::LivenessFingerprint::TEST),
                "the step's claim carries the hook's fingerprint"
            );
            assert_eq!(
                ran.daemon_answer,
                Some(true),
                "a running daemon is handed over"
            );
            assert_eq!(
                ran.command_prefix.first().map(String::as_str),
                Some(CLI_ARGV0)
            );
            let state_dir = ran.state_dir.as_ref().expect("state dir");
            assert!(state_dir.ends_with(format!("adapter-state/{}", ADAPTER.metadata().id)));
            assert_eq!(
                std::fs::metadata(state_dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert!(
                ran.deadline >= before + observer && ran.deadline <= after + observer,
                "the step is bound by the event's observer budget"
            );
        }
    }
    assert_eq!(check_ins(&fixture), 1, "a step event sends no check-in");
    assert_eq!(
        inspects(&fixture) - inspects_before,
        2,
        "each step run reaches the daemon through the handed-over client"
    );

    // A different native session than the pane context: no claim.
    run(
        &fixture,
        registration,
        "SyntheticObserver",
        &payload("SyntheticObserver", "other", "native-2", "top_level"),
    );
    assert!(step.runs.lock().unwrap()[2].claim.is_none());

    // Not run for a child's observer event or a top-level tool event.
    let before = step.runs.lock().unwrap().len();
    let child = run(
        &fixture,
        registration,
        "SyntheticObserver",
        &payload("SyntheticObserver", "child", "native-1", "child"),
    );
    run(
        &fixture,
        registration,
        "SyntheticTool",
        &payload("SyntheticTool", "tool", "native-1", "top_level"),
    );
    assert_eq!(step.runs.lock().unwrap().len(), before);
    assert_ne!(child.stdout, b"step-output");
}

/// Kills: a hook step that starts a daemon when none runs (it would create an endpoint), one
/// that is handed a daemon anyway, and one that gets no state dir without a daemon.
#[test]
fn hook_step_without_a_running_daemon_gets_none_and_starts_none() {
    let fixture = fixture("local-evidence-nodaemon", false);
    let adapter = adapter(LocalEvidence::default(), false, Some(b"quiet"));
    let outcome = run(
        &fixture,
        registration(adapter),
        "SyntheticObserver",
        &payload("SyntheticObserver", "pre", "native-1", "top_level"),
    );
    assert_eq!(outcome.stdout, b"quiet");
    let runs = adapter.step.as_ref().unwrap().runs.lock().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].daemon_answer, None);
    assert!(runs[0].claim.is_none());
    let state_dir = runs[0].state_dir.as_ref().expect("state dir");
    assert!(state_dir.is_dir());
    let context = RuntimeContext::explicit(
        fixture.args.state_dir.clone().unwrap(),
        fixture.args.host_endpoint.clone().unwrap(),
        None,
    )
    .unwrap();
    let node_dir = crate::daemon::paths::node_dir(&context.state_dir);
    for started in ["endpoint.json", "owner.lock", "namespace"] {
        assert!(
            !node_dir.join(started).exists(),
            "the step must not start a daemon: {started}"
        );
    }
    drop(fixture.iso);
}

/// Kills: an adapter that declares no step still going through the step branch (its observer
/// event keeps the neutral output).
#[test]
fn an_adapter_without_a_step_keeps_the_neutral_observer_output() {
    let fixture = fixture("local-evidence-nostep", false);
    let adapter = adapter(LocalEvidence::default(), false, None);
    let outcome = run(
        &fixture,
        registration(adapter),
        "SyntheticObserver",
        &payload("SyntheticObserver", "pre", "native-1", "top_level"),
    );
    assert_eq!(outcome.stdout, br#"{"synthetic_context":null}"#);
}
