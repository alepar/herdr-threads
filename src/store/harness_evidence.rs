//! Harness version evidence (ht-xoc.4): what hook payloads showed about each
//! (harness, version, contract id) and the latest reason a payload could not
//! be attributed to a version.
//!
//! The rows are advisory data about the harness, never authority: the daemon
//! writes them from its own process, with no seat or caller identity. A row is
//! verified once a lifecycle payload and a tool payload both matched the
//! contract; the first violation is sticky and only a different version or
//! contract id (a different row) starts clean.
//!
//! Retention: a row may be pruned once its `last_seen_at` is older than
//! [`EVIDENCE_RETENTION_MS`] (30 days; Health's window is 24 hours and doctor's
//! history keeps a month). The newest [`EVIDENCE_KEEP_PER_HARNESS`] rows of
//! each harness (by `last_seen_at`, across contract ids) are never pruned,
//! whatever their age. Pruning runs inside `record()`'s write transaction and
//! only when a row was created (the only time the table grows). [`all`] reads
//! at most [`EVIDENCE_READ_CAP`] rows of one harness, the newest by
//! `last_seen_at`.

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use super::connection::{StoreContext, store_error};
pub use crate::harness::contract::EventClass;
use crate::protocol::{results::ApiError, time::UtcMillis};

/// One evidence row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRow {
    pub harness: String,
    pub version: String,
    pub contract_id: String,
    pub first_seen_at: u64,
    pub lifecycle_ok_at: Option<u64>,
    pub tool_ok_at: Option<u64>,
    pub violation_at: Option<u64>,
    pub violation_event: Option<String>,
    pub violation_field: Option<String>,
    pub last_seen_at: u64,
}

impl EvidenceRow {
    /// A lifecycle payload and a tool payload both matched the contract.
    pub fn verified(&self) -> bool {
        self.lifecycle_ok_at.is_some() && self.tool_ok_at.is_some()
    }
}

/// What one payload showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceOutcome {
    Ok,
    Violation { field: String },
    Malformed,
}

/// One payload's evidence for a (harness, version, contract id).
#[derive(Debug, Clone, Copy)]
pub struct EvidenceRecord<'a> {
    pub harness: &'a str,
    pub version: &'a str,
    pub contract_id: &'a str,
    pub event: &'a str,
    pub class: EventClass,
    pub outcome: &'a EvidenceOutcome,
}

/// The result of recording: whether the row is new, whether this call set its
/// sticky violation, and the row afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub created: bool,
    pub fresh_violation: bool,
    pub row: EvidenceRow,
}

/// A row unseen for longer than this may be pruned (30 days).
pub const EVIDENCE_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// The newest rows of each harness that pruning never removes.
pub const EVIDENCE_KEEP_PER_HARNESS: i64 = 64;
/// The most rows `all` returns for one harness.
pub const EVIDENCE_READ_CAP: i64 = 256;

const COLUMNS: &str = "harness, version, contract_id, first_seen_at, lifecycle_ok_at, tool_ok_at, \
     violation_at, violation_event, violation_field, last_seen_at";

fn ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn opt_ms(value: Option<i64>) -> Option<u64> {
    value.map(ms)
}

fn row_of(row: &Row<'_>) -> rusqlite::Result<EvidenceRow> {
    Ok(EvidenceRow {
        harness: row.get(0)?,
        version: row.get(1)?,
        contract_id: row.get(2)?,
        first_seen_at: ms(row.get(3)?),
        lifecycle_ok_at: opt_ms(row.get(4)?),
        tool_ok_at: opt_ms(row.get(5)?),
        violation_at: opt_ms(row.get(6)?),
        violation_event: row.get(7)?,
        violation_field: row.get(8)?,
        last_seen_at: ms(row.get(9)?),
    })
}

fn now_ms(context: &StoreContext) -> i64 {
    let UtcMillis(now) = context.clock().utc_now();
    now
}

