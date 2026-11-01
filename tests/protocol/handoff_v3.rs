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
            AUTOMATIC_PLAN_VERSION, AUTOMATIC_TARGET_PLAN_VERSION, AuthorizeLaunchGate,
            AutomaticBootstrapAction, AutomaticBootstrapAttachment, AutomaticBootstrapIdentity,
            AutomaticBootstrapMutation, AutomaticBootstrapPlan,
            AutomaticBootstrapRecoveryDisposition, AutomaticCreation, AutomaticDeliveryMutation,
            AutomaticOriginal, BootstrapAttachment, BootstrapAttachmentV3, BootstrapAttempt,
            BootstrapPlanV3, BootstrapProgressV3, BootstrapQuiescenceAssertion,
            BootstrapRecoveryInspection, LAUNCH_GATE_MAX_TIMEOUT_MS, LaunchGateRefusal,
            LaunchGateReply,
        },
        ids::{HostBootId, HostCallId, HostTargetId, SeatId, TerminalId},
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

const AUTOMATIC_PLAN: &str = include_str!("../fixtures/handoff-journal/automatic-v2-plan.json");
const AUTOMATIC_CREATION: &str =
    include_str!("../fixtures/handoff-journal/automatic-v2-creation.json");
const CORE_SOCKET: &str = "/core/work.sock";

/// The automatic V2 plan the fixture pins, from the V2 progress fixture's caller.
fn v2_identity() -> AutomaticBootstrapIdentity {
    let progress: BootstrapProgress = serde_json::from_str(V2_PROGRESS).unwrap();
    let plan: AutomaticBootstrapPlan = serde_json::from_str(AUTOMATIC_PLAN).unwrap();
    AutomaticBootstrapIdentity::new(progress.identity.scope, progress.identity.claim, plan).unwrap()
}

fn v3_plan(placement: Placement, endpoint: &std::path::Path) -> AutomaticBootstrapPlan {
    let mut plan: AutomaticBootstrapPlan = serde_json::from_str(AUTOMATIC_PLAN).unwrap();
    plan.version = AUTOMATIC_TARGET_PLAN_VERSION;
    plan.payload.startup_input = None;
    plan.payload.workspace = placement.anchor().clone();
    plan.payload.handoff.namespace.host_endpoint = endpoint.to_path_buf();
    plan.placement = Some(placement);
    plan
}

fn v3_identity(placement: Placement, endpoint: &std::path::Path) -> AutomaticBootstrapIdentity {
    let v2 = v2_identity();
    AutomaticBootstrapIdentity::new(v2.scope, v2.claim, v3_plan(placement, endpoint)).unwrap()
}

fn hc_identity() -> AutomaticBootstrapIdentity {
    v3_identity(hc_target().placement, std::path::Path::new(CORE_SOCKET))
}

fn herdr_identity(created: &CreatedTarget) -> AutomaticBootstrapIdentity {
    let HostWitness::HerdrSocket(witness) = &created.witness else {
        panic!("a Herdr target")
    };
    v3_identity(created.placement.clone(), &witness.endpoint)
}

/// Kills: a V3 record that drops the `CreatedTarget` or its witness on the way through serde.
#[test]
fn v3_round_trips() {
    let attachment: BootstrapAttachment = serde_json::from_str(V2_ATTACHMENT).unwrap();
    for (created, id) in [
        (herdr_target(&attachment), None),
        (hc_target(), Some(hc_identity())),
    ] {
        let id = id.unwrap_or_else(|| herdr_identity(&created));
        let derived = AutomaticBootstrapAttachment::derive(
            &id,
            BootstrapAttempt::first(),
            created.clone(),
            SeatId::new("recipient"),
        )
        .unwrap();
        let v3 = BootstrapAttachmentV3 {
            attempt: derived.attempt,
            created: created.clone(),
            resolve_operation: derived.resolve_operation.clone(),
            resolved_seat: derived.resolved_seat.clone(),
            child: derived.child.clone(),
        };
        let back: BootstrapAttachmentV3 =
            serde_json::from_str(&serde_json::to_string(&v3).unwrap()).unwrap();
        assert_eq!(back, v3);
        let progress = BootstrapProgressV3 {
            version: 3,
            identity: id.clone(),
            attempt: BootstrapAttempt::first(),
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
            payload: id.plan.payload.clone(),
            placement: created.placement.clone(),
        };
        let back: BootstrapPlanV3 =
            serde_json::from_str(&serde_json::to_string(&plan).unwrap()).unwrap();
        assert_eq!(back, plan);
    }
    // A V3 record never decodes as a frozen record: the version-gated readers refuse it.
    let v3_plan = serde_json::to_string(&BootstrapPlanV3 {
        version: 3,
        payload: hc_identity().plan.payload,
        placement: hc_target().placement,
    })
    .unwrap();
    assert!(serde_json::from_str::<BootstrapPlan>(&v3_plan).is_err());
}

