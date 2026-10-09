use super::*;
use crate::{
    ports::{ModChannelRegistration, ModChannelSink, ModChannels},
    protocol::{
        authority::{CallerRole, Harness},
        commands::{AckModDelivered, Command},
        ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId},
        results::HarnessStatesReport,
    },
};
use serde_json::json;
use std::sync::Arc;

fn message() -> WatchMessage {
    WatchMessage {
        thread: ThreadId::new("t1"),
        thread_name: Some("ops".into()),
        sender: Some(SeatId::new("s2")),
        sender_name: Some("alice".into()),
        author_role: Some(AuthorRole::Agent),
        relays_user: false,
        user_intent: None,
        body: "x".repeat(10),
        body_len: 9000,
        truncated: true,
        ack_required: true,
    }
}

fn claim(role: CallerRole) -> CallerClaim {
    CallerClaim {
        instance: "11111111-1111-4111-8111-111111111111".into(),
        seat: SeatId::new("s1"),
        binding_generation: 1,
        role,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("n1"),
        execution: ExecutionId::new("e1"),
        target: HostTargetId::new("p1"),
    }
}

fn round_trip(line: &WatchLine) {
    let text = serde_json::to_string(line).unwrap();
    assert_eq!(&serde_json::from_str::<WatchLine>(&text).unwrap(), line);
}

#[test]
fn watch_line_message_round_trips_with_schema_kind_and_truncated() {
    let line = WatchLine::new("m1", WatchItem::Message(message()));
    let value = serde_json::to_value(&line).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["id"], "m1");
    assert_eq!(value["kind"], "message");
    assert_eq!(value["truncated"], true);
    round_trip(&line);
}

#[test]
fn watch_line_lazy_attention_status_kinds() {
    let lazy = WatchLine::new("m2", WatchItem::Lazy(message()));
    assert_eq!(serde_json::to_value(&lazy).unwrap()["kind"], "lazy");
    round_trip(&lazy);
    let attention = WatchLine::new(
        "attention:4",
        WatchItem::Attention(WatchAttention {
            attention_version: 4,
            text: "attention".into(),
        }),
    );
    let value = serde_json::to_value(&attention).unwrap();
    assert_eq!(value["kind"], "attention");
    assert_eq!(value["attention_version"], 4);
    assert_eq!(value["text"], "attention");
    round_trip(&attention);
    let status = WatchLine::new(
        "status:0",
        WatchItem::Status(WatchStatus {
            state: WatchStatusState::Refused,
            reason: Some(WatchStatusReason::Busy),
            exit: Some(2),
        }),
    );
    let value = serde_json::to_value(&status).unwrap();
    assert_eq!(value["kind"], "status");
    assert_eq!(value["state"], "refused");
    assert_eq!(value["reason"], "busy");
    assert_eq!(value["exit"], 2);
    round_trip(&status);
}

#[test]
fn refusal_close_and_local_reason_exit_codes() {
    use WatchRefusalReason as R;
    for (reason, exit) in [
        (R::NoBinding, 2),
        (R::SessionMismatch, 2),
        (R::Held, 2),
        (R::Unresolved, 2),
        (R::Cooldown, 2),
        (R::Busy, 2),
        (R::Stopping, 2),
        (R::NotClaude, 3),
        (R::Disabled, 3),
    ] {
        assert_eq!(reason.exit_code(), exit, "{reason:?}");
        assert_eq!(
            WatchStatusReason::from(reason).exit_code(),
            exit,
            "{reason:?}"
        );
    }
    use WatchCloseReason as C;
    for (reason, exit) in [
        (C::Replaced, 3),
        (C::BindingChanged, 0),
        (C::Retired, 0),
        (C::Unresolved, 0),
        (C::Stalled, 0),
        (C::Disabled, 3),
        (C::Stopping, 0),
    ] {
        assert_eq!(reason.exit_code(), exit, "{reason:?}");
    }
    use WatchStatusReason as S;
    for (reason, exit) in [
        (S::NoPane, 3),
        (S::EnvDisabled, 3),
        (S::Unsupported, 3),
        (S::DaemonUnavailable, 1),
        (S::Error, 1),
        (S::StreamEnded, 0),
    ] {
        assert_eq!(reason.exit_code(), exit, "{reason:?}");
    }
    assert_eq!(S::from(R::Cooldown), S::Cooldown);
    assert_eq!(S::from(C::BindingChanged), S::BindingChanged);
}

