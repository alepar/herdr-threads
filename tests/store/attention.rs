use super::*;
use crate::store::{effective, schema};
use rusqlite::Connection;

fn no_budget() -> Result<(), ApiError> {
    Ok(())
}

fn digest(db: &Connection, seat: &str) -> DigestRun {
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let run = seat_digest(db, "i", &SeatId::new(seat), &no_budget).unwrap();
    db.execute_batch("COMMIT").unwrap();
    run
}

/// The wake scheduler's canonical scan, run to completion.
fn wake_frontier(db: &Connection, seat: &str) -> LogicalAttentionFrontier {
    let mut position = None;
    loop {
        let slice = crate::test_support::attention_oracle::scan_effective_seat_attention(
            db,
            seat,
            position.take(),
            100,
        )
        .unwrap();
        if let Some(attention) = slice.attention {
            return attention.frontier;
        }
        position = Some(slice.position);
    }
}

fn empty() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || crate::protocol::time::UtcMillis(0)).unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,40);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane-s',1,1,0);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('o','i','resolved','native','pane-o',1,1,0);\
    ").unwrap();
    db
}

/// Every attention source the canonical rule recognises, for seat `s`, plus
/// the same shapes addressed only to seat `o`.
fn every_source() -> Connection {
    let db = empty();
    db.execute_batch("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t1','i','a','g',0,0),('t2','i','b','g',0,0),('t3','i','c','g',0,0),('t4','i','d','g',0,0),('t5','i','e','g',0,0);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-s','t1','s',1,'pending',0,3,100,100);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,accepted_at,accepted_actor_seat_id,accepted_generation,accepted_observation) VALUES ('inv-old','t2','s',1,'accepted',0,2,100,100,1,'s',1,'obs');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-o','t1','o',1,'pending',0,30,100,100);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m0','i','t2',1,'ordinary','b',0,4),('m1','i','t2',2,'ordinary','b',0,5),('mo','i','t2',3,'ordinary','b',0,31);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,acked_at,ack_actor_seat_id,ack_generation,ack_observation) VALUES ('m0','t2','s','acked',100,1,'s',1,'obs');\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m1','t2','s','pending',100),('mo','t2','o','pending',100);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p','i','actor','op',zeroblob(32),'t3',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p','t3','s',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','w-unavail','s',1,1,'{}');\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m3','i','t3',1,'ordinary','b',0,10);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p','m3','i','t3',10,0,1,0,1,1);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t4','s',1,1),('t4','o',1,1);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-member','i','t4',1,'warn','{}',0,12,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w-member',12,'t4',100,'o','invitation','inv-o');\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t5','o',1,1);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-other','i','t5',1,'warn','{}',0,33,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w-other',33,'t5',100,'o','invitation','inv-o');\
    ").unwrap();
    db
}

fn ids(class: &AttentionClass) -> Vec<&str> {
    class.items.iter().map(|item| item.id.as_str()).collect()
}

// Kills: dropping any warning source (membership-interval, affected-seat
// manifest), the manifest receipt walk, the pending filter on invitations or
// receipts, or a seat filter (o's attention leaking into s's digest). The
// frontier must equal the wake scheduler's canonical scan exactly.
#[test]
fn digest_frontier_equals_the_canonical_wake_frontier() {
    let db = every_source();
    for seat in ["s", "o"] {
        let run = digest(&db, seat);
        assert_eq!(run.frontier, wake_frontier(&db, seat), "{seat}");
        assert_eq!(run.digest.validate(), Ok(()));
    }
    let run = digest(&db, "s");
    let d = &run.digest;
    assert_eq!(ids(&d.invitations), ["inv-s"]);
    assert_eq!(d.invitations.count, 1);
    assert_eq!(ids(&d.receipts), ["m3", "m1"]);
    assert_eq!(ids(&d.warnings), ["w-member", "w-unavail"]);
    assert_eq!(d.token.invitation, Some((3, 1)));
    assert_eq!(d.token.receipt, Some((10, 0)));
    assert_eq!(d.token.warning, Some((12, 1)));
    assert_eq!(d.token.unavailability_episode, 1);
    assert!(d.unavailability_open);
    let other = digest(&db, "o").digest;
    assert_eq!(ids(&other.invitations), ["inv-o"]);
    assert_eq!(ids(&other.receipts), ["mo"]);
    assert_eq!(ids(&other.warnings), ["w-other", "w-member"]);
}

/// The pre-D2 oracle, run to completion, as (attention, decision-seq position).
fn oracle_attention(db: &Connection, seat: &str) -> (effective::EffectiveSeatAttention, i64) {
    let mut position = None;
    loop {
        let slice = crate::test_support::attention_oracle::scan_effective_seat_attention(
            db,
            seat,
            position.take(),
            100,
        )
        .unwrap();
        if let Some(attention) = slice.attention {
            return (attention, slice.position.decision_seq);
        }
        position = Some(slice.position);
    }
}

fn wake_attention(db: &Connection, seat: &str) -> WakeSeatAttention {
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let wake = wake_seat_attention(db, seat).unwrap();
    db.execute_batch("COMMIT").unwrap();
    wake
}

// Kills: any source the wake walks drop or mis-judge (invitations, manifest
// and physical receipts, programmatic notices above the offer frontier,
// recipient and affected warnings, the outstanding-attribution backlog), a seat filter
// leaking another seat's rows, and a decision-seq position derived from the
// walked rows instead of host_instances.decision_seq. Field by field against
// the oracle, on fixtures below WINDOW per source.
#[test]
fn wake_seat_attention_matches_the_oracle() {
    use effective::EffectiveSeatAttention;
    let db = every_source();
    // A programmatic notice for each seat, on top of every_source's warning
    // jobs (recipients by membership interval, attribution still outstanding).
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    db.execute_batch("\
        INSERT INTO service_authors(id,instance_id,created_at) VALUES ('svc','i',0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t6','i','f','g',0,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset,author_kind,author_service_id) VALUES ('n1','i','t6',1,'warn','{}',0,20,3,'programmatic','svc'),('n2','i','t6',2,'warn','{}',0,21,4,'programmatic','svc');\
        INSERT INTO service_notification_publications(preparation_id,message_id,decision_seq,recipient_count) VALUES ('np1','n1',20,1),('np2','n2',21,1);\
        INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('n1','s',1),('n2','o',1);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t6','s',1,1),('t6','o',1,1);\
    ").unwrap();
    let mut warning_sources_seen = 0;
    for seat in ["s", "o"] {
        let (oracle, oracle_seq) = oracle_attention(&db, seat);
        let wake = wake_attention(&db, seat);
        let EffectiveSeatAttention {
            has_pending_invitation,
            has_pending_receipt,
            latest_warning_seq,
            frontier,
        } = wake.attention;
        assert_eq!(
            has_pending_invitation, oracle.has_pending_invitation,
            "{seat} invitation"
        );
        assert_eq!(
            has_pending_receipt, oracle.has_pending_receipt,
            "{seat} receipt"
        );
        assert_eq!(
            latest_warning_seq, oracle.latest_warning_seq,
            "{seat} warning seq"
        );
        assert_eq!(frontier, oracle.frontier, "{seat} frontier");
        assert_eq!(wake.decision_seq, oracle_seq, "{seat} decision seq");
        assert_eq!(wake.decision_seq, 40);
        warning_sources_seen += usize::from(latest_warning_seq.is_some());
    }
    assert_eq!(
        warning_sources_seen, 2,
        "both seats carry actionable warnings"
    );
}

// Kills: a decision position taken from the walked rows: an ACK that settles
// the only pending receipt adds no newer row, yet the position must follow
// host_instances.decision_seq.
#[test]
fn wake_seat_attention_position_is_the_host_decision_sequence() {
    let db = every_source();
    assert_eq!(wake_attention(&db, "s").decision_seq, 40);
    db.execute_batch("UPDATE receipts SET state='acked',acked_at=2,ack_actor_seat_id='s',ack_generation=1,ack_observation='obs' WHERE message_id='m1'; UPDATE host_instances SET decision_seq=41 WHERE id='i';").unwrap();
    let after = wake_attention(&db, "s");
    assert_eq!(after.decision_seq, 41);
    let (oracle, oracle_seq) = oracle_attention(&db, "s");
    assert_eq!(oracle_seq, 41);
    assert_eq!(after.attention, oracle);
}

// Kills: a missing-seat read that fabricates empty attention.
#[test]
fn wake_seat_attention_of_an_unknown_seat_is_not_found() {
    let db = empty();
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let error = wake_seat_attention(&db, "nobody").unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

// Kills: a producer that writes (it must leave receipts, ACK state, wake and
// warning-offer rows untouched).
#[test]
fn digest_is_read_only() {
    let db = every_source();
    let snapshot = |db: &Connection| -> Vec<i64> {
        [
            "SELECT count(*) FROM receipts WHERE state='pending'",
            "SELECT count(*) FROM receipt_state",
            "SELECT count(*) FROM wake_work",
            "SELECT count(*) FROM warning_offer",
            "SELECT count(*) FROM warning_recipients",
            "SELECT total_changes()",
        ]
        .iter()
        .map(|sql| db.query_row(sql, [], |r| r.get(0)).unwrap())
        .collect()
    };
    let before = snapshot(&db);
    digest(&db, "s");
    assert_eq!(snapshot(&db), before);
}

// Kills: an ACK or acceptance advancing the token (items only leaving), or a
// new arrival that does not advance it.
#[test]
fn ack_advances_nothing_and_arrival_advances() {
    let db = every_source();
    let first = digest(&db, "s").digest.token;
    db.execute_batch("UPDATE receipts SET state='acked',acked_at=2,ack_actor_seat_id='s',ack_generation=1,ack_observation='obs' WHERE message_id='m1'; UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='s',accepted_generation=1,accepted_observation='obs' WHERE id='inv-s';").unwrap();
    let after_ack = digest(&db, "s").digest;
    assert!(!after_ack.token.advanced_beyond(&first));
    assert_eq!(after_ack.invitations.count, 0);
    assert_eq!(ids(&after_ack.receipts), ["m3"]);
    db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m9','i','t2',9,'ordinary','b',0,41); INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m9','t2','s','pending',100); UPDATE host_instances SET decision_seq=41;").unwrap();
    let arrived = digest(&db, "s").digest;
    assert!(arrived.token.advanced_beyond(&first.join(&after_ack.token)));
    assert_eq!(arrived.token.receipt, Some((41, 0)));
}

// Kills: more than MAX_DIGEST_IDS items listed, an oldest-first selection, or a
// count that stops at the listed items.
#[test]
fn class_lists_the_newest_bounded_ids_with_an_exact_count() {
    let db = empty();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','a','g',0,0)", []).unwrap();
    for n in 1..=7 {
        db.execute(
            "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES (?1,'t','s',?2,'pending',0,?2,100,100)",
            rusqlite::params![format!("inv-{n}"), n],
        ).unwrap();
    }
    let d = digest(&db, "s").digest;
    assert_eq!(d.invitations.count, 7);
    assert!(d.invitations.has_more);
    assert_eq!(ids(&d.invitations), ["inv-7", "inv-6", "inv-5", "inv-4"]);
}

/// A require-ACK handoff on thread `th` (its invitation at decision seq 1,
/// its receipt at 2) followed by `burst` newer invitations on other threads.
fn burst_after_handoff(burst: i64) -> Connection {
    let db = empty();
    db.execute_batch("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('th','i','h','g',0,0);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-h','th','s',1,'pending',0,1,100,100);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m-h','i','th',1,'ordinary','b',0,2);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m-h','th','s','pending',100);\
    ").unwrap();
    for n in 1..=burst {
        db.execute(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','b','g',0,0)",
            rusqlite::params![format!("tb-{n}")],
        )
        .unwrap();
        db.execute(
            "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES (?1,?2,'s',1,'pending',0,?3,100,100)",
            rusqlite::params![format!("inv-b{n}"), format!("tb-{n}"), n + 2],
        )
        .unwrap();
    }
    db.execute(
        "UPDATE host_instances SET decision_seq=?1",
        rusqlite::params![burst + 3],
    )
    .unwrap();
    db
}

// native-codex-matrix-2 P6: under an invitation burst the handoff thread's
// own invitation is older than the newest MAX_DIGEST_IDS, yet it is listed
// (first), so the hook's step 1 emits its plain, non-optional accept line
// immediately after the pinned inbox action; the newest burst invitations fill
// the rest after the reply template. Holds
// also when the seat walk saturates and the handoff invitation lies outside
// its window (one bounded per-thread walk). Kills: a newest-only selection,
// a listed count above MAX_DIGEST_IDS, an optional label on the handoff
// accept, or a receipt-thread lookup confined to the seat window.
#[test]
fn burst_lists_the_receipt_threads_older_invitation_first() {
    for burst in [22, WINDOW as i64 + 5] {
        let db = burst_after_handoff(burst);
        let d = digest(&db, "s").digest;
        assert_eq!(d.validate(), Ok(()), "{burst}");
        assert_eq!(ids(&d.receipts), ["m-h"]);
        let newest = |k: i64| format!("inv-b{}", burst - k);
        assert_eq!(
            ids(&d.invitations),
            ["inv-h", &newest(0), &newest(1), &newest(2)],
            "{burst}"
        );
        assert!(d.invitations.has_more);
        assert_eq!(
            d.token.invitation.map(|(seq, _)| seq),
            Some(burst as u64 + 2)
        );
        let actions = crate::harness::next_actions(&["herdr-threads".to_owned()], Some(&d));
        assert_eq!(
            actions.items[..3],
            [
                "- pending mail for th and other threads: herdr-threads inbox",
                "- accept (if topic and goal fit your role/remit): herdr-threads accept th",
                "- reply (replace <text>): herdr-threads send th --body '<text>'",
            ],
            "{burst}: {:?}",
            actions.items
        );
    }
    // Without a burst the listing is unchanged: newest first.
    let d = digest(&burst_after_handoff(2), "s").digest;
    assert_eq!(ids(&d.invitations), ["inv-h", "inv-b2", "inv-b1"]);
}

// Review N1: a listed invitation that carries a pending required membership
// reports that requirement's ID and current revision, so the hook can emit the
// exact accept-required argv; an ordinary invitation reports none, and an
// accepted requirement or one riding another invitation is not attached.
// Kills: omitting the requirement, attaching it to the wrong invitation,
// reporting a stale revision, or reporting a non-pending requirement.
#[test]
fn listed_required_invitations_carry_their_pending_requirement() {
    let db = every_source();
    db.execute_batch("\
        INSERT INTO service_authors(id,instance_id,created_at) VALUES ('author','i',0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,managed_owner_author_id) VALUES ('tm','i','m','g',0,0,'author');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-req','tm','s',1,'pending',0,6,100,100);\
        INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES ('req-1','tm','s','author','inv-req','pending',6,0);\
    ").unwrap();
    let d = digest(&db, "s").digest;
    assert_eq!(ids(&d.invitations), ["inv-req", "inv-s"]);
    let required = d.invitations.items[0].requirement.as_ref().unwrap();
    assert_eq!((required.id.as_str(), required.revision), ("req-1", 1));
    assert_eq!(d.invitations.items[1].requirement, None);
    assert!(
        d.receipts
            .items
            .iter()
            .all(|item| item.requirement.is_none())
    );
    db.execute_batch("UPDATE requirement_episodes SET revision=2 WHERE id='req-1'")
        .unwrap();
    let d = digest(&db, "s").digest;
    assert_eq!(
        d.invitations.items[0]
            .requirement
            .as_ref()
            .unwrap()
            .revision,
        2
    );
    // Wire shape: absent for ordinary invitations (older readers unaffected).
    let wire = serde_json::to_value(&d.invitations.items[1]).unwrap();
    assert!(wire.get("requirement").is_none(), "{wire}");
}

/// Count SQLite VM instructions (in units of 10) while `f` runs, with a
/// counter private to this call (ht-p03.6).
fn vm_units<T>(db: &Connection, f: impl FnOnce() -> T) -> (T, u64) {
    let counter = crate::test_support::isolation::CostCounter::default();
    let value = crate::test_support::isolation::count_vm_units(db, &counter, f);
    (value, counter.units())
}

/// `threads` instance threads, each with another seat's membership interval,
/// pending invitation, pending receipt and a warning that seat receives; the
/// digest seat is a member of three of its own threads.
fn at_scale(threads: u64) -> Connection {
    let db = empty();
    db.execute_batch(&format!("\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{threads})\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) SELECT 'x'||x,'i','topic','goal',0,0 FROM n;\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) SELECT id,'o',1,1 FROM threads;\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) SELECT 'i'||id,id,'o',1,'pending',0,2,100,100 FROM threads;\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,event_offset) SELECT 'm'||id,'i',id,1,'ordinary','b',0,3,ordinal FROM threads;\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) SELECT 'm'||id,id,'o','pending',100 FROM threads;\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) SELECT 'w'||id,'i',id,2,'warn','{{}}',0,4,ordinal FROM threads;\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'w'||id,4,id,100,'o','invitation','i'||id FROM threads;\
        INSERT INTO warning_recipients(warning_id,seat_id,generation) SELECT 'w'||id,'o',1 FROM threads;\
        UPDATE warning_jobs SET status='complete',phase='complete';\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('s1','i','a','g',0,0),('s2','i','b','g',0,0),('s3','i','c','g',0,0);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('s1','s',1,1),('s2','s',1,1),('s3','s',1,1);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-s','s3','s',1,'pending',0,5,100,100);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('ms','i','s1',1,'ordinary','b',0,6);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('ms','s1','s','pending',100);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('ws','i','s2',1,'warn','{{}}',0,7,0);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('ws',7,'s2',1000000000,'o','invitation','inv-s');\
    ")).unwrap();
    db
}

// Kills: an instance-wide walk in the producer (for example all warning
// publications instead of the seat's threads' warnings) and a
// membership-interval lookup that is not seat-leading (a scan of every
// interval). The seat's work is
// identical in indexed candidates, and within 10% in SQLite VM instructions,
// from 10^3 to 10^4 instance threads (a 10x span: any per-thread walk grows
// tenfold). The 10^5 case is
// `digest_cost_is_flat_from_a_thousand_to_a_hundred_thousand_threads`.
#[test]
fn digest_cost_is_flat_from_a_thousand_to_ten_thousand_threads() {
    assert_digest_cost_flat_in_threads(&[1_000, 10_000]);
}

#[test]
#[ignore = "seeds 10^5 instance threads; run in release"]
fn digest_cost_is_flat_from_a_thousand_to_a_hundred_thousand_threads() {
    assert_digest_cost_flat_in_threads(&[1_000, 10_000, 100_000]);
}

fn assert_digest_cost_flat_in_threads(sizes: &[u64]) {
    let mut observed = Vec::new();
    for &threads in sizes {
        let db = at_scale(threads);
        let (run, units) = vm_units(&db, || digest(&db, "s"));
        assert_eq!(ids(&run.digest.invitations), ["inv-s"]);
        assert_eq!(ids(&run.digest.receipts), ["ms"]);
        assert_eq!(ids(&run.digest.warnings), ["ws"]);
        observed.push((threads, run.work_steps, units));
    }
    eprintln!("digest cost (threads, indexed steps, vm units/10): {observed:?}");
    let (_, steps, units) = observed[0];
    for &(threads, s, u) in &observed[1..] {
        assert_eq!(s, steps, "indexed steps at {threads} threads: {observed:?}");
        assert!(
            u <= units + units / 10,
            "vm work grew at {threads} threads: {observed:?}"
        );
    }
}

