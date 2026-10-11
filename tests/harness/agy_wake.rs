//! agy declares the content-free wake prompt (ht-poh.14, R18); the woken turn's first
//! top-level `PreInvocation` pulls and injects the pending bodies. Included from
//! `src/harness/agy/delivery.rs`; the daemon rig is copied from `agy_delivery.rs`.
use super::*;
use crate::{
    harness::{
        adapter::{DecodedEvent, HookInput},
        agy::{AgyAdmission, decode::decode, witness::tests::Temp},
        registry::builtins,
    },
    notification::policy::WAKE_UP,
    ports::{LocalService, StorePort},
    protocol::{
        authority::{CallerRole, Harness, PeerIdentity},
        commands::{CheckIn, CheckInMode},
        ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId},
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::{sync::Arc, time::Duration};

const CONV: &str = "5299b3e7-20e3-4970-aa8d-477d46a420c6";

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn peer() -> PeerIdentity {
    PeerIdentity::from_kernel(501)
}
fn claim() -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Agent(builtins().agent("agy").unwrap()),
        native_session: NativeSessionId::new(CONV),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}

struct Direct<'a>(&'a DomainService);
impl HookStepDaemon for Direct<'_> {
    fn call(&self, command: Command) -> Result<CommandResult, ApiError> {
        self.0
            .handle(command, peer(), &budget())
            .map_err(|e| ApiError::new(ErrorCode::InvalidRequest, format!("{e:?}")))
    }
}

/// Seat `s` (agy) joined to thread `t` with ordinary message `m1` (body `hello agy`) pending.
fn service(temp: &Temp) -> DomainService {
    let clock: Arc<dyn Clock> = Arc::new(TestClock);
    let context = StoreContext::new(temp.0.join("store.db"), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,0,0)", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-p','inc','coherent_enumeration',1)", []).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2)", []).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1)", []).unwrap();
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','i','t',1,'ordinary','hello agy',0,3)", []).unwrap();
    db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m1','t','s','pending',300000)", []).unwrap();
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    DomainService::new("i".into(), store, clock)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
}

/// Kills: an agy policy left on the default marker (the wake would then instruct `inbox`).
#[test]
fn agy_declares_the_content_free_wake_text() {
    assert_eq!(AgyDelivery.wake_text(), WAKE_UP);
    assert_eq!(WAKE_UP, "herdr-threads: wake up");
}

/// Kills: a wake whose turn gets only the content-free text (no bodies), or an injected page
/// that echoes the wake text. The payload's `prompt` is the wake text the host typed (agy's
/// hook ignores it: the pull is keyed on the invocation, not on the prompt).
#[test]
fn the_woken_turn_receives_the_bodies_by_injection() {
    let temp = Temp::new();
    let service = service(&temp);
    let context = match service
        .handle(
            Command::CheckIn(CheckIn {
                mode: CheckInMode::Lifecycle {
                    expected_binding_generation: 1,
                },
                claim: claim(),
                operation: crate::protocol::ids::OperationId::new("ci-1"),
                lineage_parent: None,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    };
    let transcript = temp.0.join("transcript.jsonl");
    std::fs::write(&transcript, b"").unwrap();
    let payload = json!({
        "conversationId": CONV,
        "transcriptPath": transcript.to_str().unwrap(),
        "invocationNum": 0,
        "initialNumSteps": 1,
        "prompt": WAKE_UP,
    });
    let input = HookInput {
        bytes: serde_json::to_vec(&payload).unwrap(),
        registered_event: Some("PreInvocation".to_owned()),
    };
    let decoded: DecodedEvent = decode(
        &AgyAdmission {
            step: InjectionStep::SystemMessage,
        },
        &input,
    )
    .unwrap();
    let daemon = Direct(&service);
    let identity = Identity {
        path: "/opt/a/agy".into(),
        inode: 7,
        size: 100,
        mtime_ns: 5,
    };
    let rt = Runtime::new(1000, Box::new(move || identity.clone()));
    let out = run_step(
        &HookStepContext {
            input: &input,
            decoded: &decoded,
            claim: Some(context),
            daemon: Some(&daemon),
            state_dir: Some(temp.0.join("state")),
            command_prefix: vec!["herdr-threads".into()],
            deadline: Instant::now() + Duration::from_secs(30),
        },
        &rt,
    );
    let value: Value = serde_json::from_slice(&out).unwrap();
    let text = value["injectSteps"][0]["systemMessage"]["systemMessage"]
        .as_str()
        .unwrap();
    assert!(text.starts_with(crate::delivery::HEADER), "{text}");
    assert!(text.contains("[herdr-threads] message m1 in "), "{text}");
    assert!(text.contains("hello agy"), "{text}");
    assert!(!text.contains(WAKE_UP), "{text}");
}
