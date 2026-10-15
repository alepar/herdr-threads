//! Remote seats in node-local workers and reads (ht-5fr.4.14, fact-kinds spec
//! sections 14 and 15). A remote seat is a directory row homed on another node
//! (`is_local = 0`, state `remote`, no host, no pane): reconciliation, wake,
//! liveness and archival select local seats only and change no remote row; reads
//! list the remote seat compactly with state `remote` and no pane columns; pane
//! listings omit it; automatic archival defers a thread with a live remote
//! member. Remote rows are seeded directly with valid remote columns.

use herdr_threads::{
    ports::{
        EnumerationEvidence, EvidenceKind, HostId, HostSnapshot, IncarnationEvidence, ReadContext,
        SnapshotHeader, StorePort,
    },
    protocol::{
        commands::{
            Command, DirectoryMembership, DirectoryQuery, ParticipantLocationsQuery,
            ParticipantsQuery, SeatInspectQuery, SeatsQuery, ThreadQuery,
        },
        ids::{HostBootId, HostLocator, HostTargetId, SeatId, ThreadId},
        output::{OutputFormat, OutputSpec, encode_selected},
        pagination::PageRequest,
        results::{CommandResult, ContinuityStatus},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{
        SqliteStore, StoreSettings,
        archival::{self, Runtime},
        connection::StoreContext,
    },
};
use rusqlite::{Connection, params};
use std::sync::Arc;

const NODE: &str = "34750f98bd59fcfc946da45aaabe933b";
const FOREIGN_NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BOOT: &str = "00000000-0000-0000-0000-000000000001";
const SOCKET: &str = "/tmp/ht-remote-seats.sock";

struct Fixed;
impl Clock for Fixed {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    store: SqliteStore,
    db: Connection,
    host: HostId,
    _directory: Directory,
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(100_000),
        cancellation: Cancellation::default(),
    }
}

fn read() -> ReadContext {
    ReadContext {
        instance: NODE.into(),
        output: OutputSpec::default(),
        operation_scope: None,
    }
}

fn page() -> PageRequest {
    PageRequest {
        cursor: None,
        limit: 50,
        max_bytes: 65_536,
    }
}

/// One attached server with the local seat `local` (resolved on pane `pane`,
/// connected binding, a pending invitation: a due wake candidate) and the
/// remote seat `remote` with a pending invitation of its own.
fn fixture() -> Fixture {
    let directory = Directory(herdr_threads::test_support::scratch::temp_path(format!(
        "remote-seats-{}",
        uuid::Uuid::new_v4()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let settings = StoreSettings {
        daemon_boot: Some(uuid::Uuid::parse_str(BOOT).unwrap()),
        minimum_wake_delay_ms: 120_000,
        ..StoreSettings::default()
    };
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(Fixed)),
        NODE,
        settings,
    )
    .unwrap();
    let host = store
        .attach_host(&HostLocator::parse(SOCKET).unwrap(), 0, &budget())
        .unwrap()
        .0;
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    db.execute_batch(
        "UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    local_seat(&db, "local", "pane");
    remote_seat(&db, "remote");
    for seat in ["local", "remote"] {
        pending_invitation(&db, seat);
    }
    Fixture {
        store,
        db,
        host,
        _directory: directory,
    }
}

fn local_seat(db: &Connection, seat: &str, pane: &str) {
    let host: i64 = db
        .query_row("SELECT host_id FROM herdr_hosts", [], |r| r.get(0))
        .unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,?2,'resolved','native',?3,1,1,0,(SELECT id FROM local_node),1,?4)",
        params![seat, NODE, pane, host],
    )
    .unwrap();
    db.execute(
        "INSERT OR IGNORE INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch,occupancy,ui_state,verified_execution,top_level_occupant) VALUES (?1,?2,'host',1,1,0,'fresh','term-'||?2,'inc','coherent_enumeration',1,'occupied','idle','exec',1)",
        params![host, pane],
    )
    .unwrap();
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,liveness_pid,liveness_start) VALUES (?1,1,1,?2,'host',1,'codex','session','exec','fresh',0,0,'term-'||?2,'inc',4242,7)",
        params![seat, pane],
    )
    .unwrap();
}

