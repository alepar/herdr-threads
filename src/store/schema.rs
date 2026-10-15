//! The node store schema entry points (`initialize`, `verify_baseline`) and common
//! transaction primitives used by store handlers.

use crate::{
    node::{NodeTag, NodeUuid},
    ports::TimeBasis,
    protocol::{
        authority::{CARRIED_BINDING_PROVENANCES, ObligationRef},
        ids::{InvitationId, MessageId, RetirementJobId, SeatId, ThreadId, prefix},
        results::{ApiError, CommandResult, ErrorCode},
        service::EventAuthor,
        time::UtcMillis,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

#[cfg(test)]
pub(crate) mod legacy_chain;

use super::connection::{DecisionInstant, StoreContext, api_error, store_error};
use super::effective::{
    EffectiveReceiptState, ReceiptSource, effective_observation, effective_receipt,
};

/// The node store baseline: the one reviewed schema, applied verbatim to an
/// empty database and the only thing `verify_baseline` audits an existing store
/// against. Schema changes are edits to `migrations/node/0001_node_baseline.sql`
/// until the first `v*` release that ships it; after that they are additive
/// `migrations/node/000N_*.sql` deltas with `user_version` bumps.
pub(crate) const NODE_BASELINE: &str = include_str!("../../migrations/node/0001_node_baseline.sql");
/// `PRAGMA user_version` of a store created from the baseline.
pub const NODE_SCHEMA_VERSION: i64 = 1;
/// `PRAGMA application_id` of a node store ('HTND').
pub const NODE_APPLICATION_ID: i32 = 0x4854_4E44;
/// The `schema_identity.marker` of a node store.
pub const NODE_SCHEMA_MARKER: &str = "herdr-threads-node-v1";

/// Decode only persisted results, after the caller's digest has matched. Live
/// protocol responses still require disposition. Missing original context
/// cannot be recovered from today's occupant without inventing historical data.
fn decode_stored_result(json: &str) -> Result<CommandResult, ApiError> {
    let mut value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("invalid stored result: {e}"),
        )
    })?;
    if value.get("kind").and_then(|v| v.as_str()) == Some("checked_in")
        && let Some(data) = value.get_mut("data").and_then(|v| v.as_object_mut())
    {
        let complete = data
            .get("context")
            .and_then(|v| v.as_object())
            .is_some_and(|context| {
                [
                    "instance",
                    "seat",
                    "binding_generation",
                    "role",
                    "harness",
                    "native_session",
                    "execution",
                    "target",
                ]
                .iter()
                .all(|key| context.contains_key(*key))
            });
        if !complete {
            return Err(api_error(
                ErrorCode::Unsupported,
                "legacy CheckIn result lacks recorded full context; use a new explicit lifecycle request",
            ));
        }
        // This annotation is transient. The locked registration presenter
        // derives Current/Historical from the exact recorded context.
        data.entry("context_disposition")
            .or_insert_with(|| serde_json::json!("current"));
    }
    serde_json::from_value(value).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("invalid stored result: {e}"),
        )
    })
}

/// Writer-connection invariant: no occupant binding is registered without the
/// terminal and host incarnation that snapshot reconciliation needs to
/// reconfirm it after a host invalidation. Enforced at write time by
/// connection-scoped TEMP triggers (no persistent schema change, so the
/// schema audit and version are unchanged) and, at writer startup, by a
/// bounded backfill of legacy rows from the seat's own verified structural
/// proof when that proof provably describes the same binding.
pub(crate) const BINDING_EVIDENCE_GUARD: &str = "
CREATE TEMP TRIGGER IF NOT EXISTS occupant_binding_evidence_insert
BEFORE INSERT ON main.occupant_bindings
WHEN NEW.ended_at IS NULL AND (NEW.terminal_id IS NULL OR NEW.incarnation IS NULL OR NEW.terminal_id = '' OR NEW.incarnation = '')
BEGIN SELECT RAISE(ABORT, 'occupant binding lacks reconfirmation evidence'); END;
CREATE TEMP TRIGGER IF NOT EXISTS occupant_binding_evidence_register
BEFORE UPDATE OF registered_at, terminal_id, incarnation ON main.occupant_bindings
WHEN NEW.registered_at IS NOT NULL AND NEW.ended_at IS NULL
 AND NEW.registered_at IS NOT OLD.registered_at
 AND (NEW.terminal_id IS NULL OR NEW.incarnation IS NULL OR NEW.terminal_id = '' OR NEW.incarnation = '')
BEGIN SELECT RAISE(ABORT, 'occupant binding lacks reconfirmation evidence'); END;
";

/// Legacy rows written before the invariant (cooperative bindings stored no
/// terminal/incarnation) are repaired only from the seat's persisted verified
/// structural proof, and only for the seat's latest binding when that proof
/// has the same target, target generation, host boot and host epoch as the
/// binding (and the same terminal, when one was stored). Rows without such
/// evidence stay as they are: they remain fail-closed, and nothing invents
/// evidence. Returns (backfilled, still lacking evidence) latest bindings.
pub(crate) fn guard_binding_evidence(conn: &Connection) -> Result<(usize, i64), ApiError> {
    conn.execute_batch(BINDING_EVIDENCE_GUARD)
        .map_err(store_error)?;
    // Read first so an up-to-date store takes no write lock at startup.
    let lacking = || -> Result<i64, ApiError> {
        conn.query_row(
            "SELECT count(*) FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id
             WHERE s.state!='retired' AND (b.terminal_id IS NULL OR b.incarnation IS NULL)
               AND b.ordinal=(SELECT MAX(x.ordinal) FROM occupant_bindings x WHERE x.seat_id=b.seat_id)",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)
    };
    if lacking()? == 0 {
        return Ok((0, 0));
    }
    // One autocommit statement: atomic, and idempotent across restarts.
    let backfilled = conn
        .execute(
            "UPDATE occupant_bindings SET
                terminal_id=(SELECT s.structural_terminal_id FROM seats s WHERE s.id=occupant_bindings.seat_id),
                incarnation=(SELECT s.structural_incarnation FROM seats s WHERE s.id=occupant_bindings.seat_id)
             WHERE (terminal_id IS NULL OR incarnation IS NULL)
               AND ordinal=(SELECT MAX(b.ordinal) FROM occupant_bindings b WHERE b.seat_id=occupant_bindings.seat_id)
               AND EXISTS(SELECT 1 FROM seats s WHERE s.id=occupant_bindings.seat_id
                   AND s.state IN ('resolved','unresolved')
                   AND s.target_id=occupant_bindings.target_id
                   AND s.target_generation=occupant_bindings.target_generation
                   AND s.structural_host_boot=occupant_bindings.host_boot
                   AND s.structural_host_epoch=occupant_bindings.host_epoch
                   AND s.structural_terminal_id IS NOT NULL AND s.structural_terminal_id != ''
                   AND s.structural_incarnation IS NOT NULL AND s.structural_incarnation != ''
                   AND (occupant_bindings.terminal_id IS NULL OR occupant_bindings.terminal_id=s.structural_terminal_id)
                   AND (occupant_bindings.incarnation IS NULL OR occupant_bindings.incarnation=s.structural_incarnation))",
            [],
        )
        .map_err(store_error)?;
    Ok((backfilled, lacking()?))
}

/// Seats (non-retired) whose latest binding lacks reconfirmation evidence,
/// with the same predicate as `guard_binding_evidence`'s count. Read only at
/// writer startup, and only when that count is nonzero.
pub(crate) fn binding_evidence_lacking_seats(conn: &Connection) -> Result<Vec<String>, ApiError> {
    let mut stmt = conn
        .prepare(
            "SELECT b.seat_id FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id
             WHERE s.state!='retired' AND (b.terminal_id IS NULL OR b.incarnation IS NULL)
               AND b.ordinal=(SELECT MAX(x.ordinal) FROM occupant_bindings x WHERE x.seat_id=b.seat_id)
             ORDER BY s.ordinal",
        )
        .map_err(store_error)?;
    stmt.query_map([], |r| r.get(0))
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)
}

/// Whether one seat is still non-retired with a latest binding that lacks
/// evidence: one `occupant_bindings_history` lookup.
pub(crate) fn binding_evidence_lacking(conn: &Connection, seat: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id
             WHERE s.id=?1 AND s.state!='retired' AND (b.terminal_id IS NULL OR b.incarnation IS NULL)
               AND b.ordinal=(SELECT MAX(x.ordinal) FROM occupant_bindings x WHERE x.seat_id=?1))",
        [seat],
        |r| r.get(0),
    )
}

/// The endpoint recorded for an attached Herdr host that was composed without
/// a socket path (an in-process host given directly). Interim: ht-5fr.1.7's
/// `HostChoice` replaces the single-host composition that needs it.
pub const UNNAMED_HOST_ENDPOINT: &str = "unnamed-host";

