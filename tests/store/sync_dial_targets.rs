//! `store::sync_dial_targets`: the row rules of the dial-target table.
use herdr_threads::{
    node::NodeUuid,
    protocol::results::ErrorCode,
    store::sync_dial_targets::{
        self as targets, DialSource, DialState, DialTransition, Discovered, DiscoverySource,
        MAX_DIAL_TARGETS, NOT_INSTALLED_RETRY_MS, StopReason, TargetId,
    },
    sync::frame::CloseReason,
};
use rusqlite::Connection;

fn db() -> Connection {
    super::channel_archival::fixture()
}

fn node(n: u128) -> NodeUuid {
    NodeUuid::from_u128(n)
}

fn add(db: &mut Connection, dest: &str, now: i64) -> TargetId {
    let tx = db.transaction().unwrap();
    let id = targets::add_explicit(&tx, dest, None, now).unwrap();
    tx.commit().unwrap();
    id
}

fn row(db: &Connection, id: TargetId) -> targets::DialTarget {
    targets::list(db)
        .unwrap()
        .into_iter()
        .find(|row| row.target_id == id)
        .expect("row exists")
}

fn transition(db: &mut Connection, id: TargetId, t: DialTransition, now: i64) {
    let tx = db.transaction().unwrap();
    targets::record_transition(&tx, id, &t, now).unwrap();
    tx.commit().unwrap();
}

#[test]
fn checks_leading_dash_control_chars_stopped_reason() {
    let mut db = db();
    let tx = db.transaction().unwrap();
    for (dest, command, detail) in [
        (
            "-oProxyCommand=x",
            None,
            "dial target must not start with '-'",
        ),
        ("a\nb", None, "dial target contains control characters"),
        ("a\u{7f}b", None, "dial target contains control characters"),
        (
            "host",
            Some("exec x\ry"),
            "dial target contains control characters",
        ),
        ("", None, "dial target destination length out of range"),
    ] {
        let err = targets::add_explicit(&tx, dest, command, 1).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest, "{dest:?}");
        assert_eq!(err.detail, detail, "{dest:?}");
    }
    let err = targets::add_explicit(&tx, &"x".repeat(1025), None, 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    let err =
        targets::upsert_discovered(&tx, "a\tb", DiscoverySource::HerdrMachine, 1).unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::InvalidRequest,
        "discovery rejects control characters"
    );
    let err = targets::upsert_discovered(&tx, "-x", DiscoverySource::HerdrMachine, 1).unwrap_err();
    assert_eq!(
        err.code,
        ErrorCode::InvalidRequest,
        "discovery rejects a leading dash"
    );
    assert!(
        targets::list(&tx).unwrap().is_empty(),
        "nothing was inserted"
    );

    // The table enforces the same rules when written around the module.
    for sql in [
        "INSERT INTO sync_dial_targets(ssh_dest,source,created_at,updated_at) VALUES ('-x','explicit',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,created_at,updated_at) VALUES ('','explicit',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,created_at,updated_at) VALUES ('h','bogus',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,state,created_at,updated_at) VALUES ('h','explicit','stopped',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,stopped_reason,created_at,updated_at) VALUES ('h','explicit','auth_failed',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,state,stopped_reason,created_at,updated_at) VALUES ('h','explicit','stopped','busy',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,state,created_at,updated_at) VALUES ('h','explicit','weird',0,0)",
        "INSERT INTO sync_dial_targets(ssh_dest,source,remote_command,created_at,updated_at) VALUES ('h','explicit','',0,0)",
    ] {
        assert!(tx.execute(sql, []).is_err(), "{sql}");
    }
    tx.execute(
        "INSERT INTO sync_dial_targets(ssh_dest,source,state,stopped_reason,created_at,updated_at) \
         VALUES ('h','explicit','stopped','auth_failed',0,0)",
        [],
    )
    .expect("a stopped row with a reason is valid");
}

