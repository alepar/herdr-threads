//! Bounded projections of already published logical work.

use super::{
    connection::{api_error, store_error},
    effective::{EffectiveReceiptState, effective_receipt, effective_warning_by_id},
    work::commit_work_prefix,
};
use crate::{
    ports::{DurableWorkAdmission as WorkAdmission, WorkProgress},
    protocol::{
        results::{ApiError, ErrorCode},
        time::{CallBudget, Clock},
    },
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone, Copy)]
enum Kind {
    Warning,
    Send,
    Timer,
    Cleanup,
    HumanReceipts,
}
struct Job {
    kind: Kind,
    subject: String,
    position: u64,
    high_water: u64,
}

pub fn advance_work(
    db: &mut Connection,
    job_id: &str,
    admission: WorkAdmission,
    budget: &CallBudget,
    clock: &dyn Clock,
) -> Result<WorkProgress, ApiError> {
    if !(1..=16).contains(&admission.max_units) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid work admission",
        ));
    }
    if budget.is_exhausted(clock) {
        return Err(budget_error(budget));
    }
    let started = clock.monotonic_now().0;
    let mut tx = db.transaction().map_err(store_error)?;
    let job = load_job(&tx, job_id)?;
    let mut position = job.position;
    let mut units = 0;
    let mut done = false;
    let mut failure = None;
    while units < admission.max_units {
        if budget.is_exhausted(clock) {
            failure = Some(budget_error(budget));
            break;
        }
        if clock.monotonic_now().0.saturating_sub(started) >= 5 {
            break;
        }
        let mut save = tx.savepoint().map_err(store_error)?;
        match advance_unit(&save, &job, position) {
            Ok((next, complete)) => {
                save.commit().map_err(store_error)?;
                position = next;
                done = complete;
                units += 1;
                if done {
                    break;
                }
            }
            Err(error) => {
                save.rollback().map_err(store_error)?;
                failure = Some(error);
                break;
            }
        }
    }
    if units == 0 && failure.is_none() {
        return Err(budget_error(budget));
    }
    let error_text = failure
        .as_ref()
        .map(|e: &ApiError| bounded_work_diagnostic(&e.detail));
    if matches!(job.kind, Kind::Warning) {
        if let Some(error) = error_text.as_deref() {
            tx.execute(
                "UPDATE warning_jobs SET status='failed',last_error=?1 WHERE warning_id=?2",
                params![error, job.subject],
            )
            .map_err(store_error)?;
        } else if !done {
            tx.execute(
                "UPDATE warning_jobs SET status='pending',last_error=NULL WHERE warning_id=?1",
                [job.subject.as_str()],
            )
            .map_err(store_error)?;
        }
    }
    if units == 0 {
        let completed_units: i64 = tx
            .query_row(
                "SELECT completed_units FROM work_jobs WHERE id=?1",
                [job_id],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        tx.execute(
            "UPDATE work_jobs SET status='failed',last_error=?1 WHERE id=?2 AND position=?3",
            params![error_text, job_id, sql(job.position)?],
        )
        .map_err(store_error)?;
        tx.commit().map_err(store_error)?;
        return Ok(WorkProgress {
            completed_units: u64::try_from(completed_units)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative work count"))?,
            processed_this_turn: 0,
            has_more: true,
            next_position: position,
            last_error: error_text,
        });
    }
    let result = commit_work_prefix(
        &tx,
        job_id,
        job.position,
        units,
        position,
        !done,
        error_text.as_deref(),
        clock,
    )?;
    tx.commit().map_err(store_error)?;
    Ok(result)
}

fn bounded_work_diagnostic(detail: &str) -> String {
    let mut end = detail.len().min(512);
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail[..end].to_owned()
}

fn budget_error(budget: &CallBudget) -> ApiError {
    if budget.cancellation.is_cancelled() {
        api_error(ErrorCode::Cancelled, "materialization cancelled")
    } else {
        api_error(
            ErrorCode::DeadlineExceeded,
            "materialization admission exhausted",
        )
    }
}

fn sql(v: u64) -> Result<i64, ApiError> {
    i64::try_from(v).map_err(|_| api_error(ErrorCode::StoreCorrupt, "work cursor overflow"))
}

