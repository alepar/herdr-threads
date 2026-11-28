use super::*;
use crate::cli::commands::{CooperativeSelection, parse_argv, parse_argv_exact_seats};
use crate::protocol::{
    commands::Command,
    ids::*,
    results::{
        ApiError, CheckInContextDisposition, CheckInResult, ContinuityStatus, ErrorCode,
        MappingStatus, SeatConfigEntry, SeatConfigView, SeatConfigWrite, SeatInspection,
        SeatSummary, WakeBlock,
    },
    time::UtcMillis,
};
use std::collections::BTreeMap;

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

/// A daemon double: seat config values by (seat, key), the capabilities it advertises, and an
/// optional refusal for every config command.
struct Client {
    calls: std::sync::Mutex<Vec<Command>>,
    values: std::sync::Mutex<BTreeMap<(String, String), String>>,
    config_capability: bool,
    wake: std::sync::Mutex<Option<WakeBlock>>,
    refusal: std::sync::Mutex<Option<ApiError>>,
    generation: std::sync::atomic::AtomicU64,
}

const DEFAULTS: [(&str, &str); 2] = [("nudge_threshold", "0"), ("programmatic_wake", "off")];

impl Client {
    fn entry(&self, seat: &str, key: &str) -> SeatConfigEntry {
        let default = DEFAULTS.iter().find(|(k, _)| *k == key).unwrap().1;
        let stored = self
            .values
            .lock()
            .unwrap()
            .get(&(seat.to_owned(), key.to_owned()))
            .cloned();
        SeatConfigEntry {
            key: key.to_owned(),
            is_default: stored.as_deref().is_none_or(|v| v == default),
            value: stored.unwrap_or_else(|| default.to_owned()),
            tier: crate::protocol::seat_config::key(key).unwrap().tier,
            writer: None,
            updated_at: None,
        }
    }
    fn write(&self, seat: &str, key: &str, value: Option<&str>) -> SeatConfigWrite {
        let before = self.entry(seat, key);
        let mut values = self.values.lock().unwrap();
        match value {
            Some(value) => values.insert((seat.to_owned(), key.to_owned()), value.to_owned()),
            None => values.remove(&(seat.to_owned(), key.to_owned())),
        };
        drop(values);
        let after = self.entry(seat, key);
        SeatConfigWrite {
            seat: SeatId::new(seat),
            key: key.to_owned(),
            value: after.value.clone(),
            is_default: after.is_default,
            changed: before.value != after.value,
        }
    }
}

