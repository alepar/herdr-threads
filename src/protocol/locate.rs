//! `Command::LocateCaller`: the daemon verifies which of several caller-location
//! candidates is an ancestor of the calling process (`service::locate`).

use crate::protocol::authority::LivenessFingerprint;
use serde::{Deserialize, Serialize};

/// One environment family's claim about where the caller lives. The daemon
/// matches on the family only; it learns no host kind from it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum LocateCandidate {
    /// A pane id of a Herdr server this daemon reads.
    Pane { id: String },
    /// A herdr-core terminal (`HC_TERMINAL_SOCKET`, `HC_TERMINAL_ID`).
    Terminal { socket: String, id: String },
}

/// The request. `subject` is the claim's liveness fingerprint; without it the
/// daemon starts the walk at the connection's peer pid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocateCaller {
    pub candidates: Vec<LocateCandidate>,
    #[serde(default)]
    pub subject: Option<LivenessFingerprint>,
}

/// Why one candidate did not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateVerdict {
    /// The host named a root process and it is not an ancestor of the subject.
    NotAncestor,
    /// The host knows no root process for the target.
    NoRoot,
    /// The host could not be asked (unreachable or not attached).
    Unreachable,
    /// No host in this build answers for the candidate's family.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum LocateReply {
    /// `candidates[index]` runs an ancestor of the subject; `root` is its root pid.
    Verified { index: usize, root: u32 },
    Unverified {
        per_candidate: Vec<CandidateVerdict>,
    },
}