fn load_job(tx: &Connection, id: &str) -> Result<Job, ApiError> {
    let row:Option<(String,String,i64,i64)>=tx.query_row("SELECT kind,subject_id,position,high_water FROM work_jobs WHERE id=?1 AND status IN ('pending','failed')",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(store_error)?;
    let Some((kind, subject, position, high_water)) = row else {
        return Err(api_error(
            ErrorCode::Conflict,
            "work job missing or complete",
        ));
    };
    let kind = match kind.as_str() {
        "warning_attribution" => Kind::Warning,
        "send_attention" => Kind::Send,
        "receipt_timer_materialization" => Kind::Timer,
        "preparation_cleanup" => Kind::Cleanup,
        "human_receipt_reconciliation" => Kind::HumanReceipts,
        _ => return Err(api_error(ErrorCode::StoreCorrupt, "unknown work kind")),
    };
    Ok(Job {
        kind,
        subject,
        position: u64::try_from(position)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative cursor"))?,
        high_water: u64::try_from(high_water)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative high water"))?,
    })
}

fn advance_unit(tx: &Connection, job: &Job, position: u64) -> Result<(u64, bool), ApiError> {
    match job.kind {
        Kind::Warning => warning_unit(tx, &job.subject, position, job.high_water),
        Kind::Send => send_unit(tx, &job.subject, position, job.high_water),
        Kind::Timer => timer_unit(tx, &job.subject, position, job.high_water),
        Kind::Cleanup => cleanup_unit(tx, &job.subject, position),
        Kind::HumanReceipts => human_receipt_unit(tx, &job.subject, position, job.high_water),
    }
}

/// Reconcile one ordinal from each physical source. Both cursors share the
/// same numeric position; equal ordinals are processed together. A later
/// agent's rows are above the job's captured high water and stay required.
fn human_receipt_unit(
    tx: &Connection,
    seat: &str,
    position: u64,
    high_water: u64,
) -> Result<(u64, bool), ApiError> {
    let (prepared_high_water, physical_high_water): (i64, i64) = tx.query_row(
        "SELECT prepared_high_water,physical_high_water FROM human_receipt_reconciliation_bounds WHERE seat_id=?1",
        [seat], |r| Ok((r.get(0)?, r.get(1)?)),
    ).map_err(store_error)?;
    let next_prepared: Option<i64> = tx.query_row(
        "SELECT ordinal FROM prepared_recipients INDEXED BY prepared_recipients_required_seat WHERE seat_id=?1 AND ack_required=1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
        params![seat, sql(position)?, prepared_high_water], |r| r.get(0),
    ).optional().map_err(store_error)?;
    let next_physical: Option<i64> = tx.query_row(
        "SELECT ordinal FROM receipts INDEXED BY receipts_required_seat_pending WHERE seat_id=?1 AND state='pending' AND ack_required=1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
        params![seat, sql(position)?, physical_high_water], |r| r.get(0),
    ).optional().map_err(store_error)?;
    let Some(next) = next_prepared.into_iter().chain(next_physical).min() else {
        return Ok((high_water, true));
    };
    if next_prepared == Some(next) {
        let preparation: String = tx
            .query_row(
                "SELECT preparation_id FROM prepared_recipients WHERE seat_id=?1 AND ordinal=?2",
                params![seat, next],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        tx.execute("UPDATE prepared_recipients SET ack_required=0 WHERE seat_id=?1 AND ordinal=?2 AND ack_required=1",
            params![seat,next]).map_err(store_error)?;
        tx.execute("UPDATE receipt_state SET ack_required=0 WHERE seat_id=?1 AND state='pending' AND ack_required=1 AND message_id IN (SELECT message_id FROM send_manifests WHERE preparation_id=?2)",
            params![seat,preparation]).map_err(store_error)?;
    }
    if next_physical == Some(next) {
        tx.execute("UPDATE receipts SET ack_required=0 WHERE seat_id=?1 AND ordinal=?2 AND state='pending' AND ack_required=1",
            params![seat,next]).map_err(store_error)?;
    }
    Ok((
        u64::try_from(next)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative human receipt cursor"))?,
        false,
    ))
}

fn warning_unit(
    tx: &Connection,
    warning: &str,
    position: u64,
    high_water: u64,
) -> Result<(u64, bool), ApiError> {
    let (event_seq,thread,affected,phase):(i64,String,Option<String>,String)=tx.query_row("SELECT event_seq,thread_id,affected_seat_id,phase FROM warning_jobs WHERE warning_id=?1 AND status IN ('pending','failed')",[warning],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(store_error)?;
    match phase.as_str() {
        "intervals" => {
            type RowColumns = Option<(i64, String, i64, Option<i64>, Option<i64>)>;
            let row:RowColumns=tx.query_row("SELECT mi.ordinal,mi.seat_id,mi.joined_seq,mi.left_seq,s.retired_seq FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal>?2 AND mi.ordinal<=?3 ORDER BY mi.ordinal LIMIT 1",params![thread,sql(position)?,sql(high_water)?],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(store_error)?;
            if let Some((ordinal, seat, joined, left, retired)) = row {
                if joined <= event_seq
                    && left.is_none_or(|v| v > event_seq)
                    && retired.is_none_or(|v| v > event_seq)
                {
                    attribute(tx, warning, &seat, event_seq)?;
                }
                tx.execute("UPDATE warning_jobs SET cursor_ordinal=?1,processed_units=processed_units+1 WHERE warning_id=?2",params![ordinal,warning]).map_err(store_error)?;
                return Ok((
                    u64::try_from(ordinal).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative interval ordinal")
                    })?,
                    false,
                ));
            }
            tx.execute("UPDATE warning_jobs SET phase='affected',cursor_ordinal=?1,processed_units=processed_units+1 WHERE warning_id=?2",params![sql(high_water)?,warning]).map_err(store_error)?;
            Ok((high_water, false))
        }
        "affected" => {
            if let Some(seat) = affected {
                let retired: Option<i64> = tx
                    .query_row(
                        "SELECT retired_seq FROM seats WHERE id=?1",
                        [seat.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                if retired.is_none_or(|v| v > event_seq) {
                    attribute(tx, warning, &seat, event_seq)?;
                }
            }
            tx.execute("UPDATE warning_jobs SET phase='finalize',processed_units=processed_units+1 WHERE warning_id=?1",[warning]).map_err(store_error)?;
            Ok((position, false))
        }
        "finalize" => {
            tx.execute("UPDATE warning_jobs SET phase='complete',status='complete',last_error=NULL,processed_units=processed_units+1 WHERE warning_id=?1",[warning]).map_err(store_error)?;
            Ok((high_water, true))
        }
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid warning job phase",
        )),
    }
}

fn attribute(tx: &Connection, warning: &str, seat: &str, event_seq: i64) -> Result<(), ApiError> {
    let inserted=tx.execute("INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES(?1,?2,?3) ON CONFLICT(warning_id,seat_id) DO NOTHING",params![warning,seat,event_seq]).map_err(store_error)?;
    if inserted == 0 {
        let prior: i64 = tx
            .query_row(
                "SELECT generation FROM warning_recipients WHERE warning_id=?1 AND seat_id=?2",
                params![warning, seat],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if prior != event_seq {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "warning recipient generation conflicts with immutable event",
            ));
        }
    }
    if inserted > 0 && actionable(tx, warning, seat)? && !offered(tx, seat, event_seq)? {
        bump(tx, seat)?;
    }
    Ok(())
}

fn actionable(tx: &Connection, warning: &str, seat: &str) -> Result<bool, ApiError> {
    let state: String = tx
        .query_row("SELECT state FROM seats WHERE id=?1", [seat], |r| r.get(0))
        .map_err(store_error)?;
    if state == "retired" {
        return Ok(false);
    }
    let (kind,condition,affected):(String,String,Option<String>)=tx.query_row("SELECT condition_kind,condition_id,affected_seat_id FROM warning_jobs WHERE warning_id=?1",[warning],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(store_error)?;
    match kind.as_str() {
        "invitation" => tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM invitations i WHERE i.id=?1 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id))",
                [condition],
                |r| r.get(0),
            )
            .map_err(store_error),
        "receipt" => {
            let source: Option<String> = tx
                .query_row(
                    "SELECT source_message_id FROM messages WHERE id=?1",
                    [warning],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?
                .flatten();
            match (source, affected) {
                (Some(message), Some(owner)) => Ok(effective_receipt(tx, &message, &owner)?
                    .is_some_and(|r| r.state == EffectiveReceiptState::Pending)),
                _ => Ok(false),
            }
        }
        "unavailable" => {
            let Some(owner) = affected else {
                return Ok(false);
            };
            let source = effective_warning_by_id(tx, warning)?.and_then(|w| w.source_message_id);
            let Some(source) = source else {
                return Ok(false);
            };
            if !effective_receipt(tx, &source, &owner)?
                .is_some_and(|r| r.state == EffectiveReceiptState::Pending)
            {
                return Ok(false);
            }
            let episode:Option<i64>=tx.query_row("SELECT unavailability_episode FROM prepared_unavailable_warnings WHERE warning_id=?1",[warning],|r|r.get(0)).optional().map_err(store_error)?;
            let (current,open,available):(i64,i64,bool)=tx.query_row("SELECT s.unavailability_episode,s.unavailability_open,EXISTS(SELECT 1 FROM occupant_bindings b JOIN host_instances h ON h.id=s.instance_id WHERE b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL AND b.registered_at IS NOT NULL AND b.host_boot=h.host_boot AND b.host_epoch=h.host_epoch) FROM seats s WHERE s.id=?1",[owner.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(store_error)?;
            Ok(episode == Some(current) && open == 1 && !available && !condition.is_empty())
        }
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "unknown warning condition",
        )),
    }
}

