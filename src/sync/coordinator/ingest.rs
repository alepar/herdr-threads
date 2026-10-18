//! The coordinator's ingest half: received `entries` frames, the one
//! sequential ingest worker and the handling of every [`IngestOutcome`]
//! (peer-link design, sections 9 and 10).
//!
//! Responsibility (ingest)
//! - Owns: the unsolicited-`entries` rule (dropped, credit granted, counted),
//!   the FIFO of batches headed for the worker, the worker task (one batch at
//!   a time, so per-log order holds across a source switch) and the one
//!   exhaustive `match` on [`IngestOutcome`] with the penalty, pause and
//!   retry bookkeeping each outcome needs.
//! - Hides: the `spawn_blocking`, `Lane::Sync` origin and `FairWriter`
//!   background turn around `JournalPort::ingest`.
//! - Also owns: authenticating an oversize opaque claim (log, position, prev,
//!   prefix, hash, signature) before it may stall a log; an unauthenticated
//!   claim refuses only its source.
//! - Must not: decide fork policy (that is a [`ForkSink`]'s), touch SQLite, or
//!   keep a batch's credit: the frame's bytes go back to the sender as soon
//!   as its outcome is known, whatever it is.
//! - Role: the data-moving half of the receive side; routing (`reselect`)
//!   stays the actor's.
//!
//! Core and harness-neutral.

use super::*;
use crate::service::kicks::{Lane, enter_lane};
use crate::sync::envelope::OPAQUE_MAX_BYTES;
use crate::sync::envelope::{EntryPrefix, MAGIC, check_opaque_size, hex_decode, hex_decode_vec};
use crate::sync::frame::{OpaqueWire, WireItemError};
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// How long a source that served corrupt bytes is unwantable for that log (ms).
pub(super) const CORRUPT_PENALTY_MS: u64 = 600_000;
/// How long every want is paused after `Headroom` (ms).
pub(super) const HEADROOM_PAUSE_MS: u64 = 60_000;
/// How long the ingest worker waits for its `FairWriter` background turn (ms).
const INGEST_TURN_MS: u64 = 5_000;

/// A batch for the worker.
pub(super) struct IngestJob {
    pub link: u64,
    /// The node on the other end of `link`, for fork reports.
    pub source: NodeUuid,
    pub log: LogKey,
    pub entries: Vec<StoredEntry>,
    /// The frame carried an opaque item over `OPAQUE_MAX_BYTES` right after
    /// `entries`. The batch stores `entries`, then authenticates the claim:
    /// [`IngestOutcome::Oversize`] only when it is the log's next entry.
    pub oversize: Option<OversizeClaim>,
    /// The frame's wire length: the credit to return once the batch is done.
    pub len: u32,
}

/// An oversize opaque item as received: nothing about it is verified yet.
pub(super) struct OversizeClaim {
    pub wire: OpaqueWire,
    /// Its raw length in bytes.
    pub len: usize,
}

/// The worker's verdict: what the port said, or an oversize opaque item that
/// could not be authenticated as the log's next entry.
#[derive(Debug, PartialEq)]
pub(super) enum IngestVerdict {
    Port(IngestOutcome),
    /// The `stored` prefix was kept (`head` is its new head, if any was
    /// stored); only this source is refused for the log.
    OversizeRefused {
        stored: usize,
        head: Option<LogHead>,
        counter: u64,
        why: &'static str,
    },
}

/// A worker result, joined by the actor.
pub(super) struct IngestDone {
    pub link: u64,
    pub source: NodeUuid,
    pub log: LogKey,
    pub len: u32,
    pub result: Result<IngestVerdict, ApiError>,
}

/// What the worker needs; none of it is actor state.
pub(super) struct WorkerDeps {
    pub journal: Arc<dyn JournalPort>,
    pub writer: Arc<FairWriter>,
    pub clock: Arc<dyn Clock>,
    pub cancel: Cancellation,
}

