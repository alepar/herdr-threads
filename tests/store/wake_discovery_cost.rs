//! Wake discovery cost (ht-p03.12.5, spec D2): per-pass SQLite VM units depend
//! on the live non-human seats, never on settled history.
use super::*;
use crate::store::schema;
use crate::test_support::isolation::{CostCounter, count_vm_units};

#[derive(Clone, Copy)]
struct History {
    retired_seats: u64,
    settled_invitations: u64,
    settled_receipts: u64,
    settled_warnings: u64,
    wake_work: u64,
}

const BASE: History = History {
    retired_seats: 20,
    settled_invitations: 30,
    settled_receipts: 30,
    settled_warnings: 30,
    wake_work: 30,
};

impl History {
    fn grown(self, axis: &str) -> History {
        let mut h = self;
        match axis {
            "retired seats" => h.retired_seats *= 10,
            "settled invitations" => h.settled_invitations *= 10,
            "settled receipts" => h.settled_receipts *= 10,
            "settled warnings" => h.settled_warnings *= 10,
            "settled wake_work" => h.wake_work *= 10,
            "all" => {
                h.retired_seats *= 10;
                h.settled_invitations *= 10;
                h.settled_receipts *= 10;
                h.settled_warnings *= 10;
                h.wake_work *= 10;
            }
            other => panic!("unknown axis {other}"),
        }
        h
    }
}

const AXES: [&str; 6] = [
    "retired seats",
    "settled invitations",
    "settled receipts",
    "settled warnings",
    "settled wake_work",
    "all",
];

/// `live` live resolved seats (`l1` holds the one pending invitation, the rest
/// are idle) plus `history` of settled rows around them.
fn fixture(live: u64, history: History) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    // A CTE lives for one statement, so every statement gets its own.
    let each = |n: u64, statements: &[&str]| {
        for statement in statements {
            db.execute_batch(&format!(
                "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{n}) {statement}"
            ))
            .unwrap();
        }
    };
    db.execute_batch(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1000000);\
         INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);",
    )
    .unwrap();
    // Retired seats take the low ordinals, so only the live partial index can skip them.
    each(
        history.retired_seats,
        &[
            "INSERT INTO seats(id,instance_id,state,role,generation,target_generation,created_at,retired_at,retired_seq) SELECT 'r'||x,'i','retired','native',1,1,0,1,1 FROM n",
        ],
    );
    each(
        live,
        &[
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) SELECT 'l'||x,'i','resolved','native','p'||x,1,1,0 FROM n",
        ],
    );
    db.execute_batch(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-l1','t','l1',1,'pending',0,1,100,100);",
    )
    .unwrap();
    each(
        history.settled_invitations,
        &[
            "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,accepted_at,accepted_actor_seat_id,accepted_generation,accepted_observation) SELECT 'si'||x,'t','l1',x+1,'accepted',0,x+1,100,100,1,'l1',1,'obs' FROM n",
        ],
    );
    each(
        history.settled_receipts,
        &[
            "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) SELECT 'sm'||x,'i','t',x,'ordinary','b',x+1,0 FROM n",
            "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,acked_at,ack_actor_seat_id,ack_generation,ack_observation) SELECT 'sm'||x,'t','l1','acked',100,1,'l1',1,'obs' FROM n",
        ],
    );
    let offset = history.settled_receipts;
    each(
        history.settled_warnings,
        &[
            &format!(
                "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) SELECT 'sw'||x,'i','t',x+{offset},'warn','{{}}',0,x+1,x FROM n"
            ),
            "INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'sw'||x,x+1,'t',100,'l1','invitation','si'||x FROM n",
            "INSERT INTO warning_recipients(warning_id,seat_id,generation) SELECT 'sw'||x,'l1',1 FROM n",
        ],
    );
    // Settled: jobs complete, and their conditions closed (the production close
    // path deletes the open-warning projection rows and their recipients).
    db.execute_batch(
        "UPDATE warning_jobs SET status='complete',phase='complete';\
         DELETE FROM digest_open_warnings WHERE warning_id LIKE 'sw%';",
    )
    .unwrap();
    // Settled wake history: unreserved rows on retired seats and on idle live seats.
    each(
        history.retired_seats.min(history.wake_work),
        &[
            "INSERT INTO wake_work(seat_id,retry_step,last_outcome) SELECT 'r'||x,2,'submitted' FROM n",
        ],
    );
    each(
        live.saturating_sub(1).min(history.wake_work),
        &[
            "INSERT INTO wake_work(seat_id,retry_step,last_outcome) SELECT 'l'||(x+1),1,'submitted' FROM n",
        ],
    );
    db
}

