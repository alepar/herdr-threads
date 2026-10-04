//! Production-shaped retained history for scale tests. Every send, manifest
//! publication, send-worker projection, receipt-overdue warning (the due
//! scanner `scan_due`), warning attribution projection and ACK goes through
//! the real store writers (`prepare_send_step`, `publish_send`,
//! `advance_work`, `scan_due`, `ack`), so the v8 projection triggers and
//! receipt-state rows are exactly what production leaves behind. Only the
//! actors' cooperative permits are minted here, through the store's own
//! `issue_cooperative_permit`, from the seats' persisted target and
//! generation. Load-sensitive outcomes that roll back
//! without writing are retried a bounded number of times (`retryable`); every
//! other error is returned.

use crate::ports::{CooperativePermitRequest, DurableWorkAdmission, SendPreparationProgress};
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{Ack, SendMessage},
    ids::{ExecutionId, HostTargetId, MessageId, NativeSessionId, OperationId, SeatId, ThreadId},
    results::{ApiError, CommandResult, ErrorCode},
    time::{CallBudget, MonoInstant},
};
use crate::store::{
    connection::{StoreContext, api_error, store_error},
    materialization, messages, receipts, schema, seats,
};
use rusqlite::{Connection, OptionalExtension};

struct Actor {
    instance: String,
    seat: SeatId,
    target: HostTargetId,
    binding_generation: u64,
    harness: Harness,
    native_session: NativeSessionId,
    execution: ExecutionId,
}

/// A canonical UUID derived from the seat id: the execution of the live
/// cooperative binding this module gives a seat that has none.
pub fn fixture_execution(seat: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(seat, &mut hasher);
    let high = std::hash::Hasher::finish(&hasher);
    std::hash::Hash::hash(&high, &mut hasher);
    let low = std::hash::Hasher::finish(&hasher);
    uuid::Uuid::from_u128((u128::from(high) << 64) | u128::from(low))
        .hyphenated()
        .to_string()
}

/// The acting seat's cooperative context. A seat that already holds a live
/// binding acts as that occupant (its execution must be a canonical UUID); a
/// seat without one gets a live, not-yet-registered cooperative binding and
/// a current-target observation matching its persisted mapping, exactly the
/// durable context a cooperative permit is issued against.
fn actor(conn: &Connection, seat: &str) -> Result<Actor, ApiError> {
    let row: (String, String, i64) = conn
        .query_row(
            "SELECT s.instance_id,s.target_id,s.generation FROM seats s WHERE s.id=?1 AND s.state='resolved'",
            [seat],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(store_error)?;
    conn.execute(
        "INSERT OR IGNORE INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) \
         SELECT s.instance_id,s.target_id,h.host_boot,h.host_epoch,s.target_generation,0,'fresh','term-'||s.target_id,'inc','coherent_enumeration',1 \
         FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1",
        [seat],
    )
    .map_err(store_error)?;
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) \
         SELECT s.id,s.generation,s.target_generation,s.target_id,h.host_boot,h.host_epoch,'codex','hist-'||s.id,?2,'cooperative_top_level',0,'term-'||s.target_id,'inc' \
         FROM seats s JOIN host_instances h ON h.id=s.instance_id \
         WHERE s.id=?1 AND NOT EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=s.id AND b.ended_at IS NULL)",
        rusqlite::params![seat, fixture_execution(seat)],
    )
    .map_err(store_error)?;
    let (harness, native_session, execution): (String, String, String) = conn
        .query_row(
            "SELECT harness,native_session,execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
            [seat],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(store_error)?;
    Ok(Actor {
        instance: row.0,
        seat: SeatId::new(seat),
        target: HostTargetId::new(row.1),
        binding_generation: row.2 as u64,
        harness: match harness.as_str() {
            "claude" => Harness::Claude,
            "human" => Harness::Human,
            _ => Harness::Codex,
        },
        native_session: NativeSessionId::new(native_session),
        execution: ExecutionId::new(execution),
    })
}

