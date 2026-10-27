//! Seat-status gossip (ht-xg88 phase 2): this node's own seats, published on
//! `seat_status` frames, and every other node's latest snapshot, relayed and
//! persisted.
//!
//! Responsibility (seat_status)
//! - Owns: the own snapshot (`rev` per boot, bumped on each change), the
//!   held snapshot per origin and its ordering (newest-built wins across
//!   boots; `(boot, rev)` only deduplicates within one), the boots seen per
//!   origin (clone suspicion), relaying to the other cap links, the frame
//!   bound per link, and the coalesced writes to `remote_seat_status`.
//! - Hides: ages on the wire (never absolute times), and that a refresh with
//!   the same content only moves `built_at`.
//! - Must not: decide anything about a seat; every value is a display hint
//!   from the seat's home node (TRUST-POLICY C1, N5). The home-node guard on
//!   entries is the store's (`remote_seat_status::record`).
//! - Role: fed by `SyncHandle::publish_seat_status` (the daemon's
//!   `service::seat_publisher`) and by `on_frame`; ticked by housekeeping.
//!
//! Core and harness-neutral.

use super::*;
use crate::store::remote_seat_status::{RemoteSeat, RemoteSeatUpdate};
use crate::sync::frame::{MAX_SEAT_HARNESS_BYTES, MAX_SEAT_LABEL_BYTES, MAX_SEAT_STATUS_SEATS};

/// A relayed snapshot counts as a newer build only when it is at least this
/// much newer (ms): ages accumulate in whole milliseconds per hop, so a
/// snapshot coming back round a loop never looks newer.
const REFRESH_MARGIN_MS: u64 = 1_000;
/// Boots of one origin are remembered this long after their last snapshot.
const BOOT_MEMORY_MS: u64 = crate::store::sync_contacts::UNREACHABLE_AFTER_MS;

struct Own {
    rev: u64,
    built_wall: u64,
    seats: Vec<SeatStatusEntry>,
}

struct Held {
    boot: Uuid,
    rev: u64,
    built_wall: u64,
    version: String,
    seats: Vec<SeatStatusEntry>,
}

/// A batch of writes joined from the `JoinSet`.
pub(super) struct SeatWriteDone {
    updates: Vec<RemoteSeatUpdate>,
    result: Result<(), ApiError>,
}

pub(super) struct SeatStatusState {
    wanted: Arc<AtomicBool>,
    own: Option<Own>,
    held: BTreeMap<NodeUuid, Held>,
    /// Per origin, the last wall time each boot delivered a snapshot.
    boots: BTreeMap<NodeUuid, BTreeMap<Uuid, u64>>,
    /// Writes not yet handed to the store, one per origin (a newer one
    /// supersedes; a refresh keeps the pending content).
    pending: BTreeMap<NodeUuid, RemoteSeatUpdate>,
    pub(super) writes: JoinSet<SeatWriteDone>,
    failing: bool,
}

