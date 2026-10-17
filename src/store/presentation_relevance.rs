//! Transaction-local exact relevance. This is not an attention/output epoch,
//! receipt, invitation acceptance, native-wake source, or presentation ledger.
//! Source ranges are bounded before judgment. Only a coherent exhausted sweep
//! proves absence; mutations during a sweep are handled at its completion.
use super::connection::{api_error, store_error};
use crate::protocol::{
    authority::CallerClaim,
    delivery::PresentationConsumer,
    ids::BindingId,
    results::{ApiError, ErrorCode},
    time::CallBudget,
};
use rusqlite::{Connection, OptionalExtension, Params, params};
use serde::{Deserialize, Serialize};

const SOURCES: usize = 13;
const ROW_LIMIT: u16 = 100;
const SEEK_LIMIT: u16 = 26;
const POINT_LIMIT: u16 = 64;
// Each of 13 slots has <=14 nonnegative i64s/bools/enum markers (<700
// serialized bytes), and one witness has <=8 i64s/markers (<400). The
// claim and canonical binding have eight <=128-byte ASCII-graphic identifiers
// (<=256 escaped bytes each), bounded enum spelling, optional process fingerprint
// and u64 generation; the outer record has <1024 bytes of names/scalars.
// Thus <13*700+400+8*256+1024=12572 bytes.
// Keep a power-of-two ceiling with headroom, enforcing fields AND total bytes.
const PROOF_BYTES: usize = 32768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Relevance {
    Open,
    Absent,
    Unknown,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct RelevanceWork {
    pub sources: u16,
    pub fallback: u16,
    pub point_probes: u16,
    pub highwater_seeks: u16,
    pub witness_probes: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RelevanceProof {
    pub relevance: Relevance,
    pub more: bool,
    pub work: RelevanceWork,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Key {
    ordinal: i64,
    row: i64,
}
// Positive authority is an exact association, never representation absence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Authority {
    DirectAssociation,
    PreparedRepresentation(i64),
}
impl Authority {
    fn representation(self) -> Option<i64> {
        match self {
            Self::DirectAssociation => None,
            Self::PreparedRepresentation(ordinal) => Some(ordinal),
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Judge {
    Representations {
        after: i64,
    },
    Membership {
        authority: Authority,
        after: i64,
        high: i64,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Slot {
    high: i64,
    after: Key,
    exhausted: bool,
    candidate: Option<Key>,
    judge: Option<Judge>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Witness {
    slot: usize,
    key: Key,
    authority: Authority,
    interval: Option<i64>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    binding_id: BindingId,
    claim: CallerClaim,
    fence: i64,
    frontier: i64,
    native_version: i64,
    slots: [Slot; SOURCES],
    source_rotation: usize,
    fallback_rotation: usize,
    absent: bool,
    witness: Option<Witness>,
}
#[derive(Clone, Copy)]
struct Source {
    table: &'static str,
    index: &'static str,
    ordinal: &'static str,
    predicate: &'static str,
    id: &'static str,
}
// ?1=seat, ?2=instance. All nonunique predicates below are fixed index
// prefixes. Source identity is frozen by physical rowid, not peer JSON.
const SOURCE: [Source; SOURCES] = [
    Source {
        table: "warning_conditions",
        index: "presentation_conditions_affected",
        ordinal: "ordinal",
        predicate: "affected_seat_id=?1 AND clear_warning_id IS NULL",
        id: "open_warning_id",
    },
    Source {
        table: "warning_conditions",
        index: "warning_conditions_invitation_origin",
        ordinal: "ordinal",
        predicate: "invitation_originator_instance_id=?2 AND invitation_originator_seat_id=?1 AND condition_kind='invitation' AND clear_warning_id IS NULL AND invitation_originator_seat_id IS NOT NULL",
        id: "open_warning_id",
    },
    Source {
        table: "receipt_warning_origins",
        index: "receipt_warning_origins_seat",
        ordinal: "ordinal",
        predicate: "instance_id=?2 AND seat_id=?1",
        id: "warning_id",
    },
    Source {
        table: "digest_open_warning_recipients",
        index: "digest_open_warning_recipients_seat",
        ordinal: "source_ordinal",
        predicate: "seat_id=?1 AND source='job'",
        id: "warning_id",
    },
    Source {
        table: "digest_open_warning_recipients",
        index: "digest_open_warning_recipients_seat",
        ordinal: "source_ordinal",
        predicate: "seat_id=?1 AND source='prepared'",
        id: "warning_id",
    },
    Source {
        table: "digest_open_warnings",
        index: "digest_open_warnings_affected",
        ordinal: "source_ordinal",
        predicate: "affected_seat_id=?1 AND source='job'",
        id: "warning_id",
    },
    Source {
        table: "digest_open_warnings",
        index: "digest_open_warnings_affected",
        ordinal: "source_ordinal",
        predicate: "affected_seat_id=?1 AND source='prepared'",
        id: "warning_id",
    },
    Source {
        table: "digest_programmatic_warnings",
        index: "digest_programmatic_warnings_seat",
        ordinal: "ordinal",
        predicate: "seat_id=?1",
        id: "warning_id",
    },
    Source {
        table: "warning_jobs",
        index: "warning_jobs_ready",
        ordinal: "ordinal",
        predicate: "status='pending'",
        id: "warning_id",
    },
    Source {
        table: "warning_jobs",
        index: "warning_jobs_ready",
        ordinal: "ordinal",
        predicate: "status='failed'",
        id: "warning_id",
    },
    Source {
        table: "work_jobs",
        index: "work_jobs_ready",
        ordinal: "ordinal",
        predicate: "status='pending' AND kind='send_attention'",
        id: "subject_id",
    },
    Source {
        table: "work_jobs",
        index: "work_jobs_ready",
        ordinal: "ordinal",
        predicate: "status='failed' AND kind='send_attention'",
        id: "subject_id",
    },
    Source {
        table: "prepared_unavailable_warnings",
        index: "prepared_unavailable_warnings_ordinal",
        ordinal: "ordinal",
        predicate: "1",
        id: "warning_id",
    },
];
fn corrupt(message: &str) -> ApiError {
    api_error(ErrorCode::StoreCorrupt, message)
}
fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b.is_ascii_graphic())
}
fn check(budget: &CallBudget) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        Err(api_error(
            ErrorCode::ReadBudgetExhausted,
            "presentation relevance cancelled",
        ))
    } else {
        Ok(())
    }
}
fn validate(state: &State) -> Result<(), ApiError> {
    if ![
        state.binding_id.as_str(),
        state.claim.instance.as_str(),
        state.claim.seat.as_str(),
        state.claim.native_session.as_str(),
        state.claim.execution.as_str(),
        state.claim.target.as_str(),
        state.claim.harness.as_str(),
    ]
    .into_iter()
    .all(id)
        || state.fence < 0
        || state.frontier < 0
        || state.native_version < 0
        || state.source_rotation >= SOURCES
        || state.fallback_rotation >= SOURCES
    {
        return Err(corrupt("invalid relevance proof scope"));
    }
    for (n, slot) in state.slots.iter().enumerate() {
        if slot.high < 0
            || slot.after.ordinal < 0
            || slot.after.ordinal > slot.high
            || slot.after.row < 0
            || slot
                .candidate
                .is_some_and(|k| k.ordinal <= 0 || k.ordinal > slot.high || k.row <= 0)
            || slot.judge.is_some() != slot.candidate.is_some()
        {
            return Err(corrupt("invalid relevance cursor"));
        }
        if let Some(Judge::Membership { authority, .. }) = slot.judge
            && !authority_valid(
                n,
                slot.candidate.unwrap_or_default(),
                authority,
                state.slots[12].high,
            )
        {
            return Err(corrupt("invalid relevance judgment authority"));
        }
        if matches!(slot.judge, Some(Judge::Representations { .. })) && !generic(n) {
            return Err(corrupt("invalid relevance representation family"));
        }
        match slot.judge {
            Some(Judge::Representations { after }) if after < 0 || after > state.slots[12].high => {
                return Err(corrupt("invalid representation cursor"));
            }
            Some(Judge::Membership {
                authority,
                after,
                high,
            }) if after < 0
                || high < after
                || authority
                    .representation()
                    .is_some_and(|r| r <= 0 || r > state.slots[12].high) =>
            {
                return Err(corrupt("invalid membership cursor"));
            }
            _ => {}
        }
    }
    if state.absent
        && (state.witness.is_some()
            || state
                .slots
                .iter()
                .any(|slot| !slot.exhausted || slot.candidate.is_some()))
    {
        return Err(corrupt("incomplete absent relevance proof"));
    }
    if let Some(w) = &state.witness
        && (w.slot >= SOURCES
            || !authority_valid(w.slot, w.key, w.authority, state.slots[12].high)
            || w.key.row <= 0
            || w.key.ordinal <= 0
            || w.key.ordinal > state.slots[w.slot].high
            || w.authority
                .representation()
                .is_some_and(|n| n <= 0 || n > state.slots[12].high)
            || w.interval.is_some_and(|n| n <= 0))
    {
        return Err(corrupt("invalid relevance witness"));
    }
    Ok(())
}
fn load(
    db: &Connection,
    claim: &CallerClaim,
    binding: &BindingId,
    consumer: PresentationConsumer,
) -> Result<Option<State>, ApiError> {
    let raw: Option<Result<String, ApiError>> = db
        .query_row(
            "SELECT proof_json FROM presentation_warning_proofs WHERE binding_id=?1 AND consumer=?2 AND seat_id=?3",
            params![binding.as_str(), consumer.as_str(), claim.seat.as_str()],
            |r| {
                Ok(match r.get_ref(0)? {
                    rusqlite::types::ValueRef::Text(bytes) if bytes.len() <= PROOF_BYTES => {
                        std::str::from_utf8(bytes)
                            .map(str::to_owned)
                            .map_err(|_| corrupt("invalid relevance proof UTF8"))
                    }
                    _ => Err(corrupt("relevance proof type/byte bound")),
                })
            },
        )
        .optional()
        .map_err(store_error)?;
    raw.map(|raw| {
        let raw = raw?;
        if raw.len() > PROOF_BYTES {
            return Err(corrupt("relevance proof byte bound"));
        }
        let state: State =
            serde_json::from_str(&raw).map_err(|_| corrupt("invalid relevance proof JSON"))?;
        validate(&state)?;
        Ok(state)
    })
    .transpose()
}
fn save(db: &Connection, consumer: PresentationConsumer, state: &State) -> Result<(), ApiError> {
    validate(state)?;
    let raw = serde_json::to_string(state).map_err(|_| corrupt("relevance proof encoding"))?;
    if raw.len() > PROOF_BYTES {
        return Err(corrupt("relevance proof byte bound"));
    }
    db.execute("INSERT INTO presentation_warning_proofs(seat_id,consumer,proof_json,binding_id) VALUES(?1,?2,?3,?4) ON CONFLICT(binding_id,consumer) DO UPDATE SET proof_json=excluded.proof_json",params![state.claim.seat.as_str(),consumer.as_str(),raw,state.binding_id.as_str()]).map_err(store_error)?;
    Ok(())
}
fn fence(db: &Connection) -> Result<i64, ApiError> {
    let n: i64 = db
        .query_row(
            "SELECT version FROM presentation_warning_fence WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if n < 0 {
        Err(corrupt("negative relevance fence"))
    } else {
        Ok(n)
    }
}
fn confirming(db: &Connection, claim: &CallerClaim) -> Result<bool, ApiError> {
    db.query_row("SELECT EXISTS(SELECT 1 FROM presentation_confirming_invitations INDEXED BY presentation_confirming_invitations_seat WHERE seat_id=?1)",[claim.seat.as_str()],|r|r.get(0)).map_err(store_error)
}
fn snapshot(
    db: &Connection,
    claim: &CallerClaim,
    binding: &BindingId,
    frontier: i64,
    work: &mut RelevanceWork,
) -> Result<State, ApiError> {
    let mut state = State {
        claim: claim.clone(),
        binding_id: binding.clone(),
        fence: fence(db)?,
        frontier,
        native_version: db
            .query_row(
                "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id=?1),0)",
                [claim.seat.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
        slots: std::array::from_fn(|_| Slot::default()),
        source_rotation: 0,
        fallback_rotation: 0,
        absent: false,
        witness: None,
    };
    for (n, source) in SOURCE.iter().enumerate() {
        work.highwater_seeks += 1;
        let sql = format!(
            "SELECT COALESCE(MAX({}),0) FROM {} INDEXED BY {} WHERE {} AND (?1 IS NOT NULL AND ?2 IS NOT NULL)",
            source.ordinal, source.table, source.index, source.predicate
        );
        let high: i64 = db
            .query_row(&sql, params![claim.seat.as_str(), claim.instance], |r| {
                r.get(0)
            })
            .map_err(store_error)?;
        state.slots[n].high = high;
        if n == 7 {
            state.slots[n].after.ordinal = frontier.min(high);
            state.slots[n].after.row = i64::MAX;
        }
        state.slots[n].exhausted = state.slots[n].after.ordinal >= high;
    }
    validate(&state)?;
    Ok(state)
}
struct Points<'a> {
    db: &'a Connection,
    count: u16,
}
impl Points<'_> {
    // Cost includes all PK/UNIQUE joined/result probes, including absent ones.
    fn row<T>(
        &mut self,
        cost: u16,
        sql: &str,
        p: impl Params,
        map: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>, ApiError> {
        self.count = self
            .count
            .checked_add(cost)
            .filter(|n| *n <= POINT_LIMIT)
            .ok_or_else(|| corrupt("relevance keyed probe bound"))?;
        self.db
            .query_row(sql, p, map)
            .optional()
            .map_err(store_error)
    }
    fn yes(&mut self, cost: u16, sql: &str, p: impl Params) -> Result<bool, ApiError> {
        Ok(self
            .row(cost, sql, p, |r| r.get::<_, bool>(0))?
            .unwrap_or(false))
    }
}
#[derive(Debug)]
struct Warning {
    id: String,
    instance: String,
    thread: String,
    event_seq: i64,
    source_message: Option<String>,
    affected: Option<String>,
    interval_high: Option<i64>,
    episode: Option<i64>,
    programmatic: bool,
}
fn source_id(
    points: &mut Points<'_>,
    claim: &CallerClaim,
    slot: usize,
    key: Key,
) -> Result<Option<String>, ApiError> {
    let s = SOURCE[slot];
    let sql = format!(
        "SELECT {} FROM {} WHERE rowid=?3 AND {} AND {}=?4 AND (?1 IS NOT NULL AND ?2 IS NOT NULL)",
        s.id, s.table, s.predicate, s.ordinal
    );
    let value: Option<String> = points.row(
        1,
        &sql,
        params![claim.seat.as_str(), claim.instance, key.row, key.ordinal],
        |r| r.get(0),
    )?;
    if value
        .as_deref()
        .is_some_and(|s| !(id(s) || slot == 10 || slot == 11))
    {
        return Err(corrupt("invalid relevance warning identity"));
    }
    Ok(value)
}
fn direct(points: &mut Points<'_>, warning: &str) -> Result<Option<Warning>, ApiError> {
    points.row(2,"SELECT m.id,m.thread_id,m.decision_seq,m.source_message_id,j.affected_seat_id,j.interval_high_water,m.author_kind,m.instance_id FROM messages m LEFT JOIN warning_jobs j ON j.warning_id=m.id AND j.thread_id=m.thread_id AND j.event_seq=m.decision_seq WHERE m.id=?1 AND m.kind='warn'",[warning],|r|Ok(Warning{id:r.get(0)?,thread:r.get(1)?,event_seq:r.get(2)?,source_message:r.get(3)?,affected:r.get(4)?,interval_high:r.get(5)?,episode:None,programmatic:r.get::<_,Option<String>>(6)?.as_deref()==Some("programmatic"),instance:r.get(7)?}))
}
fn prepared(
    points: &mut Points<'_>,
    warning: &str,
    ordinal: i64,
) -> Result<Option<Warning>, ApiError> {
    points.row(2,"SELECT w.warning_id,sm.thread_id,sm.decision_seq,sm.message_id,w.affected_seat_id,sm.interval_high_water,w.unavailability_episode,sm.instance_id FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.ordinal=?1 AND w.warning_id=?2",params![ordinal,warning],|r|Ok(Warning{id:r.get(0)?,thread:r.get(1)?,event_seq:r.get(2)?,source_message:Some(r.get(3)?),affected:Some(r.get(4)?),interval_high:Some(r.get(5)?),episode:Some(r.get(6)?),programmatic:false,instance:r.get(7)?}))
}
fn native_rejection(points: &mut Points<'_>, warning: &str) -> Result<bool, ApiError> {
    points.yes(3,"SELECT EXISTS(SELECT 1 FROM messages m JOIN invitation_rejections r ON r.invitation_id=m.source_invitation_id JOIN invitations i ON i.id=r.invitation_id WHERE m.id=?1 AND m.kind='warn' AND m.author_kind='native' AND m.actor_seat_id=r.actor_seat_id AND m.author_service_id IS NULL AND m.thread_id=i.thread_id AND m.event_key='reject:'||r.invitation_id AND m.source_message_id IS NULL AND m.decision_at=r.rejected_at AND json_extract(m.event_json,'$.action')='reject' AND json_extract(m.event_json,'$.seat')=r.actor_seat_id AND json_extract(m.event_json,'$.invitation')=r.invitation_id AND json_extract(m.event_json,'$.reason')=r.reason)",[warning])
}
// State-only receipt actionability avoids effective_receipt's timer/availability
// range; no deadline value is used by this predicate. Logical precedence,
// terminal-state precedence, human bindings and historical waiver are exact.
fn receipt_pending(points: &mut Points<'_>, message: &str, seat: &str) -> Result<bool, ApiError> {
    type Receipt = (String, bool, bool, Option<String>, i64, Option<String>);
    let logical:Option<Receipt>=points.row(4,"SELECT COALESCE(rs.state,'pending'),s.retired_at IS NOT NULL,MIN(pr.ack_required,COALESCE(rs.ack_required,1))=0,pr.availability_provenance,sm.decision_seq,NULL FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=?2 JOIN seats s ON s.id=pr.seat_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE sm.message_id=?1",params![message,seat],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let receipt=match logical {
        Some(r)=>Some(r),
        None=>points.row(3,"SELECT r.state,s.retired_at IS NOT NULL,r.ack_required=0,NULL,COALESCE(m.decision_seq,0),NULL FROM receipts r JOIN seats s ON s.id=r.seat_id JOIN messages m ON m.id=r.message_id WHERE r.message_id=?1 AND r.seat_id=?2",params![message,seat],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?,
    };
    let Some((state, retired, not_required, provenance, seq, _)) = receipt else {
        return Ok(false);
    };
    match state.as_str() {
        "acked" | "recipient_retired" => return Ok(false),
        "pending" => {}
        _ => return Err(corrupt("unknown relevance receipt state")),
    }
    if retired || not_required || provenance.as_deref() == Some("operator_human") {
        return Ok(false);
    }
    let human=points.yes(1,"SELECT EXISTS(SELECT 1 FROM occupant_bindings INDEXED BY occupant_bindings_session_key WHERE seat_id=?1 AND ended_at IS NULL AND harness='human')",[seat])?;
    let waiver: Option<i64> = points.row(
        1,
        "SELECT through_decision_seq FROM human_receipt_waivers WHERE seat_id=?1",
        [seat],
        |r| r.get(0),
    )?;
    Ok(!human && waiver.is_none_or(|through| seq > through))
}
fn actionable(points: &mut Points<'_>, warning: &Warning) -> Result<bool, ApiError> {
    if warning.programmatic
        && points.yes(
            1,
            "SELECT EXISTS(SELECT 1 FROM service_notification_publications WHERE message_id=?1)",
            [warning.id.as_str()],
        )?
    {
        return Ok(true);
    }
    if let Some(episode) = warning.episode {
        let affected = warning
            .affected
            .as_deref()
            .ok_or_else(|| corrupt("prepared warning affected seat missing"))?;
        return points.yes(1,"SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND state!='retired' AND unavailability_open=1 AND unavailability_episode=?2)",params![affected,episode]);
    }
    let job: Option<(String, String, Option<String>)> = points.row(
        1,
        "SELECT condition_kind,condition_id,affected_seat_id FROM warning_jobs WHERE warning_id=?1",
        [warning.id.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let Some((kind, condition, affected)) = job else {
        return Ok(false);
    };
    match kind.as_str() {
        "invitation"=>points.yes(4,"SELECT EXISTS(SELECT 1 FROM invitations i JOIN seats s ON s.id=i.seat_id WHERE i.id=?1 AND i.state='pending' AND s.state!='retired' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=i.id))",[condition]),
        "receipt"=>receipt_pending(points,warning.source_message.as_deref().ok_or_else(||corrupt("receipt warning source missing"))?,affected.as_deref().ok_or_else(||corrupt("receipt warning affected missing"))?),
        "unavailable"=>Err(corrupt("unavailable warning lacks published episode")),
        _=>Err(corrupt("unknown relevance condition")),
    }
}
#[derive(Clone, Copy)]
enum Recipient {
    Yes,
    No,
    Intervals { high: i64, cutoff: i64 },
}
fn recipient(
    points: &mut Points<'_>,
    claim: &CallerClaim,
    warning: &Warning,
) -> Result<Recipient, ApiError> {
    if points.yes(2,"SELECT EXISTS(SELECT 1 FROM service_notification_publications p JOIN service_notification_recipients r ON r.preparation_id=p.preparation_id AND r.seat_id=?2 WHERE p.message_id=?1)",params![warning.id,claim.seat.as_str()])?
        || points.yes(1,"SELECT EXISTS(SELECT 1 FROM warning_recipients WHERE warning_id=?1 AND seat_id=?2)",params![warning.id,claim.seat.as_str()])? {return Ok(Recipient::Yes);}
    let Some(high) = warning.interval_high else {
        return Ok(Recipient::No);
    };
    let cutoff: Option<Option<i64>> = points.row(
        1,
        "SELECT recipient_cutoff_seq FROM warning_jobs WHERE warning_id=?1",
        [warning.id.as_str()],
        |r| r.get(0),
    )?;
    let cutoff = cutoff.flatten().unwrap_or(warning.event_seq);
    let live = points.yes(
        1,
        "SELECT retired_seq IS NULL OR retired_seq>?2 FROM seats WHERE id=?1",
        params![claim.seat.as_str(), cutoff],
    )?;
    if !live {
        return Ok(Recipient::No);
    }
    if warning.affected.as_deref() == Some(claim.seat.as_str()) {
        Ok(Recipient::Yes)
    } else {
        Ok(Recipient::Intervals { high, cutoff })
    }
}
// This applies the independent native eligibility rule before generic legacy
// actionability. Transitions/receipt origins never depend on notice frontier.
fn eligible(
    points: &mut Points<'_>,
    claim: &CallerClaim,
    warning: &Warning,
    work: &mut RelevanceWork,
) -> Result<Option<bool>, ApiError> {
    if native_rejection(points, &warning.id)? {
        return Ok(Some(false));
    }
    type Condition = (
        i64,
        String,
        String,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let open: Option<Condition> = points.row(1,
        "SELECT ordinal,condition_kind,affected_seat_id,episode,clear_warning_id,invitation_originator_seat_id FROM warning_conditions WHERE open_warning_id=?1",
        [warning.id.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    if let Some((ordinal, kind, affected, episode, clear, origin)) = open {
        if ordinal <= 0
            || !id(&affected)
            || !matches!(kind.as_str(), "invitation" | "receipt" | "unavailable")
            || ((kind == "unavailable") != episode.is_some())
            || episode.is_some_and(|n| n < 0)
        {
            return Err(corrupt("invalid relevance condition scope"));
        }
        if clear.is_some() {
            return Ok(Some(false));
        }
        if kind != "invitation" {
            // The sole canonical producer appends retained adjacent scope ranges
            // after=prior MAX through. One indexed successor is complete for
            // that chain. LIMIT precedes the after check; no filtered EXISTS.
            if !room(work, 1, 1) {
                return Err(corrupt("unreserved relevance coverage"));
            }
            work.highwater_seeks += 1;
            let sweep: Option<(i64,i64)> = points.db.query_row(
                "SELECT after_ordinal,through_ordinal FROM warning_close_sweeps INDEXED BY warning_close_sweeps_cover WHERE condition_kind=?1 AND affected_seat_id=?2 AND episode IS ?3 AND through_ordinal>=?4 ORDER BY through_ordinal,after_ordinal LIMIT 1",
                params![kind,affected,episode,ordinal], |r| Ok((r.get(0)?,r.get(1)?)))
                .optional().map_err(store_error)?;
            if let Some((after, through)) = sweep {
                work.fallback += 1;
                if after < 0 || through <= after {
                    return Err(corrupt("invalid relevance close sweep"));
                }
                if after < ordinal && ordinal <= through {
                    return Ok(Some(false));
                }
            }
        }
        let receipt_origin = points.yes(1,
            "SELECT EXISTS(SELECT 1 FROM receipt_warning_origins INDEXED BY receipt_warning_origins_exact WHERE instance_id=?1 AND seat_id=?2 AND warning_id=?3)",
            params![claim.instance,claim.seat.as_str(),warning.id])?;
        return Ok(Some(
            affected == claim.seat.as_str()
                || origin.as_deref() == Some(claim.seat.as_str())
                || receipt_origin,
        ));
    }
    if points.yes(
        1,
        "SELECT EXISTS(SELECT 1 FROM warning_conditions WHERE clear_warning_id=?1)",
        [warning.id.as_str()],
    )? {
        return Ok(Some(false));
    }
    Ok(None)
}
#[derive(Clone, Copy)]
enum Evaluation {
    Closed,
    Open,
    Membership { high: i64, cutoff: i64 },
}
fn evaluate(
    points: &mut Points<'_>,
    claim: &CallerClaim,
    slot: usize,
    key: Key,
    authority: Authority,
    interval: Option<i64>,
    work: &mut RelevanceWork,
) -> Result<Evaluation, ApiError> {
    let Some(mut warning_id) = source_id(points, claim, slot, key)? else {
        return Ok(Evaluation::Closed);
    };
    if slot == 10 || slot == 11 {
        let Some(preparation) = warning_id.strip_prefix("service-notify:") else {
            return Ok(Evaluation::Closed);
        };
        if !id(preparation) {
            return Err(corrupt("invalid notification preparation identity"));
        }
        let publication:Option<String>=points.row(2,"SELECT p.message_id FROM service_notification_publications p JOIN service_notification_recipients r ON r.preparation_id=p.preparation_id AND r.seat_id=?2 WHERE p.preparation_id=?1",params![preparation,claim.seat.as_str()],|r|r.get(0))?;
        let Some(publication) = publication else {
            return Ok(Evaluation::Closed);
        };
        if points.yes(
            1,
            "SELECT EXISTS(SELECT 1 FROM warning_recipients WHERE warning_id=?1 AND seat_id=?2)",
            params![publication, claim.seat.as_str()],
        )? {
            return Ok(Evaluation::Closed);
        }
        warning_id = publication;
    }
    if (slot == 3 || slot == 5)
        && !points.yes(
            1,
            "SELECT EXISTS(SELECT 1 FROM warning_jobs WHERE ordinal=?1 AND warning_id=?2)",
            params![key.ordinal, warning_id],
        )?
    {
        return Ok(Evaluation::Closed);
    }
    if slot == 12 {
        if points.yes(
            1,
            "SELECT EXISTS(SELECT 1 FROM warning_jobs WHERE warning_id=?1)",
            [warning_id.as_str()],
        )? {
            return Ok(Evaluation::Closed);
        }
        let active=points.yes(2,"SELECT EXISTS(SELECT 1 FROM prepared_unavailable_warnings w JOIN work_jobs j ON j.kind='send_attention' AND j.subject_id=w.preparation_id WHERE w.ordinal=?1 AND j.status IN ('pending','failed'))",[key.ordinal])?;
        if !active {
            return Ok(Evaluation::Closed);
        }
    }
    let warning = match authority.representation() {
        Some(ordinal) => prepared(points, &warning_id, ordinal)?,
        None => direct(points, &warning_id)?,
    };
    let Some(warning) = warning else {
        return Ok(Evaluation::Closed);
    };
    if warning.instance != claim.instance {
        return Ok(Evaluation::Closed);
    }
    if !id(&warning.id)
        || !id(&warning.instance)
        || !id(&warning.thread)
        || warning.event_seq <= 0
        || warning.source_message.as_deref().is_some_and(|s| !id(s))
        || warning.affected.as_deref().is_some_and(|s| !id(s))
        || warning.interval_high.is_some_and(|n| n < 0)
    {
        return Err(corrupt("invalid relevance warning fields"));
    }
    if let Some(relevant) = eligible(points, claim, &warning, work)? {
        return Ok(if relevant {
            Evaluation::Open
        } else {
            Evaluation::Closed
        });
    }
    // Origin source rows must still have their canonical open association.
    if slot <= 2 {
        return Ok(Evaluation::Closed);
    }
    if (slot == 10 || slot == 11) && !warning.programmatic {
        return Ok(Evaluation::Closed);
    }
    if matches!(authority, Authority::DirectAssociation) && !warning.programmatic {
        let associated = points.yes(3,
            "SELECT EXISTS(SELECT 1 FROM warning_jobs j JOIN messages m ON m.id=j.warning_id LEFT JOIN invitations i ON i.id=j.condition_id WHERE j.warning_id=?1 AND j.thread_id=m.thread_id AND j.event_seq=m.decision_seq AND ((j.condition_kind='invitation' AND i.thread_id=m.thread_id AND i.seat_id=j.affected_seat_id AND (m.source_invitation_id IS NULL OR m.source_invitation_id=i.id)) OR (j.condition_kind='receipt' AND m.source_message_id IS NOT NULL)))",
            [warning.id.as_str()])?;
        if !associated {
            return Ok(Evaluation::Closed);
        }
    }
    if !actionable(points, &warning)? {
        return Ok(Evaluation::Closed);
    }
    match recipient(points, claim, &warning)? {
        Recipient::Yes => Ok(Evaluation::Open),
        Recipient::No => Ok(Evaluation::Closed),
        Recipient::Intervals { high, cutoff } => {
            if let Some(ordinal) = interval {
                let hit=points.yes(1,"SELECT EXISTS(SELECT 1 FROM membership_intervals WHERE ordinal=?1 AND thread_id=?2 AND seat_id=?3 AND ordinal<=?4 AND joined_seq<=?5 AND (left_seq IS NULL OR left_seq>?5))",params![ordinal,warning.thread,claim.seat.as_str(),high,cutoff])?;
                Ok(if hit {
                    Evaluation::Open
                } else {
                    Evaluation::Closed
                })
            } else {
                Ok(Evaluation::Membership { high, cutoff })
            }
        }
    }
}
fn charge_points(work: &mut RelevanceWork, points: &Points<'_>, witness: bool) {
    if witness {
        work.witness_probes += points.count;
    } else {
        work.point_probes += points.count;
    }
}
fn generic(n: usize) -> bool {
    matches!(n, 3 | 5 | 8 | 9)
}
fn authority_valid(n: usize, key: Key, authority: Authority, high: i64) -> bool {
    match authority {
        Authority::DirectAssociation => !matches!(n, 4 | 6 | 12),
        Authority::PreparedRepresentation(ordinal) => {
            ordinal > 0
                && ordinal <= high
                && (generic(n) || (matches!(n, 4 | 6 | 12) && ordinal == key.ordinal))
        }
    }
}
fn initial_authority(n: usize, key: Key) -> Authority {
    if matches!(n, 4 | 6 | 12) {
        Authority::PreparedRepresentation(key.ordinal)
    } else {
        Authority::DirectAssociation
    }
}
fn first_judgment(
    db: &Connection,
    claim: &CallerClaim,
    n: usize,
    key: Key,
    work: &mut RelevanceWork,
) -> Result<(Option<Judge>, Option<Witness>), ApiError> {
    let authority = initial_authority(n, key);
    let mut points = Points { db, count: 0 };
    let result = evaluate(&mut points, claim, n, key, authority, None, work)?;
    charge_points(work, &points, false);
    Ok(match result {
        Evaluation::Closed if generic(n) => (Some(Judge::Representations { after: 0 }), None),
        Evaluation::Closed => (None, None),
        Evaluation::Open => (
            None,
            Some(Witness {
                slot: n,
                key,
                authority,
                interval: None,
            }),
        ),
        Evaluation::Membership { high, .. } => (
            Some(Judge::Membership {
                authority,
                after: 0,
                high,
            }),
            None,
        ),
    })
}
fn room(work: &RelevanceWork, seeks: u16, rows: u16) -> bool {
    work.highwater_seeks + seeks <= SEEK_LIMIT && work.sources + work.fallback + rows <= ROW_LIMIT
}
// Only called after reserving the complete <=2 seeks/2 physical rows step.
fn source_step(
    db: &Connection,
    state: &mut State,
    n: usize,
    work: &mut RelevanceWork,
) -> Result<(), ApiError> {
    let s = SOURCE[n];
    let slot = &state.slots[n];
    let sql = format!(
        "SELECT {},rowid FROM {} INDEXED BY {} WHERE {} AND ({},rowid)>(?3,?4) AND {}<=?5 AND (?1 IS NOT NULL AND ?2 IS NOT NULL) ORDER BY {},rowid LIMIT 1",
        s.ordinal, s.table, s.index, s.predicate, s.ordinal, s.ordinal, s.ordinal
    );
    work.highwater_seeks += 1;
    let key: Option<Key> = db
        .query_row(
            &sql,
            params![
                state.claim.seat.as_str(),
                state.claim.instance,
                slot.after.ordinal,
                slot.after.row,
                slot.high
            ],
            |r| {
                Ok(Key {
                    ordinal: r.get(0)?,
                    row: r.get(1)?,
                })
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some(key) = key else {
        state.slots[n].exhausted = true;
        return Ok(());
    };
    work.sources += 1;
    let (judge, witness) = first_judgment(db, &state.claim, n, key, work)?;
    state.slots[n].after = key;
    state.slots[n].candidate = judge.map(|_| key);
    state.slots[n].judge = judge;
    if witness.is_some() {
        state.witness = witness;
    }
    Ok(())
}
fn finish_negative(slot: &mut Slot, n: usize, authority: Authority) {
    if generic(n) {
        slot.judge = Some(Judge::Representations {
            after: authority.representation().unwrap_or(0),
        });
    } else {
        slot.candidate = None;
        slot.judge = None;
    }
}
fn accept_evaluation(
    state: &mut State,
    n: usize,
    key: Key,
    authority: Authority,
    result: Evaluation,
) {
    match result {
        Evaluation::Open => {
            state.witness = Some(Witness {
                slot: n,
                key,
                authority,
                interval: None,
            });
            state.slots[n].candidate = None;
            state.slots[n].judge = None;
        }
        Evaluation::Closed => finish_negative(&mut state.slots[n], n, authority),
        Evaluation::Membership { high, .. } => {
            state.slots[n].judge = Some(Judge::Membership {
                authority,
                after: 0,
                high,
            });
        }
    }
}
// Only called after reserving the complete <=3 seeks/3 physical rows step.
// At most one physical representation/interval is examined, before judgment.
fn fallback_step(
    db: &Connection,
    state: &mut State,
    n: usize,
    work: &mut RelevanceWork,
) -> Result<(), ApiError> {
    let key = state.slots[n]
        .candidate
        .ok_or_else(|| corrupt("missing relevance candidate"))?;
    let judge = state.slots[n]
        .judge
        .ok_or_else(|| corrupt("missing suspended judgment"))?;
    let mut points = Points { db, count: 0 };
    let Some(warning_id) = source_id(&mut points, &state.claim, n, key)? else {
        charge_points(work, &points, false);
        state.slots[n].candidate = None;
        state.slots[n].judge = None;
        return Ok(());
    };
    match judge {
        Judge::Representations { after } => {
            work.highwater_seeks += 1;
            let ordinal:Option<i64> = db.query_row(
                "SELECT ordinal FROM prepared_unavailable_warnings INDEXED BY presentation_prepared_warning WHERE warning_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
                params![warning_id,after,state.slots[12].high],|r|r.get(0)).optional().map_err(store_error)?;
            if let Some(ordinal) = ordinal {
                work.fallback += 1;
                let authority = Authority::PreparedRepresentation(ordinal);
                let current = evaluate(&mut points, &state.claim, n, key, authority, None, work)?;
                accept_evaluation(state, n, key, authority, current);
            } else {
                // The independent direct association was already examined.
                // Exhausted prepared alternatives never erase its positive truth.
                state.slots[n].candidate = None;
                state.slots[n].judge = None;
            }
        }
        Judge::Membership {
            authority,
            after,
            high,
        } => {
            let evaluation = evaluate(&mut points, &state.claim, n, key, authority, None, work)?;
            let (cutoff, current_high) = match evaluation {
                Evaluation::Membership { cutoff, high } => (cutoff, high),
                other => {
                    accept_evaluation(state, n, key, authority, other);
                    charge_points(work, &points, false);
                    return Ok(());
                }
            };
            // Keep the captured finite frontier, but current source authority
            // can narrow it after retained/operator repair. Never first accept
            // an interval that the exact saved-witness judge would reject.
            let effective_high = high.min(current_high);
            if after > effective_high {
                finish_negative(&mut state.slots[n], n, authority);
                charge_points(work, &points, false);
                return Ok(());
            }
            let warning = match authority.representation() {
                Some(ordinal) => prepared(&mut points, &warning_id, ordinal)?,
                None => direct(&mut points, &warning_id)?,
            }
            .ok_or_else(|| corrupt("judged warning disappeared in transaction"))?;
            work.highwater_seeks += 1;
            let row:Option<(i64,i64,Option<i64>)>=db.query_row(
                "SELECT ordinal,joined_seq,left_seq FROM membership_intervals INDEXED BY presentation_membership_history WHERE thread_id=?1 AND seat_id=?2 AND ordinal>?3 AND ordinal<=?4 ORDER BY ordinal LIMIT 1",
                params![warning.thread,state.claim.seat.as_str(),after,effective_high],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
            match row {
                None => finish_negative(&mut state.slots[n], n, authority),
                Some((ordinal, joined, left)) => {
                    work.fallback += 1;
                    // All source/condition inputs were checked in this same
                    // transaction. The returned physical row itself proves
                    // this interval; no second judge or coverage seek needed.
                    if joined <= cutoff && left.is_none_or(|end| end > cutoff) {
                        state.witness = Some(Witness {
                            slot: n,
                            key,
                            authority,
                            interval: Some(ordinal),
                        });
                        state.slots[n].candidate = None;
                        state.slots[n].judge = None;
                    } else {
                        state.slots[n].judge = Some(Judge::Membership {
                            authority,
                            after: ordinal,
                            high,
                        });
                    }
                }
            }
        }
    }
    charge_points(work, &points, false);
    Ok(())
}
fn witness(
    db: &Connection,
    state: &State,
    frontier: i64,
    work: &mut RelevanceWork,
) -> Result<bool, ApiError> {
    let Some(w) = &state.witness else {
        return Ok(false);
    };
    if w.slot == 7 && w.key.ordinal <= frontier {
        return Ok(false);
    }
    if !room(work, 1, 1) {
        return Ok(false);
    }
    let mut points = Points { db, count: 0 };
    let current = evaluate(
        &mut points,
        &state.claim,
        w.slot,
        w.key,
        w.authority,
        w.interval,
        work,
    )?;
    charge_points(work, &points, true);
    Ok(matches!(current, Evaluation::Open))
}
fn next_slot(state: &State, fallback: bool) -> Option<usize> {
    let start = if fallback {
        state.fallback_rotation
    } else {
        state.source_rotation
    };
    (0..SOURCES)
        .map(|offset| (start + offset) % SOURCES)
        .find(|&n| {
            let slot = &state.slots[n];
            if fallback {
                slot.candidate.is_some()
            } else {
                !slot.exhausted && slot.candidate.is_none()
            }
        })
}
fn microstep(
    db: &Connection,
    state: &mut State,
    fallback: bool,
    n: usize,
    work: &mut RelevanceWork,
) -> Result<u16, ApiError> {
    let before = work.sources + work.fallback;
    if fallback {
        fallback_step(db, state, n, work)?;
        state.fallback_rotation = (n + 1) % SOURCES;
    } else {
        source_step(db, state, n, work)?;
        state.source_rotation = (n + 1) % SOURCES;
    }
    Ok(work.sources + work.fallback - before)
}
// Full-cost reservations protect BOTH families before lending. Shares count
// all physical rows of each action (including coverage), not just source rows.
fn schedule(
    db: &Connection,
    state: &mut State,
    work: &mut RelevanceWork,
    budget: &CallBudget,
) -> Result<(), ApiError> {
    let mut lane_rows = [0u16; 2];
    let source = next_slot(state, false);
    let fallback = next_slot(state, true);
    // Reserve fallback even if the source step may create the first candidate.
    let reserve_fallback = fallback.is_some() || source.is_some();
    let reserved_seeks =
        if source.is_some() { 2 } else { 0 } + if reserve_fallback { 3 } else { 0 };
    let reserved_rows = reserved_seeks;
    if !room(work, reserved_seeks, reserved_rows) {
        return Ok(());
    }
    if let Some(n) = source {
        check(budget)?;
        lane_rows[0] += microstep(db, state, false, n, work)?;
        if state.witness.is_some() {
            return Ok(());
        }
    }
    if let Some(n) = fallback.or_else(|| next_slot(state, true)) {
        check(budget)?;
        lane_rows[1] += microstep(db, state, true, n, work)?;
        if state.witness.is_some() {
            return Ok(());
        }
    }
    // Alternating reserved passes, then alternating lending. A zero-row empty
    // seek still consumes a range; a keyed-only negative clears its candidate.
    for lending in [false, true] {
        loop {
            let mut progressed = false;
            for (family, lane) in lane_rows.iter_mut().enumerate() {
                let fallback = family == 1;
                let cost = if fallback { 3 } else { 2 };
                if !lending && *lane + cost > 50 {
                    continue;
                }
                let Some(n) = next_slot(state, fallback) else {
                    continue;
                };
                if !room(work, cost, cost) {
                    continue;
                }
                check(budget)?;
                *lane += microstep(db, state, fallback, n, work)?;
                progressed = true;
                if state.witness.is_some() {
                    return Ok(());
                }
            }
            if !progressed {
                break;
            }
        }
    }
    Ok(())
}
fn result(relevance: Relevance, work: RelevanceWork) -> RelevanceProof {
    RelevanceProof {
        relevance,
        more: relevance == Relevance::Unknown,
        work,
    }
}
/// The caller holds its deciding transaction and owns complete A2 admission.
/// Every successful V2 pull advances exactly once, including replay/control/no-output.
/// Carrying notices may then invalidate its witness; the caller revalidates that
/// cached witness before appending Attention without a second discovery sweep.
pub(super) fn advance(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
    frontier: i64,
    budget: &CallBudget,
) -> Result<RelevanceProof, ApiError> {
    check(budget)?;
    if db.is_autocommit() {
        return Err(corrupt("relevance requires caller transaction"));
    }
    if frontier < 0 {
        return Err(corrupt("negative relevance notice frontier"));
    }
    let mut work = RelevanceWork::default();
    let Some(binding) = super::invitation_content::binding_scope(db, claim)? else {
        return Ok(result(Relevance::Unknown, work));
    };
    if confirming(db, claim)? {
        return Ok(result(Relevance::Open, work));
    }
    let now = fence(db)?;
    let prior = load(db, claim, &binding, consumer)?;
    let mut state = match prior {
        Some(state) if state.claim == *claim && state.binding_id == binding => state,
        _ => snapshot(db, claim, &binding, frontier, &mut work)?,
    };
    if witness(db, &state, frontier, &mut work)? {
        return Ok(result(Relevance::Open, work));
    }
    state.witness = None;
    if state.absent && state.fence == now && state.frontier == frontier {
        return Ok(result(Relevance::Absent, work));
    }
    if state.absent {
        state = snapshot(db, claim, &binding, frontier, &mut work)?;
    }
    // An advancing notice frontier cannot leave an already-settled notice as
    // a positive witness while this older captured sweep finishes.
    if frontier >= state.slots[7].after.ordinal {
        state.slots[7].after.row = i64::MAX;
    }
    state.slots[7].after.ordinal = state.slots[7]
        .after
        .ordinal
        .max(frontier.min(state.slots[7].high));
    if state.slots[7]
        .candidate
        .is_some_and(|key| key.ordinal <= frontier)
    {
        state.slots[7].candidate = None;
        state.slots[7].judge = None;
    }
    schedule(db, &mut state, &mut work, budget)?;
    let relevance = if state.witness.is_some() {
        Relevance::Open
    } else if state
        .slots
        .iter()
        .all(|slot| slot.exhausted && slot.candidate.is_none())
    {
        if state.fence == now && state.frontier == frontier {
            state.absent = true;
            Relevance::Absent
        } else {
            // Finish the captured finite sweep before recapture. A full
            // recapture consumes13 seeks; yield if this call lacks room.
            if work.highwater_seeks + SOURCES as u16 <= SEEK_LIMIT {
                state = snapshot(db, claim, &binding, frontier, &mut work)?;
            }
            Relevance::Unknown
        }
    } else {
        Relevance::Unknown
    };
    if work.sources + work.fallback > ROW_LIMIT
        || work.highwater_seeks > SEEK_LIMIT
        || work.point_probes > ROW_LIMIT * POINT_LIMIT
        || work.witness_probes > POINT_LIMIT
    {
        return Err(corrupt("relevance total work bound"));
    }
    save(db, consumer, &state)?;
    Ok(result(relevance, work))
}
/// Cached witness inspection, never an ExplicitConfirm authority.
/// At most one indexed coverage successor and 64 witness probes; no source
/// discovery or fallback walk. A V2 pull may use this after notice settlement,
/// in addition to its one advance. ExplicitConfirm never calls this.
pub(super) fn revalidate(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
    frontier: i64,
    budget: &CallBudget,
) -> Result<RelevanceProof, ApiError> {
    check(budget)?;
    if db.is_autocommit() || frontier < 0 {
        return Err(corrupt("relevance requires transaction and valid frontier"));
    }
    let now = fence(db)?;
    let mut work = RelevanceWork::default();
    let Some(binding) = super::invitation_content::binding_scope(db, claim)? else {
        return Ok(result(Relevance::Unknown, work));
    };
    if confirming(db, claim)? {
        return Ok(result(Relevance::Open, work));
    }
    let Some(state) = load(db, claim, &binding, consumer)? else {
        return Ok(result(Relevance::Unknown, work));
    };
    if state.claim != *claim || state.binding_id != binding {
        return Ok(result(Relevance::Unknown, work));
    }
    if witness(db, &state, frontier, &mut work)? {
        return Ok(result(Relevance::Open, work));
    }
    if state.absent && state.fence == now && state.frontier == frontier {
        return Ok(result(Relevance::Absent, work));
    }
    Ok(result(Relevance::Unknown, work))
}
