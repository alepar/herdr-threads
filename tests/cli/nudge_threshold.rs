use super::*;
use crate::cli::commands::{
    CliAction, CooperativeSelection, ParseFailure, parse_argv, parse_argv_or_informational,
};
use crate::protocol::{
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

/// A daemon double holding one seat's threshold.
struct Client {
    calls: std::sync::Mutex<Vec<Command>>,
    threshold: std::sync::Mutex<NudgeThreshold>,
    generation: std::sync::atomic::AtomicU64,
    /// When set, the daemon refuses threshold reads with this error.
    threshold_refusal: std::sync::Mutex<Option<crate::protocol::results::ApiError>>,
}
impl LocalClient for Client {
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
        let mut stored = self.threshold.lock().unwrap();
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
                    directory: None,
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
                directory: None,
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
            Command::NudgeThreshold(_) if self.threshold_refusal.lock().unwrap().is_some() => {
                Err(self.threshold_refusal.lock().unwrap().clone().unwrap())
            }
            Command::NudgeThreshold(q) => Ok(CommandResult::NudgeThreshold(NudgeThresholdState {
                seat: q.seat,
                threshold: *stored,
                changed: false,
            })),
            Command::SetNudgeThreshold(set) => {
                let changed = *stored != set.value;
                *stored = set.value;
                Ok(CommandResult::NudgeThreshold(NudgeThresholdState {
                    seat: set.claim.seat,
                    threshold: set.value,
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
    fn new(stored: i64) -> Self {
        let root = std::env::temp_dir().join(format!("cli-nudge-thr-{}", uuid::Uuid::new_v4()));
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
                threshold: std::sync::Mutex::new(NudgeThreshold::new(stored).unwrap()),
                generation: std::sync::atomic::AtomicU64::new(1),
                threshold_refusal: Default::default(),
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
            &crate::test_support::test_host_locator(),
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
        let result = crate::cli::run_selected(
            parsed,
            &self.selection,
            &self.paths,
            self.instance,
            &crate::test_support::test_host_locator(),
            &self.client,
            &crate::app::SystemClock::new(),
            &mut out,
        );
        (result, String::from_utf8(out).unwrap())
    }
    fn commands(&self) -> Vec<Command> {
        self.client.calls.lock().unwrap().clone()
    }
    /// Commands other than the claim-proving seat inspection.
    fn mutating_or_reading(&self) -> Vec<Command> {
        self.commands()
            .into_iter()
            .filter(|c| !matches!(c, Command::SeatInspect(_)))
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---- parsing ----

fn parsed_value(argv: &[&str]) -> Option<NudgeThreshold> {
    match parse_argv(argv).unwrap().action {
        CliAction::NudgeThreshold(value) => value,
        other => panic!("expected nudge-threshold, got {other:?}"),
    }
}

#[test]
fn parse_reads_sets_and_clamps() {
    assert_eq!(parsed_value(&["herdr-threads", "nudge-threshold"]), None);
    assert_eq!(
        parsed_value(&["herdr-threads", "nudge-threshold", "3"]),
        Some(NudgeThreshold::new(3).unwrap())
    );
    assert_eq!(
        parsed_value(&["herdr-threads", "nudge-threshold", "9"]),
        Some(NudgeThreshold::new(9).unwrap())
    );
    for negative in ["-1", "-5"] {
        assert_eq!(
            parsed_value(&["herdr-threads", "nudge-threshold", negative]),
            Some(NudgeThreshold::ALL),
            "{negative}"
        );
    }
}

#[test]
fn parse_refuses_out_of_range_and_junk_with_exit_2() {
    for argv in [
        vec!["herdr-threads", "nudge-threshold", "10"],
        vec!["herdr-threads", "nudge-threshold", "abc"],
        vec!["herdr-threads", "nudge-threshold", "1", "2"],
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

// ---- running against a daemon with the capability ----

#[test]
fn set_text_reports_changed_then_unchanged() {
    let fixture = Fixture::new(0);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "3"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 3 (changed)\n");
    let sent = fixture.mutating_or_reading();
    let [Command::SetNudgeThreshold(set)] = sent.as_slice() else {
        panic!("expected one SetNudgeThreshold, got {sent:?}");
    };
    assert_eq!(set.value.get(), 3);
    assert_eq!(set.claim.seat.as_str(), "seat");
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "3"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 3 (unchanged)\n");
}

#[test]
fn a_negative_set_sends_minus_one() {
    let fixture = Fixture::new(0);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "-7"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: -1 (changed)\n");
    let sent = fixture.mutating_or_reading();
    let [Command::SetNudgeThreshold(set)] = sent.as_slice() else {
        panic!("expected one SetNudgeThreshold, got {sent:?}");
    };
    assert_eq!(set.value, NudgeThreshold::ALL);
}

#[test]
fn bare_reads_without_a_claim() {
    let fixture = Fixture::new(2);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 2\n");
    assert_eq!(
        fixture.commands(),
        vec![Command::NudgeThreshold(NudgeThresholdQuery {
            seat: SeatId::new("seat")
        })]
    );
}

#[test]
fn json_encodes_the_daemon_result_as_returned() {
    let fixture = Fixture::new(0);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "4", "--json"]);
    result.unwrap();
    let value: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(value["result"]["kind"], "nudge_threshold");
    assert_eq!(
        value["result"]["data"],
        serde_json::json!({"seat":"seat","threshold":4,"changed":true})
    );
}

#[test]
fn a_subagent_cannot_set_and_sends_nothing() {
    let mut fixture = Fixture::new(0);
    fixture.selection.role = Role::Subagent;
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "2"]);
    assert!(result.is_err());
    assert!(out.is_empty());
    assert!(fixture.commands().is_empty());
}

// ---- the wake-on alias ----

#[test]
fn wake_on_aliases_to_the_threshold() {
    let fixture = Fixture::new(0);
    let (result, out) = fixture.run(&["herdr-threads", "wake-on", "all"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: -1 (changed)\n");
    let sent = fixture.mutating_or_reading();
    let [Command::SetNudgeThreshold(set)] = sent.as_slice() else {
        panic!("expected one SetNudgeThreshold, got {sent:?}");
    };
    assert_eq!(set.value, NudgeThreshold::ALL);
    let (result, out) = fixture.run(&["herdr-threads", "wake-on", "nudge"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 0 (changed)\n");
    let (result, out) = fixture.run(&["herdr-threads", "wake-on"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 0\n");
}

#[test]
fn a_remote_seat_refusal_reaches_the_user() {
    let fixture = Fixture::new(0);
    *fixture.client.threshold_refusal.lock().unwrap() =
        Some(crate::protocol::results::ApiError::new(
            crate::protocol::results::ErrorCode::CallerUnverified,
            "seat s is homed on node bx1; its nudge threshold is set there",
        ));
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold"]);
    let Err(RunError::Api(error)) = result else {
        panic!("expected the daemon's refusal, got {result:?}");
    };
    assert!(
        error.detail.contains("homed on node bx1"),
        "{}",
        error.detail
    );
    assert!(out.is_empty());
}