impl Actor {
    fn claim(&self) -> CallerClaim {
        CallerClaim {
            instance: self.instance.clone(),
            seat: self.seat.clone(),
            binding_generation: self.binding_generation,
            role: CallerRole::TopLevel,
            harness: self.harness,
            native_session: self.native_session.clone(),
            execution: self.execution.clone(),
            target: self.target.clone(),
        }
    }
    fn permit(
        &self,
        context: &StoreContext,
        conn: &Connection,
        operation: &OperationId,
        obligation: ObligationRef,
        digest: [u8; 32],
    ) -> Result<MutationPermit, ApiError> {
        seats::issue_cooperative_permit(
            context,
            conn,
            &self.instance,
            CooperativePermitRequest {
                claim: self.claim(),
                operation: operation.clone(),
                obligation,
                payload_hash: digest,
                check_in_mode: None,
            },
            &unbounded(),
        )
    }
}

fn unbounded() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}

/// Bounded retries of load-sensitive outcomes (digest fix3 review S1).
const RETRIES: u32 = 200;

/// The two outcomes a loaded machine produces without anything being wrong,
/// both rolled back before any write commits: the send worker's fixed 5 ms
/// quantum elapsing before its first unit (`advance_work` then reports
/// `DeadlineExceeded` even under an unbounded budget), and a permit minted
/// here expiring (`MAX_PERMIT_MILLIS`) while the writer waits for the write
/// lock (a running daemon's worker may hold it). Every other error is
/// returned unchanged.
pub fn retryable(error: &ApiError) -> bool {
    (error.code == ErrorCode::DeadlineExceeded
        && error.detail == "materialization admission exhausted")
        || (error.code == ErrorCode::CallerUnverified && error.detail == "permit expired")
}

/// Run `f` until it succeeds, retrying only `retryable` errors, at most
/// `RETRIES` times; the last error is returned, never discarded.
fn with_retries<T>(mut f: impl FnMut() -> Result<T, ApiError>) -> Result<T, ApiError> {
    let mut attempt = 0;
    loop {
        match f() {
            Err(error) if retryable(&error) && attempt < RETRIES => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            other => return other,
        }
    }
}

/// Drain one durable work job through the production worker step, retrying
/// only `retryable` outcomes while the job is still outstanding.
fn drain_job(context: &StoreContext, conn: &mut Connection, job: &str) -> Result<(), ApiError> {
    let outstanding = |conn: &Connection| -> Result<bool, ApiError> {
        Ok(conn
            .query_row(
                "SELECT 1 FROM work_jobs WHERE id=?1 AND status IN ('pending','failed')",
                [job],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(store_error)?
            .is_some())
    };
    let mut retries = 0;
    while outstanding(conn)? {
        if let Err(error) = materialization::advance_work(
            conn,
            job,
            DurableWorkAdmission { max_units: 16 },
            &unbounded(),
            context.clock(),
        ) {
            // A running daemon's worker may finish the job first: that is the
            // same projection, so only an error on a still-outstanding job counts.
            if outstanding(conn)? {
                if !retryable(&error) || retries >= RETRIES {
                    return Err(error);
                }
                retries += 1;
            }
        }
    }
    Ok(())
}

/// Publish one send from `from` in `thread` and let the send worker project
/// it (every staged recipient gets its pending `receipt_state` row).
fn send_one(
    context: &StoreContext,
    conn: &mut Connection,
    from: &Actor,
    thread: &str,
    operation: &str,
    deadline_millis: Option<u64>,
) -> Result<MessageId, ApiError> {
    let request = SendMessage {
        thread: ThreadId::new(thread),
        body: format!("history {operation}"),
        invited_recipients: Vec::new(),
        deadline_millis,
        operation: OperationId::new(operation),
        claim: from.claim(),
        relays_user: false,
    };
    loop {
        match messages::prepare_send_step(
            context,
            conn,
            &request,
            messages::MessageLimits::default(),
            &unbounded(),
            DurableWorkAdmission { max_units: 16 },
        )? {
            SendPreparationProgress::Committed(_) => {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "history operation key reused",
                ));
            }
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { .. } => {}
        }
    }
    let digest = schema::canonical_digest(&messages::send_payload(&request))?;
    // A fresh permit per attempt: an expired one is rolled back unconsumed.
    let result = with_retries(|| {
        let mut permit = from.permit(
            context,
            conn,
            &request.operation,
            ObligationRef::Control(request.thread.clone()),
            digest,
        )?;
        messages::publish_send(context, conn, &request, &mut permit, &unbounded(), || {
            messages::MAX_BODY_BYTES
        })
    })?;
    let id = match result {
        CommandResult::MessageSent(id) => id,
        other => {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                format!("unexpected send result {other:?}"),
            ));
        }
    };
    // Drain the send worker's job here (a running daemon's worker may finish
    // it first; that is the same projection).
    drain_job(context, conn, &format!("work:send:{}", id.as_str()))?;
    Ok(id)
}

