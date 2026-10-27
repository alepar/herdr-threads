//! The coordinator's view of each peer node's health (ht-xg88 node status).
//!
//! Responsibility (peer_status)
//! - Owns: per-link application contact (`app_rx`: frames the peer's
//!   coordinator sent, never the link's own ping/pong), each direction's
//!   position and progress clock, the stall rules, the peer's `node_info`
//!   version, and the throttled shadow of all of it in `sync_peer_status`.
//! - Hides: that positions come from the active link's `peer_vector` and the
//!   actor's `local` vector, and the flush schedule.
//! - Must not: change a link, rank, dial or rx decision; every value here is
//!   diagnostic (`peer list`, `sync status`).
//! - Role: read by `on_request` (snapshot) and fed by `on_frame`,
//!   `poll_contacts` and housekeeping.
//!
//! Core and harness-neutral.

use super::*;

/// A direction with something to move and no progress for this long is
/// stalled (ms).
pub(super) const STALL_AFTER_MS: u64 = 120_000;
/// An active link whose peer coordinator sent nothing for this long (three
/// missed `seen`) is stalled at the application level (ms).
pub(super) const APP_SILENT_AFTER_MS: u64 = 30_000;
/// Least gap between two routine peer-status flushes (ms). A link coming up
/// or going down and a new version are written at the next tick.
const PEER_STATUS_FLUSH_MS: u64 = 10_000;

/// `(held, head)` of one direction: what the receiving side holds of a log
/// against that log's head.
pub(super) type Position = Option<(u64, u64)>;

/// A peer-status write joined from the actor's `JoinSet`.
pub(super) struct PeerFlushDone {
    rows: Vec<PeerStatus>,
    result: Result<(), ApiError>,
}

impl Actor {
    /// The own log with the highest incarnation this node holds, and its head.
    fn own_head(&self) -> Option<(LogKey, u64)> {
        let own = self.deps.identity.uuid();
        self.local
            .0
            .iter()
            .filter(|(log, _)| log.node == own)
            .max_by_key(|(log, _)| log.incarnation)
            .map(|(log, head)| (*log, head.counter))
    }

    /// `(tx, rx)` of an active link whose peer vector is known: what the peer
    /// holds of our active log against its head, and what we hold of the
    /// peer's active log against the head it advertises.
    pub(super) fn link_positions(&self, entry: &LinkEntry) -> (Position, Position) {
        if entry.mode != LinkMode::Active || !entry.vector_known {
            return (None, None);
        }
        let tx = self.own_head().map(|(log, head)| {
            let acked = entry.peer_vector.0.get(&log).map_or(0, |h| h.counter);
            (acked, head)
        });
        let peer = entry.handle.peer().node;
        let rx = entry
            .peer_vector
            .0
            .iter()
            .filter(|(log, _)| log.node == peer)
            .max_by_key(|(log, _)| log.incarnation)
            .map(|(log, adv)| {
                let have = self.local.0.get(log).map_or(0, |h| h.counter);
                (have, adv.counter)
            });
        (tx, rx)
    }

    /// Advance each active link's progress clocks: a direction that is caught
    /// up, or whose held counter grew, has made progress now.
    pub(super) fn track_progress(&mut self) {
        let now = self.mono();
        let ids: Vec<u64> = self.links.keys().copied().collect();
        for id in ids {
            let (tx, rx) = self.link_positions(&self.links[&id]);
            let entry = self.links.get_mut(&id).expect("listed above");
            step(tx, &mut entry.tx_mark, &mut entry.tx_progress_at, now);
            step(rx, &mut entry.rx_mark, &mut entry.rx_progress_at, now);
        }

        fn step(pos: Position, mark: &mut Option<u64>, at: &mut MonoInstant, now: MonoInstant) {
            match pos {
                Some((held, head)) if held < head => {
                    if mark.is_none_or(|m| held > m) {
                        *mark = Some(held);
                        *at = now;
                    }
                }
                // Caught up, or nothing known yet: nothing is waiting.
                _ => {
                    *mark = None;
                    *at = now;
                }
            }
        }
    }

    /// The stall flags of a link at `now` (all false off an active link).
    pub(super) fn stall_of(&self, entry: &LinkEntry, now: MonoInstant) -> Stall {
        if entry.mode != LinkMode::Active || entry.closing || !entry.ranked {
            return Stall::default();
        }
        let (tx, rx) = self.link_positions(entry);
        let over = |at: MonoInstant, limit: u64| now.0.saturating_sub(at.0) > limit;
        // Stalled only while the held counter is still the one the last
        // tick recorded: progress (or a catch-up) since that tick is not yet
        // on the clock, and must not read as a stall.
        let stuck = |pos: Position, mark: Option<u64>, at: MonoInstant| {
            pos.is_some_and(|(held, head)| held < head && mark == Some(held))
                && over(at, self.timings.stall_after_ms)
        };
        Stall {
            app: over(entry.app_rx, self.timings.app_silent_after_ms),
            tx: stuck(tx, entry.tx_mark, entry.tx_progress_at),
            rx: stuck(rx, entry.rx_mark, entry.rx_progress_at),
        }
    }

