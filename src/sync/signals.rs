//! Reachability gossip (the in-memory contact book) and the round-complete
//! signal ([`SyncSignals`]).
//!
//! Responsibility (ContactBook)
//! - Owns: `node -> (last_contact_wall_ms, Direct | Relayed)`. It records
//!   direct contact from inbound frames, builds `seen` ages (the local node at
//!   age 0, the receiving link's node excluded), merges received ages only
//!   when they are newer, and decides which rows are due for a flush (a
//!   contact that advanced at least 30 s since its last write, or whose
//!   reachable/unreachable answer differs from the stored row's; at most every
//!   10 s).
//! - Hides: that gossip carries ages and never absolute times.
//! - Must not: compare wall clocks across nodes, do I/O, or be shared outside
//!   the coordinator actor (only the `sync_node_contact` table is read
//!   elsewhere, through `store::sync_contacts::reach`).
//! - Role: pure state owned by the actor, rebuilt empty on each start; it
//!   flushes through `SyncStorePort::record_contacts`. New reachability
//!   rules go in `store::sync_contacts::reach`, not here.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use tokio::sync::Notify;

use super::frame::{MAX_SEEN_AGES, SeenAge};
use crate::node::NodeUuid;
use crate::protocol::time::MonoInstant;
use crate::store::sync_contacts::{ContactVia, NodeContact, UNREACHABLE_AFTER_MS};
use crate::store::sync_dial_targets::TargetId;

/// A row is rewritten once its contact advanced this far past the stored one.
pub const FLUSH_ADVANCE_MS: u64 = 30_000;
/// Flushes are at most this far apart.
pub const FLUSH_MIN_INTERVAL_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug)]
struct Contact {
    last_contact: u64,
    via: ContactVia,
    /// `last_contact` as last written to the store.
    persisted: Option<u64>,
}

/// See the module documentation.
#[derive(Debug)]
pub struct ContactBook {
    me: NodeUuid,
    contacts: BTreeMap<NodeUuid, Contact>,
    last_flush_at: Option<u64>,
}

impl ContactBook {
    pub fn new(me: NodeUuid) -> Self {
        Self {
            me,
            contacts: BTreeMap::new(),
            last_flush_at: None,
        }
    }

    /// A frame arrived from `node` at local wall time `now` (unix ms). The
    /// local node is not tracked.
    pub fn record_direct(&mut self, node: NodeUuid, now: u64) {
        if node == self.me {
            return;
        }
        let contact = self.contacts.entry(node).or_insert(Contact {
            last_contact: 0,
            via: ContactVia::Direct,
            persisted: None,
        });
        contact.last_contact = contact.last_contact.max(now);
        contact.via = ContactVia::Direct;
    }

    /// The contact last held for `node`, if any.
    pub fn last_contact(&self, node: &NodeUuid) -> Option<(u64, ContactVia)> {
        self.contacts
            .get(node)
            .map(|contact| (contact.last_contact, contact.via))
    }

    /// The ages to send on the link to `for_link_node`: the local node at 0,
    /// then every other known node as `now - last_contact`, newest first,
    /// within the frame's cap. The link's own node is left out: it knows when
    /// it last spoke.
    pub fn seen_ages(&self, for_link_node: &NodeUuid, now: u64) -> Vec<SeenAge> {
        let mut ages: Vec<SeenAge> = self
            .contacts
            .iter()
            .filter(|(node, _)| *node != for_link_node)
            .map(|(node, contact)| SeenAge {
                node: *node,
                age_ms: now.saturating_sub(contact.last_contact),
            })
            .collect();
        ages.sort_by_key(|age| age.age_ms);
        ages.truncate(MAX_SEEN_AGES - 1);
        let mut out = Vec::with_capacity(ages.len() + 1);
        out.push(SeenAge {
            node: self.me,
            age_ms: 0,
        });
        out.extend(ages);
        out
    }

