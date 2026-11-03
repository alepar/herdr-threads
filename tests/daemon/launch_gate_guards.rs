//! ht-kqrt.206: each guard of the launch gate, of `core_port` and of the turn edge's hint hand-off
//! is decided alone by its own test, through `DomainService::handle`. Included from
//! `src/service/dispatch.rs` so it sees the service directly.
use crate::{
    ports::{BootstrapStorePort, HostId, LocalService, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint},
        commands::{Command, TurnStateReport},
        handoff::{LocalLaunchGate, LocalLaunchGateAction},
        ids::*,
        locate::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::{
        dispatch::{DomainService, PeerIdentity},
        fair_writer::FairWriter,
        locate::environ_reader_tests::EnvHost,
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::{os::unix::fs::DirBuilderExt, sync::Arc};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const OWNER: u32 = 501;
const CORE_SOCKET: &str = "/herdr-threads/stub-core.sock";

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

struct Chain(Vec<(u32, Option<u32>)>);
impl crate::ports::ProcessInfo for Chain {
    fn process(&self, pid: u32) -> Option<crate::ports::ProcessRecord> {
        self.0
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(pid, parent)| crate::ports::ProcessRecord {
                pid: *pid,
                parent: *parent,
                start: 1,
                argv: Vec::new(),
            })
    }
}

struct Fixture {
    service: DomainService,
    db: Connection,
    _directory: PrivateDirectory,
}

/// A service over the one prebuilt `host`, the elected owner `OWNER`, and the node-wide
/// bootstrap store (the same store, as in the daemon).
fn fixture(host: Arc<EnvHost>) -> Fixture {
    let path = crate::test_support::scratch::temp_path(format!(
        "daemon-launch-gate-guards-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&path)
        .unwrap();
    let directory = PrivateDirectory(path.clone());
    let clock: Arc<dyn Clock> = Arc::new(FixedClock);
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let context =
        StoreContext::new(path.join("store.db"), clock.clone()).with_attached_host(&locator);
    let db = context.open_writer().unwrap();
    db.execute(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1,kind='named' WHERE endpoint=?1",
        [locator.as_str()],
    )
    .unwrap();
    let named_host: i64 = db
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind='named'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let store = Arc::new(SqliteStore::new(context, INSTANCE, StoreSettings::default()).unwrap());
    let service = DomainService::with_host(
        INSTANCE.into(),
        HostId::from_row(named_host),
        Arc::clone(&store) as Arc<dyn StorePort>,
        clock,
        host,
        Arc::new(FairWriter::new(32)),
    )
    .with_bootstrap_store(store as Arc<dyn BootstrapStorePort>)
    .with_processes(Arc::new(Chain(vec![(30, Some(1)), (1, None)])))
    .with_cooperative_owner(OWNER, Arc::new(FairWriter::new(8)));
    Fixture {
        service,
        db,
        _directory: directory,
    }
}

fn gate_rows(db: &Connection) -> i64 {
    db.query_row("SELECT count(*) FROM local_launch_gates", [], |r| r.get(0))
        .unwrap()
}

fn open_gate(f: &Fixture, uid: u32) -> Result<CommandResult, ApiError> {
    f.service.handle(
        Command::LocalLaunchGate(Box::new(LocalLaunchGate {
            id: OperationId::new("gate-1"),
            host: HostLocator::parse(CORE_SOCKET).unwrap(),
            action: LocalLaunchGateAction::Open,
        })),
        PeerIdentity::from_kernel(uid),
        &budget(),
    )
}

// Kills: a gate that opens for any peer (the elected-owner uid comparison removed): the stub is a
// core that starts its own targets, so only the uid decides. The owner's Open on the same
// fixture is the control: it records the row.
#[test]
fn local_gate_refuses_another_uid() {
    let f = fixture(EnvHost::shaped("t1", true, true));
    let refused = open_gate(&f, OWNER + 1).expect_err("another uid is not the elected owner");
    assert_eq!(refused.code, ErrorCode::Unauthorized);
    assert_eq!(gate_rows(&f.db), 0, "a refused Open records nothing");

    assert!(matches!(
        open_gate(&f, OWNER).expect("the owner opens the gate"),
        CommandResult::LocalLaunchGate(None)
    ));
    assert_eq!(gate_rows(&f.db), 1);
}

// Kills: a gate that opens on a host that does not start its own targets (the
// `creates_targets_itself` refusal removed): the peer is the owner and the stub answers terminal
// ids, so only that answer decides.
#[test]
fn local_gate_refuses_a_host_that_starts_no_targets() {
    let f = fixture(EnvHost::shaped("t1", true, false));
    let refused = open_gate(&f, OWNER).expect_err("the host starts no targets of its own");
    assert_eq!(refused.code, ErrorCode::Unsupported);
    assert!(
        refused
            .detail
            .contains("this host does not start its own targets"),
        "{}",
        refused.detail
    );
    assert_eq!(gate_rows(&f.db), 0, "a refused Open records nothing");
}

// Kills: `core_port` that hands a non-core session to a terminal candidate (the
// `answers_terminal_ids` refusal removed): the stub owns `t1`, and without the guard its root
// (pid 1) is an ancestor of the subject, so the candidate would verify.
#[test]
fn locate_caller_refuses_a_non_core_socket() {
    let stub = EnvHost::shaped("t1", false, false);
    let f = fixture(Arc::clone(&stub));
    let CommandResult::Located(reply) = f
        .service
        .handle(
            Command::LocateCaller(LocateCaller {
                candidates: vec![LocateCandidate::Terminal {
                    socket: CORE_SOCKET.into(),
                    id: "t1".into(),
                }],
                subject: Some(LivenessFingerprint { pid: 30, start: 1 }),
            }),
            PeerIdentity::from_kernel(OWNER),
            &budget(),
        )
        .unwrap()
    else {
        panic!("LocateCaller answers Located");
    };
    assert_eq!(
        reply,
        LocateReply::Unverified {
            per_candidate: vec![CandidateVerdict::Unsupported]
        }
    );
    assert_eq!(stub.root_calls(), 0, "a non-core host is never asked");
}

fn claim(role: CallerRole, liveness: Option<LivenessFingerprint>) -> CallerClaim {
    CallerClaim {
        instance: INSTANCE.into(),
        host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("sess"),
        execution: ExecutionId::new("e"),
        target: HostTargetId::new("alice"),
        agent_id: None,
        liveness,
    }
}

fn turn_state(
    f: &Fixture,
    claim: CallerClaim,
    hint: Option<&str>,
    peer_pid: u32,
) -> Result<CommandResult, ApiError> {
    f.service.handle(
        Command::TurnState(TurnStateReport {
            claim,
            edge: crate::harness::adapter::TurnEdge::Busy,
            tmux_hint: hint.map(str::to_owned),
        }),
        PeerIdentity::from_kernel_with_pid(OWNER, Some(peer_pid)),
        &budget(),
    )
}

// Kills: a `TurnState` handler that drops the hint (no `note_wake_hint`), one that verifies it
// for the reporting hook's pid instead of the harness the claim fingerprints, and one that lets
// a subagent claim hold a wake.
#[test]
fn turn_state_hands_the_hint_to_the_host_for_the_harness_process() {
    let stub = EnvHost::shaped("t1", false, false);
    let f = fixture(Arc::clone(&stub));
    let hint = "tmux:/sock:1:%1";
    let harness = LivenessFingerprint {
        pid: 4242,
        start: 9,
    };

    turn_state(
        &f,
        claim(CallerRole::TopLevel, Some(harness)),
        Some(hint),
        77,
    )
    .expect("a top-level turn edge");
    assert_eq!(
        stub.hints(),
        vec![("alice".to_owned(), 4242, hint.to_owned())],
        "the harness process, not the hook (77)"
    );

    turn_state(
        &f,
        claim(CallerRole::Subagent, Some(harness)),
        Some(hint),
        77,
    )
    .expect("a subagent turn edge");
    assert_eq!(stub.hints().len(), 1, "a subagent claim holds no wake");

    turn_state(&f, claim(CallerRole::TopLevel, Some(harness)), None, 77)
        .expect("a turn edge without a hint");
    assert_eq!(stub.hints().len(), 1, "no hint, nothing handed over");
}
