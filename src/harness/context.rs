//! Local cooperative context journal. No transport, ACK, or invitation acceptance lives here.
use crate::{
    harness::cache::{CacheReadError, CachedCheckIn, CachedCheckInKey},
    protocol::time::{CallBudget, Clock},
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_BYTES: usize = 1_048_576;
const MAX_PAYLOAD: usize = 65_536;
const MAX_HISTORY: usize = 128;
/// Completed entries kept for exact replay and cached-offer reads. `current`
/// stays authoritative; older completions are pruned by count, age and size.
pub const RETAIN_COMPLETED: usize = 32;
/// Completed entries older than this are pruned (the newest one is kept).
pub const RETAIN_AGE_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Terminally abandoned request keys kept so a dead request is never re-prepared.
const RETAIN_ABANDONED: usize = 32;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Harness(crate::harness::registry::OccupantHarness);
#[allow(non_upper_case_globals)]
impl Harness {
    pub const Codex: Self = Self(crate::harness::registry::OccupantHarness::Codex);
    pub const Claude: Self = Self(crate::harness::registry::OccupantHarness::Claude);
    pub const Human: Self = Self(crate::harness::registry::OccupantHarness::Human);
    pub fn occupant(self) -> crate::harness::registry::OccupantHarness {
        self.0
    }
    pub fn as_str(self) -> &'static str {
        self.0.as_str()
    }
}
impl From<crate::harness::registry::OccupantHarness> for Harness {
    fn from(value: crate::harness::registry::OccupantHarness) -> Self {
        Self(value)
    }
}
impl From<Harness> for crate::harness::registry::OccupantHarness {
    fn from(value: Harness) -> Self {
        value.0
    }
}
impl Serialize for Harness {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let _ = crate::harness::registry::builtins();
        serializer.serialize_str(match self.0 {
            crate::harness::registry::OccupantHarness::Agent(id) => id.context_spelling(),
            crate::harness::registry::OccupantHarness::Human => "Human",
        })
    }
}
impl<'de> Deserialize<'de> for Harness {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let spelling = String::deserialize(deserializer)?;
        crate::harness::registry::builtins()
            .by_context_spelling(&spelling)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    TopLevel,
    Subagent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionReference {
    Native(String),
    PluginContext(Uuid),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupantContext {
    pub format_version: u32,
    pub instance: Uuid,
    pub seat: String,
    pub target: String,
    pub harness: Harness,
    pub binding_generation: u64,
    pub execution: Uuid,
    pub session: SessionReference,
    pub role: Role,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    Startup,
    Restart,
    Resume,
    Clear,
    Tool,
    Compact,
}
impl EventKind {
    pub fn mode(self) -> CheckInMode {
        match self {
            Self::Tool | Self::Compact => CheckInMode::Current,
            _ => CheckInMode::Lifecycle,
        }
    }
}
impl OccupantContext {
    /// Build before serializing the eventual CLI intent. Caller persists this UUID in that intent.
    pub fn for_event(
        &self,
        kind: EventKind,
        execution: Uuid,
        native_session: Option<String>,
    ) -> Result<Self, ContextError> {
        if self.role == Role::Subagent {
            return Err(ContextError::Child);
        }
        if kind.mode() == CheckInMode::Current {
            return Ok(self.clone());
        }
        if execution.is_nil() || execution == self.execution {
            return Err(ContextError::Conflict);
        }
        let mut next = self.clone();
        next.execution = execution;
        next.session = match native_session {
            Some(s) if valid_text(&s) => SessionReference::Native(s),
            Some(_) => return Err(ContextError::Invalid),
            None => SessionReference::PluginContext(execution),
        };
        Ok(next)
    }
}
/// Conservative default; startup attach is an explicitly declared adapter policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedTurnPolicy {
    Strict,
    StartupAttach,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredReset {
    pub session: String,
    pub event_key: String,
    pub ordering: ObservationOrder,
}
const RESET_HINT_TTL: i64 = 600_000;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclaredResetHint {
    harness: Harness,
    target: String,
    session: String,
    event_key: String,
    process_nonce: Uuid,
    sequence: u64,
    observed_at_millis: i64,
    generation: u64,
    consumed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedKind {
    event_id: String,
    operation_id: Uuid,
    request_digest: String,
    kind: EventKind,
}
fn request_digest(request: &PendingCheckIn, kind: EventKind) -> Result<String, ContextError> {
    serde_json::to_vec(&(request, kind))
        .map(|bytes| super::setup::fingerprint(&bytes))
        .map_err(|_| ContextError::Corrupt)
}
fn qualified_policy(harness: Harness) -> QualifiedTurnPolicy {
    let registry = super::registry::builtins();
    registry
        .agent(harness.as_str())
        .ok()
        .and_then(|id| registry.by_id(id).ok())
        .map_or(QualifiedTurnPolicy::Strict, |r| r.qualified_turn_policy())
}
/// Adapter-qualified identity observation, not native attestation or continuity authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedTurn {
    pub session: String,
    pub event_key: String,
    pub reset: Option<ResetObservation>,
    pub ordering: Option<ObservationOrder>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetObservation {
    pub previous_session: String,
}
/// Synthesized callback-entry ordering. A nonce restart alone never rotates execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationOrder {
    pub process_nonce: Uuid,
    pub sequence: u64,
    pub observed_at_millis: i64,
    pub callback_budget_millis: u32,
}
impl ObservationOrder {
    pub fn validate_deadline(&self, now: i64) -> Result<(), ContextError> {
        if self.process_nonce.is_nil()
            || self.sequence == 0
            || self.observed_at_millis < 0
            || self.observed_at_millis > now
            || !(1..=5000).contains(&self.callback_budget_millis)
        {
            return Err(ContextError::Invalid);
        }
        let deadline = self
            .observed_at_millis
            .checked_add(i64::from(self.callback_budget_millis))
            .ok_or(ContextError::Invalid)?;
        if now >= deadline {
            return Err(ContextError::Conflict);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationWatermark {
    harness: Harness,
    process_nonce: Uuid,
    sequence: u64,
    observed_at_millis: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckInMode {
    Current,
    Lifecycle,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingCheckIn {
    pub operation_id: Uuid,
    pub mode: CheckInMode,
    pub context: OccupantContext,
    pub expected_generation: Option<u64>,
    pub event_id: String,
    pub payload_version: u32,
    pub payload: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckInResponse {
    pub context: OccupantContext,
    pub historical: bool,
    pub output: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextError {
    LifecycleRequired,
    Corrupt,
    WrongInstance,
    Conflict,
    Child,
    TooLarge,
    UnsafePath,
    LockTimeout,
    Invalid,
    /// The installed harness version selects no adapter recipe. The message
    /// is the registry's actionable refusal naming the supported recipes.
    UnsupportedVersion(String),
    Io(String),
    Dispatch(String),
}
impl From<std::io::Error> for ContextError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
/// Inject only CheckIn. The dispatcher must bound its own transport deadline.
pub trait CheckInDispatcher {
    fn check_in(&mut self, request: &PendingCheckIn) -> Result<CheckInResponse, ContextError>;
}
impl<F> CheckInDispatcher for F
where
    F: FnMut(&PendingCheckIn) -> Result<CheckInResponse, ContextError>,
{
    fn check_in(&mut self, p: &PendingCheckIn) -> Result<CheckInResponse, ContextError> {
        self(p)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Completed {
    request: PendingCheckIn,
    response: CheckInResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at_millis: Option<i64>,
}
/// A pending request that received a definitive, non-retryable rejection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Abandoned {
    event_id: String,
    operation_id: Uuid,
}
/// Attention token last delivered to one execution (the join of every token
/// it was shown). Last writer wins; deliberately outside the journal and never
/// replayed. Version 1 (the retired client-side frontier) reads as absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionMark {
    version: u32,
    execution: Uuid,
    token: crate::protocol::attention::AttentionToken,
}
const ATTENTION_MARK_VERSION: u32 = 2;
/// The person's `me init --operator` override for one human execution
/// (TRUST-POLICY A4): recorded only after the daemon accepted the operator
/// check-in; a client-local hint that skips the advisory agent-evidence
/// checks, never authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperatorMark {
    version: u32,
    execution: Uuid,
}
const OPERATOR_MARK_VERSION: u32 = 1;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    instance: Uuid,
    seat: String,
    current: Option<OccupantContext>,
    pending: Option<PendingCheckIn>,
    completed: Vec<Completed>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    abandoned: Vec<Abandoned>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    observation_watermarks: Vec<ObservationWatermark>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    declared_resets: Vec<DeclaredResetHint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    prepared_kinds: Vec<PreparedKind>,
}
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
/// Count and age retention. The newest completion always survives age pruning.
fn prune(state: &mut State, now: i64) {
    let newest = state.completed.len().saturating_sub(1);
    let mut index = 0;
    state.completed.retain(|done| {
        let keep = index == newest
            || done
                .completed_at_millis
                .is_none_or(|at| now.saturating_sub(at) <= RETAIN_AGE_MILLIS);
        index += 1;
        keep
    });
    let excess = state.completed.len().saturating_sub(RETAIN_COMPLETED);
    state.completed.drain(..excess);
    let excess = state.abandoned.len().saturating_sub(RETAIN_ABANDONED);
    state.abandoned.drain(..excess);
    prune_kinds(state);
}
fn prune_kinds(state: &mut State) {
    let retained: Vec<_> = state
        .completed
        .iter()
        .map(|d| (d.request.event_id.clone(), d.request.operation_id))
        .chain(
            state
                .pending
                .iter()
                .map(|p| (p.event_id.clone(), p.operation_id)),
        )
        .collect();
    state.prepared_kinds.retain(|saved| {
        retained
            .iter()
            .any(|(event, operation)| *event == saved.event_id && *operation == saved.operation_id)
    });
}
pub struct ContextJournal {
    directory: PathBuf,
    instance: Uuid,
    seat: String,
    lock_timeout: Duration,
}
impl ContextJournal {
    /// Directory must already exist, be private, and contain no symlink ancestors.
    pub fn open(
        directory: &Path,
        instance: Uuid,
        seat: &str,
        lock_timeout: Duration,
    ) -> Result<Self, ContextError> {
        if instance.is_nil() || !valid_text(seat) || lock_timeout > Duration::from_secs(10) {
            return Err(ContextError::Invalid);
        }
        if !directory.is_absolute() {
            return Err(ContextError::UnsafePath);
        }
        check_ancestors(directory)?;
        let directory = directory.canonicalize()?;
        check_ancestors(&directory)?;
        private_metadata(&directory, true)?;
        Ok(Self {
            directory,
            instance,
            seat: seat.into(),
            lock_timeout,
        })
    }
    fn lock(&self) -> Result<File, ContextError> {
        private_metadata(&self.directory, true)?;
        let path = self.directory.join("context.lock");
        if path.try_exists()? {
            private_metadata(&path, false)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        secure_options(&mut options);
        let file = options.open(path)?;
        let start = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if start.elapsed() >= self.lock_timeout {
                        return Err(ContextError::LockTimeout);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
            }
        }
    }
    fn load(&self) -> Result<State, ContextError> {
        let path = self.directory.join("context.json");
        if !path.try_exists()? {
            if fs::symlink_metadata(&path).is_ok() {
                return Err(ContextError::UnsafePath);
            }
            return Ok(State {
                version: 1,
                instance: self.instance,
                seat: self.seat.clone(),
                current: None,
                pending: None,
                completed: Vec::new(),
                abandoned: Vec::new(),
                observation_watermarks: Vec::new(),
                declared_resets: Vec::new(),
                prepared_kinds: Vec::new(),
            });
        }
        private_metadata(&path, false)?;
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let mut bytes = Vec::new();
        options
            .open(path)?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(ContextError::TooLarge);
        }
        let state: State = serde_json::from_slice(&bytes).map_err(|_| ContextError::Corrupt)?;
        if state.instance != self.instance {
            return Err(ContextError::WrongInstance);
        }
        if state.version != 1
            || state.seat != self.seat
            || state.completed.len() > MAX_HISTORY
            || state.abandoned.len() > MAX_HISTORY
            || state.declared_resets.len() > 32
            || state.declared_resets.iter().any(|r| {
                !valid_text(&r.target)
                    || !valid_text(&r.session)
                    || !valid_text(&r.event_key)
                    || r.process_nonce.is_nil()
                    || r.sequence == 0
                    || r.generation == 0
                    || r.observed_at_millis < 0
            })
            || state.observation_watermarks.len() > RETAIN_COMPLETED
            || state
                .observation_watermarks
                .iter()
                .any(|w| w.process_nonce.is_nil() || w.sequence == 0 || w.observed_at_millis < 0)
        {
            return Err(ContextError::Corrupt);
        }
        if state.prepared_kinds.len() > MAX_HISTORY + 1 {
            return Err(ContextError::Corrupt);
        }
        for (index, saved) in state.prepared_kinds.iter().enumerate() {
            let request =
                Self::request_in_state(&state, &saved.event_id).ok_or(ContextError::Corrupt)?;
            if saved.operation_id != request.operation_id
                || saved.request_digest != request_digest(request, saved.kind)?
                || saved.kind.mode() != request.mode
                || !matches!(
                    saved.kind,
                    EventKind::Startup | EventKind::Clear | EventKind::Tool
                )
                || state.prepared_kinds[..index].iter().any(|other| {
                    other.event_id == saved.event_id || other.operation_id == saved.operation_id
                })
            {
                return Err(ContextError::Corrupt);
            }
        }
        if let Some(c) = &state.current {
            self.validate_context(c)
                .map_err(|_| ContextError::Corrupt)?;
        }
        if let Some(p) = &state.pending {
            self.validate_request(p)
                .map_err(|_| ContextError::Corrupt)?;
        }
        for done in &state.completed {
            self.validate_request(&done.request)
                .map_err(|_| ContextError::Corrupt)?;
            self.validate_context(&done.response.context)
                .map_err(|_| ContextError::Corrupt)?;
        }
        Ok(state)
    }
    fn save(&self, state: &State) -> Result<(), ContextError> {
        let bytes = serde_json::to_vec(state).map_err(|_| ContextError::Corrupt)?;
        if bytes.len() > MAX_BYTES {
            return Err(ContextError::TooLarge);
        }
        let temp = self
            .directory
            .join(format!(".context-{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        secure_options(&mut options);
        let operation = (|| {
            let mut f = options.open(&temp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            let path = self.directory.join("context.json");
            if fs::symlink_metadata(&path).is_ok() {
                private_metadata(&path, false)?;
            }
            fs::rename(&temp, path)?;
            File::open(&self.directory)?.sync_all()?;
            Ok(())
        })();
        if operation.is_err() {
            let _ = fs::remove_file(temp);
        }
        operation
    }
    fn validate_context(&self, c: &OccupantContext) -> Result<(), ContextError> {
        if c.instance != self.instance {
            return Err(ContextError::WrongInstance);
        }
        if c.role != Role::TopLevel {
            return Err(ContextError::Child);
        }
        if c.format_version != 1
            || c.execution.is_nil()
            || c.seat != self.seat
            || !valid_text(&c.target)
            || matches!(&c.session,SessionReference::Native(s) if !valid_text(s))
            || matches!(&c.session,SessionReference::PluginContext(u) if u.is_nil())
        {
            return Err(ContextError::Invalid);
        }
        Ok(())
    }
    fn validate_request(&self, p: &PendingCheckIn) -> Result<(), ContextError> {
        self.validate_context(&p.context)?;
        if p.payload.len() > MAX_PAYLOAD {
            return Err(ContextError::TooLarge);
        }
        if p.operation_id.is_nil()
            || !valid_text(&p.event_id)
            || p.payload_version != 1
            || p.payload.is_empty()
            || (p.mode == CheckInMode::Current && p.expected_generation.is_some())
        {
            return Err(ContextError::Invalid);
        }
        Ok(())
    }
    /// Read one completed CheckIn from existing private files without creating or repairing them.
    pub fn read_completed(
        &self,
        key: &CachedCheckInKey,
        budget: &CallBudget,
        clock: &dyn Clock,
    ) -> Result<CachedCheckIn, CacheReadError> {
        let check_budget = || -> Result<(), CacheReadError> {
            if budget.cancellation.is_cancelled() {
                return Err(CacheReadError::Cancelled);
            }
            if clock.monotonic_now() >= budget.deadline {
                return Err(CacheReadError::DeadlineExceeded);
            }
            Ok(())
        };
        check_budget()?;
        if key.instance != self.instance
            || key.seat != self.seat
            || key.event_id.is_empty()
            || key.operation_id.is_nil()
        {
            return Err(CacheReadError::ReferenceMismatch);
        }
        private_metadata(&self.directory, true)?;
        check_ancestors(&self.directory)?;
        let lock_path = self.directory.join("context.lock");
        match fs::symlink_metadata(&lock_path) {
            Ok(_) => private_metadata(&lock_path, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CacheReadError::NotFound);
            }
            Err(error) => return Err(ContextError::from(error).into()),
        }
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let lock = options.open(&lock_path)?;
        if !lock.metadata()?.is_file() {
            return Err(ContextError::UnsafePath.into());
        }
        #[cfg(unix)]
        if lock.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(ContextError::UnsafePath.into());
        }
        let started = Instant::now();
        loop {
            check_budget()?;
            match lock.try_lock_shared() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    if started.elapsed() >= self.lock_timeout.min(Duration::from_secs(1)) {
                        return Err(ContextError::LockTimeout.into());
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(ContextError::from(error).into());
                }
            }
        }
        let path = self.directory.join("context.json");
        match fs::symlink_metadata(&path) {
            Ok(_) => private_metadata(&path, false)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(CacheReadError::NotFound);
            }
            Err(error) => return Err(ContextError::from(error).into()),
        }
        let mut file = options.open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(ContextError::UnsafePath.into());
        }
        #[cfg(unix)]
        if file.metadata()?.permissions().mode() & 0o077 != 0 {
            return Err(ContextError::UnsafePath.into());
        }
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            check_budget()?;
            let count = file.read(&mut chunk)?;
            if count == 0 {
                break;
            }
            if bytes.len() + count > MAX_BYTES {
                return Err(ContextError::TooLarge.into());
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        check_budget()?;
        let state: State = serde_json::from_slice(&bytes).map_err(|_| ContextError::Corrupt)?;
        if state.instance != self.instance {
            return Err(ContextError::WrongInstance.into());
        }
        if state.version != 1 || state.seat != self.seat || state.completed.len() > MAX_HISTORY {
            return Err(ContextError::Corrupt.into());
        }
        if let Some(current) = &state.current {
            self.validate_context(current)
                .map_err(|_| ContextError::Corrupt)?;
        }
        if let Some(pending) = &state.pending {
            self.validate_request(pending)
                .map_err(|_| ContextError::Corrupt)?;
        }
        let mut found = None;
        let mut relevant = 0usize;
        for done in &state.completed {
            check_budget()?;
            self.validate_request(&done.request)
                .map_err(|_| ContextError::Corrupt)?;
            self.validate_context(&done.response.context)
                .map_err(|_| ContextError::Corrupt)?;
            if !done.response.historical {
                let requested = &done.request.context;
                let returned = &done.response.context;
                if requested.execution != returned.execution
                    || requested.target != returned.target
                    || requested.harness != returned.harness
                    || requested.session != returned.session
                    || (done.request.mode == CheckInMode::Current
                        && requested.binding_generation != returned.binding_generation)
                    || (done.request.mode == CheckInMode::Lifecycle
                        && returned.binding_generation
                            <= done.request.expected_generation.unwrap_or(0))
                {
                    return Err(ContextError::Corrupt.into());
                }
            }
            if done.response.output.len() > MAX_PAYLOAD
                || std::str::from_utf8(&done.response.output).is_err()
            {
                return Err(ContextError::Corrupt.into());
            }
            if done.request.event_id == key.event_id
                || done.request.operation_id == key.operation_id
            {
                relevant += 1;
                if relevant > 1 {
                    return Err(ContextError::Corrupt.into());
                }
                if done.request.event_id == key.event_id
                    && done.request.operation_id == key.operation_id
                {
                    found = Some(CachedCheckIn {
                        request: done.request.clone(),
                        response: done.response.clone(),
                    });
                }
            }
        }
        check_budget()?;
        found.ok_or(CacheReadError::NotFound)
    }
    pub fn current(&self) -> Result<Option<OccupantContext>, ContextError> {
        let _lock = self.lock()?;
        Ok(self.load()?.current)
    }
    /// The current context read without taking `context.lock`, for display
    /// paths that must never wait on (or delay) a check-in. `save` replaces
    /// `context.json` by atomic rename, so this sees one whole version: the
    /// one before or the one after a concurrent write, never a torn file.
    /// Not for decisions: it can be one write behind.
    pub fn current_snapshot(&self) -> Result<Option<OccupantContext>, ContextError> {
        Ok(self.load()?.current)
    }
    pub fn pending(&self) -> Result<Option<PendingCheckIn>, ContextError> {
        let _lock = self.lock()?;
        Ok(self.load()?.pending)
    }
    /// Recover the original immutable request from pending or completed event state.
    pub fn request_for_event(
        &self,
        event_id: &str,
    ) -> Result<Option<PendingCheckIn>, ContextError> {
        if !valid_text(event_id) {
            return Err(ContextError::Invalid);
        }
        let _lock = self.lock()?;
        let state = self.load()?;
        Ok(Self::request_in_state(&state, event_id).cloned())
    }
    pub fn prepared_kind_for_event(&self, event_id: &str) -> Result<EventKind, ContextError> {
        if !valid_text(event_id) {
            return Err(ContextError::Invalid);
        }
        let _lock = self.lock()?;
        let state = self.load()?;
        let request = Self::request_in_state(&state, event_id).ok_or(ContextError::Invalid)?;
        state
            .prepared_kinds
            .iter()
            .find(|saved| {
                saved.event_id == event_id
                    && saved.operation_id == request.operation_id
                    && saved.request_digest
                        == request_digest(request, saved.kind).unwrap_or_default()
            })
            .map(|saved| saved.kind)
            .ok_or(ContextError::Invalid)
    }
    /// Read an immutable completed CheckIn even after its delivery intent was
    /// removed by a successful output flush.
    pub fn completed_for_event(
        &self,
        event_id: &str,
    ) -> Result<Option<(PendingCheckIn, CheckInResponse)>, ContextError> {
        if !valid_text(event_id) {
            return Err(ContextError::Invalid);
        }
        let _lock = self.lock()?;
        let state = self.load()?;
        Ok(state
            .completed
            .iter()
            .find(|done| done.request.event_id == event_id)
            .map(|done| (done.request.clone(), done.response.clone())))
    }
    /// Invoke a bounded local intent factory only when this event has no cached request.
    /// Lock order is context -> external CLI journal; never enter with a CLI lock held.
    /// The factory must recover any existing CLI intent left by failure before context save.
    pub fn get_or_prepare(
        &self,
        event_id: &str,
        factory: impl FnOnce(Option<&OccupantContext>) -> Result<PendingCheckIn, ContextError>,
    ) -> Result<PendingCheckIn, ContextError> {
        if !valid_text(event_id) {
            return Err(ContextError::Invalid);
        }
        let _lock = self.lock()?;
        let mut state = self.load()?;
        if let Some(request) = Self::request_in_state(&state, event_id) {
            return Ok(request.clone());
        }
        if state.pending.is_some() || state.abandoned.iter().any(|a| a.event_id == event_id) {
            return Err(ContextError::Conflict);
        }
        prune(&mut state, now_millis());
        if state.completed.len() >= MAX_HISTORY {
            return Err(ContextError::TooLarge);
        }
        let request = factory(state.current.as_ref())?;
        if request.event_id != event_id {
            return Err(ContextError::Conflict);
        }
        self.validate_request(&request)?;
        self.prepare_locked(&mut state, request.clone())?;
        Ok(request)
    }
    /// Open only existing private state; an observer never creates a context.
    pub fn open_existing(
        directory: &Path,
        instance: Uuid,
        timeout: Duration,
    ) -> Result<Option<Self>, ContextError> {
        private_metadata(directory, true)?;
        check_ancestors(directory)?;
        let path = directory.join("context.json");
        if !path.try_exists()? {
            return Ok(None);
        }
        private_metadata(&path, false)?;
        private_metadata(&directory.join("context.lock"), false)?;
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let mut bytes = Vec::new();
        options
            .open(path)?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_BYTES {
            return Err(ContextError::TooLarge);
        }
        let state: State = serde_json::from_slice(&bytes).map_err(|_| ContextError::Corrupt)?;
        let journal = Self::open(directory, instance, &state.seat, timeout)?;
        journal.load()?;
        Ok(Some(journal))
    }
    /// Cooperative metadata only. No predecessor, registration or authority is
    /// inferred here; qualified preparation resolves it under this same lock.
    pub fn record_declared_reset(
        &self,
        harness: Harness,
        target: &str,
        reset: &DeclaredReset,
        now: i64,
    ) -> Result<bool, ContextError> {
        if qualified_policy(harness) != QualifiedTurnPolicy::StartupAttach {
            return Ok(false);
        }
        if !valid_text(target) || !valid_text(&reset.session) || !valid_text(&reset.event_key) {
            return Err(ContextError::Invalid);
        }
        reset.ordering.validate_deadline(now)?;
        let _lock = self.lock()?;
        let mut state = self.load()?;
        let Some(current) = &state.current else {
            return Ok(false);
        };
        if current.harness != harness
            || current.target != target
            || current.binding_generation == 0
            || state.pending.is_some()
        {
            return Ok(false);
        }
        state
            .declared_resets
            .retain(|r| now.saturating_sub(r.observed_at_millis) <= RESET_HINT_TTL);
        if state.declared_resets.iter().any(|r| {
            r.event_key == reset.event_key
                || (r.harness == harness
                    && r.target == target
                    && r.observed_at_millis > reset.ordering.observed_at_millis)
        }) || state.observation_watermarks.iter().any(|w| {
            w.harness == harness
                && w.process_nonce == reset.ordering.process_nonce
                && reset.ordering.sequence <= w.sequence
        }) {
            return Ok(false);
        }
        if state.declared_resets.len() >= 32 {
            return Err(ContextError::TooLarge);
        }
        state.declared_resets.push(DeclaredResetHint {
            harness,
            target: target.into(),
            session: reset.session.clone(),
            event_key: reset.event_key.clone(),
            process_nonce: reset.ordering.process_nonce,
            sequence: reset.ordering.sequence,
            observed_at_millis: reset.ordering.observed_at_millis,
            generation: current.binding_generation,
            consumed: false,
        });
        state.observation_watermarks.retain(|w| {
            !(w.harness == harness && w.process_nonce == reset.ordering.process_nonce)
                && now.saturating_sub(w.observed_at_millis) <= RETAIN_AGE_MILLIS
        });
        state.observation_watermarks.push(ObservationWatermark {
            harness,
            process_nonce: reset.ordering.process_nonce,
            sequence: reset.ordering.sequence,
            observed_at_millis: reset.ordering.observed_at_millis,
        });
        let excess = state
            .observation_watermarks
            .len()
            .saturating_sub(RETAIN_COMPLETED);
        state.observation_watermarks.drain(..excess);
        prune_kinds(&mut state);
        self.save(&state)?;
        Ok(true)
    }
    /// Exact replay wins; selection and the bounded ordering watermark are saved
    /// together with the immutable request under the existing context lock.
    pub fn get_or_prepare_qualified(
        &self,
        harness: Harness,
        target: &str,
        turn: &QualifiedTurn,
        now: i64,
        factory: impl FnOnce(
            Option<&OccupantContext>,
            EventKind,
        ) -> Result<PendingCheckIn, ContextError>,
    ) -> Result<PendingCheckIn, ContextError> {
        if !valid_text(&turn.session)
            || !valid_text(&turn.event_key)
            || !valid_text(target)
            || matches!(
                harness.occupant(),
                crate::harness::registry::OccupantHarness::Human
            )
            || turn.reset.as_ref().is_some_and(|r| {
                !valid_text(&r.previous_session) || r.previous_session == turn.session
            })
        {
            return Err(ContextError::Invalid);
        }
        let _lock = self.lock()?;
        let mut state = self.load()?;
        if let Some(request) = Self::request_in_state(&state, &turn.event_key) {
            if request.context.harness != harness
                || request.context.target != target
                || request.context.session != SessionReference::Native(turn.session.clone())
            {
                return Err(ContextError::Conflict);
            }
            return Ok(request.clone());
        }
        if state.pending.is_some() || state.abandoned.iter().any(|a| a.event_id == turn.event_key) {
            return Err(ContextError::Conflict);
        }
        if let Some(order) = &turn.ordering {
            order.validate_deadline(now)?;
            if state.observation_watermarks.iter().any(|w| {
                w.harness == harness
                    && w.process_nonce == order.process_nonce
                    && order.sequence <= w.sequence
            }) {
                return Err(ContextError::Conflict);
            }
        }
        let reset_hint = state.declared_resets.iter().position(|r| {
            !r.consumed
                && r.harness == harness
                && r.target == target
                && r.session == turn.session
                && now.saturating_sub(r.observed_at_millis) <= RESET_HINT_TTL
                && state.current.as_ref().is_some_and(|c| {
                    c.binding_generation == r.generation
                        && c.session != SessionReference::Native(turn.session.clone())
                })
                && turn.ordering.as_ref().is_some_and(|o| {
                    o.observed_at_millis >= r.observed_at_millis
                        && (o.process_nonce != r.process_nonce || o.sequence > r.sequence)
                })
        });
        let kind = match state.current.as_ref() {
            None => EventKind::Startup,
            Some(current) => {
                if current.harness != harness || current.target != target {
                    return Err(ContextError::Conflict);
                }
                if current.session == SessionReference::Native(turn.session.clone()) {
                    if turn.reset.is_some() || current.binding_generation == 0 {
                        return Err(ContextError::Conflict);
                    }
                    EventKind::Tool
                } else if reset_hint.is_some() {
                    EventKind::Clear
                } else if let Some(reset) = &turn.reset {
                    if turn.ordering.is_none()
                        || current.session
                            != SessionReference::Native(reset.previous_session.clone())
                    {
                        return Err(ContextError::Conflict);
                    }
                    EventKind::Clear
                } else if qualified_policy(harness) == QualifiedTurnPolicy::StartupAttach {
                    EventKind::Startup
                } else {
                    return Err(ContextError::Conflict);
                }
            }
        };
        let request = factory(state.current.as_ref(), kind)?;
        if request.event_id != turn.event_key
            || request.mode != kind.mode()
            || request.context.harness != harness
            || request.context.target != target
            || request.context.session != SessionReference::Native(turn.session.clone())
        {
            return Err(ContextError::Conflict);
        }
        self.validate_request(&request)?;
        if let Some(order) = &turn.ordering {
            state.observation_watermarks.retain(|w| {
                !(w.harness == harness && w.process_nonce == order.process_nonce)
                    && now.saturating_sub(w.observed_at_millis) <= RETAIN_AGE_MILLIS
            });
            state.observation_watermarks.push(ObservationWatermark {
                harness,
                process_nonce: order.process_nonce,
                sequence: order.sequence,
                observed_at_millis: order.observed_at_millis,
            });
            let excess = state
                .observation_watermarks
                .len()
                .saturating_sub(RETAIN_COMPLETED);
            state.observation_watermarks.drain(..excess);
        }
        if let Some(index) = reset_hint {
            state.declared_resets[index].consumed = true;
        }
        state
            .declared_resets
            .retain(|r| now.saturating_sub(r.observed_at_millis) <= RESET_HINT_TTL);
        self.prepare_locked_kind(&mut state, request.clone(), Some(kind))?;
        Ok(request)
    }
    fn request_in_state<'a>(state: &'a State, event_id: &str) -> Option<&'a PendingCheckIn> {
        state
            .completed
            .iter()
            .find(|done| done.request.event_id == event_id)
            .map(|done| &done.request)
            .or_else(|| {
                state
                    .pending
                    .as_ref()
                    .filter(|request| request.event_id == event_id)
            })
    }
    /// Save caller's immutable operation key and payload before any CheckIn dispatch.
    pub fn prepare(&self, request: PendingCheckIn) -> Result<(), ContextError> {
        self.validate_request(&request)?;
        let _lock = self.lock()?;
        let mut state = self.load()?;
        self.prepare_locked(&mut state, request)
    }
    fn prepare_locked(
        &self,
        state: &mut State,
        request: PendingCheckIn,
    ) -> Result<(), ContextError> {
        self.prepare_locked_kind(state, request, None)
    }
    fn prepare_locked_kind(
        &self,
        state: &mut State,
        request: PendingCheckIn,
        kind: Option<EventKind>,
    ) -> Result<(), ContextError> {
        if let Some(done) = state.completed.iter().find(|d| {
            d.request.event_id == request.event_id || d.request.operation_id == request.operation_id
        }) {
            return if done.request == request {
                Ok(())
            } else {
                Err(ContextError::Conflict)
            };
        }
        if let Some(p) = &state.pending {
            return if p == &request {
                Ok(())
            } else {
                Err(ContextError::Conflict)
            };
        }
        if state
            .abandoned
            .iter()
            .any(|a| a.event_id == request.event_id || a.operation_id == request.operation_id)
        {
            return Err(ContextError::Conflict);
        }
        prune(state, now_millis());
        if state.completed.len() >= MAX_HISTORY {
            return Err(ContextError::TooLarge);
        }
        match request.mode {
            CheckInMode::Current => {
                if state.current.as_ref() != Some(&request.context) {
                    return Err(if state.current.is_none() {
                        ContextError::LifecycleRequired
                    } else {
                        ContextError::Conflict
                    });
                }
            }
            CheckInMode::Lifecycle => {
                // A lifecycle registration continues the local generation or,
                // when the service mapping has since advanced (host
                // invalidation and reconfirmation, operator repair), CASes
                // the service's newer generation. It never targets an older
                // one.
                let local = state.current.as_ref().map(|c| c.binding_generation);
                if local.is_some_and(|local| {
                    request
                        .expected_generation
                        .is_none_or(|expected| expected < local)
                }) {
                    return Err(ContextError::Conflict);
                }
                // A person's request over an agent's current context is
                // admitted without touching `current`: the daemon decides
                // (TRUST-POLICY A4) and `dispatch` replaces `current` only on
                // success.
                let person_over_agent = request.context.harness == Harness::Human;
                if state.current.as_ref().is_some_and(|c| {
                    c.execution == request.context.execution
                        || c.target != request.context.target
                        || (c.harness != request.context.harness
                            && !(person_over_agent && c.harness != Harness::Human))
                }) || state
                    .completed
                    .iter()
                    .any(|d| d.request.context.execution == request.context.execution)
                {
                    return Err(ContextError::Conflict);
                }
            }
        }
        if let Some(kind) = kind {
            state.prepared_kinds.push(PreparedKind {
                event_id: request.event_id.clone(),
                operation_id: request.operation_id,
                request_digest: request_digest(&request, kind)?,
                kind,
            });
        }
        state.pending = Some(request);
        prune_kinds(state);
        // Reserve worst-case JSON byte-array output plus a complete returned
        // context, dropping the oldest completions (size retention) to make room.
        loop {
            let size = serde_json::to_vec(&*state)
                .map_err(|_| ContextError::Corrupt)?
                .len();
            if size + MAX_PAYLOAD * 4 + 32768 <= MAX_BYTES {
                break;
            }
            if state.completed.is_empty() {
                return Err(ContextError::TooLarge);
            }
            state.completed.remove(0);
            prune_kinds(state);
        }
        self.save(state)
    }
    /// Keep the bounded context lock through dispatch and atomic completion publication.
    pub fn dispatch(
        &self,
        event_id: &str,
        dispatcher: &mut impl CheckInDispatcher,
    ) -> Result<CheckInResponse, ContextError> {
        let _lock = self.lock()?;
        let mut state = self.load()?;
        if let Some(done) = state
            .completed
            .iter()
            .find(|d| d.request.event_id == event_id)
        {
            return Ok(done.response.clone());
        }
        let request = state
            .pending
            .clone()
            .ok_or(ContextError::LifecycleRequired)?;
        if request.event_id != event_id {
            return Err(ContextError::Conflict);
        }
        let response = dispatcher.check_in(&request)?;
        self.validate_context(&response.context)?;
        if response.output.len() > MAX_PAYLOAD {
            return Err(ContextError::TooLarge);
        }
        if !response.historical {
            let a = &request.context;
            let b = &response.context;
            if a.execution != b.execution
                || a.target != b.target
                || a.harness != b.harness
                || a.session != b.session
                || (request.mode == CheckInMode::Current
                    && a.binding_generation != b.binding_generation)
                || (request.mode == CheckInMode::Lifecycle
                    && b.binding_generation <= request.expected_generation.unwrap_or(0))
            {
                return Err(ContextError::Conflict);
            }
            state.current = Some(response.context.clone());
        }
        state.completed.push(Completed {
            request,
            response: response.clone(),
            completed_at_millis: Some(now_millis()),
        });
        state.pending = None;
        prune(&mut state, now_millis());
        prune_kinds(&mut state);
        self.save(&state)?;
        Ok(response)
    }
    /// Number of retained completed entries.
    pub fn completed_len(&self) -> Result<usize, ContextError> {
        let _lock = self.lock()?;
        Ok(self.load()?.completed.len())
    }
    /// Terminal path for a pending request whose dispatch received a
    /// definitive, non-retryable rejection. Its key is recorded so it is never
    /// re-prepared, and `pending` is cleared so a fresh lifecycle event can
    /// prepare against the current generation. Returns the abandoned request,
    /// or `None` when `event_id` is not the pending request.
    pub fn abandon_pending(&self, event_id: &str) -> Result<Option<PendingCheckIn>, ContextError> {
        let _lock = self.lock()?;
        let mut state = self.load()?;
        let Some(pending) = state.pending.clone().filter(|p| p.event_id == event_id) else {
            return Ok(None);
        };
        state.abandoned.push(Abandoned {
            event_id: pending.event_id.clone(),
            operation_id: pending.operation_id,
        });
        state.pending = None;
        prune(&mut state, now_millis());
        prune_kinds(&mut state);
        self.save(&state)?;
        Ok(Some(pending))
    }
    /// TRUST-POLICY C1: install the context a continuity reattachment returned
    /// for this seat, replacing whatever was saved (a context for another pane
    /// or an older generation is historical once the service reattached the
    /// seat). A request still pending for the replaced context is moved to
    /// `abandoned` and returned so the caller can complete its intent.
    pub fn install_reattached(
        &self,
        context: OccupantContext,
    ) -> Result<Option<PendingCheckIn>, ContextError> {
        self.validate_context(&context)?;
        let _lock = self.lock()?;
        let mut state = self.load()?;
        let abandoned = state.pending.take();
        if let Some(pending) = &abandoned {
            state.abandoned.push(Abandoned {
                event_id: pending.event_id.clone(),
                operation_id: pending.operation_id,
            });
        }
        state.current = Some(context);
        prune(&mut state, now_millis());
        prune_kinds(&mut state);
        self.save(&state)?;
        Ok(abandoned)
    }
    /// Clear `current` when the service has moved the seat past it. Only the
    /// exact context the caller observed is cleared, and never while a request
    /// is pending. Completed entries stay for exact replay.
    pub fn retire_current(&self, observed: &OccupantContext) -> Result<bool, ContextError> {
        let _lock = self.lock()?;
        let mut state = self.load()?;
        if state.pending.is_some() || state.current.as_ref() != Some(observed) {
            return Ok(false);
        }
        state.current = None;
        prune_kinds(&mut state);
        self.save(&state)?;
        Ok(true)
    }
    /// Attention token last delivered to `execution`; `None` when absent,
    /// unreadable, of another version or recorded for another execution.
    pub fn attention_mark(
        &self,
        execution: Uuid,
    ) -> Option<crate::protocol::attention::AttentionToken> {
        let path = self.directory.join("attention.json");
        private_metadata(&path, false).ok()?;
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let mut bytes = Vec::new();
        options
            .open(path)
            .ok()?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        let mark: AttentionMark = serde_json::from_slice(&bytes).ok()?;
        (mark.version == ATTENTION_MARK_VERSION && mark.execution == execution)
            .then_some(mark.token)
    }
    /// Execution the person recorded with `me init --operator`; `None` when
    /// absent, unreadable or of another version.
    pub fn operator_mark(&self) -> Option<Uuid> {
        let path = self.directory.join("operator.json");
        private_metadata(&path, false).ok()?;
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let mut bytes = Vec::new();
        options
            .open(path)
            .ok()?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        let mark: OperatorMark = serde_json::from_slice(&bytes).ok()?;
        (mark.version == OPERATOR_MARK_VERSION).then_some(mark.execution)
    }
    /// Record the operator override for `execution` atomically (last writer
    /// wins); it matches only that execution.
    pub fn set_operator_mark(&self, execution: Uuid) -> Result<(), ContextError> {
        if execution.is_nil() {
            return Err(ContextError::Invalid);
        }
        let bytes = serde_json::to_vec(&OperatorMark {
            version: OPERATOR_MARK_VERSION,
            execution,
        })
        .map_err(|_| ContextError::Invalid)?;
        private_metadata(&self.directory, true)?;
        let temp = self
            .directory
            .join(format!(".operator-{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        secure_options(&mut options);
        let operation = (|| {
            let mut f = options.open(&temp)?;
            f.write_all(&bytes)?;
            let path = self.directory.join("operator.json");
            if fs::symlink_metadata(&path).is_ok() {
                private_metadata(&path, false)?;
            }
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if operation.is_err() {
            let _ = fs::remove_file(temp);
        }
        operation
    }
    /// Replace the attention mark atomically (last writer wins).
    pub fn set_attention_mark(
        &self,
        execution: Uuid,
        token: &crate::protocol::attention::AttentionToken,
    ) -> Result<(), ContextError> {
        if execution.is_nil() {
            return Err(ContextError::Invalid);
        }
        let bytes = serde_json::to_vec(&AttentionMark {
            version: ATTENTION_MARK_VERSION,
            execution,
            token: *token,
        })
        .map_err(|_| ContextError::Invalid)?;
        if bytes.len() > MAX_BYTES {
            return Err(ContextError::TooLarge);
        }
        private_metadata(&self.directory, true)?;
        let temp = self
            .directory
            .join(format!(".attention-{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        secure_options(&mut options);
        let operation = (|| {
            let mut f = options.open(&temp)?;
            f.write_all(&bytes)?;
            let path = self.directory.join("attention.json");
            if fs::symlink_metadata(&path).is_ok() {
                private_metadata(&path, false)?;
            }
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if operation.is_err() {
            let _ = fs::remove_file(temp);
        }
        operation
    }
}
fn valid_text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control)
}
fn check_ancestors(path: &Path) -> Result<(), ContextError> {
    for parent in path.ancestors() {
        if fs::symlink_metadata(parent)?.file_type().is_symlink() {
            return Err(ContextError::UnsafePath);
        }
    }
    Ok(())
}
fn private_metadata(path: &Path, directory: bool) -> Result<(), ContextError> {
    let m = fs::symlink_metadata(path)?;
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err(ContextError::UnsafePath);
    }
    #[cfg(unix)]
    if m.permissions().mode() & 0o077 != 0 {
        return Err(ContextError::UnsafePath);
    }
    Ok(())
}
fn secure_options(options: &mut OpenOptions) {
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    // The flag is per-architecture on Linux (0x20000 on x86_64, 0x8000 on
    // aarch64); a hardcoded value silently followed links on one of them.
    #[test]
    fn secure_options_refuse_a_symlinked_final_component() {
        let dir = std::env::temp_dir().join(format!("ht-nofollow-{}", Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("target"), b"x").unwrap();
        std::os::unix::fs::symlink(dir.join("target"), dir.join("link")).unwrap();
        let mut options = OpenOptions::new();
        options.read(true);
        secure_options(&mut options);
        let error = options.open(dir.join("link")).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
        assert!(options.open(dir.join("target")).is_ok());
        fs::remove_dir_all(dir).unwrap();
    }
    fn fixture() -> PendingCheckIn {
        PendingCheckIn {
            operation_id: Uuid::from_u128(2),
            mode: CheckInMode::Current,
            context: OccupantContext {
                format_version: 1,
                instance: Uuid::from_u128(1),
                seat: "seat".into(),
                target: "target".into(),
                harness: Harness::Codex,
                binding_generation: 1,
                execution: Uuid::from_u128(3),
                session: SessionReference::PluginContext(Uuid::from_u128(3)),
                role: Role::TopLevel,
            },
            expected_generation: None,
            event_id: "event".into(),
            payload_version: 1,
            payload: b"frozen request".to_vec(),
        }
    }
    #[test]
    fn legacy_identity_byte_snapshots() {
        let request = fixture();
        let saved = crate::harness::cache::CachedCheckIn {
            request: request.clone(),
            response: CheckInResponse {
                context: request.context.clone(),
                historical: false,
                output: b"offer".to_vec(),
            },
        };
        let reference = crate::harness::cache::cache_reference(
            &saved,
            &crate::protocol::output::OutputSpec::default().context,
        )
        .unwrap();
        assert_eq!(serde_json::to_vec(&request.context).unwrap(), br#"{"format_version":1,"instance":"00000000-0000-0000-0000-000000000001","seat":"seat","target":"target","harness":"Codex","binding_generation":1,"execution":"00000000-0000-0000-0000-000000000003","session":{"PluginContext":"00000000-0000-0000-0000-000000000003"},"role":"TopLevel"}"#);
        assert_eq!(serde_json::to_vec(&request).unwrap(), br#"{"operation_id":"00000000-0000-0000-0000-000000000002","mode":"Current","context":{"format_version":1,"instance":"00000000-0000-0000-0000-000000000001","seat":"seat","target":"target","harness":"Codex","binding_generation":1,"execution":"00000000-0000-0000-0000-000000000003","session":{"PluginContext":"00000000-0000-0000-0000-000000000003"},"role":"TopLevel"},"expected_generation":null,"event_id":"event","payload_version":1,"payload":[102,114,111,122,101,110,32,114,101,113,117,101,115,116]}"#);
        assert_eq!(serde_json::to_vec(&reference).unwrap(), br#"{"version":1,"key":{"instance":"00000000-0000-0000-0000-000000000001","seat":"seat","event_id":"event","operation_id":"00000000-0000-0000-0000-000000000002"},"request_sha256":"3d5c439da5297e76f8cc8957d7337460a1ae29192c8b4981604c79c4ab51b013","output_sha256":"f4844e318dbccd47e7dff89fe3ce6b0f37576b4ba90f726b925abc81088f4842","response_meta_sha256":"e998b145a9e31c19c3c3b0511e220ad1158bd33f21fee2aa0604bf1266c28328","selectors_sha256":"d3ab8a526463619ae805f5fc4f3c4661ff23952364ccf6e6c96bdbe7cf958d0c"}"#);
    }
    #[test]
    fn unknown_context_identity_is_rejected_without_rewriting_file() {
        // The journal requires a physical path; macOS temp dirs sit under /var.
        let directory = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("context-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let state = State {
            version: 1,
            instance: Uuid::from_u128(1),
            seat: "seat".into(),
            current: Some(fixture().context),
            pending: None,
            completed: vec![],
            abandoned: vec![],
            observation_watermarks: vec![],
            declared_resets: vec![],
            prepared_kinds: vec![],
        };
        let bytes = serde_json::to_string(&state)
            .unwrap()
            .replace("Codex", "Unregistered")
            .into_bytes();
        let path = directory.join("context.json");
        fs::write(&path, &bytes).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let journal = ContextJournal::open(
            &directory,
            state.instance,
            "seat",
            Duration::from_millis(50),
        )
        .unwrap();
        assert_eq!(journal.current(), Err(ContextError::Corrupt));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }
}
