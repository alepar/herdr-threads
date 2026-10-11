//! Domain pre-decision for `watch ack` (spec D6, ht-j16.3): binding checks,
//! the resume rule, live-channel gating and `record_ack` accounting.
use crate::{
    ports::{LocalService, ModChannels, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{AckModDelivered, CheckIn, CheckInMode, Command},
        ids::*,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
        watch::{ModAckOutcome, ModAckReason, ModAckReport, ModDeliveryVia},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

/// Scripted registry: `live` answers `is_live` for every binding except those in `dead`; the
/// bindings asked about are recorded, and `record_ack` calls are counted per binding.
#[derive(Default)]
struct FakeModChannels {
    live: Mutex<bool>,
    dead: Mutex<Vec<BindingId>>,
    asked: Mutex<Vec<(String, BindingId)>>,
    acked: Mutex<Vec<BindingId>>,
    acks: AtomicUsize,
}
impl FakeModChannels {
    fn set_live(&self, live: bool) {
        *self.live.lock().unwrap() = live;
    }
    fn acks(&self) -> usize {
        self.acks.load(Ordering::SeqCst)
    }
}
impl ModChannels for FakeModChannels {
    fn is_live(&self, seat: &SeatId, binding: &BindingId) -> bool {
        self.asked
            .lock()
            .unwrap()
            .push((seat.as_str().to_string(), binding.clone()));
        *self.live.lock().unwrap() && !self.dead.lock().unwrap().contains(binding)
    }
    fn record_ack(&self, _seat: &SeatId, binding: &BindingId, _now: UtcMillis) {
        self.acked.lock().unwrap().push(binding.clone());
        self.acks.fetch_add(1, Ordering::SeqCst);
    }
}

struct Fixture {
    service: DomainService,
    channels: Arc<FakeModChannels>,
    db: Connection,
    directory: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
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

/// Seats `s` (Claude), `x` (Codex) and `none` (never checked in); thread `t`
/// with `s` joined and pending ordinary messages `m1`, `m2` for it.
fn fixture() -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let directory = std::env::temp_dir().join(format!("mod-ack-dispatch-{}", uuid::Uuid::new_v4()));
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
    for (seat, target) in [("s", "p"), ("x", "px"), ("none", "pn")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,0,0,0)", [seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,3)", []).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1)", []).unwrap();
    for (n, id) in ["m1", "m2"].into_iter().enumerate() {
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','hello',0,?3)", rusqlite::params![id, n as i64 + 1, n as i64 + 2]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','s','pending',300000)", [id]).unwrap();
    }
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let channels = Arc::new(FakeModChannels::default());
    channels.set_live(true);
    let service = DomainService::new("i".into(), store, clock)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_mod_channels(channels.clone());
    Fixture {
        service,
        channels,
        db,
        directory,
    }
}

