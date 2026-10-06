//! Schema v1 and common transaction primitives used by store handlers.

use crate::{
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

use super::connection::{DecisionInstant, StoreContext, api_error, store_error};
use super::effective::{
    EffectiveReceiptState, ReceiptSource, effective_observation, effective_receipt,
};

const V1: &str = include_str!("../../migrations/0001_initial.sql");
const V2: &str = include_str!("../../migrations/0002_service_substrate.sql");
const V3: &str = include_str!("../../migrations/0003_invitation_cancellations.sql");
const V4: &str = include_str!("../../migrations/0004_voluntary_membership.sql");
const V5: &str = include_str!("../../migrations/0005_service_notifications.sql");
const V6: &str = include_str!("../../migrations/0006_retirement_health.sql");
/// Seat- and thread-leading indexes for the attention digest producer.
const V7: &str = include_str!("../../migrations/0007_attention_digest.sql");
/// Pending-only projections (and their maintaining triggers) for the digest.
const V8: &str = include_str!("../../migrations/0008_digest_pending_paths.sql");
/// Occupant bindings also accept a person's pane identity (`harness='human'`).
const V9: &str = include_str!("../../migrations/0009_human_occupant.sql");
/// B5 trust guards: reconciliation marker, wider allocation kinds and the
/// cooperative-continuity diagnostic column.
const V10: &str = include_str!("../../migrations/0010_b5_trust_guards.sql");
/// Cooperative-only skeleton (ht-p03.2): the table sweep found nothing the
/// deleted verification layer alone used, so it drops no table; B1 appends its
/// partial indexes to this migration. (Renumbered from v10 to v11 when main's
/// B5 trust guards took v10.)
const V11: &str = include_str!("../../migrations/0011_cooperative_only.sql");
/// Harness version evidence (ht-xoc.4): `harness_version_evidence` and
/// `harness_unattributed`.
const V12: &str = include_str!("../../migrations/0012_harness_version_evidence.sql");
/// Thread summaries: author role and relay claims on messages, summary blocks, jobs,
/// catch-up rows and the receipt soft-poke marker (epic ht-1ip). (Renumbered
/// from v11 to v12 when main's cooperative-only skeleton took v11, then to v13
/// when main's harness version evidence took v12.)
const V13: &str = include_str!("../../migrations/0013_thread_summaries.sql");
/// Catch-up release key (epic ht-1ip; renumbered from v12 to v13, then v14).
const V14: &str = include_str!("../../migrations/0014_catch_up_release.sql");
/// The schema version this build writes and audits (the last migration).
const V15: &str = include_str!("../../migrations/0015_preparation_retention.sql");
const V16: &str = include_str!("../../migrations/0016_human_receipt_waivers.sql");
const V17: &str = include_str!("../../migrations/0017_wake_batches.sql");
const V18: &str = include_str!("../../migrations/0018_warning_conditions.sql");
const V19: &str = include_str!("../../migrations/0019_thread_names.sql");
const V20: &str = include_str!("../../migrations/0020_recent_activity.sql");
const V21: &str = include_str!("../../migrations/0021_invitation_rejections.sql");
const V22: &str = include_str!("../../migrations/0022_user_message_intent.sql");
const V23: &str = include_str!("../../migrations/0023_channel_archival.sql");
const V24: &str = include_str!("../../migrations/0024_harness_contract_diagnostics.sql");
const V25: &str = include_str!("../../migrations/0025_harness_adapters.sql");
pub(crate) const LATEST_VERSION: i64 = 25;

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

/// The migration clock is sampled only when legacy preparations need fresh grace.
pub fn initialize(conn: &Connection, now: impl FnOnce() -> UtcMillis) -> Result<(), ApiError> {
    check_integrity(conn)?;
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(store_error)?;
    match version {
        0 => {
            let objects: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if objects != 0 {
                return Err(api_error(
                    ErrorCode::IncompatibleSchema,
                    "unversioned nonempty database",
                ));
            }
            conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
            let result = conn
                .execute_batch(V1)
                .and_then(|_| conn.execute_batch(V2))
                .and_then(|_| conn.execute_batch(V3))
                .and_then(|_| conn.execute_batch(V4))
                .and_then(|_| conn.execute_batch(V5))
                .and_then(|_| conn.execute_batch(V6))
                .and_then(|_| conn.execute_batch(V7))
                .and_then(|_| conn.execute_batch(V8))
                .and_then(|_| conn.execute_batch(V9))
                .and_then(|_| conn.execute_batch(V10))
                .and_then(|_| conn.execute_batch(V11))
                .and_then(|_| conn.execute_batch(V12))
                .and_then(|_| conn.execute_batch(V13))
                .and_then(|_| conn.execute_batch(V14))
                .and_then(|_| conn.execute_batch(V15))
                .and_then(|_| conn.execute_batch(V16))
                .and_then(|_| conn.execute_batch(V17))
                .and_then(|_| conn.execute_batch(V18))
                .and_then(|_| conn.execute_batch(V19))
                .and_then(|_| conn.execute_batch(V20))
                .and_then(|_| conn.execute_batch(V21))
                .and_then(|_| conn.execute_batch(V22))
                .and_then(|_| conn.execute_batch(V23))
                .and_then(|_| conn.execute_batch(V24))
                .and_then(|_| conn.execute_batch(V25))
                .and_then(|_| conn.pragma_update(None, "user_version", LATEST_VERSION));
            match result {
                Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
                Err(error) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(store_error(error));
                }
            }
            verify_existing(conn)
        }
        1 => {
            verify_existing_v1(conn)?;
            migrate_v1_to_v2(conn)?;
            migrate_v2_to_v3(conn)?;
            migrate_v3_to_v4(conn)?;
            migrate_v4_to_v5(conn)?;
            migrate_v5_to_v6(conn)?;
            migrate_v6_to_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        2 => {
            verify_existing_v1(conn)?;
            verify_existing_v2(conn)?;
            migrate_v2_to_v3(conn)?;
            migrate_v3_to_v4(conn)?;
            migrate_v4_to_v5(conn)?;
            migrate_v5_to_v6(conn)?;
            migrate_v6_to_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        3 => {
            verify_existing_v1(conn)?;
            verify_existing_v2(conn)?;
            verify_existing_v3(conn)?;
            migrate_v3_to_v4(conn)?;
            migrate_v4_to_v5(conn)?;
            migrate_v5_to_v6(conn)?;
            migrate_v6_to_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        4 => {
            verify_existing_v1(conn)?;
            verify_execution_index(conn)?;
            verify_existing_v2(conn)?;
            verify_existing_v3(conn)?;
            verify_existing_v4(conn)?;
            migrate_v4_to_v5(conn)?;
            migrate_v5_to_v6(conn)?;
            migrate_v6_to_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        5 => {
            verify_existing_v1(conn)?;
            verify_execution_index(conn)?;
            verify_existing_v2(conn)?;
            verify_existing_v3(conn)?;
            verify_existing_v4(conn)?;
            verify_existing_v5(conn)?;
            migrate_v5_to_v6(conn)?;
            migrate_v6_to_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        6 => {
            verify_existing_v1(conn)?;
            verify_execution_index(conn)?;
            verify_existing_v2(conn)?;
            verify_existing_v3(conn)?;
            verify_existing_v4(conn)?;
            verify_existing_v5(conn)?;
            verify_existing_v6(conn)?;
            migrate_v6_to_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        7 => {
            verify_existing_v1(conn)?;
            verify_execution_index(conn)?;
            verify_existing_v2(conn)?;
            verify_existing_v3(conn)?;
            verify_existing_v4(conn)?;
            verify_existing_v5(conn)?;
            verify_existing_v6(conn)?;
            verify_existing_v7(conn)?;
            migrate_v7_to_v8(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        8 => {
            verify_existing_v8_shape(conn)?;
            migrate_v8_to_v9(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        9 => {
            verify_existing_v9_shape(conn)?;
            migrate_v9_to_v10(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        10 => {
            verify_existing_v10_shape(conn)?;
            migrate_v10_to_v11(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        11 => {
            verify_existing_v11_shape(conn)?;
            migrate_v11_to_v12(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        12 => {
            verify_existing_v12_shape(conn)?;
            migrate_v12_to_v13(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        13 => {
            verify_existing_v13_shape(conn)?;
            migrate_v13_to_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        14 => {
            verify_existing_v13_shape(conn)?;
            verify_existing_v14(conn)?;
            migrate_v14_to_v15(conn, now())?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        15 => {
            verify_existing_v15(conn)?;
            migrate_v15_to_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        16 => {
            verify_existing_v13_shape(conn)?;
            verify_existing_v14(conn)?;
            verify_existing_v15(conn)?;
            verify_existing_v16(conn)?;
            migrate_v16_to_v17(conn)?;
            verify_existing(conn)
        }
        17 => verify_existing(conn),
        18..=25 => verify_existing(conn),
        _ => Err(api_error(
            ErrorCode::IncompatibleSchema,
            format!("unsupported schema version {version}"),
        )),
    }?;
    if (1..=17).contains(&version) {
        migrate_v17_to_v18(conn)?;
    }
    verify_existing_v18(conn)?;
    if (1..=18).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V19)
            .and_then(|_| conn.pragma_update(None, "user_version", 19));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v19(conn)?;
    if (1..=19).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V20)
            .and_then(|_| conn.pragma_update(None, "user_version", 20));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v20(conn)?;
    if (1..=20).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V21)
            .and_then(|_| conn.pragma_update(None, "user_version", 21));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v21(conn)?;
    if (1..=21).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V22)
            .and_then(|_| conn.pragma_update(None, "user_version", 22));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v22(conn)?;
    if (1..=22).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V23)
            .and_then(|_| conn.pragma_update(None, "user_version", 23));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v23(conn)?;
    if (1..=23).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V24)
            .map_err(store_error)
            .and_then(|_| verify_existing_v24(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 24)
                    .map_err(store_error)
            });
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(error);
            }
        }
    }
    if (1..=24).contains(&version) {
        verify_existing_v24(conn)?;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V25)
            .map_err(store_error)
            .and_then(|_| verify_existing_v23(conn))
            .and_then(|_| verify_existing_v25(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 25)
                    .map_err(store_error)
            });
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(error);
            }
        }
    }
    verify_existing_v23(conn)?;
    verify_existing_v25(conn)
}

/// Audit every additive archival object, including immutable/absorbing guards.
/// V23 is DDL only; each statement starts with CREATE on its own line. Splitting
/// on those boundaries retains trigger bodies (which contain semicolons).
fn verify_existing_v23(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V23
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    // Preserve quoted literals and predicate bytes: case/whitespace folding
    // inside a SQL string can silently turn a guard into a different condition.
    let normalize = |sql: &str| sql.trim().trim_end_matches(';').trim().to_owned();
    for object in ddl.trim().split("\nCREATE ") {
        let sql = if object.starts_with("CREATE ") {
            object.to_owned()
        } else {
            format!("CREATE {object}")
        };
        let mut words = sql.split_whitespace().skip(1);
        let kind = words.next().unwrap_or_default().to_ascii_lowercase();
        let name = words.next().unwrap_or_default();
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(normalize) != Some(normalize(&sql)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered archival {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn verify_existing_v22(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.split_whitespace()
            .collect::<String>()
            .trim_end_matches(';')
            .to_ascii_lowercase()
    };
    let trigger = "messages_user_intent_insert";
    let installed: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
            [trigger],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let marker = format!("CREATE TRIGGER {trigger}");
    let expected = V22.split_once(&marker).and_then(|(_, tail)| {
        tail.rfind("END;")
            .map(|end| format!("{marker}{}", &tail[..end + 4]))
    });
    if installed.as_deref().map(normalize) != expected.as_deref().map(normalize) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered user intent insertion guard",
        ));
    }
    for (table, column) in [
        ("messages", "user_intent"),
        ("summary_transitions", "rule_change"),
        ("summary_jobs", "fetched_bundle_json"),
    ] {
        let shape: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2 AND upper(type)='TEXT' AND [notnull]=0 AND dflt_value IS NULL AND pk=0)",
            params![table,column],|r|r.get(0)
        ).map_err(store_error)?;
        let installed: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        let marker = format!("ALTER TABLE {table} ADD COLUMN ");
        let expected = V22.split_once(&marker).and_then(|(_, tail)| {
            tail.split_once(';')
                .map(|(declaration, _)| normalize(declaration))
        });
        let sql = normalize(&installed);
        // SQLite inserts appended columns before existing table constraints.
        // Split only at top-level commas to retain each complete CHECK domain.
        let body = sql
            .split_once('(')
            .and_then(|(_, body)| body.strip_suffix(")strict"));
        let declaration = body.and_then(|body| {
            let mut depth = 0u32;
            let mut quoted = false;
            let mut start = 0;
            let mut found = None;
            for (offset, ch) in body
                .char_indices()
                .chain(std::iter::once((body.len(), ',')))
            {
                if ch == '\'' {
                    quoted = !quoted;
                }
                if quoted {
                    continue;
                }
                match ch {
                    '(' => depth += 1,
                    ')' => depth = depth.saturating_sub(1),
                    ',' if depth == 0 => {
                        let candidate = &body[start..offset];
                        if candidate.starts_with(&format!("{column}text")) {
                            found = Some(candidate.to_owned());
                        }
                        start = offset + 1;
                    }
                    _ => {}
                }
            }
            found
        });
        if !shape || declaration != expected {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered user intent column {table}.{column}"),
            ));
        }
    }
    Ok(())
}

fn verify_existing_v25(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.split_whitespace()
            .collect::<String>()
            .trim_end_matches(';')
            .replace('"', "")
            .to_ascii_lowercase()
    };
    for statement in V25.split(';') {
        let clean = statement
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        let statement = clean.trim();
        let kind = if statement.starts_with("CREATE TABLE ") {
            "table"
        } else if statement.starts_with("CREATE INDEX ")
            || statement.starts_with("CREATE UNIQUE INDEX ")
        {
            "index"
        } else {
            continue;
        };
        let rest = statement
            .strip_prefix("CREATE TABLE ")
            .or_else(|| statement.strip_prefix("CREATE INDEX "))
            .or_else(|| statement.strip_prefix("CREATE UNIQUE INDEX "))
            .expect("create statement");
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name == "harness_binding_sequence_v22" {
            continue;
        }
        let installed_name = name
            .strip_suffix("_v22")
            .or_else(|| name.strip_suffix("_v25"))
            .unwrap_or(&name);
        let expected = statement.replacen(&name, installed_name, 1);
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, installed_name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(normalize) != Some(normalize(&expected)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible v25 {kind} {installed_name}"),
            ));
        }
    }
    Ok(())
}