/// Records one payload's evidence in one write transaction.
pub fn record(
    context: &StoreContext,
    writer: &mut Connection,
    record: &EvidenceRecord<'_>,
) -> Result<Recorded, ApiError> {
    let now = now_ms(context);
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let key = params![record.harness, record.version, record.contract_id];
    let created = tx
        .execute(
            "INSERT OR IGNORE INTO harness_version_evidence \
             (harness, version, contract_id, first_seen_at, last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![record.harness, record.version, record.contract_id, now],
        )
        .map_err(store_error)?
        == 1;
    if created {
        tx.execute(
            "DELETE FROM harness_version_evidence \
              WHERE harness = ?1 AND last_seen_at < ?2 \
                AND (harness, version, contract_id) NOT IN ( \
                  SELECT harness, version, contract_id FROM harness_version_evidence \
                   WHERE harness = ?1 ORDER BY last_seen_at DESC LIMIT ?3)",
            params![
                record.harness,
                now.saturating_sub(EVIDENCE_RETENTION_MS),
                EVIDENCE_KEEP_PER_HARNESS
            ],
        )
        .map_err(store_error)?;
    }
    let mut fresh_violation = false;
    tx.execute(
        "UPDATE harness_version_evidence SET last_seen_at = MAX(last_seen_at, ?4) \
         WHERE harness = ?1 AND version = ?2 AND contract_id = ?3",
        params![record.harness, record.version, record.contract_id, now],
    )
    .map_err(store_error)?;
    match record.outcome {
        EvidenceOutcome::Ok => {
            let column = match record.class {
                EventClass::Lifecycle => Some("lifecycle_ok_at"),
                EventClass::Tool => Some("tool_ok_at"),
                EventClass::Other => None,
            };
            if let Some(column) = column {
                tx.execute(
                    &format!(
                        "UPDATE harness_version_evidence SET {column} = COALESCE({column}, ?4) \
                         WHERE harness = ?1 AND version = ?2 AND contract_id = ?3"
                    ),
                    params![record.harness, record.version, record.contract_id, now],
                )
                .map_err(store_error)?;
            }
        }
        EvidenceOutcome::Violation { field } => {
            fresh_violation = tx
                .execute(
                    "UPDATE harness_version_evidence \
                     SET violation_at = ?4, violation_event = ?5, violation_field = ?6 \
                     WHERE harness = ?1 AND version = ?2 AND contract_id = ?3 \
                       AND violation_at IS NULL",
                    params![
                        record.harness,
                        record.version,
                        record.contract_id,
                        now,
                        record.event,
                        field
                    ],
                )
                .map_err(store_error)?
                == 1;
        }
        EvidenceOutcome::Malformed => {}
    }
    let row = tx
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM harness_version_evidence \
                 WHERE harness = ?1 AND version = ?2 AND contract_id = ?3"
            ),
            key,
            row_of,
        )
        .map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok(Recorded {
        created,
        fresh_violation,
        row,
    })
}

pub fn get(
    db: &Connection,
    harness: &str,
    version: &str,
    contract_id: &str,
) -> Result<Option<EvidenceRow>, ApiError> {
    db.query_row(
        &format!(
            "SELECT {COLUMNS} FROM harness_version_evidence \
             WHERE harness = ?1 AND version = ?2 AND contract_id = ?3"
        ),
        params![harness, version, contract_id],
        row_of,
    )
    .optional()
    .map_err(store_error)
}

/// Rows of both harnesses with `last_seen_at >= since_ms`.
pub fn since(db: &Connection, since_ms: u64) -> Result<Vec<EvidenceRow>, ApiError> {
    let since = i64::try_from(since_ms).unwrap_or(i64::MAX);
    collect(
        db,
        &format!(
            "SELECT {COLUMNS} FROM harness_version_evidence WHERE last_seen_at >= ?1 \
             ORDER BY harness, first_seen_at, version, contract_id"
        ),
        params![since],
    )
}