impl LocalClient for Client {
    fn supports_capability(&self, name: &str, _: &crate::protocol::time::CallBudget) -> bool {
        match name {
            n if n == crate::protocol::capabilities::SEAT_CONFIG_V1 => self.config_capability,
            n if n == crate::protocol::capabilities::WAKE_STATUS_V1 => {
                self.wake.lock().unwrap().is_some()
            }
            _ => false,
        }
    }
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
        if matches!(
            command,
            Command::GetSeatConfig(_) | Command::SetSeatConfig(_) | Command::UnsetSeatConfig(_)
        ) && let Some(error) = self.refusal.lock().unwrap().clone()
        {
            return Err(error);
        }
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
                continuity_key: None,
                retirement_state: None,
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
            Command::GetSeatConfig(q) => Ok(CommandResult::SeatConfig(SeatConfigView {
                entries: match &q.key {
                    Some(key) => vec![self.entry(q.seat.as_str(), key)],
                    None => DEFAULTS
                        .iter()
                        .map(|(key, _)| self.entry(q.seat.as_str(), key))
                        .collect(),
                },
                seat: q.seat,
            })),
            Command::SetSeatConfig(set) => {
                let seat = set
                    .seat
                    .as_ref()
                    .or(set.claim.as_ref().map(|c| &c.seat))
                    .unwrap()
                    .clone();
                Ok(CommandResult::SeatConfigWritten(self.write(
                    seat.as_str(),
                    &set.key,
                    Some(&set.value),
                )))
            }
            Command::UnsetSeatConfig(unset) => {
                let seat = unset
                    .seat
                    .as_ref()
                    .or(unset.claim.as_ref().map(|c| &c.seat))
                    .unwrap()
                    .clone();
                Ok(CommandResult::SeatConfigWritten(self.write(
                    seat.as_str(),
                    &unset.key,
                    None,
                )))
            }
            Command::NudgeThreshold(q) => {
                let entry = self.entry(q.seat.as_str(), "nudge_threshold");
                Ok(CommandResult::NudgeThreshold(
                    crate::protocol::results::NudgeThresholdState {
                        seat: q.seat,
                        threshold: crate::protocol::priority::NudgeThreshold::new(
                            entry.value.parse().unwrap(),
                        )
                        .unwrap(),
                        changed: false,
                    },
                ))
            }
            Command::SetNudgeThreshold(set) => {
                let write = self.write(
                    set.claim.seat.as_str(),
                    "nudge_threshold",
                    Some(&set.value.get().to_string()),
                );
                Ok(CommandResult::NudgeThreshold(
                    crate::protocol::results::NudgeThresholdState {
                        seat: set.claim.seat,
                        threshold: set.value,
                        changed: write.changed,
                    },
                ))
            }
            Command::WakeStatus(q) => Ok(CommandResult::WakeStatus(
                q.seats
                    .into_iter()
                    .map(|seat| crate::protocol::results::SeatWakeStatus {
                        seat,
                        harness: Some("codex".into()),
                        wake: *self.wake.lock().unwrap(),
                        first: false,
                        offline: false,
                        host: None,
                        target: None,
                    })
                    .collect(),
            )),
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
    fn new() -> Self {
        Self::with_capability(true)
    }
    fn with_capability(config_capability: bool) -> Self {
        let root = crate::test_support::scratch::temp_path(format!(
            "cli-config-seat-{}",
            uuid::Uuid::new_v4()
        ));
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
                values: Default::default(),
                config_capability,
                wake: Default::default(),
                refusal: Default::default(),
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
    /// An agent call with the selected seat (`run_selected`, as a hook-registered caller).
    fn run(&self, argv: &[&str]) -> (Result<(), RunError>, String) {
        let parsed = parse_argv_exact_seats(argv).unwrap();
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
    /// A call with no derived caller selection (a `--seat` read, a person's write, a seatless shell).
    fn run_unselected(&self, argv: &[&str]) -> (Result<(), RunError>, String) {
        let parsed = parse_argv_exact_seats(argv).unwrap();
        let mut out = Vec::new();
        let result = super::run(
            &parsed,
            None,
            &self.paths,
            self.instance,
            &crate::test_support::test_host_locator(),
            &self.client,
            &crate::app::SystemClock::new(),
            &mut out,
        );
        (result, String::from_utf8(out).unwrap())
    }
    fn sent(&self) -> Vec<Command> {
        self.client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| !matches!(c, Command::SeatInspect(_)))
            .cloned()
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn detail(result: Result<(), RunError>) -> String {
    match result {
        Err(RunError::Api(error)) => error.detail,
        other => panic!("expected an API refusal, got {other:?}"),
    }
}

// ---- list: static ----

fn list_text() -> String {
    let parsed = parse_argv(["herdr-threads", "config", "seat", "list"]).unwrap();
    let mut out = Vec::new();
    run_list(&parsed, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

// Kills: a key line missing one of its five fields, a line past the stated bound, a trailer that
// does not name the human spelling, a key left out of the list.
#[test]
fn list_shows_key_values_default_tier_and_description_for_every_key() {
    let text = list_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        crate::protocol::seat_config::REGISTRY.len() + 1
    );
    for (line, key) in lines.iter().zip(crate::protocol::seat_config::REGISTRY) {
        assert_eq!(
            *line,
            format!(
                "{} {} default={} tier={} \u{2014} {}",
                key.name,
                key.values_label(),
                key.default,
                key.tier.as_str(),
                key.description
            )
        );
        assert!(line.len() < 160, "{} bytes: {line}", line.len());
    }
    assert_eq!(
        lines[0],
        "nudge_threshold -1..9 default=0 tier=agent \u{2014} idle seat wakes for items with priority <= N; -1 wakes for every item"
    );
    assert!(lines[1].starts_with("programmatic_wake on|off default=off tier=human \u{2014} "));
    let trailer = lines[2];
    assert_eq!(trailer, LIST_TRAILER);
    assert!(trailer.len() < 110, "{} bytes", trailer.len());
    // An agent shown only this output can name the command that sets programmatic_wake.
    assert!(trailer.contains("ht human config seat set [--seat S] KEY VALUE"));
}

#[test]
fn list_json_is_an_array_of_the_five_fields() {
    let parsed = parse_argv(["herdr-threads", "config", "seat", "list", "--json"]).unwrap();
    let mut out = Vec::new();
    run_list(&parsed, &mut out).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let rows = value.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1],
        serde_json::json!({
            "key": "programmatic_wake",
            "values": "on|off",
            "default": "off",
            "tier": "human",
            "description": crate::protocol::seat_config::key("programmatic_wake").unwrap().description,
        })
    );
    assert_eq!(rows[0]["tier"], "agent");
    assert_eq!(rows[0]["values"], "-1..9");
    assert_eq!(rows[0]["default"], "0");
}

// ---- get ----

#[test]
fn get_shows_every_key_with_default_markers_for_the_caller() {
    let fixture = Fixture::new();
    let (result, out) = fixture.run(&["herdr-threads", "config", "seat", "get"]);
    result.unwrap();
    assert_eq!(
        out,
        "nudge_threshold: 0 (default)\nprogrammatic_wake: off (default)\n"
    );
    assert_eq!(
        fixture.sent(),
        vec![Command::GetSeatConfig(GetSeatConfig {
            seat: SeatId::new("seat"),
            key: None
        })]
    );
}

#[test]
fn get_one_key_of_any_seat_with_an_explicit_value() {
    let fixture = Fixture::new();
    fixture
        .client
        .write("other", "programmatic_wake", Some("on"));
    let (result, out) = fixture.run_unselected(&[
        "herdr-threads",
        "config",
        "seat",
        "get",
        "programmatic_wake",
        "--seat",
        "other",
    ]);
    result.unwrap();
    assert_eq!(out, "programmatic_wake: on\n");
}

#[test]
fn a_subagent_may_read() {
    let mut fixture = Fixture::new();
    fixture.selection.role = Role::Subagent;
    let (result, out) = fixture.run(&["herdr-threads", "config", "seat", "get", "nudge_threshold"]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 0 (default)\n");
}

#[test]
fn get_json_encodes_the_daemon_result() {
    let fixture = Fixture::new();
    let (result, out) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "get",
        "nudge_threshold",
        "--json",
    ]);
    result.unwrap();
    let value: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(value["result"]["kind"], "seat_config");
    assert_eq!(
        value["result"]["data"]["entries"][0]["key"],
        "nudge_threshold"
    );
    assert_eq!(value["result"]["data"]["entries"][0]["is_default"], true);
}

// Kills: a seatless `get` that guesses a seat or reaches the daemon.
#[test]
fn a_seatless_get_without_seat_prints_the_stated_line() {
    let fixture = Fixture::new();
    let (result, out) = fixture.run_unselected(&["herdr-threads", "config", "seat", "get"]);
    assert_eq!(detail(result), GET_NEEDS_SEAT);
    assert_eq!(GET_NEEDS_SEAT.len(), 62);
    assert!(out.is_empty());
    assert!(fixture.sent().is_empty());
}

// ---- agent set / unset ----

#[test]
fn an_agent_set_sends_the_agent_tier_with_a_claim_and_no_seat() {
    let fixture = Fixture::new();
    let (result, out) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "set",
        "nudge_threshold",
        "3",
    ]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 3 (changed)\n");
    let sent = fixture.sent();
    let [Command::SetSeatConfig(set)] = sent.as_slice() else {
        panic!("expected one SetSeatConfig, got {sent:?}");
    };
    assert_eq!(set.tier, ConfigTier::Agent);
    assert_eq!(set.seat, None);
    assert_eq!(
        (set.key.as_str(), set.value.as_str()),
        ("nudge_threshold", "3")
    );
    assert_eq!(set.claim.as_ref().unwrap().seat.as_str(), "seat");
    let (result, out) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "set",
        "nudge_threshold",
        "3",
    ]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 3 (unchanged)\n");
    // A negative value is passed through for the daemon's strict -1..9.
    let (result, out) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "set",
        "nudge_threshold",
        "-1",
    ]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: -1 (changed)\n");
}

