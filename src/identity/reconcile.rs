//! Seat reconciliation decisions from ordered host observations.

use crate::ports::SnapshotGenerationId;
use crate::ports::{
    DurableWorkAdmission, GuardedInvalidationTransition, GuardedSeatTransition, HostCallContext,
    HostInvalidationFence, HostInvalidationReason, HostObservationAdmission, HostPort,
    HostSnapshot, PublishedSnapshot, ReconciliationAction, ReconciliationOutcome,
    RecoveryDisposition, SeatState, SnapshotHeader, SnapshotSavedSeat, SnapshotSeatPage,
    SnapshotTargetMatch, UnresolvedReason,
};
use crate::ports::{
    HostObservation, InvalidationSeatPage, SnapshotCleanupProgress, SnapshotStage,
    SnapshotStageProgress,
};
use crate::protocol::{
    ids::{HostBootId, HostTargetId},
    results::{ApiError, ErrorCode},
    time::{CallBudget, MonoInstant},
};

pub mod observation_store {
    use super::*;
    /// Narrow persistence surface for ordered capture and bounded reconciliation.
    pub trait ObservationStore: Send + Sync {
        fn clock(&self) -> &dyn crate::protocol::time::Clock;
        fn begin_host_observation(
            &self,
            instance: &str,
            budget: &CallBudget,
        ) -> Result<HostObservationAdmission, ApiError>;
        fn invalidate_host_observation(
            &self,
            admission: &HostObservationAdmission,
            reason: HostInvalidationReason,
            budget: &CallBudget,
        ) -> Result<Option<HostInvalidationFence>, ApiError>;
        fn mark_unresolved_from_invalidation(
            &self,
            transition: GuardedInvalidationTransition,
            budget: &CallBudget,
        ) -> Result<ReconciliationOutcome, ApiError>;
        fn saved_seats_page_for_invalidation(
            &self,
            fence: &HostInvalidationFence,
            after_ordinal: u64,
            high_water_ordinal: Option<u64>,
            limit: u8,
            budget: &CallBudget,
        ) -> Result<InvalidationSeatPage, ApiError>;
        fn begin_snapshot_stage(
            &self,
            header: SnapshotHeader,
            budget: &CallBudget,
        ) -> Result<SnapshotStage, ApiError>;
        fn stage_snapshot_targets(
            &self,
            stage: &SnapshotGenerationId,
            offset: u64,
            targets: &[HostObservation],
            admission: DurableWorkAdmission,
            budget: &CallBudget,
        ) -> Result<SnapshotStageProgress, ApiError>;
        fn seal_snapshot_stage(
            &self,
            stage: &SnapshotGenerationId,
            budget: &CallBudget,
        ) -> Result<SnapshotStage, ApiError>;
        fn publish_snapshot_stage(
            &self,
            stage: &SnapshotGenerationId,
            budget: &CallBudget,
        ) -> Result<PublishedSnapshot, ApiError>;
        fn discard_snapshot_stage(
            &self,
            stage: &SnapshotGenerationId,
            admission: DurableWorkAdmission,
            budget: &CallBudget,
        ) -> Result<SnapshotCleanupProgress, ApiError>;
        fn saved_seats_page(
            &self,
            published: &SnapshotGenerationId,
            after_ordinal: u64,
            high_water_ordinal: Option<u64>,
            limit: u8,
            budget: &CallBudget,
        ) -> Result<SnapshotSeatPage, ApiError>;
        fn apply_reconciliation_transition(
            &self,
            transition: GuardedSeatTransition,
            budget: &CallBudget,
        ) -> Result<ReconciliationOutcome, ApiError>;
        fn record_reconciliation_pass(
            &self,
            _published: &PublishedSnapshot,
            _budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            Ok(false)
        }
    }
}

