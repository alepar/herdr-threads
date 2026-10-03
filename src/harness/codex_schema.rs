//! Codex hook-schema fingerprint: the draft-07 JSON Schemas titled
//! `<event>.command.input` / `<event>.command.output` that a Codex binary
//! embeds as text, extracted, canonicalized and hashed.
//!
//! The fingerprint admits an installed Codex version that no recipe lists
//! when its embedded hook contract is byte-for-byte (after canonicalization)
//! the one a verified recipe was captured against. The extraction follows the
//! static probe in `codex-158-hook-capture/independent-schema-check.py`:
//! every embedded object whose `$schema` is draft-07 and whose `title` is a
//! hook command schema is parsed as JSON; the canonical form (object keys
//! sorted, no insignificant whitespace) of each is hashed together with its
//! title, in title order.
//!
//! Extraction is bounded by the caller's deadline (the version-observation
//! deadline) and a size cap, and its result is cached per binary identity:
//! canonical path plus `dev`/`ino`/size/mtime/ctime, recording the binary's
//! SHA-256. The in-process cache always applies; a caller that owns a private
//! state directory may also pass a persistent cache file, so one-shot hook
//! processes do not rescan an unchanged binary.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

/// Largest binary scanned. Codex 0.159.2 is about 240 MB.
pub const MAX_BINARY_BYTES: u64 = 1 << 30;
const READ_CHUNK: usize = 8 << 20;
const DRAFT_07: &str = "http://json-schema.org/draft-07/schema#";
/// A persistent cache file larger than this is ignored.
const MAX_CACHE_FILE: u64 = 64 << 10;
const MAX_CACHE_ENTRIES: usize = 16;

/// Why no fingerprint could be taken. Every variant refuses admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unextractable {
    /// The binary could not be opened or read.
    Io,
    /// Larger than [`MAX_BINARY_BYTES`].
    TooLarge,
    /// The observation deadline passed before extraction finished.
    Deadline,
    /// The binary changed while it was being read.
    Changed,
    /// No embedded hook command schema was found.
    NoSchemas,
    /// One title is embedded with two different schemas.
    Conflicting(String),
    /// The npm JS wrapper has more than one sibling platform vendor binary,
    /// so which one runs is not known.
    SeveralVendorBinaries(usize),
}

impl fmt::Display for Unextractable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io => f.write_str("binary unreadable"),
            Self::TooLarge => write!(f, "binary larger than {MAX_BINARY_BYTES} bytes"),
            Self::Deadline => f.write_str("observation deadline passed"),
            Self::Changed => f.write_str("binary changed while being read"),
            Self::NoSchemas => f.write_str("no embedded hook command schemas found"),
            Self::Conflicting(title) => write!(f, "conflicting embedded schemas for {title}"),
            Self::SeveralVendorBinaries(count) => write!(
                f,
                "{count} platform vendor binaries beside the npm codex wrapper; cannot tell which one runs"
            ),
        }
    }
}

/// The measured hook contract of one binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFingerprint {
    /// Lowercase hex SHA-256 of the whole binary.
    pub binary_sha256: String,
    /// `sha256:<hex>` over the canonical titled schema set.
    pub fingerprint: String,
    /// Number of distinct hook command schemas found.
    pub schemas: usize,
    /// The identity of the fingerprinted file.
    pub identity: BinaryIdentity,
}

