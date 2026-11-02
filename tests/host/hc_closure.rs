//! `HcHost`'s capture and the D10 closure rule against `fake_hc`.

use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use crate::test_support::fake_hc::{FakeHc, FakeTerminal};

fn context() -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: MonoInstant(10_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

fn host(fake: &FakeHc) -> HcHost {
    HcHost::new(HcClient::new(fake.socket()))
}

fn kept(address: &str) -> FakeTerminal {
    let mut t = FakeTerminal::live(address);
    t.keep = true;
    t
}

fn targets(snapshot: &HostSnapshot) -> Vec<(&str, bool)> {
    snapshot
        .targets
        .iter()
        .map(|t| (t.target.as_str(), t.terminal.is_some()))
        .collect()
}

#[test]
fn enumerate_targets_one_connection_hello_and_snapshot() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let mut done = FakeTerminal::live("t2");
    done.keep = true;
    fake.add_terminal(done);
    fake.finish("t2", "exited", Some(0), None);
    fake.add_terminal(FakeTerminal::live("machine/t1"));
    fake.add_terminal(FakeTerminal::live("machine/t2"));
    fake.finish("machine/t2", "exited", Some(0), None);

    let host = host(&fake);
    let snapshot = host.enumerate_targets(&context()).unwrap();

    let calls = fake.calls();
    let methods: Vec<&str> = calls.iter().map(|c| c.method.as_str()).collect();
    assert_eq!(methods, ["hello", "events.subscribe"]);
    assert_eq!(
        calls[0].connection, calls[1].connection,
        "hello and the snapshot share one connection"
    );
    assert_eq!(calls[1].params, serde_json::json!({}));

    // Only live local records are present targets.
    assert_eq!(targets(&snapshot), [("t1", true)]);
    assert_eq!(snapshot.enumeration, EnumerationEvidence::CoherentVerified);
    assert!(snapshot.authorizes_absence_closure());
    assert_eq!(snapshot.boot.as_str(), "hc-core:core-0001");

    // The capture keeps the finished local record with its reason and keep,
    // and drops every machine mirror, live or finished.
    let capture = host.capture().unwrap().unwrap();
    let addresses: Vec<&str> = capture.records.iter().map(|r| r.address.as_str()).collect();
    assert_eq!(addresses, ["t1", "t2"]);
    let t2 = &capture.records[1];
    assert_eq!(t2.state, "finished");
    assert_eq!(t2.reason.as_deref(), Some("exited"));
    assert!(t2.keep);
}

#[test]
fn closure_follows_herdr_core_finality() {
    // (keep, reason) -> the closure of the record the fake produces.
    let rows = [
        (false, "exited", HcClosure::Final),
        (false, "killed", HcClosure::Final),
        (false, "signaled", HcClosure::Final),
        (false, "server_stop", HcClosure::Final),
        (false, "host_shutdown", HcClosure::Final),
        (false, "holder_lost", HcClosure::Final),
        (false, "stopped_by_restart", HcClosure::Final),
        (true, "exited", HcClosure::Final),
        (true, "killed", HcClosure::Final),
        (true, "signaled", HcClosure::PendingRestart),
        (true, "server_stop", HcClosure::PendingRestart),
        (true, "host_shutdown", HcClosure::PendingRestart),
        (true, "holder_lost", HcClosure::PendingRestart),
        (true, "stopped_by_restart", HcClosure::PendingRestart),
        (true, "x_future_reason", HcClosure::Final),
        (false, "x_future_reason", HcClosure::Final),
    ];
    for (keep, reason, expected) in rows {
        let fake = FakeHc::start();
        let mut t = FakeTerminal::live("t1");
        t.keep = keep;
        fake.add_terminal(t);
        assert_eq!(
            HcClosure::of(&host(&fake).capture().unwrap().unwrap().records[0]),
            HcClosure::Present
        );
        fake.finish("t1", reason, None, None);
        let capture = host(&fake).capture().unwrap().unwrap();
        assert_eq!(
            HcClosure::of(&capture.records[0]),
            expected,
            "keep={keep} reason={reason}"
        );
    }
}

