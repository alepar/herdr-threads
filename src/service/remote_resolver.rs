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
        let seats: Vec<String> = self.store.sync_read(&self.budget(), |conn, _| {
            let mut statement = conn
                .prepare_cached(
                    "SELECT id FROM seats WHERE is_local=1 AND state='resolved' \
                     AND host_id=?1 AND target_id=?2 ORDER BY id LIMIT 2",
                )
                .map_err(crate::store::connection::store_error)?;
            let rows = statement
                .query_map(rusqlite::params![host.get(), pane], |r| r.get(0))
                .map_err(crate::store::connection::store_error)?;
            rows.collect::<Result<Vec<String>, _>>()
                .map_err(crate::store::connection::store_error)
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
        let found: Option<(bool, Option<String>)> = self
            .store
            .sync_read(&self.budget(), |conn, _| {
                use rusqlite::OptionalExtension;
                conn.query_row(
                    "SELECT is_local, target_id FROM seats WHERE id=?1 AND state<>'retired'",
                    [seat.as_str()],
                    |r| Ok((r.get::<_, i64>(0)? == 1, r.get(1)?)),
                )
                .optional()
                .map_err(crate::store::connection::store_error)
            })
            .ok()?;
        match found? {
            (true, target) => Some(AddrOutcome::Seat {
                seat: seat.as_str().to_owned(),
                path: String::new(),
                pane: target.unwrap_or_default(),
            }),
            (false, _) => Some(AddrOutcome::Refused {
                detail: format!("seat {} does not live on this machine", seat.as_str()),
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
        // A host that failed to read could hold a further match: only an
        // answer that is already ambiguous stands without it.
        if failed && pooled.len() < 2 {
            return AddrOutcome::HostUnavailable;
        }
        match pooled.as_slice() {
            [] => AddrOutcome::NoMatch,
            [(host, one)] => match self.seat_on(*host, one.pane.as_str()) {
                Ok(Some(seat)) => AddrOutcome::Seat {
                    seat,
                    path: one.path.clone(),
                    pane: one.pane.as_str().to_owned(),
                },
                Ok(None) => AddrOutcome::NoSeat {
                    path: one.path.clone(),
                    pane: one.pane.as_str().to_owned(),
                },
                Err(error) => AddrOutcome::Refused {
                    detail: error.detail,
                },
            },
            many => AddrOutcome::Ambiguous {
                candidates: many
                    .iter()
                    .map(|(host, c)| AddrCandidate {
                        path: c.path.clone(),
                        pane: c.pane.as_str().to_owned(),
                        seat: self.seat_on(*host, c.pane.as_str()).ok().flatten(),
                    })
                    .collect(),
                omitted: 0,
            },
        }
    }
}

impl AddrResolver for RemoteResolver {
    fn resolve(&self, target: AddrTarget) -> AddrOutcome {
        self.answer(&target)
    }
}
