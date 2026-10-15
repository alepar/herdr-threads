//! The coordinator's serving half: `want` subscriptions, the live tail and
//! Have deltas (peer-link design, sections 9 and 11).
//!
//! Responsibility (serve)
//! - Owns: a link's armed subscriptions (`subs_out`: the next counter to push
//!   per log), the one serve job per link that reads committed rows through
//!   [`JournalPort::serve`], the recursive halving of a batch to the link's
//!   `max_frame`, and the throttled `have` deltas.
//! - Hides: that serving is `spawn_blocking` and that a full data queue stops
//!   a push until `CreditFreed` resumes it.
//! - Also (ht-5fr.6.8): the fork half of serving. A `branch_want` is one
//!   request, one reply, no subscription: at most [`MAX_BRANCH_OUTSTANDING`]
//!   per link in FIFO order through [`JournalPort::serve_branch`] (one job
//!   per link at a time), the rest and every branch this node cannot serve
//!   answered `branch_none`. Inbound `branch_entries`/`branch_none` go to the
//!   fork sink after a shape check, and the sink's outbox and fork refs go
//!   out to `fork.v1` links only. No fork policy lives here.
//! - Must not: touch SQLite, wait on a link's queue, or serve a log the
//!   actor's `local` vector does not yet show (a want for a log not held yet
//!   stays armed and is served once the log appears).
//! - Role: the send side of replication; the receive side is `ingest`.
//!
//! Core and harness-neutral.

use super::*;

/// Least gap between two `have` deltas on one link (ms).
const HAVE_DELTA_EVERY_MS: u64 = 1_000;

/// A finished serve job, joined by the actor.
pub(super) struct ServeDone {
    pub link: u64,
    pub log: LogKey,
    /// The counter the job read from; stale if the subscription moved since.
    pub from: u64,
    pub result: Result<Vec<Envelope>, ApiError>,
}

/// Encode `envelopes` as frame bodies (built by `build`) no longer than
/// `max_frame`, halving the batch recursively. A single entry that is still
/// too long goes alone (the link refuses it as `BadSize`). Each body comes
/// with the last counter it carries.
fn encode_halved(
    envelopes: &[Envelope],
    max_frame: usize,
    build: &dyn Fn(&[Envelope]) -> PeerFrame,
    out: &mut Vec<(Vec<u8>, u64)>,
) {
    let Some(last) = envelopes.last() else {
        return;
    };
    let Ok(body) = build(envelopes).encode() else {
        return;
    };
    if body.len() <= max_frame || envelopes.len() == 1 {
        out.push((body, last.counter));
        return;
    }
    let (head, tail) = envelopes.split_at(envelopes.len() / 2);
    encode_halved(head, max_frame, build, out);
    encode_halved(tail, max_frame, build, out);
}

/// The batch limits a peer's hello allows.
fn peer_batch_limits(peer: &PeerInfo) -> BatchLimits {
    BatchLimits {
        entries: (peer.limits.batch_entries as usize).clamp(1, BatchLimits::ENTRIES),
        bytes: (peer.limits.batch_bytes as usize).clamp(1, BatchLimits::BYTES),
    }
}

/// Most `branch_want`s one link may have outstanding (queued or in flight);
/// the rest are answered `branch_none`.
pub(super) const MAX_BRANCH_OUTSTANDING: usize = 4;

/// A finished `serve_branch` job, joined by the actor.
pub(super) struct BranchDone {
    pub link: u64,
    pub want: BranchWant,
    pub result: Result<Option<Vec<Envelope>>, ApiError>,
}

