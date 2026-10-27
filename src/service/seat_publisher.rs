//! Publishes this node's own seats on `seat_status` (ht-xg88 phase 2).
//!
//! Responsibility (SeatPublisher)
//! - Owns: the periodic build of this node's advisory seat snapshot
//!   (presence, harness, last activity, workspace/tab/pane labels) and the
//!   decision when to hand it to the sync coordinator.
//! - Hides: the presence words (`busy` = connected and mid-turn or asking,
//!   `idle` = connected otherwise, `offline` = no connected binding), the
//!   label match (the seat's target and structural terminal, and its
//!   incarnation where the host reports one), and the cadence.
//! - Must not: run while no linked peer takes `seat.status.v1`, touch the
//!   observation lane, or decide anything about a seat: the snapshot is a
//!   display hint for other nodes (TRUST-POLICY C1, N5).
//! - Role: one task, spawned by the daemon after the coordinator and ended
//!   by the daemon's stop token.
//!
//! Core and harness-neutral: harness ids are passed through as opaque text.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::ports::HostId;
use crate::protocol::results::ApiError;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::service::delivery_pull::DeliveryPullState;
use crate::service::host_sessions::HostSessions;
use crate::store::SqliteStore;
use crate::store::local_seat_status::{LocalSeat, local_seats};
use crate::sync::coordinator::SyncHandle;
use crate::sync::frame::SeatStatusEntry;

/// How often the snapshot is rebuilt from the store (a change is published
/// within this).
pub const CHECK_EVERY: Duration = Duration::from_secs(5);
/// Labels are read from the hosts at most this often, and an unchanged
/// snapshot is published again this often (it keeps receivers fresh: the
/// freshness bound is three times this).
pub const REFRESH_EVERY: Duration = Duration::from_secs(60);
/// Budget of one store read, and of one label read across every host (ms).
const CALL_BUDGET_MS: u64 = 3_000;

pub struct SeatPublisherDeps {
    pub store: Arc<SqliteStore>,
    pub sessions: Arc<HostSessions>,
    pub contacts: Arc<DeliveryPullState>,
    pub sync: SyncHandle,
    pub clock: Arc<dyn Clock>,
    pub cancel: Cancellation,
    /// [`CHECK_EVERY`] and [`REFRESH_EVERY`] in production; tests shorten
    /// them.
    pub check_every: Duration,
    pub refresh_every: Duration,
    /// One daemon.log line per run of read failures.
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// Labels of one host target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Labels {
    terminal: String,
    incarnation: Option<String>,
    workspace: String,
    tab: String,
    pane: String,
}

/// The advisory entry of `seat`, given its target's current labels.
fn entry_of(
    seat: &LocalSeat,
    labels: Option<&Labels>,
    now_ms: i64,
    contact: Option<i64>,
) -> SeatStatusEntry {
    let presence = if !seat.connected {
        "offline"
    } else if seat
        .ui_state
        .as_deref()
        .is_some_and(crate::store::effective::turn_in_progress)
    {
        "busy"
    } else {
        "idle"
    };
    let labels = labels.filter(|l| {
        seat.terminal.as_deref() == Some(l.terminal.as_str())
            && (l.incarnation.is_none() || l.incarnation == seat.incarnation)
    });
    let last = contact.max(seat.last_transition_at);
    SeatStatusEntry {
        seat: seat.seat.as_str().to_owned(),
        presence: presence.to_owned(),
        harness: seat.harness.clone(),
        activity_age_ms: last.map(|at| u64::try_from(now_ms.saturating_sub(at)).unwrap_or(0)),
        workspace: labels.map(|l| l.workspace.clone()),
        tab: labels.map(|l| l.tab.clone()),
        pane: labels.map(|l| l.pane.clone()),
    }
}

/// Spawns the publisher on the current tokio runtime.
pub fn spawn(deps: SeatPublisherDeps) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run(deps))
}

/// Whether two snapshots differ in anything but their ages (ages always
/// move).
fn same_but_ages(a: &[SeatStatusEntry], b: &[SeatStatusEntry]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.seat == b.seat
                && a.presence == b.presence
                && a.harness == b.harness
                && a.workspace == b.workspace
                && a.tab == b.tab
                && a.pane == b.pane
        })
}

