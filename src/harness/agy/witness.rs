//! Local role witnesses (presence lock, subagent listings) and lifecycle reading (agy-adapter D5).
//! Every read is `symlink_metadata` or a directory listing: no file content is read, and a
//! payload path is never used. A missing, unreadable or over-budget witness fails toward child.
use super::paths::{self, AgyEvent, AgyRoots};
use crate::harness::{
    adapter::{LocalEvidence, LocalEvidenceRequest, RoleWitness, Witness},
    context::EventKind,
};
use serde_json::Value;
use std::cell::OnceCell;
use std::path::Path;
use std::time::Instant;

/// The most `brain/` entries one glob examines before it reads as `Unreadable`.
pub const BRAIN_ENTRY_CAP: usize = 65_536;
/// A conversation-state file larger than this is not a known-conversation witness.
const STATE_FILE_MAX: u64 = 4096;

#[cfg(test)]
thread_local! {
    /// `read_dir` calls made on this thread, so a test can prove a known conversation runs no glob.
    pub(super) static READ_DIR_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A regular file (not a symlink) exists at `path`: `Present`; nothing there or something else:
/// `Absent`; any other error: `Unreadable`.
fn regular_file(path: &Path) -> Witness {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Witness::Present,
        Ok(_) => Witness::Absent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Witness::Absent,
        Err(_) => Witness::Unreadable,
    }
}

/// The child's listing under one candidate parent. A parent that is not a directory has no
/// listing, so it reads `Absent`; the marker check keeps the strict reading.
pub(super) fn listing_entry(data: &Path, parent: &str, child: &str) -> Witness {
    let path = data
        .join("brain")
        .join(parent)
        .join(".system_generated")
        .join("subagents")
        .join(format!("{child}.json"));
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => Witness::Present,
        Ok(_) => Witness::Absent,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Witness::Absent
        }
        Err(_) => Witness::Unreadable,
    }
}

/// Streams `data/brain` and stats the child's listing under every entry: no content reads. More
/// than [`BRAIN_ENTRY_CAP`] entries, or the deadline, is `Unreadable`.
fn glob_listing(data: &Path, child: &str, deadline: Instant) -> Witness {
    if Instant::now() >= deadline {
        return Witness::Unreadable;
    }
    #[cfg(test)]
    READ_DIR_CALLS.with(|calls| calls.set(calls.get() + 1));
    let entries = match std::fs::read_dir(data.join("brain")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Witness::Absent,
        Err(_) => return Witness::Unreadable,
    };
    for (seen, entry) in entries.enumerate() {
        if seen >= BRAIN_ENTRY_CAP || (seen % 256 == 0 && Instant::now() >= deadline) {
            return Witness::Unreadable;
        }
        let Ok(entry) = entry else {
            return Witness::Unreadable;
        };
        let Some(parent) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        match listing_entry(data, &parent, child) {
            Witness::Present => return Witness::Present,
            Witness::Unreadable => return Witness::Unreadable,
            _ => {}
        }
    }
    Witness::Absent
}

pub(super) fn local_evidence(request: &LocalEvidenceRequest<'_>) -> LocalEvidence {
    let Some(event) = request
        .input
        .registered_event
        .as_deref()
        .and_then(AgyEvent::parse)
    else {
        return LocalEvidence::default();
    };
    let Ok(payload) = serde_json::from_slice::<Value>(&request.input.bytes) else {
        return LocalEvidence::default();
    };
    let Some(conversation) = payload
        .get("conversationId")
        .and_then(Value::as_str)
        .filter(|id| paths::valid_conversation_id(id))
    else {
        return LocalEvidence::default();
    };
    // The pane's current native session is a client-local hint; ask once, and only when needed.
    let pane = OnceCell::new();
    let pane_session = || pane.get_or_init(|| (request.pane_session)()).clone();
    let known = || {
        let state_file = request.state_dir.is_some_and(|state| {
            std::fs::symlink_metadata(paths::conversation_state(state, conversation))
                .is_ok_and(|meta| meta.file_type().is_file() && meta.len() <= STATE_FILE_MAX)
        });
        state_file || pane_session().as_deref() == Some(conversation)
    };
    let role = match request.home.filter(|home| !home.is_empty()) {
        None => RoleWitness {
            top_level_marker: Witness::Unreadable,
            child_listing: Witness::Unreadable,
        },
        Some(home) => {
            let data = AgyRoots::from_home(Path::new(home)).data;
            let marker = regular_file(&data.join("presence").join(format!("{conversation}.lock")));
            let listing = if marker != Witness::Present {
                Witness::Unchecked
            } else if known() {
                Witness::Absent
            } else {
                candidate_listing(&data, conversation, pane_session(), request.deadline)
            };
            RoleWitness {
                top_level_marker: marker,
                child_listing: listing,
            }
        }
    };
    let lifecycle = match event {
        AgyEvent::SessionStart => Some(startup_or_clear(pane_session(), conversation)),
        AgyEvent::PreInvocation
            if payload.get("invocationNum").and_then(Value::as_u64) == Some(0) && !known() =>
        {
            let steps = payload.get("initialNumSteps").and_then(Value::as_u64);
            Some(if steps.is_some_and(|steps| steps > 1) {
                EventKind::Resume
            } else {
                startup_or_clear(pane_session(), conversation)
            })
        }
        AgyEvent::PreInvocation | AgyEvent::Stop => None,
    };
    LocalEvidence {
        role: Some(role),
        lifecycle,
    }
}

/// `Clear` when the pane is bound to some other conversation, else `Startup`.
fn startup_or_clear(pane: Option<String>, conversation: &str) -> EventKind {
    match pane {
        Some(pane) if pane != conversation => EventKind::Clear,
        _ => EventKind::Startup,
    }
}

/// The bound parent first (one stat), then the bounded glob over every `brain/` entry.
fn candidate_listing(
    data: &Path,
    conversation: &str,
    pane: Option<String>,
    deadline: Instant,
) -> Witness {
    if let Some(parent) =
        pane.filter(|parent| parent != conversation && paths::valid_conversation_id(parent))
        && listing_entry(data, &parent, conversation) == Witness::Present
    {
        return Witness::Present;
    }
    glob_listing(data, conversation, deadline)
}

#[cfg(test)]
#[path = "../../../tests/harness/agy_witness.rs"]
pub(super) mod tests;