/// The identity a fresh node store is created with: the `local_node` row and
/// the self row of `node_directory`. An existing store keeps its own rows; the
/// seed is only read when the database is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeSeed {
    pub uuid: NodeUuid,
    pub tag: NodeTag,
    pub tag_k: u32,
}

/// Open or create the node store. An empty database gets `NODE_BASELINE` in one
/// `BEGIN IMMEDIATE` plus the application id, the version, the `local_node` row
/// and the self `node_directory` row from `node`; an existing node store is
/// audited by `verify_baseline`; anything else is `IncompatibleSchema`.
pub fn initialize(
    conn: &Connection,
    now: impl FnOnce() -> UtcMillis,
    node: &NodeSeed,
) -> Result<(), ApiError> {
    initialize_with(conn, now, Some(node))
}

/// [`initialize`] for a caller that may have no seed (a store context that was
/// never given one): a fresh database then fails instead of inventing an
/// identity.
pub(crate) fn initialize_with(
    conn: &Connection,
    now: impl FnOnce() -> UtcMillis,
    node: Option<&NodeSeed>,
) -> Result<(), ApiError> {
    check_integrity(conn)?;
    let application_id: i64 = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(store_error)?;
    let version = schema_version(conn)?;
    let incompatible = |detail: String| api_error(ErrorCode::IncompatibleSchema, detail);
    if application_id == 0 && version == 0 {
        let objects: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if objects != 0 {
            return Err(incompatible("unversioned nonempty database".into()));
        }
        let node = node.ok_or_else(|| {
            api_error(
                ErrorCode::InvalidRequest,
                "node seed missing: a new store needs the node identity",
            )
        })?;
        let created_at = now().0;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let created = conn
            .execute_batch(NODE_BASELINE)
            .and_then(|()| conn.pragma_update(None, "application_id", NODE_APPLICATION_ID))
            .and_then(|()| conn.pragma_update(None, "user_version", NODE_SCHEMA_VERSION))
            .and_then(|()| insert_node_rows(conn, node, created_at));
        match created {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
        return verify_baseline(conn);
    }
    if application_id == i64::from(NODE_APPLICATION_ID) {
        return match version {
            NODE_SCHEMA_VERSION => verify_baseline(conn),
            v if v > NODE_SCHEMA_VERSION => Err(incompatible(format!(
                "store is newer than this binary (schema version {v})"
            ))),
            v => Err(incompatible(format!("unsupported schema version {v}"))),
        };
    }
    if application_id == 0 && version > 0 {
        return Err(incompatible(format!(
            "pre-node schema version {version}; move the store away"
        )));
    }
    Err(incompatible(format!(
        "not a herdr-threads node store (application_id {application_id:#x})"
    )))
}

/// The creating transaction's node rows: `local_node` and the self row of
/// `node_directory`, before anything mints an ID. (ht-5fr.1.10 moves the self
/// row behind `store::node_directory`.)
fn insert_node_rows(conn: &Connection, node: &NodeSeed, created_at: i64) -> rusqlite::Result<()> {
    let (uuid, tag) = (node.uuid.to_string(), node.tag.to_string());
    conn.execute(
        "INSERT INTO local_node(id,created_at,tag,tag_k) VALUES (?1,?2,?3,?4)",
        params![uuid, created_at, tag, node.tag_k],
    )?;
    conn.execute(
        "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,?2,?3,?4)",
        params![uuid, tag, node.tag_k, created_at],
    )?;
    Ok(())
}

/// Name prefix of a fault-injection trigger that `verify_baseline` tolerates in
/// test builds (`cfg(test)` or the `test-support` feature) and nowhere else.
#[cfg(any(test, feature = "test-support"))]
const TEST_FAULT_TRIGGER_PREFIX: &str = "ht_test_fault_";

type SchemaObject = (String, String, String, String);

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn schema_objects(conn: &Connection) -> rusqlite::Result<Vec<SchemaObject>> {
    let mut statement = conn.prepare(
        "SELECT type, name, tbl_name, sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )?;
    statement
        .query_map([], |row| {
            let sql: Option<String> = row.get(3)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                normalize_sql(&sql.unwrap_or_default()),
            ))
        })?
        .collect()
}

/// The objects `NODE_BASELINE` creates, read once from an in-memory database.
fn expected_objects() -> &'static [SchemaObject] {
    static EXPECTED: OnceLock<Vec<SchemaObject>> = OnceLock::new();
    EXPECTED.get_or_init(|| {
        let memory = Connection::open_in_memory().expect("in-memory baseline database");
        memory
            .execute_batch(NODE_BASELINE)
            .expect("NODE_BASELINE applies to an empty database");
        schema_objects(&memory).expect("read baseline objects")
    })
}

/// The one schema check. `NODE_BASELINE` is the reviewed schema: the open path
/// applies it to an empty store and compares every table, index, trigger and
/// view of an existing store (normalized `type, name, tbl_name, sql`) against an
/// in-memory copy of it, plus the application id, version and identity marker.
/// The first differing object is named in the `IncompatibleSchema` detail.
/// Schema changes are edits to `migrations/node/0001_node_baseline.sql` until
/// the first `v*` release that ships it, then additive deltas.
pub fn verify_baseline(conn: &Connection) -> Result<(), ApiError> {
    let incompatible = |detail: String| api_error(ErrorCode::IncompatibleSchema, detail);
    let application_id: i64 = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(store_error)?;
    if application_id != i64::from(NODE_APPLICATION_ID) {
        return Err(incompatible(format!(
            "not a herdr-threads node store (application_id {application_id:#x})"
        )));
    }
    let version = schema_version(conn)?;
    if version != NODE_SCHEMA_VERSION {
        return Err(incompatible(format!(
            "unsupported schema version {version}"
        )));
    }
    let actual = schema_objects(conn).map_err(store_error)?;
    // Test builds only: a fault trigger a test installs before the daemon or
    // writer under test starts. Never present in a default-feature build.
    #[cfg(any(test, feature = "test-support"))]
    let actual: Vec<SchemaObject> = actual
        .into_iter()
        .filter(|object| {
            !(object.0 == "trigger" && object.1.starts_with(TEST_FAULT_TRIGGER_PREFIX))
        })
        .collect();
    let expected = expected_objects();
    let (mut a, mut e) = (actual.iter().peekable(), expected.iter().peekable());
    loop {
        let key = |o: &&SchemaObject| (o.0.clone(), o.1.clone());
        match (a.peek().map(key), e.peek().map(key)) {
            (None, None) => break,
            (Some(have), None) => {
                return Err(incompatible(format!(
                    "unexpected schema object: {} {}",
                    have.0, have.1
                )));
            }
            (None, Some(want)) => {
                return Err(incompatible(format!(
                    "missing schema object: {} {}",
                    want.0, want.1
                )));
            }
            (Some(have), Some(want)) if have < want => {
                return Err(incompatible(format!(
                    "unexpected schema object: {} {}",
                    have.0, have.1
                )));
            }
            (Some(have), Some(want)) if have > want => {
                return Err(incompatible(format!(
                    "missing schema object: {} {}",
                    want.0, want.1
                )));
            }
            (Some(have), Some(_)) => {
                let (x, y) = (a.next().expect("peeked"), e.next().expect("peeked"));
                if x != y {
                    return Err(incompatible(format!(
                        "schema object differs: {} {}",
                        have.0, have.1
                    )));
                }
            }
        }
    }
    verify_identity_marker(conn)
}

fn verify_identity_marker(conn: &Connection) -> Result<(), ApiError> {
    let marker: String = conn
        .query_row("SELECT marker FROM schema_identity LIMIT 1", [], |r| {
            r.get(0)
        })
        .map_err(|_| api_error(ErrorCode::IncompatibleSchema, "missing node schema marker"))?;
    if marker != NODE_SCHEMA_MARKER {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "unknown node schema marker",
        ));
    }
    Ok(())
}

/// The schema version stamped in the database (`PRAGMA user_version`).
fn schema_version(conn: &Connection) -> Result<i64, ApiError> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(store_error)
}

/// Read workers must not scan the whole database during connection setup.
/// The writer performs the full integrity and schema audit at startup.
pub fn verify_query_connection(conn: &Connection) -> Result<(), ApiError> {
    let application_id: i64 = conn
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(store_error)?;
    let version = schema_version(conn)?;
    if application_id != i64::from(NODE_APPLICATION_ID) || version != NODE_SCHEMA_VERSION {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            format!("unsupported schema version {version} (application_id {application_id:#x})"),
        ));
    }
    verify_identity_marker(conn)
}

