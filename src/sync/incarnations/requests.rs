//! The branch-request table: which source a `branch_want` goes to, how long
//! it may stay unanswered, and which `(source, log)` pairs are avoided.
//!
//! Owns: at most one outstanding request per branch, held from the send until
//! the worker has handled the reply ([`RequestTable::clear`]; an answered
//! request still blocks a new one, else the worker would ask for the same
//! `from` again while the reply waits in its queue), the 30 s timeout with
//! failover to the next advertising source, the 10-minute `(source, log)`
//! penalties and the oldest-last-request preference. Hides: nothing the
//! sink does not also pass in; the advertising sources of a tip arrive as
//! arguments. Must not: share penalties with the coordinator's rx-source
//! penalties (this table's are its own), send a frame, read a clock (every
//! method takes `now`), or block.
//!
//! Role: the state inside [`super::IncarnationsSink`]'s mutex; the worker's
//! 1 s timer calls [`RequestTable::sweep`], never the coordinator tick.

use std::collections::BTreeMap;

use crate::node::NodeUuid;
use crate::protocol::time::MonoInstant;
use crate::sync::LogKey;
use crate::sync::frame::BranchWant;

/// How long a `branch_want` may stay unanswered before it fails over.
pub const REQUEST_TIMEOUT_MS: u64 = 30_000;
/// How long a `(source, log)` pair is avoided after a bad reply.
pub const PENALTY_MS: u64 = 600_000;
/// Last-request stamps older than this are forgotten (the source is then
/// treated as never asked).
const LAST_REQUEST_KEEP_MS: u64 = 6 * PENALTY_MS;

/// One branch: a log and the hash of the tip the branch ends at.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BranchKey {
    pub log: LogKey,
    /// Tip hash as 64 lowercase hex characters, as on the wire.
    pub tip_hash: String,
}

impl BranchKey {
    pub fn of(want: &BranchWant) -> Self {
        Self {
            log: LogKey {
                node: want.node,
                incarnation: want.inc,
            },
            tip_hash: want.tip.hash.clone(),
        }
    }
}

/// A request sent and not yet answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outstanding {
    pub source: NodeUuid,
    pub want: BranchWant,
    pub sent_at: MonoInstant,
    /// Sources this branch already failed over from.
    tried: Vec<NodeUuid>,
    /// The reply arrived and waits for the worker; the request stays until
    /// [`RequestTable::clear`].
    answered: bool,
}

/// What [`RequestTable::fail`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failover {
    /// Re-requested from another source.
    Resent(NodeUuid, BranchWant),
    /// No untried, unpenalized source is left; the request is cleared.
    Exhausted(BranchKey),
    /// Nothing was outstanding for the branch.
    Unknown,
}

/// What [`RequestTable::sweep`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    pub sends: Vec<(NodeUuid, BranchWant)>,
    pub exhausted: Vec<BranchKey>,
}

#[derive(Debug, Default)]
pub struct RequestTable {
    outstanding: BTreeMap<BranchKey, Outstanding>,
    last_request: BTreeMap<(NodeUuid, LogKey), MonoInstant>,
    /// Penalty end per `(source, log)`.
    penalties: BTreeMap<(NodeUuid, LogKey), MonoInstant>,
}

