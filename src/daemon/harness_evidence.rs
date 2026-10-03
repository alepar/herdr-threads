//! The daemon half of harness version evidence (ht-xoc.4).
//!
//! A hook's [`HarnessEvidence`] note is recorded in `harness_version_evidence`
//! (or, with no version, only the reason goes to `harness_unattributed`). An
//! unattributed `SessionStart` (a resumed Claude session, say) is held per
//! `(harness, session id)` for up to 24 hours and credited to the session's
//! first attributed event, because the version only becomes readable once the
//! transcript has an entry. Recording never waits for the network: a first-seen
//! version or a fresh violation only asks the manifest service to fetch, and
//! that call returns at once.
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
    store::harness_evidence::{EventClass, EvidenceOutcome, EvidenceRecord},
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
struct PendingSessionStarts(VecDeque<Held>);

impl PendingSessionStarts {
    fn expire(&mut self, now_ms: i64) {
        self.0
            .retain(|held| now_ms.saturating_sub(held.received_ms) < PENDING_MAX_AGE_MS);
    }

    fn hold(&mut self, held: Held, now_ms: i64) {
        self.expire(now_ms);
        self.0
            .retain(|old| !(old.harness == held.harness && old.session_id == held.session_id));
        while self.0.len() >= PENDING_MAX_ENTRIES {
            self.0.pop_front();
        }
        self.0.push_back(held);
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
    store: Arc<dyn StorePort>,
    manifest: Option<Arc<dyn ManifestTrigger>>,
    pending: Mutex<PendingSessionStarts>,
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

impl HarnessEvidenceRecorder {
    pub fn new(
        store: Arc<dyn StorePort>,
        manifest: Option<Arc<dyn ManifestTrigger>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            store,
            manifest,
            pending: Mutex::new(PendingSessionStarts::default()),
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
            let reason = message
                .unattributed_reason
                .as_deref()
                .unwrap_or("unattributed");
            self.store
                .record_unattributed(&message.harness, reason, budget)?;
            if message.event == "SessionStart"
                && let Some(session_id) = &message.session_id
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
            return Ok(false);
        };
        let held = message
            .session_id
            .as_deref()
            .and_then(|id| self.pending().take(&message.harness, id, now_ms));
        if let Some(held) = held {
            self.store_one(
                &message.harness,
                version,
                &held.contract_id,
                &held.event,
                &held.outcome,
                budget,
            )?;
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
