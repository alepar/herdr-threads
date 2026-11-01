//! N4-hint: a named resolve records its `locator_hint` on the seat's binding and answers M9 for
//! the first binding from a second locator; `seat inspect` lists the bindings with their hint.
use super::*;
use crate::{
    ports::HostKind,
    protocol::{
        commands::{Command, ResolveSeat, SeatInspectQuery},
        ids::{HostLocator, HostTargetId, NAMED_HOST_LOCATOR, OperationId, SeatId, SeatName},
        output::OutputSpec,
        pagination::PageRequest,
        results::{CommandResult, SeatHistoryItem},
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

struct Fixture {
    path: std::path::PathBuf,
    context: StoreContext,
    conn: Connection,
    host: HostId,
}

fn fixture() -> Fixture {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-locator-hint-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
    let mut conn = context.open_writer().unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let (host, _) =
        crate::store::hosts::attach_host(&mut conn, &locator, 0, HostKind::Named).unwrap();
    Fixture {
        path,
        context,
        conn,
        host,
    }
}

impl Fixture {
    /// A named resolve from `hint`; returns the seat and M9's pair.
    fn resolve(
        &mut self,
        hint: Option<&str>,
        operation: &str,
    ) -> (SeatId, Option<(String, String)>) {
        let request = ResolveSeat {
            target: HostTargetId::new("reviewer"),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new(operation),
            locator_hint: hint.map(str::to_owned),
        };
        match resolve_named_seat(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &request,
            &budget(),
        )
        .unwrap()
        {
            CommandResult::NamedSeatResolved {
                seat,
                second_locator,
                ..
            } => (seat, second_locator),
            other => panic!("unexpected result {other:?}"),
        }
    }

    /// One open top-level binding for `session` on the seat, with no hint yet.
    fn bind(&self, seat: &SeatId, session: &str) {
        self.conn
            .execute(
                "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) \
                 VALUES (?1,1,'reviewer','b',1,'codex',?2,?3,'cooperative_top_level',0,'term','inc')",
                rusqlite::params![seat.as_str(), session, uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
    }

    fn end_bindings(&self) {
        self.conn
            .execute("UPDATE occupant_bindings SET ended_at=5", [])
            .unwrap();
    }

    fn hints(&self) -> Vec<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT locator_hint FROM occupant_bindings ORDER BY ordinal")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

fn pair(a: &str, b: &str) -> Option<(String, String)> {
    Some((a.to_owned(), b.to_owned()))
}

// Kills: no M9 on the first binding from a second locator, M9 on the first locator's own binding,
// and a hint recorded on no row.
#[test]
fn m9_on_first_binding_from_second_locator() {
    let mut f = fixture();
    let (seat, _) = f.resolve(None, "op-0");
    f.bind(&seat, "s1");
    assert_eq!(
        f.resolve(Some("w1:p1"), "op-1").1,
        None,
        "the first locator is not a second"
    );
    f.bind(&seat, "s2");
    assert_eq!(
        f.resolve(Some("w2:p9"), "op-2").1,
        pair("w1:p1", "w2:p9"),
        "the binding from the second locator fires M9"
    );
    assert_eq!(
        f.hints(),
        vec![Some("w1:p1".to_owned()), Some("w2:p9".to_owned())]
    );
    let rendered = crate::cli::human::render(
        &CommandResult::NamedSeatResolved {
            seat: seat.clone(),
            name: SeatName::parse("reviewer").unwrap(),
            created: false,
            second_locator: pair("w1:p1", "w2:p9"),
        },
        &OutputSpec::default(),
    )
    .unwrap();
    assert!(
        rendered.ends_with(include_str!("../cli/golden/m9.txt")),
        "{rendered}"
    );
    assert_eq!(
        "herdr-threads: seat  now bound from  and ".len(),
        41,
        "the fixed bytes of M9"
    );
}

// Kills: M9 on every resolve from either locator, and a repeat re-recording over an old row.
#[test]
fn no_m9_on_repeat_from_either_locator() {
    let mut f = fixture();
    let (seat, _) = f.resolve(None, "op-0");
    f.bind(&seat, "s1");
    f.resolve(Some("w1:p1"), "op-1");
    f.bind(&seat, "s2");
    assert!(f.resolve(Some("w2:p9"), "op-2").1.is_some());
    for (i, hint) in ["w1:p1", "w2:p9", "w1:p1", "w2:p9"].into_iter().enumerate() {
        assert_eq!(f.resolve(Some(hint), &format!("rep-{i}")).1, None, "{hint}");
    }
    // A restarted session from a known locator is not a second locator either, ended rows count.
    f.end_bindings();
    f.bind(&seat, "s3");
    assert_eq!(f.resolve(Some("w1:p1"), "op-3").1, None);
    assert_eq!(f.hints().len(), 3);
    assert_eq!(
        f.hints().last().cloned().flatten().as_deref(),
        Some("w1:p1"),
        "the restarted session's binding carries its locator"
    );
}

// Kills: deciding from process memory (the decision must read the persisted rows).
#[test]
fn no_m9_after_restart_repeat() {
    let mut f = fixture();
    let (seat, _) = f.resolve(None, "op-0");
    f.bind(&seat, "s1");
    f.resolve(Some("w1:p1"), "op-1");
    f.bind(&seat, "s2");
    assert!(f.resolve(Some("w2:p9"), "op-2").1.is_some());
    // A daemon restart: a fresh context and connection over the same database file.
    let context = StoreContext::new(f.path.clone(), Arc::new(FixedClock)).with_test_host();
    f.conn = context.open_writer().unwrap();
    f.context = context;
    f.bind(&seat, "s4");
    assert_eq!(
        f.resolve(Some("w2:p9"), "op-3").1,
        None,
        "the repeat after a restart is silent"
    );
    assert_eq!(f.resolve(Some("w1:p1"), "op-4").1, None);
}

// Kills: recording a hint when no binding exists (it would then mark a later binding wrongly), a
// hint on a seat's second resolve overwriting a marked binding, and a hint-less resolve marking.
#[test]
fn a_hint_waits_for_a_binding_and_never_overwrites() {
    let mut f = fixture();
    let (seat, _) = f.resolve(Some("w1:p1"), "op-0");
    assert!(f.hints().is_empty(), "no binding, nothing recorded");
    f.bind(&seat, "s1");
    assert_eq!(f.resolve(None, "op-1").1, None);
    assert_eq!(
        f.hints(),
        vec![None],
        "a resolve without a hint marks nothing"
    );
    f.resolve(Some("w1:p1"), "op-2");
    assert_eq!(
        f.resolve(Some("w2:p9"), "op-3").1,
        None,
        "no unmarked binding to take it"
    );
    assert_eq!(f.hints(), vec![Some("w1:p1".to_owned())]);
}

// Kills: `seat inspect` dropping the hint, and listing it on the wrong binding.
#[test]
fn seat_inspect_lists_bindings_by_locator() {
    let mut f = fixture();
    let (seat, _) = f.resolve(None, "op-0");
    f.bind(&seat, "s1");
    f.resolve(Some("w1:p1"), "op-1");
    f.bind(&seat, "s2");
    f.resolve(Some("w2:p9"), "op-2");
    f.bind(&seat, "s3");
    let CommandResult::SeatInspect(inspection) = crate::store::queries::query(
        &f.context,
        INSTANCE,
        &Command::SeatInspect(SeatInspectQuery {
            seat,
            page: PageRequest {
                cursor: None,
                limit: 19,
                max_bytes: 65_536,
            },
        }),
        &budget(),
    )
    .unwrap() else {
        panic!("wrong result")
    };
    let listed: Vec<(String, Option<String>)> = inspection
        .history
        .items
        .iter()
        .filter_map(|item| match item {
            SeatHistoryItem::Binding(b) => {
                Some((b.native_session.as_str().to_owned(), b.locator_hint.clone()))
            }
            SeatHistoryItem::Repair(_) => None,
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            ("s1".to_owned(), Some("w1:p1".to_owned())),
            ("s2".to_owned(), Some("w2:p9".to_owned())),
            ("s3".to_owned(), None),
        ]
    );
}
