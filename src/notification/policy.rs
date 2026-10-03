//! Pure selection and elapsed spacing for one seat's durable attention work.
//!
//! The store supplies effective, bounded reason flags and retained reservation
//! fields. This module neither queries history nor treats UTC projections as a
//! wake clock. The runtime anchors each durable reservation after commit.

use crate::ports::{HostObservation, HostUiState, ObservationProvenance, WakeCandidate};
use crate::protocol::{
    ids::{HostBootId, HostTargetId},
    time::MonoInstant,
};

pub const MARKER: &str = "herdr-threads: attention pending; run herdr-threads inbox";
const STEPS_MS: [u64; 4] = [30_000, 60_000, 120_000, 300_000];

/// Selection uses the store's effective logical candidate, including manifest
/// work before projection. The store supplies actionability and the current
/// occupant's offer frontier; this policy neither queries nor mutates either.
#[derive(Debug, Clone, Copy)]
pub struct AttentionSnapshot<'a> {
    candidate: &'a WakeCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedAttention {
    pub invites: bool,
    pub ordinary: bool,
    pub warnings: bool,
}

impl<'a> AttentionSnapshot<'a> {
    pub fn from_candidate(candidate: &'a WakeCandidate) -> Self {
        Self { candidate }
    }

    pub fn select(self) -> Option<SelectedAttention> {
        let candidate = self.candidate;
        if candidate.effectively_retired {
            return None;
        }
        let selected = SelectedAttention {
            invites: candidate.has_pending_invitation,
            ordinary: candidate.has_pending_receipt,
            warnings: candidate.actionable_warning_seq.is_some()
                && !candidate.warning_offered_for_current_occupant(),
        };
        (selected.invites || selected.ordinary || selected.warnings).then_some(selected)
    }
}

/// The final target read must match the reservation before HostPort derives a
/// SafeWakeTarget. Registration is deliberately absent: a safe idle native
/// occupant may need this generic hint to recover its first check-in.
#[derive(Debug, Clone, Copy)]
pub struct TargetState<'a> {
    pub observation: &'a HostObservation,
    pub resolved: bool,
    pub held: bool,
    pub available: bool,
    /// Supplied by the source-aware identity coordinator, not inferred from
    /// a merely fresh host response carrying cached native metadata.
    pub execution_recognized: bool,
    pub expected_target: &'a HostTargetId,
    pub expected_boot: &'a HostBootId,
    pub expected_epoch: u64,
    pub expected_generation: u64,
}

