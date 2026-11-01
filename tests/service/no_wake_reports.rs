//! ht-kqrt.31 (included from `src/service/dispatch.rs`: it drives crate-private constructors): persisted no-wake reports. `wake_status` reports a seat whose host has no wake
//! path (`wake=no_wake`) once per (sender, seat) across daemon restarts and mod channel flaps;
//! a `Composer` read or the retirement of either seat ends the episode.
use super::*;
use crate::{
    app::SystemClock,
    host::named::{NamedHost, NoBootstrap},
    ports::*,
    protocol::{
        authority::PeerIdentity,
        commands::{Command, OperatorRetire, ResolveSeat, WakeStatusQuery},
        ids::*,
        results::{CommandResult, SeatWakeStatus, WakeBlock},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::host_sessions::{HostSessions, SessionDeps},
    store::{SqliteStore, StoreSettings, connection::StoreContext, hosts::HostIndex},
    test_support::{FakeHostWorld, RefusingHost, isolation::TestIsolation},
};
use std::sync::atomic::{AtomicBool, Ordering};

const UID: u32 = 501;

/// A mod channel registry whose only answer is whether every seat's channel is live.
#[derive(Default)]
struct Flap(AtomicBool);
impl ModChannels for Flap {
    fn wake_suppressed(&self, _: &SeatId) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A composer-level host (default `Composer` wake capability) whose answer for the target
/// named `"target"` is `None` while the flag is set: a per-target wake path that comes and goes.
struct PerTargetWake(Arc<AtomicBool>);
impl HostPort for PerTargetWake {
    fn target_wake_capability(&self, target: &HostTargetId) -> WakeCapability {
        if target.as_str() == "target" && self.0.load(Ordering::SeqCst) {
            WakeCapability::None
        } else {
            self.wake_capability()
        }
    }
    fn live_topology(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        RefusingHost.live_topology(budget)
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        RefusingHost.observe_current_target(target, context)
    }
    fn observe_current_target_for_archival(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        RefusingHost.observe_current_target_for_archival(target, context)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        RefusingHost.enumerate_targets(context)
    }
    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        RefusingHost.submit_prompt(target, text, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        RefusingHost.launch_native(request, context)
    }
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        RefusingHost.pane_agent_state(target, context)
    }
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError> {
        RefusingHost.send_submit_key(target, context)
    }
}

/// One daemon incarnation over the rig's store file.
struct Boot {
    service: DomainService,
    sessions: Arc<HostSessions>,
}

pub(super) struct Rig {
    isolation: TestIsolation,
    node: String,
    clock: Arc<dyn Clock>,
    channel: Arc<Flap>,
    /// Set: the composer host answers `None` for the target `"target"`.
    target_none: Arc<AtomicBool>,
    boot: Option<Boot>,
}

impl Rig {
    pub(super) fn new(label: &str) -> Self {
        let mut rig = Self {
            isolation: TestIsolation::new(label),
            node: uuid::Uuid::new_v4().simple().to_string(),
            clock: Arc::new(SystemClock::new()),
            channel: Arc::new(Flap::default()),
            target_none: Arc::new(AtomicBool::new(false)),
            boot: None,
        };
        rig.restart();
        rig
    }

    fn db(&self) -> std::path::PathBuf {
        self.isolation.path("store.db")
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 10_000),
            cancellation: Cancellation::default(),
        }
    }

    /// Stops the daemon and starts a fresh one on the same store: in-memory state is gone.
    fn restart(&mut self) {
        if let Some(old) = self.boot.take() {
            old.sessions.stop_all();
        }
        let index = Arc::new(HostIndex::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::with_host_index(
                    self.db(),
                    Arc::clone(&self.clock),
                    Arc::clone(&index),
                )
                .with_node_seed(crate::test_support::node_seed_for(&self.node)),
                self.node.clone(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let world = FakeHostWorld::new();
        let sessions = Arc::new(HostSessions::new(
            index,
            world.plan(None).factory,
            SessionDeps::detached(store.clone(), Arc::clone(&self.clock), self.node.clone()),
        ));
        let named = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
        sessions
            .attach_builtin(
                Arc::new(NamedHost::new(
                    &self.node,
                    HostId::from_row(0),
                    store.clone(),
                    Arc::clone(&self.clock),
                )),
                Arc::new(NoBootstrap),
                &named,
            )
            .unwrap();
        let composer =
            HostLocator::parse(self.isolation.socket_path("herdr.sock").to_string_lossy()).unwrap();
        world.add(
            &composer,
            Arc::new(PerTargetWake(Arc::clone(&self.target_none))),
        );
        sessions.resolve(&composer, &self.budget()).unwrap();
        let service = DomainService::new(self.node.clone(), store, Arc::clone(&self.clock))
            .with_sessions(Arc::clone(&sessions))
            .with_mod_channels(self.channel.clone())
            .with_operator_owner(UID)
            .with_cooperative_owner(UID, Arc::new(FairWriter::new(8)));
        self.boot = Some(Boot { service, sessions });
    }

    fn call(&self, command: Command) -> CommandResult {
        self.boot
            .as_ref()
            .unwrap()
            .service
            .handle(command, PeerIdentity::from_kernel(UID), &self.budget())
            .unwrap()
    }

    pub(super) fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db()).unwrap()
    }

    /// A named seat with an open binding of `harness`.
    pub(super) fn named_seat(&self, name: &str, harness: &str) -> SeatId {
        let CommandResult::NamedSeatResolved { seat, .. } =
            self.call(Command::ResolveSeat(ResolveSeat {
                target: HostTargetId::new(name),
                host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
                operation: OperationId::new(format!("resolve-{name}")),
                locator_hint: None,
                tmux_hint: None,
            }))
        else {
            panic!("named resolve")
        };
        self.sql()
            .execute(
                "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,?3,'session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term','inc')",
                rusqlite::params![seat.as_str(), name, harness],
            )
            .unwrap();
        seat
    }

    pub(super) fn status(&self, observer: &SeatId, seat: &SeatId) -> SeatWakeStatus {
        let CommandResult::WakeStatus(mut rows) = self.call(Command::WakeStatus(WakeStatusQuery {
            all: false,
            seats: vec![seat.clone()],
            message: None,
            observer: Some(observer.clone()),
        })) else {
            panic!("wake status")
        };
        assert_eq!(rows.len(), 1);
        rows.remove(0)
    }

    /// Every open binding of `seat` ends (the process exited).
    pub(super) fn end_bindings(&self, seat: &SeatId) {
        self.sql()
            .execute(
                "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id=?1 AND ended_at IS NULL",
                [seat.as_str()],
            )
            .unwrap();
    }

    /// `doctor`'s read: the wake status of every seat with an open binding.
    pub(super) fn status_all(&self) -> Vec<SeatWakeStatus> {
        let CommandResult::WakeStatus(rows) = self.call(Command::WakeStatus(WakeStatusQuery {
            all: true,
            seats: Vec::new(),
            message: None,
            observer: None,
        })) else {
            panic!("wake status")
        };
        rows
    }

    fn reports(&self) -> Vec<(String, String)> {
        let db = self.sql();
        let mut statement = db
            .prepare("SELECT observer,seat_id FROM no_wake_reports ORDER BY observer,seat_id")
            .unwrap();
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(boot) = &self.boot {
            boot.sessions.stop_all();
        }
    }
}

