use super::attribution::{
    Attribution, INITIAL_WINDOW, LINE_EXTENSION, MAX_WINDOW, Unattributed, attribute_payload,
    attribute_reader, attribute_transcript, scan_tail_counted,
};
use serde_json::{Value, json};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/transcripts")
        .join(name)
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "herdr-threads-attribution-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A reader that counts the bytes handed out.
struct Counting {
    inner: Cursor<Vec<u8>>,
    read: u64,
}

impl Counting {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            inner: Cursor::new(bytes),
            read: 0,
        }
    }
    fn len(&self) -> u64 {
        self.inner.get_ref().len() as u64
    }
}

impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl Seek for Counting {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

fn attributed(harness: &'static str, version: &str) -> Attribution {
    Attribution::Attributed {
        harness,
        version: version.to_owned(),
    }
}

fn unattributable(reason: Unattributed) -> Attribution {
    Attribution::Unattributable { reason }
}

fn versioned_claude_line(version: &str) -> String {
    format!("{{\"type\":\"assistant\",\"version\":\"{version}\",\"cwd\":\"~/proj\"}}\n")
}

/// `n` bytes (rounded up to whole lines) of versionless `mode` records.
fn versionless_lines(n: usize) -> String {
    let line = "{\"type\":\"mode\",\"mode\":\"default\"}\n";
    line.repeat(n / line.len() + 1)
}

/// Complete versionless JSON lines totalling exactly `n` bytes (`n >= 32`):
/// 35-byte lines, the last one padded to absorb the remainder.
fn filler(n: usize) -> String {
    const LINE: usize = 35;
    assert!(n >= 32, "filler needs at least one 32-byte line");
    let line = |len: usize| {
        let overhead = "{\"type\":\"mode\",\"pad\":\"\"}\n".len();
        format!(
            "{{\"type\":\"mode\",\"pad\":\"{}\"}}\n",
            "p".repeat(len - overhead)
        )
    };
    let mut out = String::with_capacity(n);
    let mut left = n;
    while left >= 32 + LINE {
        out.push_str(&line(LINE));
        left -= LINE;
    }
    out.push_str(&line(left));
    assert_eq!(out.len(), n);
    out
}

/// A versioned Claude line of exactly `n` bytes, padded by a `pad` field.
fn versioned_line_of(version: &str, n: usize) -> String {
    let bare = versioned_claude_line(version).len() + ",\"pad\":\"\"".len();
    assert!(n >= bare);
    let line = format!(
        "{{\"type\":\"assistant\",\"version\":\"{version}\",\"cwd\":\"~/proj\",\"pad\":\"{}\"}}\n",
        "p".repeat(n - bare)
    );
    assert_eq!(line.len(), n);
    line
}

fn claude_version_of(entry: &Value) -> Option<&str> {
    entry.get("version")?.as_str()
}

#[test]
fn version_line_straddling_the_window_boundary_attributes() {
    let body = format!(
        "{}{}{}",
        filler(4096),
        versioned_line_of("2.1.286", 200),
        filler(MAX_WINDOW as usize - 100)
    );
    let mut r = Counting::new(body.into_bytes());
    let len = r.len();
    assert_eq!(len, MAX_WINDOW + 4196);
    assert_eq!(
        attribute_reader("claude", &mut r, len),
        attributed("claude", "2.1.286")
    );
}

#[test]
fn line_longer_than_the_extension_at_the_boundary_is_window_exceeded() {
    let body = format!(
        "{}{}{}",
        filler(4096),
        versioned_line_of("2.1.286", 200 * 1024),
        filler(MAX_WINDOW as usize - 100)
    );
    let mut r = Counting::new(body.into_bytes());
    let len = r.len();
    assert_eq!(
        attribute_reader("claude", &mut r, len),
        unattributable(Unattributed::WindowExceeded)
    );
    assert!(r.read <= MAX_WINDOW + LINE_EXTENSION, "read {}", r.read);
}

#[test]
fn each_line_is_parsed_once_across_growth_steps() {
    let body = format!("{}{}", versioned_claude_line("2.1.286"), filler(900 * 1024));
    let bytes = body.into_bytes();
    let len = bytes.len() as u64;
    let window = len.min(MAX_WINDOW) as usize;
    let lines_in_window = bytes[bytes.len() - window..]
        .iter()
        .filter(|b| **b == b'\n')
        .count();
    let (found, parses) =
        scan_tail_counted(&mut Cursor::new(bytes), len, claude_version_of).unwrap();
    assert_eq!(found.as_deref(), Some("2.1.286"));
    assert!(
        parses <= lines_in_window + 2,
        "{parses} parses for {lines_in_window} lines"
    );
}

#[test]
fn claude_fresh_transcript_attributes() {
    assert_eq!(
        attribute_transcript("claude", &fixture("claude-fresh.jsonl")),
        attributed("claude", "2.1.286")
    );
}

#[test]
fn claude_partial_trailing_line_is_skipped() {
    assert_eq!(
        attribute_transcript("claude", &fixture("claude-partial-tail.jsonl")),
        attributed("claude", "2.1.285")
    );
}

#[test]
fn claude_versionless_tail_with_large_line_still_attributes() {
    let mut body = versioned_claude_line("2.1.286");
    body.push_str(&format!(
        "{{\"type\":\"cost-state\",\"blob\":\"{}\"}}\n",
        "x".repeat(100 * 1024)
    ));
    body.push_str(
        "{\"type\":\"cost-state\",\"costUSD\":0.5}\n{\"type\":\"mode\",\"mode\":\"plan\"}\n",
    );
    let mut r = Counting::new(body.into_bytes());
    let len = r.len();
    assert!(len > INITIAL_WINDOW);
    assert_eq!(
        attribute_reader("claude", &mut r, len),
        attributed("claude", "2.1.286")
    );
    // The first 64 KB window holds no complete versioned line, so it grew.
    assert!(r.read > INITIAL_WINDOW, "read only {} bytes", r.read);
}

#[test]
fn claude_no_version_field_is_unattributable() {
    assert_eq!(
        attribute_transcript("claude", &fixture("claude-no-version.jsonl")),
        unattributable(Unattributed::NoVersionField)
    );
}

#[test]
fn claude_version_beyond_one_megabyte_is_window_exceeded() {
    let dir = TestDir::new();
    let mut body = versioned_claude_line("2.1.286");
    body.push_str(&versionless_lines(MAX_WINDOW as usize + 4096));
    let path = dir.write("t.jsonl", body.as_bytes());
    assert_eq!(
        attribute_transcript("claude", &path),
        unattributable(Unattributed::WindowExceeded)
    );
}

#[test]
fn claude_version_just_inside_the_window_attributes() {
    let dir = TestDir::new();
    let mut body = versioned_claude_line("2.1.286");
    body.push_str(&versionless_lines(512 * 1024));
    let path = dir.write("t.jsonl", body.as_bytes());
    assert_eq!(
        attribute_transcript("claude", &path),
        attributed("claude", "2.1.286")
    );
}

#[test]
fn newest_claude_version_wins_over_older_entries() {
    let dir = TestDir::new();
    let body = format!(
        "{}{}{}",
        versioned_claude_line("2.1.250"),
        versioned_claude_line("2.1.288"),
        "{\"type\":\"mode\"}\n"
    );
    let path = dir.write("t.jsonl", body.as_bytes());
    assert_eq!(
        attribute_transcript("claude", &path),
        attributed("claude", "2.1.288")
    );
}

#[test]
fn codex_fresh_rollout_attributes() {
    assert_eq!(
        attribute_transcript("codex", &fixture("codex-fresh.jsonl")),
        attributed("codex", "0.158.0")
    );
}

#[test]
fn codex_resumed_rollout_reads_only_the_creators_head_version() {
    // The rollout alone cannot show the resume (the spike: one `session_meta`,
    // later turns carry no version), which is why the payload/gate rule, not
    // the reader, keeps resumed sessions unattributed.
    assert_eq!(
        attribute_transcript("codex", &fixture("codex-resumed.jsonl")),
        attributed("codex", "0.159.3")
    );
}

#[test]
fn codex_later_session_meta_is_ignored() {
    let dir = TestDir::new();
    let meta = |v: &str| {
        format!("{{\"type\":\"session_meta\",\"payload\":{{\"cli_version\":\"{v}\"}}}}\n")
    };
    let body = format!(
        "{}{{\"type\":\"event_msg\",\"payload\":{{}}}}\n{}",
        meta("0.158.0"),
        meta("0.159.3")
    );
    let path = dir.write("t.jsonl", body.as_bytes());
    assert_eq!(
        attribute_transcript("codex", &path),
        attributed("codex", "0.158.0")
    );
}

#[test]
fn codex_reads_only_the_head_line() {
    let mut body = std::fs::read_to_string(fixture("codex-fresh.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    body.push('\n');
    body.push_str(&versionless_lines(3 * 1024 * 1024));
    let mut r = Counting::new(body.into_bytes());
    let len = r.len();
    assert_eq!(
        attribute_reader("codex", &mut r, len),
        attributed("codex", "0.158.0")
    );
    // The head line is shorter than one chunk; the rest is never read.
    assert!(r.read <= 4096, "read {}", r.read);
}

#[test]
fn codex_head_line_without_newline_is_no_version_field() {
    let dir = TestDir::new();
    let head = "{\"type\":\"session_meta\",\"payload\":{\"cli_version\":\"0.158.0\"}}";
    let partial = dir.write("partial.jsonl", head.as_bytes());
    assert_eq!(
        attribute_transcript("codex", &partial),
        unattributable(Unattributed::NoVersionField)
    );
    let empty = dir.write("empty.jsonl", b"");
    assert_eq!(
        attribute_transcript("codex", &empty),
        unattributable(Unattributed::NoVersionField)
    );
}

#[test]
fn codex_head_line_beyond_the_window_is_window_exceeded() {
    let mut body = "x".repeat(MAX_WINDOW as usize + 10);
    body.push('\n');
    let mut r = Counting::new(body.into_bytes());
    let len = r.len();
    assert_eq!(
        attribute_reader("codex", &mut r, len),
        unattributable(Unattributed::WindowExceeded)
    );
    assert!(r.read <= MAX_WINDOW + 4096, "read {}", r.read);
}

#[test]
fn codex_without_session_meta_is_unattributable() {
    let dir = TestDir::new();
    let path = dir.write("t.jsonl", b"{\"type\":\"event_msg\",\"payload\":{}}\n");
    assert_eq!(
        attribute_transcript("codex", &path),
        unattributable(Unattributed::NoVersionField)
    );
}

#[test]
fn transcript_says_a_after_path_moves_to_b() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TestDir::new();
    let bin = dir.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let stub = bin.join("claude");
    let write_stub = |version: &str| {
        std::fs::write(
            &stub,
            format!("#!/bin/sh\necho '{version} (Claude Code)'\n"),
        )
        .unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let transcript = dir.write("t.jsonl", versioned_claude_line("2.1.286").as_bytes());
    write_stub("2.1.299");
    // The reader never consults a binary (PATH is not touched: the lib test
    // process is shared); a stub that disagrees changes nothing.
    assert_eq!(
        attribute_transcript("claude", &transcript),
        attributed("claude", "2.1.286")
    );
    write_stub("2.1.300");
    assert_eq!(
        attribute_transcript("claude", &transcript),
        attributed("claude", "2.1.286")
    );
}

#[test]
fn missing_file_is_absent() {
    let dir = TestDir::new();
    assert_eq!(
        attribute_transcript("claude", &dir.0.join("nope.jsonl")),
        unattributable(Unattributed::Absent)
    );
}

#[test]
fn directory_is_unreadable() {
    let dir = TestDir::new();
    assert_eq!(
        attribute_transcript("claude", &dir.0),
        unattributable(Unattributed::Unreadable)
    );
}

#[test]
fn unreadable_file_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TestDir::new();
    let path = dir.write("t.jsonl", versioned_claude_line("2.1.286").as_bytes());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&path).is_ok() {
        return; // running as root: permissions do not apply
    }
    assert_eq!(
        attribute_transcript("claude", &path),
        unattributable(Unattributed::Unreadable)
    );
}

#[test]
fn payload_without_transcript_path_is_unattributable() {
    for payload in [
        json!({"hook_event_name": "PreToolUse"}),
        json!({"hook_event_name": "PreToolUse", "transcript_path": null}),
        json!({"hook_event_name": "PreToolUse", "transcript_path": ""}),
        json!({"hook_event_name": "PreToolUse", "transcript_path": 7}),
    ] {
        assert_eq!(
            attribute_payload("claude", &payload),
            unattributable(Unattributed::NoTranscriptPath),
            "{payload}"
        );
    }
}

#[test]
fn relative_transcript_path_is_unattributable() {
    let payload = json!({"hook_event_name": "PreToolUse", "transcript_path": "rel/t.jsonl"});
    assert_eq!(
        attribute_payload("claude", &payload),
        unattributable(Unattributed::NoTranscriptPath)
    );
}

#[test]
fn payload_with_transcript_path_attributes() {
    let dir = TestDir::new();
    let path = dir.write("t.jsonl", versioned_claude_line("2.1.286").as_bytes());
    let payload = json!({"transcript_path": path});
    assert_eq!(
        attribute_payload("claude", &payload),
        attributed("claude", "2.1.286")
    );
}

#[test]
fn resume_session_start_is_unattributable_without_reading() {
    let dir = TestDir::new();
    let missing = dir.0.join("nope.jsonl");
    for (harness, reason) in [
        ("claude", Unattributed::ResumeBeforeFirstEntry),
        ("codex", Unattributed::CodexResumed),
    ] {
        let payload = json!({
            "hook_event_name": "SessionStart",
            "source": "resume",
            "transcript_path": missing,
        });
        assert_eq!(
            attribute_payload(harness, &payload),
            unattributable(reason),
            "{harness}"
        );
    }
    // A startup SessionStart is read normally (and finds nothing here).
    let payload = json!({
        "hook_event_name": "SessionStart",
        "source": "startup",
        "transcript_path": missing,
    });
    assert_eq!(
        attribute_payload("claude", &payload),
        unattributable(Unattributed::Absent)
    );
}

#[test]
fn read_is_capped() {
    let mut body = versioned_claude_line("2.1.286");
    body.push_str(&versionless_lines(3 * 1024 * 1024));
    let mut r = Counting::new(body.into_bytes());
    let len = r.len();
    assert_eq!(
        attribute_reader("claude", &mut r, len),
        unattributable(Unattributed::WindowExceeded)
    );
    assert!(r.read <= MAX_WINDOW + LINE_EXTENSION, "read {}", r.read);
}

#[test]
fn prerelease_version_is_unrecognized() {
    let dir = TestDir::new();
    let path = dir.write("t.jsonl", versioned_claude_line("2.2.0-beta.1").as_bytes());
    assert_eq!(
        attribute_transcript("claude", &path),
        unattributable(Unattributed::UnrecognizedVersion)
    );
}

#[test]
fn unknown_harness_is_unrecognized() {
    assert_eq!(
        attribute_transcript("gemini", &fixture("claude-fresh.jsonl")),
        unattributable(Unattributed::UnrecognizedVersion)
    );
}

#[test]
fn reason_strings_are_pinned() {
    let pinned = [
        (
            Unattributed::NoTranscriptPath,
            "no transcript path in the payload",
        ),
        (
            Unattributed::ResumeBeforeFirstEntry,
            "resume before first entry",
        ),
        (
            Unattributed::CodexResumed,
            "codex resume: rollout version is the creating CLI's",
        ),
        (Unattributed::Absent, "transcript not found"),
        (Unattributed::Unreadable, "transcript unreadable"),
        (
            Unattributed::NoVersionField,
            "no version field in the transcript",
        ),
        (
            Unattributed::WindowExceeded,
            "no version within the 1 MB read window",
        ),
        (
            Unattributed::UnrecognizedVersion,
            "transcript version not recognized",
        ),
    ];
    for (reason, text) in pinned {
        assert_eq!(reason.as_str(), text);
    }
}

/// Kills: a blocking `File::open` on a FIFO transcript_path (the hook would hang).
#[test]
fn fifo_transcript_is_unreadable_without_blocking() {
    use std::os::unix::ffi::OsStrExt;
    let dir = TestDir::new();
    let fifo = dir.0.join("t.fifo");
    let c = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let (tx, rx) = std::sync::mpsc::channel();
    let path = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(attribute_transcript("claude", &path));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(5));
    if got.is_err() {
        // Unblock the stuck reader before failing so the thread ends.
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    }
    assert_eq!(
        got.expect("attribution blocked on a FIFO"),
        unattributable(Unattributed::Unreadable)
    );
}

