//! ht-kqrt.8: `wake_status` classifies each row against the seat's own host. Included from
//! `src/service/dispatch.rs` (it needs the crate-private `classify_wake_rows`).
use super::*;
use crate::{
    app::SystemClock,
    host::observation::HostTopology,
    ports::{
        AgentComposerState, ComposerObservation, ComposerTraits, HostCallContext, HostObservation,
        HostPort, HostSnapshot, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
        PromptOutcome, SafeWakeTarget, WakeCapability,
    },
    protocol::{
        ids::{HostLocator, HostTargetId, SeatId},
        results::{ApiError, SeatWakeStatus, WakeBlock},
        time::{CallBudget, Cancellation, MonoInstant},
    },
    service::host_sessions::SessionDeps,
    store::{SqliteStore, StoreSettings, connection::StoreContext, hosts::HostIndex},
    test_support::{FakeHostWorld, RefusingHost, isolation::TestIsolation},
};

/// A refusing host with a chosen wake capability and a chosen block for every seat.
struct CapHost {
    capability: WakeCapability,
    block: Option<WakeBlock>,
}

impl HostPort for CapHost {
    fn live_topology(&self, budget: &CallBudget) -> Result<HostTopology, ApiError> {
        RefusingHost.live_topology(budget)
    }
    fn wake_capability(&self) -> WakeCapability {
        self.capability
    }
    fn wake_block(&self, _: &SeatId) -> Option<WakeBlock> {
        self.block
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

fn row(seat: &str, harness: &str, host: Option<crate::ports::HostId>) -> SeatWakeStatus {
    SeatWakeStatus {
        seat: SeatId::new(seat),
        harness: Some(harness.into()),
        wake: None,
        first: false,
        offline: false,
        host,
        target: None,
    }
}

/// Two attached fakes, `composer` and `bare`; every seat has no channel unless `live`.
fn two_hosts(
    label: &str,
    composer_block: Option<WakeBlock>,
) -> (Vec<Arc<HostSession>>, Arc<HostSessions>, TestIsolation) {
    let isolation = TestIsolation::new(label);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let node = crate::node::NodeUuid::new_v4().to_string();
    let index = Arc::new(HostIndex::new());
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            StoreContext::with_host_index(
                isolation.path("store.db"),
                Arc::clone(&clock),
                Arc::clone(&index),
            )
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
        SessionDeps::detached(store, Arc::clone(&clock), node),
    ));
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let mut attached = Vec::new();
    for (name, capability, block) in [
        (
            "composer.sock",
            WakeCapability::Composer(ComposerTraits::HERDR_TUI),
            composer_block,
        ),
        ("bare.sock", WakeCapability::None, None),
    ] {
        let locator = HostLocator::parse(isolation.socket_path(name).to_string_lossy()).unwrap();
        world.add(&locator, Arc::new(CapHost { capability, block }));
        attached.push(sessions.resolve(&locator, &budget).unwrap());
    }
    (attached, sessions, isolation)
}

// Kills: classifying every row against one shared host (the first, or "any attached"): the same
// codex harness is `no_wake` on the host without a wake path and unblocked on the composer host,
// and a block the composer host reports never leaks onto the other host's seat.
#[test]
fn wake_rows_classified_against_their_own_host() {
    let (hosts, sessions, _isolation) = two_hosts("wsh-own", Some(WakeBlock::Focused));
    let (composer, bare) = (hosts[0].host_id(), hosts[1].host_id());
    assert_ne!(composer, bare);
    let mut rows = vec![
        row("on-composer", "codex", Some(composer)),
        row("on-bare", "codex", Some(bare)),
    ];
    classify_wake_rows(&mut rows, &hosts, &|_| false);
    assert_eq!(rows[0].wake, Some(WakeBlock::Focused));
    assert_eq!(rows[1].wake, Some(WakeBlock::NoWake));
    // Row order does not matter: the host id, not the position, picks the host.
    let mut swapped = vec![
        row("on-bare", "codex", Some(bare)),
        row("on-composer", "codex", Some(composer)),
    ];
    classify_wake_rows(&mut swapped, &hosts, &|_| false);
    assert_eq!(swapped[0].wake, Some(WakeBlock::NoWake));
    assert_eq!(swapped[1].wake, Some(WakeBlock::Focused));
    sessions.stop_all();
}

// Kills: ignoring the live-channel answer on a host without a wake path (Claude's channel wakes
// itself there, so it is not `no_wake`), or applying it to a harness without a channel.
#[test]
fn a_live_channel_lifts_no_wake_only_for_a_self_waking_harness() {
    let (hosts, sessions, _isolation) = two_hosts("wsh-channel", None);
    let bare = hosts[1].host_id();
    let mut rows = vec![
        row("claude-seat", "claude", Some(bare)),
        row("codex-seat", "codex", Some(bare)),
    ];
    classify_wake_rows(&mut rows, &hosts, &|_| true);
    assert_eq!(rows[0].wake, None);
    assert_eq!(rows[1].wake, Some(WakeBlock::NoWake));
    sessions.stop_all();
}

// Kills: a row with no host (an older store row or an unattached host) changing Herdr
// classification: it keeps `has_idle_wake`, and an unregistered harness stays unclassified.
#[test]
fn a_row_without_a_known_host_keeps_the_herdr_classification() {
    let (hosts, sessions, _isolation) = two_hosts("wsh-legacy", None);
    let mut rows = vec![
        row("a", "codex", None),
        row("b", "pi", None),
        row("c", "codex", Some(crate::ports::HostId::from_row(9_999))),
    ];
    rows[1].wake = Some(WakeBlock::Unreadable);
    classify_wake_rows(&mut rows, &hosts, &|_| false);
    assert_eq!(rows[0].wake, None);
    assert_eq!(rows[1].wake, None);
    assert_eq!(rows[2].wake, None);
    sessions.stop_all();
}

// Kills: a required host field on the wire row (an old daemon's reply would not decode), or one
// that serializes the node-local host id to a client.
#[test]
fn old_shape_wake_status_decodes() {
    let old = r#"{"seat":"s1","harness":"codex","wake":"no_wake","first":true}"#;
    let decoded: SeatWakeStatus = serde_json::from_str(old).unwrap();
    assert_eq!(decoded.host, None);
    assert_eq!(decoded.wake, Some(WakeBlock::NoWake));
    assert!(decoded.first);
    let mut with_host = decoded.clone();
    with_host.host = Some(crate::ports::HostId::from_row(3));
    let wire = serde_json::to_string(&with_host).unwrap();
    assert!(!wire.contains("host"), "{wire}");
    // A newer daemon's extra field is ignored, not refused.
    let newer = r#"{"seat":"s1","harness":null,"wake":null,"host":7,"extra":1}"#;
    assert_eq!(
        serde_json::from_str::<SeatWakeStatus>(newer).unwrap().host,
        None
    );
}
