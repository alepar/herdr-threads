//! `seat retire --self` (N3-retire): the daemon decides the retirement of the caller's named
//! seat, the M18/M20 refusals and the S4 replay scope.
use crate::{
    ports::{HostKind, LocalService, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{
            CheckIn, CheckInMode, Command, OperatorRetire, ResolveSeat, SeatInspectQuery,
            SelfRetire,
        },
        ids::*,
        pagination::PageRequest,
        results::{AbandonedKind, ApiError, CommandResult, ErrorCode, SeatHistoryItem},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::dispatch::DomainService,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::{os::unix::fs::DirBuilderExt, sync::Arc};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

struct PrivateDirectory(std::path::PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
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

struct Fixture {
    // Drop order: the service and connection release the store before its directory goes.
    service: DomainService,
    db: Connection,
    store_path: std::path::PathBuf,
    named_host: i64,
    _directory: PrivateDirectory,
}

/// A service over one seat `s`: a named seat `reviewer` on the named host, or (`named` false) a
/// pane seat on the fixture host.
fn fixture(named: bool) -> Fixture {
    let path = crate::test_support::scratch::temp_path(format!(
        "service-self-retire-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let directory = PrivateDirectory(path.clone());
    let store_path = path.join("store.db");
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let context = if named {
        StoreContext::new(store_path.clone(), clock.clone()).with_attached_host(&locator)
    } else {
        StoreContext::new(store_path.clone(), clock.clone()).with_test_host()
    };
    let db = context.open_writer().unwrap();
    let kind = if named { "named" } else { "herdr" };
    db.execute(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1,kind=?1 WHERE endpoint=?2",
        rusqlite::params![
            kind,
            if named {
                locator.as_str().to_owned()
            } else {
                crate::test_support::test_host_locator().as_str().to_owned()
            }
        ],
    )
    .unwrap();
    let host: i64 = db
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind=?1",
            [kind],
            |r| r.get(0),
        )
        .unwrap();
    let (target, name) = if named {
        ("reviewer", Some("reviewer"))
    } else {
        ("p", None)
    };
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,name) VALUES ('s',?1,'resolved','native',?2,0,0,0,(SELECT id FROM local_node),1,?3,?4)",
        rusqlite::params![INSTANCE, target, host, name]).unwrap();
    db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,?2,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc','coherent_enumeration',1)",
        rusqlite::params![host, target]).unwrap();
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, INSTANCE, StoreSettings::default()).unwrap());
    let service = DomainService::new(INSTANCE.into(), store, clock)
        .with_operator_owner(501)
        .with_cooperative_owner(
            501,
            Arc::new(crate::service::fair_writer::FairWriter::new(32)),
        );
    Fixture {
        service,
        db,
        store_path,
        named_host: host,
        _directory: directory,
    }
}

