//! Bounded, read-only SQLite queries.

use super::{
    connection::{QueryConnection, StoreContext, api_error, store_error},
    effective::{
        EffectiveReceiptState, EffectiveTimelineEntry, GlobalLogicalKinds, GlobalLogicalPosition,
        InboxCapture, InboxValidation, ReceiptScanPosition, ReceiptScanScope, TimelinePosition,
        WarningRecipientPosition, capture_inbox_token, effective_warning_by_id,
        scan_effective_receipts, scan_effective_timeline, scan_effective_warning_recipients,
        scan_effective_warnings_for_seat, scan_global_logical_candidates, validate_inbox_token,
    },
    page_fit::{FitStop, PageFit},
};
use crate::ports::{OperationReadScope, RetirementSummary};
use crate::protocol::{
    commands::{
        Command, DeliveryInspectQuery, DiagnosticsQuery, DirectoryMembership, DirectoryQuery,
        FULL_BODY_FETCH_BYTES, HistoryRange, InboxQuery, MessageQuery, OperationStatusQuery,
        ParticipantsQuery, RecipientsQuery, RetirementJobsQuery, SearchQuery, SeatInspectQuery,
        SeatsQuery, ThreadQuery, WarningsQuery,
    },
    ids::{
        ExecutionId, HostBootId, HostTargetId, MessageId, NativeSessionId, RetirementJobId, SeatId,
        TerminalId, ThreadId,
    },
    output::{
        OutputFormat, OutputSpec, PREVIEW_SNIPPET_CHARS, encode_selected, is_inlined_full_body,
    },
    pagination::{
        Consistency, Cursor, CursorDirection, CursorScope, Page, PageRequest, SearchCursorState,
        SearchPhase, StopReason,
    },
    results::{
        AckProvenance, ApiError, BindingHistory, BoundedError, CleanupState, CommandResult,
        ContinuityStatus, DeliveryAggregates, DeliveryInspection, Diagnostic, ErrorCode,
        HoldSummary, InboxItem, InvitationAcceptance, MappingStatus, MembershipStatus,
        MessageContent, MessageDetails, MessageKind, MessageSummary, OpenBindingSummary,
        OperationStatus, Participant, PendingReceipt, ReceiptStatus, Recipient, RepairHistory,
        RetirementStatus, SearchHit, SearchPage, SeatHistoryItem, SeatInspection, SeatSummary,
        StructuredEvent, ThreadDetails, ThreadSummary, WarningRecipient, WarningRef,
    },
    service::EventAuthor,
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

const CANDIDATE_LIMIT: usize = 100;

/// Each EXISTS stops at its first matching indexed row. The failed subset has
/// its own partial index, so a long healthy pending queue is never traversed.
pub fn retirement_summary(
    store: &StoreContext,
    instance: &str,
    budget: &CallBudget,
) -> Result<RetirementSummary, ApiError> {
    retirement_summary_with_observers(store, instance, budget, || {}, || {})
}

#[cfg(test)]
fn retirement_summary_with_observer(
    store: &StoreContext,
    instance: &str,
    budget: &CallBudget,
    after_pending: impl FnOnce(),
) -> Result<RetirementSummary, ApiError> {
    retirement_summary_with_observers(store, instance, budget, after_pending, || {})
}

fn retirement_summary_with_observers(
    store: &StoreContext,
    instance: &str,
    budget: &CallBudget,
    after_pending: impl FnOnce(),
    after_commit: impl FnOnce(),
) -> Result<RetirementSummary, ApiError> {
    let db = store.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|error| db.map_error(error))?;
    let pending = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM retirements r INDEXED BY retirements_status_ordinal JOIN seats s ON s.id=r.seat_id WHERE r.status='pending' AND s.instance_id=?1)",
        [instance],
        |row| row.get::<_, bool>(0),
    ).map_err(|error| db.map_error(error))?;
    after_pending();
    db.check_budget()?;
    let degraded = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM retirements r INDEXED BY retirements_failed_pending JOIN seats s ON s.id=r.seat_id WHERE r.status='pending' AND r.last_error IS NOT NULL AND s.instance_id=?1)",
        [instance],
        |row| row.get::<_, bool>(0),
    ).map_err(|error| db.map_error(error))?;
    db.execute_batch("COMMIT")
        .map_err(|error| db.map_error(error))?;
    after_commit();
    db.check_budget()?;
    Ok(RetirementSummary { pending, degraded })
}

/// Unresolved seats for Health: an indexed count over
/// `seats_instance_state_ordinal` (proportional to unresolved seats only)
/// and the oldest few by ordinal, in one read snapshot.
pub fn unresolved_seat_summary(
    store: &StoreContext,
    instance: &str,
    budget: &CallBudget,
) -> Result<crate::ports::UnresolvedSeatSummary, ApiError> {
    use crate::ports::UnresolvedReason;
    use crate::ports::{UNRESOLVED_SEAT_SAMPLE, UnresolvedSeatSample, UnresolvedSeatSummary};
    let db = store.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|error| db.map_error(error))?;
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM seats INDEXED BY seats_instance_state_ordinal WHERE instance_id=?1 AND state='unresolved'",
            [instance],
            |row| row.get(0),
        )
        .map_err(|error| db.map_error(error))?;
    db.check_budget()?;
    let mut sample = Vec::new();
    {
        let mut stmt = db
            .prepare(
                "SELECT id,target_id,unresolved_reason FROM seats INDEXED BY seats_instance_state_ordinal WHERE instance_id=?1 AND state='unresolved' ORDER BY ordinal LIMIT ?2",
            )
            .map_err(|error| db.map_error(error))?;
        let rows = stmt
            .query_map(params![instance, UNRESOLVED_SEAT_SAMPLE as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| db.map_error(error))?;
        for row in rows {
            let (seat, target, reason) = row.map_err(|error| db.map_error(error))?;
            sample.push(UnresolvedSeatSample {
                seat: SeatId::new(seat),
                target: target.map(HostTargetId::new),
                reason: match reason.as_deref() {
                    Some("host_invalidation") => Some(UnresolvedReason::HostInvalidation),
                    Some("other") => Some(UnresolvedReason::Other),
                    _ => None,
                },
            });
        }
    }
    db.execute_batch("COMMIT")
        .map_err(|error| db.map_error(error))?;
    db.check_budget()?;
    Ok(UnresolvedSeatSummary {
        count: u64::try_from(count).unwrap_or(0),
        sample,
    })
}

/// Run a single bounded read. The query connection and its SQLite snapshot are
/// dropped before this function returns.
pub fn query(
    store: &StoreContext,
    instance: &str,
    command: &Command,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    query_with_output(store, instance, command, &OutputSpec::default(), budget)
}

pub fn query_with_output(
    store: &StoreContext,
    instance: &str,
    command: &Command,
    output: &OutputSpec,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    command
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    output
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let active_budget = if matches!(command, Command::Search(_)) {
        CallBudget {
            deadline: MonoInstant(
                budget
                    .deadline
                    .0
                    .min(store.clock().monotonic_now().0.saturating_add(100)),
            ),
            cancellation: budget.cancellation.clone(),
        }
    } else {
        budget.clone()
    };
    let db = store
        .open_query(active_budget.clone())
        .map_err(|error| query_error(error, &active_budget, store.clock(), command, output))?;
    db.execute_batch("BEGIN DEFERRED").map_err(|error| {
        query_error(
            db.map_error(error),
            &active_budget,
            store.clock(),
            command,
            output,
        )
    })?;
    let result = match command {
        Command::History(q) => history(&db, instance, q, output),
        Command::PendingReceipts(q) => {
            pending_receipts(&db, instance, q, store.clock().utc_now(), output)
        }
        Command::Message(q) => message(&db, instance, q, output),
        Command::Search(q) => search(&db, store, instance, q, output, &active_budget),
        Command::Directory(q) => directory(&db, instance, q, output),
        Command::Participants(q) => participants(&db, instance, q, output),
        Command::Seats(q) => seats(&db, instance, q, output),
        Command::Diagnostics(q) => diagnostics(&db, instance, q, store.clock().utc_now(), output),
        Command::Recipients(q) => recipients(&db, instance, q, store.clock().utc_now(), output),
        Command::SeatInspect(q) => seat_inspect(&db, instance, q, output),
        Command::DeliveryInspect(q) => {
            delivery_inspect(&db, instance, q, store.clock().utc_now(), output)
        }
        Command::Thread(q) => thread_details(&db, instance, q, output),
        Command::RetirementJobs(q) => retirement_jobs(&db, instance, q, output),
        Command::Warnings(q) => warnings(&db, instance, q, output),
        Command::Inbox(q) => inbox(&db, store, instance, q, output, budget),
        Command::AttentionDigest(q) => {
            super::attention::seat_digest(&db, instance, &q.seat, &|| db.check_budget())
                .map(|run| CommandResult::AttentionDigest(run.digest))
        }
        _ => Err(api_error(
            ErrorCode::Unsupported,
            "query route not implemented",
        )),
    };
    result.map_err(|error| query_error(error, &active_budget, store.clock(), command, output))
}

/// The author-scoped `DeliveryInspect` behind the service `Receipts` read: the
/// message must be an ordinary message authored by `author` (a native message,
/// a notification or another author's message is `InvalidRequest`). The
/// ownership check and the inspection share one read transaction; nothing is
/// written and no writer or lane turn is taken.
pub fn service_receipts(
    store: &StoreContext,
    instance: &str,
    author: &crate::protocol::ids::ServiceAuthorId,
    q: &DeliveryInspectQuery,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    let command = Command::DeliveryInspect(q.clone());
    let output = OutputSpec::default();
    command
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let db = store
        .open_query(budget.clone())
        .map_err(|error| query_error(error, budget, store.clock(), &command, &output))?;
    db.execute_batch("BEGIN DEFERRED").map_err(|error| {
        query_error(
            db.map_error(error),
            budget,
            store.clock(),
            &command,
            &output,
        )
    })?;
    let result = (|| {
        if !message_owned(&db, instance, &q.message)? {
            return Err(api_error(ErrorCode::NotFound, "message not found"));
        }
        let kind: String = db
            .query_row(
                "SELECT kind FROM messages WHERE id=?1",
                [q.message.as_str()],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        let own = kind == "ordinary"
            && matches!(
                super::service_substrate::message_author(&db, &q.message)?,
                crate::protocol::service::EventAuthor::Programmatic(a) if a == *author
            );
        if !own {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "message is not authored by this service",
            ));
        }
        delivery_inspect(&db, instance, q, store.clock().utc_now(), &output)
    })();
    result.map_err(|error| query_error(error, budget, store.clock(), &command, &output))
}

fn query_error(
    error: ApiError,
    budget: &CallBudget,
    clock: &dyn Clock,
    command: &Command,
    output: &OutputSpec,
) -> ApiError {
    let mut mapped = read_budget_error(error, budget, clock);
    if mapped.code == ErrorCode::ReadBudgetExhausted
        && let Command::Search(q) = command
    {
        mapped.restart_argv = Some(contextual_argv(
            search_request_argv(q, q.page.cursor.as_deref()),
            output,
        ));
    }
    mapped
}

fn read_budget_error(error: ApiError, budget: &CallBudget, clock: &dyn Clock) -> ApiError {
    if budget.cancellation.is_cancelled() {
        api_error(ErrorCode::Cancelled, "read cancelled")
    } else if budget.deadline_passed(clock) || error.code == ErrorCode::DeadlineExceeded {
        api_error(ErrorCode::ReadBudgetExhausted, "bounded read exhausted")
    } else {
        error
    }
}

/// The facade supplies the actor scope from verified authority. It must never
/// be read from the wire command or inferred from the operation key.
pub fn query_operation_status(
    store: &StoreContext,
    instance: &str,
    actor_scope: &OperationReadScope,
    q: &OperationStatusQuery,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    if instance.is_empty() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "trusted actor scope and instance required",
        ));
    }
    let db = store.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|e| db.map_error(e))?;
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM host_instances WHERE id=?1)",
            [instance],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "instance not found"));
    }
    if let OperationReadScope::Seat(seat) = actor_scope {
        let seat_owned: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
                params![seat.as_str(), instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        if !seat_owned {
            return Err(api_error(
                ErrorCode::Unauthorized,
                "seat does not belong to instance",
            ));
        }
    }
    let actor_scope = actor_scope.actor_scope(instance);
    let json: Option<String> = db
        .query_row(
            "SELECT result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![actor_scope, q.operation.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db.map_error(e))?;
    let result_id = json
        .as_deref()
        .map(|value| {
            let result: CommandResult = serde_json::from_str(value).map_err(|e| {
                api_error(
                    ErrorCode::StoreCorrupt,
                    format!("invalid stored result: {e}"),
                )
            })?;
            let id = match result {
                CommandResult::ThreadCreated(v)
                | CommandResult::Left(v)
                | CommandResult::TopicChanged(v)
                | CommandResult::Archived(v)
                | CommandResult::Reopened(v) => Some(v.as_str().to_owned()),
                CommandResult::MessageSent(v) => Some(v.as_str().to_owned()),
                CommandResult::Invitation(v) | CommandResult::OperatorInvited(v) => {
                    Some(v.as_str().to_owned())
                }
                CommandResult::Accepted(v) => Some(v.invitation.as_str().to_owned()),
                CommandResult::SeatResolved(v)
                | CommandResult::ContinuityReattached(
                    crate::protocol::results::ContinuityReattachment { seat: v, .. },
                )
                | CommandResult::OperatorRebound(v)
                | CommandResult::OperatorRetired(v)
                | CommandResult::OperatorFreshSeat(v) => Some(v.as_str().to_owned()),
                _ => None,
            };
            Ok::<_, ApiError>(id)
        })
        .transpose()?
        .flatten();
    Ok(CommandResult::OperationStatus(OperationStatus {
        operation: q.operation.clone(),
        committed: json.is_some(),
        result_id,
    }))
}

fn retirement_jobs(
    db: &QueryConnection,
    instance: &str,
    q: &RetirementJobsQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let filter = digest(&"retirement-jobs")?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::RetirementJobs,
        "*",
        &filter,
        CursorDirection::Ascending,
    )?;
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM retirements",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(|e| db.map_error(e))
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut last = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut examined = 0usize;
    let mut stop = StopReason::Complete;
    while examined < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        type Row = (
            i64,
            String,
            String,
            i64,
            String,
            i64,
            String,
            Option<String>,
            String,
        );
        let row:Option<Row>=db.query_row("SELECT r.ordinal,r.id,r.seat_id,r.cutover_at,r.phase,r.processed_units,r.status,r.last_error,s.instance_id FROM retirements r JOIN seats s ON s.id=r.seat_id WHERE r.ordinal>?1 AND r.ordinal<=?2 ORDER BY r.ordinal LIMIT 1",params![last as i64,high as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|e|db.map_error(e))?;
        let Some((ordinal, id, seat, at, phase, processed, status, error, owner)) = row else {
            break;
        };
        examined += 1;
        if owner == instance {
            let cleanup_state = if status == "complete" {
                CleanupState::Complete
            } else if error.is_some() {
                CleanupState::Failed
            } else if processed > 0 {
                CleanupState::Running
            } else {
                CleanupState::Pending
            };
            let item = RetirementStatus {
                job: RetirementJobId::new(id),
                seat: SeatId::new(seat),
                effective_retired: true,
                retired_at: UtcMillis(at),
                cleanup_state,
                warning_history_complete: status == "complete",
                phase,
                processed_units: processed as u64,
                remaining_estimate: None,
                last_error: error
                    .map(BoundedError::parse)
                    .transpose()
                    .map_err(|e| api_error(ErrorCode::StoreCorrupt, e))?,
            };
            let raw = cursor_for(
                instance,
                CursorScope::RetirementJobs,
                "*",
                &filter,
                CursorDirection::Ascending,
                ordinal as u64,
                high,
            )
            .encode()
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
            items.push(Cand {
                item,
                argv: retirement_jobs_argv(&raw, &q.page),
                raw,
                before: last,
            });
        }
        last = ordinal as u64;
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        q.page.max_bytes,
        "retirement job cannot fit",
        |page, _| CommandResult::RetirementJobs(page),
    )?;
    if let Some(before) = cut {
        last = before;
        stop = StopReason::Bytes;
    }
    if examined == CANDIDATE_LIMIT && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = cursor_for(
            instance,
            CursorScope::RetirementJobs,
            "*",
            &filter,
            CursorDirection::Ascending,
            last,
            high,
        )
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((raw.clone(), retirement_jobs_argv(&raw, &q.page)))
    };
    Ok(CommandResult::RetirementJobs(sized(
        items,
        next,
        high,
        stop,
        CommandResult::RetirementJobs,
        output,
        q.page.max_bytes,
    )?))
}

