//! `seat.created`, seat publication at enrollment and `FactRoute`
//! (fact-kinds design sections 2 and 4; ht-5fr.4.5).
//!
//! The helpers at the top (`Node`, kinds standing in for the held ones) are
//! shared with `fact_convergence_seat`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{Connection, Transaction};
use serde::{Deserialize, Serialize};

use herdr_threads::node::{NodeIdentity, NodeUuid, tag_for};
use herdr_threads::protocol::ids::{NativeSessionId, SeatId, ThreadId, generate_public_id, prefix};
use herdr_threads::protocol::results::{ApiError, ErrorCode};
use herdr_threads::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use herdr_threads::store::bindings::{
    self, BindingKey, BindingPlacement, BindingRole, EnrollRequest, EnrollTrigger,
};
use herdr_threads::store::connection::{
    DecisionInstant, HoldSource, OWN_HISTORY_HOLD_MAX, StoreContext,
};
use herdr_threads::store::journal;
use herdr_threads::store::materialize::{FactRoute, HELD_KINDS};
use herdr_threads::store::{SqliteStore, StoreSettings, seats};
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::membership::{MembershipJoined, MembershipLeft, MembershipPayload};
use herdr_threads::sync::facts::payload::{ReferencedSeats, SeatKind};
use herdr_threads::sync::facts::seat::{
    RetireReason, SeatCreated, SeatCreatedPayload, SeatRetired, SeatRetiredPayload,
};
use herdr_threads::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry};
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::isolation::TestIsolation;
use herdr_threads::test_support::journal_oracle::{
    CONVERTED_TABLES, TableSpec, replicated_projection,
};
use herdr_threads::test_support::write_guard;

// ---- shared fixtures --------------------------------------------------------

/// A clock tests move by hand (monotonic ms).
pub struct TestClock(AtomicU64);

impl TestClock {
    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

pub fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// One journal-open store with a hand-moved clock and (optionally) an attached
/// Herdr host.
pub struct Node {
    pub store: SqliteStore,
    pub id: NodeUuid,
    pub clock: Arc<TestClock>,
    _isolation: TestIsolation,
}

impl Node {
    /// A store of node `seed`. `incarnation_seed` fixes its incarnation draw, so
    /// two stores of one node (a wipe and restore) get different logs.
    pub fn open(
        seed: u8,
        incarnation_seed: u64,
        registry: &'static FactRegistry,
        host: bool,
    ) -> Self {
        let isolation = TestIsolation::new("facts-seat");
        let clock = Arc::new(TestClock(AtomicU64::new(1)));
        let ctx = StoreContext::new(isolation.path("store.db"), clock.clone())
            .with_journal_seed([seed; 32]);
        ctx.seed_incarnations(incarnation_seed);
        let identity = Arc::new(NodeIdentity::from_seed([seed; 32]));
        let id = identity.uuid();
        let mut writer = ctx.open_writer().expect("open the writer");
        journal::open(&ctx, &mut writer, identity, "facts-seat", registry).expect("open journal");
        drop(writer);
        if host {
            ctx.attach_host("test-host").expect("attach a host");
        }
        let store = SqliteStore::new(ctx, id.to_string(), StoreSettings::default())
            .expect("open the store");
        Self {
            store,
            id,
            clock,
            _isolation: isolation,
        }
    }

    /// The usual node: a host, the shipped kinds plus `extra`.
    pub fn with(seed: u8, registry: &'static FactRegistry) -> Self {
        Self::open(seed, u64::from(seed), registry, true)
    }

    pub fn ctx(&self) -> &StoreContext {
        self.store.context()
    }

    /// Runs `f` as one command (one write transaction, the write guard armed on
    /// the writer).
    pub fn run<R>(
        &self,
        f: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let mut conn = self.ctx().open_writer().expect("open a writer");
        let _guard = write_guard::arm(&conn);
        self.ctx()
            .execute_decision(&mut conn, |_| Ok(()), |tx, d, ()| f(tx, d))
    }

    /// A raw writer for fixtures (unguarded).
    pub fn raw(&self) -> Connection {
        self.ctx().open_writer().expect("open a writer")
    }