fn claim(
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
    generation: u64,
) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(target),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}
/// A lifecycle check-in needs an execution UUID the seat has not used yet.
fn fresh_execution(mut claim: CallerClaim, n: u8) -> CallerClaim {
    claim.execution = ExecutionId::new(format!("00000000-0000-4000-8000-0000000000{n:02}"));
    claim
}
/// The claim of a session in another process: both sessions stay connected (a same-process
/// lifecycle claim would disconnect the older session, which then has no channel to ack through).
fn second_process(mut claim: CallerClaim) -> CallerClaim {
    claim.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid: 2, start: 1 });
    claim
}
/// Lifecycle check-in; returns the claim the daemon hands back.
fn check_in(f: &Fixture, claim: CallerClaim, operation: &str) -> CallerClaim {
    let command = Command::CheckIn(CheckIn {
        lineage_parent: None,
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: claim.binding_generation,
        },
        claim,
        operation: OperationId::new(operation),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    }
}
fn ack(
    f: &Fixture,
    claim: CallerClaim,
    ids: &[&str],
    operation: &str,
) -> Result<ModAckReport, crate::protocol::results::ApiError> {
    let command = Command::AckModDelivered(AckModDelivered {
        via: ModDeliveryVia::Context,
        messages: ids.iter().map(|id| MessageId::new(*id)).collect(),
        operation: OperationId::new(operation),
        claim,
    });
    match f.service.handle(command, peer(), &budget())? {
        CommandResult::ModDeliveryAcked(report) => Ok(report),
        other => panic!("unexpected {other:?}"),
    }
}
fn all(report: &ModAckReport) -> Vec<(ModAckOutcome, Option<ModAckReason>)> {
    report
        .results
        .iter()
        .map(|i| (i.result, i.reason))
        .collect()
}
fn state(f: &Fixture, id: &str) -> String {
    f.db.query_row(
        "SELECT state FROM receipts WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
fn claude(f: &Fixture, session: &str) -> CallerClaim {
    check_in(f, claim("s", "p", Harness::Claude, session, 0), "ci-1")
}
/// The binding the session holds on seat `s`.
fn binding_of(f: &Fixture, session: &str) -> BindingId {
    BindingId::new(
        f.db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s' AND native_session=?1 AND agent_id=''",
            [session],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
}
fn acked_by(f: &Fixture, id: &str) -> Option<String> {
    f.db.query_row(
        "SELECT ack_binding_id FROM receipts WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn no_live_channel_makes_every_id_retryable_and_settles_nothing() {
    let f = fixture();
    let context = claude(&f, "n1");
    f.channels.set_live(false);
    let report = ack(&f, context, &["m1", "m2"], "op").unwrap();
    assert_eq!(
        all(&report),
        vec![(ModAckOutcome::Retryable, Some(ModAckReason::NoLiveChannel)); 2]
    );
    assert_eq!(state(&f, "m1"), "pending");
    assert_eq!(state(&f, "m2"), "pending");
    assert_eq!(f.channels.acks(), 0);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM operations WHERE operation_key='op'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "a refused batch records no operation"
    );
}

#[test]
fn a_plain_call_binding_with_a_live_channel_mod_acks_and_keeps_its_provenance() {
    let f = fixture();
    // The binding is a V32-migrated row that a plain call then re-adopts, and the plain call
    // never rewrites its observation provenance.
    let first = claude(&f, "n1");
    let id = binding_of(&f, "n1");
    f.db.execute(
        "UPDATE occupant_bindings SET observation_provenance='cooperative_env_session',migrated=1,liveness='disconnected',disconnected_at=1,disconnect_reason='migrated' WHERE binding_id=?1",
        [id.as_str()],
    )
    .unwrap();
    let command = Command::CheckIn(CheckIn {
        lineage_parent: None,
        mode: CheckInMode::Current,
        claim: claim("s", "p", Harness::Claude, "n1", first.binding_generation),
        operation: OperationId::new("plain-1"),
    });
    let context = match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    };
    let (liveness, provenance): (String, String) =
        f.db.query_row(
            "SELECT liveness,observation_provenance FROM occupant_bindings WHERE binding_id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        liveness, "connected",
        "the plain call re-adopts the binding"
    );
    assert_eq!(provenance, "cooperative_env_session");
    let report = ack(&f, context, &["m1"], "op").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::Settled, None)]);
    assert_eq!(acked_by(&f, "m1").as_deref(), Some(id.as_str()));
    let provenance: String =
        f.db.query_row(
            "SELECT observation_provenance FROM occupant_bindings WHERE binding_id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(provenance, "cooperative_env_session");
}

#[test]
fn reconnect_grace_counts_as_live() {
    let f = fixture();
    let context = claude(&f, "n1");
    // The registry answers `true` during reconnect grace; the ack consults it
    // for the binding's current generation and settles.
    let report = ack(&f, context.clone(), &["m1"], "op").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::Settled, None)]);
    assert_eq!(state(&f, "m1"), "acked");
    assert_eq!(
        f.channels.asked.lock().unwrap().last().cloned(),
        Some(("s".to_string(), binding_of(&f, "n1")))
    );
}

#[test]
fn a_second_session_acks_through_its_own_binding_and_the_older_one_still_does() {
    let f = fixture();
    let first = claude(&f, "n1");
    // /clear: a new native session is a binding of its own beside the first.
    let second = check_in(
        &f,
        second_process(fresh_execution(
            claim("s", "p", Harness::Claude, "n2", first.binding_generation),
            2,
        )),
        "ci-2",
    );
    assert_eq!(second.binding_generation, first.binding_generation);
    let (n1, n2) = (binding_of(&f, "n1"), binding_of(&f, "n2"));
    assert_ne!(n1, n2);
    // Each session's ack is decided against its own binding's channel.
    let report = ack(&f, first, &["m1"], "op-1").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::Settled, None)]);
    let report = ack(&f, second, &["m2"], "op-2").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::Settled, None)]);
    let asked: Vec<BindingId> = f
        .channels
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|(_, binding)| binding.clone())
        .collect();
    assert_eq!(asked, vec![n1.clone(), n2.clone()]);
    assert_eq!(*f.channels.acked.lock().unwrap(), vec![n1, n2]);
}

#[test]
fn mod_ack_settles_as_the_channels_binding() {
    let f = fixture();
    let first = claude(&f, "n1");
    let second = check_in(
        &f,
        second_process(fresh_execution(
            claim("s", "p", Harness::Claude, "n2", first.binding_generation),
            2,
        )),
        "ci-2",
    );
    let (n1, n2) = (binding_of(&f, "n1"), binding_of(&f, "n2"));
    // n2's channel settles m1: the receipt is attributed to n2's binding, never to n1's.
    let report = ack(&f, second, &["m1"], "op-1").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::Settled, None)]);
    assert_eq!(acked_by(&f, "m1").as_deref(), Some(n2.as_str()));
    assert_eq!(*f.channels.acked.lock().unwrap(), vec![n2.clone()]);
    // n1's channel then acks the same id: the seat's receipt settles once, as n2.
    let again = ack(&f, first.clone(), &["m1"], "op-2").unwrap();
    assert_eq!(all(&again), vec![(ModAckOutcome::AlreadySettled, None)]);
    assert_eq!(acked_by(&f, "m1").as_deref(), Some(n2.as_str()));
    // And n1 settles what it acks first.
    ack(&f, first, &["m2"], "op-3").unwrap();
    assert_eq!(acked_by(&f, "m2").as_deref(), Some(n1.as_str()));
}

