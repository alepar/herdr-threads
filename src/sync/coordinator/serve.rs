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

/// Encode `envelopes` as `entries` frame bodies no longer than `max_frame`,
/// halving the batch recursively. A single entry that is still too long goes
/// alone (the link refuses it as `BadSize`). Each body comes with the last
/// counter it carries.
fn encode_halved(
    log: &LogKey,
    envelopes: &[Envelope],
    max_frame: usize,
    out: &mut Vec<(Vec<u8>, u64)>,
) {
    let Some(last) = envelopes.last() else {
        return;
    };
    let frame = PeerFrame::Entries {
        node: log.node,
        inc: log.incarnation,
        entries: envelopes.iter().map(WireEnvelope::from).collect(),
    };
    let Ok(body) = frame.encode() else {
        return;
    };
    if body.len() <= max_frame || envelopes.len() == 1 {
        out.push((body, last.counter));
        return;
    }
    let (head, tail) = envelopes.split_at(envelopes.len() / 2);
    encode_halved(log, head, max_frame, out);
    encode_halved(log, tail, max_frame, out);
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
        let limits = {
            let peer = &entry.handle.peer().limits;
            BatchLimits {
                entries: (peer.batch_entries as usize).clamp(1, BatchLimits::ENTRIES),
                bytes: (peer.batch_bytes as usize).clamp(1, BatchLimits::BYTES),
            }
        };
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
        encode_halved(&log, &envelopes, max_frame, &mut bodies);
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
