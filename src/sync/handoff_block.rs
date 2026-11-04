//! The sender block of an atomic remote handoff (TRUST-POLICY N4, N15; spec §1.1): the sender node's
//! detached signature over what it knows of one handoff message. Core and harness-neutral.
//!
//! Owns: the signing bytes, sign/verify and the single "payload restates block" predicate. Must not:
//! read the store, know `peer_requests` or obligations, or pick a key.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::ids::{MessageId, SeatId, direct_thread_id};
use crate::protocol::summary::AuthorRole;
use crate::sync::envelope::{hex_decode, hex_encode};
use crate::sync::facts::message::MessageSentPayload;
use crate::sync::facts::payload::{ReceiptObligation, RecipientSpec};

/// Signature domain of a sender block; never the opening bytes of an entry (`HTJRNL01`) or a
/// handshake (`HTPEER1-AUTH\0`).
pub const BLOCK_MAGIC: &[u8; 8] = b"HTHOFF01";
/// Largest `ack_after_ms` any verifier accepts (365 days). The sender applies a tighter bound of
/// its own (`OBLIGATION_TTL_MS`).
pub const MAX_ACK_AFTER_MS: i64 = 365 * 24 * 3_600_000;

/// What the sender's node signed about one handoff message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)] // never deny_unknown_fields
pub struct SenderBlock {
    #[serde(default)]
    pub v: u8,
    /// The launch request id.
    #[serde(default)]
    pub request: Uuid,
    #[serde(default)]
    pub sender_node: Option<NodeUuid>,
    #[serde(default)]
    pub author: Option<SeatId>,
    #[serde(default)]
    pub author_role: Option<AuthorRole>,
    #[serde(default)]
    pub message: Option<MessageId>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub require_ack: bool,
    #[serde(default)]
    pub requested_at: i64,
    /// The ACK window in ms from `requested_at`; signed.
    #[serde(default)]
    pub ack_after_ms: Option<i64>,
    /// 128 lowercase hex chars.
    #[serde(default)]
    pub sig: String,
}

/// Why a block does not verify. Callers do not branch on the variant: the target maps every one
/// to its one refusal, the materializer to `FactError::Rejected`; the variant goes only to logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockError {
    Incomplete,
    Version,
    Policy,
    Deadline,
    BadSignature,
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Incomplete => "sender block incomplete",
            Self::Version => "sender block version unsupported",
            Self::Policy => "sender block violates handoff policy",
            Self::Deadline => "sender block deadline overflows",
            Self::BadSignature => "sender block signature invalid",
        })
    }
}

impl std::error::Error for BlockError {}

