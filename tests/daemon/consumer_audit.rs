//! The 07 §5.3 consumer audit: each consumer of `HostSessions::attached()` and
//! `all_for_reads()` chooses per call site, tested here with one dialled fake
//! plus one built-in (named-shaped) fake.

use crate::{
    daemon::idle_exit::dialled_hosts_active,
    ports::{
        AgentComposerState, ArchivalEvidence, BootstrapObserver, BootstrapPaneObservation,
        ClosureAuthority, ComposerObservation, HostCallContext, HostKind, HostObservation,
        HostPort, HostSnapshot, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
        PromptOutcome, SafeWakeTarget, StorePort, WakeCapability,
    },
    protocol::{
        ids::{HostLocator, HostTargetId, SeatId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{
        archival::ArchivalHosts,
        dispatch::{no_host_attached, operator_fallback_host},
        host_sessions::{HostSession, HostSessions, SessionDeps},
        peer_launcher::creation_hosts,
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext, hosts::HostIndex},
    test_support::{fake_hosts::FakeHostWorld, isolation::TestIsolation},
};
use std::sync::Arc;

/// A fixture host with configurable answers to the policy questions.
#[derive(Clone, Copy)]
pub(super) struct AuditHost {
    kind: HostKind,
    wake: bool,
    archival: ArchivalEvidence,
    closure: ClosureAuthority,
    launch: NativeLaunchCapability,
}

impl AuditHost {
    /// A dialled Herdr-shaped host: wakes, has evidence, closes, guarded start.
    pub(super) const HERDR: AuditHost = AuditHost {
        kind: HostKind::Herdr,
        wake: true,
        archival: ArchivalEvidence::Supported,
        closure: ClosureAuthority::Closing,
        launch: NativeLaunchCapability::HostGuardedStart,
    };
    /// The named-host shape: no wake, no evidence, non-closing, no launch.
    const NAMED: AuditHost = AuditHost {
        kind: HostKind::Herdr,
        wake: false,
        archival: ArchivalEvidence::NoEvidence,
        closure: ClosureAuthority::NonClosing,
        launch: NativeLaunchCapability::Unsupported,
    };
}

fn refused<T>() -> Result<T, ApiError> {
    Err(ApiError::unsupported("audit fixture host"))
}

impl HostPort for AuditHost {
    fn kind(&self) -> HostKind {
        self.kind
    }
    fn wake_capability(&self) -> WakeCapability {
        if self.wake {
            WakeCapability::Composer(crate::ports::ComposerTraits::HERDR_TUI)
        } else {
            WakeCapability::None
        }
    }
    fn archival_evidence(&self) -> ArchivalEvidence {
        self.archival
    }
    fn closure_authority(&self) -> ClosureAuthority {
        self.closure
    }
    fn live_topology(
        &self,
        _: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        refused()
    }
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        self.launch
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        refused()
    }
    fn observe_current_target_for_archival(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        refused()
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        refused()
    }
    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        refused()
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        refused()
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        refused()
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        refused()
    }
}

struct NoObserver;
impl BootstrapObserver for NoObserver {
    fn observe_bootstrap_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<BootstrapPaneObservation, ApiError> {
        refused()
    }
}

fn loc(text: &str) -> HostLocator {
    HostLocator::parse(text).unwrap()
}

pub(super) struct Rig {
    _isolation: TestIsolation,
    world: Arc<FakeHostWorld>,
    pub(super) sessions: Arc<HostSessions>,
    store: Arc<SqliteStore>,
}

impl Rig {
    pub(super) fn new(label: &str) -> Self {
        let isolation = TestIsolation::new(label);
        let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::default());
        let instance = crate::node::NodeUuid::new_v4().to_string();
        let index = Arc::new(HostIndex::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::with_host_index(
                    isolation.path("store.db"),
                    Arc::clone(&clock),
                    Arc::clone(&index),
                )
                .with_node_seed(crate::test_support::node_seed_for(&instance)),
                instance.clone(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let world = FakeHostWorld::new();
        let deps = SessionDeps::detached(Arc::clone(&store) as Arc<dyn StorePort>, clock, instance);
        let sessions = Arc::new(HostSessions::new(index, world.plan(None).factory, deps));
        Self {
            _isolation: isolation,
            world,
            sessions,
            store,
        }
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.store.clock().monotonic_now().0 + 10_000),
            cancellation: Cancellation::default(),
        }
    }

    /// Attaches a dialled fake at `locator`.
    pub(super) fn dial(&self, locator: &str, host: AuditHost) -> Arc<HostSession> {
        let locator = loc(locator);
        self.world.add(&locator, Arc::new(host));
        self.sessions.resolve(&locator, &self.budget()).unwrap()
    }

    /// Attaches `port` as the built-in host.
    pub(super) fn builtin_port(&self, port: Arc<dyn HostPort>) -> Arc<HostSession> {
        self.sessions
            .attach_builtin(port, Arc::new(NoObserver), &loc("/builtin/named"))
            .unwrap()
    }

    /// Attaches the built-in fake.
    fn builtin(&self, host: AuditHost) -> Arc<HostSession> {
        self.sessions
            .attach_builtin(Arc::new(host), Arc::new(NoObserver), &loc("/builtin/named"))
            .unwrap()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.sessions.stop_all();
    }
}