fn offered(tx: &Connection, seat: &str, event_seq: i64) -> Result<bool, ApiError> {
    tx.query_row("SELECT EXISTS(SELECT 1 FROM warning_offer o JOIN seats s ON s.id=o.seat_id JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE o.seat_id=?1 AND o.binding_generation=s.generation AND o.execution_id=b.execution_id AND o.offered_through_seq>=?2)",params![seat,event_seq],|r|r.get(0)).map_err(store_error)
}

fn bump(tx: &Connection, seat: &str) -> Result<(), ApiError> {
    tx.execute("INSERT INTO wake_work(seat_id,attention_version) VALUES(?1,1) ON CONFLICT(seat_id) DO UPDATE SET attention_version=attention_version+1",[seat]).map_err(store_error)?;
    Ok(())
}

fn send_unit(
    tx: &Connection,
    preparation: &str,
    position: u64,
    high_water: u64,
) -> Result<(u64, bool), ApiError> {
    if let Some(preparation) = preparation.strip_prefix("service-notify:") {
        return service_notification_unit(tx, preparation, position, high_water);
    }
    let (message,seq,recipients,warnings,thread,interval_high_water):(String,i64,i64,i64,String,i64)=tx.query_row("SELECT message_id,decision_seq,recipient_count,warning_count,thread_id,interval_high_water FROM send_manifests WHERE preparation_id=?1",[preparation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(store_error)?;
    let total = recipients
        .checked_add(warnings)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "send work count overflow"))?;
    if sql(high_water)? != total {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "send work high water mismatch",
        ));
    }
    let next = sql(position)?
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "send cursor overflow"))?;
    if next <= recipients {
        let seat:String=tx.query_row("SELECT seat_id FROM prepared_recipients WHERE preparation_id=?1 AND receipt_ordinal=?2",params![preparation,next],|r|r.get(0)).map_err(store_error)?;
        project_receipt(tx, &message, &seat)?;
        return Ok((position + 1, false));
    }
    if next <= recipients + warnings {
        let offset = next - recipients;
        let (warning,affected,key):(String,String,String)=tx.query_row("SELECT warning_id,affected_seat_id,warning_key FROM prepared_unavailable_warnings WHERE preparation_id=?1 AND warning_offset=?2",params![preparation,offset],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(store_error)?;
        tx.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES(?1,?2,?3,?4,?5,'unavailable',?6)",params![warning,seq,thread,interval_high_water,affected,key]).map_err(store_error)?;
        tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES(?1,'warning_attribution',?2,?3)",params![format!("work:{warning}"),warning,interval_high_water]).map_err(store_error)?;
        return Ok((position + 1, false));
    }
    Ok((high_water, true))
}

