//! The pre-baseline 46-step migration chain. It exists only as the source of the
//! transient `baseline_matches_legacy_chain_v46` equivalence test and is deleted
//! with that test by ht-5fr.1.8.

#![allow(dead_code)]

use super::*;

const V1: &str = include_str!("../../../migrations/0001_initial.sql");
const V2: &str = include_str!("../../../migrations/0002_service_substrate.sql");
const V3: &str = include_str!("../../../migrations/0003_invitation_cancellations.sql");
const V4: &str = include_str!("../../../migrations/0004_voluntary_membership.sql");
const V5: &str = include_str!("../../../migrations/0005_service_notifications.sql");
const V6: &str = include_str!("../../../migrations/0006_retirement_health.sql");
/// Seat- and thread-leading indexes for the attention digest producer.
const V7: &str = include_str!("../../../migrations/0007_attention_digest.sql");
/// Pending-only projections (and their maintaining triggers) for the digest.
const V8: &str = include_str!("../../../migrations/0008_digest_pending_paths.sql");
/// Occupant bindings also accept a person's pane identity (`harness='human'`).
const V9: &str = include_str!("../../../migrations/0009_human_occupant.sql");
/// B5 trust guards: reconciliation marker, wider allocation kinds and the
/// cooperative-continuity diagnostic column.
const V10: &str = include_str!("../../../migrations/0010_b5_trust_guards.sql");
/// Cooperative-only skeleton (ht-p03.2): the table sweep found nothing the
/// deleted verification layer alone used, so it drops no table; B1 appends its
/// partial indexes to this migration. (Renumbered from v10 to v11 when main's
/// B5 trust guards took v10.)
const V11: &str = include_str!("../../../migrations/0011_cooperative_only.sql");
/// Harness version evidence (ht-xoc.4): `harness_version_evidence` and
/// `harness_unattributed`.
const V12: &str = include_str!("../../../migrations/0012_harness_version_evidence.sql");
/// Thread summaries: author role and relay claims on messages, summary blocks, jobs,
/// catch-up rows and the receipt soft-poke marker (epic ht-1ip). (Renumbered
/// from v11 to v12 when main's cooperative-only skeleton took v11, then to v13
/// when main's harness version evidence took v12.)
const V13: &str = include_str!("../../../migrations/0013_thread_summaries.sql");
/// Catch-up release key (epic ht-1ip; renumbered from v12 to v13, then v14).
const V14: &str = include_str!("../../../migrations/0014_catch_up_release.sql");
/// The schema version this build writes and audits (the last migration).
const V15: &str = include_str!("../../../migrations/0015_preparation_retention.sql");
const V16: &str = include_str!("../../../migrations/0016_human_receipt_waivers.sql");
const V17: &str = include_str!("../../../migrations/0017_wake_batches.sql");
const V18: &str = include_str!("../../../migrations/0018_warning_conditions.sql");
const V19: &str = include_str!("../../../migrations/0019_thread_names.sql");
const V20: &str = include_str!("../../../migrations/0020_recent_activity.sql");
const V21: &str = include_str!("../../../migrations/0021_invitation_rejections.sql");
const V22: &str = include_str!("../../../migrations/0022_user_message_intent.sql");
const V23: &str = include_str!("../../../migrations/0023_channel_archival.sql");
const V24: &str = include_str!("../../../migrations/0024_harness_contract_diagnostics.sql");
const V25: &str = include_str!("../../../migrations/0025_warning_notice_delivery.sql");
const V26: &str = include_str!("../../../migrations/0026_lazy_message_delivery.sql");
const V27: &str = include_str!("../../../migrations/0027_harness_adapters.sql");
const V28: &str = include_str!("../../../migrations/0028_handoff_topology.sql");
const V29: &str = include_str!("../../../migrations/0029_direct_threads.sql");
const V30: &str = include_str!("../../../migrations/0030_automatic_handoff.sql");
const V31: &str = include_str!("../../../migrations/0031_seat_wake_on.sql");
const V32: &str = include_str!("../../../migrations/0032_session_bindings.sql");
const V33: &str = include_str!("../../../migrations/0033_archival_scan_scope.sql");
const V34: &str = include_str!("../../../migrations/0034_binding_delivery_state.sql");
const V35: &str = include_str!("../../../migrations/0035_binding_attention_frontiers.sql");
const V36: &str = include_str!("../../../migrations/0036_in_turn_offer.sql");
const V37: &str = include_str!("../../../migrations/0037_invitation_confirmation.sql");

const V38: &str = include_str!("../../../migrations/0038_receipt_send_time.sql");
const V39: &str = include_str!("../../../migrations/0039_handoff_body_bounds.sql");
const V40: &str = include_str!("../../../migrations/0040_receipt_deadline_backfill.sql");
const V41: &str = include_str!("../../../migrations/0041_invitation_lifecycle.sql");
const V42: &str = include_str!("../../../migrations/0042_invitation_warning_origin.sql");
const V43: &str = include_str!("../../../migrations/0043_receipt_warning_origins.sql");
const V44: &str = include_str!("../../../migrations/0044_automatic_handoff_body_bounds.sql");
const V45: &str = include_str!("../../../migrations/0045_invitation_presentation.sql");
const V46: &str = include_str!("../../../migrations/0046_presentation_relevance.sql");
pub(crate) const LATEST_VERSION: i64 = 46;

