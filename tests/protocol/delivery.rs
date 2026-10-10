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
fn capability_is_defined_but_not_advertised() {
    assert_eq!(capabilities::DELIVERY_PULL, "delivery.pull_v1");
    assert!(!capabilities::ADVERTISED.contains(&capabilities::DELIVERY_PULL));
}

/// The inert seam: the domain service decodes both commands but serves neither.
#[test]
fn domain_service_answers_unsupported_for_both_commands() {
    use crate::{
        ports::{LocalService, StorePort},
        protocol::{
            authority::PeerIdentity,
            results::ErrorCode,
            time::{CallBudget, Clock, MonoInstant, UtcMillis},
        },
        service::{dispatch::DomainService, fair_writer::FairWriter},
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    use std::sync::Arc;
    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    use std::os::unix::fs::DirBuilderExt;
    let directory = std::env::temp_dir().join(format!("delivery-inert-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let context = StoreContext::new(directory.join("store.db"), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3)",
        [],
    )
    .unwrap();
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let service = DomainService::new("i".into(), store, clock)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    for command in [
        Command::DeliveryPull(pull()),
        Command::DeliveryConfirm(confirm(1)),
    ] {
        let error = service
            .handle(command, PeerIdentity::from_kernel(501), &budget)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unsupported);
        assert_eq!(error.detail, "delivery pull is not served");
    }
    drop(db);
    std::fs::remove_dir_all(directory).unwrap();
}
