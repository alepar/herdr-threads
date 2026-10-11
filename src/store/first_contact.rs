//! First-contact history stats and resume fast-forward (spec §2 First-contact stats and
//! Fast-forward; ht-54r.8). Both are computed inside the deciding transaction from the message
//! table and the summary ledger, bounded to [`MAX_THREADS`] threads plus one count; no summary
//! text is included. The presentation (hook context, CLI inbox) renders the lines here.
use crate::{
    protocol::{
        ids::{BindingId, MessageId, SeatId, ThreadId},
        pagination::DEFAULT_PAGE_BYTES,
        results::{ApiError, ErrorCode, MessageKind, MessageSummary},
        summary::{AuthorRole, UserIntent},
        time::UtcMillis,
    },
    store::{
        connection::{api_error, store_error},
        lazy_delivery, read_cursors, summary,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

pub use crate::protocol::results::{FastForward, FastForwardThread, FirstContact, ThreadHistory};

/// Most threads one stats block lists; the rest are one `+N more threads` count.
pub const MAX_THREADS: usize = 8;
/// One ordinary inbox page: the existing default byte budget of an inbox read.
pub const ORDINARY_PAGE_BYTES: u64 = DEFAULT_PAGE_BYTES as u64;
/// What a page row costs beyond its body (ids, thread topic, markers, sender): the unread backlog
/// is measured as body bytes plus this per row, a bound on the page's own accounting.
pub const ROW_OVERHEAD_BYTES: u64 = 256;
/// What a check-in offer carries beyond its inbox page (decided in the check-in transaction).
#[derive(Debug, Clone, Default)]
pub struct OfferHistory {
    pub first_contact: Option<FirstContact>,
    pub fast_forward: Option<FastForward>,
}

/// Longest message preview carried for a fast-forward's last message.
const PREVIEW_CHARS: usize = 120;
/// Longest thread name shown in a stats line.
const NAME_CHARS: usize = 60;
/// Unread lazy rows read when measuring the backlog (more than a page holds already).
const LAZY_PROBE: usize = 512;

fn count(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative count"))
}

/// The joined threads of `seat` with their names, in a stable order.
fn joined_threads(
    db: &Connection,
    seat: &SeatId,
) -> Result<Vec<(ThreadId, Option<String>)>, ApiError> {
    let mut stmt = db
        .prepare(
            "SELECT m.thread_id,t.name FROM memberships m JOIN threads t ON t.id=m.thread_id \
             WHERE m.seat_id=?1 AND m.state='joined' ORDER BY m.thread_id",
        )
        .map_err(store_error)?;
    stmt.query_map([seat.as_str()], |r| {
        Ok((
            ThreadId::new(r.get::<_, String>(0)?),
            r.get::<_, Option<String>>(1)?,
        ))
    })
    .map_err(store_error)?
    .collect::<Result<_, _>>()
    .map_err(store_error)
}

/// Newest first, thread id as the tie-break, so the cut is deterministic.
fn most_recent_first(rows: &mut [ThreadHistory]) {
    rows.sort_by(|a, b| {
        b.last_at
            .0
            .cmp(&a.last_at.0)
            .then_with(|| a.thread.as_str().cmp(b.thread.as_str()))
    });
}

/// The stats for a binding whose cursors are `cursors`: one entry per joined thread of `seat`
/// with history before its cursor (`earlier > 0`), at most [`MAX_THREADS`], most recent activity
/// first, and the number of threads cut. Counts cover the messages up to the cursor.
pub fn first_contact(
    db: &Connection,
    seat: &SeatId,
    cursors: &dyn Fn(&ThreadId) -> u64,
) -> Result<FirstContact, ApiError> {
    let mut rows = Vec::new();
    for (thread, name) in joined_threads(db, seat)? {
        let cursor = cursors(&thread);
        if cursor == 0 {
            continue;
        }
        let (earlier, last_at): (i64, Option<i64>) = db
            .query_row(
                "SELECT COUNT(*),MAX(decision_at) FROM messages WHERE thread_id=?1 AND sequence<=?2",
                params![thread.as_str(), i64::try_from(cursor).unwrap_or(i64::MAX)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(store_error)?;
        if earlier == 0 {
            continue;
        }
        rows.push(ThreadHistory {
            thread,
            name,
            earlier: count(earlier)?,
            from_people: 0,
            open_requests: 0,
            last_at: UtcMillis(last_at.unwrap_or(0)),
        });
    }
    most_recent_first(&mut rows);
    let more = rows.len().saturating_sub(MAX_THREADS) as u64;
    rows.truncate(MAX_THREADS);
    for row in &mut rows {
        let cursor = cursors(&row.thread);
        row.from_people = summary::people_message_count(db, &row.thread, 0, cursor)?;
        row.open_requests = summary::open_request_count(db, &row.thread, cursor)?;
    }
    Ok(FirstContact {
        threads: rows,
        more,
    })
}

/// The binding's unread backlog as the inbox accounts it: body bytes plus a per-row overhead of
/// the ordinary mail past its cursors and of its unread lazy messages (a bound, not a page walk).
pub fn unread_backlog_bytes(db: &Connection, binding: &BindingId) -> Result<u64, ApiError> {
    let Some(seat) = read_cursors::seat_of(db, binding)? else {
        return Ok(0);
    };
    let (rows, bytes): (i64, i64) = db
        .query_row(
            &format!(
                "SELECT COUNT(*),COALESCE(SUM(LENGTH(m.body)),0) FROM {} WHERE {} {}",
                read_cursors::RECEIVABLE_FROM,
                read_cursors::RECEIVABLE_WHERE,
                read_cursors::UNREAD_EXTRA
            ),
            params![seat.as_str(), binding.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let mut total = count(bytes)?.saturating_add(count(rows)?.saturating_mul(ROW_OVERHEAD_BYTES));
    if total > ORDINARY_PAGE_BYTES {
        return Ok(total);
    }
    for lazy in lazy_delivery::binding_lazy_unread(db, binding, LAZY_PROBE)? {
        let length: Option<i64> = db
            .query_row(
                "SELECT LENGTH(body) FROM messages WHERE id=?1",
                [lazy.message.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?
            .flatten();
        total = total
            .saturating_add(count(length.unwrap_or(0))?)
            .saturating_add(ROW_OVERHEAD_BYTES);
    }
    Ok(total)
}

/// The last ordinary message of `thread` in `(after, up_to]`, as a preview-only summary.
fn last_message(
    db: &Connection,
    thread: &ThreadId,
    after: u64,
    up_to: u64,
) -> Result<Option<MessageSummary>, ApiError> {
    let row = db
        .query_row(
            "SELECT id,sequence,actor_seat_id,body,decision_at,author_role,relays_user,author_role_backfilled,user_intent \
             FROM messages WHERE thread_id=?1 AND sequence>?2 AND sequence<=?3 AND kind='ordinary' \
             ORDER BY sequence DESC LIMIT 1",
            params![
                thread.as_str(),
                i64::try_from(after).unwrap_or(i64::MAX),
                i64::try_from(up_to).unwrap_or(i64::MAX)
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((id, sequence, author, body, at, role, relays, backfilled, intent)) = row else {
        return Ok(None);
    };
    let body = body.unwrap_or_default();
    let preview: String = body.chars().take(PREVIEW_CHARS).collect();
    Ok(Some(MessageSummary {
        user_intent: intent.as_deref().and_then(UserIntent::from_column),
        message: MessageId::new(id),
        thread: thread.clone(),
        author: author.map(SeatId::new),
        event_author: None,
        author_role: role.as_deref().and_then(AuthorRole::from_column),
        relays_user: relays != 0,
        author_role_backfilled: backfilled != 0,
        kind: MessageKind::Ordinary,
        sequence: count(sequence)?,
        created_at: UtcMillis(at),
        actor_label: None,
        preview_omitted: preview.len() < body.len(),
        preview_data: preview,
        preview_detail_argv: None,
    }))
}

/// Fast-forward `binding` when its unread backlog exceeds one ordinary inbox page: every cursor
/// jumps to its thread's head, and the gap is reported per thread (at most [`MAX_THREADS`], most
/// recent first) with the last message of each. `None` and nothing moved when the backlog fits a
/// page. Writes no receipt and no seat state: only this binding's cursors move, so pending ACK
/// obligations, which the inbox lists independently of any cursor, stay listed, and a subagent
/// binding fast-forwards only itself.
pub fn fast_forward(db: &Connection, binding: &BindingId) -> Result<Option<FastForward>, ApiError> {
    let Some(seat) = read_cursors::seat_of(db, binding)? else {
        return Ok(None);
    };
    if unread_backlog_bytes(db, binding)? <= ORDINARY_PAGE_BYTES {
        return Ok(None);
    }
    let mut gaps = Vec::new();
    let mut skipped = 0u64;
    for (thread, name) in joined_threads(db, &seat)? {
        let cursor = read_cursors::cursor_for(db, binding, &thread)?.unwrap_or(0);
        let head = read_cursors::thread_head(db, &thread)?;
        if head <= cursor {
            continue;
        }
        let (gap, last_at): (i64, Option<i64>) = db
            .query_row(
                "SELECT COUNT(*),MAX(decision_at) FROM messages WHERE thread_id=?1 AND sequence>?2 AND sequence<=?3",
                params![
                    thread.as_str(),
                    i64::try_from(cursor).unwrap_or(i64::MAX),
                    i64::try_from(head).unwrap_or(i64::MAX)
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(store_error)?;
        skipped = skipped.saturating_add(count(gap)?);
        gaps.push((
            ThreadHistory {
                thread,
                name,
                earlier: count(gap)?,
                from_people: 0,
                open_requests: 0,
                last_at: UtcMillis(last_at.unwrap_or(0)),
            },
            cursor,
            head,
        ));
    }
    gaps.sort_by(|a, b| {
        b.0.last_at
            .0
            .cmp(&a.0.last_at.0)
            .then_with(|| a.0.thread.as_str().cmp(b.0.thread.as_str()))
    });
    let more = gaps.len().saturating_sub(MAX_THREADS) as u64;
    let mut threads = Vec::new();
    for (index, (mut history, cursor, head)) in gaps.into_iter().enumerate() {
        if index < MAX_THREADS {
            history.from_people = summary::people_message_count(db, &history.thread, cursor, head)?;
            history.open_requests = summary::open_request_count(db, &history.thread, head)?;
            let last = last_message(db, &history.thread, cursor, head)?;
            threads.push(FastForwardThread {
                history: history.clone(),
                last_message: last,
            });
        }
        read_cursors::advance(db, binding, &history.thread, head)?;
    }
    Ok(Some(FastForward {
        skipped,
        threads,
        more,
    }))
}

/// One display line of text that came from a peer (a thread name, a message preview): control
/// characters and runs of whitespace become one space, cut to `chars` characters.
fn one_line(text: &str, chars: usize) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for c in text.chars() {
        if c.is_control() || c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
    }
    let trimmed = out.trim_end();
    let mut cut: String = trimmed.chars().take(chars).collect();
    if trimmed.chars().count() > chars {
        cut.push('…');
    }
    cut
}

fn label(history: &ThreadHistory) -> String {
    let id = history.thread.as_str();
    match history.name.as_deref().map(|n| one_line(n, NAME_CHARS)) {
        Some(name) if !name.is_empty() => format!("#{name} ({id})"),
        _ => format!("#{id} ({id})"),
    }
}

fn history_line(history: &ThreadHistory, now: UtcMillis) -> String {
    let minutes = now.0.saturating_sub(history.last_at.0).max(0) / 60_000;
    format!(
        "{}: {} earlier messages, {} from people ({} open requests), last {}m ago — herdr-threads summary {}",
        label(history),
        history.earlier,
        history.from_people,
        history.open_requests,
        minutes,
        history.thread.as_str()
    )
}

fn more_line(more: u64) -> Option<String> {
    (more > 0).then(|| format!("+{more} more threads (herdr-threads thread list --joined)"))
}

/// The spec's lines: one per thread, then the `+N more threads` line when threads were cut.
pub fn render_first_contact(fc: &FirstContact, now: UtcMillis) -> Vec<String> {
    let mut lines: Vec<String> = fc
        .threads
        .iter()
        .map(|history| history_line(history, now))
        .collect();
    lines.extend(more_line(fc.more));
    lines
}

/// The fast-forward lines: `resumed past <N> messages:`, then per thread the stats line in the
/// first-contact shape (counts over the gap) and the thread's last message, then the cut count.
pub fn render_fast_forward(ff: &FastForward, now: UtcMillis) -> Vec<String> {
    let mut lines = vec![format!("resumed past {} messages:", ff.skipped)];
    for thread in &ff.threads {
        lines.push(history_line(&thread.history, now));
        if let Some(last) = &thread.last_message {
            let who = last
                .author
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), |seat| seat.as_str().to_owned());
            lines.push(format!(
                "  last: #{} {who}{}: {}",
                last.sequence,
                last.author_markers(),
                one_line(&last.preview_data, PREVIEW_CHARS)
            ));
        }
    }
    lines.extend(more_line(ff.more));
    lines
}

#[cfg(test)]
#[path = "../../tests/store/first_contact.rs"]
mod tests;