/// The newest `EVIDENCE_READ_CAP` rows of one harness (by `last_seen_at`),
/// ordered by `first_seen_at, version, contract_id`.
pub fn all(db: &Connection, harness: &str) -> Result<Vec<EvidenceRow>, ApiError> {
    collect(
        db,
        &format!(
            "SELECT {COLUMNS} FROM (SELECT {COLUMNS} FROM harness_version_evidence \
             WHERE harness = ?1 ORDER BY last_seen_at DESC LIMIT ?2) \
             ORDER BY first_seen_at, version, contract_id"
        ),
        params![harness, EVIDENCE_READ_CAP],
    )
}

fn collect(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<EvidenceRow>, ApiError> {
    let mut stmt = db.prepare(sql).map_err(store_error)?;
    stmt.query_map(params, row_of)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)
}

/// Keeps the newest reason a payload of `harness` could not be attributed.
pub fn record_unattributed(
    context: &StoreContext,
    writer: &mut Connection,
    harness: &str,
    reason: &str,
) -> Result<(), ApiError> {
    writer
        .execute(
            "INSERT INTO harness_unattributed(harness, reason, at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(harness) DO UPDATE SET reason = excluded.reason, at = excluded.at",
            params![harness, reason, now_ms(context)],
        )
        .map_err(store_error)?;
    Ok(())
}

pub fn last_unattributed(
    db: &Connection,
    harness: &str,
) -> Result<Option<(String, u64)>, ApiError> {
    db.query_row(
        "SELECT reason, at FROM harness_unattributed WHERE harness = ?1",
        [harness],
        |row| Ok((row.get::<_, String>(0)?, ms(row.get(1)?))),
    )
    .optional()
    .map_err(store_error)
}

