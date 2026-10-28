//! Answers linked peers' `addr_resolve` against this node's own live view
//! (remote addresses, TRUST-POLICY N14).
//!
//! Responsibility (RemoteResolver)
//! - Owns: reading every attached host's topology within one budget, pooling
//!   the matches across hosts, and mapping the one matched pane to this
//!   node's resolved seat on it.
//! - Hides: which host a pane belongs to (the answer carries a seat ID).
//! - Must not: write, attribute, allocate or wake anything: it is a read on
//!   behalf of another node, which only learns a seat ID and then decides its
//!   own command (A2). It never consults remote seat-status labels (N13).
//! - Role: installed on the sync handle by the daemon; called by the
//!   coordinator on a blocking thread.
//!
//! Core and harness-neutral.

use std::sync::Arc;

use crate::host::observation::HostTopology;
use crate::ports::HostId;
use crate::protocol::address::{self, Narrowing, Outcome, Selector, Tiering};
use crate::protocol::results::ApiError;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::service::host_sessions::HostSessions;
use crate::store::SqliteStore;
use crate::sync::coordinator::remote_addr::AddrResolver;
use crate::sync::frame::{AddrCandidate, AddrOutcome, AddrTarget};

/// Budget of the topology reads across every host, and of the seat lookup
/// (ms).
const READ_BUDGET_MS: u64 = 3_000;

pub struct RemoteResolver {
    pub store: Arc<SqliteStore>,
    pub sessions: Arc<HostSessions>,
    pub clock: Arc<dyn Clock>,
}

/// One host's candidates for the request.
struct HostMatch {
    host: HostId,
    outcome: Outcome,
}

impl RemoteResolver {
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + READ_BUDGET_MS),
            cancellation: Cancellation::default(),
        }
    }

    /// The resolved local seat on `pane` of `host`: `Ok(None)` for none,
    /// `Err` for more than one.
    fn seat_on(&self, host: HostId, pane: &str) -> Result<Option<String>, ApiError> {
        let seats = self.store.sync_read(&self.budget(), |conn, _| {
            pane_seats(conn, host.get(), pane).map_err(crate::store::connection::store_error)
        })?;
        match seats.len() {
            0 => Ok(None),
            1 => Ok(seats.into_iter().next()),
            _ => Err(ApiError::new(
                crate::protocol::results::ErrorCode::Conflict,
                format!("pane {pane} maps to more than one seat"),
            )),
        }
    }

    fn outcome_of(target: &AddrTarget, topology: &HostTopology) -> Outcome {
        match target {
            AddrTarget::Seat { location } => Selector::new(location).resolve_with(
                topology,
                None,
                &Narrowing::default(),
                Tiering::Global,
            ),
            AddrTarget::Pane { space, tab, pane } => {
                address::resolve_parts(topology, space.as_deref(), tab.as_deref(), pane.as_deref())
            }
            AddrTarget::Unknown => Outcome::NoMatch,
        }
    }

    /// `SEAT_ID@M` (spec D2): an exact seat ID is this node's own seat, or not here at all.
    fn exact_seat(&self, location: &str) -> Option<AddrOutcome> {
        let seat = crate::protocol::ids::SeatId::parse(location).ok()?;
        match self.store.sync_read(&self.budget(), |conn, _| {
            seat_row(conn, seat.as_str()).map_err(crate::store::connection::store_error)
        }) {
            Ok(row) => exact_outcome(seat.as_str(), row),
            Err(error) => Some(AddrOutcome::Unavailable {
                reason: format!("seat lookup failed: {}", error.detail),
            }),
        }
    }

    fn answer(&self, target: &AddrTarget) -> AddrOutcome {
        if let AddrTarget::Seat { location } = target
            && let Some(outcome) = self.exact_seat(location)
        {
            return outcome;
        }
        let sessions = self.sessions.attached();
        let budget = self.budget();
        let mut matches = Vec::new();
        let mut failed = sessions.is_empty();
        for session in sessions {
            match session.port().live_topology(&budget) {
                Ok(topology) => matches.push(HostMatch {
                    host: session.host_id(),
                    outcome: Self::outcome_of(target, &topology),
                }),
                Err(_) => failed = true,
            }
        }
        pool(matches, failed, |host, pane| self.seat_on(host, pane))
    }
}

