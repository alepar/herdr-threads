//! Seam contract of ht-kqrt.53: `CreateTargetPort`, the V3 bootstrap journal records and the
//! launch-gate command. V1 and V2 bytes are pinned by `tests/fixtures/handoff-journal/`.

use herdr_threads::{
    app::SystemClock,
    cli::{journal::BootstrapPlan, topology_handoff::BootstrapProgress},
    host::native::NativeCli,
    ports::{
        CreateTargetOutcome, CreateTargetPort, CreateTargetRequest, CreatedTarget, HcPeer,
        HostCallContext, HostWitness, Placement,
    },
    protocol::{
        capabilities::{ADVERTISED, LAUNCH_GATE_V1},
        commands::Command,
        handoff::{
            AuthorizeLaunchGate, BootstrapAttachment, BootstrapAttachmentV3, BootstrapPlanV3,
            BootstrapProgressV3, LAUNCH_GATE_MAX_TIMEOUT_MS, LaunchGateRefusal, LaunchGateReply,
        },
        ids::{HostBootId, HostCallId, HostTargetId, TerminalId},
        results::CommandResult,
        time::{CallBudget, Cancellation, Clock},
    },
    test_support::{fake_herdr::FakeHerdr, scratch::temp_path},
};
use std::sync::Arc;

const V1_PLAN: &str = include_str!("../fixtures/handoff-journal/v1-plan.json");
const V2_PLAN: &str = include_str!("../fixtures/handoff-journal/v2-plan.json");
const V1_PROGRESS: &str = include_str!("../fixtures/handoff-journal/v1-progress.json");
const V2_PROGRESS: &str = include_str!("../fixtures/handoff-journal/v2-progress.json");
const V1_ATTACHMENT: &str = include_str!("../fixtures/handoff-journal/v1-attachment.json");
const V2_ATTACHMENT: &str = include_str!("../fixtures/handoff-journal/v2-attachment.json");

fn round_trip_bytes<T: serde::Serialize + serde::de::DeserializeOwned>(fixture: &str) {
    let decoded: T = serde_json::from_str(fixture).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), fixture);
}

/// Kills: a V3 change that reorders, renames or adds a field of a V1/V2 record.
#[test]
fn v1_v2_fixtures_decode_byte_identical() {
    for fixture in [V1_PLAN, V2_PLAN] {
        round_trip_bytes::<BootstrapPlan>(fixture);
    }
    for fixture in [V1_PROGRESS, V2_PROGRESS] {
        round_trip_bytes::<BootstrapProgress>(fixture);
    }
    for fixture in [V1_ATTACHMENT, V2_ATTACHMENT] {
        round_trip_bytes::<BootstrapAttachment>(fixture);
    }
    let v2: BootstrapPlan = serde_json::from_str(V2_PLAN).unwrap();
    assert_eq!(v2.version, 2);
}

fn herdr_target(attachment: &BootstrapAttachment) -> CreatedTarget {
    let tab = &attachment.created;
    CreatedTarget {
        correlation: tab.correlation.clone(),
        placement: Placement::NewTab {
            space: tab.workspace.clone(),
        },
        target: tab.tab.clone(),
        root_pane: Some(tab.root_pane.clone()),
        terminal: tab.terminal.clone(),
        host_incarnation: tab.host_incarnation.clone(),
        witness: HostWitness::HerdrSocket(tab.witness.clone()),
    }
}

fn hc_target() -> CreatedTarget {
    CreatedTarget {
        correlation: HostCallId::new("c1"),
        placement: Placement::NewTerminal {
            parent: HostTargetId::new("t1"),
        },
        target: HostTargetId::new("t2"),
        root_pane: None,
        terminal: TerminalId::new("t2"),
        host_incarnation: HostBootId::new("hc-core:core-a:pid=42:start=7:uid=501"),
        witness: HostWitness::HcCore {
            core_id: "core-a".into(),
            peer: HcPeer {
                pid: 42,
                start_seconds: 7,
                uid: 501,
            },
        },
    }
}

