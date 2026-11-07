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
        seat::{RetireReason, SeatCreated, SeatCreatedPayload, SeatRetiredPayload},
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
            continuity_key: None,
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

    /// A fresh seat id of this node.
    fn fresh_seat() -> SeatId {
        let uuid = crate::test_support::test_node_seed().uuid;
        SeatId::new(generate_public_id(prefix::SEAT, &tag_for(uuid, 0)))
    }

    /// Replays an own-log `seat.created` through its materializer (`local_origin: true`).
    fn replay_created(
        &mut self,
        seat: &SeatId,
        display_name: Option<&str>,
    ) -> Result<(), crate::protocol::results::ApiError> {
        let payload = SeatCreatedPayload {
            seat: seat.clone(),
            kind: SeatKind::Agent,
            display_name: display_name.map(str::to_owned),
        };
        self.context.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, at, ()| {
                let m = crate::store::materialize::local_only(tx, at)?;
                crate::store::seats::materialize_seat_created(tx, &m, &payload)
            },
        )
    }

    /// Replays an own-log `seat.retired` through its materializer (`local_origin: true`).
    fn replay_retired(&mut self, seat: &SeatId) -> Result<(), crate::protocol::results::ApiError> {
        let payload = SeatRetiredPayload {
            seat: seat.clone(),
            reason: RetireReason::Operator,
            retired_at: self.clock.0.load(Ordering::SeqCst),
        };
        self.context.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, at, ()| {
                let m = crate::store::materialize::local_only(tx, at)?;
                crate::store::seats::materialize_seat_retired(tx, &m, &payload)
            },
        )
    }

    fn retirement_rows(&self) -> i64 {
        self.conn
            .query_row("SELECT count(*) FROM retirements", [], |r| r.get(0))
            .unwrap()
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

// Kills: a parked named restore released by any non-named host row (the old `PARKED_READY`), which
// re-parks it on every pass, and one never released by the `kind='named'` row it waits on.
#[test]
fn named_restore_parked_for_the_named_row_releases_on_it() {
    let mut f = fixture();
    let seat = f.restore(Some("w")).unwrap();
    // The node lost its named row and the seat: park the seat's own `seat.created` on the host.
    f.conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    f.conn
        .execute("DELETE FROM seats WHERE id=?1", [seat.as_str()])
        .unwrap();
    f.conn
        .execute(
            "DELETE FROM seat_archival WHERE seat_id=?1",
            [seat.as_str()],
        )
        .unwrap();
    f.conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    f.conn
        .execute("UPDATE herdr_hosts SET kind='hc' WHERE kind='named'", [])
        .unwrap();
    // Applied is terminal by trigger; this test builds the parked state a wipe-and-pull would.
    f.conn
        .execute_batch("DROP TRIGGER journal_applied_terminal")
        .unwrap();
    let parked = f
        .conn
        .execute(
            "UPDATE journal SET apply_state='parked_dep',parked_on='wait:host',applied_seq=NULL \
             WHERE kind='seat.created'",
            [],
        )
        .unwrap();
    assert_eq!(parked, 1);
    let replay = |f: &mut Fx| {
        let context = &f.context;
        context
            .execute_decision(
                &mut f.conn,
                |_| Ok(()),
                |tx, at, ()| {
                    crate::store::apply::replay(
                        tx,
                        context,
                        at,
                        crate::store::apply::ReplayScope::Startup,
                        crate::store::apply::ApplyLimits::default(),
                    )
                },
            )
            .unwrap()
    };
    let state = |f: &Fx| -> String {
        f.conn
            .query_row(
                "SELECT apply_state FROM journal WHERE kind='seat.created'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };

    // The fixture's Herdr row and a non-named row alone do not release a named park.
    let progress = replay(&mut f);
    assert_eq!((progress.replayed, progress.parked), (0, 0), "{progress:?}");
    assert_eq!(state(&f), "parked_dep");

    // A Herdr row and a second kind: still no named row.
    let progress = replay(&mut f);
    assert_eq!((progress.replayed, progress.parked), (0, 0), "{progress:?}");

    let named = f.named;
    f.conn
        .execute(
            "UPDATE herdr_hosts SET kind='named' WHERE host_id=?1",
            [named.get()],
        )
        .unwrap();
    let progress = replay(&mut f);
    let error: Option<String> = f
        .conn
        .query_row(
            "SELECT apply_error FROM journal WHERE kind='seat.created'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(progress.applied, 1, "{progress:?} {error:?}");
    assert_eq!(state(&f), "applied");
    let row = f.row(&seat);
    assert_eq!(
        (row.0.as_str(), row.1, row.2.as_deref()),
        ("unresolved", named.get(), Some("w"))
    );
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

// Kills: a local-origin `seat.retired` replay that returns at once, so an own retired named seat
// restored after a wipe stays `unresolved` and keeps the name its successor needs.
#[test]
fn own_log_retire_keeps_the_retired_seat_retired() {
    let mut f = fixture();
    let a = Fx::fresh_seat();
    let b = Fx::fresh_seat();
    f.replay_created(&a, Some("w")).unwrap();
    f.replay_retired(&a).unwrap();
    f.replay_created(&b, Some("w")).unwrap();
    assert_eq!(f.row(&a).0, "retired");
    assert_eq!(f.row(&b).2.as_deref(), Some("w"), "B holds the name");
    assert_eq!(f.row(&b).0, "unresolved");
    let (resolved, created) = f.resolve("w", "op-1");
    assert_eq!((resolved.as_str(), created), (b.as_str(), false));
    assert_eq!(f.row(&a).0, "retired");
}

// Kills: a replayed own retirement that leaves a held row of the name nameless.
#[test]
fn replayed_retire_hands_the_name_to_a_held_row() {
    let mut f = fixture();
    let (a, _) = f.resolve("w", "op-1");
    let b = f.restore(Some("w")).unwrap();
    assert_eq!(f.row(&b).2, None, "B restored held");
    f.replay_retired(&a).unwrap();
    assert_eq!(f.row(&a).0, "retired");
    assert_eq!(f.row(&b).2.as_deref(), Some("w"), "B takes the name");
}

// Kills: a replayed own `seat.retired` that retires a restored row with a bare UPDATE: a restored
// `unresolved` row can open a binding before its fact lands, and the row must not end `retired`
// with that binding open, its catch-up active, its offers kept or its wake work armed.
#[test]
fn replayed_retire_runs_the_fence_on_an_open_binding() {
    let mut f = fixture();
    let seat = f.restore(Some("w")).unwrap();
    let id = seat.as_str();
    f.conn
        .execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) \
             VALUES (?1,1,1,'w','b',1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-w','inc')",
            [id],
        )
        .unwrap();
    let binding: String = f
        .conn
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t',?1,'topic','goal',0,0,0)",
            [&f.instance],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,state) VALUES (?1,?2,'t',0,0,'active')",
            [&binding, &id.to_owned()],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO warning_offer(binding_id,seat_id,offered_through_seq) VALUES (?1,?2,1)",
            [&binding, &id.to_owned()],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO in_turn_offer(binding_id,seat_id,item_id) VALUES (?1,?2,'i')",
            [&binding, &id.to_owned()],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO wake_work(seat_id,reason_bits) VALUES (?1,3)",
            [id],
        )
        .unwrap();
    let generation = |f: &Fx| -> i64 {
        f.conn
            .query_row("SELECT generation FROM seats WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let lifecycle = |f: &Fx| -> i64 {
        f.conn
            .query_row(
                "SELECT lifecycle_revision FROM herdr_hosts WHERE host_id=?1",
                [f.named.get()],
                |r| r.get(0),
            )
            .unwrap()
    };
    let (generation_before, lifecycle_before) = (generation(&f), lifecycle(&f));

    f.replay_retired(&seat).unwrap();

    assert_eq!(f.row(&seat).0, "retired");
    let ended: (Option<i64>, Option<String>) = f
        .conn
        .query_row(
            "SELECT ended_at,disconnect_reason FROM occupant_bindings WHERE binding_id=?1",
            [&binding],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(ended.0.is_some(), "the open binding ends");
    assert_eq!(ended.1.as_deref(), Some("seat_retired"));
    let (state, reason): (String, Option<String>) = f
        .conn
        .query_row(
            "SELECT state,end_reason FROM catch_up WHERE binding_id=?1",
            [&binding],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), reason.as_deref()),
        ("ended", Some("superseded"))
    );
    for table in ["warning_offer", "in_turn_offer"] {
        let rows: i64 = f
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} rows are deleted");
    }
    let bits: i64 = f
        .conn
        .query_row(
            "SELECT reason_bits FROM wake_work WHERE seat_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bits, 0, "wake work resets");
    assert_eq!(generation(&f), generation_before + 1);
    assert_eq!(lifecycle(&f), lifecycle_before + 1);
}

// Kills: a transition that runs again for a row the live retire already retired (a second
// `retirements` row, or a moved `retired_seq`).
#[test]
fn live_node_retire_echo_changes_nothing() {
    let mut f = fixture();
    let (a, _) = f.resolve("w", "op-1");
    f.retire(&a, "retire-1");
    let before = f.retirement_rows();
    assert_eq!(before, 1);
    let seq_before: Option<i64> = f
        .conn
        .query_row(
            "SELECT retired_seq FROM seats WHERE id=?1",
            [a.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    f.replay_retired(&a).unwrap();
    assert_eq!(f.retirement_rows(), before);
    let seq_after: Option<i64> = f
        .conn
        .query_row(
            "SELECT retired_seq FROM seats WHERE id=?1",
            [a.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seq_after, seq_before);
}

// Kills: a local-origin retirement of a seat that never had a row here turning into an error.
#[test]
fn local_retire_of_an_unknown_seat_is_ok() {
    let mut f = fixture();
    let ghost = Fx::fresh_seat();
    f.replay_retired(&ghost).unwrap();
    assert_eq!(f.retirement_rows(), 0);
}