#[test]
fn an_ack_through_a_binding_without_a_live_channel_is_retryable_while_the_other_settles() {
    let f = fixture();
    let first = claude(&f, "n1");
    let second = check_in(
        &f,
        fresh_execution(
            claim("s", "p", Harness::Claude, "n2", first.binding_generation),
            2,
        ),
        "ci-2",
    );
    // Only n1's channel is gone (closed); n2's is live.
    f.channels.dead.lock().unwrap().push(binding_of(&f, "n1"));
    let refused = ack(&f, first, &["m1"], "op-1").unwrap();
    assert_eq!(
        all(&refused),
        vec![(ModAckOutcome::Retryable, Some(ModAckReason::NoLiveChannel))]
    );
    assert_eq!(state(&f, "m1"), "pending");
    let settled = ack(&f, second, &["m1"], "op-2").unwrap();
    assert_eq!(all(&settled), vec![(ModAckOutcome::Settled, None)]);
    assert_eq!(state(&f, "m1"), "acked");
}

#[test]
fn resume_previous_generation_same_native_session_settles() {
    let f = fixture();
    let first = claude(&f, "n1");
    let second = check_in(
        &f,
        fresh_execution(
            claim("s", "p", Harness::Claude, "n1", first.binding_generation),
            2,
        ),
        "ci-2",
    );
    assert_eq!(
        second.binding_generation, first.binding_generation,
        "the same native session re-registers its binding; the mapping generation holds"
    );
    let report = ack(&f, first, &["m1"], "op").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::Settled, None)]);
    let generation: i64 =
        f.db.query_row(
            "SELECT ack_generation FROM receipts WHERE message_id='m1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        generation as u64, second.binding_generation,
        "settled under the canonical current binding, not the stale claim"
    );
    assert_eq!(f.channels.acks(), 1);
}

#[test]
fn same_generation_after_reload_settles() {
    let f = fixture();
    let context = claude(&f, "n1");
    f.channels.set_live(false);
    let early = ack(&f, context.clone(), &["m1"], "op-1").unwrap();
    assert_eq!(
        all(&early),
        vec![(ModAckOutcome::Retryable, Some(ModAckReason::NoLiveChannel))]
    );
    // The mod reloads and re-registers for the same generation; its late ack
    // settles normally.
    f.channels.set_live(true);
    let late = ack(&f, context, &["m1"], "op-2").unwrap();
    assert_eq!(all(&late), vec![(ModAckOutcome::Settled, None)]);
    assert_eq!(state(&f, "m1"), "acked");
}

#[test]
fn record_ack_called_only_when_something_settled() {
    let f = fixture();
    let context = claude(&f, "n1");
    let refused = ack(&f, context.clone(), &["ghost"], "op-1").unwrap();
    assert_eq!(
        all(&refused),
        vec![(ModAckOutcome::RefusedTerminal, Some(ModAckReason::Unknown))]
    );
    assert_eq!(f.channels.acks(), 0);
    ack(&f, context.clone(), &["m1"], "op-2").unwrap();
    assert_eq!(f.channels.acks(), 1);
    let mixed = ack(&f, context, &["m1", "ghost"], "op-3").unwrap();
    assert_eq!(
        all(&mixed),
        vec![
            (ModAckOutcome::AlreadySettled, None),
            (ModAckOutcome::RefusedTerminal, Some(ModAckReason::Unknown)),
        ]
    );
    assert_eq!(f.channels.acks(), 2, "already_settled counts");
}

#[test]
fn no_binding_or_codex_binding_is_retryable() {
    let f = fixture();
    let retryable = vec![(ModAckOutcome::Retryable, Some(ModAckReason::NoLiveChannel))];
    let nobody = claim("none", "pn", Harness::Claude, "n1", 1);
    assert_eq!(all(&ack(&f, nobody, &["m1"], "op-1").unwrap()), retryable);
    let codex = check_in(&f, claim("x", "px", Harness::Codex, "c1", 0), "ci-x");
    let mut as_claude = codex.clone();
    as_claude.harness = Harness::Claude;
    assert_eq!(
        all(&ack(&f, as_claude, &["m1"], "op-2").unwrap()),
        retryable
    );
    assert!(f.channels.asked.lock().unwrap().is_empty());
    assert_eq!(f.channels.acks(), 0);
    assert_eq!(state(&f, "m1"), "pending");
}

#[test]
fn subagent_claim_is_refused_read_only() {
    let f = fixture();
    let mut child = claude(&f, "n1");
    child.role = CallerRole::Subagent;
    let error = ack(&f, child, &["m1"], "op").unwrap_err();
    assert_eq!(error.code, ErrorCode::SubagentReadOnly);
    assert_eq!(state(&f, "m1"), "pending");
    assert!(f.channels.asked.lock().unwrap().is_empty());
}
