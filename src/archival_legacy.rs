//! Read-only, bounded legacy journal coverage; local records can only veto expiry.
//! Published journal records are immutable and renamed into this directory. Every
//! scan restarts on a directory generation change; no allocator or journal API is used.
use crate::{
    cli::journal::{IntentHeader, IntentScope, SemanticMutation},
    daemon::paths::InstancePaths,
    protocol::{
        handoff::{BootstrapIdentity, HandoffIdentity, HandoffNamespace},
        ids::ThreadId,
        output::ContinuationContext,
        results::IntentKind,
    },
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{CStr, CString, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const ENTRIES_PER_PAGE: usize = 32;
pub const RECORD_CAP: usize = 64 * 1024;
const HEADER_CAP: usize = 4096;
const PAGE_BYTES: usize = 256 * 1024;
/// Scanner-selected namespace, independent of the historical frozen payload.
/// The deciding store checks that this source actually belongs to its database.
pub struct HintSource {
    pub namespace: HandoffNamespace,
    pub database_path: PathBuf,
    /// Local traversal uncertainty only; never a canonical state or permission.
    pub veto: Arc<AtomicBool>,
}
pub enum Hint {
    /// Carries a preceding page's deciding uncertainty on an otherwise empty page.
    Coverage { veto: Arc<AtomicBool> },
    Handoff {
        identity: Box<HandoffIdentity>,
        progress_thread: Option<ThreadId>,
        source: Option<HintSource>,
        retained_completion: Option<crate::protocol::handoff::HandoffResult>,
    },
    Bootstrap {
        identity: Box<BootstrapIdentity>,
        source: HintSource,
    },
}
impl Hint {
    pub(crate) fn veto(&self) -> Option<&Arc<AtomicBool>> {
        match self {
            Self::Coverage { veto } => Some(veto),
            Self::Bootstrap { source, .. } => Some(&source.veto),
            Self::Handoff { source, .. } => source.as_ref().map(|source| &source.veto),
        }
    }
}
#[derive(Default)]
pub struct Scan {
    pub hints: Vec<Hint>,
    pub coverage: Option<String>,
    pub pending: bool,
}
fn stamp(meta: &Metadata) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        meta.dev(),
        meta.ino(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec()
    )
}
struct Directory {
    file: File,
    entries: *mut libc::DIR,
}
// Directory and its cursor have a single owner; no concurrent readdir access.
unsafe impl Send for Directory {}
impl Drop for Directory {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.entries);
        }
    }
}
impl Directory {
    fn open(path: &std::path::Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o400 == 0 {
            return Err(io::Error::other("inaccessible legacy directory"));
        }
        let duplicate = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        let entries = unsafe { libc::fdopendir(duplicate) };
        if entries.is_null() {
            unsafe { libc::close(duplicate) };
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file, entries })
    }
    fn next(&mut self) -> io::Result<Option<OsString>> {
        #[cfg(target_os = "macos")]
        let errno = unsafe { libc::__error() };
        #[cfg(not(target_os = "macos"))]
        let errno = unsafe { libc::__errno_location() };
        unsafe { *errno = 0 };
        let entry = unsafe { libc::readdir(self.entries) };
        if entry.is_null() {
            return if unsafe { *errno } == 0 {
                Ok(None)
            } else {
                Err(io::Error::last_os_error())
            };
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        Ok(Some(OsString::from_vec(name.to_bytes().to_vec())))
    }
    fn read(&self, name: &std::ffi::OsStr, cap: usize) -> io::Result<Option<Vec<u8>>> {
        let name = CString::new(name.as_bytes()).map_err(io::Error::other)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let before = file.metadata()?;
        if !before.is_file()
            || before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o400 == 0
            || before.len() > cap as u64
        {
            return Err(io::Error::other("unsafe or oversized legacy record"));
        }
        let mut bytes = Vec::new();
        (&mut file).take(cap as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > cap || stamp(&before) != stamp(&file.metadata()?) {
            return Err(io::Error::other("legacy record changed"));
        }
        Ok(Some(bytes))
    }
}
// Metadata-only equality with the selected namespace. These paths must never
// become journal/data sources: all records still come from the pinned directory.
// Resolve endpoint parents, not the socket itself, matching RuntimeContext even
// while the host socket is absent. Missing paths/context cannot establish equality.
fn namespace(context: &ContinuationContext) -> io::Result<(PathBuf, PathBuf)> {
    let directory = |path: &Path| -> io::Result<PathBuf> {
        if !path.is_absolute() {
            return Err(io::Error::other("unknown legacy namespace"));
        }
        let canonical = fs::canonicalize(path)?;
        let metadata = fs::metadata(&canonical)?;
        if !metadata.is_dir() || metadata.mode() & 0o111 == 0 {
            return Err(io::Error::other("inaccessible legacy namespace"));
        }
        Ok(canonical)
    };
    let state = context
        .state_dir
        .as_deref()
        .ok_or_else(|| io::Error::other("unknown legacy state directory"))?;
    let endpoint = Path::new(
        context
            .host
            .as_deref()
            .ok_or_else(|| io::Error::other("unknown legacy endpoint"))?,
    );
    let parent = endpoint
        .parent()
        .ok_or_else(|| io::Error::other("unknown legacy endpoint parent"))?;
    let name = endpoint
        .file_name()
        .ok_or_else(|| io::Error::other("unknown legacy endpoint name"))?;
    Ok((directory(Path::new(state))?, directory(parent)?.join(name)))
}
pub struct Source {
    root: PathBuf,
    instance: String,
    context: ContinuationContext,
    database_path: PathBuf,
    directory: Option<Directory>,
    generation: Option<String>,
    veto: bool,
    deciding_veto: Arc<AtomicBool>,
    has_hints: bool,
}
impl Source {
    pub fn new(paths: &InstancePaths, instance: String, context: ContinuationContext) -> Self {
        Self {
            root: paths.instance_dir.join("intents"),
            instance,
            context,
            database_path: paths.database_path.clone(),
            directory: None,
            generation: None,
            veto: false,
            deciding_veto: Arc::new(AtomicBool::new(false)),
            has_hints: false,
        }
    }
    fn generation(&self) -> io::Result<String> {
        match fs::symlink_metadata(&self.root) {
            Ok(meta) if meta.is_dir() => Ok(stamp(&meta)),
            Ok(_) => Err(io::Error::other("unsafe legacy root")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let parent = fs::symlink_metadata(self.root.parent().unwrap())?;
                if !parent.is_dir() {
                    return Err(io::Error::other("unsafe instance root"));
                }
                Ok(format!("absent:{}", stamp(&parent)))
            }
            Err(e) => Err(e),
        }
    }
    pub fn validate(&self, coverage: &str) -> bool {
        self.generation().is_ok_and(|current| current == coverage)
    }
    pub fn scan(&mut self, cancelled: impl Fn() -> bool) -> io::Result<Scan> {
        let result = self.scan_page(&cancelled);
        if result.is_err() {
            self.directory = None;
            self.generation = None;
            self.veto = true;
            self.deciding_veto.store(true, Ordering::Release);
        }
        result
    }
    fn scan_page(&mut self, cancelled: &impl Fn() -> bool) -> io::Result<Scan> {
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "legacy scan cancelled",
            ));
        }
        let generation = self.generation()?;
        if self.generation.as_ref() != Some(&generation) {
            self.directory = None;
            self.generation = Some(generation.clone());
            self.veto = false;
            self.deciding_veto = Arc::new(AtomicBool::new(false));
            self.has_hints = false;
        }
        if generation.starts_with("absent:") {
            return Ok(Scan {
                coverage: Some(generation),
                ..Scan::default()
            });
        }
        if self.directory.is_none() {
            self.directory = Some(Directory::open(&self.root)?);
            self.veto = false;
            self.deciding_veto = Arc::new(AtomicBool::new(false));
            self.has_hints = false;
        }
        let dir = self.directory.as_mut().unwrap();
        if stamp(&dir.file.metadata()?) != generation {
            return Err(io::Error::other("legacy directory replaced"));
        }
        let deadline = Instant::now() + Duration::from_millis(10);
        let mut result = Scan {
            pending: true,
            ..Scan::default()
        };
        let mut bytes = 0;
        for _ in 0..ENTRIES_PER_PAGE {
            if cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "legacy scan cancelled",
                ));
            }
            if Instant::now() >= deadline || bytes >= PAGE_BYTES {
                break;
            }
            let Some(name) = dir.next()? else {
                result.pending = false;
                break;
            };
            let Some(text) = name.to_str() else {
                self.veto = true;
                continue;
            };
            if harmless(text) {
                continue;
            }
            if !text.ends_with(".intent") && !text.starts_with("delivery-") {
                self.veto = true;
                continue;
            }
            // Reading the complete capped record is bounded; even a perfectly valid
            // noncompound is a coverage veto. Never trust its discriminator alone.
            let parsed = (|| {
                let terminal_reference = terminal_reference(text)?;
                let (data, terminal) = if let Some(reference) = &terminal_reference {
                    let data = dir
                        .read(&name, 128 * 1024)?
                        .ok_or_else(|| io::Error::other("legacy terminal vanished"))?;
                    bytes += data.len();
                    let terminal = read_terminal(dir, reference, &data, &mut bytes)?;
                    (terminal.original.as_bytes().to_vec(), Some(terminal))
                } else {
                    let data = dir
                        .read(&name, RECORD_CAP)?
                        .ok_or_else(|| io::Error::other("legacy intent vanished"))?;
                    bytes += data.len();
                    (data, None)
                };
                let split = data
                    .iter()
                    .position(|b| *b == b'\n')
                    .filter(|n| *n <= HEADER_CAP)
                    .ok_or_else(|| io::Error::other("legacy header cap"))?;
                let header: IntentHeader = serde_json::from_slice(&data[..split])?;
                if !matches!(
                    header.kind,
                    IntentKind::Handoff
                        | IntentKind::HandoffBootstrap
                        | IntentKind::HandoffDelivery
                ) {
                    return Err(io::Error::other("published noncompound coverage veto"));
                }
                if terminal_reference
                    .as_ref()
                    .is_some_and(|reference| reference != &header.reference)
                    || (terminal_reference.is_none()
                        && text
                            != format!(
                                "{:020}-{}.intent",
                                header.reference.ordinal,
                                header.reference.operation.as_str()
                            ))
                {
                    return Err(io::Error::other("legacy filename mismatch"));
                }
                if header.kind != IntentKind::Handoff
                    && (header.reference.ordinal == 0
                        || uuid::Uuid::parse_str(header.reference.operation.as_str()).is_err())
                {
                    return Err(io::Error::other("invalid modern journal reference"));
                }
                let semantic: SemanticMutation = serde_json::from_slice(&data[split + 1..])?;
                semantic.validate()?;
                if semantic.kind() != header.kind
                    || semantic.thread() != header.thread.as_ref()
                    || format!("{:x}", Sha256::digest(serde_json::to_vec(&semantic)?))
                        != header.semantic_digest
                {
                    return Err(io::Error::other("legacy digest or shape mismatch"));
                }
                if header.kind != IntentKind::Handoff
                    && (serde_json::from_slice::<serde_json::Value>(&data[..split])?
                        != serde_json::to_value(&header)?
                        || serde_json::from_slice::<serde_json::Value>(&data[split + 1..])?
                            != serde_json::to_value(&semantic)?)
                {
                    return Err(io::Error::other("unexpected legacy compound fields"));
                }
                let SemanticMutation::Frozen { claim, mutation } = semantic else {
                    return Err(io::Error::other("legacy compound is not frozen"));
                };
                if header.scope
                    != (IntentScope::Cooperative {
                        instance: self.instance.clone(),
                        seat: claim.seat.clone(),
                    })
                    || claim.instance != self.instance
                {
                    return Err(io::Error::other("legacy original scope mismatch"));
                }
                let selected = namespace(&self.context)?;
                let runtime = crate::daemon::paths::RuntimeContext::explicit(
                    selected.0.clone(),
                    selected.1.clone(),
                    None,
                )?;
                if InstancePaths::resolve_read_only(&runtime)?.database_path != self.database_path {
                    return Err(io::Error::other("legacy selected source paths mismatch"));
                }
                let source = HintSource {
                    namespace: HandoffNamespace {
                        instance: self.instance.clone(),
                        state_dir: selected.0,
                        host_endpoint: selected.1,
                    },
                    database_path: self.database_path.clone(),
                    veto: self.deciding_veto.clone(),
                };
                let frozen_context = match mutation.as_ref() {
                    SemanticMutation::Handoff(plan) => plan.context.clone(),
                    SemanticMutation::HandoffBootstrap(plan) => {
                        context(&plan.payload.handoff.namespace)
                    }
                    SemanticMutation::HandoffDelivery(plan) => context(&plan.payload.namespace),
                    _ => return Err(io::Error::other("legacy compound shape")),
                };
                if namespace(&frozen_context)? != namespace(&self.context)? {
                    return Err(io::Error::other("legacy namespace mismatch"));
                }
                let progress_name = OsString::from(format!(
                    "handoff-{}.progress",
                    header.reference.operation.as_str()
                ));
                let progress = dir.read(
                    &progress_name,
                    if header.kind == IntentKind::HandoffBootstrap {
                        128 * 1024
                    } else {
                        RECORD_CAP
                    },
                )?;
                if let Some(data) = &progress {
                    bytes += data.len();
                }
                match *mutation {
                    SemanticMutation::Handoff(plan) => {
                        let progress = progress
                            .as_deref()
                            .map(serde_json::from_slice::<LegacyProgress>)
                            .transpose()?;
                        Ok(Hint::Handoff {
                            identity: Box::new(HandoffIdentity {
                                compound: header.reference.operation,
                                digest: header.semantic_digest,
                                claim,
                                thread: plan.request.thread,
                                recipient: plan.recipient,
                                create_key: plan.create_key,
                                invite_key: plan.invite_key,
                                send_key: plan.send_key,
                            }),
                            progress_thread: progress.and_then(|p| p.thread),
                            source: Some(source),
                            retained_completion: None,
                        })
                    }
                    SemanticMutation::HandoffDelivery(plan) => {
                        let terminal = if let Some(terminal) = terminal {
                            Some(terminal)
                        } else {
                            let name = terminal_name(&header.reference);
                            dir.read(&name, 128 * 1024)?
                                .map(|data| {
                                    bytes += data.len();
                                    read_terminal(dir, &header.reference, &data, &mut bytes)
                                })
                                .transpose()?
                        };
                        let progress = if let Some(terminal) = &terminal {
                            Some(strict::<DeliveryProgress>(&serde_json::to_vec(
                                &terminal.progress,
                            )?)?)
                        } else {
                            progress
                                .as_deref()
                                .map(strict::<DeliveryProgress>)
                                .transpose()?
                        };
                        Ok(Hint::Handoff {
                            identity: Box::new(HandoffIdentity {
                                compound: plan.payload.keys.compound,
                                digest: header.semantic_digest,
                                claim,
                                thread: plan.payload.channel.thread().cloned(),
                                recipient: plan.recipient,
                                create_key: plan.payload.keys.create,
                                invite_key: plan.payload.keys.invite,
                                send_key: plan.payload.keys.send,
                            }),
                            progress_thread: progress.and_then(|p| p.staged.thread),
                            source: Some(source),
                            retained_completion: terminal.map(|t| t.completed),
                        })
                    }
                    SemanticMutation::HandoffBootstrap(plan) => {
                        let identity = BootstrapIdentity {
                            compound: plan.payload.handoff.keys.compound.clone(),
                            scope: header.scope,
                            claim,
                            digest: header.semantic_digest,
                            payload: plan.payload,
                        };
                        identity.validate().map_err(io::Error::other)?;
                        if let Some(bytes) = &progress {
                            validate_bootstrap_progress(bytes, &identity)?;
                        }
                        Ok(Hint::Bootstrap {
                            identity: Box::new(identity),
                            source,
                        })
                    }
                    _ => Err(io::Error::other("legacy compound shape")),
                }
            })();
            match parsed {
                Ok(hint) => {
                    self.has_hints = true;
                    result.hints.push(hint);
                }
                Err(_) => self.veto = true,
            }
        }
        if !self.validate(&generation) {
            return Err(io::Error::other("legacy source generation changed"));
        }
        if !result.pending {
            self.directory = None;
            if !self.veto && !self.deciding_veto.load(Ordering::Acquire) {
                result.coverage = Some(generation);
            }
        }
        // No record cache: this constant-memory marker prevents an empty later
        // page (including an already queued final page) from forgetting failure.
        if result.hints.is_empty() && self.has_hints {
            result.hints.push(Hint::Coverage {
                veto: self.deciding_veto.clone(),
            });
        }
        Ok(result)
    }
}
fn harmless(name: &str) -> bool {
    matches!(
        name,
        "." | ".." | "allocator.lock" | "journal-format" | "next-ordinal"
    ) || name.starts_with(".intent-")
        || name.starts_with(".counter-")
        || name.starts_with(".handoff-")
        || name.starts_with(".display-")
        || name.starts_with(".delivery-terminal-")
        || ((name.starts_with("handoff-") || name.starts_with("display-"))
            && (name.ends_with(".progress") || name.ends_with(".lock")))
}