#[test]
fn explicit_beats_discovered() {
    let mut db = db();
    let tx = db.transaction().unwrap();
    let found = match targets::upsert_discovered(&tx, "b-host", DiscoverySource::HerdrMachine, 10)
        .unwrap()
    {
        Discovered::Inserted(id) => id,
        other => panic!("{other:?}"),
    };
    tx.commit().unwrap();
    assert_eq!(row(&db, found).source, DialSource::HerdrMachine);

    let tx = db.transaction().unwrap();
    let explicit = targets::add_explicit(&tx, "b-host", Some("exec ht peer --stdio"), 20).unwrap();
    tx.commit().unwrap();
    assert_eq!(explicit, found, "the same row, upgraded");
    let upgraded = row(&db, found);
    assert_eq!(upgraded.source, DialSource::Explicit);
    assert_eq!(
        upgraded.remote_command.as_deref(),
        Some("exec ht peer --stdio")
    );
    assert_eq!((upgraded.created_at, upgraded.updated_at), (10, 20));

    // Re-adding without a command clears the override and resets state.
    transition(
        &mut db,
        found,
        DialTransition::Stopped {
            reason: StopReason::AuthFailed,
            version: "1.0.0".into(),
        },
        30,
    );
    transition(
        &mut db,
        found,
        DialTransition::Backoff {
            attempts: 4,
            next_at: 99,
            error: "boom".into(),
        },
        31,
    );
    let tx = db.transaction().unwrap();
    targets::add_explicit(&tx, "b-host", None, 40).unwrap();
    tx.commit().unwrap();
    let reset = row(&db, found);
    assert_eq!(reset.remote_command, None);
    assert_eq!(reset.state, DialState::Idle);
    assert_eq!(
        (reset.attempts, reset.next_attempt_at, reset.stopped),
        (0, None, None)
    );
    assert!(reset.enabled);
}

#[test]
fn discovery_neither_reenables_nor_overwrites() {
    let mut db = db();
    let id = add(&mut db, "host", 1);
    let tx = db.transaction().unwrap();
    assert_eq!(
        targets::upsert_discovered(&tx, "host", DiscoverySource::HerdrRemoteProcess, 50).unwrap(),
        Discovered::Present(id)
    );
    tx.commit().unwrap();
    let kept = row(&db, id);
    assert_eq!(
        kept.source,
        DialSource::Explicit,
        "explicit row not overwritten"
    );
    assert_eq!(kept.updated_at, 1, "not touched at all");

    // A forgotten row stays disabled and keeps its source.
    let tx = db.transaction().unwrap();
    assert_eq!(targets::forget(&tx, "host", 60).unwrap(), 1);
    assert_eq!(
        targets::upsert_discovered(&tx, "host", DiscoverySource::HerdrMachine, 70).unwrap(),
        Discovered::Present(id)
    );
    tx.commit().unwrap();
    let forgotten = row(&db, id);
    assert!(!forgotten.enabled);
    assert_eq!(forgotten.source, DialSource::Explicit);
    assert_eq!(forgotten.updated_at, 60);

    // A discovered row keeps its source and state against a second sighting.
    let tx = db.transaction().unwrap();
    let Discovered::Inserted(found) =
        targets::upsert_discovered(&tx, "found", DiscoverySource::HerdrRemoteProcess, 80).unwrap()
    else {
        panic!("inserted")
    };
    tx.commit().unwrap();
    transition(&mut db, found, DialTransition::Dialing, 81);
    let tx = db.transaction().unwrap();
    assert_eq!(
        targets::upsert_discovered(&tx, "found", DiscoverySource::HerdrMachine, 90).unwrap(),
        Discovered::Present(found)
    );
    tx.commit().unwrap();
    let kept = row(&db, found);
    assert_eq!(kept.source, DialSource::HerdrRemoteProcess);
    assert_eq!(kept.state, DialState::Dialing);
}

