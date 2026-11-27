//! ht-iygh D2: after the reconciliation pass a restore hold covers only the
//! saved targets of unresolved seats, never every unowned baseline target.

use super::*;
use crate::store::effective::{EffectiveRecoveryDisposition as D, effective_recovery_disposition};

const NODE: &str = "34750f98bd59fcfc946da45aaabe933b";

fn armed_baseline(targets: &[&str]) -> (StoreContext, Connection, PathBuf) {
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, targets);
    conn.execute(
        "UPDATE herdr_hosts SET baseline_hold_unclaimed=1 WHERE node_id=?1",
        [NODE],
    )
    .unwrap();
    // s1 (unresolved by the baseline helper) is saved at `pane-saved`.
    conn.execute("UPDATE seats SET target_id='pane-saved' WHERE id='s1'", [])
        .unwrap();
    (context, conn, path)
}

fn disposition(conn: &Connection, target: &str) -> D {
    effective_recovery_disposition(conn, crate::test_support::test_host_id(), target).unwrap()
}

fn held_targets(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare(crate::store::effective::RECOVERY_HOLD_TARGETS_SQL)
        .unwrap();
    statement
        .query_map([NODE], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn after_the_pass_only_an_unresolved_seats_saved_target_is_baseline_held() {
    let (_context, conn, path) = armed_baseline(&["pane-saved", "pane-free"]);
    b5_set_marker_to_recovery(&conn);
    assert_eq!(disposition(&conn, "pane-saved"), D::BaselineHeld);
    assert_eq!(disposition(&conn, "pane-free"), D::UnambiguousUnclaimed);
    assert_eq!(held_targets(&conn), vec!["pane-saved".to_owned()]);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn before_the_pass_every_unowned_baseline_target_stays_held() {
    // Before the pass a seat may still read resolved on a dead incarnation;
    // nothing is known yet about which targets its role needs.
    let (_context, conn, path) = armed_baseline(&["pane-saved", "pane-free"]);
    assert_eq!(disposition(&conn, "pane-saved"), D::BaselineHeld);
    assert_eq!(disposition(&conn, "pane-free"), D::BaselineHeld);
    assert_eq!(held_targets(&conn).len(), 2);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_waiting_or_retired_seat_holds_nothing_after_the_pass() {
    let (_context, conn, path) = armed_baseline(&["pane-saved", "pane-free"]);
    b5_set_marker_to_recovery(&conn);
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id='s1'",
        [],
    )
    .unwrap();
    assert_eq!(disposition(&conn, "pane-saved"), D::UnambiguousUnclaimed);
    assert!(held_targets(&conn).is_empty());
    drop(conn);
    let _ = std::fs::remove_file(path);
}
