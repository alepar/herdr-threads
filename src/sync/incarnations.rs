//! ForkProof: the one statement of what counts as a fork.
//!
//! Pure functions over signed [`Envelope`]s: no store, clock or network.
//!
//! Owns: the rule that a proof is two validly signed entries proven distinct
//! at one counter (same counter, or a prev-link mismatch); the choice of
//! held versus conflicting entry; the proof counter `x`, and so the freeze
//! bound `x - 1`; and the corrupt-versus-fork split (a bad signature is
//! never a fork). Hides: the order of the verify, counter-distance and
//! prev-link checks.
//!
//! Must not: touch the store, the clock or the network, act on a verdict, or
//! read payloads.
//!
//! Role: a sibling of the chain rules, which dedupe and verify the self log
//! before raising `AheadOfSelf`; these still verify first because they are
//! also called on Have-time and forwarded fork references that never passed
//! through ingest. Depends only on [`Envelope`] and
//! [`LogKey`]. Callers: the incarnations sink (synchronously, for
//! `AheadOfSelf`), the worker (ingest conflicts, Have-time and forwarded
//! fork references) and the re-check inside owner/restore rotation. A new
//! proof shape is a new [`Classified`] / [`AheadClass`] case here; callers
//! never act on an unsigned `Have` mismatch or compare entry hashes ad hoc.

use super::LogKey;
use super::envelope::{Envelope, EnvelopeError};

/// Two validly signed entries proven distinct at one counter.
///
/// `held` is the entry this node stores in `journal`; `conflicting` is the
/// other one. When a caller stores neither it passes the lower
/// `(counter, hash)` entry first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    pub log: LogKey,
    /// The counter at which two different entries are signed.
    pub x: u64,
    pub held: Envelope,
    pub conflicting: Envelope,
}

impl Proof {
    /// Highest counter still common to both branches: the log freezes here.
    pub fn freeze_bound(&self) -> u64 {
        self.x - 1
    }
}

/// Which argument of [`classify_pair`] failed verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    First,
    Second,
}

/// The verdict of [`classify_pair`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Classified {
    Proof(Box<Proof>),
    /// The same entry twice.
    Same,
    NotProof,
    Corrupt {
        which: Which,
        err: EnvelopeError,
    },
}

/// The verdict of [`classify_ahead_of_self`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AheadClass {
    /// The entry extends my head: my own log continued elsewhere (a restore).
    Restore,
    Fork(Box<Proof>),
    NotFork,
    Corrupt(EnvelopeError),
}

/// Classify two entries claimed to belong to one log signed by `pubkey`.
///
/// `a` is taken as the held entry and `b` as the conflicting one (callers
/// that store neither pass the lower `(counter, hash)` entry first).
pub fn classify_pair(pubkey: &[u8; 32], a: &Envelope, b: &Envelope) -> Classified {
    if a.log != b.log {
        return Classified::NotProof;
    }
    if let Err(err) = a.verify(pubkey) {
        return Classified::Corrupt {
            which: Which::First,
            err,
        };
    }
    if let Err(err) = b.verify(pubkey) {
        return Classified::Corrupt {
            which: Which::Second,
            err,
        };
    }
    let x = if a.counter == b.counter {
        if a.hash == b.hash {
            return Classified::Same;
        }
        a.counter
    } else {
        let (lo, hi) = if a.counter < b.counter {
            (a, b)
        } else {
            (b, a)
        };
        if lo.counter.checked_add(1) != Some(hi.counter) || hi.prev_hash == lo.hash {
            return Classified::NotProof;
        }
        lo.counter
    };
    Classified::Proof(Box::new(Proof {
        log: a.log,
        x,
        held: a.clone(),
        conflicting: b.clone(),
    }))
}

