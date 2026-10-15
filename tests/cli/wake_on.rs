use super::*;
use crate::cli::commands::{ParseFailure, parse_argv, parse_argv_or_informational};
use crate::protocol::{
    capabilities::ADVERTISED,
    ids::*,
    results::{CheckInContextDisposition, CheckInResult, SeatInspection},
    time::UtcMillis,
};

fn empty<T>() -> crate::protocol::pagination::Page<T> {
    use crate::protocol::pagination::{Consistency, Page, StopReason};
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

/// A daemon double: a seat whose stored preference starts at `stored`.
struct Client {
    calls: std::sync::Mutex<Vec<Command>>,
    stored: std::sync::Mutex<WakeOn>,
    advertise: bool,
    generation: std::sync::atomic::AtomicU64,
}
impl LocalClient for Client {
    fn supports_capability(&self, name: &str, _: &crate::protocol::time::CallBudget) -> bool {
        self.advertise && name == SEAT_WAKE_ON_V1
    }
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
        use crate::protocol::results::{ContinuityStatus, MappingStatus, SeatSummary};
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::SeatInspect(q) => Ok(CommandResult::SeatInspect(SeatInspection {
                summary: SeatSummary {
                    seat: q.seat,
                    continuity: ContinuityStatus::Resolved,
                    target: Some(HostTargetId::new("pane")),
                    generation: self.generation.load(std::sync::atomic::Ordering::SeqCst),
                    created_at: UtcMillis(0),
                    retired_at: None,
                    active: true,
                    availability: None,
                    bindings: Vec::new(),
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
                    fast_forward: None,
                    first_contact: None,
                }))
            }
            Command::WakeOn(q) => Ok(CommandResult::WakeOn(WakeOnState {
                seat: q.seat,
                wake_on: *self.stored.lock().unwrap(),
                changed: false,
            })),
            Command::SetWakeOn(set) => {
                let mut stored = self.stored.lock().unwrap();
                let changed = *stored != set.value;
                *stored = set.value;
                Ok(CommandResult::WakeOn(WakeOnState {
                    seat: set.claim.seat,
                    wake_on: set.value,
                    changed,
                }))
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

struct Fixture {
    root: std::path::PathBuf,
    paths: NodePaths,
    instance: crate::node::NodeUuid,
    selection: CooperativeSelection,
    client: Client,
}
impl Fixture {
    fn new(stored: WakeOn) -> Self {
        let root = std::env::temp_dir().join(format!("cli-wake-on-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            root.join("state"),
            root.join("host.sock"),
            None,
        )
        .unwrap();
        let fixture = Self {
            paths: NodePaths::resolve(&runtime.state_dir).unwrap(),
            root,
            instance: crate::node::NodeUuid::from_u128(1),
            selection: CooperativeSelection {
                seat: SeatId::new("seat"),
                target: HostTargetId::new("pane"),
                harness: crate::harness::context::Harness::Codex,
                role: Role::TopLevel,
            },
            client: Client {
                calls: Default::default(),
                stored: std::sync::Mutex::new(stored),
                advertise: true,
                generation: std::sync::atomic::AtomicU64::new(1),
            },
        };
        let parsed = parse_argv([
            "herdr-threads",
            "check-in",
            "--lifecycle-event",
            "launch-one",
        ])
        .unwrap();
        crate::cli::run_selected(
            parsed,
            &fixture.selection,
            &fixture.paths,
            fixture.instance,
            &fixture.client,
            &crate::app::SystemClock::new(),
            &mut Vec::new(),
        )
        .unwrap();
        fixture
            .client
            .generation
            .store(2, std::sync::atomic::Ordering::SeqCst);
        fixture.client.calls.lock().unwrap().clear();
        fixture
    }
    fn run(&self, argv: &[&str]) -> (Result<(), RunError>, String) {
        let parsed = parse_argv(argv).unwrap();
        let mut out = Vec::new();
        let result = run(
            parsed,
            &self.selection,
            &self.paths,
            self.instance,
            &self.client,
            &crate::app::SystemClock::new(),
            &mut out,
        );
        (result, String::from_utf8(out).unwrap())
    }
    fn commands(&self) -> Vec<Command> {
        self.client.calls.lock().unwrap().clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---- parsing ----

#[test]
fn wake_on_parses_a_value_or_none() {
    assert_eq!(
        parse_argv(["herdr-threads", "wake-on"]).unwrap().action,
        CliAction::WakeOn(None)
    );
    assert_eq!(
        parse_argv(["herdr-threads", "wake-on", "all"])
            .unwrap()
            .action,
        CliAction::WakeOn(Some(WakeOn::All))
    );
    assert_eq!(
        parse_argv(["herdr-threads", "wake-on", "nudge"])
            .unwrap()
            .action,
        CliAction::WakeOn(Some(WakeOn::Nudge))
    );
}

#[test]
fn an_invalid_value_is_a_usage_error_exit_2() {
    for argv in [
        vec!["herdr-threads", "wake-on", "bogus"],
        vec!["herdr-threads", "wake-on", "ALL"],
        vec!["herdr-threads", "wake-on", "all", "nudge"],
    ] {
        let error = match parse_argv_or_informational(argv.clone()) {
            Err(ParseFailure::Invalid(error)) => error,
            other => panic!("{argv:?} should be invalid, got {other:?}"),
        };
        assert_eq!(
            crate::cli::exit::api_exit_code(&error.code),
            crate::cli::exit::EXIT_USAGE,
            "{argv:?}"
        );
    }
}

// ---- running ----

#[test]
fn set_all_text_reports_the_previous_value() {
    let fixture = Fixture::new(WakeOn::Nudge);
    let (result, out) = fixture.run(&["herdr-threads", "wake-on", "all"]);
    result.unwrap();
    assert_eq!(out, "wake_on: all (was nudge)\n");
    let sent = fixture.commands();
    let Command::SetWakeOn(set) = sent.last().unwrap() else {
        panic!("expected SetWakeOn, got {sent:?}");
    };
    assert_eq!(set.value, WakeOn::All);
    assert_eq!(set.claim.seat.as_str(), "seat");
    assert_eq!(
        set.claim.role,
        crate::protocol::authority::CallerRole::TopLevel
    );
}

#[test]
fn setting_the_stored_value_reports_it_as_the_previous_one() {
    let fixture = Fixture::new(WakeOn::All);
    let (result, out) = fixture.run(&["herdr-threads", "wake-on", "all"]);
    result.unwrap();
    assert_eq!(out, "wake_on: all (was all)\n");
}

#[test]
fn bare_wake_on_reads_without_a_claim() {
    let fixture = Fixture::new(WakeOn::Nudge);
    let (result, out) = fixture.run(&["herdr-threads", "wake-on"]);
    result.unwrap();
    assert_eq!(out, "wake_on: nudge\n");
    assert_eq!(
        fixture.commands(),
        vec![Command::WakeOn(WakeOnQuery {
            seat: SeatId::new("seat")
        })]
    );
}

#[test]
fn json_carries_seat_wake_on_and_changed() {
    let fixture = Fixture::new(WakeOn::Nudge);
    let (result, out) = fixture.run(&["herdr-threads", "wake-on", "all", "--json"]);
    result.unwrap();
    let value: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(value["result"]["kind"], "wake_on");
    assert_eq!(
        value["result"]["data"],
        serde_json::json!({"seat":"seat","wake_on":"all","changed":true})
    );
    assert!(out.ends_with('\n'));
}

#[test]
fn a_subagent_cannot_set_and_sends_nothing() {
    let mut fixture = Fixture::new(WakeOn::Nudge);
    fixture.selection.role = Role::Subagent;
    let (result, out) = fixture.run(&["herdr-threads", "wake-on", "all"]);
    assert!(result.is_err());
    assert!(out.is_empty());
    assert!(fixture.commands().is_empty());
}

#[test]
fn a_daemon_without_the_capability_is_refused_before_any_call() {
    let mut fixture = Fixture::new(WakeOn::Nudge);
    fixture.client.advertise = false;
    let (result, out) = fixture.run(&["herdr-threads", "wake-on"]);
    let error = result.unwrap_err();
    assert_eq!(error.exit_code(), crate::cli::exit::EXIT_UNSUPPORTED);
    assert!(out.is_empty());
    assert!(fixture.commands().is_empty());
}

// ---- wire ----

#[test]
fn wire_shapes_round_trip_and_wake_on_is_snake_case() {
    assert_eq!(serde_json::to_value(WakeOn::Nudge).unwrap(), "nudge");
    assert_eq!(serde_json::to_value(WakeOn::All).unwrap(), "all");
    assert_eq!(WakeOn::default(), WakeOn::Nudge);
    let claim = crate::protocol::authority::CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: crate::protocol::authority::CallerRole::TopLevel,
        harness: crate::protocol::authority::Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("e"),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    };
    for command in [
        Command::SetWakeOn(SetWakeOn {
            value: WakeOn::All,
            operation: OperationId::new("op"),
            claim: claim.clone(),
        }),
        Command::WakeOn(WakeOnQuery {
            seat: SeatId::new("s"),
        }),
    ] {
        assert!(command.validate().is_ok());
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    }
    let json = serde_json::to_value(Command::SetWakeOn(SetWakeOn {
        value: WakeOn::All,
        operation: OperationId::new("op"),
        claim: claim.clone(),
    }))
    .unwrap();
    assert_eq!(json["kind"], "set_wake_on");
    assert_eq!(json["args"]["value"], "all");
    let state = CommandResult::WakeOn(WakeOnState {
        seat: SeatId::new("s"),
        wake_on: WakeOn::All,
        changed: true,
    });
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["kind"], "wake_on");
    assert_eq!(
        serde_json::from_value::<CommandResult>(json).unwrap(),
        state
    );
    // A declared subagent's claim is refused at validation.
    let mut subagent = claim;
    subagent.role = crate::protocol::authority::CallerRole::Subagent;
    assert!(
        Command::SetWakeOn(SetWakeOn {
            value: WakeOn::All,
            operation: OperationId::new("op"),
            claim: subagent,
        })
        .validate()
        .is_err()
    );
    // Unknown fields and values are rejected rather than ignored.
    assert!(serde_json::from_value::<WakeOn>(serde_json::json!("bogus")).is_err());
    assert!(
        serde_json::from_value::<WakeOnQuery>(serde_json::json!({"seat":"s","extra":1})).is_err()
    );
}

#[test]
fn the_capability_is_advertised() {
    assert_eq!(SEAT_WAKE_ON_V1, "seat.wake_on_v1");
    assert!(ADVERTISED.contains(&SEAT_WAKE_ON_V1));
}