    /// Merges ages received at local wall time `now`: each becomes the
    /// candidate contact `now - age_ms`, taken as `Relayed` only when newer
    /// than the held one. Entries about the local node are ignored. Returns
    /// how many contacts changed.
    pub fn merge_seen(&mut self, ages: &[SeenAge], now: u64) -> usize {
        let mut changed = 0;
        for age in ages {
            if age.node == self.me {
                continue;
            }
            let candidate = now.saturating_sub(age.age_ms);
            match self.contacts.get_mut(&age.node) {
                Some(contact) if contact.last_contact >= candidate => {}
                Some(contact) => {
                    contact.last_contact = candidate;
                    contact.via = ContactVia::Relayed;
                    changed += 1;
                }
                None => {
                    self.contacts.insert(
                        age.node,
                        Contact {
                            last_contact: candidate,
                            via: ContactVia::Relayed,
                            persisted: None,
                        },
                    );
                    changed += 1;
                }
            }
        }
        changed
    }

    /// The rows to write now, or none. A flush is attempted at most every
    /// [`FLUSH_MIN_INTERVAL_MS`] (a non-empty answer starts the interval). A
    /// contact is due when it was never written, advanced at least
    /// [`FLUSH_ADVANCE_MS`] past its written value, or crossed the
    /// reachable/unreachable threshold relative to what the table would say.
    /// After the store accepted them, pass them to [`Self::flushed`].
    pub fn due_for_flush(&mut self, now: u64) -> Vec<NodeContact> {
        if self
            .last_flush_at
            .is_some_and(|at| now.saturating_sub(at) < FLUSH_MIN_INTERVAL_MS)
        {
            return Vec::new();
        }
        let due: Vec<NodeContact> = self
            .contacts
            .iter()
            .filter(|(_, contact)| match contact.persisted {
                None => true,
                Some(stored) => {
                    let reachable = |last: u64| now.saturating_sub(last) <= UNREACHABLE_AFTER_MS;
                    contact.last_contact.saturating_sub(stored) >= FLUSH_ADVANCE_MS
                        || reachable(stored) != reachable(contact.last_contact)
                }
            })
            .map(|(node, contact)| NodeContact {
                node: *node,
                last_contact_at: contact.last_contact,
                via: contact.via,
            })
            .collect();
        if !due.is_empty() {
            self.last_flush_at = Some(now);
        }
        due
    }

    /// The store accepted `rows`: they are the written values from now on. A
    /// failed flush simply is not reported, and its rows are due again at the
    /// next flush.
    pub fn flushed(&mut self, rows: &[NodeContact]) {
        for row in rows {
            if let Some(contact) = self.contacts.get_mut(&row.node) {
                contact.persisted = Some(row.last_contact_at);
            }
        }
    }