/// Canonical JSON: object keys sorted by their UTF-8 bytes, no whitespace,
/// strings and numbers as serde_json writes them. Independent of whether
/// serde_json preserves insertion order.
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(item, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// `<kebab-event>.command.input|output`.
fn is_hook_title(title: &str) -> bool {
    let Some(event) = title
        .strip_suffix(".command.input")
        .or_else(|| title.strip_suffix(".command.output"))
    else {
        return false;
    };
    !event.is_empty()
        && event.len() <= 64
        && event.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    let rest = &haystack[from..];
    // SAFETY: both pointers and lengths describe live, in-bounds slices;
    // memmem only reads them.
    let found = unsafe {
        libc::memmem(
            rest.as_ptr().cast(),
            rest.len(),
            needle.as_ptr().cast(),
            needle.len(),
        )
    };
    if found.is_null() {
        None
    } else {
        Some(from + (found as usize - rest.as_ptr() as usize))
    }
}

fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// Every embedded draft-07 hook command schema, as title -> canonical JSON.
/// Identical duplicates collapse; a title embedded with two different
/// schemas is [`Unextractable::Conflicting`].
pub fn extract_schemas(
    bytes: &[u8],
    deadline: Instant,
) -> Result<BTreeMap<String, String>, Unextractable> {
    let mut schemas = BTreeMap::new();
    let mut at = 0;
    while let Some(anchor) = find(bytes, b"\"$schema\"", at) {
        at = anchor + 1;
        if Instant::now() >= deadline {
            return Err(Unextractable::Deadline);
        }
        // `"$schema"` must be the first key of an object whose value is the
        // draft-07 URI.
        let colon = skip_ws(bytes, anchor + b"\"$schema\"".len());
        if bytes.get(colon) != Some(&b':') {
            continue;
        }
        let uri = skip_ws(bytes, colon + 1);
        let quoted = format!("\"{DRAFT_07}\"");
        if !bytes[uri..].starts_with(quoted.as_bytes()) {
            continue;
        }
        let mut open = anchor;
        while open > 0 && bytes[open - 1].is_ascii_whitespace() {
            open -= 1;
        }
        if open == 0 || bytes[open - 1] != b'{' {
            continue;
        }
        open -= 1;
        let Some(Ok(value)) = serde_json::Deserializer::from_slice(&bytes[open..])
            .into_iter::<Value>()
            .next()
        else {
            continue;
        };
        let Some(title) = value.get("title").and_then(Value::as_str) else {
            continue;
        };
        if !is_hook_title(title) {
            continue;
        }
        let canonical = canonical_json(&value);
        match schemas.get(title) {
            Some(existing) if existing != &canonical => {
                return Err(Unextractable::Conflicting(title.to_owned()));
            }
            Some(_) => (),
            None => {
                schemas.insert(title.to_owned(), canonical);
            }
        }
    }
    if schemas.is_empty() {
        return Err(Unextractable::NoSchemas);
    }
    Ok(schemas)
}

/// `sha256:<hex>` over `title \n canonical \n` for each schema in title order.
pub fn fingerprint(schemas: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (title, canonical) in schemas {
        hasher.update(title.as_bytes());
        hasher.update(b"\n");
        hasher.update(canonical.as_bytes());
        hasher.update(b"\n");
    }
    format!("sha256:{}", hex(&hasher.finalize()))
}

/// Fingerprint of loose schema documents (one schema per slice), for
/// deriving a recipe's fingerprint from committed schema files.
pub fn fingerprint_documents<'a>(
    documents: impl IntoIterator<Item = &'a [u8]>,
) -> Result<String, Unextractable> {
    let mut joined = Vec::new();
    for document in documents {
        joined.extend_from_slice(document);
        joined.push(b'\n');
    }
    let far = Instant::now() + std::time::Duration::from_secs(3600);
    extract_schemas(&joined, far).map(|schemas| fingerprint(&schemas))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Identity of a binary file: canonical path and metadata. It keys the
/// fingerprint cache, and lets a caller check that the binary it ran is the
/// binary that was fingerprinted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BinaryIdentity {
    path: PathBuf,
    dev: u64,
    ino: u64,
    size: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}

impl BinaryIdentity {
    /// The canonical path the identity names.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The identity of the file `path` resolves to, if it is a regular file.
    pub fn observe(path: &Path) -> Option<Self> {
        let canonical = std::fs::canonicalize(path).ok()?;
        let meta = std::fs::metadata(&canonical).ok()?;
        meta.is_file().then(|| Self::of(&canonical, &meta))
    }

