//! Harness version attribution from the payload's transcript (ht-xoc.2).
//!
//! Claude writes runtime metadata in the payload transcript: newest `version`
//! is optional current-runtime attribution. Codex `session_meta.cli_version`
//! names only the rollout creator; the reader is an optional diagnostic API,
//! never production current-runtime attribution, including fresh startup.
//! Reading it is in-process and bounded: no exec, no process walk, no
//! inode/mtime comparison, and at most [`MAX_WINDOW`] bytes of the file (plus at
//! most [`LINE_EXTENSION`] to finish one line straddling the window's start). Only a
//! regular file is opened, non-blocking and without following a symlink (a FIFO
//! or a symlink is [`Unattributed::Unreadable`], never a hang). The result is the canonical
//! [`normalize_version`] string, or a stable [`Unattributed`] reason.
//!
//! Evidence: `docs/compatibility/harness-transcript-version.md`. Claude reads
//! the newest versioned entry by a bounded backward scan. Codex reads only the
//! rollout's head `session_meta` (its first line): a resumed Codex rollout
//! keeps its creator's single `session_meta` (spike note), so a Codex session
//! known to be resumed is never attributed (the hook's gate marks it;
//! [`Unattributed::CodexResumed`]). A Claude resume `SessionStart` is
//! [`Unattributed::ResumeBeforeFirstEntry`] and is buffered by the daemon.
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
    CodexResumed,
    CodexCreatorOnly,
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
            Self::CodexCreatorOnly => {
                "codex rollout creator metadata does not identify the current runtime"
            }
            Self::CodexResumed => "codex resume: rollout version is the creating CLI's",
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
/// Hard cap on the tail window, plus at most [`LINE_EXTENSION`] to finish a
/// line that straddles the window's start.
pub const MAX_WINDOW: u64 = 1024 * 1024;
/// Most bytes read before the window's start to find where a straddling line
/// begins; a longer line is [`Unattributed::WindowExceeded`].
pub const LINE_EXTENSION: u64 = INITIAL_WINDOW;
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

