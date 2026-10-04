//! The hook's harness evidence note (ht-xoc.4): what one payload showed about
//! the harness that sent it, sent best effort to the daemon.
//!
//! The note carries no payload content: the harness, its version attributed
//! from the transcript, the hook's own contract id, the event, the outcome
//! (`ok`, `violation` of a named field, or `malformed`) and the session id.
//! It is sent for every admission tier, inside or outside a Herdr pane, and
//! whatever the version ladder says about the version: the evidence is exactly
//! what lets an unlisted or refused version become verified.
//!
//! In the in-pane hook the note is sent after the check-in, with the time left
//! before the watchdog (ht-rlv.1): it never delays the version probe or the
//! check-in, and a check-in that uses its whole budget skips it (the gate file
//! is then unchanged, so the next event sends it).
//!
//! A per-session gate file keeps the hook from talking on every event. Only a
//! tool-class `ok` spends the session's single `ok` slot; other `ok` events
//! (Codex `SubagentStart`) ride on the hourly heartbeat. The
//! decision is made from the gate file and the payload alone, before any
//! socket I/O; only when it says "send" does the hook read the transcript for
//! the version, connect to a daemon that is already running (never started
//! for this) and look for the `hook.harness_evidence` capability. A daemon
//! without the capability gets nothing and the gate file is otherwise left
//! alone. A Codex `SessionStart` with source `resume` marks its session's gate
//! as resumed at once (even with no daemon, before any socket I/O): a resumed
//! rollout keeps the creating CLI's version, so every later note of that
//! session goes out unattributed. A hook with no state directory, or a resumed
//! session idle long enough for its gate file to be pruned, attributes from
//! the rollout head again (accepted). Every error is ignored: nothing here changes the hook's stdout or exit status.
//!
//! Registered adapters with an explicit legacy projection keep that path.
//! Rich-only adapters use negotiated v2 metadata and a separate strict
//! `evidence-v2` gate, keyed by exact runtime, domain, origin, contract and
//! session. Adapter-supplied qualifications are bounded and declared; payload
//! claims never supply them. Only a timely successful advisory response can
//! advance a send hint, and an unqualified note cannot spend a required retry.
//!
//! Evidence is advisory data about the harness, never authority (see
//! TRUST-POLICY.md): this module adds no caller attribution and no seat state.
use super::hook::HookArgs;
use crate::{
    client::local::LocalSocketClient,
    daemon::{
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
    },
    harness::{
        attribution::{Attribution, attribute_payload},
        codex_evidence,
        context::Harness,
        contract::{self, Classification, EventClass},
    },
    ports::LocalClient,
    protocol::{
        capabilities::{Capabilities, HARNESS_EVIDENCE, HARNESS_EVIDENCE_V2},
        commands::{Command, HarnessEvidence, HarnessEvidenceOutcome},
        results::CommandResult,
        time::{CallBudget, Clock},
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

/// Subdirectory of `<state>/harness` holding the per-session gate files.
pub const GATE_DIR: &str = "evidence";
/// A gate file is bounded; a larger or foreign one is ignored.
pub const MAX_GATE_BYTES: u64 = 4096;
/// One `ok` heartbeat per session per this long, exempt from the verified gate.
pub const HEARTBEAT_MS: u64 = 60 * 60 * 1000;
/// A gate file untouched for this long is pruned.
pub const GATE_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
/// At most this many gate files are removed per run.
pub const PRUNE_PER_RUN: usize = 16;
/// At most this many directory entries are looked at per run.
const PRUNE_SCAN: usize = 256;
/// Longest call the evidence send may take.
pub const CALL_CAP: Duration = Duration::from_millis(500);
/// Call budget for a foreign (not a pane of the installed instance) session.
pub const FOREIGN_CALL: Duration = Duration::from_millis(300);
/// Remembered `<event>|<field>` entries per gate file.
const SENT_KEEP: usize = 16;

/// The classified payload before attribution: everything the gate needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub harness: &'static str,
    pub contract_id: String,
    pub event: String,
    pub outcome: HarnessEvidenceOutcome,
    pub session_id: Option<String>,
    payload: Option<Value>,
}

/// One note ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub harness: &'static str,
    pub version: Option<String>,
    pub unattributed_reason: Option<String>,
    pub contract_id: String,
    pub event: String,
    pub outcome: HarnessEvidenceOutcome,
    pub session_id: Option<String>,
}

