//! Fake-locator [`HostAttachPlan`]s for tests: a map of locators to fixture
//! hosts replaces `NativeCli` and the socket test, so an in-process elected
//! daemon attaches fixture hosts through the production `HostSessions` path.
//! Absent in ordinary builds.

use crate::{
    ports::{
        AgentComposerState, BootstrapObserver, BootstrapPaneObservation, ComposerObservation,
        HostCallContext, HostObservation, HostPort, HostSnapshot, NativeLaunchCapability,
        NativeLaunchOutcome, NativeLaunchRequest, PromptOutcome, SafeWakeTarget,
    },
    protocol::{
        ids::{HostLocator, HostTargetId, SeatId},
        results::ApiError,
    },
    service::host_sessions::{HostAttachPlan, HostFactory},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};

/// A fixture server's port and bootstrap observer.
type FixtureHost = (Arc<dyn HostPort>, Arc<dyn BootstrapObserver>);

/// A bootstrap observer for a fixture host that never runs guarded bootstrap:
/// every observation is refused.
struct NoBootstrapObserver;

impl BootstrapObserver for NoBootstrapObserver {
    fn observe_bootstrap_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<BootstrapPaneObservation, ApiError> {
        Err(ApiError::unsupported("bootstrap observation unavailable"))
    }
}

/// The fixture servers of one test: which locators have a host, and which of
/// those currently have a "socket". Cloning the plan shares the world, so a
/// test makes a locator live after the daemon started (the sweep's case).
#[derive(Default)]
pub struct FakeHostWorld {
    hosts: Mutex<BTreeMap<String, FixtureHost>>,
    live: Mutex<BTreeSet<String>>,
}

impl FakeHostWorld {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registers `host` under `locator` and makes its socket exist.
    pub fn add(&self, locator: &HostLocator, host: Arc<dyn HostPort>) {
        self.add_with_observer(locator, host, Arc::new(NoBootstrapObserver));
    }

    /// [`FakeHostWorld::add`] with the bootstrap observer guarded bootstrap uses.
    pub fn add_with_observer(
        &self,
        locator: &HostLocator,
        host: Arc<dyn HostPort>,
        observer: Arc<dyn BootstrapObserver>,
    ) {
        self.hosts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(locator.as_str().to_owned(), (host, observer));
        self.set_live(locator, true);
    }

    /// Makes `locator`'s socket exist (or not).
    pub fn set_live(&self, locator: &HostLocator, live: bool) {
        let mut set = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if live {
            set.insert(locator.as_str().to_owned());
        } else {
            set.remove(locator.as_str());
        }
    }

    /// The plan whose factory builds from this world, with `hint` as the
    /// startup hint.
    pub fn plan(self: &Arc<Self>, hint: Option<HostLocator>) -> HostAttachPlan {
        let building = Arc::clone(self);
        let existing = Arc::clone(self);
        HostAttachPlan {
            hint,
            factory: HostFactory {
                build: Arc::new(move |locator| {
                    building
                        .hosts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .get(locator.as_str())
                        .cloned()
                        .unwrap_or_else(|| panic!("no fixture host for {}", locator.as_str()))
                }),
                exists: Arc::new(move |path: &Path| {
                    path.to_str().is_some_and(|path| {
                        existing
                            .live
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .contains(path)
                    })
                }),
            },
        }
    }
}

/// A plan over `hosts` (every socket exists) with `hint` as the startup hint.
pub fn fake_host_plan(
    hosts: &[(HostLocator, Arc<dyn HostPort>)],
    hint: Option<HostLocator>,
) -> HostAttachPlan {
    let world = FakeHostWorld::new();
    for (locator, host) in hosts {
        world.add(locator, Arc::clone(host));
    }
    world.plan(hint)
}

/// A plan with the one fixture `host` at `endpoint`, given as the hint.
pub fn single_host_plan_at(host: Arc<dyn HostPort>, endpoint: impl AsRef<Path>) -> HostAttachPlan {
    let locator = HostLocator::parse(endpoint.as_ref().to_string_lossy().into_owned())
        .expect("a canonical fixture endpoint");
    fake_host_plan(&[(locator.clone(), host)], Some(locator))
}

/// [`single_host_plan_at`] under the unnamed fixture endpoint
/// ([`super::TEST_HOST_ENDPOINT`]).
pub fn single_host_plan(host: Arc<dyn HostPort>) -> HostAttachPlan {
    single_host_plan_at(host, super::TEST_HOST_ENDPOINT)
}

/// [`single_host_plan_at`] whose host also serves guarded bootstrap observations.
pub fn single_host_plan_with_observer(
    host: Arc<dyn HostPort>,
    observer: Arc<dyn BootstrapObserver>,
    endpoint: impl AsRef<Path>,
) -> HostAttachPlan {
    let locator = HostLocator::parse(endpoint.as_ref().to_string_lossy().into_owned())
        .expect("a canonical fixture endpoint");
    let world = FakeHostWorld::new();
    world.add_with_observer(&locator, host, observer);
    world.plan(Some(locator))
}

/// A fixture host whose every call is refused: its session starts, observes
/// nothing and wakes nothing.
pub struct RefusingHost;

fn refused<T>() -> Result<T, ApiError> {
    Err(ApiError::unsupported("refusing fixture host"))
}

impl HostPort for RefusingHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
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
