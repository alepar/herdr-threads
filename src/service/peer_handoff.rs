//! PeerHandoffRequester: the sender node's side of an atomic remote handoff (spec §2.2-§2.3,
//! TRUST-POLICY N4, N15).
//!
//! Responsibility
//! - Owns: the claim check for a handoff draft, signing the sender block once, recording the
//!   request in the `handoff_obligations` ledger with its frozen bytes, sending the row's bytes
//!   (every poll and every re-send replays them verbatim, never a rebuilt request), settling the
//!   target's answer into the ledger, opening the pair thread locally after `Launched`,
//!   answering from the ledger for terminal rows and `resume_only` polls, and the re-send pass
//!   (expiry first, then every unlinked row on link-up and at most every
//!   [`RESEND_EVERY_MS`]).
//! - Hides: the ledger SQL (`store::handoff_obligations`), the signing domain
//!   (`SenderBlock`) and the transport (a [`RequestSender`]).
//! - Must not: author the handoff message (the target's stage does), write a replicated fact
//!   other than the pair's own `thread.direct`, or decide anything on a claim the daemon cannot
//!   verify against its store (A2).
//! - Role: `SyncAdmin::remote_launch` delegates a plan with a draft to [`Requester::poll`]; the
//!   daemon's re-send task calls [`Requester::resend_pass`].
//!
//! Core and harness-neutral.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::app::SystemClock;
use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::authority::CallerClaim;
use crate::protocol::ids::{MessageId, SeatId, direct_thread_id, prefix};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::summary::AuthorRole;
use crate::protocol::sync_admin::{HandoffLedgerOutcome, RemoteLaunchAnswer};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant};
use crate::service::fair_writer::FairWriter;
use crate::service::pacer::{Pacer, Wake};
use crate::store::connection::{StoreContext, store_error};
use crate::store::handoff_obligations::{
    self as ledger, NewObligation, OBLIGATION_TTL_MS, Obligation, State,
};
use crate::store::sync_status::node_label;
use crate::store::{SqliteStore, node_directory, public_ids, sync_contacts};
use crate::sync::coordinator::remote_launch::LaunchAnswer;
use crate::sync::coordinator::{LinkMode, LogFn, SyncHandle};
use crate::sync::frame::{LaunchRequest, LaunchState};

/// A row that is not yet linked is re-sent at most this often (ms).
pub const RESEND_EVERY_MS: i64 = 30_000;
/// How long one send waits for the target's answer.
const SEND_WAIT_MS: u64 = 10_000;
/// How long a CLI poll then waits for the launched seat to replicate here.
const REPLICA_WAIT_MS: u64 = 5_000;
/// Kept free in a poll's budget for the settle writes and the reply.
const SETTLE_MARGIN_MS: u64 = 1_000;
/// The refusal when the ACK window exceeds the ledger's lifetime (61 bytes).
const DEADLINE_TOO_LONG: &str = "--deadline for a handoff to another machine is at most 6 days";

/// Request ids with a launch send outstanding from this process. One send per request id at a
/// time from this daemon, because the coordinator answers a second outstanding launch with the
/// same id "too many launch requests in flight"; the CLI poll and the re-send task are separate
/// [`Requester`]s, so the exclusion lives here.
static IN_FLIGHT: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// The Pacers of the callers waiting for a slot, by ticket and request id; a freed slot kicks
/// every one.
static WAITERS: LazyLock<Mutex<Vec<(u64, String, Pacer)>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static NEXT_TICKET: AtomicU64 = AtomicU64::new(0);

/// The right to send one request id; dropping it frees the id and wakes the waiters.
struct SendSlot(String);

impl Drop for SendSlot {
    fn drop(&mut self) {
        IN_FLIGHT
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
        for (_, _, pacer) in WAITERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            pacer.kick();
        }
    }
}

/// Takes the slot for `id` if no send holds it.
fn try_slot(id: &str) -> Option<SendSlot> {
    IN_FLIGHT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(id.to_owned())
        .then(|| SendSlot(id.to_owned()))
}