/// Classify an entry of my own log that a peer holds beyond (or at) my head.
///
/// `my_head` is `(counter, hash)` of my head; `my_entry_at` returns my stored
/// entry at a counter. A proof's `held` is an `Envelope`, so this takes the
/// entry rather than only its hash. A missing stored entry where one is
/// needed to build a proof yields `NotFork`: nothing is proven.
pub fn classify_ahead_of_self(
    my_head: (u64, [u8; 32]),
    my_entry_at: impl Fn(u64) -> Option<Envelope>,
    pubkey: &[u8; 32],
    entry: &Envelope,
) -> AheadClass {
    if let Err(err) = entry.verify(pubkey) {
        return AheadClass::Corrupt(err);
    }
    let (head, head_hash) = my_head;
    let proof = |x: u64| match my_entry_at(x) {
        Some(mine) if mine.log == entry.log => AheadClass::Fork(Box::new(Proof {
            log: entry.log,
            x,
            held: mine,
            conflicting: entry.clone(),
        })),
        _ => AheadClass::NotFork,
    };
    if entry.counter == 0 {
        return AheadClass::NotFork;
    }
    if entry.counter <= head {
        match my_entry_at(entry.counter) {
            Some(mine) if mine.hash == entry.hash => AheadClass::NotFork,
            _ => proof(entry.counter),
        }
    } else if entry.counter == head + 1 {
        if entry.prev_hash == head_hash {
            AheadClass::Restore
        } else {
            proof(head)
        }
    } else {
        AheadClass::NotFork
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeIdentity;
    use crate::sync::envelope::{Deps, UnsignedEnvelope};
    use crate::sync::hlc::HlcStamp;

    fn id(n: u8) -> NodeIdentity {
        NodeIdentity::from_seed([n; 32])
    }

    fn entry_in(id: &NodeIdentity, inc: u32, counter: u64, prev: [u8; 32], tag: &str) -> Envelope {
        UnsignedEnvelope {
            log: LogKey {
                node: id.uuid(),
                incarnation: inc,
            },
            counter,
            prev_hash: prev,
            hlc: HlcStamp::from_parts(1_700_000_000_000 + counter, 0),
            kind: "test.kind".into(),
            kind_version: 1,
            deps: Deps::empty(),
            creates: None,
            needs: vec![],
            payload: format!(r#"{{"t":"{tag}"}}"#).into_bytes(),
        }
        .seal(id)
    }

    fn entry(id: &NodeIdentity, counter: u64, prev: [u8; 32], tag: &str) -> Envelope {
        entry_in(id, 1, counter, prev, tag)
    }

    #[test]
    fn pair_same_counter_proof() {
        let i = id(1);
        let a = entry(&i, 3, [7; 32], "a");
        let b = entry(&i, 3, [7; 32], "b");
        let Classified::Proof(p) = classify_pair(&i.public_key(), &a, &b) else {
            panic!("expected proof");
        };
        assert_eq!((p.x, p.freeze_bound()), (3, 2));
        assert_eq!((p.held, p.conflicting), (a, b));
    }

    #[test]
    fn pair_prev_link_proof_both_orders() {
        let i = id(1);
        let lo = entry(&i, 4, [1; 32], "lo");
        let hi = entry(&i, 5, [9; 32], "hi");
        for (a, b) in [(&lo, &hi), (&hi, &lo)] {
            let Classified::Proof(p) = classify_pair(&i.public_key(), a, b) else {
                panic!("expected proof");
            };
            assert_eq!(p.x, 4);
            assert_eq!(&p.held, a);
            assert_eq!(&p.conflicting, b);
        }
    }

    #[test]
    fn pair_same() {
        let i = id(1);
        let a = entry(&i, 3, [7; 32], "a");
        assert_eq!(
            classify_pair(&i.public_key(), &a, &a.clone()),
            Classified::Same
        );
    }

    #[test]
    fn pair_not_proof_distance_2() {
        let i = id(1);
        let a = entry(&i, 3, [7; 32], "a");
        let b = entry(&i, 5, [8; 32], "b");
        assert_eq!(classify_pair(&i.public_key(), &a, &b), Classified::NotProof);
    }

    #[test]
    fn pair_not_proof_different_logs() {
        let i = id(1);
        let a = entry(&i, 3, [7; 32], "a");
        let other = entry_in(&i, 2, 3, [7; 32], "b");
        assert_eq!(
            classify_pair(&i.public_key(), &a, &other),
            Classified::NotProof
        );
    }

    #[test]
    fn pair_not_proof_consistent_prev_link() {
        let i = id(1);
        let lo = entry(&i, 4, [1; 32], "lo");
        let hi = entry(&i, 5, lo.hash, "hi");
        assert_eq!(
            classify_pair(&i.public_key(), &lo, &hi),
            Classified::NotProof
        );
        assert_eq!(
            classify_pair(&i.public_key(), &hi, &lo),
            Classified::NotProof
        );
    }

    #[test]
    fn pair_corrupt_flipped_byte() {
        let i = id(1);
        let a = entry(&i, 3, [7; 32], "a");
        let mut b = entry(&i, 3, [7; 32], "b");
        let last = b.payload.len() - 3;
        b.payload[last] ^= 1;
        assert_eq!(
            classify_pair(&i.public_key(), &a, &b),
            Classified::Corrupt {
                which: Which::Second,
                err: EnvelopeError::HashMismatch
            }
        );
        assert_eq!(
            classify_pair(&i.public_key(), &b, &a),
            Classified::Corrupt {
                which: Which::First,
                err: EnvelopeError::HashMismatch
            }
        );
    }

    #[test]
    fn pair_corrupt_foreign_key() {
        let i = id(1);
        let a = entry(&i, 3, [7; 32], "a");
        let b = entry(&i, 3, [7; 32], "b");
        assert_eq!(
            classify_pair(&id(2).public_key(), &a, &b),
            Classified::Corrupt {
                which: Which::First,
                err: EnvelopeError::BadSignature
            }
        );
    }

    fn mine(i: &NodeIdentity) -> (Envelope, Envelope) {
        let e1 = entry(i, 1, [0; 32], "m1");
        let e2 = entry(i, 2, e1.hash, "m2");
        (e1, e2)
    }

    #[test]
    fn ahead_restore() {
        let i = id(1);
        let (e1, e2) = mine(&i);
        let e3 = entry(&i, 3, e2.hash, "m3");
        let lookup = |c| [&e1, &e2].into_iter().find(|e| e.counter == c).cloned();
        assert_eq!(
            classify_ahead_of_self((2, e2.hash), lookup, &i.public_key(), &e3),
            AheadClass::Restore
        );
    }

    #[test]
    fn ahead_fork_shared_counter() {
        let i = id(1);
        let (e1, e2) = mine(&i);
        let theirs = entry(&i, 2, e1.hash, "other");
        let lookup = |c| [&e1, &e2].into_iter().find(|e| e.counter == c).cloned();
        let AheadClass::Fork(p) =
            classify_ahead_of_self((2, e2.hash), lookup, &i.public_key(), &theirs)
        else {
            panic!("expected fork");
        };
        assert_eq!(p.x, 2);
        assert_eq!(p.held, e2);
        assert_eq!(p.conflicting, theirs);
    }

    #[test]
    fn ahead_fork_prev_link() {
        let i = id(1);
        let (e1, e2) = mine(&i);
        let e3 = entry(&i, 3, [0x55; 32], "m3");
        let lookup = |c| [&e1, &e2].into_iter().find(|e| e.counter == c).cloned();
        let AheadClass::Fork(p) =
            classify_ahead_of_self((2, e2.hash), lookup, &i.public_key(), &e3)
        else {
            panic!("expected fork");
        };
        assert_eq!((p.x, p.freeze_bound()), (2, 1));
        assert_eq!(p.held, e2);
        assert_eq!(p.conflicting, e3);
    }

    #[test]
    fn ahead_not_fork_equal_hash() {
        let i = id(1);
        let (e1, e2) = mine(&i);
        let lookup = |c| [&e1, &e2].into_iter().find(|e| e.counter == c).cloned();
        assert_eq!(
            classify_ahead_of_self((2, e2.hash), lookup, &i.public_key(), &e1),
            AheadClass::NotFork
        );
    }

    #[test]
    fn ahead_not_fork_gap() {
        let i = id(1);
        let (e1, e2) = mine(&i);
        let e4 = entry(&i, 4, [3; 32], "m4");
        let lookup = |c| [&e1, &e2].into_iter().find(|e| e.counter == c).cloned();
        assert_eq!(
            classify_ahead_of_self((2, e2.hash), lookup, &i.public_key(), &e4),
            AheadClass::NotFork
        );
    }

    #[test]
    fn ahead_corrupt() {
        let i = id(1);
        let (e1, e2) = mine(&i);
        let mut e3 = entry(&i, 3, e2.hash, "m3");
        e3.signature[0] ^= 1;
        let lookup = |c| [&e1, &e2].into_iter().find(|e| e.counter == c).cloned();
        assert_eq!(
            classify_ahead_of_self((2, e2.hash), lookup, &i.public_key(), &e3),
            AheadClass::Corrupt(EnvelopeError::BadSignature)
        );
    }
}
