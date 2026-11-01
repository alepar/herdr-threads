//! Named seat allocation (N3-alloc): a seat-acting resolve under a name allocates once and flags
//! the creation (M8); a read never allocates.
use super::*;
use crate::{
    ports::HostKind,
    protocol::{
        commands::{Command, ResolveSeat, SeatsQuery},
        ids::{HostLocator, HostTargetId, NAMED_HOST_LOCATOR, OperationId, SeatId, SeatName},
        output::OutputSpec,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
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

/// A store with the named host attached; returns its host id.
fn fixture() -> (StoreContext, Connection, HostId) {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-named-alloc-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(FixedClock)).with_test_host();
    let mut conn = context.open_writer().unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let (host, _) =
        crate::store::hosts::attach_host(&mut conn, &locator, 0, HostKind::Named).unwrap();
    (context, conn, host)
}

fn request(name: &str, operation: &str) -> ResolveSeat {
    ResolveSeat {
        target: HostTargetId::new(name),
        host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
        operation: OperationId::new(operation),
        locator_hint: None,
    }
}

fn resolve(
    context: &StoreContext,
    conn: &mut Connection,
    host: HostId,
    name: &str,
    operation: &str,
) -> Result<(SeatId, bool), crate::protocol::results::ApiError> {
    match resolve_named_seat(
        context,
        conn,
        INSTANCE,
        host,
        &request(name, operation),
        &budget(),
    )? {
        CommandResult::NamedSeatResolved { seat, created, .. } => Ok((seat, created)),
        other => panic!("unexpected result {other:?}"),
    }
}

fn named_rows(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT count(*) FROM seats WHERE name=?1", [name], |r| {
        r.get(0)
    })
    .unwrap()
}

// Kills: allocating on every resolve (a second row, or created=true twice), or a second resolve
// of the same name binding another seat.
#[test]
fn first_seat_acting_resolve_allocates_once() {
    let (context, mut conn, host) = fixture();
    let (first, created) = resolve(&context, &mut conn, host, "reviewer", "op-1").unwrap();
    assert!(created, "the first resolve allocates");
    let (again, created_again) = resolve(&context, &mut conn, host, "reviewer", "op-2").unwrap();
    assert_eq!(again, first, "the same name binds the same seat");
    assert!(!created_again, "only the allocating call is flagged");
    assert_eq!(named_rows(&conn, "reviewer"), 1);
    let (state, target, row_host, is_local): (String, String, i64, i64) = conn
        .query_row(
            "SELECT state,target_id,host_id,is_local FROM seats WHERE id=?1",
            [first.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), target.as_str(), row_host, is_local),
        ("resolved", "reviewer", host.get(), 1)
    );
}

// Kills: a retried call (same operation key) allocating a second seat or losing its result.
#[test]
fn a_replayed_operation_returns_the_original_result() {
    let (context, mut conn, host) = fixture();
    let first = resolve(&context, &mut conn, host, "ci-bot-1", "op-1").unwrap();
    let replay = resolve(&context, &mut conn, host, "ci-bot-1", "op-1").unwrap();
    assert_eq!(first, replay);
    assert_eq!(named_rows(&conn, "ci-bot-1"), 1);
}

// Kills: two names sharing one seat, or one name per host splitting across rows.
#[test]
fn distinct_names_get_distinct_seats() {
    let (context, mut conn, host) = fixture();
    let (a, _) = resolve(&context, &mut conn, host, "a", "op-a").unwrap();
    let (b, _) = resolve(&context, &mut conn, host, "b", "op-b").unwrap();
    assert_ne!(a, b);
}

// Kills: splitting an unresolved named seat on the next resolve of its name (the unique index
// covers unresolved rows), and reviving a retired one's identity.
#[test]
fn an_unresolved_seat_is_reused_and_a_retired_name_is_free() {
    let (context, mut conn, host) = fixture();
    let (seat, _) = resolve(&context, &mut conn, host, "w", "op-1").unwrap();
    conn.execute(
        "UPDATE seats SET state='unresolved' WHERE id=?1",
        [seat.as_str()],
    )
    .unwrap();
    let (same, created) = resolve(&context, &mut conn, host, "w", "op-2").unwrap();
    assert_eq!((same.clone(), created), (seat.clone(), false));
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1,retired_seq=1 WHERE id=?1",
        [seat.as_str()],
    )
    .unwrap();
    let (fresh, created) = resolve(&context, &mut conn, host, "w", "op-3").unwrap();
    assert!(created);
    assert_ne!(fresh, seat);
}

// Kills: accepting a target that is not a seat name, and allocating a named seat on a Herdr host.
#[test]
fn only_a_valid_name_on_the_named_host_allocates() {
    let (context, mut conn, host) = fixture();
    let too_long = "n".repeat(65);
    for bad in ["-x", ".hidden", too_long.as_str()] {
        let error = resolve(&context, &mut conn, host, bad, "op-bad").unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{bad}");
    }
    let herdr = HostId::from_row(context.attach_test_host());
    let error = resolve(&context, &mut conn, herdr, "ok", "op-herdr").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0, "a refused resolve leaves no row");
}

// Kills: a read path (seat list, a named site's seat lookup) allocating a seat for an unknown
// name. Reads reach the store through `query`, never `resolve_named_seat`.
#[test]
fn reads_create_no_seat() {
    use crate::ports::LocalService;
    use crate::service::{dispatch::DomainService, fair_writer::FairWriter};
    use crate::store::{SqliteStore, StoreSettings};
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-named-read-{}.db",
        uuid::Uuid::new_v4()
    ));
    let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::new());
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(path.clone(), clock.clone()).with_test_host(),
            INSTANCE,
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let domain = DomainService::new(INSTANCE.into(), store, clock.clone())
        .with_operator_owner(501)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(8)));
    let call_budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 5000),
        cancellation: Cancellation::default(),
    };
    let seats_by = |target: Option<&str>, host: Option<HostLocator>| {
        Command::Seats(SeatsQuery {
            page: Default::default(),
            target: target.map(HostTargetId::new),
            include_retired: false,
            filter: Default::default(),
            host,
        })
    };
    for command in [
        seats_by(None, None),
        seats_by(Some("ghost"), HostLocator::parse(NAMED_HOST_LOCATOR).ok()),
        seats_by(Some("ghost"), None),
    ] {
        let _ = domain.handle(
            command,
            crate::protocol::authority::PeerIdentity::from_kernel(501),
            &call_budget(),
        );
    }
    let db = Connection::open(&path).unwrap();
    assert_eq!(named_rows(&db, "ghost"), 0);
    let all: i64 = db
        .query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(all, 0, "no read created a seat");
    drop(domain);
    let _ = std::fs::remove_file(path);
}

// M8 (31 bytes + fields): printed once, by the call that allocated the seat.
#[test]
fn m8_golden_prints_once_on_creation() {
    let seat = SeatId::new("seat-Xy98Zw76");
    let name = SeatName::parse("reviewer").unwrap();
    let render = |created| {
        crate::cli::human::render(
            &CommandResult::NamedSeatResolved {
                seat: seat.clone(),
                name: name.clone(),
                created,
                second_locator: None,
            },
            &OutputSpec::default(),
        )
        .unwrap()
    };
    assert_eq!(render(true), include_str!("../cli/golden/m8.txt"));
    assert_eq!(
        "herdr-threads: created seat  ()".len(),
        31,
        "the fixed bytes of M8"
    );
    let second = render(false);
    assert!(
        !second.contains("created seat"),
        "a second resolve of the name prints no M8: {second}"
    );
}
