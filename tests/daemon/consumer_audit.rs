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
    /// `creates_targets_itself()`: a herdr-core shape.
    creates: bool,
    /// The one space (id and label) its topology lists, if any.
    space: Option<&'static str>,
}

impl AuditHost {
    /// A dialled Herdr-shaped host: wakes, has evidence, closes, guarded start.
    pub(super) const HERDR: AuditHost = AuditHost {
        kind: HostKind::Herdr,
        wake: true,
        archival: ArchivalEvidence::Supported,
        closure: ClosureAuthority::Closing,
        launch: NativeLaunchCapability::HostGuardedStart,
        creates: false,
        space: None,
    };
    /// A dialled herdr-core shape: no wake, no evidence, closing, creates its own targets.
    pub(super) const HC: AuditHost = AuditHost {
        kind: HostKind::Hc,
        wake: false,
        archival: ArchivalEvidence::NoEvidence,
        closure: ClosureAuthority::Closing,
        launch: NativeLaunchCapability::Unsupported,
        creates: true,
        space: None,
    };
    /// The named-host shape: no wake, no evidence, non-closing, no launch.
    /// The same host listing `space` as its one space.
    pub(super) const fn with_space(self, space: &'static str) -> AuditHost {
        AuditHost {
            space: Some(space),
            ..self
        }
    }

    const NAMED: AuditHost = AuditHost {
        kind: HostKind::Herdr,
        wake: false,
        archival: ArchivalEvidence::NoEvidence,
        closure: ClosureAuthority::NonClosing,
        launch: NativeLaunchCapability::Unsupported,
        creates: false,
        space: None,
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
    fn creates_targets_itself(&self) -> bool {
        self.creates
    }
    fn live_topology(
        &self,
        _: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        use crate::host::observation::{HostTopology, TopologySpace};
        match self.space {
            Some(space) => Ok(HostTopology {
                spaces: vec![TopologySpace {
                    id: space.to_owned(),
                    label: Some(space.to_owned()),
                }],
                tabs: Vec::new(),
                panes: Vec::new(),
            }),
            None => refused(),
        }
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
    pub(super) store: Arc<SqliteStore>,
    pub(super) clock: Arc<dyn Clock>,
    /// The dialled hosts that hold a resolved local seat (what `SessionDeps::seated` answers).
    seated: Arc<std::sync::Mutex<std::collections::BTreeSet<crate::ports::HostId>>>,
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
        let mut deps = SessionDeps::detached(
            Arc::clone(&store) as Arc<dyn StorePort>,
            Arc::clone(&clock),
            instance,
        );
        let seated = Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let reading = Arc::clone(&seated);
        deps.seated = Arc::new(move |_| Ok(reading.lock().unwrap().clone()));
        let sessions = Arc::new(HostSessions::new(index, world.plan(None).factory, deps));
        Self {
            _isolation: isolation,
            world,
            sessions,
            store,
            clock,
            seated,
        }
    }

    /// A new directory `name` under the rig's scratch root.
    pub(super) fn dir(&self, name: &str) -> std::path::PathBuf {
        let path = self._isolation.path(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A second connection to the rig's store file, for fixture rows.
    pub(super) fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(self._isolation.path("store.db")).unwrap();
        db.pragma_update(None, "foreign_keys", "ON").unwrap();
        db
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.store.clock().monotonic_now().0 + 10_000),
            cancellation: Cancellation::default(),
        }
    }

    /// Attaches a dialled fake at `locator` that holds a resolved local seat.
    pub(super) fn dial(&self, locator: &str, host: AuditHost) -> Arc<HostSession> {
        let session = self.dial_seatless(locator, host);
        self.seated.lock().unwrap().insert(session.host_id());
        session
    }

    /// Attaches a dialled fake at `locator` that holds no seat.
    pub(super) fn dial_seatless(&self, locator: &str, host: AuditHost) -> Arc<HostSession> {
        let locator = loc(locator);
        self.world.add(&locator, Arc::new(host));
        self.sessions.resolve(&locator, &self.budget()).unwrap()
    }