#[test]
fn pending_restart_is_shown_without_a_terminal_and_final_is_absent() {
    let fake = FakeHc::start();
    fake.add_terminal(kept("t1"));
    fake.add_terminal(kept("t2"));
    fake.add_terminal(kept("t3"));
    fake.finish("t1", "server_stop", None, None);
    fake.finish("t2", "exited", Some(0), None);
    let snapshot = host(&fake).enumerate_targets(&context()).unwrap();
    assert_eq!(targets(&snapshot), [("t1", false), ("t3", true)]);
    assert!(snapshot.authorizes_absence_closure());
    // The restart brings t1 back as an ordinary present terminal.
    fake.restart("t1");
    let snapshot = host(&fake).enumerate_targets(&context()).unwrap();
    assert_eq!(targets(&snapshot), [("t1", true), ("t3", true)]);
}

#[test]
fn unknown_reason_on_a_live_record_is_present() {
    let fake = FakeHc::start();
    let mut t = kept("t1");
    t.reason = Some("x_future_reason".into());
    t.pid = None;
    fake.add_terminal(t);
    let capture = host(&fake).capture().unwrap().unwrap();
    assert_eq!(HcClosure::of(&capture.records[0]), HcClosure::Present);
}

#[test]
fn unreadable_state_closes_nothing() {
    let fake = FakeHc::start();
    let mut t = FakeTerminal::live("t1");
    t.state = "x_future_state".into();
    fake.add_terminal(t);
    let error = host(&fake).enumerate_targets(&context()).unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert!(format!("{error:?}").contains("cannot read"), "{error:?}");
}

#[test]
fn core_id_rotation_is_a_new_boot_and_a_restart_is_not() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let host = host(&fake);
    let first = host.enumerate_targets(&context()).unwrap();
    // A restart: same id, the event sequence begins again.
    fake.reset_seq();
    let second = host.enumerate_targets(&context()).unwrap();
    assert_eq!(second.boot, first.boot);
    assert_eq!(second.incarnation, first.incarnation);
    assert!(second.observation_sequence > first.observation_sequence);
    // A wipe: a new permanent id.
    fake.rotate_core_id("core-0002");
    let third = host.enumerate_targets(&context()).unwrap();
    assert_eq!(third.boot.as_str(), "hc-core:core-0002");
    assert_ne!(third.incarnation, first.incarnation);
    assert!(third.authorizes_absence_closure());
}

#[test]
fn status_null_unknown_doctor_only() {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    fake.add_terminal(FakeTerminal::live("t2"));
    fake.set_status(
        "t2",
        Some(serde_json::json!({"state": "idle", "app": "claude", "msg": "", "title": ""})),
    );
    let host = host(&fake);
    let capture = host.capture().unwrap().unwrap();
    assert_eq!(StatusSource::of(&capture.records[0]), StatusSource::None);
    assert_eq!(StatusSource::of(&capture.records[1]), StatusSource::Osc7501);
    assert!(capture.records[0].status.is_none());
    // Null is Unknown, never Idle; a reported idle is Unknown too until the
    // composer is read.
    for record in &capture.records {
        assert_eq!(ui_state(record), HostUiState::Unknown);
    }
    let snapshot = host.enumerate_targets(&context()).unwrap();
    assert!(
        snapshot
            .targets
            .iter()
            .all(|t| t.ui == HostUiState::Unknown)
    );

    // The source is shown by doctor, with the target.
    let report = serde_json::json!({"hosts": {"agent_status": capture.status_rows()}});
    let text = crate::cli::doctor::render_debug_text(&report);
    assert!(text.contains("agent_status: t1 source=none\n"), "{text}");
    assert!(text.contains("agent_status: t2 source=osc7501\n"), "{text}");
    // ...and by nothing else: it is not a field of any observation.
    let rows = format!("{:?}", snapshot.targets);
    assert!(!rows.contains("osc7501"), "{rows}");
}
