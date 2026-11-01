//! The herdr-core (`hc`) host adapter and its protocol layer.
//!
//! Responsibility (HcHost / HcBoot / HcDiscovery)
//! - Owns: the one-connection-per-call line client ([`rpc::HcClient`]), the
//!   tolerant wire types ([`wire`]), boot = incarnation = `hc-core:<core.id>`
//!   ([`HcBoot`]), mapping herdr-core's `(keep, reason)` finality to closure
//!   (D10, with finished records kept in the capture), `status: null` as
//!   `Unknown`, labels from `herdr.space`/`herdr.tab` plus the `herdr-app`
//!   doc ([`labels`]), `server_process` from the `LocalEndpointWitness` peer
//!   pid, and discovery from `core.json` plus `hello`. Labels land in a later
//!   bead; today [`HcHost`] holds the boot, the capability answers, the
//!   observation counter, `target_root_process`, the capture (`hello` +
//!   `events.subscribe{}` on one connection, finished records kept with their
//!   `reason` and `keep`, `machine/tN` mirrors dropped), the D10 closure
//!   decision over it ([`HcClosure`]) and target creation (`CreateTargetPort`
//!   via `terminal.run`: `no_notify`, `outlive_parent`, `no_parent`,
//!   `keep: false`, no `caller`, no hc `name`, no `first_message`; the
//!   `ht.bootstrap` label is written as a hint and never read to classify an
//!   outcome).
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

use crate::host::continuity::LocalEndpointWitness;
use crate::ports::{
    self, AgentComposerState, ArchivalEvidence, ClosureAuthority, ComposerObservation,
    EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostCallContext, HostKind,
    HostObservation, HostPort, HostSnapshot, HostUiState, IncarnationEvidence,
    NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest, ObservationProvenance,
    PromptOutcome, SafeWakeTarget, StructuralOccupancy, WakeCapability,
};
use crate::protocol::{
    ids::{HostBootId, HostCallId, HostTargetId, SeatId, TerminalId},
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock},
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

static CALL_ID: AtomicU64 = AtomicU64::new(1);

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

/// Basenames of herdr-core terminal processes (the `hc` client, the core). `LocateCaller`
/// treats one on the walk as a wall: a candidate root above it does not verify. The hc
/// candidate's own root sits below the wall and is found first, so the wall changes no
/// outcome for an hc caller.
pub fn foreign_terminal_process_names() -> &'static [&'static str] {
    &["herdr-core", "hc"]
}

/// The `NewTerminal` parent of a creation with no caller terminal (a peer-requested
/// launch: the daemon is the caller): `create_target` sends no `parent`.
pub const NO_PARENT: &str = "-";

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

/// Finish reasons for which herdr-core keeps a `keep: true` terminal `live`
/// with no holder, waiting to restart it (`Registry::finish`, dc276b7). Any
/// other finish is final. The list only reads a `live` record; a `finished`
/// record is final whatever its reason.
const PENDING_RESTART_REASONS: &[&str] = &[
    "holder_lost",
    "stopped_by_restart",
    "server_stop",
    "host_shutdown",
    "signaled",
];

/// What one local record of a capture means for its seat (D10). It follows
/// herdr-core's own finality: the record's `state`, `keep`, `reason` and
/// whether a holder runs it. Events are only hints to capture again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HcClosure {
    /// `live` with a holder: nothing changes, the target is present. An agent
    /// that exits inside a live shell terminal is this, not a finish.
    Present,
    /// `live`, `keep: true`, no holder, a pending reason: the seat is
    /// unresolved and the core's restart of the same `tN` reattaches it (C1).
    PendingRestart,
    /// `finished`: the core never revives it and never reuses the id. The
    /// target is absent from the snapshot, so the seat retires through the
    /// pane-close fence, resolved or unresolved. Pruning (`removed`) is the
    /// same absence.
    Final,
    /// A `state` that is neither `live` nor `finished`: no decision is read.
    Unreadable,
}

