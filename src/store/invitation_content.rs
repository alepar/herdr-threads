//! Canonical invitation context and bounded presentation state. Presentation
//! bookkeeping proves cooperative offers, never acceptance or receipt ACK.
use super::{
    connection::{api_error, store_error},
    schema,
};
use crate::protocol::{
    ids::{InvitationId, ThreadId},
    results::{ApiError, ErrorCode},
};
use rusqlite::{Connection, params};

/// Publish a coalesced latest context in the producer's own transaction. The
/// sequence must be an actual allocated same-instance decision; no local hint
/// or global churn is a recipient content change.
pub(super) fn record_context_change(
    db: &Connection,
    instance: &str,
    thread: &ThreadId,
    sequence: u64,
) -> Result<(), ApiError> {
    let sequence = i64::try_from(sequence)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "context sequence exhausted"))?;
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM threads t JOIN host_instances h ON h.id=t.instance_id WHERE t.id=?1 AND t.instance_id=?2 AND h.decision_seq>=?3 AND ?3>0)",params![thread.as_str(),instance,sequence],|r|r.get(0)).map_err(store_error)?;
    if !valid {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid canonical invitation context sequence",
        ));
    }
    let changed=db.execute("INSERT INTO invitation_context_latest(thread_id,instance_id,last_change_seq,context_revision) VALUES (?1,?2,?3,1) ON CONFLICT(thread_id) DO UPDATE SET last_change_seq=excluded.last_change_seq,context_revision=invitation_context_latest.context_revision+1 WHERE invitation_context_latest.instance_id=excluded.instance_id AND invitation_context_latest.context_revision<?4 AND invitation_context_latest.last_change_seq<=excluded.last_change_seq",params![thread.as_str(),instance,sequence,i64::MAX]).map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "invitation context revision exhausted",
        ));
    }
    Ok(())
}

/// Same-ID command changes require a monotone epoch even when a command
/// returns to its original shape (None -> required -> None).
pub(super) fn record_command_change(
    db: &Connection,
    instance: &str,
    thread: &ThreadId,
    invitation: &InvitationId,
    sequence: Option<u64>,
) -> Result<(), ApiError> {
    let epoch:i64=db.query_row("SELECT i.command_context_epoch FROM invitations i JOIN threads t ON t.id=i.thread_id WHERE i.id=?1 AND i.thread_id=?2 AND t.instance_id=?3",params![invitation.as_str(),thread.as_str(),instance],|r|r.get(0)).map_err(store_error)?;
    if epoch < 0 {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "negative invitation command epoch",
        ));
    }
    let next = epoch.checked_add(1).ok_or_else(|| {
        api_error(
            ErrorCode::SequenceExhausted,
            "invitation command epoch exhausted",
        )
    })?;
    let changed=db.execute("UPDATE invitations SET command_context_epoch=?1 WHERE id=?2 AND command_context_epoch=?3",params![next,invitation.as_str(),epoch]).map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "invitation command epoch changed",
        ));
    }
    let sequence = match sequence {
        Some(v) => v,
        None => schema::next_decision_seq(db, instance)?,
    };
    record_context_change(db, instance, thread, sequence)
}

use super::connection::StoreContext;
use crate::{
    delivery::{Assembled, Assembler, DeliveryKind, invitation::InvitationContext},
    ports::{PresentationInput, PresentationOutput, PresentationWork},
    protocol::{
        authority::{COOPERATIVE_TOP_LEVEL_PROVENANCE, CallerClaim, CallerRole},
        commands::InvitationConfirmation,
        delivery::{
            DeliveryPageV2, DeliveryPresentationConfirmV2,
            DeliveryPresentationConfirmationV2 as Confirmation, DeliveryPullReplyV2,
            PresentationConsumer, PresentationMessage,
        },
        ids::MessageId,
        time::CallBudget,
    },
};
use rusqlite::{
    OptionalExtension,
    types::{Value, ValueRef},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const STATE_TABLE: &str = "invitation_presentation_consumers";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct State {
    instance_id: String,
    generation: i64,
    execution: String,
    render_key: String,
    initial_after: i64,
    initial_high: i64,
    context_after_seq: i64,
    context_after_thread: String,
    context_high: i64,
    context_thread: String,
    context_selected_seq: i64,
    context_invitation_after: i64,
    context_invitation_high: i64,
    revisit_after_bytes: i64,
    revisit_after_ordinal: i64,
    revisit_high_bytes: i64,
    revisit_high_ordinal: i64,
    cleanup_after: i64,
    dirty_after: i64,
    dirty_high: i64,
    rotation: i64,
    mod_generation: i64,
    mod_execution: String,
    mod_initial_after: i64,
    mod_initial_high: i64,
    mod_context_after_seq: i64,
    mod_context_high: i64,
    mod_context_selected_seq: i64,
    mod_context_invitation_after: i64,
    mod_context_invitation_high: i64,
    mod_cleanup_after: i64,
    mod_cleanup_high: i64,
    mod_version: i64,
    mod_context_after_thread: String,
    mod_context_thread: String,
    ordinary_publication_after: i64,
    ordinary_publication_high: i64,
    ordinary_passive_generation: i64,
    ordinary_scan_passive_generation: i64,
    ordinary_version: String,
    ordinary_done: i64,
    ordinary_cursor: Option<String>,
    ordinary_pending: String,
    ordinary_partial: String,
    ordinary_skip_body: Option<String>,
    outbox_retired_mask: i64,
    outbox_confirmed_mask: i64,
    last_descriptors: String,
    last_confirmed_mask: i64,
    control_state: String,
    outbox_selection: String,
    outbox_render_key: String,
    outbox_codec_key: String,
    outbox_prefix: String,
    outbox_token: Option<String>,
    last_token: Option<String>,
    outbox_text: String,
    outbox_descriptors: String,
    outbox_version: String,
    outbox_more: i64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            instance_id: String::new(),
            generation: 0,
            execution: String::new(),
            render_key: String::new(),
            initial_after: 0,
            initial_high: 0,
            context_after_seq: 0,
            context_after_thread: String::new(),
            context_high: 0,
            context_thread: String::new(),
            context_selected_seq: 0,
            context_invitation_after: 0,
            context_invitation_high: 0,
            revisit_after_bytes: 0,
            revisit_after_ordinal: 0,
            revisit_high_bytes: 0,
            revisit_high_ordinal: 0,
            cleanup_after: 0,
            dirty_after: 0,
            dirty_high: 0,
            rotation: 0,
            mod_generation: 0,
            mod_execution: String::new(),
            mod_initial_after: 0,
            mod_initial_high: 0,
            mod_context_after_seq: 0,
            mod_context_high: 0,
            mod_context_selected_seq: 0,
            mod_context_invitation_after: 0,
            mod_context_invitation_high: 0,
            mod_cleanup_after: 0,
            mod_cleanup_high: 0,
            mod_version: 0,
            mod_context_after_thread: String::new(),
            mod_context_thread: String::new(),
            ordinary_publication_after: 0,
            ordinary_publication_high: 0,
            ordinary_passive_generation: 0,
            ordinary_scan_passive_generation: 0,
            ordinary_version: String::new(),
            ordinary_done: 0,
            ordinary_cursor: None,
            ordinary_pending: "{\"prefix\":[],\"items\":[]}".into(),
            ordinary_partial: "{}".into(),
            ordinary_skip_body: None,
            outbox_retired_mask: 0,
            outbox_confirmed_mask: 0,
            last_descriptors: "[]".into(),
            last_confirmed_mask: 0,
            control_state: "{}".into(),
            outbox_selection: String::new(),
            outbox_render_key: String::new(),
            outbox_codec_key: String::new(),
            outbox_prefix: "[]".into(),
            outbox_token: None,
            last_token: None,
            outbox_text: String::new(),
            outbox_descriptors: "[]".into(),
            outbox_version: String::new(),
            outbox_more: 0,
        }
    }
}
fn corrupt(detail: impl Into<String>) -> ApiError {
    api_error(ErrorCode::StoreCorrupt, detail)
}
fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, ApiError> {
    serde_json::from_str(json).map_err(|e| corrupt(format!("invalid presentation state: {e}")))
}
const ORDINARY_PARTIAL_BYTES: usize = 8192 * 6 + 128 * 2 + 7;
fn partial_state(json: &str) -> Result<HashMap<String, String>, ApiError> {
    if json.len() > ORDINARY_PARTIAL_BYTES {
        return Err(corrupt("ordinary partial serialized bound"));
    }
    let partial: HashMap<String, String> = decode(json)?;
    if partial.len() > 1
        || partial.iter().any(|(id, text)| {
            MessageId::parse(id).is_err() || text.len() >= crate::delivery::page::BODY_LIMIT_BYTES
        })
    {
        return Err(corrupt("invalid ordinary partial record"));
    }
    Ok(partial)
}
fn partial_json(partial: &HashMap<String, String>) -> Result<String, ApiError> {
    let json = encode(partial)?;
    partial_state(&json)?;
    Ok(json)
}

fn encode<T: Serialize>(value: &T) -> Result<String, ApiError> {
    serde_json::to_string(value).map_err(|e| corrupt(format!("presentation encoding: {e}")))
}
fn scope_matches(db: &Connection, claim: &CallerClaim) -> Result<bool, ApiError> {
    if claim.role != CallerRole::TopLevel {
        return Ok(false);
    }
    let Ok(generation) = i64::try_from(claim.binding_generation) else {
        return Ok(false);
    };
    db.query_row("SELECT EXISTS(SELECT 1 FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id=?1 AND s.instance_id=?2 AND s.state='resolved' AND s.generation=?3 AND b.execution_id=?4 AND b.native_session=?5 AND b.observation_provenance=?6 AND s.target_id=?7 AND b.target_id=s.target_id AND b.target_generation=s.target_generation AND NOT EXISTS(SELECT 1 FROM recovery_holds h WHERE h.instance_id=s.instance_id AND h.target_id=s.target_id AND h.released_at IS NULL))",params![claim.seat.as_str(),claim.instance,generation,claim.execution.as_str(),claim.native_session.as_str(),COOPERATIVE_TOP_LEVEL_PROVENANCE,claim.target.as_str()],|r|r.get(0)).map_err(store_error)
}