    /// Attaches `port` as the built-in host.
    pub(super) fn builtin_port(&self, port: Arc<dyn HostPort>) -> Arc<HostSession> {
        let kind = port.kind();
        self.sessions
            .attach_builtin(kind, port, Arc::new(NoObserver), &loc("/builtin/named"))
            .unwrap()
    }

    /// Attaches the built-in fake.
    fn builtin(&self, host: AuditHost) -> Arc<HostSession> {
        self.sessions
            .attach_builtin(
                host.kind,
                Arc::new(host),
                Arc::new(NoObserver),
                &loc("/builtin/named"),
            )
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

// Kills: a hinted built-in or core session leading startup unfiltered, so the advertised
// guarded capabilities and Health follow a host that cannot run them until restart.
#[test]
fn startup_primary_refuses_a_reserved_or_core_hint() {
    let rig = Rig::new("audit-primary-hint");
    let reserved = loc("/builtin/named");
    rig.builtin(AuditHost::NAMED);
    let core = rig.dial("/hc/core.sock", AuditHost::HC);
    assert!(rig.sessions.startup_primary(Some(&reserved)).is_none());
    assert!(rig.sessions.startup_primary(Some(core.locator())).is_none());
    let herdr = rig.dial("/herdr/a.sock", AuditHost::HERDR);
    for hint in [&reserved, core.locator()] {
        let primary = rig.sessions.startup_primary(Some(hint)).unwrap();
        assert!(
            Arc::ptr_eq(&primary, &herdr),
            "the guarded-start host leads"
        );
    }
}

// Kills: Health following the evidence of whichever session led startup, frozen at composition:
// a reserved or core hint (or no host at all) must leave the fallback, and a guarded-start host
// attached later must be reported from then on.
#[test]
fn elected_health_evidence_follows_the_guarded_host_attached_later() {
    use crate::{app::ElectedHostEvidence, protocol::results::CapabilityState};
    let rig = Rig::new("audit-health-evidence");
    let fallback = Arc::new(crate::service::host_evidence::HostEvidenceStatus::default());
    rig.builtin(AuditHost::NAMED);
    let core = rig.dial("/hc/core.sock", AuditHost::HC);
    let evidence = |hint: Option<HostLocator>| {
        ElectedHostEvidence::of_sessions(
            &rig.sessions,
            hint,
            Arc::clone(&fallback),
            Default::default(),
        )
    };
    let reserved = evidence(Some(loc("/builtin/named")));
    let hinted_core = evidence(Some(core.locator().clone()));
    for evidence in [&reserved, &hinted_core] {
        let (status, witness, safe_prompt) = evidence.current();
        assert!(Arc::ptr_eq(&status, &fallback));
        assert_eq!(witness, CapabilityState::Unknown);
        assert_eq!(safe_prompt, CapabilityState::Unsupported);
    }
    let herdr = rig.dial_seatless("/herdr/later.sock", AuditHost::HERDR);
    for evidence in [&reserved, &hinted_core] {
        let (status, _, _) = evidence.current();
        assert!(Arc::ptr_eq(&status, herdr.evidence()));
    }
}

// Kills: Health reading refusals from the never-written fallback only. A refusal recorded on a
// dialled session and one on the built-in named session must both reach Health and doctor.
#[test]
fn refusals_of_every_session_reach_health_and_doctor() {
    use crate::{
        ports::StaleReason, protocol::time::UtcMillis, service::host_evidence::HostEvidenceStatus,
    };
    let rig = Rig::new("audit-refusals-health");
    let fallback = Arc::new(HostEvidenceStatus::default());
    let named = rig.builtin(AuditHost::NAMED);
    let herdr = rig.dial_seatless("/herdr/refusals.sock", AuditHost::HERDR);
    let refusal = |seat: &str| crate::identity::reconcile::RefusedTransition {
        seat: SeatId::new(seat),
        generation: 1,
        reason: StaleReason::new("test_reason"),
        action: std::mem::discriminant(&crate::ports::ReconciliationAction::MarkUnresolved),
        excludable: false,
    };
    herdr
        .evidence()
        .record_reconcile_page(&[refusal("sdialled01")], true, UtcMillis(1));
    named
        .evidence()
        .record_reconcile_page(&[refusal("snamed0001")], true, UtcMillis(2));
    let provider = crate::app::elected_health_provider(
        crate::node::NodeUuid::new_v4(),
        uuid::Uuid::new_v4(),
        crate::service::config::ServiceConfig::default().health_settings(),
        Arc::clone(&rig.clock),
        Arc::clone(&rig.store) as Arc<dyn StorePort>,
        Vec::<Arc<crate::service::workers::WorkerStatus>>::new(),
        crate::app::ElectedHostEvidence::of_sessions(
            &rig.sessions,
            None,
            fallback,
            Default::default(),
        ),
    );
    let budget = CallBudget {
        deadline: MonoInstant(rig.clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let inputs = provider(&budget);
    assert_eq!(inputs.transitions_refused, 2);
    let seats: Vec<&str> = inputs
        .refused_seats
        .iter()
        .map(|r| r.seat.as_str())
        .collect();
    assert!(
        seats.contains(&"sdialled01") && seats.contains(&"snamed0001"),
        "{seats:?}"
    );
    let notes = inputs.assemble().notes;
    let note = notes
        .iter()
        .find(|note| note.starts_with("reconciliation:"))
        .expect("the refusal note is emitted");
    assert!(
        note.contains("sdialled01") && note.contains("snamed0001"),
        "{note}"
    );
    let text =
        crate::cli::doctor::render_debug_text(&serde_json::json!({"daemon": {"notes": notes}}));
    assert!(
        text.contains("sdialled01") && text.contains("snamed0001"),
        "{text}"
    );
}

// Kills: a merge that exceeds the bound, double-lists a shared pair, or orders by anything but count.
#[test]
fn merged_refusals_bounds_and_sums() {
    use crate::{
        ports::StaleReason,
        protocol::time::UtcMillis,
        service::host_evidence::{HostEvidenceStatus, MAX_REFUSED_SEATS, merged_refusals},
    };
    let refuse = |ledger: &HostEvidenceStatus, seat: String, times: usize| {
        let one = crate::identity::reconcile::RefusedTransition {
            seat: SeatId::new(seat),
            generation: 1,
            reason: StaleReason::new("test_reason"),
            action: std::mem::discriminant(&crate::ports::ReconciliationAction::MarkUnresolved),
            excludable: false,
        };
        for _ in 0..times {
            ledger.record_reconcile_page(std::slice::from_ref(&one), true, UtcMillis(1));
        }
    };
    let (a, b) = (
        Arc::new(HostEvidenceStatus::default()),
        Arc::new(HostEvidenceStatus::default()),
    );
    for n in 0..MAX_REFUSED_SEATS {
        refuse(&a, format!("a{n}"), 1);
        refuse(&b, format!("b{n}"), 1);
    }
    refuse(&a, "shared".into(), 2);
    refuse(&b, "shared".into(), 3);
    let (total, pairs) = merged_refusals(&[a, b]);
    assert_eq!(total, 2 * MAX_REFUSED_SEATS as u64 + 5);
    assert_eq!(pairs.len(), MAX_REFUSED_SEATS);
    assert_eq!((pairs[0].seat.as_str(), pairs[0].count), ("shared", 5));
    assert_eq!(
        pairs.iter().filter(|p| p.seat.as_str() == "shared").count(),
        1
    );
    assert!(pairs.windows(2).all(|w| w[0].count >= w[1].count));
}

// The two-host half of the audit: an hc core is dialled first (the lowest host id) and a Herdr
// TUI second, so a consumer that takes "the first dialled host" mis-picks.

fn hc_then_tui(rig: &Rig) -> (Arc<HostSession>, Arc<HostSession>) {
    let hc = rig.dial(
        "/hc/core.sock",
        AuditHost {
            space: Some("w9"),
            ..AuditHost::HC
        },
    );
    let tui = rig.dial(
        "/herdr/a.sock",
        AuditHost {
            space: Some("w1"),
            ..AuditHost::HERDR
        },
    );
    assert!(hc.host_id() < tui.host_id(), "hc holds the lowest host id");
    (hc, tui)
}

// Kills: `first_host_pacer` taking the lowest dialled session (hc, which runs no wake lane) and
// answering no wake Pacer while a Herdr TUI's wake lane runs.
#[test]
fn first_host_pacer_two_host() {
    use crate::service::kicks::Lane;
    let rig = Rig::new("audit-pacer-two-host");
    let (hc, tui) = hc_then_tui(&rig);
    assert!(hc.wake().is_none(), "hc runs no wake lane");
    let wake = rig
        .sessions
        .first_pacer(Lane::Wakes)
        .expect("the TUI's lane");
    assert!(Arc::ptr_eq(&wake, tui.wake().unwrap().pacer()));
    // The observation lane runs on every dialled host: the first one leads.
    let observation = rig.sessions.first_pacer(Lane::Observation).unwrap();
    assert!(Arc::ptr_eq(&observation, hc.observation().pacer()));
}

// Kills: a placement that lands a launch on the wrong kind of host: a pane or a space of a TUI
// only on the TUI, a terminal only on the core, and a space both know refused as ambiguous.
#[test]
fn place_two_host() {
    use crate::{
        service::{fair_writer::FairWriter, peer_launcher::PeerLaunchService},
        sync::frame::{LaunchState, LaunchTarget},
    };
    let rig = Rig::new("audit-place-two-host");
    let (hc, tui) = hc_then_tui(&rig);
    let launcher = PeerLaunchService {
        live: Default::default(),
        store: Arc::clone(&rig.store),
        sessions: Arc::clone(&rig.sessions),
        writer: Arc::new(FairWriter::new(32)),
        clock: Arc::clone(&rig.clock),
        state_dir: std::path::PathBuf::from("/nonexistent"),
        log: Arc::new(|_| {}),
    };
    let tab = |space: &str| LaunchTarget::NewTab {
        space: space.into(),
        label: "t".into(),
    };
    let placed = launcher.place(&tab("w1")).expect("w1 is the TUI's");
    assert!(Arc::ptr_eq(&placed.session, &tui));
    let placed = launcher.place(&tab("w9")).expect("w9 is the core's");
    assert!(Arc::ptr_eq(&placed.session, &hc));
    let terminal = LaunchTarget::NewTerminal { label: "t".into() };
    let placed = launcher.place(&terminal).expect("one core");
    assert!(Arc::ptr_eq(&placed.session, &hc), "never the TUI");
    // A pane is a Herdr notion: only the guarded-start host is consulted, never the core.
    let pane = LaunchTarget::Pane {
        space: None,
        tab: None,
        pane: "w1:p1".into(),
    };
    match launcher.place(&pane) {
        Err(LaunchState::Refused { .. }) => {}
        other => panic!("no pane on the TUI fixture: {:?}", other.err()),
    }
    // A second TUI knowing w9 makes the core's space ambiguous, not silently the lowest id.
    rig.dial(
        "/herdr/b.sock",
        AuditHost {
            space: Some("w9"),
            ..AuditHost::HERDR
        },
    );
    match launcher.place(&tab("w9")) {
        Err(LaunchState::Refused { detail }) => assert!(detail.contains("ambiguous"), "{detail}"),
        other => panic!("w9 exists on two hosts: {:?}", other.err()),
    }
}

// Kills: an idle-exit input that counts a dialled host holding no resolved seat (a seatless core
// attached at startup would keep the daemon up for ever), and one that drops a seated hc host.
#[test]
fn idle_exit_seatless_core_exits() {
    let rig = Rig::new("audit-idle-seatless-core");
    rig.dial_seatless("/hc/core.sock", AuditHost::HC);
    assert_eq!(rig.sessions.attached_count(), 1);
    assert!(
        !dialled_hosts_active(&rig.sessions),
        "a seatless core alone keeps the daemon up"
    );
    rig.dial("/hc/other.sock", AuditHost::HC);
    assert!(
        dialled_hosts_active(&rig.sessions),
        "a core holding a seat keeps it up"
    );
}