fn registration_for(harness: Harness) -> Option<&'static crate::harness::registry::Registration> {
    let registry = crate::harness::registry::builtins();
    registry.by_id(registry.agent(harness.as_str()).ok()?).ok()
}

/// Classifies `stdin` against the harness's declared contract. `None` for a
/// harness without one (a person's pane).
pub fn classify_payload(
    harness: Harness,
    registered_event: Option<&str>,
    stdin: &[u8],
) -> Option<Classified> {
    let registration = registration_for(harness)?;
    classify_legacy(registration, registered_event, stdin)
}

fn classify_legacy(
    registration: &crate::harness::registry::Registration,
    registered_event: Option<&str>,
    stdin: &[u8],
) -> Option<Classified> {
    let name = registration.metadata().id;
    let input = crate::harness::adapter::HookInput {
        bytes: stdin.to_vec(),
        registered_event: registered_event.map(str::to_owned),
    };
    let observation = registration.classify(&input);
    let declared = registration
        .contracts()
        .iter()
        .find(|d| d.domain == observation.domain)?
        .contract;
    let payload: Option<Value> = (stdin.len() <= contract::MAX_PAYLOAD)
        .then(|| serde_json::from_slice::<Value>(stdin).ok())
        .flatten()
        .filter(Value::is_object);
    let classification = observation.classification;
    let discriminator = payload
        .as_ref()
        .and_then(|value| value.get(declared.discriminator))
        .and_then(Value::as_str)
        .filter(|name| {
            (1..=63).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    let (event, outcome) = match classification {
        Classification::Ok { event } => (
            registered_event.unwrap_or(event),
            HarnessEvidenceOutcome::Ok,
        ),
        Classification::Violation { event, field } => (
            registered_event.unwrap_or(event),
            HarnessEvidenceOutcome::Violation {
                field: field.to_owned(),
            },
        ),
        Classification::Malformed(_) => (
            registered_event.or(discriminator).unwrap_or("unknown"),
            HarnessEvidenceOutcome::Malformed,
        ),
    };
    let session_id = payload
        .as_ref()
        .and_then(|value| value.get("session_id"))
        .and_then(Value::as_str)
        .filter(|id| id.len() <= crate::protocol::commands::HARNESS_EVIDENCE_SESSION_BYTES)
        .map(str::to_owned);
    Some(Classified {
        harness: name,
        contract_id: registration.legacy_contract_id()?,
        event: event.to_owned(),
        outcome,
        session_id,
        payload,
    })
}

impl Classified {
    /// The legacy adapter's declared sticky creating-runtime suppression.
    fn starts_suppressed_resume(
        &self,
        registration: &crate::harness::registry::Registration,
    ) -> bool {
        registration.contracts().iter().any(|d| {
            d.holding == crate::harness::evidence::AttributionHolding::SuppressResumed
                && d.event(&self.event).is_some_and(|e| e.always_send)
        }) && self
            .payload
            .as_ref()
            .and_then(|p| p.get("source"))
            .and_then(Value::as_str)
            == Some("resume")
    }

    /// Reads the version from the payload's transcript (bounded, in-process).
    /// A Codex session known to be `resumed` is never attributed and its
    /// transcript is not opened.
    pub fn attribute(self, resumed: bool) -> Evidence {
        let attribution = match &self.payload {
            _ if resumed && self.harness == "codex" => Attribution::Unattributable {
                reason: crate::harness::attribution::Unattributed::CodexResumed,
            },
            Some(payload) => attribute_payload(self.harness, payload),
            None => Attribution::Unattributable {
                reason: crate::harness::attribution::Unattributed::NoTranscriptPath,
            },
        };
        let (version, unattributed_reason) = match attribution {
            Attribution::Attributed { version, .. } => (Some(version), None),
            Attribution::Unattributable { reason } => (None, Some(reason.as_str().to_owned())),
        };
        Evidence {
            harness: self.harness,
            version,
            unattributed_reason,
            contract_id: self.contract_id,
            event: self.event,
            outcome: self.outcome,
            session_id: self.session_id,
        }
    }
}

/// The note for one payload: classification plus attribution.
pub fn evidence_for(
    harness: Harness,
    registered_event: Option<&str>,
    stdin: &[u8],
) -> Option<Evidence> {
    classify_payload(harness, registered_event, stdin).map(|classified| classified.attribute(false))
}

impl Evidence {
    fn message(&self) -> HarnessEvidence {
        HarnessEvidence {
            harness: self.harness.to_owned(),
            version: self.version.clone(),
            unattributed_reason: self.unattributed_reason.clone(),
            contract_id: self.contract_id.clone(),
            event: self.event.clone(),
            outcome: self.outcome.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

// -- the per-session gate ---------------------------------------------------

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateState {
    pub verified: bool,
    /// When the session's tool-class `ok` was sent (ht-rlv.3).
    pub ok_sent_at_ms: Option<u64>,
    pub heartbeat_at_ms: Option<u64>,
    /// `<event>|<field or empty>` of every violation / malformed note sent.
    pub sent: Vec<String>,
    /// A Codex session that started with SessionStart source resume: its
    /// rollout's version is the creator's, so nothing it sends is attributed.
    #[serde(default)]
    pub resumed: bool,
}

fn sent_key(event: &str, outcome: &HarnessEvidenceOutcome) -> Option<String> {
    match outcome {
        HarnessEvidenceOutcome::Ok => None,
        HarnessEvidenceOutcome::Violation { field } => Some(format!("{event}|{field}")),
        HarnessEvidenceOutcome::Malformed => Some(format!("{event}|")),
    }
}

/// Only a tool event spends the session's single `ok` slot (ht-rlv.3): a
/// lifecycle `ok` is always sent and an other-class `ok` (Codex
/// `SubagentStart`) never completes verification, so it rides only on the
/// hourly heartbeat.
fn ok_class(event: &str) -> EventClass {
    crate::daemon::harness_evidence::event_class(event)
}

impl GateState {
    /// Whether this payload's note is sent, from the gate and the payload alone.
    /// `SessionStart` is always sent; a tool `ok` is sent once while the
    /// session is unverified; any other `ok` only when the heartbeat is due.
    pub fn should_send(
        &self,
        event: &str,
        outcome: &HarnessEvidenceOutcome,
        has_session: bool,
        now_ms: u64,
    ) -> bool {
        if event == "SessionStart" {
            return true;
        }
        match sent_key(event, outcome) {
            Some(key) => !has_session || !self.sent.contains(&key),
            None => {
                let heartbeat = self
                    .heartbeat_at_ms
                    .is_some_and(|at| now_ms.saturating_sub(at) >= HEARTBEAT_MS);
                has_session
                    && match ok_class(event) {
                        EventClass::Tool => {
                            (self.ok_sent_at_ms.is_none() && !self.verified) || heartbeat
                        }
                        _ => heartbeat,
                    }
            }
        }
    }

    /// The state after a note was sent and the daemon said `verified`.
    pub fn after_send(
        mut self,
        event: &str,
        outcome: &HarnessEvidenceOutcome,
        verified: bool,
        now_ms: u64,
    ) -> Self {
        self.verified = verified;
        match sent_key(event, outcome) {
            Some(key) => {
                if !self.sent.contains(&key) {
                    self.sent.push(key);
                }
                while self.sent.len() > SENT_KEEP {
                    self.sent.remove(0);
                }
            }
            None => {
                self.heartbeat_at_ms = Some(now_ms);
                if ok_class(event) == EventClass::Tool {
                    self.ok_sent_at_ms = Some(now_ms);
                }
            }
        }
        self
    }
}

/// `<state>/harness/evidence`.
pub fn gate_dir(state_dir: &Path) -> PathBuf {
    codex_evidence::dir(state_dir).join(GATE_DIR)
}

/// `<gate dir>/<harness>-<first 16 hex of sha256(session id)>.json`.
pub fn gate_path(dir: &Path, harness: &str, session_id: &str) -> PathBuf {
    let digest = Sha256::digest(session_id.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    dir.join(format!("{harness}-{hex}.json"))
}

/// The stored gate: only a bounded private regular file this user owns is
/// trusted; anything else reads as a fresh gate.
fn read_gate(path: &Path) -> GateState {
    let read = || -> Option<GateState> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if !meta.is_file()
            || meta.len() > MAX_GATE_BYTES
            || meta.uid() != crate::daemon::paths::effective_uid()
            || meta.permissions().mode() & 0o077 != 0
        {
            return None;
        }
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    };
    read().unwrap_or_default()
}

fn write_gate(state_dir: &Path, path: &Path, state: &GateState) -> io::Result<()> {
    let mut state = state.clone();
    let bytes = loop {
        let bytes = serde_json::to_vec(&state).map_err(io::Error::other)?;
        if bytes.len() as u64 <= MAX_GATE_BYTES {
            break bytes;
        }
        if state.sent.is_empty() {
            return Err(io::Error::other("gate file exceeds its bound"));
        }
        state.sent.remove(0);
    };
    codex_evidence::prepare(state_dir)?;
    crate::daemon::paths::ensure_private_dir(&gate_dir(state_dir))?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut out| out.write_all(&bytes).and_then(|()| out.sync_all()))
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Removes at most [`PRUNE_PER_RUN`] gate files older than 24 hours.
fn prune(dir: &Path, now_ms: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut removed = 0;
    for entry in entries.take(PRUNE_SCAN).flatten() {
        if removed >= PRUNE_PER_RUN {
            break;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta
            .modified()
            .ok()
            .and_then(|at| at.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(now_ms, |elapsed| elapsed.as_millis() as u64);
        if meta.is_file() && now_ms.saturating_sub(modified) >= GATE_MAX_AGE_MS {
            removed += usize::from(std::fs::remove_file(entry.path()).is_ok());
        }
    }
}

// -- the run ----------------------------------------------------------------

/// Whether a note went out, for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The gate said no (or nothing could be classified): no socket I/O.
    Suppressed,
    /// There was no daemon to talk to, or it lacks the capability.
    Unavailable,
    /// Rich evidence is unsupported by the connected daemon; no legacy downgrade.
    Unsupported,
    /// The note was sent; the daemon answered `verified`, or `None` on error.
    Sent(Option<bool>),
}

/// Legacy compatibility helper; `connect` is called only after its gate says send.
/// The production `report` uses `run_registered` with the shared budget clock;
/// rich adapters require that entrypoint rather than this clock-less helper.
pub fn run(
    harness: Harness,
    registered_event: Option<&str>,
    stdin: &[u8],
    state_dir: Option<&Path>,
    now_ms: u64,
    budget: &CallBudget,
    connect: impl FnOnce(&CallBudget) -> Option<(Arc<dyn LocalClient>, Capabilities)>,
) -> Delivery {
    let Some(registration) = registration_for(harness) else {
        return Delivery::Suppressed;
    };
    if registration.legacy_contract_id().is_none() {
        return Delivery::Unsupported;
    }
    run_registered(
        registration,
        registered_event,
        stdin,
        state_dir,
        now_ms,
        (budget, &crate::app::SystemClock::new()),
        connect,
    )
}

/// Actual evidence consumer boundary; transport projection belongs to the adapter.
/// The clock in `timing` must share the supplied call budget's monotonic epoch.
pub fn run_registered(
    registration: &crate::harness::registry::Registration,
    registered_event: Option<&str>,
    stdin: &[u8],
    state_dir: Option<&Path>,
    now_ms: u64,
    timing: (&CallBudget, &dyn Clock),
    connect: impl FnOnce(&CallBudget) -> Option<(Arc<dyn LocalClient>, Capabilities)>,
) -> Delivery {
    let (budget, clock) = timing;
    if registration.legacy_contract_id().is_none() {
        return run_v2(
            registration,
            registered_event,
            stdin,
            state_dir,
            now_ms,
            timing,
            connect,
        );
    }
    let Some(classified) = classify_legacy(registration, registered_event, stdin) else {
        return Delivery::Suppressed;
    };
    if let Some(state) = state_dir {
        prune(&gate_dir(state), now_ms);
    }
    let gate_file = state_dir
        .zip(classified.session_id.as_deref())
        .map(|(state, session)| {
            (
                state,
                gate_path(&gate_dir(state), classified.harness, session),
            )
        });
    let mut gate = gate_file
        .as_ref()
        .map_or_else(GateState::default, |(_, path)| read_gate(path));
    if classified.starts_suppressed_resume(registration) && !gate.resumed {
        gate.resumed = true;
        if let Some((state, path)) = &gate_file {
            let _ = write_gate(state, path, &gate);
        }
    }
    if !gate.should_send(
        &classified.event,
        &classified.outcome,
        classified.session_id.is_some(),
        now_ms,
    ) {
        return Delivery::Suppressed;
    }
    if budget.is_exhausted(clock) {
        return Delivery::Unavailable;
    }
    let Some((client, capabilities)) = connect(budget) else {
        return Delivery::Unavailable;
    };
    if !capabilities.supports(HARNESS_EVIDENCE) {
        return Delivery::Unavailable;
    }
    let event = classified.event.clone();
    let outcome = classified.outcome.clone();
    let input = crate::harness::adapter::HookInput {
        bytes: stdin.to_vec(),
        registered_event: registered_event.map(str::to_owned),
    };
    let (version, unattributed_reason) =
        match registration.attribute_runtime_for_session(&input, budget, gate.resumed) {
            crate::harness::adapter::RuntimeAttribution::Attributed(identity)
                if identity.key.starts_with("release:") =>
            {
                (identity.release_version.clone(), None)
            }
            crate::harness::adapter::RuntimeAttribution::Attributed(_) => (
                None,
                Some("runtime has no legacy release projection".into()),
            ),
            crate::harness::adapter::RuntimeAttribution::Unavailable { diagnostic } => {
                (None, Some(diagnostic))
            }
        };
    let evidence = Evidence {
        harness: classified.harness,
        version,
        unattributed_reason,
        contract_id: classified.contract_id,
        event: classified.event,
        outcome: classified.outcome,
        session_id: classified.session_id,
    };
    let reply = client.call(Command::HarnessEvidence(evidence.message()), budget);
    let verified = match reply {
        Ok(CommandResult::HarnessEvidenceRecorded { verified }) if !budget.is_exhausted(clock) => {
            verified
        }
        _ => return Delivery::Sent(None),
    };
    if let Some((state, path)) = &gate_file {
        let _ = write_gate(
            state,
            path,
            &gate.after_send(&event, &outcome, verified, now_ms),
        );
    }
    Delivery::Sent(Some(verified))
}

/// Versioned separately from the frozen legacy resumed gate.
pub const V2_GATE_DIR: &str = "evidence-v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct V2GateKey {
    harness: String,
    runtime: Option<crate::harness::runtime::RuntimeIdentity>,
    unavailable_reason: Option<String>,
    domain: String,
    origin: crate::harness::evidence::EvidenceOrigin,
    contract_id: String,
    session_id: Option<String>,
}
impl V2GateKey {
    fn from_note(note: &crate::protocol::commands::HarnessEvidenceV2) -> Self {
        Self {
            harness: note.harness.clone(),
            runtime: note.runtime.clone(),
            unavailable_reason: note.unavailable_reason.clone(),
            domain: note.domain.clone(),
            origin: note.origin,
            contract_id: note.contract_id.clone(),
            session_id: note.session_id.clone(),
        }
    }
    fn path(&self, state: &Path) -> PathBuf {
        let bytes = serde_json::to_vec(self).expect("bounded gate key serializes");
        v2_gate_dir(state).join(format!("{:x}.json", Sha256::digest(bytes)))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct V2GateState {
    version: u8,
    key: V2GateKey,
    verified: bool,
    milestones: Vec<String>,
    heartbeat_at_ms: Option<u64>,
    sent: Vec<String>,
}
pub fn v2_gate_dir(state: &Path) -> PathBuf {
    codex_evidence::dir(state).join(V2_GATE_DIR)
}
impl V2GateState {
    fn fresh(key: V2GateKey) -> Self {
        Self {
            version: 2,
            key,
            verified: false,
            milestones: vec![],
            heartbeat_at_ms: None,
            sent: vec![],
        }
    }
    fn read(path: &Path, key: V2GateKey) -> Self {
        let read = || -> Option<Self> {
            let meta = std::fs::symlink_metadata(path).ok()?;
            if !meta.is_file()
                || meta.len() > MAX_GATE_BYTES
                || meta.uid() != crate::daemon::paths::effective_uid()
                || meta.permissions().mode() & 0o077 != 0
            {
                return None;
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .ok()?;
            use std::io::Read;
            let mut bytes = Vec::new();
            file.take(MAX_GATE_BYTES + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() as u64 > MAX_GATE_BYTES {
                return None;
            }
            let stored: Self = serde_json::from_slice(&bytes).ok()?;
            (stored.version == 2
                && stored.key == key
                && stored.milestones.len() <= 8
                && stored.sent.len() <= SENT_KEEP)
                .then_some(stored)
        };
        read().unwrap_or_else(|| Self::fresh(key))
    }
    fn should_send(
        &self,
        descriptor: &crate::harness::adapter::ContractDescriptor,
        note: &crate::protocol::commands::HarnessEvidenceV2,
        now: u64,
    ) -> bool {
        if descriptor
            .event(&note.event)
            .is_some_and(|event| event.always_send)
        {
            return true;
        }
        if let Some(key) = v2_sent_key(note) {
            return !self.sent.contains(&key);
        }
        let heartbeat = self
            .heartbeat_at_ms
            .is_some_and(|at| now.saturating_sub(at) >= HEARTBEAT_MS);
        let required = descriptor
            .event(&note.event)
            .and_then(|e| e.milestone)
            .is_some_and(|m| {
                descriptor.required_milestones.contains(&m)
                    && !self.milestones.iter().any(|sent| sent == m)
            });
        (!self.verified && required) || heartbeat
    }
    fn after_send(
        mut self,
        descriptor: &crate::harness::adapter::ContractDescriptor,
        note: &crate::protocol::commands::HarnessEvidenceV2,
        verified: bool,
        now: u64,
    ) -> Self {
        self.verified = verified;
        if let Some(key) = v2_sent_key(note) {
            if !self.sent.contains(&key) {
                self.sent.push(key);
            }
            while self.sent.len() > SENT_KEEP {
                self.sent.remove(0);
            }
        } else {
            self.heartbeat_at_ms = Some(now);
            if let Some(milestone) = descriptor.event(&note.event).and_then(|e| e.milestone)
                && note.runtime.is_some()
                && descriptor
                    .qualifications
                    .iter()
                    .all(|required| note.qualifications.iter().any(|fact| fact == required))
                && descriptor.required_milestones.contains(&milestone)
                && !self.milestones.iter().any(|sent| sent == milestone)
            {
                self.milestones.push(milestone.into());
            }
        }
        self
    }
    fn write(&self, state: &Path, path: &Path) -> io::Result<()> {
        let mut stored = self.clone();
        let bytes = loop {
            let bytes = serde_json::to_vec(&stored).map_err(io::Error::other)?;
            if bytes.len() as u64 <= MAX_GATE_BYTES {
                break bytes;
            }
            if stored.sent.is_empty() {
                return Err(io::Error::other("v2 gate exceeds bound"));
            }
            stored.sent.remove(0);
        };
        codex_evidence::prepare(state)?;
        crate::daemon::paths::ensure_private_dir(&v2_gate_dir(state))?;
        let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut out| out.write_all(&bytes).and_then(|()| out.sync_all()))
            .and_then(|()| std::fs::rename(&temporary, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written
    }
}
fn v2_sent_key(note: &crate::protocol::commands::HarnessEvidenceV2) -> Option<String> {
    use crate::protocol::commands::HarnessEvidenceOutcomeV2 as Outcome;
    match &note.outcome {
        Outcome::Ok => None,
        Outcome::Malformed => Some(format!("{}|", note.event)),
        Outcome::Violation { field } => Some(format!("{}|{field}", note.event)),
    }
}

fn run_v2(
    registration: &crate::harness::registry::Registration,
    registered_event: Option<&str>,
    stdin: &[u8],
    state_dir: Option<&Path>,
    now_ms: u64,
    timing: (&CallBudget, &dyn Clock),
    connect: impl FnOnce(&CallBudget) -> Option<(Arc<dyn LocalClient>, Capabilities)>,
) -> Delivery {
    use crate::harness::adapter::{HookInput, RuntimeAttribution};
    use crate::protocol::commands::{HarnessEvidenceOutcomeV2 as Outcome, HarnessEvidenceV2};
    let (budget, clock) = timing;
    if budget.is_exhausted(clock) {
        return Delivery::Unavailable;
    }
    let input = HookInput {
        bytes: stdin.to_vec(),
        registered_event: registered_event.map(str::to_owned),
    };
    let observed = registration.classify(&input);
    let Some(descriptor) = registration
        .contracts()
        .iter()
        .find(|d| d.domain == observed.domain)
    else {
        return Delivery::Unsupported;
    };
    let payload: Option<Value> = (stdin.len() <= contract::MAX_PAYLOAD)
        .then(|| serde_json::from_slice(stdin).ok())
        .flatten()
        .filter(Value::is_object);
    let (event, outcome) = match observed.classification {
        Classification::Ok { event } => (event.to_owned(), Outcome::Ok),
        Classification::Violation { event, field } => (
            event.to_owned(),
            Outcome::Violation {
                field: field.into(),
            },
        ),
        Classification::Malformed(_) => (
            registered_event
                .or_else(|| {
                    payload
                        .as_ref()?
                        .get(descriptor.contract.discriminator)?
                        .as_str()
                })
                .unwrap_or("unknown")
                .to_owned(),
            Outcome::Malformed,
        ),
    };
    let (runtime, unavailable_reason) = match registration.attribute_runtime(&input, budget) {
        RuntimeAttribution::Attributed(identity) => (Some(identity), None),
        RuntimeAttribution::Unavailable { diagnostic } => (None, Some(diagnostic)),
    };
    let Ok(contract_id) = descriptor.contract_id_v2() else {
        return Delivery::Unsupported;
    };
    let qualifications = match runtime.as_ref() {
        Some(runtime) => match registration.evidence_qualifications(
            &crate::harness::adapter::EvidenceQualificationRequest {
                input: &input,
                runtime,
                descriptor,
            },
            budget,
        ) {
            Ok(facts) => facts,
            Err(_) => return Delivery::Unsupported,
        },
        None => Vec::new(),
    };
    let note = HarnessEvidenceV2 {
        harness: registration.metadata().id.into(),
        domain: descriptor.domain_id.into(),
        origin: descriptor.origin,
        runtime,
        unavailable_reason,
        contract_id,
        event,
        outcome,
        session_id: payload
            .as_ref()
            .and_then(|p| p.get("session_id"))
            .and_then(Value::as_str)
            .filter(|s| crate::harness::runtime::printable(s, 256))
            .map(str::to_owned),
        qualifications,
    };
    if note.validate().is_err() {
        return Delivery::Unsupported;
    }
    if let Some(state) = state_dir {
        prune(&v2_gate_dir(state), now_ms);
    }
    let key = V2GateKey::from_note(&note);
    let gate_file = state_dir
        .zip(note.session_id.as_ref())
        .map(|(state, _)| (state, key.path(state)));
    let gate = gate_file.as_ref().map_or_else(
        || V2GateState::fresh(key.clone()),
        |(_, path)| V2GateState::read(path, key.clone()),
    );
    if !gate.should_send(descriptor, &note, now_ms) {
        return Delivery::Suppressed;
    }
    if budget.is_exhausted(clock) {
        return Delivery::Unavailable;
    }
    let Some((client, capabilities)) = connect(budget) else {
        return Delivery::Unavailable;
    };
    if !capabilities.supports(HARNESS_EVIDENCE_V2) {
        return Delivery::Unsupported;
    }
    if budget.is_exhausted(clock) {
        return Delivery::Unavailable;
    }
    let reply = client.call(Command::HarnessEvidenceV2(note.clone()), budget);
    let verified = match reply {
        Ok(CommandResult::HarnessEvidenceV2Recorded(recorded)) if !budget.is_exhausted(clock) => {
            recorded.verified
        }
        _ => return Delivery::Sent(None),
    };
    if let Some((state, path)) = gate_file {
        let _ = gate
            .after_send(descriptor, &note, verified, now_ms)
            .write(state, &path);
    }
    Delivery::Sent(Some(verified))
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Connects to the daemon already running for this hook's instance; never
/// starts one.
fn connect_running(
    args: &HookArgs,
    clock: &Arc<dyn Clock>,
    budget: &CallBudget,
) -> Option<(Arc<dyn LocalClient>, Capabilities)> {
    let context =
        RuntimeContext::from_environment(args.state_dir.clone(), args.host_endpoint.clone())
            .ok()?;
    let paths = InstancePaths::resolve(&context).ok()?;
    let instance = read_existing_namespace(&paths).ok().flatten()?;
    let descriptor = read_descriptor(&paths, instance).ok()?;
    crate::daemon::lifecycle::check_protocol(&descriptor).ok()?;
    let client = LocalSocketClient::new(
        descriptor.endpoint,
        Arc::clone(clock),
        instance,
        Some(descriptor.boot_id),
    );
    let capabilities = client.capabilities(budget);
    Some((Arc::new(client), capabilities))
}

/// The hook's evidence step: best effort, within `call_cap` and the hook's
/// remaining time (`deadline`), every error ignored. The in-pane hook calls it
/// last, after the check-in (`hook::sequence`), so it never cuts the probe's
/// or the check-in's budget.
pub fn report(
    args: &HookArgs,
    stdin: &[u8],
    state_dir: Option<&Path>,
    deadline: Instant,
    call_cap: Duration,
    clock: Arc<dyn Clock>,
) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let cap = remaining.min(call_cap);
    if cap.is_zero() {
        return;
    }
    let budget = super::hook::budget(Instant::now() + cap, clock.as_ref());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(registration) = registration_for(args.harness) else {
            return;
        };
        run_registered(
            registration,
            args.event.as_deref(),
            stdin,
            state_dir,
            unix_ms(),
            (&budget, clock.as_ref()),
            |budget| connect_running(args, &clock, budget),
        );
    }));
}

#[cfg(test)]
#[path = "../../tests/cli/hook_evidence.rs"]
mod tests;
