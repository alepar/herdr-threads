//! Named continuity (N3-c1): the C1 "named continuity" path, the cross-kind rebind refusal, the
//! `Closed` guard and the named host's absence from `wait:host`.
use super::*;
use crate::{
    ports::HostKind,
    protocol::{
        commands::ResolveSeat,
        ids::{HostLocator, HostTargetId, NAMED_HOST_LOCATOR, OperationId, SeatId},
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    sync::facts::{
        payload::SeatKind,
        seat::{RetireReason, SeatCreatedPayload},
    },
};
use rusqlite::Connection;
use std::sync::Arc;

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

fn at() -> UtcMillis {
    UtcMillis(100)
}

/// A store with the named host and a Herdr-kind pane host attached.
fn fixture() -> (StoreContext, Connection, HostId, HostId) {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-named-cont-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(FixedClock)).with_test_host();
    let mut conn = context.open_writer().unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let (named, _) =
        crate::store::hosts::attach_host(&mut conn, &locator, 0, HostKind::Named).unwrap();
    let pane = HostId::from_row(context.attach_test_host());
    (context, conn, named, pane)
}

fn resolve(
    context: &StoreContext,
    conn: &mut Connection,
    host: HostId,
    name: &str,
    operation: &str,
) -> (SeatId, bool) {
    let request = ResolveSeat {
        target: HostTargetId::new(name),
        host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
        operation: OperationId::new(operation),
        locator_hint: None,
    };
    match resolve_named_seat(context, conn, INSTANCE, host, &request, &budget()).unwrap() {
        CommandResult::NamedSeatResolved { seat, created, .. } => (seat, created),
        other => panic!("unexpected result {other:?}"),
    }
}

fn unresolve(conn: &Connection, seat: &SeatId) {
    conn.execute(
        "UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id=?1",
        [seat.as_str()],
    )
    .unwrap();
}

/// An unresolved pane seat on `host`.
fn pane_seat(conn: &Connection, id: &str, host: HostId) -> SeatId {
    conn.execute(
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,generation,created_at,home_node,is_local,host_id) \
         VALUES (?1,?2,'unresolved','other','native',1,1,?2,1,?3)",
        rusqlite::params![id, INSTANCE, host.get()],
    )
    .unwrap();
    SeatId::new(id)
}

fn state(conn: &Connection, seat: &SeatId) -> String {
    conn.query_row(
        "SELECT state FROM seats WHERE id=?1",
        [seat.as_str()],
        |r| r.get(0),
    )
    .unwrap()
}

fn rebind(
    conn: &mut Connection,
    seat: &SeatId,
    host: HostId,
    target: &str,
) -> Result<(), crate::protocol::results::ApiError> {
    let tx = conn.transaction().unwrap();
    let result = rebind_unresolved_seat(&tx, INSTANCE, host, seat, target, 1, at());
    if result.is_ok() {
        tx.commit().unwrap();
    }
    result
}

fn named_decisions(conn: &Connection, seat: &SeatId) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM allocation_decisions WHERE seat_id=?1 AND kind='cooperative_continuity' AND operator_label='cooperative_named_seat'",
        [seat.as_str()],
        |r| r.get(0),
    )
    .unwrap()
}

