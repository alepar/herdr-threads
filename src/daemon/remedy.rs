//! Every operator-facing remedy string (root §B3 D4). ht-p03.45 owns the
//! signature and call sites; this module (ht-p03.10) owns the text. No other
//! module spells a remedy: a site asks `remedy(class, context)`.

use crate::protocol::results::ErrorClass;
use std::path::{Path, PathBuf};

/// The site that asks for a remedy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemedyContext {
    StartupFailure { log: PathBuf },
    StartupTimeout { log: PathBuf },
    Exit3,
    LaneDegraded { log: PathBuf },
    VersionSkew { daemon: String, cli: String },
    Doctor { log: PathBuf },
}

/// The state directory a log path lives under, for "back up <state>": the
/// instance directory holds both the logs and the store.
fn state_hint(log: &Path) -> String {
    let mut dir = log.parent();
    if dir.and_then(Path::file_name).is_some_and(|n| n == "logs") {
        dir = dir.and_then(Path::parent);
    }
    dir.unwrap_or(log).display().to_string()
}

fn corrupt(log: &Path) -> String {
    format!(
        "the store is damaged; see {}; back up {} before repair",
        log.display(),
        state_hint(log)
    )
}

/// The one remedy string for a failure of `class` at `context`. `None` class
/// yields the context's generic line.
pub fn remedy(class: Option<ErrorClass>, context: &RemedyContext) -> String {
    match context {
        RemedyContext::VersionSkew { daemon, cli } => format!(
            "daemon is version {daemon}, CLI is {cli}: {}",
            remedy(Some(ErrorClass::VersionSkew), &RemedyContext::Exit3)
        ),
        // Exit 3: a mismatched daemon must be stopped before ensure can start
        // this version; an unavailable one only needs ensure (ht-p03.107).
        RemedyContext::Exit3 => match class {
            Some(ErrorClass::VersionSkew) => {
                "run `herdr-threads daemon stop` then `herdr-threads daemon ensure`".to_owned()
            }
            _ => "run `herdr-threads daemon ensure`".to_owned(),
        },
        RemedyContext::StartupFailure { log } | RemedyContext::StartupTimeout { log } => {
            match class {
                Some(ErrorClass::Corrupt) => corrupt(log),
                Some(ErrorClass::Transient) => {
                    format!("temporary; retry the command; see {}", log.display())
                }
                _ => format!(
                    "the daemon did not start; see {} (last lines above)",
                    log.display()
                ),
            }
        }
        RemedyContext::LaneDegraded { log } | RemedyContext::Doctor { log } => match class {
            Some(ErrorClass::Corrupt) => corrupt(log),
            Some(ErrorClass::Unavailable) => {
                format!("the daemon or Herdr is not running; see {}", log.display())
            }
            _ => format!("temporary; retry the command; see {}", log.display()),
        },
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/error_taxonomy.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/docs/remedy_table.rs"]
mod remedy_table;
