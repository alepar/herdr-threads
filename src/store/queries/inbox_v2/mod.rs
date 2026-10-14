// Separate passive traversal: source positions and completion never enter v1
// attention. Start with Lazy, then rotate after each inspected candidate.
mod capture;
// Wired by ht-yha.5.4 (inbox v2 merge).
#[cfg_attr(not(test), allow(dead_code))]
mod deadline_window;
mod hydrate;
mod sources;

use super::*;
use crate::protocol::{
    authority::CallerClaim,
    ids::BindingId,
    pagination::{
        InboxBatchV2BodyPosition, InboxBatchV2CursorState as State, InboxBatchV2Source as Source,
        InboxBatchV2UnreadPosition,
    },
    results::InboxBatchV2Item as Item,
};

#[cfg(test)]
pub(super) use self::capture::{binding, fence_holds};
use self::hydrate::{chunk, message};
use self::{capture::*, sources::*};
pub(super) use self::{hydrate::hydrate_message, sources::unread_ordinal};

fn invalid(detail: &str) -> ApiError {
    api_error(ErrorCode::InvalidCursor, detail)
}
pub(super) struct QueryModePage {
    pub result: CommandResult,
    pub visited: usize,
    pub retired_body: Option<MessageId>,
    pub continuation_probes: usize,
    pub lazy_scope_probes: usize,
}
fn source_done(state: &State, source: Source) -> bool {
    match source {
        Source::Lazy => state.lazy_after_ordinal >= state.lazy_high_water_ordinal,
        Source::Invitations => state.attention.invitations_done,
        Source::Receipts => state.attention.receipts_done,
        Source::Warnings => state.attention.next_manifest_warning,
        Source::Unread => state
            .unread
            .is_none_or(|u| u.after_ordinal >= u.high_water_ordinal),
    }
}
fn done(state: &State) -> bool {
    [
        Source::Lazy,
        Source::Unread,
        Source::Invitations,
        Source::Receipts,
        Source::Warnings,
    ]
    .into_iter()
    .all(|s| source_done(state, s))
}
fn rotate(state: &mut State) {
    state.source = match state.source {
        Source::Lazy if state.unread.is_some() => Source::Unread,
        Source::Lazy | Source::Unread => Source::Invitations,
        Source::Invitations => Source::Receipts,
        Source::Receipts => Source::Warnings,
        Source::Warnings => Source::Lazy,
    };
}
fn page_v2(
    items: Vec<Item>,
    state: Option<&State>,
    instance: &str,
    request: &PageRequest,
    output: &OutputSpec,
    reason: StopReason,
) -> Result<Page<Item>, ApiError> {
    let next = state
        .map(|s| {
            let raw = s.encode(instance).map_err(invalid)?;
            Ok((raw.clone(), inbox_batch_argv(&s.seat, &raw, request)))
        })
        .transpose()?;
    Ok(page(
        items,
        next,
        state.map_or(0, |s| s.publication_decision_high_water),
        reason,
        output,
    ))
}
fn fits(
    items: &[Item],
    state: &State,
    instance: &str,
    request: &PageRequest,
    output: &OutputSpec,
) -> Result<bool, ApiError> {
    Ok(encode_selected(
        &CommandResult::InboxBatchV2(page_v2(
            items.to_vec(),
            Some(state),
            instance,
            request,
            output,
            StopReason::Bytes,
        )?),
        output,
    )?
    .len()
        <= request.max_bytes as usize)
}
pub(super) fn query(
    db: &Connection,
    store: &StoreContext,
    instance: &str,
    q: &InboxQuery,
    output: &OutputSpec,
    budget: &CallBudget,
) -> Result<CommandResult, ApiError> {
    query_mode(db, store, instance, q, output, budget, false).map(|page| page.result)
}

