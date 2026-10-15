//! ht-54r.14: pane-close retirement within a live Herdr incarnation, and restore holds across a
//! Herdr restart, with several session bindings on one seat. The production elected daemon runs
//! in this process against a private named Herdr session (`lane_wiring::Session`; never the
//! shared server). Bindings are made through the built executable with the cooperative caller
//! flags, one lifecycle check-in per native session.
use crate::lane_wiring::{Caller, Session, wait_until};
use rusqlite::Connection;
use std::time::Duration;

/// A seat on `pane` with two connected session bindings (two native sessions), a thread it has
/// joined, and the bindings' ids.
fn seat_with_two_bindings(
    s: &Session,
    pane: &str,
    other: &Caller,
) -> (Caller, String, Vec<String>) {
    let seat = s.ok(None, &["seat", "resolve", "--pane", pane]);
    let caller = Caller {
        seat: seat.as_str().unwrap().to_owned(),
        pane: pane.to_owned(),
    };
    let check_in = |n: u32| {
        s.ok_in_session(
            Some(&caller),
            Some(&format!("session-{n}")),
            &[
                "check-in",
                "--lifecycle-event",
                &format!("start-{n}"),
                "--native-session",
                &format!("session-{n}"),
            ],
        );
    };
    // The second session enrolls after the join, so it is seeded with a cursor on the thread.
    check_in(1);
    let thread = s
        .ok(Some(other), &["thread", "create", "--topic", "retirement"])
        .as_str()
        .unwrap()
        .to_owned();
    s.ok(Some(other), &["invite", &thread, "--seat", &caller.seat]);
    s.ok_in_session(Some(&caller), Some("session-1"), &["accept", &thread]);
    check_in(2);
    let bindings = open_bindings(&s.db(), &caller.seat);
    assert_eq!(
        bindings.len(),
        2,
        "two sessions, two bindings: {}",
        s.dump_table("occupant_bindings")
    );
    (caller, thread, bindings)
}

fn open_bindings(db: &Connection, seat: &str) -> Vec<String> {
    let mut statement = db
        .prepare("SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND liveness='connected' ORDER BY ordinal")
        .unwrap();
    statement
        .query_map([seat], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn scalar(db: &Connection, sql: &str, seat: &str) -> String {
    db.query_row(sql, [seat], |r| r.get::<_, String>(0))
        .unwrap_or_else(|e| format!("<{e}>"))
}

fn cursor_rows(db: &Connection, seat: &str) -> Vec<(String, i64)> {
    let mut statement = db
        .prepare("SELECT c.binding_id,c.seq FROM binding_cursors c JOIN occupant_bindings b ON b.binding_id=c.binding_id WHERE b.seat_id=?1 ORDER BY c.binding_id")
        .unwrap();
    statement
        .query_map([seat], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Kills: retirement that leaves a binding connected or open, or that drops the seat's
/// membership or cursor history, when a seat has more than one session binding.
#[test]
fn closing_a_pane_with_two_bindings_retires_its_seat() {
    let Some(s) = Session::new("closing_a_pane_with_two_bindings_retires_its_seat") else {
        return;
    };
    let kept_pane = s.first_pane();
    let closed_pane = s.split_pane(&kept_pane);
    let kept = s.seat(&kept_pane);
    let (closed, thread, bindings) = seat_with_two_bindings(&s, &closed_pane, &kept);
    let cursors_before = cursor_rows(&s.db(), &closed.seat);
    assert!(
        !cursors_before.is_empty(),
        "the joined-thread cursor of the session that enrolled after the join: {cursors_before:?}"
    );
    s.herdr_api(&["pane", "close", &closed_pane]);
    wait_until("the seat to retire", Duration::from_secs(60), || {
        scalar(&s.db(), "SELECT state FROM seats WHERE id=?1", &closed.seat) == "retired"
    });
    let db = s.db();
    for binding in &bindings {
        let (ended, liveness, reason): (Option<i64>, String, Option<String>) = db
            .query_row(
                "SELECT ended_at,liveness,disconnect_reason FROM occupant_bindings WHERE binding_id=?1",
                [binding],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert!(ended.is_some(), "{binding} ended");
        assert_eq!(liveness, "disconnected", "{binding}");
        assert_eq!(reason.as_deref(), Some("seat_retired"), "{binding}");
    }
    assert!(open_bindings(&db, &closed.seat).is_empty());
    wait_until("memberships to retire", Duration::from_secs(60), || {
        scalar(
            &s.db(),
            "SELECT state FROM memberships WHERE seat_id=?1",
            &closed.seat,
        ) == "retired"
    });
    // The seat record and every cursor row persist for undo and herdr-graph.
    assert_eq!(cursor_rows(&db, &closed.seat), cursors_before);
    // The pane that stayed is untouched.
    assert_eq!(
        scalar(&db, "SELECT state FROM seats WHERE id=?1", &kept.seat),
        "resolved"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT CAST(archived AS TEXT) FROM threads WHERE id=?1",
            &thread
        ),
        "0",
        "{thread}"
    );
}

/// Kills: a Herdr restart (a new incarnation) retiring or archiving instead of holding: seats
/// go unresolved under restore holds, every binding ends as `seat_unresolved`, memberships stay.
#[test]
fn isolated_herdr_restart_keeps_seats_in_restore_hold() {
    let Some(s) = Session::new("isolated_herdr_restart_keeps_seats_in_restore_hold") else {
        return;
    };
    let other_pane = s.first_pane();
    let pane = s.split_pane(&other_pane);
    let other = s.seat(&other_pane);
    let (held, thread, bindings) = seat_with_two_bindings(&s, &pane, &other);
    s.herdr.restart();
    wait_until("the seat to go unresolved", Duration::from_secs(60), || {
        scalar(&s.db(), "SELECT state FROM seats WHERE id=?1", &held.seat) == "unresolved"
    });
    let db = s.db();
    for binding in &bindings {
        let reason: Option<String> = db
            .query_row(
                "SELECT disconnect_reason FROM occupant_bindings WHERE binding_id=?1",
                [binding],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reason.as_deref(), Some("seat_unresolved"), "{binding}");
    }
    assert_eq!(
        scalar(
            &db,
            "SELECT state FROM memberships WHERE seat_id=?1",
            &held.seat
        ),
        "joined"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "no retirement"
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT CAST(archived AS TEXT) FROM threads WHERE id=?1",
            &thread
        ),
        "0",
        "no archival"
    );
    // The restore hold (C2): the new incarnation left the instance with an unclaimed baseline
    // hold, and no seat of it is retired.
    let unclaimed: i64 = db
        .query_row("SELECT baseline_hold_unclaimed FROM herdr_hosts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(unclaimed, 1, "{}", s.dump_table("herdr_hosts"));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM seats WHERE state='retired'",
            [],
            |r| { r.get::<_, i64>(0) }
        )
        .unwrap(),
        0
    );
}
