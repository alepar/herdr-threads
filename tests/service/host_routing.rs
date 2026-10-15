//! ht-5fr.2.12: the domain service routes every command that names a Herdr
//! server through its registry (`DomainService::attached_host`): the server
//! attaches lazily, within the request, before any store call, and only for
//! the elected owner's peer. Included from `src/service/dispatch.rs`; the
//! daemon-level flows are `tests/daemon/multi_host.rs`.
use super::*;
use crate::{
    app::SystemClock,
    ports::{LocalService, ModChannelSink},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint, PeerIdentity},
        commands::{AdvanceCursors, Command},
        ids::{ExecutionId, HostLocator, HostTargetId, NativeSessionId, OperationId, SeatId},
        results::ErrorCode,
        time::{CallBudget, Cancellation, MonoInstant},
        watch::{WatchFrame, WatchRefusalReason, WatchRequest},
    },
    service::host_sessions::SessionDeps,
    store::{SqliteStore, StoreSettings, connection::StoreContext, hosts::HostIndex},
    test_support::{FakeHostWorld, RefusingHost, isolation::TestIsolation},
};

struct NoSink;
impl ModChannelSink for NoSink {
    fn push(&self, _: WatchFrame) -> bool {
        true
    }
}

struct Rig {
    service: DomainService,
    sessions: Arc<HostSessions>,
    world: Arc<FakeHostWorld>,
    node: String,
    clock: Arc<dyn Clock>,
    db: std::path::PathBuf,
    server: HostLocator,
    _isolation: TestIsolation,
}

impl Rig {
    fn new(label: &str) -> Self {
        let isolation = TestIsolation::new(label);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let node = crate::node::NodeUuid::new_v4().to_string();
        let index = Arc::new(HostIndex::new());
        let db = isolation.path("store.db");
        let store: Arc<dyn StorePort> = Arc::new(
            SqliteStore::new(
                StoreContext::with_host_index(db.clone(), Arc::clone(&clock), Arc::clone(&index))
                    .with_node_seed(crate::test_support::node_seed_for(&node)),
                node.clone(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let world = FakeHostWorld::new();
        let sessions = Arc::new(HostSessions::new(
            index,
            world.plan(None).factory,
            SessionDeps::detached(Arc::clone(&store), Arc::clone(&clock), node.clone()),
        ));
        let service = DomainService::new(node.clone(), store, Arc::clone(&clock))
            .with_sessions(Arc::clone(&sessions))
            .with_operator_owner(501)
            .with_cooperative_owner(501, Arc::new(FairWriter::new(8)));
        let server =
            HostLocator::parse(isolation.socket_path("server.sock").to_string_lossy()).unwrap();
        Self {
            service,
            sessions,
            world,
            node,
            clock,
            db,
            server,
            _isolation: isolation,
        }
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 10_000),
            cancellation: Cancellation::default(),
        }
    }

    /// A claim of a seat that does not exist, from a pane of `self.server`.
    fn claim(&self, harness: Harness) -> CallerClaim {
        CallerClaim {
            instance: self.node.clone(),
            host: self.server.clone(),
            seat: SeatId::new("no-such-seat"),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness,
            native_session: NativeSessionId::new("session"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
            target: HostTargetId::new("pane-1"),
            agent_id: None,
            liveness: Some(LivenessFingerprint::TEST),
        }
    }

    fn advance(&self, peer_uid: u32) -> Result<CommandResult, ApiError> {
        self.advance_claim(peer_uid, self.claim(Harness::Codex))
    }

    fn advance_claim(&self, peer_uid: u32, claim: CallerClaim) -> Result<CommandResult, ApiError> {
        self.service.handle(
            Command::AdvanceCursors(AdvanceCursors {
                claim,
                operation: OperationId::new("advance"),
                displayed: Vec::new(),
                in_turn: false,
                invitations: Vec::new(),
            }),
            PeerIdentity::from_kernel(peer_uid),
            &self.budget(),
        )
    }

    fn host_rows(&self) -> i64 {
        rusqlite::Connection::open(&self.db)
            .unwrap()
            .query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
            .unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.sessions.stop_all();
    }
}

// Kills: a service that asks the store first and attaches never: the claim of
// a server that exists attaches it, and the store's own refusal (the seat is
// unknown) is what comes back.
#[test]
fn a_claim_attaches_its_server_before_the_store_is_asked() {
    let rig = Rig::new("hr-attach");
    rig.world.add(&rig.server, Arc::new(RefusingHost));
    assert!(rig.sessions.by_locator(&rig.server).is_none());
    let error = rig.advance(501).unwrap_err();
    assert_ne!(error.code, ErrorCode::HostUnavailable, "{error:?}");
    assert!(rig.sessions.by_locator(&rig.server).is_some());
    assert_eq!(rig.host_rows(), 1, "attached once, row written");
}

// Kills: a claim naming a server that is not there reaching the store, or
// leaving a host row or session behind.
#[test]
fn a_claim_naming_a_missing_server_is_host_unavailable_and_attaches_nothing() {
    let rig = Rig::new("hr-missing");
    let error = rig.advance(501).unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert_eq!(
        error.detail,
        format!("Herdr server {} not reachable", rig.server.as_str())
    );
    assert_eq!(rig.sessions.attached_count(), 0);
    assert_eq!(rig.host_rows(), 0);
}

// Kills: any peer being able to make the daemon attach a server: only the
// elected owner's peer attaches, another peer is refused by its route.
#[test]
fn only_the_elected_owners_peer_attaches() {
    let rig = Rig::new("hr-peer");
    rig.world.add(&rig.server, Arc::new(RefusingHost));
    let error = rig.advance(502).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unauthorized, "{error:?}");
    assert_eq!(rig.sessions.attached_count(), 0);
}

// Kills: a claim of another node attaching a server before it is refused: it
// is refused for its instance and nothing attaches.
#[test]
fn a_claim_of_another_node_attaches_nothing() {
    let rig = Rig::new("hr-instance");
    rig.world.add(&rig.server, Arc::new(RefusingHost));
    let mut claim = rig.claim(Harness::Codex);
    claim.instance = "another-node".into();
    let error = rig.advance_claim(501, claim).unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified, "{error:?}");
    assert_eq!(rig.sessions.attached_count(), 0);
}

// Kills: a watch registration that never attaches the server its claim names
// (it would be refused until some other call happened to attach it), or one
// that attaches a server that is not there.
#[test]
fn watch_register_attaches_the_claim_host() {
    let rig = Rig::new("hr-watch");
    let request = WatchRequest {
        claim: rig.claim(Harness::Claude),
        presentation_v2: false,
    };
    let sink: Arc<dyn ModChannelSink> = Arc::new(NoSink);
    // Not there: retryable, nothing attached.
    let refused = rig
        .service
        .watch_register(&request, Arc::clone(&sink), &rig.budget())
        .unwrap_err();
    assert_eq!(refused, WatchRefusalReason::Busy);
    assert_eq!(rig.sessions.attached_count(), 0);
    // There: attached by the registration, which then fails on the unknown seat.
    rig.world.add(&rig.server, Arc::new(RefusingHost));
    let refused = rig
        .service
        .watch_register(&request, sink, &rig.budget())
        .unwrap_err();
    assert_ne!(refused, WatchRefusalReason::Busy, "{refused:?}");
    assert!(rig.sessions.by_locator(&rig.server).is_some());
}