/// A server-resolved observation; no client-supplied milestone or verification flag.
#[derive(Debug, Clone, Copy)]
pub struct EvidenceRecordV2<'a> {
    pub identity: &'a crate::harness::runtime::RuntimeIdentity,
    pub descriptor: &'a crate::harness::adapter::ContractDescriptor,
    pub event: &'a str,
    pub outcome: &'a EvidenceOutcome,
    /// Server has established all descriptor role/session/runtime qualifications.
    /// False observations retain timestamps/outcomes but earn no success milestone.
    pub qualified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRowV2 {
    pub harness: String,
    pub identity: crate::harness::runtime::RuntimeIdentity,
    pub domain: String,
    pub origin: crate::harness::evidence::EvidenceOrigin,
    pub contract_id: String,
    pub first_seen_at: u64,
    pub last_seen_at: u64,
    pub milestones: std::collections::BTreeMap<String, u64>,
    pub violation_at: Option<u64>,
    pub violation_event: Option<String>,
    pub violation_field: Option<String>,
}
impl EvidenceRowV2 {
    pub fn verified(&self, descriptor: &crate::harness::adapter::ContractDescriptor) -> bool {
        self.violation_at.is_none()
            && self.harness == descriptor.contract.harness
            && self.domain == descriptor.domain_id
            && self.origin == descriptor.origin
            && descriptor.contract_id_v2().ok().as_deref() == Some(&self.contract_id)
            && !descriptor.required_milestones.is_empty()
            && descriptor
                .required_milestones
                .iter()
                .all(|m| self.milestones.contains_key(*m))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedV2 {
    pub created: bool,
    pub fresh_violation: bool,
    pub row: EvidenceRowV2,
}
fn invalid_v2(detail: impl Into<String>) -> ApiError {
    super::api_error(crate::protocol::results::ErrorCode::InvalidRequest, detail)
}
fn origin_name(origin: crate::harness::evidence::EvidenceOrigin) -> &'static str {
    use crate::harness::evidence::EvidenceOrigin::*;
    match origin {
        NativePayload => "native_payload",
        NativeShapeObservation => "native_shape_observation",
        BridgeEnvelope => "bridge_envelope",
    }
}
fn valid_harness(harness: &str) -> bool {
    !harness.is_empty()
        && harness.len() <= 64
        && harness != "human"
        && harness.as_bytes()[0].is_ascii_lowercase()
        && harness
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
fn validate_scope(harness: &str, domain: &str) -> Result<(), ApiError> {
    if !valid_harness(harness) || !crate::harness::evidence::valid_name(domain) {
        return Err(invalid_v2("invalid evidence harness/domain"));
    }
    Ok(())
}
const V2_COLUMNS: &str = "e.harness, e.identity_key, i.descriptor_json, e.domain, e.origin, e.contract_id, e.first_seen_at, e.last_seen_at, e.milestones_json, e.violation_at, e.violation_event, e.violation_field";
const V2_JOIN: &str = "harness_contract_evidence_v2 e JOIN harness_runtime_identities i ON i.harness=e.harness AND i.identity_key=e.identity_key";
const V2_ORDER: &str =
    "e.last_seen_at DESC, e.harness, e.identity_key, e.domain, e.origin, e.contract_id";
fn decode_error(column: usize, detail: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            detail.into(),
        )),
    )
}
fn row_v2(row: &Row<'_>) -> rusqlite::Result<EvidenceRowV2> {
    use crate::harness::runtime::{RuntimeDescriptor, RuntimeIdentity};
    let text: String = row.get(2)?;
    if text.len() > 4096 {
        return Err(decode_error(2, "runtime descriptor exceeds bound"));
    }
    let mut value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| decode_error(2, e.to_string()))?;
    if value
        .as_object_mut()
        .and_then(|o| o.remove("schema_version"))
        != Some(serde_json::json!(1))
    {
        return Err(decode_error(2, "invalid descriptor schema"));
    }
    let descriptor: RuntimeDescriptor =
        serde_json::from_value(value).map_err(|e| decode_error(2, e.to_string()))?;
    let identity = RuntimeIdentity {
        key: row.get(1)?,
        descriptor,
    };
    identity.validate().map_err(|e| decode_error(2, e))?;
    if identity.descriptor.canonical_json() != text {
        return Err(decode_error(2, "noncanonical runtime descriptor"));
    }
    let origin = match row.get::<_, String>(4)?.as_str() {
        "native_payload" => crate::harness::evidence::EvidenceOrigin::NativePayload,
        "native_shape_observation" => {
            crate::harness::evidence::EvidenceOrigin::NativeShapeObservation
        }
        "bridge_envelope" => crate::harness::evidence::EvidenceOrigin::BridgeEnvelope,
        _ => return Err(decode_error(4, "invalid evidence origin")),
    };
    let text: String = row.get(8)?;
    if text.len() > 4096 {
        return Err(decode_error(8, "milestones exceed bound"));
    }
    let milestones: std::collections::BTreeMap<String, u64> =
        serde_json::from_str(&text).map_err(|e| decode_error(8, e.to_string()))?;
    if milestones.len() > 32
        || milestones
            .keys()
            .any(|m| !crate::harness::evidence::valid_name(m))
    {
        return Err(decode_error(8, "invalid stored milestones"));
    }
    if serde_json::to_string(&milestones).map_err(|e| decode_error(8, e.to_string()))? != text {
        return Err(decode_error(8, "noncanonical stored milestones"));
    }
    Ok(EvidenceRowV2 {
        harness: row.get(0)?,
        identity,
        domain: row.get(3)?,
        origin,
        contract_id: row.get(5)?,
        first_seen_at: ms(row.get(6)?),
        last_seen_at: ms(row.get(7)?),
        milestones,
        violation_at: opt_ms(row.get(9)?),
        violation_event: row.get(10)?,
        violation_field: row.get(11)?,
    })
}
/// Inserts exact identity and declared domain evidence in one writer transaction.
/// Qualification is resolved by the server before this call, never by client JSON.
pub fn record_v2(
    context: &StoreContext,
    writer: &mut Connection,
    record: &EvidenceRecordV2<'_>,
) -> Result<RecordedV2, ApiError> {
    let descriptor = record.descriptor;
    descriptor.validate().map_err(invalid_v2)?;
    validate_scope(descriptor.contract.harness, descriptor.domain_id)?;
    record.identity.validate().map_err(invalid_v2)?;
    let event = descriptor
        .event(record.event)
        .ok_or_else(|| invalid_v2("undeclared native evidence event"))?;
    if let EvidenceOutcome::Violation { field } = record.outcome
        && !crate::harness::runtime::printable(field, 256)
    {
        return Err(invalid_v2("invalid violation field"));
    }
    let contract = descriptor.contract_id_v2().map_err(invalid_v2)?;
    let harness = descriptor.contract.harness;
    let domain = descriptor.domain_id;
    let origin = origin_name(descriptor.origin);
    let identity = &record.identity.key;
    let now = now_ms(context);
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let canonical = record.identity.descriptor.canonical_json();
    let existing:Option<String>=tx.query_row("SELECT descriptor_json FROM harness_runtime_identities WHERE harness=?1 AND identity_key=?2",params![harness,identity],|r|r.get(0)).optional().map_err(store_error)?;
    if existing.as_ref().is_some_and(|text| text != &canonical) {
        return Err(invalid_v2("runtime identity descriptor conflict"));
    }
    tx.execute("INSERT INTO harness_runtime_identities(harness,identity_key,descriptor_json,last_seen_at) VALUES(?1,?2,?3,?4) ON CONFLICT(harness,identity_key) DO UPDATE SET last_seen_at=MAX(last_seen_at,excluded.last_seen_at)",params![harness,identity,canonical,now]).map_err(store_error)?;
    let created=tx.execute("INSERT OR IGNORE INTO harness_contract_evidence_v2(harness,identity_key,domain,origin,contract_id,first_seen_at,last_seen_at,milestones_json) VALUES(?1,?2,?3,?4,?5,?6,?6,'{}')",params![harness,identity,domain,origin,contract,now]).map_err(store_error)?==1;
    let mut row = get_v2(&tx, harness, identity, domain, descriptor.origin, &contract)?
        .ok_or_else(|| invalid_v2("evidence row missing after insert"))?;
    let mut fresh_violation = false;
    match record.outcome {
        EvidenceOutcome::Ok if record.qualified => {
            if let Some(milestone) = event.milestone {
                row.milestones.entry(milestone.into()).or_insert(ms(now));
            }
        }
        EvidenceOutcome::Violation { field } if row.violation_at.is_none() => {
            row.violation_at = Some(ms(now));
            row.violation_event = Some(record.event.into());
            row.violation_field = Some(field.clone());
            fresh_violation = true;
        }
        _ => {}
    }
    let milestones =
        serde_json::to_string(&row.milestones).map_err(|e| invalid_v2(e.to_string()))?;
    if milestones.len() > 4096 {
        return Err(invalid_v2("milestones exceed bound"));
    }
    tx.execute("UPDATE harness_contract_evidence_v2 SET last_seen_at=MAX(last_seen_at,?6),milestones_json=?7,violation_at=?8,violation_event=?9,violation_field=?10 WHERE harness=?1 AND identity_key=?2 AND domain=?3 AND origin=?4 AND contract_id=?5",params![harness,identity,domain,origin,contract,now,milestones,row.violation_at.map(|at|i64::try_from(at).unwrap_or(i64::MAX)),row.violation_event,row.violation_field]).map_err(store_error)?;
    if created {
        tx.execute("DELETE FROM harness_contract_evidence_v2 WHERE harness=?1 AND last_seen_at<?2 AND (identity_key,domain,origin,contract_id) NOT IN (SELECT identity_key,domain,origin,contract_id FROM harness_contract_evidence_v2 WHERE harness=?1 ORDER BY last_seen_at DESC,identity_key,domain,origin,contract_id LIMIT ?3)",params![harness,now.saturating_sub(EVIDENCE_RETENTION_MS),EVIDENCE_KEEP_PER_HARNESS]).map_err(store_error)?;
        tx.execute("DELETE FROM harness_runtime_identities WHERE harness=?1 AND NOT EXISTS (SELECT 1 FROM harness_contract_evidence_v2 e WHERE e.harness=harness_runtime_identities.harness AND e.identity_key=harness_runtime_identities.identity_key)",[harness]).map_err(store_error)?;
    }
    row.last_seen_at = row.last_seen_at.max(ms(now));
    tx.commit().map_err(store_error)?;
    Ok(RecordedV2 {
        created,
        fresh_violation,
        row,
    })
}
pub fn get_v2(
    db: &Connection,
    harness: &str,
    identity: &str,
    domain: &str,
    origin: crate::harness::evidence::EvidenceOrigin,
    contract: &str,
) -> Result<Option<EvidenceRowV2>, ApiError> {
    db.query_row(&format!("SELECT {V2_COLUMNS} FROM {V2_JOIN} WHERE e.harness=?1 AND e.identity_key=?2 AND e.domain=?3 AND e.origin=?4 AND e.contract_id=?5"),params![harness,identity,domain,origin_name(origin),contract],row_v2).optional().map_err(store_error)
}
/// Newest exact rows, including their canonical runtime descriptor. Ties are stable.
pub fn all_v2(
    db: &Connection,
    harness: &str,
    since_ms: u64,
) -> Result<Vec<EvidenceRowV2>, ApiError> {
    collect_v2(
        db,
        &format!(
            "SELECT {V2_COLUMNS} FROM {V2_JOIN} WHERE e.harness=?1 AND e.last_seen_at>=?2 ORDER BY {V2_ORDER} LIMIT ?3"
        ),
        params![
            harness,
            i64::try_from(since_ms).unwrap_or(i64::MAX),
            EVIDENCE_READ_CAP
        ],
    )
}
pub fn since_v2(db: &Connection, since_ms: u64) -> Result<Vec<EvidenceRowV2>, ApiError> {
    collect_v2(
        db,
        &format!(
            "SELECT {V2_COLUMNS} FROM {V2_JOIN} WHERE e.last_seen_at>=?1 ORDER BY {V2_ORDER} LIMIT ?2"
        ),
        params![
            i64::try_from(since_ms).unwrap_or(i64::MAX),
            EVIDENCE_READ_CAP
        ],
    )
}
fn collect_v2(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<EvidenceRowV2>, ApiError> {
    let mut stmt = db.prepare(sql).map_err(store_error)?;
    stmt.query_map(params, row_v2)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)
}
/// One latest bounded reason for the exact harness/domain/origin, never an identity.
pub fn record_unattributed_v2(
    context: &StoreContext,
    writer: &mut Connection,
    harness: &str,
    domain: &str,
    origin: crate::harness::evidence::EvidenceOrigin,
    reason: &str,
) -> Result<(), ApiError> {
    validate_scope(harness, domain)?;
    if !crate::harness::runtime::printable(reason, 256) {
        return Err(invalid_v2("invalid unattributed reason"));
    }
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    tx.execute("INSERT INTO harness_unattributed_v2(harness,domain,origin,reason,at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(harness,domain,origin) DO UPDATE SET reason=excluded.reason,at=excluded.at WHERE excluded.at>=at",params![harness,domain,origin_name(origin),reason,now_ms(context)]).map_err(store_error)?;
    tx.commit().map_err(store_error)
}
pub fn last_unattributed_v2(
    db: &Connection,
    harness: &str,
    domain: &str,
    origin: crate::harness::evidence::EvidenceOrigin,
) -> Result<Option<(String, u64)>, ApiError> {
    db.query_row("SELECT reason,at FROM harness_unattributed_v2 WHERE harness=?1 AND domain=?2 AND origin=?3",params![harness,domain,origin_name(origin)],|r|Ok((r.get(0)?,ms(r.get(1)?)))).optional().map_err(store_error)
}

