//! The hc JSON-RPC client against `fake_hc` (in-thread and as the `fake-hc`
//! binary), and the contract of both with the vendored 0.1.15 OpenRPC schema.
//! The schema check is evidence, not a version gate: extra schema fields pass.

use herdr_threads::host::hc::{HcClient, HcError, HcEvent, HcEventKind};
use herdr_threads::test_support::{
    fake_hc::{FakeHc, FakeTerminal, SERVED_METHODS},
    scratch,
    spawn::{self, SpawnOwned},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, process::Stdio, time::Duration};

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn fake_with_t1() -> (FakeHc, HcClient) {
    let fake = FakeHc::start();
    fake.add_terminal(FakeTerminal::live("t1"));
    let client = HcClient::new(fake.socket());
    (fake, client)
}

/// Every client method once.
fn exercise(client: &HcClient) {
    block_on(async {
        client.hello().await.unwrap();
        client.terminal_get("t1").await.unwrap();
        client.events_subscribe_snapshot().await.unwrap();
        client.doc_get("herdr-app").await.unwrap();
    });
}

#[test]
fn hello_round_trip() {
    let (fake, client) = fake_with_t1();
    let reply = block_on(client.hello()).unwrap();
    assert_eq!(reply.value.core.id, "core-0001");
    assert_eq!(reply.value.core.name, "fakehost");
    assert_eq!(reply.value.generation, 1);
    assert_eq!(reply.value.methods, SERVED_METHODS);
    assert_eq!(reply.value.features, ["events"]);
    assert_eq!(reply.witness.endpoint, fake.socket());
    assert_eq!(reply.witness.peer_pid, std::process::id());
}

#[test]
fn terminal_get_round_trip() {
    let (fake, client) = fake_with_t1();
    fake.set_labels("t1", &[("session", "main")]);
    fake.set_turn("t1", 7);
    fake.set_status("t1", Some(json!({"state": "working", "app": "claude"})));
    let t = block_on(client.terminal_get("t1")).unwrap().value;
    assert_eq!(t.address, "t1");
    assert_eq!(t.state, "live");
    assert_eq!(t.turn, 7);
    assert_eq!(t.pid, Some(4242));
    assert_eq!(t.labels.get("session").map(String::as_str), Some("main"));
    assert_eq!(t.status.unwrap().state, "working");
    assert_eq!(t.foreground.unwrap().name, "sh");
    match block_on(client.terminal_get("t9")) {
        Err(HcError::Rpc { kind, .. }) => assert_eq!(kind.as_deref(), Some("not_found")),
        other => panic!("expected not_found, got {other:?}"),
    }
}

#[test]
fn events_subscribe_snapshot_round_trip() {
    let (fake, client) = fake_with_t1();
    fake.add_terminal(FakeTerminal::live("t2"));
    fake.set_stream_before_result(true);
    let snap = block_on(client.events_subscribe_snapshot()).unwrap().value;
    assert_eq!(snap.seq, 2, "two created events");
    let addresses: Vec<_> = snap.terminals.iter().map(|t| t.address.as_str()).collect();
    assert_eq!(addresses, ["t1", "t2"]);
    assert!(matches!(snap.events[0], HcEvent::Heartbeat { seq: 2 }));
    assert!(matches!(
        &snap.events[1],
        HcEvent::Event {
            seq: 3,
            kind: HcEventKind::Turn,
            ..
        }
    ));
    match &snap.events[2] {
        HcEvent::Missed { seq, terminals } => {
            assert_eq!((*seq, terminals.len()), (2, 2));
        }
        other => panic!("expected missed, got {other:?}"),
    }
    assert_eq!(snap.events.len(), 3);
    assert_eq!(snap.skipped, 0);
}

#[test]
fn doc_get_round_trip() {
    let (fake, client) = fake_with_t1();
    fake.set_doc("herdr-app", json!({"names": {"t1": "pane"}}), 5);
    let doc = block_on(client.doc_get("herdr-app")).unwrap().value;
    assert_eq!(doc.version, 5);
    assert_eq!(doc.value["names"]["t1"], "pane");
    let none = block_on(client.doc_get("absent")).unwrap().value;
    assert_eq!((none.version, none.value), (0, Value::Null));
}

#[test]
fn tolerates_unknown_fields_and_events() {
    let (fake, client) = fake_with_t1();
    fake.set_unknown_fields(true);
    fake.set_stream_before_result(true);
    fake.set_unknown_events(true);
    assert_eq!(block_on(client.hello()).unwrap().value.core.id, "core-0001");
    assert_eq!(
        block_on(client.terminal_get("t1")).unwrap().value.address,
        "t1"
    );
    assert_eq!(block_on(client.doc_get("x")).unwrap().value.version, 0);
    let snap = block_on(client.events_subscribe_snapshot()).unwrap().value;
    assert_eq!(snap.terminals.len(), 1);
    assert_eq!(snap.skipped, 2, "unknown event type and unknown method");
    assert!(snap.events.iter().all(|e| !e.is_unknown()));
    assert_eq!(snap.events.len(), 3);
}