fn verify_existing_v21(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.split_whitespace()
            .collect::<String>()
            .trim_end_matches(';')
            .to_ascii_lowercase()
    };
    for (kind, name) in [
        ("table", "invitation_rejections"),
        ("index", "invitations_effective_pending_unwarned"),
        ("trigger", "invitation_rejections_shape"),
        ("trigger", "invitation_rejections_immutable"),
        ("trigger", "invitation_rejections_retained"),
        ("trigger", "digest_invitation_rejected"),
        ("trigger", "invitations_rejection_projection_insert"),
        ("trigger", "invitations_rejection_projection_guard"),
        ("trigger", "invitation_rejections_project"),
    ] {
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let marker = format!("CREATE {} {name}", kind.to_uppercase());
        let end = match kind {
            "table" => ") STRICT;",
            "trigger" => "END;",
            _ => ";",
        };
        let expected = V21.split_once(&marker).and_then(|(_, tail)| {
            tail.find(end)
                .map(|position| format!("{marker}{}", &tail[..position + end.len()]))
        });
        if actual.as_deref().map(normalize) != expected.as_deref().map(normalize) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered invitation rejection {kind} {name}"),
            ));
        }
    }
    let projection:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('invitations') WHERE name='reject_recorded' AND type='INTEGER' AND [notnull]=1 AND dflt_value='0')",[],|r|r.get(0)).map_err(store_error)?;
    if !projection {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered invitation rejection projection",
        ));
    }
    Ok(())
}