pub(crate) fn check_integrity(conn: &Connection) -> Result<(), ApiError> {
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(store_error)?;
    if result != "ok" {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            format!("integrity check: {result}"),
        ));
    }
    Ok(())
}

pub fn checked_deadline(start: UtcMillis, duration_ms: i64) -> Result<UtcMillis, ApiError> {
    if duration_ms <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "duration must be positive",
        ));
    }
    start
        .0
        .checked_add(duration_ms)
        .map(UtcMillis)
        .ok_or_else(|| {
            api_error(
                ErrorCode::InvalidRequest,
                "deadline overflows UTC milliseconds",
            )
        })
}

/// Allocate the next instance decision order inside the deciding writer
/// transaction. The caller shares this one value across its bounded batch.
pub fn next_decision_seq(tx: &Connection, instance: &str) -> Result<u64, ApiError> {
    let changed = tx
        .execute(
            "UPDATE local_node SET decision_seq=decision_seq+1 WHERE id=?1 AND decision_seq<?2",
            params![instance, i64::MAX],
        )
        .map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "decision sequence missing or exhausted",
        ));
    }
    let value: i64 = tx
        .query_row(
            "SELECT decision_seq FROM local_node WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(value as u64)
}

/// Every send-eligibility writer applies its seat/binding change through this
/// helper. A changed transition and the scalar fence share the same tx.
pub fn apply_eligibility_transition(
    tx: &Transaction<'_>,
    instance: &str,
    transition: impl FnOnce(&Transaction<'_>) -> Result<bool, ApiError>,
) -> Result<bool, ApiError> {
    let changed = transition(tx)?;
    if changed {
        bump_send_eligibility(tx, instance)?;
    }
    Ok(changed)
}

fn bump_send_eligibility(db: &Connection, instance: &str) -> Result<(), ApiError> {
    let rows = db.execute(
        "UPDATE local_node SET send_eligibility_revision=send_eligibility_revision+1 WHERE id=?1 AND send_eligibility_revision<?2",
        params![instance, i64::MAX],
    ).map_err(store_error)?;
    if rows != 1 {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "send eligibility revision exhausted",
        ));
    }
    Ok(())
}

/// What `messages::stage_recipient` stages for a seat from its bindings: the
/// receipt's `eligible_at_snapshot` and `availability_provenance`
/// (`effective_registered_availability`), whether a missing availability is
/// warned about now or assumed carried (`carry_pending`), and whether an ACK
/// is expected (`open_binding_role` is not human). Its other inputs are fenced
/// elsewhere: joined membership and pending invitations (thread membership
/// revision), the unavailability episode (opened and closed only with an
/// availability change), duration configuration (its own revision).
pub(crate) type StagedSeatState = (Option<String>, bool, bool);

pub(crate) fn staged_seat_state(db: &Connection, seat: &str) -> Result<StagedSeatState, ApiError> {
    let instance: String = db
        .query_row("SELECT instance_id FROM seats WHERE id=?1", [seat], |r| {
            r.get(0)
        })
        .map_err(store_error)?;
    Ok((
        effective_registered_availability(db, seat, None)?,
        carry_pending(db, seat, &instance)?,
        open_binding_role(db, &SeatId::new(seat))? == Some("human"),
    ))
}

/// Runs one binding write that can change what a send stages for `seat` (a
/// liveness transition, a registered enrollment, a reopen or placement
/// refresh, an end) and bumps the seat's instance send fence once exactly
/// when [`staged_seat_state`] differs before and after (shared contract
/// amendment §8; constant reads of the seat, at most one row update, no
/// recipient scan). A binding joining or leaving beside an unchanged
/// available binding (a subagent, a one-shot) refuses no in-flight send.
/// `write` returns its result and whether it changed a row.
pub fn fence_seat_availability<T>(
    db: &Connection,
    seat: &str,
    write: impl FnOnce() -> Result<(T, bool), ApiError>,
) -> Result<T, ApiError> {
    let before = staged_seat_state(db, seat)?;
    let (result, changed) = write()?;
    if changed && staged_seat_state(db, seat)? != before {
        let instance: String = db
            .query_row("SELECT instance_id FROM seats WHERE id=?1", [seat], |r| {
                r.get(0)
            })
            .map_err(store_error)?;
        bump_send_eligibility(db, &instance)?;
    }
    Ok(result)
}

pub fn bump_membership_revision(tx: &Transaction<'_>, thread: &ThreadId) -> Result<(), ApiError> {
    bump_thread_revision(tx, thread, "membership_revision")
}

pub fn bump_timeline_revision(tx: &Connection, thread: &ThreadId) -> Result<(), ApiError> {
    bump_thread_revision(tx, thread, "timeline_revision")
}

fn bump_thread_revision(tx: &Connection, thread: &ThreadId, column: &str) -> Result<(), ApiError> {
    // Only the two fixed call sites supply a column name.
    let sql = format!("UPDATE threads SET {column}={column}+1 WHERE id=?1 AND {column}<?2");
    let changed = tx
        .execute(&sql, params![thread.as_str(), i64::MAX])
        .map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "thread revision missing or exhausted",
        ));
    }
    Ok(())
}

pub fn bump_lifecycle_revision(tx: &Transaction<'_>, instance: &str) -> Result<(), ApiError> {
    let changed = tx.execute("UPDATE herdr_hosts SET lifecycle_revision=lifecycle_revision+1 WHERE host_id=primary_host() AND node_id=?1 AND lifecycle_revision<?2", params![instance, i64::MAX]).map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::SequenceExhausted,
            "lifecycle revision missing or exhausted",
        ));
    }
    Ok(())
}

/// The one send-time availability projection for a registered seat. A seat is
/// effectively available only when its current registered binding matches the
/// instance boot/epoch and the canonical effective target observation
/// (`effective_observation`: the published snapshot, or a strictly newer fresh
/// current-target read in the same boot/epoch) confirms the same boot, epoch
/// and structural generation. A raw `observed_targets` row from an earlier
/// epoch never decides availability on its own: after a host outage the
/// published reconfirmation snapshot advances the epoch while such rows keep
/// the old one. A binding counts only while connected (liveness; ht-54r.23): a seat is
/// available while any open, connected, registered binding is confirmed. Returns the binding's
/// observation provenance when available: deterministically the oldest
/// qualifying top-level binding's (else the oldest subagent's), so a newer
/// binding joining or leaving beside it changes neither half of the result.
/// Recipient staging and unavailability-episode opening must both use this.
pub fn effective_registered_availability(
    db: &Connection,
    seat: &str,
    instance: Option<&str>,
) -> Result<Option<String>, ApiError> {
    type BindingColumns = Option<(String, String, i64, Option<String>, i64, String)>;
    let binding: BindingColumns = db
        .query_row(
            "SELECT s.instance_id, s.target_id, s.target_generation, h.host_boot, h.host_epoch, b.observation_provenance \
             FROM seats s JOIN herdr_hosts h ON h.host_id=primary_host() AND h.node_id=s.instance_id JOIN occupant_bindings b ON b.seat_id=s.id \
             WHERE s.id=?1 AND s.state='resolved' AND s.target_id IS NOT NULL \
               AND b.ended_at IS NULL AND b.liveness='connected' AND b.registered_at IS NOT NULL AND b.target_id=s.target_id \
               AND b.generation=s.generation AND b.target_generation=s.target_generation \
               AND b.host_boot=h.host_boot AND b.host_epoch=h.host_epoch \
               AND (?2 IS NULL OR h.node_id=?2) \
             ORDER BY (b.role='top_level') DESC, b.ordinal LIMIT 1",
            params![seat, instance],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((instance, target, target_generation, host_boot, host_epoch, provenance)) = binding
    else {
        return Ok(None);
    };
    let confirmed = effective_observation(db, &instance, &target)?.is_some_and(|observation| {
        observation.structural_generation == target_generation
            && Some(observation.host_boot.as_str()) == host_boot.as_deref()
            && observation.epoch == host_epoch
    });
    Ok(confirmed.then_some(provenance))
}

/// C4 pre-reconciliation window: true while the instance's reconciliation
/// marker lags the recovery boot/epoch (`reconciled_*` IS NOT `recovery_*`;
/// publishing a new host epoch always advances `recovery_*`) and the seat's
/// open registered binding will be structurally carried and is connected, so
/// the carry will make the seat available: resolved seat, a connected binding
/// with a provenance in `CARRIED_BINDING_PROVENANCES`, the seat's target, the
/// current host boot and the structural proof's boot, an older host epoch, and
/// an incarnation equal to the seat's structural incarnation (the binding
/// predicate the planner's `bound_epoch` and `carry_binding_forward` share,
/// narrowed to connected rows). A send optimistically assumes that carry: the
/// recipient is staged as not yet available but is not warned about; the
/// carry's availability anchor starts the receipt timer. A seat whose lagging
/// bindings are all disconnected is warned about now: their carry restores no
/// availability. If the pass does not carry it, the ordinary unavailable paths
/// open the episode.
pub fn carry_pending(db: &Connection, seat: &str, instance: &str) -> Result<bool, ApiError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM seats s JOIN herdr_hosts h ON h.host_id=primary_host() AND h.node_id=s.instance_id \
         JOIN occupant_bindings b ON b.seat_id=s.id \
         WHERE s.id=?1 AND h.node_id=?2 AND s.state='resolved' \
           AND (h.reconciled_boot IS NOT h.recovery_boot OR h.reconciled_epoch IS NOT h.recovery_epoch) \
           AND b.ended_at IS NULL AND b.registered_at IS NOT NULL AND b.liveness='connected' \
           AND b.observation_provenance IN (?3,?4,?5,?6) AND b.target_id=s.target_id \
           AND b.host_boot=h.host_boot AND b.host_boot=s.structural_host_boot AND b.host_epoch<h.host_epoch \
           AND b.incarnation IS NOT NULL AND b.incarnation=s.structural_incarnation)",
        params![
            seat,
            instance,
            CARRIED_BINDING_PROVENANCES[0],
            CARRIED_BINDING_PROVENANCES[1],
            CARRIED_BINDING_PROVENANCES[2],
            CARRIED_BINDING_PROVENANCES[3]
        ],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Lazily open one durable seat outage. A global host epoch change invalidates