// Kills: reporting again after a daemon restart or a channel flap (the in-memory set's
// behaviour), or sharing one report between two senders.
#[test]
fn one_no_wake_per_sender_seat_across_restart_and_channel_flap() {
    let mut rig = Rig::new("nwr-once");
    let (sender, other) = (
        rig.named_seat("sender", "codex"),
        rig.named_seat("other", "codex"),
    );
    let target = rig.named_seat("target", "claude");
    let first = rig.status(&sender, &target);
    assert_eq!((first.wake, first.first), (Some(WakeBlock::NoWake), true));
    assert!(!rig.status(&sender, &target).first, "second read is silent");
    // Channel flap: live (no block), then gone again.
    rig.channel.0.store(true, Ordering::SeqCst);
    let live = rig.status(&sender, &target);
    assert_eq!((live.wake, live.first), (None, false));
    rig.channel.0.store(false, Ordering::SeqCst);
    let back = rig.status(&sender, &target);
    assert_eq!((back.wake, back.first), (Some(WakeBlock::NoWake), false));
    // Daemon restart.
    rig.restart();
    let after = rig.status(&sender, &target);
    assert_eq!((after.wake, after.first), (Some(WakeBlock::NoWake), false));
    // Another sender is told once on its own.
    assert!(rig.status(&other, &target).first);
    assert!(!rig.status(&other, &target).first);
    assert_eq!(rig.reports().len(), 2);
}