fn verify_existing_v20(conn: &Connection) -> Result<(), ApiError> {
    let column: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('threads') WHERE name='last_activity' AND type='INTEGER' AND [notnull]=1 AND dflt_value='0')", [], |r|r.get(0)).map_err(store_error)?;
    let actual: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='threads_recent_activity'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let normalize = |s: &str| {
        s.split_whitespace()
            .collect::<String>()
            .to_ascii_lowercase()
    };
    if !column
        || actual.as_deref().map(normalize)
            != Some(normalize(
                "CREATE INDEX threads_recent_activity ON threads(instance_id, last_activity DESC, ordinal DESC)",
            ))
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered recent activity schema",
        ));
    }
    for name in ["threads_recent_insert", "threads_recent_update"] {
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let expected = V20
            .split("CREATE TRIGGER ")
            .find(|part| part.starts_with(name))
            .map(|part| format!("CREATE TRIGGER {}", part.split("END;").next().unwrap()) + "END");
        if actual.as_deref().map(normalize) != expected.as_deref().map(normalize) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "missing or altered recent activity trigger",
            ));
        }
    }
    Ok(())
}

fn verify_existing_v19(conn: &Connection) -> Result<(), ApiError> {
    let normalize_ddl = |sql: &str| {
        sql.split_whitespace()
            .collect::<String>()
            .to_ascii_lowercase()
    };
    let column: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('threads') WHERE name='name' AND type='TEXT' AND [notnull]=0)", [], |r| r.get(0)).map_err(store_error)?;
    let index: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='threads_instance_name'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if !column
        || index.as_deref().map(normalize_ddl)
            != Some(normalize_ddl(
                "CREATE INDEX threads_instance_name ON threads(instance_id, name)",
            ))
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered thread name schema",
        ));
    }
    Ok(())
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

/// The current (v14) shape audit: the v11 shape (v10 B5 trust guards plus the
/// statements `0011_cooperative_only.sql` adds; later additions to that file
/// extend that audit), the v12 harness version evidence tables, the v13
/// thread summaries and the v14 catch-up release key.
pub fn verify_existing(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v13_shape(conn)?;
    verify_existing_v14(conn)?;
    verify_existing_v15(conn)?;
    verify_existing_v16(conn)?;
    verify_existing_v17(conn)
}

fn migrate_v17_to_v18(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V18)
        .and_then(|_| conn.pragma_update(None, "user_version", 18));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v18(conn: &Connection) -> Result<(), ApiError> {
    for (kind, name) in [
        ("table", "warning_conditions"),
        ("index", "warning_conditions_active"),
        ("index", "warning_conditions_affected"),
        ("index", "warning_conditions_close_scope"),
        ("index", "warning_conditions_thread"),
        ("index", "warning_conditions_unavailable_reopen"),
        ("table", "warning_close_sweeps"),
        ("index", "warning_close_sweeps_cover"),
    ] {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                params![kind, name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "missing warning condition schema",
            ));
        }
    }
    Ok(())
}

/// Audit of everything v1..v13 define; a v13 store is checked with this
/// before the catch-up release migration runs.
fn verify_existing_v13_shape(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v12_shape(conn)?;
    verify_existing_v13(conn)
}

/// Audit of everything v1..v12 define; a v12 store is checked with this
/// before the summary migration runs.
fn verify_existing_v12_shape(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v11_shape(conn)?;
    verify_v12_harness_evidence(conn)
}

/// The v11 shape audit: the v10 (B5 trust guards) shape plus the statements
/// `0011_cooperative_only.sql` adds; later additions to that file extend this
/// audit. A v11 store is checked with this before the harness evidence
/// migration runs.
fn verify_existing_v11_shape(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v10_shape(conn)?;
    verify_v11_b1(conn)
}

