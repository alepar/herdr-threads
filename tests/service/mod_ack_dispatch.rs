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

/// Scripted registry: `live` answers `is_live` for every generation; the
/// generations asked about are recorded, and `record_ack` calls are counted.
#[derive(Default)]
struct FakeModChannels {
    live: Mutex<bool>,
    asked: Mutex<Vec<(String, u64)>>,
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
    fn is_live(&self, seat: &SeatId, generation: u64) -> bool {
        self.asked
            .lock()
            .unwrap()
            .push((seat.as_str().to_string(), generation));
        *self.live.lock().unwrap()
    }
    fn record_ack(&self, _seat: &SeatId, _now: UtcMillis) {
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
    }
}
/// A lifecycle check-in needs an execution UUID the seat has not used yet.
fn fresh_execution(mut claim: CallerClaim, n: u8) -> CallerClaim {
    claim.execution = ExecutionId::new(format!("00000000-0000-4000-8000-0000000000{n:02}"));
    claim
}
/// Lifecycle check-in; returns the claim the daemon hands back.
fn check_in(f: &Fixture, claim: CallerClaim, operation: &str) -> CallerClaim {
    let command = Command::CheckIn(CheckIn {
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
        Some(("s".to_string(), context.binding_generation))
    );
}

#[test]
fn older_generation_ack_is_stale_generation() {
    let f = fixture();
    let first = claude(&f, "n1");
    // /clear: a new native session rotates the generation.
    let second = check_in(
        &f,
        fresh_execution(
            claim("s", "p", Harness::Claude, "n2", first.binding_generation),
            2,
        ),
        "ci-2",
    );
    assert!(second.binding_generation > first.binding_generation);
    let report = ack(&f, first, &["m1"], "op").unwrap();
    assert_eq!(all(&report), vec![(ModAckOutcome::StaleGeneration, None)]);
    assert_eq!(state(&f, "m1"), "pending");
    assert_eq!(f.channels.acks(), 0);
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
        second.binding_generation,
        first.binding_generation + 1,
        "the same native session still rotates the generation"
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
fn subagent_claim_rejected_by_validation() {
    let f = fixture();
    let mut child = claude(&f, "n1");
    child.role = CallerRole::Subagent;
    let error = ack(&f, child, &["m1"], "op").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(state(&f, "m1"), "pending");
    assert!(f.channels.asked.lock().unwrap().is_empty());
}
