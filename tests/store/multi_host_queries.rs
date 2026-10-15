//! Seat-row host joins (ht-5fr.2.11, spec §5 "Query rule"): a query that reads a
//! host column for a seat takes it from the seat's own `host_id`, never from the
//! primary host. One store, two `herdr_hosts` rows whose columns differ, one seat
//! on each.
//!
//! This is an in-crate module (mounted from `bindings.rs`) rather than a
//! `tests/combined.rs` module: `eligible_delivery_recipient` is `pub(crate)`.
use super::tests::{db, key, placement};
use super::*;
use crate::protocol::time::UtcMillis;
use crate::store::connection::upsert_host;
use rusqlite::Connection;

const NODE: &str = "34750f98bd59fcfc946da45aaabe933b";

/// A store with the fixture host as `h1` (the primary) and a second host `h2`.
fn two_hosts() -> (Connection, i64, i64) {
    let db = db();
    let h1: i64 = db
        .query_row("SELECT min(host_id) FROM herdr_hosts", [], |r| r.get(0))
        .unwrap();
    let h2 = upsert_host(&db, "second-host", 0).unwrap();
    assert_ne!(h1, h2);
    (db, h1, h2)
}

fn add_seat(db: &Connection, id: &str, host: i64, target: &str) {
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
         VALUES (?1,?2,'resolved','native',?3,1,1,0,(SELECT id FROM local_node),1,?4)",
        rusqlite::params![id, NODE, target, host],
    )
    .unwrap();
}

#[test]
fn eligible_delivery_recipient_uses_seat_host() {
    let (db, h1, h2) = two_hosts();
    // h1 (primary) and h2 are in different boots; the one fresh observation of
    // each target is from h1's boot and epoch, so only a seat whose own host is
    // in that boot is eligible.
    db.execute(
        "UPDATE herdr_hosts SET host_boot='boot-1',host_epoch=1 WHERE host_id=?1",
        [h1],
    )
    .unwrap();
    db.execute(
        "UPDATE herdr_hosts SET host_boot='boot-2',host_epoch=2 WHERE host_id=?1",
        [h2],
    )
    .unwrap();
    add_seat(&db, "seat-h1", h1, "p1");
    add_seat(&db, "seat-h2", h2, "p2");
    // Both rows carry h1's boot and epoch; the row of h2 therefore disagrees with
    // h2's own boot.
    for (host, target) in [(h1, "p1"), (h2, "p2")] {
        db.execute(
            "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance) \
             VALUES (?1,?2,'boot-1',1,1,1,0,'fresh')",
            rusqlite::params![host, target],
        )
        .unwrap();
    }
    crate::store::seats::eligible_delivery_recipient(&db, NODE, &SeatId::new("seat-h1"))
        .expect("the seat on h1 matches h1's boot and epoch");
    // The primary host's boot would accept this seat; its own host's boot-2 does not.
    let err = crate::store::seats::eligible_delivery_recipient(&db, NODE, &SeatId::new("seat-h2"))
        .expect_err("the seat on h2 is judged against h2's boot, not the primary's");
    assert_eq!(
        err.code,
        crate::protocol::results::ErrorCode::TargetUnresolved
    );
}

#[test]
fn liveness_candidates_use_seat_host() {
    let (db, h1, h2) = two_hosts();
    // Only h2 holds its baseline unclaimed: its seat is skipped, h1's is not.
    db.execute(
        "UPDATE herdr_hosts SET baseline_hold_unclaimed=1 WHERE host_id=?1",
        [h2],
    )
    .unwrap();
    add_seat(&db, "seat-h1", h1, "p1");
    add_seat(&db, "seat-h2", h2, "p2");
    for seat in ["seat-h1", "seat-h2"] {
        let (mut k, p) = (key(seat, None), placement());
        k.seat = SeatId::new(seat);
        let request = EnrollRequest {
            key: &k,
            role: BindingRole::TopLevel,
            parent: None,
            lineage: None,
            fingerprint: None,
            placement: &p,
            trigger: EnrollTrigger::Lifecycle,
        };
        enroll_with_hooks(&db, &request, UtcMillis(5), &[]).unwrap();
    }
    let seats: Vec<String> = liveness_candidates(&db, None)
        .unwrap()
        .into_iter()
        .map(|c| c.binding.seat.as_str().to_owned())
        .collect();
    assert_eq!(
        seats,
        vec!["seat-h1".to_owned()],
        "h2's unclaimed baseline hold skips only h2's seat"
    );
}