#[test]
fn watch_reason_wire_spellings() {
    let spell = |value: &dyn erased::Ser| value.to_json();
    mod erased {
        pub trait Ser {
            fn to_json(&self) -> String;
        }
        impl<T: serde::Serialize> Ser for T {
            fn to_json(&self) -> String {
                serde_json::to_string(self).unwrap()
            }
        }
    }
    use WatchCloseReason as C;
    use WatchRefusalReason as R;
    use WatchStatusReason as S;
    let refusals = [
        (R::NoBinding, "no_binding"),
        (R::SessionMismatch, "session_mismatch"),
        (R::Held, "held"),
        (R::Unresolved, "unresolved"),
        (R::Cooldown, "cooldown"),
        (R::Busy, "busy"),
        (R::Stopping, "stopping"),
        (R::NotClaude, "not_claude"),
        (R::Disabled, "disabled"),
    ];
    for (reason, name) in refusals {
        assert_eq!(spell(&reason), format!("\"{name}\""));
        assert_eq!(spell(&S::from(reason)), format!("\"{name}\""));
    }
    let closes = [
        (C::Replaced, "replaced"),
        (C::BindingChanged, "binding_changed"),
        (C::Retired, "retired"),
        (C::Unresolved, "unresolved"),
        (C::Stalled, "stalled"),
        (C::Disabled, "disabled"),
        (C::Stopping, "stopping"),
    ];
    for (reason, name) in closes {
        assert_eq!(spell(&reason), format!("\"{name}\""));
        assert_eq!(spell(&S::from(reason)), format!("\"{name}\""));
    }
    let locals = [
        (S::NoPane, "no_pane"),
        (S::EnvDisabled, "env_disabled"),
        (S::Unsupported, "unsupported"),
        (S::DaemonUnavailable, "daemon_unavailable"),
        (S::Error, "error"),
        (S::StreamEnded, "stream_ended"),
    ];
    for (reason, name) in locals {
        assert_eq!(spell(&reason), format!("\"{name}\""));
    }
}

#[test]
fn watch_frames_wire_shape() {
    assert_eq!(
        serde_json::to_value(WatchFrame::Attention { version: 7 }).unwrap(),
        json!({"kind":"attention","args":{"version":7}})
    );
    assert_eq!(
        serde_json::to_value(WatchFrame::Close {
            reason: WatchCloseReason::BindingChanged
        })
        .unwrap(),
        json!({"kind":"close","args":{"reason":"binding_changed"}})
    );
    assert_eq!(
        serde_json::to_value(WatchOutcome::Accepted(WatchAccepted {
            attention_version: 3
        }))
        .unwrap(),
        json!({"kind":"accepted","args":{"attention_version":3}})
    );
    let refused = WatchOutcome::Refused(WatchRefusal {
        reason: WatchRefusalReason::Busy,
        detail: None,
    });
    let text = serde_json::to_string(&refused).unwrap();
    assert!(!text.contains("detail"));
    assert_eq!(
        serde_json::from_str::<WatchOutcome>(&text).unwrap(),
        refused
    );
}

#[test]
fn watch_wire_request_is_disjoint_from_other_first_frames() {
    let request = WatchWireRequest {
        version: PROTOCOL_VERSION,
        request_id: "r1".into(),
        expected_instance: "11111111-1111-4111-8111-111111111111".into(),
        expected_boot: None,
        watch: WatchRequest {
            claim: claim(CallerRole::TopLevel),
        },
    };
    let bytes = serde_json::to_vec(&request).unwrap();
    assert_eq!(WatchWireRequest::decode(&bytes).unwrap(), request);
    assert!(crate::protocol::wire::WireRequest::decode(&bytes).is_err());
    assert!(
        serde_json::from_slice::<crate::protocol::service::ServiceWireRequest>(&bytes).is_err()
    );
    let mut value = serde_json::to_value(&request).unwrap();
    let decode =
        |value: &serde_json::Value| WatchWireRequest::decode(&serde_json::to_vec(value).unwrap());
    value["version"] = json!(5);
    assert!(decode(&value).is_err());
    let mut value = serde_json::to_value(&request).unwrap();
    value["extra"] = json!(1);
    assert!(decode(&value).is_err());
    let mut value = serde_json::to_value(&request).unwrap();
    value["expected_instance"] = json!("not-a-uuid");
    assert!(decode(&value).is_err());
}