fn thread_details(
    db: &QueryConnection,
    instance: &str,
    q: &ThreadQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    type Row = (String, String, i64, i64, i64);
    let (topic,goal,archived,created_at,next_sequence):Row=db.query_row("SELECT topic,goal,archived,created_at,next_sequence FROM threads WHERE id=?1 AND instance_id=?2",params![q.thread.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|db.map_error(e))?.ok_or_else(||api_error(ErrorCode::NotFound,"thread not found"))?;
    let summary = thread_summary(
        db,
        q.thread.as_str(),
        &topic,
        archived,
        created_at,
        next_sequence,
    )?;
    let participant_count: i64 = db
        .query_row(
            "SELECT count(*) FROM memberships m WHERE m.thread_id=?1
             AND (coalesce(m.voluntary_state,m.state)!='absent' OR EXISTS (
                SELECT 1 FROM requirement_episodes r WHERE r.thread_id=m.thread_id
                AND r.seat_id=m.seat_id AND r.state IN ('pending','accepted')))",
            [q.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?;
    let manifest_pending:i64=db.query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id JOIN seats s ON s.id=pr.seat_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.thread_id=?1 AND pr.ack_required=1 AND COALESCE(rs.ack_required,1)=1 AND s.state!='retired' AND (rs.state IS NULL OR rs.state='pending') AND pr.availability_provenance IS NOT 'operator_human' AND NOT EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=pr.seat_id AND b.harness='human' AND b.ended_at IS NULL) AND NOT EXISTS(SELECT 1 FROM human_receipt_waivers w WHERE w.seat_id=pr.seat_id AND sm.decision_seq<=w.through_decision_seq)",[q.thread.as_str()],|r|r.get(0)).map_err(|e|db.map_error(e))?;
    let physical_pending:i64=db.query_row("SELECT count(*) FROM receipts r JOIN seats s ON s.id=r.seat_id JOIN messages m ON m.id=r.message_id WHERE r.thread_id=?1 AND r.state='pending' AND r.ack_required=1 AND s.state!='retired' AND NOT EXISTS(SELECT 1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=r.message_id AND pr.seat_id=r.seat_id) AND NOT EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=r.seat_id AND b.harness='human' AND b.ended_at IS NULL) AND NOT EXISTS(SELECT 1 FROM human_receipt_waivers w WHERE w.seat_id=r.seat_id AND (m.decision_seq IS NULL OR m.decision_seq<=w.through_decision_seq))",[q.thread.as_str()],|r|r.get(0)).map_err(|e|db.map_error(e))?;
    let pending_receipt_count = manifest_pending
        .checked_add(physical_pending)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "pending count overflow"))?
        as u64;
    let pq = ParticipantsQuery {
        thread: q.thread.clone(),
        page: q.page.clone(),
        caller: q.caller.clone(),
    };
    let CommandResult::Participants(participants) = participants(db, instance, &pq, output)? else {
        unreachable!()
    };
    let followup = contextual_argv(
        vec![
            "herdr-threads".into(),
            "pending-receipts".into(),
            "--thread".into(),
            q.thread.as_str().into(),
        ],
        output,
    );
    let details = |participants: Page<Participant>| {
        CommandResult::Thread(ThreadDetails {
            summary: summary.clone(),
            goal_data: goal.clone(),
            created_at: UtcMillis(created_at),
            participant_count: participant_count as u64,
            participants,
            pending_receipt_count,
            pending_receipts_argv: followup.clone(),
        })
    };
    let with_argv = |mut page: Page<Participant>| {
        if let Some(raw) = page.next_cursor.as_deref() {
            page.next_argv = Some(contextual_argv(
                vec![
                    "herdr-threads".into(),
                    "thread".into(),
                    "show".into(),
                    q.thread.as_str().into(),
                    "--cursor".into(),
                    raw.into(),
                    "--limit".into(),
                    q.page.limit.to_string(),
                    "--max-bytes".into(),
                    q.page.max_bytes.to_string(),
                ],
                output,
            ));
        }
        page
    };
    let max = q.page.max_bytes as usize;
    let whole = details(with_argv(participants.clone()));
    let required = encode_selected(&whole, output)?.len();
    if required <= max {
        return Ok(whole);
    }
    if participants.items.is_empty() {
        return Err(ApiError::invalid_budget("thread detail cannot fit")
            .with_required_minimum_bytes(required as u32));
    }
    // The embedded participants page overflows the details envelope: cut it
    // back through the page fit, each cut page continuing from its last item.
    let filter = digest(&q.thread.as_str())?;
    let (member_rev,lifecycle_rev):(i64,i64)=db.query_row("SELECT t.membership_revision,h.lifecycle_revision FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1",[q.thread.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|db.map_error(e))?;
    let start_after = q
        .page
        .cursor
        .as_ref()
        .map(|raw| {
            Cursor::decode(raw)
                .map(|c| c.after_ordinal)
                .map_err(|e| api_error(ErrorCode::InvalidCursor, e))
        })
        .transpose()?
        .unwrap_or(0);
    let mut ordinals = Vec::with_capacity(participants.items.len());
    for item in &participants.items {
        ordinals.push(
            db.query_row(
                "SELECT ordinal FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                params![q.thread.as_str(), item.seat.as_str()],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| db.map_error(e))? as u64,
        );
    }
    let cut_page = |items: &[Participant], after: u64| -> Result<Page<Participant>, ApiError> {
        let mut cursor = cursor_for(
            instance,
            CursorScope::Participants,
            q.thread.as_str(),
            &filter,
            CursorDirection::Ascending,
            after,
            participants.high_water_ordinal,
        );
        cursor.scope_revision = Some(member_rev as u64);
        cursor.filter_revision = Some(lifecycle_rev as u64);
        let mut page = participants.clone();
        page.items = items.to_vec();
        page.next_cursor = Some(
            cursor
                .encode()
                .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?,
        );
        page.has_more = true;
        page.stop_reason = StopReason::Bytes;
        Ok(with_argv(page))
    };
    let items = &participants.items;
    let page_of = |k: usize| -> Result<Page<Participant>, ApiError> {
        if k == items.len() {
            Ok(with_argv(participants.clone()))
        } else {
            cut_page(
                &items[..k],
                if k == 0 { start_after } else { ordinals[k - 1] },
            )
        }
    };
    let fit = PageFit::for_command(output, max).fit_with(
        items,
        |k| Ok(details(page_of(k)?)),
        |i| Ok(details(cut_page(&items[i..=i], ordinals[i])?)),
    )?;
    let result = details(page_of(fit.accepted)?);
    if fit.accepted == 0 {
        // No participant fits beside the envelope; the empty continuation page
        // must itself fit.
        let required = encode_selected(&result, output)?.len();
        if required > max {
            return Err(ApiError::invalid_budget("thread detail cannot fit")
                .with_required_minimum_bytes(required as u32));
        }
    }
    Ok(result)
}

fn seat_inspect_cursor(
    instance: &str,
    key: &str,
    filter: &str,
    after: u64,
    kind: u8,
    binding_high: u64,
    repair_high: u64,
) -> Result<String, ApiError> {
    let mut cursor = cursor_for(
        instance,
        CursorScope::SeatInspect,
        key,
        filter,
        CursorDirection::Ascending,
        after,
        binding_high.max(repair_high),
    );
    cursor.last_examined_key = Some(format!("{binding_high},{repair_high},{kind}"));
    cursor
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))
}

/// The seat's open (not ended) binding, or none: the one answer every A4
/// guard (launch, wake, cooperative check-in) reads.
pub(crate) fn open_binding(
    db: &Connection,
    seat: &str,
) -> Result<Option<OpenBindingSummary>, ApiError> {
    db.query_row(
        "SELECT observation_provenance,harness,target_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1",
        [seat],
        |r| {
            Ok(OpenBindingSummary {
                provenance: r.get(0)?,
                harness: r.get(1)?,
                target: HostTargetId::new(r.get::<_, String>(2)?),
            })
        },
    )
    .optional()
    .map_err(store_error)
}

fn seat_inspect_argv(q: &SeatInspectQuery, raw: &str) -> Vec<String> {
    vec![
        "herdr-threads".into(),
        "seat".into(),
        "inspect".into(),
        q.seat.as_str().into(),
        "--cursor".into(),
        raw.into(),
        "--limit".into(),
        q.page.limit.to_string(),
        "--max-bytes".into(),
        q.page.max_bytes.to_string(),
    ]
}

fn seat_inspect(
    db: &QueryConnection,
    instance: &str,
    q: &SeatInspectQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    type SeatRow = (String, Option<String>, i64, i64, Option<i64>);
    let row:SeatRow=db.query_row("SELECT state,target_id,generation,created_at,retired_at FROM seats WHERE id=?1 AND instance_id=?2",params![q.seat.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|db.map_error(e))?.ok_or_else(||api_error(ErrorCode::NotFound,"seat not found"))?;
    let summary = seat_summary(
        q.seat.as_str(),
        &row.0,
        row.1.as_deref(),
        row.2,
        row.3,
        row.4,
    )?;
    let mapping = MappingStatus {
        state: summary.continuity,
        target: summary.target.clone(),
        detail_argv: None,
    };
    let hold = if let Some(target) = &summary.target {
        db.query_row("SELECT reason FROM recovery_holds WHERE instance_id=?1 AND target_id=?2 AND released_at IS NULL",params![instance,target.as_str()],|r|r.get::<_,String>(0)).optional().map_err(|e|db.map_error(e))?.map(|reason|HoldSummary{target:target.clone(),reason_data:reason,detail_argv:vec!["herdr-threads".into(),"seat".into(),"inspect".into(),q.seat.as_str().into()]})
    } else {
        None
    };
    type Job = (String, i64, String, i64, String, Option<String>);
    let job:Option<Job>=db.query_row("SELECT id,cutover_at,phase,processed_units,status,last_error FROM retirements WHERE seat_id=?1",[q.seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|db.map_error(e))?;
    let retirement = job
        .map(|(id, at, phase, processed, status, error)| {
            let cleanup_state = if status == "complete" {
                CleanupState::Complete
            } else if error.is_some() {
                CleanupState::Failed
            } else if processed > 0 {
                CleanupState::Running
            } else {
                CleanupState::Pending
            };
            Ok(RetirementStatus {
                job: RetirementJobId::new(id),
                seat: q.seat.clone(),
                effective_retired: true,
                retired_at: UtcMillis(at),
                cleanup_state,
                warning_history_complete: status == "complete",
                phase,
                processed_units: processed as u64,
                remaining_estimate: None,
                last_error: error
                    .map(BoundedError::parse)
                    .transpose()
                    .map_err(|e| api_error(ErrorCode::StoreCorrupt, e))?,
            })
        })
        .transpose()?;
    let open_binding = open_binding(db, q.seat.as_str())?;
    let filter = digest(&q.seat.as_str())?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::SeatInspect,
        q.seat.as_str(),
        &filter,
        CursorDirection::Ascending,
    )?;
    let (binding_high, repair_high, mut last, mut last_kind) = if let Some(c) = cursor.as_ref() {
        let raw = c
            .last_examined_key
            .as_deref()
            .ok_or_else(|| api_error(ErrorCode::InvalidCursor, "seat history bounds missing"))?;
        let parts: Vec<_> = raw.split(',').collect();
        if parts.len() != 3 {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "seat history bounds malformed",
            ));
        }
        let parse = |s: &str| {
            s.parse::<u64>()
                .map_err(|_| api_error(ErrorCode::InvalidCursor, "seat history bounds malformed"))
        };
        let binding_high = parse(parts[0])?;
        let repair_high = parse(parts[1])?;
        let kind = parse(parts[2])?;
        if kind > 1 || c.after_ordinal > binding_high.max(repair_high) {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "seat history position invalid",
            ));
        }
        (binding_high, repair_high, c.after_ordinal, kind as u8)
    } else {
        let binding_high: i64 = db
            .query_row(
                "SELECT coalesce(max(ordinal),0) FROM occupant_bindings WHERE seat_id=?1",
                [q.seat.as_str()],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        let repair_high: i64 = db
            .query_row(
                "SELECT coalesce(max(ordinal),0) FROM allocation_decisions WHERE seat_id=?1",
                [q.seat.as_str()],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        (binding_high as u64, repair_high as u64, 0, 1)
    };
    let high = binding_high.max(repair_high);
    let mut items = Vec::new();
    let mut stop = StopReason::Complete;
    for _ in 0..CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        type BindingRow = (
            i64,
            i64,
            String,
            Option<String>,
            Option<String>,
            String,
            i64,
            String,
            String,
            i64,
            Option<i64>,
            Option<i64>,
            String,
        );
        let binding:Option<BindingRow>=db.query_row("SELECT ordinal,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,native_session,execution_id,observed_at,registered_at,ended_at,observation_provenance FROM occupant_bindings WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",params![q.seat.as_str(),last as i64,binding_high as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?,r.get(10)?,r.get(11)?,r.get(12)?))).optional().map_err(|e|db.map_error(e))?;
        type RepairRow = (
            i64,
            String,
            String,
            i64,
            String,
            i64,
            i64,
            Option<String>,
            Option<String>,
        );
        let repair:Option<RepairRow>=db.query_row("SELECT ordinal,target_id,kind,decided_at,host_boot,epoch,generation,operator_label,continuity_diagnostic FROM allocation_decisions WHERE seat_id=?1 AND (ordinal>?2 OR (ordinal=?2 AND ?4=0)) AND ordinal<=?3 ORDER BY ordinal LIMIT 1",params![q.seat.as_str(),last as i64,repair_high as i64,last_kind],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|e|db.map_error(e))?;
        let pick_binding = match (&binding, &repair) {
            (Some(b), Some(r)) => b.0 <= r.0,
            (Some(_), None) => true,
            _ => false,
        };
        if binding.is_none() && repair.is_none() {
            break;
        }
        let (ordinal, kind, item) = if pick_binding {
            let (
                ordinal,
                generation,
                target,
                terminal,
                incarnation,
                boot,
                epoch,
                native,
                execution,
                observed,
                registered,
                ended,
                provenance,
            ) = binding.unwrap();
            (
                ordinal as u64,
                0,
                SeatHistoryItem::Binding(BindingHistory {
                    ordinal: ordinal as u64,
                    generation: generation as u64,
                    target: HostTargetId::new(target),
                    terminal: terminal.map(TerminalId::new),
                    incarnation,
                    host_boot: HostBootId::new(boot),
                    host_epoch: epoch as u64,
                    native_session: NativeSessionId::new(native),
                    execution: ExecutionId::new(execution),
                    observed_at: UtcMillis(observed),
                    registered_at: registered.map(UtcMillis),
                    ended_at: ended.map(UtcMillis),
                    provenance,
                }),
            )
        } else {
            let (
                ordinal,
                target,
                decision_kind,
                decided,
                boot,
                epoch,
                generation,
                operator_label,
                continuity_diagnostic,
            ) = repair.unwrap();
            (
                ordinal as u64,
                1,
                SeatHistoryItem::Repair(RepairHistory {
                    ordinal: ordinal as u64,
                    target: HostTargetId::new(target),
                    decision_kind,
                    decided_at: UtcMillis(decided),
                    host_boot: HostBootId::new(boot),
                    host_epoch: epoch as u64,
                    generation: generation as u64,
                    operator_label,
                    continuity_diagnostic,
                }),
            )
        };
        let raw = seat_inspect_cursor(
            instance,
            q.seat.as_str(),
            &filter,
            ordinal,
            kind,
            binding_high,
            repair_high,
        )?;
        items.push(Cand {
            item,
            argv: seat_inspect_argv(q, &raw),
            raw,
            before: (last, last_kind),
        });
        last = ordinal;
        last_kind = kind;
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        q.page.max_bytes,
        "seat history item cannot fit",
        |history, _| {
            CommandResult::SeatInspect(SeatInspection {
                summary: summary.clone(),
                mapping: mapping.clone(),
                hold: hold.clone(),
                retirement: retirement.clone(),
                open_binding: open_binding.clone(),
                history,
            })
        },
    )?;
    if let Some((before_last, before_kind)) = cut {
        last = before_last;
        last_kind = before_kind;
        stop = StopReason::Bytes;
    }
    if items.len() == CANDIDATE_LIMIT && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = seat_inspect_cursor(
            instance,
            q.seat.as_str(),
            &filter,
            last,
            last_kind,
            binding_high,
            repair_high,
        )?;
        Some((raw.clone(), seat_inspect_argv(q, &raw)))
    };
    let result = CommandResult::SeatInspect(SeatInspection {
        summary,
        mapping,
        hold,
        retirement,
        open_binding,
        history: page(items, next, high, stop, output),
    });
    ensure_fit(&result, output, q.page.max_bytes)?;
    Ok(result)
}

