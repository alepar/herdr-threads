//! Summary job store (spec §4): the p99 lease length, the thread loader, and the
//! `Summary` / `SummaryJob` / `SummarySubmit` handlers. Entitlement is decided
//! here against the canonical view (A2); the catch-up hooks are called through
//! the contract in `store::catch_up`.
use crate::protocol::{
    authority::CallerClaim,
    ids::{LeaseToken, MessageId, SeatId, SummaryBlockId, SummaryJobId, ThreadId},
    results::{ApiError, ErrorCode, MessageKind},
    summary::{
        AuthorRole, Block, BlockHeader, BlockProvenance, BundleMessage, ChildNarrative, CoverSizes,
        Fold, Identifier, ItemBody, ItemStatus, JobBundle, JobRef, JobTicket, LEASE_MAX_MS,
        LEASE_MIN_MS, LedgerItem, Level0Records, NewStatus, P99_CLAMP_MAX_MS, P99_CLAMP_MIN_MS,
        P99_MIN_SAMPLES, P99_SAMPLE_WINDOW, PinnedText, RESERVATION_LAPSE_MS, SUBMISSION_SCHEMA,
        SeqRange, SubmitOutcome, SummaryJobOutcome, SummaryJobRequest, SummaryOutcome,
        SummaryReady, SummaryRequest, SummarySettings, SummarySubmitRequest, SummaryWork,
        Transition, is_priority, submission_budget_bytes,
    },
    time::UtcMillis,
};
use crate::store::{
    catch_up::{self, CatchUpEntry},
    connection::{api_error, store_error},
    effective,
};
use crate::summary::{
    chunk::Chunker,
    cover::{self, BlockKey, CoverPlan, StoredMeta},
    fold::{self, BlockRecords},
    identifiers, ledger,
    render::{self, rendered_size},
    validate,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Fallback block provenance labels (the daemon wrote it, not a model).
const FALLBACK_MODEL: &str = "none";
const FALLBACK_PROMPT_VERSION: &str = "daemon-fallback-v1";

/// p99 of the latest `P99_SAMPLE_WINDOW` fetch-to-submit durations of Stored
/// jobs on the instance (nearest rank); `settings.p99_cold_ms`, unclamped,
/// below `P99_MIN_SAMPLES`; otherwise clamped to [P99_CLAMP_MIN_MS, P99_CLAMP_MAX_MS].
pub fn p99_job_duration(
    conn: &Connection,
    instance: &str,
    settings: &SummarySettings,
) -> Result<u64, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT duration_ms FROM summary_job_durations WHERE instance_id=?1 \
             ORDER BY ordinal DESC LIMIT ?2",
        )
        .map_err(store_error)?;
    let mut samples: Vec<u64> = stmt
        .query_map(params![instance, P99_SAMPLE_WINDOW as i64], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(store_error)?
        .map(|row| row.map(|ms| u64::try_from(ms).unwrap_or(0)))
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    if samples.len() < P99_MIN_SAMPLES {
        return Ok(settings.p99_cold_ms);
    }
    samples.sort_unstable();
    // Nearest rank: ceil(0.99 * n), 1-based.
    let rank = (99 * samples.len()).div_ceil(100);
    Ok(samples[rank - 1].clamp(P99_CLAMP_MIN_MS, P99_CLAMP_MAX_MS))
}

/// The lease a fetch grants: `min(max(2 * p99, 60 s), 10 min)`.
fn lease_len_ms(
    conn: &Connection,
    instance: &str,
    settings: &SummarySettings,
) -> Result<u64, ApiError> {
    let p99 = p99_job_duration(conn, instance, settings)?;
    Ok(p99.saturating_mul(2).clamp(LEASE_MIN_MS, LEASE_MAX_MS))
}

/// Record one fetch-to-submit duration of a Stored job.
pub fn record_duration(
    conn: &Connection,
    instance: &str,
    job: &str,
    duration_ms: u64,
    now: UtcMillis,
) -> Result<(), ApiError> {
    conn.execute(
        "INSERT INTO summary_job_durations(instance_id, job_id, duration_ms, recorded_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![instance, job, to_i64(duration_ms), now.0],
    )
    .map_err(store_error)?;
    Ok(())
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

// ---- thread loader (ht-1ip.3): published head, bundle messages, full chunks ----

/// Longest system-message text carried in a bundle line (spec §3).
const SYSTEM_TEXT_MAX_BYTES: usize = 256;
/// Timeline scan page; `scan_effective_timeline` allows at most 100.
const SCAN_PAGE: u16 = 100;
/// Messages loaded per step by `thread_has_full_chunk`.
const FULL_CHUNK_STEP: u64 = 32;

/// The published head of a thread: its highest published sequence
/// (`next_sequence - 1`), counting materialized messages and published warnings.
pub fn published_head(conn: &Connection, thread: &ThreadId) -> Result<u64, ApiError> {
    let next: i64 = conn
        .query_row(
            "SELECT next_sequence FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    u64::try_from(next - 1).map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid next_sequence"))
}

