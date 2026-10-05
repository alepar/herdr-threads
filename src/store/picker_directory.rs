//! Read-only compact directory for the human picker. Message work is bounded
//! by sequence, independent of history size. Exact membership counts necessarily
//! visit the thread's membership/requirement rows under the query budget.
use super::*;
use crate::protocol::{
    commands::PickerDirectoryQuery,
    results::{PickerMessagePreview, PickerPage, PickerThread},
};

pub(super) fn directory(
    db: &QueryConnection,
    instance: &str,
    query: &PickerDirectoryQuery,
    now: UtcMillis,
    output: &OutputSpec,
) -> Result<CommandResult, ApiError> {
    let filter = digest(&"picker.directory_v1")?;
    let cursor = decode_cursor(
        &query.page,
        instance,
        CursorScope::PickerDirectory,
        "*",
        &filter,
        CursorDirection::Ascending,
    )?;
    let high = cursor.as_ref().map_or_else(
        || {
            db.query_row(
                "SELECT coalesce(max(ordinal),0) FROM threads WHERE instance_id=?1",
                [instance],
                |r| r.get::<_, i64>(0).map(|n| n as u64),
            )
            .map_err(|e| db.map_error(e))
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut after = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let continuation = |ordinal| {
        cursor_for(
            instance,
            CursorScope::PickerDirectory,
            "*",
            &filter,
            CursorDirection::Ascending,
            ordinal,
            high,
        )
        .encode()
        .map_err(|why| api_error(ErrorCode::InvalidCursor, why))
    };
    let mut candidates = Vec::new();
    // No inventory-wide sort or summary/history scans. At most the requested
    // rows (100 maximum) are enriched, plus one indexed continuation probe.
    let mut stmt=db.prepare("SELECT ordinal,id,name,substr(topic,1,1024),archived,created_at,next_sequence FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4").map_err(|e|db.map_error(e))?;
    let rows = stmt
        .query_map(
            params![instance, after as i64, high as i64, query.page.limit],
            |r| {
                Ok((
                    r.get::<_, i64>(0).map(|n| n as u64)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|e| db.map_error(e))?;
    for row in rows {
        db.check_budget()?;
        let (ordinal, id, name, topic_data, archived, created_at, next_sequence) =
            row.map_err(|e| db.map_error(e))?;
        let total = next_sequence
            .checked_sub(1)
            .filter(|n| *n >= 0)
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid picker timeline"))?;
        let sample_positions = total.min(512) as u16;
        let first = total - i64::from(sample_positions) + 1;
        let ordinary:i64=db.query_row("SELECT count(*) FROM messages INDEXED BY messages_thread_sequence WHERE thread_id=?1 AND sequence>=?2 AND sequence<=?3 AND kind='ordinary'",params![id,first,total],|r|r.get(0)).map_err(|e|db.map_error(e))?;
        // UNION counts a voluntary member with an accepted requirement once.
        // The voluntary marker is authoritative over compatibility raw state.
        let participant_count:i64=db.query_row("SELECT count(*) FROM (
            SELECT m.seat_id FROM memberships m JOIN seats s ON s.id=m.seat_id
            WHERE m.thread_id=?1 AND coalesce(m.voluntary_state,m.state)='joined' AND s.state!='retired'
            UNION
            SELECT r.seat_id FROM requirement_episodes r INDEXED BY requirement_episodes_effective JOIN seats s ON s.id=r.seat_id
            WHERE r.thread_id=?1 AND r.state IN ('pending','accepted') AND r.state='accepted' AND s.state!='retired'
              AND NOT EXISTS (SELECT 1 FROM memberships m WHERE m.thread_id=r.thread_id AND m.seat_id=r.seat_id AND coalesce(m.voluntary_state,m.state)='retired')
        )",[&id],|r|r.get(0)).map_err(|e|db.map_error(e))?;
        let (last_message, oldest) = if total == 0 {
            (None, created_at)
        } else {
            let latest = endpoint(db, &id, total, true)?;
            let oldest = if first == total {
                latest.at.0
            } else {
                endpoint(db, &id, first, false)?.at.0
            };
            (Some(latest), oldest)
        };
        let last_activity = last_message
            .as_ref()
            .map_or(UtcMillis(created_at), |m| m.at);
        // Signed clock reversal and future timestamps keep the one-minute
        // floor; wide subtraction preserves large valid elapsed intervals.
        let activity_window_ms = (i128::from(now.0) - i128::from(oldest))
            .max(60_000)
            .min(i128::from(u64::MAX)) as u64;
        let item = PickerThread {
            thread: ThreadId::new(id),
            name,
            topic_data,
            archived,
            participant_count: participant_count as u64,
            last_activity,
            recent_ordinary_count: ordinary as u64,
            activity_window_ms,
            sample_positions,
            last_message,
        };
        let raw = continuation(ordinal)?;
        candidates.push(Cand {
            item,
            raw,
            argv: Vec::new(),
            before: after,
        });
        after = ordinal;
    }
    let more:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM threads WHERE instance_id=?1 AND ordinal>?2 AND ordinal<=?3)",params![instance,after as i64,high as i64],|r|r.get(0)).map_err(|e|db.map_error(e))?;
    let count = candidates.len();
    let initial_after = candidates.first().map_or(after, |c| c.before);
    // Fit the exact eventual envelope. A byte cut carries `Bytes` (one JSON
    // byte longer than `Rows`), while a complete scan carries no cursor at all.
    // The shared discovery helper always estimates a `Rows` continuation and
    // therefore cannot provide a maximal prefix at these exact boundaries.
    let render_items = |items: Vec<PickerThread>, accepted: usize| {
        let stop_reason = if accepted < count {
            StopReason::Bytes
        } else if more {
            StopReason::Rows
        } else {
            StopReason::Complete
        };
        let has_more = stop_reason != StopReason::Complete;
        let next_cursor = if !has_more {
            None
        } else if accepted == 0 {
            Some(continuation(initial_after)?)
        } else {
            Some(candidates[accepted - 1].raw.clone())
        };
        Ok(CommandResult::PickerDirectory(PickerPage {
            items,
            next_cursor,
            high_water_ordinal: high,
            scope_revision: None,
            has_more,
            stop_reason,
            consistency: Consistency::BoundedLive,
        }))
    };
    let render_prefix = |accepted| {
        render_items(
            candidates[..accepted]
                .iter()
                .map(|c| c.item.clone())
                .collect(),
            accepted,
        )
    };
    let fit = PageFit::for_command(output, query.page.max_bytes as usize).fit_with(
        &candidates,
        render_prefix,
        |i| render_items(vec![candidates[i].item.clone()], i + 1),
    )?;
    if fit.accepted == 0 && count > 0 {
        let mut error = ApiError::invalid_budget("picker row cannot fit");
        error.required_minimum_bytes = fit.required_minimum;
        return Err(error);
    }
    let result = render_prefix(fit.accepted)?;
    let bytes = encode_selected(&result, output)?.len();
    if bytes > query.page.max_bytes as usize {
        return Err(ApiError::invalid_budget("picker response cannot fit")
            .with_required_minimum_bytes(bytes.min(u32::MAX as usize) as u32));
    }
    Ok(result)
}

/// Read one canonical position. The oldest sample endpoint needs only its
/// timestamp; the latest preview is sliced in SQLite before crossing into Rust.
fn endpoint(
    db: &QueryConnection,
    thread: &str,
    sequence: i64,
    preview: bool,
) -> Result<PickerMessagePreview, ApiError> {
    let physical: Option<(String, i64, String)> = db.query_row(
        "SELECT kind,decision_at,CASE WHEN ?3 THEN substr(coalesce(body,event_json,''),1,257) ELSE '' END
         FROM messages WHERE thread_id=?1 AND sequence=?2",
        params![thread,sequence,preview],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(|e|db.map_error(e))?;
    let (kind, at, source) = if let Some(physical) = physical {
        physical
    } else {
        // Match effective::timeline_entry: only the predecessor publication
        // manifest can own this exact reserved warning offset. Projection
        // cannot alter the manifest's timestamp or the warning's contents.
        let manifest: Option<(String, i64, i64, i64)> = db
            .query_row(
                "SELECT preparation_id,base_sequence,warning_count,decision_at
             FROM send_manifests WHERE thread_id=?1 AND base_sequence<?2
             ORDER BY base_sequence DESC LIMIT 1",
                params![thread, sequence],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| db.map_error(e))?;
        let Some((preparation, base, count, at)) = manifest else {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "unbacked picker timeline position",
            ));
        };
        let offset = sequence - base;
        if offset <= 0 || offset > count {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "unbacked picker warning position",
            ));
        }
        let source: Option<String> = db
            .query_row(
                "SELECT CASE WHEN ?3 THEN substr(event_json,1,257) ELSE '' END
             FROM prepared_unavailable_warnings WHERE preparation_id=?1 AND warning_offset=?2",
                params![preparation, offset, preview],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db.map_error(e))?;
        (
            "warn".into(),
            at,
            source.ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "missing picker warning"))?,
        )
    };
    let kind = match kind.as_str() {
        "ordinary" => MessageKind::Ordinary,
        "info" => MessageKind::Info,
        "warn" => MessageKind::Warn,
        _ => {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid picker message kind",
            ));
        }
    };
    let preview_data: String = source.chars().take(PREVIEW_SNIPPET_CHARS).collect();
    Ok(PickerMessagePreview {
        kind,
        sequence: sequence as u64,
        at: UtcMillis(at),
        preview_omitted: preview_data.len() < source.len(),
        preview_data,
    })
}
