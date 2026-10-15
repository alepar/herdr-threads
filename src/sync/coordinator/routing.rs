//! The pure half of the coordinator's routing: which duplicate link is
//! active, which link a log is pulled from, and whether peers hold history of
//! this node that it lacks.
//!
//! Responsibility (routing)
//! - Owns: [`duplicate_order`], [`wantable`], [`reselect`] (with the
//!   [`ResyncPin`] restriction it is handed) and [`own_history_on`], plain
//!   functions over values the actor copies out of its state, and the timing
//!   constants they and the actor share.
//! - Hides: the ranking rules, so the actor never compares counters or nodes.
//! - Must not: touch the actor's maps, send a frame or read a clock; every
//!   input is an argument. This is where relay-policy changes go (who serves
//!   whom, which duplicate carries traffic); the actor only applies the
//!   answers.
//! - Role: unit-testable policy under `SyncCoordinator`.
//!
//! Core and harness-neutral.

use crate::node::NodeUuid;
use crate::protocol::time::MonoInstant;

use crate::sync::{LogHead, LogKey, LogState, Vector};

/// An rx source that has not progressed for this long (ms) is no longer sticky
/// and, while it still advertises more, is penalized.
pub(super) const RX_STALL_MS: u64 = 30_000;
/// How long a stalled rx source is unwantable for that log (ms).
pub(super) const STALL_PENALTY_MS: u64 = 30_000;
/// How long the clone probe waits for any inbound frame on the older link (ms).
pub(super) const CLONE_PROBE_MS: u64 = 10_000;
/// Established links per peer node beyond which the newest standby is closed.
pub(super) const MAX_LINKS_PER_NODE: usize = 2;

/// How long a [`ResyncPin`] holds without a completed round with its peer
/// (ms).
pub(super) const PIN_LIFT_MS: u64 = 600_000;

/// A rebuilding node's rx pin (`sync resync`): the pinned logs are pulled
/// only from `from` while the pin holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResyncPin {
    /// The peer the rebuilt store is pulled from.
    pub from: NodeUuid,
    /// The nodes whose logs are pinned; `None` pins every log.
    pub logs: Option<Vec<NodeUuid>>,
}

impl ResyncPin {
    /// Whether `log` must come from [`ResyncPin::from`].
    pub(super) fn covers(&self, log: &LogKey) -> bool {
        self.logs
            .as_ref()
            .is_none_or(|nodes| nodes.contains(&log.node))
    }
}

/// What the duplicate order compares for one established link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DupKey {
    pub link: u64,
    /// The node that dialed the link.
    pub dialer: NodeUuid,
    /// The dialer's hello nonce.
    pub nonce: [u8; 32],
}

/// The links of one peer node, best first: ascending `(dialer node_uuid,
/// dialer hello nonce)`, the link id only breaking a tie that two distinct
/// handshakes cannot produce. The first is the active link. Both ends compute
/// it from the hellos alone.
///
/// The place to change which duplicate link carries traffic.
pub(super) fn duplicate_order(links: &[DupKey]) -> Vec<u64> {
    let mut sorted: Vec<&DupKey> = links.iter().collect();
    sorted.sort_by(|a, b| (a.dialer, a.nonce, a.link).cmp(&(b.dialer, b.nonce, b.link)));
    sorted.into_iter().map(|key| key.link).collect()
}

/// Whether `adv`, a peer's advertised head of a log, is worth pulling given
/// the `local` head: the log is not paused (fork, headroom), the peer holds
/// more than `local`, and a frozen local log is pulled only up to its freeze
/// counter (the branches beyond are the incarnations worker's).
pub(super) fn wantable(local: Option<&LogHead>, adv: &LogHead, paused: bool) -> bool {
    if paused {
        return false;
    }
    let held = local.map_or(0, |head| head.counter);
    if adv.counter <= held {
        return false;
    }
    match local {
        Some(LogHead {
            state: LogState::Frozen,
            frozen_at: Some(at),
            ..
        }) => i64::try_from(held).is_ok_and(|held| held < *at),
        _ => true,
    }
}

/// One active link as `reselect` sees it for one log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceView {
    pub link: u64,
    /// The peer node on the link.
    pub node: NodeUuid,
    /// What the link's peer advertises for the log, if anything.
    pub adv: Option<LogHead>,
    /// A live penalty for this log on this link.
    pub penalized: bool,
}

/// The link a log is currently pulled from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CurrentSource {
    pub link: u64,
    pub last_progress: MonoInstant,
}

