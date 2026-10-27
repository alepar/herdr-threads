//! Seat-status gossip (ht-xg88 phase 2): this node's own seats, published on
//! `seat_status` frames, and every other node's latest snapshot, relayed and
//! persisted.
//!
//! Responsibility (seat_status)
//! - Owns: the own snapshot (one more `rev` per publish, refreshes
//!   included), the held snapshot per origin and its ordering (same boot: a
//!   higher `rev`; another boot: built more than 1 s later), each boot's
//!   build times per origin (clone suspicion), relaying to the other cap
//!   links, the frame bound per link, and the writes to `remote_seat_status`
//!   (newest per origin, one batch in flight, backoff after a failure).
//! - Hides: ages on the wire (never absolute times).
//! - Must not: decide anything about a seat; every value is a display hint
//!   from the seat's home node (TRUST-POLICY C1, N5, N13). The home-node
//!   guard on entries is the store's (`remote_seat_status::record`).
//! - Role: fed by `SyncHandle::publish_seat_status` (the daemon's
//!   `service::seat_publisher`) and by `on_frame`; ticked by housekeeping.
//!
//! Core and harness-neutral.

use super::*;
use crate::store::remote_seat_status::{RemoteSeat, RemoteSeatUpdate};
use crate::sync::frame::{MAX_SEAT_HARNESS_BYTES, MAX_SEAT_LABEL_BYTES, MAX_SEAT_STATUS_SEATS};

/// Another boot replaces the held snapshot only when built at least this
/// much later (ms), so wall-clock jitter between relays never flips boots.
const BOOT_MARGIN_MS: u64 = 1_000;
/// A boot whose newest build is older than this is forgotten (ms).
const BOOT_MEMORY_MS: u64 = crate::store::sync_contacts::UNREACHABLE_AFTER_MS;
/// After a failed write, the next one waits this long (ms).
const WRITE_RETRY_MS: u64 = 10_000;
/// A write still in flight after this long is reported once (ms).
const WRITE_STUCK_MS: u64 = 60_000;
/// Longest seat id and presence word kept, in bytes.
const MAX_SEAT_ID_BYTES: usize = 64;
const MAX_PRESENCE_BYTES: usize = 16;

/// One snapshot, this node's own or one held for another origin.
struct Snapshot {
    boot: Uuid,
    rev: u64,
    built_wall: u64,
    version: String,
    seats: Vec<SeatStatusEntry>,
}

/// Build times seen of one boot of an origin.
#[derive(Clone, Copy)]
struct BootSeen {
    first_built: u64,
    last_built: u64,
}

/// A batch of writes joined from the `JoinSet`.
pub(super) struct SeatWriteDone {
    updates: Vec<RemoteSeatUpdate>,
    result: Result<(), ApiError>,
}