impl HcClosure {
    /// The decision for one record.
    pub fn of(terminal: &HcTerminal) -> Self {
        match terminal.state.as_str() {
            "finished" => Self::Final,
            "live" => {
                let pending = terminal.keep
                    && terminal.pid.is_none()
                    && terminal
                        .reason
                        .as_deref()
                        .is_some_and(|reason| PENDING_RESTART_REASONS.contains(&reason));
                if pending {
                    Self::PendingRestart
                } else {
                    Self::Present
                }
            }
            _ => Self::Unreadable,
        }
    }
}

/// Where an agent status came from, shown only by `doctor` (D21).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusSource {
    /// The terminal self-reported OSC 7501.
    Osc7501,
    /// No status was ever reported (`status: null`).
    None,
}

impl StatusSource {
    pub fn of(terminal: &HcTerminal) -> Self {
        if terminal.status.is_some() {
            Self::Osc7501
        } else {
            Self::None
        }
    }

    /// The word `doctor` prints.
    pub fn label(self) -> &'static str {
        match self {
            Self::Osc7501 => "osc7501",
            Self::None => "none",
        }
    }
}

/// The UI state a record's status gives. `status: null` is `Unknown`, never
/// `Idle`; a reported status stays `Unknown` until H2 reads the composer (a
/// reported `idle` still can mean a typed draft).
pub fn ui_state(_terminal: &HcTerminal) -> HostUiState {
    HostUiState::Unknown
}

/// One coherent capture: `hello` and the `events.subscribe{}` snapshot read on
/// one connection. `records` keeps every local address, live and finished,
/// with its `reason` and `keep`; `machine/tN` mirrors are dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct HcCapture {
    pub boot: HcBoot,
    /// The event sequence the snapshot was taken at (restarts reset it).
    pub seq: u64,
    pub witness: LocalEndpointWitness,
    pub records: Vec<HcTerminal>,
}

impl HcCapture {
    /// The `report.hosts.agent_status` rows `doctor` prints: each record's
    /// target and where its status came from (D21).
    pub fn status_rows(&self) -> Vec<serde_json::Value> {
        self.records
            .iter()
            .map(|record| {
                serde_json::json!({
                    "target": record.address,
                    "source": StatusSource::of(record).label(),
                })
            })
            .collect()
    }

    /// Whether every record carries a decision this build can read.
    pub fn is_readable(&self) -> bool {
        self.records
            .iter()
            .all(|record| HcClosure::of(record) != HcClosure::Unreadable)
    }
}

/// The herdr-core `HostPort`. Capability answers: no wake path, closing,
/// no archival evidence.
pub struct HcHost {
    client: HcClient,
    clock: Arc<dyn Clock>,
    epoch: AtomicU64,
    observation_sequence: AtomicU64,
    doc_names: labels::DocNames,
    /// The full argv `terminal.run` starts (the launch-gate shim, composed by
    /// the creator). Empty until set: nothing is created without it.
    gate_argv: Vec<String>,
    run_deadline: Duration,
}

impl HcHost {
    pub fn new(client: HcClient) -> Self {
        Self::with_clock(client, Arc::new(crate::app::SystemClock::new()))
    }

    pub fn with_clock(client: HcClient, clock: Arc<dyn Clock>) -> Self {
        Self {
            client,
            clock,
            epoch: AtomicU64::new(1),
            observation_sequence: AtomicU64::new(0),
            doc_names: labels::DocNames::default(),
            gate_argv: Vec::new(),
            run_deadline: rpc::RUN_DEADLINE,
        }
    }

