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
pub(crate) fn fixture() -> (
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
    };
    (store, path, reservation, preparation)
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
