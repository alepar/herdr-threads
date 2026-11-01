//! Named-seat restore (N3-restore, TRUST-POLICY D12, spike S5 named half): an own `seat.created`
//! with a `display_name` pulled back after a wipe restores unresolved on the named host; on a name
//! collision it is inserted nameless (held), and the retire transaction of the seat that owns the
//! name hands the name to the oldest held row. The restore runs through the real authoring path
//! (`journal::author::<SeatCreated>` for a seat with no row), so the held name is read back from the
//! row's own journal fact.
use super::*;
use crate::{
    node::tag_for,
    ports::{HostId, HostKind, OperatorRequest},
    protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::{Command, OperatorRetire, ResolveSeat, SeatInspectQuery},
        ids::{
            HostLocator, HostTargetId, NAMED_HOST_LOCATOR, OperationId, SeatId, generate_public_id,
            prefix,
        },
        pagination::PageRequest,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::seats::{mutate_operator, resolve_named_seat},
    sync::facts::{
        payload::SeatKind,
        seat::{SeatCreated, SeatCreatedPayload},
    },
};
use rusqlite::Connection;
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst) as u64)
    }
}

struct Fx {
    context: StoreContext,
    conn: Connection,
    clock: Arc<TestClock>,
    instance: String,
    named: HostId,
    next_seat: u32,
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Cancellation::default(),
    }
}

/// A journal-open store with the named host and the fixture Herdr host attached.
fn fixture() -> Fx {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-named-restore-{}.db",
        uuid::Uuid::new_v4()
    ));
    let clock = Arc::new(TestClock(AtomicI64::new(1_790_000_000_000)));
    let context = StoreContext::new(path, clock.clone()).with_test_host();
    let mut conn = context.open_writer().unwrap();
    crate::store::journal::open(
        &context,
        &mut conn,
        Arc::new(crate::node::NodeIdentity::from_seed(
            crate::test_support::TEST_NODE_SEED,
        )),
        "test",
        crate::sync::facts::FactRegistry::builtins(),
    )
    .unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let (named, _) =
        crate::store::hosts::attach_host(&mut conn, &locator, 0, HostKind::Named).unwrap();
    let instance = crate::test_support::test_node_seed().uuid.to_string();
    Fx {
        context,
        conn,
        clock,
        instance,
        named,
        next_seat: 0,
    }
}