#[test]
fn forget_disables_only() {
    let mut db = db();
    let id = add(&mut db, "host", 1);
    transition(&mut db, id, DialTransition::Linked { node: node(7) }, 2);
    let tx = db.transaction().unwrap();
    assert_eq!(targets::forget(&tx, "no-such", 3).unwrap(), 0);
    // By learned node uuid, not only by destination.
    assert_eq!(targets::forget(&tx, &node(7).to_string(), 4).unwrap(), 1);
    assert_eq!(
        targets::forget(&tx, "host", 5).unwrap(),
        0,
        "already disabled"
    );
    tx.commit().unwrap();

    assert_eq!(targets::list(&db).unwrap().len(), 1, "no row deleted");
    assert!(targets::enabled(&db).unwrap().is_empty());
    let kept = row(&db, id);
    assert_eq!(
        (kept.enabled, kept.state, kept.learned_node),
        (false, DialState::Linked, Some(node(7)))
    );

    // Adding it again enables the same row.
    let again = add(&mut db, "host", 6);
    assert_eq!(again, id);
    assert_eq!(targets::enabled(&db).unwrap().len(), 1);
}

#[test]
fn cap_64_invalid_request_vs_skipped() {
    let mut db = db();
    for n in 0..MAX_DIAL_TARGETS {
        add(&mut db, &format!("h{n}"), 1);
    }
    let tx = db.transaction().unwrap();
    let err = targets::add_explicit(&tx, "one-too-many", None, 2).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    assert_eq!(err.detail, "too many dial targets (64)");
    assert_eq!(
        targets::upsert_discovered(&tx, "found", DiscoverySource::HerdrMachine, 2).unwrap(),
        Discovered::Skipped,
        "discovery over the cap is skipped, not an error"
    );
    // An existing enabled target can be re-added at the cap.
    targets::add_explicit(&tx, "h0", Some("cmd"), 3).expect("no growth");
    assert!(matches!(
        targets::upsert_discovered(&tx, "h1", DiscoverySource::HerdrMachine, 3),
        Ok(Discovered::Present(_))
    ));
    // A forgotten row frees a slot; re-adding a disabled one counts again.
    assert_eq!(targets::forget(&tx, "h5", 4).unwrap(), 1);
    targets::add_explicit(&tx, "one-more", None, 5).expect("slot freed");
    assert_eq!(
        targets::add_explicit(&tx, "h5", None, 6)
            .unwrap_err()
            .detail,
        "too many dial targets (64)"
    );
    tx.commit().unwrap();
    assert_eq!(targets::list(&db).unwrap().len(), MAX_DIAL_TARGETS + 1);
    assert_eq!(targets::enabled(&db).unwrap().len(), MAX_DIAL_TARGETS);
}

#[test]
fn every_dial_transition_applied() {
    let mut db = db();
    let id = add(&mut db, "host", 1);

    transition(&mut db, id, DialTransition::Dialing, 10);
    let r = row(&db, id);
    assert_eq!(
        (r.state, r.next_attempt_at, r.updated_at),
        (DialState::Dialing, None, 10)
    );

    transition(
        &mut db,
        id,
        DialTransition::Backoff {
            attempts: 3,
            next_at: 5_000,
            error: "é".repeat(600),
        },
        20,
    );
    let r = row(&db, id);
    assert_eq!(
        (r.state, r.attempts, r.next_attempt_at),
        (DialState::Backoff, 3, Some(5_000))
    );
    assert_eq!(
        r.last_error.as_deref().map(|e| e.chars().count()),
        Some(512),
        "error cut to 512 characters"
    );
    assert_eq!(r.last_error_at, Some(20));

    transition(&mut db, id, DialTransition::Linked { node: node(9) }, 30);
    let r = row(&db, id);
    assert_eq!(
        (r.state, r.attempts, r.next_attempt_at),
        (DialState::Linked, 0, None)
    );
    assert_eq!(
        (r.learned_node, r.last_linked_at),
        (Some(node(9)), Some(30))
    );
    assert_eq!(r.last_error_at, Some(20), "the last error stays as history");

    transition(&mut db, id, DialTransition::NotInstalled, 40);
    let r = row(&db, id);
    assert_eq!(r.state, DialState::NotInstalled);
    assert_eq!(r.next_attempt_at, Some(40 + NOT_INSTALLED_RETRY_MS));
    assert_eq!(r.learned_node, Some(node(9)), "learned node survives");

    transition(
        &mut db,
        id,
        DialTransition::Stopped {
            reason: CloseReason::EnvelopeUnsupported.try_into().unwrap(),
            version: "1.2.3".into(),
        },
        50,
    );
    let r = row(&db, id);
    assert_eq!(r.state, DialState::Stopped);
    assert_eq!(
        r.stopped,
        Some((StopReason::EnvelopeUnsupported, Some("1.2.3".into())))
    );
    assert_eq!(r.next_attempt_at, None);

    // Leaving `stopped` clears the reason, so the table's CHECK holds.
    transition(&mut db, id, DialTransition::Dialing, 60);
    let r = row(&db, id);
    assert_eq!((r.state, r.stopped), (DialState::Dialing, None));

    // An unknown target is a caller bug, not a silent no-op.
    let tx = db.transaction().unwrap();
    let err =
        targets::record_transition(&tx, TargetId(9999), &DialTransition::Dialing, 1).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
}