impl RequestTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request `want` from one of `sources`. `None` when the branch already
    /// has a request outstanding, or no source is usable (all penalized).
    pub fn request(
        &mut self,
        want: BranchWant,
        sources: &[NodeUuid],
        now: MonoInstant,
    ) -> Option<NodeUuid> {
        let key = BranchKey::of(&want);
        if self.outstanding.contains_key(&key) {
            return None;
        }
        let source = self.pick(&key.log, sources, &[], now)?;
        self.send(key, source, want, Vec::new(), now);
        Some(source)
    }

    /// Match a reply from `source` to its outstanding request and mark it
    /// answered; `None` (drop the reply) when there is none from that source
    /// or it was answered already. The request stays outstanding until the
    /// worker calls [`clear`](Self::clear) after handling the reply.
    pub fn answer(&mut self, source: NodeUuid, key: &BranchKey) -> Option<Outstanding> {
        let outstanding = self.outstanding.get_mut(key)?;
        if outstanding.source != source || outstanding.answered {
            return None;
        }
        outstanding.answered = true;
        Some(outstanding.clone())
    }

    /// Avoid `(source, log)` for [`PENALTY_MS`].
    pub fn penalize(&mut self, source: NodeUuid, log: LogKey, now: MonoInstant) {
        self.penalties
            .insert((source, log), MonoInstant(now.0.saturating_add(PENALTY_MS)));
    }

    pub fn is_penalized(&self, source: NodeUuid, log: LogKey, now: MonoInstant) -> bool {
        self.penalties
            .get(&(source, log))
            .is_some_and(|until| now < *until)
    }

    /// Forget the outstanding request of `key`; whether there was one.
    pub fn clear(&mut self, key: &BranchKey) -> bool {
        self.outstanding.remove(key).is_some()
    }

    pub fn outstanding(&self, key: &BranchKey) -> Option<&Outstanding> {
        self.outstanding.get(key)
    }

    pub fn outstanding_count(&self) -> usize {
        self.outstanding.len()
    }

    /// The request of `key` failed (`branch_none`, or a timeout): move it to
    /// the next usable source that it has not been to yet.
    pub fn fail(&mut self, key: &BranchKey, sources: &[NodeUuid], now: MonoInstant) -> Failover {
        if self.outstanding.get(key).is_none_or(|o| o.answered) {
            // Nothing asked, or the reply is with the worker already.
            return Failover::Unknown;
        }
        let Some(old) = self.outstanding.remove(key) else {
            return Failover::Unknown;
        };
        let mut tried = old.tried;
        tried.push(old.source);
        match self.pick(&key.log, sources, &tried, now) {
            Some(source) => {
                self.send(key.clone(), source, old.want.clone(), tried, now);
                Failover::Resent(source, old.want)
            }
            None => Failover::Exhausted(key.clone()),
        }
    }

    /// Fail over every request unanswered for [`REQUEST_TIMEOUT_MS`], and
    /// forget expired penalties and stale last-request stamps. `sources_of`
    /// names the advertising sources of a branch.
    pub fn sweep(
        &mut self,
        now: MonoInstant,
        sources_of: &dyn Fn(&BranchKey) -> Vec<NodeUuid>,
    ) -> Swept {
        self.penalties.retain(|_, until| now < *until);
        self.last_request
            .retain(|_, at| now.0.saturating_sub(at.0) < LAST_REQUEST_KEEP_MS);
        let expired = |o: &Outstanding| now.0.saturating_sub(o.sent_at.0) >= REQUEST_TIMEOUT_MS;
        // An answered request whose reply never got handled (the worker's
        // queue dropped it) is forgotten, so the branch can be asked again.
        self.outstanding.retain(|_, o| !(o.answered && expired(o)));
        let due: Vec<BranchKey> = self
            .outstanding
            .iter()
            .filter(|(_, o)| expired(o))
            .map(|(key, _)| key.clone())
            .collect();
        let mut swept = Swept::default();
        for key in due {
            match self.fail(&key, &sources_of(&key), now) {
                Failover::Resent(source, want) => swept.sends.push((source, want)),
                Failover::Exhausted(key) => swept.exhausted.push(key),
                Failover::Unknown => {}
            }
        }
        swept
    }

    /// The usable source with the oldest last request (never asked first).
    fn pick(
        &self,
        log: &LogKey,
        sources: &[NodeUuid],
        exclude: &[NodeUuid],
        now: MonoInstant,
    ) -> Option<NodeUuid> {
        sources
            .iter()
            .copied()
            .filter(|s| !exclude.contains(s) && !self.is_penalized(*s, *log, now))
            .min_by_key(|s| (self.last_request.get(&(*s, *log)).copied(), *s))
    }

    fn send(
        &mut self,
        key: BranchKey,
        source: NodeUuid,
        want: BranchWant,
        tried: Vec<NodeUuid>,
        now: MonoInstant,
    ) {
        self.last_request.insert((source, key.log), now);
        self.outstanding.insert(
            key,
            Outstanding {
                source,
                want,
                sent_at: now,
                tried,
                answered: false,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::frame::TipRef;

    pub(crate) fn node(n: u8) -> NodeUuid {
        NodeUuid::from_bytes([n; 16])
    }

    fn want(tip: &str, from: u64) -> BranchWant {
        BranchWant {
            node: node(9),
            inc: 1,
            tip: TipRef {
                counter: 10,
                hash: tip.repeat(64),
            },
            from,
        }
    }

    fn at(secs: u64) -> MonoInstant {
        MonoInstant(secs * 1000)
    }

    #[test]
    fn sweep_fails_over_after_30s() {
        let (a, b) = (node(1), node(2));
        let mut table = RequestTable::new();
        let w = want("a", 4);
        assert_eq!(table.request(w.clone(), &[a, b], at(0)), Some(a));
        let sources = |_: &BranchKey| vec![a, b];
        // Just short of the timeout: untouched.
        assert_eq!(table.sweep(MonoInstant(29_999), &sources), Swept::default());
        // At 30 s the request moves to the other source.
        let swept = table.sweep(at(30), &sources);
        assert_eq!(swept.sends, vec![(b, w.clone())]);
        assert!(swept.exhausted.is_empty());
        let key = BranchKey::of(&w);
        assert_eq!(table.outstanding(&key).unwrap().source, b);
        assert_eq!(table.outstanding(&key).unwrap().sent_at, at(30));
        // 30 s later b has timed out too and a was already tried: nothing left.
        let swept = table.sweep(at(60), &sources);
        assert!(swept.sends.is_empty());
        assert_eq!(swept.exhausted, vec![key.clone()]);
        assert_eq!(table.outstanding(&key), None);
    }

    #[test]
    fn penalty_lasts_10_min() {
        let a = node(1);
        let mut table = RequestTable::new();
        let log = LogKey {
            node: node(9),
            incarnation: 1,
        };
        table.penalize(a, log, at(100));
        assert!(table.is_penalized(a, log, MonoInstant(100_000 + 599_999)));
        assert_eq!(
            table.request(want("a", 1), &[a], MonoInstant(100_000 + 599_999)),
            None
        );
        assert!(!table.is_penalized(a, log, MonoInstant(100_000 + 600_000)));
        assert_eq!(
            table.request(want("a", 1), &[a], MonoInstant(100_000 + 600_000)),
            Some(a)
        );
        // A penalty is per log: another log's request is not affected.
        table.penalize(a, log, at(1000));
        let other = BranchWant {
            node: node(8),
            ..want("b", 1)
        };
        assert_eq!(table.request(other, &[a], at(1001)), Some(a));
    }

    #[test]
    fn one_outstanding_request_per_branch() {
        let (a, b) = (node(1), node(2));
        let mut table = RequestTable::new();
        assert_eq!(table.request(want("a", 1), &[a, b], at(0)), Some(a));
        // The same branch, even for another `from` or another source set.
        assert_eq!(table.request(want("a", 1), &[a, b], at(1)), None);
        assert_eq!(table.request(want("a", 5), &[b], at(1)), None);
        assert_eq!(table.outstanding_count(), 1);
        // A different branch of the same log is independent.
        assert!(table.request(want("b", 1), &[a, b], at(1)).is_some());
        assert_eq!(table.outstanding_count(), 2);
        // An answered request still holds the branch until the worker has
        // handled the reply; then it may be asked again.
        let key = BranchKey::of(&want("a", 1));
        assert!(table.answer(a, &key).is_some());
        assert_eq!(table.request(want("a", 5), &[a, b], at(2)), None);
        assert!(table.clear(&key));
        assert!(table.request(want("a", 5), &[a, b], at(2)).is_some());
    }

    #[test]
    fn answered_request_is_not_failed_over_and_times_out_quietly() {
        let (a, b) = (node(1), node(2));
        let mut table = RequestTable::new();
        let w = want("a", 4);
        let key = BranchKey::of(&w);
        assert_eq!(table.request(w.clone(), &[a, b], at(0)), Some(a));
        assert!(table.answer(a, &key).is_some());
        let sources = |_: &BranchKey| vec![a, b];
        assert_eq!(table.fail(&key, &[a, b], at(1)), Failover::Unknown);
        assert_eq!(table.outstanding_count(), 1, "still held");
        // The reply was lost with the worker's queue: after the timeout the
        // entry goes without a resend, and the branch can be asked again.
        assert_eq!(table.sweep(at(30), &sources), Swept::default());
        assert_eq!(table.outstanding_count(), 0);
        assert!(table.request(w, &[a, b], at(31)).is_some());
    }

    #[test]
    fn oldest_last_request_source_preferred() {
        let (a, b, c) = (node(1), node(2), node(3));
        let mut table = RequestTable::new();
        // a at t=0, b at t=10 (via a penalty-free pick of each in turn).
        assert_eq!(table.request(want("a", 1), &[a], at(0)), Some(a));
        assert_eq!(table.request(want("b", 1), &[b], at(10)), Some(b));
        // c was never asked, so it is first; then a (older than b).
        assert_eq!(table.request(want("c", 1), &[a, b, c], at(20)), Some(c));
        assert_eq!(table.request(want("d", 1), &[b, a], at(30)), Some(a));
        // a was just asked at 30: now b is the oldest.
        assert_eq!(table.request(want("e", 1), &[a, b], at(40)), Some(b));
    }

    #[test]
    fn answer_needs_the_asked_source() {
        let (a, b) = (node(1), node(2));
        let mut table = RequestTable::new();
        let w = want("a", 1);
        table.request(w.clone(), &[a], at(0));
        let key = BranchKey::of(&w);
        assert_eq!(table.answer(b, &key), None);
        assert_eq!(table.answer(a, &BranchKey::of(&want("b", 1))), None);
        assert_eq!(table.answer(a, &key).unwrap().want, w);
        assert_eq!(table.answer(a, &key), None);
    }
}