/// v12 (ht-xoc.4): both evidence tables and the `last_seen` index must exist as
/// the migration wrote them (same whitespace/case normalization as v11).
fn verify_v12_harness_evidence(conn: &Connection) -> Result<(), ApiError> {
    if conn
        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
        .map_err(store_error)?
        >= 25
    {
        return verify_existing_v25(conn);
    }
    let normalize = |sql: &str| {
        sql.trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    for statement in V12.split(';') {
        let statement: String = statement
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        let statement = statement.trim();
        let Some(rest) = statement
            .strip_prefix("CREATE TABLE ")
            .map(|r| ("table", r))
            .or_else(|| {
                statement
                    .strip_prefix("CREATE INDEX ")
                    .map(|r| ("index", r))
            })
        else {
            continue;
        };
        let (kind, rest) = rest;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                [kind, name.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(normalize) != Some(normalize(statement)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible v12 {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn migrate_v11_to_v12(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V12)
        .and_then(|_| conn.pragma_update(None, "user_version", 12));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

/// Access paths ht-p03.12.5 probes with `INDEXED BY`; all pre-date v11 and are
/// audited by their own version's verifier, so v11 only re-asserts presence.
const B1_EXISTING_ACCESS_PATHS: [&str; 7] = [
    "digest_pending_invitations_seat",
    "digest_pending_manifest_receipts_seat",
    "receipts_seat_state_ordinal",
    "digest_open_warning_recipients_seat",
    "digest_open_warnings_affected",
    "digest_programmatic_warnings_seat",
    "occupant_bindings_current",
];

/// ht-p03.12.1 (B1): every statement below the marker in the v11 migration
/// must be installed exactly as written (same normalization as v7).
fn verify_v11_b1(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    let b1 = V11
        .split_once("-- B1 (ht-p03.12.1) partial indexes")
        .map(|(_, rest)| rest)
        .ok_or_else(|| api_error(ErrorCode::IncompatibleSchema, "v11 B1 marker missing"))?;
    for statement in b1.lines().filter(|line| !line.trim().is_empty()) {
        if statement.starts_with("ALTER TABLE work_jobs ADD COLUMN completed_at") {
            let table: Option<String> = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name='work_jobs'",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(store_error)?;
            let table = table.map(|sql| normalize(&sql)).unwrap_or_default();
            if !table
                .contains("completed_at integer check(completed_at is null or completed_at >= 0)")
            {
                return Err(api_error(
                    ErrorCode::IncompatibleSchema,
                    "work_jobs.completed_at is missing or altered",
                ));
            }
            continue;
        }
        let name = statement
            .strip_prefix("CREATE INDEX ")
            .and_then(|rest| rest.split_whitespace().next())
            .ok_or_else(|| api_error(ErrorCode::IncompatibleSchema, "invalid v11 index DDL"))?;
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(normalize) != Some(normalize(statement)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible v11 index {name}"),
            ));
        }
    }
    for name in B1_EXISTING_ACCESS_PATHS {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                [name],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing access path index {name}"),
            ));
        }
    }
    Ok(())
}

/// A v9 store is audited as v9 before allocation decisions are rebuilt.
fn verify_existing_v9_shape(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v8_shape(conn)?;
    verify_existing_v9(conn)
}

/// A v10 (B5 trust guards) store is audited as v10 before the v11
/// cooperative-only additions.
fn verify_existing_v10_shape(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v9_shape(conn)?;
    verify_existing_v10(conn)
}

fn migrate_v9_to_v10(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V10)
        .and_then(|_| conn.pragma_update(None, "user_version", 10));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn migrate_v12_to_v13(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V13)
        .and_then(|_| conn.pragma_update(None, "user_version", 13));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn migrate_v14_to_v15(conn: &Connection, now: UtcMillis) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = (|| {
        conn.execute_batch(V15)?;
        conn.execute("UPDATE send_preparations SET prepared_at=?1 WHERE status IN ('building','sealed') AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=send_preparations.id)", [now.0])?;
        conn.pragma_update(None, "user_version", 15)
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn migrate_v16_to_v17(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V17)
        .and_then(|_| conn.pragma_update(None, "user_version", 17));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v17(conn: &Connection) -> Result<(), ApiError> {
    // Compare every table and clear trigger with the migration, so an altered
    // trigger cannot silently retain or reset the wrong seat's window.
    for statement in V17.split("CREATE").skip(1) {
        let expected = format!("CREATE{}", statement.trim_end().trim_end_matches(';'));
        let mut words = expected.split_whitespace();
        let _create = words.next();
        let kind = words
            .next()
            .expect("wake batch object kind")
            .to_ascii_lowercase();
        let name = words.next().expect("wake batch object name");
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                [&kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let normalize = |sql: &str| {
            sql.split_whitespace()
                .collect::<String>()
                .to_ascii_lowercase()
        };
        if installed.is_none_or(|sql| normalize(&sql) != normalize(&expected)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "incompatible wake batching schema",
            ));
        }
    }
    Ok(())
}

fn migrate_v15_to_v16(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V16)
        .and_then(|_| conn.pragma_update(None, "user_version", 16));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v16(conn: &Connection) -> Result<(), ApiError> {
    let present: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='human_receipt_waivers')", [], |r| r.get(0))
        .map_err(store_error)?;
    let columns: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info('human_receipt_waivers') WHERE (name='seat_id' AND type='TEXT' AND pk=1) OR (name='through_decision_seq' AND type='INTEGER' AND \"notnull\"=1) OR (name='human_generation' AND type='INTEGER' AND \"notnull\"=1) OR (name='decided_at' AND type='INTEGER' AND \"notnull\"=1)",
        [], |r| r.get(0),
    ).map_err(store_error)?;
    let obligation_columns: i64 = conn.query_row(
        "SELECT (SELECT count(*) FROM pragma_table_info('prepared_recipients') WHERE name='ack_required' AND type='INTEGER' AND \"notnull\"=1) + (SELECT count(*) FROM pragma_table_info('receipt_state') WHERE name='ack_required' AND type='INTEGER' AND \"notnull\"=1) + (SELECT count(*) FROM pragma_table_info('receipts') WHERE name='ack_required' AND type='INTEGER' AND \"notnull\"=1)",
        [], |r| r.get(0),
    ).map_err(store_error)?;
    let bounds: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='human_receipt_reconciliation_bounds')", [], |r| r.get(0)).map_err(store_error)?;
    let bound_columns: i64 = conn.query_row("SELECT count(*) FROM pragma_table_info('human_receipt_reconciliation_bounds') WHERE (name='seat_id' AND type='TEXT' AND pk=1) OR (name='prepared_high_water' AND type='INTEGER' AND \"notnull\"=1) OR (name='physical_high_water' AND type='INTEGER' AND \"notnull\"=1) OR (name='decision_seq' AND type='INTEGER' AND \"notnull\"=1)", [], |r| r.get(0)).map_err(store_error)?;
    let work_kind: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='work_jobs' AND sql LIKE '%human_receipt_reconciliation%')", [], |r| r.get(0)).map_err(store_error)?;
    if !present
        || !bounds
        || !work_kind
        || columns != 4
        || bound_columns != 4
        || obligation_columns != 3
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing human receipt obligation state",
        ));
    }
    for (kind, name) in [
        ("trigger", "human_receipt_prepared_waived"),
        ("trigger", "human_receipt_state_waived"),
        ("trigger", "human_receipt_state_insert_waived"),
        ("trigger", "human_receipt_physical_waived"),
        ("index", "receipts_required_seat_pending"),
        ("index", "prepared_recipients_required_seat"),
        ("index", "receipts_required_thread_pending"),
        ("index", "receipts_required_due"),
        ("index", "receipt_state_required_due"),
        ("index", "receipts_required_poke"),
        ("index", "receipt_state_required_poke"),
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                [kind, name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !exists {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "missing human receipt pending path",
            ));
        }
    }
    Ok(())
}

fn verify_existing_v15(conn: &Connection) -> Result<(), ApiError> {
    let column: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('send_preparations') WHERE name='prepared_at' AND type='INTEGER')", [], |r| r.get(0)).map_err(store_error)?;
    for (kind, name, expected) in [
        (
            "index",
            "send_preparations_retention",
            "CREATE INDEX send_preparations_retention ON send_preparations(prepared_at, id) WHERE prepared_at IS NOT NULL AND status IN ('building','sealed')",
        ),
        (
            "trigger",
            "send_manifest_preparation_published",
            "CREATE TRIGGER send_manifest_preparation_published AFTER INSERT ON send_manifests BEGIN UPDATE send_preparations SET prepared_at=NULL WHERE id=NEW.preparation_id; END",
        ),
    ] {
        let sql: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                [kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let normalize = |s: &str| {
            s.split_whitespace()
                .collect::<String>()
                .to_ascii_lowercase()
        };
        if !column || sql.is_none_or(|s| normalize(&s) != normalize(expected)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "incompatible preparation retention schema",
            ));
        }
    }
    Ok(())
}

fn migrate_v13_to_v14(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V14)
        .and_then(|_| conn.pragma_update(None, "user_version", 14));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

/// v14: catch-up release key. The column a row end stores for the push.
fn verify_existing_v14(conn: &Connection) -> Result<(), ApiError> {
    let present: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('catch_up') WHERE name='release_seq')",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !present {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "catch-up schema lacks catch_up.release_seq",
        ));
    }
    Ok(())
}

