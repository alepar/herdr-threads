use herdr_threads::{
    ports::StorePort,
    protocol::{
        pagination::PageRequest,
        time::{CallBudget, Clock, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::sync::Arc;

pub(crate) const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const SESSION: &str = "00000000-0000-0000-0000-000000000001";
pub(crate) struct IntentClock;
impl Clock for IntentClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> herdr_threads::protocol::time::MonoInstant {
        herdr_threads::protocol::time::MonoInstant(1000)
    }
}
pub(crate) fn budget() -> CallBudget {
    CallBudget {
        deadline: herdr_threads::protocol::time::MonoInstant(10000),
        cancellation: Default::default(),
    }
}
pub(crate) fn open(path: std::path::PathBuf, boot: uuid::Uuid) -> SqliteStore {
    let context = StoreContext::new(path, Arc::new(IntentClock)).with_test_host();
    context.require_commit_durability();
    SqliteStore::new(
        context,
        INSTANCE,
        StoreSettings {
            daemon_boot: Some(boot),
            ..Default::default()
        },
    )
    .unwrap()
}
#[allow(dead_code)]
pub(crate) fn fixture() -> (
    SqliteStore,
    std::path::PathBuf,
    herdr_threads::ports::WakeReservation,
    herdr_threads::ports::WakeIntentPreparation,
) {
    fixture_with(|_| {})
}

/// [`fixture`] with `enrolled` run after the root binding exists and before the seat's wake is
/// reserved. A check-in advances the decision sequence, which stales an earlier reservation, so
/// sessions that must exist when the reservation is taken enroll here (see [`enroll_payload`]).
#[allow(dead_code)]
pub(crate) fn fixture_with(
    enrolled: impl FnOnce(&SqliteStore),
) -> (
    SqliteStore,
    std::path::PathBuf,
    herdr_threads::ports::WakeReservation,
    herdr_threads::ports::WakeIntentPreparation,
) {
    use herdr_threads::{
        harness::{
            registry::builtins,
            wake::{WakeControlTarget, WakeReplayScope, WakeScope},
        },
        protocol::{
            authority::LivenessFingerprint,
            ids::{BindingId, ExecutionId, NativeSessionId},
        },
    };
    let path = herdr_threads::test_support::scratch::temp_path(format!(
        "ht-wake-intents-{}.db",
        uuid::Uuid::new_v4()
    ));
    let store = open(path.clone(), uuid::Uuid::new_v4());
    let db = store.context().open_writer().unwrap();
    assert_eq!(
        db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    db.execute_batch("UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=1;
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
        INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pane','host',1,1,0,'fresh','term-pane','inc','native_current_target',1);
        INSERT INTO seat_config(seat_id,key,value,updated_at,writer_provenance) VALUES ('s','programmatic_wake','on',0,'operator:local-user:1000');
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0);
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited');
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) VALUES ('inv','t','s',1,'pending',0,1,100,100,(SELECT id FROM local_node));
    ").unwrap();
    let kernel: String = db
        .query_row("SELECT boot FROM node_kernel_boot WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,binding_id,liveness_pid,liveness_start,liveness_boot) VALUES ('s',1,1,'pane','host',0,'codex',?1,'execution','cooperative_top_level',0,0,'term-pane','inc','binding',10,1,?2)", rusqlite::params![SESSION,kernel]).unwrap();
    drop(db);
    enrolled(&store);
    let candidate = StorePort::wake_candidates(
        &store,
        herdr_threads::test_support::test_host_id(),
        PageRequest::default(),
        &budget(),
    )
    .unwrap()
    .items
    .remove(0);
    let reservation = StorePort::reserve_wake(
        &store,
        herdr_threads::test_support::test_host_id(),
        &candidate,
        &budget(),
    )
    .unwrap()
    .unwrap();
    let preparation = herdr_threads::ports::WakeIntentPreparation {
        adapter: builtins().agent("codex").unwrap(),
        replay_scope: WakeReplayScope::parse(format!("session:{SESSION}")).unwrap(),
        target: WakeControlTarget {
            scope: WakeScope {
                binding: BindingId::new("binding"),
                native_session: NativeSessionId::new(SESSION),
                execution: ExecutionId::new("execution"),
                generation: 1,
                process: LivenessFingerprint { pid: 10, start: 1 },
                kernel_boot: kernel,
            },
            descriptor: b"frozen-descriptor".to_vec(),
        },
        correlation: "frozen-correlation".into(),
        side_sessions: Vec::new(),
    };
    (store, path, reservation, preparation)
}

/// The fixture seat starts with `programmatic_wake on` (queue paths need it); set or clear its explicit `programmatic_wake` row (what `ht human config seat
/// set` writes), without waking anything.
#[allow(dead_code)]
pub(crate) fn set_programmatic_wake(store: &SqliteStore, seat: &str, on: bool) {
    let db = store.context().open_writer().unwrap();
    db.execute(
        "DELETE FROM seat_config WHERE seat_id=?1 AND key='programmatic_wake'",
        [seat],
    )
    .unwrap();
    if on {
        db.execute(
            "INSERT INTO seat_config(seat_id,key,value,updated_at,writer_provenance,writer_binding) VALUES (?1,'programmatic_wake','on',0,'operator:local-user:1000',NULL)",
            [seat],
        )
        .unwrap();
    }
}

pub(crate) fn native_report(
    store: &SqliteStore,
    r: &herdr_threads::ports::WakeReservation,
    p: &herdr_threads::ports::WakeIntentPreparation,
    id: &str,
    edge: herdr_threads::harness::adapter::TurnEdge,
) -> herdr_threads::protocol::commands::TurnStateReport {
    use herdr_threads::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        ids::{HostLocator, NativeTurnId},
    };
    let host:String=store.context().open_writer().unwrap().query_row("SELECT endpoint FROM herdr_hosts WHERE host_id=(SELECT host_id FROM seats WHERE id='s')",[],|r|r.get(0)).unwrap();
    herdr_threads::protocol::commands::TurnStateReport {
        claim: CallerClaim {
            auxiliary_session: false,
            instance: INSTANCE.into(),
            host: HostLocator::parse(host).unwrap(),
            seat: r.seat.clone(),
            binding_generation: p.target.scope.generation,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: p.target.scope.native_session.clone(),
            execution: p.target.scope.execution.clone(),
            target: r.target.clone(),
            agent_id: None,
            liveness: Some(p.target.scope.process),
        },
        edge,
        tmux_hint: None,
        native_turn_id: Some(NativeTurnId::new(id)),
    }
}

/// A store-level fixture, not the hook path: it enrolls (or touches) a top-level Codex session of
/// the fixture seat from a real hook `payload` (its `session_id` replaced by `session`), building
/// the claim itself and copying the payload's auxiliary-session evidence by hand. It decodes, maps
/// the event kind to a check-in mode (a fork is an independent session, a tool boundary a plain
/// call) and checks in through the daemon's real cooperative path. `pid` picks the harness process
/// (the fixture root is pid 10, start 1). The hook path is covered by
/// `codex_auxiliary_entrypoint` in `tests/hook_entrypoint.rs` (ht-z41e.33).
#[allow(dead_code)]
pub(crate) fn enroll_payload(
    store: &SqliteStore,
    payload: &[u8],
    session: &str,
    pid: u32,
    operation: &str,
) -> Result<(), herdr_threads::protocol::results::ApiError> {
    use herdr_threads::{
        harness::{
            adapter::DecodedEvent,
            codex::{CodexContract, parse_event_for_contract},
            context::{CheckInMode as EventMode, EventKind},
        },
        ports::{CooperativePermitRequest, ReadContext, RegisterAvailableRequest},
        protocol::{
            authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint, ObligationRef},
            commands::{CheckIn, CheckInMode},
            ids::{HostLocator, HostTargetId, OperationId},
        },
        store::schema::canonical_digest,
    };
    let mut value: serde_json::Value = serde_json::from_slice(payload).unwrap();
    value["session_id"] = session.into();
    let event = DecodedEvent::from_native(
        parse_event_for_contract(
            &serde_json::to_vec(&value).unwrap(),
            operation,
            &CodexContract::registered(),
        )
        .unwrap(),
    )
    .context_event()
    .unwrap();
    let host: String = store.context().open_writer().unwrap().query_row("SELECT endpoint FROM herdr_hosts WHERE host_id=(SELECT host_id FROM seats WHERE id='s')",[],|r|r.get(0)).unwrap();
    let digest = session.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let claim = CallerClaim {
        auxiliary_session: event.auxiliary_session,
        instance: INSTANCE.into(),
        host: HostLocator::parse(host).unwrap(),
        seat: herdr_threads::protocol::ids::SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: herdr_threads::protocol::ids::NativeSessionId::new(session),
        execution: herdr_threads::protocol::ids::ExecutionId::new(format!(
            "00000000-0000-4000-8000-{:012x}",
            digest >> 16
        )),
        target: HostTargetId::new("pane"),
        agent_id: None,
        liveness: Some(LivenessFingerprint { pid, start: 1 }),
    };
    let mode = match (event.kind, event.kind.mode()) {
        (EventKind::Fork, _) => CheckInMode::Fork {
            expected_binding_generation: 1,
        },
        (_, EventMode::Current) => CheckInMode::Current,
        (_, EventMode::Lifecycle) => CheckInMode::Lifecycle {
            expected_binding_generation: 1,
        },
    };
    let command = CheckIn {
        readonly: false,
        lineage_parent: None,
        mode,
        claim,
        operation: OperationId::new(operation),
        continuity_key: None,
    };
    let permit = StorePort::issue_cooperative_permit(
        store,
        CooperativePermitRequest {
            claim: command.claim.clone(),
            operation: command.operation.clone(),
            obligation: ObligationRef::CheckIn(command.claim.seat.clone()),
            payload_hash: canonical_digest(&("check_in", &command.mode, &command.claim)).unwrap(),
            check_in_mode: Some(command.mode),
        },
        &budget(),
    )?;
    StorePort::register_available(
        store,
        RegisterAvailableRequest {
            command,
            read: ReadContext {
                instance: INSTANCE.into(),
                output: Default::default(),
                operation_scope: None,
            },
            operator: None,
        },
        permit,
        &budget(),
    )
    .map(|_| ())
}