#[test]
fn an_agent_unset_sends_unset_with_a_claim() {
    let fixture = Fixture::new();
    fixture.client.write("seat", "nudge_threshold", Some("4"));
    let (result, out) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "unset",
        "nudge_threshold",
    ]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 0 (changed)\n");
    let sent = fixture.sent();
    let [Command::UnsetSeatConfig(unset)] = sent.as_slice() else {
        panic!("expected one UnsetSeatConfig, got {sent:?}");
    };
    assert_eq!(unset.tier, ConfigTier::Agent);
    assert_eq!(unset.seat, None);
    assert!(unset.claim.is_some());
}

// Kills: M12 skipped or reworded on the generic path (it must be the nudge-threshold bytes).
#[test]
fn set_nudge_threshold_on_a_no_wake_seat_prints_m12_exactly_as_nudge_threshold_does() {
    let fixture = Fixture::new();
    *fixture.client.wake.lock().unwrap() = Some(WakeBlock::NoWake);
    let (result, generic) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "set",
        "nudge_threshold",
        "3",
    ]);
    result.unwrap();
    let fixture2 = Fixture::new();
    *fixture2.client.wake.lock().unwrap() = Some(WakeBlock::NoWake);
    let (result, alias) = fixture2.run(&["herdr-threads", "nudge-threshold", "3"]);
    result.unwrap();
    assert_eq!(generic, alias);
    assert!(
        generic.ends_with("seat has no idle wake; threshold has no effect\n"),
        "{generic}"
    );
    // A wakeable seat prints no M12; programmatic_wake never does.
    *fixture.client.wake.lock().unwrap() = Some(WakeBlock::Focused);
    let (result, out) = fixture.run(&[
        "herdr-threads",
        "config",
        "seat",
        "set",
        "nudge_threshold",
        "4",
    ]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 4 (changed)\n");
}