/// Kills: a V3 record that drops the `CreatedTarget` or its witness on the way through serde.
#[test]
fn v3_round_trips() {
    let attachment: BootstrapAttachment = serde_json::from_str(V2_ATTACHMENT).unwrap();
    let progress: BootstrapProgress = serde_json::from_str(V2_PROGRESS).unwrap();
    let plan: BootstrapPlan = serde_json::from_str(V2_PLAN).unwrap();
    for created in [herdr_target(&attachment), hc_target()] {
        let v3 = BootstrapAttachmentV3 {
            attempt: attachment.attempt,
            created: created.clone(),
            resolve_operation: attachment.resolve_operation.clone(),
            resolved_seat: attachment.resolved_seat.clone(),
            handoff: attachment.handoff.clone(),
        };
        let back: BootstrapAttachmentV3 =
            serde_json::from_str(&serde_json::to_string(&v3).unwrap()).unwrap();
        assert_eq!(back, v3);
        let progress = BootstrapProgressV3 {
            version: 3,
            identity: progress.identity.clone(),
            attempt: progress.attempt,
            possible_creation: true,
            request: None,
            creation: Some(created.clone()),
            not_submitted: false,
        };
        let back: BootstrapProgressV3 =
            serde_json::from_str(&serde_json::to_string(&progress).unwrap()).unwrap();
        assert_eq!(back, progress);
        assert_eq!(back.creation, Some(created.clone()));
        let plan = BootstrapPlanV3 {
            version: 3,
            payload: plan.payload.clone(),
            placement: created.placement.clone(),
        };
        let back: BootstrapPlanV3 =
            serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(back, plan);
    }
    // A V3 record never decodes as a frozen record: the version-gated readers refuse it.
    let v3_plan = serde_json::to_string(&BootstrapPlanV3 {
        version: 3,
        payload: plan.payload.clone(),
        placement: Placement::NewTab {
            space: plan.payload.workspace.clone(),
        },
    })
    .unwrap();
    assert!(serde_json::from_str::<BootstrapPlan>(&v3_plan).is_err());
}

/// Kills: `validate` accepting a witness whose kind does not match the placement or whose
/// identity does not match the incarnation.
#[test]
fn created_target_validate_per_witness_kind() {
    let attachment: BootstrapAttachment = serde_json::from_str(V1_ATTACHMENT).unwrap();
    let herdr = herdr_target(&attachment);
    assert!(herdr.validate().is_ok());
    let mut other_space = herdr.clone();
    other_space.target = HostTargetId::new("w9:t2");
    assert!(other_space.validate().is_err());
    let mut wrong_incarnation = herdr.clone();
    wrong_incarnation.host_incarnation = HostBootId::new("herdr-server:pid=1:start=1.000001:uid=1");
    assert!(wrong_incarnation.validate().is_err());
    let mut no_root = herdr.clone();
    no_root.root_pane = None;
    assert!(no_root.validate().is_err());
    let mut terminal_placement = herdr.clone();
    terminal_placement.placement = Placement::NewTerminal {
        parent: HostTargetId::new("w1"),
    };
    assert!(terminal_placement.validate().is_err());

    let hc = hc_target();
    assert!(hc.validate().is_ok());
    let mut wrong_peer = hc.clone();
    wrong_peer.witness = HostWitness::HcCore {
        core_id: "core-a".into(),
        peer: HcPeer {
            pid: 43,
            start_seconds: 7,
            uid: 501,
        },
    };
    assert!(wrong_peer.validate().is_err());
    let mut tab_placement = hc.clone();
    tab_placement.placement = Placement::NewTab {
        space: HostTargetId::new("t1"),
    };
    assert!(tab_placement.validate().is_ok());
    tab_placement.target = HostTargetId::new("t1");
    assert!(tab_placement.validate().is_err());
    let mut same_as_parent = hc.clone();
    same_as_parent.target = HostTargetId::new("t1");
    assert!(same_as_parent.validate().is_err());
    let mut with_root = hc.clone();
    with_root.root_pane = Some(HostTargetId::new("p"));
    assert!(with_root.validate().is_err());
    // A Herdr witness never validates a core terminal's incarnation, and the reverse.
    let mut crossed = hc;
    crossed.witness = herdr.witness.clone();
    assert!(crossed.validate().is_err());
}

