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