#[test]
fn stop_reason_is_the_stop_dialing_subset_of_close_reason() {
    for reason in [
        CloseReason::EnvelopeUnsupported,
        CloseReason::SelfLink,
        CloseReason::AuthFailed,
        CloseReason::NodeCollision,
    ] {
        let stop = StopReason::try_from(reason).expect("a stop-class reason");
        assert_eq!(serde_json::to_value(reason).unwrap(), stop.as_str());
    }
    for reason in [
        CloseReason::Busy,
        CloseReason::Shutdown,
        CloseReason::ProtocolError,
        CloseReason::Unknown,
    ] {
        assert!(StopReason::try_from(reason).is_err(), "{reason:?} retries");
    }
}

#[test]
fn clear_stale_stops_by_version() {
    let mut db = db();
    let old = add(&mut db, "old", 1);
    let current = add(&mut db, "current", 1);
    let running = add(&mut db, "running", 1);
    for (id, version) in [(old, "0.9.0"), (current, "1.0.0")] {
        transition(
            &mut db,
            id,
            DialTransition::Stopped {
                reason: StopReason::EnvelopeUnsupported,
                version: version.into(),
            },
            2,
        );
    }
    transition(&mut db, running, DialTransition::Dialing, 2);
    // A stopped row with no recorded version is stale too.
    let tx = db.transaction().unwrap();
    tx.execute(
        "INSERT INTO sync_dial_targets(ssh_dest,source,state,stopped_reason,created_at,updated_at) \
         VALUES ('unversioned','explicit','stopped','self_link',0,0)",
        [],
    )
    .unwrap();
    assert_eq!(targets::clear_stale_stops(&tx, "1.0.0").unwrap(), 2);
    tx.commit().unwrap();

    let r = row(&db, old);
    assert_eq!(
        (r.state, r.stopped, r.attempts, r.next_attempt_at),
        (DialState::Idle, None, 0, None)
    );
    assert_eq!(
        row(&db, current).state,
        DialState::Stopped,
        "same version stays stopped"
    );
    assert_eq!(row(&db, running).state, DialState::Dialing);
    assert_eq!(
        targets::list(&db).unwrap().len(),
        4,
        "rows are reset, never deleted"
    );
}

/// A re-add that races a link coming up (the dialer linked between two `peer add`s)
/// must not report the live link as `idle`; a changed command still resets it.
#[test]
fn readding_a_linked_target_keeps_it_linked_unless_its_command_changes() {
    let mut db = db();
    let id = add(&mut db, "alice@box", 1);
    transition(&mut db, id, DialTransition::Linked { node: node(7) }, 2);
    assert_eq!(add(&mut db, "alice@box", 3), id);
    assert_eq!(
        row(&db, id).state,
        DialState::Linked,
        "same command: still linked"
    );
    let tx = db.transaction().unwrap();
    targets::add_explicit(&tx, "alice@box", Some("herdr-threads peer --stdio"), 4).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        row(&db, id).state,
        DialState::Idle,
        "a new command re-dials"
    );
}

