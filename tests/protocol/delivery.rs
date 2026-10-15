//! Delivery seam wire (delivery-policy spec DP4): round-trips, validation,
//! the `Command` envelope, and the inert (unserved) dispatch answer.
use super::*;
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness},
    capabilities,
    commands::{Command, MAX_BATCH_ITEMS, PermitMutation},
    ids::{ExecutionId, HostTargetId, MessageId, NativeSessionId, OperationId, SeatId},
};
use serde_json::json;

fn claim(role: CallerRole) -> CallerClaim {
    CallerClaim {
        instance: "11111111111141118111111111111111".into(),
        seat: SeatId::new("s1"),
        binding_generation: 1,
        role,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("n1"),
        execution: ExecutionId::new("e1"),
        target: HostTargetId::new("p1"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}

fn pull() -> DeliveryPull {
    DeliveryPull {
        claim: claim(CallerRole::TopLevel),
        seen_attention_version: None,
        marker_prefix: vec![],
    }
}

fn confirm(n: usize) -> DeliveryConfirm {
    DeliveryConfirm {
        claim: claim(CallerRole::TopLevel),
        ids: (0..n).map(|i| MessageId::new(format!("m{i}"))).collect(),
        operation: OperationId::new("op-1"),
    }
}

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let text = serde_json::to_string(value).unwrap();
    assert_eq!(&serde_json::from_str::<T>(&text).unwrap(), value);
}

#[test]
fn pull_round_trips_and_omits_absent_optional_fields() {
    let bare = pull();
    let value = serde_json::to_value(&bare).unwrap();
    assert!(value.get("seen_attention_version").is_none());
    assert!(value.get("marker_prefix").is_none());
    round_trip(&bare);

    let full = DeliveryPull {
        seen_attention_version: Some(77),
        marker_prefix: vec!["ht".into(), "--state-dir".into(), "/s".into()],
        ..pull()
    };
    let value = serde_json::to_value(&full).unwrap();
    assert_eq!(value["seen_attention_version"], 77);
    assert_eq!(value["marker_prefix"], json!(["ht", "--state-dir", "/s"]));
    round_trip(&full);
    // The two optional fields may be absent on the wire.
    let mut minimal = serde_json::to_value(&bare).unwrap();
    minimal.as_object_mut().unwrap().retain(|k, _| k == "claim");
    assert_eq!(
        serde_json::from_value::<DeliveryPull>(minimal).unwrap(),
        bare
    );
}

#[test]
fn pull_reply_variants_use_kind_args_snake_case() {
    let unchanged = DeliveryPullReply::Unchanged {
        attention_version: 5,
    };
    assert_eq!(
        serde_json::to_value(&unchanged).unwrap(),
        json!({"kind":"unchanged","args":{"attention_version":5}})
    );
    round_trip(&unchanged);

    let page = DeliveryPullReply::Page(DeliveryPage {
        attention_version: 6,
        text: "HEADER".into(),
        offered: vec![MessageId::new("m1")],
        truncated: vec![MessageId::new("m2")],
        more: true,
    });
    let value = serde_json::to_value(&page).unwrap();
    assert_eq!(value["kind"], "page");
    assert_eq!(value["args"]["offered"], json!(["m1"]));
    assert_eq!(value["args"]["truncated"], json!(["m2"]));
    assert_eq!(value["args"]["more"], true);
    round_trip(&page);

    for (reason, name) in [
        (DeliveryRefusalReason::NoBinding, "no_binding"),
        (DeliveryRefusalReason::SessionMismatch, "session_mismatch"),
        (DeliveryRefusalReason::Held, "held"),
        (DeliveryRefusalReason::Unresolved, "unresolved"),
        (DeliveryRefusalReason::NoHookPull, "no_hook_pull"),
        (DeliveryRefusalReason::Busy, "busy"),
    ] {
        let refused = DeliveryPullReply::Refused { reason };
        assert_eq!(
            serde_json::to_value(&refused).unwrap(),
            json!({"kind":"refused","args":{"reason":name}})
        );
        round_trip(&refused);
    }
}

#[test]
fn confirm_and_report_round_trip_with_every_outcome() {
    round_trip(&confirm(2));
    let outcomes = [
        (DeliveryConfirmOutcome::Settled, "settled"),
        (DeliveryConfirmOutcome::AlreadySettled, "already_settled"),
        (DeliveryConfirmOutcome::NotOffered, "not_offered"),
        (DeliveryConfirmOutcome::RefusedTerminal, "refused_terminal"),
        (DeliveryConfirmOutcome::StaleGeneration, "stale_generation"),
        (DeliveryConfirmOutcome::Retryable, "retryable"),
    ];
    let mut results = vec![];
    for (result, name) in outcomes {
        let item = DeliveryConfirmItem {
            id: MessageId::new("m1"),
            result,
            reason: None,
        };
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["result"], name);
        assert!(
            value.get("reason").is_none(),
            "absent reason not serialized"
        );
        round_trip(&item);
        results.push(item);
    }
    for (reason, name) in [
        (DeliveryConfirmReason::Unknown, "unknown"),
        (DeliveryConfirmReason::NotAddressed, "not_addressed"),
        (DeliveryConfirmReason::Truncated, "truncated"),
        (DeliveryConfirmReason::NoHookPull, "no_hook_pull"),
        (DeliveryConfirmReason::Busy, "busy"),
    ] {
        let item = DeliveryConfirmItem {
            id: MessageId::new("m1"),
            result: DeliveryConfirmOutcome::Retryable,
            reason: Some(reason),
        };
        assert_eq!(serde_json::to_value(&item).unwrap()["reason"], name);
        round_trip(&item);
        results.push(item);
    }
    round_trip(&DeliveryConfirmReport { results });
}