/// Seat `s` with `acked` ACKed receipts (half physical, half manifest-backed,
/// each settled through the writer's own UPDATE/INSERT so the v8 triggers run)
/// and `warns` settled warnings in a thread it is a member of: invitation
/// conditions accepted, receipt conditions ACKed, and unavailable manifest
/// warnings whose episode has closed. On top sits a constant pending set: one
/// invitation, one physical and one manifest receipt, one open warning.
fn with_settled_history(acked: u64, warns: u64) -> Connection {
    let db = empty();
    let half = acked / 2;
    let third = warns / 3;
    db.execute_batch(&format!("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('h','i','hist','g',0,0);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('h','s',1,1),('h','o',1,1);\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{half})\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) SELECT 'ph'||x,'i','h',x,'ordinary','b',0,1000000+x FROM n;\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) SELECT id,'h','s','pending',100 FROM messages WHERE id LIKE 'ph%';\
        UPDATE receipts SET state='acked',acked_at=1,ack_actor_seat_id='s',ack_generation=1,ack_observation='obs' WHERE seat_id='s';\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{half})\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) SELECT 'mf'||x,'i','h',1000000+x,'ordinary','b',0,2000000+x FROM n;\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT 'p'||id,'i','actor',id,zeroblob(32),'h',0,0,0,0,0,0,1,'sealed' FROM messages WHERE id LIKE 'mf%';\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) SELECT 'p'||id,'h','s',1,300,1 FROM messages WHERE id LIKE 'mf%';\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) SELECT 'p'||id,id,'i','h',decision_seq,0,sequence,0,1,0 FROM messages WHERE id LIKE 'mf%';\
        INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) SELECT id,'s','acked','s',1,'obs',1 FROM messages WHERE id LIKE 'mf%';\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{third})\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) SELECT 'hi'||x,'h','o',x+1,'pending',0,3,100,100 FROM n;\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq) SELECT 'w'||id,'i','h',3000000+ordinal,'warn','{{}}',0,3000000+ordinal FROM invitations WHERE id LIKE 'hi%';\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'w'||id,3000000+ordinal,'h',100,'o','invitation',id FROM invitations WHERE id LIKE 'hi%';\
        UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='o',accepted_generation=1,accepted_observation='obs' WHERE id LIKE 'hi%';\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{third})\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) SELECT 'rm'||x,'i','h',4000000+x,'ordinary','b',0,4000000+x FROM n;\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) SELECT id,'h','o','pending',100 FROM messages WHERE id LIKE 'rm%';\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,source_message_id) SELECT 'w'||id,'i','h',sequence+1000000,'warn','{{}}',0,decision_seq+1000000,id FROM messages WHERE id LIKE 'rm%';\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'w'||id,decision_seq+1000000,'h',100,'o','receipt',length(id)||':'||id||':o' FROM messages WHERE id LIKE 'rm%';\
        UPDATE receipts SET state='acked',acked_at=1,ack_actor_seat_id='o',ack_generation=1,ack_observation='obs' WHERE seat_id='o';\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{third})\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT 'pu'||x,'i','actor','u'||x,zeroblob(32),'h',0,0,0,0,0,0,0,'sealed' FROM n;\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) SELECT id,'k'||id,'wu'||id,'o',1,1,'{{}}' FROM send_preparations WHERE id LIKE 'pu%';\
        UPDATE seats SET unavailability_open=0 WHERE id='o';\
        UPDATE warning_jobs SET status='complete',phase='complete';\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-s','h','s',1,'pending',0,5,100,100),('inv-o','h','o',1,'pending',0,5,100,100);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('mp','i','h',9000001,'ordinary','b',0,6),('mm','i','h',9000002,'ordinary','b',0,7);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('mp','h','s','pending',100);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('pmm','i','actor','mm',zeroblob(32),'h',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('pmm','h','s',1,300,1);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('pmm','mm','i','h',7,0,9000002,0,1,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w-open','i','h',9000003,'warn','{{}}',0,8,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w-open',8,'h',100,'o','invitation','inv-o');\
    ")).unwrap();
    db
}

// Digest fix2 B1: per-call work is proportional to the seat's pending
// attention, never to its retained history. From 10^3 to 10^4 ACKed receipts
// on the digest seat (half physical, half manifest-backed) and from 10^3 to
// 10^4 settled warnings in its member thread (10x on both axes; the 10^5
// receipt case is
// `digest_cost_is_flat_at_a_hundred_thousand_acked_receipts`), the digest
// examines exactly the
// same indexed candidates and the same SQLite VM work (within 10%). Kills:
// the full-history receipt walk (`scan_effective_receipts(Seat)` to
// completion), a physical walk on `receipts_seat_state_ordinal` without the
// pending restriction, a manifest walk on `prepared_recipients_seat_ordinal`
// instead of the pending projection, the member-thread warning walk over every
// warn message/manifest (`messages_thread_warning`), and a missing settlement
// trigger (settled rows left in a projection).
#[test]
fn digest_cost_is_flat_in_the_seats_acked_receipts_and_settled_warnings() {
    assert_digest_cost_flat_in_settled_history(&[(1_000, 1_002), (10_000, 10_002)]);
}

#[test]
#[ignore = "seeds 10^5 ACKed receipts; run in release"]
fn digest_cost_is_flat_at_a_hundred_thousand_acked_receipts() {
    assert_digest_cost_flat_in_settled_history(&[
        (1_000, 1_002),
        (10_000, 3_000),
        (100_000, 10_002),
    ]);
}

fn assert_digest_cost_flat_in_settled_history(sizes: &[(u64, u64)]) {
    let mut observed = Vec::new();
    for &(acked, warns) in sizes {
        let db = with_settled_history(acked, warns);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM receipts WHERE seat_id='s' AND state='acked'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                + db.query_row(
                    "SELECT count(*) FROM receipt_state WHERE seat_id='s' AND state='acked'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            acked as i64
        );
        let (run, units) = vm_units(&db, || digest(&db, "s"));
        assert_eq!(ids(&run.digest.invitations), ["inv-s"]);
        assert_eq!(ids(&run.digest.receipts), ["mm", "mp"]);
        assert_eq!(ids(&run.digest.warnings), ["w-open"]);
        if acked == 1_000 {
            assert_eq!(run.frontier, wake_frontier(&db, "s"));
            assert_eq!(digest(&db, "o").frontier, wake_frontier(&db, "o"));
        }
        observed.push((acked, warns, run.work_steps, units));
    }
    eprintln!("digest cost (acked, settled warns, indexed steps, vm units/10): {observed:?}");
    let (_, _, steps, units) = observed[0];
    for &(acked, _, s, u) in &observed[1..] {
        assert_eq!(s, steps, "indexed steps at {acked} ACKed: {observed:?}");
        assert!(
            u <= units + units / 10,
            "vm work grew at {acked} ACKed: {observed:?}"
        );
    }
}

// Kills: a projection that keeps settled rows because its maintaining trigger
// is missing (each settlement path leaves the projection holding only the
// still-pending rows), and a backfill that omits pending rows or copies
// settled ones when a v7 database is upgraded.
#[test]
fn projections_hold_only_pending_rows_through_settlement_and_upgrade() {
    let db = with_settled_history(10, 9);
    let projected = |db: &Connection| -> Vec<(String, i64)> {
        [
            "digest_pending_invitations",
            "digest_pending_manifest_receipts",
            "digest_open_warnings",
            "digest_programmatic_warnings",
        ]
        .iter()
        .map(|t| {
            (
                t.to_string(),
                db.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
                    .unwrap(),
            )
        })
        .collect()
    };
    let expect = vec![
        ("digest_pending_invitations".to_string(), 2),
        ("digest_pending_manifest_receipts".to_string(), 1),
        ("digest_open_warnings".to_string(), 1),
        ("digest_programmatic_warnings".to_string(), 0),
    ];
    assert_eq!(projected(&db), expect);
    // Rebuild the projections from scratch through the migration backfill.
    db.execute_batch(
        "DELETE FROM digest_pending_invitations; DELETE FROM digest_pending_manifest_receipts; DELETE FROM digest_open_warnings; DELETE FROM digest_programmatic_warnings;",
    )
    .unwrap();
    let backfill: String = include_str!("../../migrations/0008_digest_pending_paths.sql")
        .split("\n\n")
        // The notice frontier backfill is the V8 table's shape; V35 owns the table now.
        .filter(|s| s.trim_start().starts_with("INSERT") && !s.contains("INTO digest_notice_offer"))
        .collect::<Vec<_>>()
        .join("\n");
    db.execute_batch(&backfill).unwrap();
    assert_eq!(projected(&db), expect);
    let run = digest(&db, "s");
    assert_eq!(ids(&run.digest.receipts), ["mm", "mp"]);
    assert_eq!(ids(&run.digest.warnings), ["w-open"]);
    // Settling the last pending items empties every projection.
    db.execute_batch("\
        UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='s',accepted_generation=1,accepted_observation='obs' WHERE id IN ('inv-s','inv-o');\
        INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES ('mm','s','acked','s',1,'obs',1);\
    ").unwrap();
    assert!(
        projected(&db).iter().all(|(_, n)| *n == 0),
        "{:?}",
        projected(&db)
    );
    let run = digest(&db, "s");
    assert_eq!(run.digest.invitations.count, 0);
    assert_eq!(ids(&run.digest.receipts), ["mp"]);
    assert_eq!(run.digest.warnings.count, 0);
}