/// The migration clock is sampled only for legacy preparation grace or migrated binding disconnection.
pub(crate) fn legacy_initialize(
    conn: &Connection,
    now: impl FnOnce() -> UtcMillis,
) -> Result<(), ApiError> {
    // The clock is sampled at most once; later reads reuse the first sample.
    let mut source = Some(now);
    let mut sampled: Option<UtcMillis> = None;
    let mut now =
        move || *sampled.get_or_insert_with(|| (source.take().expect("clock sampled once"))());
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
                .and_then(|_| conn.execute_batch(V26))
                .and_then(|_| conn.execute_batch(V27))
                .and_then(|_| conn.execute_batch(V28))
                .and_then(|_| conn.execute_batch(V29))
                .and_then(|_| conn.execute_batch(V30))
                .and_then(|_| conn.execute_batch(V31))
                .and_then(|_| conn.execute_batch(V32))
                .and_then(|_| conn.execute_batch(V33))
                .and_then(|_| conn.execute_batch(V34))
                .and_then(|_| conn.execute_batch(V35))
                .and_then(|_| conn.pragma_update(None, "user_version", 35));
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
        18..=LATEST_VERSION => verify_existing(conn),
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
    if (1..=26).contains(&version) {
        verify_existing_v24(conn)?;
    }
    if (1..=24).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V25)
            .and_then(|_| conn.pragma_update(None, "user_version", 25));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v25(conn)?;
    if (1..=25).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V26)
            .and_then(|_| conn.pragma_update(None, "user_version", 26));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_existing_v26(conn)?;
    if (1..=26).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V27)
            .map_err(store_error)
            .and_then(|_| verify_existing_v23(conn))
            .and_then(|_| verify_existing_v27(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 27)
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
    verify_existing_v27(conn)?;
    if (1..=27).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V28)
            .map_err(store_error)
            .and_then(|_| verify_existing_v28(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 28)
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
    if version <= 38 {
        verify_existing_v28(conn)?;
    } else {
        verify_handoff_body_bounds_schema(conn)?;
    }
    if (1..=28).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V29)
            .map_err(store_error)
            .and_then(|_| verify_existing_v29(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 29)
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
    verify_existing_v29(conn)?;
    if (1..=29).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V30)
            .map_err(store_error)
            .and_then(|_| verify_existing_v30(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 30)
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
    if version <= 43 {
        verify_existing_v30(conn)?;
    } else {
        verify_automatic_body_bounds_schema(conn)?;
    }
    if (1..=30).contains(&version) {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V31)
            .map_err(store_error)
            .and_then(|_| verify_existing_v31(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 31)
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
    verify_existing_v31(conn)?;
    if (1..=31).contains(&version) {
        migrate_v31_to_v32(conn, now())?;
    }
    verify_existing_v32(conn)?;
    if (1..=32).contains(&version) {
        migrate_v32_to_v33(conn)?;
    }
    verify_existing_v33(conn)?;
    if (1..=33).contains(&version) {
        migrate_v33_to_v34(conn)?;
    }
    verify_existing_v34(conn)?;
    if (1..=34).contains(&version) {
        migrate_v34_to_v35(conn)?;
    }
    verify_existing_v35(conn)?;
    if version <= 35 {
        migrate_v35_to_v36(conn)?;
    }
    verify_existing_v36(conn)?;
    if version <= 36 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V37)
            .map_err(store_error)
            .and_then(|_| verify_invitation_confirmation_schema(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 37)
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
    verify_invitation_confirmation_schema(conn)?;
    if version <= 37 {
        migrate_receipts_v37_to_v38(conn)?;
    }
    verify_receipt_send_time_schema(conn)?;
    if version <= 38 {
        migrate_handoff_v38_to_v39(conn)?;
    }
    verify_handoff_body_bounds_schema(conn)?;
    if version <= 39 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V40)
            .and_then(|_| conn.pragma_update(None, "user_version", 40));
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(store_error)?,
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(store_error(error));
            }
        }
    }
    verify_receipt_deadline_backfill_schema(conn)?;
    if version <= 40 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V41)
            .map_err(store_error)
            .and_then(|_| verify_invitation_lifecycle_schema(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 41)
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
    verify_invitation_lifecycle_schema(conn)?;
    if version <= 41 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V42)
            .map_err(store_error)
            .and_then(|_| verify_invitation_warning_origin_schema(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 42)
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
    verify_invitation_warning_origin_schema(conn)?;
    if version <= 42 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V43)
            .map_err(store_error)
            .and_then(|_| verify_receipt_warning_origins_schema(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 43)
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
    verify_receipt_warning_origins_schema(conn)?;
    if version <= 43 {
        migrate_automatic_v43_to_v44(conn)?;
    }
    verify_automatic_body_bounds_schema(conn)?;
    if version <= 44 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V45)
            .map_err(store_error)
            .and_then(|_| verify_invitation_presentation_schema(conn, false))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 45)
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
    verify_invitation_presentation_schema(conn, version >= 46)?;
    if version <= 45 {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        let result = conn
            .execute_batch(V46)
            .map_err(store_error)
            .and_then(|_| verify_presentation_relevance_schema(conn))
            .and_then(|_| {
                conn.pragma_update(None, "user_version", 46)
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
    verify_invitation_presentation_schema(conn, true)?;
    verify_presentation_relevance_schema(conn)
}

/// V35 (ht-54r.9): the warning offer and notice frontiers move from one row per seat to one row
/// per binding. One transaction.
fn migrate_v34_to_v35(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V35)
        .map_err(store_error)
        .and_then(|_| verify_existing_v35(conn))
        .and_then(|_| {
            conn.pragma_update(None, "user_version", 35)
                .map_err(store_error)
        });
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Objects V35 creates or rebuilds, audited exactly as written (see `V32_AUDITED`).
const V35_AUDITED: [(&str, &str, &str); 4] = [
    ("table", "warning_offer_v35", "warning_offer"),
    ("index", "warning_offer_seat", "warning_offer_seat"),
    ("table", "digest_notice_offer_v35", "digest_notice_offer"),
    (
        "index",
        "digest_notice_offer_seat",
        "digest_notice_offer_seat",
    ),
];

fn verify_existing_v35(conn: &Connection) -> Result<(), ApiError> {
    audit_rebuilt_objects(conn, V35, &V35_AUDITED, "binding attention frontier")
}

/// V36 (ht-705): per-binding receipt presentation, the wake candidate's in-turn evidence. One
/// transaction.
fn migrate_v35_to_v36(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V36)
        .map_err(store_error)
        .and_then(|_| verify_existing_v36(conn))
        .and_then(|_| {
            conn.pragma_update(None, "user_version", 36)
                .map_err(store_error)
        });
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Objects V36 creates, audited exactly as written (see `V32_AUDITED`).
const V36_AUDITED: [(&str, &str, &str); 2] = [
    ("table", "in_turn_offer", "in_turn_offer"),
    ("index", "in_turn_offer_seat", "in_turn_offer_seat"),
];

fn verify_existing_v36(conn: &Connection) -> Result<(), ApiError> {
    audit_rebuilt_objects(conn, V36, &V36_AUDITED, "receipt offer")
}

/// V33 (ht-54r.15): archival scan scope. Narrows the mutation-revision triggers so the revision
/// moves only for membership, member-liveness and seat-lifecycle changes.
fn migrate_v32_to_v33(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V33)
        .map_err(store_error)
        .and_then(|_| verify_existing_v33(conn))
        .and_then(|_| {
            conn.pragma_update(None, "user_version", 33)
                .map_err(store_error)
        });
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// V23 triggers that V33 drops or replaces. From schema version 33 the V33 text owns them, so
/// `verify_existing_v23` skips them and `verify_existing_v33` audits them byte for byte.
const V33_SUPERSEDED_V23_TRIGGERS: [&str; 5] = [
    "archival_fence_seats_insert",
    "archival_fence_seats_update",
    "archival_fence_memberships_update",
    "archival_fence_occupant_bindings_insert",
    "archival_fence_occupant_bindings_update",
];

/// Audits every object V33 creates exactly as written, and that each trigger it only drops is
/// absent.
fn verify_existing_v33(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V33
        .lines()
        .filter(|line| !line.trim_start().starts_with("--") && !line.starts_with("DROP "))
        .collect::<Vec<_>>()
        .join("\n");
    let normalize = |sql: &str| sql.trim().trim_end_matches(';').trim().to_owned();
    let mut created = Vec::new();
    for object in ddl.trim().split("\nCREATE ") {
        let sql = if object.starts_with("CREATE ") {
            object.to_owned()
        } else {
            format!("CREATE {object}")
        };
        let name = sql.split_whitespace().nth(2).unwrap_or_default().to_owned();
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [&name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref().map(normalize) != Some(normalize(&sql)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered archival scan scope trigger {name}"),
            ));
        }
        created.push(name);
    }
    for name in V33_SUPERSEDED_V23_TRIGGERS {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1)",
                [name],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if present != created.iter().any(|c| c == name) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("unexpected archival scan scope trigger state for {name}"),
            ));
        }
    }
    Ok(())
}

