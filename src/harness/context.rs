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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Harness {
    Codex,
    Claude,
    /// A person in their own Herdr pane (`herdr-threads me init`): never an
    /// agent, so it has no hooks, setup recipe or launch.
    Human,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        {
            return Err(ContextError::Corrupt);
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
        state.pending = Some(request);
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
        self.save(&state)?;
        Ok(Some(pending))
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
    #[cfg(target_os = "macos")]
    options.custom_flags(0x100);
    #[cfg(target_os = "linux")]
    options.custom_flags(0x20000);
}
