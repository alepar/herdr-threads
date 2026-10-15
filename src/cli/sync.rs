//! `herdr-threads sync status|verify|unapplied|resync|clone-cleared` (ht-5fr.8).
//!
//! Owns: building the wire `Command::Sync` for each `sync` subcommand and
//! refusing a malformed one before any daemon call. Does not: render (the
//! compact encoder in `protocol::output_compact` does), decide, or touch the
//! store; the mutating forms (`unapplied replay`, `resync`, `clone-cleared`)
//! are operator-gated by `human` routing and the daemon's peer check.

use crate::protocol::{
    commands::Command,
    results::ApiError,
    sync_admin::{DEFAULT_UNAPPLIED_PAGE, SyncCommand},
};

fn checked(command: SyncCommand) -> Result<Command, ApiError> {
    command.validate().map_err(ApiError::invalid_request)?;
    Ok(Command::Sync(command))
}

/// `sync status [--log NODE]`.
pub fn status(log: Option<String>) -> Result<Command, ApiError> {
    checked(SyncCommand::Status { log })
}

/// `sync verify`.
pub fn verify() -> Command {
    Command::Sync(SyncCommand::Verify)
}

/// `sync unapplied list [--after SEQ] [--limit N]`.
pub fn unapplied_list(after: Option<u64>, limit: Option<u16>) -> Result<Command, ApiError> {
    checked(SyncCommand::UnappliedList {
        after,
        limit: limit.unwrap_or(DEFAULT_UNAPPLIED_PAGE),
    })
}

/// `sync unapplied replay`: operator-gated.
pub fn unapplied_replay() -> Command {
    Command::Sync(SyncCommand::UnappliedReplay)
}

/// `sync resync --from NODE [--log NODE]...`: operator-gated.
pub fn resync(from: String, logs: Vec<String>) -> Result<Command, ApiError> {
    checked(SyncCommand::Resync { from, logs })
}

/// `sync clone-cleared`: operator-gated.
pub fn clone_cleared() -> Command {
    Command::Sync(SyncCommand::CloneCleared)
}