/// V34 (ht-54r.22): lazy display state onto binding cursors (legacy pending carry-over) and
/// catch-up rows keyed by binding. One transaction.
fn migrate_v33_to_v34(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V34)
        .map_err(store_error)
        .and_then(|_| verify_existing_v34(conn))
        .and_then(|_| {
            conn.pragma_update(None, "user_version", 34)
                .map_err(store_error)
        });
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Objects V34 creates or rebuilds, audited exactly as written (see `V32_AUDITED`).
const V34_AUDITED: [(&str, &str, &str); 6] = [
    ("table", "catch_up_v34", "catch_up"),
    (
        "index",
        "catch_up_extension_until",
        "catch_up_extension_until",
    ),
    ("index", "catch_up_thread_active", "catch_up_thread_active"),
    ("index", "catch_up_seat", "catch_up_seat"),
    ("index", "lazy_recipients_legacy", "lazy_recipients_legacy"),
    (
        "index",
        "lazy_recipients_seat_ordinal",
        "lazy_recipients_seat_ordinal",
    ),
];

fn verify_existing_v34(conn: &Connection) -> Result<(), ApiError> {
    audit_rebuilt_objects(conn, V34, &V34_AUDITED, "binding delivery state")?;
    let column_ok: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('lazy_recipients') WHERE name='legacy_pending' AND type='INTEGER' AND \"notnull\"=1 AND dflt_value='0')",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let table_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='lazy_recipients'",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !column_ok
        || !table_sql
            .contains("legacy_pending INTEGER NOT NULL DEFAULT 0 CHECK(legacy_pending IN (0,1))")
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered lazy_recipients.legacy_pending",
        ));
    }
    // The rebuilt catch_up must not be a stale V32-era shape.
    let shaped: bool = conn
        .query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM pragma_table_info('catch_up') WHERE name IN ('binding_generation','execution_id'))",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !shaped {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "catch_up still carries binding generation columns",
        ));
    }
    Ok(())
}