#[test]
fn retire_unseen_deletes_only_never_linked_process_rows_absent_from_the_pass() {
    let mut db = db();
    let tx = db.transaction().unwrap();
    let discovered = |tx: &rusqlite::Transaction<'_>, dest, source| {
        let Discovered::Inserted(id) = targets::upsert_discovered(tx, dest, source, 10).unwrap()
        else {
            panic!("inserted {dest}")
        };
        id
    };
    let stray = discovered(&tx, "test-only", DiscoverySource::HerdrRemoteProcess);
    let present = discovered(&tx, "open", DiscoverySource::HerdrRemoteProcess);
    let once_linked = discovered(&tx, "linked-once", DiscoverySource::HerdrRemoteProcess);
    let forgotten = discovered(&tx, "forgotten", DiscoverySource::HerdrRemoteProcess);
    let mid_dial = discovered(&tx, "mid-dial", DiscoverySource::HerdrRemoteProcess);
    let machine = discovered(&tx, "machine", DiscoverySource::HerdrMachine);
    let explicit = targets::add_explicit(&tx, "explicit", None, 10).unwrap();
    targets::forget(&tx, "forgotten", 20).unwrap();
    tx.commit().unwrap();
    transition(
        &mut db,
        once_linked,
        DialTransition::Linked { node: node(7) },
        30,
    );
    // Its link may be up before the Dialer records it: kept until it settles.
    transition(&mut db, mid_dial, DialTransition::Dialing, 30);

    let tx = db.transaction().unwrap();
    assert_eq!(
        targets::retire_unseen_remote_processes(&tx, &["open"]).unwrap(),
        targets::Retired {
            deleted: 1,
            deferred: 1
        },
        "the stray row goes; the unseen row mid-dial is kept and counted"
    );
    tx.commit().unwrap();
    let left: Vec<TargetId> = targets::list(&db)
        .unwrap()
        .into_iter()
        .map(|row| row.target_id)
        .collect();
    assert!(!left.contains(&stray), "the stray client's row is deleted");
    for kept in [present, once_linked, forgotten, mid_dial, machine, explicit] {
        assert!(left.contains(&kept), "{kept:?} kept");
    }
    assert!(
        !row(&db, forgotten).enabled,
        "a forgotten row stays forgotten"
    );

    // Seen again later, the destination is discovered afresh.
    let tx = db.transaction().unwrap();
    assert!(matches!(
        targets::upsert_discovered(&tx, "test-only", DiscoverySource::HerdrRemoteProcess, 40)
            .unwrap(),
        Discovered::Inserted(_)
    ));
}

fn events(db: &Connection) -> Vec<herdr_threads::store::sync_link_events::StoredLinkEvent> {
    herdr_threads::store::sync_link_events::page(db, None, None, 64)
        .unwrap()
        .0
}

// Kills: a stale error shown on a healthy link (ht-peer-hygiene), and a
// current one hidden while the target backs off.
#[test]
fn current_error_only_while_not_linked_and_newer_than_the_link() {
    let mut db = db();
    let id = add(&mut db, "host", 1);
    let backoff = |error: &str| DialTransition::Backoff {
        attempts: 1,
        next_at: 100,
        error: error.into(),
    };
    transition(&mut db, id, DialTransition::Dialing, 2);
    transition(&mut db, id, backoff("no route"), 3);
    assert_eq!(
        row(&db, id).current_error(),
        Some("no route"),
        "never linked"
    );
    transition(&mut db, id, DialTransition::Linked { node: node(1) }, 4);
    let linked = row(&db, id);
    assert_eq!(
        linked.last_error.as_deref(),
        Some("no route"),
        "kept for history"
    );
    assert_eq!(linked.current_error(), None, "a healthy link shows none");
    transition(&mut db, id, backoff("peer closed the link: Shutdown"), 5);
    assert_eq!(
        row(&db, id).current_error(),
        Some("peer closed the link: Shutdown"),
        "the link ended: current again"
    );
    transition(&mut db, id, DialTransition::Dialing, 6);
    assert_eq!(
        row(&db, id).current_error(),
        Some("peer closed the link: Shutdown"),
        "still current while redialing"
    );
}