/// Pick the rx source for `log` among the active links in `views`. Candidates
/// are the unpenalized links advertising a [`wantable`] head. In order:
///
/// 1. the origin itself (`view.node == log.node`), which also takes over from
///    a source that is still progressing;
/// 2. the current source, while it is a candidate and progressed within
///    [`RX_STALL_MS`];
/// 3. the highest advertised counter;
/// 4. the lower node uuid (then the lower link id, for determinism).
///
/// With `pinned_to` set (an active [`ResyncPin`] covering the log), only the
/// links to that node are candidates.
///
/// `None` when nothing is a candidate. The place to change relay policy.
pub(super) fn reselect(
    log: &LogKey,
    local: Option<&LogHead>,
    views: &[SourceView],
    paused: bool,
    current: Option<CurrentSource>,
    now: MonoInstant,
    pinned_to: Option<NodeUuid>,
) -> Option<u64> {
    let candidates: Vec<(&SourceView, u64)> = views
        .iter()
        .filter(|view| !view.penalized)
        .filter(|view| pinned_to.is_none_or(|from| view.node == from))
        .filter_map(|view| {
            let adv = view.adv.as_ref()?;
            wantable(local, adv, paused).then_some((view, adv.counter))
        })
        .collect();
    if let Some((origin, _)) = candidates.iter().find(|(view, _)| view.node == log.node) {
        return Some(origin.link);
    }
    if let Some(current) = current
        && now.0.saturating_sub(current.last_progress.0) < RX_STALL_MS
        && candidates.iter().any(|(view, _)| view.link == current.link)
    {
        return Some(current.link);
    }
    candidates
        .iter()
        .min_by(|(a, a_counter), (b, b_counter)| {
            b_counter
                .cmp(a_counter)
                .then(a.node.cmp(&b.node))
                .then(a.link.cmp(&b.link))
        })
        .map(|(view, _)| view.link)
}