/// Start the worker task. It ends when `jobs` closes, on cancellation or when
/// the actor stops listening.
pub(super) fn spawn_worker(
    deps: WorkerDeps,
    mut jobs: mpsc::UnboundedReceiver<IngestJob>,
    done: mpsc::Sender<IngestDone>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let job = tokio::select! {
                job = jobs.recv() => job,
                () = deps.cancel.cancelled() => None,
            };
            let Some(IngestJob {
                link,
                source,
                log,
                entries,
                oversize,
                len,
            }) = job
            else {
                break;
            };
            let journal = deps.journal.clone();
            let writer = deps.writer.clone();
            let clock = deps.clock.clone();
            let cancel = deps.cancel.clone();
            let result = tokio::task::spawn_blocking(move || {
                ingest_batch(
                    journal.as_ref(),
                    &writer,
                    clock.as_ref(),
                    cancel,
                    &log,
                    entries,
                    oversize,
                )
            })
            .await
            .unwrap_or_else(|e| {
                Err(ApiError::new(
                    ErrorCode::StoreBusy,
                    format!("ingest task failed: {e}"),
                ))
            });
            let finished = IngestDone {
                link,
                source,
                log,
                len,
                result,
            };
            if done.send(finished).await.is_err() {
                break;
            }
        }
    })
}

/// One batch on a blocking thread: `Lane::Sync` origin, a `FairWriter`
/// background turn, then `JournalPort::ingest` under the turn.
pub(super) fn ingest_batch(
    journal: &dyn JournalPort,
    writer: &FairWriter,
    clock: &dyn Clock,
    cancel: Cancellation,
    log: &LogKey,
    entries: Vec<StoredEntry>,
    oversize: Option<OversizeClaim>,
) -> Result<IngestVerdict, ApiError> {
    let _origin = enter_lane(Lane::Sync);
    let turn_budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(INGEST_TURN_MS)),
        cancellation: cancel.clone(),
    };
    let _turn = writer.enter_background(&turn_budget, clock)?;
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: cancel,
    };
    let outcome = if entries.is_empty() {
        None
    } else {
        Some(journal.ingest(log, entries, &budget)?)
    };
    let (stored, head, claim) = match (outcome, oversize) {
        (Some(IngestOutcome::Stored { stored, head, .. }), Some(claim)) => {
            (stored, Some(head), claim)
        }
        (None, Some(claim)) => (0, None, claim),
        (Some(other), _) => return Ok(IngestVerdict::Port(other)),
        (None, None) => unreachable!("an empty batch without an oversize entry is never queued"),
    };
    // Only an item authenticated as the log's next entry may stall the log;
    // anything else refuses just this source. `log_meta` reads after the
    // prefix was stored, so `contiguous` and `head` include it.
    let counter = claim.wire.counter;
    Ok(
        match authenticate_oversize(journal, log, &claim, &budget)? {
            Auth::Verified => IngestVerdict::Port(IngestOutcome::Oversize {
                stored,
                counter,
                len: claim.len,
            }),
            Auth::PastFreeze(frozen_at) => {
                IngestVerdict::Port(IngestOutcome::Frozen { stored, frozen_at })
            }
            Auth::Refused(why) => IngestVerdict::OversizeRefused {
                stored,
                head,
                counter,
                why,
            },
        },
    )
}

