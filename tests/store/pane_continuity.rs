//! ht-iygh D3 (TRUST-POLICY C1 *Pane continuity*): a fresh top-level session's claim at the pane
//! that is the saved target of exactly one unresolved seat, with no live session of that seat
//! anywhere threads knows of, takes that seat. The user's decision (2026-11-27): a new session in
//! a restored pane joins that pane's seat, as it would with no restart.

use super::*;

/// `s1` unresolved by a new incarnation (`other`), saved at `held-pane`, its last `claude`
/// session `sess-1`; the pass of the current recovery epoch has run.
fn d3_fixture() -> (StoreContext, Connection, PathBuf) {
    let (context, conn, path) = c1_fixture();
    conn.execute(
        "UPDATE seats SET target_id='held-pane',unresolved_reason='other' WHERE id='s1'",
        [],
    )
    .unwrap();
    b5_set_marker_to_recovery(&conn);
    (context, conn, path)
}

fn fresh(
    target: &str,
    session: &str,
    operation: &str,
) -> crate::protocol::commands::ContinuityCheckIn {
    crate::protocol::commands::ContinuityCheckIn {
        source: crate::protocol::commands::ContinuityCheckIn::SAME_SESSION_SOURCE.into(),
        ..c1_command(target, session, operation)
    }
}

fn kinds(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT kind FROM allocation_decisions WHERE seat_id='s1' ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn a_fresh_session_at_the_only_unresolved_seats_saved_pane_takes_that_seat() {
    let (context, mut conn, path) = d3_fixture();
    let result = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-take"),
        "absent",
    )
    .unwrap();
    assert!(
        matches!(&result, CommandResult::ContinuityReattached(r) if r.seat.as_str() == "s1"),
        "{result:?}"
    );
    let (state, target): (String, Option<String>) = conn
        .query_row("SELECT state,target_id FROM seats WHERE id='s1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (state.as_str(), target.as_deref()),
        ("resolved", Some("held-pane"))
    );
    let open: Vec<(String, String)> = conn
        .prepare("SELECT native_session,observation_provenance FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        open,
        vec![("sess-new".to_owned(), "cooperative_top_level".to_owned())],
        "the claiming session's own binding opens, as for any C1 reattachment"
    );
    assert_eq!(kinds(&conn), vec!["pane_continuity".to_owned()]);
    // The replay answers the same.
    let replay = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-take"),
        "absent",
    )
    .unwrap();
    assert_eq!(replay, result);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn two_unresolved_seats_saved_at_the_pane_are_real_ambiguity() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = d3_fixture();
    c1_unresolved_seat(&conn, "s3", "claude", "sess-3");
    conn.execute(
        "UPDATE seats SET target_id='held-pane',unresolved_reason='other' WHERE id='s3'",
        [],
    )
    .unwrap();
    let refused = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-two"),
        "absent",
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::NotFound, "{refused:?}");
    assert!(kinds(&conn).is_empty());
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_kept_claim_of_the_seats_own_session_leaves_the_seat_to_it() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = d3_fixture();
    // s1's own session claimed elsewhere and was kept: it is live, so the seat is its.
    conn.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) VALUES ((SELECT min(host_id) FROM herdr_hosts),'other-pane','claude','sess-1','{}',1)",
        [],
    )
    .unwrap();
    let refused = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-kept"),
        "absent",
    )
    .unwrap_err();
    let _ = (refused, ErrorCode::NotFound);
    assert!(kinds(&conn).is_empty());
    let state: String = conn
        .query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "unresolved");
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn no_pane_continuity_while_reconciliation_lags_or_for_other_unresolve_kinds() {
    use crate::protocol::results::ErrorCode;
    // Lag: the pass may still unresolve or move seats.
    let (context, mut conn, path) = c1_fixture();
    conn.execute(
        "UPDATE seats SET target_id='held-pane',unresolved_reason='other' WHERE id='s1'",
        [],
    )
    .unwrap();
    let lagging = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-lag"),
        "absent",
    )
    .unwrap_err();
    assert_eq!(lagging.code, ErrorCode::ServiceBusy, "{lagging:?}");
    // A recoverable host invalidation (hc) is the structural path's, not this one's.
    b5_set_marker_to_recovery(&conn);
    conn.execute(
        "UPDATE seats SET unresolved_reason='host_invalidation' WHERE id='s1'",
        [],
    )
    .unwrap();
    let other = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-hc"),
        "absent",
    )
    .unwrap_err();
    assert_eq!(other.code, ErrorCode::NotFound, "{other:?}");
    assert!(kinds(&conn).is_empty());
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_session_of_another_seat_never_takes_the_pane_seat_by_pane_continuity() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = d3_fixture();
    // sess-9 is a binding of another seat (s2): not a fresh session.
    c1_bind(&conn, "s2", 1, "claude", "sess-9");
    let refused = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-9", "d3-foreign"),
        "absent",
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::NotFound, "{refused:?}");
    assert!(kinds(&conn).is_empty());
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_holding_key_conflict_on_the_pane_leaves_it_to_its_key() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = d3_fixture();
    conn.execute(
        "INSERT INTO continuity_key_conflicts(host_id,target_id,key,kind,seat_id,first_seen_at,updated_at) VALUES ((SELECT min(host_id) FROM herdr_hosts),'held-pane','k-1','key_live_elsewhere','s2',1,1)",
        [],
    )
    .unwrap();
    let refused = c1_decide(
        &context,
        &mut conn,
        &fresh("held-pane", "sess-new", "d3-keyrow"),
        "absent",
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::NotFound, "{refused:?}");
    assert!(kinds(&conn).is_empty());
    drop(conn);
    let _ = std::fs::remove_file(path);
}

/// Herdr reports the seat's own session in the claimed pane: it is live there, so the fresh claim
/// takes nothing and host session continuity (or that session's own claim) restores the seat.
/// Another reported session (a stale cache, or the claimant's own) does not stop it.
#[test]
fn herdr_naming_the_seats_own_session_in_the_pane_makes_pane_continuity_stand_aside() {
    use crate::protocol::results::ErrorCode;
    let decide =
        |context: &StoreContext, conn: &mut Connection, operation: &str, reported: &str| {
            let command = fresh("held-pane", "sess-new", operation);
            let guard = crate::ports::ContinuityTargetGuard::try_new(
                "34750f98bd59fcfc946da45aaabe933b",
                crate::test_support::test_host_id(),
                &command,
                b5_fresh_observation("held-pane"),
            )
            .unwrap();
            crate::store::seats::decide_continuity(
                context,
                conn,
                "34750f98bd59fcfc946da45aaabe933b",
                crate::ports::ContinuityRequest {
                    host_agent_session: Some(reported.to_owned()),
                    command,
                    guard,
                    diagnostic: "mismatch",
                    evidence: crate::ports::ContinuityEvidence::Cooperative,
                },
            )
        };
    let (context, mut conn, path) = d3_fixture();
    let refused = decide(&context, &mut conn, "d3-own", "sess-1").unwrap_err();
    assert_eq!(refused.code, ErrorCode::NotFound, "{refused:?}");
    assert!(kinds(&conn).is_empty());
    let taken = decide(&context, &mut conn, "d3-other", "sess-unrelated").unwrap();
    assert!(
        matches!(&taken, CommandResult::ContinuityReattached(r) if r.seat.as_str() == "s1"),
        "{taken:?}"
    );
    assert_eq!(kinds(&conn), vec!["pane_continuity".to_owned()]);
    drop(conn);
    let _ = std::fs::remove_file(path);
}