/// V32 (ht-54r): session-keyed bindings. The SQL rebuild runs in one transaction; the
/// upgrade time is the disconnected_at of each open migrated binding (archival grace).
fn migrate_v31_to_v32(conn: &Connection, now: UtcMillis) -> Result<(), ApiError> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
    let result = conn
        .execute_batch(V32)
        .map_err(store_error)
        .and_then(|_| {
            conn.execute(
                "UPDATE occupant_bindings SET disconnected_at=?1 WHERE migrated=1 AND ended_at IS NULL",
                [now.0],
            )
            .map(|_| ())
            .map_err(store_error)
        })
        .and_then(|_| verify_existing_v32(conn))
        .and_then(|_| {
            conn.pragma_update(None, "user_version", 32)
                .map_err(store_error)
        });
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(store_error),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// Objects V32 creates or rebuilds, audited exactly as written: (kind, name as written in V32,
/// name as installed). The rebuilt table is installed under its final name, which SQLite's
/// RENAME writes quoted.
const V32_AUDITED: [(&str, &str, &str); 7] = [
    ("table", "occupant_bindings_v32", "occupant_bindings"),
    (
        "index",
        "occupant_bindings_session_key",
        "occupant_bindings_session_key",
    ),
    ("index", "occupant_bindings_open", "occupant_bindings_open"),
    (
        "index",
        "occupant_bindings_history",
        "occupant_bindings_history",
    ),
    (
        "index",
        "occupant_bindings_execution",
        "occupant_bindings_execution",
    ),
    ("table", "binding_cursors", "binding_cursors"),
    ("index", "binding_cursors_thread", "binding_cursors_thread"),
];

/// Compare each object `audited` lists against its statement in `source`, exactly as written.
fn audit_rebuilt_objects(
    conn: &Connection,
    source: &str,
    audited: &[(&str, &str, &str)],
    what: &str,
) -> Result<(), ApiError> {
    let normalize = |sql: &str| sql.trim().trim_end_matches(';').trim().to_owned();
    for (kind, written, installed) in audited.iter().copied() {
        let headers: &[String] = &if kind == "table" {
            vec![format!("CREATE TABLE {written} ")]
        } else {
            vec![
                format!("CREATE INDEX {written} "),
                format!("CREATE UNIQUE INDEX {written} "),
            ]
        };
        let start = headers
            .iter()
            .find_map(|header| source.find(header.as_str()))
            .expect("audited object present");
        let end = start + source[start..].find(';').expect("statement terminator");
        let mut expected = source[start..end].to_owned();
        if written != installed {
            expected = expected.replacen(written, &format!("\"{installed}\""), 1);
        }
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, installed],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(normalize) != Some(normalize(&expected)) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered {what} {kind} {installed}"),
            ));
        }
    }
    Ok(())
}

fn verify_existing_v32(conn: &Connection) -> Result<(), ApiError> {
    audit_rebuilt_objects(conn, V32, &V32_AUDITED, "session binding")?;
    for (table, column) in [
        ("receipts", "ack_binding_id"),
        ("receipt_state", "ack_binding_id"),
        ("messages", "sender_binding_id"),
    ] {
        let present: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2 AND type='TEXT')",
                params![table, column],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !present {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing session binding column {table}.{column}"),
            ));
        }
    }
    Ok(())
}

