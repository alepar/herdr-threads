//! The journal as the sync coordinator and the apply worker see it
//! (journal-core design, sections 6 and 12). A contract only: nothing here
//! touches the store.

use super::{
    BatchLimits, LogHead, LogKey, LogState, Vector, envelope::EnvelopeError, envelope::StoredEntry,
};
use crate::protocol::results::ApiError;
use crate::protocol::time::CallBudget;

/// What ingesting one batch of a log's entries did.
///
/// Owns: the closed set of ingest results. `Corrupt` and `Headroom` store
/// nothing; `Gap`, `Fork` and `Frozen` keep the valid prefix, counted in
/// `stored`. Does not own: resolving forks or pacing peers, which the caller
/// decides from the variant. A new outcome is a new variant, handled by every
/// caller's exhaustive `match`. Fork evidence holds entries as stored, known or
/// opaque.
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

/// Proof of a fork, for the caller to keep and report. Either side may be an
/// opaque entry, relayed as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForkEvidence {
    /// A peer holds entries of this node's own active log that this node does
    /// not have.
    AheadOfSelf {
        log: LogKey,
        my_head: LogHead,
        entry: Box<StoredEntry>,
    },
    /// Two different validly signed entries for one position.
    Conflict {
        held: Box<StoredEntry>,
        conflicting: Box<StoredEntry>,
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

/// What a node knows about one log it holds.
///
/// Owns: the facts a fork repair needs about a log (its public key, whether
/// it is one of this node's own incarnations, whether it is the one this
/// node authors into, its freeze and its head). Does not own: the log's
/// entries, or any decision about it. Role: the value returned by
/// [`JournalPort::log_meta`], read by the incarnations sink and worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogMeta {
    /// The key that signs the log's entries (from its genesis).
    pub pubkey: [u8; 32],
    /// The log belongs to this node (any of its incarnations).
    pub own: bool,
    /// The log is the one this node authors into now.
    pub active: bool,
    pub state: LogState,
    /// Last counter a frozen log accepts; `None` while active.
    pub frozen_at: Option<u64>,
    /// Highest contiguous counter held.
    pub contiguous: u64,
    /// Hash of the entry at `contiguous`.
    pub head: [u8; 32],
}

/// The narrow seam through which the sync coordinator and the apply worker
/// reach the journal.
///
/// Owns: the operations a peer link and the apply worker need (version
/// vector, stored-hash lookup, serving committed ranges and branches, log
/// metadata, ingesting a verified batch) plus one apply pass. Does not own: chain rules (see
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

    /// Committed entries of `log` from counter `from`, within `limits`. An
    /// entry may be opaque (a layout this node does not understand); the port
    /// relays it byte-exact.
    fn serve(
        &self,
        log: &LogKey,
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Vec<StoredEntry>, ApiError>;

    /// The batch of the branch of `log` that ends at `tip` (counter, hash),
    /// starting at counter `from`, within `limits`.
    ///
    /// Read-only, on a query connection. The branch is either held in the main
    /// chain, or recorded as a forked branch that walks back by `prev_hash`
    /// (a bounded walk) into the main chain. `None` means this node cannot
    /// serve that branch (unknown log or tip, a broken chain, or a walk over
    /// the bound); `Some(vec![])` means the branch is known and nothing lies
    /// in range.
    fn serve_branch(
        &self,
        log: &LogKey,
        tip: (u64, [u8; 32]),
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Option<Vec<StoredEntry>>, ApiError>;

    /// The public key, ownership and state of `log`, if held.
    ///
    /// Read-only, on a query connection; `None` for a log this node does not
    /// hold. The one read through which the incarnations sink and worker
    /// learn a log's pubkey and ownership.
    fn log_meta(&self, log: &LogKey, budget: &CallBudget) -> Result<Option<LogMeta>, ApiError>;

    /// Verify and store a batch of `log`'s entries, known or opaque. An opaque
    /// entry joins the hash chain by its prefix fields.
    ///
    /// The caller holds a `FairWriter` background turn.
    fn ingest(
        &self,
        log: &LogKey,
        entries: Vec<StoredEntry>,
        budget: &CallBudget,
    ) -> Result<IngestOutcome, ApiError>;

    /// Run one bounded apply pass over pending entries.
    ///
    /// The caller holds a `FairWriter` background turn.
    fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError>;
}