#[test]
fn a_subagent_cannot_write_and_sends_nothing() {
    let mut fixture = Fixture::new();
    fixture.selection.role = Role::Subagent;
    for argv in [
        [
            "herdr-threads",
            "config",
            "seat",
            "set",
            "nudge_threshold",
            "2",
        ]
        .as_slice(),
        [
            "herdr-threads",
            "config",
            "seat",
            "unset",
            "nudge_threshold",
        ]
        .as_slice(),
    ] {
        let (result, out) = fixture.run(argv);
        assert_eq!(detail(result), SUBAGENT_REFUSAL);
        assert!(out.is_empty());
    }
    assert!(fixture.client.calls.lock().unwrap().is_empty());
    assert_eq!(SUBAGENT_REFUSAL.len(), 63);
}

// ---- human tier ----

#[test]
fn a_human_set_with_seat_sends_the_human_tier_without_a_claim() {
    let fixture = Fixture::new();
    let (result, out) = fixture.run_unselected(&[
        "herdr-threads",
        "human",
        "config",
        "seat",
        "set",
        "--seat",
        "target",
        "programmatic_wake",
        "on",
    ]);
    result.unwrap();
    assert_eq!(out, "programmatic_wake: on (changed)\n");
    let sent = fixture.sent();
    let [Command::SetSeatConfig(set)] = sent.as_slice() else {
        panic!("expected one SetSeatConfig, got {sent:?}");
    };
    assert_eq!(set.tier, ConfigTier::Human);
    assert_eq!(set.seat, Some(SeatId::new("target")));
    assert_eq!(set.claim, None);
    let (result, out) = fixture.run_unselected(&[
        "herdr-threads",
        "human",
        "config",
        "seat",
        "unset",
        "--seat",
        "target",
        "programmatic_wake",
    ]);
    result.unwrap();
    assert_eq!(out, "programmatic_wake: off (changed)\n");
    let sent = fixture.sent();
    let Some(Command::UnsetSeatConfig(unset)) = sent.last() else {
        panic!("expected UnsetSeatConfig, got {sent:?}");
    };
    assert_eq!(
        (unset.tier, unset.claim.is_none()),
        (ConfigTier::Human, true)
    );
    // The person may also set another seat's agent-tier key.
    let (result, out) = fixture.run_unselected(&[
        "herdr-threads",
        "human",
        "config",
        "seat",
        "set",
        "--seat",
        "target",
        "nudge_threshold",
        "5",
    ]);
    result.unwrap();
    assert_eq!(out, "nudge_threshold: 5 (changed)\n");
}