/// v13: thread summaries. Presence audit of the tables, indexes, triggers and
/// columns the summary, catch-up and poke tasks rely on.
fn verify_existing_v13(conn: &Connection) -> Result<(), ApiError> {
    let objects = [
        ("table", "summary_blocks"),
        ("table", "summary_items"),
        ("table", "summary_transitions"),
        ("table", "summary_jobs"),
        ("table", "summary_job_durations"),
        ("table", "catch_up"),
        ("index", "summary_items_fold"),
        ("index", "summary_transitions_fold"),
        ("index", "summary_jobs_live_lease"),
        ("index", "summary_job_durations_recent"),
        ("index", "catch_up_extension_until"),
        ("index", "catch_up_thread_active"),
        ("trigger", "messages_immutable"),
        ("trigger", "messages_summary_author_insert"),
        ("trigger", "summary_blocks_immutable"),
        ("trigger", "summary_blocks_retained"),
    ];
    for (kind, name) in objects {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                [kind, name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("thread summary schema lacks {kind} {name}"),
            ));
        }
    }
    let columns = [
        ("messages", "author_role"),
        ("messages", "relays_user"),
        ("messages", "author_role_backfilled"),
        ("receipts", "soft_poked_at"),
        ("receipt_state", "soft_poked_at"),
    ];
    for (table, column) in columns {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
                [table, column],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("thread summary schema lacks {table}.{column}"),
            ));
        }
    }
    Ok(())
}

/// v10: allocation decisions admit the B5 kinds and the continuity diagnostic;
/// host instances carry the reconciliation marker.
fn verify_existing_v10(conn: &Connection) -> Result<(), ApiError> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='allocation_decisions'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let normalized = sql
        .unwrap_or_default()
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    if !normalized.contains("'cooperative_continuity'")
        || !normalized.contains("continuity_diagnostic")
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "allocation decisions do not accept the B5 kinds",
        ));
    }
    for column in ["reconciled_boot", "reconciled_epoch"] {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('host_instances') WHERE name=?1)",
                [column],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("host instances lack {column}"),
            ));
        }
    }
    Ok(())
}

/// A v8 store is audited as v8 before its occupant bindings are rebuilt.
fn verify_existing_v8_shape(conn: &Connection) -> Result<(), ApiError> {
    verify_existing_v1(conn)?;
    verify_execution_index(conn)?;
    verify_existing_v2(conn)?;
    verify_existing_v3(conn)?;
    verify_existing_v4(conn)?;
    verify_existing_v5(conn)?;
    verify_existing_v6(conn)?;
    verify_existing_v7(conn)?;
    verify_existing_v8(conn)
}

fn migrate_v10_to_v11(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V11)
        .and_then(|_| conn.pragma_update(None, "user_version", 11));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn migrate_v8_to_v9(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    // The v8 shape was already validated. Carry the allocation high-water
    // across v9's surviving-row rebuild inside that same transaction, rather
    // than reading malformed historical stores before their canonical audit.
    // The historical SQL itself remains byte-for-byte unchanged.
    let result = conn
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name='occupant_bindings'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .and_then(|sequence| {
            conn.execute_batch(V9)?;
            if let Some(sequence) = sequence {
                conn.execute(
                    "UPDATE sqlite_sequence SET seq=MAX(seq,?1) WHERE name='occupant_bindings'",
                    [sequence],
                )?;
            }
            conn.pragma_update(None, "user_version", 9)
        });
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

/// v9: occupant bindings accept the `human` harness of `me init`.
fn verify_existing_v9(conn: &Connection) -> Result<(), ApiError> {
    if conn
        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
        .map_err(store_error)?
        >= 25
    {
        return verify_existing_v25(conn);
    }
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='occupant_bindings'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let normalized = sql
        .unwrap_or_default()
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    if !normalized.contains("check(harnessin('codex','claude','human'))") {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "occupant bindings do not accept the human harness",
        ));
    }
    Ok(())
}

fn migrate_v7_to_v8(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V8)
        .and_then(|_| conn.pragma_update(None, "user_version", 8));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

/// Every v8 projection table, index and maintaining trigger must exist exactly
/// as the migration defines it. A missing or altered trigger would let a
/// projection drift from its source rows and silently hide attention, so
/// startup refuses the database instead.
fn verify_existing_v8(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    for statement in V8.split("\n\n").map(str::trim) {
        let Some(rest) = statement.strip_prefix("CREATE ") else {
            continue;
        };
        let (kind, rest) = rest
            .split_once(' ')
            .ok_or_else(|| api_error(ErrorCode::IncompatibleSchema, "invalid digest DDL"))?;
        let kind = kind.to_ascii_lowercase();
        let name = rest
            .split(|c: char| c.is_whitespace() || c == '(')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| api_error(ErrorCode::IncompatibleSchema, "invalid digest DDL"))?;
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(normalize) != Some(normalize(statement)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible attention digest projection {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn migrate_v6_to_v7(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V7)
        .and_then(|_| conn.pragma_update(None, "user_version", 7));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

/// Every digest index must exist exactly as the migration defines it: the
/// producer names them with `INDEXED BY`, so a missing or altered index would
/// otherwise fail at query time instead of at startup.
fn verify_existing_v7(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    for statement in V7.lines().filter(|line| !line.trim().is_empty()) {
        let name = statement
            .strip_prefix("CREATE INDEX ")
            .and_then(|rest| rest.split_whitespace().next())
            .ok_or_else(|| api_error(ErrorCode::IncompatibleSchema, "invalid digest index DDL"))?;
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(normalize) != Some(normalize(statement)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible attention digest index {name}"),
            ));
        }
    }
    Ok(())
}

fn migrate_v5_to_v6(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V6)
        .and_then(|_| conn.pragma_update(None, "user_version", 6));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v6(conn: &Connection) -> Result<(), ApiError> {
    let installed: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='index' AND name='retirements_failed_pending'",
        [], |row| row.get(0),
    ).optional().map_err(store_error)?;
    let normalize = |sql: &str| {
        sql.trim()
            .trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    if installed.as_deref().map(normalize) != Some(normalize(V6)) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "incompatible retirement health index",
        ));
    }
    Ok(())
}