// Kills: classifying hooks-only Claude on a host with no wake path as wakeable.
#[test]
fn hooks_only_claude_named_seat_is_none() {
    let rig = Rig::new("nwr-hooks");
    let (sender, target) = (rig.named_seat("s", "codex"), rig.named_seat("t", "claude"));
    let row = rig.status(&sender, &target);
    assert_eq!(row.wake, Some(WakeBlock::NoWake));
    assert_eq!(
        rig.reports(),
        vec![(sender.as_str().to_owned(), target.as_str().to_owned())]
    );
}

// Kills: reporting (or persisting) a no-wake for a Claude named seat whose live mod channel
// wakes it.
#[test]
fn live_mod_claude_named_seat_reports_no_block() {
    let rig = Rig::new("nwr-live");
    rig.channel.0.store(true, Ordering::SeqCst);
    let (sender, target) = (rig.named_seat("s", "codex"), rig.named_seat("t", "claude"));
    let row = rig.status(&sender, &target);
    assert_eq!((row.wake, row.first), (None, false));
    assert!(rig.reports().is_empty());
}

// Kills: a row surviving the retirement of the reported seat or of the observer.
#[test]
fn row_deleted_on_retire_of_either_seat() {
    let rig = Rig::new("nwr-retire");
    let observer = rig.named_seat("obs", "codex");
    let (a, b) = (rig.named_seat("a", "claude"), rig.named_seat("b", "claude"));
    rig.status(&observer, &a);
    rig.status(&observer, &b);
    assert_eq!(rig.reports().len(), 2);
    let retire = |seat: &SeatId| {
        rig.call(Command::OperatorRetire(OperatorRetire {
            seat: seat.clone(),
            operation: OperationId::new(format!("retire-{}", seat.as_str())),
        }));
    };
    retire(&a);
    assert_eq!(
        rig.reports(),
        vec![(observer.as_str().to_owned(), b.as_str().to_owned())]
    );
    retire(&observer);
    assert!(rig.reports().is_empty());
}

// Kills: keeping the row after a read finds the seat on a Composer host (a later None episode
// would then never re-report), or never writing a fresh row for the new episode.
#[test]
fn row_deleted_when_read_finds_composer() {
    let rig = Rig::new("nwr-composer");
    let (sender, target) = (rig.named_seat("s", "codex"), rig.named_seat("t", "claude"));
    assert!(rig.status(&sender, &target).first);
    assert_eq!(rig.reports().len(), 1);
    let composer_host: i64 = rig
        .sql()
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind='herdr'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let named_host: i64 = rig
        .sql()
        .query_row(
            "SELECT host_id FROM seats WHERE id=?1",
            [target.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let move_to = |host: i64| {
        rig.sql()
            .execute(
                "UPDATE seats SET host_id=?1 WHERE id=?2",
                rusqlite::params![host, target.as_str()],
            )
            .unwrap();
    };
    move_to(composer_host);
    let row = rig.status(&sender, &target);
    assert_eq!((row.wake, row.first), (None, false));
    assert!(rig.reports().is_empty(), "a Composer read ends the episode");
    move_to(named_host);
    assert!(
        rig.status(&sender, &target).first,
        "a new episode reports again"
    );
}

// Kills: reading only the host-level answer (a Composer host whose one target has no wake path
// never reports), or keeping the row across a Composer read (the second None episode would stay
// silent), or reporting again within one episode.
#[test]
fn per_target_none_composer_none_reports_once_per_episode() {
    let rig = Rig::new("nwr-per-target");
    let (sender, target) = (
        rig.named_seat("s", "codex"),
        rig.named_seat("target", "claude"),
    );
    let composer_host: i64 = rig
        .sql()
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind='herdr'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    rig.sql()
        .execute(
            "UPDATE seats SET host_id=?1 WHERE id=?2",
            rusqlite::params![composer_host, target.as_str()],
        )
        .unwrap();
    let read = |none: bool| {
        rig.target_none.store(none, Ordering::SeqCst);
        let row = rig.status(&sender, &target);
        (row.wake, row.first)
    };
    assert_eq!(read(false), (None, false), "a Composer answer is no block");
    assert!(rig.reports().is_empty());
    assert_eq!(
        read(true),
        (Some(WakeBlock::NoWake), true),
        "episode 1 reports"
    );
    assert_eq!(
        read(true),
        (Some(WakeBlock::NoWake), false),
        "once per episode"
    );
    assert_eq!(rig.reports().len(), 1);
    assert_eq!(read(false), (None, false));
    assert!(rig.reports().is_empty(), "a Composer read ends the episode");
    assert_eq!(
        read(true),
        (Some(WakeBlock::NoWake), true),
        "episode 2 reports"
    );
    assert_eq!(read(true), (Some(WakeBlock::NoWake), false));
}
