//! The coordinator's view of each peer node's health (ht-xg88 node status).
//!
//! Responsibility (peer_status)
//! - Owns: per-link application contact (`app_rx`: frames the peer's
//!   coordinator sent, never the link's own ping/pong), each direction's
//!   position and progress clock, the stall rules, the peer's `node_info`
//!   version, and the throttled shadow of the last-known values in
//!   `sync_peer_status`.
//! - Hides: that positions come from the active link's `peer_vector` and the
//!   actor's `local` vector, and the flush schedule and its backoff.
//! - Must not: change a link, rank, dial or rx decision; every value here is
//!   diagnostic (`peer list`, `sync status`).
//! - Role: read by `on_request` (snapshot) and fed by `on_frame`,
//!   `poll_contacts`, housekeeping and shutdown.
//!
//! Core and harness-neutral.

use super::*;

/// A direction with something to move and no progress for this long is
/// stalled (ms).
pub(super) const STALL_AFTER_MS: u64 = 120_000;
/// An active link whose peer coordinator sent nothing for this long (three
/// missed `seen`, sent every [`SEEN_EVERY_MS`]) is stalled at the
/// application level (ms).
pub(super) const APP_SILENT_AFTER_MS: u64 = 30_000;
/// Least gap between two routine peer-status writes, and the wait after a
/// failed one (ms). A new version or boot is written at the next tick.
const PEER_STATUS_FLUSH_MS: u64 = 10_000;
/// The stored app contact is rewritten only once it has advanced this much
/// (ms), as `ContactBook` does for contacts: between links it is only
/// read as "about when".
const APP_SEEN_ADVANCE_MS: u64 = 30_000;
/// A write still in flight after this long is reported once (ms).
const PEER_STATUS_STUCK_MS: u64 = 60_000;

/// One direction of a link: the receiving side holds `held` of `log`, whose
/// head is `head`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Direction {
    pub log: LogKey,
    pub held: u64,
    pub head: u64,
}

pub(super) type Position = Option<Direction>;

/// A peer-status write joined from the actor's `JoinSet`.
pub(super) struct PeerFlushDone {
    rows: Vec<PeerStatus>,
    result: Result<(), ApiError>,
}

/// The write side's health: when a write started, whether it was reported
/// stuck, and the backoff after a failure (one log line per failure run).
#[derive(Default)]
pub(super) struct FlushHealth {
    started: Option<MonoInstant>,
    stuck_logged: bool,
    retry_at: Option<MonoInstant>,
    failing: bool,
}

