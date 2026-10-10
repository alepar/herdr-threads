//! agy hook-pull delivery (ht-poh.5): the `HookPull` declaration and the adapter's `HookStep`.
//!
//! At every top-level `PreInvocation` the step confirms the frames it injected earlier (a bounded
//! scan of `transcriptPath` from the byte offset recorded at injection), pulls the next page with
//! the cached attention version and injects it as one `systemMessage` (a `userMessage` once the
//! agy executable identity is downgraded), or prints nothing. At top-level `Stop` it confirms the
//! same way and keeps the downgrade bookkeeping. Every error path prints nothing.
//!
//! State owned here (both under `<instance_dir>/adapter-state/agy/`, mode 0600):
//! - `conversations/<hash>.json` ([`ConversationState`], at most 4 KiB): created only after a
//!   `DeliveryPull` that was not refused, deleted on a `session_mismatch` refusal.
//! - `downgrade.json` ([`DowngradeRecord`]): keyed by the agy executable [`Identity`].
use super::{InjectionStep, paths};
use crate::{
    harness::adapter::{
        DeliveryMechanism, DeliveryPolicy, HookStep, HookStepContext, HookStepDaemon, IdleWake,
        SetupEnvironment,
    },
    protocol::{
        authority::{COOPERATIVE_HOOK_DELIVERY_PROVENANCE, CallerClaim},
        commands::Command,
        delivery::{
            DeliveryConfirm, DeliveryConfirmOutcome, DeliveryPull, DeliveryPullReply,
            DeliveryRefusalReason,
        },
        ids::{MessageId, OperationId},
        results::CommandResult,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::OnceCell,
    collections::BTreeSet,
    ffi::OsStr,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// The state file stays at or under this many bytes.
pub const MAX_STATE_BYTES: usize = 4096;
/// A confirm scan reads at most this many bytes after the recorded offset.
pub const TRANSCRIPT_WINDOW: u64 = 8 * 1024 * 1024;
/// Injected ids tracked per conversation: at least one full page (the oldest go first).
pub const MAX_TRACKED: usize = crate::service::delivery_pull::PAGE_MAX_BLOCKS;
const _: () = assert!(MAX_TRACKED >= crate::service::delivery_pull::PAGE_MAX_BLOCKS);
/// A downgrade is retried (one probation turn) this long after it was recorded.
pub const PROBATION_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
/// `<state_dir>/downgrade.json`.
pub const DOWNGRADE_FILE: &str = "downgrade.json";
/// The conversation state files the doctor counts, at most.
const DOCTOR_STATE_FILES: usize = 64;
/// Stop `error` fragments of an aborted injection (probe runs 15, 19b, tui-04).
const ABORT_MARKERS: [&str; 2] = ["failed to inject steps", "unknown injected step type"];

/// agy declares `(HookPull, cooperative_hook_delivery, Native)`.
pub struct AgyDelivery;
impl DeliveryPolicy for AgyDelivery {
    fn mechanism(&self) -> DeliveryMechanism {
        DeliveryMechanism::HookPull
    }
    fn provenance(&self) -> &'static str {
        COOPERATIVE_HOOK_DELIVERY_PROVENANCE
    }
    fn idle_wake(&self) -> IdleWake {
        IdleWake::Native
    }
    fn wake_text(&self) -> &'static str {
        crate::notification::policy::WAKE_UP
    }
}

// ---------------------------------------------------------------------------------------------
// Executable identity
// ---------------------------------------------------------------------------------------------

/// The agy executable a hook belongs to: the resolved target's path, inode, size and mtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub path: String,
    pub inode: u64,
    pub size: u64,
    pub mtime_ns: i64,
}
impl Identity {
    /// Stands in when no executable can be resolved, so the record still has a key.
    pub fn unknown() -> Self {
        Self {
            path: String::new(),
            inode: 0,
            size: 0,
            mtime_ns: 0,
        }
    }
}

fn identity_of(path: &Path) -> Option<Identity> {
    let real = std::fs::canonicalize(path).ok()?;
    let meta = std::fs::metadata(&real).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(Identity {
        path: real.to_str()?.to_owned(),
        inode: meta.ino(),
        size: meta.len(),
        mtime_ns: meta
            .mtime()
            .saturating_mul(1_000_000_000)
            .saturating_add(meta.mtime_nsec()),
    })
}