/// One discovery pass over the fixture: (units of 10 VM instructions, candidates).
fn pass(db: &Connection) -> (u64, usize) {
    let high: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE instance_id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let counter = CostCounter::default();
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let found = count_vm_units(db, &counter, || {
        discover_wake_candidates(db, "i", 0, high as u64, 100, &|| Ok(())).unwrap()
    });
    db.execute_batch("COMMIT").unwrap();
    assert!(!found.items.is_empty(), "the pending seat must be found");
    (counter.units(), found.items.len())
}

// Kills: any discovery step that grows with settled history: a seat walk that
// visits retired seats, a probe on a non-pending index, a wake_work probe, or
// an attention rebuild from full history. Each history axis grows 10x with the
// live seat set fixed.
#[test]
fn wake_discovery_is_flat_in_settled_history() {
    const LIVE: u64 = 40;
    const C: u64 = 30;
    let (base, found) = pass(&fixture(LIVE, BASE));
    assert_eq!(found, 1);
    for axis in AXES {
        let (grown, found) = pass(&fixture(LIVE, BASE.grown(axis)));
        assert_eq!(found, 1, "{axis}");
        assert!(
            grown <= base + base / 10 + C,
            "{axis}: {base} units grew to {grown} at 10x history"
        );
    }
}

// Kills: per-seat work that is superlinear in live seats, or a per-live-seat
// constant that depends on history. c is fitted from two sizes, checked on a
// third, and holds at 10x history too.
#[test]
fn wake_discovery_scales_with_live_seats_only() {
    let units = |live, history| pass(&fixture(live, history)).0;
    let (u50, u100) = (units(50, BASE), units(100, BASE));
    let c = (u100.saturating_sub(u50)).div_ceil(50).max(1);
    let constant = u50.saturating_sub(c * 50);
    let bound = |live: u64| c * live + constant + (c * live + constant) / 10;
    assert!(u100 <= bound(100), "fit: u50={u50} u100={u100} c={c}");
    let u200 = units(200, BASE);
    assert!(
        u200 <= bound(200),
        "third size: u200={u200} exceeds {} (c={c})",
        bound(200)
    );
    let grown = units(100, BASE.grown("all"));
    assert!(
        grown <= bound(100),
        "10x history at 100 live: {grown} exceeds {}",
        bound(100)
    );
}

// Kills: any of the seven access paths losing its INDEXED BY pin (a silent fall
// back to a scan) or the pinned index no longer serving the query.
#[test]
fn every_access_path_prepares_indexed_by() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    let plan = |sql: &str| -> String {
        let mut stmt = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let n = stmt.parameter_count();
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(std::iter::repeat_n(rusqlite::types::Null, n)),
                |r| r.get::<_, String>(3),
            )
            .unwrap();
        rows.collect::<Result<Vec<_>, _>>().unwrap().join("\n")
    };
    let mut paths: Vec<(&str, &str)> = attention::WAKE_PROBES
        .iter()
        .map(|p| (p.index, p.sql))
        .collect();
    paths.push(("seats_live_ordinal", WAKE_SEAT_WALK));
    paths.push(("occupant_bindings_current", WAKE_SEAT_WALK));
    assert_eq!(
        paths.len(),
        8,
        "six pending probes, the live seat walk and the human exclusion"
    );
    for (index, sql) in paths {
        assert!(
            sql.contains(&format!("INDEXED BY {index}")),
            "{index} is not pinned in: {sql}"
        );
        db.prepare(sql)
            .unwrap_or_else(|e| panic!("{index} does not prepare: {e}"));
        let detail = plan(sql);
        assert!(detail.contains(index), "{index} unused by plan:\n{detail}");
    }
}
