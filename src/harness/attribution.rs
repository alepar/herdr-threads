//! Harness version attribution from the payload's transcript (ht-xoc.2).
//!
//! The running harness writes its own version into the session transcript the
//! hook payload points at (`transcript_path`): Claude puts `version` on every
//! conversation entry, Codex puts `cli_version` in `session_meta` records.
//! Reading it is in-process and bounded: no exec, no process walk, no
//! inode/mtime comparison, and at most [`MAX_WINDOW`] bytes of the file (plus
//! one head line for a capped Codex rollout). The result is the canonical
//! [`normalize_version`] string, or a stable [`Unattributed`] reason.
//!
//! Evidence: `docs/compatibility/harness-transcript-version.md`. A resumed
//! Claude session appends to the old file, so a `SessionStart` with source
//! `resume` is never attributed from it ([`Unattributed::ResumeBeforeFirstEntry`]);
//! the caller buffers it until the session's first attributed event. The
//! note's Codex finding (a resumed rollout keeps the creator's `session_meta`)
//! does not change this reader: it returns the newest `session_meta` it can
//! see, and resume is excluded up front by the same payload rule. The note's
//! kill criterion (a) (a newest `session_meta` beyond the 1 MB window) was not
//! observed, so the backward tail scan is used, with a head-line fallback.
//!
//! Inert: nothing calls these functions yet (consumed by ht-xoc.4).
use super::contract::normalize_version;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Result of reading a harness version from a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribution {
    Attributed {
        harness: &'static str,
        version: String,
    },
    Unattributable {
        reason: Unattributed,
    },
}

/// Why a transcript yielded no version. [`Unattributed::as_str`] values are
/// stable: they go on the wire and into doctor output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unattributed {
    NoTranscriptPath,
    ResumeBeforeFirstEntry,
    Absent,
    Unreadable,
    NoVersionField,
    WindowExceeded,
    UnrecognizedVersion,
}

impl Unattributed {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoTranscriptPath => "no transcript path in the payload",
            Self::ResumeBeforeFirstEntry => "resume before first entry",
            Self::Absent => "transcript not found",
            Self::Unreadable => "transcript unreadable",
            Self::NoVersionField => "no version field in the transcript",
            Self::WindowExceeded => "no version within the 1 MB read window",
            Self::UnrecognizedVersion => "transcript version not recognized",
        }
    }
}

/// First tail window; it grows by this step.
pub const INITIAL_WINDOW: u64 = 64 * 1024;
/// Hard cap on the tail window.
pub const MAX_WINDOW: u64 = 1024 * 1024;
/// Chunk size when reading a Codex head line.
const HEAD_CHUNK: usize = 4096;

fn unattributable(reason: Unattributed) -> Attribution {
    Attribution::Unattributable { reason }
}

fn known_harness(harness: &str) -> Option<&'static str> {
    match harness {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}

/// Attribute a hook payload: a `SessionStart` with source `resume` is never
/// read; otherwise `transcript_path` must be a non-empty absolute path.
pub fn attribute_payload(harness: &str, payload: &Value) -> Attribution {
    if known_harness(harness).is_none() {
        return unattributable(Unattributed::UnrecognizedVersion);
    }
    let str_field = |k: &str| payload.get(k).and_then(Value::as_str);
    if str_field("hook_event_name") == Some("SessionStart") && str_field("source") == Some("resume")
    {
        return unattributable(Unattributed::ResumeBeforeFirstEntry);
    }
    match str_field("transcript_path") {
        Some(p) if !p.is_empty() && Path::new(p).is_absolute() => {
            attribute_transcript(harness, Path::new(p))
        }
        _ => unattributable(Unattributed::NoTranscriptPath),
    }
}

/// Attribute the transcript file at `path`.
pub fn attribute_transcript(harness: &str, path: &Path) -> Attribution {
    if known_harness(harness).is_none() {
        return unattributable(Unattributed::UnrecognizedVersion);
    }
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return unattributable(Unattributed::Absent);
        }
        Err(_) => return unattributable(Unattributed::Unreadable),
    };
    let len = match file.metadata() {
        Ok(m) if m.is_file() => m.len(),
        _ => return unattributable(Unattributed::Unreadable),
    };
    attribute_reader(harness, &mut file, len)
}