/// ACK `messages` as `to` through the real ACK writer (fresh permit per
/// attempt, retrying only `retryable` outcomes); every message must be newly
/// acknowledged.
fn ack_all(
    context: &StoreContext,
    conn: &mut Connection,
    to: &Actor,
    messages: &[MessageId],
    operation: &str,
) -> Result<(), ApiError> {
    let request = Ack {
        messages: messages.to_vec(),
        operation: OperationId::new(operation),
        claim: to.claim(),
    };
    let digest = schema::canonical_digest(&receipts::ack_payload(&request))?;
    let result = with_retries(|| {
        let mut permit = to.permit(
            context,
            conn,
            &request.operation,
            ObligationRef::CheckIn(to.seat.clone()),
            digest,
        )?;
        receipts::ack(context, conn, &unbounded(), &request, &mut permit)
    })?;
    match result {
        CommandResult::Acknowledged(result) if result.acknowledged.len() == messages.len() => {
            Ok(())
        }
        other => Err(api_error(
            ErrorCode::StoreCorrupt,
            format!("unexpected ACK result {other:?}"),
        )),
    }
}

/// ACK `messages` as `seat` through the real ACK writer (fresh cooperative
/// permit per attempt, retrying only `retryable` outcomes) and return the
/// writer's result unchanged, so a caller can assert a repeat or partial ACK.
pub fn ack_as(
    context: &StoreContext,
    conn: &mut Connection,
    seat: &str,
    messages: &[MessageId],
    operation: &str,
) -> Result<CommandResult, ApiError> {
    let to = actor(conn, seat)?;
    let request = Ack {
        messages: messages.to_vec(),
        operation: OperationId::new(operation),
        claim: to.claim(),
    };
    let digest = schema::canonical_digest(&receipts::ack_payload(&request))?;
    with_retries(|| {
        let mut permit = to.permit(
            context,
            conn,
            &request.operation,
            ObligationRef::CheckIn(to.seat.clone()),
            digest,
        )?;
        receipts::ack(context, conn, &unbounded(), &request, &mut permit)
    })
}