/// Claude payloads may name runtime-written metadata in a nonempty absolute
/// transcript path. Codex payloads always lack current-runtime attribution:
/// the optional transcript reader names only the rollout creator.
pub fn attribute_payload(harness: &str, payload: &Value) -> Attribution {
    if known_harness(harness).is_none() {
        return unattributable(Unattributed::UnrecognizedVersion);
    }
    let str_field = |k: &str| payload.get(k).and_then(Value::as_str);
    if str_field("hook_event_name") == Some("SessionStart") && str_field("source") == Some("resume")
    {
        return unattributable(if harness == "codex" {
            Unattributed::CodexResumed
        } else {
            Unattributed::ResumeBeforeFirstEntry
        });
    }
    if harness == "codex" {
        return unattributable(Unattributed::CodexCreatorOnly);
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
    use std::os::unix::fs::OpenOptionsExt;
    if known_harness(harness).is_none() {
        return unattributable(Unattributed::UnrecognizedVersion);
    }
    // Never open what is not a regular file: a FIFO, socket or device would
    // block or misbehave, and a symlink is not followed (ht-rlv.1).
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return unattributable(Unattributed::Absent);
        }
        _ => return unattributable(Unattributed::Unreadable),
    }
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
    {
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
    let raw = if name == "codex" {
        match read_head_line(r) {
            Ok(Head::Line(line)) => match serde_json::from_slice::<Value>(&line)
                .ok()
                .as_ref()
                .and_then(codex_version)
            {
                Some(raw) => raw.to_owned(),
                None => return unattributable(Unattributed::NoVersionField),
            },
            Ok(Head::Incomplete) => return unattributable(Unattributed::NoVersionField),
            Ok(Head::TooLong) => return unattributable(Unattributed::WindowExceeded),
            Err(_) => return unattributable(Unattributed::Unreadable),
        }
    } else {
        match scan_tail(r, len, claude_version) {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                return unattributable(if len > MAX_WINDOW {
                    Unattributed::WindowExceeded
                } else {
                    Unattributed::NoVersionField
                });
            }
            Err(_) => return unattributable(Unattributed::Unreadable),
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
/// `INITIAL_WINDOW` bytes and growing by that step up to `MAX_WINDOW`. Each
/// growth step reads only the newly needed prefix and parses only the lines
/// that prefix completes, so every line is parsed at most once.
///
/// `head` is the bytes from the scanned region's start up to its first `\n`:
/// a line that may begin earlier in the file. `head_ends` says a `\n` ends
/// it; when false the region holds no newline yet and `head` is the file's
/// unfinished trailing line, never parsed. At the window's start (`len -
/// MAX_WINDOW`) `head` is the line straddling it: the scan reads back at most
/// [`LINE_EXTENSION`] more to find its start and parses only that line.
///
/// Returns the found string and the number of JSON parses attempted.
pub(crate) fn scan_tail_counted<R: Read + Seek>(
    r: &mut R,
    len: u64,
    extract: fn(&Value) -> Option<&str>,
) -> std::io::Result<(Option<String>, usize)> {
    let floor = len.saturating_sub(MAX_WINDOW);
    let hard_floor = floor.saturating_sub(LINE_EXTENSION);
    let (mut start, mut head, mut head_ends, mut parses) = (len, Vec::new(), false, 0usize);
    let parse = |line: &[u8], parses: &mut usize| -> Option<String> {
        if line.is_empty() {
            return None;
        }
        *parses += 1;
        let value: Value = serde_json::from_slice(line).ok()?;
        extract(&value).map(str::to_owned)
    };
    loop {
        if start == 0 {
            // `head` starts at offset 0: a whole line when a '\n' ends it.
            let found = if head_ends {
                parse(&head, &mut parses)
            } else {
                None
            };
            return Ok((found, parses));
        }
        let extending = start <= floor;
        if extending && !head_ends {
            return Ok((None, parses));
        }
        let bottom = if extending { hard_floor } else { floor };
        let next = bottom.max(start.saturating_sub(INITIAL_WINDOW));
        if next == start {
            return Ok((None, parses)); // extension exhausted
        }
        let mut region = read_range(r, next, start - next)?;
        region.extend_from_slice(&head);
        start = next;
        let Some(first) = region.iter().position(|b| *b == b'\n') else {
            head = region; // still one unfinished (or cut) line
            continue;
        };
        let mut pieces: Vec<&[u8]> = region[first + 1..].split(|b| *b == b'\n').collect();
        // The last piece is the line after the region's last '\n'. It ends at
        // the old head's newline only when `head_ends`; otherwise it is the
        // file's unfinished trailing line.
        let last = pieces.pop();
        if extending {
            // Only finish the straddling line (the old head), nothing older.
            let line = last.filter(|_| head_ends);
            return Ok((line.and_then(|l| parse(l, &mut parses)), parses));
        }
        if head_ends
            && let Some(line) = last
            && let Some(found) = parse(line, &mut parses)
        {
            return Ok((Some(found), parses));
        }
        for line in pieces.into_iter().rev() {
            if let Some(found) = parse(line, &mut parses) {
                return Ok((Some(found), parses));
            }
        }
        head = region[..first].to_vec();
        head_ends = true;
    }
}

fn scan_tail<R: Read + Seek>(
    r: &mut R,
    len: u64,
    extract: fn(&Value) -> Option<&str>,
) -> std::io::Result<Option<String>> {
    scan_tail_counted(r, len, extract).map(|(found, _)| found)
}

fn read_range<R: Read + Seek>(r: &mut R, offset: u64, n: u64) -> std::io::Result<Vec<u8>> {
    r.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; n as usize];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

enum Head {
    Line(Vec<u8>),
    /// EOF before a newline: empty file, or a line still being written.
    Incomplete,
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
            return Ok(Head::Incomplete);
        }
        if let Some(i) = chunk[..n].iter().position(|b| *b == b'\n') {
            line.extend_from_slice(&chunk[..i]);
            return Ok(Head::Line(line));
        }
        line.extend_from_slice(&chunk[..n]);
    }
    Ok(Head::TooLong)
}