/// A queue `WakeControl` that observes the scope it is given, journals a helper, asks `authorize`
/// exactly once (after `before_authorize`, when set) and reports an accepted add when it grants.
/// Counters: `calls` (`add` invocations), `authorized` (`authorize` calls), `effects` (accepted).
#[allow(dead_code)]
#[derive(Default)]
pub(crate) struct ArmingControl {
    pub(crate) calls: std::sync::atomic::AtomicUsize,
    pub(crate) authorized: std::sync::atomic::AtomicUsize,
    pub(crate) effects: std::sync::atomic::AtomicUsize,
    pub(crate) before_authorize: Option<Box<dyn Fn() + Send + Sync>>,
}
impl herdr_threads::harness::wake::WakeControl for ArmingControl {
    fn observe(
        &self,
        scope: &herdr_threads::harness::wake::WakeScope,
        _: &herdr_threads::harness::wake::WakeControlContext<'_>,
    ) -> Result<
        Option<herdr_threads::harness::wake::WakeControlTarget>,
        herdr_threads::protocol::results::ApiError,
    > {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Some(herdr_threads::harness::wake::WakeControlTarget {
            scope: scope.clone(),
            descriptor: b"{}".to_vec(),
        }))
    }
    fn add(
        &self,
        target: &herdr_threads::harness::wake::WakeControlTarget,
        _: &str,
        authorize: &mut dyn FnMut() -> Result<bool, herdr_threads::protocol::results::ApiError>,
        context: &herdr_threads::harness::wake::WakeControlContext<'_>,
    ) -> herdr_threads::harness::wake::WakeAddOutcome {
        use herdr_threads::harness::wake::{WakeAddOutcome, WakeHelperWitness};
        use std::sync::atomic::Ordering;
        let observer = context
            .helper_observer
            .expect("runtime must journal every helper");
        observer
            .started(&WakeHelperWitness {
                process: target.scope.process,
                kernel_boot: target.scope.kernel_boot.clone(),
                pgid: target.scope.process.pid,
            })
            .unwrap();
        if let Some(before) = &self.before_authorize {
            before();
        }
        self.authorized.fetch_add(1, Ordering::SeqCst);
        let outcome = match authorize() {
            Ok(true) => {
                self.effects.fetch_add(1, Ordering::SeqCst);
                WakeAddOutcome::Accepted {
                    id: "queued-fixture-1".into(),
                    cleanup_error: None,
                }
            }
            Ok(false) => WakeAddOutcome::RefusedBeforeEffect(
                herdr_threads::protocol::results::ApiError::new(
                    herdr_threads::protocol::results::ErrorCode::TargetUnsafe,
                    "final guard refused",
                )
                .into(),
            ),
            Err(error) => WakeAddOutcome::RefusedBeforeEffect(error.into()),
        };
        observer.completed(true).unwrap();
        outcome
    }
    fn list(
        &self,
        _: &herdr_threads::harness::wake::WakeControlTarget,
        _: &str,
        _: &herdr_threads::harness::wake::WakeControlContext<'_>,
    ) -> Result<
        herdr_threads::harness::wake::OwnedWakeEntries,
        Box<herdr_threads::harness::wake::ControlFailure>,
    > {
        Ok(herdr_threads::harness::wake::OwnedWakeEntries {
            ids: Vec::new(),
            complete: true,
        })
    }
    fn delete(
        &self,
        _: &herdr_threads::harness::wake::WakeControlTarget,
        _: &str,
        _: &herdr_threads::harness::wake::WakeControlContext<'_>,
    ) -> Result<bool, Box<herdr_threads::harness::wake::ControlFailure>> {
        Ok(false)
    }
}