/// verified availability without visiting every seat; the first affected
/// deciding writer reconciles this marker. Repeated sightings of the same open
/// outage reuse the numeric episode, so warning keys stay stable.
pub fn ensure_unavailability_episode(tx: &Connection, seat: &SeatId) -> Result<u64, ApiError> {
    let (state, open, episode): (String, i64, i64) = tx
        .query_row(
            "SELECT state, unavailability_open, unavailability_episode FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(store_error)?;
    let available = effective_registered_availability(tx, seat.as_str(), None)?.is_some();
    if state == "retired" || available {
        return Err(api_error(
            ErrorCode::Conflict,
            "seat is not effectively unavailable",
        ));
    }
    if open == 1 {
        return u64::try_from(episode)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid unavailability episode"));
    }
    let next = episode
        .checked_add(1)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            api_error(
                ErrorCode::SequenceExhausted,
                "unavailability episode exhausted",
            )
        })?;
    tx.execute("UPDATE seats SET unavailability_episode=?1, unavailability_open=1 WHERE id=?2 AND unavailability_open=0",
        params![next, seat.as_str()]).map_err(store_error)?;
    Ok(next as u64)
}

pub fn canonical_digest<T: Serialize>(value: &T) -> Result<[u8; 32], ApiError> {
    let canonical = serde_json::to_value(value).map_err(|e| {
        api_error(
            ErrorCode::InvalidRequest,
            format!("invalid operation payload: {e}"),
        )
    })?;
    let bytes = serde_json::to_vec(&canonical).map_err(|e| {
        api_error(
            ErrorCode::InvalidRequest,
            format!("invalid operation payload: {e}"),
        )
    })?;
    Ok(Sha256::digest(bytes).into())
}

/// Replays return the original immutable result before a new decision sample.
/// Validation must inspect the whole command/batch before `apply` can write.
pub fn execute_idempotent_transaction(
    context: &StoreContext,
    conn: &mut Connection,
    actor_scope: &str,
    operation_key: &str,
    digest: [u8; 32],
    validate: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    apply: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<CommandResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    execute_idempotent_transaction_presented(
        context,
        conn,
        actor_scope,
        operation_key,
        digest,
        validate,
        apply,
        |_, result| Ok(result),
    )
}

// Allowed: idempotent transaction skeleton: identity plus validate/apply/present phases.
#[allow(clippy::too_many_arguments)]
pub fn execute_idempotent_transaction_presented(
    context: &StoreContext,
    conn: &mut Connection,
    actor_scope: &str,
    operation_key: &str,
    digest: [u8; 32],
    validate: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    apply: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<CommandResult, ApiError>,
    present: impl Fn(&Transaction<'_>, CommandResult) -> Result<CommandResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    if actor_scope.is_empty() || operation_key.is_empty() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "operation scope and key required",
        ));
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let stored: Option<(Vec<u8>, String)> = tx
        .query_row(
            "SELECT digest, result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![actor_scope, operation_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((old_digest, json)) = stored {
        if old_digest != digest {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        let result = decode_stored_result(&json)?;
        let result = present(&tx, result)?;
        tx.rollback().map_err(store_error)?;
        return Ok(result);
    }
    validate(&tx)?;
    let decision = DecisionInstant {
        utc: context.clock().utc_now(),
        monotonic: context.clock().monotonic_now(),
    };
    let result = apply(&tx, decision)?;
    let json = serde_json::to_string(&result).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("cannot encode result: {e}"),
        )
    })?;
    tx.execute(
        "INSERT INTO operations(actor_scope, operation_key, digest, result_json, decided_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![actor_scope, operation_key, digest.as_slice(), json, decision.utc.0],
    ).map_err(store_error)?;
    let result = present(&tx, result)?;
    tx.commit().map_err(store_error)?;
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct EventInput<'a> {
    pub thread: &'a ThreadId,
    pub key: &'a str,
    pub kind: &'a str,
    pub payload_json: &'a str,
    pub decision_at: UtcMillis,
    pub source_message: Option<&'a MessageId>,
    pub source_invitation: Option<&'a InvitationId>,
}

/// Returns the existing message ID for a duplicate event key, without taking
/// another timeline sequence. Callers can use `inserted` for wake decisions.
pub fn append_event_once(
    tx: &Connection,
    input: EventInput<'_>,
) -> Result<(MessageId, bool), ApiError> {
    append_event_once_at_seq(tx, input, None, None)
}

/// Write an immutable control audit with its actual native or service author.
/// C3 can project this field directly without interpreting the payload.
pub fn append_attributed_event_once(
    tx: &Transaction<'_>,
    input: EventInput<'_>,
    author: EventAuthor,
) -> Result<(MessageId, bool), ApiError> {
    append_event_once_at_seq(tx, input, None, Some(author))
}

/// A bounded batch can pass one instance decision sequence to all of its events.
pub fn append_event_once_with_decision_seq(
    tx: &Transaction<'_>,
    input: EventInput<'_>,
    decision_seq: u64,
) -> Result<(MessageId, bool), ApiError> {
    append_event_once_at_seq(tx, input, Some(decision_seq), None)
}

/// Attribute a membership transition at the same canonical frontier as its interval.
pub fn append_attributed_event_once_with_decision_seq(
    tx: &Transaction<'_>,
    input: EventInput<'_>,
    decision_seq: u64,
    author: EventAuthor,
) -> Result<(MessageId, bool), ApiError> {
    append_event_once_at_seq(tx, input, Some(decision_seq), Some(author))
}

