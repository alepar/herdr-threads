//! The neutral witness rule and its fold (agy-adapter D4): `RoleWitness::decide` and
//! `LocalEvidence::refine`.
use crate::harness::{
    adapter::*,
    context::EventKind,
    registry::{AgentHarnessId, builtins},
};

const ALL: [Witness; 4] = [
    Witness::Present,
    Witness::Absent,
    Witness::Unchecked,
    Witness::Unreadable,
];

fn witness(marker: Witness, listing: Witness) -> RoleWitness {
    RoleWitness {
        top_level_marker: marker,
        child_listing: listing,
    }
}

fn event(role: EventRole, intent: EventIntent, delivery: DeliveryEligibility) -> DecodedEvent {
    let harness: AgentHarnessId = builtins().agent("claude").unwrap();
    DecodedEvent {
        harness,
        role,
        native_session: Some("session".into()),
        event_id: "event".into(),
        intent,
        metadata: EventMetadata {
            skill_pointer: false,
            callback_deadline: None,
            context_source: "test".into(),
            capability: crate::harness::Capability::ContractValidatedInput,
            domain: ContractDomain::Native,
            native_event: "test".into(),
            shape_fields: vec![],
        },
        delivery,
        runtime: RuntimeAttribution::Unavailable {
            diagnostic: "test".into(),
        },
    }
}

fn evidence(role: Option<RoleWitness>, lifecycle: Option<EventKind>) -> LocalEvidence {
    LocalEvidence { role, lifecycle }
}

/// Kills: a rule that accepts a marker without an examined listing (or the reverse), so that
/// an unchecked or unreadable witness raises a child to top level.
#[test]
fn decide_is_top_level_only_for_a_present_marker_and_an_absent_listing() {
    let mut top_level = 0;
    for marker in ALL {
        for listing in ALL {
            let decided = witness(marker, listing).decide();
            let expected = if (marker, listing) == (Witness::Present, Witness::Absent) {
                top_level += 1;
                EventRole::TopLevel
            } else {
                EventRole::Subagent
            };
            assert_eq!(decided, expected, "{marker:?} / {listing:?}");
        }
    }
    assert_eq!(top_level, 1);
}

/// Kills: a fold that lets a witness raise a child decoded as `Subagent`.
#[test]
fn refine_never_raises_a_subagent() {
    for marker in ALL {
        for listing in ALL {
            let mut subagent = event(
                EventRole::Subagent,
                EventIntent::Lifecycle(EventKind::Startup),
                DeliveryEligibility::Context,
            );
            evidence(Some(witness(marker, listing)), Some(EventKind::Resume)).refine(&mut subagent);
            assert_eq!(
                subagent.role,
                EventRole::Subagent,
                "{marker:?} / {listing:?}"
            );
            assert!(
                matches!(subagent.intent, EventIntent::Lifecycle(EventKind::Startup)),
                "a child's lifecycle reading is ignored"
            );
        }
    }
}

/// Kills: `Unknown` becoming top level on anything but positive witnesses, and a witness that
/// is ignored for `Unknown`.
#[test]
fn refine_resolves_unknown_only_on_positive_witnesses() {
    for marker in ALL {
        for listing in ALL {
            let mut unknown = event(
                EventRole::Unknown,
                EventIntent::Observer,
                DeliveryEligibility::ObserverOnly,
            );
            evidence(Some(witness(marker, listing)), None).refine(&mut unknown);
            let expected = if (marker, listing) == (Witness::Present, Witness::Absent) {
                EventRole::TopLevel
            } else {
                EventRole::Subagent
            };
            assert_eq!(unknown.role, expected, "{marker:?} / {listing:?}");
        }
    }
}

/// Kills: a role witness that only applies to `Unknown` (a top-level input is also re-decided,
/// failing toward child), or one that leaves the role alone with no witness.
#[test]
fn refine_redecides_a_top_level_input_and_leaves_it_alone_without_a_witness() {
    let mut demoted = event(
        EventRole::TopLevel,
        EventIntent::Observer,
        DeliveryEligibility::ObserverOnly,
    );
    evidence(Some(witness(Witness::Unchecked, Witness::Unchecked)), None).refine(&mut demoted);
    assert_eq!(demoted.role, EventRole::Subagent);
    for role in [EventRole::TopLevel, EventRole::Unknown, EventRole::Subagent] {
        let mut untouched = event(
            role,
            EventIntent::Observer,
            DeliveryEligibility::ObserverOnly,
        );
        LocalEvidence::default().refine(&mut untouched);
        assert_eq!(untouched.role, role);
        assert!(matches!(untouched.intent, EventIntent::Observer));
        assert!(matches!(
            untouched.delivery,
            DeliveryEligibility::ObserverOnly
        ));
    }
}