// The person's write from a pane defaults to that pane's seat, filled by the caller derivation.
#[test]
fn a_human_set_defaults_to_the_located_caller_seat() {
    let mut parsed = parse_argv([
        "herdr-threads",
        "human",
        "config",
        "seat",
        "set",
        "programmatic_wake",
        "on",
    ])
    .unwrap();
    crate::cli::default_seat_for_test(&mut parsed.action, SeatId::new("mine"));
    let fixture = Fixture::new();
    let mut out = Vec::new();
    super::run(
        &parsed,
        None,
        &fixture.paths,
        fixture.instance,
        &crate::test_support::test_host_locator(),
        &fixture.client,
        &crate::app::SystemClock::new(),
        &mut out,
    )
    .unwrap();
    let sent = fixture.sent();
    let [Command::SetSeatConfig(set)] = sent.as_slice() else {
        panic!("{sent:?}");
    };
    assert_eq!(set.seat, Some(SeatId::new("mine")));
}

// A seatless shell: `get --seat` and `human ... --seat` work; omitting --seat is the stated line.
#[test]
fn a_seatless_human_write_without_seat_prints_the_stated_line() {
    let fixture = Fixture::new();
    let (result, out) = fixture.run_unselected(&[
        "herdr-threads",
        "human",
        "config",
        "seat",
        "set",
        "programmatic_wake",
        "on",
    ]);
    assert_eq!(detail(result), HUMAN_NEEDS_SEAT);
    assert_eq!(HUMAN_NEEDS_SEAT.len(), 64);
    assert!(out.is_empty());
    let (result, _) = fixture.run_unselected(&[
        "herdr-threads",
        "human",
        "config",
        "seat",
        "unset",
        "programmatic_wake",
    ]);
    assert_eq!(detail(result), HUMAN_NEEDS_SEAT);
    assert!(fixture.sent().is_empty());
}

// ---- capability gate and passthrough ----

// Kills: a request sent to a daemon that predates seat.config_v1.
#[test]
fn a_daemon_without_the_capability_gets_the_one_line_refusal_and_nothing_is_sent() {
    let fixture = Fixture::with_capability(false);
    assert_eq!(SEAT_CONFIG_REFUSAL.len(), 54);
    for argv in [
        vec!["herdr-threads", "config", "seat", "get", "--seat", "s"],
        vec![
            "herdr-threads",
            "human",
            "config",
            "seat",
            "set",
            "--seat",
            "s",
            "programmatic_wake",
            "on",
        ],
        vec![
            "herdr-threads",
            "human",
            "config",
            "seat",
            "unset",
            "--seat",
            "s",
            "programmatic_wake",
        ],
    ] {
        let (result, out) = fixture.run_unselected(&argv);
        assert_eq!(detail(result), SEAT_CONFIG_REFUSAL, "{argv:?}");
        assert!(out.is_empty());
    }
    for argv in [
        ["herdr-threads", "config", "seat", "get"].as_slice(),
        [
            "herdr-threads",
            "config",
            "seat",
            "set",
            "nudge_threshold",
            "1",
        ]
        .as_slice(),
        [
            "herdr-threads",
            "config",
            "seat",
            "unset",
            "nudge_threshold",
        ]
        .as_slice(),
    ] {
        let (result, out) = fixture.run(argv);
        assert_eq!(detail(result), SEAT_CONFIG_REFUSAL, "{argv:?}");
        assert!(out.is_empty());
    }
    assert!(fixture.client.calls.lock().unwrap().is_empty());
    // list never contacts the daemon, capability or not.
    assert!(list_text().starts_with("nudge_threshold "));
}

