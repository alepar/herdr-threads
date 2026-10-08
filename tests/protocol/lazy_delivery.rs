use herdr_threads::protocol::commands::Command;

const ORDINARY: &str = r#"{"kind":"send_message","args":{"thread":"t","body":"hello","invited_recipients":[],"deadline_millis":null,"operation":"op","claim":{"instance":"i","seat":"a","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}}"#;

fn lazy_value() -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(ORDINARY).unwrap();
    value["args"]["delivery_mode"] = serde_json::json!("lazy");
    value
}

#[test]
fn ordinary_send_delivery_mode_preserves_wire_bytes() {
    let command: Command = serde_json::from_str(ORDINARY).unwrap();
    assert_eq!(serde_json::to_string(&command).unwrap(), ORDINARY);
}

#[test]
fn lazy_send_delivery_mode_round_trips() {
    let value = lazy_value();
    let command: Command = serde_json::from_value(value.clone())
        .expect("lazy delivery mode is a supported wire contract");
    assert!(command.validate().is_ok());
    assert_eq!(serde_json::to_value(command).unwrap(), value);
}

#[test]
fn lazy_send_rejects_receipts_and_deadlines() {
    for (field, bad) in [
        ("invited_recipients", serde_json::json!(["recipient"])),
        ("deadline_millis", serde_json::json!(1)),
    ] {
        let mut value = lazy_value();
        value["args"][field] = bad;
        let command: Command = serde_json::from_value(value).unwrap();
        assert!(command.validate().is_err(), "lazy must reject {field}");
    }
}

#[test]
fn message_delivery_modes_exact_id_bound() {
    for (count, accepted) in [(0, false), (1, true), (100, true), (101, false)] {
        let command: Command = serde_json::from_value(serde_json::json!({"kind":"message_delivery_modes","args":{"messages":(0..count).map(|n| format!("m{n}")).collect::<Vec<_>>()}})).unwrap();
        assert_eq!(command.validate().is_ok(), accepted, "count={count}");
    }
}

#[test]
fn complete_inbox_delivery_rejects_subagent_and_bad_batch() {
    let claim =
        serde_json::from_str::<serde_json::Value>(ORDINARY).unwrap()["args"]["claim"].clone();
    for (count, accepted) in [(0, false), (1, true), (100, true), (101, false)] {
        let value = serde_json::json!({"kind":"complete_inbox_delivery","args":{"messages":(0..count).map(|n| format!("m{n}")).collect::<Vec<_>>(), "operation":"done", "claim":claim}});
        let command: Command = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(command.validate().is_ok(), accepted, "count={count}");
        let mut subagent = value;
        subagent["args"]["claim"]["role"] = serde_json::json!("subagent");
        let command: Command = serde_json::from_value(subagent).unwrap();
        assert!(command.validate().is_err());
    }
}

#[test]
fn inbox_batch_v2_keeps_v1_shapes() {
    use herdr_threads::protocol::results::{InboxBatchItem, InboxBatchV2Item};
    let fixtures = [
        serde_json::json!({"kind":"invitation","thread":"t","topic_data":"topic","invitation":"inv","required_service":null}),
        serde_json::json!({"kind":"message","thread":"t","topic_data":"topic","message":"m","sequence":1,"sender":"a","body":"hi","body_start":0,"body_end":2,"body_len":2,"ack_candidate":"m"}),
        serde_json::json!({"kind":"warning","thread":"t","topic_data":"topic","warning":"w","sequence":3}),
    ];
    for value in fixtures {
        let v1: InboxBatchItem = serde_json::from_value(value.clone()).unwrap();
        let v2: InboxBatchV2Item = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(v1).unwrap(), value);
        assert_eq!(serde_json::to_value(v2).unwrap(), value);
    }
    let lazy = serde_json::json!({"kind":"lazy_message","thread":"t","topic_data":"topic","message":"m","sequence":1,"sender":"a","author_role":"human","relays_user":true,"user_intent":"rule","author_role_backfilled":true,"body":"hi","body_start":0,"body_end":2,"body_len":2});
    let v2: InboxBatchV2Item = serde_json::from_value(lazy.clone()).unwrap();
    assert_eq!(serde_json::to_value(v2).unwrap(), lazy);
    assert!(serde_json::from_value::<InboxBatchItem>(lazy).is_err());
}

#[test]
fn inbox_batch_v2_continuation_type_round_trip_and_body_bounds() {
    use herdr_threads::protocol::{
        ids::MessageId,
        pagination::{InboxBatchV2BodyPosition, InboxBatchV2CursorState},
    };
    // Compact future state only: no production cursor codec is asserted here.
    let value = serde_json::json!({"s":"a","g":1,"e":"execution","p":"l", "a":{"ia":0,"io":0,"id":false,"ip":false,"if":null,"r":null,"d":false,"p":false,"rf":null,"pa":0,"ph":0,"ma":0,"mh":0,"n":false,"w":null,"wo":null}, "la":2,"lh":100,"dh":20,"b":{"m":"m","o":2,"l":5}});
    let state: InboxBatchV2CursorState = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(state.seat.as_str(), "a");
    assert_eq!(serde_json::to_value(state).unwrap(), value);
    let mut unbound = value;
    unbound.as_object_mut().unwrap().remove("g");
    unbound.as_object_mut().unwrap().remove("e");
    let state: InboxBatchV2CursorState = serde_json::from_value(unbound.clone()).unwrap();
    assert!(state.binding_generation.is_none());
    assert!(state.execution.is_none());
    assert_eq!(serde_json::to_value(state).unwrap(), unbound);
    for (offset, body_len, valid) in [
        (0, 0, true),
        (0, 5, true),
        (5, 5, true),
        (6, 5, false),
        (1, 0, false),
    ] {
        let body = InboxBatchV2BodyPosition {
            message: MessageId::new("m"),
            offset,
            body_len,
        };
        assert_eq!(
            body.validate().is_ok(),
            valid,
            "offset={offset}, length={body_len}"
        );
    }
}

#[test]
fn inert_lazy_routes_are_not_advertised() {
    use herdr_threads::{
        ports::{ReadContext, StorePort},
        protocol::{
            capabilities::{ADVERTISED, INBOX_BATCH_V2, LAZY_SEND, MESSAGE_DELIVERY_MODES},
            output::OutputSpec,
            results::ErrorCode,
            time::{CallBudget, Clock, MonoInstant, UtcMillis},
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    use std::sync::Arc;
    for name in [LAZY_SEND, INBOX_BATCH_V2, MESSAGE_DELIVERY_MODES] {
        assert!(
            !ADVERTISED.contains(&name),
            "inert capability {name} must not be advertised"
        );
    }
    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    let root = std::env::temp_dir().join(format!("lazy-routes-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("store.db");
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(FixedClock)),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let read = ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: i64 = db
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    for value in [
        serde_json::json!({"kind":"inbox_batch_v2","args":{"seat":"a","page":{"cursor":null,"limit":20,"max_bytes":16384}}}),
        serde_json::json!({"kind":"message_delivery_modes","args":{"messages":["m"]}}),
    ] {
        let command: Command = serde_json::from_value(value).unwrap();
        assert_eq!(
            store.query(&command, &read, &budget).unwrap_err().code,
            ErrorCode::Unsupported
        );
    }
    // Observe all database state, including non-message bookkeeping.
    assert_eq!(
        db.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    let operations: i64 = db
        .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(operations, 0);
    drop(db);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
