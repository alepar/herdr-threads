//! The per-log chain rules: what one incoming entry means for the log it
//! claims to extend (journal-core design, section 6).

use super::{LogState, envelope::Envelope, envelope::EnvelopeError};

/// What the caller already holds of one origin log, loaded inside its write
/// transaction. A plain value so [`check_next`] stays pure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldLog {
    /// The log's public key, from its genesis entry.
    pub pubkey: [u8; 32],
    /// Highest contiguous counter held.
    pub contiguous: u64,
    /// Hash of the entry at `contiguous`.
    pub head_hash: [u8; 32],
    pub state: LogState,
    /// Last counter a frozen log accepts; `None` while active.
    pub frozen_at: Option<u64>,
    /// Whether this is the local node's own active log.
    pub is_active_self: bool,
    /// The stored hash at the incoming entry's counter, which the caller
    /// fills only when that counter is at most `contiguous`.
    pub stored_hash: Option<[u8; 32]>,
}

/// The decision for one incoming entry.
///
/// Owns: the per-log append, duplicate, gap, fork, frozen and corrupt verdict.
/// Does not own: acting on it (the journal stores, stops or rolls back) or
/// the clock and store reads. New chain rule: a new variant here, so every
/// caller's exhaustive `match` fails to compile until it handles it. Role:
/// closed result of [`check_next`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainStep {
    /// A validly signed entry of this node's own active log that this node
    /// does not hold.
    AheadOfSelf,
    /// Entries before `expected` are missing.
    Gap { expected: u64 },
    /// First entry of an unknown log, self-certifying; the caller records the
    /// log with [`Envelope::genesis_pubkey`].
    Genesis,
    /// Already held with the same hash.
    Duplicate,
    /// A validly signed entry that contradicts the held chain.
    Conflict,
    /// Past the counter at which the log froze.
    BeyondFrozen,
    /// Extends the held chain by exactly one.
    Append,
    /// Not a fork: the entry is damaged or forged.
    Corrupt(EnvelopeError),
}

/// Decide what `entry` means for the log it names, given what is `held`.
///
/// Owns: the single statement of the chain rules, in this order: self log
/// (duplicate, then signature, then `AheadOfSelf`), unknown log, counter at
/// or below the held prefix, freeze, gap, then `prev_hash` and signature. Hides that order from callers. Does not:
/// touch the store or the clock, act on the step, or read the payload
/// (except through [`Envelope::genesis_pubkey`] for a genesis). Role: shared
/// by `SqliteStore` and `MemoryJournal` so they cannot disagree. A mismatched
/// hash or `prev_hash` whose signature does not verify is `Corrupt`, never a
/// fork, so a damaged transfer cannot freeze a healthy log; this holds for the
/// self log too.
pub fn check_next(held: Option<&HeldLog>, entry: &Envelope) -> ChainStep {
    let Some(held) = held else {
        if entry.counter != 1 {
            return ChainStep::Gap { expected: 1 };
        }
        return match entry
            .genesis_pubkey()
            .and_then(|pubkey| entry.verify(&pubkey))
        {
            Ok(()) => ChainStep::Genesis,
            Err(err) => ChainStep::Corrupt(err),
        };
    };
    if held.is_active_self {
        if entry.counter <= held.contiguous && held.stored_hash == Some(entry.hash) {
            return ChainStep::Duplicate;
        }
        return match entry.verify(&held.pubkey) {
            Ok(()) => ChainStep::AheadOfSelf,
            Err(err) => ChainStep::Corrupt(err),
        };
    }
    if entry.counter <= held.contiguous {
        if held.stored_hash == Some(entry.hash) {
            return ChainStep::Duplicate;
        }
        return signed_conflict(held, entry);
    }
    if let (LogState::Frozen, Some(frozen_at)) = (held.state, held.frozen_at)
        && entry.counter > frozen_at
    {
        return ChainStep::BeyondFrozen;
    }
    if entry.counter > held.contiguous.saturating_add(1) {
        return ChainStep::Gap {
            expected: held.contiguous.saturating_add(1),
        };
    }
    match entry.verify(&held.pubkey) {
        Err(err) => ChainStep::Corrupt(err),
        Ok(()) if entry.prev_hash != held.head_hash => ChainStep::Conflict,
        Ok(()) => ChainStep::Append,
    }
}