fn service_notification_unit(
    tx: &Connection,
    preparation: &str,
    position: u64,
    high_water: u64,
) -> Result<(u64, bool), ApiError> {
    let (message,seq): (String,i64)=tx.query_row(
        "SELECT message_id,decision_seq FROM service_notification_publications WHERE preparation_id=?1",
        [preparation],|r|Ok((r.get(0)?,r.get(1)?))).map_err(store_error)?;
    let row: Option<(i64,String)>=tx.query_row(
        "SELECT ordinal,seat_id FROM service_notification_recipients WHERE preparation_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
        params![preparation,sql(position)?,sql(high_water)?],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
    let Some((ordinal, seat)) = row else {
        return Ok((high_water, true));
    };
    let inserted=tx.execute(
        "INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES(?1,?2,?3) ON CONFLICT(warning_id,seat_id) DO NOTHING",
        params![message,seat,seq]).map_err(store_error)?;
    let retired: bool = tx
        .query_row(
            "SELECT state='retired' FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if inserted > 0 && !retired && !offered(tx, &seat, seq)? {
        bump(tx, &seat)?;
    }
    Ok((ordinal as u64, false))
}

fn project_receipt(tx: &Connection, message: &str, seat: &str) -> Result<(), ApiError> {
    let effective = effective_receipt(tx, message, seat)?
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "published receipt missing"))?;
    let prior: Option<String> = tx
        .query_row(
            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
            params![message, seat],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let physical_state = match effective.state {
        EffectiveReceiptState::Pending => "pending",
        EffectiveReceiptState::Acknowledged => "acked",
        EffectiveReceiptState::RecipientRetired => "recipient_retired",
        EffectiveReceiptState::NotRequired => "pending",
    };
    let ack_required = i64::from(effective.state != EffectiveReceiptState::NotRequired);
    tx.execute("INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at,ack_required) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(message_id,seat_id) DO UPDATE SET available_at=COALESCE(receipt_state.available_at,excluded.available_at),deadline_at=COALESCE(receipt_state.deadline_at,excluded.deadline_at),ack_required=MIN(receipt_state.ack_required,excluded.ack_required) WHERE receipt_state.state='pending'",params![message,seat,physical_state,effective.available_at,effective.deadline_at,ack_required]).map_err(store_error)?;
    if effective.state == EffectiveReceiptState::Pending && prior.is_none() {
        bump(tx, seat)?;
    }
    Ok(())
}