/// A file-backed store with sender `snd` and recipient `rcv`, both joined to
/// threads `hist` and `live`, holding `acked` ACKed receipts for `rcv` in
/// `hist` written through the real send/manifest/projection/ACK writers and
/// as many accepted invitations for `rcv` in `hist`, plus a constant pending
/// set in `live`: one pending invitation and one pending receipt (a real
/// send, projected, not ACKed).
struct RemoveOnDrop(std::path::PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

fn production_history(
    acked: u64,
) -> (
    crate::store::connection::StoreContext,
    Connection,
    RemoveOnDrop,
) {
    let path =
        std::env::temp_dir().join(format!("ht-digestfix3-history-{}.db", uuid::Uuid::new_v4()));
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(crate::app::SystemClock::new()),
    );
    let guard = RemoveOnDrop(path);
    let mut conn = context.open_writer().unwrap();
    // Setup speed only: the send writer's preparation lookup scans every
    // retained preparation (reported separately), so keep them cached, and
    // a throwaway store needs no fsync per writer commit.
    conn.pragma_update(None, "cache_size", -262_144).unwrap();
    conn.pragma_update(None, "synchronous", "OFF").unwrap();
    conn.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES ('snd','i','resolved','native','p-snd',1,1,0,1),('rcv','i','resolved','native','p-rcv',1,1,0,1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('hist','i','history','g',0,0),('live','i','live','g',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('hist','snd','joined'),('hist','rcv','joined'),('live','snd','joined'),('live','rcv','joined');\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('hist','snd',1,1),('hist','rcv',1,1),('live','snd',1,1),('live','rcv',1,1);\
    ").unwrap();
    crate::test_support::history::write_acked_history(
        &context, &mut conn, "hist", "snd", "rcv", acked, "hist",
    )
    .unwrap();
    // As many settled (accepted) invitations for `rcv` in `hist`, written as
    // the accept writer leaves them (an UPDATE of state, v8 triggers firing).
    if acked > 0 {
        conn.execute_batch(&format!("\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{acked})\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) SELECT 'settled-'||x,'hist','rcv',x+10,'pending',0,1,100,100 FROM n;\
        UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='rcv',accepted_generation=1,accepted_observation='obs' WHERE id LIKE 'settled-%';\
    ")).unwrap();
    }
    // The constant pending set: a live receipt (sent, projected, not ACKed)
    // and a pending invitation.
    crate::test_support::history::write_pending_sends(
        &context, &mut conn, "live", "snd", 1, "live",
    )
    .unwrap();
    conn.execute_batch("\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-live','live','rcv',2,'pending',0,1,100,100);\
    ").unwrap();
    (context, conn, guard)
}

/// Check-in's offer reads (the first inbox page, the exact warning count and
/// the first warnings page) in one read transaction, as `register_available`
/// builds them, with their SQLite VM work.
fn check_in_reads(
    db: &Connection,
) -> (
    crate::protocol::pagination::Page<crate::protocol::results::InboxItem>,
    (u64, bool),
    usize,
    u64,
) {
    use crate::protocol::{
        output::OutputSpec,
        pagination::PageRequest,
        results::CommandResult,
        time::{CallBudget, MonoInstant},
    };
    use crate::store::queries;
    let clock = crate::app::SystemClock::new();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let seat = SeatId::new("rcv");
    let page = PageRequest {
        max_bytes: 8_000,
        ..PageRequest::default()
    };
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let ((inbox, count, warnings), units) = vm_units(db, || {
        let inbox = queries::inbox_in_transaction(
            db,
            "i",
            &seat,
            &page,
            &OutputSpec::default(),
            &budget,
            &clock,
        )
        .unwrap();
        let count =
            queries::pending_warning_count_in_transaction(db, "i", &seat, &budget, &clock).unwrap();
        let CommandResult::Warnings(warnings) = queries::warnings_in_transaction(
            db,
            "i",
            &crate::protocol::commands::WarningsQuery {
                seat: seat.clone(),
                page: page.clone(),
            },
            &OutputSpec::default(),
        )
        .unwrap() else {
            panic!("warnings query returned another result")
        };
        (inbox, count, warnings.items.len())
    });
    db.execute_batch("COMMIT").unwrap();
    (inbox, count, warnings, units)
}

fn assert_check_in_reads_flat(sizes: &[u64]) {
    let mut observed = Vec::new();
    for &acked in sizes {
        let started = std::time::Instant::now();
        let (_context, db, _guard) = production_history(acked);
        let written = started.elapsed();
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM receipt_state WHERE seat_id='rcv' AND state='acked'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            acked as i64
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0,
            "production writers create no physical receipt rows"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM digest_pending_manifest_receipts",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let (inbox, count, warnings, units) = check_in_reads(&db);
        let (run, digest_units) = vm_units(&db, || digest(&db, "rcv"));
        assert_eq!(
            (
                run.digest.invitations.count,
                run.digest.receipts.count,
                run.digest.warnings.count
            ),
            (1, 1, 2),
            "{acked}"
        );
        let items: Vec<(&str, u64, u64, u64)> = inbox
            .items
            .iter()
            .map(|i| {
                (
                    i.thread.as_str(),
                    i.invitations,
                    i.pending_receipts,
                    i.warnings,
                )
            })
            .collect();
        // `rcv` has never registered, so the first send in each thread also
        // published its one unavailable warning (later sends reuse it).
        assert_eq!(items, [("hist", 0, 0, 1), ("live", 1, 1, 1)], "{acked}");
        assert_eq!((count, warnings), ((2, false), 2), "{acked}");
        eprintln!(
            "acked {acked}: check-in vm units/10 {units}, digest vm units/10 {digest_units}, history written in {written:?}"
        );
        observed.push((acked, units, digest_units, written));
    }
    eprintln!(
        "check-in reads (acked, check-in vm units/10, digest vm units/10, history write time): {observed:?}"
    );
    let (_, units, digest_units, _) = observed[0];
    for &(acked, u, d, _) in &observed[1..] {
        assert!(
            u <= units + units / 10,
            "check-in read work grew at {acked} ACKed: {observed:?}"
        );
        assert!(
            d <= digest_units + digest_units / 10,
            "digest work grew at {acked} ACKed: {observed:?}"
        );
    }
}

// Digest fix3 B1 (store level): with 10^3 and 10^4 ACKed receipts in the
// production shape (every send published with a manifest, projected by the
// send worker and ACKed through the real writers; no physical `receipts`
// rows) and as many settled invitations of the same seat, check-in's offer
// reads and the seat digest each do the same SQLite VM work (within 10%) and
// return the same exact answer: the live thread with its one invitation and
// one pending receipt, and one unavailable warning per thread. The 10^5 case
// is `check_in_reads_are_flat_at_a_hundred_thousand_acked_receipts`. Kills
// (as demonstrated in digest fix3 against the walks of that time; digest
// fix5 replaced those code paths with the bounded per-source walks, which
// this regression keeps flat): the digest's invitation walk driven from
// `invitations_seat_decision` (every invitation the seat ever had is
// visited), the inbox invitation count on
// `invitations_thread_seat_episode` (the seat's settled invitations in the
// thread),
// the thread manifest receipt walk without the pending restriction
// (`prepared_recipients` by thread instead of the v8 projection: inbox work
// grows with every ACKed receipt), a warning walk that steps through the
// thread's ordinary timeline (`scan_effective_timeline` in
// `scan_effective_warnings_for_seat`: the first warnings page then stops on
// its candidate limit inside `hist` and returns one warning instead of two).
// The default-suite guard for the same regressions (ht-zo4.7): ACKed
// receipts and settled invitations are never visited, so the work is flat
// already from 50 to 500 (10x; 500 still exceeds the first warnings page's
// candidate limit), without the minutes-long 10^4 seeding.
// It replaces the wall-clock budget check of
// hook_entrypoint::twenty_thousand_acked_receipts_stay_quiet_and_new_attention_is_emitted
// (now release-only) with deterministic SQLite VM work.
#[test]
fn check_in_reads_are_flat_from_fifty_to_five_hundred_acked_receipts() {
    assert_check_in_reads_flat(&[50, 500]);
}

#[test]
#[ignore = "writes 10^4 sends through the real writers (60-180 s in debug under parallel load); run in release"]
fn check_in_reads_are_flat_in_production_shaped_acked_receipts() {
    assert_check_in_reads_flat(&[1_000, 10_000]);
}

// The same regression at 10^5 ACKed receipts. Ignored by default only because
// writing 10^5 sends through the real writers is quadratic today (the send
// writer's preparation lookup scans every retained preparation); run it with
// `--release -- --ignored`. Kills the same mutations.
#[test]
#[ignore = "writes 10^5 sends through the real writers; run in release"]
fn check_in_reads_are_flat_at_a_hundred_thousand_acked_receipts() {
    assert_check_in_reads_flat(&[1_000, 100_000]);
}

/// Wave-2 (a) fixture: `rcv` is registered (an occupant binding on a fresh
/// observed target, so its receipts carry deadlines) and a member of `hist`
/// and `live` with `snd`. `hist` holds `settled` settled receipt-overdue
/// warnings and as many ACKed receipts, all written through the production
/// writers (`write_settled_warning_history`: send, manifest, send-worker
/// projection, due scanner, warning attribution, ACK). `live` holds the
/// constant pending set: one pending invitation and one overdue require-ack
/// message (a pending receipt with its pending receipt-overdue warning).
fn production_warning_history(
    settled: u64,
) -> (
    crate::store::connection::StoreContext,
    Connection,
    RemoveOnDrop,
) {
    let path = std::env::temp_dir().join(format!(
        "ht-digestfix4-warnings-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = crate::store::connection::StoreContext::new(
        path.clone(),
        std::sync::Arc::new(crate::app::SystemClock::new()),
    );
    let guard = RemoveOnDrop(path);
    let mut conn = context.open_writer().unwrap();
    conn.pragma_update(None, "cache_size", -262_144).unwrap();
    conn.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,unavailability_open) VALUES ('snd','i','resolved','native','p-snd',1,1,0,1,0),('rcv','i','resolved','native','p-rcv',1,1,0,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at) VALUES ('i','p-rcv','b',1,1,1,'fresh','unknown','unknown',0,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('rcv',1,1,'p-rcv','b',1,'codex','hist-rcv','00000000-0000-4000-8000-0000000000cc','fresh',0,0,'term-rcv','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('hist','i','history','g',0,0),('live','i','live','g',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('hist','snd','joined'),('hist','rcv','joined'),('live','snd','joined'),('live','rcv','joined');\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('hist','snd',1,1),('hist','rcv',1,1),('live','snd',1,1),('live','rcv',1,1);\
    ").unwrap();
    crate::test_support::history::write_settled_warning_history(
        &context, &mut conn, "hist", "snd", "rcv", settled, "hist",
    )
    .unwrap();
    crate::test_support::history::write_overdue_sends(
        &context, &mut conn, "live", "snd", "rcv", 1, "live",
    )
    .unwrap();
    conn.execute_batch("\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-live','live','rcv',2,'pending',0,1,100,100);\
    ").unwrap();
    (context, conn, guard)
}

fn assert_check_in_reads_flat_in_settled_warnings(sizes: &[u64]) {
    let mut observed = Vec::new();
    let mut first_page = None;
    for &settled in sizes {
        let started = std::time::Instant::now();
        let (_context, db, _guard) = production_warning_history(settled);
        let written = started.elapsed();
        let scalar = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        // Production shape: every settled warning is a published warn row,
        // projected by the attribution worker, with an ACKed receipt, and no
        // open-condition projection row remains for it.
        assert_eq!(
            scalar("SELECT count(*) FROM messages WHERE kind='warn' AND thread_id='hist'"),
            settled as i64
        );
        assert_eq!(
            scalar(
                "SELECT count(*) FROM receipt_state WHERE seat_id='rcv' AND state='acked' AND warning_message_id IS NOT NULL"
            ),
            settled as i64
        );
        assert_eq!(
            scalar("SELECT count(DISTINCT warning_id) FROM warning_recipients WHERE seat_id='rcv'"),
            settled as i64 + 1
        );
        assert_eq!(scalar("SELECT count(*) FROM digest_open_warnings"), 1);
        assert_eq!(
            scalar("SELECT count(*) FROM work_jobs WHERE status IN ('pending','failed')"),
            0
        );
        let (inbox, count, warnings, units) = check_in_reads(&db);
        let (run, digest_units) = vm_units(&db, || digest(&db, "rcv"));
        assert_eq!(
            (
                run.digest.invitations.count,
                run.digest.receipts.count,
                run.digest.warnings.count
            ),
            (1, 1, 1),
            "{settled}"
        );
        let items: Vec<(&str, u64, u64, u64, bool)> = inbox
            .items
            .iter()
            .map(|i| {
                (
                    i.thread.as_str(),
                    i.invitations,
                    i.pending_receipts,
                    i.warnings,
                    i.warnings_has_more,
                )
            })
            .collect();
        // Only the live thread is pending; `hist` holds settled history only.
        assert_eq!(items, [("live", 1, 1, 1, false)], "{settled}");
        assert_eq!(count, (1, false), "{settled}");
        // The warnings page is the first page of full history: settled
        // warnings stay discoverable, and the page is the same bounded size.
        assert!(warnings > 0, "{settled}");
        assert_eq!(*first_page.get_or_insert(warnings), warnings, "{settled}");
        eprintln!(
            "settled {settled}: check-in vm units/10 {units}, digest vm units/10 {digest_units}, history written in {written:?}"
        );
        observed.push((settled, units, digest_units, written));
    }
    eprintln!(
        "check-in reads (settled warnings, check-in vm units/10, digest vm units/10, history write time): {observed:?}"
    );
    let (_, units, digest_units, _) = observed[0];
    for &(settled, u, d, _) in &observed[1..] {
        assert!(
            u <= units + units / 10,
            "check-in read work grew at {settled} settled warnings: {observed:?}"
        );
        assert!(
            d <= digest_units + digest_units / 10,
            "digest work grew at {settled} settled warnings: {observed:?}"
        );
    }
}

// Wave-2 (a) (digest fix4): with 10^3 and 10^4 settled receipt-overdue
// warnings and as many ACKed receipts, all written through the production
// writers, check-in's offer reads (inbox page, pending warning count, first
// warnings page) and the seat digest each do the same SQLite VM work (within
// 10%) and return the same answer: only `live`, with its one invitation, one
// pending receipt and one pending warning; pending count 1. The 10^5 case is
// `check_in_reads_are_flat_at_a_hundred_thousand_settled_warnings`. Kills:
// M-count, the check-in count walking every published warning with
// `is_warning_recipient` alone (fix3's `exact_warning_count_in_transaction`:
// the count becomes 10^3+1, reported as (1000, true), and its work grows
// with every settled warning);
// M-inbox, the inbox per-thread count walking the thread's warning timeline
// (`scan_effective_warnings_for_seat` to completion: `hist` is listed with
// every settled warning and the work grows).
#[test]
#[ignore = "writes 10^4 sends through the real writers (60-180 s in debug under parallel load); run in release"]
fn check_in_reads_are_flat_in_production_settled_warnings() {
    assert_check_in_reads_flat_in_settled_warnings(&[1_000, 10_000]);
}

// The same regression at 10^5 settled warnings and ACKed receipts. Ignored by
// default only because writing 10^5 sends through the real writers is
// quadratic today (the send writer's preparation lookup scans every retained
// preparation); run it with `--release -- --ignored`. Kills the same
// mutations.
#[test]
#[ignore = "writes 10^5 sends through the real writers; run in release"]
fn check_in_reads_are_flat_at_a_hundred_thousand_settled_warnings() {
    assert_check_in_reads_flat_in_settled_warnings(&[1_000, 100_000]);
}

// Digest fix3 step 5, scale-invariance inventory (report-only; ignored, run
// in release with `--nocapture`). One file-backed store over a constant
// pending set in `live` grows along two history axes, measured separately:
//   A. 10^3, 10^4, 10^5 ACKed production-shaped receipts for `rcv` in `hist`
//      (real send/manifest/projection/ACK writers), each step also adding as
//      many accepted (settled) invitations for `rcv` in `hist` (SQL, v8
//      triggers firing);
//   B. then, with A at 10^5, 10^3, 10^4, 10^5 settled invitation-overdue
//      warnings in `hist` (invitations for seat `inv`, marked overdue through
//      the due path's writer `record_overdue_if_pending`, then accepted),
//      which `rcv` receives as a member: settled but historical warnings.
// At each point every canonical effective scan runs to completion in one read
// transaction and prints its SQLite VM work (units of 10 instructions), wall
// time and answer. Answers are printed, not asserted, except that the axis-A
// answers stay unchanged across A's sizes. Time box: a paged scan still
// running after `HT_INVENTORY_CAP_SECS` (default 120) stops and prints
// `INCOMPLETE` with its page count instead of an answer (a lower bound, not a
// measurement). `HT_INVENTORY_AXES=B` skips axis A (B then grows on the
// constant pending set alone).
#[test]
#[ignore = "report-only inventory; writes 10^5 sends through the real writers"]
fn scale_invariance_inventory_of_canonical_effective_scans() {
    use crate::store::effective::{
        GlobalLogicalKinds, ReceiptScanScope, scan_effective_receipts, scan_effective_timeline,
        scan_effective_warnings_for_seat, scan_global_logical_candidates,
    };
    use crate::test_support::attention_oracle::scan_effective_seat_attention;
    let (context, mut db, _guard) = production_history(0);
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES ('inv','i','resolved','native','p-inv',1,1,0,1)", []).unwrap();
    let mut written = 0u64;
    let mut warned = 0u64;
    let mut answers: std::collections::BTreeMap<&str, String> = Default::default();
    let axes = std::env::var("HT_INVENTORY_AXES").unwrap_or_else(|_| "AB".into());
    let cap = std::time::Duration::from_secs(
        std::env::var("HT_INVENTORY_CAP_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120),
    );
    let began = std::cell::Cell::new(std::time::Instant::now());
    let over = |pages: &mut u64| {
        *pages += 1;
        (began.get().elapsed() > cap).then(|| format!("INCOMPLETE after {cap:?}, {pages} pages"))
    };
    for (axis, size) in [
        ("A", 1_000u64),
        ("A", 10_000),
        ("A", 100_000),
        ("B", 1_000),
        ("B", 10_000),
        ("B", 100_000),
    ] {
        if !axes.contains(axis) {
            continue;
        }
        if axis == "B" {
            db.execute_batch(&format!("\
                WITH RECURSIVE n(x) AS (VALUES({}) UNION ALL SELECT x+1 FROM n WHERE x<{size})\
                INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) SELECT 'overdue-'||x,'hist','inv',x,'pending',0,1,100,100 FROM n;\
            ", warned + 1)).unwrap();
            let tx = db.transaction().unwrap();
            for n in warned + 1..=size {
                let outcome = crate::store::schema::record_overdue_if_pending(
                    &tx,
                    &crate::protocol::authority::ObligationRef::Invitation(
                        crate::protocol::ids::InvitationId::new(format!("overdue-{n}")),
                    ),
                    &crate::ports::TimeBasis::Decision,
                    crate::protocol::time::UtcMillis(1_000),
                )
                .unwrap();
                assert!(outcome.inserted);
            }
            tx.execute("UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='inv',accepted_generation=1,accepted_observation='obs' WHERE id LIKE 'overdue-%' AND state='pending'", []).unwrap();
            tx.commit().unwrap();
            warned = size;
        }
        if axis == "A" {
            crate::test_support::history::write_acked_history(
                &context,
                &mut db,
                "hist",
                "snd",
                "rcv",
                size - written,
                &format!("inv{size}"),
            )
            .unwrap();
            db.execute_batch(&format!("\
            WITH RECURSIVE n(x) AS (VALUES({}) UNION ALL SELECT x+1 FROM n WHERE x<{size})\
            INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) SELECT 'settled-'||x,'hist','rcv',x+10,'pending',0,1,100,100 FROM n;\
            UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='rcv',accepted_generation=1,accepted_observation='obs' WHERE id LIKE 'settled-%' AND state='pending';\
        ", written + 1)).unwrap();
            written = size;
        }
        let message: String = db
            .query_row(
                "SELECT message_id FROM send_manifests ORDER BY decision_seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let mut row = Vec::new();
        let mut measure = |name: &'static str, f: &mut dyn FnMut() -> String| {
            db.execute_batch("BEGIN DEFERRED").unwrap();
            let started = std::time::Instant::now();
            began.set(started);
            let (answer, units) = vm_units(&db, &mut *f);
            let elapsed = started.elapsed();
            db.execute_batch("COMMIT").unwrap();
            if axis == "A"
                && let Some(prior) = answers.insert(name, answer.clone())
            {
                assert_eq!(prior, answer, "{name} changed at {size}");
            }
            row.push((name, units, elapsed, answer));
        };
        let receipts_full = |scope: ReceiptScanScope| {
            let mut position = None;
            let mut pending = 0usize;
            let mut pages = 0;
            loop {
                if let Some(stop) = over(&mut pages) {
                    return stop;
                }
                let slice = scan_effective_receipts(&db, &scope, position, 100).unwrap();
                pending += slice
                    .items
                    .iter()
                    .filter(|r| r.state == EffectiveReceiptState::Pending)
                    .count();
                if !slice.has_more {
                    return format!("pending={pending}");
                }
                position = Some(slice.position);
            }
        };
        measure("attention::seat_digest (hook quiet path)", &mut || {
            let run = seat_digest(&db, "i", &SeatId::new("rcv"), &no_budget).unwrap();
            format!(
                "inv={} rec={} warn={}",
                run.digest.invitations.count, run.digest.receipts.count, run.digest.warnings.count
            )
        });
        measure("scan_effective_seat_attention(rcv) (wake)", &mut || {
            let mut position = None;
            let mut pages = 0;
            loop {
                if let Some(stop) = over(&mut pages) {
                    return stop;
                }
                let slice =
                    scan_effective_seat_attention(&db, "rcv", position.take(), 100).unwrap();
                if let Some(a) = slice.attention {
                    return format!(
                        "inv={} rec={} warn={:?}",
                        a.has_pending_invitation,
                        a.has_pending_receipt,
                        a.latest_warning_seq.is_some()
                    );
                }
                position = Some(slice.position);
            }
        });
        measure("scan_effective_receipts(Seat rcv)", &mut || {
            receipts_full(ReceiptScanScope::Seat("rcv".into()))
        });
        measure("scan_effective_receipts(Thread hist)", &mut || {
            receipts_full(ReceiptScanScope::Thread("hist".into()))
        });
        measure("scan_effective_receipts(Message latest)", &mut || {
            receipts_full(ReceiptScanScope::Message(message.clone()))
        });
        measure("scan_effective_receipts(Instance i)", &mut || {
            receipts_full(ReceiptScanScope::Instance("i".into()))
        });
        measure("scan_effective_warnings_for_seat(hist, rcv)", &mut || {
            let mut position = None;
            let mut n = 0;
            let mut pages = 0;
            loop {
                if let Some(stop) = over(&mut pages) {
                    return stop;
                }
                let slice =
                    scan_effective_warnings_for_seat(&db, "hist", "rcv", position, 100).unwrap();
                n += slice.warnings.len();
                if !slice.has_more {
                    return format!("warnings={n}");
                }
                position = Some(slice.position);
            }
        });
        measure("scan_effective_timeline(hist) (history read)", &mut || {
            let mut position = None;
            let mut n = 0;
            let mut pages = 0;
            loop {
                if let Some(stop) = over(&mut pages) {
                    return stop;
                }
                let slice = scan_effective_timeline(&db, "hist", position, 100).unwrap();
                n += slice.entries.len();
                if !slice.has_more {
                    return format!("entries={}", if n > 0 { "some" } else { "none" });
                }
                position = Some(slice.position);
            }
        });
        measure("scan_global_logical_candidates(Warnings)", &mut || {
            let mut position = None;
            let mut n = 0;
            let mut pages = 0;
            loop {
                if let Some(stop) = over(&mut pages) {
                    return stop;
                }
                let slice = scan_global_logical_candidates(
                    &db,
                    "i",
                    GlobalLogicalKinds::Warnings,
                    position,
                    100,
                )
                .unwrap();
                n += slice.candidates.len();
                if !slice.has_more {
                    return format!("warnings={n}");
                }
                position = Some(slice.position);
            }
        });
        measure("capture_inbox_token(rcv)", &mut || {
            crate::store::effective::capture_inbox_token(&db, "i", "rcv")
                .map(|_| "ok".to_string())
                .unwrap()
        });
        // Check-in's offer reads count their own VM work in one transaction.
        let started = std::time::Instant::now();
        let (inbox, count, warnings, units) = check_in_reads(&db);
        let answer = format!(
            "{:?} count={count:?} warnings={warnings}",
            inbox
                .items
                .iter()
                .map(|i| (
                    i.thread.as_str().to_owned(),
                    i.invitations,
                    i.pending_receipts,
                    i.warnings
                ))
                .collect::<Vec<_>>()
        );
        if axis == "A"
            && let Some(prior) = answers.insert("check-in offer reads", answer.clone())
        {
            assert_eq!(prior, answer, "check-in offer reads changed at {size}");
        }
        row.push((
            "check-in offer reads (inbox+count+warnings)",
            units,
            started.elapsed(),
            answer,
        ));
        for (name, units, elapsed, answer) in row {
            eprintln!("INVENTORY {axis}={size}\t{name}\t{units}\t{elapsed:?}\t{answer}");
        }
    }
}

// Digest fix3, kept under wave-2 (a): check-in's pending warning count never
// walks the instance's threads. From 10^3 to 10^4 instance threads (the 10^5
// case is `pending_warning_count_is_flat_at_a_hundred_thousand_threads`; each with
// an ordinary message and no warning) the count for `s` (one pending
// invitation-overdue warning, through its membership) is exact with the same
// SQLite VM work (within 10%). Kills: a per-thread count loop (`SELECT id FROM
// threads WHERE instance_id=?1` with a timeline walk per thread), whose work
// grows with every retained thread.
#[test]
fn pending_warning_count_is_flat_in_instance_threads() {
    assert_pending_warning_count_flat_in_threads(&[1_000, 10_000]);
}

#[test]
#[ignore = "seeds 10^5 instance threads; run in release"]
fn pending_warning_count_is_flat_at_a_hundred_thousand_threads() {
    assert_pending_warning_count_flat_in_threads(&[1_000, 10_000, 100_000]);
}

fn assert_pending_warning_count_flat_in_threads(sizes: &[u64]) {
    use crate::protocol::time::{CallBudget, MonoInstant};
    let clock = crate::app::SystemClock::new();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let mut observed = Vec::new();
    for &threads in sizes {
        let db = empty();
        db.execute_batch(&format!("\
            WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{threads})\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) SELECT 'x'||x,'i','topic','goal',0,0,2 FROM n;\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,event_offset) SELECT 'm'||id,'i',id,1,'ordinary','b',0,3,ordinal FROM threads;\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('s1','i','a','g',0,0,2);\
            INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('s1','s',1,1);\
            INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-o','s1','o',1,'pending',0,5,100,100);\
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('ws','i','s1',1,'warn','{{}}',0,7,0);\
            INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('ws',7,'s1',1000000000,'o','invitation','inv-o');\
        ")).unwrap();
        db.execute_batch("BEGIN DEFERRED").unwrap();
        let (count, units) = vm_units(&db, || {
            crate::store::queries::pending_warning_count_in_transaction(
                &db,
                "i",
                &SeatId::new("s"),
                &budget,
                &clock,
            )
            .unwrap()
        });
        db.execute_batch("COMMIT").unwrap();
        assert_eq!(count, (1, false), "{threads}");
        observed.push((threads, units));
    }
    eprintln!("pending warning count (threads, vm units/10): {observed:?}");
    let (_, units) = observed[0];
    for &(threads, u) in &observed[1..] {
        assert!(
            u <= units + units / 10,
            "warning count work grew at {threads} threads: {observed:?}"
        );
    }
}

// Wave-2 (a): pending warning counts are capped with an explicit `has_more`.
// `s` receives 1,001 pending invitation-overdue warnings in `s1` (plus one
// settled one, whose invitation was accepted). Check-in's count is
// (1000, true) and the inbox lists `s1` with (1000, true); at exactly 1,000
// pending warnings both are (1000, false). Kills: an uncapped count (1001),
// a cap without `has_more` (false at 1,001), an off-by-one cap (`>=`: true at
// exactly 1,000), and counting the settled warning.
#[test]
fn pending_warning_counts_are_capped_with_has_more() {
    use crate::protocol::{
        output::OutputSpec,
        pagination::PageRequest,
        results::MAX_PENDING_WARNING_COUNT,
        time::{CallBudget, MonoInstant},
    };
    assert_eq!(MAX_PENDING_WARNING_COUNT, 1_000);
    let clock = crate::app::SystemClock::new();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let db = empty();
    db.execute_batch("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('s1','i','a','g',0,0,1003);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('s1','s',1,1);\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1002)\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) SELECT 'inv-'||x,'s1','o',x,'pending',0,5,100,100 FROM n;\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1002)\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) SELECT 'w-'||x,'i','s1',x,'warn','{}',0,40,x FROM n;\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1002)\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'w-'||x,40,'s1',1000000000,'o','invitation','inv-'||x FROM n;\
        INSERT INTO warning_recipients(warning_id,seat_id,generation) SELECT warning_id,'s',40 FROM warning_jobs;\
        UPDATE warning_jobs SET status='complete',phase='complete';\
        UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='o',accepted_generation=1,accepted_observation='obs' WHERE id='inv-1002';\
    ").unwrap();
    let seat = SeatId::new("s");
    let read = |db: &Connection| {
        let count = crate::store::queries::pending_warning_count_in_transaction(
            db, "i", &seat, &budget, &clock,
        )
        .unwrap();
        let inbox = crate::store::queries::inbox_in_transaction(
            db,
            "i",
            &seat,
            &PageRequest::default(),
            &OutputSpec::default(),
            &budget,
            &clock,
        )
        .unwrap();
        let items: Vec<(String, u64, bool)> = inbox
            .items
            .iter()
            .map(|i| {
                (
                    i.thread.as_str().to_owned(),
                    i.warnings,
                    i.warnings_has_more,
                )
            })
            .collect();
        (count, items)
    };
    assert_eq!(
        read(&db),
        ((1_000, true), vec![("s1".to_owned(), 1_000, true)])
    );
    db.execute("UPDATE invitations SET state='accepted',accepted_at=2,accepted_actor_seat_id='o',accepted_generation=1,accepted_observation='obs' WHERE id='inv-1001'", []).unwrap();
    assert_eq!(
        read(&db),
        ((1_000, false), vec![("s1".to_owned(), 1_000, false)])
    );
}

/// Wave-2 fix1 (a) acceptance axes, each written through the production
/// writers on top of `production_warning_history(0)`'s constant pending set
/// in `live` (one invitation, one overdue require-ack receipt and its
/// receipt-overdue warning).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// `n` threads created by `snd`, each with a pending invitation of `rcv`
    /// (`control::create_thread`, `control::invite`).
    PendingInvitations,
    /// `n` require-ack messages from `snd` in `hist`, published, projected
    /// and left pending for `rcv` (long receipt duration).
    PendingReceipts,
    /// `n` programmatic service warn notices in `hist` (prepare, publish,
    /// projection worker), never offered to `rcv`.
    ProgrammaticUndelivered,
    /// The same notices, then durably offered to `rcv`'s current occupant.
    ProgrammaticDelivered,
}

/// Ten days: no receipt or invitation of the axis history becomes overdue.
const LONG_MILLIS: u64 = 864_000_000;

/// Deliver every projected notice to `rcv`'s current occupant: the state
/// after successive check-in offers have carried the whole backlog page by
/// page (each advancing the binding-scoped `digest_notice_offer` frontier
/// over the page it carried; wave-2 fix2 (a)). No projection row is deleted.
fn offer_to_current_occupant(db: &Connection) {
    db.execute_batch("\
        BEGIN IMMEDIATE;\
        INSERT INTO digest_notice_offer(binding_id,seat_id,offered_ordinal) SELECT b.binding_id,'rcv',(SELECT MAX(ordinal) FROM digest_programmatic_warnings WHERE seat_id='rcv') FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id='rcv';\
        COMMIT;\
    ").unwrap();
}

fn axis_history(
    axis: Axis,
    n: u64,
) -> (
    crate::store::connection::StoreContext,
    Connection,
    RemoveOnDrop,
) {
    use crate::test_support::history;
    let (context, mut db, guard) = production_warning_history(0);
    match axis {
        Axis::PendingInvitations => {
            history::write_pending_invitations(
                &context,
                &mut db,
                "snd",
                "rcv",
                n,
                LONG_MILLIS,
                "axis",
            )
            .unwrap();
        }
        Axis::PendingReceipts => {
            history::write_pending_sends_with_deadline(
                &context,
                &mut db,
                "hist",
                "snd",
                n,
                LONG_MILLIS,
                "axis",
            )
            .unwrap();
        }
        Axis::ProgrammaticUndelivered | Axis::ProgrammaticDelivered => {
            history::write_programmatic_warnings(&context, &mut db, "hist", n, "axis").unwrap();
            if axis == Axis::ProgrammaticDelivered {
                offer_to_current_occupant(&db);
            }
        }
    }
    (context, db, guard)
}

fn saturated(n: u64) -> (u64, bool) {
    (
        n.min(MAX_PENDING_WARNING_COUNT),
        n > MAX_PENDING_WARNING_COUNT,
    )
}

use crate::protocol::results::MAX_PENDING_WARNING_COUNT;

/// Wave-2 fix1 (a) store-level flat regression: for `axis` at each of
/// `sizes`, check-in's offer reads (first inbox page, pending warning count,
/// first warnings page) and the seat digest return the expected saturated
/// answer and do the same SQLite VM work (within 10%) as at the first size.
fn assert_axis_flat(axis: Axis, sizes: &[u64]) {
    let mut observed = Vec::new();
    for &n in sizes {
        let started = std::time::Instant::now();
        let (_context, db, _guard) = axis_history(axis, n);
        let written = started.elapsed();
        let scalar = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        let (inbox, count, warnings, units) = check_in_reads(&db);
        let (run, digest_units) = vm_units(&db, || digest(&db, "rcv"));
        let d = &run.digest;
        let class = |c: &AttentionClass| (c.count, c.count_has_more);
        let items: Vec<(String, u64, bool, u64, bool, u64, bool)> = inbox
            .items
            .iter()
            .map(|i| {
                (
                    i.thread.as_str().to_owned(),
                    i.invitations,
                    i.invitations_has_more,
                    i.pending_receipts,
                    i.pending_receipts_has_more,
                    i.warnings,
                    i.warnings_has_more,
                )
            })
            .collect();
        let live = ("live".to_owned(), 1, false, 1, false, 1, false);
        match axis {
            Axis::PendingInvitations => {
                assert_eq!(
                    scalar(
                        "SELECT count(*) FROM invitations WHERE seat_id='rcv' AND state='pending'"
                    ),
                    n as i64 + 1
                );
                assert_eq!(class(&d.invitations), saturated(n + 1), "{n}");
                let newest: String = db
                    .query_row(
                        "SELECT id FROM invitations WHERE seat_id='rcv' ORDER BY created_decision_seq DESC LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                // P6: the receipt thread's (older) invitation leads, found
                // by one bounded per-thread walk once the seat walk saturates.
                assert_eq!(d.invitations.items[0].thread.as_str(), "live", "{n}");
                assert_eq!(d.invitations.items[1].id, newest, "{n}");
                assert_eq!(
                    (class(&d.receipts), class(&d.warnings)),
                    ((1, false), (1, false))
                );
                assert_eq!(count, (1, false), "{n}");
                assert_eq!(items[0], live, "{n}");
                assert!(items[1..].iter().all(|i| i.1 == 1 && i.3 == 0), "{n}");
            }
            Axis::PendingReceipts => {
                assert_eq!(class(&d.receipts), saturated(n + 1), "{n}");
                assert_eq!(
                    (class(&d.invitations), class(&d.warnings)),
                    ((1, false), (1, false))
                );
                assert_eq!(count, (1, false), "{n}");
                let (receipts, more) = saturated(n);
                assert_eq!(
                    items,
                    [
                        ("hist".to_owned(), 0, false, receipts, more, 0, false),
                        live
                    ],
                    "{n}"
                );
            }
            Axis::ProgrammaticUndelivered => {
                assert_eq!(
                    scalar("SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='rcv'"),
                    n as i64
                );
                assert_eq!(class(&d.warnings), saturated(n + 1), "{n}");
                assert_eq!(count, saturated(n + 1), "{n}");
                let (hist, more) = saturated(n);
                assert_eq!(
                    items,
                    [("hist".to_owned(), 0, false, 0, false, hist, more), live],
                    "{n}"
                );
            }
            Axis::ProgrammaticDelivered => {
                assert_eq!(
                    scalar(
                        "SELECT count(*) FROM messages WHERE kind='warn' AND author_kind='programmatic'"
                    ),
                    n as i64
                );
                // Delivered notices stay projected below the frontier.
                assert_eq!(
                    scalar("SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='rcv'"),
                    n as i64
                );
                assert_eq!(class(&d.warnings), (1, false), "{n}");
                assert_eq!(count, (1, false), "{n}");
                assert_eq!(items, [live], "{n}");
            }
        }
        assert!(warnings > 0, "{n}");
        assert!(
            run.work_steps <= 16 * crate::store::attention::WINDOW as u64,
            "{n}: {}",
            run.work_steps
        );
        eprintln!(
            "{axis:?} {n}: check-in vm units/10 {units}, digest vm units/10 {digest_units}, digest steps {}, written in {written:?}",
            run.work_steps
        );
        observed.push((n, units, digest_units, run.work_steps, written));
    }
    eprintln!(
        "{axis:?} (n, check-in vm units/10, digest vm units/10, digest steps, write time): {observed:?}"
    );
    let (_, units, digest_units, _, _) = observed[0];
    for &(n, u, d, _, _) in &observed[1..] {
        assert!(
            u <= units + units / 10,
            "{axis:?}: check-in read work grew at {n}: {observed:?}"
        );
        assert!(
            d <= digest_units + digest_units / 10,
            "{axis:?}: digest work grew at {n}: {observed:?}"
        );
    }
}

// Wave-2 fix1 (a), bounded-walk invariant, pending-invitation axis: 10^3 and
// 10^4 pending invitations (the 10^5 case is
// `axes_are_flat_at_a_hundred_thousand`). The digest counts min(n+1, 1000)
// with `count_has_more`, lists the receipt thread's invitation and then the
// newest invitation first, and the offer
// reads and the digest do the same SQLite VM work. Kills M-walk-without-LIMIT
// (the invitation walk's `LIMIT ?3` removed: every pending invitation is
// visited, digest work grows ~10x).
#[test]
#[ignore = "writes 10^4 items through the real writers (60-180 s in debug under parallel load); run in release"]
fn pending_invitation_axis_is_flat() {
    assert_axis_flat(Axis::PendingInvitations, &[1_000, 10_000]);
}

// Pending-receipt axis: 10^3 and 10^4 pending require-ack receipts in `hist`.
// Kills M-post-filter-cap (the receipt walks read every pending row and the
// window is cut to `cap+1` afterwards: same answer, work grows with the
// pending backlog).
#[test]
#[ignore = "writes 10^4 items through the real writers (60-180 s in debug under parallel load); run in release"]
fn pending_receipt_axis_is_flat() {
    assert_axis_flat(Axis::PendingReceipts, &[1_000, 10_000]);
}

// Programmatic notices never offered: 10^3 and 10^4 in `hist`. The count
// saturates at 1000 with `has_more`; work stays flat. Kills a programmatic
// walk without its `LIMIT` (work grows) and the fix4 behaviour of building
// the full pending set before the cap.
#[test]
#[ignore = "writes 10^4 items through the real writers (60-180 s in debug under parallel load); run in release"]
fn undelivered_programmatic_axis_is_flat() {
    assert_axis_flat(Axis::ProgrammaticUndelivered, &[1_000, 10_000]);
}

// Programmatic notices offered to the current occupant: they stay projected
// below the occupant's frontier, so the answer is the constant pending set
// alone and the work is flat. Kills M-never-settled (the programmatic walks
// ignoring the frontier: 10^3/10^4 notices stay pending and the count
// saturates) and a walk that post-filters the frontier (work grows with the
// delivered rows below it).
#[test]
#[ignore = "writes 10^4 items through the real writers (60-180 s in debug under parallel load); run in release"]
fn delivered_programmatic_axis_is_flat() {
    assert_axis_flat(Axis::ProgrammaticDelivered, &[1_000, 10_000]);
}

// The four axes at 10^5 against 10^3 (release; writing 10^5 items through
// the production writers takes minutes per axis). Kills the same mutations.
#[test]
#[ignore = "writes 10^5 items per axis through the real writers; run in release"]
fn axes_are_flat_at_a_hundred_thousand() {
    for axis in [
        Axis::PendingInvitations,
        Axis::PendingReceipts,
        Axis::ProgrammaticUndelivered,
        Axis::ProgrammaticDelivered,
    ] {
        assert_axis_flat(axis, &[1_000, 100_000]);
    }
}

/// One check-in offer's writes for `rcv`, as the check-in writers make them
/// once the offer is built (`seats.rs`): the seat-wide `warning_offer`
/// checkpoint upsert through the host decision sequence, then the settlement
/// of the notice page the offer carries (the oldest `MAX_NOTICE_PAGE_ITEMS`
/// above the occupant's frontier). Returns the carried notice IDs, the rows
/// the writes changed (`total_changes()`, trigger writes included) and their
/// SQLite VM work.
fn offer_writes(db: &Connection) -> (Vec<String>, i64, u64) {
    use crate::protocol::results::MAX_NOTICE_PAGE_ITEMS;
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let carried: Vec<crate::protocol::results::WarningRef> =
        notice_offer_page(db, "rcv", MAX_NOTICE_PAGE_ITEMS + 1)
            .unwrap()
            .into_iter()
            .take(MAX_NOTICE_PAGE_ITEMS)
            .map(|offered| offered.notice)
            .collect();
    let binding = crate::protocol::ids::BindingId::new(
        db.query_row(
            "SELECT b.binding_id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id='rcv'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    let total = |db: &Connection| -> i64 {
        db.query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap()
    };
    let before = total(db);
    let ((), units) = vm_units(db, || {
        db.execute("INSERT INTO warning_offer(binding_id,seat_id,offered_through_seq) SELECT ?1,'rcv',decision_seq FROM host_instances WHERE id='i' ON CONFLICT(binding_id) DO UPDATE SET offered_through_seq=MAX(warning_offer.offered_through_seq,excluded.offered_through_seq)",
            rusqlite::params![binding.as_str()]).unwrap();
        settle_carried_notices(db, "rcv", &binding, &carried).unwrap();
    });
    let changed = total(db) - before;
    db.execute_batch("COMMIT").unwrap();
    (
        carried
            .iter()
            .map(|notice| notice.warning.as_str().to_owned())
            .collect(),
        changed,
        units,
    )
}

/// Wave-2 fix2 (a) store-level settlement cost: with `n` undelivered
/// programmatic notices (production writers), two successive offers each
/// carry the next oldest page of 16 and settle it with exactly two row
/// writes (the `warning_offer` checkpoint and the notice frontier), whatever
/// `n`; the projection keeps all `n` rows; exactly the carried notices leave
/// the pending count; and the offer's write work stays flat across sizes.
fn assert_offer_settlement_constant(sizes: &[u64]) {
    let mut observed = Vec::new();
    for &n in sizes {
        let (_context, db, _guard) = axis_history(Axis::ProgrammaticUndelivered, n);
        let scalar = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        let oldest = |offset: u64| -> Vec<String> {
            db.prepare(&format!("SELECT warning_id FROM digest_programmatic_warnings WHERE seat_id='rcv' ORDER BY ordinal LIMIT 16 OFFSET {offset}"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        let pending = || {
            db.execute_batch("BEGIN DEFERRED").unwrap();
            let count = seat_pending_warnings(&db, "rcv", &no_budget)
                .unwrap()
                .count();
            db.execute_batch("COMMIT").unwrap();
            count
        };
        // The live receipt-overdue warning is the constant pending set.
        assert_eq!(pending(), saturated(n + 1), "{n}");
        let (first, first_changed, first_units) = offer_writes(&db);
        assert_eq!(first, oldest(0), "{n}");
        assert_eq!(first_changed, 2, "{n}: first offer rows written");
        assert_eq!(pending(), saturated(n + 1 - 16), "{n}");
        let (second, second_changed, second_units) = offer_writes(&db);
        assert_eq!(second, oldest(16), "{n}");
        assert_eq!(second_changed, 2, "{n}: second offer rows written");
        assert_eq!(pending(), saturated(n + 1 - 32), "{n}");
        assert_eq!(
            scalar("SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='rcv'"),
            n as i64
        );
        assert_eq!(
            scalar("SELECT offered_ordinal FROM digest_notice_offer WHERE seat_id='rcv'"),
            scalar(
                "SELECT ordinal FROM digest_programmatic_warnings WHERE seat_id='rcv' ORDER BY ordinal LIMIT 1 OFFSET 31"
            ),
            "{n}"
        );
        observed.push((n, first_units, second_units));
    }
    eprintln!(
        "offer settlement (n, first offer vm units/10, second offer vm units/10): {observed:?}"
    );
    let (_, first, second) = observed[0];
    for &(n, f, s) in &observed[1..] {
        assert!(
            f <= first + first / 10 + 5 && s <= second + second / 10 + 5,
            "offer settlement work grew at {n}: {observed:?}"
        );
    }
}

// Kills M-delete-all-settlement (settlement deletes the seat's projected
// notices, as the fix5 offer triggers did: rows written grow with n and the
// projection loses rows) and M-settle-beyond-page (the frontier jumps past
// the carried page: the pending count drops by more than 16 and the second
// offer carries the wrong page).
#[test]
#[ignore = "writes 10^4 notices through the real writers (60-180 s in debug under parallel load); run in release"]
fn offer_settlement_writes_are_constant() {
    assert_offer_settlement_constant(&[1_000, 10_000]);
}

// The same at 3x10^5 undelivered notices against 10^3 (release; the
// production writers take minutes to write them).
#[test]
#[ignore = "writes 3x10^5 notices through the real writers; run in release"]
fn offer_settlement_writes_are_constant_at_three_hundred_thousand() {
    assert_offer_settlement_constant(&[1_000, 300_000]);
}

// Kills: a v8 backfill that rebuilds `digest_open_warnings` but not
// `digest_open_warning_recipients` (an upgraded database would list the
// warning for nobody: the recipient walk reads the recipients projection).
// Seat `s` received warning `w1` about seat `o`'s pending invitation before the
// recipient projection existed; rebuilding the projections from the migration's
// INSERT statements alone must bring the recipient row and the digest back.
#[test]
fn recipients_backfill_covers_existing_warnings() {
    let db = empty();
    db.execute_batch("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','g',0,0);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-o','t','o',1,'pending',0,3,100,100);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('w1','i','t',1,'warn','{}',0,12,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('w1',12,'t',100,'o','invitation','inv-o');\
        INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('w1','s',1);\
        UPDATE warning_jobs SET status='complete',phase='complete';\
    ").unwrap();
    let recipient_rows = |db: &Connection| -> Vec<(String, String)> {
        db.prepare("SELECT seat_id,warning_id FROM digest_open_warning_recipients ORDER BY seat_id,warning_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let live = vec![("s".to_string(), "w1".to_string())];
    assert_eq!(recipient_rows(&db), live);
    assert_eq!(ids(&digest(&db, "s").digest.warnings), ["w1"]);
    // The projections as a pre-projection (v7) database would have them: empty.
    db.execute_batch(
        "DELETE FROM digest_open_warnings; DELETE FROM digest_open_warning_recipients;",
    )
    .unwrap();
    assert!(recipient_rows(&db).is_empty());
    assert!(ids(&digest(&db, "s").digest.warnings).is_empty());
    let backfill: String = include_str!("../../migrations/0008_digest_pending_paths.sql")
        .split("\n\n")
        // The notice frontier backfill is the V8 table's shape; V35 owns the table now.
        .filter(|s| s.trim_start().starts_with("INSERT") && !s.contains("INTO digest_notice_offer"))
        .collect::<Vec<_>>()
        .join("\n");
    db.execute_batch(&backfill).unwrap();
    assert_eq!(recipient_rows(&db), live);
    let run = digest(&db, "s");
    assert_eq!(ids(&run.digest.warnings), ["w1"]);
    assert_eq!(run.digest.warnings.count, 1);
}

// Kills narrowing a saturated warning window to nothing: a full window of
// newer transitions about another seat must not hide an older unoffered
// service notice for this seat (ht-pb7 review). The wake keeps the
// conservative unnarrowed answer, and the offer probe saturates true. The
// unsaturated control kills a fallback that ignores saturation.
#[test]
fn saturated_other_seat_transitions_do_not_hide_an_older_waking_notice() {
    let db = empty();
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    db.execute_batch("\
        INSERT INTO service_authors(id,instance_id,created_at) VALUES ('svc','i',0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t6','i','f','g',0,0);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t6','s',1,1),('t6','o',1,1);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset,author_kind,author_service_id) VALUES ('n0','i','t6',1,'warn','{}',0,20,1,'programmatic','svc');\
        INSERT INTO service_notification_publications(preparation_id,message_id,decision_seq,recipient_count) VALUES ('np0','n0',20,1);\
    ").unwrap();
    let transitions = |from: i64, to: i64| {
        db.execute_batch("BEGIN").unwrap();
        for n in from..to {
            let (id, seq) = (format!("w{n}"), 21 + n);
            db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES (?1,'i','t6',?2,'warn','{}',0,?3,1)", params![id, n + 2, seq]).unwrap();
            db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt','t6',?1,'o',?1,?2)", params![id, seq]).unwrap();
            db.execute(
                "INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES (?1,'s',?2)",
                params![id, seq],
            )
            .unwrap();
        }
        db.execute_batch("COMMIT").unwrap();
    };
    // Unsaturated, all informational: other seats' transitions never wake s.
    transitions(0, 3);
    assert_eq!(wake_attention(&db, "s").attention.latest_warning_seq, None);
    assert!(!seat_has_pending_wake_notices(&db, "s").unwrap());
    // The older service notice is delivered to s (projected before them).
    db.execute(
        "INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('n0','s',1)",
        [],
    )
    .unwrap();
    // Keep the notice older than every transition: below them by ordinal,
    // above the unoffered frontier (0).
    db.execute_batch(
        "UPDATE digest_programmatic_warnings SET ordinal=ordinal+10 WHERE warning_id!='n0';\
         UPDATE digest_programmatic_warnings SET ordinal=1 WHERE warning_id='n0';",
    )
    .unwrap();
    assert_eq!(
        wake_attention(&db, "s").attention.latest_warning_seq,
        Some(20)
    );
    // Saturated: a full window of newer transitions hides the service notice.
    transitions(3, WINDOW as i64);
    assert!(
        seat_pending_warnings(&db, "s", &no_budget)
            .unwrap()
            .saturated,
        "the fixture saturates the notice walk"
    );
    let wake = wake_attention(&db, "s");
    assert!(
        wake.attention.latest_warning_seq.is_some(),
        "a saturated window must stay conservative: {:?}",
        wake.attention
    );
    assert!(seat_has_pending_wake_notices(&db, "s").unwrap());
}

// ---- Per-binding attention (ht-54r.9) ------------------------------------------------------

use crate::protocol::ids::{BindingId, ThreadId};

/// Seat `s` holds two top-level sessions (bindings `bind-A`, `bind-B`, B the newer) and a subagent
/// of A (`bind-C`), all joined to thread `t` with a cursor at 0.
fn per_binding() -> Connection {
    let db = empty();
    db.execute_batch("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','a','g',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','s','joined',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,binding_id) VALUES \
            ('s',1,1,'pane-s','host',1,'claude','sess-A','ea','cooperative_top_level',0,0,'bind-A'),\
            ('s',1,1,'pane-s','host',1,'claude','sess-B','eb','cooperative_top_level',0,0,'bind-B');\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,binding_id,role,parent_binding,agent_id) VALUES \
            ('s',1,1,'pane-s','host',1,'claude','sess-A','ec','cooperative_hook_agent',0,0,'bind-C','subagent','bind-A','ag');\
        INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES ('bind-A','t',0),('bind-B','t',0),('bind-C','t',0);\
    ").unwrap();
    db
}

/// One ordinary message `m{n}` (thread sequence n) published to seat `s`.
fn deliver(db: &Connection, n: i64) {
    db.execute_batch(&format!("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p{n}','i','actor','op{n}',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p{n}','t','s',1,300,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m{n}','i','t',{n},'ordinary','b',0,{d});\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p{n}','m{n}','i','t',{d},0,{n},0,1,0);\
    ", d = 10 + n)).unwrap();
}

/// One informational notice `n{n}` projected for seat `s`.
fn notice(db: &Connection, n: i64) {
    db.execute_batch(&format!("\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('n{n}','i','t',{seq},'warn','{{}}',0,{d},1);\
        INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s','n{n}','t',{d},1);\
    ", seq = 100 + n, d = 20 + n)).unwrap();
}

fn bdigest(db: &Connection, binding: &str) -> DigestRun {
    db.execute_batch("BEGIN DEFERRED").unwrap();
    let run = binding_digest(db, "i", &BindingId::new(binding), &no_budget).unwrap();
    db.execute_batch("COMMIT").unwrap();
    run
}

fn mail_ids(run: &DigestRun) -> Vec<&str> {
    run.digest.mail.as_ref().map_or_else(Vec::new, ids)
}

/// A binding displays what it was shown: its cursor moves over the thread and its notice frontier
/// over the offered page.
fn display(db: &Connection, binding: &str, through: u64) {
    let binding = BindingId::new(binding);
    crate::store::read_cursors::advance(db, &binding, &ThreadId::new("t"), through).unwrap();
    let page = notice_offer_page_for(db, "s", &binding, 16).unwrap();
    settle_offered_notices(db, "s", &binding, &page).unwrap();
}

// Kills: a digest that is seat-wide (a second binding would see no mail of its own), a mail walk
// that ignores the binding's cursor, and a notice frontier shared by the seat's sessions. Two
// sessions of one seat each get a notice for one new message, and their tokens name their own
// binding so neither is ever compared with the other's mark.
#[test]
fn two_bindings_both_get_a_notice_for_one_new_message() {
    let db = per_binding();
    deliver(&db, 1);
    notice(&db, 1);
    let a = bdigest(&db, "bind-A");
    let b = bdigest(&db, "bind-B");
    for run in [&a, &b] {
        assert_eq!(run.digest.validate(), Ok(()));
        assert_eq!(mail_ids(run), ["m1"]);
        assert_eq!(run.digest.mail.as_ref().unwrap().count, 1);
        assert_eq!(ids(&run.digest.warnings), ["n1"]);
        assert_eq!(run.digest.token.mail, Some((11, 1)));
        assert!(!run.digest.is_empty());
        assert!(run.digest.summary().contains("mail=1 [m1@t]"));
    }
    assert!(a.digest.token.binding.is_some() && b.digest.token.binding.is_some());
    assert_ne!(a.digest.token.binding, b.digest.token.binding);
    // A token read for one binding is never compared with the other's mark: against it, the
    // pending mail counts as new.
    assert!(b.digest.token.advanced_beyond(&a.digest.token));
    assert!(!a.digest.token.advanced_beyond(&a.digest.token));
    // The seat-level digest carries neither the mail class nor a binding.
    let seat = digest(&db, "s");
    assert!(seat.digest.mail.is_none() && seat.digest.token.binding.is_none());
}

// Kills: a display that settles the seat's frontier or moves a shared cursor (clearing the other
// session's notice), and a frontier write that is not keyed by binding.
#[test]
fn display_by_one_binding_does_not_clear_the_others_notice() {
    let db = per_binding();
    deliver(&db, 1);
    notice(&db, 1);
    display(&db, "bind-A", 1);
    let a = bdigest(&db, "bind-A");
    assert!(mail_ids(&a).is_empty(), "A displayed the mail");
    assert!(
        ids(&a.digest.warnings).is_empty(),
        "A was offered the notice"
    );
    let b = bdigest(&db, "bind-B");
    assert_eq!(mail_ids(&b), ["m1"], "B's mail is untouched");
    assert_eq!(ids(&b.digest.warnings), ["n1"], "B's notice is untouched");
    let frontier = |binding: &str| binding_notice_frontier(&db, binding).unwrap();
    assert!(frontier("bind-A") > 0);
    assert_eq!(frontier("bind-B"), 0);
    // The notice stays settled for A when a newer one arrives for both.
    notice(&db, 2);
    assert_eq!(ids(&bdigest(&db, "bind-A").digest.warnings), ["n2"]);
    assert_eq!(ids(&bdigest(&db, "bind-B").digest.warnings), ["n2", "n1"]);
    // The seat view follows the newest top-level session (B), which still owes both.
    assert_eq!(notice_frontier(&db, "s").unwrap(), 0);
}

// Kills: a subagent that reads the seat's mail through its parent's cursor, one that writes the
// parent's frontier or cursor (its own reads advance nothing), and one that sees notices its
// parent was already offered.
#[test]
fn subagent_gets_its_own_notices_and_never_advances_the_parents() {
    let db = per_binding();
    deliver(&db, 1);
    notice(&db, 1);
    let rows = |sql: &str| -> Vec<(String, i64)> {
        let mut stmt = db.prepare(sql).unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    display(&db, "bind-A", 1);
    let cursors_before = rows("SELECT binding_id,seq FROM binding_cursors ORDER BY binding_id");
    let frontiers_before =
        rows("SELECT binding_id,offered_ordinal FROM digest_notice_offer ORDER BY binding_id");
    assert_eq!(frontiers_before.len(), 1, "only the parent has a frontier");
    // The parent displayed both, the subagent has seen neither: its own stream is still unread,
    // but the notice its parent was already offered is not repeated to it.
    let c = bdigest(&db, "bind-C");
    assert_eq!(mail_ids(&c), ["m1"]);
    assert!(ids(&c.digest.warnings).is_empty());
    notice(&db, 2);
    let c = bdigest(&db, "bind-C");
    assert_eq!(
        ids(&c.digest.warnings),
        ["n2"],
        "a new notice reaches the subagent"
    );
    assert!(binding_has_pending_notices(&db, &BindingId::new("bind-C")).unwrap());
    // Its token names its own binding, never the parent's.
    let a = bdigest(&db, "bind-A");
    assert_ne!(c.digest.token.binding, a.digest.token.binding);
    // Reading the subagent's digest (any number of times) moved nothing of the parent's.
    bdigest(&db, "bind-C");
    assert_eq!(
        rows("SELECT binding_id,seq FROM binding_cursors ORDER BY binding_id"),
        cursors_before
    );
    assert_eq!(
        rows("SELECT binding_id,offered_ordinal FROM digest_notice_offer ORDER BY binding_id"),
        frontiers_before
    );
    // A prospective digest (a subagent that has not enrolled) reads the same frontier by hash.
    let hash = crate::protocol::attention::binding_token_hash("s", "claude", "sess-A", "ag");
    let parent = binding_notice_frontier(&db, "bind-A").unwrap();
    let prospective = prospective_digest(&db, "i", &SeatId::new("s"), hash, parent, &no_budget)
        .unwrap()
        .digest;
    assert_eq!(prospective.token.binding, c.digest.token.binding);
    assert_eq!(ids(&prospective.warnings), ["n2"]);
    assert!(
        prospective
            .mail
            .as_ref()
            .is_some_and(|mail| mail.count == 0)
    );
}

// Kills: a mail walk that ignores the catch-up hold (spec §7): ordinary messages above an active
// row's frontier are not pushed to any binding of the seat, and take the row's release key once it
// ends, so the release makes them new attention again.
#[test]
fn held_mail_is_not_pushed_until_the_catch_up_releases_it() {
    let db = per_binding();
    db.execute(
        "INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,state) VALUES ('bind-A','s','t',0,0,'active')",
        [],
    )
    .unwrap();
    deliver(&db, 1);
    for binding in ["bind-A", "bind-B"] {
        let run = bdigest(&db, binding);
        assert!(mail_ids(&run).is_empty(), "{binding} is held");
        assert!(run.digest.token.mail.is_none(), "{binding}");
    }
    db.execute(
        "UPDATE catch_up SET state='ended',end_reason='ready',ended_at=5,release_seq=30 WHERE binding_id='bind-A'",
        [],
    )
    .unwrap();
    for binding in ["bind-A", "bind-B"] {
        let run = bdigest(&db, binding);
        assert_eq!(mail_ids(&run), ["m1"], "{binding} is released");
        assert_eq!(
            run.digest.token.mail,
            Some((30, 0)),
            "{binding}: the release key"
        );
    }
}

/// The occupant a hook-pull settles for: `rcv`'s canonical open binding.
fn rcv_binding(db: &Connection) -> (u64, String, String) {
    db.query_row(
        "SELECT s.generation,b.native_session,b.execution_id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id='rcv'",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

fn rcv_binding_id(db: &Connection) -> crate::protocol::ids::BindingId {
    crate::protocol::ids::BindingId::new(db.query_row(
        "SELECT binding_id FROM occupant_bindings WHERE seat_id='rcv' AND ended_at IS NULL ORDER BY ordinal DESC LIMIT 1", [], |r|r.get::<_,String>(0)
    ).unwrap())
}

fn notice_refs(db: &Connection, limit: usize) -> Vec<crate::protocol::results::WarningRef> {
    notice_offer_page(db, "rcv", limit)
        .unwrap()
        .into_iter()
        .map(|offered| offered.notice)
        .collect()
}

#[test]
fn newest_pending_notice_follows_the_frontier() {
    let (_context, db, _guard) = axis_history(Axis::ProgrammaticUndelivered, 3);
    let page = notice_offer_page(&db, "rcv", 10).unwrap();
    let total = page.len();
    assert!(total >= 3, "the fixture projects the live warning too");
    let newest = page.last().unwrap().ordinal;
    assert_eq!(newest_pending_notice(&db, "rcv").unwrap(), Some(newest));
    let (generation, session, _) = rcv_binding(&db);
    let carried = notice_refs(&db, 2);
    assert!(
        settle_pulled_notices(
            &db,
            "i",
            "rcv",
            &rcv_binding_id(&db),
            generation,
            &session,
            &carried
        )
        .unwrap()
    );
    assert_eq!(
        newest_pending_notice(&db, "rcv").unwrap(),
        Some(newest),
        "notices are still above the frontier"
    );
    let rest = notice_refs(&db, 10);
    assert_eq!(rest.len(), total - 2);
    assert!(
        settle_pulled_notices(
            &db,
            "i",
            "rcv",
            &rcv_binding_id(&db),
            generation,
            &session,
            &rest
        )
        .unwrap()
    );
    assert_eq!(newest_pending_notice(&db, "rcv").unwrap(), None);
    assert_eq!(
        newest_pending_notice(&db, "o").unwrap(),
        None,
        "no projection, no notice"
    );
}

#[test]
fn settle_pulled_notices_settles_exactly_the_carried_prefix_for_the_open_binding() {
    let (_context, db, _guard) = axis_history(Axis::ProgrammaticUndelivered, 4);
    let (generation, session, execution) = rcv_binding(&db);
    let all = notice_refs(&db, 10);
    let prefix = all[..2].to_vec();
    assert!(
        settle_pulled_notices(
            &db,
            "i",
            "rcv",
            &rcv_binding_id(&db),
            generation,
            &session,
            &prefix
        )
        .unwrap()
    );
    // Session-keyed bindings: the frontier row is the canonical open binding's own.
    let (binding, binding_execution): (String, String) = db
        .query_row(
            "SELECT o.binding_id,b.execution_id FROM digest_notice_offer o JOIN occupant_bindings b ON b.binding_id=o.binding_id WHERE o.seat_id='rcv'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let open: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='rcv' AND ended_at IS NULL AND native_session=?1",
            [&session],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(binding, open);
    assert_eq!(
        binding_execution, execution,
        "the canonical binding's own row, not a client value"
    );
    assert_eq!(
        notice_refs(&db, 10),
        all[2..].to_vec(),
        "exactly the prefix left the page"
    );
    let projected: i64 = db
        .query_row(
            "SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='rcv'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        projected as usize,
        all.len(),
        "no projection row is deleted"
    );
    assert!(
        !settle_pulled_notices(
            &db,
            "i",
            "rcv",
            &rcv_binding_id(&db),
            generation,
            &session,
            &[]
        )
        .unwrap()
    );
}

#[test]
fn settle_pulled_notices_refuses_a_stale_generation_another_session_or_a_non_prefix() {
    let (_context, db, _guard) = axis_history(Axis::ProgrammaticUndelivered, 3);
    let (generation, session, _) = rcv_binding(&db);
    let all = notice_refs(&db, 10);
    let frontier = || -> i64 {
        db.query_row(
            "SELECT COALESCE(MAX(offered_ordinal),0) FROM digest_notice_offer WHERE seat_id='rcv'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let before = frontier();
    for (label, at_generation, native, carried) in [
        (
            "stale generation",
            generation + 1,
            session.as_str(),
            all[..1].to_vec(),
        ),
        (
            "another session",
            generation,
            "other-session",
            all[..1].to_vec(),
        ),
        (
            "not the oldest",
            generation,
            session.as_str(),
            all[1..2].to_vec(),
        ),
        (
            "not a prefix of the page",
            generation,
            session.as_str(),
            vec![all[0].clone(), all[2].clone()],
        ),
        ("more than the page holds", generation, session.as_str(), {
            let mut more = all.clone();
            more.push(all[0].clone());
            more
        }),
    ] {
        assert!(
            !settle_pulled_notices(
                &db,
                "i",
                "rcv",
                &rcv_binding_id(&db),
                at_generation,
                native,
                &carried
            )
            .unwrap(),
            "{label}"
        );
        assert_eq!(frontier(), before, "{label}: frontier unchanged");
    }
    assert!(
        !settle_pulled_notices(
            &db,
            "other-instance",
            "rcv",
            &rcv_binding_id(&db),
            generation,
            &session,
            &all[..1]
        )
        .unwrap()
    );
    assert_eq!(frontier(), before);
}

// The following fixtures exercise the canonical presentation relevance helper.
// They use this existing private attention host; no public test-only accessor.
fn relevance_claim() -> crate::protocol::authority::CallerClaim {
    use crate::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        ids::{ExecutionId, HostTargetId, NativeSessionId},
    };
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("relevance-session"),
        execution: ExecutionId::new("relevance-execution"),
        target: HostTargetId::new("pane-s"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}
fn relevance_db() -> Connection {
    let db = empty();
    db.execute_batch("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s',1,1,'pane-s','host',1,'codex','relevance-session','relevance-execution','cooperative_top_level',0,0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('relevance-thread','i','topic','goal',0,0);").unwrap();
    db
}
fn relevance_budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}
fn relevance_pull(db: &Connection) -> crate::store::presentation_relevance::RelevanceProof {
    use crate::protocol::delivery::PresentationConsumer;
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let proof = crate::store::presentation_relevance::advance(
        db,
        &relevance_claim(),
        PresentationConsumer::Hook,
        0,
        &relevance_budget(),
    )
    .unwrap();
    db.execute_batch("COMMIT").unwrap();
    assert!(proof.work.sources + proof.work.fallback <= 100);
    assert!(proof.work.highwater_seeks <= 26);
    assert!(proof.work.point_probes <= 6400);
    assert!(proof.work.witness_probes <= 64);
    proof
}

#[test]
fn presentation_relevance_required_lazy_overlay_tracks_digest_and_requirement() {
    let db = relevance_db();
    db.execute_batch("INSERT INTO service_authors(id,instance_id,created_at) VALUES ('relevance-service','i',0);
        UPDATE threads SET managed_owner_author_id='relevance-service' WHERE id='relevance-thread';
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,confirmation,inviter_seat_id) VALUES ('relevance-invite','relevance-thread','s',1,'pending',0,2,100,100,'lazy','o');").unwrap();
    let count = || {
        db.query_row(
            "SELECT count(*) FROM presentation_confirming_invitations WHERE seat_id='s'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(count(), 0);
    db.execute_batch("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES ('relevance-requirement','relevance-thread','s','relevance-service','relevance-invite','pending',3,0);").unwrap();
    assert_eq!(count(), 1);
    assert_eq!(
        relevance_pull(&db).relevance,
        crate::store::presentation_relevance::Relevance::Open
    );
    db.execute_batch("DELETE FROM digest_pending_invitations WHERE seat_id='s' AND invitation_id='relevance-invite';").unwrap();
    assert_eq!(count(), 0);
    db.execute_batch("INSERT INTO digest_pending_invitations(seat_id,invitation_id,thread_id,created_decision_seq,ordinal) SELECT seat_id,id,thread_id,created_decision_seq,ordinal FROM invitations WHERE id='relevance-invite';
        UPDATE requirement_episodes SET revision=revision+1,state='accepted',accepted_by_seat_id='s',accepted_generation=1,accepted_observation='cooperative',accepted_at=1 WHERE id='relevance-requirement';").unwrap();
    assert_eq!(
        count(),
        1,
        "accepted requirement remains effective while invitation remains pending"
    );
    db.execute_batch("UPDATE requirement_episodes SET revision=revision+1,state='released',released_at=2 WHERE id='relevance-requirement';").unwrap();
    assert_eq!(count(), 0);
}

#[test]
fn presentation_relevance_quiet_proof_does_not_fence_itself_and_overflow_aborts_writer() {
    use crate::store::presentation_relevance::Relevance;
    let db = relevance_db();
    let fence = || {
        db.query_row(
            "SELECT version FROM presentation_warning_fence WHERE singleton=1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = fence();
    assert_eq!(relevance_pull(&db).relevance, Relevance::Absent);
    assert_eq!(
        fence(),
        before,
        "private proof write must not invalidate absence"
    );
    let quiet = relevance_pull(&db);
    assert_eq!(quiet.relevance, Relevance::Absent);
    assert_eq!(
        quiet.work.sources + quiet.work.fallback + quiet.work.highwater_seeks,
        0
    );
    assert_eq!(fence(), before);
    db.execute(
        "UPDATE presentation_warning_fence SET version=?1 WHERE singleton=1",
        [i64::MAX],
    )
    .unwrap();
    assert!(db.execute("INSERT INTO memberships(thread_id,seat_id,state) VALUES ('relevance-thread','s','invited')",[]).is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM memberships WHERE thread_id='relevance-thread'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(fence(), i64::MAX);
}

#[test]
fn presentation_relevance_walks_retained_membership_and_revalidates_exact_witness() {
    use crate::{
        protocol::delivery::PresentationConsumer,
        store::presentation_relevance::{self, Relevance},
    };
    let db = relevance_db();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('relevance-other-invite','relevance-thread','o',1,'pending',0,2,100,100);
        WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<257)
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'unpublished-'||x,20,'relevance-thread',1000000,'o','invitation','relevance-other-invite' FROM n;
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES ('relevance-warning','i','relevance-thread',1,'warn','{}',0,20,1);
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('relevance-warning',20,'relevance-thread',1000000,'o','invitation','relevance-other-invite');
        WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<301)
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) SELECT 'relevance-thread','s',x,CASE WHEN x=301 THEN 1 ELSE 50 END,CASE WHEN x=301 THEN 30 ELSE 60 END FROM n;").unwrap();
    let mut sources = 0u64;
    let mut fallback = 0u64;
    let mut opened = false;
    for _ in 0..1000 {
        let proof = relevance_pull(&db);
        sources += u64::from(proof.work.sources);
        fallback += u64::from(proof.work.fallback);
        assert_ne!(
            proof.relevance,
            Relevance::Absent,
            "retained witness has not been exhausted"
        );
        if proof.relevance == Relevance::Open {
            opened = true;
            break;
        }
        assert!(proof.more);
    }
    assert!(opened, "finite captured source/history must converge");
    assert!(
        sources >= 258 && fallback >= 301,
        "charge physical candidates including unpublished/invalid ones"
    );
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let exact = presentation_relevance::revalidate(
        &db,
        &relevance_claim(),
        PresentationConsumer::Hook,
        0,
        &relevance_budget(),
    )
    .unwrap();
    db.execute_batch("COMMIT").unwrap();
    assert_eq!(exact.relevance, Relevance::Open);
    assert_eq!(
        exact.work.sources + exact.work.fallback + exact.work.highwater_seeks,
        0
    );
    assert!(exact.work.witness_probes > 0 && exact.work.witness_probes <= 64);
    db.execute("UPDATE membership_intervals SET left_seq=2 WHERE thread_id='relevance-thread' AND seat_id='s' AND episode=301",[]).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let uncertain = presentation_relevance::revalidate(
        &db,
        &relevance_claim(),
        PresentationConsumer::Hook,
        0,
        &relevance_budget(),
    )
    .unwrap();
    db.execute_batch("COMMIT").unwrap();
    assert_eq!(
        uncertain.relevance,
        Relevance::Unknown,
        "lost witness alone is not exact absence"
    );
    assert!(uncertain.more);
    let mut closed = false;
    for _ in 0..1000 {
        if relevance_pull(&db).relevance == Relevance::Absent {
            closed = true;
            break;
        }
    }
    assert!(
        closed,
        "quiescent full proof must converge after witness loss"
    );
}

// Kills global negative-fence stability being required for a positive retained
// direct warning. The ordinary Lazy writes use the real stage/publication
// producers, rather than incrementing the relevance fence by hand.
fn relevance_publication_binding(db: &Connection) -> BindingId {
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) SELECT 'o',1,1,'pane-o','host',1,'codex','relevance-author-session','relevance-author-execution','cooperative_top_level',0,0 WHERE NOT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id='o' AND harness='codex' AND native_session='relevance-author-session' AND agent_id='')", []).unwrap();
    BindingId::new(db.query_row("SELECT binding_id FROM occupant_bindings WHERE seat_id='o' AND harness='codex' AND native_session='relevance-author-session' AND agent_id=''", [], |r|r.get::<_,String>(0)).unwrap())
}

fn relevance_publish_unrelated_lazy(db: &mut Connection, n: u32) {
    use crate::store::messages::{PublicationAuthor, insert_publication, stage_recipient};
    let tx = db.transaction().unwrap();
    let preparation = format!("prep-relevance-churn-{n}");
    let operation = format!("relevance-churn-{n}");
    tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,'i','seat:o',?2,zeroblob(32),'relevance-unrelated',0,0,0,0,0,0,0,'building','lazy')",params![preparation,operation]).unwrap();
    let (mut recipients, mut warnings) = (0, 0);
    stage_recipient(
        &tx,
        &preparation,
        "i",
        &ThreadId::new("relevance-unrelated"),
        "relevance-third",
        crate::store::messages::Staging {
            duration: 100,
            priority: crate::protocol::priority::DeliveryPriority::DEFAULT,
            ack: false,
        },
        &mut recipients,
        &mut warnings,
    )
    .unwrap();
    assert_eq!((recipients, warnings), (0, 0));
    tx.execute(
        "UPDATE send_preparations SET status='sealed' WHERE id=?1",
        [&preparation],
    )
    .unwrap();
    let publication = insert_publication(
        &tx,
        i64::from(n) + 100,
        "seat:o",
        &operation,
        &[0; 32],
        &ThreadId::new("relevance-unrelated"),
        "unrelated lazy publication",
        PublicationAuthor::Native {
            seat: &SeatId::new("o"),
            binding: &relevance_publication_binding(&tx),
            observation: "cooperative_top_level",
            relays_user: false,
            user_intent: None,
        },
    )
    .unwrap();
    let row: (String, String, i64) = tx.query_row("SELECT m.delivery_mode,r.seat_id,(SELECT count(*) FROM receipts WHERE message_id=m.id) FROM messages m JOIN lazy_recipients r ON r.message_id=m.id JOIN send_manifests sm ON sm.message_id=m.id WHERE m.id=?1",[publication.message.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row, ("lazy".into(), "relevance-third".into(), 0));
    tx.commit().unwrap();
}

fn relevance_legacy_membership_fixture() -> Connection {
    let db = relevance_db();
    db.execute_batch("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('relevance-third','i','resolved','native','pane-third',1,1,0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('relevance-unrelated','i','unrelated','unrelated',0,0);
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('relevance-unrelated','o','joined'),('relevance-unrelated','relevance-third','joined');
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('relevance-unrelated','o',1,1),('relevance-unrelated','relevance-third',1,1);
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('relevance-churn-invite','relevance-thread','o',1,'pending',0,2,100,100);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset,source_invitation_id) VALUES ('relevance-churn-warning','i','relevance-thread',1,'warn','{}',0,20,1,'relevance-churn-invite');
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('relevance-churn-warning',20,'relevance-thread',1000000,'o','invitation','relevance-churn-invite');
        WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<301)
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) SELECT 'relevance-thread','s',x,CASE WHEN x=301 THEN 1 ELSE 50 END,CASE WHEN x=301 THEN 30 ELSE 60 END FROM n;").unwrap();
    db
}

// Suspend before the only qualifying interval. The cursor and high-water
// values below come from persisted proof/source rows, not fixture row counts.
fn relevance_suspend_legacy_membership(db: &Connection) -> (i64, i64) {
    use crate::store::presentation_relevance::Relevance;
    let qualifying: i64 = db.query_row("SELECT ordinal FROM membership_intervals WHERE thread_id='relevance-thread' AND seat_id='s' AND episode=301", [], |r| r.get(0)).unwrap();
    for _ in 0..32 {
        let proof = relevance_pull(db);
        assert_eq!(proof.relevance, Relevance::Unknown);
        let raw: String = db.query_row("SELECT proof_json FROM presentation_warning_proofs WHERE seat_id='s' AND consumer='hook'", [], |r| r.get(0)).unwrap();
        let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
        if let Some(after) = state["slots"][8]["judge"]["Membership"]["after"].as_i64()
            && after > 1
            && after + 1 < qualifying
        {
            assert_eq!(
                state["slots"][8]["judge"]["Membership"]["high"].as_i64(),
                Some(1_000_000)
            );
            return (after, qualifying);
        }
    }
    panic!("fixture must suspend an unfinished direct membership walk");
}

#[test]
fn presentation_relevance_suspended_membership_respects_repaired_current_high() {
    use crate::store::presentation_relevance::Relevance;
    // No normal producer decreases this high-water. These are explicit retained
    // operator-repair states whose current inputs must still govern first Open.
    for position in [-1, 0, 1] {
        let db = relevance_legacy_membership_fixture();
        let (after, qualifying) = relevance_suspend_legacy_membership(&db);
        let repaired_high = after + position;
        assert!(repaired_high < qualifying);
        db.execute("UPDATE warning_jobs SET interval_high_water=?1 WHERE warning_id='relevance-churn-warning'", [repaired_high]).unwrap();
        let mut absent = false;
        for _ in 0..256 {
            let proof = relevance_pull(&db);
            assert_ne!(
                proof.relevance,
                Relevance::Open,
                "a captured frontier cannot authorize an interval above current high (cursor offset {position})"
            );
            if proof.relevance == Relevance::Absent {
                absent = true;
                break;
            }
        }
        assert!(absent, "repaired finite membership range must exhaust");
    }
}

#[test]
fn presentation_relevance_suspended_membership_unchanged_high_still_opens() {
    use crate::store::presentation_relevance::Relevance;
    let db = relevance_legacy_membership_fixture();
    let (_, qualifying) = relevance_suspend_legacy_membership(&db);
    let mut opened = false;
    for _ in 0..256 {
        let proof = relevance_pull(&db);
        assert_ne!(proof.relevance, Relevance::Absent);
        if proof.relevance == Relevance::Open {
            opened = true;
            break;
        }
    }
    assert!(
        opened,
        "unchanged captured/current range still finds its recipient"
    );
    let raw: String = db.query_row("SELECT proof_json FROM presentation_warning_proofs WHERE seat_id='s' AND consumer='hook'", [], |r| r.get(0)).unwrap();
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(state["witness"]["interval"].as_i64(), Some(qualifying));
    assert_eq!(relevance_pull(&db).relevance, Relevance::Open);
}

#[test]
fn presentation_relevance_direct_legacy_survives_canonical_lazy_churn() {
    use crate::store::presentation_relevance::Relevance;
    let mut db = relevance_legacy_membership_fixture();
    let shape: (i64, i64, i64) = db.query_row("SELECT (SELECT count(*) FROM warning_conditions),(SELECT count(*) FROM invitations WHERE seat_id='s'),(SELECT count(*) FROM warning_recipients WHERE warning_id='relevance-churn-warning' AND seat_id='s')",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(shape, (0, 0, 0), "retained pre-ledger membership authority");
    let mut opened = false;
    for n in 0..256 {
        let before: i64 = db
            .query_row(
                "SELECT version FROM presentation_warning_fence WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        relevance_publish_unrelated_lazy(&mut db, n);
        let after: i64 = db
            .query_row(
                "SELECT version FROM presentation_warning_fence WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            after > before,
            "real unrelated publication must mutate the fence"
        );
        let proof = relevance_pull(&db);
        assert_ne!(
            proof.relevance,
            Relevance::Absent,
            "the invitation and historical recipient remain actionable"
        );
        if proof.relevance == Relevance::Open {
            opened = true;
            break;
        }
    }
    assert!(
        opened,
        "canonical unrelated Lazy publications must not prevent finite retained direct membership discovery"
    );
    relevance_publish_unrelated_lazy(&mut db, 256);
    let witnessed = relevance_pull(&db);
    assert_eq!(
        witnessed.relevance,
        Relevance::Open,
        "unrelated Lazy publication must preserve exact positive witness"
    );
    assert_eq!(
        witnessed.work.sources + witnessed.work.fallback + witnessed.work.highwater_seeks,
        0,
        "legacy invitation witness needs no condition coverage or discovery"
    );
}

// Kills a physically open condition being treated as actionable after its
// canonical close sweep but before the budgeted clear worker publishes.
#[test]
fn presentation_relevance_canonical_receipt_sweep_closes_before_clear_publication() {
    use crate::protocol::{authority::ObligationRef, time::UtcMillis};
    use crate::store::presentation_relevance::Relevance;
    let mut db = relevance_db();
    db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('relevance-close-message','i','relevance-thread',1,'ordinary','body',0,2);
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('relevance-close-message','relevance-thread','s','pending',100,0,100);
        UPDATE threads SET next_sequence=2 WHERE id='relevance-thread';").unwrap();
    let warning = {
        let tx = db.transaction().unwrap();
        let overdue = schema::record_overdue_if_pending(
            &tx,
            &ObligationRef::Receipt {
                message: MessageId::new("relevance-close-message"),
                seat: SeatId::new("s"),
            },
            &crate::ports::TimeBasis::Decision,
            UtcMillis(101),
        )
        .unwrap();
        assert!(overdue.inserted);
        let warning = overdue.warning.unwrap();
        tx.commit().unwrap();
        warning
    };
    let initial = relevance_pull(&db);
    assert_eq!(
        initial.relevance,
        Relevance::Open,
        "the real producer's condition is initially actionable"
    );
    assert_eq!(
        initial.work.sources, 1,
        "positive early exit examines one actual source"
    );
    assert_eq!(
        initial.work.highwater_seeks, 15,
        "thirteen captures plus source and empty coverage seek share one meter"
    );
    let witness = relevance_pull(&db);
    assert_eq!(witness.relevance, Relevance::Open);
    assert_eq!(
        witness.work.sources + witness.work.fallback,
        0,
        "empty coverage seek is not a fictitious physical row"
    );
    assert_eq!(
        witness.work.highwater_seeks, 1,
        "condition witness honestly charges its indexed range seek"
    );
    assert!(witness.work.witness_probes > 0 && witness.work.witness_probes <= 64);
    {
        let tx = db.transaction().unwrap();
        let cutoff = schema::next_decision_seq(&tx, "i").unwrap();
        tx.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',?1,1,102)",[cutoff as i64]).unwrap();
        assert_eq!(
            schema::clear_waived_receipt_conditions_for_seat(&tx, "s", UtcMillis(102)).unwrap(),
            1
        );
        tx.commit().unwrap();
    }
    let state: (i64, i64, String) = db.query_row("SELECT c.clear_warning_id IS NULL,(SELECT count(*) FROM warning_close_sweeps s WHERE s.condition_kind='receipt' AND s.affected_seat_id=c.affected_seat_id AND s.episode IS NULL AND s.after_ordinal<c.ordinal AND c.ordinal<=s.through_ordinal),(SELECT state FROM receipts WHERE message_id='relevance-close-message' AND seat_id='s') FROM warning_conditions c WHERE c.open_warning_id=?1",[warning.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        state,
        (1, 1, "pending".into()),
        "canonical sweep exists while clear worker and receipt reconciliation remain undrained"
    );
    let mut absent = false;
    for _ in 0..64 {
        let proof = relevance_pull(&db);
        assert_ne!(
            proof.relevance,
            Relevance::Open,
            "canonical logical closure must invalidate positive relevance before physical clear publication"
        );
        if proof.relevance == Relevance::Absent {
            absent = true;
            break;
        }
    }
    assert!(
        absent,
        "quiescent covered union must converge to exact absence"
    );
    assert!(
        db.query_row(
            "SELECT clear_warning_id IS NULL FROM warning_conditions WHERE open_warning_id=?1",
            [warning.as_str()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        "the helper must not materialize the clear itself"
    );
}

// Kills a prepared same-ID representation shadowing a currently actionable
// independent direct source. Boolean relevance preserves both associations.
#[test]
fn presentation_relevance_direct_association_survives_prepared_same_id() {
    use crate::store::presentation_relevance::Relevance;
    let db = relevance_db();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('relevance-union-invite','relevance-thread','o',1,'pending',0,2,100,100);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset,source_invitation_id) VALUES ('relevance-union-warning','i','relevance-thread',1,'warn','{}',0,20,1,'relevance-union-invite');
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES ('relevance-union-warning',20,'relevance-thread',0,'o','invitation','relevance-union-invite');
        INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('relevance-union-warning','s',20);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('relevance-union-other','i','other','other',0,0);
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('prep-relevance-union','i','seat:o','union',zeroblob(32),'relevance-union-other',0,0,0,0,0,0,0,'sealed');
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('prep-relevance-union','union-key','relevance-union-warning','o',1,1,'{}');
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('msg-relevance-union','i','relevance-union-other',1,'ordinary','other',0,21);
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-relevance-union','msg-relevance-union','i','relevance-union-other',21,0,1,0,0,1);").unwrap();
    db.execute("UPDATE seats SET unavailability_open=0 WHERE id='o'", [])
        .unwrap();
    // This is an explicit retained coexistence fixture, not a claim that the
    // post-ledger publication producer admits conflicting warning job IDs.
    assert_eq!(
        db.query_row(
            "SELECT unavailability_open FROM seats WHERE id='o'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let mut open = false;
    for _ in 0..64 {
        if relevance_pull(&db).relevance == Relevance::Open {
            open = true;
            break;
        }
    }
    assert!(
        open,
        "nonactionable prepared association cannot erase actionable direct invitation source"
    );
    db.execute(
        "UPDATE warning_jobs SET event_seq=21 WHERE warning_id='relevance-union-warning'",
        [],
    )
    .unwrap();
    let mut absent = false;
    for _ in 0..64 {
        let proof = relevance_pull(&db);
        assert_ne!(
            proof.relevance,
            Relevance::Open,
            "a job no longer matching its immutable direct event cannot remain positive authority"
        );
        if proof.relevance == Relevance::Absent {
            absent = true;
            break;
        }
    }
    assert!(
        absent,
        "neither retained association remains actionable after exact direct source mismatch"
    );
}

// One real receipt condition on a fresh thread, deliberately left physically
// open. A current waiver closes its canonical scope without draining workers.
fn relevance_append_covered_receipt(db: &mut Connection, n: u32) {
    use crate::protocol::time::UtcMillis;
    use crate::{ports::TimeBasis, protocol::authority::ObligationRef};
    let tx = db.transaction().unwrap();
    let thread = format!("relevance-covered-thread-{n}");
    let message = format!("relevance-covered-message-{n}");
    tx.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES (?1,'i','covered','covered',0,0,2)",[&thread]).unwrap();
    let seq = schema::next_decision_seq(&tx, "i").unwrap();
    tx.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,actor_seat_id) VALUES (?1,'i',?2,1,'ordinary','covered',0,?3,'o')",params![message,thread,seq as i64]).unwrap();
    tx.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,?2,'s','pending',100,0,100)",params![message,thread]).unwrap();
    assert!(
        schema::record_overdue_if_pending(
            &tx,
            &ObligationRef::Receipt {
                message: MessageId::new(&message),
                seat: SeatId::new("s")
            },
            &TimeBasis::Decision,
            UtcMillis(1000 + i64::from(n))
        )
        .unwrap()
        .inserted
    );
    let cutoff = schema::next_decision_seq(&tx, "i").unwrap();
    tx.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',?1,1,?2) ON CONFLICT(seat_id) DO UPDATE SET through_decision_seq=excluded.through_decision_seq,decided_at=excluded.decided_at",params![cutoff as i64,1000+i64::from(n)]).unwrap();
    assert_eq!(
        schema::clear_waived_receipt_conditions_for_seat(&tx, "s", UtcMillis(1000 + i64::from(n)))
            .unwrap(),
        1
    );
    tx.commit().unwrap();
}

#[test]
fn presentation_relevance_coverage_reserves_fallback_progress_under_source_load() {
    use crate::store::presentation_relevance::Relevance;
    let mut db = relevance_legacy_membership_fixture();
    for n in 0..64 {
        relevance_append_covered_receipt(&mut db, n);
    }
    let legacy_ordinal: i64 = db
        .query_row(
            "SELECT ordinal FROM warning_jobs WHERE warning_id='relevance-churn-warning'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut prior_after = 0;
    let mut suspended_progress = 0;
    let mut source_rows = 0u64;
    let mut fallback_rows = 0u64;
    let mut open = false;
    for n in 0..512 {
        relevance_publish_unrelated_lazy(&mut db, n);
        let proof = relevance_pull(&db);
        source_rows += u64::from(proof.work.sources);
        fallback_rows += u64::from(proof.work.fallback);
        assert_ne!(
            proof.relevance,
            Relevance::Absent,
            "covered receipts do not settle the independent invitation"
        );
        if proof.relevance == Relevance::Open {
            open = true;
            break;
        }
        let raw:String=db.query_row("SELECT proof_json FROM presentation_warning_proofs WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
        let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let slot = &state["slots"][8];
        if slot["candidate"]["ordinal"].as_i64() == Some(legacy_ordinal)
            && let Some(after) = slot["judge"]["Membership"]["after"].as_i64()
        {
            assert!(
                after >= prior_after,
                "unrelated writes must not reset the independent membership cursor"
            );
            if after > prior_after {
                suspended_progress += 1;
            }
            prior_after = after;
        }
    }
    assert!(
        open,
        "coverage-heavy source traffic must lend and reserve enough fallback work for finite Open"
    );
    assert!(
        source_rows >= 64 && fallback_rows >= 301 && suspended_progress > 1,
        "exercise multiple charged source/fallback quanta"
    );
}

fn relevance_seed_stale_sources(db: &Connection, count: u32) {
    for n in 0..count {
        let first = 2 * n + 1;
        let second = first + 1;
        let first_event = first + 1;
        let second_event = second + 1;
        db.execute_batch(&format!("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('relevance-stale-a-{n}','i','relevance-thread',{first},'ordinary','stale',0,{first_event}),('relevance-stale-b-{n}','i','relevance-thread',{second},'ordinary','stale',0,{second_event});
        INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq,invitation_originator_instance_id,invitation_originator_seat_id) VALUES ('invitation','relevance-thread','stale-invitation-{n}','s','relevance-stale-a-{n}',{first_event},'i','s');
        INSERT INTO receipt_warning_origins(instance_id,seat_id,warning_id,source_message_id) VALUES ('i','s','relevance-stale-a-{n}','relevance-stale-b-{n}');
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,status) VALUES ('relevance-stale-a-{n}',{first_event},'relevance-thread',0,'s','invitation','stale-invitation-{n}','pending'),('relevance-stale-b-{n}',{second_event},'relevance-thread',0,'s','invitation','stale-invitation-{n}','failed');
        INSERT OR IGNORE INTO digest_open_warnings(source,source_ordinal,warning_id,thread_id,affected_seat_id,condition_kind,condition_id) SELECT 'job',ordinal,warning_id,thread_id,'s',condition_kind,condition_id FROM warning_jobs WHERE warning_id IN ('relevance-stale-a-{n}','relevance-stale-b-{n}');
        INSERT OR IGNORE INTO digest_open_warning_recipients(seat_id,warning_id,thread_id,source,source_ordinal) SELECT 's',warning_id,thread_id,'job',ordinal FROM warning_jobs WHERE warning_id IN ('relevance-stale-a-{n}','relevance-stale-b-{n}');
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('prep-relevance-stale-{n}','i','seat:o','stale-{n}',zeroblob(32),'relevance-thread',0,0,0,0,0,0,0,'sealed');
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('prep-relevance-stale-{n}','stale-prepared-{n}','relevance-stale-prepared-{n}','s',1,1,'{{}}');
        INSERT OR IGNORE INTO digest_open_warnings(source,source_ordinal,warning_id,thread_id,affected_seat_id,condition_kind,condition_id,episode) SELECT 'prepared',ordinal,warning_id,'relevance-thread','s','unavailable',warning_key,1 FROM prepared_unavailable_warnings WHERE preparation_id='prep-relevance-stale-{n}';
        INSERT OR IGNORE INTO digest_open_warning_recipients(seat_id,warning_id,thread_id,source,source_ordinal) SELECT 's',warning_id,'relevance-thread','prepared',ordinal FROM prepared_unavailable_warnings WHERE preparation_id='prep-relevance-stale-{n}';
        INSERT INTO work_jobs(id,kind,subject_id,high_water,status) VALUES ('relevance-stale-work-a-{n}','send_attention','service-notify:relevance-stale-notify-a-{n}',0,'pending'),('relevance-stale-work-b-{n}','send_attention','service-notify:relevance-stale-notify-b-{n}',0,'failed');")).unwrap();
    }
}

#[test]
fn presentation_relevance_all_thirteen_physical_sources_progress_through_stale_candidates() {
    use crate::store::presentation_relevance::Relevance;
    let db = relevance_db();
    // Deliberately stale derived/source candidates: immutable ordinary message
    // identities cannot be current warnings. This exercises physical source
    // rotation, not a fabricated canonical swept invitation/programmatic warn.
    relevance_seed_stale_sources(&db, 1);
    assert_eq!(db.query_row("SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='s' AND warning_id='relevance-stale-a-0'",[],|r|r.get::<_,i64>(0)).unwrap(),1,"canonical source7 projection already exists");
    let mut absent = false;
    let mut rows = 0u64;
    for _ in 0..13 {
        let proof = relevance_pull(&db);
        rows += u64::from(proof.work.sources);
        assert_ne!(
            proof.relevance,
            Relevance::Open,
            "stale candidates confer no warning authority"
        );
        if proof.relevance == Relevance::Absent {
            absent = true;
            break;
        }
    }
    assert!(
        absent && rows >= 13,
        "all thirteen physical sources and suspended empty fallback must finish"
    );
    let raw:String=db.query_row("SELECT proof_json FROM presentation_warning_proofs WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let state: serde_json::Value = serde_json::from_str(&raw).unwrap();
    for slot in state["slots"].as_array().unwrap() {
        assert!(
            slot["high"].as_i64().unwrap() > 0,
            "fixture must populate each fixed physical source"
        );
        assert_eq!(slot["exhausted"], true);
        assert!(slot["after"]["ordinal"].as_i64().unwrap() > 0);
    }
}

// Observe the actual advance() statements; no test copy of the walker's SQL.
// A missing source, unbounded MAX/range scan, or sort must fail this matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelevanceStatementKind {
    SourceHead,
    SourceRange,
    Representations,
    Membership,
    Coverage,
}
fn relevance_statement_kind(sql: &str) -> Option<RelevanceStatementKind> {
    if sql.starts_with("SELECT COALESCE(MAX(") && sql.contains(" INDEXED BY ") {
        Some(RelevanceStatementKind::SourceHead)
    } else if sql.starts_with("SELECT ") && sql.contains(",rowid FROM ") {
        Some(RelevanceStatementKind::SourceRange)
    } else if sql.starts_with("SELECT ") && sql.ends_with(" LIMIT 1") {
        // Recognize the actual emitted auxiliary range by its dedicated index;
        // its SQL and parameters are still captured from SQLite, never copied.
        if sql.contains(" INDEXED BY presentation_prepared_warning ") {
            Some(RelevanceStatementKind::Representations)
        } else if sql.contains(" INDEXED BY presentation_membership_history ") {
            Some(RelevanceStatementKind::Membership)
        } else if sql.contains(" INDEXED BY warning_close_sweeps_cover ") {
            Some(RelevanceStatementKind::Coverage)
        } else {
            None
        }
    } else {
        None
    }
}
#[derive(Default)]
struct RelevanceStatementWork {
    sql: String,
    first_expanded: String,
    last_expanded: String,
    executions: u64,
    rows: u64,
    max_vm: i32,
    max_scan: i32,
    max_sort: i32,
}
#[derive(Default)]
struct RelevanceTraceState {
    all_selects: bool,
    work: std::collections::BTreeMap<String, RelevanceStatementWork>,
    rows: u64,
    panicked: bool,
}
struct RelevanceTrace<'a> {
    db: &'a Connection,
    state: Box<std::cell::RefCell<RelevanceTraceState>>,
}
impl<'a> RelevanceTrace<'a> {
    fn install(db: &'a Connection) -> Self {
        let state = Box::new(std::cell::RefCell::new(RelevanceTraceState::default()));
        // The box stays at this address until the callback is unregistered.
        assert_eq!(
            unsafe {
                rusqlite::ffi::sqlite3_trace_v2(
                    db.handle(),
                    rusqlite::ffi::SQLITE_TRACE_STMT
                        | rusqlite::ffi::SQLITE_TRACE_ROW
                        | rusqlite::ffi::SQLITE_TRACE_PROFILE,
                    Some(relevance_statement_trace),
                    (&*state as *const std::cell::RefCell<RelevanceTraceState>)
                        .cast_mut()
                        .cast(),
                )
            },
            rusqlite::ffi::SQLITE_OK
        );
        Self { db, state }
    }
    fn finish(self) -> Vec<RelevanceStatementWork> {
        self.unregister();
        let mut state = self.state.borrow_mut();
        assert!(!state.panicked, "SQLite trace callback panicked");
        std::mem::take(&mut state.work).into_values().collect()
    }
    fn unregister(&self) {
        unsafe {
            rusqlite::ffi::sqlite3_trace_v2(self.db.handle(), 0, None, std::ptr::null_mut());
        }
    }
}
impl Drop for RelevanceTrace<'_> {
    fn drop(&mut self) {
        self.unregister();
    }
}
unsafe extern "C" fn relevance_statement_trace(
    event: u32,
    data: *mut std::ffi::c_void,
    statement: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> i32 {
    use rusqlite::ffi;
    let state = unsafe { &*data.cast::<std::cell::RefCell<RelevanceTraceState>>() };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let stmt = statement.cast::<ffi::sqlite3_stmt>();
        let raw = unsafe { ffi::sqlite3_sql(stmt) };
        if raw.is_null() {
            return;
        }
        let sql = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy();
        if relevance_statement_kind(&sql).is_none()
            && !(state.borrow().all_selects && sql.starts_with("SELECT "))
        {
            return;
        }
        let mut state = state.borrow_mut();
        if event == ffi::SQLITE_TRACE_STMT {
            state.rows = 0;
            for metric in [
                ffi::SQLITE_STMTSTATUS_VM_STEP,
                ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP,
                ffi::SQLITE_STMTSTATUS_SORT,
            ] {
                unsafe {
                    ffi::sqlite3_stmt_status(stmt, metric, 1);
                }
            }
        } else if event == ffi::SQLITE_TRACE_ROW {
            state.rows += 1;
        } else if event == ffi::SQLITE_TRACE_PROFILE {
            let expanded = unsafe { ffi::sqlite3_expanded_sql(stmt) };
            assert!(!expanded.is_null(), "expanded production SQL unavailable");
            let expanded_sql = unsafe { std::ffi::CStr::from_ptr(expanded) }
                .to_string_lossy()
                .into_owned();
            unsafe {
                ffi::sqlite3_free(expanded.cast());
            }
            let rows = state.rows;
            let entry = state.work.entry(sql.into_owned()).or_default();
            if entry.executions == 0 {
                entry.sql = unsafe { std::ffi::CStr::from_ptr(raw) }
                    .to_string_lossy()
                    .into_owned();
                entry.first_expanded = expanded_sql.clone();
            }
            entry.last_expanded = expanded_sql;
            entry.executions += 1;
            entry.rows += rows;
            entry.max_vm = entry
                .max_vm
                .max(unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_VM_STEP, 0) });
            entry.max_scan = entry.max_scan.max(unsafe {
                ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_FULLSCAN_STEP, 0)
            });
            entry.max_sort = entry
                .max_sort
                .max(unsafe { ffi::sqlite3_stmt_status(stmt, ffi::SQLITE_STMTSTATUS_SORT, 0) });
        }
    }));
    if result.is_err() {
        state.borrow_mut().panicked = true;
    }
    0
}

#[test]
fn presentation_relevance_all_thirteen_actual_source_heads_and_ranges_have_bounded_plans_and_vm() {
    use crate::store::presentation_relevance::Relevance;
    // Independent physical-row geometry for one stale fixture group. Two job
    // rows feed each job projection; every other source contributes one row.
    let sources = [
        ("warning_conditions", "affected_seat_id=?1", 1),
        (
            "warning_conditions",
            "invitation_originator_instance_id=?2",
            1,
        ),
        (
            "receipt_warning_origins",
            "instance_id=?2 AND seat_id=?1",
            1,
        ),
        ("digest_open_warning_recipients", "source='job'", 2),
        ("digest_open_warning_recipients", "source='prepared'", 1),
        ("digest_open_warnings", "source='job'", 2),
        ("digest_open_warnings", "source='prepared'", 1),
        ("digest_programmatic_warnings", "seat_id=?1", 1),
        ("warning_jobs", "status='pending'", 1),
        ("warning_jobs", "status='failed'", 1),
        ("work_jobs", "status='pending'", 1),
        ("work_jobs", "status='failed'", 1),
        ("prepared_unavailable_warnings", " WHERE 1 AND ", 1),
    ];
    let mut measurements = Vec::new();
    for count in [32u32, 512] {
        let db = relevance_db();
        relevance_seed_stale_sources(&db, count);
        let trace = RelevanceTrace::install(&db);
        let mut absent = false;
        let mut charged_rows = 0u64;
        let mut charged_fallback = 0u64;
        let mut charged_seeks = 0u64;
        let mut charged_witness = 0u64;
        for _ in 0..count * 4 {
            let proof = relevance_pull(&db);
            charged_rows += u64::from(proof.work.sources);
            charged_fallback += u64::from(proof.work.fallback);
            charged_seeks += u64::from(proof.work.highwater_seeks);
            charged_witness += u64::from(proof.work.witness_probes);
            assert_ne!(
                proof.relevance,
                Relevance::Open,
                "stale candidates cannot confer authority"
            );
            if proof.relevance == Relevance::Absent {
                absent = true;
                break;
            }
        }
        let work = trace.finish();
        assert!(absent, "finite retained source union must finish");
        let source_work: Vec<_> = work
            .iter()
            .filter(|entry| {
                matches!(
                    relevance_statement_kind(&entry.sql),
                    Some(RelevanceStatementKind::SourceHead | RelevanceStatementKind::SourceRange)
                )
            })
            .collect();
        assert_eq!(
            source_work.len(),
            26,
            "thirteen actual heads and thirteen actual ranges"
        );
        assert_eq!(charged_rows, 15 * u64::from(count));
        // The two job projections each contain two stale candidates/group,
        // and pending/failed jobs each contain one. These six candidates have
        // an independent prepared-association alternative: an empty indexed
        // representation seek still consumes the shared seek meter.
        let representations: Vec<_> = work
            .iter()
            .filter(|entry| {
                relevance_statement_kind(&entry.sql)
                    == Some(RelevanceStatementKind::Representations)
            })
            .collect();
        assert_eq!(
            representations.len(),
            1,
            "one emitted prepared-alternative range form"
        );
        let representations = representations[0];
        assert_eq!(representations.executions, 6 * u64::from(count));
        assert_eq!(
            representations.rows, 0,
            "no retained prepared row has a stale job warning id"
        );
        assert!(
            work.iter().all(|entry| !matches!(
                relevance_statement_kind(&entry.sql),
                Some(RelevanceStatementKind::Membership | RelevanceStatementKind::Coverage)
            )),
            "ordinary stale identities need no canonical membership/coverage range"
        );
        assert_eq!(
            charged_fallback, 0,
            "empty representation seeks are not physical fallback rows"
        );
        assert_eq!(charged_witness, 0, "no stale candidate creates a witness");
        assert_eq!(
            work.iter().map(|entry| entry.executions).sum::<u64>(),
            charged_seeks,
            "all source and separately recognized auxiliary seeks share one meter"
        );
        let mut matrix = Vec::new();
        for (table, predicate, multiplier) in sources {
            for head in [true, false] {
                let entries: Vec<_> = source_work
                    .iter()
                    .copied()
                    .filter(|entry| {
                        entry.sql.contains(&format!(" FROM {table} INDEXED BY "))
                            && entry.sql.contains(predicate)
                            && entry.sql.starts_with("SELECT COALESCE(MAX(") == head
                    })
                    .collect();
                assert_eq!(
                    entries.len(),
                    1,
                    "one actual query form: {table}/{predicate}, head={head}"
                );
                let entry = entries[0];
                if head {
                    assert_eq!(
                        (entry.executions, entry.rows),
                        (1, 1),
                        "one captured highwater"
                    );
                } else {
                    assert_eq!(
                        entry.rows,
                        multiplier * u64::from(count),
                        "physical rows: {table}/{predicate}"
                    );
                    assert!(
                        entry.executions <= entry.rows + 1,
                        "at most one terminal empty seek"
                    );
                }
                assert_eq!(entry.max_scan, 0, "full scan: {}", entry.sql);
                assert_eq!(entry.max_sort, 0, "sort: {}", entry.sql);
                assert!(
                    entry.max_vm <= 100,
                    "bounded statement work: {} VM={}",
                    entry.sql,
                    entry.max_vm
                );
                for expanded in [&entry.first_expanded, &entry.last_expanded] {
                    let plan: Vec<String> = db
                        .prepare(&format!("EXPLAIN QUERY PLAN {expanded}"))
                        .unwrap()
                        .query_map([], |row| row.get(3))
                        .unwrap()
                        .collect::<Result<_, _>>()
                        .unwrap();
                    assert!(
                        plan.iter()
                            .any(|line| line.contains("SEARCH ") && line.contains(table)),
                        "actual head/range must seek source: {plan:?}"
                    );
                    assert!(
                        plan.iter()
                            .all(|line| !line.contains("SCAN ") && !line.contains("TEMP B-TREE")),
                        "actual head/range must not scan/sort: {plan:?}"
                    );
                    eprintln!(
                        "retained_groups={count} {table}/{predicate} head={head} executions={} rows={} max_vm={} plan={plan:?}",
                        entry.executions, entry.rows, entry.max_vm
                    );
                }
                matrix.push((entry.sql.clone(), entry.max_vm));
            }
        }
        assert_eq!(
            representations.max_scan, 0,
            "prepared-alternative full scan"
        );
        assert_eq!(representations.max_sort, 0, "prepared-alternative sort");
        assert!(
            representations.max_vm <= 100,
            "bounded prepared-alternative VM work"
        );
        for expanded in [
            &representations.first_expanded,
            &representations.last_expanded,
        ] {
            let plan: Vec<String> = db
                .prepare(&format!("EXPLAIN QUERY PLAN {expanded}"))
                .unwrap()
                .query_map([], |row| row.get(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(
                plan.iter()
                    .any(|line| line.contains("SEARCH prepared_unavailable_warnings")),
                "actual prepared alternative must seek source: {plan:?}"
            );
            assert!(
                plan.iter()
                    .all(|line| !line.contains("SCAN ") && !line.contains("TEMP B-TREE")),
                "actual prepared alternative must not scan/sort: {plan:?}"
            );
            eprintln!(
                "retained_groups={count} prepared-alternative executions={} rows={} max_vm={} plan={plan:?}",
                representations.executions, representations.rows, representations.max_vm
            );
        }
        matrix.push((representations.sql.clone(), representations.max_vm));
        measurements.push(matrix);
    }
    for ((small_sql, small_vm), (large_sql, large_vm)) in
        measurements[0].iter().zip(&measurements[1])
    {
        assert_eq!(
            small_sql, large_sql,
            "same observed statement form at both scales"
        );
        assert!(
            large_vm <= &(small_vm + 8),
            "source statement work must stay flat across 16x history: {small_sql} VM={small_vm}/{large_vm}"
        );
    }
}

#[test]
fn presentation_relevance_successor_plan_and_vm_stay_bounded_in_retained_canonical_sweeps() {
    const SQL: &str = "SELECT after_ordinal,through_ordinal FROM warning_close_sweeps INDEXED BY warning_close_sweeps_cover WHERE condition_kind=?1 AND affected_seat_id=?2 AND episode IS ?3 AND through_ordinal>=?4 ORDER BY through_ordinal,after_ordinal LIMIT 1";
    let mut measured = Vec::new();
    for count in [32u32, 512] {
        let mut db = relevance_db();
        for n in 0..count {
            relevance_append_covered_receipt(&mut db, n);
        }
        let target = i64::from(count / 2);
        let plan: Vec<String> = db
            .prepare(&format!("EXPLAIN QUERY PLAN {SQL}"))
            .unwrap()
            .query_map(params!["receipt", "s", None::<i64>, target], |r| r.get(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            plan.iter()
                .any(|line| line.contains("warning_close_sweeps_cover"))
        );
        assert!(
            plan.iter().all(|line| !line.contains("TEMP B-TREE")),
            "successor ordering must use covering index: {plan:?}"
        );
        let (row, units) = vm_units(&db, || {
            db.query_row(SQL, params!["receipt", "s", None::<i64>, target], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap()
        });
        assert_eq!(
            row,
            (target - 1, target),
            "real producer retains adjacent exact scope ranges"
        );
        assert_eq!(
            db.query_row(
                SQL,
                params!["receipt", "s", None::<i64>, i64::from(count)],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            (i64::from(count) - 1, i64::from(count))
        );
        assert!(
            db.query_row(
                SQL,
                params!["receipt", "s", None::<i64>, i64::from(count) + 1],
                |r| r.get::<_, i64>(0)
            )
            .optional()
            .unwrap()
            .is_none()
        );
        assert!(
            db.query_row(SQL, params!["receipt", "o", None::<i64>, target], |r| r
                .get::<_, i64>(0))
                .optional()
                .unwrap()
                .is_none()
        );
        assert!(
            db.query_row(SQL, params!["unavailable", "s", Some(99i64), target], |r| r
                .get::<_, i64>(0))
                .optional()
                .unwrap()
                .is_none()
        );
        measured.push(units);
        let mut absent = false;
        for _ in 0..512 {
            let proof = relevance_pull(&db);
            assert_ne!(
                proof.relevance,
                crate::store::presentation_relevance::Relevance::Open,
                "retained covered receipt conditions cannot revive"
            );
            if proof.relevance == crate::store::presentation_relevance::Relevance::Absent {
                absent = true;
                break;
            }
        }
        assert!(
            absent,
            "finite coverage-heavy union must converge under shared caps"
        );
    }
    assert!(
        measured[1] <= measured[0] + 4,
        "indexed successor VM work must not scan retained history: {measured:?}"
    );
}

#[test]
fn presentation_relevance_sweep_mutations_fence_and_overflow_roll_back_without_self_invalidation() {
    let db = relevance_db();
    let version = || {
        db.query_row(
            "SELECT version FROM presentation_warning_fence WHERE singleton=1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let hooks:i64=db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name LIKE 'presentation_fence_%'",[],|r|r.get(0)).unwrap();
    assert_eq!(hooks, 87);
    // Hook unit coverage uses a rolled-back scope row, not a new canonical
    // sweep writer or permission to delete the retained producer's chain.
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let before = version();
    db.execute("INSERT INTO warning_close_sweeps(id,condition_kind,affected_seat_id,after_ordinal,through_ordinal,close_decision_seq,interval_high_water,decision_at) VALUES ('relevance-hook-sweep','receipt','s',0,1,1,0,1)",[]).unwrap();
    assert_eq!(version(), before + 1);
    db.execute(
        "UPDATE warning_close_sweeps SET decision_at=2 WHERE id='relevance-hook-sweep'",
        [],
    )
    .unwrap();
    assert_eq!(version(), before + 2);
    db.execute(
        "DELETE FROM warning_close_sweeps WHERE id='relevance-hook-sweep'",
        [],
    )
    .unwrap();
    assert_eq!(version(), before + 3);
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(version(), before);
    assert_eq!(
        relevance_pull(&db).relevance,
        crate::store::presentation_relevance::Relevance::Absent
    );
    assert_eq!(
        version(),
        before,
        "proof persistence is excluded from invalidation"
    );
    for value in [Some(i64::MAX), Some(-1), None] {
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        if let Some(value) = value {
            db.pragma_update(None, "ignore_check_constraints", "ON")
                .unwrap();
            db.execute(
                "UPDATE presentation_warning_fence SET version=?1 WHERE singleton=1",
                [value],
            )
            .unwrap();
            db.pragma_update(None, "ignore_check_constraints", "OFF")
                .unwrap();
        } else {
            db.execute(
                "DELETE FROM presentation_warning_fence WHERE singleton=1",
                [],
            )
            .unwrap();
        }
        assert!(db.execute("INSERT INTO warning_close_sweeps(id,condition_kind,affected_seat_id,after_ordinal,through_ordinal,close_decision_seq,interval_high_water,decision_at) VALUES ('relevance-rejected-sweep','receipt','s',0,1,1,0,1)",[]).is_err());
        assert_eq!(
            db.query_row("SELECT count(*) FROM warning_close_sweeps", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(version(), before);
    }
}

#[test]
fn presentation_relevance_unavailable_exact_episode_and_late_publication_are_swept() {
    use crate::protocol::time::UtcMillis;
    use crate::store::{
        messages::{self, PublicationAuthor},
        presentation_relevance::Relevance,
    };
    let mut db = relevance_db();
    db.execute(
        "UPDATE seats SET unavailability_open=1,unavailability_episode=1 WHERE id='s'",
        [],
    )
    .unwrap();
    // Stage two genuine unavailable-recipient snapshots in the same episode,
    // then hold the second sealed preparation across the canonical close.
    for n in 0..2 {
        let tx = db.transaction().unwrap();
        let thread = format!("relevance-episode-{n}");
        let prep = format!("prep-relevance-episode-{n}");
        tx.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','episode','episode',0,0)",[&thread]).unwrap();
        tx.execute("INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,'s','joined'),(?1,'o','joined')",[&thread]).unwrap();
        tx.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,'s',1,1),(?1,'o',1,1)",[&thread]).unwrap();
        tx.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES (?1,'i','seat:o',?2,zeroblob(32),?2,0,0,0,0,0,1000000,0,'building')",params![prep,thread]).unwrap();
        let (mut recipients, mut warnings) = (0, 0);
        messages::stage_recipient(
            &tx,
            &prep,
            "i",
            &ThreadId::new(&thread),
            "s",
            messages::Staging {
                duration: 100,
                priority: crate::protocol::priority::DeliveryPriority::p0(),
                ack: true,
            },
            &mut recipients,
            &mut warnings,
        )
        .unwrap();
        assert_eq!((recipients, warnings), (1, 1));
        tx.execute("UPDATE send_preparations SET status='sealed',recipient_count=?2,warning_count=?3 WHERE id=?1",params![prep,recipients,warnings]).unwrap();
        tx.commit().unwrap();
    }
    let publish = |db: &mut Connection, n: i64| {
        let tx = db.transaction().unwrap();
        let thread = format!("relevance-episode-{n}");
        messages::insert_publication(
            &tx,
            200 + n,
            "seat:o",
            &thread,
            &[0; 32],
            &ThreadId::new(&thread),
            "episode publication",
            PublicationAuthor::Native {
                seat: &SeatId::new("o"),
                binding: &relevance_publication_binding(&tx),
                observation: "cooperative_top_level",
                relays_user: false,
                user_intent: None,
            },
        )
        .unwrap();
        tx.commit().unwrap();
    };
    publish(&mut db, 0);
    let mut open = false;
    for _ in 0..64 {
        if relevance_pull(&db).relevance == Relevance::Open {
            open = true;
            break;
        }
    }
    assert!(open, "published current episode is initially actionable");
    {
        let tx = db.transaction().unwrap();
        assert_eq!(
            schema::clear_warning_conditions_for_unavailable(&tx, "s", 2, UtcMillis(201)).unwrap(),
            0
        );
        tx.commit().unwrap();
    }
    assert_eq!(
        relevance_pull(&db).relevance,
        Relevance::Open,
        "wrong episode cannot cover exact prepared witness"
    );
    {
        let tx = db.transaction().unwrap();
        assert_eq!(
            schema::clear_warning_conditions_for_unavailable(&tx, "s", 1, UtcMillis(202)).unwrap(),
            1
        );
        tx.execute("UPDATE seats SET unavailability_open=0 WHERE id='s'", [])
            .unwrap();
        tx.commit().unwrap();
    }
    publish(&mut db, 1);
    let ranges:Vec<(i64,i64)>=db.prepare("SELECT after_ordinal,through_ordinal FROM warning_close_sweeps WHERE condition_kind='unavailable' AND affected_seat_id='s' AND episode=1 ORDER BY through_ordinal").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(
        ranges,
        [(0, 1), (1, 2)],
        "late old-episode publication extends the retained canonical chain"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE clear_warning_id IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2,
        "both physical clears remain undrained"
    );
    let mut absent = false;
    for _ in 0..128 {
        let proof = relevance_pull(&db);
        assert_ne!(
            proof.relevance,
            Relevance::Open,
            "both old-episode conditions are logically covered"
        );
        if proof.relevance == Relevance::Absent {
            absent = true;
            break;
        }
    }
    assert!(absent);
}

#[test]
fn presentation_relevance_private_proof_refuses_corruption_without_effects() {
    use crate::protocol::delivery::PresentationConsumer;
    let db = relevance_db();
    relevance_pull(&db);
    let snapshot = || {
        db.query_row("SELECT hex(p.proof_json),f.version FROM presentation_warning_proofs p CROSS JOIN presentation_warning_fence f WHERE p.seat_id='s' AND p.consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap()
    };
    for value in [" ".repeat(32769), "null".into(), "{".into()] {
        db.execute_batch("BEGIN IMMEDIATE; PRAGMA ignore_check_constraints=ON")
            .unwrap();
        db.execute("UPDATE presentation_warning_proofs SET proof_json=?1 WHERE seat_id='s' AND consumer='hook'",[value]).unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let before = snapshot();
        let result = crate::store::presentation_relevance::advance(
            &db,
            &relevance_claim(),
            PresentationConsumer::Hook,
            0,
            &relevance_budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "{result:?}"
        );
        assert_eq!(snapshot(), before);
        db.execute_batch("ROLLBACK").unwrap();
    }
    db.execute_batch("BEGIN IMMEDIATE; PRAGMA ignore_check_constraints=ON; UPDATE presentation_warning_proofs SET proof_json=CAST(X'80' AS TEXT); PRAGMA ignore_check_constraints=OFF").unwrap();
    let before = snapshot();
    let result = crate::store::presentation_relevance::advance(
        &db,
        &relevance_claim(),
        PresentationConsumer::Hook,
        0,
        &relevance_budget(),
    );
    assert!(matches!(
        result,
        Err(crate::protocol::results::ApiError {
            code: crate::protocol::results::ErrorCode::StoreCorrupt,
            ..
        })
    ));
    assert_eq!(snapshot(), before);
    db.execute_batch("ROLLBACK").unwrap();
    assert!(
        db.execute("UPDATE presentation_warning_proofs SET proof_json=NULL", [])
            .is_err()
    );
}

// Kills: exact-key helper hiding retained-relation scans or bypassing logical
// authority. Observe the actual production statements on linked SQLite.
#[test]
fn presentation_capacity_due_exact_keys_have_bounded_actual_sql_plans_and_vm() {
    use crate::{
        ports::PresentationWork,
        protocol::{ids::MessageId, results::InboxBatchV2Item},
        store::queries,
    };
    let mut matrices = Vec::new();
    for history in [32, 2048] {
        let db = every_source();
        db.execute_batch("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s',1,1,'pane-s','host',1,'codex','due-session','due-execution','cooperative_top_level',0,0);
            INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,acked_at,ack_actor_seat_id,ack_generation,ack_observation) VALUES ('m3','t3','s','acked',100,1,'s',1,'obs');").unwrap();
        for n in 0..history {
            db.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s',?1,?1,1,'cooperative_top_level')", [100+n]).unwrap();
            db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t2',?2,'ordinary','unrelated',0,?2)", params![format!("history-{n}"),100+n]).unwrap();
            db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t2','o','pending',100)", [format!("history-{n}")]).unwrap();
        }
        db.execute_batch("UPDATE host_instances SET decision_seq=5000 WHERE id='i';
            UPDATE seats SET nudge_threshold=-1 WHERE id='s';
            INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES ('prep-due-lazy','i','scope','lazy-op',zeroblob(32),'t3',0,0,0,0,0,0,0,'building','lazy'),('prep-due-unpublished','i','scope','unpublished-op',zeroblob(32),'t3',0,0,0,0,0,0,0,'building','lazy');
            INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES ('prep-due-lazy','msg-due-lazy','t3','s'),('prep-due-unpublished','msg-due-unpublished','t3','s');
            INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,delivery_mode) VALUES ('msg-due-lazy','i','t3',50,'ordinary','lazy body',0,30,'lazy'),('msg-due-unpublished','i','t3',51,'ordinary','not published',0,32,'lazy'),('future-message','i','t3',52,'ordinary','future',0,9999,'ordinary');
            INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-due-lazy','msg-due-lazy','i','t3',30,0,50,0,1,0);").unwrap();
        let binding = crate::protocol::ids::BindingId::new(db.query_row("SELECT binding_id FROM occupant_bindings WHERE seat_id='s' AND native_session='due-session'", [], |r|r.get::<_,String>(0)).unwrap());
        db.execute(
            "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES (?1,'t3',1)",
            [binding.as_str()],
        )
        .unwrap();
        let trace = RelevanceTrace::install(&db);
        trace.state.borrow_mut().all_selects = true;
        let mut work = PresentationWork::default();
        let seat = SeatId::new("s");
        let context = queries::ordinary_due_context(&db, "i", &seat, &binding, &mut work).unwrap();
        for id in ["m1", "m3"] {
            let item = queries::ordinary_presentation_message_by_id(
                &db,
                "i",
                &seat,
                &binding,
                &MessageId::new(id),
                context,
                &mut work,
            )
            .unwrap()
            .unwrap();
            assert!(matches!(
                item,
                InboxBatchV2Item::Message {
                    ack_candidate: Some(_),
                    ..
                }
            ));
        }
        let lazy = queries::ordinary_presentation_message_by_id(
            &db,
            "i",
            &seat,
            &binding,
            &MessageId::new("msg-due-lazy"),
            context,
            &mut work,
        )
        .unwrap();
        assert!(matches!(
            lazy,
            Some(InboxBatchV2Item::LazyMessage { wakes: true, .. })
        ));
        for id in [
            "absent",
            "mo",
            "w-member",
            "msg-due-unpublished",
            "future-message",
        ] {
            assert!(
                queries::ordinary_presentation_message_by_id(
                    &db,
                    "i",
                    &seat,
                    &binding,
                    &MessageId::new(id),
                    context,
                    &mut work
                )
                .unwrap()
                .is_none()
            );
        }
        assert!(
            queries::ordinary_presentation_message_by_id(
                &db,
                "other-instance",
                &seat,
                &binding,
                &MessageId::new("m1"),
                context,
                &mut work
            )
            .unwrap()
            .is_none()
        );
        // Capture the waiver path too; it is a separate canonical lookup, never ACK evidence.
        db.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',40,1,0)", []).unwrap();
        assert!(
            queries::ordinary_presentation_message_by_id(
                &db,
                "i",
                &seat,
                &binding,
                &MessageId::new("m3"),
                context,
                &mut work
            )
            .unwrap()
            .is_none()
        );
        let statements = trace.finish();
        assert!(
            statements
                .iter()
                .any(|entry| entry.sql.contains("FROM lazy_recipients"))
        );
        assert!(
            statements
                .iter()
                .any(|entry| entry.sql.contains("FROM seat_availability"))
        );
        assert!(
            statements
                .iter()
                .any(|entry| entry.sql.contains("FROM human_receipt_waivers"))
        );
        assert!(
            statements
                .iter()
                .any(|entry| entry.sql.contains("FROM receipts r"))
        );
        assert!(work.ordinary_due_key_probes <= 3 + 10 * 18);
        assert!(work.ordinary_due_seeks <= 10);
        let mut matrix = Vec::new();
        for statement in statements {
            assert_eq!(statement.max_scan, 0, "full scan: {}", statement.sql);
            assert_eq!(statement.max_sort, 0, "sort: {}", statement.sql);
            assert!(
                statement.max_vm <= 300,
                "VM {}: {}",
                statement.max_vm,
                statement.sql
            );
            assert!(
                statement.rows <= statement.executions,
                "at most one result per keyed statement"
            );
            for expanded in [&statement.first_expanded, &statement.last_expanded] {
                let plan = db
                    .prepare(&format!("EXPLAIN QUERY PLAN {expanded}"))
                    .unwrap()
                    .query_map([], |r| r.get::<_, String>(3))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert!(
                    plan.iter().all(|line| (!line.contains("SCAN ")
                        || line.contains("SCAN CONSTANT ROW"))
                        && !line.contains("TEMP B-TREE")),
                    "{plan:?}"
                );
                assert!(
                    plan.iter().any(|line| line.contains("SEARCH ")),
                    "keyed production statement must seek: {plan:?}"
                );
                eprintln!(
                    "due retained={history} executions={} rows={} vm={} plan={plan:?}",
                    statement.executions, statement.rows, statement.max_vm
                );
            }
            matrix.push((statement.sql, statement.max_vm));
        }
        matrices.push(matrix);
    }
    assert_eq!(
        matrices[0], matrices[1],
        "keyed work stays independent of unrelated history"
    );
}

// Main review F2. Kills: a bound reader's informational-notice check that follows the seat's
// newest top-level binding instead of the reader's own frontier. B (the newest) was offered n1;
// A was not, so A's bound read still owes it while the seat view (B's) does not, and the reverse.
#[test]
fn bound_reader_informational_notice_follows_its_own_frontier() {
    let db = per_binding();
    notice(&db, 1);
    db.execute(
        "INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('unavailable','t','c1','s','n1',21)",
        [],
    )
    .unwrap();
    display(&db, "bind-B", 0);
    assert!(binding_notice_frontier(&db, "bind-B").unwrap() > 0);
    let pending =
        |binding: Option<&str>| informational_notice_pending_for(&db, "s", "n1", binding).unwrap();
    assert_eq!(pending(None), Some(false), "the seat view follows B");
    assert_eq!(pending(Some("bind-B")), Some(false));
    assert_eq!(
        pending(Some("bind-A")),
        Some(true),
        "A was never offered n1"
    );
    // A subagent of A reads its parent's frontier until it has its own: once A is offered n1
    // (a non-zero parent frontier), C no longer owes it, which an empty own frontier would.
    assert_eq!(pending(Some("bind-C")), Some(true));
    display(&db, "bind-A", 0);
    assert!(binding_notice_frontier(&db, "bind-A").unwrap() > 0);
    assert_eq!(pending(Some("bind-A")), Some(false));
    assert_eq!(
        pending(Some("bind-C")),
        Some(false),
        "C follows A's frontier"
    );
}

// Receipt settlement is seat-wide; physical recovery and read progress belong to the binding.
#[test]
fn presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement() {
    use crate::{
        ports::PresentationWork,
        protocol::{ids::BindingId, results::InboxBatchV2Item},
        store::queries,
    };
    let db = every_source();
    db.execute_batch("INSERT INTO occupant_bindings(binding_id,seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES
        ('replay-A','s',1,1,'pane-s','host',1,'codex','session-A','execution-A','cooperative_top_level',0,0),
        ('replay-B','s',1,1,'pane-s','host',1,'codex','session-B','execution-B','cooperative_top_level',0,0);
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t3','s','joined');
        INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES ('replay-A','t3',50),('replay-B','t3',0);
        INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_actor_seat_id,ack_generation,ack_observation,ack_binding_id) VALUES ('m3','s','acked',1,'s',1,'cooperative_top_level','replay-A');
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES ('prep-replay-lazy','i','scope','prep-replay-lazy',zeroblob(32),'t3',0,0,0,0,0,0,0,'building','lazy');
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,state,legacy_pending) VALUES ('prep-replay-lazy','msg-replay-lazy','t3','s','pending',0);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,delivery_mode) VALUES ('msg-replay-lazy','i','t3',50,'ordinary','lazy body',0,30,'lazy');
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('prep-replay-lazy','msg-replay-lazy','i','t3',30,0,50,0,1,0);
        UPDATE lazy_recipients SET state='displayed' WHERE message_id='msg-replay-lazy';").unwrap();
    let seat = SeatId::new("s");
    for (binding, unread) in [("replay-A", false), ("replay-B", true)] {
        let binding = BindingId::new(binding);
        let mut work = PresentationWork::default();
        let context = queries::ordinary_due_context(&db, "i", &seat, &binding, &mut work).unwrap();
        let expected_context_probes = 3;
        let context_probes = work.ordinary_due_key_probes;
        assert_eq!(
            context_probes,
            expected_context_probes,
            "{}: exact-binding context probes",
            binding.as_str()
        );
        let ordinary = queries::ordinary_presentation_message_by_id(
            &db,
            "i",
            &seat,
            &binding,
            &MessageId::new("m3"),
            context,
            &mut work,
        )
        .unwrap();
        assert_eq!(ordinary.is_some(), unread);
        if let Some(ordinary) = ordinary {
            assert!(matches!(
                ordinary,
                InboxBatchV2Item::Message {
                    ack_candidate: None,
                    ..
                }
            ));
        }
        let expected_ordinary_probes = if unread { 18 } else { 16 };
        let ordinary_end_probes = work.ordinary_due_key_probes;
        assert_eq!(
            ordinary_end_probes - context_probes,
            expected_ordinary_probes,
            "{}: settled ordinary eligibility and optional hydration probes",
            binding.as_str()
        );
        let lazy = queries::ordinary_presentation_message_by_id(
            &db,
            "i",
            &seat,
            &binding,
            &MessageId::new("msg-replay-lazy"),
            context,
            &mut work,
        )
        .unwrap();
        assert_eq!(lazy.is_some(), unread);
        // Lazy charges one message point, seven existing eligibility relations and
        // three canonical episode points: direct pair, seat and current invitation.
        // The envelope is charged even when own-cursor scope short-circuits them;
        // an unread body adds three hydration points. A: 3+16+11=30; B: 3+18+14=35.
        let expected_lazy_probes = if unread { 14 } else { 11 };
        assert_eq!(
            work.ordinary_due_key_probes - ordinary_end_probes,
            expected_lazy_probes,
            "{}: lazy eligibility and optional hydration probes",
            binding.as_str()
        );
        assert_eq!(
            work.ordinary_due_key_probes,
            expected_context_probes + expected_ordinary_probes + expected_lazy_probes,
            "{}: complete exact recovery probe total",
            binding.as_str()
        );
        assert_eq!(work.ordinary_due_seeks, 1);
    }
    assert_eq!(
        db.query_row(
            "SELECT ack_binding_id FROM receipt_state WHERE message_id='m3' AND seat_id='s'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "replay-A"
    );
    assert_eq!(
        db.query_row(
            "SELECT seq FROM binding_cursors WHERE binding_id='replay-B' AND thread_id='t3'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn pulled_notice_settlement_uses_exact_binding_when_sessions_match_across_harnesses() {
    let (_context, db, _guard) = axis_history(Axis::ProgrammaticUndelivered, 3);
    let binding = rcv_binding_id(&db);
    let (generation, session, _) = rcv_binding(&db);
    let carried = notice_refs(&db, 1);
    db.execute("INSERT INTO occupant_bindings(binding_id,seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) SELECT 'same-session-sibling',seat_id,generation,target_generation,target_id,host_boot,host_epoch,'claude',native_session,'sibling-execution',observation_provenance,observed_at,registered_at,terminal_id,incarnation FROM occupant_bindings WHERE binding_id=?1", [binding.as_str()]).unwrap();
    assert!(
        settle_pulled_notices(&db, "i", "rcv", &binding, generation, &session, &carried).unwrap()
    );
    assert!(binding_notice_frontier(&db, binding.as_str()).unwrap() > 0);
    assert_eq!(
        binding_notice_frontier(&db, "same-session-sibling").unwrap(),
        0
    );
}
