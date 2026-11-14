//! The role of the session a CLI call names, from the variables the herdr-threads plugin sets on
//! every shell call (opencode design D2). opencode exports none of its own session ids, so the
//! plugin's `shell.env` hook writes the call's session, its root top-level parent and the
//! plugin's own process id. The pid echo must equal opencode's `OPENCODE_PID`, else the
//! variables were inherited from an outer opencode (a nested `--pure` one sets none of its own)
//! and the call cannot be attributed.
use crate::harness::adapter::CallerSession;

/// The session the shell call belongs to (the plugin's `shell.env` `sessionID`).
pub const SESSION_VAR: &str = "HERDR_THREADS_OPENCODE_SESSION";
/// The root top-level session of a child; empty for a top-level session, `?` when the plugin's
/// parent lookup failed.
pub const PARENT_VAR: &str = "HERDR_THREADS_OPENCODE_PARENT";
/// The plugin's own process id, echoed beside opencode's `OPENCODE_PID`.
pub const PID_ECHO_VAR: &str = "HERDR_THREADS_OPENCODE_PID";
/// opencode exports its own pid to its shell tool's children.
pub const PID_VAR: &str = "OPENCODE_PID";

/// `ses_` followed by 1 to 64 ASCII letters or digits.
pub fn valid_session_id(id: &str) -> bool {
    id.strip_prefix("ses_").is_some_and(|rest| {
        (1..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

pub(super) fn caller_session(session: &str, env: &dyn Fn(&str) -> Option<String>) -> CallerSession {
    if !valid_session_id(session) {
        return CallerSession::Unattributable("invalid opencode session id".into());
    }
    let pid = env(PID_VAR).filter(|pid| !pid.is_empty());
    let echo = env(PID_ECHO_VAR).filter(|pid| !pid.is_empty());
    match (pid, echo) {
        (Some(pid), Some(echo)) if pid.trim() == echo.trim() => {}
        _ => {
            return CallerSession::Unattributable(format!(
                "{PID_ECHO_VAR} does not match {PID_VAR}, so the session variables were \
                 inherited from another opencode process"
            ));
        }
    }
    match env(PARENT_VAR) {
        None => CallerSession::TopLevel,
        Some(parent) if parent.is_empty() => CallerSession::TopLevel,
        Some(parent) if valid_session_id(&parent) && parent != session => {
            CallerSession::Subagent { parent }
        }
        Some(_) => CallerSession::MalformedMarker { var: PARENT_VAR },
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/opencode_witness.rs"]
mod tests;