fn budget_context(clock: &dyn Clock) -> HostCallContext {
    HostCallContext {
        budget: CallBudget {
            deadline: herdr_threads::protocol::time::MonoInstant(clock.monotonic_now().0 + 60_000),
            cancellation: Cancellation::default(),
        },
        expected_boot: None,
        expected_epoch: None,
    }
}

/// Kills: a delegate that skips `create_tab` (no `tab.create` reaches Herdr), creates for a
/// placement it cannot serve, or returns evidence that does not validate.
#[test]
fn native_cli_create_target_delegates_to_create_tab() {
    let socket = temp_path(format!("ht-3-{}.sock", uuid::Uuid::new_v4().simple()));
    let herdr = FakeHerdr::start("v3", &socket, &[]);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap();
    let witness = runtime.block_on(async {
        let stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
        herdr_threads::host::continuity::capture_peer_witness(&stream, &socket).unwrap()
    });
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let native = NativeCli::new(socket.clone(), clock.clone());
    let request = CreateTargetRequest {
        correlation: HostCallId::new("corr-1"),
        placement: Placement::NewTab {
            space: HostTargetId::new("w1"),
        },
        cwd: "/tmp".into(),
        label: "launch".into(),
        focus: false,
        env: Default::default(),
        directory_context: None,
        expected_witness: HostWitness::HerdrSocket(witness),
    };
    let context = budget_context(clock.as_ref());
    let CreateTargetOutcome::Created(created) = native.create_target(&request, &context) else {
        panic!("native tab creation must succeed");
    };
    assert_eq!(herdr.tabs_created().len(), 1);
    assert_eq!(herdr.tabs_created()[0]["workspace_id"], "w1");
    assert_eq!(created.placement, request.placement);
    assert!(created.target.as_str().starts_with("w1:t"));
    assert!(created.validate().is_ok());

    // A core terminal placement is refused before anything reaches Herdr.
    let mut terminal = request.clone();
    terminal.placement = Placement::NewTerminal {
        parent: HostTargetId::new("w1"),
    };
    assert!(matches!(
        native.create_target(&terminal, &context),
        CreateTargetOutcome::NotSubmitted(_)
    ));
    assert_eq!(herdr.tabs_created().len(), 1);
}

fn gate() -> AuthorizeLaunchGate {
    AuthorizeLaunchGate {
        compound: herdr_threads::protocol::ids::OperationId::new("c-1"),
        attempt: herdr_threads::protocol::handoff::BootstrapAttempt::first(),
        core_id: "core-a".into(),
        terminal: "t7".into(),
        timeout_ms: 5_000,
    }
}

/// Kills: a command or reply that fails its serde round trip, a new field that is required on
/// the wire, an unbounded timeout, and advertising the capability before its handler exists.
#[test]
fn launch_gate_command_reply_round_trip() {
    let command = Command::AuthorizeLaunchGate(Box::new(gate()));
    let back: Command = serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
    assert_eq!(back, command);
    for reply in [
        LaunchGateReply::Authorized,
        LaunchGateReply::Refused {
            reason: LaunchGateRefusal::WitnessMismatch,
        },
    ] {
        let result = CommandResult::LaunchGate(reply);
        let back: CommandResult =
            serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
        assert_eq!(back, result);
    }
    // New fields default: an older sender's request without a timeout, a reply without a reason.
    let mut value = serde_json::to_value(gate()).unwrap();
    value.as_object_mut().unwrap().remove("timeout_ms");
    let decoded: AuthorizeLaunchGate = serde_json::from_value(value).unwrap();
    assert!(decoded.validate().is_ok());
    assert_eq!(decoded.timeout_ms, 10_000);
    let refused: LaunchGateReply = serde_json::from_str(r#"{"kind":"refused"}"#).unwrap();
    assert_eq!(
        refused,
        LaunchGateReply::Refused {
            reason: LaunchGateRefusal::UnknownBootstrap
        }
    );
    for timeout_ms in [0, LAUNCH_GATE_MAX_TIMEOUT_MS + 1] {
        let mut bad = gate();
        bad.timeout_ms = timeout_ms;
        assert!(bad.validate().is_err());
    }
    assert!(!ADVERTISED.contains(&LAUNCH_GATE_V1));
}