fn append_event_once_at_seq(
    tx: &Connection,
    input: EventInput<'_>,
    decision_seq: Option<u64>,
    author: Option<EventAuthor>,
) -> Result<(MessageId, bool), ApiError> {
    if input.key.is_empty()
        || input.key.len() > 768
        || input.payload_json.len() > 4096
        || !matches!(input.kind, "info" | "warn")
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid bounded event",
        ));
    }
    if !matches!(
        serde_json::from_str::<serde_json::Value>(input.payload_json),
        Ok(serde_json::Value::Object(_))
    ) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "event payload must be a JSON object",
        ));
    }
    if let Some((id, thread, kind, source_message, source_invitation, author_kind, author_seat, author_service)) = tx.query_row(
        "SELECT id, thread_id, kind, source_message_id, source_invitation_id, author_kind, actor_seat_id, author_service_id FROM messages WHERE event_key=?1",
        [input.key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?,
            r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?)))
        .optional().map_err(store_error)? {
        let author_matches=match &author {
            None=>author_kind.as_deref().is_none_or(|kind|kind=="built_in") && author_seat.is_none() && author_service.is_none(),
            Some(EventAuthor::Native(seat))=>author_kind.as_deref()==Some("native") && author_seat.as_deref()==Some(seat.as_str()) && author_service.is_none(),
            Some(EventAuthor::Programmatic(service))=>author_kind.as_deref()==Some("programmatic") && author_service.as_deref()==Some(service.as_str()) && author_seat.is_none(),
            Some(EventAuthor::BuiltIn)=>author_kind.as_deref().is_none_or(|kind|kind=="built_in") && author_seat.is_none() && author_service.is_none(),
        };
        if thread != input.thread.as_str() || kind != input.kind
            || source_message.as_deref() != input.source_message.map(MessageId::as_str)
            || source_invitation.as_deref() != input.source_invitation.map(InvitationId::as_str)
            || !author_matches {
            return Err(api_error(ErrorCode::Conflict, "event key belongs to a different event"));
        }
        return Ok((MessageId::new(id), false));
    }
    let (next, instance): (i64, String) = tx
        .query_row(
            "SELECT next_sequence, instance_id FROM threads WHERE id=?1",
            [input.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let seq = match decision_seq {
        Some(seq) if seq > 0 && seq <= i64::MAX as u64 => {
            let current: i64 = tx
                .query_row(
                    "SELECT decision_seq FROM local_node WHERE id=?1",
                    [&instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if current != seq as i64 {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "event decision sequence is not current for instance",
                ));
            }
            seq
        }
        Some(_) => {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "invalid event decision sequence",
            ));
        }
        None => next_decision_seq(tx, &instance)?,
    };
    let manifest_owns_decision: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM send_manifests WHERE instance_id=?1 AND decision_seq=?2)",
            params![instance, seq as i64],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if manifest_owns_decision {
        return Err(api_error(
            ErrorCode::Conflict,
            "send publication owns event offsets",
        ));
    }
    let prior_offset: i64 = tx.query_row(
        "SELECT COALESCE(MAX(event_offset),-1) FROM messages WHERE instance_id=?1 AND decision_seq=?2",
        params![instance, seq as i64], |r| r.get(0),
    ).map_err(store_error)?;
    let event_offset = prior_offset
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "event offset exhausted"))?;
    let following = next
        .checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "timeline sequence exhausted"))?;
    tx.execute(
        "UPDATE threads SET next_sequence=?1, updated_at=MAX(updated_at,?2) WHERE id=?3",
        params![following, input.decision_at.0, input.thread.as_str()],
    )
    .map_err(store_error)?;
    let id = MessageId::new(super::public_ids::fresh(
        tx,
        prefix::EVENT,
        &[("messages", "id", prefix::EVENT)],
    )?);
    let author_role = match &author {
        Some(EventAuthor::Native(seat)) => open_binding_role(tx, seat)?,
        Some(EventAuthor::Programmatic(_)) => Some("service"),
        Some(EventAuthor::BuiltIn) | None => None,
    };
    let (author_kind, author_seat, author_service, actor_label) = match &author {
        Some(EventAuthor::Native(seat)) => (Some("native"), Some(seat.as_str()), None, None),
        Some(EventAuthor::Programmatic(service)) => (
            Some("programmatic"),
            None,
            Some(service.as_str()),
            Some("herdr-graph"),
        ),
        Some(EventAuthor::BuiltIn) | None => (None, None, None, None),
    };
    tx.execute("INSERT INTO messages(id, instance_id, thread_id, sequence, kind, event_key, event_json, decision_at, source_message_id, source_invitation_id, decision_seq, event_offset, author_kind, actor_seat_id, author_service_id, actor_label, author_role) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![id.as_str(), instance, input.thread.as_str(), next, input.kind, input.key, input.payload_json,
            input.decision_at.0, input.source_message.map(MessageId::as_str), input.source_invitation.map(InvitationId::as_str), seq as i64, event_offset,
            author_kind,author_seat,author_service,actor_label,author_role])
        .map_err(store_error)?;
    bump_timeline_revision(tx, input.thread)?;
    Ok((id, true))
}

/// Spec §1: the author role a seat's open bindings give its messages at decision time. A seat
/// holds many bindings (ht-54r.3). It reads `human` when a `human` binding is open (a person and
/// an agent never share a seat, TRUST-POLICY A4); otherwise `agent` while any binding is open;
/// no open binding records NULL (read as agent).
pub fn open_binding_role(tx: &Connection, seat: &SeatId) -> Result<Option<&'static str>, ApiError> {
    let (open, human): (bool, bool) = tx
        .query_row(
            "SELECT COUNT(*)>0, COALESCE(MAX(harness='human'),0) FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok(open.then_some(if human { "human" } else { "agent" }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveState {
    pub state: String,
    pub retired_at: Option<UtcMillis>,
}

pub fn effective_membership_state(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<Option<EffectiveState>, ApiError> {
    tx.query_row("SELECT CASE WHEN s.state='retired' THEN 'retired' ELSE m.state END, s.retired_at FROM memberships m JOIN seats s ON s.id=m.seat_id WHERE m.thread_id=?1 AND m.seat_id=?2",
        params![thread.as_str(), seat.as_str()], |r| Ok(EffectiveState { state: r.get(0)?, retired_at: r.get::<_, Option<i64>>(1)?.map(UtcMillis) }))
        .optional().map_err(store_error)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverdueOutcome {
    pub warning: Option<MessageId>,
    pub inserted: bool,
}

/// The retirement time basis is valid only for a job and its fenced seat.
/// Ordinary scans ignore effectively terminal obligations. This helper does
/// not settle the obligation; the handler performs settlement in the same tx.
pub fn record_overdue_if_pending(
    tx: &Transaction<'_>,
    obligation: &ObligationRef,
    basis: &TimeBasis,
    decision_at: UtcMillis,
) -> Result<OverdueOutcome, ApiError> {
    record_overdue_inner(tx, obligation, basis, decision_at, None)
}

#[allow(clippy::too_many_arguments)]
fn enqueue_transition_warning(
    tx: &Connection,
    warning: &str,
    event_seq: i64,
    thread: &str,
    interval_high_water: i64,
    affected: &str,
    condition_kind: &str,
    recipient_cutoff_seq: Option<i64>,
) -> Result<(), ApiError> {
    tx.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,recipient_cutoff_seq) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![warning,event_seq,thread,interval_high_water,affected,condition_kind,format!("transition:{warning}"),recipient_cutoff_seq]).map_err(store_error)?;
    if condition_kind == "unavailable" {
        tx.execute("INSERT INTO digest_open_warnings(source,source_ordinal,warning_id,thread_id,affected_seat_id,condition_kind,condition_id) SELECT 'job',ordinal,warning_id,thread_id,affected_seat_id,condition_kind,condition_id FROM warning_jobs WHERE warning_id=?1",
            [warning]).map_err(store_error)?;
    }
    tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'warning_attribution',?2,?3)",
        params![format!("work:{warning}"),warning,interval_high_water]).map_err(store_error)?;
    Ok(())
}

/// Freeze the native rejection's audience in its deciding transaction. Reuse
/// the bounded transition fanout without creating an open warning condition.
pub(crate) fn enqueue_invitation_rejection_notice(
    tx: &Connection,
    warning: &str,
    thread: &str,
    actor: &str,
) -> Result<(), ApiError> {
    let seq: i64 = tx
        .query_row(
            "SELECT decision_seq FROM messages WHERE id=?1",
            [warning],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let high_water: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
            [thread],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    enqueue_transition_warning(
        tx,
        warning,
        seq,
        thread,
        high_water,
        actor,
        "invitation",
        Some(seq),
    )
}

/// Manifest publication fixes the unavailable warning's logical event and
/// recipient high water. Persist its transition before a projection worker can
/// lag or the seat's episode can close.
pub fn ensure_unavailability_reopen_ready(
    tx: &Transaction<'_>,
    preparation: &str,
    thread: &str,
) -> Result<(), ApiError> {
    let pending_prior: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM prepared_unavailable_warnings w JOIN warning_conditions c INDEXED BY warning_conditions_unavailable_reopen ON c.thread_id=?2 AND c.affected_seat_id=w.affected_seat_id AND c.condition_kind='unavailable' AND c.clear_warning_id IS NULL AND c.episode<>w.unavailability_episode WHERE w.preparation_id=?1 AND EXISTS(SELECT 1 FROM warning_close_sweeps s WHERE s.condition_kind='unavailable' AND s.affected_seat_id=c.affected_seat_id AND s.episode=c.episode AND s.after_ordinal<c.ordinal AND c.ordinal<=s.through_ordinal))",
        params![preparation,thread], |r| r.get(0),
    ).map_err(store_error)?;
    if pending_prior {
        return Err(api_error(
            ErrorCode::StoreBusy,
            "unavailability clear must publish before episode reopens",
        ));
    }
    Ok(())
}