// Kills: an idle-exit input that counts the built-in session, so a daemon
// holding only the named host never exits.
#[test]
fn idle_exit_with_only_builtin_session_exits() {
    let rig = Rig::new("audit-idle");
    rig.builtin(AuditHost::NAMED);
    assert_eq!(rig.sessions.all_for_reads().len(), 1);
    assert!(
        !dialled_hosts_active(&rig.sessions),
        "a built-in host alone keeps the daemon up"
    );
    rig.dial("/herdr/a.sock", AuditHost::HERDR);
    assert!(
        dialled_hosts_active(&rig.sessions),
        "a dialled host keeps it up"
    );
}

// Kills: archival sampling a host that has no evidence (it would be read as
// incoherent forever) or the built-in one.
#[test]
fn archival_skips_no_evidence_host() {
    let rig = Rig::new("audit-archival");
    let with = rig.dial("/herdr/a.sock", AuditHost::HERDR);
    let without = rig.dial(
        "/herdr/b.sock",
        AuditHost {
            archival: ArchivalEvidence::NoEvidence,
            ..AuditHost::HERDR
        },
    );
    rig.builtin(AuditHost::HERDR);
    let listed: Vec<_> = ArchivalHosts::attached(&*rig.sessions)
        .iter()
        .map(|host| host.host_id)
        .collect();
    assert_eq!(listed, vec![with.host_id()]);
    assert!(ArchivalHosts::get(&*rig.sessions, with.host_id()).is_some());
    assert!(ArchivalHosts::get(&*rig.sessions, without.host_id()).is_none());
}

// Kills: `operator_host` ignoring the built-in host, or failing with another text.
#[test]
fn operator_host_falls_back_to_builtin() {
    let rig = Rig::new("audit-operator");
    let builtin = rig.builtin(AuditHost::NAMED);
    let chosen = operator_fallback_host(&rig.sessions).unwrap();
    assert!(Arc::ptr_eq(&chosen, &builtin));
    let dialled = rig.dial("/herdr/a.sock", AuditHost::HERDR);
    let chosen = operator_fallback_host(&rig.sessions).unwrap();
    assert!(Arc::ptr_eq(&chosen, &dialled), "dialled first");
}

#[test]
fn operator_host_none_prints_no_host_attached() {
    let rig = Rig::new("audit-operator-none");
    let error = operator_fallback_host(&rig.sessions).err().unwrap();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert_eq!(error.detail, "no host attached");
    assert_eq!(no_host_attached().detail, "no host attached");
}

// Kills: a discovery gate that opens for a dialled session of another kind.
#[test]
fn any_reachable_ignores_non_herdr_kind() {
    let rig = Rig::new("audit-reachable");
    rig.builtin(AuditHost::HERDR);
    rig.dial(
        "/herdr/a.sock",
        AuditHost {
            kind: HostKind::Unknown,
            ..AuditHost::HERDR
        },
    );
    assert!(!rig.sessions.any_reachable());
    rig.dial("/herdr/b.sock", AuditHost::HERDR);
    assert!(rig.sessions.any_reachable());
}

// Kills: placing a peer launch on a host that cannot create a guarded target,
// or on the built-in host.
#[test]
fn place_ignores_host_without_creation_capability() {
    let rig = Rig::new("audit-place");
    rig.builtin(AuditHost::HERDR);
    rig.dial("/herdr/a.sock", AuditHost::NAMED);
    assert!(creation_hosts(&rig.sessions).is_empty());
    let capable = rig.dial("/herdr/b.sock", AuditHost::HERDR);
    let found = creation_hosts(&rig.sessions);
    assert_eq!(found.len(), 1);
    assert!(Arc::ptr_eq(&found[0], &capable));
}

// Kills: a startup primary that falls back to the lowest dialled host (or the
// built-in one) without a guarded-start launch capability.
#[test]
fn startup_primary_picks_only_host_guarded_start_dialled_host() {
    let rig = Rig::new("audit-primary");
    rig.builtin(AuditHost::HERDR);
    rig.dial("/herdr/a.sock", AuditHost::NAMED);
    assert!(rig.sessions.startup_primary(None).is_none());
    let capable = rig.dial("/herdr/b.sock", AuditHost::HERDR);
    let primary = rig.sessions.startup_primary(None).unwrap();
    assert!(Arc::ptr_eq(&primary, &capable));
}