/// What authenticating an oversize claim found.
enum Auth {
    Verified,
    /// Authentic but beyond the counter at which the log froze.
    PastFreeze(u64),
    Refused(&'static str),
}

/// Check an oversize claim against the frame's log and the local chain: same
/// log, the next position after the head, prefix, hash over the raw bytes and
/// the author's signature. The raw bytes are hashed because a `(hash, sig)`
/// pair is public: without binding it to the position, a peer could replay an
/// earlier entry's pair at `head + 1` and stall the log. The signature check
/// mirrors `envelope::verify_hash_sig` (`verify_strict`, private there).
fn authenticate_oversize(
    journal: &dyn JournalPort,
    log: &LogKey,
    claim: &OversizeClaim,
    budget: &CallBudget,
) -> Result<Auth, ApiError> {
    let wire = &claim.wire;
    let named = hex_decode::<16>(&wire.node).map(|node| LogKey {
        node: NodeUuid::from_bytes(node),
        incarnation: wire.inc,
    });
    if named != Some(*log) {
        return Ok(Auth::Refused("names another log"));
    }
    let Some(meta) = journal.log_meta(log, budget)? else {
        return Ok(Auth::Refused("unknown log"));
    };
    if let Some(frozen_at) = meta.frozen_at
        && wire.counter > frozen_at
    {
        return Ok(Auth::PastFreeze(frozen_at));
    }
    if wire.counter != meta.contiguous + 1 {
        return Ok(Auth::Refused("not the next position"));
    }
    let prev = hex_decode::<32>(&wire.prev);
    if prev != Some(meta.head) {
        return Ok(Auth::Refused("prev does not follow the head"));
    }
    let (Some(hash), Some(sig)) = (hex_decode::<32>(&wire.hash), hex_decode::<64>(&wire.sig))
    else {
        return Ok(Auth::Refused("malformed hash or signature"));
    };
    let prefix = hex_decode_vec(&wire.raw).and_then(|raw| {
        let parsed = EntryPrefix::parse(&raw).ok()?;
        let matches = parsed.magic != *MAGIC
            && parsed.log == *log
            && parsed.counter == wire.counter
            && parsed.prev_hash == meta.head;
        matches.then_some(raw)
    });
    let Some(raw) = prefix else {
        return Ok(Auth::Refused("prefix does not match"));
    };
    if <[u8; 32]>::from(Sha256::digest(&raw)) != hash {
        return Ok(Auth::Refused("hash mismatch"));
    }
    let signed = VerifyingKey::from_bytes(&meta.pubkey).is_ok_and(|key| {
        key.verify_strict(&hash, &Signature::from_bytes(&sig))
            .is_ok()
    });
    if !signed {
        return Ok(Auth::Refused("bad signature"));
    }
    Ok(Auth::Verified)
}

impl Actor {
    /// An `entries` frame arrived on `link_id`. Not from the current rx source
    /// of the log: dropped, its credit granted and counted. Otherwise decoded
    /// (a shape error is `Corrupt` for the log) and queued for the worker.
    pub(super) fn on_entries(
        &mut self,
        link_id: u64,
        node: NodeUuid,
        inc: u32,
        entries: Vec<WireItem>,
        wire_len: usize,
    ) {
        let log = LogKey {
            node,
            incarnation: inc,
        };
        let len = u32::try_from(wire_len).unwrap_or(u32::MAX);
        let routed = self.rx.get(&log).is_some_and(|src| src.link == link_id)
            && self
                .links
                .get(&link_id)
                .is_some_and(|l| l.mode == LinkMode::Active && !l.closing);
        if !routed {
            self.grant(link_id, len);
            let in_grace = self
                .replaced
                .get(&(log, link_id))
                .is_some_and(|until| until.0 > self.mono().0);
            if !in_grace && let Some(entry) = self.links.get_mut(&link_id) {
                entry.unsolicited += 1;
            }
            return;
        }
        // In order, up to the first opaque item over the size bound: it is
        // carried as an unverified claim for the worker, the entries before
        // it are kept.
        let mut decoded = Vec::with_capacity(entries.len());
        let mut oversize = None;
        for item in entries {
            // The size is checked here, before the item is decoded, so the
            // wire struct moves into the claim without copying its raw hex.
            if let WireItem::Opaque(wire) = &item
                && check_opaque_size(wire.raw.len() / 2).is_err()
            {
                let WireItem::Opaque(wire) = item else {
                    unreachable!("matched above")
                };
                oversize = Some(OversizeClaim {
                    len: wire.raw.len() / 2,
                    wire,
                });
                break;
            }
            match item.into_stored() {
                Ok(entry) => decoded.push(entry),
                // Not reachable: oversize items were taken above. Treated as
                // a malformed entry rather than a panic.
                Err(WireItemError::TooLarge(_) | WireItemError::Envelope(_)) => {
                    self.grant(link_id, len);
                    self.on_corrupt(link_id, log, "malformed entry");
                    return;
                }
            }
        }
        if decoded.is_empty() && oversize.is_none() {
            self.grant(link_id, len);
            return;
        }
        let source = self
            .links
            .get(&link_id)
            .map_or(node, |l| l.handle.peer().node);
        let job = IngestJob {
            link: link_id,
            source,
            log,
            entries: decoded,
            oversize,
            len,
        };
        if self.ingest_jobs.send(job).is_err() {
            self.grant(link_id, len);
        }
    }

    /// Return `len` bytes of credit to the sender on `link_id`.
    pub(super) fn grant(&self, link_id: u64, len: u32) {
        if let Some(entry) = self.links.get(&link_id) {
            // A grant is never dropped while the link lives (it accumulates in
            // the link's pending counter, whatever the control queue holds).
            // The only error is `LinkGone`: the link's `Ended` event follows
            // and drops the entry, so there is nothing to do.
            let _ = entry.handle.grant(len);
        }
    }