#[test]
fn unknown_fields_are_rejected() {
    let mut value = serde_json::to_value(pull()).unwrap();
    value["extra"] = json!(1);
    assert!(serde_json::from_value::<DeliveryPull>(value).is_err());
    let mut value = serde_json::to_value(confirm(1)).unwrap();
    value["extra"] = json!(1);
    assert!(serde_json::from_value::<DeliveryConfirm>(value).is_err());
}

#[test]
fn pull_validation_bounds_marker_prefix() {
    let with = |prefix: Vec<String>| DeliveryPull {
        marker_prefix: prefix,
        ..pull()
    };
    assert!(Command::DeliveryPull(pull()).validate().is_ok());
    let eight = vec!["a".to_string(); 8];
    assert!(Command::DeliveryPull(with(eight)).validate().is_ok());
    let nine = vec!["a".to_string(); 9];
    assert!(Command::DeliveryPull(with(nine)).validate().is_err());
    // Exactly 4096 bytes in total is valid; 4097 is not.
    let exact = vec!["x".repeat(2048), "y".repeat(2048)];
    assert!(Command::DeliveryPull(with(exact)).validate().is_ok());
    let over = vec!["x".repeat(2048), "y".repeat(2049)];
    assert!(Command::DeliveryPull(with(over)).validate().is_err());
    // Bytes, not chars: 2049 two-byte chars alone exceed the bound.
    let wide = vec!["é".repeat(2049)];
    assert!(Command::DeliveryPull(with(wide)).validate().is_err());
}

#[test]
fn confirm_validation_bounds_ids() {
    assert!(Command::DeliveryConfirm(confirm(0)).validate().is_err());
    assert!(Command::DeliveryConfirm(confirm(1)).validate().is_ok());
    assert!(
        Command::DeliveryConfirm(confirm(MAX_BATCH_ITEMS))
            .validate()
            .is_ok()
    );
    assert!(
        Command::DeliveryConfirm(confirm(MAX_BATCH_ITEMS + 1))
            .validate()
            .is_err()
    );
    let mut duplicate = confirm(3);
    duplicate.ids[2] = duplicate.ids[0].clone();
    assert!(Command::DeliveryConfirm(duplicate).validate().is_err());
}

#[test]
fn declared_subagent_claims_are_refused_by_the_claim_arm() {
    let mut subagent_pull = pull();
    subagent_pull.claim = claim(CallerRole::Subagent);
    assert!(Command::DeliveryPull(subagent_pull).validate().is_err());
    let mut subagent_confirm = confirm(1);
    subagent_confirm.claim = claim(CallerRole::Subagent);
    assert!(
        Command::DeliveryConfirm(subagent_confirm)
            .validate()
            .is_err()
    );
}

