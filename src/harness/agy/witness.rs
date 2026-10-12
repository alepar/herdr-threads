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

#[cfg(test)]
thread_local! {
    /// `read_dir` calls made on this thread, so a test can prove the pane-bound conversation runs no glob.
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

/// The parent whose subagent listing names `child`: one bounded walk of `data/brain`, like
/// [`glob_listing`]. `Err`: unreadable or over budget.
fn listing_parent(data: &Path, child: &str, deadline: Instant) -> Result<Option<String>, ()> {
    let entries = match std::fs::read_dir(data.join("brain")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    for (seen, entry) in entries.enumerate() {
        if seen >= BRAIN_ENTRY_CAP || (seen % 256 == 0 && Instant::now() >= deadline) {
            return Err(());
        }
        let entry = entry.map_err(|_| ())?;
        let Some(parent) = entry
            .file_name()
            .to_str()
            .filter(|parent| *parent != child && paths::valid_conversation_id(parent))
            .map(str::to_owned)
        else {
            continue;
        };
        match listing_entry(data, &parent, child) {
            Witness::Present => return Ok(Some(parent)),
            Witness::Unreadable => return Err(()),
            _ => {}
        }
    }
    Ok(None)
}

/// The role of the conversation a CLI call names (`ANTIGRAVITY_CONVERSATION_ID`), from the same
/// witnesses the hooks read: a presence lock is a top-level conversation (agy never creates one for
/// a child); otherwise the subagent listing names its parent. Neither, or an unreadable witness,
/// cannot be attributed and fails toward child (refused).
pub(super) fn caller_session(
    conversation: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> crate::harness::adapter::CallerSession {
    use crate::harness::adapter::CallerSession;
    const BUDGET: std::time::Duration = std::time::Duration::from_millis(500);
    if !paths::valid_conversation_id(conversation) {
        return CallerSession::Unattributable("invalid agy conversation id".into());
    }
    let Some(home) = env("HOME").filter(|home| !home.is_empty()) else {
        return CallerSession::Unattributable(
            "HOME is unset, so agy's witnesses cannot be read".into(),
        );
    };
    let data = AgyRoots::from_home(Path::new(&home)).data;
    match regular_file(&data.join("presence").join(format!("{conversation}.lock"))) {
        Witness::Present => return CallerSession::TopLevel,
        Witness::Unreadable => {
            return CallerSession::Unattributable("agy presence lock unreadable".into());
        }
        _ => {}
    }
    match listing_parent(&data, conversation, Instant::now() + BUDGET) {
        Ok(Some(parent)) => CallerSession::Subagent { parent },
        Ok(None) => CallerSession::Unattributable(
            "agy conversation has no presence lock and no parent listing".into(),
        ),
        Err(()) => CallerSession::Unattributable("agy subagent listings unreadable".into()),
    }
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
    // "Known" means only: the pane's current native session is this conversation (P == C). The
    // conversation-state file under `state_dir` is ht-poh.5's delivery cache and never an input here
    // (A2: a client-local file is not lifecycle authority).
    let known = || pane_session().as_deref() == Some(conversation);
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