fn seats(
    db: &QueryConnection,
    instance: &str,
    q: &SeatsQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let filter = digest(&(
        "seats",
        q.target.as_ref().map(HostTargetId::as_str),
        q.include_retired,
    ))?;
    let target = q.target.as_ref().map(|t| t.as_str().to_owned());
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::Seats,
        "*",
        &filter,
        CursorDirection::Descending,
    )?;
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM seats WHERE instance_id=?1",
                [instance],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(|e| db.map_error(e))
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut last = cursor
        .as_ref()
        .map_or_else(|| high.saturating_add(1), |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut stop = StopReason::Complete;
    type Row = (i64, String, String, Option<String>, i64, i64, Option<i64>);
    let map_row = |r: &rusqlite::Row<'_>| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        ))
    };
    // Resolve both a candidate and the final-page probe through the same
    // indexed query. A sparse pane target must not scan unrelated seat history.
    let next_row = |before: u64| -> Result<Option<Row>, ApiError> {
        let before = before as i64;
        let high = high as i64;
        let row: rusqlite::Result<Option<Row>> = match (&target, q.include_retired) {
            (None, true) => db
                .query_row(
                    "SELECT ordinal,id,state,target_id,generation,created_at,retired_at FROM seats WHERE instance_id=?1 AND ordinal<?2 AND ordinal<=?3 ORDER BY ordinal DESC LIMIT 1",
                    params![instance, before, high],
                    map_row,
                )
                .optional(),
            (None, false) => db
                .query_row(
                    "SELECT ordinal,id,state,target_id,generation,created_at,retired_at FROM seats WHERE instance_id=?1 AND state='resolved' AND ordinal<?2 AND ordinal<=?3 UNION ALL SELECT ordinal,id,state,target_id,generation,created_at,retired_at FROM seats WHERE instance_id=?1 AND state='unresolved' AND ordinal<?2 AND ordinal<=?3 ORDER BY ordinal DESC LIMIT 1",
                    params![instance, before, high],
                    map_row,
                )
                .optional(),
            (Some(target), false) => db
                .query_row(
                    "SELECT ordinal,id,state,target_id,generation,created_at,retired_at FROM seats WHERE ordinal IN (SELECT ordinal FROM seats WHERE instance_id=?1 AND target_id=?4 AND state='resolved' UNION ALL SELECT ordinal FROM seats WHERE instance_id=?1 AND state='unresolved' AND target_id=?4) AND ordinal<?2 AND ordinal<=?3 ORDER BY ordinal DESC LIMIT 1",
                    params![instance, before, high, target],
                    map_row,
                )
                .optional(),
            (Some(target), true) => db
                .query_row(
                    "SELECT ordinal,id,state,target_id,generation,created_at,retired_at FROM seats WHERE instance_id=?1 AND target_id=?4 AND ordinal<?2 AND ordinal<=?3 ORDER BY ordinal DESC LIMIT 1",
                    params![instance, before, high, target],
                    map_row,
                )
                .optional(),
        };
        row.map_err(|error| db.map_error(error))
    };
    for _ in 0..CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        let row = next_row(last)?;
        let Some((ordinal, id, state, target, generation, created_at, retired_at)) = row else {
            break;
        };
        let item = seat_summary(
            &id,
            &state,
            target.as_deref(),
            generation,
            created_at,
            retired_at,
        )?;
        let raw = cursor_for(
            instance,
            CursorScope::Seats,
            "*",
            &filter,
            CursorDirection::Descending,
            ordinal as u64,
            high,
        )
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        items.push(Cand {
            item,
            argv: seats_argv(&raw, q),
            raw,
            before: last,
        });
        last = ordinal as u64;
    }
    if stop == StopReason::Rows && next_row(last)?.is_none() {
        stop = StopReason::Complete;
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        q.page.max_bytes,
        "seat cannot fit",
        |page, _| CommandResult::Seats(page),
    )?;
    if let Some(before) = cut {
        last = before;
        stop = StopReason::Bytes;
    }
    if items.len() == CANDIDATE_LIMIT && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = cursor_for(
            instance,
            CursorScope::Seats,
            "*",
            &filter,
            CursorDirection::Descending,
            last,
            high,
        )
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((raw.clone(), seats_argv(&raw, q)))
    };
    Ok(CommandResult::Seats(sized(
        items,
        next,
        high,
        stop,
        CommandResult::Seats,
        output,
        q.page.max_bytes,
    )?))
}

fn seat_summary(
    id: &str,
    state: &str,
    target: Option<&str>,
    generation: i64,
    created_at: i64,
    retired_at: Option<i64>,
) -> Result<SeatSummary, ApiError> {
    let continuity = match state {
        "resolved" => ContinuityStatus::Resolved,
        "unresolved" => ContinuityStatus::Unresolved,
        "retired" => ContinuityStatus::Retired,
        _ => return Err(api_error(ErrorCode::StoreCorrupt, "invalid seat state")),
    };
    Ok(SeatSummary {
        seat: SeatId::new(id),
        continuity,
        target: target.map(HostTargetId::new),
        generation: generation as u64,
        created_at: UtcMillis(created_at),
        retired_at: retired_at.map(UtcMillis),
    })
}

fn participants(
    db: &QueryConnection,
    instance: &str,
    q: &ParticipantsQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let scope:(i64,i64)=db.query_row("SELECT t.membership_revision,h.lifecycle_revision FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1 AND t.instance_id=?2",params![q.thread.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|db.map_error(e))?.ok_or_else(||api_error(ErrorCode::NotFound,"thread not found"))?;
    let filter = digest(&q.thread.as_str())?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::Participants,
        q.thread.as_str(),
        &filter,
        CursorDirection::Ascending,
    )?;
    if cursor.as_ref().is_some_and(|c| {
        c.scope_revision != Some(scope.0 as u64) || c.filter_revision != Some(scope.1 as u64)
    }) {
        return Err(
            ApiError::cursor_stale("participant status changed").with_restart_argv(
                contextual_argv(
                    vec![
                        "herdr-threads".into(),
                        "thread".into(),
                        "participants".into(),
                        q.thread.as_str().into(),
                        "--limit".into(),
                        q.page.limit.to_string(),
                        "--max-bytes".into(),
                        q.page.max_bytes.to_string(),
                    ],
                    output,
                ),
            ),
        );
    }
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM memberships WHERE thread_id=?1",
                [q.thread.as_str()],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(|e| db.map_error(e))
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut last = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut examined = 0usize;
    let mut stop = StopReason::Complete;
    while examined < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        type Row = (
            i64,
            String,
            i64,
            String,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            String,
            Option<i64>,
        );
        let row:Option<Row>=db.query_row("SELECT m.ordinal,m.seat_id,m.episode,m.state,m.joined_at,m.left_at,m.retired_at,s.state,s.retired_at FROM memberships m JOIN seats s ON s.id=m.seat_id WHERE m.thread_id=?1 AND m.ordinal>?2 AND m.ordinal<=?3 ORDER BY m.ordinal LIMIT 1",params![q.thread.as_str(),last as i64,high as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|e|db.map_error(e))?;
        let Some((
            ordinal,
            seat,
            episode,
            _state,
            joined_at,
            left_at,
            materialized_retired,
            seat_state,
            retired_at,
        )) = row
        else {
            break;
        };
        examined += 1;
        let projection =
            super::service_substrate::effective_membership(db, &q.thread, &SeatId::new(&seat))?;
        let before_last = last;
        last = ordinal as u64;
        let Some(native_state) = projection.native_state else {
            continue;
        };
        let effective = membership_status(native_state)?;
        let physical = match projection.voluntary {
            crate::protocol::service::VoluntaryMembershipState::Absent => effective,
            crate::protocol::service::VoluntaryMembershipState::Invited => {
                MembershipStatus::Invited
            }
            crate::protocol::service::VoluntaryMembershipState::Joined => MembershipStatus::Joined,
            crate::protocol::service::VoluntaryMembershipState::Left => MembershipStatus::Left,
            crate::protocol::service::VoluntaryMembershipState::Retired => {
                MembershipStatus::Retired
            }
        };
        let job: Option<(String, i64, Option<String>)> = db
            .query_row(
                "SELECT status,processed_units,last_error FROM retirements WHERE seat_id=?1",
                [&seat],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| db.map_error(e))?;
        let cleanup_state = job.map(|(status, processed, error)| {
            if status == "complete" {
                CleanupState::Complete
            } else if error.is_some() {
                CleanupState::Failed
            } else if processed > 0 {
                CleanupState::Running
            } else {
                CleanupState::Pending
            }
        });
        let accepted:Option<(String,String,i64,String,i64)>=db.query_row("SELECT id,accepted_actor_seat_id,accepted_generation,accepted_observation,accepted_at FROM invitations WHERE thread_id=?1 AND seat_id=?2 AND episode=?3 AND state='accepted'",params![q.thread.as_str(),seat,episode],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|db.map_error(e))?;
        let accepted_invitation =
            accepted.map(
                |(id, actor, generation, observation, at)| InvitationAcceptance {
                    invitation: crate::protocol::ids::InvitationId::new(id),
                    actor: SeatId::new(actor),
                    generation: generation as u64,
                    native_observation: super::control::canonical_observation(&observation),
                    accepted_at: UtcMillis(at),
                },
            );
        let item = Participant {
            is_self: q
                .caller
                .as_ref()
                .is_some_and(|caller| caller.as_str() == seat),
            seat: SeatId::new(&seat),
            requirement: {
                let mut required = super::service_substrate::current_requirement(
                    db,
                    &q.thread,
                    &SeatId::new(&seat),
                )?;
                if seat_state == "retired"
                    && let Some(episode) = &mut required
                {
                    episode.state = crate::protocol::service::RequirementState::Retired;
                }
                required
            },
            episode: episode as u64,
            joined: effective == MembershipStatus::Joined,
            retired: seat_state == "retired",
            physical_state: physical,
            effective_state: effective,
            joined_at: joined_at.map(UtcMillis),
            left_at: left_at.map(UtcMillis),
            retirement_cutover: retired_at.or(materialized_retired).map(UtcMillis),
            cleanup_state,
            accepted_invitation,
        };
        let mut next_cursor = cursor_for(
            instance,
            CursorScope::Participants,
            q.thread.as_str(),
            &filter,
            CursorDirection::Ascending,
            ordinal as u64,
            high,
        );
        next_cursor.scope_revision = Some(scope.0 as u64);
        next_cursor.filter_revision = Some(scope.1 as u64);
        let raw = next_cursor
            .encode()
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        items.push(Cand {
            item,
            argv: thread_participants_argv(q.thread.as_str(), &raw, &q.page),
            raw,
            before: before_last,
        });
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        q.page.max_bytes,
        "participant cannot fit",
        |page, _| CommandResult::Participants(page),
    )?;
    if let Some(before) = cut {
        last = before;
        stop = StopReason::Bytes;
    }
    if examined == CANDIDATE_LIMIT && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let mut c = cursor_for(
            instance,
            CursorScope::Participants,
            q.thread.as_str(),
            &filter,
            CursorDirection::Ascending,
            last,
            high,
        );
        c.scope_revision = Some(scope.0 as u64);
        c.filter_revision = Some(scope.1 as u64);
        let raw = c
            .encode()
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((
            raw.clone(),
            thread_participants_argv(q.thread.as_str(), &raw, &q.page),
        ))
    };
    Ok(CommandResult::Participants(sized(
        items,
        next,
        high,
        stop,
        CommandResult::Participants,
        output,
        q.page.max_bytes,
    )?))
}

fn membership_status(state: &str) -> Result<MembershipStatus, ApiError> {
    match state {
        "invited" => Ok(MembershipStatus::Invited),
        "joined" => Ok(MembershipStatus::Joined),
        "left" => Ok(MembershipStatus::Left),
        "retired" => Ok(MembershipStatus::Retired),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid membership state",
        )),
    }
}

fn directory(
    db: &QueryConnection,
    instance: &str,
    q: &DirectoryQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let member_relevant = q.membership.is_some() && q.membership_filter != DirectoryMembership::All;
    let member_revision: i64 = if member_relevant {
        db.query_row("SELECT coalesce((SELECT revision FROM filter_revisions WHERE instance_id=?1 AND scope_kind='directory' AND scope_key=?2),0)",params![instance,format!("member:{}",q.membership.as_ref().unwrap().as_str())],|r|r.get(0)).map_err(|e|db.map_error(e))?
    } else {
        0
    };
    let lifecycle_revision: i64 = if member_relevant {
        db.query_row(
            "SELECT lifecycle_revision FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?
    } else {
        0
    };
    let topic_revision: i64 = if q.topic_contains.is_some() {
        db.query_row("SELECT coalesce((SELECT revision FROM filter_revisions WHERE instance_id=?1 AND scope_kind='topic' AND scope_key='all'),0)",[instance],|r|r.get(0)).map_err(|e|db.map_error(e))?
    } else {
        0
    };
    let filter = digest(&(
        q.membership.as_ref().map(|s| s.as_str()),
        q.membership_filter,
        q.topic_contains.as_deref(),
    ))?;
    let scope_key = "*";
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::Directory,
        scope_key,
        &filter,
        CursorDirection::Ascending,
    )?;
    if cursor.as_ref().is_some_and(|c| {
        c.scope_revision != Some(member_revision as u64)
            || c.filter_revision != Some(topic_revision as u64)
            || c.last_examined_key.as_deref() != Some(&*lifecycle_revision.to_string())
    }) {
        return Err(ApiError::cursor_stale("directory filter changed")
            .with_restart_argv(contextual_argv(directory_argv(q, None), output)));
    }
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM threads WHERE instance_id=?1",
                [instance],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(|e| db.map_error(e))
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut last = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut examined = 0usize;
    let mut stop = StopReason::Complete;
    while examined < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        type Row = (i64, String, String, i64, i64, i64);
        let row:Option<Row>=db.query_row("SELECT ordinal,id,topic,archived,created_at,next_sequence FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",params![instance,last as i64,high as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|db.map_error(e))?;
        let Some((ordinal, id, topic, archived, created_at, next_sequence)) = row else {
            break;
        };
        examined += 1;
        let topic_match = q
            .topic_contains
            .as_ref()
            .is_none_or(|needle| topic.contains(needle));
        let membership_match = if let Some(seat) = &q.membership {
            let state =
                super::service_substrate::effective_membership(db, &ThreadId::new(&id), seat)?
                    .native_state;
            match q.membership_filter {
                DirectoryMembership::All => true,
                DirectoryMembership::Joined => state == Some("joined"),
                DirectoryMembership::Invited => state == Some("invited"),
                DirectoryMembership::Default => matches!(state, Some("joined" | "invited")),
            }
        } else {
            true
        };
        if topic_match && membership_match {
            let item = thread_summary(db, &id, &topic, archived, created_at, next_sequence)?;
            let mut next_cursor = cursor_for(
                instance,
                CursorScope::Directory,
                scope_key,
                &filter,
                CursorDirection::Ascending,
                ordinal as u64,
                high,
            );
            next_cursor.scope_revision = Some(member_revision as u64);
            next_cursor.filter_revision = Some(topic_revision as u64);
            next_cursor.last_examined_key = Some(lifecycle_revision.to_string());
            let next = next_cursor
                .encode()
                .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
            items.push(Cand {
                item,
                argv: directory_argv(q, Some(&next)),
                raw: next,
                before: last,
            });
        }
        last = ordinal as u64;
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        q.page.max_bytes,
        "directory item cannot fit",
        |page, _| CommandResult::Directory(page),
    )?;
    if let Some(before) = cut {
        last = before;
        stop = StopReason::Bytes;
    }
    if examined == CANDIDATE_LIMIT && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let mut next_cursor = cursor_for(
            instance,
            CursorScope::Directory,
            scope_key,
            &filter,
            CursorDirection::Ascending,
            last,
            high,
        );
        next_cursor.scope_revision = Some(member_revision as u64);
        next_cursor.filter_revision = Some(topic_revision as u64);
        next_cursor.last_examined_key = Some(lifecycle_revision.to_string());
        let raw = next_cursor
            .encode()
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((raw.clone(), directory_argv(q, Some(&raw))))
    };
    Ok(CommandResult::Directory(sized(
        items,
        next,
        high,
        stop,
        CommandResult::Directory,
        output,
        q.page.max_bytes,
    )?))
}

