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
//!   `DeliveryPull` that was not refused, deleted on a `session_mismatch` refusal; a file untouched for 30 days is
//!   deleted when another conversation's file is created.
//! - `downgrade.json` ([`DowngradeRecord`]): keyed by the agy executable [`Identity`].
use super::{InjectionStep, paths};
use crate::{
    harness::adapter::{
        DeliveryMechanism, DeliveryPolicy, HookStep, HookStepCompletionPayload, HookStepContext,
        HookStepDaemon, HookStepOutput, IdleWake, SetupEnvironment,
    },
    protocol::{
        authority::{COOPERATIVE_HOOK_DELIVERY_PROVENANCE, CallerClaim},
        commands::Command,
        delivery::{
            DeliveryConfirm, DeliveryConfirmOutcome, DeliveryPresentationConfirmV2,
            DeliveryPresentationConfirmationV2, DeliveryPull, DeliveryPullReply,
            DeliveryPullReplyV2, DeliveryPullV2, DeliveryRefusalReason, PresentationConsumer,
            PresentationControlStampV2, PresentationRenderRequest, PresentationRenderSelection,
        },
        ids::{MessageId, OperationId},
        results::CommandResult,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeSet,
    ffi::OsStr,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// The state file stays at or under this many bytes.
pub const MAX_STATE_BYTES: usize = 4096;
/// A conversation state file untouched this long is deleted when another conversation's state
/// file is first created.
pub const CONVERSATION_STATE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// At most this many directory entries are looked at per prune.
const PRUNE_SCAN_LIMIT: usize = 256;
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
    /// Transcript size when the page carrying `id` was injected. An entry more than
    /// [`TRANSCRIPT_WINDOW`] behind the transcript end is dropped unconfirmed.
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
    /// Highest emitted V2 Attention epoch, retained after its scoped clear so
    /// an old replay cannot resurrect it. Absent preserves historical V1 bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_presentation_epoch: Option<String>,
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
            seen_presentation_epoch: None,
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
        // An empty id list injected nothing a confirm could prove.
        if !ids.is_empty() {
            self.injected_this_turn = true;
        }
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
    /// A probation turn (one `systemMessage` retry) is under way. It is exactly one turn and ends
    /// at that turn's `Stop`: a confirm clears the record, a miss or an inconclusive end
    /// re-downgrades with a fresh timestamp; a probation with no `Stop` for 24 h re-downgrades at
    /// the next injection.
    pub probation: bool,
    /// When the probation turn opened (ms). `None` for a record written before this field existed.
    #[serde(default)]
    pub probation_at: Option<u64>,
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
            probation_at: None,
        }
    }
    fn downgrade(&mut self, reason: &str, now: u64) {
        self.downgraded_at = Some(now);
        self.reason = Some(reason.to_owned());
        self.probation = false;
        self.probation_at = None;
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
        self.run_with_completion(context).stdout
    }
    fn run_with_completion(&self, context: &HookStepContext<'_>) -> HookStepOutput {
        run_step_with_completion(context, &Runtime::live())
    }
    fn complete(
        &self,
        payload: &[u8],
        daemon: &dyn HookStepDaemon,
        state_dir: &Path,
        deadline: Instant,
    ) {
        complete_step(payload, daemon, state_dir, deadline);
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
    run_step_with_completion(context, runtime).stdout
}
fn run_step_with_completion(context: &HookStepContext<'_>, runtime: &Runtime) -> HookStepOutput {
    let empty = || HookStepOutput {
        stdout: Vec::new(),
        completion: None,
        diagnostic: None,
    };
    let Some(state_dir) = context.state_dir.as_deref() else {
        return empty();
    };
    let Some(payload) = parse_payload(context) else {
        return empty();
    };
    let step = Step {
        context,
        runtime,
        state_dir,
        payload: &payload,
        state_path: paths::conversation_state(state_dir, &payload.conversation),
        completion: RefCell::new(None),
        diagnostic: RefCell::new(None),
    };
    let stdout = match context.decoded.metadata.native_event.as_str() {
        "PreInvocation" => step.pre_invocation(),
        "Stop" => {
            step.stop();
            Vec::new()
        }
        _ => Vec::new(),
    };
    HookStepOutput {
        stdout,
        completion: step.completion.into_inner(),
        diagnostic: step.diagnostic.into_inner(),
    }
}