fn verify_existing_v29(conn: &Connection) -> Result<(), ApiError> {
    let expected = V29
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    let actual: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='direct_threads'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if actual.as_deref().map(str::trim) != Some(expected.trim().trim_end_matches(';')) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered direct pair registry",
        ));
    }
    Ok(())
}

/// The two DP8 columns: type, nullability and default are part of the contract
/// (a fresh seat reads `nudge`; the lazy frontier starts absent).
fn verify_existing_v31(conn: &Connection) -> Result<(), ApiError> {
    for (table, column, kind, not_null, default) in [
        ("seats", "wake_on", "TEXT", true, Some("'nudge'")),
        ("wake_work", "last_lazy_ordinal", "INTEGER", false, None),
    ] {
        let found: Option<(String, bool, Option<String>)> = conn
            .query_row(
                &format!("SELECT type,\"notnull\",dflt_value FROM pragma_table_info('{table}') WHERE name=?1"),
                [column],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(store_error)?;
        if found != Some((kind.to_owned(), not_null, default.map(str::to_owned))) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered wake_on column {table}.{column}"),
            ));
        }
    }
    Ok(())
}

fn verify_existing_v26(conn: &Connection) -> Result<(), ApiError> {
    let v34 = schema_version(conn)? >= 34;
    // Compare every lazy object, including trigger bodies with embedded semicolons.
    let ddl = V26
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in ddl
        .split("\n\n")
        .map(str::trim)
        .filter(|s| s.starts_with("CREATE "))
    {
        let sql = statement.trim_end_matches(';');
        let words = sql.split_whitespace().collect::<Vec<_>>();
        let kind = words[1].to_ascii_lowercase();
        let name = words[2];
        // V34 appends one column to lazy_recipients; SQLite stores it right after `state`.
        let appended;
        let sql = if kind == "table" && name == "lazy_recipients" && v34 {
            appended = sql.replacen(
                "CHECK(state IN ('pending','displayed'))",
                "CHECK(state IN ('pending','displayed')), legacy_pending INTEGER NOT NULL DEFAULT 0 CHECK(legacy_pending IN (0,1))",
                1,
            );
            appended.as_str()
        } else {
            sql
        };
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(str::trim) != Some(sql) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered lazy delivery {kind} {name}"),
            ));
        }
    }
    for table in ["messages", "send_preparations"] {
        let valid: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name='delivery_mode' AND type='TEXT' AND \"notnull\"=1 AND dflt_value=\"'ordinary'\")", [table], |r| r.get(0)).map_err(store_error)?;
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !valid || !sql.contains("delivery_mode TEXT NOT NULL DEFAULT 'ordinary' CHECK(delivery_mode IN ('ordinary','lazy'))") {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "invalid recorded delivery mode",
            ));
        }
    }
    Ok(())
}

