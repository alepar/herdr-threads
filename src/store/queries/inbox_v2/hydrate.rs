use super::*;

pub(super) fn message(
    db: &Connection,
    id: &str,
    lazy: bool,
    current_agent: bool,
    state: &State,
) -> Result<Option<Item>, ApiError> {
    hydrate_message(
        db,
        id,
        lazy,
        current_agent,
        &state.seat,
        state.body.as_ref(),
    )
}
pub(in crate::store::queries) fn hydrate_message(
    db: &Connection,
    id: &str,
    lazy: bool,
    current_agent: bool,
    seat: &SeatId,
    position: Option<&crate::protocol::pagination::InboxBatchV2BodyPosition>,
) -> Result<Option<Item>, ApiError> {
    let row=db.query_row("SELECT thread_id,sequence,actor_seat_id,body,author_role,relays_user,user_intent,author_role_backfilled,priority FROM messages WHERE id=?1 AND kind='ordinary'",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)? as u64,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,bool>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,bool>(7)?,r.get::<_,i64>(8)?))).optional().map_err(store_error)?;
    let Some((
        thread,
        sequence,
        sender,
        body,
        role,
        relays_user,
        intent,
        author_role_backfilled,
        priority,
    )) = row
    else {
        return Ok(None);
    };
    let offset = position.map_or(0, |b| b.offset);
    let body_len = body.len() as u64;
    if position.is_some_and(|b| b.message.as_str() != id || b.body_len != body_len)
        || offset > body_len
        || !body.is_char_boundary(offset as usize)
    {
        return Err(invalid("inbox body changed or invalid boundary"));
    }
    let thread_id = ThreadId::new(&thread);
    let topic_data = batch_topic(db, &thread)?;
    let message = MessageId::new(id);
    let sender = sender.map(SeatId::new);
    let author_role = role
        .as_deref()
        .and_then(crate::protocol::summary::AuthorRole::from_column);
    let user_intent = intent
        .map(|v| {
            crate::protocol::summary::UserIntent::from_column(&v)
                .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid user intent"))
        })
        .transpose()?;
    let body = body[offset as usize..].to_owned();
    // Computed once, here, against the seat's current threshold (D1); an item staged before a
    // threshold change reads the new value without a re-send.
    let priority = crate::protocol::priority::DeliveryPriority::from_stored(priority)?;
    let wakes = lazy && crate::store::seats::nudge_threshold(db, seat)?.admits(priority);
    Ok(Some(if lazy {
        Item::LazyMessage {
            priority: Some(priority),
            thread: thread_id,
            topic_data,
            message,
            sequence,
            sender,
            author_role,
            relays_user,
            user_intent,
            author_role_backfilled,
            body,
            body_start: offset,
            body_end: body_len,
            body_len,
            wakes,
        }
    } else {
        Item::Message {
            priority: Some(priority),
            thread: thread_id,
            topic_data,
            message: message.clone(),
            sequence,
            sender,
            author_role,
            relays_user,
            user_intent,
            author_role_backfilled,
            body,
            body_start: offset,
            body_end: body_len,
            body_len,
            ack_candidate: current_agent.then_some(message),
        }
    }))
}
pub(super) fn chunk(item: &Item, end: usize) -> Option<(Item, InboxBatchV2BodyPosition)> {
    let mut chunk = item.clone();
    let position = match &mut chunk {
        Item::Message {
            message,
            body,
            body_start,
            body_end,
            body_len,
            ack_candidate,
            ..
        } => {
            if !body.is_char_boundary(end) || end == 0 {
                return None;
            }
            body.truncate(end);
            *body_end = *body_start + end as u64;
            *ack_candidate = None;
            Some(InboxBatchV2BodyPosition {
                message: message.clone(),
                offset: *body_end,
                body_len: *body_len,
            })
        }
        Item::LazyMessage {
            message,
            body,
            body_start,
            body_end,
            body_len,
            ..
        } => {
            if !body.is_char_boundary(end) || end == 0 {
                return None;
            }
            body.truncate(end);
            *body_end = *body_start + end as u64;
            Some(InboxBatchV2BodyPosition {
                message: message.clone(),
                offset: *body_end,
                body_len: *body_len,
            })
        }
        _ => None,
    };
    position.map(|p| (chunk, p))
}
