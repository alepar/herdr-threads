//! The herdr-core (`hc`) host adapter and its protocol layer.
//!
//! Responsibility (HcHost / HcBoot / HcDiscovery)
//! - Owns: the one-connection-per-call line client ([`rpc::HcClient`]), the
//!   tolerant wire types ([`wire`]), boot = incarnation = `hc-core:<core.id>`
//!   ([`HcBoot`]), mapping herdr-core's `(keep, reason)` finality to closure
//!   (D10, with finished records kept in the capture), `status: null` as
//!   `Unknown`, labels from `herdr.space`/`herdr.tab` plus the `herdr-app`
//!   doc ([`labels`]), `server_process` from the `LocalEndpointWitness` peer
//!   pid, and discovery from `core.json` plus `hello`. Closure and labels
//!   land in later beads; today [`HcHost`] holds the boot, the capability
//!   answers, the observation counter, `target_root_process` and target
//!   creation (`CreateTargetPort` via `terminal.run`: `no_notify`,
//!   `outlive_parent`, `no_parent`, `keep: false`, no `caller`, no hc `name`,
//!   no `first_message`; the `ht.bootstrap` label is written as a hint and
//!   never read to classify an outcome).
//! - Discovery ([`HcDiscovery`], `discovery.rs`): lists
//!   `$XDG_STATE_HOME|~/.local/state/herdr-core/*/core.json`, derives each
//!   socket by the core's rules (`HC_SOCKET` or config, else
//!   `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`, else
//!   `<state>/<session>.sock`), keeps a core only when its `hello` `core.id`
//!   matches `core.json`, dedupes by id. Used when no caller site names a
//!   core; a scan never runs `hc` and never attaches.
//! - Hides: the hc protocol, the socket path rules, and which hc methods it
//!   must never call (`terminal.list`, `doc.set`, `caller`).
//! - Must not: spawn `hc`, treat agent exit as closure, keep `machine/tN`
//!   mirrors, or attach on a diagnostic scan.
//! - Where new behaviour goes: here, in `src/host/hc/`. Callers never call hc
//!   directly or read hc state files.
//! - Lifecycle: built by `HostFactory.build(Hc, locator)` on a dialled
//!   attach, lives as long as its `HostSession`, holds no long-lived
//!   connection. The observation counter is per-instance mutable state.
//! - Errors: a failed `hello` gives `CompleteUnverified` (closes nothing). A
//!   core id mismatch is a new boot (C2 holds). An RPC error before a launch
//!   record exists is `NotSubmitted`; `spawn_failed`, a timeout or a
//!   disconnect is `OutcomeUnknown` (H3).
//! - Role: Adapter (a `HostPort` impl over herdr-core's JSON-RPC).

pub mod discovery;
pub mod labels;
pub mod rpc;
pub mod wire;

pub use discovery::{DiscoveredCore, HcDiscovery};
pub use rpc::{HcClient, HcError, HcReply, RunRequest, Snapshot};
pub use wire::{
    AppDoc, CoreInfo, HcEvent, HcEventKind, HcForeground, HcStatus, HcTerminal, HelloReply,
};