impl SeatStatusState {
    pub(super) fn new(wanted: Arc<AtomicBool>) -> Self {
        Self {
            wanted,
            own: None,
            held: BTreeMap::new(),
            boots: BTreeMap::new(),
            pending: BTreeMap::new(),
            writes: JoinSet::new(),
            failing: false,
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

/// The entries as this node keeps and sends them: at most
/// [`MAX_SEAT_STATUS_SEATS`], every text field bounded, empty labels absent.
fn sanitize(seats: Vec<SeatStatusEntry>) -> Vec<SeatStatusEntry> {
    let label =
        |text: Option<String>, max: usize| text.map(|t| bounded(&t, max)).filter(|t| !t.is_empty());
    seats
        .into_iter()
        .filter(|entry| !entry.seat.is_empty() && entry.seat.len() <= 64)
        .take(MAX_SEAT_STATUS_SEATS)
        .map(|entry| SeatStatusEntry {
            presence: bounded(&entry.presence, 16),
            harness: label(entry.harness, MAX_SEAT_HARNESS_BYTES),
            workspace: label(entry.workspace, MAX_SEAT_LABEL_BYTES),
            tab: label(entry.tab, MAX_SEAT_LABEL_BYTES),
            pane: label(entry.pane, MAX_SEAT_LABEL_BYTES),
            ..entry
        })
        .collect()
}

impl Actor {
    /// A link that takes `seat_status` frames now: active, ranked, open,
    /// announced and with the capability.
    fn seat_ready(entry: &LinkEntry) -> bool {
        entry.mode == LinkMode::Active
            && entry.ranked
            && !entry.closing
            && entry.announced
            && entry
                .handle
                .peer()
                .caps
                .iter()
                .any(|c| c == SEAT_STATUS_CAP)
    }

    /// Send `frame` on `link_id`, dropping seats from the end until it fits
    /// the link's negotiated frame size.
    fn send_seat_frame(&self, link_id: u64, mut frame: SeatStatusFrame) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        let max = entry.handle.peer().limits.max_frame as usize;
        loop {
            let frame_ref = PeerFrame::SeatStatus(frame);
            let size = frame_ref.encode().map_or(usize::MAX, |bytes| bytes.len());
            let PeerFrame::SeatStatus(back) = frame_ref else {
                unreachable!("built above");
            };
            frame = back;
            if size <= max || frame.seats.is_empty() {
                break;
            }
            let keep = frame.seats.len() / 2;
            frame.seats.truncate(keep);
        }
        let _ = entry.handle.send(PeerFrame::SeatStatus(frame));
    }

    fn own_frame(&self, now: u64) -> Option<SeatStatusFrame> {
        let own = self.seats.own.as_ref()?;
        Some(SeatStatusFrame {
            origin: self.deps.identity.uuid(),
            boot: self.deps.boot,
            rev: own.rev,
            age_ms: now.saturating_sub(own.built_wall),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            seats: own.seats.clone(),
        })
    }

    fn held_frame(origin: NodeUuid, held: &Held, now: u64) -> SeatStatusFrame {
        SeatStatusFrame {
            origin,
            boot: held.boot,
            rev: held.rev,
            age_ms: now.saturating_sub(held.built_wall),
            version: held.version.clone(),
            seats: held.seats.clone(),
        }
    }

    /// The publisher's latest view of this node's seats: a changed one gets
    /// the next `rev`; either way it is built now and goes to every ready
    /// link (the publisher calls at least every refresh period, which keeps
    /// the receivers' copies fresh).
    pub(super) fn on_publish_seats(&mut self, seats: Vec<SeatStatusEntry>) {
        let seats = sanitize(seats);
        let now = self.wall();
        let rev = match &self.seats.own {
            Some(own) if own.seats == seats => own.rev,
            Some(own) => own.rev + 1,
            None => 1,
        };
        self.seats.own = Some(Own {
            rev,
            built_wall: now,
            seats,
        });
        let Some(frame) = self.own_frame(now) else {
            return;
        };
        let ids: Vec<u64> = self
            .links
            .iter()
            .filter(|(_, l)| Self::seat_ready(l) && l.seat_status_sent)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.send_seat_frame(id, frame.clone());
        }
    }

    /// A `seat_status` from `link_id`: kept when newer than the held one of
    /// its origin, then relayed to the other ready links and queued for the
    /// store; dropped otherwise (and never relayed).
    pub(super) fn on_seat_status(&mut self, link_id: u64, frame: SeatStatusFrame) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        if !entry
            .handle
            .peer()
            .caps
            .iter()
            .any(|c| c == SEAT_STATUS_CAP)
            || frame.origin == self.deps.identity.uuid()
        {
            return;
        }
        let now = self.wall();
        let origin = frame.origin;
        let built = now.saturating_sub(frame.age_ms);
        let boots = self.seats.boots.entry(origin).or_default();
        boots.insert(frame.boot, now);
        boots.retain(|_, at| now.saturating_sub(*at) <= BOOT_MEMORY_MS);
        let version = if frame.version.len() <= MAX_NODE_VERSION_BYTES {
            frame.version
        } else {
            String::new()
        };
        let seats = sanitize(frame.seats);
        // `Some(true)`: new content; `Some(false)`: same content, newer build.
        let accepted = match self.seats.held.get(&origin) {
            None => Some(true),
            Some(held) if held.boot == frame.boot && frame.rev > held.rev => Some(true),
            Some(held) if held.boot == frame.boot && frame.rev == held.rev => {
                (built > held.built_wall + REFRESH_MARGIN_MS).then_some(false)
            }
            Some(held) if held.boot == frame.boot => None,
            Some(held) => (built > held.built_wall + REFRESH_MARGIN_MS).then_some(true),
        };
        let Some(content) = accepted else {
            return;
        };
        let held = Held {
            boot: frame.boot,
            rev: frame.rev,
            built_wall: built,
            version,
            seats,
        };
        let other_boot_at = self.seats.boots.get(&origin).and_then(|boots| {
            boots
                .iter()
                .filter(|(boot, _)| **boot != held.boot)
                .map(|(_, at)| *at)
                .max()
        });
        let update = RemoteSeatUpdate {
            origin,
            boot: held.boot,
            rev: held.rev,
            built_at: built,
            version: (!held.version.is_empty() && content).then(|| held.version.clone()),
            other_boot_at,
            seats: content.then(|| {
                held.seats
                    .iter()
                    .map(|entry| RemoteSeat {
                        seat: entry.seat.clone(),
                        presence: entry.presence.clone(),
                        harness: entry.harness.clone(),
                        activity_at: entry.activity_age_ms.map(|age| built.saturating_sub(age)),
                        workspace: entry.workspace.clone(),
                        tab: entry.tab.clone(),
                        pane: entry.pane.clone(),
                    })
                    .collect()
            }),
        };
        match self.seats.pending.get_mut(&origin) {
            // A refresh of content not yet written keeps that content.
            Some(pending) if update.seats.is_none() && pending.rev == update.rev => {
                pending.built_at = update.built_at;
                pending.other_boot_at = update.other_boot_at;
            }
            _ => {
                self.seats.pending.insert(origin, update);
            }
        }
        let relay = Self::held_frame(origin, &held, now);
        self.seats.held.insert(origin, held);
        let ids: Vec<u64> = self
            .links
            .iter()
            .filter(|(id, l)| {
                **id != link_id
                    && l.handle.peer().node != origin
                    && Self::seat_ready(l)
                    && l.seat_status_sent
            })
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.send_seat_frame(id, relay.clone());
        }
    }

    /// Housekeeping: keep [`SyncHandle::seat_status_wanted`] current, give a
    /// newly ready link every snapshot this node holds, and write what is
    /// pending (one batch in flight).
    pub(super) fn seat_status_tick(&mut self) {
        let wanted = self.links.values().any(Self::seat_ready);
        self.seats.wanted.store(wanted, Ordering::Release);
        let now = self.wall();
        let fresh: Vec<u64> = self
            .links
            .iter()
            .filter(|(_, l)| Self::seat_ready(l) && !l.seat_status_sent)
            .map(|(id, _)| *id)
            .collect();
        for id in fresh {
            let peer = self.links[&id].handle.peer().node;
            let mut frames: Vec<SeatStatusFrame> = self.own_frame(now).into_iter().collect();
            frames.extend(
                self.seats
                    .held
                    .iter()
                    .filter(|(origin, _)| **origin != peer)
                    .map(|(origin, held)| Self::held_frame(*origin, held, now)),
            );
            for frame in frames {
                self.send_seat_frame(id, frame);
            }
            if let Some(entry) = self.links.get_mut(&id) {
                entry.seat_status_sent = true;
            }
        }
        self.write_seat_status();
    }

    fn write_seat_status(&mut self) {
        if !self.seats.writes.is_empty() || self.seats.pending.is_empty() {
            return;
        }
        let updates: Vec<RemoteSeatUpdate> = std::mem::take(&mut self.seats.pending)
            .into_values()
            .collect();
        let store = self.deps.sync_store.clone();
        self.seats.writes.spawn_blocking(move || {
            let result = store.record_remote_seat_status(&updates, &unbounded_budget());
            SeatWriteDone { updates, result }
        });
    }

    /// A joined write: a failure puts its updates back (unless newer ones
    /// arrived meanwhile) and is logged once per run of failures.
    pub(super) fn on_seat_status_written(
        &mut self,
        joined: Result<SeatWriteDone, tokio::task::JoinError>,
    ) {
        let failure = match joined {
            Ok(SeatWriteDone { result: Ok(()), .. }) => {
                if std::mem::take(&mut self.seats.failing) {
                    (self.deps.log)("sync: seat status writes recovered");
                }
                return;
            }
            Ok(SeatWriteDone {
                updates,
                result: Err(e),
            }) => {
                for update in updates {
                    self.seats.pending.entry(update.origin).or_insert(update);
                }
                format!("{:?}: {:?}", e.code, e.detail)
            }
            Err(e) => format!("write task failed: {e}"),
        };
        if !std::mem::replace(&mut self.seats.failing, true) {
            (self.deps.log)(&format!("sync: seat status write failed: {failure}"));
        }
    }

    /// Best effort at shutdown: finish the write in flight, then write what
    /// is pending, each step bounded by [`CLOSE_WRITE_TIMEOUT`].
    pub(super) async fn final_seat_status_flush(&mut self) {
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.seats.writes.join_next().await {
                self.on_seat_status_written(joined);
            }
        })
        .await;
        self.write_seat_status();
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.seats.writes.join_next().await {
                self.on_seat_status_written(joined);
            }
        })
        .await;
    }
}
