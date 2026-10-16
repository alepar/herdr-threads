//! W8: right after the daemon starts, before its first reconciliation, Health
//! is `starting` (the `HEALTH_STARTING_NOTE` marker on a `degraded` wire
//! state) instead of an unexplained `degraded`; a real problem, a completed
//! reconciliation or the startup bound ends it. Each test names the mutation
//! it kills.

use super::health_budget::ready_inputs;
use super::*;
use crate::protocol::results::{HEALTH_STARTING_NOTE, HealthState};

/// What the production provider reports in the first seconds: store and
/// scheduler ready, nothing observed from the host or the harnesses yet.
fn fresh_boot_inputs(since_start_ms: u64) -> HealthInputs {
    let mut inputs = ready_inputs();
    inputs.host = ComponentStatus::Unknown;
    inputs.coherent_enumeration = CapabilityState::Unknown;
    inputs.set_harness("claude", HarnessStatus::Unknown);
    inputs.set_harness("codex", HarnessStatus::Unknown);
    inputs.last_reconciliation_at = None;
    inputs.since_start_ms = Some(since_start_ms);
    inputs
}

/// Kills: reporting a plain `degraded` (or a new wire state an older CLI
/// cannot decode) before the first reconciliation.
#[test]
fn fresh_boot_before_first_reconciliation_is_starting() {
    let health = fresh_boot_inputs(1_000).assemble();
    assert_eq!(
        health.state,
        HealthState::Degraded,
        "the wire state is unchanged"
    );
    assert!(health.starting(), "{health:?}");
    assert!(health.notes.iter().any(|note| note == HEALTH_STARTING_NOTE));
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line == FIRST_RECONCILIATION_OVERDUE_LINE)
    );
    health.validate().unwrap();
    // The key set an older CLI decodes is unchanged.
    let value = serde_json::to_value(&health).unwrap();
    assert!(value.get("starting").is_none(), "{value}");
}

/// Kills: keeping `starting` once the first reconciliation completed.
#[test]
fn first_reconciliation_ends_starting() {
    let mut inputs = ready_inputs();
    inputs.last_reconciliation_at = Some(crate::protocol::time::UtcMillis(5));
    inputs.since_start_ms = Some(1_000);
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy);
    assert!(!health.starting());
    // Still pending harness observation after a completed reconciliation:
    // the ordinary classification (degraded), not starting.
    let mut inputs = fresh_boot_inputs(1_000);
    inputs.host = ComponentStatus::Ready;
    inputs.coherent_enumeration = CapabilityState::Supported;
    inputs.last_reconciliation_at = Some(crate::protocol::time::UtcMillis(5));
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert!(!health.starting());
    assert!(!health.notes.iter().any(|note| note == HEALTH_STARTING_NOTE));
}

/// Kills: an unbounded `starting` that hides a daemon that never reconciles.
#[test]
fn startup_bound_turns_starting_into_degraded() {
    let just_inside = fresh_boot_inputs(STARTUP_GRACE_MS - 1).assemble();
    assert!(just_inside.starting());
    let health = fresh_boot_inputs(STARTUP_GRACE_MS).assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert!(!health.starting());
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line == FIRST_RECONCILIATION_OVERDUE_LINE),
        "{health:?}"
    );
    health.validate().unwrap();
}

/// Kills: masking a real problem as `starting` during the startup window.
#[test]
fn real_problem_during_startup_is_degraded() {
    type Problem = (&'static str, fn(&mut HealthInputs));
    let problems: Vec<Problem> = vec![
        ("host unavailable", |inputs| {
            inputs.host = ComponentStatus::Unavailable("socket gone".into())
        }),
        ("scheduler degraded", |inputs| {
            inputs.scheduler = ComponentStatus::Degraded("lane failed".into())
        }),
        ("database degraded", |inputs| {
            inputs.database = ComponentStatus::Degraded("busy".into())
        }),
        ("harness refused", |inputs| {
            inputs.set_harness("claude", HarnessStatus::Refused("bad version".into()))
        }),
        ("no wake path", |inputs| {
            inputs.safe_prompt = CapabilityState::Unsupported;
        }),
        ("retirement degraded", |inputs| {
            inputs.retirement.degraded = true
        }),
        // After a Herdr restart seats stay unresolved until the first
        // coherent snapshot: conservatively degraded, not starting.
        ("unresolved seats", |inputs| {
            inputs.unresolved = Some(crate::ports::UnresolvedSeatSummary {
                holds: Default::default(),
                count: 1,
                sample: Vec::new(),
            })
        }),
        // A failed lane, with the scheduler component input still ready.
        ("degraded lane", |inputs| {
            inputs.degraded_lanes = vec![LaneDegradation {
                lane: "observation",
                summary: "lane observation failed: StoreBusy".into(),
                class: None,
            }]
        }),
    ];
    for (name, apply) in problems {
        let mut inputs = fresh_boot_inputs(1_000);
        apply(&mut inputs);
        let health = inputs.assemble();
        assert_eq!(health.state, HealthState::Degraded, "{name}");
        assert!(!health.starting(), "{name}: {health:?}");
        assert!(
            !health
                .limitations
                .iter()
                .any(|line| line == FIRST_RECONCILIATION_OVERDUE_LINE),
            "{name}"
        );
    }
}

/// Kills: inventing a start time: a provider that supplies none keeps
/// today's classification and adds no line.
#[test]
fn unknown_start_time_keeps_the_ordinary_classification() {
    let mut inputs = fresh_boot_inputs(0);
    inputs.since_start_ms = None;
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert!(!health.starting());
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line == FIRST_RECONCILIATION_OVERDUE_LINE)
    );
}

/// Kills: a client that trusts a marker contradicting the rest of Health.
#[test]
fn starting_marker_needs_a_degraded_unreconciled_health() {
    let mut health = fresh_boot_inputs(1_000).assemble();
    assert!(health.starting());
    health.last_reconciliation_at = Some(crate::protocol::time::UtcMillis(1));
    assert!(!health.starting());
    let mut health = fresh_boot_inputs(1_000).assemble();
    health.state = HealthState::Unavailable;
    assert!(!health.starting());
}

/// Kills: a pending-observation matcher that relabels real issues.
#[test]
fn pending_observation_lines_are_only_the_unobserved_harness_lines() {
    let health = fresh_boot_inputs(1_000).assemble();
    let unknown: Vec<&String> = health
        .limitations
        .iter()
        .filter(|line| line.starts_with("harness "))
        .collect();
    assert!(!unknown.is_empty());
    assert!(unknown.iter().all(|line| is_pending_observation_line(line)));
    for line in [
        "harness claude unsupported: bad version",
        "host unavailable: socket gone",
        "untested Herdr 9.9.9",
        FIRST_RECONCILIATION_OVERDUE_LINE,
        "harness  unknown: executable availability has not been observed yet",
    ] {
        assert!(!is_pending_observation_line(line), "{line}");
    }
}
