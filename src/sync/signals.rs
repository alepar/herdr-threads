//! Reachability gossip: the in-memory contact book.
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

use super::frame::{MAX_SEEN_AGES, SeenAge};
use crate::node::NodeUuid;
use crate::store::sync_contacts::{ContactVia, NodeContact, UNREACHABLE_AFTER_MS};

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
}