pub fn record_published_unavailable_conditions(
    tx: &Transaction<'_>,
    preparation: &str,
    thread: &str,
    event_seq: i64,
    interval_high_water: i64,
    decision_at: UtcMillis,
) -> Result<(), ApiError> {
    let rows: Vec<(String, String, i64)> = {
        let mut stmt = tx.prepare("SELECT warning_id,affected_seat_id,unavailability_episode FROM prepared_unavailable_warnings WHERE preparation_id=?1 ORDER BY warning_offset").map_err(store_error)?;
        stmt.query_map([preparation], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
    };
    for (warning, seat, episode) in rows {
        let identity = format!("{}:{}:{episode}", seat.len(), seat);
        tx.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,episode,open_warning_id,opened_seq) VALUES ('unavailable',?1,?2,?3,?4,?5,?6)",
            params![thread,identity,seat,episode,warning,event_seq]).map_err(store_error)?;
        enqueue_transition_warning(
            tx,
            &warning,
            event_seq,
            thread,
            interval_high_water,
            &seat,
            "unavailable",
            None,
        )?;
        // A manifest published after this episode's canonical close is both
        // opened and cleared in the publication transaction. The sweep's
        // captured ordinal cannot include this newly published condition.
        let still_open: bool = tx.query_row(
            "SELECT unavailability_open=1 AND unavailability_episode=?2 AND state!='retired' FROM seats WHERE id=?1",
            params![seat,episode], |r| r.get(0),
        ).map_err(store_error)?;
        if !still_open {
            enqueue_warning_close_sweep(tx, "unavailable", &seat, Some(episode), decision_at)?;
        }
        // The v8 prepared projection is source-gated and will be removed on
        // episode close. The transition job is the durable notification source.
        tx.execute(
            "DELETE FROM digest_open_warnings WHERE source='prepared' AND warning_id=?1",
            [&warning],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

pub fn clear_warning_conditions_for_unavailable(
    tx: &Transaction<'_>,
    seat: &str,
    episode: i64,
    decision_at: UtcMillis,
) -> Result<u32, ApiError> {
    enqueue_warning_close_sweep(tx, "unavailable", seat, Some(episode), decision_at)
}

pub fn clear_open_unavailability_for_seat(
    tx: &Transaction<'_>,
    seat: &str,
    decision_at: UtcMillis,
) -> Result<u32, ApiError> {
    let episode: Option<i64> = tx
        .query_row(
            "SELECT unavailability_episode FROM seats WHERE id=?1 AND unavailability_open=1",
            [seat],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    episode.map_or(Ok(0), |episode| {
        clear_warning_conditions_for_unavailable(tx, seat, episode, decision_at)
    })
}

pub(crate) fn clear_warning_condition(
    tx: &Connection,
    ordinal: i64,
    decision_at: UtcMillis,
) -> Result<(), ApiError> {
    clear_warning_condition_at_snapshot(tx, ordinal, decision_at, None)
}

/// Optional recipient cutoff and global membership high water were captured
/// by a prior canonical close decision, before this event was materialized.
pub(crate) fn clear_warning_condition_at_snapshot(
    tx: &Connection,
    ordinal: i64,
    decision_at: UtcMillis,
    snapshot: Option<(i64, i64)>,
) -> Result<(), ApiError> {
    let (kind,thread,affected,open,condition_id): (String,String,String,String,String) = tx.query_row(
        "SELECT condition_kind,thread_id,affected_seat_id,open_warning_id,condition_id FROM warning_conditions WHERE ordinal=?1 AND clear_warning_id IS NULL",
        [ordinal], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).map_err(store_error)?;
    let thread_id = ThreadId::new(&thread);
    let key = format!("warning-clear:{open}");
    let payload = serde_json::json!({"event":"warning_cleared","condition":kind,"open_warning":open,"affected_seat":affected}).to_string();
    let (clear, inserted) = append_event_once(
        tx,
        EventInput {
            thread: &thread_id,
            key: &key,
            kind: "warn",
            payload_json: &payload,
            decision_at,
            source_message: None,
            source_invitation: None,
        },
    )?;
    if !inserted {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "clear warning event already existed without condition close",
        ));
    }
    let seq: i64 = tx
        .query_row(
            "SELECT decision_seq FROM messages WHERE id=?1",
            [clear.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let high_water: i64 = if let Some((_, high_water)) = snapshot {
        high_water
    } else {
        tx.query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
            [&thread],
            |r| r.get(0),
        )
        .map_err(store_error)?
    };
    tx.execute("UPDATE warning_conditions SET clear_warning_id=?1,cleared_seq=?2 WHERE ordinal=?3 AND clear_warning_id IS NULL",
        params![clear.as_str(),seq,ordinal]).map_err(store_error)?;
    enqueue_transition_warning(
        tx,
        clear.as_str(),
        seq,
        &thread,
        high_water,
        &affected,
        &kind,
        snapshot.map(|(cutoff, _)| cutoff),
    )?;
    // The original inviter retains the informational clear even after leaving.
    // Unlike an open condition this projection does not enqueue wake work.
    if kind == "invitation"
        && let Some(origin) = super::effective::invitation_originator(tx, &condition_id)?
    {
        tx.execute("INSERT OR IGNORE INTO warning_recipients(warning_id,seat_id,generation) SELECT ?1,id,?3 FROM seats WHERE id=?2 AND state!='retired'",params![clear.as_str(),origin,seq]).map_err(store_error)?;
    }
    Ok(())
}

/// Close only a condition opened by this schema, once every warned receipt
/// under its thread/recipient backlog has ceased to be required and pending.
pub fn clear_warning_conditions_for_receipts(
    tx: &Transaction<'_>,
    thread: &str,
    seat: &str,
    decision_at: UtcMillis,
) -> Result<u32, ApiError> {
    let open: Option<(i64,String)> = tx.query_row(
        "SELECT c.ordinal,c.open_warning_id FROM warning_conditions c WHERE c.condition_kind='receipt' AND c.thread_id=?1 AND c.condition_id=?2 AND c.clear_warning_id IS NULL AND NOT EXISTS(SELECT 1 FROM warning_close_sweeps s WHERE s.condition_kind='receipt' AND s.affected_seat_id=c.affected_seat_id AND c.ordinal<=s.through_ordinal) ORDER BY c.ordinal DESC LIMIT 1",
        params![thread,seat], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(store_error)?;
    let Some((ordinal, warning)) = open else {
        return Ok(0);
    };
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM receipts r JOIN messages m ON m.id=r.message_id WHERE r.thread_id=?1 AND r.seat_id=?2 AND r.warning_message_id=?3 AND r.state='pending' AND r.ack_required=1 AND NOT EXISTS(SELECT 1 FROM human_receipt_waivers w WHERE w.seat_id=r.seat_id AND m.decision_seq<=w.through_decision_seq)) OR EXISTS(SELECT 1 FROM receipt_state rs JOIN messages m ON m.id=rs.message_id WHERE m.thread_id=?1 AND rs.seat_id=?2 AND rs.warning_message_id=?3 AND rs.state='pending' AND rs.ack_required=1 AND NOT EXISTS(SELECT 1 FROM human_receipt_waivers w WHERE w.seat_id=rs.seat_id AND m.decision_seq<=w.through_decision_seq))",
        params![thread,seat,warning], |r| r.get(0),
    ).map_err(store_error)?;
    if pending {
        return Ok(0);
    }
    clear_warning_condition(tx, ordinal, decision_at)?;
    Ok(1)
}

pub fn clear_waived_receipt_conditions_for_seat(
    tx: &Transaction<'_>,
    seat: &str,
    decision_at: UtcMillis,
) -> Result<u32, ApiError> {
    enqueue_warning_close_sweep(tx, "receipt", seat, None, decision_at)
}

fn enqueue_warning_close_sweep(
    tx: &Transaction<'_>,
    kind: &str,
    seat: &str,
    episode: Option<i64>,
    decision_at: UtcMillis,
) -> Result<u32, ApiError> {
    let high_water: i64 = tx.query_row(
        "SELECT COALESCE(MAX(ordinal),0) FROM warning_conditions INDEXED BY warning_conditions_close_scope WHERE condition_kind=?1 AND affected_seat_id=?2 AND episode IS ?3 AND clear_warning_id IS NULL",
        params![kind, seat, episode], |r| r.get(0),
    ).map_err(store_error)?;
    if high_water == 0 {
        return Ok(0);
    }
    let after_ordinal: i64 = tx.query_row(
        "SELECT COALESCE(MAX(through_ordinal),0) FROM warning_close_sweeps INDEXED BY warning_close_sweeps_cover WHERE condition_kind=?1 AND affected_seat_id=?2 AND episode IS ?3",
        params![kind,seat,episode], |r| r.get(0),
    ).map_err(store_error)?;
    if high_water <= after_ordinal {
        return Ok(0);
    }
    let close_decision_seq: i64 = tx.query_row(
        "SELECT h.decision_seq FROM seats s JOIN local_node h ON h.id=s.instance_id WHERE s.id=?1",
        [seat], |r| r.get(0),
    ).map_err(store_error)?;
    let interval_high_water: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let id = format!(
        "warning-close:{kind}:{seat}:{}:{high_water}",
        episode.unwrap_or(0)
    );
    let inserted = tx.execute(
        "INSERT INTO warning_close_sweeps(id,condition_kind,affected_seat_id,episode,after_ordinal,through_ordinal,close_decision_seq,interval_high_water,decision_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(id) DO NOTHING",
        params![id,kind,seat,episode,after_ordinal,high_water,close_decision_seq,interval_high_water,decision_at.0],
    ).map_err(store_error)?;
    if inserted != 0 {
        tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'warning_condition_close',?2,?3)",
            params![format!("work:{id}"),id,high_water]).map_err(store_error)?;
    }
    Ok(u32::from(inserted != 0))
}