/// Removes a waiter's Pacer from [`WAITERS`] when the wait ends.
struct Waiting(u64);

impl Drop for Waiting {
    fn drop(&mut self) {
        WAITERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(ticket, _, _)| *ticket != self.0);
    }
}

/// Whether a caller is waiting for the slot of `id` (tests gate on the poll having entered the wait).
#[cfg(test)]
fn waiting_for(id: &str) -> bool {
    WAITERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|(_, waiting, _)| waiting == id)
}

/// Waits up to `until` (wall time) for the slot for `id` on its own [`Pacer`]: a freed slot kicks
/// it, cancellation ends the wait, and the tick is the time left.
fn wait_slot(id: &str, until: Duration, cancel: &Cancellation) -> Option<SendSlot> {
    let pacer = Pacer::new("handoff-slot", Arc::new(SystemClock::new()), cancel.clone());
    let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
    WAITERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((ticket, id.to_owned(), pacer.clone()));
    let _waiting = Waiting(ticket);
    let end = Instant::now() + until;
    loop {
        if let Some(slot) = try_slot(id) {
            return Some(slot);
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() || cancel.is_cancelled() {
            return None;
        }
        pacer.wait_blocking(left);
    }
}

/// Sends one frozen request to a node and returns its answer; `None` when none came in time.
/// The production impl wraps [`SyncHandle::launch_remote`]; tests record the bytes.
pub trait RequestSender: Send + Sync {
    fn send(
        &self,
        node: NodeUuid,
        bytes: &[u8],
        wait: Duration,
        cancel: &Cancellation,
    ) -> Option<LaunchAnswer>;
}

/// The coordinator as a [`RequestSender`]: decodes the row's bytes and sends that request.
#[derive(Clone)]
pub struct SyncSender {
    pub sync: SyncHandle,
    pub runtime: tokio::runtime::Handle,
}

impl RequestSender for SyncSender {
    fn send(
        &self,
        node: NodeUuid,
        bytes: &[u8],
        wait: Duration,
        cancel: &Cancellation,
    ) -> Option<LaunchAnswer> {
        let Ok(request) = serde_json::from_slice::<LaunchRequest>(bytes) else {
            return Some(LaunchAnswer::Answered(LaunchState::Unavailable {
                reason: "recorded request unreadable".into(),
            }));
        };
        self.runtime
            .block_on(self.sync.launch_remote(node, request, wait, cancel))
    }
}

/// What a poll answers: the answer and the ledger's message id for the request.
pub struct Polled {
    pub answer: RemoteLaunchAnswer,
    pub message: String,
}

/// What the re-send task remembers between passes.
#[derive(Default)]
pub struct ResendState {
    last_send: HashMap<String, i64>,
    linked: HashSet<NodeUuid>,
    /// When the last pass pruned old terminal rows (`None` until the first).
    last_prune: Option<i64>,
}

/// How often a long-lived daemon prunes old terminal rows.
const PRUNE_EVERY_MS: i64 = 3_600_000;

/// See the module documentation.
pub struct Requester<'a> {
    pub store: &'a SqliteStore,
    pub writer: &'a FairWriter,
    pub clock: &'a dyn Clock,
    pub identity: &'a NodeIdentity,
    pub log: &'a LogFn,
    pub sender: &'a dyn RequestSender,
    /// The re-send task takes the writer's background lane, a CLI poll the foreground one.
    pub background: bool,
}

fn terminal(state: State) -> bool {
    !matches!(state, State::Sent | State::Launched)
}

fn caller_unverified(detail: &str) -> ApiError {
    ApiError::new(ErrorCode::CallerUnverified, detail)
}

fn req8(request: &str) -> &str {
    request.get(..8).unwrap_or(request)
}

