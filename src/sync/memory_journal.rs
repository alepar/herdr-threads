//! The in-memory [`JournalPort`] for link and routing tests, without SQLite.
//!
//! Compiled only for tests and the `test-support` feature.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::chain::{ChainStep, HeldLog, check_next};
use super::envelope::{
    Deps, Envelope, EnvelopeError, GENESIS_KIND, StoredEntry, UnsignedEnvelope, hex_encode,
};
use super::facts::GenesisPayload;
use super::hlc::HlcStamp;
use super::journal_port::{ApplyProgress, ForkEvidence, IngestOutcome, JournalPort, LogMeta};
use super::{BatchLimits, LogHead, LogKey, LogState, Vector};
use crate::node::NodeIdentity;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::CallBudget;

/// Longest ingest batch, in entries (the same bound as `SqliteStore`).
const INGEST_BATCH_MAX: usize = 256;

/// Most forked entries `serve_branch` walks before it answers `None` (the
/// same bound as `SqliteStore`).
const BRANCH_WALK_MAX_STEPS: u64 = 65_536;

/// The one incarnation of the log a `MemoryJournal` authors into.
const SELF_INCARNATION: u32 = 1;

/// Wall-clock origin of the stamps `author_raw` seals; entry `n` is `n` ms later.
const HLC_BASE_MS: u64 = 1_700_000_000_000;

/// The test-only second implementation of [`JournalPort`].
///
/// Owns: `BTreeMap<LogKey, Vec<Envelope>>`, the public key of each log, the
/// freeze marks and the per-log "processed" mark, plus `author_raw` for sealing
/// validly signed entries on its own log. Never skips verification: every
/// ingested entry goes through [`check_next`] and `Envelope::verify`, exactly
/// as `SqliteStore` does, and every chain rule comes only from `ChainRules`
/// (`chain::check_next`), never re-derived here. Does not own: kinds or
/// handlers (its `apply_pending` only marks entries processed), disk
/// headroom (there is no disk, so `IngestOutcome::Headroom` never happens) or
/// fork resolution. Role: lets link and routing tests (ht-5fr.5) run without
/// SQLite on data a real node would accept or refuse alike; the contract suite
/// `tests/sync/journal_port_contract.rs` proves it equivalent to `SqliteStore`.
/// A new `JournalPort` behaviour is mirrored here and added to that suite.
pub struct MemoryJournal {
    identity: Arc<NodeIdentity>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    logs: BTreeMap<LogKey, Vec<Envelope>>,
    pubkeys: BTreeMap<LogKey, [u8; 32]>,
    /// Last counter a frozen log accepts.
    frozen: BTreeMap<LogKey, u64>,
    /// Highest counter `apply_pending` has marked processed.
    processed: BTreeMap<LogKey, u64>,
    /// Entries of forked branches, by `(log, counter, hash)`.
    forked: BTreeMap<(LogKey, u64, [u8; 32]), Envelope>,
    /// Test hook: the walk bound of `serve_branch`; 0 means the default.
    branch_walk_bound: u64,
}