// Kills: splitting the role (a new seat for an unresolved name), leaving the seat unresolved,
// and reattaching without the `cooperative_named_seat` record.
#[test]
fn unresolved_named_seat_reattached_by_name() {
    let (context, mut conn, named, _) = fixture();
    let (seat, _) = resolve(&context, &mut conn, named, "w", "op-1");
    assert_eq!(
        named_decisions(&conn, &seat),
        0,
        "allocation is not continuity"
    );
    let generation = |conn: &Connection| -> i64 {
        conn.query_row(
            "SELECT generation FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .unwrap()
    };
    let before = generation(&conn);
    unresolve(&conn, &seat);
    let (again, created) = resolve(&context, &mut conn, named, "w", "op-2");
    assert_eq!((again, created), (seat.clone(), false));
    assert_eq!(state(&conn, &seat), "resolved");
    let reason: Option<String> = conn
        .query_row(
            "SELECT unresolved_reason FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reason, None);
    assert_eq!(
        generation(&conn),
        before + 1,
        "a reattachment is a new binding generation"
    );
    assert_eq!(named_decisions(&conn, &seat), 1);
    // A resolved seat is just bound again: no second record.
    resolve(&context, &mut conn, named, "w", "op-3");
    assert_eq!(named_decisions(&conn, &seat), 1);
}

// Kills: retire not ending continuity (a retired seat reattached, or its name never reusable).
#[test]
fn retired_name_mints_new_seat() {
    let (context, mut conn, named, _) = fixture();
    let (old, _) = resolve(&context, &mut conn, named, "w", "op-1");
    unresolve(&conn, &old);
    conn.execute(
        "UPDATE seats SET state='retired',unresolved_reason=NULL,retired_at=1,retired_seq=1 WHERE id=?1",
        [old.as_str()],
    )
    .unwrap();
    let (fresh, created) = resolve(&context, &mut conn, named, "w", "op-2");
    assert!(created);
    assert_ne!(fresh, old);
    assert_eq!(state(&conn, &old), "retired");
    assert_eq!(state(&conn, &fresh), "resolved");
    assert_eq!(
        named_decisions(&conn, &old) + named_decisions(&conn, &fresh),
        0
    );
}

// Kills: a named seat rebinding onto a pane or a terminal host (and the converse).
#[test]
fn named_never_rebinds_onto_pane_or_terminal() {
    let (context, mut conn, named, pane) = fixture();
    let terminal_locator = HostLocator::parse("/run/hc-test/core.sock").unwrap();
    let (terminal, _) =
        crate::store::hosts::attach_host(&mut conn, &terminal_locator, 0, HostKind::Hc).unwrap();
    let (seat, _) = resolve(&context, &mut conn, named, "w", "op-1");
    unresolve(&conn, &seat);
    for host in [pane, terminal] {
        let error = rebind(&mut conn, &seat, host, "pane-9").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(
            state(&conn, &seat),
            "unresolved",
            "a refused rebind moves nothing"
        );
    }
    let (host, target): (i64, String) = conn
        .query_row(
            "SELECT host_id,target_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((host, target.as_str()), (named.get(), "w"));
    // The allowed direction still works: onto its own host.
    rebind(&mut conn, &seat, named, "w").unwrap();
    assert_eq!(state(&conn, &seat), "resolved");
}

#[test]
fn pane_never_rebinds_onto_named_host() {
    let (_, mut conn, named, pane) = fixture();
    let seat = pane_seat(&conn, "seat-pane0001", pane);
    let error = rebind(&mut conn, &seat, named, "w").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(state(&conn, &seat), "unresolved");
    let host: i64 = conn
        .query_row(
            "SELECT host_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(host, pane.get());
    // Pane to pane (the operator repair and C1 case) is untouched.
    rebind(&mut conn, &seat, pane, "pane-2").unwrap();
    assert_eq!(state(&conn, &seat), "resolved");
}

// Kills: rebinding through a host whose kind this build cannot construct, on either side.
#[test]
fn rebind_skips_kind_outside_implemented_set() {
    let (_, mut conn, _, pane) = fixture();
    let other_locator = HostLocator::parse("/run/hc-test/other.sock").unwrap();
    let (other, _) =
        crate::store::hosts::attach_host(&mut conn, &other_locator, 0, HostKind::Hc).unwrap();
    conn.execute(
        "UPDATE herdr_hosts SET kind='future' WHERE host_id=?1",
        [other.get()],
    )
    .unwrap();
    let seat = pane_seat(&conn, "seat-pane0002", pane);
    let error = rebind(&mut conn, &seat, other, "t1").unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::HostUnavailable,
        "unknown destination kind"
    );
    conn.execute(
        "UPDATE herdr_hosts SET kind='future' WHERE host_id=?1",
        [pane.get()],
    )
    .unwrap();
    let error = rebind(&mut conn, &seat, pane, "t1").unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::HostUnavailable,
        "unknown source kind"
    );
    assert_eq!(state(&conn, &seat), "unresolved");
}

// Kills: a `Closed` retirement of a named seat (the host never closes by absence).
#[test]
fn closed_retire_refused_for_named_seat() {
    let (context, mut conn, named, pane) = fixture();
    let (seat, _) = resolve(&context, &mut conn, named, "w", "op-1");
    let named_closed = {
        let tx = conn.transaction().unwrap();
        let result = crate::store::control::begin_retirement_authored(
            &tx,
            &context,
            crate::store::connection::DecisionInstant {
                utc: at(),
                monotonic: MonoInstant(100),
            },
            RetireReason::Closed,
            seat.clone(),
            INSTANCE,
            "named-host:node=x",
            1,
            "w",
            1,
        );
        drop(tx);
        result
    };
    let error = named_closed.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(
        error.detail.contains("never retired as closed"),
        "{error:?}"
    );
    assert_eq!(state(&conn, &seat), "resolved");
    let jobs: i64 = conn
        .query_row("SELECT count(*) FROM retirements", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jobs, 0, "the guard writes nothing");
    // The guard is the named host's alone: a pane seat reaches the fence (which refuses this
    // synthetic proof for its own reason, not the guard's).
    let pane_seat = pane_seat(&conn, "seat-pane0003", pane);
    let tx = conn.transaction().unwrap();
    let other = crate::store::control::begin_retirement_authored(
        &tx,
        &context,
        crate::store::connection::DecisionInstant {
            utc: at(),
            monotonic: MonoInstant(100),
        },
        RetireReason::Closed,
        pane_seat,
        INSTANCE,
        "boot",
        1,
        "pane-x",
        1,
    );
    if let Err(error) = other {
        assert!(
            !error.detail.contains("never retired as closed"),
            "{error:?}"
        );
    }
}

// Kills: an own-seat restore landing on the named host when it is the only host row, and the
// restore choosing the named host over a Herdr host.
#[test]
fn restore_never_lands_on_the_named_host() {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-named-restore-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(FixedClock));
    let mut conn = context.open_writer().unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    crate::store::hosts::attach_host(&mut conn, &locator, 5, HostKind::Named).unwrap();
    let payload = SeatCreatedPayload {
        seat: SeatId::new("seat-old00001"),
        kind: SeatKind::Agent,
        display_name: None,
    };
    let restore = |conn: &mut Connection| {
        let tx = conn.transaction().unwrap();
        let input = crate::store::materialize::local_only(
            &tx,
            crate::store::connection::DecisionInstant {
                utc: at(),
                monotonic: MonoInstant(100),
            },
        )
        .unwrap();
        let result = materialize_seat_created(&tx, &input, &payload);
        drop(input);
        if result.is_ok() {
            tx.commit().unwrap();
        }
        result
    };
    let error = restore(&mut conn).unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::HostUnavailable,
        "only the named host: no host"
    );
    let herdr = HostLocator::parse("/run/herdr-test/herdr.sock").unwrap();
    // Attached earlier than the named host, so the greatest `last_attached_at` is the named one.
    let (herdr_id, _) =
        crate::store::hosts::attach_host(&mut conn, &herdr, 1, HostKind::Herdr).unwrap();
    restore(&mut conn).unwrap();
    let host: i64 = conn
        .query_row(
            "SELECT host_id FROM seats WHERE id='seat-old00001'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(host, herdr_id.get());
}
