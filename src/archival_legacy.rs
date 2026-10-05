//! Read-only, bounded legacy journal coverage; local records can only veto expiry.
//! Published journal records are immutable and renamed into this directory. Every
//! scan restarts on a directory generation change; no allocator or journal API is used.
use crate::{
    cli::journal::{IntentHeader, IntentScope, SemanticMutation},
    daemon::paths::InstancePaths,
    protocol::{
        handoff::HandoffIdentity, ids::ThreadId, output::ContinuationContext, results::IntentKind,
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
    time::{Duration, Instant},
};

pub const ENTRIES_PER_PAGE: usize = 32;
pub const RECORD_CAP: usize = 64 * 1024;
const HEADER_CAP: usize = 4096;
const PAGE_BYTES: usize = 256 * 1024;
pub struct Hint {
    pub identity: HandoffIdentity,
    pub progress_thread: Option<ThreadId>,
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
    directory: Option<Directory>,
    generation: Option<String>,
    veto: bool,
}
impl Source {
    pub fn new(paths: &InstancePaths, instance: String, context: ContinuationContext) -> Self {
        Self {
            root: paths.instance_dir.join("intents"),
            instance,
            context,
            directory: None,
            generation: None,
            veto: false,
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
            if !text.ends_with(".intent") {
                self.veto = true;
                continue;
            }
            // Reading the complete capped record is bounded; even a perfectly valid
            // noncompound is a coverage veto. Never trust its discriminator alone.
            let parsed = (|| {
                let data = dir
                    .read(&name, RECORD_CAP)?
                    .ok_or_else(|| io::Error::other("legacy intent vanished"))?;
                bytes += data.len();
                let split = data
                    .iter()
                    .position(|b| *b == b'\n')
                    .filter(|n| *n <= HEADER_CAP)
                    .ok_or_else(|| io::Error::other("legacy header cap"))?;
                let header: IntentHeader = serde_json::from_slice(&data[..split])?;
                if header.kind != IntentKind::Handoff {
                    return Err(io::Error::other("published noncompound coverage veto"));
                }
                if text
                    != format!(
                        "{:020}-{}.intent",
                        header.reference.ordinal,
                        header.reference.operation.as_str()
                    )
                {
                    return Err(io::Error::other("legacy filename mismatch"));
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
                let SemanticMutation::Frozen { claim, mutation } = semantic else {
                    return Err(io::Error::other("legacy compound is not frozen"));
                };
                let SemanticMutation::Handoff(plan) = *mutation else {
                    return Err(io::Error::other("legacy compound shape"));
                };
                if header.scope
                    != (IntentScope::Cooperative {
                        instance: self.instance.clone(),
                        seat: claim.seat.clone(),
                    })
                    || claim.instance != self.instance
                    || namespace(&plan.context)? != namespace(&self.context)?
                {
                    return Err(io::Error::other("legacy namespace mismatch"));
                }
                let progress_name = OsString::from(format!(
                    "handoff-{}.progress",
                    header.reference.operation.as_str()
                ));
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Progress {
                    thread: Option<ThreadId>,
                    invitation: Option<crate::protocol::results::CommandResult>,
                    message: Option<crate::protocol::results::CommandResult>,
                    possible_start: bool,
                    launch: Option<serde_json::Value>,
                }
                let progress = dir
                    .read(&progress_name, RECORD_CAP)?
                    .map(|data| {
                        bytes += data.len();
                        serde_json::from_slice::<Progress>(&data)
                    })
                    .transpose()?;
                let progress_thread = progress.and_then(|p| {
                    let _ = (p.invitation, p.message, p.possible_start, p.launch);
                    p.thread
                });
                Ok(Hint {
                    identity: HandoffIdentity {
                        compound: header.reference.operation,
                        digest: header.semantic_digest,
                        claim,
                        thread: plan.request.thread,
                        recipient: plan.recipient,
                        create_key: plan.create_key,
                        invite_key: plan.invite_key,
                        send_key: plan.send_key,
                    },
                    progress_thread,
                })
            })();
            match parsed {
                Ok(hint) => result.hints.push(hint),
                Err(_) => self.veto = true,
            }
        }
        if !self.validate(&generation) {
            return Err(io::Error::other("legacy source generation changed"));
        }
        if !result.pending {
            self.directory = None;
            if !self.veto {
                result.coverage = Some(generation);
            }
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
        || ((name.starts_with("handoff-") || name.starts_with("display-"))
            && (name.ends_with(".progress") || name.ends_with(".lock")))
}