impl Actor {
    /// A peer's `want`: arm the subscription from `from` and start pushing.
    pub(super) fn on_want(&mut self, link_id: u64, log: LogKey, from: u64) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        entry.subs_out.insert(log, from.max(1));
        self.push_link(link_id);
    }

    /// A peer's `unwant`: drop the subscription.
    pub(super) fn on_unwant(&mut self, link_id: u64, log: &LogKey) {
        if let Some(entry) = self.links.get_mut(&link_id) {
            entry.subs_out.remove(log);
        }
    }

    /// Push every armed subscription that has something to send.
    pub(super) fn push_all(&mut self) {
        let ids: Vec<u64> = self
            .links
            .iter()
            .filter(|(_, l)| !l.subs_out.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.push_link(id);
        }
    }

    /// Start a serve job for the next armed log of `link_id` with committed
    /// rows to send, rotating from the log served last so one busy log does
    /// not starve the others. One job per link at a time.
    pub(super) fn push_link(&mut self, link_id: u64) {
        if self.serving.contains(&link_id) {
            return;
        }
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        if entry.mode != LinkMode::Active || entry.closing || !entry.ranked {
            return;
        }
        let ready = |log: &LogKey, next: &u64| {
            self.local
                .0
                .get(log)
                .is_some_and(|head| *next <= head.counter)
        };
        let after = entry.push_cursor;
        let pick = entry
            .subs_out
            .iter()
            .filter(|(log, next)| ready(log, next))
            .find(|(log, _)| after.is_none_or(|cursor| **log > cursor))
            .or_else(|| entry.subs_out.iter().find(|(log, next)| ready(log, next)))
            .map(|(log, next)| (*log, *next));
        let Some((log, from)) = pick else {
            return;
        };
        let limits = peer_batch_limits(entry.handle.peer());
        let journal = self.deps.journal.clone();
        self.serving.insert(link_id);
        self.serve_jobs.spawn_blocking(move || {
            let budget = CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            };
            let result = journal.serve(&log, from, limits, &budget);
            ServeDone {
                link: link_id,
                log,
                from,
                result,
            }
        });
    }

    /// A serve job finished: queue its rows as `entries` frames (halved to
    /// `max_frame`) while the link's data queue accepts, advance the
    /// subscription past what was queued, and keep pushing.
    pub(super) fn on_served(&mut self, done: ServeDone) {
        let ServeDone {
            link,
            log,
            from,
            result,
        } = done;
        self.serving.remove(&link);
        let Some(entry) = self.links.get_mut(&link) else {
            return;
        };
        if entry.subs_out.get(&log) != Some(&from) {
            // Re-asked or unwanted while the job ran: its rows are stale.
            self.push_link(link);
            return;
        }
        let envelopes = match result {
            Ok(envelopes) => envelopes,
            Err(e) => {
                (self.deps.log)(&format!(
                    "sync: serve {}/{} failed: {:?}",
                    uuid8(&log.node),
                    log.incarnation,
                    e.code
                ));
                return;
            }
        };
        if envelopes.is_empty() {
            // Nothing committed there yet: stay armed (live tail).
            return;
        }
        let max_frame = entry.handle.peer().limits.max_frame as usize;
        let mut bodies = Vec::new();
        let build = |batch: &[Envelope]| PeerFrame::Entries {
            node: log.node,
            inc: log.incarnation,
            entries: batch.iter().map(WireEnvelope::from).collect(),
        };
        encode_halved(&envelopes, max_frame, &build, &mut bodies);
        let mut sent_to = None;
        let mut blocked = false;
        for (body, last_counter) in bodies {
            match entry.handle.try_send_entries(body) {
                Ok(()) => sent_to = Some(last_counter),
                Err(SendError::Full | SendError::LinkGone) => {
                    blocked = true;
                    break;
                }
                Err(SendError::BadSize) => {
                    (self.deps.log)(&format!(
                        "sync: entry of {}/{} at {} exceeds the link's max frame; not served",
                        uuid8(&log.node),
                        log.incarnation,
                        last_counter
                    ));
                    entry.subs_out.remove(&log);
                    blocked = true;
                    break;
                }
            }
        }
        if let Some(last) = sent_to {
            if let Some(next) = entry.subs_out.get_mut(&log) {
                *next = last + 1;
            }
            entry.push_cursor = Some(log);
        }
        if !blocked {
            self.push_link(link);
        }
    }

    /// Send each active, announced link the logs whose head changed since the
    /// last `have` it got, at most once a second per link.
    pub(super) fn flush_have_deltas(&mut self, now: MonoInstant) {
        let ids: Vec<u64> = self
            .links
            .iter()
            .filter(|(_, l)| l.mode == LinkMode::Active && !l.closing && l.announced)
            .filter(|(_, l)| {
                l.last_have_at
                    .is_none_or(|at| now.0.saturating_sub(at.0) >= HAVE_DELTA_EVERY_MS)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let Some(entry) = self.links.get_mut(&id) else {
                continue;
            };
            let changed: Vec<(LogKey, LogHead)> = self
                .local
                .0
                .iter()
                .filter(|(key, head)| entry.last_have_sent.0.get(key) != Some(head))
                .take(MAX_HAVE_LOGS)
                .map(|(key, head)| (*key, *head))
                .collect();
            if changed.is_empty() {
                continue;
            }
            let logs = changed
                .iter()
                .map(|(key, head)| have_log(key, head))
                .collect();
            let sent = entry.handle.send(PeerFrame::Have {
                full: false,
                ask: false,
                logs,
                forks: Vec::new(),
            });
            if sent.is_ok() {
                for (key, head) in changed {
                    entry.last_have_sent.0.insert(key, head);
                }
                entry.last_have_at = Some(now);
            }
        }
    }
}

