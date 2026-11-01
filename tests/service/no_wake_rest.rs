//! ht-kqrt.32 (included from `src/service/dispatch.rs` beside `no_wake_reports_tests`): the
//! no-wake remainder. A named seat whose bindings ended is `offline` (M19), is never a poke
//! candidate, and `doctor`'s wake report answers over never-settled `wake_work` (S6).
use super::no_wake_reports_tests::Rig;
use crate::{
    protocol::{ids::SeatId, results::WakeBlock},
    store::{connection::StoreContext, poke},
    test_support::history,
};
use rusqlite::Connection;
use std::sync::Arc;

const WINDOW_MS: u64 = 100_000;

// Kills: an offline named seat that classifies as nothing (its harness lost with its binding), an
// online one that reads offline, or a pane seat's row changing.
#[test]
fn offline_named_seat_reads_no_wake_offline() {
    let rig = Rig::new("nwrest-offline");
    let (sender, target) = (rig.named_seat("s", "codex"), rig.named_seat("t", "codex"));
    let live = rig.status(&sender, &target);
    assert_eq!((live.wake, live.offline), (Some(WakeBlock::NoWake), false));
    rig.end_bindings(&target);
    let off = rig.status(&sender, &target);
    assert_eq!((off.wake, off.offline), (Some(WakeBlock::NoWake), true));
    let line = crate::cli::wake_warning::lines(&[crate::protocol::results::SeatWakeStatus {
        first: true,
        ..off
    }]);
    assert_eq!(
        line,
        format!("warning: {} wake=no_wake offline\n", target.as_str())
    );
}

/// Production-shaped receipts in a bare store: `snd` sends to `rcv`, whose binding is open.
fn fixture(rcv_name: Option<&str>) -> (StoreContext, Connection, std::path::PathBuf) {
    let path =
        crate::test_support::scratch::temp_path(format!("ht-nwrest-{}.db", uuid::Uuid::new_v4()));
    let context =
        StoreContext::new(path.clone(), Arc::new(crate::app::SystemClock::new())).with_test_host();
    let conn = context.open_writer().unwrap();
    conn.execute_batch("\
        UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=1;\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,unavailability_open,home_node,is_local,host_id) VALUES ('snd','34750f98bd59fcfc946da45aaabe933b','resolved','native','p-snd',1,1,0,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts)),('rcv','34750f98bd59fcfc946da45aaabe933b','resolved','native','p-rcv',1,1,0,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));\
        INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p-rcv','b',1,1,1,'fresh','unknown','unknown',0,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('rcv',1,1,'p-rcv','b',1,'codex','hist-rcv','hist-rcv','fresh',0,0,'term-rcv','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('ta','34750f98bd59fcfc946da45aaabe933b','a','g',0,0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('ta','snd','joined'),('ta','rcv','joined');\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('ta','snd',1,1),('ta','rcv',1,1);\
    ").unwrap();
    if let Some(name) = rcv_name {
        conn.execute("UPDATE seats SET name=?1 WHERE id='rcv'", [name])
            .unwrap();
    }
    (context, conn, path)
}

fn due(conn: &Connection) -> usize {
    let (deadline, available): (i64, i64) = conn
        .query_row(
            "SELECT deadline_at, available_at FROM receipt_state WHERE seat_id='rcv' LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(deadline - available, WINDOW_MS as i64);
    poke::due_pokes(
        conn,
        deadline - 40_000,
        &crate::protocol::summary::SummarySettings::default(),
        16,
    )
    .unwrap()
    .len()
}

fn clean(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

// Kills: a soft poke (the recipient-side reminder) becoming due for a named seat whose bindings
// ended, or the filter spilling onto a pane seat or a named seat that is still bound.
#[test]
fn deadline_to_offline_no_wake_seat_no_poke_no_reminder() {
    for (name, ended, want) in [
        (None, true, 1),         // a pane seat keeps its poke candidacy, bound or not
        (Some("rcv"), false, 1), // a bound named seat: host (no wake lane) decides, not the store
        (Some("rcv"), true, 0),  // an offline named seat: no reminder
    ] {
        let (context, mut conn, path) = fixture(name);
        history::write_pending_sends_with_deadline(
            &context, &mut conn, "ta", "snd", 1, WINDOW_MS, "one",
        )
        .unwrap();
        if ended {
            conn.execute(
                "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='rcv'",
                [],
            )
            .unwrap();
        }
        assert_eq!(due(&conn), want, "name={name:?} ended={ended}");
        drop(conn);
        clean(&path);
    }
}

// Kills: `wake_status(all)` (what `doctor` reads) failing or hiding a named seat that holds a
// never-settled `wake_work` reservation, which no wake lane will ever settle.
#[test]
fn s6_doctor_on_never_settled_wake_work() {
    let rig = Rig::new("nwrest-s6");
    let (a, b) = (rig.named_seat("a", "codex"), rig.named_seat("b", "claude"));
    for seat in [&a, &b] {
        rig.sql()
            .execute(
                "INSERT INTO wake_work(seat_id,binding_generation,reservation_id,reservation_boot,reserved_at_utc) VALUES (?1,1,?2,?3,0)",
                rusqlite::params![
                    seat.as_str(),
                    format!("attempt-{}", seat.as_str()),
                    uuid::Uuid::new_v4().to_string()
                ],
            )
            .unwrap();
    }
    rig.end_bindings(&b);
    let rows = rig.status_all();
    let find = |seat: &SeatId| rows.iter().find(|row| &row.seat == seat).unwrap();
    assert_eq!(find(&a).wake, Some(WakeBlock::NoWake));
    assert!(!find(&a).offline);
    // `doctor` lists open-binding seats only: the offline one is not in `all`.
    assert!(rows.iter().all(|row| row.seat != b), "{rows:?}");
    // The unsettled reservation is untouched: doctor reads, it never settles.
    let left: i64 = rig
        .sql()
        .query_row(
            "SELECT count(*) FROM wake_work WHERE reservation_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 2);
}