pub fn clear_warning_condition_for_invitation(
    tx: &Transaction<'_>,
    invitation: &str,
    decision_at: UtcMillis,
) -> Result<bool, ApiError> {
    let open: Option<i64> = tx.query_row(
        "SELECT ordinal FROM warning_conditions WHERE condition_kind='invitation' AND condition_id=?1 AND clear_warning_id IS NULL",
        [invitation], |r| r.get(0),
    ).optional().map_err(store_error)?;
    let Some(ordinal) = open else {
        return Ok(false);
    };
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM invitations i WHERE i.id=?1 AND i.state='pending' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id))",
        [invitation], |r| r.get(0),
    ).map_err(store_error)?;
    if pending {
        return Ok(false);
    }
    clear_warning_condition(tx, ordinal, decision_at)?;
    Ok(true)
}

pub fn record_overdue_with_decision_seq(
    tx: &Transaction<'_>,
    obligation: &ObligationRef,
    basis: &TimeBasis,
    decision_at: UtcMillis,
    decision_seq: u64,
) -> Result<OverdueOutcome, ApiError> {
    record_overdue_inner(tx, obligation, basis, decision_at, Some(decision_seq))
}

fn record_overdue_inner(
    tx: &Transaction<'_>,
    obligation: &ObligationRef,
    basis: &TimeBasis,
    decision_at: UtcMillis,
    decision_seq: Option<u64>,
) -> Result<OverdueOutcome, ApiError> {
    let (thread, seat, deadline, state, source_message, source_invitation, key, manifest_receipt) =
        match obligation {
            ObligationRef::Invitation(id) => {
                let row: (String, String, Option<i64>, String) = tx
                .query_row(
                    "SELECT i.thread_id, i.seat_id, CASE WHEN i.confirmation='lazy' THEN NULL ELSE i.deadline_at END, CASE WHEN EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) THEN 'rejected' WHEN c.invitation_id IS NOT NULL THEN 'cancelled' ELSE i.state END FROM invitations i LEFT JOIN invitation_cancellations c ON c.invitation_id=i.id WHERE i.id=?1",
                    [id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .map_err(store_error)?;
                (
                    ThreadId::new(row.0),
                    SeatId::new(row.1),
                    row.2,
                    row.3,
                    None,
                    Some(id.clone()),
                    format!("overdue:invitation:{}", id.as_str()),
                    false,
                )
            }
            ObligationRef::Receipt { message, seat } => {
                let row = effective_receipt(tx, message.as_str(), seat.as_str())?
                    .ok_or_else(|| api_error(ErrorCode::NotFound, "receipt obligation missing"))?;
                if row.state == EffectiveReceiptState::NotRequired {
                    return Ok(OverdueOutcome {
                        warning: None,
                        inserted: false,
                    });
                }
                // A retirement worker classifies the obligation at its cutover.
                // The current effective projection is already retired, so read
                // the durable pre-cutover settlement state for that path.
                let status = if matches!(basis, TimeBasis::Retirement(_)) {
                    match row.source {
                        ReceiptSource::Physical => tx.query_row(
                            "SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2",
                            params![message.as_str(), seat.as_str()],
                            |r| r.get::<_, String>(0),
                        ).map_err(store_error)?,
                        ReceiptSource::Manifest => tx.query_row(
                            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
                            params![message.as_str(), seat.as_str()],
                            |r| r.get::<_, String>(0),
                        ).optional().map_err(store_error)?.unwrap_or_else(|| "pending".to_owned()),
                    }
                } else {
                    match row.state {
                        EffectiveReceiptState::Pending => "pending",
                        EffectiveReceiptState::Acknowledged => "acked",
                        EffectiveReceiptState::RecipientRetired => "recipient_retired",
                        EffectiveReceiptState::NotRequired => "not_required",
                    }
                    .to_owned()
                };
                (
                    ThreadId::new(row.thread_id),
                    seat.clone(),
                    row.deadline_at,
                    status,
                    Some(message.clone()),
                    None,
                    format!(
                        "overdue:receipt:{}:{}:{}",
                        message.as_str().len(),
                        message.as_str(),
                        seat.as_str()
                    ),
                    row.source == ReceiptSource::Manifest,
                )
            }
            _ => {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "not a deadline obligation",
                ));
            }
        };
    if state != "pending" {
        return Ok(OverdueOutcome {
            warning: None,
            inserted: false,
        });
    }
    let seat_state: (String, Option<i64>) = tx
        .query_row(
            "SELECT state, retired_at FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let classify_at = match basis {
        TimeBasis::Decision => {
            if seat_state.0 == "retired" {
                return Ok(OverdueOutcome {
                    warning: None,
                    inserted: false,
                });
            }
            decision_at.0
        }
        TimeBasis::Retirement(job) => {
            let (job_seat, cutover): (String, i64) = tx
                .query_row(
                    "SELECT seat_id, cutover_at FROM retirements WHERE id=?1",
                    [job.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(store_error)?;
            if job_seat != seat.as_str()
                || seat_state.0 != "retired"
                || seat_state.1 != Some(cutover)
            {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "retirement job does not match fenced seat",
                ));
            }
            cutover
        }
    };
    let comparison_deadline = if matches!(obligation, ObligationRef::Receipt { .. }) {
        super::receipts::effective_deadline_for(tx, seat.as_str(), thread.as_str(), deadline)?
    } else {
        deadline
    };
    if comparison_deadline.is_none_or(|deadline| classify_at < deadline) {
        return Ok(OverdueOutcome {
            warning: None,
            inserted: false,
        });
    }
    let effective_status = if matches!(basis, TimeBasis::Retirement(_)) {
        "recipient_retired"
    } else {
        "pending"
    };
    let payload = serde_json::json!({
        "obligation": match obligation { ObligationRef::Invitation(_) => "invitation", _ => "receipt" },
        "seat": seat.as_str(), "deadline_at": deadline,
        "classified_at": classify_at, "materialized_at": decision_at.0,
        "current_status": effective_status, "physical_status": "pending",
    }).to_string();
    let event = EventInput {
        thread: &thread,
        key: &key,
        kind: "warn",
        payload_json: &payload,
        decision_at,
        source_message: source_message.as_ref(),
        source_invitation: source_invitation.as_ref(),
    };
    let condition = match obligation {
        ObligationRef::Invitation(id) => ("invitation", id.as_str()),
        ObligationRef::Receipt { seat, .. } => ("receipt", seat.as_str()),
        _ => unreachable!(),
    };
    if matches!(basis, TimeBasis::Decision) && condition.0 == "receipt" {
        // A human waiver may have marked the prior backlog closed while its
        // worker is still draining other threads. Close this one old row now
        // so a reopened warning follows its clear on this thread's timeline.
        let old: Vec<(i64, i64, i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT c.ordinal,s.decision_at,s.close_decision_seq,s.interval_high_water FROM warning_conditions c JOIN warning_close_sweeps s INDEXED BY warning_close_sweeps_cover ON s.condition_kind='receipt' AND s.affected_seat_id=c.affected_seat_id AND s.episode IS NULL AND s.after_ordinal<c.ordinal AND c.ordinal<=s.through_ordinal WHERE c.condition_kind='receipt' AND c.thread_id=?1 AND c.condition_id=?2 AND c.clear_warning_id IS NULL ORDER BY c.ordinal LIMIT 2",
            ).map_err(store_error)?;
            stmt.query_map(params![thread.as_str(), condition.1], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
        };
        if old.len() > 1 {
            return Err(api_error(
                ErrorCode::StoreBusy,
                "warning close sweep must catch up before reopen",
            ));
        }
        if let Some((ordinal, close_at, cutoff, intervals)) = old.first() {
            clear_warning_condition_at_snapshot(
                tx,
                *ordinal,
                UtcMillis(*close_at),
                Some((*cutoff, *intervals)),
            )?;
        }
    }
    let active: Option<String> = if matches!(basis, TimeBasis::Decision) {
        tx.query_row(
            "SELECT c.open_warning_id FROM warning_conditions c WHERE c.condition_kind=?1 AND c.thread_id=?2 AND c.condition_id=?3 AND c.clear_warning_id IS NULL AND NOT EXISTS(SELECT 1 FROM warning_close_sweeps s WHERE s.condition_kind=c.condition_kind AND s.affected_seat_id=c.affected_seat_id AND c.ordinal<=s.through_ordinal AND (c.condition_kind='receipt' OR s.episode=c.episode)) ORDER BY c.ordinal DESC LIMIT 1",
            params![condition.0, thread.as_str(), condition.1],
            |r| r.get(0),
        ).optional().map_err(store_error)?
    } else {
        None
    };
    let (warning, inserted) = match active {
        Some(id) => (MessageId::new(id), false),
        None => append_event_once_at_seq(tx, event, decision_seq, None)?,
    };
    let (source_table, source_where, source_parameters): (&str, &str, Vec<String>) =
        match obligation {
            ObligationRef::Invitation(id) => ("invitations", "id=?2", vec![id.as_str().to_owned()]),
            ObligationRef::Receipt { message, seat } => (
                if manifest_receipt {
                    "receipt_state"
                } else {
                    "receipts"
                },
                "message_id=?2 AND seat_id=?3",
                vec![message.as_str().to_owned(), seat.as_str().to_owned()],
            ),
            _ => unreachable!(),
        };
    let marker_sql = format!(
        "UPDATE {source_table} SET warning_message_id=?1 WHERE {source_where} AND (warning_message_id IS NULL OR warning_message_id=?1)"
    );
    if manifest_receipt {
        tx.execute("INSERT INTO receipt_state(message_id, seat_id, state, warning_message_id) VALUES (?1, ?2, 'pending', ?3) ON CONFLICT(message_id,seat_id) DO UPDATE SET warning_message_id=excluded.warning_message_id WHERE receipt_state.warning_message_id IS NULL OR receipt_state.warning_message_id=excluded.warning_message_id",
            params![source_message.as_ref().unwrap().as_str(), seat.as_str(), warning.as_str()]).map_err(store_error)?;
    }
    let changed = match source_parameters.as_slice() {
        [one] => tx.execute(&marker_sql, params![warning.as_str(), one]),
        [one, two] => tx.execute(&marker_sql, params![warning.as_str(), one, two]),
        _ => unreachable!(),
    }
    .map_err(store_error)?;
    if changed != 1 {
        return Err(api_error(
            ErrorCode::Conflict,
            "overdue source warning marker conflicts",
        ));
    }
    if matches!(basis, TimeBasis::Decision) && inserted {
        let event_seq: i64 = tx
            .query_row(
                "SELECT decision_seq FROM messages WHERE id=?1",
                [warning.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        let interval_high_water: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals WHERE thread_id=?1",
                [thread.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        let origin = source_invitation
            .as_ref()
            .map(|id| super::effective::invitation_originator(tx, id.as_str()))
            .transpose()?
            .flatten();
        tx.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq,invitation_originator_instance_id,invitation_originator_seat_id) VALUES (?1,?2,?3,?4,?5,?6,CASE WHEN ?7 IS NOT NULL THEN (SELECT instance_id FROM threads WHERE id=?2) END,?7)",
            params![condition.0,thread.as_str(),condition.1,seat.as_str(),warning.as_str(),event_seq,origin]).map_err(store_error)?;
        enqueue_transition_warning(
            tx,
            warning.as_str(),
            event_seq,
            thread.as_str(),
            interval_high_water,
            seat.as_str(),
            condition.0,
            None,
        )?;
        if let Some(origin) = origin {
            tx.execute("INSERT OR IGNORE INTO warning_recipients(warning_id,seat_id,generation) SELECT ?1,id,?3 FROM seats WHERE id=?2 AND state!='retired'",params![warning.as_str(),origin,event_seq]).map_err(store_error)?;
            tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) SELECT id,4,1 FROM seats WHERE id=?1 AND state!='retired' ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|4,attention_version=attention_version+1",[origin]).map_err(store_error)?;
        }
    }
    Ok(OverdueOutcome {
        warning: Some(warning),
        inserted,
    })
}