fn claim(named: bool, harness: Harness, session: &str, execution: &str) -> CallerClaim {
    CallerClaim {
        host: if named {
            HostLocator::parse(NAMED_HOST_LOCATOR).unwrap()
        } else {
            crate::test_support::test_host_locator()
        },
        instance: INSTANCE.into(),
        seat: SeatId::new("s"),
        binding_generation: 0,
        role: CallerRole::TopLevel,
        harness,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new(execution),
        target: HostTargetId::new(if named { "reviewer" } else { "p" }),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}
const EXEC_1: &str = "00000000-0000-4000-8000-000000000001";
const EXEC_2: &str = "00000000-0000-4000-8000-000000000002";

fn peer() -> PeerIdentity {
    PeerIdentity::from_kernel(501)
}

/// Checks `claim` in and returns the context the daemon answers with.
fn check_in(f: &Fixture, claim: CallerClaim, operation: &str) -> CallerClaim {
    let command = Command::CheckIn(CheckIn {
        lineage_parent: None,
        readonly: false,
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

fn retire(
    f: &Fixture,
    claim: &CallerClaim,
    operation: &str,
    force: bool,
) -> Result<CommandResult, ApiError> {
    f.service.handle(
        Command::SelfRetire(SelfRetire {
            operation: OperationId::new(operation),
            claim: claim.clone(),
            force,
        }),
        peer(),
        &budget(),
    )
}

/// One pending invitation and one message wanting an ACK, both for seat `s`.
fn pending_obligations(db: &Connection) {
    db.execute_batch(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
         SELECT 't',(SELECT id FROM local_node),'x','',0,0,0; \
         INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,event_offset) \
         SELECT 'm1',(SELECT id FROM local_node),'t',1,'ordinary','b',0,1,1; \
         INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) \
         VALUES ('m1','t','s','pending',100,0,100); \
         INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) \
         SELECT 'i1','t','s',1,'pending',0,1,100,100,(SELECT id FROM local_node);",
    )
    .unwrap();
}

fn seat_state(db: &Connection) -> String {
    db.query_row("SELECT state FROM seats WHERE id='s'", [], |r| r.get(0))
        .unwrap()
}

fn decision_label(db: &Connection) -> (String, Option<String>) {
    db.query_row(
        "SELECT kind,operator_label FROM allocation_decisions WHERE seat_id='s' ORDER BY ordinal DESC LIMIT 1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn only_error(result: Result<CommandResult, ApiError>) -> ApiError {
    result.expect_err("the call is refused")
}

// Kills: a retire that leaves the seat live, drops the abandoned list, records the wrong wire
// reason or local label, or lets the name stay taken.
#[test]
fn self_retire_retires_and_lists_abandoned() {
    let f = fixture(true);
    let me = check_in(&f, claim(true, Harness::Codex, "session-n", EXEC_1), "in");
    pending_obligations(&f.db);
    let CommandResult::SelfRetired(retired) = retire(&f, &me, "retire-1", false).unwrap() else {
        panic!("a self retirement answers SelfRetired");
    };
    assert_eq!(retired.seat, SeatId::new("s"));
    let rows: Vec<(AbandonedKind, &str)> = retired
        .abandoned
        .iter()
        .map(|item| (item.kind, item.id.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            (AbandonedKind::Invitation, "i1"),
            (AbandonedKind::Ack, "m1")
        ]
    );
    assert_eq!(retired.more, 0);
    assert_eq!(
        crate::cli::self_retire::render(&retired),
        "invite i1\nack m1\nretired s\n"
    );
    assert_eq!(seat_state(&f.db), "retired");
    // Locally a cooperative self retirement; on the wire an Operator retirement.
    assert_eq!(
        decision_label(&f.db),
        (
            "operator_retire".to_owned(),
            Some(format!(
                "cooperative_self_retire:{}",
                f.db.query_row(
                    "SELECT binding_id FROM occupant_bindings WHERE seat_id='s' LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap()
            ))
        )
    );
    // `seat inspect` shows the local label.
    let inspected = f
        .service
        .handle(
            Command::SeatInspect(SeatInspectQuery {
                seat: SeatId::new("s"),
                page: PageRequest {
                    cursor: None,
                    limit: 20,
                    max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
                },
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::SeatInspect(inspection) = inspected else {
        panic!("inspection");
    };
    assert!(
        inspection.history.items.iter().any(|item| matches!(
            item,
            SeatHistoryItem::Repair(repair)
                if repair.operator_label.as_deref().is_some_and(|label| label.starts_with("cooperative_self_retire:"))
        )),
        "seat inspect shows cooperative_self_retire"
    );
    // Reusing the name mints a new seat.
    let context = StoreContext::new(f.store_path.clone(), Arc::new(FixedClock));
    let mut conn = context.open_writer().unwrap();
    let reuse = crate::store::seats::resolve_named_seat(
        &context,
        &mut conn,
        INSTANCE,
        crate::ports::HostId::from_row(f.named_host),
        &ResolveSeat {
            target: HostTargetId::new("reviewer"),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new("resolve-again"),
            locator_hint: None,
            tmux_hint: None,
        },
        &budget(),
    )
    .unwrap();
    let CommandResult::NamedSeatResolved { seat, created, .. } = reuse else {
        panic!("named resolve");
    };
    assert!(created, "the retired name is free again");
    assert_ne!(seat, SeatId::new("s"), "a new seat, never the retired one");
}

// Kills: a pane seat retiring itself, or M20 drifting from its 57-byte golden.
#[test]
fn refused_for_pane_seat_m20() {
    let f = fixture(false);
    let me = check_in(&f, claim(false, Harness::Codex, "session-n", EXEC_1), "in");
    let error = only_error(retire(&f, &me, "retire-1", false));
    assert_eq!(
        format!("{}\n", error.detail),
        include_str!("../cli/golden/m20.txt")
    );
    assert_eq!(error.detail.len(), 57);
    assert_eq!(seat_state(&f.db), "resolved");
}

// Kills: a declared subagent retiring the seat of its parent.
#[test]
fn refused_for_subagent() {
    let f = fixture(true);
    let me = check_in(&f, claim(true, Harness::Codex, "session-n", EXEC_1), "in");
    let mut child = me.clone();
    child.role = CallerRole::Subagent;
    child.agent_id = Some(AgentId::new("agent-1"));
    let error = only_error(retire(&f, &child, "retire-1", false));
    assert!(
        matches!(
            error.code,
            ErrorCode::InvalidRequest | ErrorCode::CallerUnverified | ErrorCode::SubagentReadOnly
        ),
        "{error:?}"
    );
    assert_eq!(seat_state(&f.db), "resolved");
    let operations: i64 =
        f.db.query_row(
            "SELECT count(*) FROM operations WHERE actor_scope LIKE 'self-retire:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(operations, 0, "a refused subagent leaves no decision");
}

// Kills: retiring under a second connected top-level binding without --force, and --force being
// ignored.
#[test]
fn refused_with_other_connected_binding_m18_unless_force() {
    let f = fixture(true);
    let me = check_in(&f, claim(true, Harness::Codex, "session-n", EXEC_1), "in-1");
    let other = check_in(
        &f,
        claim(true, Harness::Claude, "session-o", EXEC_2),
        "in-2",
    );
    let other_binding: String =
        f.db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s' AND harness='claude'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let error = only_error(retire(&f, &me, "retire-1", false));
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(
        error.detail,
        crate::store::seats::m18(&SeatId::new("s"), std::slice::from_ref(&other_binding))
    );
    assert_eq!(
        format!(
            "{}\n",
            crate::store::seats::m18(
                &SeatId::new("seat-Xy98Zw76"),
                &["bnd-a".to_owned(), "bnd-b".to_owned()]
            )
        ),
        include_str!("../cli/golden/m18.txt")
    );
    assert_eq!(
        crate::store::seats::m18(&SeatId::new("s"), &[]).len(),
        65 + 1,
        "65 bytes of fixed text plus the seat id"
    );
    assert_eq!(seat_state(&f.db), "resolved", "a refusal retires nothing");
    let _ = other;
    assert!(matches!(
        retire(&f, &me, "retire-2", true).unwrap(),
        CommandResult::SelfRetired(_)
    ));
    assert_eq!(seat_state(&f.db), "retired");
}

// Kills: a replay of a committed retirement reaching the live-seat precheck (the seat is retired
// now) instead of returning the stored result, and a replay with another payload passing.
#[test]
fn s4_replay_returns_stored_result() {
    let f = fixture(true);
    let me = check_in(&f, claim(true, Harness::Codex, "session-n", EXEC_1), "in");
    pending_obligations(&f.db);
    let first = retire(&f, &me, "retire-1", false).unwrap();
    assert_eq!(seat_state(&f.db), "retired");
    let replay = retire(&f, &me, "retire-1", false).unwrap();
    assert_eq!(replay, first, "the stored result, not NotFound");
    let rows: i64 = f
        .db
        .query_row(
            "SELECT count(*) FROM operations WHERE actor_scope='self-retire:s' AND operation_key='retire-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1, "the scope is self-retire:<seat>");
    let decisions: i64 = f
        .db
        .query_row(
            "SELECT count(*) FROM allocation_decisions WHERE seat_id='s' AND kind='operator_retire'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(decisions, 1, "a replay decides nothing again");
    // A new operation on the retired seat is the live-seat refusal.
    let error = only_error(retire(&f, &me, "retire-2", false));
    assert!(
        matches!(
            error.code,
            ErrorCode::NotFound | ErrorCode::CallerUnverified
        ),
        "{error:?}"
    );
    // The same key with another payload is not a replay.
    let mismatch = only_error(retire(&f, &me, "retire-1", true));
    assert_eq!(mismatch.code, ErrorCode::OperationPayloadMismatch);
}

// Kills: `seat retire SEAT --operator` refusing a named seat, or the name staying taken.
#[test]
fn operator_retire_of_named_seat_then_new_seat() {
    let f = fixture(true);
    let _ = check_in(&f, claim(true, Harness::Codex, "session-n", EXEC_1), "in");
    let result = f
        .service
        .store
        .mutate_operator(
            crate::ports::OperatorRequest::Retire(OperatorRetire {
                seat: SeatId::new("s"),
                operation: OperationId::new("op-retire"),
            }),
            crate::protocol::authority::OperatorActor::from_peer(peer(), 501).unwrap(),
            &budget(),
        )
        .unwrap();
    assert!(matches!(result, CommandResult::OperatorRetired(_)));
    assert_eq!(seat_state(&f.db), "retired");
    let context = StoreContext::new(f.store_path.clone(), Arc::new(FixedClock));
    let mut conn = context.open_writer().unwrap();
    let CommandResult::NamedSeatResolved { seat, created, .. } =
        crate::store::seats::resolve_named_seat(
            &context,
            &mut conn,
            INSTANCE,
            crate::ports::HostId::from_row(f.named_host),
            &ResolveSeat {
                target: HostTargetId::new("reviewer"),
                host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
                operation: OperationId::new("resolve-again"),
                locator_hint: None,
                tmux_hint: None,
            },
            &budget(),
        )
        .unwrap()
    else {
        panic!("named resolve");
    };
    assert!(created);
    assert_ne!(seat, SeatId::new("s"));
}

// Kills: a SelfRetire a daemon that predates it would decode as another command. The wire shape
// is its own externally tagged variant, so an older daemon's Command enum rejects it as an
// unknown variant (the upgrade path); the CLI additionally refuses before sending when the
// daemon does not advertise `seat.host_sites_v1` (M13).
#[test]
fn old_daemon_answers_upgrade_error() {
    #[derive(Debug, serde::Deserialize)]
    #[serde(tag = "kind", content = "args", rename_all = "snake_case")]
    enum OlderCommand {
        #[allow(dead_code)]
        Health,
    }
    let command = Command::SelfRetire(SelfRetire {
        operation: OperationId::new("op"),
        claim: claim(true, Harness::Codex, "session-n", EXEC_1),
        force: false,
    });
    let wire = serde_json::to_value(&command).unwrap();
    assert_eq!(wire["kind"], "self_retire", "{wire}");
    let error = serde_json::from_value::<OlderCommand>(wire).unwrap_err();
    assert!(error.to_string().contains("unknown variant"), "{error}");
}

#[test]
fn kind_is_named_for_the_named_fixture() {
    let f = fixture(true);
    let kind: String =
        f.db.query_row(
            "SELECT kind FROM herdr_hosts WHERE host_id=?1",
            [f.named_host],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "named");
    let _ = HostKind::Named;
}
