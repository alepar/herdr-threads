//! The journal as the sync coordinator and the apply worker see it
//! (journal-core design, sections 6 and 12). A contract only: nothing here
//! touches the store.

use super::{BatchLimits, LogHead, LogKey, Vector, envelope::Envelope, envelope::EnvelopeError};
use crate::protocol::results::ApiError;
use crate::protocol::time::CallBudget;

/// What ingesting one batch of a log's entries did.
///
/// Owns: the closed set of ingest results. `Corrupt` and `Headroom` store
/// nothing; `Gap`, `Fork` and `Frozen` keep the valid prefix, counted in
/// `stored`. Does not own: resolving forks or pacing peers, which the caller
/// decides from the variant. A new outcome is a new variant, handled by every
/// caller's exhaustive `match`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestOutcome {
    /// Every entry was stored or already held; `head` is the new contiguous head.
    Stored {
        stored: usize,
        duplicates: usize,
        head: LogHead,
    },
    /// The batch skips ahead; `expected` is the next counter wanted.
    Gap { stored: usize, expected: u64 },
    /// A validly signed entry contradicts the held chain.
    Fork {
        stored: usize,
        evidence: ForkEvidence,
    },
    /// An entry lies beyond the counter at which the log froze.
    Frozen { stored: usize, frozen_at: u64 },
    /// An entry is damaged or forged; nothing was stored.
    Corrupt {
        log: LogKey,
        counter: u64,
        reason: EnvelopeError,
    },
    /// Not enough free disk space; nothing was stored. Pause pulls, do not
    /// penalise the peer.
    Headroom,
}

/// Proof of a fork, for the caller to keep and report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForkEvidence {
    /// A peer holds entries of this node's own active log that this node does
    /// not have.
    AheadOfSelf {
        log: LogKey,
        my_head: LogHead,
        entry: Box<Envelope>,
    },
    /// Two different validly signed entries for one position.
    Conflict {
        held: Box<Envelope>,
        conflicting: Box<Envelope>,
    },
}

/// What one apply pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplyProgress {
    pub applied: usize,
    pub parked: usize,
    pub failed: usize,
    pub replayed: usize,
    /// More entries are ready; run another pass.
    pub has_more: bool,
    /// The node's tag counter if this pass raised it, for the post-commit
    /// identity update.
    pub self_tag_k: Option<u32>,
}

/// The narrow seam through which the sync coordinator and the apply worker
/// reach the journal.
///
/// Owns: the four operations a peer link and the apply worker need (version
/// vector, stored-hash lookup, serving committed ranges, ingesting a verified
/// batch) plus one apply pass. Does not own: chain rules (see
/// `chain::check_next`), fork resolution, peer pacing or the store's
/// transaction handling. Role: implemented by `SqliteStore` and by the
/// in-memory `MemoryJournal`, which run the same chain rules and signature
/// checks so link tests cannot pass on data real nodes reject. New journal
/// behaviour for the sync layer is a method here or a new outcome variant.
///
/// Every method returns `Err` only for store errors; protocol outcomes
/// (gap, fork, corruption, no headroom) are `Ok` variants of the result.
pub trait JournalPort: Send + Sync {
    /// The head of every log this node holds.
    fn vector(&self, budget: &CallBudget) -> Result<Vector, ApiError>;

    /// The stored hash at `counter` of `log`, if held.
    fn stored_hash(
        &self,
        log: &LogKey,
        counter: u64,
        budget: &CallBudget,
    ) -> Result<Option<[u8; 32]>, ApiError>;

    /// Committed entries of `log` from counter `from`, within `limits`.
    fn serve(
        &self,
        log: &LogKey,
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Vec<Envelope>, ApiError>;

    /// Verify and store a batch of `log`'s entries.
    ///
    /// The caller holds a `FairWriter` background turn.
    fn ingest(
        &self,
        log: &LogKey,
        entries: Vec<Envelope>,
        budget: &CallBudget,
    ) -> Result<IngestOutcome, ApiError>;

    /// Run one bounded apply pass over pending entries.
    ///
    /// The caller holds a `FairWriter` background turn.
    fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError>;
}