/// Compact one-line text of a system event: the event JSON re-encoded
/// compactly, control characters replaced by spaces, cut to 256 bytes at a
/// char boundary.
fn system_text(event_json: Option<&str>) -> String {
    let raw = event_json.unwrap_or("");
    let compact = serde_json::from_str::<serde_json::Value>(raw)
        .map_or_else(|_| raw.to_string(), |value| value.to_string());
    let mut text: String = compact
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if text.len() > SYSTEM_TEXT_MAX_BYTES {
        let mut cut = SYSTEM_TEXT_MAX_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

fn bad_row(detail: &'static str) -> ApiError {
    api_error(ErrorCode::StoreCorrupt, detail)
}

fn physical_message(conn: &Connection, id: &str, sequence: u64) -> Result<BundleMessage, ApiError> {
    #[allow(clippy::type_complexity)]
    let row: (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT kind, actor_seat_id, body, event_json, decision_at, author_role, relays_user \
             FROM messages WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .map_err(store_error)?;
    let (kind, actor, body, event_json, decision_at, role, relays) = row;
    let (kind, text) = match kind.as_str() {
        "ordinary" => (MessageKind::Ordinary, body.unwrap_or_default()),
        "info" => (MessageKind::Info, system_text(event_json.as_deref())),
        "warn" => (MessageKind::Warn, system_text(event_json.as_deref())),
        _ => return Err(bad_row("unknown message kind")),
    };
    Ok(BundleMessage {
        sequence,
        message: MessageId::parse(id).map_err(|_| bad_row("invalid message id"))?,
        kind,
        author: actor
            .map(|seat| SeatId::parse(seat).map_err(|_| bad_row("invalid actor seat")))
            .transpose()?,
        author_role: role.as_deref().and_then(AuthorRole::from_column),
        relays_user: relays != 0,
        created_at: UtcMillis(decision_at),
        text,
    })
}

fn published_warning_message(
    conn: &Connection,
    warning: &effective::EffectiveWarning,
) -> Result<BundleMessage, ApiError> {
    let decision_at: i64 = conn
        .query_row(
            "SELECT sm.decision_at FROM send_manifests sm \
             JOIN prepared_unavailable_warnings w ON w.preparation_id=sm.preparation_id \
             WHERE w.warning_id=?1",
            [&warning.id],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(BundleMessage {
        sequence: u64::try_from(warning.sequence)
            .map_err(|_| bad_row("invalid warning sequence"))?,
        message: MessageId::parse(warning.id.clone()).map_err(|_| bad_row("invalid warning id"))?,
        kind: MessageKind::Warn,
        author: None,
        author_role: None,
        relays_user: false,
        created_at: UtcMillis(decision_at),
        text: system_text(Some(&warning.event_json)),
    })
}

/// Every position `first_seq..=last_seq` of a thread as one `BundleMessage`,
/// in order: materialized rows, and published warnings that have no row yet.
pub fn bundle_messages(
    conn: &Connection,
    thread: &ThreadId,
    first_seq: u64,
    last_seq: u64,
) -> Result<Vec<BundleMessage>, ApiError> {
    let invalid = || api_error(ErrorCode::InvalidRequest, "invalid summary sequence range");
    if first_seq == 0 || last_seq < first_seq {
        return Err(invalid());
    }
    let high_water = i64::try_from(last_seq).map_err(|_| invalid())?;
    let mut position = effective::TimelinePosition {
        after_sequence: i64::try_from(first_seq - 1).map_err(|_| invalid())?,
        high_water_sequence: high_water,
    };
    let mut out = Vec::new();
    loop {
        let slice =
            effective::scan_effective_timeline(conn, thread.as_str(), Some(position), SCAN_PAGE)?;
        for entry in slice.entries {
            out.push(match entry {
                effective::EffectiveTimelineEntry::Physical { id, sequence, .. } => {
                    let sequence =
                        u64::try_from(sequence).map_err(|_| bad_row("invalid sequence"))?;
                    physical_message(conn, &id, sequence)?
                }
                effective::EffectiveTimelineEntry::PublishedWarning(warning) => {
                    published_warning_message(conn, &warning)?
                }
            });
        }
        if !slice.has_more {
            return Ok(out);
        }
        position = slice.position;
    }
}

/// Whether the thread already holds one full chunk below its published head:
/// walks from sequence 1 with a `Chunker` and stops at the first closed chunk
/// (true) or at the head (false), reading one chunk's worth of messages plus
/// at most one step.
pub fn thread_has_full_chunk(
    conn: &Connection,
    thread: &ThreadId,
    settings: &SummarySettings,
) -> Result<bool, ApiError> {
    let head = published_head(conn, thread)?;
    let mut chunker = Chunker::new(u64::from(settings.chunk_bytes));
    let mut next = 1;
    while next <= head {
        let last = head.min(next + FULL_CHUNK_STEP - 1);
        for message in bundle_messages(conn, thread, next, last)? {
            if !chunker
                .push(message.sequence, rendered_size(&message))
                .is_empty()
            {
                return Ok(true);
            }
        }
        next = last + 1;
    }
    Ok(false)
}

// ---- handlers (ht-1ip.5): plan, lease, bundle, submit ----

fn parse_id<T>(
    parse: impl FnOnce(String) -> Result<T, &'static str>,
    value: String,
    detail: &'static str,
) -> Result<T, ApiError> {
    parse(value).map_err(|_| bad_row(detail))
}

fn seq_u64(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value).map_err(|_| bad_row("negative summary value"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Caller {
    seat: SeatId,
    /// The claim names the seat's current open binding (generation and
    /// execution); only then may a call enter or end catch-up.
    current: bool,
}

/// A2 entitlement, decided against the canonical view. The claim's instance
/// must be the service instance, the seat must belong to it, and the seat may
/// read the thread by exactly the predicate `history` applies: the thread
/// exists in the instance (history has no per-seat read gate). Either claim
/// role is admitted: a summary worker is a declared subagent of the seat.
fn check_caller(
    conn: &Connection,
    instance: &str,
    claim: &CallerClaim,
    thread: &ThreadId,
) -> Result<Caller, ApiError> {
    if claim.instance != instance {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "caller instance does not match service",
        ));
    }
    let seat: Option<(String, String)> = conn
        .query_row(
            "SELECT instance_id, state FROM seats WHERE id=?1",
            [claim.seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((_, state)) = seat.filter(|(owner, _)| owner == instance) else {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "seat does not belong to instance",
        ));
    };
    let known: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
            params![thread.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !known {
        return Err(api_error(ErrorCode::NotFound, "thread not found"));
    }
    let current = state == "resolved"
        && conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 \
                 AND ended_at IS NULL AND generation=?2 AND execution_id=?3)",
                params![
                    claim.seat.as_str(),
                    to_i64(claim.binding_generation),
                    claim.execution.as_str()
                ],
                |r| r.get(0),
            )
            .map_err(store_error)?;
    Ok(Caller {
        seat: claim.seat.clone(),
        current,
    })
}

#[derive(Debug, Clone)]
struct JobRow {
    id: String,
    thread: ThreadId,
    version: String,
    level: u32,
    index: u64,
    range: SeqRange,
    lease_seat: Option<String>,
    lease_token: Option<String>,
    fetched_at: Option<i64>,
    lease_until: Option<i64>,
    rejections: u32,
    last_submit_token: Option<String>,
    last_submit_digest: Option<Vec<u8>>,
    last_submit_result: Option<String>,
    block_id: Option<String>,
}

const JOB_COLUMNS: &str = "id, thread_id, chunking_version, level, idx, first_seq, last_seq, \
     lease_seat_id, lease_token, fetched_at, lease_until, rejections, last_submit_token, \
     last_submit_digest, last_submit_result, block_id";

fn job_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(JobRow, String)> {
    let thread: String = r.get(1)?;
    let level: i64 = r.get(3)?;
    let index: i64 = r.get(4)?;
    let first: i64 = r.get(5)?;
    let last: i64 = r.get(6)?;
    let rejections: i64 = r.get(11)?;
    Ok((
        JobRow {
            id: r.get(0)?,
            thread: ThreadId::new("placeholder"),
            version: r.get(2)?,
            level: u32::try_from(level).unwrap_or(u32::MAX),
            index: u64::try_from(index).unwrap_or(0),
            range: SeqRange {
                first_seq: u64::try_from(first).unwrap_or(0),
                last_seq: u64::try_from(last).unwrap_or(0),
            },
            lease_seat: r.get(7)?,
            lease_token: r.get(8)?,
            fetched_at: r.get(9)?,
            lease_until: r.get(10)?,
            rejections: u32::try_from(rejections).unwrap_or(u32::MAX),
            last_submit_token: r.get(12)?,
            last_submit_digest: r.get(13)?,
            last_submit_result: r.get(14)?,
            block_id: r.get(15)?,
        },
        thread,
    ))
}

fn finish_job((mut job, thread): (JobRow, String)) -> Result<JobRow, ApiError> {
    job.thread = parse_id(ThreadId::parse, thread, "invalid job thread")?;
    Ok(job)
}

fn load_job(conn: &Connection, instance: &str, id: &str) -> Result<JobRow, ApiError> {
    let row = conn
        .query_row(
            &format!("SELECT {JOB_COLUMNS} FROM summary_jobs WHERE id=?1 AND instance_id=?2"),
            params![id, instance],
            job_from_row,
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "summary job not found"))?;
    finish_job(row)
}

impl JobRow {
    /// A lease is live while its token is held and `lease_until` is ahead of
    /// `now`; an unfetched reservation carries `reserved_at + 60 s`.
    fn live(&self, now: i64) -> bool {
        self.lease_token.is_some() && self.lease_until.is_some_and(|until| until > now)
    }
    fn job_ref(&self) -> Result<JobRef, ApiError> {
        Ok(JobRef {
            job_id: parse_id(SummaryJobId::parse, self.id.clone(), "invalid job id")?,
            level: self.level,
            index: self.index,
            range: self.range,
            lease_until: UtcMillis(self.lease_until.unwrap_or(0)),
        })
    }
    fn ticket(&self, settings: &SummarySettings) -> Result<JobTicket, ApiError> {
        Ok(JobTicket {
            job_id: parse_id(SummaryJobId::parse, self.id.clone(), "invalid job id")?,
            lease_token: parse_id(
                LeaseToken::parse,
                self.lease_token.clone().unwrap_or_default(),
                "invalid lease token",
            )?,
            lease_until: UtcMillis(self.lease_until.unwrap_or(0)),
            level: self.level,
            index: self.index,
            range: self.range,
            budget_bytes: submission_budget_bytes(self.level, settings.narrative_bytes),
        })
    }
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

/// Every priority message of the thread up to `up_to_seq` (spec §1 predicate),
/// as bundle messages in sequence order.
fn priority_messages(
    conn: &Connection,
    thread: &ThreadId,
    up_to_seq: u64,
) -> Result<Vec<BundleMessage>, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, sequence FROM messages WHERE thread_id=?1 AND sequence<=?2 \
             AND kind='ordinary' AND (author_role='human' OR relays_user=1) ORDER BY sequence",
        )
        .map_err(store_error)?;
    let rows: Vec<(String, i64)> = stmt
        .query_map(params![thread.as_str(), to_i64(up_to_seq)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    rows.into_iter()
        .map(|(id, sequence)| physical_message(conn, &id, seq_u64(sequence)?))
        .collect()
}

/// Sequences of the priority messages up to `up_to_seq`: the spec §1 predicate
/// on ordinary messages only, the same set `priority_messages` and
/// `ledger::prefill` use. An info or warn event never counts, even when its
/// stamped `author_role` is human.
fn priority_sequences(
    conn: &Connection,
    thread: &ThreadId,
    up_to_seq: u64,
) -> Result<HashSet<u64>, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT sequence, author_role, relays_user FROM messages \
             WHERE thread_id=?1 AND sequence<=?2 AND kind='ordinary'",
        )
        .map_err(store_error)?;
    let rows = stmt
        .query_map(params![thread.as_str(), to_i64(up_to_seq)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut set = HashSet::new();
    for row in rows {
        let (sequence, role, relays) = row.map_err(store_error)?;
        let role = role.as_deref().and_then(AuthorRole::from_column);
        if is_priority(role, relays != 0) {
            set.insert(seq_u64(sequence)?);
        }
    }
    Ok(set)
}

fn item_kind(body: &ItemBody) -> &'static str {
    match body {
        ItemBody::UserInstruction { .. } => "user_instruction",
        ItemBody::Decision { .. } => "decision",
        ItemBody::OpenItem { .. } => "open_item",
    }
}

fn status_str(status: NewStatus) -> &'static str {
    match status {
        NewStatus::Done => "done",
        NewStatus::Resolved => "resolved",
        NewStatus::Superseded => "superseded",
    }
}

fn status_from(value: &str) -> Result<NewStatus, ApiError> {
    match value {
        "done" => Ok(NewStatus::Done),
        "resolved" => Ok(NewStatus::Resolved),
        "superseded" => Ok(NewStatus::Superseded),
        _ => Err(bad_row("unknown transition status")),
    }
}

/// The level-0 records of the thread's current-version blocks whose range
/// starts at or below `up_to_seq` (and, for bundles, stored at or before
/// `created_at_le`), oldest first.
fn load_level0_records(
    conn: &Connection,
    thread: &ThreadId,
    version: &str,
    up_to_seq: u64,
    created_at_le: Option<i64>,
) -> Result<Vec<(SeqRange, Level0Records)>, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, first_seq, last_seq FROM summary_blocks \
             WHERE thread_id=?1 AND chunking_version=?2 AND level=0 AND first_seq<=?3 \
             AND (?4 IS NULL OR created_at<=?4) ORDER BY first_seq",
        )
        .map_err(store_error)?;
    let blocks: Vec<(String, i64, i64)> = stmt
        .query_map(
            params![thread.as_str(), version, to_i64(up_to_seq), created_at_le],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    let mut out = Vec::new();
    for (id, first, last) in blocks {
        let mut records = Level0Records {
            items: Vec::new(),
            identifiers: Vec::new(),
            transitions: Vec::new(),
        };
        let mut items = conn
            .prepare(
                "SELECT kind, item_id, seq, body_json FROM summary_items \
                 WHERE block_id=?1 ORDER BY rowid",
            )
            .map_err(store_error)?;
        let rows = items
            .query_map([&id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(store_error)?;
        for row in rows {
            let (kind, item_id, seq, body) = row.map_err(store_error)?;
            if kind == "identifier" {
                records.identifiers.push(
                    serde_json::from_str::<Identifier>(&body)
                        .map_err(|_| bad_row("invalid stored identifier"))?,
                );
            } else {
                records.items.push(LedgerItem {
                    id: item_id,
                    seq: seq_u64(seq)?,
                    body: serde_json::from_str::<ItemBody>(&body)
                        .map_err(|_| bad_row("invalid stored item"))?,
                });
            }
        }
        let mut transitions = conn
            .prepare(
                "SELECT target_id, new_status, cite_seq FROM summary_transitions \
                 WHERE block_id=?1 ORDER BY ordinal",
            )
            .map_err(store_error)?;
        let rows = transitions
            .query_map([&id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(store_error)?;
        for row in rows {
            let (target_id, status, cite) = row.map_err(store_error)?;
            records.transitions.push(Transition {
                target_id,
                new_status: status_from(&status)?,
                cite_seq: seq_u64(cite)?,
            });
        }
        out.push((
            SeqRange {
                first_seq: seq_u64(first)?,
                last_seq: seq_u64(last)?,
            },
            records,
        ));
    }
    Ok(out)
}

type BlockRow = (
    String,
    i64,
    i64,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    i64,
);

fn block_by_key(
    conn: &Connection,
    thread: &ThreadId,
    version: &str,
    key: BlockKey,
) -> Result<Option<Block>, ApiError> {
    let row: Option<BlockRow> = conn
        .query_row(
            "SELECT id, first_seq, last_seq, children_json, source_hash, narrative, fallback, \
             author_seat_id, model, prompt_version, created_at FROM summary_blocks \
             WHERE thread_id=?1 AND chunking_version=?2 AND level=?3 AND idx=?4",
            params![thread.as_str(), version, key.level, to_i64(key.index)],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((
        id,
        first,
        last,
        children,
        source_hash,
        narrative,
        fallback,
        author,
        model,
        prompt,
        created,
    )) = row
    else {
        return Ok(None);
    };
    let children: Vec<String> =
        serde_json::from_str(&children).map_err(|_| bad_row("invalid block children"))?;
    Ok(Some(Block {
        block_id: parse_id(SummaryBlockId::parse, id, "invalid block id")?,
        header: BlockHeader {
            thread: thread.clone(),
            chunking_version: version.to_string(),
            level: key.level,
            index: key.index,
            range: SeqRange {
                first_seq: seq_u64(first)?,
                last_seq: seq_u64(last)?,
            },
            children: children
                .into_iter()
                .map(|c| parse_id(SummaryBlockId::parse, c, "invalid child block id"))
                .collect::<Result<_, _>>()?,
            source_hash,
            fallback: fallback != 0,
            provenance: BlockProvenance::DerivedSummary,
            author_seat: parse_id(SeatId::parse, author, "invalid author seat")?,
            prompt_version: prompt,
            model,
            created_at: UtcMillis(created),
        },
        narrative,
    }))
}

fn block_by_id(conn: &Connection, id: &str) -> Result<Block, ApiError> {
    let (thread, version, level, index): (String, String, i64, i64) = conn
        .query_row(
            "SELECT thread_id, chunking_version, level, idx FROM summary_blocks WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(store_error)?;
    block_by_key(
        conn,
        &parse_id(ThreadId::parse, thread, "invalid block thread")?,
        &version,
        BlockKey {
            level: u32::try_from(level).unwrap_or(u32::MAX),
            index: seq_u64(index)?,
        },
    )?
    .ok_or_else(|| bad_row("block vanished"))
}

/// Every full level-0 chunk up to `frontier` as a `summary_jobs` row (created
/// unleased the first time it is planned). Resumes after the highest planned
/// chunk so each call renders only new messages. Returns the level-0 jobs
/// whose range ends at or below `frontier`, ascending.
fn plan_level0(
    tx: &Connection,
    instance: &str,
    thread: &ThreadId,
    version: &str,
    settings: &SummarySettings,
    frontier: u64,
    now: UtcMillis,
) -> Result<Vec<(String, u64, SeqRange)>, ApiError> {
    let last: Option<(i64, i64)> = tx
        .query_row(
            "SELECT idx, last_seq FROM summary_jobs WHERE thread_id=?1 AND chunking_version=?2 \
             AND level=0 ORDER BY idx DESC LIMIT 1",
            params![thread.as_str(), version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let (next_index, mut next_first) = match last {
        Some((idx, last_seq)) => (seq_u64(idx)? + 1, seq_u64(last_seq)? + 1),
        None => (0, 1),
    };
    let mut chunker = Chunker::resume(u64::from(settings.chunk_bytes), next_index, next_first);
    while next_first <= frontier {
        let page_last = frontier.min(next_first + FULL_CHUNK_STEP * 2 - 1);
        for message in bundle_messages(tx, thread, next_first, page_last)? {
            for span in chunker.push(message.sequence, rendered_size(&message)) {
                tx.execute(
                    "INSERT OR IGNORE INTO summary_jobs(id, instance_id, thread_id, \
                     chunking_version, level, idx, first_seq, last_seq, created_at) \
                     VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, ?8)",
                    params![
                        new_id("sj"),
                        instance,
                        thread.as_str(),
                        version,
                        to_i64(span.index),
                        to_i64(span.range.first_seq),
                        to_i64(span.range.last_seq),
                        now.0
                    ],
                )
                .map_err(store_error)?;
            }
        }
        next_first = page_last + 1;
    }
    let mut stmt = tx
        .prepare(
            "SELECT id, idx, first_seq, last_seq FROM summary_jobs WHERE thread_id=?1 \
             AND chunking_version=?2 AND level=0 AND last_seq<=?3 ORDER BY idx",
        )
        .map_err(store_error)?;
    let rows = stmt
        .query_map(params![thread.as_str(), version, to_i64(frontier)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, idx, first, last) = row.map_err(store_error)?;
        out.push((
            id,
            seq_u64(idx)?,
            SeqRange {
                first_seq: seq_u64(first)?,
                last_seq: seq_u64(last)?,
            },
        ));
    }
    Ok(out)
}

fn stored_metas(
    conn: &Connection,
    thread: &ThreadId,
    version: &str,
    frontier: u64,
) -> Result<Vec<StoredMeta>, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT level, idx, first_seq, last_seq, length(CAST(narrative AS BLOB)) \
             FROM summary_blocks WHERE thread_id=?1 AND chunking_version=?2 AND last_seq<=?3 \
             ORDER BY level, idx",
        )
        .map_err(store_error)?;
    let rows = stmt
        .query_map(params![thread.as_str(), version, to_i64(frontier)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (level, idx, first, last, bytes) = row.map_err(store_error)?;
        out.push(StoredMeta {
            key: BlockKey {
                level: u32::try_from(level).unwrap_or(u32::MAX),
                index: seq_u64(idx)?,
            },
            chunking_version: version.to_string(),
            range: SeqRange {
                first_seq: seq_u64(first)?,
                last_seq: seq_u64(last)?,
            },
            narrative_bytes: seq_u64(bytes)?,
        });
    }
    Ok(out)
}

/// The job row of a rollup, created unleased when the cover first needs it.
fn ensure_rollup_job(
    tx: &Connection,
    instance: &str,
    thread: &ThreadId,
    version: &str,
    key: BlockKey,
    stored: &[StoredMeta],
    now: UtcMillis,
) -> Result<String, ApiError> {
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM summary_jobs WHERE thread_id=?1 AND chunking_version=?2 \
             AND level=?3 AND idx=?4",
            params![thread.as_str(), version, key.level, to_i64(key.index)],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let kids = cover::children(key);
    let range_of = |child: &BlockKey| {
        stored
            .iter()
            .find(|m| m.key == *child)
            .map(|m| m.range)
            .ok_or_else(|| bad_row("rollup child block missing"))
    };
    let first = range_of(&kids[0])?.first_seq;
    let last = range_of(&kids[kids.len() - 1])?.last_seq;
    let id = new_id("sj");
    tx.execute(
        "INSERT INTO summary_jobs(id, instance_id, thread_id, chunking_version, level, idx, \
         first_seq, last_seq, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            instance,
            thread.as_str(),
            version,
            key.level,
            to_i64(key.index),
            to_i64(first),
            to_i64(last),
            now.0
        ],
    )
    .map_err(store_error)?;
    Ok(id)
}

/// `Summary`: plan the cover to the frontier, then answer Ready (the cover,
/// the fold and the raw tail) or Work (jobs leased to the caller).
pub fn summary(
    tx: &Transaction<'_>,
    instance: &str,
    request: &SummaryRequest,
    settings: &SummarySettings,
    now: UtcMillis,
) -> Result<SummaryOutcome, ApiError> {
    let caller = check_caller(tx, instance, &request.claim, &request.thread)?;
    let thread = &request.thread;
    let version = render::chunking_version(settings);
    // Frontier: an active catch-up row's F for this seat, else the published head.
    let active: Option<i64> = tx
        .query_row(
            "SELECT frontier_seq FROM catch_up WHERE seat_id=?1 AND thread_id=?2 \
             AND state='active'",
            params![caller.seat.as_str(), thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let head = published_head(tx, thread)?;
    let frontier = match active {
        Some(f) => seq_u64(f)?.min(head),
        None => head,
    };

    let level0 = plan_level0(tx, instance, thread, &version, settings, frontier, now)?;
    let stored = stored_metas(tx, thread, &version, frontier)?;
    let plan = cover::plan_cover(&version, level0.len() as u64, &stored, settings)
        .map_err(|_| bad_row("inconsistent summary blocks"))?;
    let covered_end = level0.last().map_or(0, |(_, _, range)| range.last_seq);

    if plan.ready() {
        let ready = assemble_ready(tx, thread, &version, frontier, &plan, covered_end, settings)?;
        if caller.current {
            catch_up::on_ready(
                tx,
                &caller.seat,
                thread,
                request.claim.binding_generation,
                now,
                settings,
            )?;
        }
        return Ok(SummaryOutcome::Ready(ready));
    }

    // Work: eligible jobs, level 0 first (ascending chunk order), then rollups.
    let mut eligible: Vec<String> = plan
        .missing_level0
        .iter()
        .filter_map(|index| level0.iter().find(|(_, i, _)| i == index))
        .map(|(id, _, _)| id.clone())
        .collect();
    for key in &plan.needed_rollups {
        eligible.push(ensure_rollup_job(
            tx, instance, thread, &version, *key, &stored, now,
        )?);
    }
    let (jobs, leased_elsewhere) =
        lease_pass(tx, instance, &caller.seat, &eligible, settings, now)?;
    let frontier = if caller.current {
        catch_up::enter_or_keep(
            tx,
            &CatchUpEntry {
                seat: &caller.seat,
                thread,
                frontier_seq: frontier,
                binding_generation: request.claim.binding_generation,
                execution: &request.claim.execution,
                now,
            },
            settings,
        )?
    } else {
        frontier
    };
    Ok(SummaryOutcome::Work(SummaryWork {
        frontier,
        jobs,
        leased_elsewhere,
    }))
}

/// Work leasing, in one place: a live lease of the same seat is re-returned
/// with its token; a live lease of another seat is listed as leased elsewhere;
/// a free job (never leased, lapsed reservation, expired lease) is reserved to
/// the caller, at most `max_new_leases` per response, for 60 s until fetched.
fn lease_pass(
    tx: &Connection,
    instance: &str,
    seat: &SeatId,
    eligible: &[String],
    settings: &SummarySettings,
    now: UtcMillis,
) -> Result<(Vec<JobTicket>, Vec<JobRef>), ApiError> {
    let mut tickets = Vec::new();
    let mut elsewhere = Vec::new();
    let mut fresh = 0u32;
    for id in eligible {
        let mut job = load_job(tx, instance, id)?;
        if job.block_id.is_some() {
            continue;
        }
        if job.live(now.0) {
            if job.lease_seat.as_deref() == Some(seat.as_str()) {
                tickets.push(job.ticket(settings)?);
            } else {
                elsewhere.push(job.job_ref()?);
            }
            continue;
        }
        if fresh >= settings.max_new_leases {
            continue;
        }
        fresh += 1;
        let token = uuid::Uuid::new_v4().to_string();
        let until = now.0.saturating_add(to_i64(RESERVATION_LAPSE_MS));
        tx.execute(
            "UPDATE summary_jobs SET lease_seat_id=?1, lease_token=?2, reserved_at=?3, \
             fetched_at=NULL, lease_until=?4, attempts=attempts+1 WHERE id=?5",
            params![seat.as_str(), token, now.0, until, id],
        )
        .map_err(store_error)?;
        job.lease_seat = Some(seat.as_str().to_string());
        job.lease_token = Some(token);
        job.lease_until = Some(until);
        tickets.push(job.ticket(settings)?);
    }
    Ok((tickets, elsewhere))
}

/// Spec §5: transitions the fold guards refused are logged, one line per
/// computation (both callers run per agent request, never per scheduler tick).
fn warn_dropped(thread: &ThreadId, site: &str, state: &fold::FoldState) {
    if let Some(report) = fold::drop_report(&state.dropped) {
        eprintln!(
            "herdr-threads: warning: summary fold of {} ({site}): {report}",
            thread.as_str()
        );
    }
}

fn assemble_ready(
    tx: &Connection,
    thread: &ThreadId,
    version: &str,
    frontier: u64,
    plan: &CoverPlan,
    covered_end: u64,
    settings: &SummarySettings,
) -> Result<SummaryReady, ApiError> {
    let blocks = plan
        .cover
        .iter()
        .map(|key| {
            block_by_key(tx, thread, version, *key)?.ok_or_else(|| bad_row("cover block missing"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let records = load_level0_records(tx, thread, version, frontier, None)?;
    let priority = priority_sequences(tx, thread, frontier)?;
    let inputs: Vec<BlockRecords<'_>> = records
        .iter()
        .map(|(range, records)| BlockRecords {
            range: *range,
            records,
        })
        .collect();
    let state = fold::compute(&inputs, frontier, &|seq| priority.contains(&seq));
    warn_dropped(thread, "ready", &state);
    // Ready's window starts where the cover's level-0 narratives start.
    let window_first = blocks
        .iter()
        .filter(|b| b.header.level == 0)
        .map(|b| b.header.range.first_seq)
        .min()
        .unwrap_or(covered_end + 1);
    let fold = fold::render(&state, window_first);

    let mut tail = Vec::new();
    let mut tail_complete = true;
    if covered_end < frontier {
        let mut bytes = 0u64;
        for message in bundle_messages(tx, thread, covered_end + 1, frontier)? {
            bytes += rendered_size(&message);
            if bytes > u64::from(settings.chunk_bytes) && !tail.is_empty() {
                tail_complete = false;
                break;
            }
            tail.push(message);
        }
    }
    let narrative_bytes: u64 = blocks.iter().map(|b| b.narrative.len() as u64).sum();
    let over_budget = plan.over_budget
        || narrative_bytes > u64::from(settings.display_bytes)
        || fold.rendered_bytes > settings.fold_display_bytes;
    let sizes = CoverSizes {
        narrative_bytes: u32::try_from(narrative_bytes).unwrap_or(u32::MAX),
        display_bytes: settings.display_bytes,
        fold_bytes: fold.rendered_bytes,
        fold_display_bytes: settings.fold_display_bytes,
    };
    Ok(SummaryReady {
        frontier,
        cover: blocks,
        fold,
        tail,
        tail_complete,
        over_budget,
        sizes,
    })
}

/// The fold of the stored level-0 records up to `up_to`, rendered for `window_first`.
/// `extra` stands in for chunks not yet stored (the prefill of their priority
/// messages).
fn bundle_fold(
    conn: &Connection,
    job: &JobRow,
    fetched_at: i64,
    with_prefill: bool,
) -> Result<Fold, ApiError> {
    let range = job.range;
    let thread = &job.thread;
    let stored = load_level0_records(conn, thread, &job.version, range.last_seq, Some(fetched_at))?;
    let prefill = Level0Records {
        items: if with_prefill {
            ledger::prefill(&priority_messages(conn, thread, range.last_seq)?)
        } else {
            Vec::new()
        },
        identifiers: Vec::new(),
        transitions: Vec::new(),
    };
    let mut inputs: Vec<BlockRecords<'_>> = stored
        .iter()
        .map(|(range, records)| BlockRecords {
            range: *range,
            records,
        })
        .collect();
    if with_prefill {
        inputs.push(BlockRecords {
            range: SeqRange {
                first_seq: 1,
                last_seq: range.last_seq,
            },
            records: &prefill,
        });
    }
    let priority = priority_sequences(conn, thread, range.last_seq)?;
    let state = fold::compute(&inputs, range.last_seq, &|seq| priority.contains(&seq));
    warn_dropped(thread, "job bundle", &state);
    Ok(fold::render(&state, range.first_seq))
}

/// The bundle of a job as of `fetched_at`: deterministic, so submit rebuilds
/// the one the worker was shown.
fn build_bundle(
    conn: &Connection,
    job: &JobRow,
    fetched_at: i64,
    settings: &SummarySettings,
) -> Result<JobBundle, ApiError> {
    let thread = &job.thread;
    let range = job.range;
    let mut messages = Vec::new();
    let mut children = Vec::new();
    let mut pinned = Vec::new();
    let fold;
    if job.level == 0 {
        messages = bundle_messages(conn, thread, range.first_seq, range.last_seq)?;
        fold = bundle_fold(conn, job, fetched_at, true)?;
    } else {
        let key = BlockKey {
            level: job.level,
            index: job.index,
        };
        for child in cover::children(key) {
            let block = block_by_key(conn, thread, &job.version, child)?
                .ok_or_else(|| api_error(ErrorCode::Conflict, "rollup children are not stored"))?;
            children.push(ChildNarrative {
                block_id: block.block_id,
                level: child.level,
                index: child.index,
                range: block.header.range,
                narrative: block.narrative,
                fallback: block.header.fallback,
            });
        }
        fold = bundle_fold(conn, job, fetched_at, false)?;
        for entry in &fold.entries {
            if let ItemBody::UserInstruction {
                text: None,
                text_ref: Some(_),
                ..
            } = &entry.item.body
                && entry.status == ItemStatus::Open
            {
                let raw = bundle_messages(conn, thread, entry.item.seq, entry.item.seq)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| bad_row("pinned instruction message missing"))?;
                pinned.push(PinnedText {
                    item_id: entry.item.id.clone(),
                    seq: entry.item.seq,
                    text: Some(raw.text),
                    text_ref: None,
                });
            }
        }
    }
    let mut bundle = JobBundle {
        job_id: parse_id(SummaryJobId::parse, job.id.clone(), "invalid job id")?,
        thread: thread.clone(),
        chunking_version: job.version.clone(),
        level: job.level,
        index: job.index,
        range,
        submission_schema: SUBMISSION_SCHEMA,
        budget_bytes: submission_budget_bytes(job.level, settings.narrative_bytes),
        narrative_bytes: settings.narrative_bytes,
        messages,
        children,
        fold,
        pinned,
        size_bytes: 0,
        oversized: false,
    };
    render::bound_bundle(&mut bundle, settings.bundle_bytes);
    Ok(bundle)
}

/// The caller must hold the job's lease: its seat and token.
fn holds_lease(job: &JobRow, seat: &SeatId, token: &str) -> bool {
    job.lease_seat.as_deref() == Some(seat.as_str()) && job.lease_token.as_deref() == Some(token)
}

fn stale_version(job: &JobRow, settings: &SummarySettings) -> Result<(), ApiError> {
    if job.version != render::chunking_version(settings) {
        return Err(api_error(
            ErrorCode::Conflict,
            "summary job was planned under an older chunking_version",
        ));
    }
    Ok(())
}

/// `SummaryJob`: the fetch starts the lease clock and returns the bundle.
pub fn summary_job(
    tx: &Transaction<'_>,
    instance: &str,
    request: &SummaryJobRequest,
    settings: &SummarySettings,
    now: UtcMillis,
) -> Result<SummaryJobOutcome, ApiError> {
    let mut job = load_job(tx, instance, request.job_id.as_str())?;
    let caller = check_caller(tx, instance, &request.claim, &job.thread)?;
    if !holds_lease(&job, &caller.seat, request.lease_token.as_str()) {
        // A reservation that lapsed and was taken by another seat.
        if job.block_id.is_none()
            && job.live(now.0)
            && job.lease_seat.as_deref() != Some(caller.seat.as_str())
        {
            return Ok(SummaryJobOutcome::ReservationLapsed {
                leased_elsewhere: job.job_ref()?,
            });
        }
        return Err(api_error(
            ErrorCode::Unauthorized,
            "lease token was not issued to this seat",
        ));
    }
    if job.block_id.is_some() {
        return Err(api_error(ErrorCode::Conflict, "summary job already stored"));
    }
    stale_version(&job, settings)?;
    let fetched_at = match job.fetched_at {
        // An unfetched reservation is honoured while still free, lapsed or not.
        None => {
            let len = lease_len_ms(tx, instance, settings)?;
            let until = now.0.saturating_add(to_i64(len));
            tx.execute(
                "UPDATE summary_jobs SET fetched_at=?1, lease_until=?2 WHERE id=?3",
                params![now.0, until, job.id],
            )
            .map_err(store_error)?;
            job.fetched_at = Some(now.0);
            job.lease_until = Some(until);
            now.0
        }
        Some(fetched) => {
            if !job.live(now.0) {
                return Err(api_error(ErrorCode::Conflict, "summary lease expired"));
            }
            fetched
        }
    };
    Ok(SummaryJobOutcome::Bundle(build_bundle(
        tx, &job, fetched_at, settings,
    )?))
}

struct NewBlock<'a> {
    seat: &'a SeatId,
    narrative: &'a str,
    fallback: bool,
    model: &'a str,
    prompt_version: &'a str,
    children: Vec<String>,
    source_hash: String,
    records: Option<&'a Level0Records>,
}

/// Insert a block (and a level-0 block's records) for the job; blocks are unique
/// per (thread, chunking_version, level, idx).
fn insert_block(
    tx: &Connection,
    instance: &str,
    job: &JobRow,
    new: &NewBlock<'_>,
    now: UtcMillis,
) -> Result<SummaryBlockId, ApiError> {
    let id = new_id("sb");
    tx.execute(
        "INSERT INTO summary_blocks(id, instance_id, thread_id, chunking_version, level, idx, \
         first_seq, last_seq, children_json, source_hash, narrative, fallback, provenance, \
         author_seat_id, model, prompt_version, job_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'derived_summary', ?13, \
         ?14, ?15, ?16, ?17)",
        params![
            id,
            instance,
            job.thread.as_str(),
            job.version,
            job.level,
            to_i64(job.index),
            to_i64(job.range.first_seq),
            to_i64(job.range.last_seq),
            serde_json::to_string(&new.children).map_err(|_| bad_row("unencodable children"))?,
            new.source_hash,
            new.narrative,
            i64::from(new.fallback),
            new.seat.as_str(),
            new.model,
            new.prompt_version,
            job.id,
            now.0
        ],
    )
    .map_err(store_error)?;
    if let Some(records) = new.records {
        for item in &records.items {
            tx.execute(
                "INSERT INTO summary_items(block_id, kind, item_id, thread_id, chunking_version, \
                 seq, body_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    item_kind(&item.body),
                    item.id,
                    job.thread.as_str(),
                    job.version,
                    to_i64(item.seq),
                    serde_json::to_string(&item.body).map_err(|_| bad_row("unencodable item"))?
                ],
            )
            .map_err(store_error)?;
        }
        for identifier in &records.identifiers {
            tx.execute(
                "INSERT INTO summary_items(block_id, kind, item_id, thread_id, chunking_version, \
                 seq, body_json) VALUES (?1, 'identifier', ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    identifier.value,
                    job.thread.as_str(),
                    job.version,
                    to_i64(identifier.seqs.iter().copied().min().unwrap_or(1).max(1)),
                    serde_json::to_string(identifier)
                        .map_err(|_| bad_row("unencodable identifier"))?
                ],
            )
            .map_err(store_error)?;
        }
        for (ordinal, transition) in records.transitions.iter().enumerate() {
            tx.execute(
                "INSERT INTO summary_transitions(block_id, ordinal, thread_id, chunking_version, \
                 target_id, new_status, cite_seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    ordinal as i64,
                    job.thread.as_str(),
                    job.version,
                    transition.target_id,
                    status_str(transition.new_status),
                    to_i64(transition.cite_seq)
                ],
            )
            .map_err(store_error)?;
        }
    }
    parse_id(SummaryBlockId::parse, id, "invalid block id")
}

fn source_hash(bundle: &JobBundle) -> String {
    let mut hasher = Sha256::new();
    for message in &bundle.messages {
        hasher.update(render::render_message(message).as_bytes());
    }
    for child in &bundle.children {
        hasher.update(child.block_id.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(child.narrative.as_bytes());
        hasher.update([0]);
    }
    hex(&hasher.finalize())
}

fn record_submit(
    tx: &Connection,
    job: &JobRow,
    token: &str,
    digest: &[u8],
    outcome: &SubmitOutcome,
    rejections: u32,
    block: Option<&SummaryBlockId>,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE summary_jobs SET rejections=?1, last_submit_token=?2, last_submit_digest=?3, \
         last_submit_result=?4, block_id=COALESCE(?5, block_id) WHERE id=?6",
        params![
            rejections,
            token,
            digest,
            serde_json::to_string(outcome).map_err(|_| bad_row("unencodable outcome"))?,
            block.map(SummaryBlockId::as_str),
            job.id
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// `SummarySubmit`: validate against the rebuilt bundle and store the block.
/// Idempotent per (job_id, lease_token): the same body returns the same result;
/// a stored job returns its block without counting progress. The first
/// rejection is returned; the second stores a final ledger-only fallback block.
pub fn summary_submit(
    tx: &Transaction<'_>,
    instance: &str,
    request: &SummarySubmitRequest,
    settings: &SummarySettings,
    now: UtcMillis,
) -> Result<SubmitOutcome, ApiError> {
    let job = load_job(tx, instance, request.job_id.as_str())?;
    let caller = check_caller(tx, instance, &request.claim, &job.thread)?;
    let token = request.lease_token.as_str();
    if !holds_lease(&job, &caller.seat, token) {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "no lease token issued to this seat for the job",
        ));
    }
    if let Some(block_id) = &job.block_id {
        let block = block_by_id(tx, block_id)?;
        return Ok(SubmitOutcome::Stored {
            block_id: block.block_id,
            fallback: block.header.fallback,
        });
    }
    let digest = Sha256::digest(
        serde_json::to_vec(&request.submission)
            .map_err(|_| api_error(ErrorCode::InvalidRequest, "unencodable submission"))?,
    );
    if job.last_submit_token.as_deref() == Some(token)
        && job.last_submit_digest.as_deref() == Some(digest.as_slice())
        && let Some(result) = &job.last_submit_result
    {
        return serde_json::from_str(result).map_err(|_| bad_row("invalid stored result"));
    }
    stale_version(&job, settings)?;
    let Some(fetched_at) = job.fetched_at else {
        return Err(api_error(
            ErrorCode::Conflict,
            "summary job was not fetched",
        ));
    };
    if !job.live(now.0) {
        return Err(api_error(ErrorCode::Conflict, "summary lease expired"));
    }
    let bundle = build_bundle(tx, &job, fetched_at, settings)?;
    let at = |seq: u64| bundle.messages.iter().find(|m| m.sequence == seq);
    let verdict = validate::validate(
        &bundle,
        &request.submission,
        &|seq| {
            at(seq).is_some_and(|m| {
                m.kind == MessageKind::Ordinary && is_priority(m.author_role, m.relays_user)
            })
        },
        &|seq| at(seq).map(|m| m.text.clone()),
    );
    let children: Vec<String> = bundle
        .children
        .iter()
        .map(|c| c.block_id.as_str().to_string())
        .collect();
    let prefill = ledger::prefill(&bundle.messages);
    let found = identifiers::extract(&bundle.messages, &settings.tracker_prefixes);
    let hash = source_hash(&bundle);
    match verdict {
        Ok(parsed) => {
            let records = (job.level == 0).then(|| {
                ledger::level0_records(&prefill, &found, &parsed, &job.version, job.index)
            });
            let block_id = insert_block(
                tx,
                instance,
                &job,
                &NewBlock {
                    seat: &caller.seat,
                    narrative: &parsed.narrative,
                    fallback: false,
                    model: &parsed.model,
                    prompt_version: &parsed.prompt_version,
                    children,
                    source_hash: hash,
                    records: records.as_ref(),
                },
                now,
            )?;
            let outcome = SubmitOutcome::Stored {
                block_id: block_id.clone(),
                fallback: false,
            };
            record_submit(
                tx,
                &job,
                token,
                &digest,
                &outcome,
                job.rejections,
                Some(&block_id),
            )?;
            record_duration(
                tx,
                instance,
                &job.id,
                u64::try_from(now.0 - fetched_at).unwrap_or(0),
                now,
            )?;
            catch_up::on_progress(tx, &job.thread, now, settings)?;
            Ok(outcome)
        }
        Err(reason) if job.rejections == 0 => {
            let outcome = SubmitOutcome::Rejected {
                reasons: vec![reason],
            };
            record_submit(tx, &job, token, &digest, &outcome, 1, None)?;
            Ok(outcome)
        }
        Err(_) => {
            // Second rejection: the daemon's ledger-only fallback, final.
            let records = (job.level == 0).then(|| ledger::fallback_records(&prefill, &found));
            let block_id = insert_block(
                tx,
                instance,
                &job,
                &NewBlock {
                    seat: &caller.seat,
                    narrative: "",
                    fallback: true,
                    model: FALLBACK_MODEL,
                    prompt_version: FALLBACK_PROMPT_VERSION,
                    children,
                    source_hash: hash,
                    records: records.as_ref(),
                },
                now,
            )?;
            let outcome = SubmitOutcome::Stored {
                block_id: block_id.clone(),
                fallback: true,
            };
            record_submit(
                tx,
                &job,
                token,
                &digest,
                &outcome,
                job.rejections + 1,
                Some(&block_id),
            )?;
            catch_up::on_progress(tx, &job.thread, now, settings)?;
            Ok(outcome)
        }
    }
}

#[cfg(test)]
#[path = "../../tests/store/summary.rs"]
mod handler_tests;

#[cfg(test)]
mod loader_tests {
    use super::*;
    use crate::store::schema;

    /// Thread `t1`: human ordinary (1), relayed agent ordinary (2), info (3).
    /// Thread `t3`: ordinary (1) plus a published, unmaterialized warning (2).
    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        schema::initialize(&db, || crate::protocol::time::UtcMillis(0)).unwrap();
        db.execute_batch(
            "\
            INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,40);\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane-s',1,1,0);\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t1','i','a','g',0,0,4),('t3','i','c','g',0,0,3);\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,author_role,relays_user) VALUES ('m1','i','t1',1,'ordinary','s','hello',60000,1,'human',0);\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,author_role,relays_user) VALUES ('m2','i','t1',2,'ordinary','s','second',120000,2,'agent',1);\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq) VALUES ('m3','i','t1',3,'info','{\"b\": 1,\n \"a\": \"x\"}',180000,3);\
            INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p','i','actor','op',zeroblob(32),'t3',0,0,0,0,0,0,1,'sealed');\
            INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p','t3','s',1,300,0);\
            INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','w-unavail','s',1,1,'{\"kind\":\"unavailable\"}');\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq) VALUES ('m4','i','t3',1,'ordinary','s','b',7000,10);\
            INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p','m4','i','t3',10,7000,1,0,1,1);\
        ",
        )
        .unwrap();
        db
    }

    fn t(id: &str) -> ThreadId {
        ThreadId::new(id)
    }

    #[test]
    fn published_head_is_next_sequence_minus_one() {
        let db = db();
        assert_eq!(published_head(&db, &t("t1")).unwrap(), 3);
        assert_eq!(published_head(&db, &t("t3")).unwrap(), 2);
        assert!(published_head(&db, &t("missing")).is_err());
    }

    #[test]
    fn bundle_messages_cover_every_position() {
        let db = db();
        let all = bundle_messages(&db, &t("t1"), 1, 3).unwrap();
        assert_eq!(
            all.iter().map(|m| m.sequence).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(all[0].kind, MessageKind::Ordinary);
        assert_eq!(all[0].text, "hello");
        assert_eq!(all[0].author, Some(SeatId::new("s")));
        assert_eq!(all[0].author_role, Some(AuthorRole::Human));
        assert!(!all[0].relays_user);
        assert_eq!(all[0].created_at, UtcMillis(60_000));
        assert_eq!(all[1].author_role, Some(AuthorRole::Agent));
        assert!(all[1].relays_user);
        assert_eq!(all[2].kind, MessageKind::Info);
        assert_eq!(all[2].author, None);
        assert_eq!(all[2].text, r#"{"a":"x","b":1}"#, "compact, one line");
        // A sub-range starts at its first position.
        let tail = bundle_messages(&db, &t("t1"), 2, 3).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].sequence, 2);
        // A published warning with no materialized row.
        let pub_thread = bundle_messages(&db, &t("t3"), 1, 2).unwrap();
        assert_eq!(pub_thread.len(), 2);
        let warning = &pub_thread[1];
        assert_eq!(warning.sequence, 2);
        assert_eq!(warning.kind, MessageKind::Warn);
        assert_eq!(warning.author, None);
        assert_eq!(warning.message.as_str(), "w-unavail");
        assert_eq!(warning.text, r#"{"kind":"unavailable"}"#);
        assert_eq!(warning.created_at, UtcMillis(7000));
        assert!(bundle_messages(&db, &t("t1"), 0, 3).is_err());
    }

    #[test]
    fn system_text_is_cut_at_a_char_boundary() {
        let long = format!("{{\"k\":\"{}\"}}", "é".repeat(200));
        let text = system_text(Some(&long));
        assert!(text.len() <= SYSTEM_TEXT_MAX_BYTES);
        assert!(text.len() >= SYSTEM_TEXT_MAX_BYTES - 1);
        assert!(text.starts_with("{\"k\":\"é"));
    }

    #[test]
    fn full_chunk_detection() {
        let db = db();
        let mut settings = SummarySettings::default();
        // Default 24 KiB is far above the three tiny messages.
        assert!(!thread_has_full_chunk(&db, &t("t1"), &settings).unwrap());
        // 100 bytes: the first two messages already reach it.
        settings.chunk_bytes = 100;
        assert!(thread_has_full_chunk(&db, &t("t1"), &settings).unwrap());
        // The published warning counts toward the walk.
        settings.chunk_bytes = 60;
        assert!(thread_has_full_chunk(&db, &t("t3"), &settings).unwrap());
    }
}