use crate::ports::{
    self, AgentComposerState, ArchivalEvidence, ClosureAuthority, ComposerObservation,
    EnumerationEvidence, HostCallContext, HostKind, HostObservation, HostPort, HostSnapshot,
    IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
    PromptOutcome, SafeWakeTarget, WakeCapability,
};
use crate::protocol::{
    ids::{HostBootId, HostTargetId, SeatId},
    results::{ApiError, ErrorCode},
    time::CallBudget,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Environment herdr-core sets for a terminal: its id and socket (read for
/// caller recognition) and its scope and session. Every child the daemon side
/// spawns has these removed (K5); the H3 gate shim removes all but them.
pub const ENV_VARS: &[&str] = &[
    HC_TERMINAL_ID_ENV,
    HC_TERMINAL_SOCKET_ENV,
    "HC_SCOPE",
    "HC_SESSION",
];
/// The terminal id herdr-core sets for a terminal's processes.
pub(crate) const HC_TERMINAL_ID_ENV: &str = "HC_TERMINAL_ID";
/// The core socket herdr-core sets for a terminal's processes.
pub(crate) const HC_TERMINAL_SOCKET_ENV: &str = "HC_TERMINAL_SOCKET";

const BOOT_PREFIX: &str = "hc-core:";
const MAX_BOOT_BYTES: usize = 128;

/// A herdr-core boot: `hc-core:<core.id>`. The socket is only a dial address;
/// a different core id at a known locator is a new boot (C2 holds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcBoot(HostBootId);

impl HcBoot {
    /// The boot a `hello` reply names; `None` when the core id is empty or the
    /// boot would be non-ASCII or longer than 128 bytes.
    pub fn from_hello(hello: &HelloReply) -> Option<Self> {
        let id = hello.core.id.as_str();
        if id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_graphic())
            || BOOT_PREFIX.len() + id.len() > MAX_BOOT_BYTES
        {
            return None;
        }
        HostBootId::parse(format!("{BOOT_PREFIX}{id}"))
            .ok()
            .map(Self)
    }

    pub fn id(&self) -> &HostBootId {
        &self.0
    }
}

/// The herdr-core `HostPort`. Capability answers: no wake path, closing,
/// no archival evidence.
pub struct HcHost {
    client: HcClient,
    observation_sequence: AtomicU64,
    doc_names: labels::DocNames,
    /// The full argv `terminal.run` starts (the launch-gate shim, composed by
    /// the creator). Empty until set: nothing is created without it.
    gate_argv: Vec<String>,
    run_deadline: Duration,
}

impl HcHost {
    pub fn new(client: HcClient) -> Self {
        Self {
            client,
            observation_sequence: AtomicU64::new(0),
            doc_names: labels::DocNames::default(),
            gate_argv: Vec::new(),
            run_deadline: rpc::RUN_DEADLINE,
        }
    }

    /// Sets the argv `create_target` starts (the gate shim and its harness argv).
    pub fn with_gate_argv(mut self, argv: Vec<String>) -> Self {
        self.gate_argv = argv;
        self
    }

    /// Overrides the `terminal.run` budget (default [`rpc::RUN_DEADLINE`]).
    pub fn with_run_deadline(mut self, deadline: Duration) -> Self {
        self.run_deadline = deadline;
        self
    }

    /// The next value of this host's own observation counter (starts at 1).
    pub fn next_observation_sequence(&self) -> u64 {
        self.observation_sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Runs one client call to completion from synchronous code, on a private
    /// runtime in its own thread so it is safe inside an async context.
    fn run<T: Send>(
        future: impl std::future::Future<Output = Result<T, HcError>> + Send,
    ) -> Result<T, HcError> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(HcError::Io)?
                        .block_on(future)
                })
                .join()
                .unwrap_or_else(|_| Err(HcError::Decode("hc call panicked".into())))
        })
    }

    /// This core's session name (`hello` `core.name`), the `<session>` of its addresses.
    fn session(&self) -> Option<String> {
        Self::run(self.client.hello())
            .ok()
            .map(|reply| reply.value.core.name)
            .filter(|name| !name.is_empty())
    }

    /// `<session>:tN` for one of this core's terminals; `None` when `hello` fails or
    /// names no session, or `target` is not a `tN` address.
    pub fn display_address(&self, target: &HostTargetId) -> Option<labels::HcAddress> {
        Some(labels::HcAddress::new(
            self.session()?,
            labels::TerminalId::parse(target.as_str())?,
        ))
    }

    /// Live local terminals (no `machine/tN` mirrors) and the app doc's names. The doc is
    /// advisory: a failed read gives no names.
    fn read_labelled(
        &self,
        budget: &CallBudget,
    ) -> Result<(Vec<HcTerminal>, std::sync::Arc<labels::AppNames>), ApiError> {
        if budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("hc labels"));
        }
        let snapshot = Self::run(self.client.events_subscribe_snapshot())
            .map_err(|e| ApiError::host_unavailable(e.to_string()))?
            .value;
        let names = match Self::run(self.client.doc_get(labels::APP_DOC)) {
            Ok(reply) => self.doc_names.names(&reply.value),
            Err(_) => Default::default(),
        };
        let live = snapshot
            .terminals
            .into_iter()
            .filter(|t| t.state == "live" && !t.address.contains('/'))
            .collect();
        Ok((live, names))
    }

    fn unavailable(what: &str) -> ApiError {
        ApiError::host_unavailable(format!("hc host: {what} is not served yet"))
    }

    fn unknown_boot() -> HostBootId {
        HostBootId::new("unverified-hc-host")
    }
}

/// The hc host has no bootstrap pane observation yet: every call is refused.
pub struct HcBootstrapObserver;

impl crate::ports::BootstrapObserver for HcBootstrapObserver {
    fn observe_bootstrap_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<crate::ports::BootstrapPaneObservation, ApiError> {
        Err(HcHost::unavailable("bootstrap observation"))
    }
}

fn api_error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}