pub fn bump_filter_revision(
    tx: &Transaction<'_>,
    instance: &str,
    kind: &str,
    key: &str,
) -> Result<(), ApiError> {
    if !matches!(kind, "directory" | "inbox" | "topic") {
        return Err(api_error(ErrorCode::InvalidRequest, "invalid filter scope"));
    }
    tx.execute("INSERT INTO filter_revisions(instance_id, scope_kind, scope_key, revision) VALUES (?1, ?2, ?3, 1) ON CONFLICT(instance_id, scope_kind, scope_key) DO UPDATE SET revision=revision+1",
        params![instance, kind, key]).map_err(store_error)?;
    Ok(())
}

pub fn retirement_audit_key(job: &RetirementJobId, thread: &ThreadId) -> String {
    format!(
        "retirement:{}:{}:{}",
        job.as_str().len(),
        job.as_str(),
        thread.as_str()
    )
}

/// Exact replay and bounded predecision validation share the accepted writer
/// boundary. Instance checks run before replay; replay performs no domain writes.
// Allowed: idempotent transaction skeleton: identity, budget and its phase closures.
#[allow(clippy::too_many_arguments)]
pub fn execute_budgeted_idempotent_transaction(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    actor_scope: &str,
    operation_key: &str,
    digest: [u8; 32],
    before_replay: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    validate: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    apply: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<CommandResult, ApiError>,
    present: impl Fn(&Transaction<'_>, CommandResult) -> Result<CommandResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    execute_budgeted_idempotent_transaction_with_constraints(
        context,
        conn,
        budget,
        None,
        actor_scope,
        operation_key,
        digest,
        before_replay,
        validate,
        apply,
        present,
    )
}

// Allowed: idempotent transaction skeleton: identity, budgets and its phase closures.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_budgeted_idempotent_transaction_with_constraints(
    context: &StoreContext,
    conn: &mut Connection,
    budget: &crate::protocol::time::CallBudget,
    issuance: Option<&crate::protocol::time::CallBudget>,
    actor_scope: &str,
    operation_key: &str,
    digest: [u8; 32],
    before_replay: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    validate: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    apply: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<CommandResult, ApiError>,
    present: impl Fn(&Transaction<'_>, CommandResult) -> Result<CommandResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    if actor_scope.is_empty() || operation_key.is_empty() {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "operation scope and key required",
        ));
    }
    context.execute_budgeted_decision_with_constraints(conn,budget,issuance,|tx| {
        before_replay(tx)?;
        let stored: Option<(Vec<u8>,String)>=tx.query_row(
            "SELECT digest,result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![actor_scope,operation_key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
        if let Some((old,json))=stored {
            if old != digest {return Err(api_error(ErrorCode::OperationPayloadMismatch,"operation key reused with different payload"));}
            let result=decode_stored_result(&json)?;
            return Ok(Some(result));
        }
        validate(tx)?;
        Ok(None)
    },|tx,decision,replay| {
        if let Some(result)=replay {return present(tx,result);}
        let result=apply(tx,decision)?;
        let json=serde_json::to_string(&result).map_err(|e|api_error(ErrorCode::StoreCorrupt,format!("cannot encode result: {e}")))?;
        tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES (?1,?2,?3,?4,?5)",
            params![actor_scope,operation_key,digest.as_slice(),json,decision.utc.0]).map_err(store_error)?;
        present(tx,result)
    })
}

/// Accountable decisions use the explicit current call budget plus independent
/// issuance constraints, and validate instance before historical lookup.
// Allowed: accountable transaction skeleton: identity, budgets and its phase closures.
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_accountable_transaction(
    context: &StoreContext,
    conn: &mut Connection,
    current_budget: &crate::protocol::time::CallBudget,
    cooperative: (
        crate::protocol::authority::CallerClaim,
        crate::protocol::time::CallBudget,
        Option<crate::protocol::authority::HandoffRequirement>,
    ),
    actor_scope: &str,
    operation_key: &str,
    digest: [u8; 32],
    validate: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    apply: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<CommandResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    let (claim, issuance, delivery) = cooperative;
    execute_budgeted_idempotent_transaction_with_constraints(
        context,
        conn,
        current_budget,
        Some(&issuance),
        actor_scope,
        operation_key,
        digest,
        |tx| {
            super::seats::cooperative_instance(tx, &claim.instance, &claim)?;
            if let Some(delivery) = &delivery {
                super::handoff::validate_handoff_requirement(tx, delivery, true)?;
            } else {
                super::topology_handoff::guard_unscoped_child_phase(
                    tx,
                    &claim.instance,
                    actor_scope,
                    operation_key,
                )?;
            }
            Ok(())
        },
        validate,
        apply,
        |_, result| Ok(result),
    )
}
