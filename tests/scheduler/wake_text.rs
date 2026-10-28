//! The text a native wake types is chosen by the bound harness's delivery policy (R18).
use crate::{
    harness::registry::builtins,
    notification::{
        dispatch::{NativeWakeDispatcher, ReservationCheck},
        policy::{MARKER, WAKE_TEXTS, WAKE_UP},
    },
    ports::{
        EvidenceKind, ExecutionEvidence, HookPullOutlook, HostCallContext, HostObservation,
        HostPort, HostSnapshot, HostUiState, IncarnationEvidence, NativeLaunchCapability,
        NativeLaunchOutcome, NativeLaunchRequest, NativeOccupant, NotificationPort,
        ObservationProvenance, PromptOutcome, ReservedWakeAuthority, SafeWakeTarget,
        StructuralOccupancy, WakeAttentionWitness, WakeOutcome, WakeReservation,
    },
    protocol::{
        authority::Harness,
        ids::{
            BindingId, ExecutionId, HostBootId, HostCallId, HostTargetId, NativeSessionId, SeatId,
            TerminalId, WakeAttemptId,
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
    fn wake_text_evidence_current(
        &self,
        _: &WakeReservation,
        _: &CallBudget,
    ) -> Result<bool, ApiError> {
        Ok(true)
    }
    fn is_current(&self, _: &WakeReservation, _: &CallBudget) -> Result<bool, ApiError> {
        Ok(true)
    }
}

/// Records every typed prompt text. Shared with the store's wake tests (ht-705), which drive the
/// real dispatcher against a real store.
pub(crate) struct TextRecordingHost {
    pub(crate) observation: HostObservation,
    /// Set on the safe target the host hands back (a recovery hint carries none of its own).
    pub(crate) host_bound_harness: Option<&'static str>,
    /// The pane's foreground pids the host reports.
    pub(crate) foreground: Vec<u32>,
    pub(crate) typed: Mutex<Vec<String>>,
}
impl HostPort for TextRecordingHost {
    fn live_topology(
        &self,
        _: &crate::protocol::time::CallBudget,
    ) -> Result<crate::host::observation::HostTopology, crate::protocol::results::ApiError> {
        Err(crate::protocol::results::ApiError::unsupported(
            "fixture host has no topology",
        ))
    }
    fn foreground_pids(&self, _: &HostTargetId, _: &HostCallContext) -> Result<Vec<u32>, ApiError> {
        Ok(self.foreground.clone())
    }
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

pub(crate) fn observation(verified_idle: bool) -> HostObservation {
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
        targets: Vec::new(),
    }
}

/// An outlook that says whether any seat holds ids parked for `inbox`: for every target list, or (with
/// `only`) just for that one.
struct FakeOutlook {
    parked: bool,
    only: Option<Vec<BindingId>>,
}
impl HookPullOutlook for FakeOutlook {
    fn holds_parked(&self, _: &SeatId, targets: &[BindingId], _: u64) -> bool {
        self.parked && self.only.as_ref().is_none_or(|only| only == targets)
    }
}

/// The one prompt a native wake with this authority types.
fn typed_for(
    authority: ReservedWakeAuthority,
    verified_idle: bool,
    host_bound_harness: Option<&'static str>,
) -> Vec<String> {
    typed_with(authority, verified_idle, host_bound_harness, false)
}

/// Like [`typed_for`], through a dispatcher whose outlook reports `parked`.
fn typed_with(
    authority: ReservedWakeAuthority,
    verified_idle: bool,
    host_bound_harness: Option<&'static str>,
    parked: bool,
) -> Vec<String> {
    typed_through(
        authority,
        observation(verified_idle),
        host_bound_harness,
        FakeOutlook { parked, only: None },
        Vec::new(),
    )
}

/// Like [`typed_with`], through the given outlook and with the host reporting `foreground` pids.
fn typed_through(
    authority: ReservedWakeAuthority,
    observation: HostObservation,
    host_bound_harness: Option<&'static str>,
    outlook: FakeOutlook,
    foreground: Vec<u32>,
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
        observation,
        host_bound_harness,
        foreground,
        typed: Mutex::new(vec![]),
    };
    let dispatch =
        NativeWakeDispatcher::new(&host, &CurrentReservation, &FixedClock).with_hook_pull(&outlook);
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

/// Kills: a pending-count line for a hook-served binding, or none for a plain-CLI Codex one
/// (ht-piggyback); an unregistered harness has no hooks.
#[test]
fn registry_turn_delivery_follows_hook_registration_evidence() {
    use crate::protocol::authority::{
        COOPERATIVE_ENV_SESSION_PROVENANCE, COOPERATIVE_TOP_LEVEL_PROVENANCE,
        MANAGED_LAUNCH_PROVENANCE,
    };
    let registry = builtins();
    let codex = |provenance| registry.binding_turn_delivery("codex", Some(provenance));
    assert!(codex(COOPERATIVE_TOP_LEVEL_PROVENANCE));
    assert!(!codex(COOPERATIVE_ENV_SESSION_PROVENANCE));
    assert!(!codex(MANAGED_LAUNCH_PROVENANCE));
    for id in ["claude", "hermes", "agy"] {
        assert!(
            registry.binding_turn_delivery(id, Some(COOPERATIVE_ENV_SESSION_PROVENANCE)),
            "{id}"
        );
    }
    assert!(!registry.binding_turn_delivery("unknown", None));
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

fn cooperative_in(harness: &str, binding_generation: Option<u64>) -> ReservedWakeAuthority {
    ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new("terminal"),
        incarnation: "incarnation".into(),
        binding_generation,
        harness: Some(harness.to_owned()),
        targets: Vec::new(),
    }
}

/// Kills: a content-free wake for a seat whose pending ids are parked for `inbox` (an empty turn),
/// a lookup without a generation, and a marker forced on harnesses or hints that never needed it.
#[test]
fn a_seat_holding_parked_ids_is_woken_with_the_marker() {
    let agy =
        |generation, parked| typed_with(cooperative_in("agy", generation), false, None, parked);
    assert_eq!(agy(Some(1), true), vec![MARKER.to_owned()]);
    assert_eq!(
        agy(Some(1), false),
        vec!["herdr-threads: wake up".to_owned()]
    );
    assert_eq!(
        agy(None, true),
        vec!["herdr-threads: wake up".to_owned()],
        "no generation, no lookup"
    );
    assert_eq!(
        typed_with(cooperative_in("claude", Some(1)), false, None, true),
        vec![MARKER.to_owned()]
    );
    let registered = ReservedWakeAuthority::Registered {
        binding_generation: 1,
        execution: ExecutionId::new("execution"),
    };
    assert_eq!(
        typed_with(registered.clone(), true, Some("agy"), true),
        vec![MARKER.to_owned()]
    );
    assert_eq!(
        typed_with(registered, true, Some("agy"), false),
        vec!["herdr-threads: wake up".to_owned()]
    );
    let hint = ReservedWakeAuthority::RecoveryHint {
        execution: ExecutionId::new("execution"),
    };
    assert_eq!(
        typed_with(hint, true, Some("agy"), true),
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

/// Kills: an `inbox` marker chosen from the whole seat (or the other binding) instead of the binding the composer
/// wake reaches: two bindings share one composer, and only the one whose process is in the foreground is typed to.
#[test]
fn the_inbox_marker_follows_the_composer_bindings_parking() {
    let (a, b) = (BindingId::new("binding-a"), BindingId::new("binding-b"));
    let two_targets = || ReservedWakeAuthority::Cooperative {
        terminal: TerminalId::new("terminal"),
        incarnation: "incarnation".into(),
        binding_generation: Some(1),
        harness: None,
        targets: [(&a, 111), (&b, 222)]
            .into_iter()
            .map(|(binding, pid)| crate::ports::WakeTarget {
                provenance: String::new(),
                binding: binding.clone(),
                harness: "agy".to_owned(),
                fingerprint_pid: Some(pid),
            })
            .collect(),
    };
    let typed = |parked_only: &BindingId| {
        // Several targets need an idle pane; the composer wake is cooperative, so no verified occupant.
        let mut idle = observation(false);
        idle.ui = HostUiState::Idle;
        typed_through(
            two_targets(),
            idle,
            None,
            FakeOutlook {
                parked: true,
                only: Some(vec![parked_only.clone()]),
            },
            vec![111],
        )
    };
    assert_eq!(
        typed(&a),
        vec![MARKER.to_owned()],
        "the foreground binding A holds parked ids"
    );
    assert_eq!(
        typed(&b),
        vec!["herdr-threads: wake up".to_owned()],
        "only the other binding B holds parked ids: the woken one has none"
    );
}

/// Kills: hook evidence overriding recovery or parked-item inbox fallbacks, a content-free wake for
/// a plain CLI binding (no turn-start hook to inject), and a harness whose turn-start hook injects
/// (Codex `UserPromptSubmit`, Claude `UserPromptSubmit`, Hermes `pre_llm_call`) left on the marker.
#[test]
fn hook_evidence_keeps_recovery_and_parked_inbox_hints() {
    for harness in ["codex", "claude", "hermes"] {
        let authority = |provenance: &str| ReservedWakeAuthority::Cooperative {
            terminal: TerminalId::new("terminal"),
            incarnation: "incarnation".into(),
            binding_generation: Some(1),
            harness: Some(harness.into()),
            targets: vec![crate::ports::WakeTarget {
                binding: BindingId::new("binding"),
                harness: harness.into(),
                provenance: provenance.into(),
                fingerprint_pid: Some(111),
            }],
        };
        for (provenance, parked, expected) in [
            ("cooperative_top_level", false, WAKE_UP),
            ("cooperative_top_level", true, MARKER),
            ("cooperative_env_session", false, MARKER),
        ] {
            assert_eq!(
                typed_through(
                    authority(provenance),
                    observation(false),
                    None,
                    FakeOutlook { parked, only: None },
                    vec![111]
                ),
                [expected],
                "{harness} {provenance} parked={parked}"
            );
        }
        assert_eq!(
            typed_for(
                ReservedWakeAuthority::RecoveryHint {
                    execution: ExecutionId::new("execution"),
                },
                true,
                Some(harness)
            ),
            [MARKER],
            "{harness}"
        );
    }
}