fn timer_unit(
    tx: &Connection,
    anchor: &str,
    position: u64,
    high_water: u64,
) -> Result<(u64, bool), ApiError> {
    let seat: String = tx
        .query_row(
            "SELECT seat_id FROM seat_availability WHERE ordinal=?1",
            [anchor],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let row:Option<(i64,Option<String>)>=tx.query_row("SELECT pr.ordinal,sm.message_id FROM prepared_recipients pr LEFT JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE pr.seat_id=?1 AND pr.ordinal>?2 AND pr.ordinal<=?3 ORDER BY pr.ordinal LIMIT 1",params![seat,sql(position)?,sql(high_water)?],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
    if let Some((ordinal, message)) = row {
        if let Some(message) = message {
            project_receipt(tx, &message, &seat)?;
        }
        return Ok((
            u64::try_from(ordinal)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative receipt ordinal"))?,
            false,
        ));
    }
    Ok((high_water, true))
}

fn cleanup_unit(
    tx: &Connection,
    preparation: &str,
    position: u64,
) -> Result<(u64, bool), ApiError> {
    if let Some(preparation) = preparation.strip_prefix("service-notify:") {
        let safe: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM service_notification_preparations p WHERE p.id=?1 AND p.status='discarded' AND NOT EXISTS(SELECT 1 FROM service_notification_publications sp WHERE sp.preparation_id=p.id))",
            [preparation], |r| r.get(0)).map_err(store_error)?;
        if !safe {
            return Err(api_error(
                ErrorCode::Conflict,
                "notification cleanup requires unpublished discarded state",
            ));
        }
        let row: Option<i64> = tx.query_row(
            "SELECT ordinal FROM service_notification_recipients WHERE preparation_id=?1 ORDER BY ordinal LIMIT 1",
            [preparation], |r| r.get(0)).optional().map_err(store_error)?;
        if let Some(ordinal) = row {
            tx.execute(
                "DELETE FROM service_notification_recipients WHERE ordinal=?1",
                [ordinal],
            )
            .map_err(store_error)?;
            return Ok((position, false));
        }
        return Ok((position, true));
    }
    let safe:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM send_preparations p WHERE p.id=?1 AND p.status='discarded' AND NOT EXISTS(SELECT 1 FROM send_manifests sm WHERE sm.preparation_id=p.id))",[preparation],|r|r.get(0)).map_err(store_error)?;
    if !safe {
        return Err(api_error(
            ErrorCode::Conflict,
            "preparation cleanup requires unpublished discarded state",
        ));
    }
    let recipient:Option<String>=tx.query_row("SELECT seat_id FROM prepared_recipients WHERE preparation_id=?1 ORDER BY receipt_ordinal LIMIT 1",[preparation],|r|r.get(0)).optional().map_err(store_error)?;
    if let Some(seat) = recipient {
        tx.execute(
            "DELETE FROM prepared_recipients WHERE preparation_id=?1 AND seat_id=?2",
            params![preparation, seat],
        )
        .map_err(store_error)?;
        return Ok((position, false));
    }
    let warning:Option<String>=tx.query_row("SELECT warning_key FROM prepared_unavailable_warnings WHERE preparation_id=?1 ORDER BY warning_offset LIMIT 1",[preparation],|r|r.get(0)).optional().map_err(store_error)?;
    if let Some(key) = warning {
        tx.execute(
            "DELETE FROM prepared_unavailable_warnings WHERE preparation_id=?1 AND warning_key=?2",
            params![preparation, key],
        )
        .map_err(store_error)?;
        return Ok((position, false));
    }
    Ok((position, true))
}

#[cfg(test)]
#[path = "../../tests/store/materialization.rs"]
mod tests;