fn verify_existing_v25(conn: &Connection) -> Result<(), ApiError> {
    let expected = V25
        .split("\n\n")
        .find_map(|statement| {
            let start = statement.find("CREATE TRIGGER ")?;
            Some(statement[start..].trim().trim_end_matches(';'))
        })
        .ok_or_else(|| {
            api_error(
                ErrorCode::IncompatibleSchema,
                "missing warning delivery DDL",
            )
        })?;
    let actual: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='digest_transition_warning_projected'",
        [], |row| row.get(0),
    ).optional().map_err(store_error)?;
    if actual.as_deref().map(str::trim) != Some(expected) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered warning delivery projection",
        ));
    }
    Ok(())
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
    // From V33 the scan-scope triggers are owned and audited by `verify_existing_v33`.
    let v33 = schema_version(conn)? >= 33;
    for object in ddl.trim().split("\nCREATE ") {
        let sql = if object.starts_with("CREATE ") {
            object.to_owned()
        } else {
            format!("CREATE {object}")
        };
        let mut words = sql.split_whitespace().skip(1);
        let kind = words.next().unwrap_or_default().to_ascii_lowercase();
        let name = words.next().unwrap_or_default();
        if v33 && kind == "trigger" && V33_SUPERSEDED_V23_TRIGGERS.contains(&name) {
            continue;
        }
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

fn verify_existing_v27(conn: &Connection) -> Result<(), ApiError> {
    let normalize = |sql: &str| {
        sql.split_whitespace()
            .collect::<String>()
            .trim_end_matches(';')
            .replace('"', "")
            .to_ascii_lowercase()
    };
    for statement in V27.split(';') {
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
        // V32 rebuilds occupant_bindings and replaces its current-binding index; its own
        // audit (`verify_existing_v32`) owns them. The history and execution indexes are
        // recreated identically and stay audited here.
        if schema_version(conn)? >= 32
            && (name == "occupant_bindings_v22" || name == "occupant_bindings_current")
        {
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
                format!("incompatible v26 {kind} {installed_name}"),
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
        >= 27
    {
        return verify_existing_v27(conn);
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
    let v32 = schema_version(conn)? >= 32;
    for name in B1_EXISTING_ACCESS_PATHS {
        let name = if v32 && name == "occupant_bindings_current" {
            "occupant_bindings_open"
        } else {
            name
        };
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
        >= 27
    {
        return verify_existing_v27(conn);
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
        // V35 rebuilds the notice frontier per binding; its own audit owns it from then on.
        if name == "digest_notice_offer" && schema_version(conn)? >= 35 {
            continue;
        }
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
    // V32 replaces the one-open-binding-per-seat index with the non-unique access path.
    let current_binding_index = if schema_version(conn)? >= 32 {
        "occupant_bindings_open"
    } else {
        "occupant_bindings_current"
    };
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
        current_binding_index,
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

fn migrate_handoff_v38_to_v39(conn: &Connection) -> Result<(), ApiError> {
    // SQLite ignores foreign_keys changes inside a transaction. Disable it
    // before BEGIN so rebuilding the cyclic parent/recovery tables cannot
    // delete child rows; verify the complete graph before committing.
    if !conn.is_autocommit() {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "handoff bounds migration requires an idle connection",
        ));
    }
    let foreign_keys: bool = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(store_error)?;
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(store_error)?;
    let migrate = (|| {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        conn.execute_batch(V39).map_err(store_error)?;
        verify_handoff_body_bounds_schema(conn)?;
        let violation: Option<String> = conn
            .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
            .optional()
            .map_err(store_error)?;
        if violation.is_some() {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "handoff bounds migration found a foreign-key violation",
            ));
        }
        conn.pragma_update(None, "user_version", 39)
            .map_err(store_error)?;
        conn.execute_batch("COMMIT").map_err(store_error)
    })();
    if migrate.is_err() && !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    let restore = conn
        .pragma_update(None, "foreign_keys", foreign_keys)
        .map_err(store_error);
    migrate.and(restore)
}

fn migrate_automatic_v43_to_v44(conn: &Connection) -> Result<(), ApiError> {
    // SQLite ignores foreign_keys changes inside a transaction. Disable it
    // before BEGIN so rebuilding the cyclic parent/recovery tables cannot
    // delete child rows; verify the complete graph before committing.
    if !conn.is_autocommit() {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "automatic handoff bounds migration requires an idle connection",
        ));
    }
    let foreign_keys: bool = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(store_error)?;
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(store_error)?;
    let migrate = (|| {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        conn.execute_batch(V44).map_err(store_error)?;
        verify_automatic_body_bounds_schema(conn)?;
        let violation: Option<String> = conn
            .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
            .optional()
            .map_err(store_error)?;
        if violation.is_some() {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "automatic handoff bounds migration found a foreign-key violation",
            ));
        }
        conn.pragma_update(None, "user_version", 44)
            .map_err(store_error)?;
        conn.execute_batch("COMMIT").map_err(store_error)
    })();
    if migrate.is_err() && !conn.is_autocommit() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    let restore = conn
        .pragma_update(None, "foreign_keys", foreign_keys)
        .map_err(store_error);
    migrate.and(restore)
}