async fn run(deps: SeatPublisherDeps) {
    let deps = Arc::new(deps);
    let pacer = crate::service::pacer::Pacer::new(
        "seat_publisher",
        Arc::clone(&deps.clock),
        deps.cancel.clone(),
    );
    let mut labels: HashMap<(HostId, String), Labels> = HashMap::new();
    let mut labels_at: Option<MonoInstant> = None;
    let mut published: Option<(Vec<SeatStatusEntry>, MonoInstant)> = None;
    let mut failing = false;
    let refresh_ms = u64::try_from(deps.refresh_every.as_millis()).unwrap_or(u64::MAX);
    loop {
        if deps.sync.seat_status_wanted() {
            let now = deps.clock.monotonic_now();
            let due = |at: Option<MonoInstant>| {
                at.is_none_or(|at| now.0.saturating_sub(at.0) >= refresh_ms)
            };
            if due(labels_at) {
                let read = Arc::clone(&deps);
                let held = labels.clone();
                if let Ok(fresh) =
                    tokio::task::spawn_blocking(move || read_labels(&read, held)).await
                {
                    labels = fresh;
                    labels_at = Some(now);
                }
            }
            let read = Arc::clone(&deps);
            match tokio::task::spawn_blocking(move || read_seats(&read)).await {
                Ok(Ok(seats)) => {
                    if std::mem::take(&mut failing) {
                        (deps.log)("seat status: local seat reads recovered");
                    }
                    let now_ms = deps.clock.utc_now().0;
                    let entries: Vec<SeatStatusEntry> = seats
                        .iter()
                        .map(|seat| {
                            let key = seat.host.zip(seat.target.clone());
                            let contact = deps.contacts.last_contact(&seat.seat).map(|at| at.0);
                            entry_of(
                                seat,
                                key.as_ref().and_then(|k| labels.get(k)),
                                now_ms,
                                contact,
                            )
                        })
                        .collect();
                    let publish = match &published {
                        None => true,
                        Some((old, at)) => !same_but_ages(old, &entries) || due(Some(*at)),
                    };
                    // A publish the coordinator's queue refused is tried
                    // again on the next check, not at the next refresh.
                    if publish && deps.sync.publish_seat_status(entries.clone()) {
                        published = Some((entries, now));
                    }
                }
                failed => {
                    if !std::mem::replace(&mut failing, true) {
                        let why = match failed {
                            Ok(Err(e)) => format!("{:?}: {}", e.code, e.detail),
                            Err(e) => format!("read task failed: {e}"),
                            Ok(Ok(_)) => unreachable!("matched above"),
                        };
                        (deps.log)(&format!("seat status: local seat read failed: {why}"));
                    }
                }
            }
        } else {
            // Nobody listens: the next listener gets a fresh snapshot at once.
            published = None;
            labels_at = None;
        }
        if let crate::service::pacer::Wake::Cancelled = pacer.wait(deps.check_every).await {
            return;
        }
    }
}

fn budget(deps: &SeatPublisherDeps) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(deps.clock.monotonic_now().0.saturating_add(CALL_BUDGET_MS)),
        cancellation: deps.cancel.clone(),
    }
}

fn read_seats(deps: &SeatPublisherDeps) -> Result<Vec<LocalSeat>, ApiError> {
    deps.store
        .sync_read(&budget(deps), |conn, _| local_seats(conn))
}

/// Every attached host's labels, by `(host, target)`, within one budget for
/// all hosts. A host that does not answer keeps its labels from `held` (a
/// blip never strips its seats' labels until the next refresh).
fn read_labels(
    deps: &SeatPublisherDeps,
    held: HashMap<(HostId, String), Labels>,
) -> HashMap<(HostId, String), Labels> {
    let budget = budget(deps);
    let mut out = HashMap::new();
    for session in deps.sessions.attached() {
        let Ok(found) = session.port().seat_labels(&budget) else {
            let host = session.host_id();
            out.extend(
                held.iter()
                    .filter(|((h, _), _)| *h == host)
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
            continue;
        };
        for label in found {
            out.insert(
                (session.host_id(), label.target.as_str().to_owned()),
                Labels {
                    terminal: label.terminal,
                    incarnation: label.incarnation,
                    workspace: label.workspace_label.unwrap_or(label.workspace_id),
                    tab: label.tab_label.unwrap_or(label.tab_id),
                    pane: label
                        .pane_label
                        .unwrap_or_else(|| label.target.as_str().to_owned()),
                },
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ids::SeatId;

    fn seat(connected: bool, ui: Option<&str>) -> LocalSeat {
        LocalSeat {
            seat: SeatId::new("snSGbl096JZZ"),
            host: None,
            target: Some("w1:p1".into()),
            terminal: Some("t1".into()),
            incarnation: Some("i1".into()),
            harness: Some("claude".into()),
            connected,
            ui_state: ui.map(str::to_owned),
            last_transition_at: Some(1_000),
        }
    }

    fn labels(terminal: &str, incarnation: Option<&str>) -> Labels {
        Labels {
            terminal: terminal.into(),
            incarnation: incarnation.map(str::to_owned),
            workspace: "office".into(),
            tab: "main".into(),
            pane: "claude".into(),
        }
    }

    // Kills: a mid-turn seat shown idle, a disconnected one shown connected,
    // labels of another pane incarnation attached to the seat, and an
    // activity age that ignores the later of contact and binding change.
    #[test]
    fn entry_presence_labels_and_activity() {
        let busy = entry_of(&seat(true, Some("active_turn")), None, 10_000, None);
        assert_eq!(busy.presence, "busy");
        assert_eq!(busy.activity_age_ms, Some(9_000));
        assert_eq!(
            entry_of(&seat(true, Some("idle")), None, 0, None).presence,
            "idle"
        );
        assert_eq!(entry_of(&seat(true, None), None, 0, None).presence, "idle");
        assert_eq!(
            entry_of(&seat(false, Some("active_turn")), None, 0, None).presence,
            "offline"
        );
        let matched = entry_of(
            &seat(true, None),
            Some(&labels("t1", Some("i1"))),
            10_000,
            Some(4_000),
        );
        assert_eq!(
            (
                matched.workspace.as_deref(),
                matched.tab.as_deref(),
                matched.pane.as_deref()
            ),
            (Some("office"), Some("main"), Some("claude"))
        );
        assert_eq!(matched.activity_age_ms, Some(6_000));
        assert_eq!(
            entry_of(&seat(true, None), Some(&labels("t2", None)), 0, None).workspace,
            None
        );
        assert_eq!(
            entry_of(&seat(true, None), Some(&labels("t1", Some("i2"))), 0, None).workspace,
            None
        );
        assert!(
            entry_of(&seat(true, None), Some(&labels("t1", None)), 0, None)
                .workspace
                .is_some()
        );
    }
}