const PRESENTATION_V2: &str = "delivery.presentation_v2";
// State and optional probation record <=4096 each; six <=128 claim fields
// cost <=2048 escaped bytes (five graphic IDs, one ASCII instance String).
// Three64hex digests, optional32hex token, <=32 canonical decimal epoch stamps
// (<3072 bytes), and fixed keys/scalars (<1024) total <14560. Nested records
// are typed objects, not JSON strings; the actual complete serializer is capped.
// Enforce each component and the final serialized envelope, rather than assuming
// the private transport cap bounds a nested state or an opaque caller ID.
const COMPLETION_MAX_BYTES: usize = 16_384;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmissionCompletion {
    state: ConversationState,
    probation: Option<DowngradeRecord>,
    previous_probation_digest: Option<String>,
    claim: CallerClaim,
    conversation_key: String,
    previous_digest: Option<String>,
    presentation_token: Option<String>,
    emitted_controls: Vec<PresentationControlStampV2>,
    now_ms: u64,
    existed: bool,
}
fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn valid_hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_completion(completion: &EmissionCompletion) -> bool {
    let claim = &completion.claim;
    let bounded_id = |id: &str| !id.is_empty() && id.len() <= 128 && id.is_ascii();
    completion.state.v == 1
        && completion
            .state
            .seen_presentation_epoch
            .as_deref()
            .is_none_or(|v| decimal_epoch(v).is_some())
        && completion.emitted_controls.len() <= MAX_TRACKED
        && crate::protocol::delivery::validate_control_stamps(&completion.emitted_controls).is_ok()
        && completion.emitted_controls.iter().all(|stamp| match stamp {
            PresentationControlStampV2::Attention { epoch, .. }
            | PresentationControlStampV2::Cleared { epoch, .. } => decimal_epoch(epoch).is_some(),
        })
        && completion.state.injected.len() <= MAX_TRACKED
        && serde_json::to_vec(&completion.state).is_ok_and(|bytes| bytes.len() <= MAX_STATE_BYTES)
        && completion.probation.as_ref().is_none_or(|record| {
            record.v == 1
                && record.probation
                && serde_json::to_vec(record).is_ok_and(|bytes| bytes.len() <= MAX_STATE_BYTES)
        })
        && completion
            .previous_probation_digest
            .as_deref()
            .is_none_or(|v| valid_hex(v, 64))
        && [
            claim.instance.as_str(),
            claim.seat.as_str(),
            claim.harness.as_str(),
            claim.native_session.as_str(),
            claim.execution.as_str(),
            claim.target.as_str(),
        ]
        .into_iter()
        .all(bounded_id)
        && claim.role == crate::protocol::authority::CallerRole::TopLevel
        && valid_hex(&completion.conversation_key, 64)
        && completion.conversation_key == hex_digest(claim.native_session.as_str().as_bytes())
        && completion
            .previous_digest
            .as_deref()
            .is_none_or(|v| valid_hex(v, 64))
        && completion
            .presentation_token
            .as_deref()
            .is_none_or(|v| valid_hex(v, 32))
        && completion.state.generation == Some(claim.binding_generation)
}
fn decimal_epoch(epoch: &str) -> Option<u64> {
    if !crate::protocol::delivery::valid_presentation_epoch(epoch) {
        return None;
    }
    epoch.parse().ok()
}
fn apply_emitted_controls(state: &mut ConversationState, controls: &[PresentationControlStampV2]) {
    for stamp in controls {
        match stamp {
            PresentationControlStampV2::Attention { epoch, version } => {
                let Some(incoming) = decimal_epoch(epoch) else {
                    continue;
                };
                let previous = state
                    .seen_presentation_epoch
                    .as_deref()
                    .and_then(decimal_epoch);
                if previous.is_none_or(|old| incoming > old) {
                    state.seen_presentation_epoch = Some(epoch.clone());
                    state.seen_attention_version = Some(*version);
                }
            }
            PresentationControlStampV2::Cleared { epoch, version } => {
                if state.seen_presentation_epoch.as_deref() == Some(epoch.as_str())
                    && state.seen_attention_version == Some(*version)
                {
                    state.seen_attention_version = None;
                }
            }
        }
    }
}
fn complete_step(payload: &[u8], daemon: &dyn HookStepDaemon, state_dir: &Path, deadline: Instant) {
    if payload.len() > COMPLETION_MAX_BYTES || Instant::now() >= deadline {
        return;
    }
    let Ok(mut completion) = serde_json::from_slice::<EmissionCompletion>(payload) else {
        return;
    };
    if !valid_completion(&completion) {
        return;
    }
    if let Some(token) = completion.presentation_token.as_ref() {
        if !daemon.supports_capability(PRESENTATION_V2) {
            return;
        }
        let reply = daemon.call(Command::DeliveryPresentationConfirmV2(
            DeliveryPresentationConfirmV2 {
                claim: completion.claim.clone(),
                consumer: PresentationConsumer::Hook,
                token: token.clone(),
                presentation_lease: None,
                emitted_controls: completion.emitted_controls.clone(),
            },
        ));
        let Ok(CommandResult::DeliveryPresentationConfirmedV2(reply)) = reply else {
            return;
        };
        if !matches!(
            reply.status,
            DeliveryPresentationConfirmationV2::Confirmed
                | DeliveryPresentationConfirmationV2::AlreadyConfirmed
        ) || reply.acknowledged_controls.len() > MAX_TRACKED
            || reply
                .acknowledged_controls
                .iter()
                .enumerate()
                .any(|(index, stamp)| {
                    !completion.emitted_controls.contains(stamp)
                        || reply.acknowledged_controls[..index].contains(stamp)
                        || match stamp {
                            PresentationControlStampV2::Attention { epoch, .. }
                            | PresentationControlStampV2::Cleared { epoch, .. } => {
                                decimal_epoch(epoch).is_none()
                            }
                        }
                })
        {
            return;
        }
        // Obsolete frozen controls may be retired canonically without a current
        // seen acknowledgement. Only the exact returned captured subset is a hint.
        completion.emitted_controls = reply.acknowledged_controls;
    }
    if Instant::now() >= deadline {
        return;
    }
    // Match the existing historical 128-bit filename; the payload carries a full
    // digest, never arbitrary filename/path material. It carries no authority.
    let path = state_dir
        .join("conversations")
        .join(format!("{}.json", &completion.conversation_key[..32]));
    let current = read_capped(&path, MAX_STATE_BYTES).map(|bytes| hex_digest(&bytes));
    if current != completion.previous_digest {
        return;
    }
    // This detects an observed change, not a lock/CAS: local hints retain
    // their historical cooperative concurrency limits.
    if completion.probation.is_some() {
        let current = read_capped(&downgrade_path(state_dir), MAX_STATE_BYTES)
            .map(|bytes| hex_digest(&bytes));
        if current != completion.previous_probation_digest {
            return;
        }
    }
    if !completion.existed {
        prune_conversation_states(state_dir, &path, completion.now_ms);
    }
    apply_emitted_controls(&mut completion.state, &completion.emitted_controls);
    completion.state.save(&path);
    if let Some(probation) = completion.probation {
        save_downgrade(state_dir, &probation);
    }
}