impl TargetState<'_> {
    pub fn may_hint(self) -> bool {
        self.resolved
            && !self.held
            && self.available
            && self.execution_recognized
            && self.observation.target == *self.expected_target
            && self.observation.host_boot == *self.expected_boot
            && self.observation.epoch == self.expected_epoch
            && self.observation.generation == self.expected_generation
            && self.observation.provenance == ObservationProvenance::FreshCurrentTarget
            && self.observation.ui == HostUiState::Idle
            && self
                .observation
                .occupant
                .as_ref()
                .is_some_and(|occupant| occupant.is_top_level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryError {
    InvalidMinimum,
    InvalidHistory,
    ClockOverflow,
    InFlight,
    TooEarly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryConfig {
    minimum_delay_ms: u64,
}

impl RetryConfig {
    pub fn new(minimum_delay_ms: u64) -> Result<Self, RetryError> {
        if minimum_delay_ms < STEPS_MS[0] {
            return Err(RetryError::InvalidMinimum);
        }
        Ok(Self { minimum_delay_ms })
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            minimum_delay_ms: STEPS_MS[0],
        }
    }
}

/// Mirrors wake_work retry_step, minimum_delay_ms, effective_delay_ms and
/// last_reservation_id presence. It remains persisted after reasons clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableRetry {
    pub retry_step: u8,
    pub minimum_delay_ms: u64,
    pub effective_delay_ms: u64,
    pub ever_reserved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryGuard {
    config: RetryConfig,
    durable: DurableRetry,
    anchor: Option<MonoInstant>,
    shortened: bool,
    in_flight: bool,
}

impl RetryGuard {
    pub fn never_reserved(config: RetryConfig) -> Self {
        Self {
            config,
            durable: DurableRetry {
                retry_step: 0,
                minimum_delay_ms: 0,
                effective_delay_ms: 0,
                ever_reserved: false,
            },
            anchor: None,
            shortened: false,
            in_flight: false,
        }
    }

    /// Every previous reservation, including a former in-flight attempt or a
    /// seat with empty reasons, receives a full boot guard. UTC is not an input.
    pub fn from_durable(
        config: RetryConfig,
        durable: DurableRetry,
        boot_mono: MonoInstant,
    ) -> Result<Self, RetryError> {
        if !durable.ever_reserved {
            return Ok(Self::never_reserved(config));
        }
        if durable.retry_step > 3
            || durable.minimum_delay_ms < STEPS_MS[0]
            || durable.effective_delay_ms < durable.minimum_delay_ms
            || durable.effective_delay_ms < STEPS_MS[durable.retry_step as usize]
        {
            return Err(RetryError::InvalidHistory);
        }
        let guard = Self {
            config,
            durable,
            anchor: Some(boot_mono),
            shortened: false,
            in_flight: false,
        };
        guard.earliest()?;
        Ok(guard)
    }

    pub fn durable(self) -> DurableRetry {
        self.durable
    }

    pub fn earliest(self) -> Result<Option<MonoInstant>, RetryError> {
        let Some(anchor) = self.anchor else {
            return Ok(None);
        };
        let delay = if self.shortened {
            self.durable
                .minimum_delay_ms
                .max(self.config.minimum_delay_ms)
        } else {
            self.durable
                .effective_delay_ms
                .max(self.config.minimum_delay_ms)
        };
        anchor
            .0
            .checked_add(delay)
            .map(MonoInstant)
            .map(Some)
            .ok_or(RetryError::ClockOverflow)
    }

    pub fn eligible(self, now: MonoInstant) -> bool {
        !self.in_flight
            && self
                .earliest()
                .is_ok_and(|earliest| earliest.is_none_or(|at| now.0 >= at.0))
    }

    /// Called only after reservation commit, with a fresh monotonic sample.
    pub fn reserve(
        mut self,
        committed_at: MonoInstant,
    ) -> Result<(Self, DurableRetry), RetryError> {
        if self.in_flight {
            return Err(RetryError::InFlight);
        }
        if !self.eligible(committed_at) {
            return Err(RetryError::TooEarly);
        }
        let step = if self.durable.ever_reserved {
            self.durable.retry_step.saturating_add(1).min(3)
        } else {
            0
        };
        let effective = self.config.minimum_delay_ms.max(STEPS_MS[step as usize]);
        committed_at
            .0
            .checked_add(effective)
            .ok_or(RetryError::ClockOverflow)?;
        self.durable = DurableRetry {
            retry_step: step,
            minimum_delay_ms: self.config.minimum_delay_ms,
            effective_delay_ms: effective,
            ever_reserved: true,
        };
        self.anchor = Some(committed_at);
        self.shortened = false;
        self.in_flight = true;
        Ok((self, self.durable))
    }

    /// Completion, timeout, cancellation and target discard all move the anchor
    /// forward. New attention received in flight retains its shorter delay from
    /// this advanced anchor. Calling this on a non-current attempt is a runtime
    /// fence error.
    pub fn complete(mut self, completed_at: MonoInstant) -> Result<Self, RetryError> {
        if !self.in_flight {
            return Err(RetryError::InvalidHistory);
        }
        let anchor = self.anchor.ok_or(RetryError::InvalidHistory)?;
        self.anchor = Some(MonoInstant(anchor.0.max(completed_at.0)));
        self.in_flight = false;
        self.earliest()?;
        Ok(self)
    }

    /// Back to the pre-reservation guard after a pre-send refusal: same
    /// durable ladder row and anchor, not in flight. A shortening applied by
    /// new attention while the attempt was in flight is kept.
    pub fn restored_to(self, prior: Self) -> Self {
        Self {
            shortened: prior.shortened || self.shortened,
            in_flight: false,
            ..prior
        }
    }

    pub fn new_attention(mut self) -> Result<Self, RetryError> {
        self.shortened = true;
        self.earliest()?;
        Ok(self)
    }
}

#[cfg(test)]
#[path = "../../tests/scheduler/policy.rs"]
mod tests;