/// One `have` row for a local log head.
pub(super) fn have_log(key: &LogKey, head: &LogHead) -> HaveLog {
    HaveLog {
        node: key.node,
        inc: key.incarnation,
        counter: head.counter,
        head: hex_encode(&head.head_hash),
        frozen_at: match (head.state, head.frozen_at) {
            (LogState::Frozen, Some(at)) => u64::try_from(at).ok(),
            _ => None,
        },
    }
}

impl Actor {
    /// The fork refs to advertise, capped so a misbehaving sink cannot make
    /// the peer reject the `have`.
    pub(super) fn fork_refs(&self) -> Vec<ForkRef> {
        let mut refs = self.deps.fork_sink.fork_refs();
        refs.truncate(MAX_HAVE_FORKS);
        refs
    }

    /// A peer's `branch_want`: queue it (at most
    /// [`MAX_BRANCH_OUTSTANDING`] per link, else `branch_none`) and serve the
    /// queue's front. Ignored from a link without `fork.v1`.
    pub(super) fn on_branch_want(&mut self, link_id: u64, want: BranchWant) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        if !entry.fork_active() {
            return;
        }
        if entry.branch_queue.len() >= MAX_BRANCH_OUTSTANDING {
            send_branch_none(entry, &want);
            return;
        }
        entry.branch_queue.push_back(want);
        self.start_branch(link_id);
    }

    /// Start the serve job for the front of the link's queue, unless one is
    /// running. A tip that is not a hash cannot be served: `branch_none`.
    fn start_branch(&mut self, link_id: u64) {
        loop {
            let Some(entry) = self.links.get_mut(&link_id) else {
                return;
            };
            if entry.branch_busy {
                return;
            }
            let Some(want) = entry.branch_queue.front().cloned() else {
                return;
            };
            let Some(tip_hash) = hex_decode::<32>(&want.tip.hash) else {
                entry.branch_queue.pop_front();
                send_branch_none(entry, &want);
                continue;
            };
            entry.branch_busy = true;
            let limits = peer_batch_limits(entry.handle.peer());
            let journal = self.deps.journal.clone();
            self.branch_jobs.spawn_blocking(move || {
                let budget = CallBudget {
                    deadline: MonoInstant(u64::MAX),
                    cancellation: Cancellation::default(),
                };
                let log = LogKey {
                    node: want.node,
                    incarnation: want.inc,
                };
                let result = journal.serve_branch(
                    &log,
                    (want.tip.counter, tip_hash),
                    want.from,
                    limits,
                    &budget,
                );
                BranchDone {
                    link: link_id,
                    want,
                    result,
                }
            });
            return;
        }
    }

    /// A `serve_branch` job finished: answer the request (halved to
    /// `max_frame`, or `branch_none`) and start the next queued one. A reply
    /// whose request left the queue (the link was demoted meanwhile) is
    /// dropped.
    pub(super) fn on_branch_served(&mut self, done: BranchDone) {
        let BranchDone { link, want, result } = done;
        if let Some(entry) = self.links.get_mut(&link) {
            entry.branch_busy = false;
            if entry.branch_queue.front() == Some(&want) {
                entry.branch_queue.pop_front();
                let batch = match result {
                    Ok(Some(batch)) if !batch.is_empty() => Some(batch),
                    Ok(_) => None,
                    Err(e) => {
                        (self.deps.log)(&format!(
                            "sync: serve_branch {}/{} failed: {:?}",
                            uuid8(&want.node),
                            want.inc,
                            e.code
                        ));
                        None
                    }
                };
                let sent = batch.is_some_and(|batch| send_branch_entries(entry, &want, &batch));
                if !sent {
                    send_branch_none(entry, &want);
                }
            }
        }
        self.start_branch(link);
    }

    /// An inbound `branch_entries`/`branch_none`: a shape error drops the
    /// frame and counts against the source (`on_corrupt`); otherwise the sink
    /// gets it, and the credit goes back after.
    pub(super) fn on_branch_reply(&mut self, link_id: u64, frame: PeerFrame, wire_len: usize) {
        let len = u32::try_from(wire_len).unwrap_or(u32::MAX);
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        let source = entry.handle.peer().node;
        let fork_cap = entry.fork_cap();
        match frame {
            PeerFrame::BranchEntries {
                node,
                inc,
                tip_hash,
                entries,
                more,
            } => {
                if !fork_cap {
                    self.grant(link_id, len);
                    return;
                }
                let decoded: Result<Vec<Envelope>, _> =
                    entries.into_iter().map(Envelope::try_from).collect();
                let Ok(decoded) = decoded else {
                    self.grant(link_id, len);
                    let log = LogKey {
                        node,
                        incarnation: inc,
                    };
                    self.on_corrupt(link_id, log, "malformed branch entry");
                    return;
                };
                self.deps.fork_sink.on_branch(
                    source,
                    BranchReply::Entries {
                        node,
                        inc,
                        tip_hash,
                        entries: decoded.iter().map(WireEnvelope::from).collect(),
                        more,
                    },
                );
                self.grant(link_id, len);
            }
            PeerFrame::BranchNone {
                node,
                inc,
                tip_hash,
            } if fork_cap => {
                self.deps.fork_sink.on_branch(
                    source,
                    BranchReply::None {
                        node,
                        inc,
                        tip_hash,
                    },
                );
            }
            _ => {}
        }
    }

    /// The 1 s tick: hand the sink's `branch_want`s to the active `fork.v1`
    /// link of their node (dropped when there is none), and when its refs
    /// changed send one `have` delta carrying them to every such link.
    pub(super) fn drain_fork_outbox(&mut self) {
        let ForkOutbox {
            sends,
            refs_changed,
        } = self.deps.fork_sink.outbox();
        for (node, want) in sends {
            let link = self
                .by_node
                .get(&node)
                .into_iter()
                .flatten()
                .find_map(|id| self.links.get(id).filter(|l| l.fork_active()));
            if let Some(entry) = link {
                let _ = entry.handle.send(PeerFrame::BranchWant(want));
            }
        }
        if refs_changed {
            let forks = self.fork_refs();
            for entry in self
                .links
                .values()
                .filter(|l| l.fork_active() && l.announced)
            {
                let _ = entry.handle.send(PeerFrame::Have {
                    full: false,
                    ask: false,
                    logs: Vec::new(),
                    forks: forks.clone(),
                });
            }
        }
    }
}

/// Answer `want` with `branch_none`.
fn send_branch_none(entry: &LinkEntry, want: &BranchWant) {
    let _ = entry.handle.send(PeerFrame::BranchNone {
        node: want.node,
        inc: want.inc,
        tip_hash: want.tip.hash.clone(),
    });
}

/// Queue `batch` as `branch_entries` frames halved to the link's
/// `max_frame`; `false` when any frame did not fit the data queue.
fn send_branch_entries(entry: &LinkEntry, want: &BranchWant, batch: &[Envelope]) -> bool {
    let max_frame = entry.handle.peer().limits.max_frame as usize;
    let build = |part: &[Envelope]| PeerFrame::BranchEntries {
        node: want.node,
        inc: want.inc,
        tip_hash: want.tip.hash.clone(),
        entries: part.iter().map(WireEnvelope::from).collect(),
        more: part.last().is_some_and(|e| e.counter < want.tip.counter),
    };
    let mut bodies = Vec::new();
    encode_halved(batch, max_frame, &build, &mut bodies);
    !bodies.is_empty()
        && bodies
            .into_iter()
            .all(|(body, _)| entry.handle.try_send_entries(body).is_ok())
}