    /// Every row whose last contact is not what the store holds, regardless
    /// of the advance and spacing rules: the final best-effort flush at
    /// shutdown. Pass the accepted rows to [`Self::flushed`].
    pub fn unflushed(&self) -> Vec<NodeContact> {
        self.contacts
            .iter()
            .filter(|(_, contact)| contact.persisted != Some(contact.last_contact))
            .map(|(node, contact)| NodeContact {
                node: *node,
                last_contact_at: contact.last_contact,
                via: contact.via,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SyncSignals
// ---------------------------------------------------------------------------

/// Whether the first sync round is done, as [`SyncSignals::round_state`]
/// answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundState {
    /// No link is active and no dial target is enabled.
    NoLinks,
    /// Every active link completed a round that started at or after `since`,
    /// and every enabled dial target's node is among those links.
    Complete,
    /// This many active links or enabled dial targets are not caught up.
    Pending { links: u32 },
}

/// One active link's round, as the actor publishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkRound {
    /// The peer node of the link, whatever its origin.
    pub node: NodeUuid,
    /// When the round started (the coordinator's monotonic clock).
    pub started: MonoInstant,
    pub complete: bool,
}

#[derive(Default)]
struct Inner {
    links: BTreeMap<u64, LinkRound>,
    /// Enabled dial targets with their learned node (None until it links).
    targets: BTreeMap<TargetId, Option<NodeUuid>>,
    requested: Option<MonoInstant>,
}

/// The in-memory "first sync round complete" signal.
///
/// Responsibility (SyncSignals)
/// - Owns: per active link the round start, completion and node, the enabled
///   dial targets with their learned node, `request_round(at)` and
///   `round_state(since) -> RoundState::{NoLinks, Complete, Pending{links}}`.
///   The completion rule: every active link completed a round since `since`,
///   and every enabled dial target's node is among those links (root spec §7).
/// - Hides: the completion rule (a full `Have` after the start, and every
///   advertised-ahead log caught up or unwantable), which the actor applies
///   before it publishes.
/// - Must not: persist anything or block; it is a lock-protected snapshot the
///   actor writes and the deadline lane reads.
/// - Relationships: rounds are written by the SyncCoordinator actor, targets
///   by the actor's Dialer;
///   `SyncHandle::signals()` hands the `Arc` to the deadline-lane composition
///   in `app.rs` (ht-5fr.13). `request_round` records the request and wakes the
///   actor, which sends `Have{full, ask: true}` on every active link.
/// - Lifecycle: created by `SyncCoordinator::start`, shared as `Arc` for the
///   daemon's life.
/// - Role: new sync readiness signals go here; readers never inspect
///   coordinator link state to decide whether sync caught up.
#[derive(Default)]
pub struct SyncSignals {
    inner: Mutex<Inner>,
    wake: Notify,
}

impl SyncSignals {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ask for a fresh round at `at` on every active link: the request is
    /// recorded (the latest wins) and the actor woken. Never waits. With no
    /// active link the request waits for the first activation, whose round
    /// starts later than `at`.
    pub fn request_round(&self, at: MonoInstant) {
        {
            let mut inner = self.lock();
            inner.requested = Some(inner.requested.map_or(at, |held| held.max(at)));
        }
        self.wake.notify_one();
    }

    /// `Complete` when every active link completed a round that started at or
    /// after `since` and every enabled dial target's learned node has an
    /// active link. A linked target with an incomplete round counts once.
    pub fn round_state(&self, since: MonoInstant) -> RoundState {
        let inner = self.lock();
        if inner.links.is_empty() && inner.targets.is_empty() {
            return RoundState::NoLinks;
        }
        let pending_links = inner
            .links
            .values()
            .filter(|round| !(round.complete && round.started >= since))
            .count();
        let unlinked = inner
            .targets
            .values()
            .filter(|node| node.is_none_or(|n| !inner.links.values().any(|round| round.node == n)))
            .count();
        match u32::try_from(pending_links.saturating_add(unlinked)) {
            Ok(0) => RoundState::Complete,
            Ok(links) => RoundState::Pending { links },
            Err(_) => RoundState::Pending { links: u32::MAX },
        }
    }

    // The actor's side.

    /// Resolves when a round was requested since the last wake.
    pub(super) async fn requested(&self) {
        self.wake.notified().await;
    }

    /// Takes the recorded request, if any.
    pub(super) fn take_request(&self) -> Option<MonoInstant> {
        self.lock().requested.take()
    }

    /// Replaces the published rounds with the actor's current ones.
    pub(super) fn publish(&self, rounds: BTreeMap<u64, LinkRound>) {
        self.lock().links = rounds;
    }

    /// Replaces the set of enabled dial targets. A held id keeps its held
    /// node when that is `Some` (the in-memory `Established` can be newer than
    /// the row the reload read); ids not given are dropped.
    pub(super) fn publish_targets(
        &self,
        targets: impl IntoIterator<Item = (TargetId, Option<NodeUuid>)>,
    ) {
        let mut inner = self.lock();
        let held = std::mem::take(&mut inner.targets);
        inner.targets = targets
            .into_iter()
            .map(|(id, node)| {
                let kept = held.get(&id).copied().flatten();
                (id, kept.or(node))
            })
            .collect();
    }

    /// Records the node a held target's dial established with. An unknown id
    /// is ignored (a cancelled task can still report).
    pub(super) fn target_linked(&self, id: TargetId, node: NodeUuid) {
        if let Some(slot) = self.lock().targets.get_mut(&id) {
            *slot = Some(node);
        }
    }

    /// Test-only: sets both published views as given.
    #[cfg(test)]
    pub(crate) fn publish_for_test(
        &self,
        rounds: BTreeMap<u64, LinkRound>,
        targets: BTreeMap<TargetId, Option<NodeUuid>>,
    ) {
        let mut inner = self.lock();
        inner.links = rounds;
        inner.targets = targets;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(n: u128) -> NodeUuid {
        NodeUuid::from_u128(n)
    }

    const ME: u128 = 1;

    fn book() -> ContactBook {
        ContactBook::new(node(ME))
    }

    fn ages(book: &ContactBook, link: u128, now: u64) -> Vec<(NodeUuid, u64)> {
        book.seen_ages(&node(link), now)
            .into_iter()
            .map(|age| (age.node, age.age_ms))
            .collect()
    }

    #[test]
    fn seen_ages_relative_self_zero_receiver_excluded() {
        let mut book = book();
        book.record_direct(node(2), 1_000);
        book.record_direct(node(3), 4_000);
        book.record_direct(node(ME), 4_500);
        assert_eq!(
            ages(&book, 2, 10_000),
            vec![(node(ME), 0), (node(3), 6_000)],
            "self at 0 first, link node 2 excluded, ages relative to now, self not tracked"
        );
        assert_eq!(
            ages(&book, 9, 10_000),
            vec![(node(ME), 0), (node(3), 6_000), (node(2), 9_000)],
            "newest first"
        );
        assert_eq!(
            ages(&book, 9, 500),
            vec![(node(ME), 0), (node(2), 0), (node(3), 0)],
            "a contact in the future ages 0, never wraps"
        );
    }

    #[test]
    fn seen_ages_fit_the_frame_cap() {
        let mut book = book();
        for n in 0..(MAX_SEEN_AGES as u128 + 10) {
            book.record_direct(node(100 + n), 1_000 + n as u64);
        }
        let ages = book.seen_ages(&node(2), 1_000_000);
        assert_eq!(ages.len(), MAX_SEEN_AGES);
        assert_eq!(ages[0].node, node(ME));
    }

    #[test]
    fn merge_only_newer_marks_relayed() {
        let mut book = book();
        book.record_direct(node(2), 8_000);
        let seen = |n, age_ms| SeenAge {
            node: node(n),
            age_ms,
        };
        // 2: candidate 10_000 - 5_000 = 5_000 is older than 8_000; 3 is new;
        // an entry about self is ignored.
        let changed = book.merge_seen(&[seen(2, 5_000), seen(3, 2_000), seen(ME, 0)], 10_000);
        assert_eq!(changed, 1);
        assert_eq!(
            book.last_contact(&node(2)),
            Some((8_000, ContactVia::Direct))
        );
        assert_eq!(
            book.last_contact(&node(3)),
            Some((8_000, ContactVia::Relayed))
        );
        assert_eq!(book.last_contact(&node(ME)), None);
        // A newer relayed candidate replaces and re-marks.
        assert_eq!(book.merge_seen(&[seen(2, 500)], 10_000), 1);
        assert_eq!(
            book.last_contact(&node(2)),
            Some((9_500, ContactVia::Relayed))
        );
        // An equal candidate is not newer.
        assert_eq!(book.merge_seen(&[seen(2, 500)], 10_000), 0);
        assert_eq!(
            book.last_contact(&node(2)),
            Some((9_500, ContactVia::Relayed))
        );
        // An echo of our own report (no transit time) cannot advance a
        // contact: the age we sent for node 3 at t=20_000 is 12_000.
        let echoed = book.seen_ages(&node(9), 20_000);
        let age = echoed.iter().find(|a| a.node == node(3)).unwrap().clone();
        assert_eq!(book.merge_seen(&[age], 20_000), 0);
        assert_eq!(
            book.last_contact(&node(3)),
            Some((8_000, ContactVia::Relayed))
        );
    }

    #[test]
    fn direct_contact_overrides_a_relayed_one_and_never_moves_back() {
        let mut book = book();
        book.merge_seen(
            &[SeenAge {
                node: node(2),
                age_ms: 1_000,
            }],
            10_000,
        );
        assert_eq!(
            book.last_contact(&node(2)),
            Some((9_000, ContactVia::Relayed))
        );
        book.record_direct(node(2), 9_500);
        assert_eq!(
            book.last_contact(&node(2)),
            Some((9_500, ContactVia::Direct))
        );
        book.record_direct(node(2), 9_000);
        assert_eq!(
            book.last_contact(&node(2)),
            Some((9_500, ContactVia::Direct)),
            "a wall-clock step back keeps the later contact"
        );
    }

    #[test]
    fn flush_due_on_30s_advance_or_crossing_at_most_every_10s() {
        let mut book = book();
        book.record_direct(node(2), 1_000);
        // Never written: due at once.
        let first = book.due_for_flush(1_000);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].node, node(2));
        assert_eq!(first[0].last_contact_at, 1_000);
        assert_eq!(first[0].via, ContactVia::Direct);
        // Until the store accepts it, it stays due, but not within 10 s.
        assert!(
            book.due_for_flush(5_000).is_empty(),
            "within 10 s of the attempt"
        );
        assert_eq!(book.due_for_flush(11_000).len(), 1, "failed flush retried");
        book.flushed(&first);

        // A 29.9 s advance is not enough; 30 s is.
        book.record_direct(node(2), 30_999);
        assert!(book.due_for_flush(40_000).is_empty());
        book.record_direct(node(2), 31_000);
        let advance = book.due_for_flush(41_000);
        assert_eq!(advance.len(), 1);
        assert_eq!(advance[0].last_contact_at, 31_000);
        book.flushed(&advance);

        // The 10 s spacing holds even when a row is due.
        book.record_direct(node(2), 61_000);
        assert!(
            book.due_for_flush(45_000).is_empty(),
            "10 s since 41_000 not over"
        );
        assert_eq!(book.due_for_flush(51_000).len(), 1);
    }

    #[test]
    fn flush_due_on_a_threshold_crossing_without_a_30s_advance() {
        let mut book = book();
        book.record_direct(node(2), 0);
        let written = book.due_for_flush(0);
        book.flushed(&written);
        // Contact advanced 20 s, below the 30 s advance rule.
        book.record_direct(node(2), 20_000);
        assert!(
            book.due_for_flush(100_000).is_empty(),
            "both readings reachable"
        );
        // At 190 s the stored 0 reads unreachable while 20 s reads reachable:
        // the table would be wrong, so the row is due.
        let crossing = book.due_for_flush(190_000);
        assert_eq!(crossing.len(), 1);
        assert_eq!(crossing[0].last_contact_at, 20_000);
        book.flushed(&crossing);
        // Both now unreachable at 210 s: nothing to correct.
        assert!(book.due_for_flush(210_000).is_empty());
    }

    #[test]
    fn nothing_is_due_without_contacts() {
        assert!(book().due_for_flush(1_000_000).is_empty());
    }

    #[test]
    fn unflushed_lists_every_changed_row_ignoring_spacing() {
        let mut book = book();
        book.record_direct(node(2), 1_000);
        let rows = book.due_for_flush(1_000);
        book.flushed(&rows);
        assert!(book.unflushed().is_empty(), "everything written");
        book.record_direct(node(2), 2_000);
        book.record_direct(node(3), 2_500);
        let rows = book.unflushed();
        assert_eq!(rows.len(), 2, "a 1 s advance and a new node both count");
        assert!(
            book.due_for_flush(3_000).is_empty(),
            "the ordinary rules still hold"
        );
    }

    fn round(node: NodeUuid, started: u64, complete: bool) -> LinkRound {
        LinkRound {
            node,
            started: MonoInstant(started),
            complete,
        }
    }

    fn publish(signals: &SyncSignals, rounds: &[(u64, LinkRound)]) {
        signals.publish(rounds.iter().copied().collect());
    }

    #[test]
    fn round_state_no_links_pending_complete() {
        let signals = SyncSignals::new();
        assert_eq!(signals.round_state(MonoInstant(0)), RoundState::NoLinks);
        publish(
            &signals,
            &[
                (1, round(node(1), 10, false)),
                (2, round(node(2), 10, true)),
            ],
        );
        assert_eq!(
            signals.round_state(MonoInstant(0)),
            RoundState::Pending { links: 1 }
        );
        publish(
            &signals,
            &[(1, round(node(1), 10, true)), (2, round(node(2), 10, true))],
        );
        assert_eq!(signals.round_state(MonoInstant(10)), RoundState::Complete);
        publish(&signals, &[]);
        assert_eq!(signals.round_state(MonoInstant(0)), RoundState::NoLinks);
    }

    #[test]
    fn round_started_before_since_is_pending_even_when_complete() {
        let signals = SyncSignals::new();
        publish(
            &signals,
            &[(1, round(node(1), 10, true)), (2, round(node(2), 30, true))],
        );
        assert_eq!(
            signals.round_state(MonoInstant(11)),
            RoundState::Pending { links: 1 },
            "link 1 completed a round older than `since`"
        );
        assert_eq!(signals.round_state(MonoInstant(10)), RoundState::Complete);
    }

    const T1: TargetId = TargetId(1);
    const T2: TargetId = TargetId(2);

    #[test]
    fn two_targets_one_unlinked_keeps_the_round_pending() {
        let (a, b) = (node(0xa), node(0xb));
        let signals = SyncSignals::new();
        signals.publish_targets([(T1, Some(a)), (T2, None)]);
        publish(&signals, &[(1, round(a, 10, true))]);
        assert_eq!(
            signals.round_state(MonoInstant(10)),
            RoundState::Pending { links: 1 },
            "the unlinked target holds the round"
        );
        signals.target_linked(T2, b);
        assert_eq!(
            signals.round_state(MonoInstant(10)),
            RoundState::Pending { links: 1 },
            "a learned node without a published link is still unlinked"
        );
        publish(
            &signals,
            &[(1, round(a, 10, true)), (2, round(b, 20, false))],
        );
        assert_eq!(
            signals.round_state(MonoInstant(10)),
            RoundState::Pending { links: 1 },
            "linked but incomplete counts once, through the link"
        );
        publish(
            &signals,
            &[(1, round(a, 10, true)), (2, round(b, 20, true))],
        );
        assert_eq!(signals.round_state(MonoInstant(10)), RoundState::Complete);
        publish(
            &signals,
            &[(1, round(a, 10, true)), (2, round(b, 12, true))],
        );
        assert_eq!(
            signals.round_state(MonoInstant(15)),
            RoundState::Pending { links: 2 },
            "both rounds started before `since`"
        );
        publish(
            &signals,
            &[(1, round(a, 20, true)), (2, round(b, 12, true))],
        );
        assert_eq!(
            signals.round_state(MonoInstant(15)),
            RoundState::Pending { links: 1 }
        );
    }

    #[test]
    fn targets_alone_are_pending_and_no_targets_no_links_is_no_links() {
        let signals = SyncSignals::new();
        signals.publish_targets([(T1, None)]);
        assert_eq!(
            signals.round_state(MonoInstant(0)),
            RoundState::Pending { links: 1 }
        );
        signals.publish_targets([]);
        assert_eq!(signals.round_state(MonoInstant(0)), RoundState::NoLinks);
    }

    #[test]
    fn a_target_linked_through_an_inbound_link_counts() {
        let a = node(0xa);
        let signals = SyncSignals::new();
        signals.publish_targets([(T1, Some(a))]);
        publish(&signals, &[(7, round(a, 5, true))]);
        assert_eq!(signals.round_state(MonoInstant(5)), RoundState::Complete);
    }

    #[test]
    fn publish_targets_keeps_a_newer_learned_node() {
        let (a, b, c) = (node(0xa), node(0xb), node(0xc));
        let signals = SyncSignals::new();
        signals.publish_targets([(T1, None)]);
        signals.target_linked(T1, b);
        signals.publish_targets([(T1, Some(a))]);
        publish(&signals, &[(1, round(b, 1, true))]);
        assert_eq!(
            signals.round_state(MonoInstant(1)),
            RoundState::Complete,
            "the in-memory B outlives the row's older A"
        );
        signals.publish_targets([]);
        publish(&signals, &[]);
        assert_eq!(signals.round_state(MonoInstant(1)), RoundState::NoLinks);
        signals.target_linked(TargetId(9), c);
        assert_eq!(
            signals.round_state(MonoInstant(1)),
            RoundState::NoLinks,
            "an unknown id adds nothing"
        );
    }

    #[test]
    fn request_round_is_recorded_once_with_the_latest_time() {
        let signals = SyncSignals::new();
        assert_eq!(signals.take_request(), None);
        signals.request_round(MonoInstant(5));
        signals.request_round(MonoInstant(9));
        signals.request_round(MonoInstant(7));
        assert_eq!(signals.take_request(), Some(MonoInstant(9)));
        assert_eq!(signals.take_request(), None, "taken");
    }

    #[tokio::test(start_paused = true)]
    async fn request_round_wakes_a_waiting_actor_and_leaves_a_permit_otherwise() {
        let signals = std::sync::Arc::new(SyncSignals::new());
        // A request made before anyone waits is not lost.
        signals.request_round(MonoInstant(1));
        tokio::time::timeout(std::time::Duration::from_secs(1), signals.requested())
            .await
            .expect("the stored permit resolves the wait");
        // A waiter is woken by a later request.
        let waiter = {
            let signals = signals.clone();
            tokio::spawn(async move { signals.requested().await })
        };
        tokio::task::yield_now().await;
        signals.request_round(MonoInstant(2));
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("woken")
            .unwrap();
    }
}