fn load_raw(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
) -> Result<State, ApiError> {
    // The field set is this private typed state, never caller-selected SQL.
    let fields =
        serde_json::to_value(State::default()).map_err(|_| corrupt("presentation fields"))?;
    let names = fields
        .as_object()
        .ok_or_else(|| corrupt("presentation fields"))?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let sql = format!(
        "SELECT {} FROM {STATE_TABLE} WHERE seat_id=?1 AND consumer=?2",
        names.join(",")
    );
    let raw = db
        .query_row(
            &sql,
            params![claim.seat.as_str(), consumer.as_str()],
            |row| {
                let mut map = serde_json::Map::new();
                for (n, name) in names.iter().enumerate() {
                    // SQL CHECKs protect canonical writes. Repeat private JSON byte
                    // ceilings before allocating/parsing a repaired or corrupt row.
                    let limit = match name.as_str() {
                        "last_descriptors" | "control_state" => Some(4096),
                        "outbox_descriptors" => Some(262144),
                        "outbox_prefix" => Some(24601),
                        "outbox_selection" => Some(128),
                        "outbox_text" => Some(65536),
                        "ordinary_pending" => Some(ORDINARY_CARRY_BYTES),
                        "ordinary_partial" => Some(ORDINARY_PARTIAL_BYTES),
                        "ordinary_cursor" => Some(8192),
                        "ordinary_skip_body"
                        | "execution"
                        | "mod_execution"
                        | "instance_id"
                        | "context_after_thread"
                        | "context_thread"
                        | "mod_context_after_thread"
                        | "mod_context_thread" => Some(128),
                        "ordinary_version" | "outbox_version" => Some(20),
                        "render_key" | "outbox_render_key" | "outbox_codec_key" => Some(64),
                        "outbox_token" | "last_token" => Some(32),
                        _ => None,
                    };
                    if let (Some(limit), ValueRef::Text(bytes)) = (limit, row.get_ref(n)?) {
                        if bytes.len() > limit {
                            return Ok(Err(corrupt(format!(
                                "presentation {name} serialized bound"
                            ))));
                        }
                        if matches!(
                            name.as_str(),
                            "instance_id"
                                | "context_after_thread"
                                | "context_thread"
                                | "mod_context_after_thread"
                                | "mod_context_thread"
                        ) && !bytes.iter().all(u8::is_ascii_graphic)
                            && !(matches!(
                                name.as_str(),
                                "context_after_thread" | "mod_context_after_thread"
                            ) && bytes == "\u{10ffff}".as_bytes())
                        {
                            return Ok(Err(corrupt("invalid presentation identity cursor")));
                        }
                        if std::str::from_utf8(bytes).is_err() {
                            return Ok(Err(corrupt("invalid presentation UTF8")));
                        }
                    }
                    let nullable = matches!(
                        name.as_str(),
                        "ordinary_cursor" | "ordinary_skip_body" | "outbox_token" | "last_token"
                    );
                    let text =
                        fields.get(name).is_some_and(serde_json::Value::is_string) || nullable;
                    match (text, nullable, row.get_ref(n)?) {
                        (true, _, ValueRef::Text(_))
                        | (true, true, ValueRef::Null)
                        | (false, _, ValueRef::Integer(_)) => {}
                        _ => return Ok(Err(corrupt("invalid private presentation field type"))),
                    }
                    let value = match row.get_ref(n)? {
                        ValueRef::Null => serde_json::Value::Null,
                        ValueRef::Integer(v) => serde_json::json!(v),
                        ValueRef::Text(v) => {
                            serde_json::Value::String(match std::str::from_utf8(v) {
                                Ok(text) => text.to_owned(),
                                Err(_) => return Ok(Err(corrupt("invalid presentation UTF8"))),
                            })
                        }
                        _ => {
                            return Err(rusqlite::Error::InvalidColumnType(
                                n,
                                name.clone(),
                                row.get_ref(n)?.data_type(),
                            ));
                        }
                    };
                    map.insert(name.clone(), value);
                }
                Ok(Ok(serde_json::Value::Object(map)))
            },
        )
        .optional()
        .map_err(store_error)?
        .transpose()?;
    let state: State = raw
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| corrupt(format!("invalid presentation fields: {e}")))?
        .unwrap_or_default();
    let encoded =
        serde_json::to_value(&state).map_err(|_| corrupt("presentation field validation"))?;
    if encoded
        .as_object()
        .is_none_or(|fields| fields.values().any(|v| v.as_i64().is_some_and(|n| n < 0)))
    {
        return Err(corrupt("negative presentation state counter"));
    }
    checked_publication_state(&state)?;
    Ok(state)
}
fn load(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
) -> Result<State, ApiError> {
    let state = load_raw(db, claim, consumer)?;
    let generation = i64::try_from(claim.binding_generation)
        .map_err(|_| corrupt("presentation generation overflow"))?;
    if state.generation == generation
        && state.execution == claim.execution.as_str()
        && state.instance_id == claim.instance
    {
        return Ok(state);
    }
    // Only an actual presentation operation resets presentation state. Keep
    // independent passive maintenance truth across this lazy binding reset.
    let mut fresh = State {
        instance_id: claim.instance.clone(),
        generation,
        execution: claim.execution.as_str().into(),
        ..State::default()
    };
    let previous: Controls = decode(&state.control_state)?;
    previous.checked()?;
    fresh.control_state = encode(&Controls {
        counter: previous.counter,
        ..Controls::default()
    })?;
    // The first of the two existing owners initializes the passive signal.
    // A prior same-instance ModRefresh transition already owns these counters.
    if state.instance_id == claim.instance
        && state.mod_generation == generation
        && state.mod_execution == claim.execution.as_str()
    {
        fresh.ordinary_publication_after = state.ordinary_publication_after;
        fresh.ordinary_publication_high = state.ordinary_publication_high;
        fresh.ordinary_passive_generation = state.ordinary_passive_generation;
        fresh.ordinary_scan_passive_generation = state.ordinary_scan_passive_generation;
    }
    fresh.mod_generation = state.mod_generation;
    fresh.mod_execution = state.mod_execution;
    fresh.mod_initial_after = state.mod_initial_after;
    fresh.mod_initial_high = state.mod_initial_high;
    fresh.mod_context_after_seq = state.mod_context_after_seq;
    fresh.mod_context_high = state.mod_context_high;
    fresh.mod_context_selected_seq = state.mod_context_selected_seq;
    fresh.mod_context_invitation_after = state.mod_context_invitation_after;
    fresh.mod_context_invitation_high = state.mod_context_invitation_high;
    fresh.mod_cleanup_after = state.mod_cleanup_after;
    fresh.mod_cleanup_high = state.mod_cleanup_high;
    fresh.mod_version = state.mod_version;
    fresh.mod_context_after_thread = state.mod_context_after_thread;
    fresh.mod_context_thread = state.mod_context_thread;
    Ok(fresh)
}
fn save(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
    state: &State,
) -> Result<(), ApiError> {
    checked_publication_state(state)?;
    let object = serde_json::to_value(state).map_err(|_| corrupt("presentation state"))?;
    let object = object
        .as_object()
        .ok_or_else(|| corrupt("presentation state"))?;
    let mut columns = vec!["seat_id".to_owned(), "consumer".to_owned()];
    let mut values = vec![
        Value::Text(claim.seat.as_str().into()),
        Value::Text(consumer.as_str().into()),
    ];
    for (name, value) in object {
        columns.push(name.clone());
        values.push(match value {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::String(v) => Value::Text(v.clone()),
            serde_json::Value::Number(v) => Value::Integer(
                v.as_i64()
                    .ok_or_else(|| corrupt("presentation counter overflow"))?,
            ),
            _ => return Err(corrupt("invalid presentation scalar")),
        });
    }
    let assignments = columns
        .iter()
        .skip(2)
        .map(|v| format!("{v}=excluded.{v}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "INSERT INTO {STATE_TABLE}({}) VALUES ({}) ON CONFLICT(seat_id,consumer) DO UPDATE SET {assignments}",
        columns.join(","),
        vec!["?"; columns.len()].join(",")
    );
    db.execute(&sql, rusqlite::params_from_iter(values))
        .map_err(store_error)?;
    Ok(())
}

/// One bounded canonical hydration statement, including full goal and effective
/// requirement. No per-invitation body or requirement lookup.
pub(super) const HYDRATE_SQL: &str = "SELECT i.id,i.ordinal,i.thread_id,t.topic,t.goal,i.episode,t.topic_revision,i.command_context_epoch,i.confirmation,i.deadline_at,CASE WHEN r.id IS NULL THEN NULL ELSE json_object('requirement',r.id,'revision',r.revision,'invitation',r.invitation_id,'thread',r.thread_id,'seat',r.seat_id,'issuer',r.issuer_author_id,'state',r.state,'accepted_by',r.accepted_by_seat_id,'accepted_at',r.accepted_at) END FROM json_each(?1) wanted CROSS JOIN digest_pending_invitations p ON p.invitation_id=wanted.value AND p.seat_id=?2 CROSS JOIN invitations i ON i.id=p.invitation_id CROSS JOIN threads t ON t.id=i.thread_id LEFT JOIN requirement_episodes r ON r.invitation_id=i.id AND r.state IN ('pending','accepted') WHERE t.instance_id=?3";
fn hydrate_many(
    db: &Connection,
    claim: &CallerClaim,
    ids: &[InvitationId],
) -> Result<HashMap<InvitationId, (InvitationContext, i64)>, ApiError> {
    if ids.len() > 100 {
        return Err(corrupt("presentation hydration exceeds source bound"));
    }
    let mut statement = db.prepare(HYDRATE_SQL).map_err(store_error)?;
    let rows = statement
        .query_map(
            params![encode(&ids)?, claim.seat.as_str(), claim.instance],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .map_err(store_error)?;
    let mut result = HashMap::new();
    for row in rows {
        let (
            id,
            ordinal,
            thread,
            topic,
            goal,
            episode,
            revision,
            epoch,
            confirmation,
            deadline,
            required,
        ) = row.map_err(store_error)?;
        let confirmation = match confirmation.as_deref() {
            None => None,
            Some("lazy") => Some(InvitationConfirmation::Lazy),
            Some("require_ack") => Some(InvitationConfirmation::RequireAck),
            _ => return Err(corrupt("invalid invitation confirmation")),
        };
        let nonnegative =
            |n| u64::try_from(n).map_err(|_| corrupt("negative invitation context counter"));
        let id = InvitationId::new(id);
        let context = InvitationContext {
            id: id.clone(),
            thread: ThreadId::new(thread),
            topic,
            goal,
            episode: nonnegative(episode)?,
            topic_revision: nonnegative(revision)?,
            command_epoch: nonnegative(epoch)?,
            confirmation,
            deadline: (confirmation != Some(InvitationConfirmation::Lazy))
                .then_some(crate::protocol::time::UtcMillis(deadline)),
            required: required.map(|v| decode(&v)).transpose()?,
        };
        if context.key().len() > 4096 {
            return Err(corrupt("invitation context key exceeds bound"));
        }
        result.insert(id, (context, ordinal));
    }
    Ok(result)
}

pub(super) fn render_selection(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
) -> Result<Option<crate::delivery::presentation::FrozenRenderIdentity>, ApiError> {
    if !scope_matches(db, claim)? {
        return Ok(None);
    }
    let state = load_raw(db, claim, consumer)?;
    if state.generation
        != i64::try_from(claim.binding_generation)
            .map_err(|_| corrupt("presentation generation"))?
        || state.execution != claim.execution.as_str()
        || state.instance_id != claim.instance
    {
        return Ok(None);
    }
    state
        .outbox_token
        .as_ref()
        .map(|token| {
            Ok(crate::delivery::presentation::FrozenRenderIdentity {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
                generation: claim.binding_generation,
                execution: claim.execution.clone(),
                native_session: claim.native_session.clone(),
                consumer,
                token: token.clone(),
                render_key: state.outbox_render_key.clone(),
                descriptor_key: state.outbox_codec_key.clone(),
                selection: decode(&state.outbox_selection)?,
            })
        })
        .transpose()
}

fn checked_publication_state(state: &State) -> Result<(), ApiError> {
    if state.ordinary_publication_after < 0
        || state.ordinary_publication_high < state.ordinary_publication_after
        || state.ordinary_scan_passive_generation < 0
        || state.ordinary_passive_generation < state.ordinary_scan_passive_generation
    {
        return Err(corrupt("invalid ordinary publication progress"));
    }
    Ok(())
}
#[derive(Default)]
struct PublicationProgress {
    visits: usize,
    probes: usize,
    seeks: usize,
    more: bool,
}
pub(super) const PUBLICATION_RANGE_SQL: &str = "SELECT preparation_id,message_id,thread_id,decision_seq FROM send_manifests INDEXED BY send_manifests_instance_decision WHERE instance_id=?1 AND decision_seq>?2 AND decision_seq<=?3 ORDER BY decision_seq LIMIT 100";
fn publication_head(db: &Connection, instance: &str) -> Result<i64, ApiError> {
    db.query_row("SELECT decision_seq FROM send_manifests INDEXED BY send_manifests_instance_decision WHERE instance_id=?1 ORDER BY decision_seq DESC LIMIT 1",[instance],|r|r.get(0)).optional().map_err(store_error).map(|v|v.unwrap_or(0))
}
fn advance_ordinary_publications(
    db: &Connection,
    claim: &CallerClaim,
    state: &mut State,
) -> Result<PublicationProgress, ApiError> {
    checked_publication_state(state)?;
    let mut work = PublicationProgress::default();
    if state.ordinary_publication_after == state.ordinary_publication_high {
        work.seeks += 1;
        state.ordinary_publication_high =
            publication_head(db, &claim.instance)?.max(state.ordinary_publication_after);
    }
    if state.ordinary_publication_after == state.ordinary_publication_high {
        return Ok(work);
    }
    work.seeks += 1;
    let mut statement = db.prepare(PUBLICATION_RANGE_SQL).map_err(store_error)?;
    let rows = statement
        .query_map(
            params![
                claim.instance,
                state.ordinary_publication_after,
                state.ordinary_publication_high
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    let mut changed = false;
    for (preparation, message, thread, seq) in rows {
        work.visits += 1;
        state.ordinary_publication_after = seq;
        work.probes += 1;
        let recipient:Option<(String,String,String)>=db.query_row("SELECT message_id,thread_id,state FROM lazy_recipients WHERE preparation_id=?1 AND seat_id=?2",params![preparation,claim.seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
        if recipient
            .as_ref()
            .is_none_or(|(m, t, status)| m != &message || t != &thread || status != "pending")
        {
            continue;
        }
        work.probes += 1;
        let published:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND instance_id=?2 AND thread_id=?3 AND decision_seq=?4 AND delivery_mode='lazy')",params![message,claim.instance,thread,seq],|r|r.get(0)).map_err(store_error)?;
        changed |= published;
    }
    if work.visits < 100 {
        state.ordinary_publication_after = state.ordinary_publication_high;
    }
    if state.ordinary_publication_after < state.ordinary_publication_high {
        work.more = true;
    } else {
        work.seeks += 1;
        work.more = publication_head(db, &claim.instance)? > state.ordinary_publication_after;
    }
    if changed {
        state.ordinary_passive_generation = state
            .ordinary_passive_generation
            .checked_add(1)
            .ok_or_else(|| {
                api_error(
                    ErrorCode::SequenceExhausted,
                    "ordinary passive generation exhausted",
                )
            })?;
        state.mod_version = state.mod_version.checked_add(1).ok_or_else(|| {
            api_error(
                ErrorCode::SequenceExhausted,
                "passive content version exhausted",
            )
        })?;
    }
    Ok(work)
}

fn control_stamp(
    item: &crate::protocol::delivery::PresentationItemV2,
) -> Option<crate::protocol::delivery::PresentationControlStampV2> {
    use crate::protocol::delivery::{
        PresentationControlStampV2 as Stamp, PresentationItemV2 as Item,
    };
    match item {
        Item::Attention {
            epoch,
            attention_version,
            ..
        } => Some(Stamp::Attention {
            epoch: epoch.clone(),
            version: *attention_version,
        }),
        Item::AttentionCleared {
            epoch,
            attention_version,
        } => Some(Stamp::Cleared {
            epoch: epoch.clone(),
            version: *attention_version,
        }),
        _ => None,
    }
}
fn descriptor_mask(len: usize) -> Result<u32, ApiError> {
    if len > 32 {
        return Err(corrupt("presentation descriptor bound"));
    }
    Ok(if len == 32 {
        u32::MAX
    } else {
        (1u32 << len) - 1
    })
}
fn checked_masks(
    descriptors: &[FrozenPresentation],
    retired: i64,
    confirmed: i64,
) -> Result<(u32, u32), ApiError> {
    let allowed = descriptor_mask(descriptors.len())?;
    let retired = u32::try_from(retired).map_err(|_| corrupt("presentation retired mask"))?;
    let confirmed = u32::try_from(confirmed).map_err(|_| corrupt("presentation confirmed mask"))?;
    if retired & !allowed != 0 || confirmed & !retired != 0 {
        return Err(corrupt("presentation descriptor masks"));
    }
    let stamps = descriptors
        .iter()
        .filter_map(|d| control_stamp(&d.item))
        .collect::<Vec<_>>();
    crate::protocol::delivery::validate_control_stamps(&stamps).map_err(corrupt)?;
    Ok((retired, confirmed))
}
fn retire_outbox(state: &mut State) -> Result<(), ApiError> {
    let descriptors: Vec<FrozenPresentation> = decode(&state.outbox_descriptors)?;
    checked_masks(
        &descriptors,
        state.outbox_retired_mask,
        state.outbox_confirmed_mask,
    )?;
    let mut stamps = Vec::new();
    let mut mask = 0u32;
    for (slot, descriptor) in descriptors.iter().enumerate() {
        if let Some(stamp) = control_stamp(&descriptor.item) {
            if state.outbox_confirmed_mask & (1i64 << slot) != 0 {
                mask |= 1u32 << stamps.len();
            }
            stamps.push(stamp);
        }
    }
    state.last_token = state.outbox_token.take();
    state.last_descriptors = encode(&stamps)?;
    state.last_confirmed_mask = i64::from(mask);
    state.outbox_text.clear();
    state.outbox_descriptors = "[]".into();
    state.outbox_version.clear();
    state.outbox_more = 0;
    state.outbox_retired_mask = 0;
    state.outbox_confirmed_mask = 0;
    state.outbox_selection.clear();
    state.outbox_render_key.clear();
    state.outbox_codec_key.clear();
    Ok(())
}
pub(super) fn confirm(
    db: &Connection,
    request: &DeliveryPresentationConfirmV2,
) -> Result<crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2, ApiError> {
    use crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 as Reply;
    request.validate().map_err(ApiError::invalid_request)?;
    if !scope_matches(db, &request.claim)? {
        return Ok(Reply::empty(Confirmation::StaleBinding));
    }
    let mut state = load(db, &request.claim, request.consumer)?;
    let current = state.outbox_token.as_deref() == Some(&request.token);
    let last = state.last_token.as_deref() == Some(&request.token);
    if !current && !last {
        return Ok(Reply::empty(Confirmation::NotCurrentPage));
    }
    let mut controls: Controls = decode(&state.control_state)?;
    controls.checked()?;
    if last && !current {
        let stamps: Vec<crate::protocol::delivery::PresentationControlStampV2> =
            decode(&state.last_descriptors)?;
        crate::protocol::delivery::validate_control_stamps(&stamps).map_err(corrupt)?;
        let mask = u32::try_from(state.last_confirmed_mask)
            .map_err(|_| corrupt("last presentation mask"))?;
        if mask & !descriptor_mask(stamps.len())? != 0 {
            return Err(corrupt("last presentation mask"));
        }
        if request
            .emitted_controls
            .iter()
            .any(|stamp| !stamps.contains(stamp))
        {
            return Err(ApiError::invalid_request("foreign presentation control"));
        }
        let acknowledged_controls = stamps
            .into_iter()
            .enumerate()
            .filter_map(|(slot, stamp)| {
                (mask & (1u32 << slot) != 0
                    && request.emitted_controls.contains(&stamp)
                    && controls.applicable(&stamp))
                .then_some(stamp)
            })
            .collect();
        return Ok(Reply {
            status: Confirmation::AlreadyConfirmed,
            acknowledged_controls,
        });
    }
    let descriptors: Vec<FrozenPresentation> = decode(&state.outbox_descriptors)?;
    let (mut retired, mut confirmed) = checked_masks(
        &descriptors,
        if current {
            state.outbox_retired_mask
        } else {
            i64::from(descriptor_mask(descriptors.len())?)
        },
        if current {
            state.outbox_confirmed_mask
        } else {
            state.last_confirmed_mask
        },
    )?;
    // Complete subset validation precedes all state/episode effects.
    for stamp in &request.emitted_controls {
        if !descriptors
            .iter()
            .any(|d| control_stamp(&d.item).as_ref() == Some(stamp))
        {
            return Err(ApiError::invalid_request("foreign presentation control"));
        }
    }
    let mut acknowledged_controls = Vec::new();
    let mut changed = false;
    for (slot, descriptor) in descriptors.iter().enumerate() {
        let bit = 1u32 << slot;
        if let Some(stamp) = control_stamp(&descriptor.item) {
            if !request.emitted_controls.contains(&stamp) {
                continue;
            }
            if retired & bit == 0 {
                retired |= bit;
                changed = true;
                if controls.applicable(&stamp) {
                    controls.confirm(&stamp);
                    confirmed |= bit;
                }
            }
            if confirmed & bit != 0 && controls.applicable(&stamp) {
                acknowledged_controls.push(stamp);
            }
        } else if retired & bit == 0 {
            retired |= bit;
            confirmed |= bit;
            changed = true;
        }
    }
    if current {
        state.outbox_retired_mask = i64::from(retired);
        state.outbox_confirmed_mask = i64::from(confirmed);
        state.control_state = encode(&controls)?;
        if retired == descriptor_mask(descriptors.len())? {
            retire_outbox(&mut state)?;
        }
        if changed {
            save(db, &request.claim, request.consumer, &state)?;
        }
    }
    Ok(Reply {
        status: if changed {
            Confirmation::Confirmed
        } else {
            Confirmation::AlreadyConfirmed
        },
        acknowledged_controls,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Initial,
    Context,
    Revisit,
}
#[derive(Debug, Clone)]
struct Candidate {
    id: InvitationId,
    source: Source,
}

fn pending_high(db: &Connection, claim: &CallerClaim) -> Result<i64, ApiError> {
    Ok(db.query_row("SELECT ordinal FROM digest_pending_invitations INDEXED BY invitation_content_initial WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 1",[claim.seat.as_str()],|r|r.get(0)).optional().map_err(store_error)?.unwrap_or(0))
}
fn context_high(db: &Connection, claim: &CallerClaim) -> Result<i64, ApiError> {
    Ok(db.query_row("SELECT last_change_seq FROM invitation_context_latest INDEXED BY invitation_context_changed WHERE instance_id=?1 ORDER BY last_change_seq DESC,thread_id DESC LIMIT 1",[&claim.instance],|r|r.get(0)).optional().map_err(store_error)?.unwrap_or(0))
}
fn initial_candidates(
    db: &Connection,
    claim: &CallerClaim,
    state: &mut State,
    limit: usize,
) -> Result<Vec<Candidate>, ApiError> {
    if state.initial_after >= state.initial_high {
        state.initial_high = pending_high(db, claim)?.max(state.initial_high);
    }
    let mut query=db.prepare("SELECT invitation_id,ordinal FROM digest_pending_invitations INDEXED BY invitation_content_initial WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4").map_err(store_error)?;
    let rows = query
        .query_map(
            params![
                claim.seat.as_str(),
                state.initial_after,
                state.initial_high,
                limit as i64
            ],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .map_err(store_error)?;
    let mut result = Vec::new();
    for row in rows {
        let (id, ordinal) = row.map_err(store_error)?;
        state.initial_after = ordinal;
        result.push(Candidate {
            id: InvitationId::new(id),
            source: Source::Initial,
        });
    }
    if result.len() < limit {
        state.initial_after = state.initial_high;
    }
    Ok(result)
}
/// Each latest-topic row and each recipient expansion charges the same quota.
/// A selected thread's frozen ordinal high water prevents a busy producer from
/// extending this finite walk; later changes are picked up in the next sweep.
fn context_candidates(
    db: &Connection,
    claim: &CallerClaim,
    state: &mut State,
    limit: usize,
) -> Result<(Vec<Candidate>, usize), ApiError> {
    if state.context_thread.is_empty() && state.context_after_seq >= state.context_high {
        state.context_high = context_high(db, claim)?.max(state.context_high);
    }
    let mut result = Vec::new();
    let mut visited = 0;
    while visited < limit {
        if state.context_thread.is_empty() {
            let next=db.query_row("SELECT last_change_seq,thread_id FROM invitation_context_latest INDEXED BY invitation_context_changed WHERE instance_id=?1 AND (last_change_seq,thread_id)>(?2,?3) AND last_change_seq<=?4 ORDER BY last_change_seq,thread_id LIMIT 1",params![claim.instance,state.context_after_seq,state.context_after_thread,state.context_high],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).optional().map_err(store_error)?;
            let Some((seq, thread)) = next else {
                state.context_after_seq = state.context_high;
                state.context_after_thread = "\u{10ffff}".into();
                break;
            };
            visited += 1;
            state.context_selected_seq = seq;
            state.context_thread = thread;
            state.context_invitation_after = 0;
            state.context_invitation_high = state.initial_high;
            if visited == limit {
                break;
            }
        }
        let room = limit - visited;
        let mut query=db.prepare("SELECT invitation_id,ordinal FROM digest_pending_invitations INDEXED BY invitation_content_thread WHERE seat_id=?1 AND thread_id=?2 AND ordinal>?3 AND ordinal<=?4 ORDER BY ordinal LIMIT ?5").map_err(store_error)?;
        let rows = query
            .query_map(
                params![
                    claim.seat.as_str(),
                    state.context_thread,
                    state.context_invitation_after,
                    state.context_invitation_high,
                    room as i64
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .map_err(store_error)?;
        let mut count = 0;
        for row in rows {
            let (id, ordinal) = row.map_err(store_error)?;
            state.context_invitation_after = ordinal;
            result.push(Candidate {
                id: InvitationId::new(id),
                source: Source::Context,
            });
            count += 1;
            visited += 1;
        }
        if count < room {
            state.context_after_seq = state.context_selected_seq;
            state.context_after_thread = std::mem::take(&mut state.context_thread);
            state.context_selected_seq = 0;
            state.context_invitation_after = 0;
            state.context_invitation_high = 0;
            // Empty expansions are charged for their topic row, never an
            // unbounded scan hidden behind a recipient mismatch predicate.
        }
    }
    Ok((result, visited))
}

fn bounded_private_text(
    row: &rusqlite::Row<'_>,
    index: usize,
    limit: usize,
) -> rusqlite::Result<String> {
    let value = row.get_ref(index)?;
    if let ValueRef::Text(bytes) = value
        && bytes.len() <= limit
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        return Ok(text.to_owned());
    }
    Err(rusqlite::Error::FromSqlConversionFailure(
        index,
        value.data_type(),
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid bounded private presentation text",
        )),
    ))
}
fn private_read_error(error: rusqlite::Error) -> ApiError {
    match error {
        rusqlite::Error::FromSqlConversionFailure(..) | rusqlite::Error::InvalidColumnType(..) => {
            corrupt("invalid private presentation text")
        }
        other => store_error(other),
    }
}

#[derive(Debug, Clone)]
struct Item {
    key: String,
    partial: i64,
    complete: i64,
    source: Source,
}
struct ItemRender<'a> {
    key: &'a str,
    required_bytes: usize,
}
fn item(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
    context: &InvitationContext,
    ordinal: i64,
    render: ItemRender<'_>,
    source: Source,
) -> Result<Item, ApiError> {
    let ItemRender {
        key: render_key,
        required_bytes,
    } = render;
    let key = context.key();
    let existing=db.query_row("SELECT generation,execution,context_key,partial_count,complete_count,output_class FROM invitation_presentation_items WHERE seat_id=?1 AND invitation_id=?2 AND consumer=?3",params![claim.seat.as_str(),context.id.as_str(),consumer.as_str()],|r|Ok((r.get::<_,i64>(0)?,bounded_private_text(r,1,128)?,bounded_private_text(r,2,4096)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).optional().map_err(private_read_error)?;
    let proposed = match source {
        Source::Initial => 2,
        Source::Context => 3,
        Source::Revisit => 4,
    };
    let (partial, complete, output_class) = match existing {
        Some((generation, execution, old, partial, complete, class))
            if generation == claim.binding_generation as i64
                && execution == claim.execution.as_str()
                && old == key =>
        {
            (partial, complete, class)
        }
        _ => (0, 0, proposed),
    };
    let size = if complete == 1 {
        i64::MAX
    } else {
        i64::try_from(required_bytes).map_err(|_| corrupt("invitation render size overflow"))?
    };
    db.execute("INSERT INTO invitation_presentation_items(seat_id,invitation_id,consumer,generation,execution,context_key,render_key,partial_count,complete_count,required_render_bytes,revisit_ordinal,mod_key,output_class) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'',?12) ON CONFLICT(seat_id,invitation_id,consumer) DO UPDATE SET generation=excluded.generation,execution=excluded.execution,context_key=excluded.context_key,render_key=excluded.render_key,partial_count=excluded.partial_count,complete_count=excluded.complete_count,required_render_bytes=excluded.required_render_bytes,revisit_ordinal=excluded.revisit_ordinal,output_class=excluded.output_class",params![claim.seat.as_str(),context.id.as_str(),consumer.as_str(),claim.binding_generation as i64,claim.execution.as_str(),key,render_key,partial,complete,size,ordinal,output_class]).map_err(store_error)?;
    Ok(Item {
        key,
        partial,
        complete,
        source: match output_class {
            2 => Source::Initial,
            3 => Source::Context,
            4 => Source::Revisit,
            _ => return Err(corrupt("invalid presentation output class")),
        },
    })
}
/// Ten cleanup visits are charged inside the twenty revisit visits. Removed
/// rows are deleted only when visited; no lifecycle fanout or cascade scan.
fn revisit_candidates(
    db: &Connection,
    claim: &CallerClaim,
    consumer: PresentationConsumer,
    state: &mut State,
    limit: usize,
    standalone: usize,
) -> Result<(Vec<Candidate>, usize), ApiError> {
    let mut visited = 0;
    let mut result = Vec::new();
    let cleanup_limit = limit.min(10);
    let mut query=db.prepare("SELECT invitation_id,revisit_ordinal FROM invitation_presentation_items INDEXED BY invitation_presentation_cleanup WHERE seat_id=?1 AND consumer=?2 AND revisit_ordinal>?3 ORDER BY revisit_ordinal LIMIT ?4").map_err(store_error)?;
    let rows = query
        .query_map(
            params![
                claim.seat.as_str(),
                consumer.as_str(),
                state.cleanup_after,
                cleanup_limit as i64
            ],
            |r| Ok((bounded_private_text(r, 0, 128)?, r.get::<_, i64>(1)?)),
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(private_read_error)?;
    for (id, ordinal) in &rows {
        let id =
            InvitationId::parse(id).map_err(|_| corrupt("invalid private invitation identity"))?;
        visited += 1;
        state.cleanup_after = *ordinal;
        let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=?1 AND invitation_id=?2)",params![claim.seat.as_str(),id.as_str()],|r|r.get(0)).map_err(store_error)?;
        if !pending {
            // The fixed Watch row also carries a not-yet-observed passive
            // removal. Keep that current-row evidence until maintenance
            // advances its content version, but exclude it from output work.
            db.execute("UPDATE invitation_presentation_items SET required_render_bytes=?4 WHERE seat_id=?1 AND consumer=?2 AND invitation_id=?3 AND consumer='watch' AND mod_key!=''",params![claim.seat.as_str(),consumer.as_str(),id.as_str(),i64::MAX]).map_err(store_error)?;
            db.execute("DELETE FROM invitation_presentation_items WHERE seat_id=?1 AND consumer=?2 AND invitation_id=?3 AND (consumer!='watch' OR mod_key='')",params![claim.seat.as_str(),consumer.as_str(),id.as_str()]).map_err(store_error)?;
        }
    }
    if rows.len() < cleanup_limit {
        state.cleanup_after = 0;
    }
    // On a render-key change, a bounded ordinal walk refreshes sizing. It
    // never resets allocation counters or initial/context discovery.
    if state.dirty_after < state.dirty_high && visited < limit {
        let mut query=db.prepare("SELECT invitation_id,revisit_ordinal FROM invitation_presentation_items INDEXED BY invitation_presentation_cleanup WHERE seat_id=?1 AND consumer=?2 AND revisit_ordinal>?3 AND revisit_ordinal<=?4 ORDER BY revisit_ordinal LIMIT ?5").map_err(store_error)?;
        let rows = query
            .query_map(
                params![
                    claim.seat.as_str(),
                    consumer.as_str(),
                    state.dirty_after,
                    state.dirty_high,
                    (limit - visited) as i64
                ],
                |r| Ok((bounded_private_text(r, 0, 128)?, r.get::<_, i64>(1)?)),
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(private_read_error)?;
        for (id, ordinal) in &rows {
            let id = InvitationId::parse(id)
                .map_err(|_| corrupt("invalid private invitation identity"))?;
            visited += 1;
            state.dirty_after = *ordinal;
            result.push(Candidate {
                id,
                source: Source::Revisit,
            });
        }
        if visited < limit {
            state.dirty_after = state.dirty_high;
        }
    }
    if visited == limit {
        return Ok((result, visited));
    }
    if (state.revisit_after_bytes, state.revisit_after_ordinal)
        >= (state.revisit_high_bytes, state.revisit_high_ordinal)
    {
        let high=db.query_row("SELECT required_render_bytes,revisit_ordinal FROM invitation_presentation_items INDEXED BY invitation_presentation_revisit WHERE seat_id=?1 AND consumer=?2 AND generation=?3 AND required_render_bytes<=?4 ORDER BY required_render_bytes DESC,revisit_ordinal DESC LIMIT 1",params![claim.seat.as_str(),consumer.as_str(),state.generation,standalone as i64],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).optional().map_err(store_error)?;
        (state.revisit_high_bytes, state.revisit_high_ordinal) = high.unwrap_or((0, 0));
        state.revisit_after_bytes = 0;
        state.revisit_after_ordinal = 0;
    }
    let mut query=db.prepare("SELECT invitation_id,required_render_bytes,revisit_ordinal FROM invitation_presentation_items INDEXED BY invitation_presentation_revisit WHERE seat_id=?1 AND consumer=?2 AND generation=?3 AND (required_render_bytes,revisit_ordinal)>(?4,?5) AND (required_render_bytes,revisit_ordinal)<=(?6,?7) AND required_render_bytes<=?8 ORDER BY required_render_bytes,revisit_ordinal LIMIT ?9").map_err(store_error)?;
    let rows = query
        .query_map(
            params![
                claim.seat.as_str(),
                consumer.as_str(),
                state.generation,
                state.revisit_after_bytes,
                state.revisit_after_ordinal,
                state.revisit_high_bytes,
                state.revisit_high_ordinal,
                standalone as i64,
                (limit - visited) as i64
            ],
            |r| {
                Ok((
                    bounded_private_text(r, 0, 128)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(private_read_error)?;
    for (id, bytes, ordinal) in rows {
        let id =
            InvitationId::parse(id).map_err(|_| corrupt("invalid private invitation identity"))?;
        visited += 1;
        state.revisit_after_bytes = bytes;
        state.revisit_after_ordinal = ordinal;
        result.push(Candidate {
            id,
            source: Source::Revisit,
        });
    }
    if visited < limit {
        state.revisit_after_bytes = state.revisit_high_bytes;
        state.revisit_after_ordinal = state.revisit_high_ordinal;
    }
    Ok((result, visited))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Controls {
    counter: String,
    last_exact: Option<bool>,
    active_epoch: Option<String>,
    pending_attention: Option<crate::protocol::delivery::PresentationControlStampV2>,
    allocated_attention: Option<crate::protocol::delivery::PresentationControlStampV2>,
    pending_clear: Option<crate::protocol::delivery::PresentationControlStampV2>,
    confirmed_attention: Option<crate::protocol::delivery::PresentationControlStampV2>,
    confirmed_clear_epoch: Option<String>,
    attempted_epoch: Option<String>,
    fresh_since_control: bool,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            counter: "0".into(),
            last_exact: None,
            active_epoch: None,
            pending_attention: None,
            allocated_attention: None,
            pending_clear: None,
            confirmed_attention: None,
            confirmed_clear_epoch: None,
            attempted_epoch: None,
            fresh_since_control: false,
        }
    }
}
impl Controls {
    fn checked(&self) -> Result<(), ApiError> {
        use crate::protocol::delivery::{
            PresentationControlStampV2 as Stamp, valid_presentation_epoch,
        };
        if !valid_presentation_epoch(&self.counter)
            || [
                &self.active_epoch,
                &self.confirmed_clear_epoch,
                &self.attempted_epoch,
            ]
            .iter()
            .any(|v| v.as_ref().is_some_and(|s| !valid_presentation_epoch(s)))
            || [
                &self.pending_attention,
                &self.allocated_attention,
                &self.pending_clear,
                &self.confirmed_attention,
            ]
            .iter()
            .any(|v| {
                v.as_ref().is_some_and(|s| match s {
                    Stamp::Attention { epoch, .. } | Stamp::Cleared { epoch, .. } => {
                        !valid_presentation_epoch(epoch)
                    }
                })
            })
        {
            return Err(corrupt("invalid presentation control epoch"));
        }
        Ok(())
    }
    fn open(&mut self, version: u64) -> Result<(), ApiError> {
        use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
        self.checked()?;
        if self.last_exact != Some(true) {
            let next = self
                .counter
                .parse::<u64>()
                .map_err(|_| corrupt("invalid presentation counter"))?
                .checked_add(1)
                .ok_or_else(|| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "presentation episode exhausted",
                    )
                })?;
            self.counter = next.to_string();
            self.active_epoch = Some(self.counter.clone());
            self.last_exact = Some(true);
            self.pending_attention = Some(Stamp::Attention {
                epoch: self.counter.clone(),
                version,
            });
        } else if self.allocated_attention.as_ref().is_none_or(
            |v| !matches!(v,Stamp::Attention{epoch,..} if Some(epoch)==self.active_epoch.as_ref()),
        ) {
            self.pending_attention = self.active_epoch.as_ref().map(|epoch| Stamp::Attention {
                epoch: epoch.clone(),
                version,
            });
        }
        Ok(())
    }
    fn absent(&mut self) {
        use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
        self.last_exact = Some(false);
        self.active_epoch = None;
        self.pending_attention = None;
        if let Some(Stamp::Attention { epoch, version }) = &self.allocated_attention
            && self.confirmed_clear_epoch.as_ref() != Some(epoch)
        {
            self.pending_clear = Some(Stamp::Cleared {
                epoch: epoch.clone(),
                version: *version,
            });
        }
    }
    fn pending(&self) -> Option<crate::protocol::delivery::PresentationControlStampV2> {
        self.pending_clear
            .clone()
            .or_else(|| self.pending_attention.clone())
    }
    fn allocated(&mut self, stamp: &crate::protocol::delivery::PresentationControlStampV2) {
        use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
        match stamp {
            Stamp::Attention { epoch, .. } => {
                self.allocated_attention = Some(stamp.clone());
                self.attempted_epoch = Some(epoch.clone());
                self.pending_attention = None;
            }
            Stamp::Cleared { .. } => {}
        }
        self.fresh_since_control = false;
    }
    fn applicable(&self, stamp: &crate::protocol::delivery::PresentationControlStampV2) -> bool {
        use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
        match stamp {
            Stamp::Attention { epoch, .. } => {
                self.last_exact == Some(true) && self.active_epoch.as_ref() == Some(epoch)
            }
            Stamp::Cleared { epoch, .. } => {
                self.confirmed_clear_epoch.as_ref() == Some(epoch)
                    || matches!(&self.pending_clear,Some(Stamp::Cleared{epoch:pending,..}) if pending==epoch)
            }
        }
    }
    fn confirm(&mut self, stamp: &crate::protocol::delivery::PresentationControlStampV2) {
        use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
        match stamp {
            Stamp::Attention { epoch, .. }
                if self.last_exact == Some(true) && self.active_epoch.as_ref() == Some(epoch) =>
            {
                self.confirmed_attention = Some(stamp.clone())
            }
            Stamp::Cleared { epoch, .. } => {
                self.confirmed_clear_epoch = Some(epoch.clone());
                if matches!(&self.pending_clear,Some(Stamp::Cleared{epoch:pending,..}) if pending==epoch)
                {
                    self.pending_clear = None;
                }
                if matches!(&self.allocated_attention,Some(Stamp::Attention{epoch:allocated,..}) if allocated==epoch)
                {
                    self.allocated_attention = None;
                }
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingMessage {
    content: PresentationMessage,
    marker_lazy: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrdinaryCarry {
    prefix: Vec<String>,
    items: Vec<PendingMessage>,
}
const ORDINARY_CARRY_BYTES: usize = 155731;
fn ordinary_carry(json: &str) -> Result<OrdinaryCarry, ApiError> {
    if json.len() > ORDINARY_CARRY_BYTES {
        return Err(corrupt("ordinary carry serialized bound"));
    }
    let carry: OrdinaryCarry = if json == "[]" {
        OrdinaryCarry {
            prefix: Vec::new(),
            items: Vec::new(),
        }
    } else {
        decode(json)?
    };
    if carry.items.len() > 32
        || carry.prefix.len() > 8
        || carry.prefix.iter().map(String::len).sum::<usize>() > 4096
        || carry.items.iter().any(|v| {
            let m = &v.content.content;
            let sender = m.sender.as_ref().map_or("service", |id| id.as_str());
            m.body.len() > 8192
                || m.body_len > 65536
                || m.body.len() as u64 > m.body_len
                || m.truncated != (m.body_len > 8192)
                || m.sender_name.as_deref() != Some(sender)
                || m.thread_name
                    .as_ref()
                    .is_some_and(|name| name.chars().count() > 121)
                || (v.content.lazy && !v.marker_lazy)
                || (v.marker_lazy && m.ack_required)
        })
    {
        return Err(corrupt("invalid ordinary raw carry shape"));
    }
    Ok(carry)
}
/// Invert only the exact marker generated for this source variant and prefix.
pub(super) fn remove_generated_marker(
    body: &str,
    prefix: &[String],
    id: &MessageId,
    marker_lazy: bool,
) -> Result<String, ApiError> {
    let marker = crate::protocol::watch::truncation_marker_for(prefix, id, marker_lazy);
    body.strip_suffix(&marker)
        .map(str::to_owned)
        .ok_or_else(|| corrupt("ordinary generated marker mismatch"))
}
fn ordinary_carry_json(carry: &OrdinaryCarry) -> Result<String, ApiError> {
    let json = encode(carry)?;
    ordinary_carry(&json)?;
    Ok(json)
}

fn new_token(db: &Connection) -> Result<String, ApiError> {
    for _ in 0..8 {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let used:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM invitation_presentation_consumers WHERE outbox_token=?1 UNION ALL SELECT 1 FROM invitation_presentation_consumers WHERE last_token=?1)",[&token],|r|r.get(0)).map_err(store_error)?;
        if !used {
            return Ok(token);
        }
    }
    Err(api_error(
        ErrorCode::StoreBusy,
        "presentation token collision",
    ))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenPresentation {
    item: crate::protocol::delivery::PresentationItemV2,
    context_key: Option<String>,
}
fn frozen_profile(
    input: &PresentationInput,
) -> Result<crate::delivery::presentation::PresentationRenderProfile, ApiError> {
    use crate::{
        delivery::presentation::PresentationRenderProfile as Profile,
        protocol::delivery::PresentationRenderSelection as Selection,
    };
    let frozen = input
        .frozen_render
        .as_ref()
        .ok_or_else(|| corrupt("missing validated frozen render"))?;
    match &frozen.identity.selection {
        Selection::WatchJsonLinesV1 => Ok(Profile::WatchJsonLinesV1),
        Selection::Hook { selector } => {
            let codec = frozen
                .codec
                .ok_or_else(|| corrupt("missing frozen registered codec"))?;
            if crate::delivery::presentation::descriptor_key(codec)
                != frozen.identity.descriptor_key
            {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "frozen presentation codec changed",
                ));
            }
            Ok(Profile::Hook {
                confirmable_selector: *selector,
                passive_selector: *selector,
                confirmable: codec,
                passive: codec,
            })
        }
    }
}
struct PageContent {
    items: Vec<crate::protocol::delivery::PresentationItemV2>,
    offered: Vec<MessageId>,
    truncated: Vec<MessageId>,
}
fn page(
    state: &State,
    consumer: PresentationConsumer,
    content: PageContent,
    version: u64,
    more: bool,
    profile: &crate::delivery::presentation::PresentationRenderProfile,
    sizer: &mut crate::delivery::presentation::Sizer,
) -> Result<DeliveryPullReplyV2, ApiError> {
    let PageContent {
        items,
        offered,
        truncated,
    } = content;
    let (render, bytes) = sizer.measure(profile, &items).map_err(|_| {
        api_error(
            ErrorCode::Conflict,
            "presentation final output exceeds bound",
        )
    })?;
    let text = if consumer == PresentationConsumer::Hook {
        crate::delivery::presentation::presentation_hook_text(&items)
    } else {
        String::new()
    };
    if text.len() > 65536 {
        return Err(corrupt("presentation plaintext bound"));
    }
    let emitted_controls = crate::delivery::presentation::emitted_controls(&items);
    Ok(DeliveryPullReplyV2::Page(DeliveryPageV2 {
        attention_version: version,
        text,
        offered,
        truncated,
        more,
        presentation_token: state.outbox_token.clone(),
        items,
        render,
        output_bytes: bytes as u32,
        emitted_controls,
    }))
}
fn replay(
    db: &Connection,
    input: &PresentationInput,
    state: &mut State,
    work: &mut PresentationWork,
    sizer: &mut crate::delivery::presentation::Sizer,
    relevance: super::presentation_relevance::Relevance,
    relevance_more: bool,
) -> Result<Option<DeliveryPullReplyV2>, ApiError> {
    use crate::protocol::delivery::PresentationItemV2 as Item;
    if state.outbox_token.is_none() {
        return Ok(None);
    }
    let descriptors: Vec<FrozenPresentation> = decode(&state.outbox_descriptors)?;
    let (mut retired, confirmed) = checked_masks(
        &descriptors,
        state.outbox_retired_mask,
        state.outbox_confirmed_mask,
    )?;
    let ids = descriptors
        .iter()
        .enumerate()
        .filter(|(slot, _)| retired & (1u32 << slot) == 0)
        .filter_map(|(_, v)| match &v.item {
            Item::Invitation { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let current = hydrate_many(db, &input.pull.claim, &ids)?;
    let mut kept = Vec::new();
    let controls: Controls = decode(&state.control_state)?;
    controls.checked()?;
    for (slot, descriptor) in descriptors.iter().enumerate() {
        if retired & (1u32 << slot) != 0 {
            continue;
        }
        work.replay += 1;
        // Unknown cannot erase an allocated, possibly emitted Attention. Keep
        // its original slot and token, but do not physically replay it.
        if matches!(&descriptor.item, Item::Attention { .. })
            && relevance == super::presentation_relevance::Relevance::Unknown
        {
            continue;
        }
        let valid=match &descriptor.item {
            Item::Invitation{id,..}=>current.get(id).is_some_and(|(context,_)|descriptor.context_key.as_deref()==Some(context.key().as_str())),
            Item::Notice{warning,thread,..}=>db.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND thread_id=?2 AND kind='warn')",params![warning.as_str(),thread.as_str()],|r|r.get::<_,bool>(0)).map_err(store_error)?,
            Item::Attention{..}|Item::AttentionCleared{..}=>controls.applicable(&control_stamp(&descriptor.item).ok_or_else(||corrupt("missing presentation stamp"))?),
            Item::Message{..}=>return Err(corrupt("ordinary message in recovery outbox")),
        };
        if valid {
            kept.push(descriptor.clone());
        } else {
            retired |= 1u32 << slot;
        }
    }
    state.outbox_retired_mask = i64::from(retired);
    state.outbox_confirmed_mask = i64::from(confirmed);
    if retired == descriptor_mask(descriptors.len())? {
        retire_outbox(state)?;
        return Ok(None);
    }
    let profile = frozen_profile(input)?;
    let items = kept.iter().map(|v| v.item.clone()).collect::<Vec<_>>();
    state.outbox_text = crate::delivery::presentation::presentation_hook_text(&items);
    Ok(Some(page(
        state,
        input.pull.consumer,
        PageContent {
            items,
            offered: Vec::new(),
            truncated: Vec::new(),
        },
        input.attention_version,
        state.outbox_more != 0
            || relevance_more
            || relevance == super::presentation_relevance::Relevance::Unknown,
        &profile,
        sizer,
    )?))
}
/// Reconcile only the bounded offer snapshot, never the full pending queue.
/// Canonical terminal facts release capacity; missing/ambiguous facts retain it.
fn reclaim_terminal_offers(
    db: &Connection,
    input: &mut PresentationInput,
    work: &mut PresentationWork,
) -> Result<Vec<MessageId>, ApiError> {
    use crate::delivery::ledger::MAX_OFFERS_PER_SEAT;
    if input.known_messages.len() > MAX_OFFERS_PER_SEAT
        || input.blocked_messages.len() > MAX_OFFERS_PER_SEAT
        || input.new_message_slots > MAX_OFFERS_PER_SEAT
    {
        return Err(ApiError::invalid_request("ordinary offer snapshot bound"));
    }
    let claim = &input.pull.claim;
    let mut known = input
        .known_messages
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut reclaimed = Vec::new();
    for id in &known {
        work.offer_reconciliation_visits += 1;
        work.offer_reconciliation_probes += 1;
        let mode: Option<String> = db.query_row(
            "SELECT delivery_mode FROM messages WHERE id=?1 AND instance_id=?2 AND kind='ordinary'",
            params![id.as_str(), claim.instance], |r| r.get(0),
        ).optional().map_err(store_error)?;
        let terminal = match mode.as_deref() {
            Some("lazy") => {
                // One unique recipient lookup and one manifest unique-key lookup.
                work.offer_reconciliation_probes += 2;
                let state: Option<String> = db.query_row(
                    "SELECT r.state FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id AND sm.thread_id=r.thread_id WHERE r.message_id=?1 AND r.seat_id=?2 AND sm.instance_id=?3",
                    params![id.as_str(), claim.seat.as_str(), claim.instance], |r| r.get(0),
                ).optional().map_err(store_error)?;
                state.as_deref() == Some("displayed")
            }
            Some("ordinary") => {
                // Logical addressed authority wins even when its sparse state
                // is absent. Charge all three unique lookups including misses.
                work.offer_reconciliation_probes += 3;
                let logical: Option<(String, bool)> = db.query_row(
                    "SELECT COALESCE(rs.state,'pending'),MIN(pr.ack_required,COALESCE(rs.ack_required,1))=0 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=?2 LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE sm.message_id=?1 AND sm.instance_id=?3",
                    params![id.as_str(), claim.seat.as_str(), claim.instance], |r| Ok((r.get(0)?,r.get(1)?)),
                ).optional().map_err(store_error)?;
                let state = if logical.is_some() {
                    logical
                } else {
                    work.offer_reconciliation_probes += 1;
                    db.query_row(
                        "SELECT state,ack_required=0 FROM receipts WHERE message_id=?1 AND seat_id=?2",
                        params![id.as_str(), claim.seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?)),
                    ).optional().map_err(store_error)?
                };
                state.is_some_and(|(state, not_required)| {
                    matches!(state.as_str(), "acked" | "recipient_retired")
                        || (state == "pending" && not_required)
                })
            }
            _ => false,
        };
        if terminal {
            reclaimed.push(id.clone());
        }
    }
    for id in &reclaimed {
        known.remove(id);
    }
    // The delta follows the sorted snapshot; membership needs no quadratic walk.
    input
        .blocked_messages
        .retain(|id| reclaimed.binary_search(id).is_err());
    input.new_message_slots = input
        .new_message_slots
        .saturating_add(reclaimed.len())
        .min(MAX_OFFERS_PER_SEAT - known.len());
    input.known_messages = known.into_iter().collect();
    Ok(reclaimed)
}

fn validate_due_snapshot(input: &PresentationInput) -> Result<(), ApiError> {
    use crate::delivery::ledger::MAX_OFFERS_PER_SEAT;
    let Some(snapshot) = &input.ordinary_due_snapshot else {
        return Ok(());
    };
    let known = input
        .known_messages
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let blocked = input
        .blocked_messages
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if known.len() != input.known_messages.len()
        || blocked.len() != input.blocked_messages.len()
        || !blocked.is_subset(&known)
        || snapshot.stamps.len() > MAX_OFFERS_PER_SEAT
        || snapshot
            .stamps
            .keys()
            .collect::<std::collections::BTreeSet<_>>()
            != known
        || snapshot.generation != input.pull.claim.binding_generation
        || snapshot.native_session != input.pull.claim.native_session.as_str()
        || snapshot
            .cache
            .scope
            .as_ref()
            .is_some_and(|scope| scope.passive_generation > i64::MAX as u64)
        || snapshot.stamps.values().any(|stamp| stamp.count == 0)
        || snapshot.cache.deferred.len() > MAX_OFFERS_PER_SEAT
        || snapshot
            .cache
            .deferred
            .iter()
            .any(|(id, stamp)| snapshot.stamps.get(id) != Some(stamp))
        || (snapshot.cache.scope.is_none() && !snapshot.cache.deferred.is_empty())
    {
        return Err(ApiError::invalid_request("invalid ordinary due snapshot"));
    }
    Ok(())
}

fn validate_owner_replay(input: &PresentationInput) -> Result<(), ApiError> {
    let Some(replay) = &input.owner_replay else {
        return Ok(());
    };
    let valid = input.ordinary_due_snapshot.as_ref().is_some_and(|known| {
        input.pull.consumer == crate::protocol::delivery::PresentationConsumer::Watch
            && replay.scope.seat == input.pull.claim.seat
            && replay.scope.generation == input.pull.claim.binding_generation
            && replay.scope.native_session == input.pull.claim.native_session.as_str()
            && replay.scope.execution == input.pull.claim.execution
            && Some(replay.scope.lease.as_str()) == input.pull.presentation_lease.as_deref()
            && replay.scope.incarnation == known.incarnation
            && replay.remaining.len() <= crate::delivery::ledger::MAX_OFFERS_PER_SEAT
            && (replay.active || replay.remaining.is_empty())
            && replay.remaining.iter().all(|(id, stamp)| {
                stamp.count != 0
                    && MessageId::parse(id.as_str()).is_ok()
                    && known.stamps.get(id) == Some(stamp)
            })
    });
    if !valid {
        return Err(ApiError::invalid_request("invalid ordinary due snapshot"));
    }
    Ok(())
}

fn due_update(
    input: &PresentationInput,
    state: &State,
    render_key: &str,
) -> Result<Option<crate::delivery::ledger::OrdinaryDueSnapshot>, ApiError> {
    let Some(mut update) = input.ordinary_due_snapshot.clone() else {
        return Ok(None);
    };
    let mut key = [0; 32];
    if render_key.len() != 64 || !render_key.is_ascii() {
        return Err(corrupt("invalid render key"));
    }
    for (n, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&render_key[n * 2..n * 2 + 2], 16)
            .map_err(|_| corrupt("invalid render key"))?;
    }
    let scope = crate::delivery::ledger::OrdinaryDueScope {
        execution: input.pull.claim.execution.clone(),
        ordinary_version: input.ordinary_version,
        passive_generation: state.ordinary_passive_generation as u64,
        render_key: key,
    };
    if update.cache.scope.as_ref() != Some(&scope) {
        update.cache.deferred.clear();
        update.cache.scope = Some(scope);
    }
    update.cache.deferred.retain(|id, stamp| {
        input.known_messages.contains(id) && update.stamps.get(id) == Some(stamp)
    });
    Ok(Some(update))
}

fn due_keys(
    input: &PresentationInput,
    update: Option<&crate::delivery::ledger::OrdinaryDueSnapshot>,
    consumed: &[MessageId],
) -> Vec<MessageId> {
    if !input.retry_messages {
        return Vec::new();
    }
    let Some(update) = update else {
        return Vec::new();
    };
    input
        .known_messages
        .iter()
        .filter(|id| {
            !input.blocked_messages.contains(id)
                && !update.cache.deferred.contains_key(id)
                && !consumed.contains(id)
        })
        .cloned()
        .collect()
}

fn ordinary_runnable(
    input: &PresentationInput,
    state: &State,
    slots: usize,
    update: Option<&crate::delivery::ledger::OrdinaryDueSnapshot>,
    consumed: &[MessageId],
) -> Result<bool, ApiError> {
    if slots == 0 {
        return Ok(!due_keys(input, update, consumed).is_empty());
    }
    Ok(!ordinary_carry(&state.ordinary_pending)?.items.is_empty()
        || state.ordinary_done == 0
        || state.ordinary_scan_passive_generation != state.ordinary_passive_generation
        || state.ordinary_version != input.ordinary_version.to_string())
}

fn assemble_message(
    assembler: &mut Assembler,
    value: crate::protocol::results::InboxBatchV2Item,
    prefix: &[String],
) -> Result<Option<PendingMessage>, ApiError> {
    let marker_lazy = matches!(
        &value,
        crate::protocol::results::InboxBatchV2Item::LazyMessage { .. }
    );
    if let Assembled::Item(mut item) = assembler
        .push(value)
        .map_err(|_| corrupt("invalid ordinary body chunks"))?
    {
        if item.message.truncated {
            item.message.body =
                remove_generated_marker(&item.message.body, prefix, &item.id, marker_lazy)?;
        }
        Ok(Some(PendingMessage {
            marker_lazy,
            content: PresentationMessage {
                id: item.id,
                lazy: item.kind == DeliveryKind::Lazy,
                content: item.message,
            },
        }))
    } else {
        Ok(None)
    }
}

fn message_output(
    message: &PendingMessage,
    prefix: &[String],
) -> crate::protocol::delivery::PresentationItemV2 {
    let mut content = message.content.content.clone();
    if content.truncated {
        content
            .body
            .push_str(&crate::protocol::watch::truncation_marker_for(
                prefix,
                &message.content.id,
                message.marker_lazy,
            ));
    }
    crate::protocol::delivery::PresentationItemV2::Message {
        id: message.content.id.clone(),
        lazy: message.content.lazy,
        content,
    }
}

pub(super) fn pull(
    db: &Connection,
    context: &StoreContext,
    mut input: PresentationInput,
    budget: &CallBudget,
) -> Result<PresentationOutput, ApiError> {
    use crate::{
        delivery::presentation::{self, PresentationRenderProfile},
        protocol::delivery::{PresentationItemV2 as OutputItem, PresentationRenderSelection},
    };
    input.pull.validate().map_err(ApiError::invalid_request)?;
    validate_due_snapshot(&input)?;
    validate_owner_replay(&input)?;
    if !input.render_profile.validate() {
        return Err(ApiError::invalid_request("invalid presentation profile"));
    }
    let claim = &input.pull.claim;
    let consumer = input.pull.consumer;
    if !scope_matches(db, claim)? {
        return Ok(PresentationOutput {
            reply: DeliveryPullReplyV2::Refused {
                reason: crate::protocol::delivery::DeliveryRefusalReason::SessionMismatch,
            },
            work: PresentationWork::default(),
            reclaimed_messages: Vec::new(),
            ordinary_due_update: None,
            owner_replay: Default::default(),
        });
    }
    if render_selection(db, claim, consumer)?.as_ref()
        != input.frozen_render.as_ref().map(|v| &v.identity)
    {
        return Ok(PresentationOutput {
            reply: DeliveryPullReplyV2::Refused {
                reason: crate::protocol::delivery::DeliveryRefusalReason::Busy,
            },
            work: PresentationWork::default(),
            reclaimed_messages: Vec::new(),
            ordinary_due_update: None,
            owner_replay: Default::default(),
        });
    }
    if let Some(frozen) = &input.frozen_render
        && matches!(
            frozen.identity.selection,
            PresentationRenderSelection::Hook { .. }
        )
    {
        frozen_profile(&input)?;
    }
    let mut work = PresentationWork::default();
    let reclaimed_messages = reclaim_terminal_offers(db, &mut input, &mut work)?;
    let mut owner_progress = crate::ports::WatchReplayProgress::default();
    let recovery_active = input.owner_replay.as_ref().is_some_and(|v| v.active);
    if recovery_active {
        owner_progress.source = crate::ports::OrdinaryPresentationSource::RecoveryControlOnly;
    }
    if let Some(replay) = &mut input.owner_replay {
        replay.remaining.retain(|id, stamp| {
            if reclaimed_messages.contains(id) {
                owner_progress.completed.insert(id.clone(), *stamp);
                false
            } else {
                true
            }
        });
    }
    let owner_pending = input
        .owner_replay
        .as_ref()
        .is_some_and(|v| !v.remaining.is_empty());
    let claim = &input.pull.claim;
    let mut state = load(db, claim, consumer)?;
    let mut sizer = presentation::Sizer::new();
    let publications = advance_ordinary_publications(db, claim, &mut state)?;
    work.ordinary_publication_visits = publications.visits;
    work.ordinary_publication_probes = publications.probes;
    work.ordinary_publication_seeks = publications.seeks;
    let frontier = super::attention::notice_frontier(db, claim.seat.as_str())?;
    let proof = super::presentation_relevance::advance(db, claim, consumer, frontier, budget)?;
    work.relevance_calls = 1;
    work.relevance_sources = usize::from(proof.work.sources);
    work.relevance_fallback = usize::from(proof.work.fallback);
    work.relevance_point_probes = usize::from(proof.work.point_probes);
    work.relevance_seeks = usize::from(proof.work.highwater_seeks);
    work.relevance_witness_probes = usize::from(proof.work.witness_probes);
    let mut controls: Controls = decode(&state.control_state)?;
    controls.checked()?;
    match proof.relevance {
        super::presentation_relevance::Relevance::Open => controls.open(input.attention_version)?,
        super::presentation_relevance::Relevance::Absent => controls.absent(),
        super::presentation_relevance::Relevance::Unknown => {}
    }
    state.control_state = encode(&controls)?;
    let render_key = input.render_profile.render_key(&input.pull.marker_prefix);
    let mut ordinary_due_update = due_update(&input, &state, &render_key)?;
    let replay_ordinary = ordinary_runnable(
        &input,
        &state,
        input.new_message_slots,
        ordinary_due_update.as_ref(),
        &[],
    )?;
    if let Some(reply) = replay(
        db,
        &input,
        &mut state,
        &mut work,
        &mut sizer,
        proof.relevance,
        proof.more || publications.more || replay_ordinary || owner_pending,
    )? {
        save(db, claim, consumer, &state)?;
        owner_progress.snapshot = input.owner_replay.take();
        return finish(
            reply,
            work,
            &mut sizer,
            reclaimed_messages,
            ordinary_due_update,
            owner_progress,
        );
    }
    if state.render_key != render_key {
        state.render_key = render_key.clone();
        state.dirty_after = 0;
        state.dirty_high = pending_high(db, claim)?.max(state.initial_high);
        state.revisit_after_bytes = 0;
        state.revisit_after_ordinal = 0;
        state.revisit_high_bytes = 0;
        state.revisit_high_ordinal = 0;
    }
    // A previously deferred control gets a separate opportunity before fresh
    // discovery. Unknown withholds Attention; a known retired clear survives it.
    let pending_control = controls.pending().filter(|stamp| {
        !matches!(
            stamp,
            crate::protocol::delivery::PresentationControlStampV2::Attention { .. }
        ) || proof.relevance == super::presentation_relevance::Relevance::Open
    });
    let control_only = controls.fresh_since_control && pending_control.is_some();
    let standalone = 65536;
    let initial = if control_only {
        Vec::new()
    } else {
        initial_candidates(db, claim, &mut state, 40)?
    };
    work.initial = initial.len();
    let (changed, visited) = if control_only {
        (Vec::new(), 0)
    } else {
        context_candidates(db, claim, &mut state, 40 + (40 - work.initial))?
    };
    work.context = visited;
    let (revisited, visited) = if control_only {
        (Vec::new(), 0)
    } else {
        revisit_candidates(
            db,
            claim,
            consumer,
            &mut state,
            100 - work.initial - work.context,
            standalone,
        )?
    };
    work.revisit = visited;
    let candidates = initial
        .into_iter()
        .chain(changed)
        .chain(revisited)
        .collect::<Vec<_>>();
    let ids = candidates.iter().map(|v| v.id.clone()).collect::<Vec<_>>();
    let hydrated = hydrate_many(db, claim, &ids)?;
    let mut invitation_rows = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.id.clone()) {
            continue;
        }
        let Some((invitation, ordinal)) = hydrated.get(&candidate.id) else {
            continue;
        };
        let full = OutputItem::Invitation {
            id: invitation.id.clone(),
            content: invitation.watch(&input.pull.marker_prefix, false),
        };
        let full_size = sizer
            .measure(&input.render_profile, std::slice::from_ref(&full))
            .map_or_else(|bytes| bytes, |(_, bytes)| bytes);
        let row = item(
            db,
            claim,
            consumer,
            invitation,
            *ordinal,
            ItemRender {
                key: &render_key,
                required_bytes: full_size,
            },
            candidate.source,
        )?;
        if row.complete == 0 {
            let partial = full_size > 65536;
            if partial && row.partial == 1 {
                continue;
            }
            let output = if partial {
                OutputItem::Invitation {
                    id: invitation.id.clone(),
                    content: invitation.watch(&input.pull.marker_prefix, true),
                }
            } else {
                full
            };
            if !partial
                || sizer
                    .measure(&input.render_profile, std::slice::from_ref(&output))
                    .is_ok()
            {
                invitation_rows.push((row.source, invitation.clone(), row, output, partial));
            }
        }
    }
    let mut carry = ordinary_carry(&state.ordinary_pending)?;
    let mut messages = std::mem::take(&mut carry.items);
    if messages.len() > 32 {
        return Err(corrupt("ordinary presentation carry bound"));
    }
    // A walk freezes its native/passive tuple at START. A later publication
    // never resets an active body/carry or consumes its next capture signal.
    let owner_mode = owner_pending;
    let normal_mode = !owner_mode && input.new_message_slots > 0;
    owner_progress.source = if control_only {
        owner_progress.source
    } else if owner_mode {
        crate::ports::OrdinaryPresentationSource::OwnerReplay
    } else if normal_mode {
        crate::ports::OrdinaryPresentationSource::Normal
    } else {
        crate::ports::OrdinaryPresentationSource::LogicalDue
    };
    if normal_mode
        && !control_only
        && state.ordinary_done == 1
        && messages.is_empty()
        && (state.ordinary_version != input.ordinary_version.to_string()
            || state.ordinary_scan_passive_generation != state.ordinary_passive_generation
            || input.retry_messages)
    {
        state.ordinary_cursor = None;
        state.ordinary_done = 0;
        state.ordinary_partial = "{}".into();
        state.ordinary_skip_body = None;
    }
    if normal_mode && !control_only && messages.is_empty() && state.ordinary_done == 0 {
        if state.ordinary_cursor.is_none() {
            state.ordinary_version = input.ordinary_version.to_string();
            state.ordinary_scan_passive_generation = state.ordinary_passive_generation;
        }
        let (ordinary, visited) = super::queries::ordinary_presentation_page(
            db,
            context,
            &claim.instance,
            &claim.seat,
            state.ordinary_cursor.clone(),
            budget,
        )?;
        work.ordinary = visited;
        let mut assembler =
            Assembler::new(None).with_command_prefix(input.pull.marker_prefix.clone());
        assembler.partial = partial_state(&state.ordinary_partial)?;
        if let Some(id) = &state.ordinary_skip_body {
            MessageId::parse(id).map_err(|_| corrupt("invalid ordinary skipped body identity"))?;
            let current = state
                .ordinary_cursor
                .as_ref()
                .map(|cursor| {
                    crate::protocol::pagination::InboxBatchV2CursorState::decode_for(
                        cursor,
                        &claim.instance,
                        &claim.seat,
                    )
                })
                .transpose()
                .map_err(corrupt)?;
            if current
                .as_ref()
                .and_then(|v| v.body.as_ref())
                .map(|v| v.message.as_str())
                != Some(id.as_str())
                || !assembler.partial.is_empty()
            {
                return Err(corrupt("invalid ordinary skipped body cursor"));
            }
            assembler.emitted.insert(id.clone());
        }
        carry.prefix = input.pull.marker_prefix.clone();
        for value in ordinary.items {
            if let Some(message) = assemble_message(&mut assembler, value, &carry.prefix)? {
                messages.push(message);
            }
        }
        state.ordinary_partial = partial_json(&assembler.partial)?;
        let continued = ordinary
            .next_cursor
            .as_ref()
            .map(|cursor| {
                crate::protocol::pagination::InboxBatchV2CursorState::decode_for(
                    cursor,
                    &claim.instance,
                    &claim.seat,
                )
            })
            .transpose()
            .map_err(corrupt)?;
        state.ordinary_skip_body = continued
            .and_then(|v| v.body)
            .map(|v| v.message.as_str().to_owned())
            .filter(|id| assembler.emitted.contains(id));
        state.ordinary_cursor = ordinary.next_cursor;
        state.ordinary_done = i64::from(!ordinary.has_more);
    }

    let notice_page = if control_only {
        Vec::new()
    } else {
        super::attention::notice_offer_page(db, claim.seat.as_str(), 17)?
    };
    work.notices = notice_page.len();
    let notices = notice_page
        .into_iter()
        .take(16)
        .map(|notice| {
            let output = OutputItem::Notice {
                warning: notice.notice.warning.clone(),
                thread: notice.notice.thread.clone(),
                body: crate::protocol::watch::notice_marker_for(
                    &input.pull.marker_prefix,
                    &notice.notice.warning,
                ),
            };
            (notice, output)
        })
        .collect::<Vec<_>>();
    let mut outputs = Vec::new();
    let mut offered = Vec::new();
    let mut truncated = Vec::new();
    let mut descriptors = Vec::new();
    let mut carried_notices = Vec::new();
    let mut shown_message_count = 0;
    let mut message_slots = input.new_message_slots;
    for n in 0..if control_only { 0 } else { 5 } {
        let class = (state.rotation as usize + n) % 5;
        if class == 0 && !normal_mode {
            let candidates = if owner_mode {
                input
                    .owner_replay
                    .as_ref()
                    .expect("owner mode snapshot")
                    .remaining
                    .keys()
                    .cloned()
                    .collect()
            } else {
                due_keys(&input, ordinary_due_update.as_ref(), &[])
            };
            if !candidates.is_empty() && outputs.len() < 32 {
                let due_context = super::queries::ordinary_due_context(
                    db,
                    &claim.instance,
                    &claim.seat,
                    &mut work,
                )?;
                for id in candidates.into_iter().take(100) {
                    if outputs.len() == 32 || work.ordinary_due_hydrated == 32 {
                        break;
                    }
                    if budget.is_exhausted(context.clock()) {
                        return Err(api_error(
                            ErrorCode::ReadBudgetExhausted,
                            "ordinary due budget exhausted",
                        ));
                    }
                    work.ordinary += 1;
                    let value = super::queries::ordinary_presentation_message_by_id(
                        db,
                        &claim.instance,
                        &claim.seat,
                        &id,
                        due_context,
                        &mut work,
                    )?;
                    let stamp = ordinary_due_update
                        .as_ref()
                        .expect("exact keys require snapshot")
                        .stamps[&id];
                    let Some(value) = value else {
                        if owner_mode {
                            owner_progress.completed.insert(id, stamp);
                        } else {
                            ordinary_due_update
                                .as_mut()
                                .expect("exact snapshot")
                                .cache
                                .deferred
                                .insert(id, stamp);
                        }
                        continue;
                    };
                    work.ordinary_due_hydrated += 1;
                    let mut assembler =
                        Assembler::new(None).with_command_prefix(input.pull.marker_prefix.clone());
                    let message =
                        assemble_message(&mut assembler, value, &input.pull.marker_prefix)?
                            .ok_or_else(|| corrupt("incomplete exact ordinary body"))?;
                    let output = message_output(&message, &input.pull.marker_prefix);
                    let mut trial = outputs.clone();
                    trial.push(output.clone());
                    if sizer.measure(&input.render_profile, &trial).is_err() {
                        if !outputs.is_empty() {
                            break;
                        }
                        if owner_mode {
                            owner_progress.completed.insert(id, stamp);
                        } else {
                            ordinary_due_update
                                .as_mut()
                                .expect("exact snapshot")
                                .cache
                                .deferred
                                .insert(id, stamp);
                        }
                        continue;
                    }
                    if owner_mode {
                        owner_progress.completed.insert(id.clone(), stamp);
                        owner_progress.emitted.insert(id.clone(), stamp);
                    }
                    outputs.push(output);
                    if message.content.content.truncated {
                        truncated.push(id);
                    } else {
                        offered.push(id);
                    }
                }
            }
        } else if class == 0 {
            for message in &messages {
                if input.blocked_messages.contains(&message.content.id)
                    || (!input.retry_messages && input.known_messages.contains(&message.content.id))
                {
                    shown_message_count += 1;
                    continue;
                }
                if outputs.len() == 32 {
                    break;
                }
                if !input.known_messages.contains(&message.content.id) && message_slots == 0 {
                    break;
                }
                let output = message_output(message, &carry.prefix);
                let mut trial = outputs.clone();
                trial.push(output.clone());
                if sizer.measure(&input.render_profile, &trial).is_err() {
                    break;
                }
                if !input.known_messages.contains(&message.content.id) {
                    message_slots -= 1;
                }
                outputs.push(output);
                shown_message_count += 1;
                if message.content.content.truncated {
                    truncated.push(message.content.id.clone());
                } else {
                    offered.push(message.content.id.clone());
                }
            }
        } else if class == 1 {
            for (notice, output) in &notices {
                if outputs.len() == 32 {
                    break;
                }
                let mut trial = outputs.clone();
                trial.push(output.clone());
                if sizer.measure(&input.render_profile, &trial).is_err() {
                    continue;
                }
                outputs.push(output.clone());
                descriptors.push(FrozenPresentation {
                    item: output.clone(),
                    context_key: None,
                });
                carried_notices.push(notice.notice.clone());
            }
        } else {
            let source = match class {
                2 => Source::Initial,
                3 => Source::Context,
                _ => Source::Revisit,
            };
            // Full-fitting candidates receive the rotating class's first opportunity.
            for partial_first in [false, true] {
                for (origin, invitation, row, output, partial) in &invitation_rows {
                    if *origin != source || *partial != partial_first || outputs.len() == 32 {
                        continue;
                    }
                    let mut trial = outputs.clone();
                    trial.push(output.clone());
                    if sizer.measure(&input.render_profile, &trial).is_err() {
                        continue;
                    }
                    outputs.push(output.clone());
                    descriptors.push(FrozenPresentation {
                        item: output.clone(),
                        context_key: Some(row.key.clone()),
                    });
                    db.execute("UPDATE invitation_presentation_items SET output_class=CASE WHEN ?4=1 THEN 4 ELSE output_class END,partial_count=MAX(partial_count,?4),complete_count=MAX(complete_count,?5),required_render_bytes=CASE WHEN ?5=1 THEN ?6 ELSE required_render_bytes END WHERE seat_id=?1 AND consumer=?2 AND invitation_id=?3 AND generation=?7 AND execution=?8 AND context_key=?9",params![claim.seat.as_str(),consumer.as_str(),invitation.id.as_str(),i64::from(*partial),i64::from(!partial),i64::MAX,state.generation,state.execution,row.key]).map_err(store_error)?;
                }
            }
        }
    }
    let fresh_output = !outputs.is_empty();
    if let Some(stamp) = pending_control {
        use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
        let output = match &stamp {
            Stamp::Attention { epoch, version } => OutputItem::Attention {
                epoch: epoch.clone(),
                attention_version: *version,
                text: crate::protocol::watch::attention_marker_for(&input.pull.marker_prefix),
            },
            Stamp::Cleared { epoch, version } => OutputItem::AttentionCleared {
                epoch: epoch.clone(),
                attention_version: *version,
            },
        };
        let mut trial = outputs.clone();
        trial.push(output.clone());
        if outputs.len() < 32 && sizer.measure(&input.render_profile, &trial).is_ok() {
            outputs.push(output.clone());
            descriptors.push(FrozenPresentation {
                item: output,
                context_key: None,
            });
            controls.allocated(&stamp);
        } else if fresh_output {
            controls.fresh_since_control = true;
        }
    }
    state.control_state = encode(&controls)?;
    if normal_mode && !control_only {
        messages.drain(..shown_message_count);
        carry.items = messages.clone();
        state.ordinary_pending = ordinary_carry_json(&carry)?;
    }
    if !carried_notices.is_empty() {
        work.notice_validations = carried_notices.len();
        super::attention::settle_carried_notices(
            db,
            claim.seat.as_str(),
            state.generation,
            claim.execution.as_str(),
            &carried_notices,
        )?;
    }
    if !descriptors.is_empty() {
        let frozen_items = descriptors
            .iter()
            .map(|v| v.item.clone())
            .collect::<Vec<_>>();
        let selection = input.render_profile.selection(&outputs);
        let replay_profile = match &input.render_profile {
            PresentationRenderProfile::WatchJsonLinesV1 => {
                PresentationRenderProfile::WatchJsonLinesV1
            }
            _ => {
                let PresentationRenderSelection::Hook { selector } = selection else {
                    return Err(corrupt("frozen native selection"));
                };
                let codec = input
                    .render_profile
                    .codec(&selection)
                    .ok_or_else(|| corrupt("frozen native codec"))?;
                PresentationRenderProfile::Hook {
                    confirmable_selector: selector,
                    passive_selector: selector,
                    confirmable: codec,
                    passive: codec,
                }
            }
        };
        sizer
            .measure(&replay_profile, &frozen_items)
            .map_err(|_| corrupt("replay plan output bound"))?;
        state.outbox_retired_mask = 0;
        state.outbox_confirmed_mask = 0;
        state.outbox_token = Some(new_token(db)?);
        state.outbox_text = presentation::presentation_hook_text(&frozen_items);
        state.outbox_descriptors = encode(&descriptors)?;
        state.outbox_selection = encode(&selection)?;
        state.outbox_render_key = render_key;
        state.outbox_prefix = encode(&input.pull.marker_prefix)?;
        state.outbox_codec_key = match &input.render_profile {
            PresentationRenderProfile::WatchJsonLinesV1 => String::new(),
            _ => presentation::descriptor_key(
                input
                    .render_profile
                    .codec(&selection)
                    .ok_or_else(|| corrupt("selected presentation codec"))?,
            ),
        };
    }
    let eligible:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM invitation_presentation_items INDEXED BY invitation_presentation_revisit WHERE seat_id=?1 AND consumer=?2 AND generation=?3 AND required_render_bytes<=?4)",params![claim.seat.as_str(),consumer.as_str(),state.generation,standalone as i64],|r|r.get(0)).map_err(store_error)?;
    let nonordinary_more = publications.more
        || proof.more
        // Relevance preceded notice settlement; revisit this active episode boundedly.
        || (!carried_notices.is_empty() && controls.active_epoch.is_some())
        || controls.pending().is_some()
        || eligible
        || state.dirty_after < state.dirty_high
        || (state.revisit_after_bytes, state.revisit_after_ordinal)
            < (state.revisit_high_bytes, state.revisit_high_ordinal)
        || state.initial_after < state.initial_high
        || !state.context_thread.is_empty()
        || state.context_after_seq < state.context_high
        || work.notices > carried_notices.len()
        || invitation_rows.len() > descriptors.len();
    state.outbox_more = i64::from(nonordinary_more);
    // Physical owner replay never consumes an unchanged logical due episode.
    let consumed = offered
        .iter()
        .chain(&truncated)
        .filter(|id| !owner_progress.emitted.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    // Pruning can exhaust active recovery before source selection. Querying a
    // normal/due source is not a logical allocation: a passive-only result
    // must still preserve the current native outlook for this recovery call.
    if recovery_active && !owner_mode && consumed.is_empty() {
        owner_progress.source = crate::ports::OrdinaryPresentationSource::RecoveryControlOnly;
    }
    let owner_more = input.owner_replay.as_ref().is_some_and(|replay| {
        replay
            .remaining
            .iter()
            .any(|(id, stamp)| owner_progress.completed.get(id) != Some(stamp))
    });
    let more = nonordinary_more
        || owner_more
        || ordinary_runnable(
            &input,
            &state,
            message_slots,
            ordinary_due_update.as_ref(),
            &consumed,
        )?;
    if fresh_output {
        state.rotation = (state.rotation + 1) % 5;
    }
    let reply = if outputs.is_empty() && !more {
        DeliveryPullReplyV2::Unchanged {
            attention_version: input.attention_version,
        }
    } else {
        page(
            &state,
            consumer,
            PageContent {
                items: outputs,
                offered,
                truncated,
            },
            input.attention_version,
            more,
            &input.render_profile,
            &mut sizer,
        )?
    };
    save(db, claim, consumer, &state)?;
    owner_progress.snapshot = input.owner_replay.take();
    finish(
        reply,
        work,
        &mut sizer,
        reclaimed_messages,
        ordinary_due_update,
        owner_progress,
    )
}
fn finish(
    reply: DeliveryPullReplyV2,
    mut work: PresentationWork,
    sizer: &mut crate::delivery::presentation::Sizer,
    reclaimed_messages: Vec<MessageId>,
    ordinary_due_update: Option<crate::delivery::ledger::OrdinaryDueSnapshot>,
    owner_replay: crate::ports::WatchReplayProgress,
) -> Result<PresentationOutput, ApiError> {
    sizer
        .rpc(&crate::protocol::results::CommandResult::DeliveryPulledV2(
            reply.clone(),
        ))
        .map_err(|_| api_error(ErrorCode::Conflict, "presentation RPC result exceeds bound"))?;
    work.output_passes = sizer.work.output_passes;
    work.output_bytes = sizer.work.output_bytes;
    work.rpc_passes = sizer.work.rpc_passes;
    work.rpc_bytes = sizer.work.rpc_bytes;
    Ok(PresentationOutput {
        reply,
        work,
        reclaimed_messages,
        ordinary_due_update,
        owner_replay,
    })
}

/// Canonical, independent passive maintenance. Each call inspects at most
/// forty topic/recipient candidates plus ten initial and ten lifecycle rows.
/// It never advances presentation frontiers, counters or the fixed outbox.
pub(super) fn refresh(
    db: &Connection,
    instance: &str,
    seat: &crate::protocol::ids::SeatId,
    generation: u64,
    native: &str,
) -> Result<Option<crate::ports::ModPresentationRefresh>, ApiError> {
    let Ok(generation_i64) = i64::try_from(generation) else {
        return Ok(None);
    };
    let binding=db.query_row("SELECT b.harness,b.execution_id,s.target_id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id=?1 AND s.instance_id=?2 AND s.state='resolved' AND s.generation=?3 AND b.native_session=?4 AND b.observation_provenance=?5",params![seat.as_str(),instance,generation_i64,native,COOPERATIVE_TOP_LEVEL_PROVENANCE],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(store_error)?;
    let Some((harness, execution, target)) = binding else {
        return Ok(None);
    };
    let claim = CallerClaim {
        instance: instance.into(),
        seat: seat.clone(),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness: serde_json::from_value(serde_json::Value::String(harness))
            .map_err(|_| corrupt("unknown refresh binding harness"))?,
        native_session: crate::protocol::ids::NativeSessionId::parse(native).map_err(corrupt)?,
        execution: crate::protocol::ids::ExecutionId::parse(execution).map_err(corrupt)?,
        target: crate::protocol::ids::HostTargetId::parse(target).map_err(corrupt)?,
    };
    if !scope_matches(db, &claim)? {
        return Ok(None);
    }
    let consumer = PresentationConsumer::Watch;
    let mut state = load_raw(db, &claim, consumer)?;
    if !state.instance_id.is_empty() && state.instance_id != claim.instance {
        return Err(corrupt("foreign passive presentation instance"));
    }
    // Evaluate ORIGINAL headers before installing either owner's new scope.
    let same_instance = state.instance_id == claim.instance;
    let signal_initialized = same_instance
        && ((state.generation == generation_i64 && state.execution == claim.execution.as_str())
            || (state.mod_generation == generation_i64
                && state.mod_execution == claim.execution.as_str()));
    if !signal_initialized {
        state.ordinary_publication_after = 0;
        state.ordinary_publication_high = 0;
        state.ordinary_passive_generation = 0;
        state.ordinary_scan_passive_generation = 0;
    }
    if state.instance_id.is_empty() {
        state.instance_id = claim.instance.clone();
        state.generation = generation_i64;
        state.execution = claim.execution.as_str().into();
    }
    if state.mod_generation != generation_i64 || state.mod_execution != claim.execution.as_str() {
        state.mod_initial_after = 0;
        state.mod_initial_high = 0;
        state.mod_context_after_seq = 0;
        state.mod_context_high = 0;
        state.mod_context_selected_seq = 0;
        state.mod_context_invitation_after = 0;
        state.mod_context_invitation_high = 0;
        state.mod_cleanup_after = 0;
        state.mod_cleanup_high = 0;
        state.mod_context_after_thread = String::new();
        state.mod_context_thread = String::new();
        state.mod_generation = generation_i64;
        state.mod_execution = claim.execution.as_str().into();
    }
    let publications = advance_ordinary_publications(db, &claim, &mut state)?;
    let mut walk = State {
        initial_after: state.mod_initial_after,
        initial_high: state.mod_initial_high,
        context_after_seq: state.mod_context_after_seq,
        context_high: state.mod_context_high,
        context_selected_seq: state.mod_context_selected_seq,
        context_invitation_after: state.mod_context_invitation_after,
        context_invitation_high: state.mod_context_invitation_high,
        context_after_thread: state.mod_context_after_thread.clone(),
        context_thread: state.mod_context_thread.clone(),
        ..State::default()
    };
    let initial = initial_candidates(db, &claim, &mut walk, 10)?;
    let (context, context_visits) = context_candidates(db, &claim, &mut walk, 40)?;
    let initial_visits = initial.len();
    let ids = initial
        .into_iter()
        .chain(context)
        .map(|v| v.id)
        .collect::<Vec<_>>();
    let hydrated = hydrate_many(db, &claim, &ids)?;
    for (id, (context, ordinal)) in hydrated {
        let key = context.key();
        let old=db.query_row("SELECT mod_key,mod_generation,mod_execution FROM invitation_presentation_items WHERE seat_id=?1 AND consumer='watch' AND invitation_id=?2",params![seat.as_str(),id.as_str()],|r|Ok((bounded_private_text(r,0,4096)?,r.get::<_,i64>(1)?,bounded_private_text(r,2,128)?))).optional().map_err(private_read_error)?;
        if old.as_ref().is_none_or(|(old, g, e)| {
            old != &key || *g != generation_i64 || e != claim.execution.as_str()
        }) {
            state.mod_version = state.mod_version.checked_add(1).ok_or_else(|| {
                api_error(
                    ErrorCode::SequenceExhausted,
                    "passive content version exhausted",
                )
            })?;
            db.execute("INSERT INTO invitation_presentation_items(seat_id,invitation_id,consumer,generation,execution,context_key,render_key,output_class,partial_count,complete_count,required_render_bytes,revisit_ordinal,mod_key,mod_generation,mod_execution) VALUES (?1,?2,'watch',?3,?4,'','',2,0,0,?5,?6,?7,?3,?4) ON CONFLICT(seat_id,invitation_id,consumer) DO UPDATE SET mod_key=excluded.mod_key,mod_generation=excluded.mod_generation,mod_execution=excluded.mod_execution",params![seat.as_str(),id.as_str(),generation_i64,claim.execution.as_str(),i64::MAX,ordinal,key]).map_err(store_error)?;
        }
    }
    if state.mod_cleanup_high == 0 {
        state.mod_cleanup_high=db.query_row("SELECT revisit_ordinal FROM invitation_presentation_items INDEXED BY invitation_presentation_cleanup WHERE seat_id=?1 AND consumer='watch' ORDER BY revisit_ordinal DESC LIMIT 1",[seat.as_str()],|r|r.get(0)).optional().map_err(store_error)?.unwrap_or(0);
    }
    let mut query=db.prepare("SELECT invitation_id,revisit_ordinal,mod_key,mod_generation,mod_execution FROM invitation_presentation_items INDEXED BY invitation_presentation_cleanup WHERE seat_id=?1 AND consumer='watch' AND revisit_ordinal>?2 AND revisit_ordinal<=?3 ORDER BY revisit_ordinal LIMIT 10").map_err(store_error)?;
    let cleanup = query
        .query_map(
            params![
                seat.as_str(),
                state.mod_cleanup_after,
                state.mod_cleanup_high
            ],
            |r| {
                Ok((
                    bounded_private_text(r, 0, 128)?,
                    r.get::<_, i64>(1)?,
                    bounded_private_text(r, 2, 4096)?,
                    r.get::<_, i64>(3)?,
                    bounded_private_text(r, 4, 128)?,
                ))
            },
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(private_read_error)?;
    for (id, ordinal, key, g, e) in &cleanup {
        let id =
            InvitationId::parse(id).map_err(|_| corrupt("invalid private invitation identity"))?;
        state.mod_cleanup_after = *ordinal;
        let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM digest_pending_invitations WHERE seat_id=?1 AND invitation_id=?2)",params![seat.as_str(),id.as_str()],|r|r.get(0)).map_err(store_error)?;
        if !pending {
            if !key.is_empty() && *g == generation_i64 && e == claim.execution.as_str() {
                state.mod_version = state.mod_version.checked_add(1).ok_or_else(|| {
                    api_error(
                        ErrorCode::SequenceExhausted,
                        "passive content version exhausted",
                    )
                })?;
            }
            db.execute("DELETE FROM invitation_presentation_items WHERE seat_id=?1 AND consumer='watch' AND invitation_id=?2",params![seat.as_str(),id.as_str()]).map_err(store_error)?;
        }
    }
    let cleanup_more = cleanup.len() == 10 && state.mod_cleanup_after < state.mod_cleanup_high;
    if !cleanup_more {
        state.mod_cleanup_after = 0;
        state.mod_cleanup_high = 0;
    }
    state.mod_initial_after = walk.initial_after;
    state.mod_initial_high = walk.initial_high;
    state.mod_context_after_seq = walk.context_after_seq;
    state.mod_context_high = walk.context_high;
    state.mod_context_selected_seq = walk.context_selected_seq;
    state.mod_context_invitation_after = walk.context_invitation_after;
    state.mod_context_invitation_high = walk.context_invitation_high;
    state.mod_context_after_thread = walk.context_after_thread;
    state.mod_context_thread = walk.context_thread;
    let more = publications.more
        || cleanup_more
        || state.mod_initial_after < state.mod_initial_high
        || !state.mod_context_thread.is_empty()
        || state.mod_context_after_seq < state.mod_context_high;
    let version = u64::try_from(state.mod_version)
        .map_err(|_| corrupt("negative passive content version"))?;
    save(db, &claim, consumer, &state)?;
    Ok(Some(crate::ports::ModPresentationRefresh {
        version,
        more,
        context_visits,
        lifecycle_visits: initial_visits + cleanup.len(),
        ordinary_publication_visits: publications.visits,
        ordinary_publication_probes: publications.probes,
        ordinary_publication_seeks: publications.seeks,
    }))
}