pub(super) struct SeatStatusState {
    wanted: Arc<AtomicBool>,
    own: Option<Snapshot>,
    held: BTreeMap<NodeUuid, Snapshot>,
    boots: BTreeMap<NodeUuid, BTreeMap<Uuid, BootSeen>>,
    /// The newest write per origin not yet handed to the store.
    pending: BTreeMap<NodeUuid, RemoteSeatUpdate>,
    pub(super) writes: JoinSet<SeatWriteDone>,
    write_started: Option<MonoInstant>,
    stuck_logged: bool,
    retry_at: Option<MonoInstant>,
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
            write_started: None,
            stuck_logged: false,
            retry_at: None,
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
/// [`MAX_SEAT_STATUS_SEATS`] with a usable seat id, labels bounded, an empty
/// presence as `-`, a harness id too long to be one dropped (never cut into
/// another id), empty labels absent.
fn sanitize(seats: Vec<SeatStatusEntry>) -> Vec<SeatStatusEntry> {
    let label = |text: Option<String>| {
        text.map(|t| bounded(&t, MAX_SEAT_LABEL_BYTES))
            .filter(|t| !t.is_empty())
    };
    seats
        .into_iter()
        .filter(|entry| !entry.seat.is_empty() && entry.seat.len() <= MAX_SEAT_ID_BYTES)
        .take(MAX_SEAT_STATUS_SEATS)
        .map(|entry| SeatStatusEntry {
            presence: match bounded(&entry.presence, MAX_PRESENCE_BYTES) {
                presence if presence.is_empty() => "-".to_owned(),
                presence => presence,
            },
            harness: entry
                .harness
                .filter(|h| !h.is_empty() && h.len() <= MAX_SEAT_HARNESS_BYTES),
            workspace: label(entry.workspace),
            tab: label(entry.tab),
            pane: label(entry.pane),
            ..entry
        })
        .collect()
}

/// Which held snapshot an incoming one of `boot`/`rev`, built at `built`,
/// leaves: `true` when it replaces the held one.
fn newer(held: Option<&Snapshot>, boot: Uuid, rev: u64, built: u64) -> bool {
    match held {
        None => true,
        Some(held) if held.boot == boot => rev > held.rev,
        Some(held) => built > held.built_wall.saturating_add(BOOT_MARGIN_MS),
    }
}

/// The last build of a boot other than `current` that was built after
/// `current` started: two boots alive at once (a clone, N8), never a plain
/// restart, whose old boot's builds all precede the new boot's first.
fn other_live_boot(boots: &BTreeMap<Uuid, BootSeen>, current: Uuid) -> Option<u64> {
    let started = boots.get(&current)?.first_built;
    boots
        .iter()
        .filter(|(boot, seen)| **boot != current && seen.last_built > started)
        .map(|(_, seen)| seen.last_built)
        .max()
}

impl LinkEntry {
    fn seat_status_cap(&self) -> bool {
        self.handle
            .peer()
            .caps
            .iter()
            .any(|cap| cap == SEAT_STATUS_CAP)
    }
}

impl Actor {
    /// A link that takes `seat_status` frames now: active, ranked, open,
    /// announced and with the capability.
    fn seat_ready(entry: &LinkEntry) -> bool {
        entry.mode == LinkMode::Active
            && entry.ranked
            && !entry.closing
            && entry.announced
            && entry.seat_status_cap()
    }

    /// Send `frame` on `link_id`, dropping seats from the end until it fits
    /// the link's negotiated frame size.
    fn send_seat_frame(&self, link_id: u64, mut frame: SeatStatusFrame) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        let max = entry.handle.peer().limits.max_frame as usize;
        loop {
            let size = PeerFrame::SeatStatus(frame.clone())
                .encode()
                .map_or(usize::MAX, |bytes| bytes.len());
            if size <= max || frame.seats.is_empty() {
                break;
            }
            // Shrink to the share that fits, at least one seat less.
            let len = frame.seats.len();
            let keep = (len * max / size.max(1)).min(len - 1);
            frame.seats.truncate(keep);
        }
        let _ = entry.handle.send(PeerFrame::SeatStatus(frame));
    }

    fn frame_of(origin: NodeUuid, snapshot: &Snapshot, now: u64) -> SeatStatusFrame {
        SeatStatusFrame {
            origin,
            boot: snapshot.boot,
            rev: snapshot.rev,
            age_ms: now.saturating_sub(snapshot.built_wall),
            version: snapshot.version.clone(),
            seats: snapshot.seats.clone(),
        }
    }

