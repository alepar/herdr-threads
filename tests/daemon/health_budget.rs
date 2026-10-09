//! ht-p03.27: the Health line budget (16 on the wire, 12 assembled, 4 of
//! headroom) and the lane folding rule. Each test names the mutation it kills.

use super::*;
use crate::daemon::remedy::{RemedyContext, remedy};
use crate::ports::{
    BindingEvidenceStartup, UNRESOLVED_SEAT_SAMPLE, UnresolvedReason, UnresolvedSeatSample,
    UnresolvedSeatSummary,
};
use crate::protocol::ids::{HostTargetId, SeatId};
use crate::protocol::results::ErrorClass;
use std::path::PathBuf;

const LOG: &str = "/state/instances/i/logs/daemon.log";

fn lane(name: &'static str) -> LaneDegradation {
    LaneDegradation {
        lane: name,
        summary: format!("lane {name} failed: StoreBusy"),
        class: Some(ErrorClass::Transient),
    }
}

fn pointer(class: Option<ErrorClass>) -> String {
    format!(
        "degraded: {}",
        remedy(class, &RemedyContext::LaneDegraded { log: LOG.into() })
    )
}

pub(super) fn ready_inputs() -> HealthInputs {
    let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    inputs.database = ComponentStatus::Ready;
    inputs.schema = ComponentStatus::Ready;
    inputs.host = ComponentStatus::Ready;
    inputs.scheduler = ComponentStatus::Ready;
    inputs.coherent_enumeration = CapabilityState::Supported;
    inputs.safe_prompt = CapabilityState::Supported;
    inputs.current_execution = CapabilityState::Unsupported;
    inputs.claude = HarnessStatus::Supported("claude 2.1.286: listed".into());
    inputs.codex = HarnessStatus::Supported("codex 0.159.3: listed".into());
    inputs.unresolved = Some(UnresolvedSeatSummary::default());
    inputs.log_path = Some(PathBuf::from(LOG));
    inputs
}

/// Every source at its worst: all components degraded, both harnesses
/// refused, two harness broken lines, no wake path, retirement pending and degraded, binding evidence
/// lacking, unresolved seats with a full sample, a refusal count, and every
/// lane degraded with a retry suffix.
pub(super) fn worst_case_inputs() -> HealthInputs {
    let mut inputs = ready_inputs();
    let long = "x".repeat(400);
    inputs.database = ComponentStatus::Degraded(long.clone());
    inputs.schema = ComponentStatus::Degraded(long.clone());
    inputs.host = ComponentStatus::Unavailable(long.clone());
    inputs.scheduler = ComponentStatus::Degraded(long.clone());
    inputs.current_execution = CapabilityState::Unsupported;
    inputs.safe_prompt = CapabilityState::Unsupported;
    inputs.claude = HarnessStatus::Refused(long.clone());
    inputs.codex = HarnessStatus::Refused(long.clone());
    inputs.retirement = RetirementHealth {
        pending: true,
        degraded: true,
    };
    inputs.binding_evidence = Some(BindingEvidenceStartup {
        backfilled: u64::MAX,
        still_lacking: u64::MAX,
    });
    inputs.unresolved = Some(UnresolvedSeatSummary {
        count: u64::MAX,
        sample: (0..UNRESOLVED_SEAT_SAMPLE)
            .map(|n| UnresolvedSeatSample {
                seat: SeatId::new(format!("seat-{n}")),
                target: Some(HostTargetId::new(format!("w{n}:p{n}"))),
                reason: Some(UnresolvedReason::HostInvalidation),
            })
            .collect(),
    });
    inputs.transitions_refused = u64::MAX;
    // One broken line per harness, at the longest a verdict line can be.
    inputs.harness_version_lines = ["claude", "codex"]
        .into_iter()
        .map(|harness| format!("harness {harness} 9.9.9 broken: {long}"))
        .collect();
    inputs.degraded_lanes = [
        "deadline",
        "wake",
        "observation",
        "retention",
        "admission-observer",
    ]
    .into_iter()
    .map(|name| LaneDegradation {
        lane: name,
        summary: format!("lane {name} failed: StoreBusy; retrying (attempt 9, next ≤ 30s)"),
        class: Some(ErrorClass::Transient),
    })
    .collect();
    inputs
}