#[test]
fn status_null_decodes() {
    let (fake, client) = fake_with_t1();
    fake.set_status("t1", None);
    let line = fake
        .respond(r#"{"jsonrpc":"2.0","id":1,"method":"terminal.get","params":{"terminal":"t1"}}"#);
    assert!(line[0].contains(r#""status":null"#), "{}", line[0]);
    let t = block_on(client.terminal_get("t1")).unwrap().value;
    assert_eq!(t.status, None);
}

#[test]
fn finish_applies_registry_rule() {
    let (fake, client) = fake_with_t1();
    fake.add_terminal(FakeTerminal::live("t2"));
    fake.set_keep("t2", true);
    fake.finish("t1", "exited", Some(0), None);
    fake.finish("t2", "holder_lost", None, None);
    let t1 = block_on(client.terminal_get("t1")).unwrap().value;
    assert_eq!(
        (t1.state.as_str(), t1.reason.as_deref()),
        ("finished", Some("exited"))
    );
    assert_eq!((t1.pid, t1.foreground.is_none()), (Some(4242), true));
    let t2 = block_on(client.terminal_get("t2")).unwrap().value;
    assert_eq!(
        (t2.state.as_str(), t2.reason.as_deref()),
        ("live", Some("holder_lost"))
    );
    assert_eq!(t2.pid, None);
    assert!(t2.keep && t2.foreground.is_none());
    // An unknown reason finishes even a kept terminal and survives decoding.
    fake.set_keep("t1", true);
    fake.add_terminal(FakeTerminal::live("t3"));
    fake.set_keep("t3", true);
    fake.finish("t3", "x_future_reason", None, None);
    let t3 = block_on(client.terminal_get("t3")).unwrap().value;
    assert_eq!(
        (t3.state.as_str(), t3.reason.as_deref()),
        ("finished", Some("x_future_reason"))
    );
    fake.prune("t1");
    assert_eq!(fake.events().last().unwrap()["type"], "removed");
    assert!(block_on(client.terminal_get("t1")).is_err());
}

#[test]
fn core_id_rotation_seq_reset_and_witness_change() {
    let (mut fake, client) = fake_with_t1();
    let first = block_on(client.hello()).unwrap();
    fake.rotate_core_id("core-0002");
    let second = block_on(client.hello()).unwrap();
    assert_eq!(second.value.core.id, "core-0002");
    assert_eq!(
        block_on(client.events_subscribe_snapshot())
            .unwrap()
            .value
            .terminals
            .len(),
        0
    );
    fake.add_terminal(FakeTerminal::live("t1"));
    fake.reset_seq();
    assert_eq!(
        block_on(client.events_subscribe_snapshot())
            .unwrap()
            .value
            .seq,
        0
    );

    let pinned = HcClient::new(fake.socket()).expecting(first.witness.clone());
    assert!(block_on(pinned.hello()).is_ok());
    fake.rebind();
    assert!(matches!(
        block_on(pinned.hello()),
        Err(HcError::WitnessChanged)
    ));
}

#[test]
fn never_sends_forbidden_methods() {
    let (fake, client) = fake_with_t1();
    exercise(&client);
    let calls = fake.calls();
    let methods: BTreeSet<_> = calls.iter().map(|c| c.method.as_str()).collect();
    assert_eq!(
        methods,
        BTreeSet::from(["hello", "terminal.get", "events.subscribe", "doc.get"]),
        "the client exercised every method and nothing else"
    );
    assert!(!methods.contains("terminal.list") && !methods.contains("doc.set"));
    for call in &calls {
        assert!(
            call.params.get("caller").is_none(),
            "{} carries caller: {}",
            call.method,
            call.params
        );
    }
}

fn schema() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/hc/hc-0.1.15.openrpc.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn every_sent_method_and_param_in_vendored_schema() {
    let schema = schema();
    assert_eq!(schema["info"]["version"], "0.1.15");
    let methods = schema["methods"].as_array().unwrap();
    let params_of = |name: &str| -> Option<BTreeSet<String>> {
        methods.iter().find(|m| m["name"] == name).map(|m| {
            m["params"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["name"].as_str().unwrap().to_owned())
                .collect()
        })
    };
    for served in SERVED_METHODS {
        assert!(
            params_of(served).is_some(),
            "fake serves {served}, absent from the schema"
        );
    }
    let (fake, client) = fake_with_t1();
    exercise(&client);
    let calls = fake.calls();
    assert_eq!(calls.len(), 4);
    for call in &calls {
        let allowed = params_of(&call.method)
            .unwrap_or_else(|| panic!("{} is not in the vendored schema", call.method));
        for key in call.params.as_object().unwrap().keys() {
            assert!(
                allowed.contains(key),
                "{}: param {key} is not in the vendored schema ({allowed:?})",
                call.method
            );
        }
    }
    // The hello reply's method list is also schema methods.
    let hello = block_on(client.hello()).unwrap().value;
    for method in hello.methods {
        assert!(params_of(&method).is_some(), "{method}");
    }
}

#[test]
fn fake_hc_bin_serves_same_code() {
    let socket = scratch::temp_path(format!("ht-fake-hc-bin-{}", uuid::Uuid::new_v4().simple()));
    let mut child = spawn::command(env!("CARGO_BIN_EXE_fake-hc"))
        .arg("--socket")
        .arg(&socket)
        .stdin(Stdio::null())
        .spawn_owned()
        .unwrap();
    let client = HcClient::new(&socket);
    let mut hello = None;
    for _ in 0..200 {
        match block_on(client.hello()) {
            Ok(reply) => {
                hello = Some(reply);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    let hello = hello.expect("fake-hc did not come up");
    assert_eq!(hello.value.core.id, "core-0001");
    assert_ne!(
        hello.witness.peer_pid,
        std::process::id(),
        "served by the child"
    );
    let t = block_on(client.terminal_get("t1")).unwrap().value;
    assert_eq!((t.address.as_str(), t.state.as_str()), ("t1", "live"));
    child.stop();
    assert!(block_on(client.hello()).is_err(), "child stopped");
}