    /// `node_info` from the peer: kept for display when the link negotiated
    /// [`NODE_INFO_CAP`] and the text is within bounds.
    pub(super) fn on_node_info(&mut self, link_id: u64, info: NodeInfo) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        let negotiated = entry.handle.peer().caps.iter().any(|c| c == NODE_INFO_CAP);
        if negotiated
            && !info.version.is_empty()
            && info.version.len() <= MAX_NODE_VERSION_BYTES
            && !info.version.chars().any(char::is_control)
        {
            entry.peer_version = Some(info.version);
        }
    }

    /// Send our `node_info` on a just-established link that negotiated it.
    pub(super) fn send_node_info(&self, link_id: u64) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        if entry.handle.peer().caps.iter().any(|c| c == NODE_INFO_CAP) {
            let _ = entry.handle.send(PeerFrame::NodeInfo(NodeInfo {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }));
        }
    }

    /// The current status row of every node with an established link: from
    /// its active link if it has one, else from any of its links.
    fn peer_rows(&self) -> BTreeMap<NodeUuid, PeerStatus> {
        let mut rows = BTreeMap::new();
        for (node, ids) in &self.by_node {
            let entries: Vec<&LinkEntry> = ids.iter().filter_map(|id| self.links.get(id)).collect();
            let Some(entry) = entries
                .iter()
                .find(|l| l.mode == LinkMode::Active && !l.closing)
                .or_else(|| entries.first())
            else {
                continue;
            };
            let active = entry.mode == LinkMode::Active && !entry.closing;
            let (tx, rx) = self.link_positions(entry);
            let since = u64::try_from(entry.since.0).unwrap_or(0);
            rows.insert(
                *node,
                PeerStatus {
                    node: *node,
                    version: entry.peer_version.clone(),
                    caps: entry.handle.peer().caps.clone(),
                    boot: entry.handle.peer().boot,
                    linked_since: active.then_some(since),
                    last_linked_at: if active { self.wall() } else { since },
                    tx,
                    rx,
                    app_seen_at: Some(entry.app_rx_wall),
                },
            );
        }
        rows
    }

    /// Write the rows that changed since the last write: at once when a node's
    /// link came up or went down or its version or boot changed, else at most
    /// every [`PEER_STATUS_FLUSH_MS`]. At most one write is in flight.
    pub(super) fn flush_peer_status(&mut self) {
        if !self.peer_flushes.is_empty() {
            return;
        }
        let now = self.mono();
        let mut current = self.peer_rows();
        // A node whose links all ended: its last row, no longer linked.
        for (node, flushed) in &self.peer_flushed {
            if !current.contains_key(node) && flushed.linked_since.is_some() {
                current.insert(
                    *node,
                    PeerStatus {
                        linked_since: None,
                        ..flushed.clone()
                    },
                );
            }
        }
        let routine = self
            .peer_flushed_at
            .is_none_or(|at| now.0.saturating_sub(at.0) >= PEER_STATUS_FLUSH_MS);
        let rows: Vec<PeerStatus> = current
            .into_values()
            .filter(|row| match self.peer_flushed.get(&row.node) {
                None => true,
                Some(old) => {
                    let urgent = old.linked_since.is_some() != row.linked_since.is_some()
                        || old.version != row.version
                        || old.boot != row.boot;
                    let changed = old.tx != row.tx || old.rx != row.rx || old.caps != row.caps;
                    urgent || (routine && (changed || old.app_seen_at != row.app_seen_at))
                }
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        self.peer_flushed_at = Some(now);
        let store = self.deps.sync_store.clone();
        self.peer_flushes.spawn_blocking(move || {
            let result = store.record_peer_status(&rows, &unbounded_budget());
            PeerFlushDone { rows, result }
        });
    }

    pub(super) fn on_peer_status_flushed(&mut self, done: PeerFlushDone) {
        match done.result {
            Ok(()) => {
                for row in done.rows {
                    self.peer_flushed.insert(row.node, row);
                }
            }
            Err(e) => (self.deps.log)(&format!(
                "sync: peer status flush failed: {:?}: {:?}",
                e.code, e.detail
            )),
        }
    }
}