impl Fx {
    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    /// `seat.created` for a fresh seat id of this node with no row: the local-origin restore.
    fn restore(
        &mut self,
        display_name: Option<&str>,
    ) -> Result<SeatId, crate::protocol::results::ApiError> {
        self.next_seat += 1;
        let uuid = crate::test_support::test_node_seed().uuid;
        let seat = SeatId::new(generate_public_id(prefix::SEAT, &tag_for(uuid, 0)));
        let payload = SeatCreatedPayload {
            seat: seat.clone(),
            kind: SeatKind::Agent,
            display_name: display_name.map(str::to_owned),
        };
        self.context.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, at, ()| {
                crate::store::journal::author::<SeatCreated>(tx, &self.context, at, &payload)
                    .map(|_| ())
            },
        )?;
        Ok(seat)
    }

    fn resolve(&mut self, name: &str, operation: &str) -> (SeatId, bool) {
        let request = ResolveSeat {
            target: HostTargetId::new(name),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new(operation),
            locator_hint: None,
            tmux_hint: None,
        };
        match resolve_named_seat(
            &self.context,
            &mut self.conn,
            &self.instance,
            self.named,
            &request,
            &budget(),
        )
        .unwrap()
        {
            CommandResult::NamedSeatResolved { seat, created, .. } => (seat, created),
            other => panic!("unexpected result {other:?}"),
        }
    }

    fn unresolve(&self, seat: &SeatId) {
        self.conn
            .execute(
                "UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id=?1",
                [seat.as_str()],
            )
            .unwrap();
    }

    fn retire(&mut self, seat: &SeatId, operation: &str) {
        let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
        mutate_operator(
            &self.context,
            &mut self.conn,
            &self.instance,
            OperatorRequest::Retire(OperatorRetire {
                seat: seat.clone(),
                operation: OperationId::new(operation),
            }),
            actor,
            None,
        )
        .unwrap();
    }

    /// `(state, host_id, name, target_id, published)` of `seat`.
    fn row(&self, seat: &SeatId) -> (String, i64, Option<String>, Option<String>, bool) {
        self.conn
            .query_row(
                "SELECT state,host_id,name,target_id,published FROM seats WHERE id=?1",
                [seat.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
    }

    fn inspect_hold(&self, seat: &SeatId) -> Option<String> {
        let CommandResult::SeatInspect(inspection) = crate::store::queries::query(
            &self.context,
            &self.instance,
            &Command::SeatInspect(SeatInspectQuery {
                seat: seat.clone(),
                page: PageRequest {
                    cursor: None,
                    limit: 5,
                    max_bytes: 65_536,
                },
            }),
            &budget(),
        )
        .unwrap() else {
            panic!("wrong result")
        };
        inspection.hold.map(|hold| hold.reason_data)
    }
}

// Kills: restoring a named seat onto a pane host, leaving it resolved, dropping its name or
// target, or never publishing it; and a restored name that named continuity cannot reattach.
#[test]
fn display_name_restores_unresolved_on_named_host() {
    let mut f = fixture();
    let seat = f.restore(Some("w")).unwrap();
    assert_eq!(
        f.row(&seat),
        (
            "unresolved".to_owned(),
            f.named.get(),
            Some("w".to_owned()),
            Some("w".to_owned()),
            true
        )
    );
    assert_eq!(held_name(&f.conn, seat.as_str()).unwrap(), None);
    assert_eq!(f.inspect_hold(&seat), None);
    let (again, created) = f.resolve("w", "op-1");
    assert_eq!((again.as_str(), created), (seat.as_str(), false));
    assert_eq!(f.row(&seat).0, "resolved");
}

// Kills: a restore that violates `seats_live_name` (the authoring would fail), one that steals
// the name from the live seat, one that merges the rows, and a missing held-name display.
#[test]
fn collision_with_resolved_inserts_nameless() {
    let mut f = fixture();
    let (existing, _) = f.resolve("w", "op-1");
    let held = f.restore(Some("w")).unwrap();
    assert_eq!(
        f.row(&held),
        (
            "unresolved".to_owned(),
            f.named.get(),
            None,
            Some("w".to_owned()),
            true
        )
    );
    assert_eq!(f.row(&existing).2.as_deref(), Some("w"));
    assert_eq!(f.row(&existing).0, "resolved");
    let (resolved, created) = f.resolve("w", "op-2");
    assert_eq!((resolved.as_str(), created), (existing.as_str(), false));
    assert_eq!(
        f.row(&held).2,
        None,
        "named continuity never picks the held row"
    );
    assert_eq!(f.row(&held).0, "unresolved");
    assert_eq!(
        held_name(&f.conn, held.as_str()).unwrap().as_deref(),
        Some("w")
    );
    assert_eq!(held_name(&f.conn, existing.as_str()).unwrap(), None);
    assert_eq!(f.inspect_hold(&held).as_deref(), Some("held_name:w"));
    assert_eq!(f.inspect_hold(&existing), None);
    let listed = held_names(&f.conn).unwrap();
    assert_eq!(
        listed,
        vec![HeldName {
            seat: held,
            name: "w".to_owned(),
            holder: Some(existing)
        }]
    );
}

// Kills: treating only a resolved seat as a collision (the unique index also covers unresolved).
#[test]
fn collision_with_unresolved_inserts_nameless() {
    let mut f = fixture();
    let (existing, _) = f.resolve("w", "op-1");
    f.unresolve(&existing);
    let held = f.restore(Some("w")).unwrap();
    assert_eq!(f.row(&held).2, None);
    assert_eq!(f.row(&held).0, "unresolved");
    assert_eq!(f.row(&existing).2.as_deref(), Some("w"));
    assert_eq!(f.inspect_hold(&held).as_deref(), Some("held_name:w"));
    let (resolved, created) = f.resolve("w", "op-2");
    assert_eq!((resolved.as_str(), created), (existing.as_str(), false));
    assert_eq!(
        f.row(&existing).0,
        "resolved",
        "the existing seat reattached"
    );
    assert_eq!(f.row(&held).2, None);
    assert_eq!(f.row(&held).0, "unresolved");
}

// Kills: no hand-back, a hand-back to the newest row, one that gives two rows the name, and a
// name that the next resolve does not reattach.
#[test]
fn operator_retire_hands_name_to_oldest_held_row() {
    let mut f = fixture();
    let (existing, _) = f.resolve("w", "op-1");
    f.advance(1_000);
    let oldest = f.restore(Some("w")).unwrap();
    f.advance(1_000);
    let newer = f.restore(Some("w")).unwrap();
    let other = f.restore(Some("v")).unwrap();
    assert_eq!(
        f.row(&other).2.as_deref(),
        Some("v"),
        "another name is no collision"
    );
    f.retire(&existing, "retire-1");
    assert_eq!(f.row(&existing).0, "retired");
    assert_eq!(
        f.row(&oldest).2.as_deref(),
        Some("w"),
        "the oldest held row takes the name"
    );
    assert_eq!(f.row(&newer).2, None, "the newer one keeps waiting");
    assert_eq!(f.row(&other).2.as_deref(), Some("v"));
    let (resolved, created) = f.resolve("w", "op-2");
    assert_eq!((resolved.as_str(), created), (oldest.as_str(), false));
    assert_eq!(f.row(&oldest).0, "resolved");
    assert_eq!(f.inspect_hold(&newer).as_deref(), Some("held_name:w"));
    assert_eq!(
        held_names(&f.conn).unwrap(),
        vec![HeldName {
            seat: newer,
            name: "w".to_owned(),
            holder: Some(oldest)
        }]
    );
}

// Kills: a retirement of the held row that clears or moves the existing seat's name.
#[test]
fn retiring_held_row_leaves_existing_alone() {
    let mut f = fixture();
    let (existing, _) = f.resolve("w", "op-1");
    let held = f.restore(Some("w")).unwrap();
    f.retire(&held, "retire-1");
    assert_eq!(f.row(&held).0, "retired");
    assert_eq!(f.row(&held).2, None);
    let after = f.row(&existing);
    assert_eq!(
        (after.0.as_str(), after.2.as_deref()),
        ("resolved", Some("w"))
    );
    assert!(held_names(&f.conn).unwrap().is_empty());
    let (resolved, created) = f.resolve("w", "op-2");
    assert_eq!((resolved.as_str(), created), (existing.as_str(), false));
}

// Kills: placing an unnamed restore on the latest host of a mixed-kind node (the old
// `ORDER BY last_attached_at`), and refusing it on a single-kind node.
#[test]
fn unnamed_rows_placed_only_on_single_kind_nodes() {
    let mut f = fixture();
    let herdr = crate::test_support::test_host_id();
    let seat = f.restore(None).unwrap();
    let row = f.row(&seat);
    assert_eq!(
        (row.0.as_str(), row.1, row.2),
        ("unresolved", herdr.get(), None)
    );
    let hc = HostLocator::parse("/hc.sock").unwrap();
    crate::store::hosts::attach_host(&mut f.conn, &hc, 5_000, HostKind::Hc).unwrap();
    let held = f.restore(None).unwrap_err();
    assert_eq!(held.code, ErrorCode::HostUnavailable, "{held:?}");
    assert!(held.detail.contains("more than one kind"), "{held:?}");
    let rows: i64 = f
        .conn
        .query_row(
            "SELECT count(*) FROM seats WHERE state='unresolved'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "the mixed-node restore inserted nothing");
}

// Kills (S5 two-kind half, D12): an unnamed restore placed on either host of a node that has a
// Herdr TUI and a herdr-core row, whichever attached last (the old `ORDER BY last_attached_at`),
// and a held restore that leaves a seat row behind on either host.
#[test]
fn s5_unnamed_restored_row_held_unresolved_on_two_kind_node() {
    for hc_first in [true, false] {
        let mut f = fixture();
        let herdr = crate::test_support::test_host_id();
        let hc = HostLocator::parse("/hc/core.sock").unwrap();
        // The fixture's Herdr row is host 1; a second Herdr-kind row would not change the kinds.
        let (hc_id, _) = crate::store::hosts::attach_host(
            &mut f.conn,
            &hc,
            if hc_first { 9_000 } else { 1_000 },
            HostKind::Hc,
        )
        .unwrap();
        let held = f.restore(None).unwrap_err();
        assert_eq!(held.code, ErrorCode::HostUnavailable, "{held:?}");
        assert!(held.detail.contains("more than one kind"), "{held:?}");
        for host in [herdr, hc_id] {
            let seats: i64 = f
                .conn
                .query_row(
                    "SELECT count(*) FROM seats WHERE host_id=?1 AND instance_id=(SELECT id FROM local_node) \
                     AND state<>'retired'",
                    [host.get()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                seats,
                0,
                "nothing placed on host {} (hc_first={hc_first})",
                host.get()
            );
        }
    }
}