fn migrate_v4_to_v5(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V5)
        .and_then(|_| conn.pragma_update(None, "user_version", 5));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v5(conn: &Connection) -> Result<(), ApiError> {
    // Compare the installed DDL with the migration. Names alone do not protect
    // the recipient page order or the durable keys, FKs and CHECK constraints.
    for (kind, name) in [
        ("table", "service_notification_preparations"),
        ("table", "service_notification_recipients"),
        ("table", "service_notification_publications"),
        ("index", "requirement_episodes_thread_ordinal"),
        ("index", "service_notification_preparations_key"),
        ("index", "service_notification_recipients_page"),
    ] {
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let Some(installed) = installed else {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing schema {kind} {name}"),
            ));
        };
        let prefix = format!("CREATE {} {name}", kind.to_ascii_uppercase());
        let expected = V5
            .split_once(&prefix)
            .expect("v5 object prefix")
            .1
            .split_once(';')
            .expect("v5 object terminator")
            .0;
        let normalized = |sql: &str| {
            sql.trim_end_matches(';')
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        if normalized(&installed) != normalized(&format!("{prefix}{expected}")) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible schema {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn migrate_v3_to_v4(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    // A previously-left row was overwritten by frozen C2. Recover its exact
    // timestamp only from the retained interval and corresponding leave audit.
    let ambiguous=conn.query_row("SELECT count(*) FROM memberships m WHERE m.state='invited' AND EXISTS (
        SELECT 1 FROM requirement_episodes r JOIN invitations i ON i.id=r.invitation_id
        WHERE r.thread_id=m.thread_id AND r.seat_id=m.seat_id AND r.created_decision_seq=i.created_decision_seq AND i.episode=m.episode)
        AND EXISTS (SELECT 1 FROM membership_intervals mi WHERE mi.thread_id=m.thread_id AND mi.seat_id=m.seat_id AND mi.left_seq IS NOT NULL)
        AND NOT EXISTS (SELECT 1 FROM membership_intervals mi JOIN messages msg ON msg.thread_id=mi.thread_id AND msg.decision_seq=mi.left_seq
            WHERE mi.thread_id=m.thread_id AND mi.seat_id=m.seat_id AND mi.left_seq IS NOT NULL
            AND json_extract(msg.event_json,'$.action')='leave' AND json_extract(msg.event_json,'$.seat')=m.seat_id)",[],|r|r.get::<_,i64>(0));
    let ambiguous = match ambiguous {
        Ok(count) => count,
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(store_error(error));
        }
    };
    if ambiguous != 0 {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "v3 required-only membership has unrecoverable prior left state",
        ));
    }
    let result = conn
        .execute_batch(V4)
        .and_then(|_| conn.pragma_update(None, "user_version", 4));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v4(conn: &Connection) -> Result<(), ApiError> {
    let installed: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='memberships'",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let normalized = installed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if !normalized.contains("voluntary_state text check(voluntary_state in ('absent','invited','joined','left','retired'))") {
        return Err(api_error(ErrorCode::IncompatibleSchema,"missing or weakened voluntary membership marker"));
    }
    Ok(())
}

fn migrate_v2_to_v3(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V3)
        .and_then(|_| conn.pragma_update(None, "user_version", 3));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v3(conn: &Connection) -> Result<(), ApiError> {
    for (kind, name) in [
        ("table", "invitation_cancellations"),
        ("trigger", "invitation_cancellations_shape"),
        ("trigger", "invitation_cancellations_immutable"),
        ("trigger", "invitation_cancellations_retained"),
    ] {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                params![kind, name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing schema {kind} {name}"),
            ));
        }
    }
    let normalize = |sql: &str| {
        sql.trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    let shapes = [
        (
            "table",
            "invitation_cancellations",
            "CREATE TABLE invitation_cancellations",
            ") STRICT;",
        ),
        (
            "trigger",
            "invitation_cancellations_shape",
            "CREATE TRIGGER invitation_cancellations_shape",
            "END;",
        ),
        (
            "trigger",
            "invitation_cancellations_immutable",
            "CREATE TRIGGER invitation_cancellations_immutable",
            "END;",
        ),
        (
            "trigger",
            "invitation_cancellations_retained",
            "CREATE TRIGGER invitation_cancellations_retained",
            "END;",
        ),
    ];
    for (kind, name, prefix, terminator) in shapes {
        let installed: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        let tail = V3.split_once(prefix).expect("v3 object prefix").1;
        let body = tail.split_once(terminator).expect("v3 object terminator").0;
        let expected = format!("{prefix}{body}{terminator}");
        if normalize(&installed) != normalize(&expected) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible schema {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn migrate_v1_to_v2(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch("CREATE INDEX IF NOT EXISTS occupant_bindings_execution ON occupant_bindings(seat_id,execution_id)")
        .and_then(|_| conn.execute_batch(V2))
        .and_then(|_| conn.pragma_update(None, "user_version", 2));
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(store_error(error))
        }
    }
}

fn verify_existing_v2(conn: &Connection) -> Result<(), ApiError> {
    for (kind, name) in [
        ("table", "service_authors"),
        ("table", "requirement_episodes"),
        ("index", "threads_managed_owner"),
        ("index", "requirement_episodes_effective"),
        ("index", "requirement_episodes_seat_pending"),
        ("index", "requirement_episodes_thread_seat"),
        ("index", "messages_author_service"),
        ("trigger", "threads_managed_owner_instance_insert"),
        ("trigger", "threads_managed_owner_instance_update"),
        ("trigger", "threads_managed_owner_immutable"),
        ("trigger", "service_authors_immutable"),
        ("trigger", "service_authors_retained"),
        ("trigger", "messages_author_shape_insert"),
        ("trigger", "requirement_episodes_owner_insert"),
        ("trigger", "requirement_episodes_identity_immutable"),
        ("trigger", "requirement_episodes_revision_forward"),
        ("trigger", "requirement_episodes_acceptance_provenance"),
        ("trigger", "requirement_episodes_state_forward"),
        ("trigger", "requirement_episodes_retained"),
    ] {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                params![kind, name],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing schema {kind} {name}"),
            ));
        }
    }
    for (table, column) in [
        ("threads", "managed_owner_author_id"),
        ("messages", "author_kind"),
        ("messages", "author_service_id"),
        ("requirement_episodes", "accepted_by_seat_id"),
        ("requirement_episodes", "revision"),
    ] {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
                params![table, column],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing schema column {table}.{column}"),
            ));
        }
    }
    // This partial UNIQUE index is the database-level guard against two live
    // obligations. A same-name ordinary or weaker partial index is unsafe.
    let effective_index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='index' AND name='requirement_episodes_effective'",
            [], |row| row.get(0),
        )
        .map_err(store_error)?;
    let normalized = effective_index_sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    if normalized
        != "create unique index requirement_episodes_effective on requirement_episodes(thread_id, seat_id) where state in ('pending', 'accepted')"
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "incompatible effective requirement index",
        ));
    }
    // Compare the complete installed trigger against this version's migration.
    // A same-name trigger with a weaker WHEN expression cannot protect history.
    let installed: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='requirement_episodes_acceptance_provenance'",
            [],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    let prefix = "CREATE TRIGGER requirement_episodes_acceptance_provenance";
    let migration_tail = V2.split_once(prefix).expect("v2 acceptance trigger").1;
    let trigger_body = migration_tail
        .split_once("\nEND;")
        .expect("v2 acceptance trigger terminator")
        .0;
    let expected = format!("{prefix}{trigger_body}\nEND");
    let normalize = |sql: &str| {
        sql.trim_end_matches(';')
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase()
    };
    if normalize(&installed) != normalize(&expected) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "incompatible requirement acceptance provenance trigger",
        ));
    }
    // The trigger protects updates; the table CHECK requires a complete tuple
    // for initial accepted rows and rejects partial tuples in terminal rows.
    let installed_table: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='requirement_episodes'",
            [],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    let table_prefix = "CREATE TABLE requirement_episodes";
    let table_tail = V2.split_once(table_prefix).expect("v2 requirement table").1;
    let table_body = table_tail
        .split_once(") STRICT;")
        .expect("v2 requirement table terminator")
        .0;
    let expected_table = format!("{table_prefix}{table_body}) STRICT");
    if normalize(&installed_table) != normalize(&expected_table) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "incompatible requirement episode table",
        ));
    }
    Ok(())
}

