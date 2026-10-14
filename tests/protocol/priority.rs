use super::*;
use crate::protocol::commands::{DeliveryMode, WakeOn};
use crate::protocol::time::UtcMillis;

fn p(n: u8) -> DeliveryPriority {
    DeliveryPriority::new(n).unwrap()
}

#[test]
fn delivery_priority_range_default_and_p0() {
    for n in 0..=9 {
        assert_eq!(p(n).get(), n);
    }
    assert!(DeliveryPriority::new(10).is_err());
    assert_eq!(DeliveryPriority::default().get(), 1);
    assert_eq!(DeliveryPriority::DEFAULT.get(), 1);
    assert_eq!(DeliveryPriority::p0().get(), 0);
    assert_eq!(DeliveryPriority::parse("3").unwrap().get(), 3);
    for bad in ["", "-1", "10", "x", " 1"] {
        assert!(DeliveryPriority::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn delivery_priority_stored_value_out_of_range_is_store_corrupt() {
    assert_eq!(DeliveryPriority::from_stored(5).unwrap().get(), 5);
    for bad in [-1i64, 10, i64::MAX] {
        let error = DeliveryPriority::from_stored(bad).unwrap_err();
        assert_eq!(
            error.code,
            crate::protocol::results::ErrorCode::StoreCorrupt
        );
    }
}

#[test]
fn delivery_priority_serde_is_a_bare_integer_and_rejects_out_of_range() {
    assert_eq!(serde_json::to_string(&p(4)).unwrap(), "4");
    assert_eq!(serde_json::from_str::<DeliveryPriority>("0").unwrap(), p(0));
    assert!(serde_json::from_str::<DeliveryPriority>("10").is_err());
    assert!(serde_json::from_str::<DeliveryPriority>("-1").is_err());
}

#[test]
fn delivery_priority_legacy_mapping_both_ways() {
    assert_eq!(DeliveryPriority::from_mode(DeliveryMode::Ordinary), p(0));
    assert_eq!(DeliveryPriority::from_mode(DeliveryMode::Lazy), p(1));
    assert_eq!(
        p(0).to_mode(),
        LegacyMode {
            mode: DeliveryMode::Ordinary,
            lossy: false
        }
    );
    assert_eq!(
        p(1).to_mode(),
        LegacyMode {
            mode: DeliveryMode::Lazy,
            lossy: false
        }
    );
    for n in 2..=9 {
        assert_eq!(
            p(n).to_mode(),
            LegacyMode {
                mode: DeliveryMode::Lazy,
                lossy: true
            }
        );
    }
}

#[test]
fn delivery_priority_marker_only_when_not_default() {
    assert_eq!(p(1).marker(), None);
    assert_eq!(p(0).marker(), Some("[p0]"));
    assert_eq!(p(9).marker(), Some("[p9]"));
    for n in (0..=9).filter(|n| *n != 1) {
        assert_eq!(p(n).marker().unwrap(), format!("[p{n}]"));
        assert_eq!(p(n).marker().unwrap().len(), 4);
    }
}

#[test]
fn nudge_threshold_range_clamping_and_default() {
    assert_eq!(NudgeThreshold::default().get(), 0);
    assert_eq!(NudgeThreshold::new(-1).unwrap().get(), -1);
    assert_eq!(NudgeThreshold::new(-7).unwrap().get(), -1);
    assert_eq!(NudgeThreshold::new(i64::MIN).unwrap().get(), -1);
    assert_eq!(NudgeThreshold::new(9).unwrap().get(), 9);
    assert!(NudgeThreshold::new(10).is_err());
    assert_eq!(NudgeThreshold::parse("-3").unwrap().get(), -1);
    assert!(NudgeThreshold::parse("x").is_err());
    assert_eq!(
        serde_json::to_string(&NudgeThreshold::new(3).unwrap()).unwrap(),
        "3"
    );
    assert_eq!(
        serde_json::from_str::<NudgeThreshold>("-5").unwrap().get(),
        -1
    );
    assert!(serde_json::from_str::<NudgeThreshold>("10").is_err());
    assert!(NudgeThreshold::from_stored(-2).is_err());
    assert!(NudgeThreshold::from_stored(10).is_err());
    assert_eq!(NudgeThreshold::from_stored(-1).unwrap().get(), -1);
}

#[test]
fn nudge_threshold_admits_is_p_le_t_and_negative_admits_all() {
    for t in -1i64..=9 {
        let threshold = NudgeThreshold::new(t).unwrap();
        for n in 0..=9u8 {
            assert_eq!(
                threshold.admits(p(n)),
                t < 0 || i64::from(n) <= t,
                "T={t} p={n}"
            );
        }
    }
    // Priority 0 wakes under every threshold.
    assert!((-1..=9).all(|t| {
        NudgeThreshold::new(t)
            .unwrap()
            .admits(DeliveryPriority::p0())
    }));
}

#[test]
fn nudge_threshold_wake_on_alias_both_ways() {
    assert_eq!(NudgeThreshold::from_wake_on(WakeOn::All).get(), -1);
    assert_eq!(NudgeThreshold::from_wake_on(WakeOn::Nudge).get(), 0);
    assert_eq!(
        NudgeThreshold::new(-1).unwrap().to_wake_on(),
        Some(WakeOn::All)
    );
    assert_eq!(
        NudgeThreshold::new(0).unwrap().to_wake_on(),
        Some(WakeOn::Nudge)
    );
    for t in 1..=9 {
        assert_eq!(
            NudgeThreshold::new(t).unwrap().to_wake_on(),
            None,
            "T={t} is inexpressible"
        );
    }
    // An old client's WakeOnQuery: `all` iff T < 0, else `nudge`.
    assert_eq!(
        NudgeThreshold::new(-1).unwrap().legacy_wake_on(),
        WakeOn::All
    );
    for t in 0..=9 {
        assert_eq!(
            NudgeThreshold::new(t).unwrap().legacy_wake_on(),
            WakeOn::Nudge
        );
    }
}

fn key(
    priority: u8,
    deadline: Option<i64>,
    seq: u64,
    rank: SourceRank,
    ordinal: u64,
) -> DeliveryOrderKey {
    DeliveryOrderKey {
        priority: p(priority),
        deadline: deadline.map(UtcMillis),
        decision_seq: seq,
        rank,
        ordinal,
    }
}

#[test]
fn order_key_tiebreaks() {
    use SourceRank::*;
    // priority first
    assert!(key(0, None, 9, DeliveryRow, 9) < key(1, Some(1), 1, Invitation, 1));
    // within a priority: a deadline beats no deadline, earlier deadline first
    assert!(key(0, Some(500), 9, Receipt, 9) < key(0, None, 1, Invitation, 1));
    assert!(key(0, Some(100), 9, Receipt, 9) < key(0, Some(200), 1, Receipt, 1));
    // equal deadline -> decision_seq
    assert!(key(0, Some(100), 3, DeliveryRow, 9) < key(0, Some(100), 4, Invitation, 1));
    // equal decision_seq -> rank (Invitation 0 < Warning 1 < Receipt 2 < DeliveryRow 3)
    assert!(key(0, None, 5, Invitation, 9) < key(0, None, 5, Warning, 1));
    assert!(key(0, None, 5, Warning, 9) < key(0, None, 5, Receipt, 1));
    assert!(key(0, None, 5, Receipt, 9) < key(0, None, 5, DeliveryRow, 1));
    // equal rank -> ordinal
    assert!(key(0, None, 5, DeliveryRow, 1) < key(0, None, 5, DeliveryRow, 2));
    assert_eq!(
        key(2, None, 5, DeliveryRow, 1).cmp(&key(2, None, 5, DeliveryRow, 1)),
        std::cmp::Ordering::Equal
    );
    let mut keys = vec![
        key(1, None, 2, DeliveryRow, 7),
        key(0, None, 1, Warning, 3),
        key(0, Some(50), 8, Receipt, 2),
        key(0, None, 1, Invitation, 4),
        key(3, None, 1, DeliveryRow, 1),
    ];
    keys.sort();
    assert_eq!(
        keys,
        vec![
            key(0, Some(50), 8, Receipt, 2),
            key(0, None, 1, Invitation, 4),
            key(0, None, 1, Warning, 3),
            key(1, None, 2, DeliveryRow, 7),
            key(3, None, 1, DeliveryRow, 1),
        ]
    );
}

#[test]
fn order_key_continuation_round_trip_and_canonical() {
    use SourceRank::*;
    let samples = [
        key(0, None, 0, Invitation, 0),
        key(9, Some(i64::MAX), u64::MAX, DeliveryRow, u64::MAX),
        key(0, Some(i64::MIN), 1, Receipt, 2),
        key(4, Some(-5), 1 << 40, Warning, 300),
    ];
    for k in samples {
        let mut bytes = Vec::new();
        k.write_compact(&mut bytes);
        bytes.extend_from_slice(b"tail");
        let mut input = bytes.as_slice();
        assert_eq!(DeliveryOrderKey::read_compact(&mut input).unwrap(), k);
        assert_eq!(input, b"tail", "reader consumes exactly the key");
    }
    // header bit 7 set, priority nibble 10, and truncated input are rejected
    for bad in [&[0x80u8, 0, 0][..], &[0x0a, 0, 0][..], &[0x00][..], &[][..]] {
        let mut input = bad;
        assert!(
            DeliveryOrderKey::read_compact(&mut input).is_err(),
            "{bad:?}"
        );
    }
    // an overlong varint (0 encoded as 0x80 0x00) is noncanonical
    let mut input: &[u8] = &[0x30, 0x80, 0x00, 0x00];
    assert!(DeliveryOrderKey::read_compact(&mut input).is_err());
}

fn claim() -> crate::protocol::authority::CallerClaim {
    use crate::protocol::authority::{CallerRole, Harness};
    use crate::protocol::ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId};
    crate::protocol::authority::CallerClaim {
        instance: "11111111-1111-4111-8111-111111111111".into(),
        seat: SeatId::new("s1"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("n1"),
        execution: ExecutionId::new("e1"),
        target: HostTargetId::new("p1"),
        agent_id: None,
        liveness: None,
    }
}

#[test]
fn send_message_priority_is_omitted_when_absent_and_round_trips() {
    use crate::protocol::commands::SendMessage;
    use crate::protocol::ids::{OperationId, ThreadId};
    let send = SendMessage {
        delivery_mode: DeliveryMode::Ordinary,
        priority: None,
        user_intent: None,
        thread: ThreadId::new("t1"),
        body: "hi".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("op1"),
        claim: claim(),
        relays_user: false,
    };
    let absent = serde_json::to_value(&send).unwrap();
    assert!(absent.get("priority").is_none());
    let with = SendMessage {
        priority: Some(p(3)),
        ..send.clone()
    };
    let value = serde_json::to_value(&with).unwrap();
    assert_eq!(value["priority"], 3);
    assert_eq!(
        serde_json::from_value::<SendMessage>(value.clone()).unwrap(),
        with
    );
    assert_eq!(serde_json::from_value::<SendMessage>(absent).unwrap(), send);
    let mut bad = value;
    bad["priority"] = serde_json::json!(10);
    assert!(serde_json::from_value::<SendMessage>(bad).is_err());
}

#[test]
fn delivery_modes_query_and_reply_keep_the_old_shape() {
    use crate::protocol::commands::MessageDeliveryModesQuery;
    use crate::protocol::ids::MessageId;
    use crate::protocol::results::MessageDeliveryMode;
    let query = MessageDeliveryModesQuery {
        messages: vec![MessageId::new("m1")],
        with_priority: false,
    };
    assert_eq!(
        serde_json::to_value(&query).unwrap(),
        serde_json::json!({"messages": ["m1"]})
    );
    let old: MessageDeliveryModesQuery =
        serde_json::from_value(serde_json::json!({"messages": ["m1"]})).unwrap();
    assert!(!old.with_priority);
    let asked = MessageDeliveryModesQuery {
        with_priority: true,
        ..query
    };
    assert_eq!(serde_json::to_value(&asked).unwrap()["with_priority"], true);
    let reply = MessageDeliveryMode {
        message: MessageId::new("m1"),
        delivery_mode: DeliveryMode::Lazy,
        priority: None,
    };
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        serde_json::json!({"message": "m1", "delivery_mode": "lazy"})
    );
    let with = MessageDeliveryMode {
        priority: Some(p(5)),
        ..reply
    };
    assert_eq!(serde_json::to_value(&with).unwrap()["priority"], 5);
    assert!(
        serde_json::from_value::<MessageDeliveryMode>(
            serde_json::json!({"message": "m1", "delivery_mode": "lazy", "bogus": 1})
        )
        .is_err(),
        "the reply stays deny_unknown_fields"
    );
}

#[test]
fn watch_message_and_inbox_items_omit_new_fields_when_absent() {
    use crate::protocol::ids::{MessageId, SeatId, ThreadId};
    use crate::protocol::results::InboxBatchV2Item;
    use crate::protocol::watch::WatchMessage;
    let message = WatchMessage {
        thread: ThreadId::new("t1"),
        thread_name: None,
        sender: Some(SeatId::new("s2")),
        sender_name: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        body: "x".into(),
        body_len: 1,
        truncated: false,
        ack_required: false,
        wakes: false,
        priority: None,
        deadline_at: None,
    };
    let absent = serde_json::to_value(&message).unwrap();
    assert!(absent.get("priority").is_none());
    assert!(absent.get("deadline_at").is_none());
    assert_eq!(
        serde_json::from_value::<WatchMessage>(absent).unwrap(),
        message
    );
    let with = WatchMessage {
        priority: Some(p(0)),
        deadline_at: Some(UtcMillis(1234)),
        ..message
    };
    let value = serde_json::to_value(&with).unwrap();
    assert_eq!(value["priority"], 0);
    assert_eq!(value["deadline_at"], 1234);
    assert_eq!(serde_json::from_value::<WatchMessage>(value).unwrap(), with);

    for lazy in [false, true] {
        let item = |priority| {
            let mut json = serde_json::json!({
                "kind": if lazy { "lazy_message" } else { "message" },
                "thread": "t1", "topic_data": "", "message": "m1", "sequence": 1,
                "sender": null, "body": "b", "body_start": 0, "body_end": 1, "body_len": 1,
            });
            if !lazy {
                json["ack_candidate"] = serde_json::Value::Null;
            }
            if let Some(n) = priority {
                json["priority"] = serde_json::json!(n);
            }
            json
        };
        let old: InboxBatchV2Item = serde_json::from_value(item(None)).unwrap();
        assert!(
            serde_json::to_value(&old)
                .unwrap()
                .get("priority")
                .is_none()
        );
        let new: InboxBatchV2Item = serde_json::from_value(item(Some(2))).unwrap();
        assert_eq!(serde_json::to_value(&new).unwrap()["priority"], 2);
        match (lazy, new) {
            (
                false,
                InboxBatchV2Item::Message {
                    priority, message, ..
                },
            )
            | (
                true,
                InboxBatchV2Item::LazyMessage {
                    priority, message, ..
                },
            ) => {
                assert_eq!(priority, Some(p(2)));
                assert_eq!(message, MessageId::new("m1"));
            }
            (_, other) => panic!("unexpected item {other:?}"),
        }
    }
}

#[test]
fn nudge_threshold_commands_serialize_with_deny_unknown_fields() {
    use crate::protocol::commands::{Command, NudgeThresholdQuery, SetNudgeThreshold};
    use crate::protocol::ids::{OperationId, SeatId};
    use crate::protocol::results::{CommandResult, NudgeThresholdState};
    let query = Command::NudgeThreshold(NudgeThresholdQuery {
        seat: SeatId::new("s1"),
    });
    let text = serde_json::to_string(&query).unwrap();
    assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), query);
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    // The extra field goes wherever the seat field lives: find it by walking.
    fn inject(value: &mut serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) if map.contains_key("seat") => {
                map.insert("bogus".into(), serde_json::json!(1));
                true
            }
            serde_json::Value::Object(map) => map.values_mut().any(inject),
            _ => false,
        }
    }
    assert!(inject(&mut json));
    assert!(serde_json::from_value::<Command>(json).is_err());

    let set = Command::SetNudgeThreshold(SetNudgeThreshold {
        value: NudgeThreshold::new(3).unwrap(),
        operation: OperationId::new("op1"),
        claim: claim(),
    });
    let text = serde_json::to_string(&set).unwrap();
    assert_eq!(serde_json::from_str::<Command>(&text).unwrap(), set);
    assert_eq!(set.accountable_claim(), Some(&claim()));

    let state = CommandResult::NudgeThreshold(NudgeThresholdState {
        seat: SeatId::new("s1"),
        threshold: NudgeThreshold::new(-1).unwrap(),
        changed: true,
    });
    let text = serde_json::to_string(&state).unwrap();
    assert!(text.contains("-1"));
    assert_eq!(serde_json::from_str::<CommandResult>(&text).unwrap(), state);
}

#[test]
fn new_capabilities_are_defined_and_only_send_priority_is_advertised() {
    use crate::protocol::capabilities::*;
    // `send.priority_v1` is advertised with its handler (ht-yha.3).
    assert!(ADVERTISED.contains(&SEND_PRIORITY_V1));
    for name in [SEAT_NUDGE_THRESHOLD_V1, DIRECT_AUTO_JOIN_V1] {
        assert!(!ADVERTISED.contains(&name), "{name}");
    }
    assert_eq!(SEND_PRIORITY_V1, "send.priority_v1");
    assert_eq!(SEAT_NUDGE_THRESHOLD_V1, "seat.nudge_threshold_v1");
    assert_eq!(DIRECT_AUTO_JOIN_V1, "direct.auto_join_v1");
}