    /// A worker result: the credit goes back first, then the outcome is acted
    /// on.
    pub(super) fn on_ingested(&mut self, done: IngestDone) {
        let IngestDone {
            link,
            source,
            log,
            len,
            result,
        } = done;
        self.grant(link, len);
        let outcome = match result {
            Ok(IngestVerdict::Port(outcome)) => outcome,
            Ok(IngestVerdict::OversizeRefused {
                head, counter, why, ..
            }) => {
                self.on_oversize_refused(link, log, head, counter, why);
                self.update_rounds();
                return;
            }
            Err(e) => {
                self.on_ingest_error(log, &e);
                return;
            }
        };
        // New outcomes are handled here; fork policy goes in a `ForkSink`
        // implementation, never in this match.
        match outcome {
            IngestOutcome::Stored { head, .. } => self.on_stored(link, log, head),
            IngestOutcome::Gap { expected, .. } => self.on_gap(link, log, expected),
            IngestOutcome::Fork { evidence, .. } => {
                match self.deps.fork_sink.on_fork(log, evidence, source) {
                    ForkAck::Paused => self.pause_log(log),
                    ForkAck::Corrupt => self.on_corrupt(link, log, "fork evidence not proven"),
                }
            }
            IngestOutcome::Frozen { .. } => {
                // The log froze: stop pulling past it. The refreshed local
                // vector says how far `wantable` still allows.
                self.drop_rx(log);
                self.request_local();
            }
            IngestOutcome::Corrupt {
                counter, reason, ..
            } => {
                self.on_corrupt(link, log, &format!("counter {counter}: {reason:?}"));
            }
            IngestOutcome::Oversize { counter, len, .. } => {
                self.on_oversize(log, counter, len);
            }
            IngestOutcome::Headroom => self.on_headroom(),
        }
        // A pause, penalty or freeze can make the rest of a round unwantable.
        self.update_rounds();
    }

    /// An **authenticated** opaque entry over [`OPAQUE_MAX_BYTES`]: the
    /// entries before it are stored, the log stalls on this node (no source is penalised, none is
    /// asked again) and the size is logged once.
    fn on_oversize(&mut self, log: LogKey, counter: u64, len: usize) {
        self.drop_rx(log);
        if self.oversize_logs.insert(log) {
            (self.deps.log)(&format!(
                "sync: log {}/{} entry {counter} is an opaque entry of {len} bytes over {OPAQUE_MAX_BYTES}; log stalled on this node",
                uuid8(&log.node),
                log.incarnation
            ));
        }
        self.request_local();
    }

    /// An oversize opaque item that is not authenticated as the log's next
    /// entry refuses only this source (time-limited penalty), never the log:
    /// the stored prefix is kept and relayed, other sources are asked.
    fn on_oversize_refused(
        &mut self,
        link_id: u64,
        log: LogKey,
        head: Option<LogHead>,
        counter: u64,
        why: &str,
    ) {
        if let Some(head) = head {
            self.on_stored(link_id, log, head);
        }
        self.on_corrupt(
            link_id,
            log,
            &format!("counter {counter}: unverified oversize opaque entry: {why}"),
        );
    }

    /// `Unwant` the log on its rx source and forget the source.
    fn drop_rx(&mut self, log: LogKey) {
        if let Some(src) = self.rx.remove(&log) {
            self.send_unwant(src.link, &log);
        }
    }

    fn on_stored(&mut self, link_id: u64, log: LogKey, head: LogHead) {
        let now = self.mono();
        let counter = head.counter;
        let newer = self.local.0.get(&log).is_none_or(|h| h.counter <= counter);
        if newer {
            self.local.0.insert(log, head);
        }
        if let Some(src) = self.rx.get_mut(&log)
            && src.link == link_id
        {
            src.last_progress = now;
        }
        // Contact: the link's node is alive; the log's author is relayed
        // (not directly heard) unless it is the link's node.
        let wall = self.wall();
        let link_node = self.links.get(&link_id).map(|l| l.handle.peer().node);
        if let Some(node) = link_node {
            self.contacts.record_direct(node, wall);
        }
        if link_node != Some(log.node) {
            let author = SeenAge {
                node: log.node,
                age_ms: 0,
            };
            self.contacts.merge_seen(&[author], wall);
        }
        self.retry_logs.remove(&log);
        if self.retry_logs.is_empty() && self.deps.pacer.attempts() > 0 {
            self.deps.pacer.on_success();
        }
        self.deps.status.record_success(self.deps.clock.utc_now());
        // Relay to the other subscribers of the log.
        let subscribers: Vec<u64> = self
            .links
            .iter()
            .filter(|(id, l)| **id != link_id && l.subs_out.contains_key(&log))
            .map(|(id, _)| *id)
            .collect();
        for id in subscribers {
            self.push_link(id);
        }
        self.reselect_log(log);
        self.update_own_history();
        self.flush_have_deltas(now);
        self.update_rounds();
    }

