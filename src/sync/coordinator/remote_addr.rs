//! Remote address resolution over the peer link (`addr.resolve.v1`).
//!
//! Responsibility (remote_addr)
//! - Owns: this node's outstanding `addr_resolve` requests (by id, with the
//!   link they went out on and their reply channel), the bound on requests a
//!   peer may have in flight here, and the hand-off of an inbound request to
//!   the daemon's [`AddrResolver`].
//! - Hides: request ids and the link a request travels on.
//! - Must not: resolve anything itself, or decide anything about a seat: the
//!   answering node's resolver decides against its own canonical view
//!   (TRUST-POLICY A2, N14), and the asking node only learns a seat ID.
//! - Role: fed by `SyncHandle::resolve_remote` and by `on_frame`; a link's
//!   end fails its requests; housekeeping drops expired ones.
//!
//! Core and harness-neutral.

use std::sync::OnceLock;

use super::*;
use crate::sync::frame::{
    ADDR_RESOLVE_CAP, AddrOutcome, AddrResolve, AddrResolved, AddrTarget, MAX_ADDR_DETAIL_BYTES,
    MAX_ADDR_TEXT_BYTES,
};

/// Requests one peer may have in flight on this node; past it the answer is
/// `refused: busy`.
pub const MAX_INBOUND_IN_FLIGHT: usize = 8;
/// Requests this node may have outstanding, across all links.
const MAX_OUTSTANDING: usize = 64;
/// A request still unanswered after this long is dropped by housekeeping
/// (the caller's own wait is shorter).
const OUTSTANDING_TTL_MS: u64 = 60_000;

/// Resolves an address against this node's own live topology and seat
/// table, for a linked peer. Called on a blocking thread.
pub trait AddrResolver: Send + Sync {
    fn resolve(&self, target: AddrTarget) -> AddrOutcome;
}

/// Where the daemon's resolver is installed once its hosts exist.
pub type ResolverSlot = Arc<OnceLock<Arc<dyn AddrResolver>>>;

/// What became of one outbound request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteAnswer {
    /// The node answered.
    Answered(AddrOutcome),
    /// No established active link to the node.
    NotLinked,
    /// The link does not carry [`ADDR_RESOLVE_CAP`]; the peer's version when
    /// known.
    NoCap { version: Option<String> },
    /// The link ended before the answer, or too many requests are
    /// outstanding here.
    LinkLost,
}

pub(super) struct Outstanding {
    link_id: u64,
    at: MonoInstant,
    reply: oneshot::Sender<RemoteAnswer>,
}

/// One inbound request, answered on a blocking thread.
pub(super) struct Served {
    link_id: u64,
    id: Uuid,
    outcome: AddrOutcome,
}

pub(super) struct RemoteAddrState {
    pub(super) resolver: ResolverSlot,
    outstanding: BTreeMap<Uuid, Outstanding>,
    in_flight: BTreeMap<u64, usize>,
    pub(super) serving: JoinSet<Served>,
}

impl RemoteAddrState {
    pub(super) fn new(resolver: ResolverSlot) -> Self {
        Self {
            resolver,
            outstanding: BTreeMap::new(),
            in_flight: BTreeMap::new(),
            serving: JoinSet::new(),
        }
    }
}

