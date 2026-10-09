//! Freeze every public thread argument/filter before dispatch or intent creation.
use super::commands::{CliAction, MutationSpec};
use crate::protocol::{commands::Command, ids::ThreadId};

/// Only thread selectors are walked. Recovery references, jobs, invitations,
/// messages and lease tokens keep their exact identifier semantics.
pub fn selector_mut(action: &mut CliAction) -> Option<&mut ThreadId> {
    match action {
        CliAction::Mutation(
            MutationSpec::Topic { thread, .. }
            | MutationSpec::Name { thread, .. }
            | MutationSpec::Invite { thread, .. }
            | MutationSpec::AcceptRequired { thread, .. }
            | MutationSpec::Reject { thread, .. }
            | MutationSpec::Send { thread, .. }
            | MutationSpec::Join(thread)
            | MutationSpec::Accept(thread)
            | MutationSpec::Leave(thread)
            | MutationSpec::Archive(thread)
            | MutationSpec::Reopen(thread),
        ) => Some(thread),
        CliAction::Wire(command) => match command {
            Command::Thread(q) => Some(&mut q.thread),
            Command::ThreadName(q) => Some(&mut q.thread),
            Command::History(q) => Some(&mut q.thread),
            Command::Participants(q) => Some(&mut q.thread),
            Command::ActiveWarnings(q) => Some(&mut q.thread),
            Command::PendingReceipts(q) => q.thread.as_mut(),
            Command::Diagnostics(q) => q.thread.as_mut(),
            Command::Search(q) => q.thread.as_mut(),
            _ => None,
        },
        CliAction::Handoff(q) => q.thread.as_mut(),
        CliAction::Follow(q) => Some(&mut q.thread),
        CliAction::Summary(super::summary::SummaryCli::Summary { thread }) => Some(thread),
        _ => None,
    }
}

/// Resolution may query the daemon once; downstream operations retain the ID.
/// A digest-style string is a valid name first. Its old misuse hint appears only
/// after a failed name lookup, so it never shadows a permitted exact name.
pub fn resolve_cli_threads<E: From<crate::protocol::results::ApiError>>(
    parsed: &mut super::commands::ParsedCli,
    resolve: impl FnOnce(&str) -> Result<ThreadId, E>,
) -> Result<(), E> {
    let Some(selector) = parsed.thread_selector.as_deref() else {
        return Ok(());
    };
    let resolved = resolve(selector)?;
    let thread = selector_mut(&mut parsed.action).expect("thread selector has a typed action");
    *thread = resolved;
    parsed.thread_selector = None;
    Ok(())
}

/// Attention digest display references may also be names; lookup gets first say.
pub fn selector_error(
    mut error: crate::protocol::results::ApiError,
    selector: &str,
) -> crate::protocol::results::ApiError {
    use crate::protocol::results::ErrorCode;
    if error.code == ErrorCode::NotFound
        && let Some((item, thread)) = selector.split_once('@')
        && !item.is_empty()
        && !thread.is_empty()
        && !thread.contains('@')
    {
        error.code = ErrorCode::InvalidRequest;
        error.detail = format!(
            "`{selector}` was not found as a thread name; it resembles an attention digest item (ITEM@THREAD); pass the bare thread ID `{thread}`, for example `herdr-threads accept {thread}`"
        );
    }
    error
}
