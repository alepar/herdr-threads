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
        let slice =
            effective::scan_effective_seat_attention(db, seat, position.take(), 100).unwrap();
        if let Some(attention) = slice.attention {
            return attention.frontier;
        }
        position = Some(slice.position);
    }
}

fn empty() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
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
// before the read and ACK; the newest burst invitations fill the rest. Holds
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
                "- accept: herdr-threads accept th",
                "- read: herdr-threads read th --recent 20",
                "- ACK after reading: herdr-threads ack m-h",
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

/// Count SQLite VM instructions (in units of 10) while `f` runs.
fn vm_units<T>(db: &Connection, f: impl FnOnce() -> T) -> (T, u64) {
    use rusqlite::ffi;
    use std::sync::atomic::{AtomicU64, Ordering};
    static UNITS: AtomicU64 = AtomicU64::new(0);
    extern "C" fn count(_: *mut std::ffi::c_void) -> std::ffi::c_int {
        UNITS.fetch_add(1, Ordering::SeqCst);
        0
    }
    UNITS.store(0, Ordering::SeqCst);
    // SAFETY: the handle belongs to `db`, which outlives both calls; the
    // handler is removed before returning.
    unsafe { ffi::sqlite3_progress_handler(db.handle(), 10, Some(count), std::ptr::null_mut()) };
    let value = f();
    unsafe { ffi::sqlite3_progress_handler(db.handle(), 0, None, std::ptr::null_mut()) };
    (value, UNITS.load(Ordering::SeqCst))
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
// from 10^3 to 10^5 instance threads.
#[test]
fn digest_cost_is_flat_from_a_thousand_to_a_hundred_thousand_threads() {
    let mut observed = Vec::new();
    for threads in [1_000u64, 10_000, 100_000] {
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
// attention, never to its retained history. From 10^3 to 10^5 ACKed receipts
// on the digest seat (half physical, half manifest-backed) and from 10^3 to
// 10^4 settled warnings in its member thread, the digest examines exactly the
// same indexed candidates and the same SQLite VM work (within 10%). Kills:
// the full-history receipt walk (`scan_effective_receipts(Seat)` to
// completion), a physical walk on `receipts_seat_state_ordinal` without the
// pending restriction, a manifest walk on `prepared_recipients_seat_ordinal`
// instead of the pending projection, the member-thread warning walk over every
// warn message/manifest (`messages_thread_warning`), and a missing settlement
// trigger (settled rows left in a projection).
#[test]
fn digest_cost_is_flat_in_the_seats_acked_receipts_and_settled_warnings() {
    let mut observed = Vec::new();
    for (acked, warns) in [(1_000u64, 1_002u64), (10_000, 3_000), (100_000, 10_002)] {
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
        .filter(|s| s.trim_start().starts_with("INSERT"))
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
    // retained preparation (reported separately), so keep them cached.
    conn.pragma_update(None, "cache_size", -262_144).unwrap();
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
#[test]
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
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('rcv',1,1,'p-rcv','b',1,'codex','hist-rcv','hist-rcv','fresh',0,0,'term-rcv','inc');\
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
        GlobalLogicalKinds, ReceiptScanScope, scan_effective_receipts,
        scan_effective_seat_attention, scan_effective_timeline, scan_effective_warnings_for_seat,
        scan_global_logical_candidates,
    };
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
// walks the instance's threads. From 10^3 to 10^5 instance threads (each with
// an ordinary message and no warning) the count for `s` (one pending
// invitation-overdue warning, through its membership) is exact with the same
// SQLite VM work (within 10%). Kills: a per-thread count loop (`SELECT id FROM
// threads WHERE instance_id=?1` with a timeline walk per thread), whose work
// grows with every retained thread.
#[test]
fn pending_warning_count_is_flat_in_instance_threads() {
    use crate::protocol::time::{CallBudget, MonoInstant};
    let clock = crate::app::SystemClock::new();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let mut observed = Vec::new();
    for threads in [1_000u64, 10_000, 100_000] {
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
/// page (each advancing the occupant-scoped `digest_notice_offer` frontier
/// over the page it carried; wave-2 fix2 (a)). No projection row is deleted.
fn offer_to_current_occupant(db: &Connection) {
    db.execute_batch("\
        BEGIN IMMEDIATE;\
        INSERT INTO digest_notice_offer(seat_id,binding_generation,execution_id,offered_ordinal) SELECT 'rcv',s.generation,b.execution_id,(SELECT MAX(ordinal) FROM digest_programmatic_warnings WHERE seat_id='rcv') FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id='rcv';\
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
            // The control writers verify the inviter's current observed target.
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at) VALUES ('i','p-snd','b',1,1,2,'fresh','unknown','unknown',0,0)", []).unwrap();
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
fn pending_invitation_axis_is_flat() {
    assert_axis_flat(Axis::PendingInvitations, &[1_000, 10_000]);
}

// Pending-receipt axis: 10^3 and 10^4 pending require-ack receipts in `hist`.
// Kills M-post-filter-cap (the receipt walks read every pending row and the
// window is cut to `cap+1` afterwards: same answer, work grows with the
// pending backlog).
#[test]
fn pending_receipt_axis_is_flat() {
    assert_axis_flat(Axis::PendingReceipts, &[1_000, 10_000]);
}

// Programmatic notices never offered: 10^3 and 10^4 in `hist`. The count
// saturates at 1000 with `has_more`; work stays flat. Kills a programmatic
// walk without its `LIMIT` (work grows) and the fix4 behaviour of building
// the full pending set before the cap.
#[test]
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
    let (generation, execution): (i64, String) = db
        .query_row(
            "SELECT s.generation,b.execution_id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE s.id='rcv'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let total = |db: &Connection| -> i64 {
        db.query_row("SELECT total_changes()", [], |r| r.get(0))
            .unwrap()
    };
    let before = total(db);
    let ((), units) = vm_units(db, || {
        db.execute("INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) SELECT 'rcv',?1,?2,decision_seq FROM host_instances WHERE id='i' ON CONFLICT(seat_id) DO UPDATE SET binding_generation=excluded.binding_generation,execution_id=excluded.execution_id,offered_through_seq=MAX(warning_offer.offered_through_seq,excluded.offered_through_seq)",
            rusqlite::params![generation, execution]).unwrap();
        settle_carried_notices(db, "rcv", generation, &execution, &carried).unwrap();
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