fn verify_execution_index(conn: &Connection) -> Result<(), ApiError> {
    let mut statement = conn
        .prepare("PRAGMA index_list('occupant_bindings')")
        .map_err(store_error)?;
    let shapes = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(4)?,
            ))
        })
        .map_err(store_error)?;
    let mut valid = false;
    for shape in shapes {
        let (name, unique, partial) = shape.map_err(store_error)?;
        if name == "occupant_bindings_execution" {
            valid = !unique && !partial;
        }
    }
    let mut statement = conn
        .prepare("PRAGMA index_xinfo('occupant_bindings_execution')")
        .map_err(store_error)?;
    let columns: Vec<(i64, i64, Option<String>, bool, String)> = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, bool>(5)?,
                (
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ),
            ))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?
        .into_iter()
        .filter_map(|(key, metadata)| key.then_some(metadata))
        .collect();
    let expected_ids: Vec<i64> = ["seat_id", "execution_id"]
        .into_iter()
        .map(|name| {
            conn.query_row(
                "SELECT cid FROM pragma_table_info('occupant_bindings') WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .map_err(store_error)
        })
        .collect::<Result<_, _>>()?;
    let expected = vec![
        (
            0,
            expected_ids[0],
            Some("seat_id".to_owned()),
            false,
            "BINARY".to_owned(),
        ),
        (
            1,
            expected_ids[1],
            Some("execution_id".to_owned()),
            false,
            "BINARY".to_owned(),
        ),
    ];
    if !valid || columns != expected {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or incompatible occupant execution index",
        ));
    }
    Ok(())
}

fn verify_existing_v1(conn: &Connection) -> Result<(), ApiError> {
    check_integrity(conn)?;
    verify_query_connection_version(conn, 1, true)?;
    let required_tables = [
        "schema_identity",
        "host_instances",
        "snapshot_generations",
        "snapshot_targets",
        "recovery_baseline_releases",
        "observed_targets",
        "recovery_baseline_targets",
        "recovery_holds",
        "allocation_decisions",
        "seats",
        "occupant_bindings",
        "threads",
        "memberships",
        "invitations",
        "messages",
        "receipts",
        "operations",
        "wake_work",
        "retirements",
        "retirement_audits",
        "filter_revisions",
        "delivery_observations",
        "membership_intervals",
        "seat_availability",
        "send_preparations",
        "prepared_recipients",
        "prepared_unavailable_warnings",
        "send_manifests",
        "receipt_state",
        "warning_jobs",
        "warning_recipients",
        "warning_offer",
        "work_jobs",
    ];
    let required_indexes = [
        "allocation_decisions_target",
        "snapshot_generations_status",
        "snapshot_generations_captured_active",
        "snapshot_targets_terminal",
        "snapshot_targets_generation_ordinal",
        "seats_live_target",
        "seats_instance_ordinal",
        "seats_instance_state_ordinal",
        "seats_unresolved_generation",
        "occupant_bindings_current",
        "occupant_bindings_history",
        "allocation_decisions_seat_history",
        "threads_instance_ordinal",
        "threads_instance_topic_ordinal",
        "memberships_seat_ordinal",
        "memberships_thread_ordinal",
        "memberships_thread_state",
        "invitations_due",
        "invitations_seat_thread_pending",
        "invitations_thread_seat_episode",
        "invitations_seat_decision",
        "messages_thread_sequence",
        "messages_kind_ordinal",
        "messages_instance_logical",
        "messages_instance_ordinary_logical",
        "messages_instance_warning_logical",
        "receipts_due",
        "receipts_seat_state_ordinal",
        "receipts_seat_thread_state_ordinal",
        "receipts_thread_seat_pending",
        "retirements_status_ordinal",
        "retirements_progress",
        "membership_intervals_thread_ordinal",
        "membership_intervals_snapshot",
        "membership_intervals_open",
        "seat_availability_after",
        "invitations_pending_unwarned",
        "receipts_pending_unwarned",
        "receipts_message_ordinal",
        "receipts_thread_pending_ordinal",
        "send_preparations_status",
        "prepared_recipients_seat_ordinal",
        "prepared_recipients_seat_thread_ordinal",
        "prepared_recipients_preparation_ordinal",
        "prepared_recipients_thread_ordinal",
        "prepared_unavailable_warnings_key",
        "prepared_unavailable_warnings_ordinal",
        "send_manifests_thread_sequence",
        "send_manifests_decision",
        "send_manifests_instance_decision",
        "send_manifests_warning_decision",
        "receipt_state_pending_due",
        "warning_jobs_ready",
        "warning_recipients_seat_generation",
        "warning_recipients_warning_ordinal",
        "work_jobs_ready",
        "wake_work_active_attempt",
    ];
    let required_triggers = [
        "host_active_snapshot_published",
        "host_baseline_snapshot_published",
        "seats_ordinal_immutable",
        "threads_ordinal_immutable",
        "memberships_ordinal_immutable",
        "invitations_ordinal_immutable",
        "invitations_decision_immutable",
        "messages_immutable",
        "send_manifests_immutable",
        "messages_instance_thread",
        "manifests_instance_source",
        "messages_retained",
        "receipts_ordinal_immutable",
        "prepared_recipients_ordinal_immutable",
        "seats_retirement_terminal",
        "retirements_provenance_immutable",
    ];
    for (kind, name) in required_tables
        .iter()
        .map(|n| ("table", *n))
        .chain(required_indexes.iter().map(|n| ("index", *n)))
        .chain(required_triggers.iter().map(|n| ("trigger", *n)))
    {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                params![kind, name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !exists {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing schema {kind} {name}"),
            ));
        }
    }
    for (table, column) in [
        ("host_instances", "decision_seq"),
        ("host_instances", "active_snapshot_id"),
        ("host_instances", "recovery_baseline_generation_id"),
        ("host_instances", "baseline_hold_unclaimed"),
        ("host_instances", "invalidation_revision"),
        ("host_instances", "observation_admission_sequence"),
        ("host_instances", "observation_decided_sequence"),
        ("snapshot_generations", "captured_lifecycle_revision"),
        ("snapshot_generations", "admission_sequence"),
        ("snapshot_generations", "published_invalidation_revision"),
        ("snapshot_targets", "observation_sequence"),
        ("snapshot_targets", "connection_epoch"),
        ("snapshot_targets", "incarnation_source_kind"),
        ("warning_recipients", "ordinal"),
        ("prepared_unavailable_warnings", "ordinal"),
        ("observed_targets", "observation_sequence"),
        ("observed_targets", "connection_epoch"),
        ("observed_targets", "incarnation"),
        ("observed_targets", "incarnation_source_kind"),
        ("observed_targets", "terminal_id"),
        ("observed_targets", "occupancy"),
        ("observed_targets", "verified_execution"),
        ("host_instances", "send_eligibility_revision"),
        ("threads", "goal"),
        ("threads", "membership_revision"),
        ("threads", "timeline_revision"),
        ("seats", "retired_seq"),
        ("seats", "unresolved_reason"),
        ("seats", "unresolved_from_generation_id"),
        ("seats", "unresolved_prior_binding_generation"),
        ("seats", "unavailability_episode"),
        ("seats", "target_generation"),
        ("seats", "structural_terminal_id"),
        ("seats", "structural_incarnation"),
        ("seats", "structural_incarnation_kind"),
        ("seats", "structural_host_boot"),
        ("seats", "structural_host_epoch"),
        ("seats", "structural_connection_epoch"),
        ("seats", "structural_observation_sequence"),
        ("occupant_bindings", "target_generation"),
        ("seats", "unavailability_open"),
        ("invitations", "warning_message_id"),
        ("invitations", "created_decision_seq"),
        ("invitations", "accepted_actor_seat_id"),
        ("invitations", "accepted_generation"),
        ("invitations", "accepted_observation"),
        ("receipts", "warning_message_id"),
        ("messages", "decision_seq"),
        ("messages", "instance_id"),
        ("wake_work", "last_invitation_seq"),
        ("wake_work", "last_invitation_offset"),
        ("wake_work", "last_receipt_seq"),
        ("wake_work", "last_receipt_offset"),
        ("wake_work", "last_warning_seq"),
        ("wake_work", "last_warning_offset"),
        ("send_manifests", "instance_id"),
        ("prepared_recipients", "ordinal"),
        ("prepared_recipients", "thread_id"),
    ] {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(store_error)?;
        let present = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(store_error)?
            .any(|name| name.is_ok_and(|name| name == column));
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing schema column {table}.{column}"),
            ));
        }
    }
    Ok(())
}