fn context(namespace: &HandoffNamespace) -> ContinuationContext {
    ContinuationContext {
        state_dir: Some(namespace.state_dir.to_string_lossy().into()),
        host: Some(namespace.host_endpoint.to_string_lossy().into()),
    }
}
fn strict<T: serde::de::DeserializeOwned + serde::Serialize>(bytes: &[u8]) -> io::Result<T> {
    let value: T = serde_json::from_slice(bytes)?;
    if serde_json::from_slice::<serde_json::Value>(bytes)? != serde_json::to_value(&value)? {
        return Err(io::Error::other("unexpected legacy progress fields"));
    }
    Ok(value)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyProgress {
    thread: Option<ThreadId>,
    #[serde(rename = "invitation")]
    _invitation: Option<crate::protocol::results::CommandResult>,
    #[serde(rename = "message")]
    _message: Option<crate::protocol::results::CommandResult>,
    #[serde(rename = "possible_start")]
    _possible_start: bool,
    #[serde(rename = "launch")]
    _launch: Option<serde_json::Value>,
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DeliveryProgress {
    staged: crate::cli::handoff::StagedWork,
    report: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    report_digest: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DeliveryTerminal {
    version: u32,
    original: String,
    completed: crate::protocol::handoff::HandoffResult,
    progress: DeliveryProgress,
    progress_digest: String,
}
fn terminal_reference(name: &str) -> io::Result<Option<crate::cli::journal::IntentRef>> {
    if !name.starts_with("delivery-") {
        return Ok(None);
    }
    let (ordinal, operation) = name
        .strip_prefix("delivery-")
        .and_then(|s| s.strip_suffix(".terminal"))
        .and_then(|s| s.split_once('-'))
        .ok_or_else(|| io::Error::other("unknown delivery record"))?;
    let ordinal: u64 = ordinal.parse().map_err(io::Error::other)?;
    let operation =
        crate::protocol::ids::OperationId::parse(operation.to_owned()).map_err(io::Error::other)?;
    let reference = crate::cli::journal::IntentRef { ordinal, operation };
    if name != terminal_name(&reference).to_string_lossy() {
        return Err(io::Error::other("legacy terminal filename mismatch"));
    }
    Ok(Some(reference))
}
fn terminal_name(reference: &crate::cli::journal::IntentRef) -> OsString {
    format!(
        "delivery-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    )
    .into()
}
fn digest(value: &impl serde::Serialize) -> io::Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn read_terminal(
    dir: &Directory,
    reference: &crate::cli::journal::IntentRef,
    data: &[u8],
    bytes: &mut usize,
) -> io::Result<DeliveryTerminal> {
    use crate::cli::journal::Journal;
    use crate::protocol::{handoff::HandoffState, results::CommandResult};
    let terminal: DeliveryTerminal = strict(data)?;
    if terminal.version != 1 || terminal.progress_digest != digest(&terminal.progress)? {
        return Err(io::Error::other("legacy delivery terminal digest mismatch"));
    }
    let pending = Journal::decode_delivery_origin(reference, terminal.original.as_bytes())?;
    let SemanticMutation::Frozen { mutation, .. } = pending.semantic else {
        return Err(io::Error::other("delivery original not frozen"));
    };
    let SemanticMutation::HandoffDelivery(plan) = *mutation else {
        return Err(io::Error::other("delivery original shape"));
    };
    let staged = &terminal.progress.staged;
    let report = terminal
        .progress
        .report
        .as_ref()
        .ok_or_else(|| io::Error::other("delivery report missing"))?;
    let participation = report["participation"]
        .as_str()
        .ok_or_else(|| io::Error::other("delivery participation missing"))?;
    if terminal.progress.report_digest.as_deref() != Some(digest(report)?.as_str())
        || terminal.completed.state != HandoffState::Completed
        || terminal.completed.compound != plan.payload.keys.compound
        || terminal.completed.thread.is_none()
        || terminal.completed.thread != staged.thread
        || plan
            .payload
            .channel
            .thread()
            .is_some_and(|thread| Some(thread) != staged.thread.as_ref())
        || !matches!(&staged.message, Some(CommandResult::MessageSent(id)) if !id.as_str().is_empty())
        || !matches!(
            participation,
            "joined" | "invited_pending" | "staged_unbound"
        )
        || match &staged.invitation {
            Some(CommandResult::Invitation(id)) => {
                !staged.invitation_attempted || id.as_str().is_empty()
            }
            Some(CommandResult::AlreadyJoined(joined)) => {
                Some(&joined.thread) != staged.thread.as_ref() || joined.seat != plan.recipient
            }
            _ => true,
        }
        || report
            != &serde_json::json!({"compound":plan.payload.keys.compound,"thread":staged.thread,"recipient":plan.recipient,"participation":participation,"outcome":"staged","invitation":staged.invitation,"message":staged.message,"recovery_ref":reference.recovery_ref()})
    {
        return Err(io::Error::other("invalid retained delivery report"));
    }
    let original_name = OsString::from(format!(
        "{:020}-{}.intent",
        reference.ordinal,
        reference.operation.as_str()
    ));
    if let Some(original) = dir.read(&original_name, RECORD_CAP)? {
        *bytes += original.len();
        if original != terminal.original.as_bytes() {
            return Err(io::Error::other("delivery original contradiction"));
        }
    }
    let progress_name =
        OsString::from(format!("handoff-{}.progress", reference.operation.as_str()));
    if let Some(progress) = dir.read(&progress_name, RECORD_CAP)? {
        *bytes += progress.len();
        let progress: DeliveryProgress = strict(&progress)?;
        if serde_json::to_value(&progress)? != serde_json::to_value(&terminal.progress)? {
            return Err(io::Error::other("delivery progress contradiction"));
        }
    }
    Ok(terminal)
}

// Read-only layout of the owner-frozen Task9 producer. This is not a writer or
// a state transition: every uncertain/version/identity/phase mismatch vetoes.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct BootstrapProgress {
    version: u32,
    identity: BootstrapIdentity,
    attempt: crate::protocol::handoff::BootstrapAttempt,
    possible_creation: bool,
    request: Option<crate::ports::CreateTabRequest>,
    creation: Option<crate::ports::CreatedTab>,
    not_submitted: bool,
}
fn validate_bootstrap_progress(bytes: &[u8], identity: &BootstrapIdentity) -> io::Result<()> {
    let progress: BootstrapProgress = strict(bytes)?;
    progress.identity.validate().map_err(io::Error::other)?;
    if progress.version != 1
        || serde_json::to_vec(&progress.identity)? != serde_json::to_vec(identity)?
        || (!progress.possible_creation
            && (progress.request.is_some()
                || progress.creation.is_some()
                || progress.not_submitted))
        || (progress.not_submitted && progress.creation.is_some())
    {
        return Err(io::Error::other(
            "legacy bootstrap progress identity or phase mismatch",
        ));
    }
    if let Some(request) = &progress.request
        && (request.workspace != identity.payload.workspace
            || request.cwd.as_os_str() != identity.payload.cwd.as_os_str()
            || request.label != identity.payload.label
            || request.focus != identity.payload.focus
            || request.env != identity.payload.env
            || request.expected_witness.endpoint.as_os_str()
                != identity.payload.handoff.namespace.host_endpoint.as_os_str())
    {
        return Err(io::Error::other("legacy bootstrap request mismatch"));
    }
    if let Some(created) = &progress.creation {
        created.validate().map_err(io::Error::other)?;
        if created.workspace != identity.payload.workspace
            || created.witness.endpoint.as_os_str()
                != identity.payload.handoff.namespace.host_endpoint.as_os_str()
            || progress.request.as_ref().is_none_or(|request| {
                request.correlation != created.correlation
                    || request.expected_witness != created.witness
            })
        {
            return Err(io::Error::other("legacy bootstrap evidence mismatch"));
        }
    }
    Ok(())
}