/// Counters past `i64::MAX` (a peer may advertise any `u64`) are stored
/// clamped: display data must never fail a write.
fn clamp(value: u64) -> u64 {
    value.min(i64::MAX as u64)
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
        let tx = self.own_head().map(|(log, head)| Direction {
            log,
            held: entry.peer_vector.0.get(&log).map_or(0, |h| h.counter),
            head,
        });
        let peer = entry.handle.peer().node;
        let rx = entry
            .peer_vector
            .0
            .iter()
            .filter(|(log, _)| log.node == peer)
            .max_by_key(|(log, _)| log.incarnation)
            .map(|(log, adv)| Direction {
                log: *log,
                held: self.local.0.get(log).map_or(0, |h| h.counter),
                head: adv.counter,
            });
        (tx, rx)
    }

    /// Advance each active link's progress clocks: a direction that is caught
    /// up, whose held counter grew, or whose active log changed (a new
    /// incarnation) has made progress now.
    pub(super) fn track_progress(&mut self) {
        let now = self.mono();
        let ids: Vec<u64> = self.links.keys().copied().collect();
        for id in ids {
            let (tx, rx) = self.link_positions(&self.links[&id]);
            let entry = self.links.get_mut(&id).expect("listed above");
            step(tx, &mut entry.tx_mark, &mut entry.tx_progress_at, now);
            step(rx, &mut entry.rx_mark, &mut entry.rx_progress_at, now);
        }

        fn step(
            pos: Position,
            mark: &mut Option<(LogKey, u64)>,
            at: &mut MonoInstant,
            now: MonoInstant,
        ) {
            match pos {
                Some(d) if d.held < d.head => {
                    if mark.is_none_or(|(log, held)| log != d.log || d.held > held) {
                        *mark = Some((d.log, d.held));
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

    /// The stall flags of a link at `now`, given its positions (all false off
    /// an active, ranked, open link).
    pub(super) fn stall_of(
        &self,
        entry: &LinkEntry,
        tx: Position,
        rx: Position,
        now: MonoInstant,
    ) -> Stall {
        if entry.mode != LinkMode::Active || entry.closing || !entry.ranked {
            return Stall::default();
        }
        let over = |at: MonoInstant, limit: u64| now.0.saturating_sub(at.0) > limit;
        // Stalled only while the held counter of the same log is still the
        // one the last tick recorded: progress (or a catch-up) since that
        // tick is not yet on the clock, and must not read as a stall.
        let stuck = |pos: Position, mark: Option<(LogKey, u64)>, at: MonoInstant| {
            pos.is_some_and(|d| d.held < d.head && mark == Some((d.log, d.held)))
                && over(at, self.timings.stall_after_ms)
        };
        Stall {
            app: over(entry.app_rx, self.timings.app_silent_after_ms),
            tx: stuck(tx, entry.tx_mark, entry.tx_progress_at),
            rx: stuck(rx, entry.rx_mark, entry.rx_progress_at),
        }
    }

    /// `node_info` from the peer: kept for display, once per link, when the
    /// link negotiated [`NODE_INFO_CAP`] and the text looks like a version
    /// (1..=[`MAX_NODE_VERSION_BYTES`] of `[0-9A-Za-z.+-]`).
    pub(super) fn on_node_info(&mut self, link_id: u64, info: NodeInfo) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        let negotiated = entry.handle.peer().caps.iter().any(|c| c == NODE_INFO_CAP);
        if negotiated && entry.peer_version.is_none() && plausible_version(&info.version) {
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
    fn peer_rows(&self) -> Vec<PeerStatus> {
        let mut rows = Vec::new();
        for (node, ids) in &self.by_node {
            let entries: Vec<&LinkEntry> = ids.iter().filter_map(|id| self.links.get(id)).collect();
            let Some(entry) = entries
                .iter()
                .find(|l| l.mode == LinkMode::Active && !l.closing)
                .or_else(|| entries.first())
            else {
                continue;
            };
            let (tx, rx) = self.link_positions(entry);
            let pair = |d: Direction| (clamp(d.held), clamp(d.head));
            rows.push(PeerStatus {
                node: *node,
                version: entry.peer_version.clone(),
                boot: entry.handle.peer().boot,
                tx: tx.map(pair),
                rx: rx.map(pair),
                app_seen_at: entry.app_rx_wall.map(clamp),
            });
        }
        rows
    }

    /// The rows worth writing: a node never written, or one whose version or
    /// boot changed, at once; changed positions at most every
    /// [`PEER_STATUS_FLUSH_MS`]; app contact once it advanced
    /// [`APP_SEEN_ADVANCE_MS`].
    fn due_peer_rows(&self, routine: bool) -> Vec<PeerStatus> {
        self.peer_rows()
            .into_iter()
            .filter(|row| match self.peer_flushed.get(&row.node) {
                None => true,
                Some(old) => {
                    let urgent = old.version != row.version || old.boot != row.boot;
                    let app_advanced = row.app_seen_at.is_some_and(|at| {
                        old.app_seen_at
                            .is_none_or(|was| at.saturating_sub(was) >= APP_SEEN_ADVANCE_MS)
                    });
                    urgent || (routine && (old.tx != row.tx || old.rx != row.rx || app_advanced))
                }
            })
            .collect()
    }

    /// One housekeeping step of the shadow: at most one write in flight,
    /// none during a failure backoff, and a write stuck for
    /// [`PEER_STATUS_STUCK_MS`] reported once.
    pub(super) fn flush_peer_status(&mut self) {
        let now = self.mono();
        if !self.peer_flushes.is_empty() {
            if let Some(started) = self.peer_flush.started
                && !self.peer_flush.stuck_logged
                && now.0.saturating_sub(started.0) >= PEER_STATUS_STUCK_MS
            {
                self.peer_flush.stuck_logged = true;
                (self.deps.log)(&format!(
                    "sync: peer status write still in flight after {} s",
                    PEER_STATUS_STUCK_MS / 1000
                ));
            }
            return;
        }
        if self.peer_flush.retry_at.is_some_and(|at| now.0 < at.0) {
            return;
        }
        let routine = self
            .peer_flushed_at
            .is_none_or(|at| now.0.saturating_sub(at.0) >= PEER_STATUS_FLUSH_MS);
        let rows = self.due_peer_rows(routine);
        if rows.is_empty() {
            return;
        }
        if routine {
            self.peer_flushed_at = Some(now);
        }
        self.peer_flush.started = Some(now);
        self.peer_flush.stuck_logged = false;
        let store = self.deps.sync_store.clone();
        self.peer_flushes.spawn_blocking(move || {
            let result = store.record_peer_status(&rows, &unbounded_budget());
            PeerFlushDone { rows, result }
        });
    }

    /// A joined write: success records the rows; a failure or a panicked
    /// write backs off [`PEER_STATUS_FLUSH_MS`] and is logged once per run of
    /// failures.
    pub(super) fn on_peer_status_joined(
        &mut self,
        joined: Result<PeerFlushDone, tokio::task::JoinError>,
    ) {
        self.peer_flush.started = None;
        let failure = match joined {
            Ok(PeerFlushDone {
                rows,
                result: Ok(()),
            }) => {
                for row in rows {
                    self.peer_flushed.insert(row.node, row);
                }
                if std::mem::take(&mut self.peer_flush.failing) {
                    (self.deps.log)("sync: peer status writes recovered");
                }
                self.peer_flush.retry_at = None;
                return;
            }
            Ok(PeerFlushDone { result: Err(e), .. }) => format!("{:?}: {:?}", e.code, e.detail),
            Err(e) => format!("write task failed: {e}"),
        };
        let now = self.mono();
        self.peer_flush.retry_at = Some(MonoInstant(now.0.saturating_add(PEER_STATUS_FLUSH_MS)));
        if !std::mem::replace(&mut self.peer_flush.failing, true) {
            (self.deps.log)(&format!(
                "sync: peer status flush failed: {failure}; retrying every {} s",
                PEER_STATUS_FLUSH_MS / 1000
            ));
        }
    }

    /// Best effort at shutdown, while the links still exist: finish the write
    /// in flight, then write every row that differs from the store. Each step
    /// is bounded by [`CLOSE_WRITE_TIMEOUT`].
    pub(super) async fn final_peer_status_flush(&mut self) {
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.peer_flushes.join_next().await {
                self.on_peer_status_joined(joined);
            }
        })
        .await;
        let rows: Vec<PeerStatus> = self
            .peer_rows()
            .into_iter()
            .filter(|row| self.peer_flushed.get(&row.node) != Some(row))
            .collect();
        if rows.is_empty() {
            return;
        }
        let store = self.deps.sync_store.clone();
        let call = tokio::task::spawn_blocking(move || {
            let result = store.record_peer_status(&rows, &unbounded_budget());
            PeerFlushDone { rows, result }
        });
        if let Ok(joined) = timeout(CLOSE_WRITE_TIMEOUT, call).await {
            self.on_peer_status_joined(joined);
        }
    }
}

/// A daemon version as `node_info` should carry one: non-empty, at most
/// [`MAX_NODE_VERSION_BYTES`], only `[0-9A-Za-z.+-]`.
fn plausible_version(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NODE_VERSION_BYTES
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}