fn search(
    db: &QueryConnection,
    store: &StoreContext,
    instance: &str,
    q: &SearchQuery,
    output: &OutputSpec,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    let filter = digest(&(q.literal.as_str(), q.thread.as_ref().map(|t| t.as_str())))?;
    let key = q.thread.as_ref().map_or("*", |t| t.as_str());
    if let Some(thread) = &q.thread {
        let owned: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
                params![thread.as_str(), instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        if !owned {
            return Err(api_error(ErrorCode::NotFound, "thread not found"));
        }
    }
    let topic_key = q.thread.as_ref().map_or("all", |t| t.as_str());
    let topic_revision:i64=db.query_row("SELECT coalesce((SELECT revision FROM filter_revisions WHERE instance_id=?1 AND scope_kind='topic' AND scope_key=?2),0)",params![instance,topic_key],|r|r.get(0)).map_err(|e|db.map_error(e))?;
    let initial = if let Some(raw) = &q.page.cursor {
        let c = Cursor::decode_for(
            raw,
            instance,
            CursorScope::SearchCandidates,
            key,
            &filter,
            CursorDirection::Ascending,
            2,
        )
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        if c.search.is_none() {
            return Err(api_error(ErrorCode::InvalidCursor, "search phase missing"));
        }
        if c.search.as_ref().is_some_and(|s| {
            s.body_high_water > i64::MAX as u64
                || s.last_decision_seq.is_some() != s.last_event_offset.is_some()
                || s.last_decision_seq
                    .is_some_and(|seq| seq > s.body_high_water)
                || s.last_event_offset.is_some_and(|offset| offset != 0)
                || (s.phase == SearchPhase::Topic && s.last_decision_seq.is_some())
        }) {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "search logical position invalid",
            ));
        }
        if c.search.as_ref().is_some_and(|s| {
            s.phase == SearchPhase::Topic && s.topic_revision != topic_revision as u64
        }) {
            return Err(ApiError::cursor_stale("search topic filter changed")
                .with_restart_argv(contextual_argv(search_request_argv(q, None), output)));
        }
        c
    } else {
        let topic_high: i64 = db
            .query_row(
                "SELECT coalesce(max(ordinal),0) FROM threads WHERE instance_id=?1",
                [instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        let body_high: i64 = db
            .query_row(
                "SELECT decision_seq FROM host_instances WHERE id=?1",
                [instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        let mut c = cursor_for(
            instance,
            CursorScope::SearchCandidates,
            key,
            &filter,
            CursorDirection::Ascending,
            0,
            body_high.max(topic_high) as u64,
        );
        c.order_version = 2;
        c.search = Some(SearchCursorState {
            phase: SearchPhase::Topic,
            topic_high_water: topic_high as u64,
            body_high_water: body_high as u64,
            topic_revision: topic_revision as u64,
            last_decision_seq: None,
            last_event_offset: None,
        });
        c
    };
    let mut cursor = initial;
    let mut items = Vec::new();
    let mut examined = 0u16;
    let mut examined_bytes = 0u64;
    let mut stop = StopReason::Complete;
    #[cfg(feature = "test-support")]
    crate::test_support::search_barrier::pause(&q.literal, budget, store.clock());
    loop {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        if examined >= q.max_candidates
            || examined as usize >= CANDIDATE_LIMIT
            || examined_bytes >= 1_048_576
        {
            stop = StopReason::Work;
            break;
        }
        search_budget_check(budget, store.clock())?;
        let state = cursor.search.as_ref().unwrap();
        match state.phase {
            SearchPhase::Topic => {
                type TopicRow = (i64, String, String, i64, i64, i64);
                let next:Option<TopicRow>=db.query_row("SELECT ordinal,id,topic,archived,created_at,next_sequence FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",params![instance,cursor.after_ordinal as i64,state.topic_high_water as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|db.map_error(e))?;
                let Some((ordinal, id, topic, archived, created_at, next_sequence)) = next else {
                    cursor.search.as_mut().unwrap().phase = SearchPhase::Body;
                    cursor.after_ordinal = 0;
                    continue;
                };
                let cost = topic.len() as u64;
                if examined_bytes + cost > 1_048_576 {
                    stop = StopReason::Work;
                    break;
                }
                if q.thread.as_ref().is_none_or(|wanted| wanted.as_str() == id)
                    && literal_contains_bounded(&topic, &q.literal, budget, store.clock())?
                {
                    let summary =
                        thread_summary(db, &id, &topic, archived, created_at, next_sequence)?;
                    let mut trial = cursor.clone();
                    trial.after_ordinal = ordinal as u64;
                    let raw = trial
                        .encode()
                        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
                    search_budget_check(budget, store.clock())?;
                    items.push(Cand {
                        item: SearchHit::Topic(summary),
                        argv: search_argv(q, &raw),
                        raw,
                        before: SearchWalk {
                            cursor: cursor.clone(),
                            examined,
                            bytes: examined_bytes,
                            cost,
                        },
                    });
                }
                cursor.after_ordinal = ordinal as u64;
                examined += 1;
                examined_bytes += cost;
            }
            SearchPhase::Body => {
                let position = GlobalLogicalPosition {
                    after_decision_seq: state.last_decision_seq.unwrap_or(0) as i64,
                    after_event_offset: state.last_event_offset.map_or(-1, |offset| offset as i64),
                    high_water_decision_seq: state.body_high_water as i64,
                };
                let slice = scan_global_logical_candidates(
                    db,
                    instance,
                    GlobalLogicalKinds::Ordinary,
                    Some(position),
                    1,
                )?;
                let Some(candidate) = slice.candidates.into_iter().next() else {
                    break;
                };
                type BodyRow = (i64, Option<String>, Option<String>, Option<String>, i64);
                let (seq, author, label, body, at): BodyRow = db
                    .query_row(
                        "SELECT sequence,actor_seat_id,actor_label,body,decision_at FROM messages WHERE id=?1 AND instance_id=?2 AND kind='ordinary'",
                        params![candidate.id, instance],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                    .optional()
                    .map_err(|e| db.map_error(e))?
                    .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "logical search candidate missing"))?;
                let cost = body.as_ref().map_or(0, |v| v.len()) as u64;
                if examined_bytes + cost > 1_048_576 {
                    stop = StopReason::Work;
                    break;
                }
                let body_matches = if q
                    .thread
                    .as_ref()
                    .is_none_or(|wanted| wanted.as_str() == candidate.thread_id)
                {
                    match body.as_deref() {
                        Some(value) => {
                            literal_contains_bounded(value, &q.literal, budget, store.clock())?
                        }
                        None => false,
                    }
                } else {
                    false
                };
                if body_matches {
                    let summary = message_summary(
                        db,
                        &candidate.id,
                        &ThreadId::new(&candidate.thread_id),
                        seq,
                        &candidate.kind,
                        author.as_deref(),
                        label.as_deref(),
                        body.as_deref(),
                        None,
                        at,
                        output,
                    )?;
                    let mut trial = cursor.clone();
                    trial.search.as_mut().unwrap().last_decision_seq =
                        Some(candidate.decision_seq as u64);
                    trial.search.as_mut().unwrap().last_event_offset =
                        Some(candidate.event_offset as u64);
                    let raw = trial
                        .encode()
                        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
                    search_budget_check(budget, store.clock())?;
                    items.push(Cand {
                        item: SearchHit::Body(summary),
                        argv: search_argv(q, &raw),
                        raw,
                        before: SearchWalk {
                            cursor: cursor.clone(),
                            examined,
                            bytes: examined_bytes,
                            cost,
                        },
                    });
                }
                cursor.search.as_mut().unwrap().last_decision_seq =
                    Some(candidate.decision_seq as u64);
                cursor.search.as_mut().unwrap().last_event_offset =
                    Some(candidate.event_offset as u64);
                examined += slice.visited;
                examined_bytes += cost;
            }
        }
    }
    let (items, cut) = fit_candidates(
        items,
        cursor.high_water_ordinal,
        output,
        q.page.max_bytes,
        "search match cannot fit",
        |matches, at| {
            CommandResult::Search(SearchPage {
                matches,
                examined_candidates: at.before.examined + 1,
                examined_utf8_bytes: at.before.bytes + at.before.cost,
            })
        },
    )?;
    if let Some(before) = cut {
        cursor = before.cursor;
        examined = before.examined;
        examined_bytes = before.bytes;
        stop = StopReason::Bytes;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = cursor
            .encode()
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((raw.clone(), search_argv(q, &raw)))
    };
    let page = page(items, next, cursor.high_water_ordinal, stop, output);
    let result = CommandResult::Search(SearchPage {
        matches: page,
        examined_candidates: examined,
        examined_utf8_bytes: examined_bytes,
    });
    ensure_fit(&result, output, q.page.max_bytes)?;
    search_budget_check(budget, store.clock())?;
    Ok(result)
}

fn search_budget_check(budget: &CallBudget, clock: &dyn Clock) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        Err(api_error(ErrorCode::Cancelled, "search cancelled"))
    } else if budget.deadline_passed(clock) {
        Err(api_error(
            ErrorCode::ReadBudgetExhausted,
            "search budget exhausted",
        ))
    } else {
        Ok(())
    }
}

fn literal_contains_bounded(
    haystack: &str,
    needle: &str,
    budget: &CallBudget,
    clock: &dyn Clock,
) -> Result<bool, ApiError> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return Ok(true);
    }
    let mut prefix = vec![0usize; needle.len()];
    for i in 1..needle.len() {
        if i % 4096 == 0 {
            search_budget_check(budget, clock)?;
        }
        let mut k = prefix[i - 1];
        while k > 0 && needle[i] != needle[k] {
            k = prefix[k - 1];
        }
        if needle[i] == needle[k] {
            k += 1;
        }
        prefix[i] = k;
    }
    let mut matched = 0usize;
    for (i, byte) in haystack.iter().enumerate() {
        if i % 4096 == 0 {
            search_budget_check(budget, clock)?;
        }
        while matched > 0 && *byte != needle[matched] {
            matched = prefix[matched - 1];
        }
        if *byte == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            search_budget_check(budget, clock)?;
            return Ok(true);
        }
    }
    search_budget_check(budget, clock)?;
    Ok(false)
}

fn thread_summary(
    db: &QueryConnection,
    id: &str,
    topic: &str,
    archived: i64,
    created_at: i64,
    next_sequence: i64,
) -> Result<ThreadSummary, ApiError> {
    let ordinary: i64 = db
        .query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1 AND kind='ordinary'",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?;
    let joined:i64=db.query_row("SELECT count(*) FROM memberships m JOIN seats s ON s.id=m.seat_id WHERE m.thread_id=?1 AND m.state='joined' AND s.state!='retired'",[id],|r|r.get(0)).map_err(|e|db.map_error(e))?;
    let total = next_sequence
        .checked_sub(1)
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid next sequence"))?;
    if ordinary > total {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "message count exceeds timeline",
        ));
    }
    Ok(ThreadSummary {
        thread: ThreadId::new(id),
        managed_owner: super::service_substrate::managed_owner(db, &ThreadId::new(id))?,
        topic_data: topic.into(),
        topic_omitted: false,
        topic_detail_argv: None,
        archived: archived != 0,
        orphaned: joined == 0,
        message_count: total as u64,
        created_at: UtcMillis(created_at),
        ordinary_count: ordinary as u64,
        system_count: (total - ordinary) as u64,
        joined_count: joined as u64,
    })
}

fn published_warning_detail(
    db: &QueryConnection,
    instance: &str,
    q: &MessageQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let warning = effective_warning_by_id(db, q.message.as_str())?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "message not found"))?;
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
            params![warning.thread_id, instance],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "message not found"));
    }
    if q.body.cursor.is_some() || q.body.offset.is_some() {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "system event has no body offset",
        ));
    }
    let source = warning
        .source_message_id
        .as_deref()
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "published warning source missing"))?;
    let decision_at: i64 = db
        .query_row(
            "SELECT decision_at FROM send_manifests WHERE message_id=?1",
            [source],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?;
    let summary = message_summary(
        db,
        &warning.id,
        &ThreadId::new(&warning.thread_id),
        warning.sequence,
        "warn",
        None,
        None,
        None,
        Some(&warning.event_json),
        decision_at,
        output,
    )?;
    let event_json = serde_json::from_str(&warning.event_json)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "published warning JSON malformed"))?;
    let result = CommandResult::Message(MessageDetails {
        summary,
        content: MessageContent::System {
            event: StructuredEvent {
                kind: MessageKind::Warn,
                event_json,
                source_message: Some(MessageId::new(source)),
                source_invitation: None,
                decision_at: UtcMillis(decision_at),
                classified_at: None,
                materialized_at: None,
            },
            current_condition: None,
        },
    });
    ensure_fit(&result, output, q.body.max_bytes)?;
    Ok(result)
}

fn message(
    db: &QueryConnection,
    instance: &str,
    q: &MessageQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    type Row = (
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
        Option<String>,
    );
    let row:Option<Row>=db.query_row("SELECT m.thread_id,m.sequence,m.kind,m.actor_seat_id,m.actor_label,m.body,m.event_json,m.decision_at,m.source_message_id,m.source_invitation_id FROM messages m JOIN threads t ON t.id=m.thread_id WHERE m.id=?1 AND t.instance_id=?2",params![q.message.as_str(),instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional().map_err(|e|db.map_error(e))?;
    let Some(row) = row else {
        return published_warning_detail(db, instance, q, output);
    };
    let (thread, seq, kind, author, label, body, event, at, source_message, source_invitation) =
        row;
    let summary = message_summary(
        db,
        q.message.as_str(),
        &ThreadId::new(&thread),
        seq,
        &kind,
        author.as_deref(),
        label.as_deref(),
        body.as_deref(),
        event.as_deref(),
        at,
        output,
    )?;
    if kind != "ordinary" {
        if q.body.cursor.is_some() || q.body.offset.is_some() {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "system event has no body offset",
            ));
        }
        let event_json = serde_json::from_str(
            event
                .as_deref()
                .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "system event missing"))?,
        )
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "system event malformed"))?;
        let result = CommandResult::Message(MessageDetails {
            summary: summary.clone(),
            content: MessageContent::System {
                event: StructuredEvent {
                    kind: summary.kind,
                    event_json,
                    source_message: source_message.map(MessageId::new),
                    source_invitation: source_invitation
                        .map(crate::protocol::ids::InvitationId::new),
                    decision_at: UtcMillis(at),
                    classified_at: None,
                    materialized_at: None,
                },
                current_condition: None,
            },
        });
        ensure_fit(&result, output, q.body.max_bytes)?;
        return Ok(result);
    }
    let body = body.ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "ordinary body missing"))?;
    let filter = digest(&q.message.as_str())?;
    let offset = if let Some(encoded) = &q.body.cursor {
        let cursor = Cursor::decode_for(
            encoded,
            instance,
            CursorScope::MessageBody,
            q.message.as_str(),
            &filter,
            CursorDirection::Ascending,
            1,
        )
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        if cursor.high_water_ordinal != body.len() as u64 {
            return Err(api_error(ErrorCode::InvalidCursor, "body length changed"));
        }
        cursor.after_ordinal as usize
    } else {
        q.body.offset.unwrap_or(0) as usize
    };
    if offset > body.len() || !body.is_char_boundary(offset) {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "invalid UTF-8 body offset",
        ));
    }
    let boundaries: Vec<usize> = body[offset..]
        .char_indices()
        .map(|(i, _)| offset + i)
        .chain(std::iter::once(body.len()))
        .collect();
    let make = |end: usize| -> Result<CommandResult, ApiError> {
        let complete = end == body.len();
        let next = if complete {
            None
        } else {
            Some(
                cursor_for(
                    instance,
                    CursorScope::MessageBody,
                    q.message.as_str(),
                    &filter,
                    CursorDirection::Ascending,
                    end as u64,
                    body.len() as u64,
                )
                .encode()
                .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?,
            )
        };
        let argv = next.as_ref().map(|cursor| {
            contextual_argv(
                vec![
                    "herdr-threads".into(),
                    "body".into(),
                    q.message.as_str().into(),
                    "--cursor".into(),
                    cursor.clone(),
                    "--max-bytes".into(),
                    q.body.max_bytes.to_string(),
                ],
                output,
            )
        });
        Ok(CommandResult::Message(MessageDetails {
            summary: summary.clone(),
            content: MessageContent::Ordinary {
                body_data: body[offset..end].into(),
                body_offset: offset as u64,
                body_total_bytes: body.len() as u64,
                body_complete: complete,
                body_next_cursor: next,
                body_next_argv: argv,
            },
        }))
    };
    let fits = |page: &CommandResult| -> Result<bool, ApiError> {
        Ok(encode_selected(page, output)?.len() <= q.body.max_bytes as usize)
    };
    // The complete page carries no cursor/argv, so it can be smaller than a
    // partial page: the size predicate is not monotonic. Test it first.
    let full = make(body.len())?;
    if fits(&full)? {
        return Ok(full);
    }
    // Binary-search partial boundaries only (all but the final body.len()).
    let partial = &boundaries[..boundaries.len() - 1];
    if partial.is_empty() {
        return Err(ApiError::invalid_budget("body continuation cannot fit")
            .with_required_minimum_bytes(encode_selected(&full, output)?.len() as u32));
    }
    let mut low = 0usize;
    let mut upper = partial.len();
    while low + 1 < upper {
        let middle = (low + upper) / 2;
        if fits(&make(partial[middle])?)? {
            low = middle;
        } else {
            upper = middle;
        }
    }
    let result = make(partial[low])?;
    if !fits(&result)? || low == 0 {
        let minimum = make(partial.get(1).copied().unwrap_or(body.len()))?;
        return Err(
            ApiError::invalid_budget("body continuation cannot fit one character")
                .with_required_minimum_bytes(encode_selected(&minimum, output)?.len() as u32),
        );
    }
    Ok(result)
}

fn ensure_fit(result: &CommandResult, output: &OutputSpec, max: u32) -> Result<(), ApiError> {
    let bytes = encode_selected(result, output)?;
    if bytes.len() > max as usize {
        return Err(ApiError::invalid_budget("selected response cannot fit")
            .with_required_minimum_bytes(bytes.len() as u32));
    }
    Ok(())
}