/// The parent-process image when it is agy itself, else the PATH-resolved `agy` (the parent was a
/// shell or launcher). Both are resolved through symlinks before they are measured.
pub fn resolve_identity(parent_image: Option<&Path>, path_env: Option<&OsStr>) -> Option<Identity> {
    let names_agy = |path: &Path| {
        matches!(
            path.file_name().and_then(OsStr::to_str),
            Some("agy" | "antigravity")
        )
    };
    if let Some(image) = parent_image {
        let real = std::fs::canonicalize(image).ok();
        if (names_agy(image) || real.as_deref().is_some_and(names_agy))
            && let Some(identity) = identity_of(image)
        {
            return Some(identity);
        }
    }
    let found = crate::cli::hook::resolve_on_path("agy", path_env)?;
    identity_of(&found)
}

/// The image of this process's parent (`proc_pidpath` on macOS, `/proc/<ppid>/exe` on Linux).
fn parent_image() -> Option<PathBuf> {
    // SAFETY: getppid has no preconditions and cannot fail.
    let ppid = unsafe { libc::getppid() };
    if ppid <= 1 {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: the buffer is writable for `buffer.len()` bytes and the call writes at most that.
        let written =
            unsafe { libc::proc_pidpath(ppid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if written <= 0 {
            return None;
        }
        buffer.truncate(written as usize);
        Some(PathBuf::from(std::ffi::OsString::from_vec(buffer)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::fs::read_link(format!("/proc/{ppid}/exe")).ok()
    }
}

/// The identity of the agy this hook runs under; [`Identity::unknown`] when nothing resolves.
pub fn current_identity() -> Identity {
    resolve_identity(
        parent_image().as_deref(),
        std::env::var_os("PATH").as_deref(),
    )
    .unwrap_or_else(Identity::unknown)
}

// ---------------------------------------------------------------------------------------------
// State directory
// ---------------------------------------------------------------------------------------------

/// `<instance_dir>/adapter-state/agy/` for a state root and host endpoint, without creating it.
pub fn adapter_state_dir(state: Option<PathBuf>, host: Option<PathBuf>) -> Option<PathBuf> {
    let context = crate::daemon::paths::RuntimeContext::from_environment(state, host).ok()?;
    Some(
        crate::daemon::paths::instance_dir(&context)
            .join("adapter-state")
            .join("agy"),
    )
}

/// The state directory named by a registered hook command's own argv (`--state-dir`,
/// `--host-endpoint`); `None` outside a hook command, so a test binary never reads a real one.
fn argv_state_dir(args: impl IntoIterator<Item = std::ffi::OsString>) -> Option<PathBuf> {
    let mut args = args.into_iter();
    let (mut state, mut host) = (None, None);
    while let Some(arg) = args.next() {
        if arg == "--state-dir" {
            state = args.next().map(PathBuf::from);
        } else if arg == "--host-endpoint" {
            host = args.next().map(PathBuf::from);
        }
    }
    adapter_state_dir(Some(state?), host)
}

fn setup_state_dir(env: &SetupEnvironment) -> Option<PathBuf> {
    adapter_state_dir(env.state_dir.clone(), env.host_endpoint.clone())
}

// ---------------------------------------------------------------------------------------------
// Small file helpers
// ---------------------------------------------------------------------------------------------

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// A regular file of at most `cap` bytes, read whole.
fn read_capped(path: &Path, cap: usize) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > cap as u64 {
        return None;
    }
    std::fs::read(path).ok()
}

/// Write `bytes` privately (0600) through a temp file and a rename.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    let written = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    match written.and_then(|()| std::fs::rename(&temp, path)) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Per-conversation state
// ---------------------------------------------------------------------------------------------

/// One frame id the hook injected and has not seen confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Injected {
    pub id: String,
    /// Transcript size when the page carrying `id` was injected.
    pub offset: u64,
    pub turn: u64,
    /// Injected as a `userMessage` (a downgraded identity): never evidence about `systemMessage`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub user: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationState {
    pub v: u32,
    pub seen_attention_version: Option<u64>,
    pub injected: Vec<Injected>,
    /// Counts `invocationNum == 0` events.
    pub turn: u64,
    pub injected_this_turn: bool,
    /// The binding generation of the last non-refused pull (the doctor's "generation G").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}
impl Default for ConversationState {
    fn default() -> Self {
        Self {
            v: 1,
            seen_attention_version: None,
            injected: Vec::new(),
            turn: 0,
            injected_this_turn: false,
            generation: None,
        }
    }
}
impl ConversationState {
    fn load(path: &Path) -> (Self, bool) {
        match read_capped(path, MAX_STATE_BYTES)
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|state| state.v == 1)
        {
            Some(state) => (state, true),
            None => (Self::default(), false),
        }
    }
    /// Serialized bytes of at most [`MAX_STATE_BYTES`]: the oldest injected ids go first.
    fn serialize_bounded(&mut self) -> Vec<u8> {
        while self.injected.len() > MAX_TRACKED {
            self.injected.remove(0);
        }
        loop {
            let bytes = serde_json::to_vec(self).unwrap_or_default();
            if bytes.len() <= MAX_STATE_BYTES || self.injected.is_empty() {
                return bytes;
            }
            self.injected.remove(0);
        }
    }
    fn save(&mut self, path: &Path) {
        let bytes = self.serialize_bounded();
        let _ = write_private(path, &bytes);
    }
    /// Record `ids` as injected at `offset`; an id offered again moves to the new position.
    fn record_injection(&mut self, ids: &[MessageId], offset: u64, user: bool) {
        for id in ids {
            self.injected.retain(|entry| entry.id != id.as_str());
            self.injected.push(Injected {
                id: id.as_str().to_owned(),
                offset,
                turn: self.turn,
                user,
            });
        }
        self.injected_this_turn = true;
    }
}

// ---------------------------------------------------------------------------------------------
// Transcript scan
// ---------------------------------------------------------------------------------------------

/// What a bounded transcript scan saw.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scan {
    pub found: BTreeSet<String>,
    /// The transcript was opened and read. `false` is non-evidence.
    pub readable: bool,
    /// Everything after the offset lay inside the window. `false`: an id not found proves nothing.
    pub complete: bool,
}

/// The step source agy logs for a step a hook injected (genuine agy 1.3.2 capture,
/// `tests/fixtures/agy/delivery/transcript-genuine-*.jsonl`).
const INJECTED_SOURCE: &str = "SYSTEM_SDK";
/// The step types of a hook injection: `systemMessage` is logged as `SYSTEM_MESSAGE`, and
/// `userMessage` as `USER_INPUT` (same capture fixtures).
const INJECTED_TYPES: [&str; 2] = ["SYSTEM_MESSAGE", "USER_INPUT"];

/// An enum spelling without its proto prefix, upper-cased: `CORTEX_STEP_SOURCE_SYSTEM_SDK` and
/// `system_sdk` both read `SYSTEM_SDK`.
fn normalized(value: &str) -> String {
    let bare = value
        .strip_prefix("CORTEX_STEP_TYPE_")
        .or_else(|| value.strip_prefix("CORTEX_STEP_SOURCE_"))
        .unwrap_or(value);
    bare.to_ascii_uppercase()
}

/// The decoded text of a transcript record, only when the record is a step a hook injected
/// (source `SYSTEM_SDK`, type `SYSTEM_MESSAGE` or `USER_INPUT`). Tool output, model output and
/// user-typed input are never injected steps.
fn injected_text(record: &serde_json::Value) -> Option<&str> {
    let source = normalized(record.get("source")?.as_str()?);
    let kind = normalized(record.get("type")?.as_str()?);
    if source != INJECTED_SOURCE || !INJECTED_TYPES.contains(&kind.as_str()) {
        return None;
    }
    record.get("content")?.as_str()
}

/// A frame header line: `[herdr-threads] message <id> in …` or `[herdr-threads] lazy <id> in …`,
/// at column 0 of a line of the decoded `text` (split on real newlines only). A body line is
/// indented by two spaces, so a header quoted inside a body never starts a line.
fn header_ids(text: &str, wanted: &BTreeSet<&str>, found: &mut BTreeSet<String>) {
    for line in text.split('\n') {
        let Some(rest) = line
            .strip_prefix("[herdr-threads] message ")
            .or_else(|| line.strip_prefix("[herdr-threads] lazy "))
        else {
            continue;
        };
        let id = rest.split(' ').next().unwrap_or(rest);
        if wanted.contains(id) {
            found.insert(id.to_owned());
        }
    }
}

/// Scan `path` from byte `from` (bounded to [`TRANSCRIPT_WINDOW`]) for the frame headers of
/// `wanted`. The window is decoded record by record (one JSON object per line): only
/// hook-injected steps are read, and a header counts only at column 0 of a line of a step's
/// decoded text. A record that does not decode (garbage, invalid UTF-8, not an object, or cut
/// by the window end) is skipped, as is a first record the window starts inside of. A
/// transcript shorter than `from` was replaced: it is scanned from its start.
pub fn scan_transcript(path: &Path, from: u64, wanted: &BTreeSet<&str>) -> Scan {
    let mut scan = Scan::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return scan;
    };
    let Ok(meta) = file.metadata() else {
        return scan;
    };
    if !meta.is_file() {
        return scan;
    }
    let start = if from > meta.len() { 0 } else { from };
    let remaining = meta.len() - start;
    let window = remaining.min(TRANSCRIPT_WINDOW);
    let mut buffer = vec![0u8; window as usize];
    // A window that starts mid-line begins inside a record: that first segment is partial.
    let mut partial_first = false;
    if start > 0 {
        let mut previous = [0u8; 1];
        if file.seek(SeekFrom::Start(start - 1)).is_err() || file.read_exact(&mut previous).is_err()
        {
            return scan;
        }
        partial_first = previous[0] != b'\n';
    } else if file.seek(SeekFrom::Start(0)).is_err() {
        return scan;
    }
    if file.read_exact(&mut buffer).is_err() {
        return scan;
    }
    scan.readable = true;
    scan.complete = remaining <= TRANSCRIPT_WINDOW;
    for (index, segment) in buffer.split(|byte| *byte == b'\n').enumerate() {
        if index == 0 && partial_first {
            continue;
        }
        // `-` and ASCII letters are never JSON-escaped: a record with a header contains this.
        if !segment
            .windows(b"herdr-threads".len())
            .any(|window| window == b"herdr-threads")
        {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<serde_json::Value>(segment) else {
            continue;
        };
        if let Some(text) = injected_text(&record) {
            header_ids(text, wanted, &mut scan.found);
        }
    }
    scan
}

// ---------------------------------------------------------------------------------------------
// Downgrade record
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DowngradeRecord {
    pub v: u32,
    pub identity: Identity,
    /// Turn ids of injection misses, on distinct turns.
    pub misses: Vec<u64>,
    /// Set while the identity is downgraded to `userMessage`.
    pub downgraded_at: Option<u64>,
    /// `"abort"` or `"misses"`.
    pub reason: Option<String>,
    /// A probation turn (one `systemMessage` retry) is under way.
    pub probation: bool,
}
impl DowngradeRecord {
    fn blank(identity: &Identity) -> Self {
        Self {
            v: 1,
            identity: identity.clone(),
            misses: Vec::new(),
            downgraded_at: None,
            reason: None,
            probation: false,
        }
    }
    fn downgrade(&mut self, reason: &str, now: u64) {
        self.downgraded_at = Some(now);
        self.reason = Some(reason.to_owned());
        self.probation = false;
        self.misses.clear();
    }
    /// The step to inject with at `now`: a downgraded identity gets `userMessage` except on its
    /// probation turn, which opens 24 h after the downgrade.
    pub fn step(&self, now: u64) -> InjectionStep {
        match self.downgraded_at {
            Some(at) if !self.probation && now < at.saturating_add(PROBATION_AFTER_MS) => {
                InjectionStep::UserMessage
            }
            _ => InjectionStep::SystemMessage,
        }
    }
}

fn downgrade_path(state_dir: &Path) -> PathBuf {
    state_dir.join(DOWNGRADE_FILE)
}

fn load_downgrade(state_dir: &Path) -> Option<DowngradeRecord> {
    read_capped(&downgrade_path(state_dir), MAX_STATE_BYTES)
        .and_then(|bytes| serde_json::from_slice::<DowngradeRecord>(&bytes).ok())
        .filter(|record| record.v == 1)
}

fn save_downgrade(state_dir: &Path, record: &DowngradeRecord) {
    if let Ok(bytes) = serde_json::to_vec(record) {
        let _ = write_private(&downgrade_path(state_dir), &bytes);
    }
}

fn remove_downgrade(state_dir: &Path) {
    let _ = std::fs::remove_file(downgrade_path(state_dir));
}

/// `admit`'s lookup: `userMessage` only while the record of this very identity says so.
pub fn admission_step_in(state_dir: Option<&Path>, now: u64) -> InjectionStep {
    let Some(record) = state_dir.and_then(load_downgrade) else {
        return InjectionStep::SystemMessage;
    };
    if record.downgraded_at.is_some() && record.identity == current_identity() {
        record.step(now)
    } else {
        InjectionStep::SystemMessage
    }
}

/// The injection step a hook invocation of this process admits with.
pub fn admission_step() -> InjectionStep {
    admission_step_in(argv_state_dir(std::env::args_os()).as_deref(), now_ms())
}

/// `setup agy` clears the downgrade record.
pub fn clear_downgrade(env: &SetupEnvironment) {
    if let Some(dir) = setup_state_dir(env) {
        remove_downgrade(&dir);
    }
}

// ---------------------------------------------------------------------------------------------
// The step
// ---------------------------------------------------------------------------------------------

/// Clock and identity of one run, injectable for tests.
pub struct Runtime {
    pub now_ms: u64,
    identity: OnceCell<Identity>,
    resolver: Box<dyn Fn() -> Identity>,
}
impl Runtime {
    pub fn new(now_ms: u64, resolver: Box<dyn Fn() -> Identity>) -> Self {
        Self {
            now_ms,
            identity: OnceCell::new(),
            resolver,
        }
    }
    fn live() -> Self {
        Self::new(now_ms(), Box::new(current_identity))
    }
    fn identity(&self) -> &Identity {
        self.identity.get_or_init(|| (self.resolver)())
    }
}

pub struct AgyHookStep;
impl HookStep for AgyHookStep {
    fn run(&self, context: &HookStepContext<'_>) -> Vec<u8> {
        run_step(context, &Runtime::live())
    }
}

/// What the step needs from the hook payload.
struct Payload {
    conversation: String,
    invocation: Option<u64>,
    transcript: Option<PathBuf>,
    termination: String,
    error: String,
}
fn parse_payload(context: &HookStepContext<'_>) -> Option<Payload> {
    let value: Value = serde_json::from_slice(&context.input.bytes).ok()?;
    let text = |key: &str| value.get(key).and_then(Value::as_str);
    let conversation = text("conversationId").filter(|id| paths::valid_conversation_id(id))?;
    Some(Payload {
        conversation: conversation.to_owned(),
        invocation: value.get("invocationNum").and_then(Value::as_u64),
        transcript: text("transcriptPath")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute()),
        termination: text("terminationReason").unwrap_or_default().to_owned(),
        error: text("error").unwrap_or_default().to_owned(),
    })
}