    /// The witness a creation here expects: this core's id and its peer, read from one
    /// `hello`.
    pub fn core_witness(&self) -> Result<ports::HostWitness, ApiError> {
        let hello = Self::run(self.client.hello())
            .map_err(|e| ApiError::host_unavailable(e.to_string()))?;
        if hello.value.core.id.is_empty() {
            return Err(ApiError::host_unavailable("hc hello names no core id"));
        }
        Ok(ports::HostWitness::HcCore {
            core_id: hello.value.core.id.clone(),
            peer: ports::HcPeer {
                pid: hello.witness.peer_pid,
                start_seconds: hello.witness.start_seconds,
                uid: hello.witness.peer_uid,
            },
        })
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

    /// A daemon restart opens an epoch above every epoch it persisted for this
    /// host, so ordering against durable publications stays monotonic within
    /// one boot. Never lowers the epoch.
    pub fn resume_after_epoch(&self, persisted: u64) {
        self.epoch
            .fetch_max(persisted.saturating_add(1), Ordering::AcqRel);
    }

    /// `hello` + `events.subscribe{}` on one connection. The boot is the core
    /// id `hello` names; a different id than a seat's is a new boot (C2).
    /// `None` when `hello` or the snapshot failed or named no usable core id.
    pub fn capture(&self) -> Option<HcCapture> {
        let reply = Self::run(self.client.hello_and_snapshot()).ok()?;
        let (hello, snapshot) = reply.value;
        let boot = HcBoot::from_hello(&hello)?;
        let records = snapshot
            .terminals
            .into_iter()
            .filter(|terminal| !terminal.address.contains('/'))
            .collect();
        Some(HcCapture {
            boot,
            seq: snapshot.seq,
            witness: reply.witness,
            records,
        })
    }

    fn unverified(boot: HostBootId, epoch: u64) -> HostSnapshot {
        HostSnapshot {
            boot,
            epoch,
            observation_sequence: 0,
            complete: false,
            enumeration: EnumerationEvidence::CompleteUnverified,
            incarnation: IncarnationEvidence::Unknown,
            targets: Vec::new(),
        }
    }

    /// A present target. `terminal` is `None` for a record waiting for the
    /// core to restart it: the target is shown with no runnable terminal.
    fn target(
        &self,
        record: &HcTerminal,
        boot: &HostBootId,
        epoch: u64,
        sequence: u64,
        started: crate::protocol::time::MonoInstant,
        terminal: Option<TerminalId>,
    ) -> HostObservation {
        let completed = self.clock.monotonic_now();
        HostObservation {
            target: HostTargetId::new(record.address.clone()),
            host_boot: boot.clone(),
            epoch,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: completed,
            provenance: ObservationProvenance::CoherentEnumeration,
            occupant: None,
            ui: ui_state(record),
            focused: false,
            terminal,
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: boot.as_str().to_owned(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("hc-{}", CALL_ID.fetch_add(1, Ordering::Relaxed))),
            connection_epoch: epoch,
            observation_sequence: sequence,
            started_at_mono: started,
            completed_at_mono: completed,
        }
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

/// The same-response bootstrap scope of a herdr-core terminal: one `hello`, then one
/// `terminal.get` pinned to the `hello`'s peer.
pub struct HcBootstrapObserver {
    host: Arc<HcHost>,
}

impl HcBootstrapObserver {
    pub fn new(host: Arc<HcHost>) -> Self {
        Self { host }
    }
}

impl crate::ports::BootstrapObserver for HcBootstrapObserver {
    fn observe_bootstrap_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<crate::ports::BootstrapPaneObservation, ApiError> {
        self.host.observe_bootstrap(target, context)
    }
}

impl HcHost {
    /// One terminal read for a bootstrap attachment. A changed core (boot or peer) is
    /// `StaleHostObservation`; an absent or finished terminal is `NotFound`.
    fn observe_bootstrap(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<crate::ports::BootstrapPaneObservation, ApiError> {
        if context.budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("hc observe_bootstrap"));
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        let started = self.clock.monotonic_now();
        let hello = Self::run(self.client.hello())
            .map_err(|e| ApiError::host_unavailable(e.to_string()))?;
        let boot = HcBoot::from_hello(&hello.value)
            .ok_or_else(|| ApiError::host_unavailable("hc hello names no usable core id"))?;
        let stale = |what: &str| api_error(ErrorCode::StaleHostObservation, what);
        if context
            .expected_boot
            .as_ref()
            .is_some_and(|expected| expected != boot.id())
        {
            return Err(stale("hc core changed before the bootstrap read"));
        }
        let reply = Self::run(
            self.client
                .clone()
                .expecting(hello.witness.clone())
                .terminal_get(target.as_str()),
        )
        .map_err(|e| match e {
            HcError::WitnessChanged | HcError::Witness(_) => {
                stale("hc peer changed before the bootstrap read")
            }
            HcError::Rpc { .. } => api_error(ErrorCode::NotFound, "no such terminal"),
            other => ApiError::host_unavailable(other.to_string()),
        })?;
        let not_found = || api_error(ErrorCode::NotFound, "no such terminal");
        let record = reply.value;
        if record.address != target.as_str() {
            return Err(not_found());
        }
        let terminal = match HcClosure::of(&record) {
            HcClosure::Present => TerminalId::new(record.address.clone()),
            _ => return Err(not_found()),
        };
        let sequence = self.next_observation_sequence();
        let mut observation =
            self.target(&record, boot.id(), epoch, sequence, started, Some(terminal));
        observation.provenance = ObservationProvenance::FreshCurrentTarget;
        crate::ports::BootstrapPaneObservation::try_new_terminal(
            observation,
            hello.value.core.id.clone(),
            reply.witness,
        )
        .map_err(stale)
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
            ports::Placement::NewTerminal { parent } => {
                Some(parent.as_str().to_owned()).filter(|p| p != NO_PARENT)
            }
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

    fn creates_targets_itself(&self) -> bool {
        true
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

    fn answers_terminal_ids(&self) -> bool {
        true
    }

    fn server_label(&self) -> &'static str {
        "herdr-core"
    }

    /// The pid serving the core socket (the `herdr-core serve` process): the peer of a fresh
    /// connection's `LocalEndpointWitness`. `None` when the core does not answer.
    fn server_process(&self) -> Option<u32> {
        let hello = Self::run(self.client.hello()).ok()?;
        Some(hello.witness.peer_pid).filter(|pid| *pid != 0)
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

    /// One terminal read from a fresh capture, as an enumeration would show it. An absent,
    /// finished or unreadable record is `NotFound`: a current read never invents a target.
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        if context.budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("hc observe_current_target"));
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        let started = self.clock.monotonic_now();
        let capture = self
            .capture()
            .ok_or_else(|| ApiError::host_unavailable("hc host: the core is not readable"))?;
        let boot = capture.boot.0.clone();
        if !capture.is_readable() {
            return Err(ApiError::host_unavailable(
                "hc host: the capture is not readable",
            ));
        }
        let not_found = || ApiError::new(ErrorCode::NotFound, "no such terminal");
        let record = capture
            .records
            .iter()
            .find(|record| record.address == target.as_str())
            .ok_or_else(not_found)?;
        let terminal = match HcClosure::of(record) {
            HcClosure::Present => Some(TerminalId::new(record.address.clone())),
            HcClosure::PendingRestart => None,
            HcClosure::Final | HcClosure::Unreadable => return Err(not_found()),
        };
        let sequence = self.next_observation_sequence();
        let mut observation = self.target(record, &boot, epoch, sequence, started, terminal);
        observation.provenance = ObservationProvenance::FreshCurrentTarget;
        Ok(observation)
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
        let session = self.session();
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
                    session: session.clone(),
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

    /// The capture read as a coherent enumeration (D10). `live` records are
    /// the present targets; a kept terminal waiting for a restart is shown
    /// with no terminal (its seat becomes unresolved); a `finished` record is
    /// absent (its seat retires through the pane-close fence). A failed
    /// `hello` or snapshot, or a record state this build cannot read, is
    /// `CompleteUnverified` and closes nothing.
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        if context.budget.cancellation.is_cancelled() {
            return Err(ApiError::cancelled("hc enumerate_targets"));
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        let started = self.clock.monotonic_now();
        let Some(capture) = self.capture() else {
            return Ok(Self::unverified(Self::unknown_boot(), epoch));
        };
        let boot = capture.boot.0.clone();
        if !capture.is_readable() {
            return Ok(Self::unverified(boot, epoch));
        }
        let sequence = self.next_observation_sequence();
        let targets = capture
            .records
            .iter()
            .filter_map(|record| {
                let terminal = match HcClosure::of(record) {
                    HcClosure::Present => Some(TerminalId::new(record.address.clone())),
                    HcClosure::PendingRestart => None,
                    HcClosure::Final | HcClosure::Unreadable => return None,
                };
                Some(self.target(record, &boot, epoch, sequence, started, terminal))
            })
            .collect();
        Ok(HostSnapshot {
            incarnation: IncarnationEvidence::Verified {
                identity: boot.as_str().to_owned(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            boot,
            epoch,
            observation_sequence: sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            targets,
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

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/host/hc_closure.rs"]
mod closure_tests;