/// Attribute a transcript of `len` bytes behind `r` (generic over the reader
/// so tests can count the bytes read).
pub fn attribute_reader<R: Read + Seek>(harness: &str, r: &mut R, len: u64) -> Attribution {
    let Some(name) = known_harness(harness) else {
        return unattributable(Unattributed::UnrecognizedVersion);
    };
    let extract: fn(&Value) -> Option<&str> = match name {
        "claude" => claude_version,
        _ => codex_version,
    };
    let (found, reached_start) = match scan_tail(r, len, extract) {
        Ok(v) => v,
        Err(_) => return unattributable(Unattributed::Unreadable),
    };
    let raw = match found {
        Some(raw) => raw,
        None if name == "codex" && !reached_start => match read_head_line(r) {
            Ok(Head::Line(line)) => match serde_json::from_slice::<Value>(&line)
                .ok()
                .as_ref()
                .and_then(codex_version)
            {
                Some(raw) => raw.to_owned(),
                None => return unattributable(Unattributed::NoVersionField),
            },
            Ok(Head::TooLong) => return unattributable(Unattributed::WindowExceeded),
            Err(_) => return unattributable(Unattributed::Unreadable),
        },
        None => {
            return unattributable(if len > MAX_WINDOW {
                Unattributed::WindowExceeded
            } else {
                Unattributed::NoVersionField
            });
        }
    };
    match normalize_version(name, &raw) {
        Some(version) => Attribution::Attributed {
            harness: name,
            version,
        },
        None => unattributable(Unattributed::UnrecognizedVersion),
    }
}

fn claude_version(entry: &Value) -> Option<&str> {
    entry.get("version")?.as_str()
}

fn codex_version(entry: &Value) -> Option<&str> {
    if entry.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    entry.get("payload")?.get("cli_version")?.as_str()
}

/// Newest complete line whose JSON `extract`s a string, reading the last
/// `INITIAL_WINDOW` bytes and growing by that step up to `MAX_WINDOW`. Only
/// the newly needed prefix is read on each growth, so total bytes read equal
/// the final window. The flag is true when the window reached offset 0.
fn scan_tail<R: Read + Seek>(
    r: &mut R,
    len: u64,
    extract: fn(&Value) -> Option<&str>,
) -> std::io::Result<(Option<String>, bool)> {
    let cap = len.min(MAX_WINDOW);
    let mut window = INITIAL_WINDOW.min(cap);
    let mut buf = read_range(r, len - window, window)?;
    loop {
        let start = len - window;
        if let Some(raw) = newest_in(&buf, start > 0, extract) {
            return Ok((Some(raw), start == 0));
        }
        if window >= cap {
            return Ok((None, start == 0));
        }
        let grown = (window + INITIAL_WINDOW).min(cap);
        let mut next = read_range(r, len - grown, grown - window)?;
        next.extend_from_slice(&buf);
        buf = next;
        window = grown;
    }
}

fn read_range<R: Read + Seek>(r: &mut R, offset: u64, n: u64) -> std::io::Result<Vec<u8>> {
    r.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; n as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

/// Scan the complete lines of `buf` newest first. The last piece after the
/// final `\n` is dropped (empty, or a line still being written) and so is the
/// first when the buffer starts mid-file (it may be cut). Lines that are not
/// JSON, or carry no field `extract` wants, are skipped.
fn newest_in(
    buf: &[u8],
    starts_mid_file: bool,
    extract: fn(&Value) -> Option<&str>,
) -> Option<String> {
    let mut pieces: Vec<&[u8]> = buf.split(|b| *b == b'\n').collect();
    pieces.pop();
    if starts_mid_file && !pieces.is_empty() {
        pieces.remove(0);
    }
    pieces
        .into_iter()
        .rev()
        .filter(|line| !line.is_empty())
        .find_map(|line| {
            let value: Value = serde_json::from_slice(line).ok()?;
            extract(&value).map(str::to_owned)
        })
}

enum Head {
    Line(Vec<u8>),
    TooLong,
}

/// The first line of the file, read in small chunks up to `MAX_WINDOW`.
fn read_head_line<R: Read + Seek>(r: &mut R) -> std::io::Result<Head> {
    r.seek(SeekFrom::Start(0))?;
    let mut line = Vec::new();
    let mut chunk = [0u8; HEAD_CHUNK];
    while (line.len() as u64) < MAX_WINDOW {
        let n = r.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if let Some(i) = chunk[..n].iter().position(|b| *b == b'\n') {
            line.extend_from_slice(&chunk[..i]);
            return Ok(Head::Line(line));
        }
        line.extend_from_slice(&chunk[..n]);
    }
    Ok(Head::TooLong)
}