// Kills: link ends double-recorded (the coordinator records ups and downs),
// a dial failure or stop missing from the history, and a backoff loop that
// floods it.
#[test]
fn dial_outcomes_are_recorded_and_repeats_collapse() {
    let mut db = db();
    let id = add(&mut db, "host", 1);
    let backoff = |error: &str| DialTransition::Backoff {
        attempts: 1,
        next_at: 100,
        error: error.into(),
    };
    transition(&mut db, id, DialTransition::Dialing, 2);
    transition(&mut db, id, backoff("no route"), 3);
    transition(&mut db, id, DialTransition::Dialing, 4);
    transition(&mut db, id, backoff("no route"), 5);
    transition(&mut db, id, DialTransition::Linked { node: node(1) }, 6);
    transition(&mut db, id, backoff("peer closed the link: Shutdown"), 7);
    transition(&mut db, id, DialTransition::NotInstalled, 8);
    transition(
        &mut db,
        id,
        DialTransition::Stopped {
            reason: StopReason::AuthFailed,
            version: "1".into(),
        },
        9,
    );
    let rows = events(&db);
    let shape: Vec<(&str, i64, Option<&str>)> = rows
        .iter()
        .map(|e| (e.kind.as_str(), e.n, e.detail.as_deref()))
        .collect();
    assert_eq!(
        shape,
        vec![
            ("stopped", 1, Some("auth_failed")),
            ("not_installed", 1, None),
            ("dial_failed", 2, Some("no route")),
        ],
        "newest first; the end of a link is the coordinator's"
    );
    let failed = rows.last().unwrap();
    assert_eq!((failed.at, failed.last_at), (3, 5));
    assert_eq!(failed.dest.as_deref(), Some("host"));
    assert_eq!(failed.origin, "dialed");
    assert_eq!(rows[0].node.as_deref(), Some(node(1).to_string().as_str()));
}

// Kills: an unbounded history, a detail stored past its bound, and a page
// that skips or repeats rows.
#[test]
fn link_events_are_bounded_and_paged() {
    use herdr_threads::store::sync_link_events::{
        self as history, LinkEvent, LinkEventKind, LinkOrigin, MAX_AGE_MS, MAX_DETAIL_BYTES,
        MAX_ROWS,
    };
    let mut db = db();
    let record = |db: &mut Connection, at: i64, detail: String| {
        let tx = db.transaction().unwrap();
        history::record(
            &tx,
            &LinkEvent {
                at,
                kind: LinkEventKind::Down,
                origin: LinkOrigin::Inbound,
                node: Some(node(7)),
                dest: None,
                detail: Some(detail),
            },
        )
        .unwrap();
        tx.commit().unwrap();
    };
    record(&mut db, 1, "é".repeat(MAX_DETAIL_BYTES));
    let stored = events(&db).remove(0).detail.unwrap();
    assert_eq!(stored.len(), MAX_DETAIL_BYTES, "cut on a char boundary");
    for i in 0..(MAX_ROWS + 5) {
        record(&mut db, 2 + i, format!("e{i}"));
    }
    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM sync_link_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, MAX_ROWS);
    let (first, more) = history::page(&db, Some(node(7)), None, 3).unwrap();
    assert!(more);
    let (second, _) = history::page(&db, None, Some(first[2].seq), 3).unwrap();
    assert_eq!(second[0].seq, first[2].seq - 1, "no gap, no repeat");
    assert!(
        history::page(&db, Some(node(8)), None, 3)
            .unwrap()
            .0
            .is_empty()
    );
    record(&mut db, 2_000 + MAX_AGE_MS, "late".into());
    assert_eq!(events(&db).len(), 1, "older than 30 days is pruned");
}
