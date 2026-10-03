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
//! A per-session gate file keeps the hook from talking on every event. The
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
        contract::{self, Classification},
    },
    ports::LocalClient,
    protocol::{
        capabilities::{Capabilities, HARNESS_EVIDENCE},
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

fn harness_name(harness: Harness) -> Option<&'static str> {
    match harness {
        Harness::Claude => Some("claude"),
        Harness::Codex => Some("codex"),
        Harness::Human => None,
    }
}

/// Classifies `stdin` against the harness's declared contract. `None` for a
/// harness without one (a person's pane).
pub fn classify_payload(
    harness: Harness,
    registered_event: Option<&str>,
    stdin: &[u8],
) -> Option<Classified> {
    let name = harness_name(harness)?;
    let declared = contract::contract_for(name)?;
    let payload: Option<Value> = (stdin.len() <= contract::MAX_PAYLOAD)
        .then(|| serde_json::from_slice::<Value>(stdin).ok())
        .flatten()
        .filter(Value::is_object);
    let classification = contract::classify(declared, registered_event, stdin);
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
        contract_id: contract::contract_id(declared),
        event: event.to_owned(),
        outcome,
        session_id,
        payload,
    })
}

impl Classified {
    /// Whether this is a Codex `SessionStart` with source `resume`.
    fn starts_codex_resume(&self) -> bool {
        self.harness == "codex"
            && self.event == "SessionStart"
            && self
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

impl GateState {
    /// Whether this payload's note is sent, from the gate and the payload alone.
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
                has_session
                    && ((self.ok_sent_at_ms.is_none() && !self.verified)
                        || self
                            .heartbeat_at_ms
                            .is_some_and(|at| now_ms.saturating_sub(at) >= HEARTBEAT_MS))
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
                if event != "SessionStart" {
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
    /// The note was sent; the daemon answered `verified`, or `None` on error.
    Sent(Option<bool>),
}

/// Decides, then (only when sending) attributes, connects and sends.
/// `connect` is called at most once, only after the gate said "send".
pub fn run(
    harness: Harness,
    registered_event: Option<&str>,
    stdin: &[u8],
    state_dir: Option<&Path>,
    now_ms: u64,
    budget: &CallBudget,
    connect: impl FnOnce(&CallBudget) -> Option<(Arc<dyn LocalClient>, Capabilities)>,
) -> Delivery {
    let Some(classified) = classify_payload(harness, registered_event, stdin) else {
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
    if classified.starts_codex_resume() && !gate.resumed {
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
    let Some((client, capabilities)) = connect(budget) else {
        return Delivery::Unavailable;
    };
    if !capabilities.supports(HARNESS_EVIDENCE) {
        return Delivery::Unavailable;
    }
    let event = classified.event.clone();
    let outcome = classified.outcome.clone();
    let evidence = classified.attribute(gate.resumed);
    let reply = client.call(Command::HarnessEvidence(evidence.message()), budget);
    let verified = match reply {
        Ok(CommandResult::HarnessEvidenceRecorded { verified }) => verified,
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
/// remaining time, every error ignored.
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
        run(
            args.harness,
            args.event.as_deref(),
            stdin,
            state_dir,
            unix_ms(),
            &budget,
            |budget| connect_running(args, &clock, budget),
        )
    }));
}

#[cfg(test)]
#[path = "../../tests/cli/hook_evidence.rs"]
mod tests;