pub(super) fn current_agent(
    db: &Connection,
    seat: &SeatId,
    reading: Option<&BindingId>,
) -> Result<bool, ApiError> {
    let selector = if reading.is_some() {
        "b.binding_id=?2"
    } else {
        "?2 IS NULL ORDER BY b.ordinal DESC LIMIT 1"
    };
    let current_harness:Option<String>=db.query_row(&format!("SELECT b.harness FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.id=?1 AND s.state='resolved' AND b.ended_at IS NULL AND b.registered_at IS NOT NULL AND b.role='top_level' AND b.liveness='connected' AND b.target_id=s.target_id AND b.target_generation=s.target_generation AND b.native_session<>'' AND b.execution_id<>'' AND {selector}"),params![seat.as_str(),reading.map(|b| b.as_str())],|r|r.get(0)).optional().map_err(store_error)?;
    Ok(current_harness
        .as_deref()
        .is_some_and(|harness| crate::harness::registry::builtins().agent(harness).is_ok()))
}
pub(super) fn query_mode(
    db: &Connection,
    store: &StoreContext,
    instance: &str,
    q: &InboxQuery,
    output: &OutputSpec,
    budget: &CallBudget,
    ordinary_only: bool,
) -> Result<QueryModePage, ApiError> {
    let seat = q.seat.as_ref().ok_or_else(|| {
        api_error(
            ErrorCode::InvalidRequest,
            "inbox batch requires a resolved seat",
        )
    })?;
    // A thread-scoped page (ht-br0) is a first page only: a continuation cursor does not
    // carry the scope, so a scoped request never resumes one.
    if q.thread.is_some() && q.page.cursor.is_some() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "a thread-scoped inbox page has no continuation",
        ));
    }
    let owned:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2 AND state!='retired')",params![seat.as_str(),instance],|r|r.get(0)).map_err(store_error)?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "seat not found"));
    }
    let reader = reader(db, seat, q.claim.as_ref())?;
    let reading = bound(&reader);
    if ordinary_only && reading.is_none() {
        return Err(invalid("presentation reader has no binding"));
    }
    // The private ordinary presentation has its own lazy eligibility probe and never reads a
    // thread-scoped page (ht-br0 is a public text-inbox read only).
    if ordinary_only && q.thread.is_some() {
        return Err(invalid("presentation page has no thread scope"));
    }
    let mut state = match q.page.cursor.as_ref() {
        Some(raw) => State::decode_for(raw, instance, seat).map_err(invalid)?,
        None => capture(db, instance, seat, &reader, ordinary_only)?,
    };
    if ordinary_only {
        state.attention.invitations_done = true;
        state.attention.invitation_frontier = Some((0, 0));
        // Warnings have their own presentation source. Normalize older or
        // public cursors as well as fresh captures before source rotation.
        state.attention.next_manifest_warning = true;
    }
    let fence = match reading {
        Some(_) => {
            identity(db, seat, &reader)? == state.binding_generation.zip(state.execution.clone())
        }
        None => fence_holds(db, seat, state.binding_generation, state.execution.as_ref())?,
    };
    if !fence || state.unread.is_some() != reading.is_some() {
        return Err(invalid("inbox binding changed"));
    }
    let decision: i64 = db
        .query_row(
            "SELECT decision_seq FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if state.publication_decision_high_water > decision as u64 {
        return Err(invalid("inbox snapshot is in the future"));
    }
    // Receipt eligibility is canonical registered-agent eligibility (as
    // in inbox v1, with its exact current-binding predicate, and display-ACK
    // settlement), never a fixed harness list:
    // a Hermes binding's pending receipts are ACK candidates too.
    let current_agent = current_agent(db, seat, reading)?;
    // Thread scope (ht-br0): only the reader's unread lazy rows of that thread, under the
    // unchanged per-binding (or seat) lazy eligibility. Every other source starts done, so the
    // page shows (and a display settles) nothing else: no ordinary, receipt, invitation or
    // warning rows and no first-contact stats. A scoped page is a first page, so there is no
    // continued body to validate here.
    if q.thread.is_some() {
        state.attention.invitations_done = true;
        state.attention.receipts_done = true;
        state.attention.next_manifest_warning = true;
        if let Some(unread) = &mut state.unread {
            unread.after_ordinal = unread.high_water_ordinal;
        }
        // A claim that names no binding of the seat would complete in the pre-binding
        // seat-wide form, whose cursor walks this page cannot bound: it shows no row.
        if matches!(reader, Reader::Unknown) {
            state.lazy_after_ordinal = state.lazy_high_water_ordinal;
        }
    }
    // Progress is compared to the original capture, before retirement.
    let initial = state.clone();
    let mut continuation = None;
    let mut retired_body = None;
    let mut continuation_probes = 0;
    let mut lazy_scope_probes = 0;
    if let Some(body) = state.body.clone() {
        let lazy = state.source == Source::Lazy;
        if message(db, body.message.as_str(), lazy, current_agent, &state)?.is_none() {
            return Err(invalid("continued body is absent"));
        }
        let addressed = if ordinary_only && state.source == Source::Unread {
            let validated = continued_ordinary(
                db,
                instance,
                &state,
                reading.expect("bound private reader"),
                &mut continuation_probes,
            )?;
            if validated.retire {
                state
                    .unread
                    .as_mut()
                    .expect("captured unread position")
                    .after_ordinal = validated.ordinal as u64;
                state.body = None;
                retired_body = Some(validated.message);
            } else {
                continuation = Some(validated);
            }
            true
        } else if state.source == Source::Unread {
            reading.is_some_and(|binding| {
                unread_ordinal(
                    db,
                    seat,
                    binding,
                    body.message.as_str(),
                    state.publication_decision_high_water,
                )
                .ok()
                .flatten()
                .zip(state.unread)
                .is_some_and(|(ordinal, u)| {
                    ordinal as u64 > u.after_ordinal && ordinal as u64 <= u.high_water_ordinal
                })
            })
        } else if lazy {
            db.query_row("SELECT EXISTS(SELECT 1 FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id AND sm.thread_id=r.thread_id JOIN messages m ON m.id=r.message_id AND m.instance_id=sm.instance_id AND m.thread_id=r.thread_id AND m.decision_seq=sm.decision_seq WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3 AND sm.decision_seq<=?4 AND r.ordinal>?5 AND r.ordinal<=?6 AND m.kind='ordinary' AND m.delivery_mode='lazy')",params![seat.as_str(),body.message.as_str(),instance,state.publication_decision_high_water as i64,state.lazy_after_ordinal as i64,state.lazy_high_water_ordinal as i64],|r|r.get::<_,bool>(0)).map_err(store_error)?
        } else {
            crate::store::effective::effective_receipt(db, body.message.as_str(), seat.as_str())?
                .is_some_and(|r| {
                    r.decision_seq.unwrap_or(0) <= state.publication_decision_high_water as i64
                })
        };
        if !addressed {
            return Err(invalid("continued body is not captured addressed content"));
        }
    }
    let mut items = Vec::new();
    // A session the daemon has not enrolled yet reads the page a fresh binding would get,
    // with the history stats it would start from (spec §2 First-contact stats). Only the
    // first page, and only when it leaves room for mail; the CLI's advance right after the
    // display enrolls the binding, so it is shown once.
    if matches!(reader, Reader::Unknown)
        && q.page.cursor.is_none()
        && q.thread.is_none()
        && q.page.limit >= 2
    {
        let first_contact = crate::store::first_contact::first_contact(db, seat, &|thread| {
            crate::store::read_cursors::seed_preview(db, seat, thread).unwrap_or(0)
        })?;
        if !first_contact.threads.is_empty() {
            let item = Item::FirstContact(first_contact);
            let alone = page_v2(
                vec![item.clone()],
                Some(&state),
                instance,
                &q.page,
                output,
                StopReason::Bytes,
            )?;
            if encode_selected(&CommandResult::InboxBatchV2(alone), output)?.len()
                <= q.page.max_bytes as usize / 2
            {
                items.push(item);
            }
        }
    }
    // A retired body consumes one of the existing candidate visits.
    let mut visited = usize::from(retired_body.is_some());
    let mut stop = StopReason::Complete;
    while !done(&state) {
        if budget.is_exhausted(store.clock()) {
            return Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "inbox v2 budget exhausted",
            ));
        }
        if items.len() >= q.page.limit as usize {
            stop = StopReason::Rows;
            break;
        }
        if visited >= CANDIDATE_LIMIT
            || (!ordinary_only && state.source == Source::Warnings && CANDIDATE_LIMIT - visited < 2)
        {
            stop = StopReason::Work;
            break;
        }
        if source_done(&state, state.source) {
            rotate(&mut state);
            continue;
        }
        let before = state.clone();
        let (item, work) = scan(
            db,
            instance,
            &mut state,
            current_agent,
            reading,
            ordinary_only,
            continuation.as_ref(),
            &mut lazy_scope_probes,
            &mut retired_body,
            q.thread.as_ref(),
        )?;
        visited += work;
        state.body = None;
        rotate(&mut state);
        if let Some(mut item) = item {
            if matches!(
                &item,
                Item::Invitation {
                    goal_data: Some(_),
                    ..
                }
            ) && !fits(
                std::slice::from_ref(&item),
                &state,
                instance,
                &q.page,
                output,
            )? && let Item::Invitation { goal_data, .. } = &mut item
            {
                *goal_data = None;
            }
            let mut trial = items.clone();
            trial.push(item.clone());
            if fits(&trial, &state, instance, &q.page, output)? {
                items.push(item);
                continue;
            }
            let len = match &item {
                Item::Message { body, .. } | Item::LazyMessage { body, .. } => Some(body.len()),
                _ => None,
            };
            let mut chosen = None;
            if let Some(len) = len {
                let (mut lo, mut hi) = (1, len);
                while lo <= hi {
                    let mid = lo + (hi - lo) / 2;
                    let end = match &item {
                        Item::Message { body, .. } | Item::LazyMessage { body, .. } => (mid..=len)
                            .find(|&n| body.is_char_boundary(n))
                            .unwrap_or(len),
                        _ => unreachable!(),
                    };
                    let (chunk, position) = chunk(&item, end).expect("positive UTF8 chunk");
                    let mut next = before.clone();
                    next.body = Some(position);
                    let mut trial = items.clone();
                    trial.push(chunk.clone());
                    if fits(&trial, &next, instance, &q.page, output)? {
                        chosen = Some((chunk, next));
                        lo = end.saturating_add(1)
                    } else {
                        hi = mid - 1
                    }
                }
            }
            if let Some((chunk, next)) = chosen {
                items.push(chunk);
                state = next;
                stop = StopReason::Bytes;
                break;
            }
            state = before;
            if items.is_empty() {
                return Err(ApiError::invalid_budget(
                    "inbox v2 item or body chunk cannot fit",
                ));
            }
            stop = StopReason::Bytes;
            break;
        }
    }
    if !done(&state) && state == initial {
        return Err(ApiError::invalid_budget(
            "inbox v2 continuation cannot advance",
        ));
    }
    let next = (!done(&state)).then_some(&state);
    let page = page_v2(
        items,
        next,
        instance,
        &q.page,
        output,
        if next.is_none() {
            StopReason::Complete
        } else {
            stop
        },
    )?;
    if encode_selected(&CommandResult::InboxBatchV2(page.clone()), output)?.len()
        > q.page.max_bytes as usize
    {
        return Err(ApiError::invalid_budget("selected response cannot fit"));
    }
    if continuation_probes > 15
        || lazy_scope_probes > 300
        || lazy_scope_probes > visited.saturating_mul(3)
        || (ordinary_only && visited > CANDIDATE_LIMIT)
    {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "private continuation work bound",
        ));
    }
    Ok(QueryModePage {
        result: CommandResult::InboxBatchV2(page),
        visited,
        retired_body,
        continuation_probes,
        lazy_scope_probes,
    })
}