fn digest<T: serde::Serialize>(value: &T) -> Result<String, ApiError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "cannot encode query filter"))?;
    let hash = Sha256::digest(bytes);
    Ok(hash[..16].iter().map(|b| format!("{b:02x}")).collect())
}

fn cursor_for(
    instance: &str,
    scope: CursorScope,
    key: &str,
    filter_digest: &str,
    direction: CursorDirection,
    after: u64,
    high: u64,
) -> Cursor {
    Cursor {
        instance: instance.into(),
        scope,
        scope_key: key.into(),
        filter_digest: filter_digest.into(),
        direction,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: after,
        high_water_ordinal: high,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
}

fn decode_cursor(
    page: &PageRequest,
    instance: &str,
    scope: CursorScope,
    key: &str,
    filter_digest: &str,
    direction: CursorDirection,
) -> Result<Option<Cursor>, ApiError> {
    page.cursor
        .as_ref()
        .map(|raw| {
            Cursor::decode_for(raw, instance, scope, key, filter_digest, direction, 1)
                .map_err(|why| api_error(ErrorCode::InvalidCursor, why))
        })
        .transpose()
}

/// `--limit`/`--max-bytes` for a continuation, each only when it differs from
/// the CLI default it would otherwise repeat (ht-4is.8.18: shorter commands).
fn page_bounds(page: &PageRequest) -> Vec<String> {
    let mut argv = Vec::new();
    if page.limit != crate::protocol::pagination::DEFAULT_PAGE_LIMIT {
        argv.extend(["--limit".into(), page.limit.to_string()]);
    }
    if page.max_bytes != crate::protocol::pagination::DEFAULT_PAGE_BYTES {
        argv.extend(["--max-bytes".into(), page.max_bytes.to_string()]);
    }
    argv
}

fn continuation(command: &str, key: Option<&str>, cursor: &str, page: &PageRequest) -> Vec<String> {
    let mut args = vec!["herdr-threads".into(), command.into()];
    if let Some(key) = key {
        args.push(key.into());
    }
    args.extend(["--cursor".into(), cursor.into()]);
    args.extend(page_bounds(page));
    args
}

fn directory_argv(q: &DirectoryQuery, cursor: Option<&str>) -> Vec<String> {
    let mut argv = vec!["herdr-threads".into(), "thread".into(), "list".into()];
    if let Some(seat) = &q.membership {
        argv.extend(["--seat".into(), seat.as_str().into()]);
    }
    match q.membership_filter {
        DirectoryMembership::Joined => argv.push("--joined".into()),
        DirectoryMembership::Invited => argv.push("--invited".into()),
        DirectoryMembership::All => argv.push("--all".into()),
        DirectoryMembership::Default => {}
    }
    if let Some(search) = &q.topic_contains {
        argv.extend(["--search".into(), search.clone()]);
    }
    if let Some(cursor) = cursor {
        argv.extend(["--cursor".into(), cursor.into()]);
    }
    argv.extend([
        "--limit".into(),
        q.page.limit.to_string(),
        "--max-bytes".into(),
        q.page.max_bytes.to_string(),
    ]);
    argv
}

fn seats_argv(cursor: &str, q: &SeatsQuery) -> Vec<String> {
    let mut argv = vec!["herdr-threads".into(), "seat".into(), "list".into()];
    if q.include_retired {
        argv.push("--include-retired".into());
    }
    argv.extend([
        "--cursor".into(),
        cursor.into(),
        "--limit".into(),
        q.page.limit.to_string(),
        "--max-bytes".into(),
        q.page.max_bytes.to_string(),
    ]);
    argv
}

fn retirement_jobs_argv(cursor: &str, page: &PageRequest) -> Vec<String> {
    let mut argv = vec!["herdr-threads".into(), "seat".into(), "retirements".into()];
    argv.extend([
        "--cursor".into(),
        cursor.into(),
        "--limit".into(),
        page.limit.to_string(),
        "--max-bytes".into(),
        page.max_bytes.to_string(),
    ]);
    argv
}

fn search_argv(q: &SearchQuery, cursor: &str) -> Vec<String> {
    search_request_argv(q, Some(cursor))
}

fn search_request_argv(q: &SearchQuery, cursor: Option<&str>) -> Vec<String> {
    let mut argv = vec!["herdr-threads".into(), "search".into(), q.literal.clone()];
    if let Some(thread) = &q.thread {
        argv.extend(["--thread".into(), thread.as_str().into()]);
    }
    if let Some(cursor) = cursor {
        argv.extend(["--cursor".into(), cursor.into()]);
    }
    argv.extend([
        "--limit".into(),
        q.page.limit.to_string(),
        "--max-bytes".into(),
        q.page.max_bytes.to_string(),
    ]);
    argv
}

fn thread_participants_argv(thread: &str, cursor: &str, page: &PageRequest) -> Vec<String> {
    let mut argv = vec![
        "herdr-threads".into(),
        "thread".into(),
        "participants".into(),
        thread.into(),
    ];
    argv.extend(["--cursor".into(), cursor.into()]);
    argv.extend(page_bounds(page));
    argv
}

fn delivery_recipients_argv(message: &str, cursor: &str, page: &PageRequest) -> Vec<String> {
    let mut argv = vec![
        "herdr-threads".into(),
        "delivery".into(),
        "recipients".into(),
        message.into(),
    ];
    argv.extend([
        "--cursor".into(),
        cursor.into(),
        "--limit".into(),
        page.limit.to_string(),
        "--max-bytes".into(),
        page.max_bytes.to_string(),
    ]);
    argv
}

fn page<T>(
    items: Vec<T>,
    next: Option<(String, Vec<String>)>,
    high: u64,
    reason: StopReason,
    output: &OutputSpec,
) -> Page<T> {
    let has_more = next.is_some();
    let (next_cursor, next_argv) = next.map_or((None, None), |(c, a)| {
        (Some(c), Some(contextual_argv(a, output)))
    });
    Page {
        items,
        next_cursor,
        next_argv,
        high_water_ordinal: high,
        scope_revision: None,
        has_more,
        stop_reason: reason,
        consistency: Consistency::BoundedLive,
    }
}

fn contextual_argv(argv: Vec<String>, output: &OutputSpec) -> Vec<String> {
    let mut parts = argv.into_iter();
    let mut result = vec![parts.next().unwrap_or_else(|| "herdr-threads".into())];
    if let Some(state) = &output.context.state_dir {
        result.extend(["--state-dir".into(), state.clone()]);
    }
    if let Some(host) = &output.context.host {
        result.extend(["--host-endpoint".into(), host.as_str().into()]);
    }
    if output.format == OutputFormat::Json {
        result.push("--json".into());
    }
    result.extend(parts);
    result
}

fn sized<R: Fn(Page<T>) -> CommandResult, T: Clone>(
    items: Vec<T>,
    next: Option<(String, Vec<String>)>,
    high: u64,
    reason: StopReason,
    wrap: R,
    output: &OutputSpec,
    max: u32,
) -> Result<Page<T>, ApiError> {
    let candidate = page(items, next, high, reason, output);
    let bytes = encode_selected(&wrap(candidate.clone()), output)?;
    if bytes.len() > max as usize {
        return Err(ApiError::invalid_budget("selected response cannot fit")
            .with_required_minimum_bytes(bytes.len().min(u32::MAX as usize) as u32));
    }
    Ok(candidate)
}

/// One walked item awaiting the page fit: the item, the continuation cursor
/// that would follow it, and the walk state before it (restored when the fit
/// cuts the page short there).
#[derive(Clone)]
struct Cand<T, P = u64> {
    item: T,
    raw: String,
    argv: Vec<String>,
    before: P,
}

/// Search walk state before a candidate hit; `cost` is that candidate's UTF-8
/// byte cost, counted into the page's `examined_utf8_bytes` once it is in.
#[derive(Clone)]
struct SearchWalk {
    cursor: Cursor,
    examined: u16,
    bytes: u64,
    cost: u64,
}

/// Fit the walked items into `max` bytes through `PageFit`. Returns the
/// accepted items and, when the byte budget cut the page short, the walk state
/// before the first rejected item. A first item that cannot fit alone is
/// `InvalidBudget` with `detail`.
fn fit_candidates<T: Clone, P: Clone>(
    cands: Vec<Cand<T, P>>,
    high: u64,
    output: &OutputSpec,
    max: u32,
    detail: &'static str,
    wrap: impl Fn(Page<T>, &Cand<T, P>) -> CommandResult,
) -> Result<(Vec<T>, Option<P>), ApiError> {
    if cands.is_empty() {
        return Ok((Vec::new(), None));
    }
    let render_items = |items: Vec<T>, cursor: &Cand<T, P>| {
        Ok(wrap(
            page(
                items,
                Some((cursor.raw.clone(), cursor.argv.clone())),
                high,
                StopReason::Rows,
                output,
            ),
            cursor,
        ))
    };
    let fit = PageFit::for_command(output, max as usize).fit_with(
        &cands,
        |k| {
            render_items(
                cands[..k].iter().map(|c| c.item.clone()).collect(),
                &cands[k.max(1) - 1],
            )
        },
        |i| render_items(vec![cands[i].item.clone()], &cands[i]),
    )?;
    if fit.accepted == 0 {
        let mut error = ApiError::invalid_budget(detail);
        error.required_minimum_bytes = fit.required_minimum;
        return Err(error);
    }
    let cut = (fit.stop == FitStop::Bytes).then(|| cands[fit.accepted].before.clone());
    let mut items: Vec<T> = cands.into_iter().map(|c| c.item).collect();
    items.truncate(fit.accepted);
    Ok((items, cut))
}

fn history(
    db: &QueryConnection,
    instance: &str,
    q: &crate::protocol::commands::HistoryQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let current_high: i64 = db
        .query_row(
            "SELECT next_sequence-1 FROM threads WHERE id=?1 AND instance_id=?2",
            params![q.thread.as_str(), instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db.map_error(e))?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "thread not found"))?;
    let filter = digest(&q.thread.as_str())?;
    let cursor = if let Some(raw) = &q.page.cursor {
        // Either direction is a history cursor; the binding still covers it.
        let direction = Cursor::decode(raw)
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?
            .direction;
        let c = Cursor::decode_for(
            raw,
            instance,
            CursorScope::History,
            q.thread.as_str(),
            &filter,
            direction,
            1,
        )
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        if c.search.is_some() {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "history cursor phase mismatch",
            ));
        }
        Some(c)
    } else {
        None
    };
    let high = cursor
        .as_ref()
        .map_or(current_high.max(0) as u64, |c| c.high_water_ordinal);
    let direction = cursor.as_ref().map_or_else(
        || {
            if matches!(q.initial, Some(HistoryRange::After { .. })) {
                CursorDirection::Ascending
            } else {
                CursorDirection::Descending
            }
        },
        |c| c.direction,
    );
    let descending = direction == CursorDirection::Descending;
    let initial = match q.initial {
        Some(HistoryRange::After { sequence }) => sequence.saturating_add(1),
        Some(HistoryRange::Before { sequence }) => sequence.saturating_sub(1).min(high),
        _ if descending => high,
        _ => 1,
    };
    let mut next_sequence = cursor.as_ref().map_or(initial, |c| {
        if descending {
            c.after_ordinal.saturating_sub(1)
        } else {
            c.after_ordinal.saturating_add(1)
        }
    });
    let take = match q.initial {
        Some(HistoryRange::Recent { count }) => count.min(q.page.limit),
        _ => q.page.limit,
    } as usize;
    let mut items = Vec::new();
    let mut last = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut stop = StopReason::Complete;
    while items.len() < take && next_sequence >= 1 && next_sequence <= high {
        let seq = i64::try_from(next_sequence)
            .map_err(|_| api_error(ErrorCode::SequenceExhausted, "history sequence exhausted"))?;
        let slice = scan_effective_timeline(
            db,
            q.thread.as_str(),
            Some(TimelinePosition {
                after_sequence: seq - 1,
                high_water_sequence: seq,
            }),
            1,
        )?;
        let entry = slice
            .entries
            .into_iter()
            .next()
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "timeline position missing"))?;
        let summary = timeline_summary(db, &q.thread, entry, output, q.full_bodies)?;
        let raw = cursor_for(
            instance,
            CursorScope::History,
            q.thread.as_str(),
            &filter,
            direction,
            next_sequence,
            high,
        )
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        items.push(Cand {
            item: summary,
            argv: continuation("read", Some(q.thread.as_str()), &raw, &q.page),
            raw,
            before: (last, next_sequence),
        });
        last = next_sequence;
        next_sequence = if descending {
            next_sequence.saturating_sub(1)
        } else {
            next_sequence.saturating_add(1)
        };
    }
    let fit = |cands: Vec<Cand<MessageSummary, (u64, u64)>>| {
        fit_candidates(
            cands,
            high,
            output,
            q.page.max_bytes,
            "history item cannot fit",
            |page, _| CommandResult::History(page),
        )
    };
    // A first body that cannot fit the page even alone keeps today's clipped
    // preview and body cursor; later bodies that do not fit end the page and
    // lead the next one, where they are first.
    let first_inlined = items.first().is_some_and(|c| is_inlined_full_body(&c.item));
    let clipped_retry = first_inlined.then(|| items.clone());
    let (items, cut) = match (fit(items), clipped_retry) {
        (Err(e), Some(mut retry)) if e.code == ErrorCode::InvalidBudget => {
            let first = &mut retry[0].item;
            first.preview_data = first
                .preview_data
                .chars()
                .take(PREVIEW_SNIPPET_CHARS)
                .collect();
            first.preview_omitted = true;
            fit(retry)?
        }
        (fitted, _) => fitted?,
    };
    if let Some((before_last, before_next)) = cut {
        last = before_last;
        next_sequence = before_next;
        stop = StopReason::Bytes;
    }
    if stop == StopReason::Complete && next_sequence >= 1 && next_sequence <= high {
        stop = StopReason::Rows;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = cursor_for(
            instance,
            CursorScope::History,
            q.thread.as_str(),
            &filter,
            direction,
            last,
            high,
        )
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((
            raw.clone(),
            continuation("read", Some(q.thread.as_str()), &raw, &q.page),
        ))
    };
    Ok(CommandResult::History(sized(
        items,
        next,
        high,
        stop,
        CommandResult::History,
        output,
        q.page.max_bytes,
    )?))
}