/// `text` cut to at most `max` bytes on a char boundary.
fn bounded(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn oversized(target: &AddrTarget) -> bool {
    let long = |t: &Option<String>| t.as_ref().is_some_and(|t| t.len() > MAX_ADDR_TEXT_BYTES);
    match target {
        AddrTarget::Seat { location } => location.len() > MAX_ADDR_TEXT_BYTES,
        AddrTarget::Pane { space, tab, pane } => long(space) || long(tab) || long(pane),
        AddrTarget::Unknown => false,
    }
}

fn refused(detail: &str) -> AddrOutcome {
    AddrOutcome::Refused {
        detail: bounded(detail, MAX_ADDR_DETAIL_BYTES),
    }
}

/// The answer as this node sends it: every text bounded, at most
/// [`crate::sync::frame::MAX_ADDR_CANDIDATES`] candidates.
fn bound_outcome(outcome: AddrOutcome) -> AddrOutcome {
    use crate::sync::frame::MAX_ADDR_CANDIDATES;
    let b = |t: String| bounded(&t, MAX_ADDR_DETAIL_BYTES);
    match outcome {
        AddrOutcome::Seat { seat, path, pane } => AddrOutcome::Seat {
            seat: b(seat),
            path: b(path),
            pane: b(pane),
        },
        AddrOutcome::NoSeat { path, pane } => AddrOutcome::NoSeat {
            path: b(path),
            pane: b(pane),
        },
        AddrOutcome::Ambiguous {
            mut candidates,
            omitted,
        } => {
            let extra = candidates.len().saturating_sub(MAX_ADDR_CANDIDATES);
            candidates.truncate(MAX_ADDR_CANDIDATES);
            for c in &mut candidates {
                c.path = b(std::mem::take(&mut c.path));
                c.pane = b(std::mem::take(&mut c.pane));
                c.seat = c.seat.take().map(b);
            }
            AddrOutcome::Ambiguous {
                candidates,
                omitted: omitted.saturating_add(u32::try_from(extra).unwrap_or(u32::MAX)),
            }
        }
        AddrOutcome::Refused { detail } => refused(&detail),
        other => other,
    }
}

impl Actor {
    fn addr_link(&self, node: &NodeUuid) -> Option<u64> {
        self.by_node.get(node)?.iter().copied().find(|id| {
            self.links
                .get(id)
                .is_some_and(|l| l.mode == LinkMode::Active && l.ranked && !l.closing)
        })
    }

    /// Send `target` to `node` and answer `reply` when it answers.
    pub(super) fn on_resolve_remote(
        &mut self,
        node: NodeUuid,
        target: AddrTarget,
        reply: oneshot::Sender<RemoteAnswer>,
    ) {
        let Some(link_id) = self.addr_link(&node) else {
            let _ = reply.send(RemoteAnswer::NotLinked);
            return;
        };
        let entry = &self.links[&link_id];
        if !entry
            .handle
            .peer()
            .caps
            .iter()
            .any(|c| c == ADDR_RESOLVE_CAP)
        {
            let _ = reply.send(RemoteAnswer::NoCap {
                version: entry.peer_version.clone(),
            });
            return;
        }
        if self.addr.outstanding.len() >= MAX_OUTSTANDING {
            let _ = reply.send(RemoteAnswer::LinkLost);
            return;
        }
        let id = Uuid::new_v4();
        if entry
            .handle
            .send(PeerFrame::AddrResolve(AddrResolve { id, target }))
            .is_err()
        {
            let _ = reply.send(RemoteAnswer::LinkLost);
            return;
        }
        let at = self.mono();
        self.addr
            .outstanding
            .insert(id, Outstanding { link_id, at, reply });
    }

    /// An `addr_resolve` from a peer: answered by the resolver on a blocking
    /// thread, or at once when it cannot be.
    pub(super) fn on_addr_resolve(&mut self, link_id: u64, request: AddrResolve) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        if !entry
            .handle
            .peer()
            .caps
            .iter()
            .any(|c| c == ADDR_RESOLVE_CAP)
        {
            return;
        }
        let answer_now = |outcome: AddrOutcome| {
            let _ = entry.handle.send(PeerFrame::AddrResolved(AddrResolved {
                id: request.id,
                outcome,
            }));
        };
        if matches!(request.target, AddrTarget::Unknown) {
            return answer_now(refused("unsupported target"));
        }
        if oversized(&request.target) {
            return answer_now(refused("address too long"));
        }
        let Some(resolver) = self.addr.resolver.get().cloned() else {
            return answer_now(refused("resolver unavailable"));
        };
        let busy = self.addr.in_flight.entry(link_id).or_default();
        if *busy >= MAX_INBOUND_IN_FLIGHT {
            return answer_now(refused("busy"));
        }
        *busy += 1;
        let id = request.id;
        let target = request.target;
        self.addr.serving.spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || resolver.resolve(target))
                .await
                .unwrap_or_else(|_| refused("resolver failed"));
            Served {
                link_id,
                id,
                outcome,
            }
        });
    }

    /// A served request: its answer goes back on the link it came on.
    pub(super) fn on_addr_served(&mut self, joined: Result<Served, tokio::task::JoinError>) {
        let Ok(served) = joined else {
            return;
        };
        if let Some(busy) = self.addr.in_flight.get_mut(&served.link_id) {
            *busy = busy.saturating_sub(1);
            if *busy == 0 {
                self.addr.in_flight.remove(&served.link_id);
            }
        }
        if let Some(entry) = self.links.get(&served.link_id) {
            let _ = entry.handle.send(PeerFrame::AddrResolved(AddrResolved {
                id: served.id,
                outcome: bound_outcome(served.outcome),
            }));
        }
    }

    /// An `addr_resolved`: completes the request of that id, only from the
    /// link it went out on. A late or unknown one is dropped.
    pub(super) fn on_addr_resolved(&mut self, link_id: u64, reply: AddrResolved) {
        if self
            .addr
            .outstanding
            .get(&reply.id)
            .is_some_and(|o| o.link_id == link_id)
            && let Some(done) = self.addr.outstanding.remove(&reply.id)
        {
            let _ = done.reply.send(RemoteAnswer::Answered(reply.outcome));
        }
    }

    /// A link ended: its outstanding requests fail at once.
    pub(super) fn addr_link_ended(&mut self, link_id: u64) {
        self.addr.in_flight.remove(&link_id);
        let lost: Vec<Uuid> = self
            .addr
            .outstanding
            .iter()
            .filter(|(_, o)| o.link_id == link_id)
            .map(|(id, _)| *id)
            .collect();
        for id in lost {
            if let Some(done) = self.addr.outstanding.remove(&id) {
                let _ = done.reply.send(RemoteAnswer::LinkLost);
            }
        }
    }

    /// Housekeeping: requests nobody waits for any more are dropped.
    pub(super) fn addr_housekeeping(&mut self) {
        let now = self.mono();
        self.addr.outstanding.retain(|_, o| {
            !o.reply.is_closed() && now.0.saturating_sub(o.at.0) <= OUTSTANDING_TTL_MS
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::frame::AddrCandidate;

    #[test]
    fn outcomes_are_bounded_before_sending() {
        let long = "x".repeat(MAX_ADDR_DETAIL_BYTES + 10);
        let candidates = (0..40)
            .map(|i| AddrCandidate {
                path: long.clone(),
                pane: format!("w1:p{i}"),
                seat: None,
            })
            .collect();
        let AddrOutcome::Ambiguous {
            candidates,
            omitted,
        } = bound_outcome(AddrOutcome::Ambiguous {
            candidates,
            omitted: 2,
        })
        else {
            panic!("ambiguous")
        };
        assert_eq!(candidates.len(), crate::sync::frame::MAX_ADDR_CANDIDATES);
        assert_eq!(omitted, 10);
        assert!(
            candidates
                .iter()
                .all(|c| c.path.len() == MAX_ADDR_DETAIL_BYTES)
        );
        assert_eq!(
            bound_outcome(refused(&long)),
            AddrOutcome::Refused {
                detail: "x".repeat(MAX_ADDR_DETAIL_BYTES)
            }
        );
    }

    #[test]
    fn oversized_targets_are_detected() {
        let long = "y".repeat(MAX_ADDR_TEXT_BYTES + 1);
        assert!(oversized(&AddrTarget::Seat {
            location: long.clone()
        }));
        assert!(oversized(&AddrTarget::Pane {
            space: None,
            tab: Some(long),
            pane: None
        }));
        assert!(!oversized(&AddrTarget::Seat {
            location: "a/b".into()
        }));
    }
}
