//! The daemon half of harness version evidence (ht-xoc.4).
//!
//! A hook's [`HarnessEvidence`] note is recorded in `harness_version_evidence`
//! (or, with no version, reasons and bounded session/contract failures are advisory). An
//! unattributed `SessionStart` (a resumed Claude session, say; not a Codex
//! resume, whose rollout only names the creating CLI) is held per
//! `(harness, session id)` for up to 24 hours and credited to the session's
//! first attributed event, because the version only becomes readable once the
//! transcript has an entry. Recording never waits for the network: a first-seen
//! version or a fresh violation only asks the manifest service to fetch, and
//! that call returns at once.
//!
//! A store error never drops a held start: it stays held (or is put back)
//! until a write of it succeeds.
//!
//! The rows are advisory data about the harness. The ACL is the daemon's own
//! process; no seat or caller identity is involved.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::{
    harness::manifest::{FetchReason, ManifestService},
    ports::StorePort,
    protocol::{
        commands::{HarnessEvidence, HarnessEvidenceOutcome},
        results::ApiError,
        time::{CallBudget, Clock},
    },
    store::harness_evidence::{
        DiagnosticRecord, EventClass, EvidenceOutcome, EvidenceRecord, Recorded,
    },
};

/// A held unattributed `SessionStart` is credited only within this long.
pub const PENDING_MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;
/// At most this many held session starts; the oldest is evicted first.
pub const PENDING_MAX_ENTRIES: usize = 1024;

/// Where the recorder asks for a manifest fetch. [`ManifestService`] is the
/// production implementation; a call never blocks.
pub trait ManifestTrigger: Send + Sync {
    fn ensure_manifest(&self, harness: &str, reason: FetchReason);
}

impl ManifestTrigger for ManifestService {
    fn ensure_manifest(&self, harness: &str, reason: FetchReason) {
        ManifestService::ensure_manifest(self, harness, reason);
    }
}

/// The two store writes the recorder makes. Production passes the daemon's
/// `StorePort`; tests pass a store that fails on cue.
pub trait EvidenceWrites: Send + Sync {
    fn record_harness_evidence(
        &self,
        record: &EvidenceRecord<'_>,
        budget: &CallBudget,
    ) -> Result<Recorded, ApiError>;