/// Complete native stdout for one top-level `PreInvocation` or `Stop`; empty on any failure.
pub fn run_step(context: &HookStepContext<'_>, runtime: &Runtime) -> Vec<u8> {
    let Some(state_dir) = context.state_dir.as_deref() else {
        return Vec::new();
    };
    let Some(payload) = parse_payload(context) else {
        return Vec::new();
    };
    let step = Step {
        context,
        runtime,
        state_dir,
        payload: &payload,
        state_path: paths::conversation_state(state_dir, &payload.conversation),
    };
    match context.decoded.metadata.native_event.as_str() {
        "PreInvocation" => step.pre_invocation(),
        "Stop" => {
            step.stop();
            Vec::new()
        }
        _ => Vec::new(),
    }
}

struct Step<'a> {
    context: &'a HookStepContext<'a>,
    runtime: &'a Runtime,
    state_dir: &'a Path,
    payload: &'a Payload,
    state_path: PathBuf,
}

impl Step<'_> {
    fn in_time(&self) -> bool {
        Instant::now() < self.context.deadline
    }
    fn daemon(&self) -> Option<(&dyn HookStepDaemon, &CallerClaim)> {
        Some((self.context.daemon?, self.context.claim.as_ref()?))
    }
    /// The record of the current identity (blank when none exists or it belongs to another one).
    fn downgrade(&self) -> DowngradeRecord {
        match load_downgrade(self.state_dir) {
            Some(record) if record.identity == *self.runtime.identity() => record,
            _ => DowngradeRecord::blank(self.runtime.identity()),
        }
    }
    fn transcript_len(&self) -> u64 {
        self.payload
            .transcript
            .as_deref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |meta| meta.len())
    }

    /// Scan for the tracked frames and send `DeliveryConfirm` for the ids found.
    fn confirm(&self, state: &mut ConversationState, downgrade: &mut DowngradeRecord) -> Scan {
        if state.injected.is_empty() {
            return Scan {
                readable: true,
                complete: true,
                ..Scan::default()
            };
        }
        let Some(path) = self.payload.transcript.as_deref() else {
            return Scan::default();
        };
        let wanted: BTreeSet<&str> = state.injected.iter().map(|e| e.id.as_str()).collect();
        let from = state.injected.iter().map(|e| e.offset).min().unwrap_or(0);
        let scan = scan_transcript(path, from, &wanted);
        if scan.found.is_empty() {
            return scan;
        }
        // A systemMessage frame reached the transcript: a probation turn is over.
        if downgrade.probation
            && state
                .injected
                .iter()
                .any(|e| !e.user && scan.found.contains(&e.id))
        {
            remove_downgrade(self.state_dir);
            *downgrade = DowngradeRecord::blank(self.runtime.identity());
        }
        let ids: Vec<MessageId> = scan
            .found
            .iter()
            .map(|id| MessageId::new(id.as_str()))
            .collect();
        let settled = match self.daemon() {
            Some((daemon, claim)) if self.in_time() => daemon
                .call(Command::DeliveryConfirm(DeliveryConfirm {
                    claim: claim.clone(),
                    ids: ids.clone(),
                    operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
                }))
                .ok()
                .and_then(|result| match result {
                    CommandResult::DeliveryConfirmed(report) => Some(report),
                    _ => None,
                }),
            _ => None,
        };
        if let Some(report) = settled {
            let retry: BTreeSet<&str> = report
                .results
                .iter()
                .filter(|item| item.result == DeliveryConfirmOutcome::Retryable)
                .map(|item| item.id.as_str())
                .collect();
            state
                .injected
                .retain(|e| !scan.found.contains(&e.id) || retry.contains(e.id.as_str()));
        }
        scan
    }

    fn pre_invocation(&self) -> Vec<u8> {
        let Some((daemon, claim)) = self.daemon() else {
            return Vec::new();
        };
        let (mut state, existed) = ConversationState::load(&self.state_path);
        if self.payload.invocation == Some(0) {
            state.turn += 1;
            state.injected_this_turn = false;
        } else if state.turn == 0 {
            state.turn = 1;
        }
        if state
            .generation
            .is_some_and(|g| g != claim.binding_generation)
        {
            state.injected.clear();
            state.seen_attention_version = None;
            state.injected_this_turn = false;
        }
        let mut downgrade = match load_downgrade(self.state_dir) {
            Some(_) => self.downgrade(),
            None => DowngradeRecord::blank(&Identity::unknown()),
        };
        self.confirm(&mut state, &mut downgrade);
        if !self.in_time() {
            if existed {
                state.save(&self.state_path);
            }
            return Vec::new();
        }
        let reply = daemon.call(Command::DeliveryPull(DeliveryPull {
            claim: claim.clone(),
            seen_attention_version: state.seen_attention_version,
            marker_prefix: self.context.command_prefix.clone(),
        }));
        let reply = match reply {
            Ok(CommandResult::DeliveryPulled(reply)) => reply,
            _ => {
                if existed {
                    state.save(&self.state_path);
                }
                return Vec::new();
            }
        };
        match reply {
            DeliveryPullReply::Refused { reason } => {
                if reason == DeliveryRefusalReason::SessionMismatch {
                    let _ = std::fs::remove_file(&self.state_path);
                } else if existed {
                    state.save(&self.state_path);
                }
                Vec::new()
            }
            DeliveryPullReply::Unchanged { attention_version } => {
                state.seen_attention_version = Some(attention_version);
                state.generation = Some(claim.binding_generation);
                state.save(&self.state_path);
                Vec::new()
            }
            DeliveryPullReply::Page(page) => {
                state.seen_attention_version = Some(page.attention_version);
                state.generation = Some(claim.binding_generation);
                let mut record = self.downgrade();
                let step = record.step(self.runtime.now_ms);
                if record.downgraded_at.is_some()
                    && !record.probation
                    && step == InjectionStep::SystemMessage
                {
                    // The probation turn opens: one systemMessage retry.
                    record.probation = true;
                    save_downgrade(self.state_dir, &record);
                }
                let user = step == InjectionStep::UserMessage;
                let output = step.render(&page.text);
                let offset = self.transcript_len();
                state.record_injection(&page.offered, offset, user);
                state.save(&self.state_path);
                serde_json::to_vec(&output).unwrap_or_default()
            }
        }
    }

    fn stop(&self) {
        let (mut state, existed) = ConversationState::load(&self.state_path);
        let mut downgrade = match load_downgrade(self.state_dir) {
            Some(_) => self.downgrade(),
            None => DowngradeRecord::blank(&Identity::unknown()),
        };
        let scan = if existed {
            self.confirm(&mut state, &mut downgrade)
        } else {
            Scan::default()
        };
        // Re-read under the real identity before any write.
        let mut record = self.downgrade();
        let live = record.downgraded_at.is_none() || record.probation;
        let abort = ABORT_MARKERS
            .iter()
            .any(|marker| self.payload.error.contains(marker));
        if abort && live {
            record.downgrade("abort", self.runtime.now_ms);
            save_downgrade(self.state_dir, &record);
        } else if existed && live && self.is_miss(&state, &scan) {
            if record.probation {
                record.downgrade("misses", self.runtime.now_ms);
            } else {
                if !record.misses.contains(&state.turn) {
                    record.misses.push(state.turn);
                }
                if record.misses.len() >= 2 {
                    record.downgrade("misses", self.runtime.now_ms);
                }
            }
            save_downgrade(self.state_dir, &record);
        }
        if existed {
            state.save(&self.state_path);
        }
    }

    /// An injection miss: this turn injected a systemMessage frame that is still absent from a
    /// transcript read in full, and the turn ended normally.
    fn is_miss(&self, state: &ConversationState, scan: &Scan) -> bool {
        let normal = matches!(
            self.payload.termination.as_str(),
            "NO_TOOL_CALL" | "model_stop"
        );
        normal
            && state.injected_this_turn
            && scan.readable
            && scan.complete
            && state
                .injected
                .iter()
                .any(|e| e.turn == state.turn && !e.user && !scan.found.contains(&e.id))
    }
}