/// Kills: a typed V3 struct that drifts from the record the store holds, in field order or
/// in what it omits.
#[test]
fn v3_structs_are_the_automatic_record_bytes() {
    let id = hc_identity();
    let plan = BootstrapPlanV3 {
        version: 3,
        payload: id.plan.payload.clone(),
        placement: hc_target().placement,
    };
    let bytes = serde_json::to_string(&plan).unwrap();
    assert_eq!(bytes, serde_json::to_string(&id.plan).unwrap());
    let automatic: AutomaticBootstrapPlan = serde_json::from_str(&bytes).unwrap();
    assert_eq!(serde_json::to_string(&automatic).unwrap(), bytes);
    assert_eq!(AutomaticBootstrapPlan::from(plan), automatic);

    let derived = AutomaticBootstrapAttachment::derive(
        &id,
        BootstrapAttempt::first(),
        hc_target(),
        SeatId::new("recipient"),
    )
    .unwrap();
    let bytes = serde_json::to_string(&derived).unwrap();
    let typed: BootstrapAttachmentV3 = serde_json::from_str(&bytes).unwrap();
    assert_eq!(serde_json::to_string(&typed).unwrap(), bytes);
    assert_eq!(derived.created, AutomaticCreation::Target(hc_target()));
}

/// Kills: an untagged creation that re-spells a V2 tab, takes a tab with a stray key as a
/// tab or a target, or reads a core terminal as a tab.
#[test]
fn automatic_creation_keeps_v2_bytes_and_decodes_v3() {
    let tab: AutomaticCreation = serde_json::from_str(AUTOMATIC_CREATION).unwrap();
    assert!(matches!(tab, AutomaticCreation::Tab(_)));
    assert_eq!(serde_json::to_string(&tab).unwrap(), AUTOMATIC_CREATION);

    let hc = serde_json::to_string(&hc_target()).unwrap();
    let target: AutomaticCreation = serde_json::from_str(&hc).unwrap();
    assert_eq!(target, AutomaticCreation::Target(hc_target()));
    assert_eq!(serde_json::to_string(&target).unwrap(), hc);

    let mut stray: serde_json::Value = serde_json::from_str(AUTOMATIC_CREATION).unwrap();
    stray["extra"] = serde_json::json!(1);
    assert!(serde_json::from_value::<AutomaticCreation>(stray).is_err());
}

/// Kills: a plan whose version and placement disagree, or a V3 plan that keeps a startup
/// template or a workspace other than its anchor.
#[test]
fn v3_plan_pairs_version_and_placement() {
    let ok = v3_plan(hc_target().placement, std::path::Path::new(CORE_SOCKET));
    assert!(ok.validate().is_ok());
    let mut v2_with_placement = ok.clone();
    v2_with_placement.version = 2;
    assert!(v2_with_placement.validate().is_err());
    let mut v3_without = ok.clone();
    v3_without.placement = None;
    assert!(v3_without.validate().is_err());
    let mut with_template = ok.clone();
    with_template.payload.startup_input =
        Some(herdr_threads::harness::adapter::StartupInputTemplate::positional(0));
    assert!(with_template.payload.startup_input.is_some());
    assert!(with_template.validate().is_err());
    let mut other_workspace = ok;
    other_workspace.payload.workspace = HostTargetId::new("elsewhere");
    assert!(other_workspace.validate().is_err());
}

/// Kills: a V3 plan that insists the caller's host is the target core, a V2 plan that
/// stops insisting, and a child that does not carry the V3 marker.
#[test]
fn v3_claim_host_may_differ_from_the_namespace() {
    let v2 = v2_identity();
    let id = hc_identity();
    assert_ne!(id.claim.host.as_path(), std::path::Path::new(CORE_SOCKET));
    assert!(id.validate().is_ok());
    // The same values on a V2 plan are refused.
    let mut v2_plan = v2.plan.clone();
    v2_plan.payload.handoff.namespace.host_endpoint = CORE_SOCKET.into();
    assert!(AutomaticBootstrapIdentity::new(v2.scope.clone(), v2.claim.clone(), v2_plan).is_err());

    let child = id.child(hc_target(), SeatId::new("recipient")).unwrap();
    assert!(child.targets_own_host());
    child.validate().unwrap();
    let AutomaticOriginal::Handoff(plan) = &child.original else {
        panic!("a handoff original")
    };
    assert_eq!(plan.version, AUTOMATIC_TARGET_PLAN_VERSION);
    assert_eq!(plan.launch.target, hc_target().target);
    // The same child spelled as a V2 one is refused: only a V3 child may leave its host.
    let mut as_v2 = plan.as_ref().clone();
    as_v2.version = AUTOMATIC_PLAN_VERSION;
    assert!(
        AutomaticDeliveryMutation::new(
            child.scope.clone(),
            child.claim.clone(),
            AutomaticOriginal::Handoff(Box::new(as_v2)),
        )
        .is_err()
    );
    // A creation on another placement is not this plan's.
    let mut elsewhere = hc_target();
    elsewhere.placement = Placement::NewTerminal {
        parent: HostTargetId::new("t9"),
    };
    assert!(id.child(elsewhere, SeatId::new("recipient")).is_err());
    // A V3 creation on a V2 plan, and a tab on a V3 plan, are refused.
    assert!(v2.child(hc_target(), SeatId::new("recipient")).is_err());
    let attachment: BootstrapAttachment = serde_json::from_str(V2_ATTACHMENT).unwrap();
    assert!(
        id.child(attachment.created.clone(), SeatId::new("recipient"))
            .is_err()
    );
}