    /// Ready links other than `except` and not of `origin`'s own node.
    fn seat_targets(&self, except: Option<u64>, origin: NodeUuid) -> Vec<u64> {
        self.links
            .iter()
            .filter(|(id, l)| {
                Some(**id) != except
                    && l.handle.peer().node != origin
                    && Self::seat_ready(l)
                    && l.seat_status_sent
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// The publisher's view of this node's seats: the next `rev`, built now,
    /// to every ready link.
    pub(super) fn on_publish_seats(&mut self, seats: Vec<SeatStatusEntry>) {
        let now = self.wall();
        let rev = self.seats.own.as_ref().map_or(1, |own| own.rev + 1);
        let own = Snapshot {
            boot: self.deps.boot,
            rev,
            built_wall: now,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            seats: sanitize(seats),
        };
        let me = self.deps.identity.uuid();
        let frame = Self::frame_of(me, &own, now);
        self.seats.own = Some(own);
        for id in self.seat_targets(None, me) {
            self.send_seat_frame(id, frame.clone());
        }
    }

    /// A `seat_status` from `link_id`: kept when it replaces the held one of
    /// its origin, then relayed to the other ready links and queued for the
    /// store; dropped otherwise (and never relayed). Only origins this node
    /// knows (a log it holds, or contact with it, direct or relayed by
    /// `seen`) are taken, so a peer cannot grow the held set with made-up
    /// nodes.
    pub(super) fn on_seat_status(&mut self, link_id: u64, frame: SeatStatusFrame) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        let origin = frame.origin;
        if !entry.seat_status_cap()
            || origin == self.deps.identity.uuid()
            || (self.contacts.last_contact(&origin).is_none()
                && !self.local.0.keys().any(|log| log.node == origin))
        {
            return;
        }
        let now = self.wall();
        let built = now.saturating_sub(frame.age_ms);
        // Build times, not arrival times: a stale copy of an old boot that a
        // relay sends late moves nothing forward.
        let boots = self.seats.boots.entry(origin).or_default();
        let seen = boots.entry(frame.boot).or_insert(BootSeen {
            first_built: built,
            last_built: built,
        });
        seen.first_built = seen.first_built.min(built);
        seen.last_built = seen.last_built.max(built);
        boots.retain(|_, seen| now.saturating_sub(seen.last_built) <= BOOT_MEMORY_MS);
        if !newer(self.seats.held.get(&origin), frame.boot, frame.rev, built) {
            return;
        }
        let snapshot = Snapshot {
            boot: frame.boot,
            rev: frame.rev,
            built_wall: built,
            version: if frame.version.len() <= MAX_NODE_VERSION_BYTES {
                frame.version
            } else {
                String::new()
            },
            seats: sanitize(frame.seats),
        };
        let other_boot_at = self
            .seats
            .boots
            .get(&origin)
            .and_then(|boots| other_live_boot(boots, snapshot.boot));
        self.seats.pending.insert(
            origin,
            RemoteSeatUpdate {
                origin,
                boot: snapshot.boot,
                rev: snapshot.rev,
                built_at: built,
                version: (!snapshot.version.is_empty()).then(|| snapshot.version.clone()),
                other_boot_at,
                seats: snapshot
                    .seats
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
                    .collect(),
            },
        );
        let relay = Self::frame_of(origin, &snapshot, now);
        self.seats.held.insert(origin, snapshot);
        for id in self.seat_targets(Some(link_id), origin) {
            self.send_seat_frame(id, relay.clone());
        }
    }

    /// Housekeeping: keep [`SyncHandle::seat_status_wanted`] current, give a
    /// newly ready link every snapshot this node holds, and write what is
    /// pending.
    pub(super) fn seat_status_tick(&mut self) {
        let wanted = self.links.values().any(Self::seat_ready);
        self.seats.wanted.store(wanted, Ordering::Release);
        let now = self.wall();
        let me = self.deps.identity.uuid();
        let fresh: Vec<u64> = self
            .links
            .iter()
            .filter(|(_, l)| Self::seat_ready(l) && !l.seat_status_sent)
            .map(|(id, _)| *id)
            .collect();
        for id in fresh {
            let peer = self.links[&id].handle.peer().node;
            let mut frames: Vec<SeatStatusFrame> = self
                .seats
                .own
                .iter()
                .map(|own| Self::frame_of(me, own, now))
                .collect();
            frames.extend(
                self.seats
                    .held
                    .iter()
                    .filter(|(origin, _)| **origin != peer)
                    .map(|(origin, held)| Self::frame_of(*origin, held, now)),
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
        let now = self.mono();
        if !self.seats.writes.is_empty() {
            if let Some(started) = self.seats.write_started
                && !self.seats.stuck_logged
                && now.0.saturating_sub(started.0) >= WRITE_STUCK_MS
            {
                self.seats.stuck_logged = true;
                (self.deps.log)(&format!(
                    "sync: seat status write still in flight after {} s",
                    WRITE_STUCK_MS / 1000
                ));
            }
            return;
        }
        if self.seats.pending.is_empty() || self.seats.retry_at.is_some_and(|at| now.0 < at.0) {
            return;
        }
        let updates: Vec<RemoteSeatUpdate> = std::mem::take(&mut self.seats.pending)
            .into_values()
            .collect();
        self.seats.write_started = Some(now);
        self.seats.stuck_logged = false;
        let store = self.deps.sync_store.clone();
        self.seats.writes.spawn_blocking(move || {
            let result = store.record_remote_seat_status(&updates, &unbounded_budget());
            SeatWriteDone { updates, result }
        });
    }

    /// A joined write. A failure puts its updates back unless a newer one of
    /// the same origin arrived meanwhile, waits [`WRITE_RETRY_MS`], and is
    /// logged once per run of failures (and once on recovery).
    pub(super) fn on_seat_status_written(
        &mut self,
        joined: Result<SeatWriteDone, tokio::task::JoinError>,
    ) {
        self.seats.write_started = None;
        let failure = match joined {
            Ok(SeatWriteDone { result: Ok(()), .. }) => {
                self.seats.retry_at = None;
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
        let now = self.mono();
        self.seats.retry_at = Some(MonoInstant(now.0.saturating_add(WRITE_RETRY_MS)));
        if !std::mem::replace(&mut self.seats.failing, true) {
            (self.deps.log)(&format!(
                "sync: seat status write failed: {failure}; retrying every {} s",
                WRITE_RETRY_MS / 1000
            ));
        }
    }

    /// Best effort at shutdown: finish the write in flight, then write what
    /// is pending, each step bounded by [`CLOSE_WRITE_TIMEOUT`].
    pub(super) async fn final_seat_status_flush(&mut self) {
        self.join_seat_writes().await;
        self.seats.retry_at = None;
        self.write_seat_status();
        self.join_seat_writes().await;
    }

    async fn join_seat_writes(&mut self) {
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.seats.writes.join_next().await {
                self.on_seat_status_written(joined);
            }
        })
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boots(entries: &[(u128, u64, u64)]) -> BTreeMap<Uuid, BootSeen> {
        entries
            .iter()
            .map(|(boot, first, last)| {
                (
                    Uuid::from_u128(*boot),
                    BootSeen {
                        first_built: *first,
                        last_built: *last,
                    },
                )
            })
            .collect()
    }

    // Kills: a plain restart (the old boot's builds all before the new
    // boot's first) read as a clone, and two boots alive at once not read
    // as one.
    #[test]
    fn only_overlapping_boots_are_a_clone() {
        let restart = boots(&[(1, 0, 50_000), (2, 60_000, 90_000)]);
        assert_eq!(other_live_boot(&restart, Uuid::from_u128(2)), None);
        let clone = boots(&[(1, 0, 95_000), (2, 60_000, 90_000)]);
        assert_eq!(other_live_boot(&clone, Uuid::from_u128(2)), Some(95_000));
    }

    // Kills: an empty presence stored (it fails the column CHECK and the
    // whole batch), a long harness id cut into another id, and labels left
    // over their bound.
    #[test]
    fn sanitize_keeps_entries_storable() {
        let entry = SeatStatusEntry {
            seat: "sQ0VnTDasAbY".into(),
            presence: String::new(),
            harness: Some("h".repeat(17)),
            activity_age_ms: None,
            workspace: Some("é".repeat(40)),
            tab: Some(String::new()),
            pane: None,
        };
        let out = sanitize(vec![
            entry.clone(),
            SeatStatusEntry {
                seat: String::new(),
                ..entry
            },
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].presence, "-");
        assert_eq!(out[0].harness, None);
        assert!(out[0].workspace.as_ref().unwrap().len() <= MAX_SEAT_LABEL_BYTES);
        assert_eq!(out[0].tab, None);
    }

    // Kills: a same-boot copy with an equal or lower rev accepted (a relay
    // loop would circulate it), and a boot switch accepted on an older or
    // jitter-close build.
    #[test]
    fn newer_orders_by_rev_within_a_boot_and_build_across() {
        let held = Snapshot {
            boot: Uuid::from_u128(1),
            rev: 5,
            built_wall: 10_000,
            version: String::new(),
            seats: Vec::new(),
        };
        let one = Uuid::from_u128(1);
        let two = Uuid::from_u128(2);
        assert!(newer(None, one, 1, 0));
        assert!(newer(Some(&held), one, 6, 0));
        assert!(!newer(Some(&held), one, 5, 99_000));
        assert!(!newer(Some(&held), one, 4, 99_000));
        assert!(!newer(Some(&held), two, 1, 10_500));
        assert!(newer(Some(&held), two, 1, 11_001));
    }
}