/// Maps a failed `terminal.run` to an outcome. Only a JSON-RPC error other
/// than `spawn_failed` proves the record was never made; `spawn_failed`, a
/// timeout, a disconnect or an undecodable reply leave it unknown. Never
/// consults `ht.bootstrap` or any terminal state.
fn run_failure(error: HcError) -> ports::CreateTargetOutcome {
    use ports::CreateTargetOutcome::{NotSubmitted, OutcomeUnknown};
    match &error {
        HcError::Rpc { kind, .. } if kind.as_deref() != Some("spawn_failed") => {
            NotSubmitted(api_error(ErrorCode::HostUnavailable, error.to_string()))
        }
        HcError::Timeout => {
            OutcomeUnknown(api_error(ErrorCode::DeadlineExceeded, error.to_string()))
        }
        _ => OutcomeUnknown(api_error(ErrorCode::HostUnavailable, error.to_string())),
    }
}

impl ports::CreateTargetPort for HcHost {
    /// `terminal.run` starts the gate shim argv. Nothing is submitted unless
    /// `hello` first shows the expected core and peer, and the run is pinned
    /// to that peer's witness.
    fn create_target(
        &self,
        request: &ports::CreateTargetRequest,
        context: &HostCallContext,
    ) -> ports::CreateTargetOutcome {
        use ports::CreateTargetOutcome::{Created, NotSubmitted, OutcomeUnknown};
        let ports::HostWitness::HcCore {
            core_id: expected_core,
            peer: expected_peer,
        } = &request.expected_witness
        else {
            return NotSubmitted(api_error(
                ErrorCode::InvalidRequest,
                "hc host creates terminals against an hc core witness only",
            ));
        };
        let Some(cwd) = request.cwd.to_str().filter(|_| request.cwd.is_absolute()) else {
            return NotSubmitted(api_error(ErrorCode::InvalidRequest, "invalid launch cwd"));
        };
        if self.gate_argv.is_empty() || request.label.trim().is_empty() {
            return NotSubmitted(api_error(
                ErrorCode::InvalidRequest,
                "hc launch needs a gate argv and a label",
            ));
        }
        if context.budget.cancellation.is_cancelled() {
            return NotSubmitted(api_error(ErrorCode::Cancelled, "hc create_target"));
        }
        let client = self.client.clone().with_deadline(self.run_deadline);
        let hello = match Self::run(client.hello()) {
            Ok(reply) => reply,
            Err(e) => {
                return NotSubmitted(api_error(ErrorCode::HostUnavailable, e.to_string()));
            }
        };
        let witness = hello.witness.clone();
        let peer = ports::HcPeer {
            pid: witness.peer_pid,
            start_seconds: witness.start_seconds,
            uid: witness.peer_uid,
        };
        let boot = HcBoot::from_hello(&hello.value);
        if &hello.value.core.id != expected_core
            || &peer != expected_peer
            || context
                .expected_boot
                .as_ref()
                .is_some_and(|expected| Some(expected) != boot.as_ref().map(HcBoot::id))
        {
            return NotSubmitted(api_error(
                ErrorCode::StaleHostObservation,
                "hc core changed before launch",
            ));
        }
        let mut labels = std::collections::BTreeMap::new();
        let parent = match &request.placement {
            ports::Placement::NewTab { space } => {
                labels.insert(labels::LABEL_SPACE.to_owned(), space.as_str().to_owned());
                labels.insert(
                    labels::LABEL_TAB.to_owned(),
                    format!("ht-{}", uuid::Uuid::new_v4().simple()),
                );
                labels.insert(
                    labels::LABEL_BOOTSTRAP.to_owned(),
                    request.correlation.as_str().to_owned(),
                );
                labels.insert(labels::LABEL_HT.to_owned(), request.label.clone());
                None
            }
            ports::Placement::NewTerminal { parent } => Some(parent.as_str().to_owned()),
        };
        let run = RunRequest {
            argv: self.gate_argv.clone(),
            cwd: cwd.to_owned(),
            env: request.env.clone(),
            labels,
            parent,
        };
        let reply = match Self::run(client.clone().expecting(witness).terminal_run(&run)) {
            Ok(reply) => reply,
            Err(HcError::WitnessChanged | HcError::Witness(_)) => {
                return NotSubmitted(api_error(
                    ErrorCode::StaleHostObservation,
                    "hc peer changed before launch",
                ));
            }
            Err(e) => return run_failure(e),
        };
        let address = reply.value.address;
        let created = ports::CreatedTarget {
            correlation: request.correlation.clone(),
            placement: request.placement.clone(),
            target: HostTargetId::new(address.clone()),
            root_pane: None,
            terminal: crate::protocol::ids::TerminalId::new(address),
            host_incarnation: HostBootId::new(format!(
                "{BOOT_PREFIX}{expected_core}:pid={}:start={}:uid={}",
                peer.pid, peer.start_seconds, peer.uid
            )),
            witness: request.expected_witness.clone(),
        };
        // The record exists now: a result this host cannot vouch for stays unknown.
        let coherent = !created.target.as_str().is_empty() && created.validate().is_ok();
        if coherent {
            Created(Box::new(created))
        } else {
            OutcomeUnknown(api_error(
                ErrorCode::StaleHostObservation,
                "incoherent hc terminal creation evidence",
            ))
        }
    }
}