fn put_str(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

impl SenderBlock {
    /// The bytes the signature covers (hashed with SHA-256 before signing).
    pub fn signing_bytes(&self) -> Result<Vec<u8>, BlockError> {
        let (Some(node), Some(author), Some(message)) =
            (&self.sender_node, &self.author, &self.message)
        else {
            return Err(BlockError::Incomplete);
        };
        let mut out = Vec::with_capacity(128 + self.body.len());
        out.extend_from_slice(BLOCK_MAGIC);
        out.push(self.v);
        out.extend_from_slice(self.request.as_bytes());
        out.extend_from_slice(node.as_bytes());
        put_str(&mut out, author.as_str());
        put_str(&mut out, self.author_role.map_or("", AuthorRole::as_str));
        put_str(&mut out, message.as_str());
        put_str(&mut out, &self.body);
        out.push(self.priority);
        out.push(u8::from(self.require_ack));
        out.extend_from_slice(&self.requested_at.to_be_bytes());
        out.push(u8::from(self.ack_after_ms.is_some()));
        out.extend_from_slice(&self.ack_after_ms.unwrap_or(0).to_be_bytes());
        Ok(out)
    }

    fn digest(&self) -> Result<[u8; 32], BlockError> {
        Ok(Sha256::digest(self.signing_bytes()?).into())
    }

    /// `requested_at + ack_after_ms`, the one place that adds them.
    pub fn deadline_at(&self) -> Result<Option<i64>, BlockError> {
        match self.ack_after_ms {
            None => Ok(None),
            Some(ms) => self
                .requested_at
                .checked_add(ms)
                .map(Some)
                .ok_or(BlockError::Deadline),
        }
    }

    pub fn sign(&mut self, identity: &NodeIdentity) -> Result<(), BlockError> {
        self.sig = hex_encode(&identity.sign(&self.digest()?));
        Ok(())
    }

    pub fn verify(&self, pubkey: &[u8; 32]) -> Result<(), BlockError> {
        if self.sender_node.is_none() || self.author.is_none() || self.message.is_none() {
            return Err(BlockError::Incomplete);
        }
        if self.v != 1 {
            return Err(BlockError::Version);
        }
        if self.priority != 0 || !self.require_ack {
            return Err(BlockError::Policy);
        }
        if let Some(ms) = self.ack_after_ms
            && !(1..=MAX_ACK_AFTER_MS).contains(&ms)
        {
            return Err(BlockError::Policy);
        }
        self.deadline_at()?;
        let sig = hex_decode::<64>(&self.sig).ok_or(BlockError::BadSignature)?;
        let key = VerifyingKey::from_bytes(pubkey).map_err(|_| BlockError::BadSignature)?;
        key.verify_strict(&self.digest()?, &Signature::from_bytes(&sig))
            .map_err(|_| BlockError::BadSignature)
    }

    /// What a handoff `message.sent` restating this block is, for `recipient`. A full struct
    /// literal: a field added to `MessageSentPayload` fails to compile here until it is pinned.
    pub fn expected_payload(
        &self,
        recipient: &SeatId,
        sent_at: i64,
    ) -> Result<MessageSentPayload, BlockError> {
        let (Some(author), Some(message)) = (&self.author, &self.message) else {
            return Err(BlockError::Incomplete);
        };
        let thread = direct_thread_id(author, recipient).map_err(|_| BlockError::Incomplete)?;
        Ok(MessageSentPayload {
            message: message.clone(),
            thread,
            author: author.clone(),
            body: self.body.clone(),
            author_role: self.author_role,
            priority: 0,
            require_ack: true,
            relays_user: false,
            user_intent: None,
            sent_at,
            duration_ms: self.ack_after_ms.unwrap_or(0),
            recipients: vec![RecipientSpec {
                seat: recipient.clone(),
                receipt: ReceiptObligation::Ack,
                deadline_at: self.deadline_at()?,
            }],
            handoff: Some(self.clone()),
        })
    }

    /// Whether `payload` is exactly the message this block describes. Does not verify the
    /// signature; the caller passes the key to [`SenderBlock::verify`].
    pub fn restates(&self, payload: &MessageSentPayload) -> bool {
        let [recipient] = payload.recipients.as_slice() else {
            return false;
        };
        self.expected_payload(&recipient.seat, payload.sent_at)
            .is_ok_and(|expected| expected == *payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Mutate<T> = Box<dyn Fn(&mut T)>;

    fn identity() -> NodeIdentity {
        NodeIdentity::from_seed([7; 32])
    }

    fn seat(id: &str) -> SeatId {
        SeatId::new(id)
    }

    fn unsigned() -> SenderBlock {
        SenderBlock {
            v: 1,
            request: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            sender_node: Some(identity().uuid()),
            author: Some(seat("sAAAAAAAAAAA")),
            author_role: Some(AuthorRole::Agent),
            message: Some(MessageId::new("mHOFFabcdefgh")),
            body: "do the thing".into(),
            priority: 0,
            require_ack: true,
            requested_at: 1_700_000_000_000,
            ack_after_ms: Some(600_000),
            sig: String::new(),
        }
    }

    fn signed() -> SenderBlock {
        let mut b = unsigned();
        b.sign(&identity()).unwrap();
        b
    }

    fn resign(mut b: SenderBlock) -> SenderBlock {
        b.sign(&identity()).unwrap();
        b
    }

    fn key() -> [u8; 32] {
        identity().public_key()
    }

    #[test]
    fn signing_bytes_golden() {
        let b = unsigned();
        let bytes = b.signing_bytes().unwrap();
        assert!(bytes.starts_with(b"HTHOFF01"));
        let mut by_hand = Vec::new();
        by_hand.extend_from_slice(b"HTHOFF01");
        by_hand.push(1);
        by_hand.extend_from_slice(&0x0123_4567_89ab_cdef_0123_4567_89ab_cdef_u128.to_be_bytes());
        by_hand.extend_from_slice(identity().uuid().as_bytes());
        for s in ["sAAAAAAAAAAA", "agent", "mHOFFabcdefgh", "do the thing"] {
            by_hand.extend_from_slice(&(s.len() as u32).to_be_bytes());
            by_hand.extend_from_slice(s.as_bytes());
        }
        by_hand.extend_from_slice(&[0, 1]);
        by_hand.extend_from_slice(&1_700_000_000_000_i64.to_be_bytes());
        by_hand.push(1);
        by_hand.extend_from_slice(&600_000_i64.to_be_bytes());
        assert_eq!(bytes, by_hand);
        assert_eq!(hex_encode(&bytes), GOLDEN_BYTES_HEX);
        assert_eq!(signed().sig, GOLDEN_SIG);
    }

    #[test]
    fn sign_verify_round_trip() {
        assert_eq!(signed().verify(&key()), Ok(()));
    }

    #[test]
    fn every_tampered_field_fails() {
        let other_node = NodeIdentity::from_seed([8; 32]).uuid();
        let cases: Vec<(&str, Mutate<SenderBlock>)> = vec![
            ("request", Box::new(|b| b.request = Uuid::from_u128(1))),
            (
                "sender_node",
                Box::new(move |b| b.sender_node = Some(other_node)),
            ),
            (
                "author",
                Box::new(|b| b.author = Some(seat("sBBBBBBBBBBB"))),
            ),
            (
                "author_role",
                Box::new(|b| b.author_role = Some(AuthorRole::Human)),
            ),
            ("author_role none", Box::new(|b| b.author_role = None)),
            (
                "message",
                Box::new(|b| b.message = Some(MessageId::new("mOTHER000000"))),
            ),
            ("body", Box::new(|b| b.body.push('!'))),
            ("requested_at", Box::new(|b| b.requested_at += 1)),
            (
                "ack_after_ms other",
                Box::new(|b| b.ack_after_ms = Some(600_001)),
            ),
            ("ack_after_ms none", Box::new(|b| b.ack_after_ms = None)),
            (
                "sig",
                Box::new(|b| {
                    let first = if b.sig.starts_with('0') { "1" } else { "0" };
                    b.sig.replace_range(0..1, first);
                }),
            ),
        ];
        for (name, mutate) in cases {
            let mut b = signed();
            mutate(&mut b);
            assert!(b.verify(&key()).is_err(), "tampered {name} verified");
        }
    }

    #[test]
    fn journal_domain_signature_does_not_verify() {
        let b = unsigned();
        let mut bytes = b.signing_bytes().unwrap();
        bytes[..8].copy_from_slice(b"HTJRNL01");
        let hash: [u8; 32] = Sha256::digest(&bytes).into();
        let mut forged = b.clone();
        forged.sig = hex_encode(&identity().sign(&hash));
        assert_eq!(forged.verify(&key()), Err(BlockError::BadSignature));

        let raw: [u8; 32] = Sha256::digest(b.signing_bytes().unwrap()).into();
        let mut wrong_key = b;
        wrong_key.sig = hex_encode(&NodeIdentity::from_seed([9; 32]).sign(&raw));
        assert_eq!(wrong_key.verify(&key()), Err(BlockError::BadSignature));
    }

    #[test]
    fn verify_enforces_invariants() {
        let with = |f: &dyn Fn(&mut SenderBlock)| {
            let mut b = unsigned();
            f(&mut b);
            resign(b)
        };
        assert_eq!(with(&|b| b.v = 2).verify(&key()), Err(BlockError::Version));
        assert_eq!(
            with(&|b| b.priority = 1).verify(&key()),
            Err(BlockError::Policy)
        );
        assert_eq!(
            with(&|b| b.require_ack = false).verify(&key()),
            Err(BlockError::Policy)
        );
        for bad in [0, -5, MAX_ACK_AFTER_MS + 1] {
            assert_eq!(
                with(&|b| b.ack_after_ms = Some(bad)).verify(&key()),
                Err(BlockError::Policy),
                "ack_after_ms {bad}"
            );
        }
        assert_eq!(
            with(&|b| b.ack_after_ms = Some(MAX_ACK_AFTER_MS)).verify(&key()),
            Ok(())
        );
        assert_eq!(with(&|b| b.ack_after_ms = None).verify(&key()), Ok(()));
    }

    #[test]
    fn incomplete_block_refused_before_signature() {
        let cases: Vec<Mutate<SenderBlock>> = vec![
            Box::new(|b| b.sender_node = None),
            Box::new(|b| b.author = None),
            Box::new(|b| b.message = None),
        ];
        for mutate in cases {
            let mut b = signed();
            mutate(&mut b);
            assert_eq!(b.verify(&key()), Err(BlockError::Incomplete));
            assert_eq!(b.signing_bytes(), Err(BlockError::Incomplete));
        }
    }

    #[test]
    fn deadline_at_checked() {
        assert_eq!(unsigned().deadline_at(), Ok(Some(1_700_000_600_000)));
        let mut none = unsigned();
        none.ack_after_ms = None;
        assert_eq!(none.deadline_at(), Ok(None));
        let mut over = unsigned();
        over.requested_at = i64::MAX - 1;
        over.ack_after_ms = Some(10);
        assert_eq!(over.deadline_at(), Err(BlockError::Deadline));
        assert_eq!(resign(over).verify(&key()), Err(BlockError::Deadline));
    }

    fn payload() -> (SenderBlock, MessageSentPayload) {
        let b = signed();
        let p = b.expected_payload(&seat("sRECIPIENT01"), 42).unwrap();
        (b, p)
    }

    #[test]
    fn restates_accepts_exact_and_other_sent_at() {
        let (b, mut p) = payload();
        assert!(b.restates(&p));
        p.sent_at = 99;
        assert!(b.restates(&p));
    }

    #[test]
    fn restates_rejects_each_differing_field() {
        let (b, good) = payload();
        let mut other_block = b.clone();
        other_block.body.push('x');
        let cases: Vec<(&str, Mutate<MessageSentPayload>)> = vec![
            (
                "message",
                Box::new(|p| p.message = MessageId::new("mOTHER000000")),
            ),
            (
                "thread",
                Box::new(|p| p.thread = crate::protocol::ids::ThreadId::new("dOTHER00000")),
            ),
            ("author", Box::new(|p| p.author = seat("sBBBBBBBBBBB"))),
            ("body", Box::new(|p| p.body.push('!'))),
            (
                "author_role",
                Box::new(|p| p.author_role = Some(AuthorRole::Human)),
            ),
            ("priority", Box::new(|p| p.priority = 1)),
            ("require_ack", Box::new(|p| p.require_ack = false)),
            ("relays_user", Box::new(|p| p.relays_user = true)),
            (
                "user_intent",
                Box::new(|p| p.user_intent = Some(crate::protocol::summary::UserIntent::Query)),
            ),
            ("duration_ms", Box::new(|p| p.duration_ms += 1)),
            (
                "receipt",
                Box::new(|p| p.recipients[0].receipt = ReceiptObligation::None),
            ),
            (
                "deadline_at",
                Box::new(|p| p.recipients[0].deadline_at = None),
            ),
            ("handoff none", Box::new(|p| p.handoff = None)),
            (
                "handoff changed",
                Box::new(move |p| p.handoff = Some(other_block.clone())),
            ),
            (
                "two same recipients",
                Box::new(|p| p.recipients.push(p.recipients[0].clone())),
            ),
            (
                "two recipients",
                Box::new(|p| {
                    let mut second = p.recipients[0].clone();
                    second.seat = seat("sOTHERRECIP1");
                    p.recipients.push(second);
                }),
            ),
            ("no recipients", Box::new(|p| p.recipients.clear())),
        ];
        for (name, mutate) in cases {
            let mut p = good.clone();
            mutate(&mut p);
            assert!(!b.restates(&p), "restates accepted a differing {name}");
        }
    }

    #[test]
    fn serde_defaults() {
        let b: SenderBlock = serde_json::from_str("{}").unwrap();
        assert_eq!(b, SenderBlock::default());
        assert!(b.sender_node.is_none() && b.author.is_none() && b.message.is_none());
        let extra: SenderBlock = serde_json::from_str(r#"{"v":1,"future":true}"#).unwrap();
        assert_eq!(extra.v, 1);
    }

    const GOLDEN_BYTES_HEX: &str = "4854484f46463031010123456789abcdef0123456789abcdeffe812c12f3ab4ce6ac5db69ac352f9060000000c734141414141414141414141000000056167656e740000000d6d484f464661626364656667680000000c646f20746865207468696e6700010000018bcfe568000100000000000927c0";
    const GOLDEN_SIG: &str = "ebf99075c92d9c5754b5a036e477a2b33325aa1add39e3dca6ee4eddb9d306414b9b87e019cd492b86f31d0da53957b26e0a5c16aab937186635c672d00f4301";
}