/// Kills: a source added without budget (the previous cap left one line of
/// headroom and no test), a fold that still renders one line per lane, and a
/// budget that drops the notes' or limitations' tail silently.
#[test]
fn worst_case_health_fits_twelve_lines() {
    // The version verdict lines are limitations too: they sit ahead of the
    // unresolved-seat samples and survive the fold (asserted below).
    let health = worst_case_inputs().assemble();
    assert!(
        health.limitations.len() <= HEALTH_LINE_BUDGET,
        "{} limitation lines: {:#?}",
        health.limitations.len(),
        health.limitations
    );
    assert!(
        health.notes.len() <= HEALTH_LINE_BUDGET,
        "{} note lines: {:#?}",
        health.notes.len(),
        health.notes
    );
    assert_eq!(HEALTH_LINE_BUDGET, 16 - 4);
    health.validate().expect("the wire cap still holds");
    // The budget cut the lowest-priority lines, not the leading ones.
    assert!(health.limitations[0].starts_with("database degraded: "));
    for harness in ["claude", "codex"] {
        assert!(
            health
                .limitations
                .iter()
                .any(|line| line.starts_with(&format!("harness {harness} 9.9.9 broken: "))),
            "{:#?}",
            health.limitations
        );
    }
    // The unresolved-seat samples go first; then the tail folds into one line
    // counting what it hides (with the two version lines the summary line
    // itself no longer fits).
    assert!(
        health
            .limitations
            .iter()
            .all(|line| !line.starts_with("unresolved seat ")),
        "{:#?}",
        health.limitations
    );
    assert!(
        health.limitations.last().is_some_and(|line| line
            .ends_with("more lines not shown; run `herdr-threads doctor` and see the daemon log")),
        "{:#?}",
        health.limitations
    );
}

/// Kills: more than two lanes still rendering a line each, a fold that drops
/// the lane names or the log path, and a fold that also keeps the per-lane
/// scheduler line.
#[test]
fn more_than_two_degraded_lanes_fold_into_one_summary() {
    let mut inputs = ready_inputs();
    inputs.scheduler = ComponentStatus::Degraded("lane deadline failed: StoreBusy".into());
    inputs.degraded_lanes = vec![lane("deadline"), lane("wake"), lane("retention")];
    let health = inputs.assemble();
    assert_eq!(
        health.limitations,
        vec![format!(
            "scheduler degraded: 3 lanes degraded (deadline, wake, retention): see {LOG}"
        )]
    );
    assert_eq!(health.state, HealthState::Degraded);
}

/// Kills: folding at two (the rule is "more than two"), losing a lane's own
/// summary, and omitting the log pointer.
#[test]
fn two_degraded_lanes_keep_their_own_lines() {
    let mut inputs = ready_inputs();
    inputs.scheduler = ComponentStatus::Degraded("lane deadline failed: StoreBusy".into());
    inputs.degraded_lanes = vec![lane("deadline"), lane("wake")];
    let health = inputs.assemble();
    assert_eq!(
        health.limitations,
        vec![
            "scheduler degraded: lane deadline failed: StoreBusy".to_owned(),
            "scheduler degraded: lane wake failed: StoreBusy".to_owned(),
            pointer(Some(ErrorClass::Transient)),
        ]
    );
}