/// A failed seat lookup: a seat conflict cannot change on retry, a store
/// failure can.
fn lookup_failed(error: ApiError) -> AddrOutcome {
    if error.code == crate::protocol::results::ErrorCode::Conflict {
        AddrOutcome::Refused {
            detail: error.detail,
        }
    } else {
        AddrOutcome::Unavailable {
            reason: format!("seat lookup failed: {}", error.detail),
        }
    }
}

/// The published resolved local seats on `pane` of host `host_id`, at most
/// two (only a published seat exists on other nodes).
pub fn pane_seats(
    conn: &rusqlite::Connection,
    host_id: i64,
    pane: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut statement = conn.prepare_cached(
        "SELECT id FROM seats WHERE is_local=1 AND published=1 AND state='resolved' \
         AND host_id=?1 AND target_id=?2 ORDER BY id LIMIT 2",
    )?;
    let rows = statement.query_map(rusqlite::params![host_id, pane], |r| r.get(0))?;
    rows.collect()
}

/// What `SEAT_ID@M` needs of one seat row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatRow {
    pub local: bool,
    pub published: bool,
    pub target: Option<String>,
}

/// The row of `seat`, retired included (a bare exact seat ID resolves a
/// retired seat too).
pub fn seat_row(conn: &rusqlite::Connection, seat: &str) -> rusqlite::Result<Option<SeatRow>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT is_local, published, target_id FROM seats WHERE id=?1",
        [seat],
        |r| {
            Ok(SeatRow {
                local: r.get::<_, i64>(0)? == 1,
                published: r.get::<_, i64>(1)? == 1,
                target: r.get(2)?,
            })
        },
    )
    .optional()
}

/// The answer for `SEAT_ID@M` from its row: only this node's own published
/// seat (retired included) is a seat; another node's, or one never shared
/// with other nodes, is refused (asking again cannot help); an unknown ID
/// falls through to name matching (`None`).
pub fn exact_outcome(seat: &str, row: Option<SeatRow>) -> Option<AddrOutcome> {
    let row = row?;
    Some(match row {
        SeatRow { local: false, .. } => AddrOutcome::Refused {
            detail: format!("seat {seat} does not live on this machine"),
        },
        SeatRow {
            published: false, ..
        } => AddrOutcome::Refused {
            detail: format!("seat {seat} is not shared with other machines"),
        },
        SeatRow { target, .. } => AddrOutcome::Seat {
            seat: seat.to_owned(),
            path: String::new(),
            pane: target.unwrap_or_default(),
        },
    })
}

/// The answer from every read host's matches (`failed`: some host could not
/// be read), mapping the one matched pane to its seat through `seat_on`.
fn pool(
    matches: Vec<HostMatch>,
    failed: bool,
    mut seat_on: impl FnMut(HostId, &str) -> Result<Option<String>, ApiError>,
) -> AddrOutcome {
    if matches
        .iter()
        .any(|m| matches!(m.outcome, Outcome::InvalidPath))
    {
        return AddrOutcome::Refused {
            detail: r"invalid path: use 1-3 nonempty components; escapes: \/, \\ and \@".into(),
        };
    }
    let pooled: Vec<(HostId, address::Candidate)> = matches
        .into_iter()
        .flat_map(|m| {
            let host = m.host;
            match m.outcome {
                Outcome::Resolved(c) => vec![(host, c)],
                Outcome::Ambiguous(cs) => cs.into_iter().map(|c| (host, c)).collect(),
                Outcome::NoMatch | Outcome::InvalidPath => Vec::new(),
            }
        })
        .collect();
    // A host that failed to read could hold a further match: only an answer
    // that is already ambiguous stands without it.
    if failed && pooled.len() < 2 {
        return AddrOutcome::HostUnavailable;
    }
    match pooled.as_slice() {
        [] => AddrOutcome::NoMatch,
        [(host, one)] => match seat_on(*host, one.pane.as_str()) {
            Ok(Some(seat)) => AddrOutcome::Seat {
                seat,
                path: one.path.clone(),
                pane: one.pane.as_str().to_owned(),
            },
            Ok(None) => AddrOutcome::NoSeat {
                path: one.path.clone(),
                pane: one.pane.as_str().to_owned(),
            },
            Err(error) => lookup_failed(error),
        },
        many => AddrOutcome::Ambiguous {
            candidates: many
                .iter()
                .map(|(host, c)| AddrCandidate {
                    path: c.path.clone(),
                    pane: c.pane.as_str().to_owned(),
                    seat: seat_on(*host, c.pane.as_str()).ok().flatten(),
                })
                .collect(),
            omitted: 0,
        },
    }
}