/// Saved seats arrive in bounded pages from the durable store. This policy
/// never enumerates a namespace or allocates a seat on snapshot observation.
pub const MAX_SAVED_SEATS_PER_PLAN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryTargetDecision {
    pub target: HostTargetId,
    pub disposition: RecoveryDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPage {
    pub targets: Vec<RecoveryTargetDecision>,
    pub next_offset: Option<usize>,
}

/// Structural reconfirmation (root decision, wave-2 fix1 (b); seat-identity
/// "the same live terminal ID within verified unchanged host context can
/// bridge daemon reconnect"). The fresh coherent publication shows the same
/// terminal ID, in the same host boot and verified incarnation, as the
/// evidence stored on the seat's latest binding, and the observed target
/// itself carries qualified incarnation evidence. Occupancy and execution
/// are deliberately not consulted: the production Herdr adapter reports both
/// Unknown. A missing, partial or different binding evidence never bridges.
fn same_terminal_binding_bridge(
    saved: &SnapshotSavedSeat,
    observed: &SnapshotTargetMatch,
    terminal: &crate::protocol::ids::TerminalId,
    publication: &PublishedSnapshot,
) -> bool {
    let Some(bound) = saved.latest_binding_evidence.as_ref() else {
        return false;
    };
    &bound.terminal == terminal
        && bound.host_boot == publication.boot
        && bound.incarnation == publication.incarnation
        && observed.terminal.as_ref() == Some(terminal)
        && observed.connection_epoch.is_some_and(|epoch| epoch > 0)
        && observed.incarnation_source.is_some()
        && observed.structural_generation > 0
        && observed.observation_sequence > 0
}

/// Structural reconfirmation of a seat that was resolved but never
/// registered (root decision, wave-2 fix2 (b); seat-identity: resolve "may
/// allocate an empty shell pane before native launch", and "the same live
/// terminal ID within verified unchanged host context can bridge daemon
/// reconnect"). Such a seat has no occupant binding at all, so no binding
/// evidence can exist; its own verified structural proof (written by the
/// allocation from a verified observation) is the only identity evidence and
/// is applied under the same rule: the fresh coherent publication shows the
/// proof's terminal ID, in the proof's host boot and verified incarnation,
/// and the observed target carries qualified incarnation evidence. A seat
/// with any binding row (live or ended, with or without evidence) never uses
/// this bridge. Occupancy and execution are not consulted.
fn never_registered_structural_bridge(
    saved: &SnapshotSavedSeat,
    observed: &SnapshotTargetMatch,
    terminal: &crate::protocol::ids::TerminalId,
    publication: &PublishedSnapshot,
) -> bool {
    // `binding_execution` is the latest binding row's (NOT NULL) execution:
    // None exactly when the seat never had an occupant binding.
    if saved.binding_execution.is_some()
        || saved.active_binding_execution.is_some()
        || saved.latest_binding_evidence.is_some()
    {
        return false;
    }
    let Some(proof) = saved.structural_proof.as_ref() else {
        return false;
    };
    proof.terminal() == terminal
        && saved.target.as_ref() == Some(proof.target())
        && proof.target_generation() > 0
        && proof.connection_epoch() > 0
        && proof.host_boot() == &publication.boot
        && proof.incarnation() == publication.incarnation
        && proof.host_epoch() <= publication.epoch
        && (proof.host_epoch() < publication.epoch
            || proof.observation_sequence() <= publication.observation_sequence)
        && observed.terminal.as_ref() == Some(terminal)
        && observed.connection_epoch.is_some_and(|epoch| epoch > 0)
        && observed.incarnation_source.is_some()
        && observed.structural_generation > 0
        && observed.observation_sequence > 0
}

/// Decide from the store's indexed matches for one fenced saved-seat page.
/// The size of the published namespace does not affect this pass.
pub fn plan_page(page: &SnapshotSeatPage) -> Result<Vec<GuardedSeatTransition>, ApiError> {
    if page.seats.len() > MAX_SAVED_SEATS_PER_PLAN
        || usize::from(page.visited) > MAX_SAVED_SEATS_PER_PLAN
        || page.seats.len() > usize::from(page.visited)
    {
        return Err(stale("saved-seat page exceeds bounded read admission"));
    }
    let mut actions = Vec::new();
    for saved in &page.seats {
        if saved.state == SeatState::Retired {
            continue;
        }
        let recoverable = saved.state == SeatState::Unresolved
            && saved.unresolved_reason == Some(UnresolvedReason::HostInvalidation);
        let structural_proof = saved.structural_proof.as_ref().filter(|proof| {
            saved.target.as_ref() == Some(proof.target())
                && saved.terminal.as_ref() == Some(proof.terminal())
                && proof.target_generation() > 0
                && proof.connection_epoch() > 0
        });
        // A seat allocated after this publication cannot be decided from that
        // older image. The next ordered publication will assess it.
        let publication_before_proof = saved.state == SeatState::Resolved
            && structural_proof.is_some_and(|proof| {
                proof.host_boot() == &page.publication.boot
                    && (proof.host_epoch() > page.publication.epoch
                        || proof.host_epoch() == page.publication.epoch
                            && proof.observation_sequence() > page.publication.observation_sequence)
            });
        let same_incarnation = if recoverable {
            saved.bound_boot.as_ref() == Some(&page.publication.boot)
                && saved.bound_incarnation.as_deref() == Some(page.publication.incarnation.as_str())
        } else {
            structural_proof.is_some_and(|proof| {
                proof.host_boot() == &page.publication.boot
                    && proof.incarnation() == page.publication.incarnation
                    && proof.host_epoch() <= page.publication.epoch
            })
        };
        let action = if saved.state == SeatState::Unresolved && !recoverable {
            Some(ReconciliationAction::MarkUnresolved)
        } else if publication_before_proof {
            None
        } else if !same_incarnation {
            Some(ReconciliationAction::MarkUnresolved)
        } else if let Some(observed) = saved.observed_match.as_ref() {
            if observed.observation_sequence == 0
                || observed.observation_sequence > page.publication.observation_sequence
                || observed.structural_generation == 0
                || (saved.terminal.is_some() && saved.terminal != observed.terminal)
            {
                return Err(stale("indexed seat match conflicts with publication"));
            }
            let Some(terminal) = observed.terminal.as_ref() else {
                actions.push(GuardedSeatTransition {
                    publication: page.publication.clone(),
                    seat: saved.seat.clone(),
                    expected_binding_generation: saved.binding_generation,
                    expected_target: saved.target.clone(),
                    expected_terminal: saved.terminal.clone(),
                    action: ReconciliationAction::MarkUnresolved,
                });
                continue;
            };
            if recoverable {
                if saved.terminal.as_ref() != Some(terminal) {
                    Some(ReconciliationAction::MarkUnresolved)
                } else if same_terminal_binding_bridge(saved, observed, terminal, &page.publication)
                    || never_registered_structural_bridge(
                        saved,
                        observed,
                        terminal,
                        &page.publication,
                    )
                {
                    Some(ReconciliationAction::ReconfirmStructure {
                        target: observed.target.clone(),
                        terminal: terminal.clone(),
                    })
                } else {
                    Some(ReconciliationAction::MarkUnresolved)
                }
            } else if saved.target.as_ref() != Some(&observed.target) {
                Some(ReconciliationAction::Move {
                    target: observed.target.clone(),
                    terminal: terminal.clone(),
                })
            } else if saved.state == SeatState::Resolved
                && saved.terminal.as_ref() == Some(terminal)
                && saved.active_binding_execution.is_some()
                && saved
                    .bound_epoch
                    .is_some_and(|epoch| epoch < page.publication.epoch)
                && observed.connection_epoch.is_some_and(|epoch| epoch > 0)
                && observed.incarnation_source.is_some()
            {
                // C4: same terminal, boot and incarnation in a newer host
                // epoch; the open registered binding follows the seat.
                // `bound_epoch` is Some only for a binding whose provenance is
                // in `authority::CARRIED_BINDING_PROVENANCES`, the single
                // definition shared with the applier.
                Some(ReconciliationAction::CarryForward {
                    target: observed.target.clone(),
                    terminal: terminal.clone(),
                })
            } else {
                None
            }
        } else if let Some(target) = saved.target.as_ref() {
            Some(ReconciliationAction::BeginRetirement {
                absent_target: target.clone(),
            })
        } else {
            Some(ReconciliationAction::MarkUnresolved)
        };
        if let Some(action) = action {
            actions.push(GuardedSeatTransition {
                publication: page.publication.clone(),
                seat: saved.seat.clone(),
                expected_binding_generation: saved.binding_generation,
                expected_target: saved.target.clone(),
                expected_terminal: saved.terminal.clone(),
                action,
            });
        }
    }
    Ok(actions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcilePageProgress {
    pub next_after_ordinal: Option<u64>,
    pub high_water_ordinal: u64,
    pub transition_count: u8,
    pub retirements_started: u8,
    /// Guarded per-seat transitions the store refused on this page. They are
    /// skipped, never allowed to abort the other seats' reconciliation.
    pub transitions_refused: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationOutcome {
    Published(PublishedSnapshot),
    Invalidated {
        reason: HostInvalidationReason,
        fence: HostInvalidationFence,
        /// Host or publication error, when there was one. Invalidation is a
        /// fail-closed outcome, not a successful coherent publication.
        cause: Option<ApiError>,
    },
    /// A newer durable host decision already superseded this attempt.
    Superseded,
    /// The capture failed with the same reason as the last durable
    /// invalidation, whose unresolved-marking pass completed, and nothing was
    /// published since: no invalidation was written and no continuation is
    /// armed. The admission (fence) commit still happened (pacer spec D4).
    InvalidationRepeated {
        reason: HostInvalidationReason,
        cause: Option<ApiError>,
    },
    /// The capture failed for unavailability
    /// ([`HostInvalidationReason::is_unavailability`]): nothing was written
    /// and no continuation is armed; seats and bindings stay as they were
    /// until a capture succeeds.
    Frozen {
        reason: HostInvalidationReason,
        cause: Option<ApiError>,
    },
}

/// Apply at most one saved-seat page. Each transition is a separate guarded
/// store decision; retirement returns after its constant-size fence.
pub fn reconcile_published_page(
    store: &(impl observation_store::ObservationStore + ?Sized),
    published: &PublishedSnapshot,
    after_ordinal: u64,
    high_water_ordinal: Option<u64>,
    budget: &CallBudget,
) -> Result<ReconcilePageProgress, ApiError> {
    let page = store.saved_seats_page(
        &published.id,
        after_ordinal,
        high_water_ordinal,
        MAX_SAVED_SEATS_PER_PLAN as u8,
        budget,
    )?;
    if page.publication != *published
        || page.after_ordinal < after_ordinal
        || page.high_water_ordinal < page.after_ordinal
        || (page.has_more && page.after_ordinal == after_ordinal)
    {
        return Err(stale(
            "saved-seat page changed publication or failed to advance",
        ));
    }
    let mut transitions = 0u8;
    let mut retirements = 0u8;
    let mut refused = 0u8;
    for transition in plan_page(&page)? {
        let result = store.apply_reconciliation_transition(transition, budget)?;
        match result {
            // A refused transition is that seat's alone (D2): its guarded
            // decision changed nothing, so it is recorded and skipped, and
            // the rest of the page still reconciles. The next ordered
            // publication reassesses the refused seat from fresh evidence.
            ReconciliationOutcome::Stale => {
                refused = refused.saturating_add(1);
                continue;
            }
            ReconciliationOutcome::RetirementStarted(_) => {
                retirements += 1;
            }
            ReconciliationOutcome::Applied | ReconciliationOutcome::Unchanged => {}
        }
        transitions += 1;
    }
    Ok(ReconcilePageProgress {
        next_after_ordinal: page.has_more.then_some(page.after_ordinal),
        high_water_ordinal: page.high_water_ordinal,
        transition_count: transitions,
        retirements_started: retirements,
        transitions_refused: refused,
    })
}

/// Project one saved-seat page under the exact durable fail-closed fence.
/// A denied capture has no absence authority and can only mark continuity
/// unresolved. The caller schedules further pages outside this writer turn.
pub fn reconcile_invalidated_page(
    store: &(impl observation_store::ObservationStore + ?Sized),
    fence: &HostInvalidationFence,
    after_ordinal: u64,
    high_water_ordinal: Option<u64>,
    budget: &CallBudget,
) -> Result<ReconcilePageProgress, ApiError> {
    let page = store.saved_seats_page_for_invalidation(
        fence,
        after_ordinal,
        high_water_ordinal,
        MAX_SAVED_SEATS_PER_PLAN as u8,
        budget,
    )?;
    if page.fence != *fence
        || page.seats.len() > MAX_SAVED_SEATS_PER_PLAN
        || usize::from(page.visited) > MAX_SAVED_SEATS_PER_PLAN
        || page.seats.len() > usize::from(page.visited)
        || page.after_ordinal < after_ordinal
        || page.high_water_ordinal < page.after_ordinal
        || (page.has_more && page.after_ordinal == after_ordinal)
        || page
            .seats
            .iter()
            .any(|saved| saved.observed_match.is_some())
    {
        return Err(stale("invalidation seat page is not a bounded fenced read"));
    }
    let mut transitions = 0u8;
    for saved in &page.seats {
        // A retired seat has nothing to fail closed, and an unresolved one
        // already is (its reason, `other` or an earlier invalidation, is kept).
        // Marking it again is refused as Stale, which used to abort the pass
        // at the first such seat: the marker was never recorded, so every
        // retry wrote a new invalidation, and seats past it were never marked.
        if matches!(saved.state, SeatState::Retired | SeatState::Unresolved) {
            continue;
        }
        let result = store.mark_unresolved_from_invalidation(
            GuardedInvalidationTransition {
                fence: fence.clone(),
                seat: saved.seat.clone(),
                expected_binding_generation: saved.binding_generation,
                expected_target: saved.target.clone(),
                expected_terminal: saved.terminal.clone(),
            },
            budget,
        )?;
        if result == ReconciliationOutcome::Stale {
            return Err(ApiError::cursor_stale(
                "host invalidation or saved seat changed",
            ));
        }
        if matches!(result, ReconciliationOutcome::RetirementStarted(_)) {
            return Err(stale("host invalidation cannot retire a seat"));
        }
        transitions += 1;
    }
    Ok(ReconcilePageProgress {
        next_after_ordinal: page.has_more.then_some(page.after_ordinal),
        high_water_ordinal: page.high_water_ordinal,
        transition_count: transitions,
        retirements_started: 0,
        transitions_refused: 0,
    })
}

/// A single host capture stays in the ordered lane through hidden staging and
/// atomic publication. The store releases its writer after every bounded step.
pub fn observe_and_publish(
    host: &dyn HostPort,
    store: &(impl observation_store::ObservationStore + ?Sized),
    lane: &mut ObservationLane,
    instance: &str,
    context: &HostCallContext,
    maintenance_budget: &CallBudget,
) -> Result<ObservationOutcome, ApiError> {
    let ticket = lane.begin_observation()?;
    let admission = match store.begin_host_observation(instance, &context.budget) {
        Ok(admission) => admission,
        Err(error) => {
            lane.mark_unavailable();
            let _ = lane.discard(ticket);
            return Err(error);
        }
    };
    let capture = host.enumerate_targets(context);
    let reason = match &capture {
        Err(_) => Some(HostInvalidationReason::HostUnavailable),
        Ok(_) if context.budget.is_exhausted(store.clock()) => {
            Some(HostInvalidationReason::HostUnavailable)
        }
        Ok(snapshot)
            if !snapshot.complete
                || matches!(
                    snapshot.enumeration,
                    crate::ports::EnumerationEvidence::Partial
                ) =>
        {
            Some(HostInvalidationReason::PartialEnumeration)
        }
        Ok(snapshot)
            if matches!(
                snapshot.incarnation,
                crate::ports::IncarnationEvidence::Unknown
            ) =>
        {
            Some(HostInvalidationReason::UnknownIncarnation)
        }
        Ok(snapshot)
            if !snapshot.authorizes_absence_closure() || lane.check_snapshot(snapshot).is_err() =>
        {
            Some(HostInvalidationReason::CoherenceLost)
        }
        Ok(_) => None,
    };
    if let Some(reason) = reason {
        let cause = capture.err();
        lane.mark_unavailable();
        let _ = lane.discard(ticket);
        return invalidate_or_skip(store, lane, &admission, reason, cause, maintenance_budget);
    }
    let snapshot = capture.expect("reason classified host failure");
    let admission_for_failure = admission.clone();
    let result = publish_captured(store, lane, ticket, admission, &snapshot, &context.budget);
    match result {
        Ok(published) => Ok(ObservationOutcome::Published(published)),
        Err(error) => {
            lane.mark_unavailable();
            let _ = lane.discard(ticket);
            invalidate_or_skip(
                store,
                lane,
                &admission_for_failure,
                HostInvalidationReason::PublicationFailed,
                Some(error),
                maintenance_budget,
            )
        }
    }
}

/// Writes the durable invalidation for a failed capture, unless the lane
/// already holds the same reason's completed marking pass with no publication
/// since (pacer spec D4): then the write is skipped and the outcome is
/// `InvalidationRepeated`. The admission commit has already happened.
fn invalidate_or_skip(
    store: &(impl observation_store::ObservationStore + ?Sized),
    lane: &mut ObservationLane,
    admission: &HostObservationAdmission,
    reason: HostInvalidationReason,
    cause: Option<ApiError>,
    maintenance_budget: &CallBudget,
) -> Result<ObservationOutcome, ApiError> {
    if reason.is_unavailability() {
        return Ok(ObservationOutcome::Frozen { reason, cause });
    }
    if lane.skips_repeated_invalidation(reason) {
        return Ok(ObservationOutcome::InvalidationRepeated { reason, cause });
    }
    // A new durable invalidation supersedes the previous marking pass; the
    // marker returns only when this one's last page commits.
    lane.last_invalidation_reason = None;
    store
        .invalidate_host_observation(admission, reason, maintenance_budget)
        .map(|accepted| match accepted {
            Some(fence) => ObservationOutcome::Invalidated {
                reason,
                fence,
                cause,
            },
            None => ObservationOutcome::Superseded,
        })
}

fn publish_captured(
    store: &(impl observation_store::ObservationStore + ?Sized),
    lane: &mut ObservationLane,
    ticket: ObservationTicket,
    admission: HostObservationAdmission,
    snapshot: &HostSnapshot,
    budget: &CallBudget,
) -> Result<PublishedSnapshot, ApiError> {
    ensure_budget(store, budget)?;
    lane.check_snapshot(snapshot)?;
    let header = SnapshotHeader::from_captured(admission, snapshot).map_err(stale)?;
    let stage = store.begin_snapshot_stage(header.clone(), budget)?;
    let admission =
        DurableWorkAdmission::new(MAX_SAVED_SEATS_PER_PLAN as u8).expect("fixed bounded admission");
    let mut did_publish = false;
    let published = (|| {
        let mut offset = 0u64;
        for chunk in snapshot.targets.chunks(MAX_SAVED_SEATS_PER_PLAN) {
            // A writer turn may stage a prefix of the chunk (its quantum ran
            // out); stage the rest in the following turns.
            let mut targets = chunk;
            while !targets.is_empty() {
                ensure_budget(store, budget)?;
                let progress =
                    store.stage_snapshot_targets(&stage.id, offset, targets, admission, budget)?;
                let visited = usize::from(progress.visited);
                offset += visited as u64;
                if progress.stage.id != stage.id
                    || progress.stage.staged_targets != offset
                    || visited == 0
                    || visited > targets.len()
                {
                    return Err(stale("snapshot staging did not advance exactly"));
                }
                targets = &targets[visited..];
            }
        }
        ensure_budget(store, budget)?;
        let sealed = store.seal_snapshot_stage(&stage.id, budget)?;
        if sealed.id != stage.id
            || !sealed.sealed
            || sealed.staged_targets != header.expected_targets
        {
            return Err(stale("snapshot seal is incomplete"));
        }
        ensure_budget(store, budget)?;
        let published = store.publish_snapshot_stage(&stage.id, budget)?;
        did_publish = true;
        if published.id != stage.id
            || published.instance != header.instance
            || published.boot != header.boot
            || published.epoch != header.epoch
            || published.observation_sequence != header.observation_sequence
            || published.incarnation != header.incarnation
            || published.target_count != header.expected_targets
        {
            return Err(stale("published snapshot differs from captured authority"));
        }
        lane.accept_applied(snapshot, ticket, store.clock().monotonic_now())?;
        Ok(published)
    })();
    if published.is_err() && !did_publish {
        // One bounded cleanup turn; the durable cleanup worker resumes any
        // remaining hidden rows independently of this failed host call.
        let _ = store.discard_snapshot_stage(&stage.id, admission, budget);
    }
    published
}

fn ensure_budget(
    store: &(impl observation_store::ObservationStore + ?Sized),
    budget: &CallBudget,
) -> Result<(), ApiError> {
    if !budget.is_exhausted(store.clock()) {
        return Ok(());
    }
    Err(ApiError::new(
        if budget.cancellation.is_cancelled() {
            ErrorCode::Cancelled
        } else {
            ErrorCode::DeadlineExceeded
        },
        "host observation publication budget exhausted",
    ))
}

/// Classify one bounded page of a captured coherent recovery baseline. The
/// store must stage pages invisibly and publish the whole baseline atomically.
pub fn plan_recovery_baseline_page(
    snapshot: &HostSnapshot,
    offset: usize,
    unresolved_saved_seats: bool,
    mut is_owned: impl FnMut(&HostTargetId) -> bool,
) -> Result<RecoveryPage, ApiError> {
    if !snapshot.authorizes_absence_closure() || offset > snapshot.targets.len() {
        return Err(stale(
            "recovery baseline lacks coherent complete enumeration",
        ));
    }
    let end = offset
        .saturating_add(MAX_SAVED_SEATS_PER_PLAN)
        .min(snapshot.targets.len());
    let targets = snapshot.targets[offset..end]
        .iter()
        .map(|observed| RecoveryTargetDecision {
            target: observed.target.clone(),
            disposition: if is_owned(&observed.target) {
                RecoveryDisposition::AlreadyOwned
            } else if unresolved_saved_seats {
                RecoveryDisposition::HeldForRepair
            } else {
                RecoveryDisposition::UnambiguousUnclaimed
            },
        })
        .collect();
    Ok(RecoveryPage {
        targets,
        next_offset: (end < snapshot.targets.len()).then_some(end),
    })
}

/// The single observation lane records only applied snapshots. Host calls and
/// persistence happen outside this state; event payloads only set `dirty`.
#[derive(Debug, Default)]
pub struct ObservationLane {
    boot: Option<HostBootId>,
    epoch: u64,
    sequence: u64,
    last_snapshot_at: Option<MonoInstant>,
    dirty: bool,
    /// Set by an explicit target capture; the next lane capture runs even
    /// inside a backoff wait; cleared when an observation begins.
    explicit_capture: bool,
    /// Set by a failed or discarded attempt: the retry waits for the lane
    /// Pacer's backoff instead of re-running on the next poll. Host-event
    /// hints still set `dirty`.
    retry_after_failure: bool,
    available: bool,
    next_ticket: u64,
    in_flight: Option<u64>,
    /// Reason of the last durable invalidation whose unresolved-marking pass
    /// completed; cleared by any page error, by a new invalidation and by a
    /// publication. Empty after a restart (pacer spec D4).
    last_invalidation_reason: Option<HostInvalidationReason>,
    /// Failures answered without a durable invalidation since the last
    /// publication.
    repeat_failures: u32,
    /// Lane captures that ran to an outcome; an explicit target read that
    /// needs a capture waits for this to advance.
    completed_captures: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservationTicket(u64);

impl ObservationLane {
    /// Local availability fails closed as soon as a capture is known unusable,
    /// even while its durable invalidation write is still pending.
    pub fn mark_unavailable(&mut self) {
        self.available = false;
        self.retry_after_failure = true;
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    pub fn mark_event_dirty(&mut self) {
        self.dirty = true;
        self.available = false;
    }

    /// An explicit target capture asks for one lane capture that ignores the
    /// failure backoff gate. Not evidence: availability is unchanged.
    pub fn request_explicit_capture(&mut self) {
        self.explicit_capture = true;
    }

    pub fn note_capture_completed(&mut self) {
        self.completed_captures = self.completed_captures.wrapping_add(1);
    }

    pub fn completed_captures(&self) -> u64 {
        self.completed_captures
    }

    pub fn snapshot_due(&self, now: MonoInstant) -> bool {
        self.snapshot_due_gated(now, None)
    }

    /// `retry_at` is the lane Pacer's `next_retry_at()`. A host-event hint is
    /// always due; a retry after failure waits for the backoff; otherwise the
    /// 5 s cadence applies.
    pub fn snapshot_due_gated(&self, now: MonoInstant, retry_at: Option<MonoInstant>) -> bool {
        if self.dirty || self.explicit_capture {
            return true;
        }
        if self.retry_after_failure {
            return retry_at.is_none_or(|at| now.0 >= at.0);
        }
        self.last_snapshot_at
            .is_none_or(|last| now.0.saturating_sub(last.0) >= 5_000)
    }

    /// The skip predicate (pacer spec D4): same reason as the last completed
    /// marking pass, and no publication since (a publication clears it).
    /// Counts the skipped failure.
    pub fn skips_repeated_invalidation(&mut self, reason: HostInvalidationReason) -> bool {
        if self.last_invalidation_reason != Some(reason) {
            return false;
        }
        self.repeat_failures = self.repeat_failures.saturating_add(1);
        true
    }

    /// Feeds one page result of the continuation armed by an `Invalidated`
    /// outcome. Only the last page's commit marks the pass complete; any page
    /// error clears the marker so the next failure re-runs from ordinal 0.
    pub fn note_invalidation_page(
        &mut self,
        reason: HostInvalidationReason,
        page: &Result<ReconcilePageProgress, ApiError>,
    ) {
        match page {
            Ok(progress) if progress.next_after_ordinal.is_none() => {
                self.last_invalidation_reason = Some(reason);
            }
            Ok(_) => {}
            Err(_) => self.last_invalidation_reason = None,
        }
    }

    pub fn last_invalidation_reason(&self) -> Option<HostInvalidationReason> {
        self.last_invalidation_reason
    }

    pub fn repeat_failures(&self) -> u32 {
        self.repeat_failures
    }

    /// Clear only hints known before dispatch. A later event remains dirty
    /// after this observation applies and schedules another host read.
    pub fn begin_observation(&mut self) -> Result<ObservationTicket, ApiError> {
        if self.in_flight.is_some() {
            return Err(stale("another host observation is still in flight"));
        }
        self.next_ticket = self
            .next_ticket
            .checked_add(1)
            .ok_or_else(|| stale("observation ticket exhausted"))?;
        self.in_flight = Some(self.next_ticket);
        self.dirty = false;
        self.explicit_capture = false;
        self.retry_after_failure = false;
        Ok(ObservationTicket(self.next_ticket))
    }

    pub fn discard(&mut self, ticket: ObservationTicket) -> Result<(), ApiError> {
        if self.in_flight != Some(ticket.0) {
            return Err(stale("late host observation ticket"));
        }
        self.in_flight = None;
        self.retry_after_failure = true;
        Ok(())
    }

    pub fn check_snapshot(&self, snapshot: &HostSnapshot) -> Result<(), ApiError> {
        if !snapshot.has_coherent_order() || snapshot.epoch > i64::MAX as u64 {
            return Err(stale("snapshot lacks trusted bounded observation order"));
        }
        if self.boot.as_ref() == Some(&snapshot.boot)
            && (snapshot.epoch < self.epoch
                || (snapshot.epoch == self.epoch && snapshot.observation_sequence <= self.sequence))
        {
            return Err(stale("snapshot is not newer than applied host observation"));
        }
        Ok(())
    }

    /// Call only after the store accepts this snapshot's bounded transition.
    pub fn accept_applied(
        &mut self,
        snapshot: &HostSnapshot,
        ticket: ObservationTicket,
        now: MonoInstant,
    ) -> Result<(), ApiError> {
        if self.in_flight != Some(ticket.0) {
            return Err(stale("late host observation ticket"));
        }
        self.check_snapshot(snapshot)?;
        self.boot = Some(snapshot.boot.clone());
        self.epoch = snapshot.epoch;
        self.sequence = snapshot.observation_sequence;
        self.last_snapshot_at = Some(now);
        self.last_invalidation_reason = None;
        self.repeat_failures = 0;
        // An event received after dispatch remains an invalidation hint;
        // the older in-flight snapshot cannot restore local availability.
        self.available = !self.dirty;
        self.in_flight = None;
        Ok(())
    }
}

fn stale(detail: &str) -> ApiError {
    ApiError::stale_host_observation(detail)
}

#[cfg(test)]
#[path = "../../tests/identity/reconcile.rs"]
mod tests;