#[test]
fn mod_ack_item_line_shape() {
    let item = ModAckItem {
        id: MessageId::new("m1"),
        result: ModAckOutcome::RefusedTerminal,
        reason: Some(ModAckReason::Truncated),
    };
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        json!({"id":"m1","result":"refused_terminal","reason":"truncated"})
    );
    let bare = ModAckItem {
        id: MessageId::new("m1"),
        result: ModAckOutcome::Settled,
        reason: None,
    };
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        json!({"id":"m1","result":"settled"})
    );
    for (outcome, name, counts) in [
        (ModAckOutcome::Settled, "settled", true),
        (ModAckOutcome::AlreadySettled, "already_settled", true),
        (ModAckOutcome::RefusedTerminal, "refused_terminal", false),
        (ModAckOutcome::StaleGeneration, "stale_generation", false),
        (ModAckOutcome::Retryable, "retryable", false),
    ] {
        assert_eq!(serde_json::to_value(outcome).unwrap(), json!(name));
        assert_eq!(outcome.counts_as_mod_ack(), counts, "{name}");
    }
    for (reason, name) in [
        (ModAckReason::Unknown, "unknown"),
        (ModAckReason::NotAddressed, "not_addressed"),
        (ModAckReason::Truncated, "truncated"),
        (ModAckReason::NoLiveChannel, "no_live_channel"),
        (ModAckReason::Busy, "busy"),
        (ModAckReason::Unreachable, "unreachable"),
    ] {
        assert_eq!(serde_json::to_value(reason).unwrap(), json!(name));
    }
}

#[test]
fn mod_delivery_setting_and_via_spellings() {
    assert_eq!(ModDeliverySetting::default(), ModDeliverySetting::On);
    assert_eq!(
        serde_json::from_str::<ModDeliverySetting>("\"off\"").unwrap(),
        ModDeliverySetting::Off
    );
    assert!(serde_json::from_str::<ModDeliverySetting>("\"maybe\"").is_err());
    for (via, name) in [
        (ModDeliveryVia::Context, "context"),
        (ModDeliveryVia::Submit, "submit"),
        (ModDeliveryVia::Append, "append"),
    ] {
        assert_eq!(serde_json::to_value(via).unwrap(), json!(name));
    }
}

#[test]
fn truncation_marker_names_body_command() {
    assert_eq!(
        truncation_marker(&MessageId::new("m9")),
        "…truncated; run herdr-threads body m9"
    );
}

