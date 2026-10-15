//! The apply side of the journal: the context a fact handler runs with.
//!
//! This file currently holds only the handler-facing contract
//! ([`ApplyContext`], [`Provenance`]). ApplyEngine (readiness, savepoints,
//! outcomes, replay) and the DB-backed context methods (`provenance`,
//! `is_local_seat`, `delivery_suppressed`) land with ht-5fr.3.8.

use std::marker::PhantomData;

use crate::node::NodeUuid;
use crate::store::connection::DecisionInstant;
use crate::sync::LogKey;
use crate::sync::facts::FactKind;
use crate::sync::hlc::HlcStamp;

/// Where an applied entry came from, as handlers and attribution see it.
///
/// Closed set. Any incarnation of this node's own log is `Local`; only an
/// entry authored by another node is `ReplicatedFrom`. The `as_str` text
/// lands with ApplyEngine (ht-5fr.3.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    Local,
    ReplicatedFrom(NodeUuid),
}

/// A fact queued by a handler through [`ApplyContext::follow_up`]: kind name,
/// major and the serialized payload. ApplyEngine authors it after the
/// triggering entry is recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFact {
    pub kind: &'static str,
    pub major: u32,
    pub payload: Vec<u8>,
}

/// What a fact handler may read and do while its entry is applied.
///
/// Owns: the facts about the entry being applied (origin log, counter,
/// `ingest_seq`, the origin's `hlc`, the local `decision` instant, the one
/// local `decision_seq` allocated for this entry, the local node), the
/// follow-up queue and the pending self-tag change. Handlers read these
/// fields and nothing else about the engine. Local positions come from
/// `decision_seq`, never from `next_decision_seq`: ApplyEngine allocates one
/// per entry, so replay and local order stay reproducible. Must not: let a
/// handler author through the journal directly (it queues with
/// [`follow_up`](Self::follow_up)), or carry a local position into a payload.
/// Role: passed `&mut` to `FactKind::apply`; built by ApplyEngine per entry.
pub struct ApplyContext<'a> {
    pub origin: LogKey,
    pub counter: u64,
    pub ingest_seq: i64,
    pub hlc: HlcStamp,
    pub decision: DecisionInstant,
    pub decision_seq: i64,
    pub local_node: NodeUuid,
    follow_ups: Vec<QueuedFact>,
    self_tag_k: Option<u32>,
    _entry: PhantomData<&'a ()>,
}

impl ApplyContext<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        origin: LogKey,
        counter: u64,
        ingest_seq: i64,
        hlc: HlcStamp,
        decision: DecisionInstant,
        decision_seq: i64,
        local_node: NodeUuid,
    ) -> Self {
        Self {
            origin,
            counter,
            ingest_seq,
            hlc,
            decision,
            decision_seq,
            local_node,
            follow_ups: Vec::new(),
            self_tag_k: None,
            _entry: PhantomData,
        }
    }

    /// The origin's "sent at", in Unix ms.
    pub fn origin_ms(&self) -> u64 {
        self.hlc.ms()
    }

    /// True when the entry's origin node is this node, in any incarnation.
    pub fn is_local_origin(&self) -> bool {
        self.origin.node == self.local_node
    }

    /// Queues a typed fact. Only queues: ApplyEngine authors it after the
    /// triggering entry is recorded, so its deps include the trigger.
    pub fn follow_up<K: FactKind>(
        &mut self,
        payload: &K::Payload,
    ) -> Result<(), serde_json::Error> {
        self.follow_ups.push(QueuedFact {
            kind: K::NAME,
            major: K::MAJOR,
            payload: serde_json::to_vec(payload)?,
        });
        Ok(())
    }

    /// The queued follow-ups, in queue order.
    pub fn follow_ups(&self) -> &[QueuedFact] {
        &self.follow_ups
    }

    /// Takes the queued follow-ups, leaving the queue empty.
    pub fn take_follow_ups(&mut self) -> Vec<QueuedFact> {
        std::mem::take(&mut self.follow_ups)
    }

    /// Carries a tag change out to be applied after commit.
    pub fn set_self_tag_k(&mut self, k: u32) {
        self.self_tag_k = Some(k);
    }

    pub fn self_tag_k(&self) -> Option<u32> {
        self.self_tag_k
    }
}