impl Requester<'_> {
    fn write<R>(
        &self,
        budget: &CallBudget,
        apply: impl FnOnce(
            &rusqlite::Transaction<'_>,
            &StoreContext,
            crate::store::connection::DecisionInstant,
        ) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let _turn = if self.background {
            self.writer.enter_background(budget, self.clock)?
        } else {
            self.writer.enter_foreground(budget, self.clock)?
        };
        self.store.sync_operator_quantum(budget, apply)
    }

    fn now(&self) -> i64 {
        self.clock.utc_now().0
    }

    fn fresh_budget(&self, cancel: &Cancellation, ms: u64) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0.saturating_add(ms)),
            cancellation: cancel.clone(),
        }
    }

    /// The longest a send (or the wait for its slot) may take: what the budget has left after the
    /// replica wait and the settle margin, at most [`SEND_WAIT_MS`].
    fn send_window(&self, budget: &CallBudget) -> Duration {
        let remaining = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        let room = remaining.saturating_sub(REPLICA_WAIT_MS + SETTLE_MARGIN_MS);
        Duration::from_millis(room.min(SEND_WAIT_MS))
    }

    /// Answers `request` from the ledger or the target. Every wait is cut from the budget so the
    /// answer fits the transport bound. `claim` is the CLI's claim for a draft;
    /// `resume_only` marks a request file older than the ledger's lifetime (never signs).
    pub fn poll(
        &self,
        node: NodeUuid,
        request: LaunchRequest,
        claim: Option<&CallerClaim>,
        resume_only: bool,
        budget: &CallBudget,
    ) -> Result<Polled, ApiError> {
        self.expire(budget)?;
        let id = request.id.to_string();
        let existing = self.store.sync_read(budget, |conn, _| {
            ledger::get(conn, &id).map_err(store_error)
        })?;
        let row = match existing {
            Some(row) if terminal(row.state) => return self.ledger_answer(&row, budget),
            None if resume_only => {
                return Ok(Polled {
                    answer: RemoteLaunchAnswer::HandoffLedger {
                        outcome: HandoffLedgerOutcome::Unknown,
                    },
                    message: String::new(),
                });
            }
            existing => self.record(&request, claim, existing, node, budget)?,
        };
        if terminal(row.state) {
            return self.ledger_answer(&row, budget);
        }
        let wait = self.send_window(budget);
        // A re-send of this id may be in flight: wait for it rather than have the coordinator
        // refuse a second outstanding launch. After a wait, the re-send may have settled the row.
        let (row, slot) = match try_slot(&id) {
            Some(slot) => (row, Some(slot)),
            None => match wait_slot(&id, wait, &budget.cancellation) {
                Some(slot) => {
                    let fresh = self.store.sync_read(budget, |conn, _| {
                        ledger::get(conn, &id).map_err(store_error)
                    })?;
                    match fresh {
                        Some(fresh) if terminal(fresh.state) => {
                            return self.ledger_answer(&fresh, budget);
                        }
                        fresh => (fresh.unwrap_or(row), Some(slot)),
                    }
                }
                None => (row, None),
            },
        };
        // No slot, or the slot wait used the window up: settled as a timeout, the CLI's "still
        // pending" path, without sending.
        let window = self.send_window(budget);
        let (answer, used) = match slot {
            Some(slot) if !window.is_zero() => {
                let answer =
                    self.sender
                        .send(node, &row.request_bytes, window, &budget.cancellation);
                drop(slot);
                (answer, window)
            }
            _ => (None, wait),
        };
        let settled = self.settle(&row, node, answer, used, true, REPLICA_WAIT_MS, budget)?;
        Ok(Polled {
            answer: settled.answer,
            message: row.message,
        })
    }

    /// Checks the claim and records the request: the existing row, or a new one signed once.
    fn record(
        &self,
        request: &LaunchRequest,
        claim: Option<&CallerClaim>,
        existing: Option<Obligation>,
        node: NodeUuid,
        budget: &CallBudget,
    ) -> Result<Obligation, ApiError> {
        let now = self.now();
        self.write(budget, |tx, ctx, _| {
            let instance: String = tx
                .query_row("SELECT id FROM local_node", [], |r| r.get(0))
                .map_err(store_error)?;
            let claim = claim.ok_or_else(|| caller_unverified("handoff needs a caller claim"))?;
            crate::store::seats::cooperative_claim(tx, ctx, &instance, claim)?;
            if claim.seat.as_str() != request.requester_seat {
                return Err(caller_unverified("claim does not match requesting seat"));
            }
            let (bindings, human): (i64, bool) = tx
                .query_row(
                    "SELECT COUNT(*), COALESCE(MAX(harness='human'),0) FROM occupant_bindings \
                     WHERE seat_id=?1 AND ended_at IS NULL AND role='top_level'",
                    [claim.seat.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(store_error)?;
            let retired: bool = tx
                .query_row(
                    "SELECT state='retired' FROM seats WHERE id=?1",
                    [claim.seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if bindings == 0 || retired {
                return Err(caller_unverified(
                    "requesting seat has no open top-level binding",
                ));
            }
            let id = request.id.to_string();
            if let Some(row) = ledger::get(tx, &id).map_err(store_error)? {
                return Ok(row);
            }
            // The row may have been pruned between the read and here; `existing` only says it was
            // there, the claim check above is what lets a new one be recorded.
            let _ = &existing;
            let draft = request
                .plan
                .handoff()
                .ok_or_else(|| ApiError::new(ErrorCode::InvalidRequest, "not a handoff request"))?;
            if draft.ack_after_ms.is_some_and(|ms| ms > OBLIGATION_TTL_MS) {
                return Err(ApiError::new(ErrorCode::InvalidRequest, DEADLINE_TOO_LONG));
            }
            let message =
                public_ids::fresh(tx, prefix::MESSAGE, &[("messages", "id", prefix::MESSAGE)])?;
            let invalid = |e: &dyn std::fmt::Display| {
                ApiError::new(ErrorCode::InvalidRequest, format!("handoff block: {e}"))
            };
            let mut block = draft.clone();
            block.v = 1;
            block.request = request.id;
            block.sender_node = Some(self.identity.uuid());
            block.author = Some(claim.seat.clone());
            block.author_role = Some(if human {
                AuthorRole::Human
            } else {
                AuthorRole::Agent
            });
            block.message = Some(MessageId::new(message.clone()));
            block.priority = 0;
            block.require_ack = true;
            block.requested_at = now;
            block.sign(self.identity).map_err(|e| invalid(&e))?;
            block
                .verify(&self.identity.public_key())
                .map_err(|e| invalid(&e))?;
            let deadline_at = block.deadline_at().map_err(|e| invalid(&e))?;
            let mut frozen = request.clone();
            frozen.plan.message = Some(Box::new(block));
            let bytes = serde_json::to_vec(&frozen)
                .map_err(|e| ApiError::new(ErrorCode::InvalidRequest, e.to_string()))?;
            let (row, _) = ledger::insert_or_get(
                tx,
                &NewObligation {
                    request: &id,
                    target_node: &node.to_string(),
                    author: claim.seat.as_str(),
                    message: &message,
                    requested_at: now,
                    deadline_at,
                    request_bytes: &bytes,
                },
            )
            .map_err(store_error)?;
            Ok(row)
        })
    }

    /// Settles `answer` into the ledger and maps it for the caller.
    #[allow(clippy::too_many_arguments)]
    fn settle(
        &self,
        row: &Obligation,
        node: NodeUuid,
        answer: Option<LaunchAnswer>,
        wait: Duration,
        shown: bool,
        replica_wait_ms: u64,
        budget: &CallBudget,
    ) -> Result<Settled, ApiError> {
        let now = self.now();
        let mut failed = None;
        let mapped = match answer {
            None => RemoteLaunchAnswer::Timeout {
                secs: wait.as_secs(),
            },
            Some(LaunchAnswer::NotLinked | LaunchAnswer::LinkLost) => {
                let last = self
                    .store
                    .sync_read(budget, |conn, _| sync_contacts::get(conn, &node))?;
                RemoteLaunchAnswer::Unreachable {
                    last_contact_at: last.and_then(|c| i64::try_from(c.last_contact_at).ok()),
                }
            }
            Some(LaunchAnswer::NoCap { version, cap }) => {
                failed = Some(format!("needs {cap}"));
                RemoteLaunchAnswer::NoCap {
                    version,
                    cap: cap.to_owned(),
                }
            }
            Some(LaunchAnswer::Answered(state)) => {
                match &state {
                    LaunchState::Refused { detail }
                    | LaunchState::Failed {
                        detail,
                        staged: None,
                        ..
                    } => failed = Some(detail.clone()),
                    _ => {}
                }
                RemoteLaunchAnswer::Answered { state }
            }
        };
        if let Some(detail) = failed {
            let text = serde_json::to_string(&mapped)
                .map_err(|e| ApiError::new(ErrorCode::InvalidRequest, e.to_string()))?;
            let changed = self.write(budget, |tx, _, _| {
                ledger::fail(tx, &row.request, &text, shown, now).map_err(store_error)
            })?;
            return Ok(Settled {
                answer: mapped,
                failed: changed.then_some(detail),
            });
        }
        if let RemoteLaunchAnswer::Answered {
            state: LaunchState::Launched { seat, .. },
        } = &mapped
        {
            self.write(budget, |tx, _, _| {
                ledger::launched(tx, &row.request, seat, now).map(|_| ())
            })?;
            if !crate::service::sync_admin::wait_replicated(
                self.store,
                self.clock,
                seat,
                &node,
                replica_wait_ms,
                budget,
            )? {
                return Ok(Settled {
                    answer: RemoteLaunchAnswer::NotReplicated { seat: seat.clone() },
                    failed: None,
                });
            }
            if let Err(error) = self.open_pair(row, seat, budget) {
                (self.log)(&format!(
                    "handoff {} pair not opened: {}",
                    req8(&row.request),
                    error.detail
                ));
            }
        }
        Ok(Settled {
            answer: mapped,
            failed: None,
        })
    }

    /// Opens the pair thread locally (the claim-free resolver); a pair that exists is left.
    fn open_pair(&self, row: &Obligation, seat: &str, budget: &CallBudget) -> Result<(), ApiError> {
        let (author, recipient) = (SeatId::new(&row.author), SeatId::new(seat));
        let thread = direct_thread_id(&author, &recipient)
            .map_err(|e| ApiError::new(ErrorCode::InvalidRequest, e.to_string()))?;
        self.write(budget, |tx, ctx, decision| {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1)",
                    [thread.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let author_live: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND is_local=1 AND state!='retired')",
                    [author.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if exists || !author_live {
                return Ok(());
            }
            let instance: String = tx
                .query_row("SELECT id FROM local_node", [], |r| r.get(0))
                .map_err(store_error)?;
            crate::store::control::open_pair(
                tx, ctx, decision, &instance, &author, &recipient, None,
            )
            .map(|_| ())
        })
    }

    /// The answer a terminal row gives, with no send. A failed or expired row whose alert no CLI
    /// showed is marked shown: this poll is the CLI showing it.
    fn ledger_answer(&self, row: &Obligation, budget: &CallBudget) -> Result<Polled, ApiError> {
        let outcome = match row.state {
            State::Linked => {
                let seat = row.seat.clone().unwrap_or_default();
                let thread = direct_thread_id(&SeatId::new(&row.author), &SeatId::new(&seat))
                    .map(|t| t.as_str().to_owned())
                    .unwrap_or_default();
                HandoffLedgerOutcome::Linked {
                    seat,
                    thread,
                    message: row.message.clone(),
                }
            }
            State::Expired => HandoffLedgerOutcome::Expired {
                seat: row.seat.clone(),
            },
            _ => HandoffLedgerOutcome::Unknown,
        };
        let answer = match (row.state, &row.answer) {
            (State::Failed, Some(text)) => {
                serde_json::from_str(text).unwrap_or(RemoteLaunchAnswer::Unknown)
            }
            _ => RemoteLaunchAnswer::HandoffLedger { outcome },
        };
        if row.alert_pending() {
            let now = self.now();
            self.write(budget, |tx, _, _| {
                ledger::mark_alerts_shown(tx, &row.author, std::slice::from_ref(&row.request), now)
                    .map(|_| ())
            })?;
        }
        Ok(Polled {
            answer,
            message: row.message.clone(),
        })
    }

    /// Expires every row past its lifetime, one `daemon.log` line each.
    pub fn expire(&self, budget: &CallBudget) -> Result<(), ApiError> {
        let now = self.now();
        let any = self.store.sync_read(budget, |conn, _| {
            Ok(ledger::resend_set(conn, None)
                .map_err(store_error)?
                .iter()
                .any(|row| row.expires_at <= now))
        })?;
        if !any {
            return Ok(());
        }
        let expired = self.write(budget, |tx, _, _| ledger::expire_due(tx, now))?;
        for row in expired {
            let to = self.tag_of(&row.target_node, budget);
            let line = match &row.seat {
                None => format!("handoff {} to {to} expired unconfirmed", req8(&row.request)),
                Some(seat) => format!(
                    "handoff {} to {to} expired unlinked: seat {seat}",
                    req8(&row.request)
                ),
            };
            (self.log)(&line);
        }
        Ok(())
    }

    /// Deletes terminal rows past their retention.
    pub fn prune(&self, budget: &CallBudget) -> Result<usize, ApiError> {
        let now = self.now();
        self.write(budget, |tx, _, _| ledger::prune(tx, now))
    }

    /// At daemon start: expire what is past its lifetime and prune old terminal rows.
    pub fn startup(&self, budget: &CallBudget) -> Result<(), ApiError> {
        self.expire(budget)?;
        self.prune(budget).map(|_| ())
    }

    fn tag_of(&self, target_node: &str, budget: &CallBudget) -> String {
        let tag = target_node.parse::<NodeUuid>().ok().and_then(|uuid| {
            self.store
                .sync_read(budget, |conn, _| {
                    node_directory::get(conn, uuid).map_err(store_error)
                })
                .ok()
                .flatten()
        });
        node_label(tag.map(|row| row.tag.to_string()).as_deref(), target_node)
    }

    /// One re-send pass: expiry first (an error is logged, not fatal; a cancellation is not logged), then every `sent|launched` row whose target is linked and
    /// that is due (the target just linked, or its last send is [`RESEND_EVERY_MS`] old). An
    /// answer is settled with `shown = false`; the target launches once per id, so a re-send
    /// never starts a second agent. A row whose settlement errors is logged and skipped.
    pub fn resend_pass(
        &self,
        state: &mut ResendState,
        linked_now: &HashSet<NodeUuid>,
        cancel: &Cancellation,
    ) -> Result<(), ApiError> {
        let budget = self.fresh_budget(cancel, SEND_WAIT_MS * 2);
        if let Err(error) = self.expire(&budget)
            && error.code != ErrorCode::Cancelled
        {
            (self.log)(&format!("handoff expiry failed: {}", error.detail));
        }
        let now = self.now();
        if state
            .last_prune
            .is_none_or(|at| now.saturating_sub(at) >= PRUNE_EVERY_MS)
        {
            state.last_prune = Some(now);
            if let Err(error) = self.prune(&budget)
                && error.code != ErrorCode::Cancelled
            {
                (self.log)(&format!("handoff prune failed: {}", error.detail));
            }
        }
        let newly: HashSet<NodeUuid> = linked_now.difference(&state.linked).copied().collect();
        state.linked.clone_from(linked_now);
        let rows = self.store.sync_read(&budget, |conn, _| {
            ledger::resend_set(conn, None).map_err(store_error)
        })?;
        state
            .last_send
            .retain(|request, _| rows.iter().any(|row| &row.request == request));
        for row in rows {
            let Ok(node) = row.target_node.parse::<NodeUuid>() else {
                continue;
            };
            let now = self.now();
            let due = newly.contains(&node)
                || state
                    .last_send
                    .get(&row.request)
                    .is_none_or(|at| now.saturating_sub(*at) >= RESEND_EVERY_MS);
            if !linked_now.contains(&node) || !due {
                continue;
            }
            // A CLI poll is sending this id: skip it and leave it due for the next pass.
            let Some(slot) = try_slot(&row.request) else {
                continue;
            };
            state.last_send.insert(row.request.clone(), now);
            let wait = Duration::from_millis(SEND_WAIT_MS);
            let answer = self.sender.send(node, &row.request_bytes, wait, cancel);
            drop(slot);
            let budget = self.fresh_budget(cancel, SEND_WAIT_MS);
            let settled = match self.settle(&row, node, answer, wait, false, 0, &budget) {
                Ok(settled) => settled,
                Err(error) if error.code == ErrorCode::Cancelled => continue,
                Err(error) => {
                    (self.log)(&format!(
                        "handoff {} to {} re-send failed: {}",
                        req8(&row.request),
                        self.tag_of(&row.target_node, &budget),
                        error.detail
                    ));
                    continue;
                }
            };
            if let Some(detail) = settled.failed {
                (self.log)(&format!(
                    "handoff {} to {} failed: {detail}",
                    req8(&row.request),
                    self.tag_of(&row.target_node, &budget)
                ));
            }
        }
        Ok(())
    }
}

/// How often the re-send task looks at the links.
const TASK_PERIOD_MS: u64 = 2_000;

/// What the re-send task owns.
pub struct ResendDeps {
    pub store: Arc<SqliteStore>,
    pub writer: Arc<FairWriter>,
    pub clock: Arc<dyn Clock>,
    pub identity: Arc<NodeIdentity>,
    pub log: LogFn,
    pub sync: SyncHandle,
    pub runtime: tokio::runtime::Handle,
}

/// Starts the daemon's re-send thread: at start it expires and prunes, then every
/// [`TASK_PERIOD_MS`] it reads the coordinator's links and runs a pass. It ends when `stop` is
/// cancelled.
pub fn start_resend_task(
    deps: ResendDeps,
    stop: Cancellation,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("handoff-resend".into())
        .spawn(move || {
            let sender = SyncSender {
                sync: deps.sync.clone(),
                runtime: deps.runtime.clone(),
            };
            let requester = Requester {
                store: &deps.store,
                writer: &deps.writer,
                clock: deps.clock.as_ref(),
                identity: &deps.identity,
                log: &deps.log,
                sender: &sender,
                background: true,
            };
            // A Stop mid-pass is a cancellation, which no lane logs.
            let report = |what: &str, error: &ApiError| {
                if error.code != ErrorCode::Cancelled {
                    (deps.log)(&format!("handoff {what} failed: {}", error.detail));
                }
            };
            let start = requester.fresh_budget(&stop, SEND_WAIT_MS);
            if let Err(error) = requester.startup(&start) {
                report("startup", &error);
            }
            let mut state = ResendState::default();
            let pacer = Pacer::new("handoff-resend", deps.clock.clone(), stop.clone());
            while !stop.is_cancelled() {
                let ask = requester.fresh_budget(&stop, TASK_PERIOD_MS);
                if let Ok(snapshot) = deps.runtime.block_on(deps.sync.snapshot(&ask)) {
                    let linked: HashSet<NodeUuid> = snapshot
                        .links
                        .iter()
                        .filter(|link| link.mode == LinkMode::Active)
                        .map(|link| link.node)
                        .collect();
                    if let Err(error) = requester.resend_pass(&mut state, &linked, &stop) {
                        report("resend", &error);
                    }
                }
                if pacer.wait_blocking(Duration::from_millis(TASK_PERIOD_MS)) == Wake::Cancelled {
                    return;
                }
            }
        })
}

struct Settled {
    answer: RemoteLaunchAnswer,
    /// The detail of a `sent -> failed` transition this settle made.
    failed: Option<String>,
}

#[cfg(test)]
#[path = "../../tests/service/peer_handoff.rs"]
mod tests;
