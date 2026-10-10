//! The text a native wake types is chosen by the bound harness's delivery policy (R18).
use crate::{
    harness::registry::builtins,
    notification::{
        dispatch::{NativeWakeDispatcher, ReservationCheck},
        policy::{MARKER, WAKE_TEXTS, WAKE_UP},
    },
    ports::{
        EvidenceKind, ExecutionEvidence, HostCallContext, HostObservation, HostPort, HostSnapshot,
        HostUiState, IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome,
        NativeLaunchRequest, NativeOccupant, NotificationPort, ObservationProvenance,
        PromptOutcome, ReservedWakeAuthority, SafeWakeTarget, StructuralOccupancy,
        WakeAttentionWitness, WakeOutcome, WakeReservation,
    },
    protocol::{
        authority::Harness,
        ids::{
            ExecutionId, HostBootId, HostCallId, HostTargetId, NativeSessionId, SeatId, TerminalId,
            WakeAttemptId,
        },
        results::ApiError,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
};
use std::sync::Mutex;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}
struct CurrentReservation;
impl ReservationCheck for CurrentReservation {
    fn is_current(&self, _: &WakeReservation, _: &CallBudget) -> Result<bool, ApiError> {
        Ok(true)
    }
}

/// Records every typed prompt text.
struct TextRecordingHost {
    observation: HostObservation,
    /// Set on the safe target the host hands back (a recovery hint carries none of its own).
    host_bound_harness: Option<&'static str>,
    typed: Mutex<Vec<String>>,
}
impl HostPort for TextRecordingHost {
    fn observe_current_target_for_archival(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<crate::ports::ComposerObservation, ApiError> {
        Err(ApiError::unsupported("no archival observation"))
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::HostGuardedStart
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observation.clone())
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
            return None;
        };
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: observation.terminal.clone()?,
            incarnation: identity.clone(),
            basis: match &observation.occupant {
                Some(occupant) => crate::ports::WakeTargetBasis::VerifiedOccupant {
                    session: occupant.session.clone(),
                    execution: occupant.execution.clone(),
                },
                None => crate::ports::WakeTargetBasis::CooperativeAgent,
            },
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: self.host_bound_harness.map(str::to_owned),
        })
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.typed.lock().unwrap().push(text.to_owned());
        Ok(PromptOutcome::Submitted)
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<crate::ports::AgentComposerState, ApiError> {
        Ok(crate::ports::AgentComposerState::Submitted)
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        unreachable!("a submitted prompt needs no submit key")
    }
}

fn observation(verified_idle: bool) -> HostObservation {
    HostObservation {
        focused: false,
        target: HostTargetId::new("target"),
        host_boot: HostBootId::new("boot"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(0),
        observed_at_mono: MonoInstant(0),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: verified_idle.then(|| NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("session"),
            execution: ExecutionId::new("execution"),
            is_top_level: true,
        }),
        ui: if verified_idle {
            HostUiState::Idle
        } else {
            HostUiState::Unknown
        },
        terminal: Some(TerminalId::new("terminal")),
        occupancy: if verified_idle {
            StructuralOccupancy::Occupied
        } else {
            StructuralOccupancy::Unknown
        },
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: if verified_idle {
            ExecutionEvidence::Verified {
                execution: ExecutionId::new("execution"),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            }
        } else {
            ExecutionEvidence::Unknown
        },
        call_id: HostCallId::new("call"),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(0),
        completed_at_mono: MonoInstant(0),
    }
}

fn reservation(authority: ReservedWakeAuthority) -> WakeReservation {
    WakeReservation {
        attempt: WakeAttemptId::new("attempt"),
        seat: SeatId::new("seat"),
        attention_witness: WakeAttentionWitness::from_complete(
            "i".into(),
            SeatId::new("seat"),
            1,
            true,
            false,
            None,
            0,
            false,
            Default::default(),
        ),
        reasons: vec!["invitation".into()],
        retained_effective_delay_ms: 30_000,
        lease_until: MonoInstant(5_000),
        retained_minimum_delay_ms: 30_000,
        reserved_at_utc: UtcMillis(0),
        daemon_boot: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
        host_boot: HostBootId::new("boot"),
        host_epoch: 1,
        target: HostTargetId::new("target"),
        target_generation: 1,
        authority,
    }
}

fn cooperative(harness: &str) -> ReservedWakeAuthority {
    ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new("terminal"),
        incarnation: "incarnation".into(),
        binding_generation: None,
        harness: Some(harness.to_owned()),
    }
}

/// The one prompt a native wake with this authority types.
fn typed_for(
    authority: ReservedWakeAuthority,
    verified_idle: bool,
    host_bound_harness: Option<&'static str>,
) -> Vec<String> {
    let context = HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: Some(HostBootId::new("boot")),
        expected_epoch: Some(1),
    };
    let host = TextRecordingHost {
        observation: observation(verified_idle),
        host_bound_harness,
        typed: Mutex::new(vec![]),
    };
    let dispatch = NativeWakeDispatcher::new(&host, &CurrentReservation, &FixedClock);
    assert_eq!(
        dispatch
            .attempt_wake(reservation(authority), &context)
            .unwrap(),
        WakeOutcome::Submitted
    );
    host.typed.into_inner().unwrap()
}

/// Kills: a registry that returns the minimal text for every harness, or none for agy.
#[test]
fn registry_wake_text_is_the_marker_unless_the_policy_injects() {
    let registry = builtins();
    for id in ["claude", "codex", "hermes", "unknown"] {
        assert_eq!(registry.wake_text(id), MARKER, "{id}");
    }
    assert_eq!(registry.wake_text("agy"), WAKE_UP);
}

/// Kills: a dispatcher that types the marker for every harness, or the minimal text for all.
#[test]
fn native_wake_types_the_bound_harness_wake_text() {
    assert_eq!(
        typed_for(cooperative("agy"), false, None),
        vec!["herdr-threads: wake up".to_owned()]
    );
    assert_eq!(
        typed_for(cooperative("claude"), false, None),
        vec![MARKER.to_owned()]
    );
    assert_eq!(
        typed_for(cooperative("codex"), false, None),
        vec![MARKER.to_owned()]
    );
}

/// Kills: a recovery hint that follows the harness choice (the hook path failed, so the hint
/// must name `inbox`).
#[test]
fn a_recovery_hint_always_names_inbox() {
    let typed = typed_for(
        ReservedWakeAuthority::RecoveryHint {
            execution: ExecutionId::new("execution"),
        },
        true,
        // Even a target that names agy: the hook path is what failed.
        Some("agy"),
    );
    assert_eq!(typed, vec![MARKER.to_owned()]);
}

/// Kills: a multi-line wake text (the composer check reads one line) and a minimal text that
/// carries content or an instruction.
#[test]
fn wake_texts_are_one_line_and_the_minimal_one_is_content_free() {
    for text in WAKE_TEXTS {
        assert!(!text.contains('\n'), "{text:?}");
    }
    assert!(!WAKE_UP.contains("inbox"));
    assert!(!WAKE_UP.contains("run"));
}