/// Kills: a degraded lane that reads without the daemon log pointer, a
/// pointer rendered while every lane is fine, and a pointer when the
/// provider has no log path (it must not invent one).
#[test]
fn degraded_text_names_the_daemon_log() {
    let mut inputs = ready_inputs();
    inputs.scheduler = ComponentStatus::Degraded("lane wake failed: StoreBusy".into());
    inputs.degraded_lanes = vec![lane("wake")];
    let degraded = inputs.assemble();
    assert!(
        degraded
            .limitations
            .contains(&pointer(Some(ErrorClass::Transient))),
        "{:?}",
        degraded.limitations
    );

    let healthy = ready_inputs().assemble();
    assert!(healthy.limitations.is_empty(), "{:?}", healthy.limitations);
    assert_eq!(healthy.state, HealthState::Healthy);

    let mut unbound = ready_inputs();
    unbound.log_path = None;
    unbound.scheduler = ComponentStatus::Degraded("lane wake failed: StoreBusy".into());
    unbound.degraded_lanes = vec![lane("wake")];
    let unbound = unbound.assemble();
    assert_eq!(
        unbound.limitations,
        vec!["scheduler degraded: lane wake failed: StoreBusy".to_owned()]
    );
}

/// Kills: `transitions_refused` staying a counter nobody reads.
#[test]
fn transitions_refused_reaches_health_as_a_note() {
    let mut inputs = ready_inputs();
    inputs.transitions_refused = 2;
    let health = inputs.assemble();
    assert_eq!(
        health.state,
        HealthState::Healthy,
        "a note, not a degradation"
    );
    assert!(
        health
            .notes
            .iter()
            .any(|note| note.contains("refused 2 seat transition(s)")),
        "{:?}",
        health.notes
    );
    assert!(
        !ready_inputs()
            .assemble()
            .notes
            .iter()
            .any(|note| note.contains("refused")),
        "no refusals, no note"
    );
}

/// Kills: a pointer class taken from the first lane only, one that ranks
/// Unavailable over Corrupt, and a pointer that ignores the lanes' classes.
#[test]
fn pointer_class_prefers_corrupt_then_unavailable() {
    let classed = |name: &'static str, class| LaneDegradation {
        class: Some(class),
        ..lane(name)
    };
    let last_line = |lanes: Vec<LaneDegradation>| {
        let mut inputs = ready_inputs();
        inputs.scheduler = ComponentStatus::Degraded("lane failed".into());
        inputs.degraded_lanes = lanes;
        inputs.assemble().limitations.last().cloned()
    };
    let transient_unavailable = vec![
        classed("deadline", ErrorClass::Transient),
        classed("wake", ErrorClass::Unavailable),
    ];
    let unavailable = pointer(Some(ErrorClass::Unavailable));
    assert_ne!(unavailable, pointer(Some(ErrorClass::Transient)));
    assert_eq!(last_line(transient_unavailable), Some(unavailable.clone()));

    // Two lanes only: more than two fold into one line with no pointer.
    let with_corrupt = vec![
        classed("wake", ErrorClass::Unavailable),
        classed("retention", ErrorClass::Corrupt),
    ];
    let corrupt = pointer(Some(ErrorClass::Corrupt));
    assert_ne!(corrupt, unavailable);
    assert_eq!(last_line(with_corrupt), Some(corrupt));
}

/// An untested newer Herdr is one limitation line next to the version; it is
/// a warning, not a degradation. Kills: dropping the line, or degrading on it.
#[test]
fn untested_herdr_release_warns_without_degrading() {
    let release = crate::host::compatibility::admit(Some("0.10.0"), Some(23)).unwrap();
    let mut inputs = ready_inputs();
    inputs.host_version = Some(release.summary());
    inputs.host_release_warning = release.warning();
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy);
    assert_eq!(health.host.version.as_deref(), Some("0.10.0 (protocol 23)"));
    let warnings: Vec<_> = health
        .limitations
        .iter()
        .filter(|line| line.starts_with("untested Herdr"))
        .collect();
    assert_eq!(warnings, ["untested Herdr 0.10.0; tested 0.9.1-0.9.3"]);

    let tested = crate::host::compatibility::admit(Some("0.9.2"), Some(22)).unwrap();
    let mut inputs = ready_inputs();
    inputs.host_release_warning = tested.warning();
    let health = inputs.assemble();
    assert!(!health.limitations.iter().any(|line| line.contains("Herdr")));
}