/// Kills: a lifecycle reading that converts the wrong intents or kinds: `Compact`, `Tool` and
/// `Restart` must be ignored, `Current` and `DeclaredReset`-like intents must stay, and the
/// delivery changes only together with the intent.
#[test]
fn lifecycle_converts_observer_and_lifecycle_intents_for_startup_clear_resume_only() {
    for kind in [EventKind::Startup, EventKind::Clear, EventKind::Resume] {
        for intent in [
            EventIntent::Observer,
            EventIntent::Lifecycle(EventKind::Compact),
        ] {
            let mut turn = event(
                EventRole::TopLevel,
                intent,
                DeliveryEligibility::ObserverOnly,
            );
            evidence(None, Some(kind)).refine(&mut turn);
            assert!(
                matches!(turn.intent, EventIntent::Lifecycle(found) if found == kind),
                "{kind:?}"
            );
            assert!(matches!(turn.delivery, DeliveryEligibility::Context));
            // Only a `resume` source can reattach a seat (C1): the converted event says so.
            let source = match kind {
                EventKind::Startup => "startup",
                EventKind::Clear => "clear",
                _ => "resume",
            };
            assert_eq!(turn.metadata.context_source, source, "{kind:?}");
        }
        let mut current = event(
            EventRole::TopLevel,
            EventIntent::Current,
            DeliveryEligibility::ObserverOnly,
        );
        evidence(None, Some(kind)).refine(&mut current);
        assert!(matches!(current.intent, EventIntent::Current));
        assert_eq!(current.metadata.context_source, "test", "{kind:?}");
        assert!(matches!(
            current.delivery,
            DeliveryEligibility::ObserverOnly
        ));
    }
    for kind in [EventKind::Compact, EventKind::Tool, EventKind::Restart] {
        let mut turn = event(
            EventRole::TopLevel,
            EventIntent::Observer,
            DeliveryEligibility::ObserverOnly,
        );
        evidence(None, Some(kind)).refine(&mut turn);
        assert!(matches!(turn.intent, EventIntent::Observer), "{kind:?}");
        assert!(matches!(turn.delivery, DeliveryEligibility::ObserverOnly));
    }
}

/// Kills: a lifecycle reading applied to a non-top-level event, and one evaluated before the
/// role fold (a witness that demotes the event must also suppress the lifecycle).
#[test]
fn lifecycle_is_ignored_unless_the_role_is_top_level_after_the_witness() {
    for role in [EventRole::Subagent, EventRole::Unknown] {
        let mut turn = event(
            role,
            EventIntent::Observer,
            DeliveryEligibility::ObserverOnly,
        );
        evidence(None, Some(EventKind::Resume)).refine(&mut turn);
        assert!(matches!(turn.intent, EventIntent::Observer), "{role:?}");
    }
    let mut demoted = event(
        EventRole::TopLevel,
        EventIntent::Observer,
        DeliveryEligibility::ObserverOnly,
    );
    evidence(
        Some(witness(Witness::Present, Witness::Present)),
        Some(EventKind::Resume),
    )
    .refine(&mut demoted);
    assert_eq!(demoted.role, EventRole::Subagent);
    assert!(matches!(demoted.intent, EventIntent::Observer));
    let mut promoted = event(
        EventRole::Unknown,
        EventIntent::Observer,
        DeliveryEligibility::ObserverOnly,
    );
    evidence(
        Some(witness(Witness::Present, Witness::Absent)),
        Some(EventKind::Resume),
    )
    .refine(&mut promoted);
    assert_eq!(promoted.role, EventRole::TopLevel);
    assert!(matches!(
        promoted.intent,
        EventIntent::Lifecycle(EventKind::Resume)
    ));
}