fn verify_existing_v28(conn: &Connection) -> Result<(), ApiError> {
    verify_ddl(conn, V28, "bootstrap topology")
}
fn verify_handoff_body_bounds_schema(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V28
        .replace(
            "length(identity_json)<=131072",
            "length(identity_json)<=524288",
        )
        .replace(
            "length(result_json)<=262144",
            "length(result_json)<=1048576",
        );
    verify_ddl(conn, &ddl, "bootstrap topology")
}
fn verify_automatic_body_bounds_schema(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V30
        .replace(
            "length(original_json)<=131072",
            "length(original_json)<=524288",
        )
        .replace(
            "length(attachment_json)<=131072",
            "length(attachment_json)<=524288",
        )
        .replace(
            "length(result_json)<=262144",
            "length(result_json)<=2097152",
        );
    verify_ddl(conn, &ddl, "automatic handoff")
}
fn verify_existing_v30(conn: &Connection) -> Result<(), ApiError> {
    verify_ddl(conn, V30, "automatic handoff")
}
fn verify_ddl(conn: &Connection, source: &str, label: &str) -> Result<(), ApiError> {
    let ddl = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    // Each top-level CREATE begins on its own line; trigger-body semicolons
    // remain part of the complete SQL. Preserve constraints, FK clauses,
    // predicates and quoted literals exactly, as in the archival23 audit.
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
                format!("missing or altered {label} {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn verify_presentation_relevance_schema(conn: &Connection) -> Result<(), ApiError> {
    let expected = Connection::open_in_memory().map_err(store_error)?;
    // Only DDL dependency stubs: no retained content or historical migration
    // replay is needed to reconstruct the exact new catalog objects.
    expected.execute_batch("CREATE TABLE digest_pending_invitations(seat_id TEXT,invitation_id TEXT,ordinal INTEGER); CREATE TABLE invitations(id TEXT,seat_id TEXT,confirmation TEXT); CREATE TABLE requirement_episodes(invitation_id TEXT,state TEXT); CREATE TABLE warning_conditions(affected_seat_id TEXT,ordinal INTEGER,clear_warning_id TEXT); CREATE TABLE membership_intervals(thread_id TEXT,seat_id TEXT,ordinal INTEGER); CREATE TABLE prepared_unavailable_warnings(warning_id TEXT,ordinal INTEGER); CREATE TABLE seats(id TEXT); CREATE TABLE invitation_presentation_consumers(binding_id TEXT,seat_id TEXT,consumer TEXT);").map_err(store_error)?;
    let mut dependencies = std::collections::BTreeSet::from([
        "digest_pending_invitations",
        "invitations",
        "requirement_episodes",
        "warning_conditions",
        "membership_intervals",
        "prepared_unavailable_warnings",
        "seats",
        "invitation_presentation_consumers",
    ]);
    for line in V46
        .lines()
        .filter(|line| line.starts_with("CREATE TRIGGER "))
    {
        let table = line
            .rsplit_once(" ON ")
            .ok_or_else(|| {
                api_error(
                    ErrorCode::IncompatibleSchema,
                    "invalid relevance trigger dependency",
                )
            })?
            .1
            .trim();
        if dependencies.insert(table) {
            expected
                .execute_batch(&format!("CREATE TABLE {table}(id TEXT)"))
                .map_err(store_error)?;
        }
    }
    expected.execute_batch(V46).map_err(store_error)?;
    let mut statement = expected
        .prepare("SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")
        .map_err(store_error)?;
    for row in statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(store_error)?
    {
        let (kind, name, sql) = row.map_err(store_error)?;
        if dependencies.contains(name.as_str()) {
            continue;
        }
        let installed: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if installed.as_deref() != Some(sql.as_str()) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered presentation relevance {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn verify_invitation_presentation_schema(conn: &Connection, revised: bool) -> Result<(), ApiError> {
    let expected = Connection::open_in_memory().map_err(store_error)?;
    expected.execute_batch("CREATE TABLE invitations(id TEXT); CREATE TABLE digest_pending_invitations(seat_id TEXT,thread_id TEXT,ordinal INTEGER); CREATE TABLE threads(id TEXT,instance_id TEXT); CREATE TABLE host_instances(id TEXT);").map_err(store_error)?;
    expected.execute_batch(V45).map_err(store_error)?;
    if revised {
        // Reconstruct the final catalog using the exact released-in-this-change
        // ALTER statements. Keep the pre45 check exact during the upgrade.
        for statement in V46
            .lines()
            .filter(|line| line.starts_with("ALTER TABLE invitation_presentation_consumers "))
        {
            expected.execute_batch(statement).map_err(store_error)?;
        }
    }
    let mut objects=expected.prepare("SELECT type,name,sql FROM sqlite_master WHERE name NOT IN ('invitations','digest_pending_invitations','threads','host_instances') AND name NOT LIKE 'sqlite_%'").map_err(store_error)?;
    for row in objects
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(store_error)?
    {
        let (kind, name, sql) = row.map_err(store_error)?;
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref() != Some(sql.as_str()) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("incompatible invitation presentation object {name}"),
            ));
        }
    }
    let column:Option<(String,i64,Option<String>)>=conn.query_row("SELECT type,\"notnull\",dflt_value FROM pragma_table_info('invitations') WHERE name='command_context_epoch'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
    if column != Some(("INTEGER".into(), 1, Some("0".into()))) {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "invalid invitation command epoch column",
        ));
    }
    Ok(())
}

fn verify_receipt_warning_origins_schema(conn: &Connection) -> Result<(), ApiError> {
    // SQLite itself parses the immutable DDL, including compound trigger
    // bodies. Compare every created object, rather than splitting on semicolons.
    let expected = Connection::open_in_memory().map_err(store_error)?;
    // A schema-only reference needs parent names for trigger creation; no data
    // is copied and foreign keys remain off in this isolated parser connection.
    expected.execute_batch("CREATE TABLE receipts(ordinal INTEGER PRIMARY KEY,message_id,seat_id,warning_message_id,state,ack_required,deadline_at); CREATE TABLE receipt_state(message_id,seat_id,warning_message_id); CREATE TABLE warning_conditions(condition_kind,clear_warning_id,open_warning_id,cleared_seq);").map_err(store_error)?;
    // Population statements require the canonical parent schema. Parse only
    // CREATE objects, retaining whole trigger bodies.
    let mut sql = String::new();
    for line in V43.lines() {
        if sql.is_empty() && !line.trim_start().starts_with("CREATE ") {
            continue;
        }
        sql.push_str(line);
        sql.push('\n');
        let complete = if sql.starts_with("CREATE TRIGGER ") {
            line.trim_end().ends_with("END;")
        } else {
            line.trim_end().ends_with(';')
        };
        if complete {
            expected.execute_batch(&sql).map_err(store_error)?;
            sql.clear();
        }
    }
    let mut stmt=expected.prepare("SELECT type,name,sql FROM sqlite_master WHERE name NOT IN ('receipts','receipt_state','warning_conditions') AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(store_error)?;
    let objects = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(store_error)?;
    for object in objects {
        let (kind, name, sql) = object.map_err(store_error)?;
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref() != Some(sql.as_str()) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered receipt warning origin {name}"),
            ));
        }
    }
    Ok(())
}

