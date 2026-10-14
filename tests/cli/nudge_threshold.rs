use super::*;
use crate::cli::commands::{
    CliAction, CooperativeSelection, ParseFailure, parse_argv, parse_argv_or_informational,
};
use crate::protocol::{
    commands::{WakeOn, WakeOnQuery},
    ids::*,
    results::{CheckInContextDisposition, CheckInResult, SeatInspection, WakeOnState},
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

/// A daemon double holding one seat's threshold (modern) or wake preference (old).
struct Client {
    calls: std::sync::Mutex<Vec<Command>>,
    threshold: std::sync::Mutex<NudgeThreshold>,
    modern: bool,
    wake_on: bool,
    generation: std::sync::atomic::AtomicU64,
}
impl LocalClient for Client {
    fn supports_capability(&self, name: &str, _: &crate::protocol::time::CallBudget) -> bool {
        (self.modern && name == SEAT_NUDGE_THRESHOLD_V1)
            || (self.wake_on && name == SEAT_WAKE_ON_V1)
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
            Command::WakeOn(q) => Ok(CommandResult::WakeOn(WakeOnState {
                seat: q.seat,
                wake_on: stored.legacy_wake_on(),
                changed: false,
            })),
            Command::SetWakeOn(set) => {
                let new = NudgeThreshold::from_wake_on(set.value);
                let changed = *stored != new;
                *stored = new;
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
    paths: InstancePaths,
    instance: uuid::Uuid,
    selection: CooperativeSelection,
    client: Client,
}
impl Fixture {
    fn new(stored: i64, modern: bool) -> Self {
        let root = std::env::temp_dir().join(format!("cli-nudge-thr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            root.join("state"),
            root.join("host.sock"),
            None,
        )
        .unwrap();
        let fixture = Self {
            paths: InstancePaths::resolve(&runtime).unwrap(),
            root,
            instance: uuid::Uuid::from_u128(1),
            selection: CooperativeSelection {
                seat: SeatId::new("seat"),
                target: HostTargetId::new("pane"),
                harness: crate::harness::context::Harness::Codex,
                role: Role::TopLevel,
            },
            client: Client {
                calls: Default::default(),
                threshold: std::sync::Mutex::new(NudgeThreshold::new(stored).unwrap()),
                modern,
                wake_on: true,
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
        let result = crate::cli::run_selected(
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
    let fixture = Fixture::new(0, true);
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
    let fixture = Fixture::new(0, true);
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
    let fixture = Fixture::new(2, true);
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
    let fixture = Fixture::new(0, true);
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
    let mut fixture = Fixture::new(0, true);
    fixture.selection.role = Role::Subagent;
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "2"]);
    assert!(result.is_err());
    assert!(out.is_empty());
    assert!(fixture.commands().is_empty());
}

// ---- running against an older daemon ----

#[test]
fn an_old_daemon_maps_zero_and_minus_one_to_wake_on() {
    let fixture = Fixture::new(0, false);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "-1"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: -1 (changed)\n");
    let sent = fixture.mutating_or_reading();
    let [Command::SetWakeOn(set)] = sent.as_slice() else {
        panic!("expected one SetWakeOn, got {sent:?}");
    };
    assert_eq!(set.value, WakeOn::All);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "0"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 0 (changed)\n");
}

#[test]
fn an_old_daemon_reads_through_wake_on() {
    let fixture = Fixture::new(-1, false);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: -1\n");
    assert_eq!(
        fixture.commands(),
        vec![Command::WakeOn(WakeOnQuery {
            seat: SeatId::new("seat")
        })]
    );
}

#[test]
fn an_old_daemon_refuses_other_values_with_an_upgrade_hint_and_sends_nothing() {
    let fixture = Fixture::new(0, false);
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold", "3"]);
    let error = result.unwrap_err();
    assert_eq!(error.exit_code(), crate::cli::exit::EXIT_UNSUPPORTED);
    assert!(
        format!("{error:?}").contains("supports nudge-threshold -1 or 0 only"),
        "{error:?}"
    );
    assert!(out.is_empty());
    assert!(fixture.commands().is_empty());
}

#[test]
fn a_daemon_with_neither_capability_is_refused_before_any_call() {
    let mut fixture = Fixture::new(0, false);
    fixture.client.wake_on = false;
    let (result, out) = fixture.run(&["herdr-threads", "nudge-threshold"]);
    assert_eq!(
        result.unwrap_err().exit_code(),
        crate::cli::exit::EXIT_UNSUPPORTED
    );
    assert!(out.is_empty());
    assert!(fixture.commands().is_empty());
}

// ---- the wake-on alias ----

#[test]
fn wake_on_aliases_to_the_threshold_on_a_capable_daemon() {
    let fixture = Fixture::new(0, true);
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