/// `count` require-ack sends from `sender` in `thread` (whose joined members
/// other than the sender are the recipients), each published and projected by
/// the send worker, and ACKed by `recipient` in batches of 100 right after each
/// batch is sent (well inside the receipt duration, so no overdue warning is
/// published even when a daemon's due scanner runs meanwhile). Returns the
/// message IDs. `tag` keeps operation keys unique across calls.
pub fn write_acked_history(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    sender: &str,
    recipient: &str,
    count: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    let from = actor(conn, sender)?;
    let to = actor(conn, recipient)?;
    let mut ids = Vec::with_capacity(count as usize);
    let mut batch = 0u64;
    while (ids.len() as u64) < count {
        let chunk = (count - ids.len() as u64).min(100);
        let sent = (0..chunk)
            .map(|n| {
                send_one(
                    context,
                    conn,
                    &from,
                    thread,
                    &format!("{tag}-send-{}", ids.len() as u64 + n),
                    None,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        ack_all(context, conn, &to, &sent, &format!("{tag}-ack-{batch}"))?;
        ids.extend(sent);
        batch += 1;
    }
    Ok(ids)
}

/// Settled warning history (wave-2 (a) scale probe): `count` receipts of
/// `recipient` made overdue exactly as `write_overdue_sends` does, each batch
/// of 100 then ACKed by `recipient` through the real ACK writer, which settles
/// both the receipts and their receipt-overdue warnings. Returns the message
/// IDs; every one has an overdue warning and an ACKed receipt, or an error is
/// returned.
pub fn write_settled_warning_history(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    sender: &str,
    recipient: &str,
    count: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    let to = actor(conn, recipient)?;
    let mut ids = Vec::with_capacity(count as usize);
    let mut batch = 0u64;
    while (ids.len() as u64) < count {
        let chunk = (count - ids.len() as u64).min(100);
        let sent = write_overdue_sends(
            context,
            conn,
            thread,
            sender,
            recipient,
            chunk,
            &format!("{tag}-{batch}"),
        )?;
        ack_all(
            context,
            conn,
            &to,
            &sent,
            &format!("{tag}-warned-ack-{batch}"),
        )?;
        ids.extend(sent);
        batch += 1;
    }
    Ok(ids)
}

/// At most 100 require-ack sends from `sender` in `thread`, each with a 1 ms
/// receipt duration, sent, published and projected by the send worker; the
/// receipt due scanner (`receipts::scan_due`, the daemon's production writer)
/// then publishes one receipt-overdue warning per receipt of `recipient` (the
/// thread's members at the event receive it), and the warning attribution
/// worker projects it. The receipts and warnings are left pending.
/// `recipient` must be available (registered) so that its receipts have a
/// deadline. Returns the message IDs; every one has an overdue warning, or an
/// error is returned.
pub fn write_overdue_sends(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    sender: &str,
    recipient: &str,
    count: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    if count > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "at most 100 overdue sends per call",
        ));
    }
    let from = actor(conn, sender)?;
    let to = actor(conn, recipient)?;
    let sent = (0..count)
        .map(|n| {
            send_one(
                context,
                conn,
                &from,
                thread,
                &format!("{tag}-overdue-{n}"),
                Some(1),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Every receipt must carry its overdue warning (a running daemon's due
    // scanner may publish some first: the same writer).
    let warning_of = |conn: &Connection, id: &MessageId| -> Result<Option<String>, ApiError> {
        Ok(conn
            .query_row(
                "SELECT warning_message_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
                [id.as_str(), to.seat.as_str()],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(store_error)?
            .flatten())
    };
    let mut scans = 0u32;
    loop {
        let mut missing = None;
        for id in &sent {
            if warning_of(conn, id)?.is_none() {
                missing = Some(id.clone());
                break;
            }
        }
        let Some(missing) = missing else { break };
        if scans >= RETRIES {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                format!("receipt never became overdue: {}", missing.as_str()),
            ));
        }
        scans += 1;
        // The 1 ms durations must elapse on the store clock first.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let mut cursor = receipts::ReceiptDueCursor::default();
        while receipts::scan_due(context, conn, 100, &mut cursor)?.more {}
    }
    for id in &sent {
        let warning = warning_of(conn, id)?
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "overdue warning vanished"))?;
        drain_job(context, conn, &format!("work:{warning}"))?;
    }
    Ok(sent)
}

/// `count` require-ack sends from `sender` in `thread`, published and
/// projected through the real writers and left pending.
pub fn write_pending_sends(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    sender: &str,
    count: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    let from = actor(conn, sender)?;
    (0..count)
        .map(|n| {
            send_one(
                context,
                conn,
                &from,
                thread,
                &format!("{tag}-pending-{n}"),
                None,
            )
        })
        .collect()
}

/// `count` require-ack sends from `sender` in `thread`, published and
/// projected through the real writers and left pending, each with a receipt
/// duration of `deadline_millis` (a long one keeps a running daemon's due
/// scanner from publishing overdue warnings while the history is written).
pub fn write_pending_sends_with_deadline(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    sender: &str,
    count: u64,
    deadline_millis: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    let from = actor(conn, sender)?;
    (0..count)
        .map(|n| {
            send_one(
                context,
                conn,
                &from,
                thread,
                &format!("{tag}-pending-{n}"),
                Some(deadline_millis),
            )
        })
        .collect()
}

/// `count` threads created by `inviter` through the real thread writer
/// (`control::create_thread`), each followed by a pending invitation of
/// `invitee` through the real invite writer (`control::invite`), with an
/// invitation duration of `deadline_millis`. Returns the invitation IDs.
pub fn write_pending_invitations(
    context: &StoreContext,
    conn: &mut Connection,
    inviter: &str,
    invitee: &str,
    count: u64,
    deadline_millis: u64,
    tag: &str,
) -> Result<Vec<String>, ApiError> {
    use crate::protocol::commands::{CreateThread, Invite};
    use crate::store::control;
    let from = actor(conn, inviter)?;
    let mut invitations = Vec::with_capacity(count as usize);
    for n in 0..count {
        let create = CreateThread {
            name: None,
            topic: format!("{tag} {n}"),
            goal: String::new(),
            operation: OperationId::new(format!("{tag}-thread-{n}")),
            claim: from.claim(),
        };
        let digest = control::cooperative_payload_hash("create_thread", &create)?;
        let thread = match with_retries(|| {
            let permit = from.permit(
                context,
                conn,
                &create.operation,
                ObligationRef::CheckIn(from.seat.clone()),
                digest,
            )?;
            control::create_thread(context, conn, &unbounded(), &create, permit)
        })? {
            CommandResult::ThreadCreated(thread) => thread,
            other => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    format!("unexpected thread result {other:?}"),
                ));
            }
        };
        let invite = Invite {
            thread: thread.clone(),
            seat: SeatId::new(invitee),
            deadline_millis: Some(deadline_millis),
            operation: OperationId::new(format!("{tag}-invite-{n}")),
            claim: from.claim(),
        };
        let digest = control::cooperative_payload_hash("invite", &invite)?;
        match with_retries(|| {
            let permit = from.permit(
                context,
                conn,
                &invite.operation,
                ObligationRef::Control(invite.thread.clone()),
                digest,
            )?;
            control::invite(context, conn, &unbounded(), &invite, permit, None)
        })? {
            CommandResult::Invitation(id) => invitations.push(id.as_str().to_owned()),
            other => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    format!("unexpected invite result {other:?}"),
                ));
            }
        }
    }
    Ok(invitations)
}