    /// The batch skipped ahead: ask the same source again from `expected`,
    /// unless that is exactly what was last asked (a stale batch of an
    /// earlier subscription).
    fn on_gap(&mut self, link_id: u64, log: LogKey, expected: u64) {
        let Some(src) = self.rx.get_mut(&log) else {
            return;
        };
        if src.link != link_id || src.requested_from == expected {
            return;
        }
        src.requested_from = expected;
        src.last_progress = self.deps.clock.monotonic_now();
        if let Some(entry) = self.links.get(&link_id) {
            let _ = entry.handle.send(PeerFrame::Want {
                node: log.node,
                inc: log.incarnation,
                from: expected,
            });
        }
    }

    /// Penalize `(link, log)` for [`CORRUPT_PENALTY_MS`], stop pulling the log
    /// from it and pick another source. With none, the same source is
    /// re-wanted when the penalty expires.
    pub(super) fn on_corrupt(&mut self, link_id: u64, log: LogKey, why: &str) {
        let now = self.mono();
        (self.deps.log)(&format!(
            "sync: corrupt {}/{} from link {link_id} ({why}); source penalized",
            uuid8(&log.node),
            log.incarnation
        ));
        if let Some(entry) = self.links.get_mut(&link_id) {
            entry
                .errors
                .insert(log, MonoInstant(now.0 + CORRUPT_PENALTY_MS));
        }
        if self.rx.get(&log).is_some_and(|src| src.link == link_id) {
            self.drop_rx(log);
        }
        self.reselect_log(log);
    }

    /// The sink holds the fork: no `Want` for the log until it reports the
    /// log resumed.
    fn pause_log(&mut self, log: LogKey) {
        self.paused.insert(log);
        self.drop_rx(log);
    }

    /// No disk headroom: `Unwant` every rx subscription and pause all wants.
    /// The housekeeping tick re-picks when the pause is over.
    fn on_headroom(&mut self) {
        let now = self.mono();
        self.headroom_until = Some(MonoInstant(now.0 + HEADROOM_PAUSE_MS));
        let sources = std::mem::take(&mut self.rx);
        for (log, src) in sources {
            self.send_unwant(src.link, &log);
        }
    }

    /// A store error: report it on the sync lane and ask again after the
    /// pacer's backoff (see [`Actor::on_retry_due`]).
    fn on_ingest_error(&mut self, log: LogKey, error: &ApiError) {
        // A call that shutdown cancelled is not a lane fault.
        if !self.deps.cancel.is_cancelled() {
            self.deps.status.record_failure(Lane::Sync, error);
        }
        self.retry_logs.insert(log);
        if self.deps.pacer.attempts() == 0 {
            self.deps.pacer.on_failure();
        }
    }

    /// The pacer's backoff is due: ask again for every log whose batch failed
    /// to store. While any remain, the next step is scheduled at once (an
    /// ingest `Stored` clears it), so a due backoff never spins.
    pub(super) fn on_retry_due(&mut self) {
        let logs = std::mem::take(&mut self.retry_logs);
        if logs.is_empty() {
            self.deps.pacer.on_success();
            return;
        }
        for log in logs {
            match self.rx.get(&log).map(|src| src.link) {
                Some(link) => self.start_rx(log, link),
                None => self.reselect_log(log),
            }
        }
        self.deps.pacer.on_failure();
    }

    /// Resume the logs the fork sink reports handled, and end an expired
    /// headroom pause.
    pub(super) fn lift_pauses(&mut self, now: MonoInstant) {
        let mut lifted = false;
        for log in self.deps.fork_sink.resumed() {
            if self.paused.remove(&log) {
                lifted = true;
                self.reselect_log(log);
            }
        }
        if lifted {
            self.request_local();
        }
        if self.headroom_until.is_some_and(|until| until.0 <= now.0) {
            self.headroom_until = None;
            self.reselect_all();
        }
    }
}