#[test]
fn a_daemon_refusal_passes_through_unchanged() {
    let fixture = Fixture::new();
    let refusal = ApiError::new(
        ErrorCode::InvalidRequest,
        "seat s is homed on node bx1; seat config is set there",
    );
    *fixture.client.refusal.lock().unwrap() = Some(refusal.clone());
    let (result, out) = fixture.run_unselected(&[
        "herdr-threads",
        "human",
        "config",
        "seat",
        "set",
        "--seat",
        "s",
        "nope",
        "x",
    ]);
    assert_eq!(detail(result), refusal.detail);
    assert!(out.is_empty());
}

/// The pane-seat lookup the production caller derivation sends: one resolved seat `mine` on the
/// pane `w1:p1`; every other command goes to the config double.
struct PaneClient<'a> {
    inner: &'a Client,
}
impl LocalClient for PaneClient<'_> {
    fn supports_capability(&self, name: &str, budget: &crate::protocol::time::CallBudget) -> bool {
        self.inner.supports_capability(name, budget)
    }
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
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let Command::Seats(query) = &command else {
            return self.inner.call(command, budget);
        };
        self.inner.calls.lock().unwrap().push(command.clone());
        assert_eq!(query.target, Some(HostTargetId::new("w1:p1")));
        let mut page = empty();
        page.items = vec![SeatSummary {
            seat: SeatId::new("mine"),
            continuity: ContinuityStatus::Resolved,
            target: Some(HostTargetId::new("w1:p1")),
            generation: 1,
            created_at: UtcMillis(0),
            retired_at: None,
            active: true,
            availability: None,
            bindings: Vec::new(),
            directory: None,
        }];
        Ok(CommandResult::Seats(page))
    }
}

// A person's `config seat set|unset` with no `--seat`, run from a Herdr pane through the production
// caller derivation (`run_caller_scoped`: pane-seat lookup, `SeatDefault`, then the config run),
// writes the located seat as the human tier with no agent claim.
#[test]
fn a_human_write_without_seat_in_a_pane_targets_the_pane_seat_end_to_end() {
    for (verb, tail) in [
        ("set", &["programmatic_wake", "on"][..]),
        ("unset", &["programmatic_wake"][..]),
    ] {
        let mut argv = vec!["herdr-threads", "human", "config", "seat", verb];
        argv.extend_from_slice(tail);
        let parsed = parse_argv(argv).unwrap();
        let fixture = Fixture::new();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            fixture.root.join("state"),
            fixture.root.join("host.sock"),
            None,
        )
        .unwrap();
        let clock: std::sync::Arc<dyn crate::protocol::time::Clock> =
            std::sync::Arc::new(crate::app::SystemClock::new());
        let connection = crate::cli::LazyConnection::new(|| {
            Ok((
                fixture.instance,
                PaneClient {
                    inner: &fixture.client,
                },
            ))
        });
        let mut out = Vec::new();
        let rest = crate::cli::run_caller_scoped(
            parsed,
            Some("w1:p1"),
            &runtime,
            &fixture.paths,
            &connection,
            &clock,
            &|| crate::cli::cooperative_budget(clock.as_ref()),
            &mut out,
        )
        .unwrap();
        assert!(rest.is_none(), "{verb}: the action ran");
        let sent = fixture.sent();
        let (seat, key, tier, claim) = match sent.as_slice() {
            [Command::Seats(_), Command::SetSeatConfig(set)] if verb == "set" => {
                (&set.seat, &set.key, set.tier, &set.claim)
            }
            [Command::Seats(_), Command::UnsetSeatConfig(unset)] if verb == "unset" => {
                (&unset.seat, &unset.key, unset.tier, &unset.claim)
            }
            other => panic!("{verb}: {other:?}"),
        };
        assert_eq!(seat, &Some(SeatId::new("mine")), "{verb}");
        assert_eq!(key, "programmatic_wake", "{verb}");
        assert_eq!(
            tier,
            crate::protocol::seat_config::ConfigTier::Human,
            "{verb}"
        );
        assert!(claim.is_none(), "{verb}: a person's write carries no claim");
    }
}