fn timeline_summary(
    db: &QueryConnection,
    thread: &ThreadId,
    entry: EffectiveTimelineEntry,
    output: &OutputSpec,
    full_bodies: bool,
) -> Result<MessageSummary, ApiError> {
    match entry {
        EffectiveTimelineEntry::Physical { id, sequence, kind } => {
            type Row = (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                i64,
            );
            let row:Row=db.query_row("SELECT actor_seat_id,actor_label,body,event_json,decision_at FROM messages WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|db.map_error(e))?;
            let summary = message_summary(
                db,
                &id,
                thread,
                sequence,
                &kind,
                row.0.as_deref(),
                row.1.as_deref(),
                row.2.as_deref(),
                row.3.as_deref(),
                row.4,
                output,
            )?;
            match row.2.as_deref() {
                Some(body) if full_bodies && kind == "ordinary" => {
                    inline_full_body(summary, body, output)
                }
                _ => Ok(summary),
            }
        }
        EffectiveTimelineEntry::PublishedWarning(warning) => {
            let source = warning.source_message_id.as_deref().ok_or_else(|| {
                api_error(ErrorCode::StoreCorrupt, "published warning source missing")
            })?;
            let decision_at: i64 = db
                .query_row(
                    "SELECT decision_at FROM send_manifests WHERE message_id=?1",
                    [source],
                    |r| r.get(0),
                )
                .map_err(|e| db.map_error(e))?;
            message_summary(
                db,
                &warning.id,
                thread,
                warning.sequence,
                "warn",
                None,
                None,
                None,
                Some(&warning.event_json),
                decision_at,
                output,
            )
        }
    }
}

/// `full_bodies`: replace a clipped preview with the complete body when a
/// `Message` fetch of it (bounded by `FULL_BODY_FETCH_BYTES`, text output, as
/// the human `read` issues it) would return the body whole. A longer body keeps
/// its clipped preview and body cursor: the CLI fetches it as before. Whether
/// the page budget holds the inlined body is the page fit's decision.
fn inline_full_body(
    mut summary: MessageSummary,
    body: &str,
    output: &OutputSpec,
) -> Result<MessageSummary, ApiError> {
    if !summary.preview_omitted
        || body.chars().nth(PREVIEW_SNIPPET_CHARS).is_none()
        || body.len() > FULL_BODY_FETCH_BYTES as usize
    {
        return Ok(summary);
    }
    let fetch = CommandResult::Message(MessageDetails {
        summary: summary.clone(),
        content: MessageContent::Ordinary {
            body_data: body.into(),
            body_offset: 0,
            body_total_bytes: body.len() as u64,
            body_complete: true,
            body_next_cursor: None,
            body_next_argv: None,
        },
    });
    let text = OutputSpec {
        format: OutputFormat::Text,
        context: output.context.clone(),
    };
    if encode_selected(&fetch, &text)?.len() <= FULL_BODY_FETCH_BYTES as usize {
        summary.preview_data = body.into();
        summary.preview_omitted = false;
    }
    Ok(summary)
}

// Allowed: one argument per selected message column plus output shaping.
#[allow(clippy::too_many_arguments)]
fn message_summary(
    db: &QueryConnection,
    id: &str,
    thread: &ThreadId,
    seq: i64,
    kind: &str,
    author: Option<&str>,
    label: Option<&str>,
    body: Option<&str>,
    event: Option<&str>,
    at: i64,
    output: &OutputSpec,
) -> Result<MessageSummary, ApiError> {
    let physical: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?;
    let (author_role, relays_user, author_role_backfilled) = if physical {
        db.query_row(
            "SELECT author_role,relays_user,author_role_backfilled FROM messages WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, i64>(1)? != 0,
                    r.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .map_err(|e| db.map_error(e))
        .map(|(role, relays, backfilled)| {
            (
                role.as_deref()
                    .and_then(crate::protocol::summary::AuthorRole::from_column),
                relays,
                backfilled,
            )
        })?
    } else {
        (None, false, false)
    };
    let event_author = if physical {
        Some(super::service_substrate::message_author(
            db,
            &MessageId::new(id),
        )?)
    } else {
        None
    };
    let kind = match kind {
        "ordinary" => MessageKind::Ordinary,
        "info" => MessageKind::Info,
        "warn" => MessageKind::Warn,
        _ => return Err(api_error(ErrorCode::StoreCorrupt, "invalid message kind")),
    };
    let source = body.or(event).unwrap_or("");
    let snippet: String = source.chars().take(PREVIEW_SNIPPET_CHARS).collect();
    Ok(MessageSummary {
        message: MessageId::new(id),
        thread: thread.clone(),
        author: author.map(SeatId::new),
        event_author,
        author_role,
        relays_user,
        author_role_backfilled,
        kind,
        sequence: seq as u64,
        created_at: UtcMillis(at),
        actor_label: label.map(str::to_owned),
        preview_omitted: snippet.len() < source.len(),
        preview_data: snippet,
        preview_detail_argv: Some(contextual_argv(
            vec!["herdr-threads".into(), "body".into(), id.into()],
            output,
        )),
    })
}

fn receipt_status(state: &str) -> Result<ReceiptStatus, ApiError> {
    match state {
        "pending" => Ok(ReceiptStatus::Pending),
        "acked" => Ok(ReceiptStatus::Acknowledged),
        "recipient_retired" => Ok(ReceiptStatus::Retired),
        _ => Err(api_error(ErrorCode::StoreCorrupt, "invalid receipt state")),
    }
}

/// Frozen deadline, the later effective deadline while a catch-up extension is
/// in force (spec §8), and the deferral end while that extension is still
/// ahead of `now`. `overdue` is judged against the effective deadline.
struct DeadlineDisplay {
    effective: Option<UtcMillis>,
    deferred_until: Option<UtcMillis>,
    overdue: bool,
}

fn deadline_display(
    db: &QueryConnection,
    receipt: &super::effective::EffectiveReceipt,
    now: UtcMillis,
) -> Result<DeadlineDisplay, ApiError> {
    let effective = super::receipts::effective_deadline(db, receipt)?;
    let extended = effective.filter(|at| Some(*at) > receipt.deadline_at);
    Ok(DeadlineDisplay {
        effective: extended.map(UtcMillis),
        deferred_until: extended.filter(|at| *at > now.0).map(UtcMillis),
        overdue: effective.is_some_and(|at| at <= now.0),
    })
}

fn recipient_item(
    db: &QueryConnection,
    receipt: &super::effective::EffectiveReceipt,
    now: UtcMillis,
) -> Result<Recipient, ApiError> {
    let display = if receipt.state == EffectiveReceiptState::Pending {
        deadline_display(db, receipt, now)?
    } else {
        DeadlineDisplay {
            effective: None,
            deferred_until: None,
            overdue: false,
        }
    };
    let effective_status = match receipt.state {
        EffectiveReceiptState::Pending => ReceiptStatus::Pending,
        EffectiveReceiptState::Acknowledged => ReceiptStatus::Acknowledged,
        EffectiveReceiptState::RecipientRetired => ReceiptStatus::Retired,
        EffectiveReceiptState::NotRequired => ReceiptStatus::NotRequired,
    };
    let physical_state: Option<String> = db
        .query_row(
            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
            params![receipt.message_id, receipt.seat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db.map_error(e))?
        .or(db
            .query_row(
                "SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2",
                params![receipt.message_id, receipt.seat_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db.map_error(e))?);
    let physical_status = receipt_status(physical_state.as_deref().unwrap_or("pending"))?;
    let job: Option<(String, i64, Option<String>)> = db
        .query_row(
            "SELECT status,processed_units,last_error FROM retirements WHERE seat_id=?1",
            [&receipt.seat_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| db.map_error(e))?;
    let cleanup_state = job.map(|(status, processed, error)| {
        if status == "complete" {
            CleanupState::Complete
        } else if error.is_some() {
            CleanupState::Failed
        } else if processed > 0 {
            CleanupState::Running
        } else {
            CleanupState::Pending
        }
    });
    let ack_provenance = match (
        &receipt.ack_actor_seat_id,
        receipt.ack_generation,
        &receipt.ack_observation,
        receipt.acked_at,
    ) {
        (Some(actor), Some(generation), Some(observation), Some(at)) => Some(AckProvenance {
            actor: SeatId::new(actor),
            generation: generation as u64,
            native_observation: super::control::canonical_observation(observation),
            decided_at: UtcMillis(at),
        }),
        (None, None, None, None) => None,
        _ => {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "incomplete ACK provenance",
            ));
        }
    };
    Ok(Recipient {
        seat: SeatId::new(&receipt.seat_id),
        status: effective_status.clone(),
        physical_status,
        effective_status,
        retirement_cutover: receipt.retired_at.map(UtcMillis),
        cleanup_state,
        ack_provenance,
        deadline: if receipt.state == EffectiveReceiptState::Pending {
            receipt.deadline_at.map(UtcMillis)
        } else {
            None
        },
        effective_deadline: display.effective,
        deferred_until: display.deferred_until,
    })
}

// Allowed: shared pager: query key, page, output, scope and two result adapters.
#[allow(clippy::too_many_arguments)]
fn recipient_collection<F, A>(
    db: &QueryConnection,
    instance: &str,
    message: &MessageId,
    request: &PageRequest,
    now: UtcMillis,
    output: &OutputSpec,
    scope: CursorScope,
    wrap: F,
    argv: A,
) -> Result<CommandResult, ApiError>
where
    F: Fn(Page<Recipient>) -> CommandResult,
    A: Fn(&str) -> Vec<String>,
{
    let filter = digest(&message.as_str())?;
    let cursor = decode_cursor(
        request,
        instance,
        scope,
        message.as_str(),
        &filter,
        CursorDirection::Ascending,
    )?;
    let mut position = cursor
        .as_ref()
        .map(receipt_position_from_cursor)
        .transpose()?;
    let mut items = Vec::new();
    let mut visited = 0usize;
    let mut stop = StopReason::Complete;
    while visited < CANDIDATE_LIMIT {
        if items.len() == request.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        let before = position.clone();
        let slice = scan_effective_receipts(
            db,
            &ReceiptScanScope::Message(message.as_str().into()),
            position.clone(),
            1,
        )?;
        if slice.visited == 0 {
            position = Some(slice.position);
            break;
        }
        visited += slice.visited as usize;
        for receipt in &slice.items {
            let item = recipient_item(db, receipt, now)?;
            let raw = receipt_cursor(instance, scope, message.as_str(), &filter, &slice.position)?;
            let high = slice
                .position
                .physical_high_water
                .max(slice.position.manifest_high_water) as u64;
            items.push(Cand {
                item,
                argv: argv(&raw),
                raw,
                before: (before.clone(), high),
            });
        }
        position = Some(slice.position);
        if !slice.has_more {
            break;
        }
    }
    let (items, cut) = fit_candidates(
        items,
        0,
        output,
        request.max_bytes,
        "recipient cannot fit",
        |mut recipients, at| {
            recipients.high_water_ordinal = at.before.1;
            wrap(recipients)
        },
    )?;
    if let Some((before, _)) = cut {
        position = before;
        stop = StopReason::Bytes;
    }
    let position = position
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "recipient scan did not initialize"))?;
    if stop == StopReason::Complete && visited == CANDIDATE_LIMIT {
        stop = StopReason::Work;
    }
    let high = position
        .physical_high_water
        .max(position.manifest_high_water) as u64;
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = receipt_cursor(instance, scope, message.as_str(), &filter, &position)?;
        Some((raw.clone(), argv(&raw)))
    };
    let result = wrap(page(items, next, high, stop, output));
    ensure_fit(&result, output, request.max_bytes)?;
    Ok(result)
}

fn message_owned(
    db: &QueryConnection,
    instance: &str,
    message: &MessageId,
) -> Result<bool, ApiError> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN threads t ON t.id=m.thread_id WHERE m.id=?1 AND t.instance_id=?2)",params![message.as_str(),instance],|r|r.get(0)).map_err(|e|db.map_error(e))
}

fn recipients(
    db: &QueryConnection,
    instance: &str,
    q: &RecipientsQuery,
    now: UtcMillis,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    if let Some(warning) = effective_warning_by_id(db, q.message.as_str())? {
        let owned: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
                params![warning.thread_id, instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        if !owned {
            return Err(api_error(ErrorCode::NotFound, "warning not found"));
        }
        return warning_recipients(db, instance, q, output);
    }
    if !message_owned(db, instance, &q.message)? {
        return Err(api_error(ErrorCode::NotFound, "message not found"));
    }
    recipient_collection(
        db,
        instance,
        &q.message,
        &q.page,
        now,
        output,
        CursorScope::Recipients,
        CommandResult::Recipients,
        |raw| delivery_recipients_argv(q.message.as_str(), raw, &q.page),
    )
}

fn warning_recipients(
    db: &QueryConnection,
    instance: &str,
    q: &RecipientsQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let filter = digest(&q.message.as_str())?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::WarningRecipients,
        q.message.as_str(),
        &filter,
        CursorDirection::Ascending,
    )?;
    let mut position = cursor.as_ref().map(|c| {
        let ledger = c.last_examined_key.as_deref() == Some("ledger");
        WarningRecipientPosition {
            warning_id: q.message.as_str().to_owned(),
            interval_after: if ledger { 0 } else { c.after_ordinal as i64 },
            interval_high_water: if ledger {
                0
            } else {
                c.high_water_ordinal as i64
            },
            affected_done: ledger || c.last_examined_key.as_deref() == Some("affected"),
            ledger_after: if ledger { c.after_ordinal as i64 } else { 0 },
            ledger_high_water: if ledger {
                c.high_water_ordinal as i64
            } else {
                0
            },
        }
    });
    let mut items = Vec::new();
    let mut visited = 0usize;
    let mut stop = StopReason::Complete;
    while visited < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        let before = position.clone();
        let slice = scan_effective_warning_recipients(db, q.message.as_str(), position.clone(), 1)?;
        visited += slice.visited as usize;
        position = Some(slice.position.clone());
        for seat in slice.seats {
            let item = WarningRecipient {
                seat: SeatId::new(seat),
            };
            let raw = warning_recipient_cursor(instance, q, &slice.position, &filter)?;
            items.push(Cand {
                item,
                argv: delivery_recipients_argv(q.message.as_str(), &raw, &q.page),
                raw,
                before: (
                    before.clone(),
                    slice
                        .position
                        .interval_high_water
                        .max(slice.position.ledger_high_water) as u64,
                ),
            });
        }
        if !slice.has_more {
            break;
        }
    }
    let (items, cut) = fit_candidates(
        items,
        0,
        output,
        q.page.max_bytes,
        "warning recipient cannot fit",
        |mut recipients, at| {
            recipients.high_water_ordinal = at.before.1;
            CommandResult::WarningRecipients(recipients)
        },
    )?;
    if let Some((before, _)) = cut {
        position = before;
        stop = StopReason::Bytes;
    }
    let position = position
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "warning scan did not initialize"))?;
    if stop == StopReason::Complete && visited == CANDIDATE_LIMIT {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = warning_recipient_cursor(instance, q, &position, &filter)?;
        Some((
            raw.clone(),
            delivery_recipients_argv(q.message.as_str(), &raw, &q.page),
        ))
    };
    Ok(CommandResult::WarningRecipients(sized(
        items,
        next,
        position.interval_high_water.max(position.ledger_high_water) as u64,
        stop,
        CommandResult::WarningRecipients,
        output,
        q.page.max_bytes,
    )?))
}

fn warning_recipient_cursor(
    instance: &str,
    q: &RecipientsQuery,
    position: &WarningRecipientPosition,
    filter: &str,
) -> Result<String, ApiError> {
    let ledger = position.ledger_high_water > 0;
    let mut c = cursor_for(
        instance,
        CursorScope::WarningRecipients,
        q.message.as_str(),
        filter,
        CursorDirection::Ascending,
        if ledger {
            position.ledger_after as u64
        } else {
            position.interval_after as u64
        },
        if ledger {
            position.ledger_high_water as u64
        } else {
            position.interval_high_water as u64
        },
    );
    if ledger {
        c.last_examined_key = Some("ledger".into());
    } else if position.affected_done {
        c.last_examined_key = Some("affected".into());
    }
    c.encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))
}

