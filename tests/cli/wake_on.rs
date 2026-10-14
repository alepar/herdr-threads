use crate::cli::commands::{CliAction, ParseFailure, parse_argv, parse_argv_or_informational};
use crate::protocol::{
    capabilities::{ADVERTISED, SEAT_WAKE_ON_V1},
    commands::{Command, SetWakeOn, WakeOn, WakeOnQuery},
    ids::*,
    results::{CommandResult, WakeOnState},
};

// ---- parsing ----

#[test]
fn wake_on_parses_a_value_or_none() {
    assert_eq!(
        parse_argv(["herdr-threads", "wake-on"]).unwrap().action,
        CliAction::WakeOn(None)
    );
    assert_eq!(
        parse_argv(["herdr-threads", "wake-on", "all"])
            .unwrap()
            .action,
        CliAction::WakeOn(Some(WakeOn::All))
    );
    assert_eq!(
        parse_argv(["herdr-threads", "wake-on", "nudge"])
            .unwrap()
            .action,
        CliAction::WakeOn(Some(WakeOn::Nudge))
    );
}

#[test]
fn an_invalid_value_is_a_usage_error_exit_2() {
    for argv in [
        vec!["herdr-threads", "wake-on", "bogus"],
        vec!["herdr-threads", "wake-on", "ALL"],
        vec!["herdr-threads", "wake-on", "all", "nudge"],
    ] {
        let error = match parse_argv_or_informational(argv.clone()) {
            Err(ParseFailure::Invalid(error)) => error,
            other => panic!("{argv:?} should be invalid, got {other:?}"),
        };
        assert_eq!(
            crate::cli::exit::api_exit_code(&error.code),
            crate::cli::exit::EXIT_USAGE,
            "{argv:?}"
        );
    }
}

// ---- wire ----

#[test]
fn wire_shapes_round_trip_and_wake_on_is_snake_case() {
    assert_eq!(serde_json::to_value(WakeOn::Nudge).unwrap(), "nudge");
    assert_eq!(serde_json::to_value(WakeOn::All).unwrap(), "all");
    assert_eq!(WakeOn::default(), WakeOn::Nudge);
    let claim = crate::protocol::authority::CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: crate::protocol::authority::CallerRole::TopLevel,
        harness: crate::protocol::authority::Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("e"),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    };
    for command in [
        Command::SetWakeOn(SetWakeOn {
            value: WakeOn::All,
            operation: OperationId::new("op"),
            claim: claim.clone(),
        }),
        Command::WakeOn(WakeOnQuery {
            seat: SeatId::new("s"),
        }),
    ] {
        assert!(command.validate().is_ok());
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    }
    let json = serde_json::to_value(Command::SetWakeOn(SetWakeOn {
        value: WakeOn::All,
        operation: OperationId::new("op"),
        claim: claim.clone(),
    }))
    .unwrap();
    assert_eq!(json["kind"], "set_wake_on");
    assert_eq!(json["args"]["value"], "all");
    let state = CommandResult::WakeOn(WakeOnState {
        seat: SeatId::new("s"),
        wake_on: WakeOn::All,
        changed: true,
    });
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["kind"], "wake_on");
    assert_eq!(
        serde_json::from_value::<CommandResult>(json).unwrap(),
        state
    );
    // A declared subagent's claim is refused at validation.
    let mut subagent = claim;
    subagent.role = crate::protocol::authority::CallerRole::Subagent;
    assert!(
        Command::SetWakeOn(SetWakeOn {
            value: WakeOn::All,
            operation: OperationId::new("op"),
            claim: subagent,
        })
        .validate()
        .is_err()
    );
    // Unknown fields and values are rejected rather than ignored.
    assert!(serde_json::from_value::<WakeOn>(serde_json::json!("bogus")).is_err());
    assert!(
        serde_json::from_value::<WakeOnQuery>(serde_json::json!({"seat":"s","extra":1})).is_err()
    );
}

#[test]
fn the_capability_is_advertised() {
    assert_eq!(SEAT_WAKE_ON_V1, "seat.wake_on_v1");
    assert!(ADVERTISED.contains(&SEAT_WAKE_ON_V1));
}