#[test]
fn commands_round_trip_through_the_envelope_and_confirm_is_a_permit_mutation() {
    let command = Command::DeliveryPull(DeliveryPull {
        seen_attention_version: Some(3),
        marker_prefix: vec!["ht".into()],
        ..pull()
    });
    let value = serde_json::to_value(&command).unwrap();
    assert_eq!(value["kind"], "delivery_pull");
    assert_eq!(value["args"]["claim"]["seat"], "s1");
    round_trip(&command);
    // A pull is a read: it is not an accountable mutation.
    assert!(PermitMutation::try_from(command).is_err());

    let command = Command::DeliveryConfirm(confirm(2));
    let value = serde_json::to_value(&command).unwrap();
    assert_eq!(value["kind"], "delivery_confirm");
    assert_eq!(value["args"]["claim"]["seat"], "s1");
    round_trip(&command);
    match PermitMutation::try_from(command).unwrap() {
        PermitMutation::DeliveryConfirm(v) => assert_eq!(v, confirm(2)),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn results_round_trip_through_the_result_envelope() {
    use crate::protocol::results::CommandResult;
    let pulled = CommandResult::DeliveryPulled(DeliveryPullReply::Unchanged {
        attention_version: 9,
    });
    assert_eq!(
        serde_json::to_value(&pulled).unwrap()["kind"],
        "delivery_pulled"
    );
    round_trip(&pulled);
    let confirmed = CommandResult::DeliveryConfirmed(DeliveryConfirmReport { results: vec![] });
    assert_eq!(
        serde_json::to_value(&confirmed).unwrap()["kind"],
        "delivery_confirmed"
    );
    round_trip(&confirmed);
}

#[test]
fn capability_is_defined_and_advertised() {
    assert_eq!(capabilities::DELIVERY_PULL, "delivery.pull_v1");
    assert!(capabilities::ADVERTISED.contains(&capabilities::DELIVERY_PULL));
}

// Missing/overpermissive V2 decoding would let a token acknowledge the wrong
// presentation scope; these exercise the actual public command decoder.
#[test]
fn presentation_v2_decodes_scoped_commands_without_changing_v1() {
    let caller = serde_json::to_value(claim(CallerRole::TopLevel)).unwrap();
    let request = json!({"kind":"delivery_pull_v2","args":{
        "render":{"kind":"hook","confirmable_selector":0,"passive_selector":0},"claim":caller,"consumer":"hook","marker_prefix":["ht"]
    }});
    let parsed = serde_json::from_value::<Command>(request.clone());
    assert!(parsed.is_ok(), "V2 public pull must decode: {parsed:?}");
    let parsed = parsed.unwrap();
    assert!(parsed.validate().is_ok());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), request);

    let confirmation = json!({"kind":"delivery_presentation_confirm_v2","args":{
        "claim":caller,"consumer":"hook","token":"0123456789abcdef0123456789abcdef"
    }});
    let parsed = serde_json::from_value::<Command>(confirmation.clone());
    assert!(parsed.is_ok(), "V2 public confirm must decode: {parsed:?}");
    let parsed = parsed.unwrap();
    assert!(parsed.validate().is_ok());
    assert_eq!(serde_json::to_value(&parsed).unwrap(), confirmation);
    // Presentation completion must not become a message ACK permit.
    assert!(PermitMutation::try_from(parsed).is_err());

    let historical = serde_json::to_string(&Command::DeliveryPull(pull())).unwrap();
    assert!(!historical.contains("consumer"));
    assert!(!historical.contains("token"));
    assert_eq!(
        serde_json::from_str::<Command>(&historical).unwrap(),
        Command::DeliveryPull(pull())
    );
}

#[test]
fn presentation_v2_rejects_invalid_tokens_consumers_and_watch_without_lease() {
    let request = json!({"kind":"delivery_pull_v2","args":{
        "render":{"kind":"hook","confirmable_selector":0,"passive_selector":0},"claim":claim(CallerRole::TopLevel),"consumer":"hook"
    }});
    let command = serde_json::from_value::<Command>(request.clone());
    assert!(
        command.is_ok(),
        "valid V2 request must decode before negative controls: {command:?}"
    );
    assert!(command.unwrap().validate().is_ok());
    let confirmation = json!({"kind":"delivery_presentation_confirm_v2","args":{
        "claim":claim(CallerRole::TopLevel),"consumer":"hook",
        "token":"0123456789abcdef0123456789abcdef"
    }});
    let command = serde_json::from_value::<Command>(confirmation.clone()).unwrap();
    assert!(command.validate().is_ok());
    for token in [
        "",
        "0",
        "0123456789ABCDEF0123456789abcdef",
        "g123456789abcdef0123456789abcdef",
        "0123456789abcdef0123456789abcdef0",
    ] {
        let mut bad = confirmation.clone();
        bad["args"]["token"] = json!(token);
        let command = serde_json::from_value::<Command>(bad)
            .expect("malformed Confirm token must decode before validation");
        assert!(
            command.validate().is_err(),
            "invalid token accepted: {token}"
        );
    }
    let mut watch_confirmation = confirmation;
    watch_confirmation["args"]["consumer"] = json!("watch");
    let command = serde_json::from_value::<Command>(watch_confirmation.clone()).unwrap();
    assert!(
        command.validate().is_err(),
        "Watch confirmation requires a lease"
    );
    watch_confirmation["args"]["presentation_lease"] = json!("g123456789abcdef0123456789abcdef");
    let command = serde_json::from_value::<Command>(watch_confirmation.clone()).unwrap();
    assert!(
        command.validate().is_err(),
        "Watch confirmation rejects a malformed lease"
    );
    watch_confirmation["args"]["presentation_lease"] = json!("fedcba9876543210fedcba9876543210");
    let command = serde_json::from_value::<Command>(watch_confirmation).unwrap();
    assert!(command.validate().is_ok());
    let mut watch = request.clone();
    watch["args"]["consumer"] = json!("watch");
    watch["args"]["render"] = json!({"kind":"watch_json_lines_v1"});
    let command = serde_json::from_value::<Command>(watch.clone()).unwrap();
    assert!(
        command.validate().is_err(),
        "claim alone cannot authorize Watch"
    );
    watch["args"]["presentation_lease"] = json!("fedcba9876543210fedcba9876543210");
    assert!(
        serde_json::from_value::<Command>(watch)
            .unwrap()
            .validate()
            .is_ok()
    );
    let mut extra = request.clone();
    extra["args"]["bogus"] = json!(1);
    assert!(serde_json::from_value::<Command>(extra).is_err());
    let mut invented = request;
    invented["args"]["consumer"] = json!("third");
    assert!(serde_json::from_value::<Command>(invented).is_err());
}