fn delivery_inspect(
    db: &QueryConnection,
    instance: &str,
    q: &DeliveryInspectQuery,
    now: UtcMillis,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    if !message_owned(db, instance, &q.message)? {
        return Err(api_error(ErrorCode::NotFound, "message not found"));
    }
    type Row = (
        String,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
    );
    let (thread,sequence,kind,author,label,body,event,at):Row=db.query_row("SELECT thread_id,sequence,kind,actor_seat_id,actor_label,body,event_json,decision_at FROM messages WHERE id=?1",[q.message.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(|e|db.map_error(e))?;
    let summary = message_summary(
        db,
        q.message.as_str(),
        &ThreadId::new(&thread),
        sequence,
        &kind,
        author.as_deref(),
        label.as_deref(),
        body.as_deref(),
        event.as_deref(),
        at,
        output,
    )?;
    let manifest_count: Option<i64> = db
        .query_row(
            "SELECT recipient_count FROM send_manifests WHERE message_id=?1",
            [q.message.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| db.map_error(e))?;
    let committed = if let Some(count) = manifest_count {
        count
    } else {
        db.query_row(
            "SELECT count(*) FROM receipts WHERE message_id=?1",
            [q.message.as_str()],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?
    };
    let acknowledged: i64 = if manifest_count.is_some() {
        db.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1 AND state='acked'",
            [q.message.as_str()],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?
    } else {
        db.query_row(
            "SELECT count(*) FROM receipts WHERE message_id=?1 AND state='acked'",
            [q.message.as_str()],
            |r| r.get(0),
        )
        .map_err(|e| db.map_error(e))?
    };
    let aggregates = DeliveryAggregates {
        committed: committed as u64,
        attempted: None,
        submitted: None,
        read: None,
        acknowledged: acknowledged as u64,
    };
    recipient_collection(
        db,
        instance,
        &q.message,
        &q.page,
        now,
        output,
        CursorScope::DeliveryInspect,
        |page| {
            CommandResult::DeliveryInspect(DeliveryInspection {
                message: summary.clone(),
                delivery: aggregates.clone(),
                recipients: page,
            })
        },
        |raw| {
            vec![
                "herdr-threads".into(),
                "delivery".into(),
                "inspect".into(),
                q.message.as_str().into(),
                "--cursor".into(),
                raw.into(),
                "--limit".into(),
                q.page.limit.to_string(),
                "--max-bytes".into(),
                q.page.max_bytes.to_string(),
            ]
        },
    )
}

fn diagnostics_argv(q: &DiagnosticsQuery, raw: &str) -> Vec<String> {
    let route = if q.seat.is_none() && q.thread.is_none() {
        "overdue"
    } else {
        "diagnostics"
    };
    let mut argv = continuation(route, None, raw, &q.page);
    if let Some(seat) = &q.seat {
        argv.splice(2..2, ["--seat".into(), seat.as_str().into()]);
    }
    if let Some(thread) = &q.thread {
        argv.splice(2..2, ["--thread".into(), thread.as_str().into()]);
    }
    argv
}

fn diagnostics_cursor(
    instance: &str,
    filter: &str,
    phase: SearchPhase,
    invitation_after: u64,
    invitation_high: u64,
    receipts: &ReceiptScanPosition,
) -> Result<String, ApiError> {
    let high = invitation_high.max(
        receipts
            .physical_high_water
            .max(receipts.manifest_high_water) as u64,
    );
    let mut cursor = cursor_for(
        instance,
        CursorScope::Diagnostics,
        "*",
        filter,
        CursorDirection::Ascending,
        0,
        high,
    );
    cursor.scope_revision = Some(invitation_high);
    cursor.filter_revision = Some(invitation_after);
    cursor.last_examined_key = Some(receipt_position_text(receipts));
    cursor.search = Some(SearchCursorState {
        phase,
        topic_high_water: invitation_high,
        body_high_water: high,
        topic_revision: 0,
        last_decision_seq: None,
        last_event_offset: None,
    });
    cursor
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))
}

/// Diagnostics walk state before a candidate, and the page high water at it.
#[derive(Clone)]
struct DiagnosticsWalk {
    phase: SearchPhase,
    invitation_after: u64,
    receipts: ReceiptScanPosition,
    high: u64,
}

fn diagnostics(
    db: &QueryConnection,
    instance: &str,
    q: &DiagnosticsQuery,
    now: UtcMillis,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let filter = digest(&(
        q.seat.as_ref().map(|s| s.as_str()),
        q.thread.as_ref().map(|t| t.as_str()),
    ))?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::Diagnostics,
        "*",
        &filter,
        CursorDirection::Ascending,
    )?;
    let (mut phase, mut invitation_after, invitation_high, mut receipts) =
        if let Some(c) = cursor.as_ref() {
            let phase = c
                .search
                .as_ref()
                .ok_or_else(|| api_error(ErrorCode::InvalidCursor, "diagnostics phase missing"))?
                .phase;
            (
                phase,
                c.filter_revision.ok_or_else(|| {
                    api_error(ErrorCode::InvalidCursor, "invitation position missing")
                })?,
                c.scope_revision.ok_or_else(|| {
                    api_error(ErrorCode::InvalidCursor, "invitation high water missing")
                })?,
                receipt_position_from_cursor(c)?,
            )
        } else {
            let invitation_high: i64 = db
                .query_row(
                    "SELECT coalesce(max(ordinal),0) FROM invitations",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| db.map_error(e))?;
            let physical_high_water: i64 = db
                .query_row("SELECT coalesce(max(ordinal),0) FROM receipts", [], |r| {
                    r.get(0)
                })
                .map_err(|e| db.map_error(e))?;
            let manifest_high_water: i64 = db
                .query_row(
                    "SELECT coalesce(max(ordinal),0) FROM prepared_recipients",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| db.map_error(e))?;
            (
                SearchPhase::Topic,
                0,
                invitation_high as u64,
                ReceiptScanPosition {
                    physical_after: 0,
                    manifest_after: 0,
                    physical_high_water,
                    manifest_high_water,
                    next_manifest: false,
                },
            )
        };
    if invitation_after > invitation_high {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "invitation position outside high water",
        ));
    }
    let mut items = Vec::new();
    let mut examined = 0usize;
    let mut stop = StopReason::Complete;
    while examined < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        match phase {
            SearchPhase::Topic => {
                type Row = (i64, String, String, String, String, i64);
                let row:Option<Row>=db.query_row("SELECT i.ordinal,i.id,i.thread_id,i.seat_id,CASE WHEN c.invitation_id IS NOT NULL THEN 'cancelled' ELSE i.state END,i.deadline_at FROM invitations i LEFT JOIN invitation_cancellations c ON c.invitation_id=i.id WHERE i.ordinal>?1 AND i.ordinal<=?2 ORDER BY i.ordinal LIMIT 1",params![invitation_after as i64,invitation_high as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|db.map_error(e))?;
                let Some((ordinal, id, thread, seat, state, deadline)) = row else {
                    phase = SearchPhase::Body;
                    continue;
                };
                examined += 1;
                let belongs: bool = db
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
                        params![thread, instance],
                        |r| r.get(0),
                    )
                    .map_err(|e| db.map_error(e))?;
                if belongs
                    && state == "pending"
                    && deadline <= now.0
                    && q.seat.as_ref().is_none_or(|s| s.as_str() == seat)
                    && q.thread.as_ref().is_none_or(|t| t.as_str() == thread)
                {
                    let retired: bool = db
                        .query_row(
                            "SELECT state='retired' FROM seats WHERE id=?1",
                            [&seat],
                            |r| r.get(0),
                        )
                        .map_err(|e| db.map_error(e))?;
                    if !retired {
                        let item = Diagnostic {
                            subject: format!("overdue_invitation:{id}"),
                            detail_data: format!("thread={thread} seat={seat} deadline={deadline}"),
                        };
                        let raw = diagnostics_cursor(
                            instance,
                            &filter,
                            phase,
                            ordinal as u64,
                            invitation_high,
                            &receipts,
                        )?;
                        items.push(Cand {
                            item,
                            argv: diagnostics_argv(q, &raw),
                            raw,
                            before: DiagnosticsWalk {
                                phase,
                                invitation_after,
                                receipts: receipts.clone(),
                                high: invitation_high.max(
                                    receipts
                                        .physical_high_water
                                        .max(receipts.manifest_high_water)
                                        as u64,
                                ),
                            },
                        });
                    }
                }
                invitation_after = ordinal as u64;
            }
            SearchPhase::Body => {
                let before = receipts.clone();
                let slice = scan_effective_receipts(
                    db,
                    &ReceiptScanScope::Instance(instance.into()),
                    Some(receipts.clone()),
                    1,
                )?;
                if slice.visited == 0 {
                    break;
                }
                examined += slice.visited as usize;
                for receipt in &slice.items {
                    if receipt.state != EffectiveReceiptState::Pending
                        || super::receipts::effective_deadline(db, receipt)?
                            .is_none_or(|d| d > now.0)
                        || q.seat
                            .as_ref()
                            .is_some_and(|s| s.as_str() != receipt.seat_id)
                        || q.thread
                            .as_ref()
                            .is_some_and(|t| t.as_str() != receipt.thread_id)
                    {
                        continue;
                    }
                    let item = Diagnostic {
                        subject: format!(
                            "overdue_receipt:{}:{}",
                            receipt.message_id, receipt.seat_id
                        ),
                        detail_data: format!(
                            "thread={} seat={} deadline={} source={:?}",
                            receipt.thread_id,
                            receipt.seat_id,
                            receipt.deadline_at.unwrap(),
                            receipt.source
                        ),
                    };
                    let raw = diagnostics_cursor(
                        instance,
                        &filter,
                        phase,
                        invitation_after,
                        invitation_high,
                        &slice.position,
                    )?;
                    items.push(Cand {
                        item,
                        argv: diagnostics_argv(q, &raw),
                        raw,
                        before: DiagnosticsWalk {
                            phase,
                            invitation_after,
                            receipts: before.clone(),
                            high: invitation_high.max(
                                slice
                                    .position
                                    .physical_high_water
                                    .max(slice.position.manifest_high_water)
                                    as u64,
                            ),
                        },
                    });
                }
                receipts = slice.position;
                if !slice.has_more {
                    break;
                }
            }
        }
    }
    let (items, cut) = fit_candidates(
        items,
        0,
        output,
        q.page.max_bytes,
        "diagnostic cannot fit",
        |mut diagnostics, at| {
            diagnostics.high_water_ordinal = at.before.high;
            CommandResult::Diagnostics(diagnostics)
        },
    )?;
    if let Some(before) = cut {
        phase = before.phase;
        invitation_after = before.invitation_after;
        receipts = before.receipts;
        stop = StopReason::Bytes;
    }
    if examined == CANDIDATE_LIMIT && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    let high = invitation_high.max(
        receipts
            .physical_high_water
            .max(receipts.manifest_high_water) as u64,
    );
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = diagnostics_cursor(
            instance,
            &filter,
            phase,
            invitation_after,
            invitation_high,
            &receipts,
        )?;
        Some((raw.clone(), diagnostics_argv(q, &raw)))
    };
    Ok(CommandResult::Diagnostics(sized(
        items,
        next,
        high,
        stop,
        CommandResult::Diagnostics,
        output,
        q.page.max_bytes,
    )?))
}

fn receipt_position_text(position: &ReceiptScanPosition) -> String {
    format!(
        "{},{},{},{},{}",
        position.physical_after,
        position.manifest_after,
        position.physical_high_water,
        position.manifest_high_water,
        u8::from(position.next_manifest)
    )
}

fn receipt_position_from_cursor(cursor: &Cursor) -> Result<ReceiptScanPosition, ApiError> {
    let raw = cursor
        .last_examined_key
        .as_deref()
        .ok_or_else(|| api_error(ErrorCode::InvalidCursor, "receipt position missing"))?;
    let parts: Vec<_> = raw.split(',').collect();
    if parts.len() != 5 {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "receipt position malformed",
        ));
    }
    let number = |s: &str| {
        s.parse::<i64>()
            .map_err(|_| api_error(ErrorCode::InvalidCursor, "receipt position malformed"))
    };
    let next_manifest = match parts[4] {
        "0" => false,
        "1" => true,
        _ => {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "receipt source malformed",
            ));
        }
    };
    let position = ReceiptScanPosition {
        physical_after: number(parts[0])?,
        manifest_after: number(parts[1])?,
        physical_high_water: number(parts[2])?,
        manifest_high_water: number(parts[3])?,
        next_manifest,
    };
    if position.physical_after < 0
        || position.manifest_after < 0
        || position.physical_after > position.physical_high_water
        || position.manifest_after > position.manifest_high_water
    {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "receipt position outside high water",
        ));
    }
    Ok(position)
}

fn receipt_cursor(
    instance: &str,
    scope: CursorScope,
    key: &str,
    filter: &str,
    position: &ReceiptScanPosition,
) -> Result<String, ApiError> {
    let high = position
        .physical_high_water
        .max(position.manifest_high_water) as u64;
    let mut cursor = cursor_for(
        instance,
        scope,
        key,
        filter,
        CursorDirection::Ascending,
        0,
        high,
    );
    cursor.last_examined_key = Some(receipt_position_text(position));
    cursor
        .encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))
}

fn pending_receipts_argv(
    q: &crate::protocol::commands::PendingReceiptsQuery,
    cursor: &str,
) -> Vec<String> {
    let mut argv = vec!["herdr-threads".into(), "pending-receipts".into()];
    if let Some(seat) = &q.seat {
        argv.extend(["--seat".into(), seat.as_str().into()]);
    }
    if let Some(thread) = &q.thread {
        argv.extend(["--thread".into(), thread.as_str().into()]);
    }
    argv.extend(["--cursor".into(), cursor.into()]);
    argv.extend(page_bounds(&q.page));
    argv
}

pub fn warnings_in_transaction(
    db: &Connection,
    instance: &str,
    q: &WarningsQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    q.page
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    output
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![q.seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "seat not found"));
    }
    let filter = digest(&q.seat.as_str())?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::Warnings,
        q.seat.as_str(),
        &filter,
        CursorDirection::Ascending,
    )?;
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM threads WHERE instance_id=?1",
                [instance],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(store_error)
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut thread_after = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut timeline = cursor
        .as_ref()
        .and_then(|c| c.last_examined_key.as_ref())
        .map(|s| {
            let (after, high) = s
                .split_once(':')
                .ok_or_else(|| api_error(ErrorCode::InvalidCursor, "warning position malformed"))?;
            Ok::<_, ApiError>(TimelinePosition {
                after_sequence: after.parse().map_err(|_| {
                    api_error(ErrorCode::InvalidCursor, "warning sequence malformed")
                })?,
                high_water_sequence: high.parse().map_err(|_| {
                    api_error(ErrorCode::InvalidCursor, "warning high water malformed")
                })?,
            })
        })
        .transpose()?;
    let mut items = Vec::new();
    let mut visited = 0usize;
    let mut stop = StopReason::Complete;
    while visited < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        let row: Option<(i64, String, i64)> = if timeline.is_some() {
            db.query_row(
                "SELECT ordinal,id,next_sequence FROM threads WHERE instance_id=?1 AND ordinal=?2",
                params![instance, thread_after as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(store_error)?
        } else {
            db.query_row("SELECT ordinal,id,next_sequence FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",params![instance,thread_after as i64,high as i64],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?
        };
        let Some((ordinal, thread, next_sequence)) = row else {
            break;
        };
        if timeline.is_none() {
            thread_after = ordinal as u64;
            timeline = Some(TimelinePosition {
                after_sequence: 0,
                high_water_sequence: next_sequence - 1,
            });
            visited += 1;
            if visited == CANDIDATE_LIMIT {
                stop = StopReason::Work;
                break;
            }
        }
        let before = timeline.clone();
        let slice =
            scan_effective_warnings_for_seat(db, &thread, q.seat.as_str(), timeline.clone(), 1)?;
        visited += slice.visited as usize;
        timeline = Some(slice.position.clone());
        for warning in slice.warnings {
            let item = WarningRef {
                warning: MessageId::new(&warning.id),
                thread: ThreadId::new(&thread),
                sequence: warning.sequence as u64,
                event_seq: warning.event_seq as u64,
            };
            let raw = warning_cursor(instance, q, thread_after, high, timeline.as_ref(), &filter)?;
            items.push(Cand {
                item,
                argv: warning_argv(q, &raw),
                raw,
                before: (thread_after, before.clone()),
            });
        }
        if !slice.has_more {
            timeline = None;
        }
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        q.page.max_bytes,
        "warning cannot fit",
        |page, _| CommandResult::Warnings(page),
    )?;
    if let Some((before_after, before_timeline)) = cut {
        thread_after = before_after;
        timeline = before_timeline;
        stop = StopReason::Bytes;
    }
    if stop == StopReason::Complete && visited == CANDIDATE_LIMIT {
        stop = StopReason::Work;
    }
    let more_threads: bool = if timeline.is_some() {
        true
    } else {
        db.query_row("SELECT EXISTS(SELECT 1 FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3)",params![instance,thread_after as i64,high as i64],|r|r.get(0)).map_err(store_error)?
    };
    if !more_threads {
        stop = StopReason::Complete;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = warning_cursor(instance, q, thread_after, high, timeline.as_ref(), &filter)?;
        Some((raw.clone(), warning_argv(q, &raw)))
    };
    Ok(CommandResult::Warnings(sized(
        items,
        next,
        high,
        stop,
        CommandResult::Warnings,
        output,
        q.page.max_bytes,
    )?))
}

fn warnings(
    db: &QueryConnection,
    instance: &str,
    q: &WarningsQuery,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    warnings_in_transaction(db, instance, q, output)
}

/// Pending-warning count for check-in (root contract decision, wave-2 (a)):
/// the seat's pending (actionable) warnings as defined by
/// `attention::pending_warnings`, read inside the caller's snapshot through
/// per-source `LIMIT cap+1` walks of the v8 pending-only projections and the
/// projection backlog (bounded-walk invariant, wave-2 fix1 (a)). Returns the
/// count saturated at `MAX_PENDING_WARNING_COUNT` and whether more may be
/// pending. Settled warnings, and programmatic notices already offered to
/// the seat's current occupant, are not counted; the full history stays with
/// the paginated `warnings` route. The caller's SQLite progress handler also
/// interrupts long statements before a check-in transaction can commit.
pub fn pending_warning_count_in_transaction(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    budget: &CallBudget,
    clock: &dyn Clock,
) -> Result<(u64, bool), ApiError> {
    let check = || {
        if budget.is_exhausted(clock) {
            Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "warning count budget exhausted",
            ))
        } else {
            Ok(())
        }
    };
    check()?;
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "seat not found"));
    }
    Ok(super::attention::seat_pending_warnings(db, seat.as_str(), &check)?.count())
}

fn warning_cursor(
    instance: &str,
    q: &WarningsQuery,
    thread_after: u64,
    high: u64,
    timeline: Option<&TimelinePosition>,
    filter: &str,
) -> Result<String, ApiError> {
    let mut c = cursor_for(
        instance,
        CursorScope::Warnings,
        q.seat.as_str(),
        filter,
        CursorDirection::Ascending,
        thread_after,
        high,
    );
    c.last_examined_key =
        timeline.map(|p| format!("{}:{}", p.after_sequence, p.high_water_sequence));
    c.encode()
        .map_err(|e| api_error(ErrorCode::InvalidCursor, e))
}

fn warning_argv(q: &WarningsQuery, cursor: &str) -> Vec<String> {
    vec![
        "herdr-threads".into(),
        "warnings".into(),
        "--seat".into(),
        q.seat.as_str().into(),
        "--cursor".into(),
        cursor.into(),
        "--limit".into(),
        q.page.limit.to_string(),
        "--max-bytes".into(),
        q.page.max_bytes.to_string(),
    ]
}

fn inbox(
    db: &QueryConnection,
    store: &StoreContext,
    instance: &str,
    q: &InboxQuery,
    output: &OutputSpec,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    let seat = q
        .seat
        .as_ref()
        .ok_or_else(|| api_error(ErrorCode::InvalidRequest, "inbox requires a resolved seat"))?;
    Ok(CommandResult::Inbox(inbox_in_transaction(
        db,
        instance,
        seat,
        &q.page,
        output,
        budget,
        store.clock(),
    )?))
}