/// Kills: a V3 mutation an older daemon would decode and half-record: the old decoder denies
/// the unknown `placement`, and the V3 child's version is not the one it accepts.
#[test]
fn v3_mutation_is_refused_by_the_pre_v3_decoder() {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct OldPlan {
        version: u32,
        payload: serde::de::IgnoredAny,
    }
    let v2 = serde_json::to_string(&v2_identity().plan).unwrap();
    assert!(serde_json::from_str::<OldPlan>(&v2).is_ok());
    let v3 = serde_json::to_string(&hc_identity().plan).unwrap();
    assert!(serde_json::from_str::<OldPlan>(&v3).is_err());
    assert_ne!(AUTOMATIC_TARGET_PLAN_VERSION, AUTOMATIC_PLAN_VERSION);
    let child = hc_identity()
        .child(hc_target(), SeatId::new("recipient"))
        .unwrap();
    let AutomaticOriginal::Handoff(plan) = &child.original else {
        panic!("a handoff original")
    };
    assert_ne!(plan.version, AUTOMATIC_PLAN_VERSION);
}

/// Kills: a V3 plan that accepts a created-pane recovery (it names a Herdr pane), and a
/// cancellation or not-created disposition that stops validating.
#[test]
fn v3_created_pane_recovery_is_refused_and_cancelled_validates() {
    let attachment: BootstrapAttachment = serde_json::from_str(V2_ATTACHMENT).unwrap();
    let recover = |id: AutomaticBootstrapIdentity, disposition| AutomaticBootstrapMutation {
        identity: id,
        action: AutomaticBootstrapAction::Recover {
            attempt: BootstrapAttempt::first(),
            inspection: BootstrapRecoveryInspection {
                version: 1,
                digest: "f".repeat(64),
            },
            disposition,
        },
    };
    let created_pane = || AutomaticBootstrapRecoveryDisposition::CreatedPane {
        evidence: Box::new(attachment.created.clone()),
        structural_reference: HostCallId::new("ref"),
    };
    let cancelled = || AutomaticBootstrapRecoveryDisposition::Cancelled {
        reason: "inspected quiescence".into(),
        quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
        attached_child: None,
    };
    let not_created = || AutomaticBootstrapRecoveryDisposition::NotCreated {
        quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
    };
    assert!(recover(v2_identity(), created_pane()).validate().is_ok());
    assert!(recover(hc_identity(), created_pane()).validate().is_err());
    assert!(recover(hc_identity(), cancelled()).validate().is_ok());
    assert!(recover(hc_identity(), not_created()).validate().is_ok());
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
        gate_argv: Vec::new(),
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
        socket: None,
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
    // The daemon serves it since ht-kqrt.57 (`service::launch_gate`).
    assert!(ADVERTISED.contains(&LAUNCH_GATE_V1));
}

/// Kills: a `gate_argv` that is always serialized (every released request record would change
/// bytes) or that does not round-trip when set.
#[test]
fn create_target_request_without_gate_argv_keeps_its_bytes() {
    let mut request = CreateTargetRequest {
        correlation: HostCallId::new("c"),
        placement: Placement::NewTerminal {
            parent: HostTargetId::new("t1"),
        },
        cwd: "/w".into(),
        label: "l".into(),
        focus: false,
        env: Default::default(),
        directory_context: None,
        expected_witness: HostWitness::HcCore {
            core_id: "core".into(),
            peer: HcPeer {
                pid: 1,
                start_seconds: 2,
                uid: 3,
            },
        },
        gate_argv: Vec::new(),
    };
    let bytes = serde_json::to_string(&request).unwrap();
    assert!(!bytes.contains("gate_argv"), "{bytes}");
    assert_eq!(
        serde_json::from_str::<CreateTargetRequest>(&bytes).unwrap(),
        request
    );
    request.gate_argv = vec!["shim".into()];
    let with = serde_json::to_string(&request).unwrap();
    assert!(with.contains(r#""gate_argv":["shim"]"#), "{with}");
    assert_eq!(
        serde_json::from_str::<CreateTargetRequest>(&with).unwrap(),
        request
    );
}