    fn record_contract_diagnostic(
        &self,
        record: &DiagnosticRecord<'_>,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;

    fn record_unattributed(
        &self,
        harness: &str,
        reason: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
}

struct PortWrites(Arc<dyn StorePort>);

impl EvidenceWrites for PortWrites {
    fn record_harness_evidence(
        &self,
        record: &EvidenceRecord<'_>,
        budget: &CallBudget,
    ) -> Result<Recorded, ApiError> {
        self.0.record_harness_evidence(record, budget)
    }

    fn record_contract_diagnostic(
        &self,
        record: &DiagnosticRecord<'_>,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.0.record_contract_diagnostic(record, budget)
    }

    fn record_unattributed(
        &self,
        harness: &str,
        reason: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.0.record_unattributed(harness, reason, budget)
    }
}

struct Held {
    harness: String,
    session_id: String,
    received_ms: i64,
    contract_id: String,
    event: String,
    outcome: EvidenceOutcome,
}

/// Unattributed `SessionStart` outcomes, oldest first.
#[derive(Default)]
struct PendingSessionStarts(VecDeque<Held>, PendingV2, SuppressedSessions);

impl PendingSessionStarts {
    fn expire(&mut self, now_ms: i64) {
        self.0
            .retain(|held| now_ms.saturating_sub(held.received_ms) < PENDING_MAX_AGE_MS);
        self.1.expire(now_ms);
    }

    fn bound(&mut self) {
        while self.0.len() + self.1.0.len() > PENDING_MAX_ENTRIES {
            // Ties evict legacy first; restore and retry use this same budget.
            let legacy_first = match (self.0.front(), self.1.0.front()) {
                (Some(a), Some(b)) => a.received_ms <= b.received_ms,
                (Some(_), None) => true,
                _ => false,
            };
            if legacy_first {
                self.0.pop_front();
            } else {
                self.1.0.pop_front();
            }
        }
    }

    fn hold(&mut self, held: Held, now_ms: i64) {
        self.expire(now_ms);
        self.0
            .retain(|old| !(old.harness == held.harness && old.session_id == held.session_id));
        self.0.push_back(held);
        self.bound();
    }

    /// Puts back a held start whose write failed. A newer hold for the same
    /// session wins (as in `hold`) and an expired entry is dropped; otherwise
    /// it goes back in `received_ms` order, within the bound.
    fn restore(&mut self, held: Held, now_ms: i64) {
        self.expire(now_ms);
        if now_ms.saturating_sub(held.received_ms) >= PENDING_MAX_AGE_MS
            || self
                .0
                .iter()
                .any(|old| old.harness == held.harness && old.session_id == held.session_id)
        {
            return;
        }
        let at = self
            .0
            .iter()
            .position(|old| old.received_ms > held.received_ms)
            .unwrap_or(self.0.len());
        self.0.insert(at, held);
        self.bound();
    }

    fn take(&mut self, harness: &str, session_id: &str, now_ms: i64) -> Option<Held> {
        self.expire(now_ms);
        let at = self
            .0
            .iter()
            .position(|held| held.harness == harness && held.session_id == session_id)?;
        self.0.remove(at)
    }
}

pub struct HarnessEvidenceRecorder {
    store: Arc<dyn EvidenceWrites>,
    manifest: Option<Arc<dyn ManifestTrigger>>,
    pending: Arc<Mutex<PendingSessionStarts>>,
    clock: Arc<dyn Clock>,
}

/// The event class the daemon files an event name under.
pub fn event_class(event: &str) -> EventClass {
    match event {
        "SessionStart" => EventClass::Lifecycle,
        "PreToolUse" | "PostToolUse" => EventClass::Tool,
        _ => EventClass::Other,
    }
}

fn outcome_of(outcome: &HarnessEvidenceOutcome) -> EvidenceOutcome {
    match outcome {
        HarnessEvidenceOutcome::Ok => EvidenceOutcome::Ok,
        HarnessEvidenceOutcome::Violation { field } => EvidenceOutcome::Violation {
            field: field.clone(),
        },
        HarnessEvidenceOutcome::Malformed => EvidenceOutcome::Malformed,
    }
}

/// Only Claude's runtime-written metadata can credit a held start. Codex's
/// creator header never qualifies a current runtime, including fresh startup.
fn holds_session_start(message: &HarnessEvidence) -> bool {
    message.harness == "claude" && message.event == "SessionStart"
}

impl HarnessEvidenceRecorder {
    pub fn new(
        store: Arc<dyn StorePort>,
        manifest: Option<Arc<dyn ManifestTrigger>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::with_writes(Arc::new(PortWrites(store)), manifest, clock)
    }

    pub fn with_writes(
        writes: Arc<dyn EvidenceWrites>,
        manifest: Option<Arc<dyn ManifestTrigger>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store: writes,
            manifest,
            pending: Arc::new(Mutex::new(PendingSessionStarts::default())),
            clock,
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, PendingSessionStarts> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records one note; the result is whether the note's evidence row is
    /// verified afterwards (`false` for an unattributed note).
    pub fn record(&self, message: &HarnessEvidence, budget: &CallBudget) -> Result<bool, ApiError> {
        let now_ms = self.clock.utc_now().0;
        let outcome = outcome_of(&message.outcome);
        let Some(version) = message.version.as_deref() else {
            if holds_session_start(message)
                && let Some(session_id) = message.session_id.as_ref().filter(|id| !id.is_empty())
            {
                self.pending().hold(
                    Held {
                        harness: message.harness.clone(),
                        session_id: session_id.clone(),
                        received_ms: now_ms,
                        contract_id: message.contract_id.clone(),
                        event: message.event.clone(),
                        outcome,
                    },
                    now_ms,
                );
            }
            // Held first: a failed reason write must not lose the start.
            let mut reason = message
                .unattributed_reason
                .clone()
                .unwrap_or_else(|| "unattributed".into());
            match &message.outcome {
                HarnessEvidenceOutcome::Violation { field } => {
                    if let Some(session_id) =
                        message.session_id.as_deref().filter(|id| !id.is_empty())
                    {
                        self.store.record_contract_diagnostic(
                            &DiagnosticRecord {
                                harness: &message.harness,
                                session_id,
                                contract_id: &message.contract_id,
                                event: &message.event,
                                field,
                            },
                            budget,
                        )?;
                    } else {
                        // No session identity: bounded local parse diagnostic only.
                        reason = format!(
                            "contract input failure: {}/{}; no session identity",
                            message.event, field
                        );
                    }
                }
                HarnessEvidenceOutcome::Malformed => {
                    reason = format!(
                        "contract input malformed: {}; runtime metadata unavailable",
                        message.event
                    );
                }
                HarnessEvidenceOutcome::Ok => {}
            }
            while reason.len() > crate::protocol::commands::HARNESS_EVIDENCE_TEXT_BYTES {
                reason.pop();
            }
            self.store
                .record_unattributed(&message.harness, &reason, budget)?;
            return Ok(false);
        };
        let held = message
            .session_id
            .as_deref()
            .and_then(|id| self.pending().take(&message.harness, id, now_ms));
        if let Some(held) = held {
            // Put it back if its write fails: the lifecycle half of verification
            // and a SessionStart violation must not be lost to one store error.
            if let Err(error) = self.store_one(
                &message.harness,
                version,
                &held.contract_id,
                &held.event,
                &held.outcome,
                budget,
            ) {
                self.pending().restore(held, now_ms);
                return Err(error);
            }
        }
        self.store_one(
            &message.harness,
            version,
            &message.contract_id,
            &message.event,
            &outcome,
            budget,
        )
    }

    fn store_one(
        &self,
        harness: &str,
        version: &str,
        contract_id: &str,
        event: &str,
        outcome: &EvidenceOutcome,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let recorded = self.store.record_harness_evidence(
            &EvidenceRecord {
                harness,
                version,
                contract_id,
                event,
                class: event_class(event),
                outcome,
            },
            budget,
        )?;
        if let Some(manifest) = &self.manifest {
            if recorded.created {
                manifest.ensure_manifest(
                    harness,
                    FetchReason::UnseenVersion {
                        version: version.to_owned(),
                    },
                );
            }
            if recorded.fresh_violation {
                manifest.ensure_manifest(harness, FetchReason::FreshViolation);
            }
        }
        Ok(recorded.row.verified())
    }
}

#[cfg(test)]
#[path = "../../tests/daemon/harness_evidence.rs"]
mod tests;

#[derive(Clone, PartialEq, Eq)]
struct V2HoldKey {
    harness: String,
    session: String,
    domain: String,
    origin: crate::harness::evidence::EvidenceOrigin,
    contract: String,
}
// Fixed 8-KiB monotonic filter, never cleared by hold expiry/eviction. A
// collision or saturation can only withhold advisory lifecycle credit. It
// cannot forget a resumed session during this recorder's lifetime.
struct SuppressedSessions([u64; 1024]);
impl Default for SuppressedSessions {
    fn default() -> Self {
        Self([0; 1024])
    }
}
impl SuppressedSessions {
    fn indices(key: &V2HoldKey) -> [usize; 4] {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        // Length-prefix every exact field, so concatenation cannot alias keys.
        for field in [
            key.harness.as_str(),
            key.session.as_str(),
            key.domain.as_str(),
            match key.origin {
                crate::harness::evidence::EvidenceOrigin::NativePayload => "native_payload",
                crate::harness::evidence::EvidenceOrigin::NativeShapeObservation => {
                    "native_shape_observation"
                }
                crate::harness::evidence::EvidenceOrigin::BridgeEnvelope => "bridge_envelope",
            },
            key.contract.as_str(),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
        let digest = hash.finalize();
        std::array::from_fn(|i| u16::from_be_bytes([digest[i * 2], digest[i * 2 + 1]]) as usize)
    }
    fn mark(&mut self, key: &V2HoldKey) {
        for index in Self::indices(key) {
            self.0[index / 64] |= 1 << (index % 64);
        }
    }
    fn contains(&self, key: &V2HoldKey) -> bool {
        Self::indices(key)
            .iter()
            .all(|index| self.0[index / 64] & (1 << (index % 64)) != 0)
    }
}
struct HeldV2 {
    key: V2HoldKey,
    received_ms: i64,
    note: crate::protocol::commands::HarnessEvidenceV2,
}
#[derive(Default)]
struct PendingV2(VecDeque<HeldV2>);
impl PendingV2 {
    fn expire(&mut self, now: i64) {
        self.0
            .retain(|h| now.saturating_sub(h.received_ms) < PENDING_MAX_AGE_MS);
    }
    fn push(&mut self, held: HeldV2, now: i64) {
        self.expire(now);
        self.0.retain(|h| h.key != held.key);
        while self.0.len() >= PENDING_MAX_ENTRIES {
            self.0.pop_front();
        }
        self.0.push_back(held);
    }
    fn restore(&mut self, held: HeldV2, now: i64) {
        self.expire(now);
        if now.saturating_sub(held.received_ms) >= PENDING_MAX_AGE_MS
            || self.0.iter().any(|h| h.key == held.key)
        {
            return;
        }
        let at = self
            .0
            .iter()
            .position(|h| h.received_ms > held.received_ms)
            .unwrap_or(self.0.len());
        self.0.insert(at, held);
        while self.0.len() > PENDING_MAX_ENTRIES {
            self.0.pop_front();
        }
    }
}
/// Cached exact-domain existence lookup. It may skip a refresh, never verify evidence.
/// The runtime manifest reader supplies this source in its own leaf.
pub trait RichManifestSource: Send + Sync {
    fn contains(
        &self,
        harness: &str,
        identity: &crate::harness::runtime::RuntimeIdentity,
        domain: &str,
        origin: crate::harness::evidence::EvidenceOrigin,
        contract_id: &str,
    ) -> bool;
}
struct EmptyRichManifestSource;
impl RichManifestSource for EmptyRichManifestSource {
    fn contains(
        &self,
        _: &str,
        _: &crate::harness::runtime::RuntimeIdentity,
        _: &str,
        _: crate::harness::evidence::EvidenceOrigin,
        _: &str,
    ) -> bool {
        false
    }
}
/// Exact-domain advisory recorder. Legacy recording remains a separate lane.
pub struct HarnessEvidenceRecorderV2 {
    rich_manifest: Arc<dyn RichManifestSource>,
    registry: &'static crate::harness::registry::Registry,
    pending: Arc<Mutex<PendingSessionStarts>>,
    store: Arc<dyn StorePort>,
    manifest: Option<Arc<dyn ManifestTrigger>>,
    clock: Arc<dyn Clock>,
}
impl HarnessEvidenceRecorderV2 {
    pub fn new(
        store: Arc<dyn StorePort>,
        manifest: Option<Arc<dyn ManifestTrigger>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            manifest,
            clock,
            rich_manifest: Arc::new(EmptyRichManifestSource),
            registry: crate::harness::registry::builtins(),
            pending: Arc::new(Mutex::new(PendingSessionStarts::default())),
        }
    }
    pub fn record(
        &self,
        message: &crate::protocol::commands::HarnessEvidenceV2,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        message.validate().map_err(ApiError::invalid_request)?;
        let registry = self.registry;
        let registration = registry
            .by_id(
                registry
                    .agent(&message.harness)
                    .map_err(|_| Self::unsupported())?,
            )
            .map_err(|_| Self::unsupported())?;
        let descriptor = registration
            .contracts()
            .iter()
            .find(|d| {
                d.domain_id == message.domain
                    && d.origin == message.origin
                    && d.contract_id_v2().ok().as_deref() == Some(&message.contract_id)
            })
            .ok_or_else(Self::unsupported)?;
        if descriptor.event(&message.event).is_none()
            || message
                .qualifications
                .iter()
                .any(|q| !descriptor.qualifications.contains(&q.as_str()))
        {
            return Err(Self::unsupported());
        }
        if let crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation { field } =
            &message.outcome
            && !descriptor
                .contract
                .events
                .iter()
                .filter(|e| e.event == message.event)
                .any(|e| e.fields.iter().any(|f| f.path == field))
        {
            return Err(Self::unsupported());
        }
        let now = self.clock.utc_now().0;
        let key = message.session_id.as_ref().map(|session| V2HoldKey {
            harness: message.harness.clone(),
            session: session.clone(),
            domain: message.domain.clone(),
            origin: message.origin,
            contract: message.contract_id.clone(),
        });
        let lifecycle = descriptor.contract.events.iter().any(|e| {
            e.event == message.event && e.class == crate::harness::contract::EventClass::Lifecycle
        });
        let resumed = message
            .unavailable_reason
            .as_deref()
            .is_some_and(|reason| descriptor.resumed_unavailable_reason == Some(reason));
        let Some(runtime) = &message.runtime else {
            if lifecycle && let Some(key) = &key {
                let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
                pending.expire(now);
                if resumed {
                    pending.2.mark(key);
                    pending.1.0.retain(|h| &h.key != key);
                } else if !message.unavailable_reason.as_deref().is_some_and(|reason| {
                    registration
                        .nonholding_unavailable_reasons(descriptor)
                        .contains(&reason)
                }) && descriptor.may_hold(false)
                    && !pending.2.contains(key)
                {
                    pending.1.push(
                        HeldV2 {
                            key: key.clone(),
                            received_ms: now,
                            note: message.clone(),
                        },
                        now,
                    );
                    pending.bound();
                }
            }
            if let crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation { field } =
                &message.outcome
                && let Some(session_id) = message.session_id.as_deref()
            {
                self.store.record_contract_diagnostic(
                    &DiagnosticRecord {
                        harness: &message.harness,
                        session_id,
                        contract_id: &message.contract_id,
                        event: &message.event,
                        field,
                    },
                    budget,
                )?;
            }
            self.store.record_unattributed_v2(
                &message.harness,
                &message.domain,
                message.origin,
                message
                    .unavailable_reason
                    .as_deref()
                    .expect("validated reason"),
                budget,
            )?;
            return Ok(false);
        };
        let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
        pending.expire(now);
        let suppressed = key.as_ref().is_some_and(|key| pending.2.contains(key));
        let held = key
            .as_ref()
            .and_then(|key| pending.1.0.iter().position(|h| &h.key == key))
            .and_then(|at| pending.1.0.remove(at));
        // Eligibility and the synchronous SQLite commit share the suppression
        // boundary. Lock order is pending -> Store writer; neither Store nor
        // legacy recording acquires pending while holding that writer.
        let held_recorded = if let Some(held) = held {
            match self.store_one(&held.note, runtime, descriptor, !suppressed, budget) {
                Ok(recorded) => Some((held.note, recorded)),
                Err(error) => {
                    // Restoration is inside the same boundary: suppression
                    // cannot finish between the failed commit and restoring.
                    pending.expire(now);
                    if !pending.2.contains(&held.key) {
                        pending.1.restore(held, now);
                        pending.bound();
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let recorded = self.store_one(
            message,
            runtime,
            descriptor,
            !(suppressed && lifecycle),
            budget,
        );
        drop(pending);
        // External lookup/trigger callbacks may reenter the recorder. Preserve
        // a committed held note's fetch even when the direct write failed.
        if let Some((note, recorded)) = held_recorded {
            self.trigger_manifest(&note, runtime, descriptor, &recorded);
        }
        let recorded = recorded?;
        self.trigger_manifest(message, runtime, descriptor, &recorded);
        Ok(recorded.row.verified(descriptor))
    }

    fn store_one(
        &self,
        message: &crate::protocol::commands::HarnessEvidenceV2,
        runtime: &crate::harness::runtime::RuntimeIdentity,
        descriptor: &crate::harness::adapter::ContractDescriptor,
        eligible: bool,
        budget: &CallBudget,
    ) -> Result<crate::store::harness_evidence::RecordedV2, ApiError> {
        let outcome = match &message.outcome {
            crate::protocol::commands::HarnessEvidenceOutcomeV2::Ok => EvidenceOutcome::Ok,
            crate::protocol::commands::HarnessEvidenceOutcomeV2::Malformed => {
                EvidenceOutcome::Malformed
            }
            crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation { field } => {
                EvidenceOutcome::Violation {
                    field: field.clone(),
                }
            }
        };
        let qualified = eligible
            && descriptor
                .qualifications
                .iter()
                .all(|q| message.qualifications.iter().any(|fact| fact == q));
        self.store.record_harness_evidence_v2(
            &crate::store::harness_evidence::EvidenceRecordV2 {
                identity: runtime,
                descriptor,
                event: &message.event,
                outcome: &outcome,
                qualified,
            },
            budget,
        )
    }
    fn trigger_manifest(
        &self,
        message: &crate::protocol::commands::HarnessEvidenceV2,
        runtime: &crate::harness::runtime::RuntimeIdentity,
        descriptor: &crate::harness::adapter::ContractDescriptor,
        recorded: &crate::store::harness_evidence::RecordedV2,
    ) {
        if let Some(manifest) = &self.manifest {
            if recorded.created
                && !self.rich_manifest.contains(
                    &message.harness,
                    runtime,
                    descriptor.domain_id,
                    descriptor.origin,
                    &message.contract_id,
                )
            {
                manifest.ensure_manifest(
                    &message.harness,
                    FetchReason::UnseenRuntime {
                        identity: runtime.clone(),
                        domain: descriptor.domain_id.into(),
                        origin: descriptor.origin,
                        contract_id: message.contract_id.clone(),
                    },
                );
            }
            if recorded.fresh_violation {
                manifest.ensure_manifest(&message.harness, FetchReason::FreshViolation);
            }
        }
    }
    /// Shares the daemon's aggregate holding budget with the legacy recorder.
    /// Compose before either recorder receives notes.
    pub fn with_legacy_pending(mut self, legacy: &HarnessEvidenceRecorder) -> Self {
        self.pending = Arc::clone(&legacy.pending);
        self
    }
    pub fn with_rich_manifest_source(mut self, source: Arc<dyn RichManifestSource>) -> Self {
        self.rich_manifest = source;
        self
    }
    pub fn with_registry(mut self, registry: &'static crate::harness::registry::Registry) -> Self {
        self.registry = registry;
        self
    }
    fn unsupported() -> ApiError {
        ApiError::new(
            crate::protocol::results::ErrorCode::Unsupported,
            "unsupported harness evidence descriptor",
        )
    }
}