/// Accepts every connection for the fixture's service author; the store's
/// own decision transaction still takes the database write lock first.
struct FixtureGate(crate::protocol::ids::ServiceAuthorId);
struct FixtureGuard(crate::protocol::ids::ServiceAuthorId);
impl crate::ports::ServiceDecisionGuard for FixtureGuard {
    fn author(&self) -> &crate::protocol::ids::ServiceAuthorId {
        &self.0
    }
}
impl crate::ports::ServiceAuthorityGate for FixtureGate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: crate::protocol::ids::ServiceAuthorId,
    ) -> Result<crate::ports::ServiceConnectionAuthority, ApiError> {
        Ok(crate::ports::ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            1,
            author,
        ))
    }
    fn decision_guard<'a>(
        &'a self,
        _: &crate::ports::ServiceWriteTransactionProof,
        connection: &crate::ports::ServiceConnectionAuthority,
    ) -> Result<Box<dyn crate::ports::ServiceDecisionGuard + 'a>, ApiError> {
        if connection.author() != &self.0 {
            return Err(api_error(ErrorCode::Unauthorized, "wrong service"));
        }
        Ok(Box::new(FixtureGuard(self.0.clone())))
    }
    fn revoke_exact(&self, _: &crate::ports::ServiceConnectionAuthority) -> bool {
        true
    }
}