/// The check-in builder calls this on its writer Transaction after the
/// provisional binding is visible. It shares that transaction's snapshot and
/// returns an exact count for every included thread or an explicit error.
pub fn inbox_in_transaction(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    request: &PageRequest,
    output: &OutputSpec,
    budget: &CallBudget,
    clock: &dyn Clock,
) -> Result<Page<InboxItem>, ApiError> {
    request
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    output
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    if budget.is_exhausted(clock) {
        return Err(api_error(
            ErrorCode::ReadBudgetExhausted,
            "inbox count budget exhausted",
        ));
    }
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "seat not found"));
    }
    let filter = digest(&seat.as_str())?;
    let cursor = decode_cursor(
        request,
        instance,
        CursorScope::Inbox,
        seat.as_str(),
        &filter,
        CursorDirection::Ascending,
    )?;
    let start_after = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut token = match cursor.as_ref() {
        Some(cursor) => InboxCapture::from_cursor_state(cursor.inbox.clone().ok_or_else(|| {
            api_error(ErrorCode::InvalidCursor, "inbox publication token missing")
        })?),
        None => capture_inbox_token(db, instance, seat.as_str())?,
    };
    let mut validation_visits = 0usize;
    if let Some(current) = cursor.as_ref() {
        match validate_inbox_token(
            db,
            instance,
            seat.as_str(),
            start_after,
            token,
            CANDIDATE_LIMIT as u16,
            budget,
            clock,
        )? {
            InboxValidation::Changed => {
                return Err(
                    ApiError::cursor_stale("inbox inclusion changed").with_restart_argv(
                        contextual_argv(inbox_request_argv(seat, None, request), output),
                    ),
                );
            }
            InboxValidation::More { token: next, .. } => {
                let high = current.high_water_ordinal;
                let mut continuation = current.clone();
                continuation.inbox = Some(next.to_cursor_state());
                let raw = continuation
                    .encode()
                    .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
                return sized(
                    Vec::new(),
                    Some((raw.clone(), inbox_argv(seat, &raw, request))),
                    high,
                    StopReason::Work,
                    CommandResult::Inbox,
                    output,
                    request.max_bytes,
                );
            }
            InboxValidation::Current {
                token: next,
                visited,
            } => {
                token = next;
                validation_visits = visited as usize;
            }
        }
    }
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM threads WHERE instance_id=?1",
                [instance],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(store_error)
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let check = || {
        if budget.is_exhausted(clock) {
            Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "inbox warning count budget exhausted",
            ))
        } else {
            Ok(())
        }
    };
    // The projection backlog is read once per page; every other source is a
    // per-thread `LIMIT cap+1` walk (bounded-walk invariant).
    let backlog = super::attention::warning_backlog(db, seat.as_str(), &check)?;
    let mut last = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut visited = validation_visits;
    let mut stop = StopReason::Complete;
    while visited < CANDIDATE_LIMIT {
        if budget.is_exhausted(clock) {
            return Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "inbox count budget exhausted",
            ));
        }
        if items.len() == request.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        let row:Option<(i64,String)>=db.query_row("SELECT ordinal,id FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",params![instance,last as i64,high as i64],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
        let Some((ordinal, thread)) = row else { break };
        visited += 1;
        // Pending only, per source at most `cap+1` rows (wave-2 fix1 (a)).
        let (invitations, invitations_has_more) =
            super::attention::pending_invitations(db, seat.as_str(), Some(&thread), i64::MAX)?
                .count();
        check()?;
        let (pending, pending_receipts_has_more) =
            super::attention::pending_receipts(db, seat.as_str(), Some(&thread))?.count();
        check()?;
        let (warnings, warnings_has_more) =
            super::attention::pending_warnings(db, seat.as_str(), Some(&thread), &backlog, &check)?
                .count();
        if invitations > 0 || pending > 0 || warnings > 0 {
            let pending_requirement =
                super::service_substrate::current_requirement(db, &ThreadId::new(&thread), seat)?
                    .filter(|r| r.state == crate::protocol::service::RequirementState::Pending);
            let item = InboxItem {
                thread: ThreadId::new(&thread),
                invitations,
                invitations_has_more,
                pending_receipts: pending,
                pending_receipts_has_more,
                warnings,
                warnings_has_more,
                pending_requirement,
            };
            let mut trial_cursor = cursor_for(
                instance,
                CursorScope::Inbox,
                seat.as_str(),
                &filter,
                CursorDirection::Ascending,
                ordinal as u64,
                high,
            );
            let mut trial_token = token.clone();
            trial_token.reset_for_next_thread_prefix();
            trial_cursor.inbox = Some(trial_token.to_cursor_state());
            let raw = trial_cursor
                .encode()
                .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
            items.push(Cand {
                item,
                argv: inbox_argv(seat, &raw, request),
                raw,
                before: last,
            });
        }
        last = ordinal as u64;
    }
    let (items, cut) = fit_candidates(
        items,
        high,
        output,
        request.max_bytes,
        "inbox item cannot fit",
        |page, _| CommandResult::Inbox(page),
    )?;
    if let Some(before) = cut {
        last = before;
        stop = StopReason::Bytes;
    }
    if stop == StopReason::Complete && visited == CANDIDATE_LIMIT {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let mut next_cursor = cursor_for(
            instance,
            CursorScope::Inbox,
            seat.as_str(),
            &filter,
            CursorDirection::Ascending,
            last,
            high,
        );
        if last > start_after {
            token.reset_for_next_thread_prefix();
        }
        next_cursor.inbox = Some(token.to_cursor_state());
        let raw = next_cursor
            .encode()
            .map_err(|e| api_error(ErrorCode::InvalidCursor, e))?;
        Some((raw.clone(), inbox_argv(seat, &raw, request)))
    };
    if budget.is_exhausted(clock) {
        return Err(api_error(
            ErrorCode::ReadBudgetExhausted,
            "inbox count budget exhausted",
        ));
    }
    sized(
        items,
        next,
        high,
        stop,
        CommandResult::Inbox,
        output,
        request.max_bytes,
    )
}

fn inbox_argv(seat: &SeatId, cursor: &str, page: &PageRequest) -> Vec<String> {
    inbox_request_argv(seat, Some(cursor), page)
}

fn inbox_request_argv(seat: &SeatId, cursor: Option<&str>, page: &PageRequest) -> Vec<String> {
    let mut argv = vec![
        "herdr-threads".into(),
        "inbox".into(),
        "--seat".into(),
        seat.as_str().into(),
    ];
    argv.extend(page_bounds(page));
    if let Some(cursor) = cursor {
        argv.push("--cursor".into());
        argv.push(cursor.into());
    }
    argv
}

fn pending_receipts(
    db: &QueryConnection,
    instance: &str,
    q: &crate::protocol::commands::PendingReceiptsQuery,
    now: UtcMillis,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    if q.seat.is_none() && q.thread.is_none() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "pending receipts require seat or thread",
        ));
    }
    let scope_key = q
        .seat
        .as_ref()
        .map(|s| s.as_str())
        .or_else(|| q.thread.as_ref().map(|t| t.as_str()))
        .unwrap();
    let owned: bool = if let Some(seat) = &q.seat {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
    } else {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
            params![scope_key, instance],
            |r| r.get(0),
        )
    }
    .map_err(|e| db.map_error(e))?;
    if !owned {
        return Err(api_error(
            ErrorCode::NotFound,
            "pending receipt scope not found",
        ));
    }
    if let Some(thread) = &q.thread {
        let thread_owned: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
                params![thread.as_str(), instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        if !thread_owned {
            return Err(api_error(ErrorCode::NotFound, "thread not found"));
        }
    }
    let filter = digest(&(
        q.seat.as_ref().map(|v| v.as_str()),
        q.thread.as_ref().map(|v| v.as_str()),
    ))?;
    let cursor = decode_cursor(
        &q.page,
        instance,
        CursorScope::PendingReceipts,
        scope_key,
        &filter,
        CursorDirection::Ascending,
    )?;
    let mut position = cursor
        .as_ref()
        .map(receipt_position_from_cursor)
        .transpose()?;
    let mut items = Vec::new();
    let mut visited = 0usize;
    let mut stop = StopReason::Complete;
    while visited < CANDIDATE_LIMIT {
        if items.len() == q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        let before = position.clone();
        let slice = scan_effective_receipts(
            db,
            &if q.seat.is_some() {
                ReceiptScanScope::Seat(scope_key.into())
            } else {
                ReceiptScanScope::Thread(scope_key.into())
            },
            position.clone(),
            1,
        )?;
        if slice.visited == 0 {
            position = Some(slice.position);
            break;
        }
        visited += slice.visited as usize;
        let has_more = slice.has_more;
        for receipt in &slice.items {
            if receipt.state != EffectiveReceiptState::Pending
                || q.thread
                    .as_ref()
                    .is_some_and(|t| t.as_str() != receipt.thread_id)
            {
                continue;
            }
            let (sender, sender_author) = match super::service_substrate::message_author(
                db,
                &MessageId::new(&receipt.message_id),
            )? {
                EventAuthor::Native(seat) => (Some(seat), None),
                author @ EventAuthor::Programmatic(_) => (None, Some(author)),
                EventAuthor::BuiltIn => {
                    return Err(api_error(ErrorCode::StoreCorrupt, "receipt sender missing"));
                }
            };
            let display = deadline_display(db, receipt, now)?;
            let item = PendingReceipt {
                message: MessageId::new(&receipt.message_id),
                thread: ThreadId::new(&receipt.thread_id),
                seat: SeatId::new(&receipt.seat_id),
                sequence: receipt.sequence as u64,
                sender,
                sender_author,
                decision_at: UtcMillis(receipt.decision_at),
                available_at: receipt.available_at.map(UtcMillis),
                deadline: receipt.deadline_at.map(UtcMillis),
                overdue: display.overdue,
                effective_deadline: display.effective,
                deferred_until: display.deferred_until,
            };
            let raw = receipt_cursor(
                instance,
                CursorScope::PendingReceipts,
                scope_key,
                &filter,
                &slice.position,
            )?;
            items.push(Cand {
                item,
                argv: pending_receipts_argv(q, &raw),
                raw,
                before: (
                    before.clone(),
                    slice
                        .position
                        .physical_high_water
                        .max(slice.position.manifest_high_water) as u64,
                ),
            });
        }
        position = Some(slice.position);
        if !has_more {
            break;
        }
    }
    let (items, cut) = fit_candidates(
        items,
        0,
        output,
        q.page.max_bytes,
        "pending receipt cannot fit",
        |mut receipts, at| {
            receipts.high_water_ordinal = at.before.1;
            CommandResult::PendingReceipts(receipts)
        },
    )?;
    if let Some((before, _)) = cut {
        position = before;
        stop = StopReason::Bytes;
    }
    let position = position
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "receipt scan did not initialize"))?;
    let high = position
        .physical_high_water
        .max(position.manifest_high_water) as u64;
    if stop == StopReason::Complete && visited == CANDIDATE_LIMIT {
        stop = StopReason::Work;
    }
    let next = if stop == StopReason::Complete {
        None
    } else {
        let raw = receipt_cursor(
            instance,
            CursorScope::PendingReceipts,
            scope_key,
            &filter,
            &position,
        )?;
        Some((raw.clone(), pending_receipts_argv(q, &raw)))
    };
    Ok(CommandResult::PendingReceipts(sized(
        items,
        next,
        high,
        stop,
        CommandResult::PendingReceipts,
        output,
        q.page.max_bytes,
    )?))
}

/// Hot threads of one seat for the recovery hook text (spec §9), computed in
/// one read transaction: threads with a pending receipt (earliest effective
/// deadline first), then other pending attention (invitations, actionable
/// warnings), then joined threads whose latest ordinary message is newer than
/// `hot_window_ms`, newest first. The first `q.limit` are returned in full,
/// up to `MAX_HOT_OVERFLOW` more as bare ids.
pub fn hot_threads(
    store: &StoreContext,
    instance: &str,
    q: &crate::protocol::commands::HotThreadsQuery,
    hot_window_ms: u64,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    Command::HotThreads(q.clone())
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let db = store.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|e| db.map_error(e))?;
    let now = store.clock().utc_now().0;
    let result = hot_threads_in(&db, instance, q, now, hot_window_ms, &|| db.check_budget());
    result.map_err(|error| read_budget_error(error, budget, store.clock()))
}

/// A hot-thread topic: control characters stripped, cut to `HOT_TOPIC_BYTES`
/// at a char boundary.
pub fn hot_topic(topic: &str) -> String {
    let mut out = String::new();
    for c in topic.chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > crate::protocol::results::HOT_TOPIC_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

fn hot_threads_in(
    db: &Connection,
    instance: &str,
    q: &crate::protocol::commands::HotThreadsQuery,
    now: i64,
    hot_window_ms: u64,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<CommandResult, ApiError> {
    use crate::protocol::results::{HotReason, HotThread, HotThreads, MAX_HOT_OVERFLOW};
    use std::collections::BTreeMap;
    let seat = q.seat.as_str();
    let decision_seq: i64 = db
        .query_row(
            "SELECT h.decision_seq FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
            params![seat, instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "seat not found"))?;
    // thread -> (reason, earliest effective deadline of its pending receipts)
    let mut found: BTreeMap<String, (HotReason, Option<i64>)> = BTreeMap::new();
    let receipts = super::attention::pending_receipts(db, seat, None)?;
    check_budget()?;
    for item in &receipts.items {
        let deadline = match super::effective::effective_receipt(db, &item.id, seat)? {
            Some(receipt) => super::receipts::effective_deadline(db, &receipt)?,
            None => None,
        };
        let entry = found
            .entry(item.thread_id.clone())
            .or_insert((HotReason::PendingReceipt, deadline));
        entry.1 = match (entry.1, deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    let invitations = super::attention::pending_invitations(db, seat, None, decision_seq)?;
    check_budget()?;
    let warnings = super::attention::seat_pending_warnings(db, seat, check_budget)?;
    check_budget()?;
    for item in invitations.items.iter().chain(&warnings.items) {
        found
            .entry(item.thread_id.clone())
            .or_insert((HotReason::Attention, None));
    }
    let since = now.saturating_sub(i64::try_from(hot_window_ms).unwrap_or(i64::MAX));
    let latest = |thread: &str| -> Result<i64, ApiError> {
        Ok(db
            .query_row(
                "SELECT decision_at FROM messages WHERE thread_id=?1 AND kind='ordinary' ORDER BY sequence DESC LIMIT 1",
                [thread],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?
            .unwrap_or(0))
    };
    let joined: Vec<String> = {
        let mut stmt = db
            .prepare(
                "SELECT t.id FROM memberships ms JOIN threads t ON t.id=ms.thread_id WHERE ms.seat_id=?1 AND ms.state='joined' AND t.instance_id=?2 AND t.archived=0 ORDER BY ms.ordinal DESC LIMIT 512",
            )
            .map_err(store_error)?;
        stmt.query_map(params![seat, instance], |r| r.get(0))
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
    };
    check_budget()?;
    for thread in joined {
        if !found.contains_key(&thread) && latest(&thread)? > since {
            found.insert(thread, (HotReason::Recent, None));
        }
    }
    let mut rows = Vec::with_capacity(found.len());
    for (thread, (reason, deadline)) in found {
        let topic: String = db
            .query_row(
                "SELECT topic FROM threads WHERE id=?1 AND instance_id=?2",
                params![thread, instance],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?
            .unwrap_or_default();
        let last_activity = latest(&thread)?;
        rows.push(HotThread {
            thread: ThreadId::new(thread),
            topic_data: hot_topic(&topic),
            reason,
            effective_deadline: deadline.map(UtcMillis),
            last_activity: UtcMillis(last_activity),
        });
    }
    let rank = |reason: HotReason| match reason {
        HotReason::PendingReceipt => 0,
        HotReason::Attention => 1,
        HotReason::Recent => 2,
    };
    rows.sort_by(|a, b| {
        rank(a.reason)
            .cmp(&rank(b.reason))
            // A receipt without a deadline sorts after every dated one.
            .then_with(|| {
                a.effective_deadline
                    .map_or((1, 0), |d| (0, d.0))
                    .cmp(&b.effective_deadline.map_or((1, 0), |d| (0, d.0)))
            })
            .then_with(|| b.last_activity.0.cmp(&a.last_activity.0))
            .then_with(|| a.thread.as_str().cmp(b.thread.as_str()))
    });
    let limit = q.limit as usize;
    let overflow = rows
        .iter()
        .skip(limit)
        .take(MAX_HOT_OVERFLOW)
        .map(|row| row.thread.clone())
        .collect();
    rows.truncate(limit);
    Ok(CommandResult::HotThreads(HotThreads {
        hot: rows,
        overflow,
    }))
}

#[cfg(test)]
#[path = "../../tests/store/queries.rs"]
mod tests;