#[cfg(test)]
#[path = "../../tests/store/harness_evidence.rs"]
mod tests;

/// Fixed producer scope: an unavailable-runtime violation under one hook contract.
/// No diagnostic row can qualify a runtime, capability, receipt or binding.
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticRecord<'a> {
    pub harness: &'a str,
    pub session_id: &'a str,
    pub contract_id: &'a str,
    pub event: &'a str,
    pub field: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRow {
    pub harness: String,
    pub session_id: String,
    pub contract_id: String,
    pub event: String,
    pub field: String,
    pub first_seen_at: u64,
    pub last_seen_at: u64,
}

impl DiagnosticRow {
    pub fn line(&self) -> String {
        format!(
            "harness {} contract input failure: {}/{}; runtime metadata unavailable",
            self.harness, self.event, self.field
        )
    }
}

/// Hard storage cap per harness, separate from historical version retention.
pub const DIAGNOSTIC_KEEP_PER_HARNESS: i64 = 256;
/// At most this many scoped failures are read/projected, newest first.
pub const DIAGNOSTIC_READ_CAP: i64 = 20;
/// Expired failures leave the projection even without another write.
pub const DIAGNOSTIC_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Preserve the first event/field; only another violation touches last_seen_at.
/// Creation prunes expiry and evicts deterministically in the same transaction.
pub fn record_diagnostic(
    context: &StoreContext,
    writer: &mut Connection,
    record: &DiagnosticRecord<'_>,
) -> Result<(), ApiError> {
    let printable = |value: &str, bound: usize| {
        !value.is_empty() && value.len() <= bound && !value.chars().any(char::is_control)
    };
    if !valid_harness(record.harness)
        || !printable(record.session_id, 256)
        || record.contract_id.len() != 16
        || !record
            .contract_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || record.event.len() > 63
        || !record
            .event
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        || !record
            .event
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || !printable(record.field, 128)
    {
        return Err(invalid_v2("invalid bounded contract diagnostic"));
    }
    let now = now_ms(context);
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    tx.execute(
        "DELETE FROM harness_contract_diagnostics WHERE harness=?1 AND last_seen_at < ?2",
        params![record.harness, now.saturating_sub(DIAGNOSTIC_RETENTION_MS)],
    )
    .map_err(store_error)?;
    tx.execute("INSERT INTO harness_contract_diagnostics
        (harness,session_id,contract_id,event,field,first_seen_at,last_seen_at)
        VALUES (?1,?2,?3,?4,?5,?6,?6)
        ON CONFLICT(harness,session_id,contract_id) DO UPDATE SET last_seen_at=MAX(last_seen_at,excluded.last_seen_at)",
        params![record.harness,record.session_id,record.contract_id,record.event,record.field,now]).map_err(store_error)?;
    tx.execute("DELETE FROM harness_contract_diagnostics WHERE harness=?1 AND (session_id,contract_id) NOT IN (
        SELECT session_id,contract_id FROM harness_contract_diagnostics WHERE harness=?1
        ORDER BY last_seen_at DESC,first_seen_at DESC,session_id,contract_id LIMIT ?2)",
        params![record.harness, DIAGNOSTIC_KEEP_PER_HARNESS]).map_err(store_error)?;
    tx.commit().map_err(store_error)
}

pub fn diagnostics(
    db: &Connection,
    harness: &str,
    now: UtcMillis,
) -> Result<Vec<DiagnosticRow>, ApiError> {
    let mut stmt = db
        .prepare(
            "SELECT harness,session_id,contract_id,event,field,first_seen_at,last_seen_at
        FROM harness_contract_diagnostics WHERE harness=?1 AND last_seen_at >= ?2
        ORDER BY last_seen_at DESC,first_seen_at DESC,session_id,contract_id LIMIT ?3",
        )
        .map_err(store_error)?;
    stmt.query_map(
        params![
            harness,
            now.0.saturating_sub(DIAGNOSTIC_RETENTION_MS),
            DIAGNOSTIC_READ_CAP
        ],
        |row| {
            Ok(DiagnosticRow {
                harness: row.get(0)?,
                session_id: row.get(1)?,
                contract_id: row.get(2)?,
                event: row.get(3)?,
                field: row.get(4)?,
                first_seen_at: ms(row.get(5)?),
                last_seen_at: ms(row.get(6)?),
            })
        },
    )
    .map_err(store_error)?
    .collect::<Result<_, _>>()
    .map_err(store_error)
}