impl AddrResolver for RemoteResolver {
    fn resolve(&self, target: AddrTarget) -> AddrOutcome {
        self.answer(&target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::observation::{TopologyPane, TopologySpace, TopologyTab};
    use crate::protocol::ids::HostTargetId;

    fn topology(panes: &[(&str, &str)]) -> HostTopology {
        HostTopology {
            spaces: vec![TopologySpace {
                id: "w1".into(),
                label: Some("office".into()),
            }],
            tabs: vec![TopologyTab {
                id: "w1:t1".into(),
                space: "w1".into(),
                label: Some("main".into()),
            }],
            panes: panes
                .iter()
                .map(|(id, label)| TopologyPane {
                    target: HostTargetId::parse((*id).to_owned()).unwrap(),
                    space: "w1".into(),
                    tab: "w1:t1".into(),
                    label: Some((*label).into()),
                    agent_names: Vec::new(),
                })
                .collect(),
        }
    }

    fn on(host: i64, panes: &[(&str, &str)], location: &str) -> HostMatch {
        HostMatch {
            host: HostId::from_row(host),
            outcome: RemoteResolver::outcome_of(
                &AddrTarget::Seat {
                    location: location.into(),
                },
                &topology(panes),
            ),
        }
    }

    fn seats(host: HostId, pane: &str) -> Result<Option<String>, ApiError> {
        Ok(match (host.get(), pane) {
            (1, "w1:p1") => Some("sAAAseat0001".into()),
            (2, "w1:p9") => Some("sAAAseat0002".into()),
            (2, "w1:p8") => {
                return Err(ApiError::new(
                    crate::protocol::results::ErrorCode::Conflict,
                    "pane w1:p8 maps to more than one seat",
                ));
            }
            _ => None,
        })
    }

    // Kills: matches not pooled across hosts, a failed host ignored (a false
    // unique match or NoMatch), an unseated pane answered as a seat, a seat
    // conflict or an invalid path not refused, and no host read as NoMatch.
    #[test]
    fn matches_pool_across_hosts_and_a_failed_host_withholds_a_unique_answer() {
        let one = on(1, &[("w1:p1", "claude")], "claude");
        let other = on(2, &[("w1:p9", "claude")], "claude");
        let none = on(2, &[("w1:p9", "codex")], "claude");
        // One match on two read hosts: that pane's seat.
        assert_eq!(
            pool(vec![one.clone_match(), none.clone_match()], false, seats),
            AddrOutcome::Seat {
                seat: "sAAAseat0001".into(),
                path: "office/main/claude".into(),
                pane: "w1:p1".into(),
            }
        );
        // The same name on two hosts: ambiguous, both listed with seats.
        let AddrOutcome::Ambiguous { candidates, .. } =
            pool(vec![one.clone_match(), other.clone_match()], false, seats)
        else {
            panic!("ambiguous")
        };
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[1].seat.as_deref(), Some("sAAAseat0002"));
        // A failed host: a unique or empty answer is withheld, an ambiguous
        // one stands.
        assert_eq!(
            pool(vec![one.clone_match()], true, seats),
            AddrOutcome::HostUnavailable
        );
        assert_eq!(
            pool(vec![none.clone_match()], true, seats),
            AddrOutcome::HostUnavailable
        );
        assert_eq!(pool(Vec::new(), true, seats), AddrOutcome::HostUnavailable);
        assert!(matches!(
            pool(vec![one.clone_match(), other.clone_match()], true, seats),
            AddrOutcome::Ambiguous { .. }
        ));
        assert_eq!(
            pool(vec![none.clone_match()], false, seats),
            AddrOutcome::NoMatch
        );
        // An unseated pane, a seat conflict, an invalid path.
        assert!(matches!(
            pool(vec![on(1, &[("w1:p2", "spare")], "spare")], false, seats),
            AddrOutcome::NoSeat { .. }
        ));
        assert_eq!(
            pool(vec![on(2, &[("w1:p8", "dup")], "dup")], false, seats),
            AddrOutcome::Refused {
                detail: "pane w1:p8 maps to more than one seat".into()
            }
        );
        assert!(matches!(
            pool(
                vec![on(1, &[("w1:p1", "claude")], r"path:a\x")],
                false,
                seats
            ),
            AddrOutcome::Refused { .. }
        ));
    }

    impl HostMatch {
        fn clone_match(&self) -> Self {
            Self {
                host: self.host,
                outcome: self.outcome.clone(),
            }
        }
    }
}