/// Whether some active link advertises a log of `own` above what `local`
/// holds of it: the own-history hold's `Links` flag. For a frozen log the
/// advertised counter counts only up to its freeze counter (`local`'s when
/// `local` froze it, else the peer's); the branches beyond are the
/// incarnations worker's flag. A node no peer advertises beyond itself (a
/// fresh node) is never on.
pub(super) fn own_history_on<'a>(
    own: NodeUuid,
    local: &Vector,
    active_peers: impl IntoIterator<Item = &'a Vector>,
) -> bool {
    active_peers.into_iter().any(|peer| {
        peer.0
            .iter()
            .filter(|(log, _)| log.node == own)
            .any(|(log, adv)| {
                let held = local.0.get(log);
                let have = held.map_or(0, |head| head.counter);
                let mut counter = adv.counter;
                let freezes = [held, Some(adv)].into_iter().flatten().filter_map(|head| {
                    match (head.state, head.frozen_at) {
                        (LogState::Frozen, Some(at)) => u64::try_from(at).ok(),
                        _ => None,
                    }
                });
                for at in freezes {
                    counter = counter.min(at);
                }
                counter > have
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(n: u8) -> NodeUuid {
        NodeUuid::from_bytes([n; 16])
    }

    fn log(n: u8) -> LogKey {
        LogKey {
            node: node(n),
            incarnation: 1,
        }
    }

    fn head(counter: u64) -> LogHead {
        LogHead {
            counter,
            head_hash: [0; 32],
            state: LogState::Active,
            frozen_at: None,
        }
    }

    fn frozen(counter: u64, at: i64) -> LogHead {
        LogHead {
            state: LogState::Frozen,
            frozen_at: Some(at),
            ..head(counter)
        }
    }

    fn view(link: u64, n: u8, counter: u64) -> SourceView {
        SourceView {
            link,
            node: node(n),
            adv: Some(head(counter)),
            penalized: false,
        }
    }

    const NOW: MonoInstant = MonoInstant(100_000);

    fn pick(
        views: &[SourceView],
        local: Option<LogHead>,
        current: Option<CurrentSource>,
    ) -> Option<u64> {
        reselect(&log(9), local.as_ref(), views, false, current, NOW, None)
    }

    #[test]
    fn reselect_prefers_origin() {
        // Node 9 is the origin of the log; a peer with a higher counter loses.
        let views = [view(1, 2, 50), view(2, 9, 10), view(3, 3, 40)];
        assert_eq!(pick(&views, Some(head(5)), None), Some(2));
    }

    #[test]
    fn reselect_highest_counter_then_lower_uuid() {
        let views = [view(1, 5, 20), view(2, 3, 30), view(3, 4, 30)];
        assert_eq!(
            pick(&views, Some(head(5)), None),
            Some(2),
            "highest counter, then the lower node uuid"
        );
        let views = [view(1, 5, 20), view(2, 3, 30), view(3, 4, 31)];
        assert_eq!(pick(&views, Some(head(5)), None), Some(3), "counter wins");
        // A link that does not advertise more than local is no candidate.
        let views = [view(1, 3, 5), view(2, 4, 4)];
        assert_eq!(pick(&views, Some(head(5)), None), None);
    }

    #[test]
    fn reselect_sticky_within_30s() {
        let views = [view(1, 5, 20), view(2, 3, 30)];
        let fresh = CurrentSource {
            link: 1,
            last_progress: MonoInstant(NOW.0 - (RX_STALL_MS - 1)),
        };
        assert_eq!(
            pick(&views, Some(head(5)), Some(fresh)),
            Some(1),
            "a source that progressed within 30 s is kept over a higher counter"
        );
        let stalled = CurrentSource {
            link: 1,
            last_progress: MonoInstant(NOW.0 - RX_STALL_MS),
        };
        assert_eq!(
            pick(&views, Some(head(5)), Some(stalled)),
            Some(2),
            "at 30 s without progress the ranking decides again"
        );
        // A current source that is no candidate any more is dropped.
        let gone = CurrentSource {
            link: 7,
            last_progress: NOW,
        };
        assert_eq!(pick(&views, Some(head(5)), Some(gone)), Some(2));
    }

    #[test]
    fn reselect_origin_takes_over() {
        let views = [view(1, 5, 20), view(2, 9, 6)];
        let fresh = CurrentSource {
            link: 1,
            last_progress: NOW,
        };
        assert_eq!(
            pick(&views, Some(head(5)), Some(fresh)),
            Some(2),
            "the origin takes over a progressing source"
        );
    }

    #[test]
    fn penalized_and_paused_logs_unwantable() {
        let mut views = [view(1, 9, 20), view(2, 3, 30)];
        views[0].penalized = true;
        assert_eq!(
            pick(&views, Some(head(5)), None),
            Some(2),
            "a penalized origin is skipped"
        );
        views[1].penalized = true;
        assert_eq!(pick(&views, Some(head(5)), None), None);
        let open = [view(1, 3, 30)];
        assert_eq!(
            reselect(&log(9), Some(&head(5)), &open, true, None, NOW, None),
            None,
            "a paused log is unwantable"
        );
        // Frozen local log: pulled only below its freeze counter.
        assert!(wantable(Some(&frozen(4, 5)), &head(9), false));
        assert!(!wantable(Some(&frozen(5, 5)), &head(9), false));
        assert!(
            wantable(None, &head(1), false),
            "an unknown log is wantable"
        );
        assert!(!wantable(None, &head(0), false));
    }

    #[test]
    fn duplicate_order_min_dialer_uuid_then_nonce() {
        let key = |link, dialer, nonce: u8| DupKey {
            link,
            dialer: node(dialer),
            nonce: [nonce; 32],
        };
        let links = [key(1, 7, 1), key(2, 3, 9), key(3, 3, 2), key(4, 8, 0)];
        assert_eq!(
            duplicate_order(&links),
            vec![3, 2, 1, 4],
            "dialer uuid first, then hello nonce"
        );
        assert_eq!(duplicate_order(&[]), Vec::<u64>::new());
        // The order does not depend on the input order.
        let mut reversed = links;
        reversed.reverse();
        assert_eq!(duplicate_order(&reversed), vec![3, 2, 1, 4]);
    }

    #[test]
    fn own_history_on_cases() {
        let own = node(1);
        let own_log = |inc| LogKey {
            node: own,
            incarnation: inc,
        };
        let vector = |entries: &[(LogKey, LogHead)]| Vector(entries.iter().copied().collect());

        // Another own incarnation beyond local: on.
        let local = vector(&[(own_log(2), head(1))]);
        let peer = vector(&[(own_log(1), head(7)), (log(5), head(99))]);
        assert!(own_history_on(own, &local, [&peer]));

        // Caught up: off.
        let local = vector(&[(own_log(1), head(7)), (own_log(2), head(1))]);
        assert!(!own_history_on(own, &local, [&peer]));

        // The peer advertises no own log beyond local (fresh node): off.
        let only_foreign = vector(&[(log(5), head(99))]);
        assert!(!own_history_on(own, &Vector::default(), [&only_foreign]));
        assert!(!own_history_on(own, &Vector::default(), []));
        let zero = vector(&[(own_log(1), head(0))]);
        assert!(!own_history_on(own, &Vector::default(), [&zero]));

        // A frozen own log counts only up to its freeze counter.
        let local = vector(&[(own_log(1), frozen(3, 3))]);
        let beyond = vector(&[(own_log(1), head(9))]);
        assert!(!own_history_on(own, &local, [&beyond]));
        let local = vector(&[(own_log(1), frozen(2, 3))]);
        assert!(own_history_on(own, &local, [&beyond]));
        // A peer's freeze caps its advertisement too.
        let capped = vector(&[(own_log(1), frozen(9, 4))]);
        assert!(!own_history_on(
            own,
            &vector(&[(own_log(1), head(4))]),
            [&capped]
        ));
        assert!(own_history_on(
            own,
            &vector(&[(own_log(1), head(3))]),
            [&capped]
        ));
    }
}