impl HostPort for HcHost {
    fn kind(&self) -> HostKind {
        HostKind::Hc
    }

    fn wake_capability(&self) -> WakeCapability {
        WakeCapability::None
    }

    fn closure_authority(&self) -> ClosureAuthority {
        ClosureAuthority::Closing
    }

    fn archival_evidence(&self) -> ArchivalEvidence {
        ArchivalEvidence::NoEvidence
    }

    fn target_root_process(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<Option<u32>, ApiError> {
        if context.budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("hc target_root_process"));
        }
        let reply = Self::run(self.client.terminal_get(target.as_str()))
            .map_err(|e| ApiError::host_unavailable(e.to_string()))?;
        Ok(reply.value.pid)
    }

    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }

    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Err(Self::unavailable("observe_current_target"))
    }

    fn observe_current_target_for_archival(
        &self,
        _: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<ComposerObservation, ApiError> {
        Err(Self::unavailable("archival observation"))
    }

    fn seat_labels(
        &self,
        budget: &CallBudget,
    ) -> Result<Vec<crate::host::observation::SeatHostLabels>, ApiError> {
        let (terminals, names) = self.read_labelled(budget)?;
        Ok(terminals
            .into_iter()
            .map(|t| {
                let space = t.labels.get(labels::LABEL_SPACE);
                let tab = t.labels.get(labels::LABEL_TAB);
                crate::host::observation::SeatHostLabels {
                    terminal: t.address.clone(),
                    incarnation: None,
                    target: HostTargetId::new(&t.address),
                    workspace_id: space.cloned().unwrap_or_else(|| "-".into()),
                    workspace_label: space.and_then(|id| names.space_name(id)).map(str::to_owned),
                    tab_id: tab.cloned().unwrap_or_else(|| "-".into()),
                    tab_label: tab.and_then(|id| names.tab_label(id)),
                    pane_label: t.name.filter(|name| !name.is_empty()),
                }
            })
            .collect())
    }

    fn live_topology(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::host::observation::HostTopology, ApiError> {
        use crate::host::observation::{HostTopology, TopologyPane, TopologySpace, TopologyTab};
        let (terminals, names) = self.read_labelled(budget)?;
        let mut topology = HostTopology {
            spaces: Vec::new(),
            tabs: Vec::new(),
            panes: Vec::new(),
        };
        for t in terminals {
            // A terminal without both labels is not in the app, so it has no place here.
            let (Some(space), Some(tab)) = (
                t.labels.get(labels::LABEL_SPACE),
                t.labels.get(labels::LABEL_TAB),
            ) else {
                continue;
            };
            if !topology.spaces.iter().any(|s| &s.id == space) {
                topology.spaces.push(TopologySpace {
                    id: space.clone(),
                    label: names.space_name(space).map(str::to_owned),
                });
            }
            if !topology.tabs.iter().any(|x| &x.id == tab) {
                topology.tabs.push(TopologyTab {
                    id: tab.clone(),
                    space: space.clone(),
                    label: names.tab_label(tab),
                });
            }
            topology.panes.push(TopologyPane {
                target: HostTargetId::new(&t.address),
                space: space.clone(),
                tab: tab.clone(),
                label: t.name.filter(|name| !name.is_empty()),
                agent_names: Vec::new(),
            });
        }
        Ok(topology)
    }

    /// Only the failed-`hello` outcome: nothing is enumerated and nothing is
    /// closed (`CompleteUnverified`, not `CoherentVerified`). The snapshot and
    /// its capture rule are ht-kqrt.67's.
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        let boot = Self::run(self.client.hello())
            .ok()
            .and_then(|reply| HcBoot::from_hello(&reply.value))
            .map(|boot| boot.0)
            .unwrap_or_else(Self::unknown_boot);
        Ok(HostSnapshot {
            boot,
            epoch: 0,
            observation_sequence: 0,
            complete: false,
            enumeration: EnumerationEvidence::CompleteUnverified,
            incarnation: IncarnationEvidence::Unknown,
            targets: Vec::new(),
        })
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
        Err(Self::unavailable("submit_prompt"))
    }

    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        Err(Self::unavailable("launch_native"))
    }

    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        Err(Self::unavailable("pane_agent_state"))
    }

    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Err(Self::unavailable("send_submit_key"))
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/host/hc_host.rs"]
mod tests;

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/host/hc_labels.rs"]
mod label_tests;

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/host/hc_terminal_run.rs"]
mod terminal_run_tests;