#[test]
fn presentation_confirmation_exact_control_shape_and_removed_piggyback() {
    let base = json!({"kind":"delivery_presentation_confirm_v2","args":{"claim":claim(CallerRole::TopLevel),"consumer":"hook","token":"0123456789abcdef0123456789abcdef","emitted_controls":[{"kind":"attention","epoch":"18446744073709551615","version":9}]}});
    let command: Command = serde_json::from_value(base.clone()).unwrap();
    assert!(command.validate().is_ok());
    assert_eq!(serde_json::to_value(command).unwrap(), base);
    for controls in [
        json!([{"kind":"attention","epoch":"01","version":9}]),
        json!([{"kind":"attention","epoch":"1","version":9},{"kind":"attention","epoch":"1","version":9}]),
        json!(vec![
            json!({"kind":"attention","epoch":"1","version":9});
            33
        ]),
    ] {
        let mut invalid = base.clone();
        invalid["args"]["emitted_controls"] = controls;
        assert!(!serde_json::from_value::<Command>(invalid).is_ok_and(|v| v.validate().is_ok()));
    }
    let removed = json!({"kind":"delivery_pull_v2","args":{"claim":claim(CallerRole::TopLevel),"consumer":"hook","render":{"kind":"hook","confirmable_selector":0,"passive_selector":0},"confirmed_token":"0123456789abcdef0123456789abcdef"}});
    assert!(serde_json::from_value::<Command>(removed).is_err());
}

#[test]
fn presentation_confirmation_maximum_valid_controls_fit_actual_framed_wrapper_reserve() {
    use crate::protocol::{
        delivery::{
            DeliveryPresentationConfirmationReplyV2, DeliveryPresentationConfirmationV2,
            PresentationControlStampV2,
        },
        results::CommandResult,
        wire::{MAX_WIRE_FRAME_BYTES, PROTOCOL_VERSION, WireResponse, encode_wire_response},
    };
    let acknowledged_controls = (0..32)
        .map(|n| PresentationControlStampV2::Attention {
            epoch: (u64::MAX - n).to_string(),
            version: u64::MAX,
        })
        .collect();
    let result =
        CommandResult::DeliveryPresentationConfirmedV2(DeliveryPresentationConfirmationReplyV2 {
            status: DeliveryPresentationConfirmationV2::Confirmed,
            acknowledged_controls,
        });
    let result_bytes = serde_json::to_vec(&result).unwrap().len();
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: "\"".repeat(128),
        instance: crate::node::NodeUuid::from_u128(1).to_string(),
        daemon_boot: uuid::Uuid::from_u128(2).to_string(),
        result: Ok(result),
    };
    let framed = encode_wire_response(&response).unwrap();
    assert_eq!(
        u32::from_be_bytes(framed[..4].try_into().unwrap()) as usize,
        framed.len() - 4
    );
    assert!(framed.len() - result_bytes <= 512 && framed.len() <= MAX_WIRE_FRAME_BYTES);
    assert!(serde_json::from_slice::<WireResponse>(&framed[4..]).is_ok());
}