/// Kills: following a symlinked transcript_path (only a regular file is read).
#[test]
fn symlinked_transcript_is_unreadable() {
    let dir = TestDir::new();
    let real = dir.write("real.jsonl", versioned_claude_line("2.1.286").as_bytes());
    let link = dir.0.join("link.jsonl");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(
        attribute_transcript("claude", &link),
        unattributable(Unattributed::Unreadable)
    );
}

// The optional creator reader stays usable; production never calls it current runtime.
#[test]
fn task3_versionless_codex_creator_is_not_current_runtime_even_at_startup() {
    let transcript = fixture("codex-fresh.jsonl");
    assert!(matches!(
        attribute_transcript("codex", &transcript),
        Attribution::Attributed { .. }
    ));
    for event in ["SessionStart", "PreToolUse"] {
        let payload =
            json!({"hook_event_name": event, "source": "startup", "transcript_path": transcript});
        assert!(
            matches!(
                attribute_payload("codex", &payload),
                Attribution::Unattributable { .. }
            ),
            "{event}"
        );
    }
    let payload = json!({"hook_event_name": "SessionStart", "source": "resume", "transcript_path": transcript});
    assert_eq!(
        attribute_payload("codex", &payload),
        unattributable(Unattributed::CodexResumed)
    );
}