    fn of(path: &Path, meta: &std::fs::Metadata) -> Self {
        Self {
            path: path.to_owned(),
            dev: meta.dev(),
            ino: meta.ino(),
            size: meta.len(),
            mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
            ctime_ns: i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    key: BinaryIdentity,
    binary_sha256: String,
    fingerprint: String,
    schemas: usize,
}

impl CacheEntry {
    fn result(&self) -> BinaryFingerprint {
        BinaryFingerprint {
            binary_sha256: self.binary_sha256.clone(),
            fingerprint: self.fingerprint.clone(),
            schemas: self.schemas,
            identity: self.key.clone(),
        }
    }
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct CacheFile {
    version: u32,
    entries: Vec<CacheEntry>,
}

static MEMORY: Mutex<Vec<CacheEntry>> = Mutex::new(Vec::new());

fn memory_lookup(key: &BinaryIdentity) -> Option<BinaryFingerprint> {
    let cache = MEMORY.lock().ok()?;
    cache
        .iter()
        .find(|entry| &entry.key == key)
        .map(CacheEntry::result)
}

fn memory_store(entry: &CacheEntry) {
    if let Ok(mut cache) = MEMORY.lock() {
        cache.retain(|old| old.key.path != entry.key.path);
        if cache.len() >= MAX_CACHE_ENTRIES {
            cache.remove(0);
        }
        cache.push(entry.clone());
    }
}

fn read_cache_file(path: &Path) -> CacheFile {
    let read = || -> Option<CacheFile> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        // Only a private regular file this user owns is trusted: a cache hit
        // admits without rescanning, so a foreign or group/other-writable
        // file is ignored (and replaced on the next write).
        if !meta.is_file()
            || meta.len() > MAX_CACHE_FILE
            || meta.uid() != crate::daemon::paths::effective_uid()
            || meta.permissions().mode() & 0o077 != 0
        {
            return None;
        }
        let file: CacheFile = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        (file.version == 1).then_some(file)
    };
    read().unwrap_or(CacheFile {
        version: 1,
        entries: Vec::new(),
    })
}

fn write_cache_file(path: &Path, entry: &CacheEntry) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = read_cache_file(path);
    file.entries.retain(|old| old.key.path != entry.key.path);
    while file.entries.len() >= MAX_CACHE_ENTRIES {
        file.entries.remove(0);
    }
    file.entries.push(entry.clone());
    let Ok(bytes) = serde_json::to_vec(&file) else {
        return;
    };
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut out| out.write_all(&bytes).and_then(|()| out.sync_all()));
    if written.is_err() || std::fs::rename(&temporary, path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// Where a fingerprint may be cached beyond this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FingerprintCache<'a> {
    /// In-process only.
    Memory,
    /// Consult a persistent cache file but never write it (diagnostics).
    ReadOnly(&'a Path),
    /// Consult and update a persistent cache file (0600, bounded) in a
    /// caller-owned private directory.
    ReadWrite(&'a Path),
}

impl<'a> FingerprintCache<'a> {
    fn file(self) -> Option<&'a Path> {
        match self {
            Self::Memory => None,
            Self::ReadOnly(path) | Self::ReadWrite(path) => Some(path),
        }
    }
}

/// Whether `canonical` is the npm JS wrapper (`codex.js`) rather than a
/// native binary: by name, or by a `#!/usr/bin/env node` first line.
fn is_npm_wrapper(canonical: &Path) -> bool {
    if canonical.file_name().is_some_and(|name| name == "codex.js") {
        return true;
    }
    const SHEBANG: &[u8] = b"#!/usr/bin/env node";
    let mut head = [0u8; SHEBANG.len()];
    File::open(canonical)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && head == SHEBANG
}

/// The file whose embedded schemas describe the Codex at `binary`: the
/// binary itself, or, when it resolves to the npm JS wrapper
/// (`<prefix>/node_modules/@openai/codex/bin/codex.js`), the single sibling
/// `<prefix>/node_modules/@openai/codex-<platform>/vendor/<triple>/bin/codex`
/// native binary of the same package version. No vendor sibling is
/// [`Unextractable::NoSchemas`] (the wrapper embeds none); several is
/// [`Unextractable::SeveralVendorBinaries`].
pub fn fingerprint_target(binary: &Path) -> Result<PathBuf, Unextractable> {
    let canonical = std::fs::canonicalize(binary).map_err(|_| Unextractable::Io)?;
    if !is_npm_wrapper(&canonical) {
        return Ok(canonical);
    }
    // <scope>/codex/bin/codex.js: the package root is two levels up.
    let scope = canonical
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or(Unextractable::NoSchemas)?;
    let mut found = Vec::new();
    let packages = std::fs::read_dir(scope).map_err(|_| Unextractable::NoSchemas)?;
    for package in packages.flatten() {
        if !package.file_name().to_string_lossy().starts_with("codex-") {
            continue;
        }
        let Ok(triples) = std::fs::read_dir(package.path().join("vendor")) else {
            continue;
        };
        for triple in triples.flatten() {
            let candidate = triple.path().join("bin").join("codex");
            if std::fs::metadata(&candidate).is_ok_and(|meta| meta.is_file()) {
                found.push(candidate);
            }
        }
    }
    match found.len() {
        0 => Err(Unextractable::NoSchemas),
        1 => Ok(found.remove(0)),
        count => Err(Unextractable::SeveralVendorBinaries(count)),
    }
}

/// Read, hash and fingerprint `binary` before `deadline`, reusing a cached
/// result for the same binary identity. `cache_file`, if given, is a
/// persistent cache in a caller-owned private directory; it is best effort.
pub fn fingerprint_binary(
    binary: &Path,
    deadline: Instant,
    cache_file: Option<&Path>,
) -> Result<BinaryFingerprint, Unextractable> {
    fingerprint_binary_with(
        binary,
        deadline,
        cache_file.map_or(FingerprintCache::Memory, FingerprintCache::ReadWrite),
    )
}

/// [`fingerprint_binary`] with an explicit cache access mode.
pub fn fingerprint_binary_with(
    binary: &Path,
    deadline: Instant,
    cache: FingerprintCache<'_>,
) -> Result<BinaryFingerprint, Unextractable> {
    let canonical =
        std::fs::canonicalize(fingerprint_target(binary)?).map_err(|_| Unextractable::Io)?;
    let before = std::fs::metadata(&canonical).map_err(|_| Unextractable::Io)?;
    if !before.is_file() {
        return Err(Unextractable::Io);
    }
    if before.len() > MAX_BINARY_BYTES {
        return Err(Unextractable::TooLarge);
    }
    let key = BinaryIdentity::of(&canonical, &before);
    if let Some(hit) = memory_lookup(&key) {
        return Ok(hit);
    }
    if let Some(path) = cache.file()
        && let Some(entry) = read_cache_file(path)
            .entries
            .into_iter()
            .find(|entry| entry.key == key)
    {
        memory_store(&entry);
        return Ok(entry.result());
    }
    let mut file = File::open(&canonical).map_err(|_| Unextractable::Io)?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; READ_CHUNK];
    loop {
        if Instant::now() >= deadline {
            return Err(Unextractable::Deadline);
        }
        let read = file.read(&mut chunk).map_err(|_| Unextractable::Io)?;
        if read == 0 {
            break;
        }
        if (bytes.len() + read) as u64 > MAX_BINARY_BYTES {
            return Err(Unextractable::TooLarge);
        }
        hasher.update(&chunk[..read]);
        bytes.extend_from_slice(&chunk[..read]);
    }
    let after = file.metadata().map_err(|_| Unextractable::Io)?;
    let path_after = std::fs::metadata(&canonical).map_err(|_| Unextractable::Io)?;
    if BinaryIdentity::of(&canonical, &after) != key
        || BinaryIdentity::of(&canonical, &path_after) != key
        || bytes.len() as u64 != key.size
    {
        return Err(Unextractable::Changed);
    }
    let schemas = extract_schemas(&bytes, deadline)?;
    let entry = CacheEntry {
        key,
        binary_sha256: hex(&hasher.finalize()),
        fingerprint: fingerprint(&schemas),
        schemas: schemas.len(),
    };
    memory_store(&entry);
    if let FingerprintCache::ReadWrite(path) = cache {
        write_cache_file(path, &entry);
    }
    Ok(entry.result())
}

/// Test-only: forget in-process cache entries.
#[cfg(test)]
pub(crate) fn clear_memory_cache_for_test() {
    if let Ok(mut cache) = MEMORY.lock() {
        cache.clear();
    }
}

/// Test-only: forget the in-process entries for the file `binary` resolves
/// to, leaving other tests' entries in place (a fresh process's view of that
/// binary).
#[cfg(test)]
pub(crate) fn forget_memory_entry_for_test(binary: &Path) {
    let canonical = std::fs::canonicalize(binary).unwrap_or_else(|_| binary.to_path_buf());
    if let Ok(mut cache) = MEMORY.lock() {
        cache.retain(|entry| entry.key.path != canonical);
    }
}