#[test]
fn ledger_entry_round_trips() {
    let entry = ModLedgerEntry {
        at: 1,
        kind: ModLedgerKind::Acked,
        ids: vec!["m1".into()],
        via: Some(ModDeliveryVia::Submit),
        turn: Some("t".into()),
        reason: None,
    };
    let value = serde_json::to_value(&entry).unwrap();
    assert_eq!(
        value,
        json!({"at":1,"kind":"acked","ids":["m1"],"via":"submit","turn":"t"})
    );
    assert_eq!(
        serde_json::from_value::<ModLedgerEntry>(value).unwrap(),
        entry
    );
    for (kind, name) in [
        (ModLedgerKind::Received, "received"),
        (ModLedgerKind::Delivered, "delivered"),
        (ModLedgerKind::Acked, "acked"),
        (ModLedgerKind::Held, "held"),
        (ModLedgerKind::Submit, "submit"),
        (ModLedgerKind::Refused, "refused"),
        (ModLedgerKind::Restart, "restart"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
    }
}

#[test]
fn mod_channel_status_shape() {
    let status = ModChannelStatus {
        mod_delivery: ModDeliverySetting::On,
        live_channels: 1,
        channels: vec![ModChannelEntry {
            seat: SeatId::new("s1"),
            harness: "claude".into(),
            binding_generation: 4,
            connected_since: UtcMillis(10),
            state: ModChannelState::ReconnectGrace,
        }],
    };
    let value = serde_json::to_value(&status).unwrap();
    assert_eq!(value["channels"][0]["state"], "reconnect_grace");
    assert_eq!(value["mod_delivery"], "on");
    assert_eq!(
        serde_json::from_value::<ModChannelStatus>(value).unwrap(),
        status
    );
    for (state, name) in [
        (ModChannelState::Live, "live"),
        (ModChannelState::ReconnectGrace, "reconnect_grace"),
        (ModChannelState::RebindGrace, "rebind_grace"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(name));
    }
}

#[test]
fn constants_match_spec() {
    assert_eq!(WATCH_LINE_SCHEMA, 1);
    assert_eq!(WATCH_BODY_LIMIT_BYTES, 8192);
    assert_eq!(WATCH_PAGE_MAX_ITEMS, 32);
    assert_eq!(WATCH_PAGE_MAX_BYTES, 65536);
    assert_eq!(MAX_WATCH_CONNECTIONS, 64);
    assert_eq!(MOD_RECONNECT_GRACE_MS, 30_000);
    assert_eq!(MOD_REBIND_GRACE_MS, 30_000);
    assert_eq!(MOD_STALL_AFTER_MS, 600_000);
    assert_eq!(MOD_STALL_COOLDOWN_MS, 600_000);
    assert_eq!(MOD_ACK_RETRY_MS, 30_000);
    assert_eq!(MOD_DELIVERY_ENV, "HERDR_THREADS_MOD_DELIVERY");
    assert_eq!(MOD_LEDGER_ENV, "HERDR_THREADS_MOD_LEDGER");
    assert_eq!(
        [
            WATCH_EXIT_STREAM_ENDED,
            WATCH_EXIT_ERROR,
            WATCH_EXIT_RETRY,
            WATCH_EXIT_STOP
        ],
        [0, 1, 2, 3]
    );
}

#[test]
fn mod_watch_capability_is_advertised() {
    use crate::protocol::capabilities::{ADVERTISED, MOD_WATCH};
    assert_eq!(MOD_WATCH, "mod.watch_v1");
    assert!(ADVERTISED.contains(&MOD_WATCH));
}

#[test]
fn provenance_constants() {
    use crate::protocol::authority::{
        COOPERATIVE_MOD_CHANNEL_PROVENANCE, COOPERATIVE_MOD_DELIVERY_PROVENANCE,
    };
    assert_eq!(
        COOPERATIVE_MOD_CHANNEL_PROVENANCE,
        "cooperative_mod_channel"
    );
    assert_eq!(
        COOPERATIVE_MOD_DELIVERY_PROVENANCE,
        "cooperative_mod_delivery"
    );
}

#[test]
fn trust_policy_records_mod_provenances_and_decision() {
    let text =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/TRUST-POLICY.md")).unwrap();
    assert!(text.contains("`cooperative_mod_channel`"));
    assert!(text.contains("`cooperative_mod_delivery`"));
    assert!(text.contains("### Mod delivery is the receipt"));
}

#[test]
fn ack_mod_delivered_command_shape_and_validation() {
    let ack = |ids: Vec<&str>, role: CallerRole| {
        Command::AckModDelivered(AckModDelivered {
            via: ModDeliveryVia::Context,
            messages: ids.into_iter().map(MessageId::new).collect(),
            operation: OperationId::new("op1"),
            claim: claim(role),
        })
    };
    let ok = ack(vec!["m1"], CallerRole::TopLevel);
    assert_eq!(
        serde_json::to_value(&ok).unwrap()["kind"],
        "ack_mod_delivered"
    );
    assert!(ok.validate().is_ok());
    let many: Vec<String> = (0..100).map(|i| format!("m{i}")).collect();
    assert!(
        ack(
            many.iter().map(String::as_str).collect(),
            CallerRole::TopLevel
        )
        .validate()
        .is_ok()
    );
    assert!(ack(vec![], CallerRole::TopLevel).validate().is_err());
    let too_many: Vec<String> = (0..101).map(|i| format!("m{i}")).collect();
    assert!(
        ack(
            too_many.iter().map(String::as_str).collect(),
            CallerRole::TopLevel
        )
        .validate()
        .is_err()
    );
    assert!(
        ack(vec!["m1", "m1"], CallerRole::TopLevel)
            .validate()
            .is_err()
    );
    assert!(ack(vec!["m1"], CallerRole::Subagent).validate().is_err());
}

#[test]
fn harness_states_report_omits_absent_mod_channels() {
    let report = HarnessStatesReport {
        harnesses: vec![],
        mod_channels: None,
    };
    assert!(
        serde_json::to_value(&report)
            .unwrap()
            .get("mod_channels")
            .is_none()
    );
    let old: HarnessStatesReport = serde_json::from_str(r#"{"harnesses":[]}"#).unwrap();
    assert_eq!(old, report);
}

struct DropSink;
impl ModChannelSink for DropSink {
    fn push(&self, _frame: WatchFrame) -> bool {
        false
    }
}

#[test]
fn no_mod_channels_is_inert() {
    let channels = crate::ports::NoModChannels;
    let seat = SeatId::new("s1");
    assert!(!channels.is_live(&seat, 1));
    assert!(!channels.seat_live(&seat));
    assert!(!channels.stalled(&seat, UtcMillis(0)));
    assert!(channels.status().is_none());
    let registration = ModChannelRegistration {
        seat: seat.clone(),
        binding_generation: 1,
        native_session: NativeSessionId::new("n1"),
        harness: Harness::Claude,
        registered_at: UtcMillis(0),
    };
    assert_eq!(
        channels
            .register(registration, Arc::new(DropSink))
            .unwrap_err(),
        WatchRefusalReason::Disabled
    );
    channels.notify(&seat, 1);
    channels.record_attention_push(&seat, 1, UtcMillis(0));
    channels.record_ack(&seat, 1, UtcMillis(0));
    channels.unregister(crate::ports::ModChannelId(1), UtcMillis(0));
    channels.close(&seat, WatchCloseReason::Stopping, UtcMillis(0));
}