/// Read workers must not scan the whole database during connection setup.
/// The writer performs the full integrity and schema audit at startup.
pub fn verify_query_connection(conn: &Connection) -> Result<(), ApiError> {
    verify_query_connection_version(conn, LATEST_VERSION, false)
}

fn verify_query_connection_version(
    conn: &Connection,
    expected_version: i64,
    allow_v2: bool,
) -> Result<(), ApiError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(store_error)?;
    if version != expected_version && !(allow_v2 && (2..=LATEST_VERSION).contains(&version)) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            format!("unsupported schema version {version}"),
        ));
    }
    let marker: String = conn
        .query_row("SELECT marker FROM schema_identity LIMIT 1", [], |r| {
            r.get(0)
        })
        .map_err(|_| {
            api_error(
                ErrorCode::IncompatibleSchema,
                "missing amended v1 schema marker",
            )
        })?;
    if marker != "herdr-threads-shared-v1-r7" {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "unknown amended v1 schema marker",
        ));
    }
    Ok(())
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
            "UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id=?1 AND decision_seq<?2",
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
            "SELECT decision_seq FROM host_instances WHERE id=?1",
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
        let rows = tx.execute(
            "UPDATE host_instances SET send_eligibility_revision=send_eligibility_revision+1 WHERE id=?1 AND send_eligibility_revision<?2",
            params![instance, i64::MAX],
        ).map_err(store_error)?;
        if rows != 1 {
            return Err(api_error(
                ErrorCode::SequenceExhausted,
                "send eligibility revision exhausted",
            ));
        }
    }
    Ok(changed)
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
    let changed = tx.execute("UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id=?1 AND lifecycle_revision<?2", params![instance, i64::MAX]).map_err(store_error)?;
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
/// the old one. Returns the binding's observation provenance when available.
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
             FROM seats s JOIN host_instances h ON h.id=s.instance_id JOIN occupant_bindings b ON b.seat_id=s.id \
             WHERE s.id=?1 AND s.state='resolved' AND s.target_id IS NOT NULL \
               AND b.ended_at IS NULL AND b.registered_at IS NOT NULL AND b.target_id=s.target_id \
               AND b.generation=s.generation AND b.target_generation=s.target_generation \
               AND b.host_boot=h.host_boot AND b.host_epoch=h.host_epoch \
               AND (?2 IS NULL OR h.id=?2) LIMIT 1",
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
/// open registered binding will be structurally carried: resolved seat, a
/// provenance in `CARRIED_BINDING_PROVENANCES`, the current host boot, an older
/// host epoch, and an incarnation equal to the seat's structural incarnation.
/// A send optimistically assumes that carry: the recipient is staged as not yet
/// available but is not warned about; the carry's availability anchor starts
/// the receipt timer. If the pass does not carry it, the ordinary unavailable
/// paths open the episode.
pub fn carry_pending(db: &Connection, seat: &str, instance: &str) -> Result<bool, ApiError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM seats s JOIN host_instances h ON h.id=s.instance_id \
         JOIN occupant_bindings b ON b.seat_id=s.id \
         WHERE s.id=?1 AND h.id=?2 AND s.state='resolved' \
           AND (h.reconciled_boot IS NOT h.recovery_boot OR h.reconciled_epoch IS NOT h.recovery_epoch) \
           AND b.ended_at IS NULL AND b.registered_at IS NOT NULL \
           AND b.observation_provenance IN (?3,?4) \
           AND b.host_boot=h.host_boot AND b.host_epoch<h.host_epoch \
           AND b.incarnation IS NOT NULL AND b.incarnation=s.structural_incarnation)",
        params![
            seat,
            instance,
            CARRIED_BINDING_PROVENANCES[0],
            CARRIED_BINDING_PROVENANCES[1]
        ],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Lazily open one durable seat outage. A global host epoch change invalidates
/// verified availability without visiting every seat; the first affected
/// deciding writer reconciles this marker. Repeated sightings of the same open
/// outage reuse the numeric episode, so warning keys stay stable.
pub fn ensure_unavailability_episode(tx: &Transaction<'_>, seat: &SeatId) -> Result<u64, ApiError> {
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
                    "SELECT decision_seq FROM host_instances WHERE id=?1",
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

/// Spec §1: the author role a seat's open binding gives its messages at decision
/// time. A `human` harness reads as `human`, any other harness as `agent`; no
/// open binding records NULL (read as agent).
pub fn open_binding_role(tx: &Connection, seat: &SeatId) -> Result<Option<&'static str>, ApiError> {
    let harness: Option<String> = tx
        .query_row(
            "SELECT harness FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(harness.map(|h| if h == "human" { "human" } else { "agent" }))
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
    let (kind,thread,affected,open): (String,String,String,String) = tx.query_row(
        "SELECT condition_kind,thread_id,affected_seat_id,open_warning_id FROM warning_conditions WHERE ordinal=?1 AND clear_warning_id IS NULL",
        [ordinal], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
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
        "SELECT h.decision_seq FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1",
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
                let row: (String, String, i64, String) = tx
                .query_row(
                    "SELECT i.thread_id, i.seat_id, i.deadline_at, CASE WHEN EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) THEN 'rejected' WHEN c.invitation_id IS NOT NULL THEN 'cancelled' ELSE i.state END FROM invitations i LEFT JOIN invitation_cancellations c ON c.invitation_id=i.id WHERE i.id=?1",
                    [id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .map_err(store_error)?;
                (
                    ThreadId::new(row.0),
                    SeatId::new(row.1),
                    Some(row.2),
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
        tx.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES (?1,?2,?3,?4,?5,?6)",
            params![condition.0,thread.as_str(),condition.1,seat.as_str(),warning.as_str(),event_seq]).map_err(store_error)?;
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
    ),
    actor_scope: &str,
    operation_key: &str,
    digest: [u8; 32],
    validate: impl FnOnce(&Transaction<'_>) -> Result<(), ApiError>,
    apply: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<CommandResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    let (claim, issuance) = cooperative;
    execute_budgeted_idempotent_transaction_with_constraints(
        context,
        conn,
        current_budget,
        Some(&issuance),
        actor_scope,
        operation_key,
        digest,
        |tx| super::seats::cooperative_instance(tx, &claim.instance, &claim),
        validate,
        apply,
        |_, result| Ok(result),
    )
}

fn verify_existing_v24(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V24
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for sql in ddl.split(';').map(str::trim).filter(|sql| !sql.is_empty()) {
        let mut words = sql.split_whitespace().skip(1);
        let kind = words.next().unwrap_or_default().to_ascii_lowercase();
        let name = words.next().unwrap_or_default();
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(str::trim) != Some(sql) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered harness diagnostic {kind} {name}"),
            ));
        }
    }
    Ok(())
}
