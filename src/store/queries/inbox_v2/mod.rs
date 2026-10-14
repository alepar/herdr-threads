// Separate passive traversal: a min-head merge over the ordering classes (`order::OrderClass`), each
// walked by its own persisted position; completion never enters v1 attention.
mod capture;
mod deadline_window;
mod hydrate;
mod order;
mod sources;

use super::*;
use crate::protocol::{
    authority::CallerClaim,
    ids::BindingId,
    pagination::{
        DeadlineCursor, DeadlineHighWaters, DeadlineWalks, DeliveryRowsCursor,
        InboxBatchV2BodyPosition, InboxBatchV2CursorState as State, InboxBatchV2Source as Source,
        WarningsCursor,
    },
    results::InboxBatchV2Item as Item,
};

#[cfg(test)]
pub(super) use self::capture::{binding, fence_holds};
use self::hydrate::chunk;
use self::order::{Merge, OrderClass, PageSubset, row_filter, row_key};
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
fn done(state: &State) -> bool {
    OrderClass::ALL.into_iter().all(|class| class.done(state)) && state.body.is_none()
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
    let subset = match (ordinary_only, q.thread.as_ref()) {
        (true, _) => PageSubset::Presentation,
        (false, Some(thread)) => PageSubset::ThreadScoped(thread),
        (false, None) => PageSubset::All,
    };
    let mut state = match q.page.cursor.as_ref() {
        Some(raw) => State::decode_for(raw, instance, seat).map_err(invalid)?,
        None => capture(db, instance, seat, &reader, &subset)?,
    };
    subset.apply(&mut state, &reader);
    let fence = match reading {
        Some(_) => {
            identity(db, seat, &reader)? == state.binding_generation.zip(state.execution.clone())
        }
        None => fence_holds(db, seat, state.binding_generation, state.execution.as_ref())?,
    };
    if !fence || state.deadline.high.unread.is_some() != reading.is_some() {
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
    // Progress is compared to the original capture, before retirement.
    let initial = state.clone();
    let dh = state.publication_decision_high_water;
    let mut retired_body = None;
    let mut continuation_probes = 0;
    if let Some((source, body)) = state.body.clone() {
        let lazy = source == Source::Lazy;
        if hydrate_message(
            db,
            body.message.as_str(),
            lazy,
            current_agent,
            seat,
            Some(&body),
        )?
        .is_none()
        {
            return Err(invalid("continued body is absent"));
        }
        let addressed = if ordinary_only && source == Source::Unread {
            let validated = continued_ordinary(
                db,
                instance,
                &state,
                reading.expect("bound private reader"),
                &mut continuation_probes,
            )?;
            if validated.retire {
                // The settled body is no longer unread or pending, so the re-collected window
                // does not list it again.
                state.body = None;
                retired_body = Some(validated.message);
            }
            true
        } else if source == Source::Unread {
            reading.is_some_and(|binding| {
                unread_ordinal(db, seat, binding, body.message.as_str(), dh)
                    .ok()
                    .flatten()
                    .zip(
                        state
                            .deadline
                            .window_start
                            .unread_after
                            .zip(state.deadline.high.unread),
                    )
                    .is_some_and(|(ordinal, (after, high))| {
                        ordinal as u64 > after && ordinal as u64 <= high
                    })
            })
        } else if lazy {
            lazy_body_position(db, instance, seat, body.message.as_str(), dh)?
                .is_some_and(|position| state.rows.after.is_none_or(|after| position > after))
        } else {
            crate::store::effective::effective_receipt(db, body.message.as_str(), seat.as_str())?
                .is_some_and(|r| r.decision_seq.unwrap_or(0) <= dh as i64)
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
    let filter = row_filter(db, &subset, &reader, seat)?;
    // A retired body consumes one of the existing candidate visits.
    let mut merge = Merge::new(
        db,
        instance,
        seat,
        reading,
        current_agent,
        filter,
        &subset,
        usize::from(retired_body.is_some()),
    );
    let stop;
    loop {
        if budget.is_exhausted(store.clock()) {
            return Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "inbox v2 budget exhausted",
            ));
        }
        // A partly shown body resumes first; otherwise the smallest head of the unfinished
        // classes is next.
        let resumed = state.body.clone();
        let before;
        let (class, emitted) = match &resumed {
            None => {
                if !merge.ensure(&mut state)? {
                    stop = StopReason::Work;
                    break;
                }
                let Some((class, key)) = merge.pick() else {
                    stop = StopReason::Complete;
                    break;
                };
                if items.len() >= q.page.limit as usize {
                    stop = StopReason::Rows;
                    break;
                }
                before = state.clone();
                let emitted = merge.item(class, &state, None)?;
                merge.advance(class, &mut state, key);
                state.last = Some(key);
                (class, emitted)
            }
            Some((source, body)) => {
                if items.len() >= q.page.limit as usize {
                    stop = StopReason::Rows;
                    break;
                }
                if merge.visited >= CANDIDATE_LIMIT {
                    stop = StopReason::Work;
                    break;
                }
                match OrderClass::of(*source) {
                    OrderClass::DeliveryRows => {
                        // The one physical visit of the resumed row.
                        merge.visited += 1;
                        let position =
                            lazy_body_position(db, instance, seat, body.message.as_str(), dh)?
                                .ok_or_else(|| {
                                    invalid("continued body is not captured addressed content")
                                })?;
                        // Recheck only scope. A captured tail must survive its own cursor
                        // progress, but a cursorless reader cannot borrow a later invitation
                        // episode for an older publication.
                        if let Some(binding) = reading {
                            if ordinary_only {
                                merge.lazy_scope_probes += 3;
                            }
                            let visible: bool = db
                                .query_row(
                                    super::LAZY_CONTINUATION_SCOPE_SQL,
                                    params![
                                        seat.as_str(),
                                        body.message.as_str(),
                                        binding.as_str(),
                                        instance
                                    ],
                                    |r| r.get(0),
                                )
                                .map_err(store_error)?;
                            if !visible {
                                let legacy_pending: i64 = db
                                    .query_row(
                                        "SELECT legacy_pending FROM lazy_recipients WHERE seat_id=?1 AND message_id=?2",
                                        params![seat.as_str(), body.message.as_str()],
                                        |r| r.get(0),
                                    )
                                    .map_err(store_error)?;
                                // Positive captured publication/address and accumulator
                                // validation preceded this. Only modern private tails may
                                // retire on scope loss; public and legacy continuations
                                // remain strict.
                                if ordinary_only && legacy_pending == 0 {
                                    state.rows.after = Some(position);
                                    state.body = None;
                                    retired_body = Some(body.message.clone());
                                    continue;
                                }
                                return Err(invalid(
                                    "continued body is not captured unread content",
                                ));
                            }
                        }
                        before = state.clone();
                        let key = row_key(position);
                        let emitted = merge.item(OrderClass::DeliveryRows, &state, Some(body))?;
                        merge.advance(OrderClass::DeliveryRows, &mut state, key);
                        state.last = Some(key);
                        (OrderClass::DeliveryRows, emitted)
                    }
                    OrderClass::Deadline => {
                        // The body is emitted straight from its message, whether or not the walks
                        // still list it; the window only drops its item so it is not shown twice.
                        if !merge.ensure_class(OrderClass::Deadline, &mut state)? {
                            stop = StopReason::Work;
                            break;
                        }
                        before = state.clone();
                        let ack = merge.body_ack(&body.message, dh)?;
                        let emitted = merge
                            .body_item(&body.message, ack, body)?
                            .map(|item| (item, *source));
                        if let Some(key) = merge.settle_body(&mut state, &body.message) {
                            state.last = Some(key);
                        }
                        (OrderClass::Deadline, emitted)
                    }
                    OrderClass::Warnings => {
                        return Err(invalid("continued body is not captured addressed content"));
                    }
                }
            }
        };
        state.body = None;
        let Some((mut item, source)) = emitted else {
            continue;
        };
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
                next.body = Some((source, position));
                merge.pin_partial(class, &mut next);
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
    let visited = merge.visited;
    let lazy_scope_probes = merge.lazy_scope_probes;
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