/// A directory row for a seat homed on another node: no host, no target.
fn remote_seat(db: &Connection, seat: &str) {
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,host_id,published) \
         VALUES (?1,?2,'remote','native',0,0,?3,0,'agent',NULL,1)",
        params![seat, NODE, FOREIGN_NODE],
    )
    .unwrap();
}

fn retire_remote(db: &Connection, seat: &str) {
    db.execute(
        "UPDATE seats SET state='retired',retired_at=1,retired_seq=1 WHERE id=?1",
        [seat],
    )
    .unwrap();
}

fn pending_invitation(db: &Connection, seat: &str) {
    let thread = format!("t-{seat}");
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES (?1,?2,'topic','goal',0,0,0)",
        params![thread, NODE],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,?2,'invited')",
        params![thread, seat],
    )
    .unwrap();
    db.execute(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) VALUES (?1,?2,?3,1,'pending',0,1,100,100,(SELECT id FROM local_node))",
        params![format!("inv-{seat}"), thread, seat],
    )
    .unwrap();
}

/// Every column of every remote seat row plus the rows that hang off them.
fn remote_rows(db: &Connection) -> String {
    let dump = |sql: &str| -> Vec<String> {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |r| {
                (0..columns)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(|cells| cells.join("|"))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    [
        dump("SELECT * FROM seats WHERE is_local=0 ORDER BY id"),
        dump("SELECT seat_id,thread_id,state,episode FROM memberships WHERE seat_id IN (SELECT id FROM seats WHERE is_local=0) ORDER BY seat_id,thread_id"),
        dump("SELECT * FROM wake_work WHERE seat_id IN (SELECT id FROM seats WHERE is_local=0)"),
        dump("SELECT * FROM occupant_bindings WHERE seat_id IN (SELECT id FROM seats WHERE is_local=0)"),
    ]
    .iter()
    .map(|section| section.join("\n"))
    .collect::<Vec<_>>()
    .join("\n--\n")
}

// Kills: a snapshot reconciliation that plans, unresolves or retires a remote
// seat (a remote seat is absent from every local server's snapshot).
#[test]
fn reconciliation_ignores_remote_seats() {
    let f = fixture();
    let before = remote_rows(&f.db);
    let admission = f
        .store
        .begin_host_observation(NODE, f.host, &budget())
        .unwrap();
    let snapshot = HostSnapshot {
        boot: HostBootId::new("host"),
        epoch: 1,
        observation_sequence: 1,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: Vec::new(),
    };
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    let stage = f.store.begin_snapshot_stage(header, &budget()).unwrap();
    f.store.seal_snapshot_stage(&stage.id, &budget()).unwrap();
    let published = f
        .store
        .publish_snapshot_stage(&stage.id, &budget())
        .unwrap();
    let saved = f
        .store
        .saved_seats_page(&published.id, 0, None, 16, &budget())
        .unwrap();
    let paged: Vec<&str> = saved.seats.iter().map(|s| s.seat.as_str()).collect();
    assert_eq!(paged, ["local"], "the saved-seat pass names no remote seat");
    for transition in herdr_threads::identity::reconcile::plan_page(&saved).unwrap() {
        f.store
            .apply_reconciliation_transition(transition, &budget())
            .unwrap();
    }
    let local: String =
        f.db.query_row("SELECT state FROM seats WHERE id='local'", [], |r| r.get(0))
            .unwrap();
    assert_eq!(
        local, "unresolved",
        "the absent local seat is reconciled, so the pass ran"
    );
    assert_eq!(remote_rows(&f.db), before, "no remote row changed");
}

// Kills: a wake scan, batch cleanup or poke scan that selects a remote seat
// (here it holds a pending invitation, so it would otherwise be a candidate).
#[test]
fn wake_candidates_never_select_remote() {
    let f = fixture();
    let before = remote_rows(&f.db);
    let items = f
        .store
        .wake_candidates(f.host, PageRequest::default(), &budget())
        .unwrap()
        .items;
    let seats: Vec<&str> = items.iter().map(|c| c.seat.as_str()).collect();
    assert_eq!(seats, ["local"], "only the local seat is a wake candidate");
    f.db.execute(
        "INSERT INTO wake_batches(seat_id,deadline_at) SELECT id,0 FROM seats",
        [],
    )
    .unwrap();
    let batched = f
        .store
        .wake_batch_seats(f.host, None, 16, &budget())
        .unwrap();
    let batched: Vec<&str> = batched.iter().map(SeatId::as_str).collect();
    assert_eq!(
        batched,
        ["local"],
        "batch cleanup never visits a remote seat"
    );
    let pokes = f.store.poke_candidates(f.host, 16, &budget()).unwrap();
    assert!(pokes.iter().all(|due| due.seat.as_str() != "remote"));
    assert_eq!(remote_rows(&f.db), before, "no remote row changed");
}

// Kills: a liveness sample that judges a remote seat's binding (it has no
// process here) and disconnects it.
#[test]
fn liveness_sample_ignores_remote() {
    let f = fixture();
    let before = remote_rows(&f.db);
    let candidates = f.store.liveness_candidates(None, &budget()).unwrap();
    let seats: Vec<&str> = candidates.iter().map(|c| c.binding.seat.as_str()).collect();
    assert_eq!(seats, ["local"], "only the local binding is sampled");
    let by_seat = f
        .store
        .liveness_candidates(Some(&SeatId::new("remote")), &budget())
        .unwrap();
    assert!(by_seat.is_empty(), "a remote seat is never a candidate");
    assert_eq!(remote_rows(&f.db), before, "no remote row changed");
}

fn archival_runtime(at: i64) -> Runtime {
    Runtime {
        boot: "boot".into(),
        mono: at,
        utc: UtcMillis(at),
        after_ms: 3_600_000,
        hosts: herdr_threads::test_support::test_host_evidence(0, true),
        legacy_source: Some("covered-source".into()),
    }
}

fn archived(db: &Connection) -> bool {
    db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

/// Quiet-window ticks every minute from `from` through `until`; the first tick
/// at which the thread was archived.
fn archived_between(db: &mut Connection, from: i64, until: i64) -> Option<i64> {
    for at in (from..=until).step_by(60_000) {
        for _ in 0..20 {
            if !archival::advance(db, NODE, &archival_runtime(at))
                .unwrap()
                .has_more
            {
                break;
            }
        }
        if archived(db) {
            return Some(at);
        }
    }
    None
}

fn archival_thread_with_remote(state: &str) -> Connection {
    let db = super::channel_archival::fixture();
    super::channel_archival::thread(&db);
    remote_seat(&db, "r");
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','r',?1)",
        [state],
    )
    .unwrap();
    db
}

// Kills: an archival scan that samples, enrolls or rewrites a remote seat, or
// that starts a quiet period for a thread it must defer.
#[test]
fn archival_scan_changes_no_remote_row() {
    let mut db = archival_thread_with_remote("joined");
    let before = remote_rows(&db);
    let channel = |db: &Connection| -> (Option<i64>, i64, i64) {
        db.query_row(
            "SELECT quiet_mono,scan_phase,scan_cursor FROM channel_archival WHERE thread_id='t'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    };
    assert_eq!(archived_between(&mut db, 0, 60_000), None);
    let early = channel(&db);
    assert_eq!(archived_between(&mut db, 120_000, 3_600_000 + 60_000), None);
    assert_eq!(remote_rows(&db), before, "no remote row changed");
    let sampled: i64 = db
        .query_row(
            "SELECT count(*) FROM seat_archival WHERE seat_id='r'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sampled, 0, "a remote seat has no sampler row");
    assert_eq!(
        channel(&db),
        early,
        "a deferred thread never advances its quiet-period or member scan"
    );
}

// Kills: an archival that treats a joined or invited remote member as inactive
// and archives the thread after the quiet period.
#[test]
fn thread_with_joined_remote_member_never_archived() {
    for state in ["joined", "invited"] {
        let mut db = archival_thread_with_remote(state);
        assert_eq!(
            archived_between(&mut db, 0, 2 * 3_600_000),
            None,
            "a {state} remote member defers archival well past the quiet period"
        );
    }
}

// Kills: a rule that keys on locality alone and so blocks on a retired remote
// seat forever; and a retirement that does not restart the quiet period.
#[test]
fn thread_with_only_retired_remote_member_eligible() {
    let mut db = archival_thread_with_remote("joined");
    retire_remote(&db, "r");
    assert_eq!(
        archived_between(&mut db, 0, 3_600_000 - 60_000),
        None,
        "the quiet period still applies"
    );
    assert_eq!(
        archived_between(&mut db, 3_600_000, 3_600_000),
        Some(3_600_000),
        "a retired remote member does not defer the thread"
    );

    let mut db = archival_thread_with_remote("joined");
    assert_eq!(archived_between(&mut db, 0, 2 * 3_600_000), None);
    retire_remote(&db, "r");
    // The deferral did not touch the quiet period, which has run since the
    // thread was seeded: the retirement alone makes it eligible.
    assert_eq!(
        archived_between(&mut db, 2 * 3_600_000 + 60_000, 4 * 3_600_000),
        Some(2 * 3_600_000 + 60_000),
        "the member's retirement lifts the rule"
    );
}

fn thread_with_both_members(f: &Fixture) {
    f.db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t-both',?1,'both','goal',0,0,0)",
        [NODE],
    )
    .unwrap();
    for seat in ["local", "remote"] {
        f.db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t-both',?1,'joined',5)",
            [seat],
        )
        .unwrap();
    }
}

fn query(f: &Fixture, command: Command) -> CommandResult {
    f.store.query(&command, &read(), &budget()).unwrap()
}

fn text(result: &CommandResult, format: OutputFormat) -> String {
    let spec = OutputSpec {
        format,
        ..OutputSpec::default()
    };
    String::from_utf8(encode_selected(result, &spec).unwrap()).unwrap()
}

// Kills: a read that hides the remote member, mislabels it as an inactive
// local seat, or fails on the `remote` seat state.
#[test]
fn reads_render_remote_state_and_null_panes() {
    let f = fixture();
    thread_with_both_members(&f);

    let participants = query(
        &f,
        Command::Participants(ParticipantsQuery {
            thread: ThreadId::new("t-both"),
            page: page(),
            caller: None,
        }),
    );
    let CommandResult::Participants(list) = &participants else {
        panic!("wrong result")
    };
    let remote = list
        .items
        .iter()
        .find(|p| p.seat.as_str() == "remote")
        .expect("the remote member is listed");
    assert_eq!(remote.availability.as_deref(), Some("remote"));
    assert!(!remote.active && remote.bindings.is_empty());
    let local = list
        .items
        .iter()
        .find(|p| p.seat.as_str() == "local")
        .unwrap();
    assert_eq!(local.availability.as_deref(), Some("available"));
    let compact = text(&participants, OutputFormat::Text);
    assert!(
        compact.contains("remote joined remote\n"),
        "compact row carries the 6-byte state: {compact}"
    );
    assert!(compact.contains("local joined active\n"), "{compact}");
    let human = herdr_threads::cli::human::render(&participants, &read().output).unwrap();
    assert!(
        human
            .lines()
            .any(|l| l.contains("remote joined") && l.ends_with(", remote")),
        "human row carries the state: {human}"
    );

    let CommandResult::Thread(details) = query(
        &f,
        Command::Thread(ThreadQuery {
            thread: ThreadId::new("t-both"),
            page: page(),
            caller: None,
        }),
    ) else {
        panic!("wrong result")
    };
    assert_eq!(details.participants.items.len(), 2);
    assert!(
        details
            .participants
            .items
            .iter()
            .any(|p| p.seat.as_str() == "remote" && p.availability.as_deref() == Some("remote"))
    );

    let CommandResult::Directory(directory) = query(
        &f,
        Command::Directory(DirectoryQuery {
            recent: false,
            membership: Some(SeatId::new("remote")),
            membership_filter: DirectoryMembership::Joined,
            topic_contains: None,
            page: page(),
        }),
    ) else {
        panic!("wrong result")
    };
    let listed: Vec<&str> = directory.items.iter().map(|t| t.thread.as_str()).collect();
    assert_eq!(listed, ["t-both"], "the remote seat's membership is listed");
    assert_eq!(directory.items[0].joined_count, 2, "remote joined counts");

    let seats = query(
        &f,
        Command::Seats(SeatsQuery {
            page: page(),
            target: None,
            include_retired: false,
        }),
    );
    let CommandResult::Seats(listing) = &seats else {
        panic!("wrong result")
    };
    let remote = listing
        .items
        .iter()
        .find(|s| s.seat.as_str() == "remote")
        .expect("the seats listing includes the remote seat");
    assert_eq!(remote.continuity, ContinuityStatus::Remote);
    assert!(remote.target.is_none(), "no pane column");
    assert!(remote.availability.is_none());
    assert!(listing.items.iter().any(|s| s.seat.as_str() == "local"));
    let compact = text(&seats, OutputFormat::Json);
    assert!(compact.contains("\"continuity\":\"remote\""), "{compact}");
    let human = herdr_threads::cli::human::render(&seats, &read().output).unwrap();
    assert!(
        human
            .lines()
            .any(|l| l.starts_with("remote") && l.contains(" remote ")),
        "human seats row carries state remote: {human}"
    );

    let CommandResult::SeatInspect(inspection) = query(
        &f,
        Command::SeatInspect(SeatInspectQuery {
            seat: SeatId::new("remote"),
            page: PageRequest { limit: 1, ..page() },
        }),
    ) else {
        panic!("wrong result")
    };
    assert_eq!(inspection.summary.continuity, ContinuityStatus::Remote);
    assert!(inspection.open_binding.is_none());
}

// Kills: a pane-oriented listing that shows a remote seat: the pane-filtered
// seats listing and the participant locations (no target, no terminal).
#[test]
fn pane_listings_omit_remote() {
    let f = fixture();
    thread_with_both_members(&f);
    let CommandResult::Seats(on_pane) = query(
        &f,
        Command::Seats(SeatsQuery {
            page: page(),
            target: Some(HostTargetId::new("pane")),
            include_retired: true,
        }),
    ) else {
        panic!("wrong result")
    };
    let seats: Vec<&str> = on_pane.items.iter().map(|s| s.seat.as_str()).collect();
    assert_eq!(seats, ["local"], "only the pane's local seat is listed");

    let CommandResult::ParticipantLocations(locations) = query(
        &f,
        Command::ParticipantLocations(ParticipantLocationsQuery {
            thread: ThreadId::new("t-both"),
            seats: vec![SeatId::new("local"), SeatId::new("remote")],
        }),
    ) else {
        panic!("wrong result")
    };
    let remote = locations
        .iter()
        .find(|l| l.seat.as_str() == "remote")
        .expect("the remote member has a location row");
    assert_eq!(remote.continuity, ContinuityStatus::Remote);
    assert!(remote.target.is_none() && remote.terminal.is_none() && remote.incarnation.is_none());
    let local = locations
        .iter()
        .find(|l| l.seat.as_str() == "local")
        .unwrap();
    assert_eq!(
        local.target.as_ref().map(HostTargetId::as_str),
        Some("pane")
    );
}