/// `count` programmatic service `warn` notices in `thread`, each staged
/// (`prepare_notify_step_internal`), published
/// (`publish_notify_internal`) and projected to its recipients by the
/// send-attention worker (`advance_work`), all through the production
/// writers, from the instance's reserved service author. Every joined member
/// of `thread` receives each notice. Returns the notice message IDs.
pub fn write_programmatic_warnings(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    count: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    publish_programmatic_warnings(context, conn, thread, count, tag, true)
}

/// As `write_programmatic_warnings`, but the recipient projection work of
/// each notice is left outstanding (its staged recipients are the only path
/// to it, as before the send-attention worker runs).
pub fn write_unprojected_programmatic_warnings(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    count: u64,
    tag: &str,
) -> Result<Vec<MessageId>, ApiError> {
    publish_programmatic_warnings(context, conn, thread, count, tag, false)
}

fn publish_programmatic_warnings(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &str,
    count: u64,
    tag: &str,
    drain: bool,
) -> Result<Vec<MessageId>, ApiError> {
    use crate::protocol::service::{NotificationSeverity, ServiceNotify, ServiceResult};
    use crate::store::{service_events, service_substrate};
    let (instance, boot): (String, String) = conn
        .query_row(
            "SELECT t.instance_id,h.host_boot FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1",
            [thread],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let author = {
        let tx = conn.transaction().map_err(store_error)?;
        let author =
            service_substrate::ensure_reserved_author(&tx, &instance, context.clock().utc_now())?;
        tx.commit().map_err(store_error)?;
        author
    };
    let gate = FixtureGate(author.clone());
    let connection = crate::ports::ServiceConnectionAuthority::new(instance, boot, 1, author);
    let mut notices = Vec::with_capacity(count as usize);
    for n in 0..count {
        let request = ServiceNotify {
            thread: ThreadId::new(thread),
            severity: NotificationSeverity::Warn,
            event_json: serde_json::json!({"text": format!("{tag} notice {n}")}),
            operation: OperationId::new(format!("{tag}-notify-{n}")),
        };
        loop {
            match with_retries(|| {
                match service_events::prepare_notify_step_internal(
                    context,
                    conn,
                    &connection,
                    &gate,
                    &request,
                    &unbounded(),
                    16,
                )? {
                    service_events::PrepareProgress::Step(step) => Ok(step),
                    service_events::PrepareProgress::AudienceDrift => Err(api_error(
                        ErrorCode::Conflict,
                        "notification audience changed; retry preparation",
                    )),
                }
            })? {
                service_events::PreparationStep::More { .. } => {}
                service_events::PreparationStep::Ready { .. } => break,
                service_events::PreparationStep::Committed(_) => {
                    return Err(api_error(
                        ErrorCode::Conflict,
                        "history notification operation key reused",
                    ));
                }
            }
        }
        let message = match with_retries(|| {
            match service_events::publish_notify_internal(
                context,
                conn,
                &connection,
                &gate,
                &request,
                &unbounded(),
            )? {
                service_events::PublishProgress::Committed(result) => Ok(result),
                service_events::PublishProgress::AudienceDrift => Err(api_error(
                    ErrorCode::Conflict,
                    "notification audience changed",
                )),
            }
        })? {
            ServiceResult::Notification(notice) => notice.summary.message,
            other => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    format!("unexpected notification result {other:?}"),
                ));
            }
        };
        if drain {
            let preparation =
                crate::protocol::ids::notification_preparation_id_for_message(message.as_str())
                    .ok_or_else(|| {
                        api_error(ErrorCode::StoreCorrupt, "invalid notification message ID")
                    })?;
            drain_job(context, conn, &format!("work:service-notify:{preparation}"))?;
        }
        notices.push(message);
    }
    Ok(notices)
}

/// Drain the recipient projection job of programmatic notice `notice`
/// through the production worker step.
pub fn drain_programmatic_projection(
    context: &StoreContext,
    conn: &mut Connection,
    notice: &MessageId,
) -> Result<(), ApiError> {
    let preparation =
        crate::protocol::ids::notification_preparation_id_for_message(notice.as_str())
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid notification message ID"))?;
    drain_job(context, conn, &format!("work:service-notify:{preparation}"))
}