    pub fn query<T>(
        &self,
        sql: &str,
        map: impl Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let db = self
            .ctx()
            .open_query(budget())
            .expect("open a query connection");
        let mut stmt = db.prepare(sql).expect("prepare");
        stmt.query_map([], map)
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    /// A seat ID minted under this node's tag `k`.
    pub fn mint_seat(&self, k: u32) -> SeatId {
        SeatId::new(generate_public_id(prefix::SEAT, &tag_for(self.id, k)))
    }

    /// A local resolved seat row (what allocation by observation leaves).
    pub fn add_local_seat(&self, seat: &SeatId) {
        self.raw()
            .execute(
                "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
                 VALUES (?1,(SELECT id FROM local_node),'resolved','native','p-'||?1,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
                [seat.as_str()],
            )
            .expect("insert a local seat");
    }

    /// A published remote seat row homed on another node.
    pub fn add_remote_seat(&self, seat: &SeatId) {
        self.raw()
            .execute(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) \
                 VALUES (?1,(SELECT id FROM local_node),'remote','native',0,0,?2,0,'agent',1)",
                rusqlite::params![seat.as_str(), "ab".repeat(16)],
            )
            .expect("insert a remote seat");
    }

    /// Enrolls one top-level binding of `harness` for `seat` in one command.
    pub fn enroll(&self, seat: &SeatId, harness: &str, session: &str) -> Result<(), ApiError> {
        self.run(|tx, _| enroll_in(tx, seat, harness, session))
    }

    /// The same enrollment on a bare transaction, outside the write runner
    /// that publishes: the binding exists and the seat is still unpublished.
    pub fn enroll_unpublished(&self, seat: &SeatId, harness: &str, session: &str) {
        let mut conn = self.raw();
        let tx = conn.transaction().unwrap();
        enroll_in(&tx, seat, harness, session).unwrap();
        tx.commit().unwrap();
    }

    /// `(kind, apply_state, apply_error)` of every journal entry, oldest first.
    pub fn journal(&self) -> Vec<(String, String, Option<String>)> {
        self.query(
            "SELECT kind,apply_state,apply_error FROM journal ORDER BY ingest_seq",
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }

    pub fn journal_kinds(&self) -> Vec<String> {
        self.journal().into_iter().map(|(k, _, _)| k).collect()
    }

    pub fn count_kind(&self, kind: &str) -> usize {
        self.journal().iter().filter(|(k, _, _)| k == kind).count()
    }

    /// `(state, is_local, remote_kind, published, created_at, home_node)`.
    #[allow(clippy::type_complexity)]
    pub fn seat(&self, seat: &SeatId) -> Option<(String, bool, Option<String>, bool, i64, String)> {
        let sql = format!(
            "SELECT state,is_local,remote_kind,published,created_at,home_node FROM seats WHERE id='{}'",
            seat.as_str()
        );
        self.query(&sql, |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .into_iter()
        .next()
    }

    /// The `seat.created` payloads in journal order.
    pub fn seat_created_payloads(&self) -> Vec<SeatCreatedPayload> {
        self.query(
            "SELECT payload FROM journal WHERE kind='seat.created' ORDER BY ingest_seq",
            |r| r.get::<_, Vec<u8>>(0),
        )
        .into_iter()
        .map(|b| serde_json::from_slice(&b).expect("a seat.created payload"))
        .collect()
    }

    pub fn seats_projection(&self) -> String {
        let specs: Vec<TableSpec> = CONVERTED_TABLES
            .iter()
            .filter(|t| t.name == "seats")
            .copied()
            .collect();
        format!("{:?}", replicated_projection(&self.store, self.id, &specs))
    }

    /// Publishes `seat` the way a command's backstop would.
    pub fn publish(&self, seat: &SeatId) -> Result<bool, ApiError> {
        self.run(|tx, d| seats::ensure_published(tx, self.ctx(), d, seat))
    }
}

fn enroll_in(tx: &Connection, seat: &SeatId, harness: &str, session: &str) -> Result<(), ApiError> {
    let key = BindingKey {
        seat: seat.clone(),
        harness: harness.into(),
        native_session: NativeSessionId::new(session),
        agent_id: None,
    };
    let place = BindingPlacement {
        generation: 1,
        target_generation: 1,
        target: "p".into(),
        terminal: "term".into(),
        incarnation: "inc".into(),
        host_boot: "b".into(),
        host_epoch: 1,
        execution: format!("e-{session}"),
        provenance: "cooperative_top_level".into(),
        observed_at: UtcMillis(5),
        registered_at: Some(UtcMillis(5)),
    };
    bindings::enroll(
        tx,
        &EnrollRequest {
            key: &key,
            role: BindingRole::TopLevel,
            parent: None,
            lineage: None,
            fingerprint: None,
            placement: &place,
            trigger: EnrollTrigger::Lifecycle,
        },
        UtcMillis(5),
    )
    .map(|_| ())
}

// ---- test kinds -------------------------------------------------------------

/// The payload of every test kind: the seats it names.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Refs {
    pub seats: Vec<SeatId>,
}

impl ReferencedSeats for Refs {
    fn referenced_seats(&self) -> Vec<SeatId> {
        self.seats.clone()
    }
}

macro_rules! test_kind {
    ($ty:ident, $name:literal) => {
        pub struct $ty;
        impl FactKind for $ty {
            const NAME: &'static str = $name;
            const MAJOR: u32 = 1;
            type Payload = Refs;
            fn creates(_: &Refs) -> Option<EntityKey> {
                None
            }
            fn needs(_: &Refs) -> Vec<EntityKey> {
                Vec::new()
            }
            fn apply(
                _: &Transaction<'_>,
                _: &mut ApplyContext<'_>,
                _: Refs,
            ) -> Result<(), FactError> {
                Ok(())
            }
        }
    };
}

test_kind!(Settled, "receipt.settled");
test_kind!(Answered, "invitation.answered");
test_kind!(Free, "test.free");

pub fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<Settled>().with::<Answered>().with::<Free>())
}

fn refs(seats: &[&SeatId]) -> Refs {
    Refs {
        seats: seats.iter().map(|s| (*s).clone()).collect(),
    }
}

/// Emits `K` on `route` for `payload`; returns whether the local materializer ran.
fn emit<K: FactKind<Payload = Refs>>(
    node: &Node,
    route: FactRoute,
    payload: &Refs,
) -> Result<bool, ApiError> {
    node.run(|tx, d| {
        let mut ran = false;
        route.emit::<K>(tx, node.ctx(), d, payload, None, |_, _, _| {
            ran = true;
            Ok(())
        })?;
        Ok(ran)
    })
}

/// Emits the real `seat.retired` for `seat` on the journal route.
fn emit_retired(node: &Node, seat: &SeatId) -> Result<bool, ApiError> {
    node.run(|tx, d| {
        FactRoute::Journal
            .emit::<SeatRetired>(
                tx,
                node.ctx(),
                d,
                &retired_payload(seat),
                None,
                |_, _, _| Ok(()),
            )
            .map(|authored| authored.is_some())
    })
}

pub fn retired_payload(seat: &SeatId) -> SeatRetiredPayload {
    SeatRetiredPayload {
        seat: seat.clone(),
        reason: RetireReason::Closed,
        retired_at: 1_700_000_000_000,
    }
}

// ---- seat.created -----------------------------------------------------------

#[test]
fn payload_round_trip_creates_needs() {
    let seat = SeatId::new("sAbc12345678");
    let payload = SeatCreatedPayload {
        seat: seat.clone(),
        kind: SeatKind::Human,
        display_name: None,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        json,
        r#"{"seat":"sAbc12345678","kind":"human","display_name":null}"#
    );
    assert_eq!(
        serde_json::from_str::<SeatCreatedPayload>(&json).unwrap(),
        payload
    );
    // An additive field is ignored and a missing display_name defaults to null.
    let newer: SeatCreatedPayload =
        serde_json::from_str(r#"{"seat":"sAbc12345678","kind":"agent","future":1}"#).unwrap();
    assert_eq!(newer.kind, SeatKind::Agent);
    assert_eq!(newer.display_name, None);
    assert_eq!(SeatCreated::NAME, "seat.created");
    assert_eq!(SeatCreated::MAJOR, 1);
    assert_eq!(
        SeatCreated::creates(&payload).map(|k| k.as_str().to_owned()),
        Some("sAbc12345678".to_owned())
    );
    assert!(SeatCreated::needs(&payload).is_empty());
    assert_eq!(payload.referenced_seats(), vec![seat]);
    assert!(
        FactRegistry::builtins().lookup("seat.created", 1).is_some(),
        "builtins() registers seat.created"
    );
}

#[test]
fn apply_twice_noop() {
    let node = Node::with(1, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    assert!(node.publish(&seat).unwrap());
    let after_first = node.seat(&seat);
    let projection = node.seats_projection();
    // A second authored copy applies (to the already published row) and changes nothing.
    node.run(|tx, d| {
        journal::author::<SeatCreated>(
            tx,
            node.ctx(),
            d,
            &SeatCreatedPayload {
                seat: seat.clone(),
                kind: SeatKind::Agent,
                display_name: None,
            },
        )
        .map(|_| ())
    })
    .unwrap();
    assert_eq!(node.seat(&seat), after_first);
    assert_eq!(node.seats_projection(), projection);
    assert!(
        node.journal()
            .iter()
            .all(|(_, state, _)| state == "applied")
    );
}

#[test]
fn reauthored_under_second_incarnation_noop() {
    let node = Node::with(2, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    node.publish(&seat).unwrap();
    let before = (node.seat(&seat), node.seats_projection());
    node.run(|tx, d| journal::begin_incarnation(tx, node.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    node.run(|tx, d| {
        journal::author::<SeatCreated>(
            tx,
            node.ctx(),
            d,
            &SeatCreatedPayload {
                seat: seat.clone(),
                kind: SeatKind::Agent,
                display_name: None,
            },
        )
        .map(|_| ())
    })
    .unwrap();
    assert_eq!(
        node.count_kind("seat.created"),
        2,
        "the copy is in the new log"
    );
    assert_eq!((node.seat(&seat), node.seats_projection()), before);
}

// ---- publication at enrollment ----------------------------------------------

#[test]
fn first_enrollment_authors_one_seat_created() {
    let node = Node::with(3, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    assert_eq!(node.count_kind("seat.created"), 0);
    node.enroll(&seat, "codex", "session-1").unwrap();
    assert_eq!(node.count_kind("seat.created"), 1);
    let created = node.seat_created_payloads();
    assert_eq!(created.len(), 1);
    assert_eq!(
        (created[0].seat.clone(), created[0].kind),
        (seat.clone(), SeatKind::Agent)
    );
    let row = node.seat(&seat).unwrap();
    assert!(row.3, "the enrolled seat is published");
}

#[test]
fn later_enrollment_or_rebind_authors_nothing() {
    let node = Node::with(4, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    node.enroll(&seat, "codex", "session-1").unwrap();
    node.enroll(&seat, "codex", "session-2").unwrap();
    node.enroll(&seat, "claude", "session-3").unwrap();
    // The same session again is a claim match, also nothing.
    node.enroll(&seat, "claude", "session-3").unwrap();
    assert_eq!(node.count_kind("seat.created"), 1);
}

#[test]
fn observation_allocated_unenrolled_authors_nothing() {
    let node = Node::with(5, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    // Commands that run without enrolling it author nothing for it.
    node.run(|_, _| Ok(())).unwrap();
    node.run(|tx, d| {
        FactRoute::Journal
            .emit::<Free>(tx, node.ctx(), d, &refs(&[]), None, |_, _, _| Ok(()))
            .map(|_| ())
    })
    .unwrap();
    assert_eq!(node.count_kind("seat.created"), 0);
    assert!(!node.seat(&seat).unwrap().3, "never published");
}

#[test]
fn failed_enrollment_command_publishes_nothing() {
    let node = Node::with(6, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    let failed: Result<(), ApiError> = node.run(|tx, _| {
        enroll_in(tx, &seat, "codex", "s")?;
        Err(ApiError::new(ErrorCode::InvalidRequest, "abort"))
    });
    assert!(failed.is_err());
    assert_eq!(node.count_kind("seat.created"), 0);
    // The queue did not leak into the next command.
    node.run(|_, _| Ok(())).unwrap();
    assert_eq!(node.count_kind("seat.created"), 0);
}

#[test]
fn human_bound_seat_publishes_human_once() {
    let node = Node::with(7, registry());
    let seat = node.mint_seat(0);
    node.add_local_seat(&seat);
    // Bound outside the write runner, so the seat is still unpublished.
    node.enroll_unpublished(&seat, "human", "sess");
    assert_eq!(node.count_kind("seat.created"), 0);
    assert!(node.publish(&seat).unwrap(), "the first call authors");
    assert!(!node.publish(&seat).unwrap(), "the second authors nothing");
    let created = node.seat_created_payloads();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].kind, SeatKind::Human);
}

#[test]
fn ensure_published_ignores_remote_and_unknown_seats() {
    let node = Node::with(8, registry());
    let remote = node.mint_seat(0);
    node.add_remote_seat(&remote);
    assert!(!node.publish(&remote).unwrap());
    assert!(!node.publish(&SeatId::new("sNoSuchSeat1")).unwrap());
    assert_eq!(node.count_kind("seat.created"), 0);
}

// ---- FactRoute --------------------------------------------------------------

#[test]
fn journal_route_publishes_referenced_local_seats_first() {
    let node = Node::with(9, registry());
    let (a, b, remote) = (node.mint_seat(0), node.mint_seat(0), node.mint_seat(0));
    node.add_local_seat(&a);
    node.add_local_seat(&b);
    node.add_remote_seat(&remote);
    let ran = emit::<Free>(&node, FactRoute::Journal, &refs(&[&a, &remote, &b])).unwrap();
    assert!(
        !ran,
        "the journal route does not call the local materializer"
    );
    let kinds = node.journal_kinds();
    let tail: Vec<&str> = kinds
        .iter()
        .rev()
        .take(3)
        .rev()
        .map(String::as_str)
        .collect();
    assert_eq!(tail, ["seat.created", "seat.created", "test.free"]);
    let published: Vec<SeatId> = node
        .seat_created_payloads()
        .into_iter()
        .map(|p| p.seat)
        .collect();
    assert_eq!(
        published,
        vec![a.clone(), b.clone()],
        "only the local seats, in payload order"
    );
    assert!(node.seat(&a).unwrap().3 && node.seat(&b).unwrap().3);
    // Idempotent: the same emit publishes nothing more.
    emit::<Free>(&node, FactRoute::Journal, &refs(&[&a, &b])).unwrap();
    assert_eq!(node.count_kind("seat.created"), 2);
}

#[test]
fn local_only_refuses_remote_seat() {
    let node = Node::with(10, registry());
    let (local, remote) = (node.mint_seat(0), node.mint_seat(0));
    node.add_local_seat(&local);
    node.add_remote_seat(&remote);
    let before = node.journal().len();
    let error = emit::<Free>(&node, FactRoute::LocalOnly, &refs(&[&local, &remote])).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert_eq!(error.detail, "service threads are node-local");
    assert_eq!(node.journal().len(), before, "nothing journaled");
    assert!(!node.seat(&local).unwrap().3, "nothing published");
    // Without a remote seat the local materializer runs, still without publishing.
    assert!(emit::<Free>(&node, FactRoute::LocalOnly, &refs(&[&local])).unwrap());
    assert_eq!(node.journal().len(), before);
    assert!(!node.seat(&local).unwrap().3);
}

#[test]
fn route_follows_journaled() {
    let node = Node::with(11, registry());
    node.raw()
        .execute_batch(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
               VALUES('tJournaled1',(SELECT id FROM local_node),'topic','',1,1,1); \
             INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
               VALUES('tManaged001',(SELECT id FROM local_node),'topic','',1,1,0);",
        )
        .unwrap();
    let route = |thread: &str| node.run(|tx, _| FactRoute::for_thread(tx, &ThreadId::new(thread)));
    assert_eq!(route("tJournaled1").unwrap(), FactRoute::Journal);
    assert_eq!(route("tManaged001").unwrap(), FactRoute::LocalOnly);
    assert_eq!(route("tMissing001").unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(FactRoute::from_journaled(true), FactRoute::Journal);
    assert_eq!(FactRoute::from_journaled(false), FactRoute::LocalOnly);
}

// ---- restore-window authoring hold ------------------------------------------

pub fn hold_on(node: &Node) {
    node.ctx().sync().unwrap().own_history_hold().set(
        HoldSource::Links,
        true,
        node.clock.monotonic_now(),
    );
}

pub fn hold_off(node: &Node) {
    node.ctx().sync().unwrap().own_history_hold().set(
        HoldSource::Links,
        false,
        node.clock.monotonic_now(),
    );
}

/// Emits the real membership fact `K` for `seat` in `thread` on the journal
/// route; returns whether it authored an entry.
fn emit_membership<K: FactKind<Payload = MembershipPayload>>(
    node: &Node,
    thread: &ThreadId,
    seat: &SeatId,
    n: u64,
) -> Result<bool, ApiError> {
    let payload = MembershipPayload {
        thread: thread.clone(),
        seat: seat.clone(),
        n,
        at: 1,
    };
    node.run(|tx, d| {
        FactRoute::Journal
            .emit::<K>(tx, node.ctx(), d, &payload, None, |_, _, _| Ok(()))
            .map(|authored| authored.is_some())
    })
}

/// Emits each held kind for `payload` on the journal route. The membership
/// kinds are the real ones, in `thread` (which only the success cases need to
/// exist).
fn emit_held(node: &Node, thread: &ThreadId, payload: &Refs) -> Vec<Result<bool, ApiError>> {
    let seat = &payload.seats[0];
    vec![
        emit::<Settled>(node, FactRoute::Journal, payload),
        emit_membership::<MembershipJoined>(node, thread, seat, 1),
        emit_membership::<MembershipLeft>(node, thread, seat, 2),
        emit::<Answered>(node, FactRoute::Journal, payload),
        emit_retired(node, seat),
    ]
}

#[test]
fn held_kinds_list_matches_the_spec() {
    assert_eq!(
        HELD_KINDS,
        [
            "receipt.settled",
            "membership.joined",
            "membership.left",
            "invitation.answered",
            "seat.retired"
        ]
    );
    assert_eq!(Settled::NAME, HELD_KINDS[0]);
}

#[test]
fn held_kinds_refused_while_hold_active() {
    let node = Node::with(12, registry());
    let local = node.mint_seat(0);
    node.add_local_seat(&local);
    hold_on(&node);
    let before = node.journal().len();
    for result in emit_held(&node, &ThreadId::new("tNoThread001"), &refs(&[&local])) {
        let error = result.unwrap_err();
        assert_eq!(error.code, ErrorCode::ServiceBusy, "retryable");
        assert_eq!(error.detail, "own history catching up");
    }
    assert_eq!(node.journal().len(), before, "no journal row");
    assert!(
        !node.seat(&local).unwrap().3,
        "refused before anything is written, publication included"
    );
}

#[test]
fn remote_seat_seat_created_non_held_and_local_only_unaffected() {
    let node = Node::with(13, registry());
    let (local, remote) = (node.mint_seat(0), node.mint_seat(0));
    node.add_local_seat(&local);
    node.add_remote_seat(&remote);
    hold_on(&node);
    // A held kind for a remote seat only. None is refused by the hold: the
    // membership kinds and `seat.retired` fail only on the parked dependency
    // (no fact created the thread or the fixture seat), not "own history
    // catching up".
    let mut results = emit_held(&node, &ThreadId::new("tNoThread001"), &refs(&[&remote]));
    let retired = results.pop().unwrap().unwrap_err();
    assert_eq!(retired.code, ErrorCode::Conflict);
    assert!(retired.detail.contains("not applicable"), "{retired:?}");
    let [settled, joined, left, answered]: [_; 4] = results.try_into().unwrap();
    settled.unwrap();
    answered.unwrap();
    for parked in [joined, left] {
        let error = parked.unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict, "{error:?}");
        assert!(error.detail.contains("not applicable"), "{error:?}");
    }
    // seat.created for the local seat.
    node.run(|tx, d| {
        FactRoute::Journal
            .emit::<SeatCreated>(
                tx,
                node.ctx(),
                d,
                &SeatCreatedPayload {
                    seat: local.clone(),
                    kind: SeatKind::Agent,
                    display_name: None,
                },
                None,
                |_, _, _| Ok(()),
            )
            .map(|_| ())
    })
    .unwrap();
    // A non-held kind for a local seat.
    emit::<Free>(&node, FactRoute::Journal, &refs(&[&local])).unwrap();
    // The local-only route, held kind and local seat.
    assert!(emit::<Settled>(&node, FactRoute::LocalOnly, &refs(&[&local])).unwrap());
    assert_eq!(
        node.count_kind("receipt.settled"),
        1,
        "only the remote-seat one"
    );
}

#[test]
fn held_authoring_succeeds_after_clear_or_cap() {
    let node = Node::with(14, registry());
    let local = node.mint_seat(0);
    node.add_local_seat(&local);
    hold_on(&node);
    assert!(emit::<Settled>(&node, FactRoute::Journal, &refs(&[&local])).is_err());
    hold_off(&node);
    let thread = node.create_thread(&local, "held", None).unwrap();
    for result in emit_held(&node, &thread, &refs(&[&local])) {
        result.unwrap();
    }
    // The flag stays set, the 5-minute cap passes.
    let other = node.mint_seat(0);
    node.add_local_seat(&other);
    hold_on(&node);
    assert!(emit::<Settled>(&node, FactRoute::Journal, &refs(&[&other])).is_err());
    node.clock.advance(OWN_HISTORY_HOLD_MAX);
    emit::<Settled>(&node, FactRoute::Journal, &refs(&[&other])).unwrap();
}

// ---- origin authority --------------------------------------------------------

#[test]
fn foreign_origin_seat_created_failed_no_row() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(21, reg),
        Node::with(22, reg),
        Node::with(23, reg),
    );
    // C names a seat under B's tag (C applies its own fact locally; the
    // authority check is for the nodes that receive it).
    let seat = b.mint_seat(0);
    c.run(|tx, d| {
        journal::author::<SeatCreated>(
            tx,
            c.ctx(),
            d,
            &SeatCreatedPayload {
                seat: seat.clone(),
                kind: SeatKind::Agent,
                display_name: None,
            },
        )
        .map(|_| ())
    })
    .unwrap();
    fact_exchange::sync(&c.store, &a.store);
    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, _, _)| k == "seat.created")
            .collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].1, "failed");
        let error = failed[0].2.clone().unwrap();
        assert!(
            error.contains("is not the authorized author of seat.created for")
                && error.contains(seat.as_str()),
            "{error}"
        );
        assert!(node.seat(&seat).is_none(), "no row inserted");
    }
}

#[test]
fn pre_retag_tag_seat_created_applies() {
    let reg = registry();
    let (a, b) = (Node::with(24, reg), Node::with(25, reg));
    // The seat is minted under B's tag 0, then B retags, then publishes it.
    let seat = b.mint_seat(0);
    b.add_local_seat(&seat);
    b.run(|tx, d| {
        journal::author::<herdr_threads::sync::facts::NodeRetag>(
            tx,
            b.ctx(),
            d,
            &herdr_threads::sync::facts::RetagPayload {
                k: 1,
                tag: tag_for(b.id, 1),
            },
        )
        .map(|_| ())
    })
    .unwrap();
    assert!(b.publish(&seat).unwrap());
    fact_exchange::sync(&a.store, &b.store);
    let row = a.seat(&seat).expect("A holds the seat");
    assert_eq!((row.0.as_str(), row.1, row.3), ("remote", false, true));
}

// ---- fork residual -----------------------------------------------------------

/// Documented behaviour, not convergence (root section 12 accepted limit): two
/// `seat.created` for one seat with different kinds leave a store with the one
/// it applied first.
#[test]
fn two_kinds_for_one_seat_keep_first_applied() {
    let reg = registry();
    let (a, b) = (Node::with(26, reg), Node::with(27, reg));
    let seat = a.mint_seat(0);
    a.add_local_seat(&seat);
    a.publish(&seat).unwrap(); // agent
    a.run(|tx, d| {
        journal::author::<SeatCreated>(
            tx,
            a.ctx(),
            d,
            &SeatCreatedPayload {
                seat: seat.clone(),
                kind: SeatKind::Human,
                display_name: None,
            },
        )
        .map(|_| ())
    })
    .unwrap();
    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(a.count_kind("seat.created"), 2);
    assert_eq!(
        b.seat(&seat).unwrap().2.as_deref(),
        Some("agent"),
        "B kept the first applied"
    );
}

// ---- seat.retired -----------------------------------------------------------

/// Authors a `seat.retired` copy for `seat` on `node` (what the retirement
/// fence authors, without the fence's local writes).
fn author_retired(node: &Node, seat: &SeatId) {
    node.run(|tx, d| {
        journal::author::<SeatRetired>(tx, node.ctx(), d, &retired_payload(seat)).map(|_| ())
    })
    .unwrap();
}

/// `(seat_id, origin, cutover_at, closure_boot)` of every retirement row.
#[allow(clippy::type_complexity)]
pub fn retirement_rows(node: &Node) -> Vec<(String, String, i64, Option<String>)> {
    node.query(
        "SELECT seat_id,origin,cutover_at,closure_boot FROM retirements ORDER BY ordinal",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
}

/// A seat homed on `home`, published and synced to every node in `others`.
fn published_seat(home: &Node, others: &[&Node]) -> SeatId {
    let seat = home.mint_seat(0);
    home.add_local_seat(&seat);
    home.enroll(&seat, "codex", "session-1").unwrap();
    for other in others {
        fact_exchange::sync(&home.store, &other.store);
    }
    seat
}

fn retired_failures(node: &Node) -> Vec<String> {
    node.journal()
        .into_iter()
        .filter(|(k, state, _)| k == "seat.retired" && state == "failed")
        .map(|(_, _, error)| error.unwrap_or_default())
        .collect()
}

#[test]
fn seat_retired_round_trip_needs() {
    let seat = SeatId::new("sAbc12345678");
    let payload = SeatRetiredPayload {
        seat: seat.clone(),
        reason: RetireReason::Operator,
        retired_at: 42,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        json,
        r#"{"seat":"sAbc12345678","reason":"operator","retired_at":42}"#
    );
    assert_eq!(
        serde_json::from_str::<SeatRetiredPayload>(&json).unwrap(),
        payload
    );
    let closed: SeatRetiredPayload = serde_json::from_str(
        r#"{"seat":"sAbc12345678","reason":"closed","retired_at":1,"future":1}"#,
    )
    .unwrap();
    assert_eq!(
        closed.reason,
        RetireReason::Closed,
        "additive field ignored"
    );
    assert!(
        serde_json::from_str::<SeatRetiredPayload>(
            r#"{"seat":"sAbc12345678","reason":"robot","retired_at":1}"#
        )
        .is_err(),
        "reason is a closed set"
    );
    assert_eq!((SeatRetired::NAME, SeatRetired::MAJOR), ("seat.retired", 1));
    assert_eq!(SeatRetired::creates(&payload), None);
    assert_eq!(
        SeatRetired::needs(&payload)
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect::<Vec<_>>(),
        vec!["sAbc12345678".to_owned()],
        "needs the seat's seat.created"
    );
    assert_eq!(payload.referenced_seats(), vec![seat]);
    assert!(
        FactRegistry::builtins().lookup("seat.retired", 1).is_some(),
        "builtins() registers seat.retired"
    );
    assert!(HELD_KINDS.contains(&SeatRetired::NAME));
}

#[test]
fn seat_retired_remote_row_and_replicated_job_row() {
    let reg = registry();
    let (a, b) = (Node::with(51, reg), Node::with(52, reg));
    let seat = published_seat(&b, &[&a]);
    assert_eq!(a.seat(&seat).unwrap().0, "remote");
    author_retired(&b, &seat);
    fact_exchange::sync(&a.store, &b.store);
    let row = a.seat(&seat).unwrap();
    assert_eq!((row.0.as_str(), row.1, row.3), ("retired", false, true));
    let seq: (i64, i64) = a.query(
        &format!(
            "SELECT retired_at, retired_seq FROM seats WHERE id='{}'",
            seat.as_str()
        ),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )[0];
    assert_eq!(seq.0, retired_payload(&seat).retired_at);
    let applied_seq: i64 = a.query(
        "SELECT applied_seq FROM journal WHERE kind='seat.retired'",
        |r| r.get(0),
    )[0];
    assert_eq!(seq.1, applied_seq, "retired_seq is the entry's applied_seq");
    assert_eq!(
        retirement_rows(&a),
        vec![(
            seat.as_str().to_owned(),
            "replicated".to_owned(),
            seq.0,
            None
        )]
    );
    // B's own row is untouched by its own fact (the fence retires it, not the handler).
    assert_eq!(b.seat(&seat).unwrap().0, "resolved");
    assert!(retirement_rows(&b).is_empty());
}

#[test]
fn seat_retired_apply_twice_noop() {
    let reg = registry();
    let (a, b) = (Node::with(53, reg), Node::with(54, reg));
    let seat = published_seat(&b, &[&a]);
    author_retired(&b, &seat);
    fact_exchange::sync(&a.store, &b.store);
    let after_first = (
        a.seat(&seat),
        a.seats_projection(),
        retirement_rows(&a),
        a.query("SELECT count(*) FROM messages", |r| r.get::<_, i64>(0)),
    );
    // A second copy in the same log applies to the retired row and changes nothing.
    author_retired(&b, &seat);
    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(a.count_kind("seat.retired"), 2);
    let after_second = (
        a.seat(&seat),
        a.seats_projection(),
        retirement_rows(&a),
        a.query("SELECT count(*) FROM messages", |r| r.get::<_, i64>(0)),
    );
    assert_eq!(after_second, after_first);
    assert!(a.journal().iter().all(|(_, state, _)| state == "applied"));
}

#[test]
fn seat_retired_reauthored_second_incarnation_noop() {
    let reg = registry();
    let (a, b) = (Node::with(55, reg), Node::with(56, reg));
    let seat = published_seat(&b, &[&a]);
    author_retired(&b, &seat);
    fact_exchange::sync(&a.store, &b.store);
    let before_a = (a.seat(&seat), a.seats_projection(), retirement_rows(&a));
    let before_b = (b.seat(&seat), b.seats_projection(), retirement_rows(&b));
    // B restarts as a second incarnation and re-authors the same fact.
    b.run(|tx, d| journal::begin_incarnation(tx, b.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    author_retired(&b, &seat);
    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(
        b.count_kind("seat.retired"),
        2,
        "the copy is in the new log"
    );
    assert_eq!(a.count_kind("seat.retired"), 2);
    assert_eq!(
        (a.seat(&seat), a.seats_projection(), retirement_rows(&a)),
        before_a,
        "no second retirement row on A"
    );
    assert_eq!(
        (b.seat(&seat), b.seats_projection(), retirement_rows(&b)),
        before_b
    );
    assert_eq!(
        a.query(
            "SELECT count(*) FROM messages WHERE kind<>'ordinary'",
            |r| r.get::<_, i64>(0)
        ),
        vec![0],
        "no audit event"
    );
    assert!(a.journal().iter().all(|(_, state, _)| state == "applied"));
}

#[test]
fn seat_retired_foreign_origin_failed() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(57, reg),
        Node::with(58, reg),
        Node::with(59, reg),
    );
    let seat = published_seat(&b, &[&a, &c]);
    // C holds B's seat as remote and authors a retirement for it.
    author_retired(&c, &seat);
    fact_exchange::sync(&c.store, &a.store);
    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        let failures = retired_failures(node);
        assert_eq!(failures.len(), 1, "{:?}", node.journal());
        assert!(
            failures[0].contains("is not the authorized author of seat.retired for")
                && failures[0].contains(seat.as_str()),
            "{}",
            failures[0]
        );
        assert!(retirement_rows(node).is_empty(), "retired nothing");
    }
    assert_eq!(a.seat(&seat).unwrap().0, "remote");
    assert_eq!(b.seat(&seat).unwrap().0, "resolved");
}

#[test]
fn seat_retired_home_origin_applies() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(60, reg),
        Node::with(61, reg),
        Node::with(62, reg),
    );
    let seat = published_seat(&b, &[&a, &c]);
    author_retired(&b, &seat);
    fact_exchange::sync(&b.store, &a.store);
    fact_exchange::sync(&b.store, &c.store);
    for node in [&a, &c] {
        assert_eq!(node.seat(&seat).unwrap().0, "retired");
        assert_eq!(retirement_rows(node).len(), 1);
        assert!(retired_failures(node).is_empty());
    }
    assert!(retired_failures(&b).is_empty());
}