// ---------------------------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------------------------

/// The `hooks.agy.delivery` projection: the downgrade record of the PATH-resolved agy and the
/// count of ids offered in an earlier turn and never confirmed.
pub fn doctor_json(env: &SetupEnvironment) -> Value {
    let Some(dir) = setup_state_dir(env) else {
        return json!({});
    };
    let mut out = json!({});
    if let Some(record) = load_downgrade(&dir).filter(|r| r.downgraded_at.is_some()) {
        let current = resolve_identity(None, env.path.as_deref());
        out["downgrade"] = json!({
            "path": record.identity.path,
            "reason": record.reason,
            "downgraded_at": record.downgraded_at,
            "probation": record.probation,
            // Another identity retries systemMessage, so its record no longer applies.
            "identity_current": current.map(|c| c == record.identity),
        });
    }
    let (mut count, mut generation) = (0usize, None::<u64>);
    if let Ok(entries) = std::fs::read_dir(dir.join("conversations")) {
        for entry in entries.flatten().take(DOCTOR_STATE_FILES) {
            let Some(state) = read_capped(&entry.path(), MAX_STATE_BYTES)
                .and_then(|bytes| serde_json::from_slice::<ConversationState>(&bytes).ok())
            else {
                continue;
            };
            let stale = state
                .injected
                .iter()
                .filter(|e| e.turn < state.turn)
                .count();
            if stale > 0 {
                count += stale;
                generation = generation.max(state.generation);
            }
        }
    }
    if count > 0 {
        out["never_confirmed"] = json!({"count": count, "generation": generation});
    }
    out
}

/// The doctor sentences of a [`doctor_json`] value.
pub fn doctor_lines(delivery: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    let downgrade = &delivery["downgrade"];
    if downgrade.is_object() && downgrade["identity_current"] != json!(false) {
        lines.push(format!(
            "downgraded to userMessage ({}) for {}",
            downgrade["reason"].as_str().unwrap_or("unknown"),
            downgrade["path"].as_str().unwrap_or("unknown"),
        ));
    }
    let never = &delivery["never_confirmed"];
    if let Some(count) = never["count"].as_u64() {
        lines.push(format!(
            "offered, never confirmed ({count} in generation {})",
            never["generation"]
                .as_u64()
                .map_or_else(|| "unknown".to_owned(), |g| g.to_string()),
        ));
    }
    lines
}

#[cfg(test)]
#[path = "../../../tests/harness/agy_delivery.rs"]
mod tests;
#[cfg(test)]
#[path = "../../../tests/harness/agy_wake.rs"]
mod wake_tests;
