//! The coordinator's half of the `peer events` history (2.8.0).
//!
//! Responsibility (link_events)
//! - Owns: the bounded queue of link up and down events and its write
//!   through `SyncStorePort::record_link_events`, at most one in flight, on
//!   the housekeeping tick and once more at shutdown.
//! - Hides: the queue bound (the oldest events are dropped past
//!   [`MAX_QUEUED`]) and that a failed write is dropped after one log line.
//! - Must not: change a link, rank, dial or rx decision; the history is
//!   diagnostic display data (N13). Dial outcomes before a link is up are
//!   recorded by `sync_dial_targets::record_transition`, not here.
//! - Role: fed by `on_admission` (up) and `on_ended` (down).
//!
//! Core and harness-neutral.

use super::*;
use crate::store::sync_link_events::{LinkEvent, LinkOrigin};

/// Events held for the next write; older ones are dropped beyond this.
const MAX_QUEUED: usize = 256;

/// A finished write.
pub(super) struct LinkEventsDone {
    result: Result<(), ApiError>,
}

/// The queue and its write in flight.
#[derive(Default)]
pub(super) struct LinkEventLog {
    pending: VecDeque<LinkEvent>,
    pub(super) writes: JoinSet<LinkEventsDone>,
    failure_logged: bool,
}

impl Actor {
    /// Queues one event for `link_id`'s peer `node`.
    pub(super) fn note_link_event(
        &mut self,
        kind: LinkEventKind,
        origin: &Origin,
        node: NodeUuid,
        detail: Option<String>,
    ) {
        let (origin, dest) = match origin {
            Origin::Inbound => (LinkOrigin::Inbound, None),
            Origin::Dialed { dest, .. } => (LinkOrigin::Dialed, Some(dest.clone())),
        };
        let log = &mut self.link_events;
        if log.pending.len() >= MAX_QUEUED {
            log.pending.pop_front();
        }
        log.pending.push_back(LinkEvent {
            at: self.deps.clock.utc_now().0,
            kind,
            origin,
            node: Some(node),
            dest,
            detail,
        });
    }

    /// Writes the queued events in one background call.
    pub(super) fn flush_link_events(&mut self) {
        let log = &mut self.link_events;
        if !log.writes.is_empty() || log.pending.is_empty() {
            return;
        }
        let events: Vec<LinkEvent> = log.pending.drain(..).collect();
        let store = self.deps.sync_store.clone();
        log.writes.spawn_blocking(move || LinkEventsDone {
            result: store.record_link_events(&events, &unbounded_budget()),
        });
    }

    pub(super) fn on_link_events_written(
        &mut self,
        joined: Result<LinkEventsDone, tokio::task::JoinError>,
    ) {
        let failure = match joined {
            Ok(LinkEventsDone { result: Ok(()) }) => {
                self.link_events.failure_logged = false;
                return;
            }
            Ok(LinkEventsDone { result: Err(e) }) => format!("{:?}: {:?}", e.code, e.detail),
            Err(e) => e.to_string(),
        };
        if !std::mem::replace(&mut self.link_events.failure_logged, true) {
            (self.deps.log)(&format!("sync: link event write failed: {failure}"));
        }
    }

    /// Best effort at shutdown: finish the write in flight, then write what
    /// is queued, each bounded.
    pub(super) async fn final_link_events_flush(&mut self) {
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.link_events.writes.join_next().await {
                self.on_link_events_written(joined);
            }
        })
        .await;
        self.flush_link_events();
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.link_events.writes.join_next().await {
                self.on_link_events_written(joined);
            }
        })
        .await;
    }
}
