use herdr_threads::protocol::commands::Command;

#[test]
fn hook_receipt_only_completion_round_trips() {
    let claim =
        serde_json::from_str::<serde_json::Value>(ORDINARY).unwrap()["args"]["claim"].clone();
    let value = serde_json::json!({"kind":"complete_inbox_delivery","args":{"messages":[],"receipt_candidates":["m"],"via":"hook_context","operation":"hook-done","claim":claim}});
    let command: Command = serde_json::from_value(value.clone()).unwrap();
    assert!(command.validate().is_ok());
    assert_eq!(serde_json::to_value(command).unwrap(), value);
}

#[test]
fn hook_receipt_completion_preserves_legacy_and_rejects_invalid_sources() {
    let claim =
        serde_json::from_str::<serde_json::Value>(ORDINARY).unwrap()["args"]["claim"].clone();
    let legacy = serde_json::json!({"kind":"complete_inbox_delivery","args":{"messages":["lazy"],"operation":"done","claim":claim}});
    let command: Command = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(command).unwrap(), legacy);
    for (via, ids, accepted) in [
        (Some("hook_context"), vec!["m"], true),
        (None, vec!["m"], false),
        (Some("hook_context"), vec!["m", "m"], false),
    ] {
        let mut value = legacy.clone();
        value["args"]["receipt_candidates"] = serde_json::json!(ids);
        if let Some(via) = via {
            value["args"]["via"] = serde_json::json!(via);
        }
        let command: Command = serde_json::from_value(value).unwrap();
        assert_eq!(command.validate().is_ok(), accepted);
    }
}

#[test]
fn hook_receipt_completion_digest_freezes_source_ids_and_original_claim() {
    use herdr_threads::protocol::commands::CompleteInboxDelivery;
    use herdr_threads::store::control::cooperative_payload_hash;
    let claim =
        serde_json::from_str::<serde_json::Value>(ORDINARY).unwrap()["args"]["claim"].clone();
    let value = serde_json::json!({"messages":[],"receipt_candidates":["m"],"operation":"done","claim":claim,"via":"hook_context"});
    let original: CompleteInboxDelivery = serde_json::from_value(value.clone()).unwrap();
    let digest = cooperative_payload_hash("complete_inbox_delivery", &original).unwrap();
    for field in ["source", "ids", "claim"] {
        let mut changed = value.clone();
        match field {
            "source" => {
                changed.as_object_mut().unwrap().remove("via");
            }
            "ids" => changed["receipt_candidates"] = serde_json::json!(["other"]),
            "claim" => changed["claim"]["target"] = serde_json::json!("other"),
            _ => unreachable!(),
        }
        let changed: CompleteInboxDelivery = serde_json::from_value(changed).unwrap();
        assert_ne!(
            digest,
            cooperative_payload_hash("complete_inbox_delivery", &changed).unwrap(),
            "{field}"
        );
    }
}

const ORDINARY: &str = r#"{"kind":"send_message","args":{"thread":"t","body":"hello","invited_recipients":[],"deadline_millis":null,"operation":"op","claim":{"instance":"34750f98bd59fcfc946da45aaabe933b","host":"/host.sock","seat":"a","binding_generation":1,"role":"top_level","harness":"codex","native_session":"n","execution":"00000000-0000-4000-8000-0000000000aa","target":"pa"}}}"#;

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
fn inbox_batch_v2_continuation_body_bounds() {
    use herdr_threads::protocol::{ids::MessageId, pagination::InboxBatchV2BodyPosition};
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
fn activated_lazy_routes_are_advertised() {
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
    assert!(ADVERTISED.contains(&MESSAGE_DELIVERY_MODES));
    for name in [LAZY_SEND, INBOX_BATCH_V2] {
        assert!(
            ADVERTISED.contains(&name),
            "installed capability {name} must be advertised"
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
    let root = herdr_threads::test_support::scratch::temp_path(format!(
        "lazy-routes-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("store.db");
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        StoreSettings::default(),
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let read = ReadContext {
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: i64 = db
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    let value = serde_json::json!({"kind":"inbox_batch_v2","args":{"seat":"a","page":{"cursor":null,"limit":20,"max_bytes":16384}}});
    let command: Command = serde_json::from_value(value).unwrap();
    assert_eq!(
        store.query(&command, &read, &budget).unwrap_err().code,
        ErrorCode::NotFound
    );
    let metadata: Command = serde_json::from_value(
        serde_json::json!({"kind":"message_delivery_modes","args":{"messages":["m"]}}),
    )
    .unwrap();
    assert_eq!(
        store.query(&metadata, &read, &budget).unwrap_err().code,
        ErrorCode::NotFound
    );
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