fn verify_invitation_warning_origin_schema(conn: &Connection) -> Result<(), ApiError> {
    let table: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='warning_conditions'",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    for column in [
        "invitation_originator_instance_id TEXT REFERENCES host_instances(id)",
        "invitation_originator_seat_id TEXT REFERENCES seats(id)",
    ] {
        if !table.contains(column) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "missing invitation warning origin column",
            ));
        }
    }
    let index = V42
        .split("CREATE INDEX ")
        .nth(1)
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let trigger = V42
        .split("CREATE TRIGGER ")
        .nth(1)
        .unwrap()
        .trim()
        .trim_end_matches(';');
    for (kind, sql) in [
        ("index", format!("CREATE INDEX {index}")),
        ("trigger", format!("CREATE TRIGGER {trigger}")),
    ] {
        let name = sql.split_whitespace().nth(2).unwrap();
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(str::trim) != Some(sql.trim()) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered invitation warning origin {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn verify_invitation_lifecycle_schema(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V41
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in ddl.split(';').map(str::trim).filter(|sql| !sql.is_empty()) {
        let name = statement.split_whitespace().nth(2).unwrap_or_default();
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(str::trim) != Some(statement) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered invitation lifecycle index {name}"),
            ));
        }
    }
    Ok(())
}

fn verify_receipt_deadline_backfill_schema(conn: &Connection) -> Result<(), ApiError> {
    let expected = V40
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    let expected = expected.split(';').next().unwrap().trim();
    let actual: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='receipt_deadline_backfill'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if actual
        .as_deref()
        .map(|sql| sql.split_whitespace().collect::<String>())
        != Some(expected.split_whitespace().collect::<String>())
    {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing or altered receipt deadline backfill table",
        ));
    }
    let marker: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM receipt_deadline_backfill WHERE singleton=1)",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !marker {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "missing receipt deadline backfill cursor",
        ));
    }
    Ok(())
}

fn verify_invitation_confirmation_schema(conn: &Connection) -> Result<(), ApiError> {
    let table: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='invitations'",
            [],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    for column in [
        "confirmation TEXT CHECK(confirmation IN ('lazy','require_ack'))",
        "inviter_seat_id TEXT REFERENCES seats(id)",
    ] {
        if !table.contains(column) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "invalid invitation confirmation column",
            ));
        }
    }
    for statement in V37.split("\n\n").filter(|sql| sql.starts_with("CREATE ")) {
        let expected = statement.trim().trim_end_matches(';');
        let mut words = expected.split_whitespace().skip(1);
        let kind = words.next().unwrap_or_default().to_ascii_lowercase();
        let name = words.next().unwrap_or_default();
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(str::trim) != Some(expected) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered invitation confirmation {kind} {name}"),
            ));
        }
    }
    Ok(())
}

fn migrate_receipts_v37_to_v38(conn: &Connection) -> Result<(), ApiError> {
    if !conn.is_autocommit() {
        return Err(api_error(
            ErrorCode::IncompatibleSchema,
            "receipt upgrade requires autocommit",
        ));
    }
    let foreign_keys: bool = conn
        .pragma_query_value(None, "foreign_keys", |r| r.get(0))
        .map_err(store_error)?;
    conn.pragma_update(None, "foreign_keys", false)
        .map_err(store_error)?;
    let result = (|| {
        conn.execute_batch("BEGIN IMMEDIATE").map_err(store_error)?;
        conn.execute_batch(V38).map_err(store_error)?;
        verify_receipt_send_time_schema(conn)?;
        let violation: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
                [],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if violation {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                "receipt upgrade foreign key violation",
            ));
        }
        conn.pragma_update(None, "user_version", 38)
            .map_err(store_error)?;
        conn.execute_batch("COMMIT").map_err(store_error)
    })();
    if result.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    let restored = conn
        .pragma_update(None, "foreign_keys", foreign_keys)
        .map_err(store_error);
    result.and(restored)
}

fn verify_receipt_send_time_schema(conn: &Connection) -> Result<(), ApiError> {
    let ddl = V38
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in ddl
        .split("\n\n")
        .map(str::trim)
        .filter(|s| s.starts_with("CREATE ") && !s.starts_with("CREATE TEMP "))
    {
        let sql = statement.trim_end_matches(';');
        let words = sql.split_whitespace().collect::<Vec<_>>();
        let kind = words[1].to_ascii_lowercase();
        let name = words[2];
        let actual: Option<String> = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type=?1 AND name=?2",
                params![kind, name],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if actual.as_deref().map(str::trim) != Some(sql) {
            return Err(api_error(
                ErrorCode::IncompatibleSchema,
                format!("missing or altered receipt send-time {kind} {name}"),
            ));
        }
    }
    Ok(())
}