impl MemoryJournal {
    pub fn new(identity: impl Into<Arc<NodeIdentity>>) -> Self {
        Self {
            identity: identity.into(),
            inner: Mutex::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The log this journal authors into; held only once [`Self::bootstrap`]
    /// or [`Self::author_raw`] has written its genesis.
    pub fn self_log(&self) -> LogKey {
        LogKey {
            node: self.identity.uuid(),
            incarnation: SELF_INCARNATION,
        }
    }

    /// Writes the own log's genesis if there is none yet; returns the log.
    pub fn bootstrap(&self) -> LogKey {
        self.ensure_genesis(&mut self.lock());
        self.self_log()
    }

    fn ensure_genesis(&self, inner: &mut Inner) {
        let log = self.self_log();
        if inner.logs.contains_key(&log) {
            return;
        }
        let payload = serde_json::to_vec(&GenesisPayload {
            pubkey: hex_encode(&self.identity.public_key()),
            tag: self.identity.tag().to_string(),
            tag_k: self.identity.tag_k(),
            incarnation: SELF_INCARNATION,
            predecessor: None,
            host_label: "memory".into(),
        })
        .expect("a genesis payload serializes");
        let genesis = self
            .seal(&log, 1, [0u8; 32], GENESIS_KIND, payload)
            .expect("a genesis payload is well formed");
        inner.pubkeys.insert(log, self.identity.public_key());
        inner.logs.insert(log, vec![genesis]);
        inner.processed.insert(log, 1);
    }

    fn seal(
        &self,
        log: &LogKey,
        counter: u64,
        prev_hash: [u8; 32],
        kind: &str,
        payload: Vec<u8>,
    ) -> Result<Envelope, EnvelopeError> {
        let unsigned = UnsignedEnvelope {
            log: *log,
            counter,
            prev_hash,
            hlc: HlcStamp::from_parts(HLC_BASE_MS + counter, 0),
            kind: kind.to_owned(),
            kind_version: 1,
            deps: Deps::empty(),
            creates: None,
            needs: Vec::new(),
            payload,
        };
        unsigned.validate_shape()?;
        Ok(unsigned.seal(&self.identity))
    }

    /// Seals and appends a validly signed entry (`kind` v1, no deps, no keys)
    /// on the own log, writing the genesis first on first use. The entry is
    /// born processed, like a locally authored one. The shape limits are the
    /// real ones: a bad `kind` or payload is an `Err`, and `node.genesis` is
    /// refused (it exists only at counter 1).
    pub fn author_raw(
        &self,
        kind: &str,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Envelope, EnvelopeError> {
        let mut inner = self.lock();
        self.ensure_genesis(&mut inner);
        let log = self.self_log();
        let held = &inner.logs[&log];
        let head = held.last().expect("the own log holds its genesis");
        let entry = self.seal(&log, head.counter + 1, head.hash, kind, payload.into())?;
        let len = held.len() as u64 + 1;
        inner
            .logs
            .get_mut(&log)
            .expect("the own log exists")
            .push(entry.clone());
        inner.processed.insert(log, len);
        Ok(entry)
    }

    /// Test hook: the most forked entries `serve_branch` walks before it
    /// answers `None` (the default is the real bound).
    pub fn set_branch_walk_bound(&self, steps: u64) {
        self.lock().branch_walk_bound = steps;
    }

    /// Records entries of a forked branch of `log`, ignoring exact
    /// duplicates, like `journal::store_forked`, and returns how many were
    /// new. The log must be known (`NotFound`); each entry must name it and
    /// verify against its key (`InvalidRequest` otherwise).
    pub fn store_forked(&self, log: &LogKey, entries: &[Envelope]) -> Result<usize, ApiError> {
        let mut inner = self.lock();
        let Some(pubkey) = inner.pubkeys.get(log).copied() else {
            return Err(ApiError::new(ErrorCode::NotFound, "unknown log"));
        };
        let mut inserted = 0;
        for entry in entries {
            if entry.log != *log {
                return Err(ApiError::new(
                    ErrorCode::InvalidRequest,
                    "forked entry names another log",
                ));
            }
            entry.verify(&pubkey).map_err(|_| {
                ApiError::new(ErrorCode::InvalidRequest, "forked entry does not verify")
            })?;
            if inner
                .forked
                .insert((*log, entry.counter, entry.hash), entry.clone())
                .is_none()
            {
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    /// Closes `log` after counter `at`, lower-only, like `journal::freeze`:
    /// a request at or above an existing freeze changes nothing. Refuses the
    /// own log (`Conflict`) and an unknown log (`NotFound`).
    pub fn freeze(&self, log: &LogKey, at: u64) -> Result<(), ApiError> {
        let mut inner = self.lock();
        if !inner.logs.contains_key(log) {
            return Err(ApiError::new(ErrorCode::NotFound, "unknown log"));
        }
        if *log == self.self_log() {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "cannot freeze the active self log",
            ));
        }
        let at = inner.frozen.get(log).map_or(at, |current| at.min(*current));
        inner.frozen.insert(*log, at);
        Ok(())
    }
}

fn head_of(entries: &[Envelope], state: LogState, frozen_at: Option<u64>) -> LogHead {
    LogHead {
        counter: entries.len() as u64,
        head_hash: entries.last().map_or([0u8; 32], |e| e.hash),
        state,
        frozen_at: frozen_at.map(|at| i64::try_from(at).unwrap_or(i64::MAX)),
    }
}

fn cancelled(budget: &CallBudget) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        return Err(ApiError::new(ErrorCode::Cancelled, "call cancelled"));
    }
    Ok(())
}

fn malformed_batch(log: &LogKey, counter: u64, field: &'static str) -> IngestOutcome {
    IngestOutcome::Corrupt {
        log: *log,
        counter,
        reason: EnvelopeError::Malformed { field },
    }
}

impl JournalPort for MemoryJournal {
    fn vector(&self, budget: &CallBudget) -> Result<Vector, ApiError> {
        cancelled(budget)?;
        let inner = self.lock();
        Ok(Vector(
            inner
                .logs
                .iter()
                .map(|(log, entries)| {
                    let frozen = inner.frozen.get(log).copied();
                    let state = if frozen.is_some() {
                        LogState::Frozen
                    } else {
                        LogState::Active
                    };
                    (*log, head_of(entries, state, frozen))
                })
                .collect(),
        ))
    }

    fn stored_hash(
        &self,
        log: &LogKey,
        counter: u64,
        budget: &CallBudget,
    ) -> Result<Option<[u8; 32]>, ApiError> {
        cancelled(budget)?;
        let inner = self.lock();
        Ok(counter
            .checked_sub(1)
            .and_then(|ix| usize::try_from(ix).ok())
            .and_then(|ix| inner.logs.get(log)?.get(ix))
            .map(|e| e.hash))
    }

    fn serve(
        &self,
        log: &LogKey,
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Vec<Envelope>, ApiError> {
        cancelled(budget)?;
        let inner = self.lock();
        let Some(entries) = inner.logs.get(log) else {
            return Ok(Vec::new());
        };
        let start = usize::try_from(from.max(1) - 1).unwrap_or(usize::MAX);
        let mut out: Vec<Envelope> = Vec::new();
        let mut bytes = 0usize;
        for entry in entries.iter().skip(start) {
            if out.len() >= limits.entries.max(1) {
                break;
            }
            let size = entry.canonical_bytes().len();
            // Always at least one entry, even an oversized one.
            if !out.is_empty() && bytes.saturating_add(size) > limits.bytes {
                break;
            }
            bytes = bytes.saturating_add(size);
            out.push(entry.clone());
        }
        Ok(out)
    }

    fn serve_branch(
        &self,
        log: &LogKey,
        tip: (u64, [u8; 32]),
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Option<Vec<Envelope>>, ApiError> {
        cancelled(budget)?;
        let inner = self.lock();
        let Some(main) = inner.logs.get(log) else {
            return Ok(None);
        };
        let from = from.max(1);
        let held = |counter: u64| {
            counter
                .checked_sub(1)
                .and_then(|ix| usize::try_from(ix).ok())
                .and_then(|ix| main.get(ix))
        };
        let (tip_counter, tip_hash) = tip;
        // The branch ascending from `from`, whole; limits apply at the end.
        let mut branch: Vec<&Envelope> = Vec::new();
        if held(tip_counter).is_some_and(|e| e.hash == tip_hash) {
            branch.extend(
                main.iter()
                    .skip(usize::try_from(from - 1).unwrap_or(usize::MAX)),
            );
            branch.truncate(
                usize::try_from(tip_counter.saturating_sub(from - 1)).unwrap_or(usize::MAX),
            );
        } else {
            let bound = match inner.branch_walk_bound {
                0 => BRANCH_WALK_MAX_STEPS,
                bound => bound,
            };
            let mut forked: Vec<&Envelope> = Vec::new();
            let mut key = (tip_counter, tip_hash);
            let mut join = None;
            loop {
                let Some(entry) = inner.forked.get(&(*log, key.0, key.1)) else {
                    return Ok(None);
                };
                if forked.len() as u64 >= bound {
                    return Ok(None);
                }
                forked.push(entry);
                if entry.counter <= from {
                    break;
                }
                if held(entry.counter - 1).is_some_and(|e| e.hash == entry.prev_hash) {
                    join = Some(entry.counter - 1);
                    break;
                }
                key = (entry.counter - 1, entry.prev_hash);
            }
            if let Some(join) = join {
                branch.extend(
                    main.iter()
                        .skip(usize::try_from(from - 1).unwrap_or(usize::MAX)),
                );
                branch
                    .truncate(usize::try_from(join.saturating_sub(from - 1)).unwrap_or(usize::MAX));
            }
            branch.extend(forked.into_iter().rev().filter(|e| e.counter >= from));
        }
        let mut out: Vec<Envelope> = Vec::new();
        let mut bytes = 0usize;
        for entry in branch {
            if out.len() >= limits.entries.max(1) {
                break;
            }
            let size = entry.canonical_bytes().len();
            if !out.is_empty() && bytes.saturating_add(size) > limits.bytes {
                break;
            }
            bytes = bytes.saturating_add(size);
            out.push(entry.clone());
        }
        Ok(Some(out))
    }

    fn log_meta(&self, log: &LogKey, budget: &CallBudget) -> Result<Option<LogMeta>, ApiError> {
        cancelled(budget)?;
        let inner = self.lock();
        let (Some(entries), Some(pubkey)) = (inner.logs.get(log), inner.pubkeys.get(log)) else {
            return Ok(None);
        };
        let frozen_at = inner.frozen.get(log).copied();
        Ok(Some(LogMeta {
            pubkey: *pubkey,
            own: log.node == self.identity.uuid(),
            active: *log == self.self_log(),
            state: if frozen_at.is_some() {
                LogState::Frozen
            } else {
                LogState::Active
            },
            frozen_at,
            contiguous: entries.len() as u64,
            head: entries.last().map_or([0u8; 32], |e| e.hash),
        }))
    }

    fn ingest(
        &self,
        log: &LogKey,
        entries: Vec<Envelope>,
        budget: &CallBudget,
    ) -> Result<IngestOutcome, ApiError> {
        cancelled(budget)?;
        if entries.len() > INGEST_BATCH_MAX {
            return Ok(malformed_batch(log, 0, "batch"));
        }
        if let Some(stray) = entries.iter().find(|entry| entry.log != *log) {
            return Ok(malformed_batch(log, stray.counter, "log"));
        }
        let mut inner = self.lock();
        // Work on copies and write them back at the end, so a `Corrupt` entry
        // anywhere in the batch stores nothing, as the SQLite transaction does.
        let mut work: Option<Vec<Envelope>> = inner.logs.get(log).cloned();
        let mut pubkey = inner.pubkeys.get(log).copied();
        let frozen_at = inner.frozen.get(log).copied();
        let state = if frozen_at.is_some() {
            LogState::Frozen
        } else {
            LogState::Active
        };
        let is_active_self = *log == self.self_log();
        let (mut stored, mut duplicates) = (0usize, 0usize);
        let mut stop: Option<IngestOutcome> = None;
        for entry in &entries {
            let step = {
                let view = match (&work, pubkey) {
                    (Some(held), Some(pubkey)) => Some(HeldLog {
                        pubkey,
                        contiguous: held.len() as u64,
                        head_hash: held.last().map_or([0u8; 32], |e| e.hash),
                        state,
                        frozen_at,
                        is_active_self,
                        stored_hash: usize::try_from(entry.counter)
                            .ok()
                            .and_then(|c| c.checked_sub(1))
                            .and_then(|ix| held.get(ix))
                            .map(|e| e.hash),
                    }),
                    _ => None,
                };
                check_next(view.as_ref(), &StoredEntry::from(entry.clone()))
            };
            match step {
                ChainStep::Duplicate => duplicates += 1,
                ChainStep::Corrupt(reason) => {
                    return Ok(IngestOutcome::Corrupt {
                        log: *log,
                        counter: entry.counter,
                        reason,
                    });
                }
                ChainStep::AheadOfSelf => {
                    let held = work.as_deref().unwrap_or_default();
                    stop = Some(IngestOutcome::Fork {
                        stored,
                        evidence: ForkEvidence::AheadOfSelf {
                            log: *log,
                            my_head: head_of(held, state, frozen_at),
                            entry: Box::new(entry.clone()),
                        },
                    });
                    break;
                }
                ChainStep::Gap { expected } => {
                    stop = Some(IngestOutcome::Gap { stored, expected });
                    break;
                }
                ChainStep::BeyondFrozen => {
                    stop = Some(IngestOutcome::Frozen {
                        stored,
                        frozen_at: frozen_at.unwrap_or(0),
                    });
                    break;
                }
                ChainStep::Conflict => {
                    let held = work.as_deref().unwrap_or_default();
                    let held_counter = if entry.counter <= held.len() as u64 {
                        entry.counter
                    } else {
                        entry.counter - 1
                    };
                    let Some(held_entry) = usize::try_from(held_counter)
                        .ok()
                        .and_then(|c| c.checked_sub(1))
                        .and_then(|ix| held.get(ix))
                    else {
                        return Err(ApiError::new(
                            ErrorCode::StoreCorrupt,
                            "conflict without a held entry",
                        ));
                    };
                    stop = Some(IngestOutcome::Fork {
                        stored,
                        evidence: ForkEvidence::Conflict {
                            held: Box::new(held_entry.clone()),
                            conflicting: Box::new(entry.clone()),
                        },
                    });
                    break;
                }
                ChainStep::Genesis | ChainStep::Append => {
                    if let Err(reason) = entry.validate_shape() {
                        return Ok(IngestOutcome::Corrupt {
                            log: *log,
                            counter: entry.counter,
                            reason,
                        });
                    }
                    if step == ChainStep::Genesis {
                        match entry.genesis_pubkey() {
                            Ok(key) => pubkey = Some(key),
                            Err(reason) => {
                                return Ok(IngestOutcome::Corrupt {
                                    log: *log,
                                    counter: entry.counter,
                                    reason,
                                });
                            }
                        }
                        work = Some(Vec::new());
                    }
                    work.get_or_insert_with(Vec::new).push(entry.clone());
                    stored += 1;
                }
            }
        }
        let head = head_of(work.as_deref().unwrap_or_default(), state, frozen_at);
        if let (Some(work), Some(pubkey)) = (work, pubkey) {
            inner.pubkeys.insert(*log, pubkey);
            inner.logs.insert(*log, work);
        }
        Ok(stop.unwrap_or(IngestOutcome::Stored {
            stored,
            duplicates,
            head,
        }))
    }

    /// There are no handlers: every stored entry is marked processed and
    /// counted as applied.
    fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
        cancelled(budget)?;
        let mut inner = self.lock();
        let Inner {
            logs, processed, ..
        } = &mut *inner;
        let mut applied = 0usize;
        for (log, entries) in logs.iter() {
            let held = entries.len() as u64;
            let done = processed.insert(*log, held).unwrap_or(0);
            applied += usize::try_from(held - done).unwrap_or(usize::MAX);
        }
        Ok(ApplyProgress {
            applied,
            ..ApplyProgress::default()
        })
    }
}