struct Step<'a> {
    context: &'a HookStepContext<'a>,
    runtime: &'a Runtime,
    state_dir: &'a Path,
    payload: &'a Payload,
    state_path: PathBuf,
    completion: RefCell<Option<HookStepCompletionPayload>>,
    diagnostic: RefCell<Option<String>>,
}

/// Delete regular `*.json` files in `<state_dir>/conversations` (other than `keep`) whose mtime is
/// older than [`CONVERSATION_STATE_TTL_MS`] before `now_ms`. Errors are ignored.
fn prune_conversation_states(state_dir: &Path, keep: &Path, now_ms: u64) {
    let Ok(entries) = std::fs::read_dir(state_dir.join("conversations")) else {
        return;
    };
    for entry in entries.flatten().take(PRUNE_SCAN_LIMIT) {
        let path = entry.path();
        if path == keep || path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(mtime_ms) = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
        else {
            continue;
        };
        if now_ms.saturating_sub(mtime_ms) > CONVERSATION_STATE_TTL_MS {
            let _ = std::fs::remove_file(&path);
        }
    }
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
        // An entry more than one window behind the end can never again be scanned completely, so
        // its absence is never evidence; dropping it keeps it from anchoring the scan too early.
        if let Ok(meta) = std::fs::metadata(path) {
            let len = meta.len();
            state
                .injected
                .retain(|e| e.offset.saturating_add(TRANSCRIPT_WINDOW) >= len);
            if state.injected.is_empty() {
                return Scan {
                    readable: true,
                    complete: true,
                    ..Scan::default()
                };
            }
        }
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

    /// Prune old state files when this call creates the conversation's file.
    fn prune_if_new(&self, existed: bool) {
        if !existed && self.in_time() {
            prune_conversation_states(self.state_dir, &self.state_path, self.runtime.now_ms);
        }
    }

    fn pre_invocation(&self) -> Vec<u8> {
        let Some((daemon, claim)) = self.daemon() else {
            return Vec::new();
        };
        let (mut state, existed) = ConversationState::load(&self.state_path);
        let previous_digest =
            read_capped(&self.state_path, MAX_STATE_BYTES).map(|bytes| hex_digest(&bytes));
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
            state.seen_presentation_epoch = None;
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
        // Capability discovery precedes every V2 progress/token operation. Old
        // daemons retain the old pull bytes and historical local-hint semantics.
        let modern = daemon.supports_capability(PRESENTATION_V2);
        // Freeze both preferences before the request. Replay may echo another
        // still-supported frozen selector; never reread local state to encode it.
        let mut frozen_record = modern.then(|| self.downgrade());
        if let Some(record) = frozen_record.as_mut()
            && record.probation
            && record
                .probation_at
                .is_none_or(|at| self.runtime.now_ms >= at.saturating_add(PROBATION_AFTER_MS))
        {
            let reason = record.reason.clone().unwrap_or_else(|| "misses".to_owned());
            record.downgrade(&reason, self.runtime.now_ms);
            save_downgrade(self.state_dir, record);
        }
        let confirmable_step = frozen_record
            .as_ref()
            .map(|record| record.step(self.runtime.now_ms))
            .unwrap_or(InjectionStep::SystemMessage);
        let passive_step = if frozen_record
            .as_ref()
            .is_some_and(|record| record.downgraded_at.is_some())
        {
            InjectionStep::UserMessage
        } else {
            InjectionStep::SystemMessage
        };
        let mut selected_step = None;
        let mut expected_output_bytes = None;
        let mut emitted_controls = Vec::new();
        let (reply, presentation_token) = if modern {
            match daemon.call(Command::DeliveryPullV2(DeliveryPullV2 {
                claim: claim.clone(),
                consumer: PresentationConsumer::Hook,
                render: PresentationRenderRequest::Hook {
                    confirmable_selector: confirmable_step.presentation_selector(),
                    passive_selector: passive_step.presentation_selector(),
                },
                presentation_lease: None,
                marker_prefix: self.context.command_prefix.clone(),
            })) {
                Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page))) => {
                    let PresentationRenderSelection::Hook { selector } = page.render else {
                        *self.diagnostic.borrow_mut() = Some("presentation codec mismatch".into());
                        return Vec::new();
                    };
                    let Some(step) = InjectionStep::from_presentation_selector(selector) else {
                        *self.diagnostic.borrow_mut() = Some("presentation codec mismatch".into());
                        return Vec::new();
                    };
                    if page.items.len() > MAX_TRACKED
                        || (page.presentation_token.is_none()
                            && page.items.iter().any(|item| {
                                !matches!(
                                    item,
                                    crate::protocol::delivery::PresentationItemV2::Message { .. }
                                )
                            }))
                        || page.output_bytes > 65_536
                        || crate::delivery::presentation::emitted_controls(&page.items)
                            != page.emitted_controls
                        || crate::delivery::presentation::presentation_hook_text(&page.items)
                            != page.text
                        || page.emitted_controls.iter().any(|stamp| match stamp {
                            PresentationControlStampV2::Attention { epoch, .. }
                            | PresentationControlStampV2::Cleared { epoch, .. } => {
                                decimal_epoch(epoch).is_none()
                            }
                        })
                    {
                        *self.diagnostic.borrow_mut() = Some("presentation codec mismatch".into());
                        return Vec::new();
                    }
                    if page.items.is_empty() && page.text.is_empty() && page.output_bytes == 0 {
                        return Vec::new(); // obsolete-empty replay is not an emitted control
                    }
                    selected_step = Some(step);
                    expected_output_bytes = Some(page.output_bytes);
                    emitted_controls = page.emitted_controls;
                    let token = page.presentation_token;
                    (
                        DeliveryPullReply::Page(crate::protocol::delivery::DeliveryPage {
                            attention_version: page.attention_version,
                            text: page.text,
                            offered: page.offered,
                            truncated: page.truncated,
                            more: page.more,
                        }),
                        token,
                    )
                }
                Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Unchanged {
                    attention_version,
                })) => (DeliveryPullReply::Unchanged { attention_version }, None),
                Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { reason })) => {
                    (DeliveryPullReply::Refused { reason }, None)
                }
                _ => return Vec::new(),
            }
        } else {
            let reply = daemon.call(Command::DeliveryPull(DeliveryPull {
                claim: claim.clone(),
                seen_attention_version: state.seen_attention_version,
                marker_prefix: self.context.command_prefix.clone(),
            }));
            match reply {
                Ok(CommandResult::DeliveryPulled(reply)) => (reply, None),
                _ => {
                    if existed {
                        state.save(&self.state_path);
                    }
                    return Vec::new();
                }
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
                if modern {
                    return Vec::new();
                }
                state.seen_attention_version = Some(attention_version);
                state.generation = Some(claim.binding_generation);
                self.prune_if_new(existed);
                state.save(&self.state_path);
                Vec::new()
            }
            DeliveryPullReply::Page(page) => {
                if !modern {
                    state.seen_attention_version = Some(page.attention_version);
                }
                state.generation = Some(claim.binding_generation);
                let mut record = frozen_record.unwrap_or_else(|| self.downgrade());
                let now = self.runtime.now_ms;
                // A probation turn that never reached a `Stop` (agy killed) ends after 24 h.
                if record.probation
                    && record
                        .probation_at
                        .is_none_or(|at| now >= at.saturating_add(PROBATION_AFTER_MS))
                {
                    let reason = record.reason.clone().unwrap_or_else(|| "misses".to_owned());
                    record.downgrade(&reason, now);
                    save_downgrade(self.state_dir, &record);
                }
                // A page with no offered id (a notices-only or attention-only page)
                // has nothing a transcript scan could confirm: it never opens or
                // spends a probation turn, and a downgraded harness gets the step
                // known to work.
                let confirmable = !page.offered.is_empty();
                let step = selected_step.unwrap_or_else(|| {
                    if confirmable {
                        record.step(now)
                    } else if record.downgraded_at.is_some() {
                        InjectionStep::UserMessage
                    } else {
                        InjectionStep::SystemMessage
                    }
                });
                let previous_probation_digest =
                    read_capped(&downgrade_path(self.state_dir), MAX_STATE_BYTES)
                        .map(|bytes| hex_digest(&bytes));
                let mut probation = None;
                if confirmable
                    && record.downgraded_at.is_some()
                    && !record.probation
                    && step == InjectionStep::SystemMessage
                    && (!modern || confirmable_step == InjectionStep::SystemMessage)
                {
                    // Opening the retry is output-coupled. An un-emitted or
                    // unconfirmed page cannot spend this one probation turn.
                    record.probation = true;
                    record.probation_at = Some(now);
                    probation = Some(record);
                }
                let user = step == InjectionStep::UserMessage;
                let stdout = if modern {
                    let Some(bytes) = step.encode_presentation(&page.text) else {
                        *self.diagnostic.borrow_mut() = Some("presentation codec mismatch".into());
                        return Vec::new();
                    };
                    if Some(bytes.len() as u32) != expected_output_bytes {
                        *self.diagnostic.borrow_mut() = Some("presentation codec mismatch".into());
                        return Vec::new();
                    }
                    bytes
                } else {
                    // Historical V1 wire bytes and hint semantics are unchanged.
                    let Ok(bytes) = serde_json::to_vec(&step.render(&page.text)) else {
                        return Vec::new();
                    };
                    bytes
                };
                let offset = self.transcript_len();
                state.record_injection(&page.offered, offset, user);
                // Apply the existing bounded-state pruning policy before deriving
                // the payload. No seen/injected/token state is saved during preparation.
                let _ = state.serialize_bounded();
                let completion = EmissionCompletion {
                    state,
                    probation,
                    previous_probation_digest,
                    claim: claim.clone(),
                    conversation_key: hex_digest(self.payload.conversation.as_bytes()),
                    previous_digest,
                    presentation_token,
                    emitted_controls,
                    now_ms: self.runtime.now_ms,
                    existed,
                };
                if !valid_completion(&completion) {
                    return Vec::new();
                }
                let Ok(bytes) = serde_json::to_vec(&completion) else {
                    return Vec::new();
                };
                if bytes.len() > COMPLETION_MAX_BYTES {
                    return Vec::new();
                }
                let Some(payload) = HookStepCompletionPayload::new(bytes) else {
                    return Vec::new();
                };
                *self.completion.borrow_mut() = Some(payload);
                stdout
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
        } else if existed && record.probation && state.injected_this_turn {
            // The probation turn ended without a confirm and without a miss (cancelled, an
            // unrelated error, an unreadable or incomplete scan): it proved nothing, and
            // userMessage is known to work, so fail safe with a fresh wait.
            let reason = record.reason.clone().unwrap_or_else(|| "misses".to_owned());
            record.downgrade(&reason, self.runtime.now_ms);
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