/// A different entry where one is held: a fork only if its author signed it.
fn signed_conflict(held: &HeldLog, entry: &Envelope) -> ChainStep {
    match entry.verify(&held.pubkey) {
        Ok(()) => ChainStep::Conflict,
        Err(err) => ChainStep::Corrupt(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeIdentity;
    use crate::sync::{
        LogKey,
        envelope::{Deps, GENESIS_KIND, UnsignedEnvelope},
        hlc::HlcStamp,
    };

    fn identity(n: u8) -> NodeIdentity {
        NodeIdentity::from_seed([n; 32])
    }

    fn log(id: &NodeIdentity) -> LogKey {
        LogKey {
            node: id.uuid(),
            incarnation: 1,
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn genesis_with(
        id: &NodeIdentity,
        announced: &NodeIdentity,
        signer: &NodeIdentity,
    ) -> Envelope {
        UnsignedEnvelope {
            log: log(id),
            counter: 1,
            prev_hash: [0; 32],
            hlc: HlcStamp::from_parts(1_700_000_000_000, 0),
            kind: GENESIS_KIND.into(),
            kind_version: 1,
            deps: Deps::empty(),
            creates: None,
            needs: vec![],
            payload: format!(r#"{{"pubkey":"{}"}}"#, hex(&announced.public_key())).into_bytes(),
        }
        .seal(signer)
    }

    fn genesis(id: &NodeIdentity) -> Envelope {
        genesis_with(id, id, id)
    }

    fn entry(
        id: &NodeIdentity,
        signer: &NodeIdentity,
        counter: u64,
        prev: [u8; 32],
        tag: &str,
    ) -> Envelope {
        UnsignedEnvelope {
            log: log(id),
            counter,
            prev_hash: prev,
            hlc: HlcStamp::from_parts(1_700_000_000_000 + counter, 0),
            kind: "thread.message".into(),
            kind_version: 1,
            deps: Deps::empty(),
            creates: None,
            needs: vec![],
            payload: format!(r#"{{"t":"{tag}"}}"#).into_bytes(),
        }
        .seal(signer)
    }

    /// A held log of `id` with entries 1..=3 (hashes `h1..h3`), head at 3.
    fn held(id: &NodeIdentity) -> (HeldLog, [Envelope; 3]) {
        let e1 = genesis(id);
        let e2 = entry(id, id, 2, e1.hash, "two");
        let e3 = entry(id, id, 3, e2.hash, "three");
        let held = HeldLog {
            pubkey: id.public_key(),
            contiguous: 3,
            head_hash: e3.hash,
            state: LogState::Active,
            frozen_at: None,
            is_active_self: false,
            stored_hash: None,
        };
        (held, [e1, e2, e3])
    }

    #[test]
    fn ahead_of_self() {
        let id = identity(1);
        let (mut h, [_, _, e3]) = held(&id);
        h.is_active_self = true;
        let next = entry(&id, &id, 4, e3.hash, "four");
        assert_eq!(check_next(Some(&h), &next), ChainStep::AheadOfSelf);
        // An own entry already held is a duplicate, not evidence.
        h.stored_hash = Some(e3.hash);
        assert_eq!(check_next(Some(&h), &e3), ChainStep::Duplicate);
    }

    #[test]
    fn self_log_held_entry_redelivered_is_duplicate() {
        let id = identity(1);
        let (mut h, [_, e2, _]) = held(&id);
        h.is_active_self = true;
        h.stored_hash = Some(e2.hash);
        assert_eq!(check_next(Some(&h), &e2), ChainStep::Duplicate);
    }

    #[test]
    fn self_log_bad_signature_is_corrupt() {
        let id = identity(1);
        let (mut h, [e1, e2, e3]) = held(&id);
        h.is_active_self = true;
        let forged = entry(&id, &identity(9), 4, e3.hash, "four");
        assert_eq!(
            check_next(Some(&h), &forged),
            ChainStep::Corrupt(EnvelopeError::BadSignature)
        );
        h.stored_hash = Some(e2.hash);
        let rival = entry(&id, &identity(9), 2, e1.hash, "rival");
        assert_eq!(
            check_next(Some(&h), &rival),
            ChainStep::Corrupt(EnvelopeError::BadSignature)
        );
    }

    #[test]
    fn self_log_tampered_payload_is_corrupt() {
        let id = identity(1);
        let (mut h, [_, _, e3]) = held(&id);
        h.is_active_self = true;
        let mut t = entry(&id, &id, 4, e3.hash, "four");
        t.payload = br#"{"t":"changed"}"#.to_vec();
        assert_eq!(
            check_next(Some(&h), &t),
            ChainStep::Corrupt(EnvelopeError::HashMismatch)
        );
    }

    #[test]
    fn self_log_valid_rival_at_held_counter_is_ahead_of_self() {
        let id = identity(1);
        let (mut h, [e1, e2, _]) = held(&id);
        h.is_active_self = true;
        h.stored_hash = Some(e2.hash);
        let rival = entry(&id, &id, 2, e1.hash, "rival");
        assert_eq!(check_next(Some(&h), &rival), ChainStep::AheadOfSelf);
    }

    #[test]
    fn gap_reports_expected() {
        let id = identity(1);
        let (h, [_, _, e3]) = held(&id);
        let far = entry(&id, &id, 6, e3.hash, "six");
        assert_eq!(check_next(Some(&h), &far), ChainStep::Gap { expected: 4 });
        let unknown_log = entry(&id, &id, 2, [0; 32], "x");
        assert_eq!(
            check_next(None, &unknown_log),
            ChainStep::Gap { expected: 1 }
        );
    }

    #[test]
    fn genesis_ok() {
        let id = identity(1);
        assert_eq!(check_next(None, &genesis(&id)), ChainStep::Genesis);
    }

    #[test]
    fn genesis_bad_uuid_is_corrupt() {
        let id = identity(1);
        let other = identity(2);
        // Announces a key that does not hash to the log's node.
        let bad = genesis_with(&id, &other, &other);
        assert_eq!(
            check_next(None, &bad),
            ChainStep::Corrupt(EnvelopeError::UuidMismatch)
        );
    }

    #[test]
    fn genesis_bad_signature_is_corrupt() {
        let id = identity(1);
        let forged = genesis_with(&id, &id, &identity(2));
        assert_eq!(
            check_next(None, &forged),
            ChainStep::Corrupt(EnvelopeError::BadSignature)
        );
    }

    #[test]
    fn genesis_wrong_kind_is_corrupt() {
        let id = identity(1);
        let mut not_genesis = entry(&id, &id, 1, [0; 32], "x");
        not_genesis.counter = 1;
        assert_eq!(
            check_next(None, &not_genesis),
            ChainStep::Corrupt(EnvelopeError::NotGenesis)
        );
    }

    #[test]
    fn duplicate_same_hash() {
        let id = identity(1);
        let (mut h, [_, e2, _]) = held(&id);
        h.stored_hash = Some(e2.hash);
        assert_eq!(check_next(Some(&h), &e2), ChainStep::Duplicate);
    }

    #[test]
    fn conflict_same_counter_different_hash_valid_sig() {
        let id = identity(1);
        let (mut h, [e1, e2, _]) = held(&id);
        h.stored_hash = Some(e2.hash);
        let rival = entry(&id, &id, 2, e1.hash, "rival");
        assert_ne!(rival.hash, e2.hash);
        assert_eq!(check_next(Some(&h), &rival), ChainStep::Conflict);
    }

    #[test]
    fn conflict_with_bad_signature_is_corrupt() {
        let id = identity(1);
        let (mut h, [e1, e2, _]) = held(&id);
        h.stored_hash = Some(e2.hash);
        let forged = entry(&id, &identity(9), 2, e1.hash, "rival");
        assert_eq!(
            check_next(Some(&h), &forged),
            ChainStep::Corrupt(EnvelopeError::BadSignature)
        );
    }

    #[test]
    fn beyond_frozen() {
        let id = identity(1);
        let (mut h, [_, _, e3]) = held(&id);
        h.state = LogState::Frozen;
        h.frozen_at = Some(3);
        let next = entry(&id, &id, 4, e3.hash, "four");
        assert_eq!(check_next(Some(&h), &next), ChainStep::BeyondFrozen);
        // Freeze outranks a gap: a far entry of a frozen log is still beyond it.
        let far = entry(&id, &id, 9, e3.hash, "nine");
        assert_eq!(check_next(Some(&h), &far), ChainStep::BeyondFrozen);
        // At or below the freeze counter the chain rules apply as for an active log.
        h.contiguous = 2;
        h.head_hash = held(&id).1[1].hash;
        assert_eq!(check_next(Some(&h), &e3), ChainStep::Append);
    }

    #[test]
    fn append_next_counter_matching_prev_hash() {
        let id = identity(1);
        let (h, [_, _, e3]) = held(&id);
        let next = entry(&id, &id, 4, e3.hash, "four");
        assert_eq!(check_next(Some(&h), &next), ChainStep::Append);
    }

    #[test]
    fn append_with_wrong_prev_hash_valid_sig_is_conflict() {
        let id = identity(1);
        let (h, [e1, _, _]) = held(&id);
        let fork = entry(&id, &id, 4, e1.hash, "four");
        assert_eq!(check_next(Some(&h), &fork), ChainStep::Conflict);
    }

    #[test]
    fn append_with_wrong_prev_hash_bad_sig_is_corrupt() {
        let id = identity(1);
        let (h, [e1, _, _]) = held(&id);
        let forged = entry(&id, &identity(9), 4, e1.hash, "four");
        assert_eq!(
            check_next(Some(&h), &forged),
            ChainStep::Corrupt(EnvelopeError::BadSignature)
        );
    }

    #[test]
    fn corrupt_shape() {
        let id = identity(1);
        let (h, [_, _, e3]) = held(&id);
        let mut tampered = entry(&id, &id, 4, e3.hash, "four");
        tampered.payload = br#"{"t":"changed"}"#.to_vec();
        assert_eq!(
            check_next(Some(&h), &tampered),
            ChainStep::Corrupt(EnvelopeError::HashMismatch)
        );
    }
}
