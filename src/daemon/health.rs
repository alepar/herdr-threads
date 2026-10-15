//! Health is assembled from injected observations without scanning durable work.
use crate::protocol::results::{
    ApiError, AttachedHostHealth, CapabilityState, ComponentState, ErrorClass,
    HEALTH_STARTING_NOTE, HarnessHealth, HarnessState, Health, HealthComponent, HealthSettings,
    HealthState, MAX_HEALTH_HOSTS, bound_endpoint_tail,
};
use crate::protocol::time::UtcMillis;
use crate::protocol::wire::PROTOCOL_VERSION;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentStatus {
    Unknown,
    Ready,
    Degraded(String),
    Unsupported(String),
    Unavailable(String),
}

pub use crate::harness::adapter::HarnessStatus;

impl HarnessStatus {
    pub fn state(&self) -> HarnessState {
        match self {
            Self::Unknown => HarnessState::Unknown,
            Self::NotInstalled(_)
            | Self::PresentUnqualified { .. }
            | Self::Refused(_)
            | Self::VersionRefused(_) => HarnessState::Unsupported,
            Self::ContractDeclared { .. } | Self::Cooperative { .. } | Self::Optimistic(_) => {
                HarnessState::Cooperative
            }
            Self::Supported(_) => HarnessState::Supported,
        }
    }

    /// Whether this harness leaves Health `healthy`: cooperative or
    /// supported, absent here, or present without optional qualification.
    fn acceptable(&self) -> bool {
        matches!(
            self,
            Self::NotInstalled(_)
                | Self::PresentUnqualified { .. }
                | Self::VersionRefused(_)
                | Self::ContractDeclared { .. }
                | Self::Cooperative { .. }
                | Self::Supported(_)
                | Self::Optimistic(_)
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetirementHealth {
    /// A persisted cleanup cursor exists; it supplies no exact backlog count.
    pub pending: bool,
    pub degraded: bool,
}

/// One scheduler lane that recorded a failure after its last success: its
/// name and its typed, redacted Health summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneDegradation {
    /// `deadline`, `retention`, ..., or `observation[<host_id>]` /
    /// `wake[<host_id>]` for one attached server's lanes (operator-facing).
    pub lane: String,
    pub summary: String,
    /// The failure's class (its code's default class); `None` when the
    /// failure carries no code or the code has no class.
    pub class: Option<ErrorClass>,
}

/// The class the degraded pointer's remedy is chosen for: `Corrupt` if any
/// lane is, else `Unavailable` if any is, else the first lane's class.
fn pointer_class(lanes: &[LaneDegradation]) -> Option<ErrorClass> {
    [ErrorClass::Corrupt, ErrorClass::Unavailable]
        .into_iter()
        .find(|class| lanes.iter().any(|lane| lane.class == Some(*class)))
        .or_else(|| lanes.first().and_then(|lane| lane.class))
}

/// The most Health lines (limitations, and notes, each) the daemon itself
/// assembles: the wire cap is 16 and the remaining 4 are headroom for lines
/// a later producer adds. Past it the lowest-priority lines are folded, never silently lost.
pub const HEALTH_LINE_BUDGET: usize = 12;

/// More degraded lanes than this fold into one summary line.
pub const LANE_LINES_BEFORE_FOLD: usize = 2;

pub struct HealthInputs {
    pub instance: crate::node::NodeUuid,
    pub boot: Uuid,
    pub database: ComponentStatus,
    pub schema: ComponentStatus,
    pub host: ComponentStatus,
    pub host_version: Option<String>,
    /// "untested Herdr X.Y.Z; ..." for a release newer than the tested
    /// range: a limitation that does not degrade Health.
    pub host_release_warning: Option<String>,
    pub current_execution: CapabilityState,
    pub coherent_enumeration: CapabilityState,
    pub safe_prompt: CapabilityState,
    pub receipt_registration: CapabilityState,
    pub scheduler: ComponentStatus,
    /// Completion time of the deadline scheduler's last error-free pass;
    /// None until one completed in this daemon boot.
    pub last_scheduler_tick_at: Option<UtcMillis>,
    /// Executable availability and declared core cooperation on daemon PATH,
    /// in registry order. A harness with a legacy contract is always present
    /// (`Unknown` until observed); any other registered harness only once
    /// observed, so an optional harness the observer has not reached yet is
    /// not an unknown core harness.
    pub harnesses: Vec<(String, HarnessStatus)>,
    pub retirement: RetirementHealth,
    pub settings: Option<HealthSettings>,
    /// Completion time of the last reconciliation pass over a verified
    /// coherent publication; None until one completed in this daemon boot.
    pub last_reconciliation_at: Option<UtcMillis>,
    /// The writer's startup backfill count with `still_lacking` recomputed
    /// when Health is read (never frozen at boot). None means no durable
    /// writer reported one, never "nothing lacking".
    pub binding_evidence: Option<crate::ports::BindingEvidenceStartup>,
    /// Unresolved seats now. None means no reliable observation, never zero.
    pub unresolved: Option<crate::ports::UnresolvedSeatSummary>,
    /// The elected daemon's log (`logs::daemon_log_path`); None when the
    /// provider is not bound to an instance. Rendered only as the
    /// `degraded: <remedy>` pointer (`remedy()`'s lane-degraded line) while a
    /// lane is degraded.
    pub log_path: Option<std::path::PathBuf>,
    /// Lanes whose latest pass failed (a failure recorded after their last
    /// success), in `Lane::ALL` order.
    pub degraded_lanes: Vec<LaneDegradation>,
    /// Guarded seat transitions the store refused during reconciliation since
    /// boot (skipped, never fatal).
    pub transitions_refused: u64,
    /// Actual contract failure lines: at most one per harness in the 24-hour
    /// Health window. Metadata/version/manifest advice alone adds no limitation.
    /// When the evidence store could not be read this is the single
    /// [`HARNESS_EVIDENCE_UNAVAILABLE_LINE`] (see [`harness_version_lines`]),
    /// so Health is `degraded` rather than silently clean.
    pub harness_version_lines: Vec<String>,
    /// Milliseconds since this daemon's Health provider was built at boot,
    /// on its monotonic clock. None (no start time supplied) keeps the
    /// ordinary classification: never `starting`, no overdue line.
    pub since_start_ms: Option<u64>,
    /// One input per attached Herdr server, ascending by host id (from
    /// `HostSessions::health_entries`). Empty means a store-only daemon:
    /// the `host`, `host_version`, capability and `last_reconciliation_at`
    /// fields above then stand as given. Non-empty, `assemble` replaces them
    /// with the aggregate over these entries (worst reachability, weakest
    /// capability, earliest reconciliation).
    pub hosts: Vec<HostHealthInput>,
}

/// One attached Herdr server's observations, as `HostSessions::health_entries`
/// reads them from the session. Operator-only (see `AttachedHostHealth`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostHealthInput {
    pub host_id: i64,
    /// The server's socket path, unbounded; `assemble` bounds what it prints.
    pub endpoint: String,
    pub status: ComponentStatus,
    pub version: Option<String>,
    pub current_execution: CapabilityState,
    pub coherent_enumeration: CapabilityState,
    pub safe_prompt: CapabilityState,
    pub last_reconciliation_at: Option<UtcMillis>,
}

fn reach_state(status: &ComponentStatus) -> ComponentState {
    match status {
        ComponentStatus::Unknown => ComponentState::Unknown,
        ComponentStatus::Ready => ComponentState::Ready,
        ComponentStatus::Degraded(_) | ComponentStatus::Unsupported(_) => ComponentState::Degraded,
        ComponentStatus::Unavailable(_) => ComponentState::Unavailable,
    }
}

/// Larger is worse.
fn reach_rank(state: ComponentState) -> u8 {
    match state {
        ComponentState::Ready => 0,
        ComponentState::Unknown => 1,
        ComponentState::Degraded => 2,
        ComponentState::Unavailable => 3,
    }
}

/// The weakest capability: `Unsupported`, then `Unknown`, then `Supported`.
fn weakest(states: impl Iterator<Item = CapabilityState>) -> CapabilityState {
    states
        .min_by_key(|state| match state {
            CapabilityState::Unsupported => 0,
            CapabilityState::Unknown => 1,
            CapabilityState::Supported => 2,
        })
        .unwrap_or(CapabilityState::Unknown)
}

/// One `Herdr server <endpoint> unreachable` limitation per unreachable
/// attached server, folded past [`LANE_LINES_BEFORE_FOLD`] into
/// `<n> Herdr servers unreachable`. About 40 bytes a line plus the endpoint
/// (at most 128), bounded to 256 by `push`. The cause (the worst server's
/// detail) rides on the aggregate `host unavailable: ...` line.
fn unreachable_lines(hosts: &[HostHealthInput]) -> Vec<String> {
    let unreachable: Vec<&HostHealthInput> = hosts
        .iter()
        .filter(|host| matches!(host.status, ComponentStatus::Unavailable(_)))
        .collect();
    if unreachable.len() > LANE_LINES_BEFORE_FOLD {
        return vec![format!("{} Herdr servers unreachable", unreachable.len())];
    }
    unreachable
        .iter()
        .map(|host| {
            format!(
                "Herdr server {} unreachable",
                bound_endpoint_tail(&host.endpoint)
            )
        })
        .collect()
}

/// How long after daemon start Health may report `starting` while no host
/// reconciliation has completed. The first capture has a 5 s budget and the
/// first pass normally completes within 1-5 s of start; 30 s leaves room for
/// a slow host or a loaded machine and still surfaces a daemon that never
/// reconciles as `degraded` well before anyone would wait on it.
pub const STARTUP_GRACE_MS: u64 = 30_000;

/// The limitation that explains a `degraded` Health once the startup bound
/// passed with no completed reconciliation and nothing else wrong.
pub const FIRST_RECONCILIATION_OVERDUE_LINE: &str =
    "reconciliation: none completed within 30 s of daemon start";

/// The tail of the `harness NAME unknown: ...` limitation listed until the
/// bounded boot observation of that harness finishes.
const HARNESS_UNOBSERVED_SUFFIX: &str =
    " unknown: executable availability has not been observed yet";

/// Whether a Health limitation only says something has not been observed
/// yet (`harness NAME unknown: ...`). Doctor labels these `pending:` instead
/// of `issue:` while the daemon reports itself starting.
pub fn is_pending_observation_line(line: &str) -> bool {
    line.strip_prefix("harness ")
        .and_then(|rest| rest.strip_suffix(HARNESS_UNOBSERVED_SUFFIX))
        .is_some_and(|name| !name.is_empty() && !name.contains(' '))
}

/// The limitation shown while the harness version evidence cannot be read.
pub const HARNESS_EVIDENCE_UNAVAILABLE_LINE: &str = "harness contract diagnostics unavailable: the evidence store could not be read, so actual input failures would not show here";

/// Health's version lines from the provider's answer: the lines as read, or
/// the one unavailable limitation when the evidence store could not be read.
pub fn harness_version_lines(answer: &Result<Vec<String>, ApiError>) -> Vec<String> {
    match answer {
        Ok(lines) => lines.clone(),
        Err(_) => vec![HARNESS_EVIDENCE_UNAVAILABLE_LINE.to_owned()],
    }
}

fn bounded(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let prefix_limit = limit.saturating_sub('…'.len_utf8());
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        if index + ch.len_utf8() > prefix_limit {
            break;
        }
        end = index + ch.len_utf8();
    }
    let mut output = String::with_capacity(limit);
    output.push_str(&text[..end]);
    output.push('…');
    output
}

/// Health's statement of the cooperative wake basis when current native
/// execution is unverified.
#[cfg(test)]
pub const COOPERATIVE_WAKE_LINE: &str = "wake cooperative: prompts only Herdr's detected idle/done claude or codex agent in the seat's terminal, rechecked immediately before submission; native execution and composer contents are unverified";

/// Declared core contracts grant cooperative caller claims, never native receipt
/// or exact-runtime qualification. Historical captures remain diagnostic evidence.
pub fn cooperative_receipt_line() -> String {
    cooperative_receipt_line_for(crate::harness::registry::builtins())
}

pub fn cooperative_receipt_line_for(registry: &crate::harness::registry::Registry) -> String {
    let admissions = registry
        .registrations()
        .iter()
        .map(|registration| {
            format!(
                "{} {}",
                registration.metadata().id,
                registration
                    .receipt_admission_summary()
                    .unwrap_or_else(|| "admission unknown".into())
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    bounded(
        &format!(
            "receipt cooperative: declared input contracts; accept/ACK is recorded as cooperative_top_level; runtime and native model receipt unverified; contracts: {admissions}"
        ),
        256,
    )
}

pub fn cooperative_wake_line_for(registry: &crate::harness::registry::Registry) -> String {
    let ids: Vec<_> = registry
        .registrations()
        .iter()
        .filter(|registration| !registration.metadata().host_kinds.is_empty())
        .map(|registration| registration.metadata().id)
        .collect();
    let kinds = match ids.as_slice() {
        [] => "agent kind unavailable".to_owned(),
        [id] => (*id).to_owned(),
        [first, last] => format!("{first} or {last}"),
        many => format!(
            "{} or {}",
            many[..many.len() - 1].join(", "),
            many[many.len() - 1]
        ),
    };
    bounded(
        &format!(
            "wake cooperative: prompts only Herdr's detected idle/done {kinds} agent in the seat's terminal, rechecked immediately before submission; native execution and composer contents are unverified"
        ),
        256,
    )
}

/// Health's limitation when the host offers neither native current-execution
/// authority nor the cooperative wake prompt: no wake can be delivered.
pub const NO_WAKE_LINE: &str = "wake unavailable: the host offers neither native current execution nor a safe cooperative prompt";

fn component(
    label: &str,
    status: ComponentStatus,
    limitations: &mut Vec<String>,
) -> HealthComponent {
    let (state, kind, detail) = match status {
        ComponentStatus::Unknown => {
            return HealthComponent {
                state: ComponentState::Unknown,
                detail: None,
            };
        }
        ComponentStatus::Ready => {
            return HealthComponent {
                state: ComponentState::Ready,
                detail: None,
            };
        }
        ComponentStatus::Degraded(detail) => (ComponentState::Degraded, "degraded", detail),
        ComponentStatus::Unsupported(detail) => (ComponentState::Degraded, "unsupported", detail),
        ComponentStatus::Unavailable(detail) => {
            (ComponentState::Unavailable, "unavailable", detail)
        }
    };
    let prefix = format!("{label} {kind}: ");
    if limitations.len() < 16 {
        limitations.push(format!(
            "{}{}",
            prefix,
            bounded(&detail, 256 - prefix.len())
        ));
    }
    HealthComponent {
        state,
        detail: Some(bounded(&detail, 256)),
    }
}

/// The one line a fold of more than two degraded lanes renders.
fn folded_lanes_text(lanes: &[LaneDegradation], log: Option<&std::path::Path>) -> String {
    let names: Vec<&str> = lanes.iter().map(|lane| lane.lane.as_str()).collect();
    let mut text = format!("{} lanes degraded ({})", lanes.len(), names.join(", "));
    if let Some(log) = log {
        text.push_str(&format!(": see {}", log.display()));
    }
    text
}

/// Keeps `lines` within [`HEALTH_LINE_BUDGET`]: the unresolved-seat sample
/// lines at `sample_at..sample_at + sample_len` go first (the summary line
/// before them already states the count), then the tail folds into one line
/// counting what it hides.
fn fit_budget(lines: &mut Vec<String>, sample_at: usize, sample_len: usize) {
    if lines.len() > HEALTH_LINE_BUDGET {
        let drop = (lines.len() - HEALTH_LINE_BUDGET).min(sample_len);
        lines.drain(sample_at + sample_len - drop..sample_at + sample_len);
    }
    if lines.len() > HEALTH_LINE_BUDGET {
        let hidden = lines.len() - (HEALTH_LINE_BUDGET - 1);
        lines.truncate(HEALTH_LINE_BUDGET - 1);
        lines.push(format!(
            "{hidden} more lines not shown; run `herdr-threads doctor` and see the daemon log"
        ));
    }
}

fn push(lines: &mut Vec<String>, line: String) {
    if lines.len() < 16 {
        lines.push(bounded(&line, 256));
    }
}

/// The harness's limitation or note line.
///
/// Ordinary declared operation reports executable availability and cooperative
/// claims. Metadata absence is informational; actual contract failures arrive
/// separately through harness_version_lines. Historical admission variants remain
/// available for explicit diagnostic callers, never ordinary runtime gates.
fn harness_line(
    name: &str,
    status: &HarnessStatus,
    limitations: &mut Vec<String>,
    notes: &mut Vec<String>,
) {
    match status {
        HarnessStatus::Unknown => push(
            limitations,
            format!("harness {name}{HARNESS_UNOBSERVED_SUFFIX}"),
        ),
        HarnessStatus::NotInstalled(detail) => {
            push(notes, format!("harness {name} not installed: {detail}"))
        }
        HarnessStatus::Refused(detail) => {
            push(limitations, format!("harness {name} unsupported: {detail}"))
        }
        HarnessStatus::PresentUnqualified { detail }
        | HarnessStatus::ContractDeclared { detail } => {
            push(notes, format!("harness {name}: {detail}"));
        }
        HarnessStatus::VersionRefused(_)
        | HarnessStatus::Cooperative { .. }
        | HarnessStatus::Supported(_)
        | HarnessStatus::Optimistic(_) => {}
    }
}

/// One deterministic line for Health, doctor and the daemon log.
pub fn binding_evidence_line(report: crate::ports::BindingEvidenceStartup) -> String {
    if report.still_lacking > 0 {
        format!(
            "store startup binding evidence: backfilled {}, still lacking {}; those seats cannot \
             be reconfirmed automatically after a host invalidation: repair each with \
             `seat rebind SEAT --pane PANE --operator`",
            report.backfilled, report.still_lacking
        )
    } else {
        format!(
            "store startup binding evidence: backfilled {}, still lacking 0",
            report.backfilled
        )
    }
}

/// Health's binding-evidence line: the startup backfill count and the
/// still-lacking count as rechecked when Health was read. At most 256 bytes
/// (the Health limitation bound) for every count.
pub fn binding_evidence_health_line(report: crate::ports::BindingEvidenceStartup) -> String {
    let line = if report.still_lacking > 0 {
        format!(
            "binding evidence: backfilled {} at store startup, still lacking {} now; after a \
             host invalidation those seats stay unresolved until their agent registers again or \
             `seat rebind SEAT --pane PANE --operator`",
            report.backfilled, report.still_lacking
        )
    } else {
        format!(
            "binding evidence: backfilled {} at store startup, still lacking 0 now",
            report.backfilled
        )
    };
    bounded(&line, 256)
}

/// The unresolved-seats guidance line (at most 256 bytes for every count).
pub fn unresolved_seats_line(summary: &crate::ports::UnresolvedSeatSummary) -> String {
    bounded(
        &format!(
            "unresolved seats: {}; their lifecycle check-ins are refused until resolved; a \
             host-invalidated seat reconfirms when a coherent snapshot shows its pane again; \
             repair one that stays unresolved: `seat rebind SEAT --pane PANE --operator`",
            summary.count
        ),
        256,
    )
}

/// One bounded line naming one of the oldest unresolved seats.
pub fn unresolved_seat_line(sample: &crate::ports::UnresolvedSeatSample) -> String {
    use crate::ports::UnresolvedReason;
    bounded(
        &format!(
            "unresolved seat {} on {} ({})",
            sample.seat.as_str(),
            sample
                .target
                .as_ref()
                .map(|target| target.as_str())
                .unwrap_or("no pane"),
            match sample.reason {
                Some(UnresolvedReason::HostInvalidation) => "host_invalidation",
                Some(UnresolvedReason::Other) => "other",
                None => "unknown",
            }
        ),
        256,
    )
}

impl HealthInputs {
    pub fn unknown(instance: crate::node::NodeUuid, boot: Uuid) -> Self {
        Self {
            instance,
            boot,
            database: ComponentStatus::Unknown,
            schema: ComponentStatus::Unknown,
            host: ComponentStatus::Unknown,
            host_version: None,
            host_release_warning: None,
            current_execution: CapabilityState::Unknown,
            coherent_enumeration: CapabilityState::Unknown,
            safe_prompt: CapabilityState::Unknown,
            receipt_registration: CapabilityState::Unknown,
            scheduler: ComponentStatus::Unknown,
            last_scheduler_tick_at: None,
            harnesses: crate::harness::registry::builtins()
                .registrations()
                .iter()
                .filter(|r| r.legacy_contract_id().is_some())
                .map(|r| (r.metadata().id.to_owned(), HarnessStatus::Unknown))
                .collect(),
            retirement: RetirementHealth::default(),
            settings: None,
            last_reconciliation_at: None,
            binding_evidence: None,
            unresolved: None,
            log_path: None,
            degraded_lanes: Vec::new(),
            transitions_refused: 0,
            harness_version_lines: Vec::new(),
            since_start_ms: None,
            hosts: Vec::new(),
        }
    }

    /// Sets one harness's status, keeping the registry order of the rows
    /// already present; a harness not yet present is appended.
    pub fn set_harness(&mut self, id: &str, status: HarnessStatus) {
        match self.harnesses.iter_mut().find(|(known, _)| known == id) {
            Some((_, slot)) => *slot = status,
            None => self.harnesses.push((id.to_owned(), status)),
        }
    }

    /// The status recorded for `id`, if any.
    pub fn harness(&self, id: &str) -> Option<&HarnessStatus> {
        self.harnesses
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, status)| status)
    }

    /// The v1 `HarnessHealth` wire projection: the one place that names the
    /// two harnesses the frozen shape carries. A harness without a row is
    /// `unknown`.
    fn v1_harness_health(&self) -> HarnessHealth {
        let state = |id: &str| {
            self.harness(id)
                .map_or(HarnessState::Unknown, HarnessStatus::state)
        };
        HarnessHealth {
            codex: state("codex"),
            claude: state("claude"),
        }
    }

    pub fn assemble(mut self) -> Health {
        // Attached servers: the aggregate replaces the single-host fields and
        // each server gets its own entry and, when not ready, its own line.
        self.hosts.truncate(MAX_HEALTH_HOSTS);
        let hosts = std::mem::take(&mut self.hosts);
        let mut every_host_wakes = None;
        if !hosts.is_empty() {
            // The worst server's status (the first on a tie) stands for the
            // aggregate, including its detail on the `host ...` line.
            self.host = hosts
                .iter()
                .fold(None::<&HostHealthInput>, |worst, host| match worst {
                    Some(w)
                        if reach_rank(reach_state(&w.status))
                            >= reach_rank(reach_state(&host.status)) =>
                    {
                        Some(w)
                    }
                    _ => Some(host),
                })
                .map_or(ComponentStatus::Unknown, |host| host.status.clone());
            self.host_version = hosts.iter().find_map(|host| host.version.clone());
            self.current_execution = weakest(hosts.iter().map(|host| host.current_execution));
            self.coherent_enumeration = weakest(hosts.iter().map(|host| host.coherent_enumeration));
            self.safe_prompt = weakest(hosts.iter().map(|host| host.safe_prompt));
            self.last_reconciliation_at = hosts
                .iter()
                .map(|host| host.last_reconciliation_at)
                .min()
                .flatten();
            every_host_wakes = Some(hosts.iter().all(|host| {
                host.safe_prompt == CapabilityState::Supported
                    || host.current_execution == CapabilityState::Supported
            }));
        }
        let v1_harness = self.v1_harness_health();
        let mut health = Health::unknown(
            self.instance.to_string(),
            self.boot.to_string(),
            env!("CARGO_PKG_VERSION").into(),
            PROTOCOL_VERSION,
        );
        health.settings = self.settings;
        health.last_reconciliation_at = self.last_reconciliation_at;
        health.database = component("database", self.database, &mut health.limitations);
        health.schema = component("schema", self.schema, &mut health.limitations);
        let host = component("host", self.host, &mut health.limitations);
        health.host.reachability = host.state;
        for line in unreachable_lines(&hosts) {
            push(&mut health.limitations, line);
        }
        health.hosts = hosts
            .iter()
            .map(|host| AttachedHostHealth {
                host_id: host.host_id,
                endpoint: bound_endpoint_tail(&host.endpoint),
                reachability: reach_state(&host.status),
                version: host.version.as_deref().map(|value| bounded(value, 128)),
                coherent_enumeration: host.coherent_enumeration,
                safe_prompt: host.safe_prompt,
                current_execution: host.current_execution,
                last_reconciliation_at: host.last_reconciliation_at,
            })
            .collect();
        health.host.version = self
            .host_version
            .as_deref()
            .map(|value| bounded(value, 128));
        if let Some(warning) = &self.host_release_warning {
            push(&mut health.limitations, warning.clone());
        }
        health.host.current_execution = self.current_execution;
        health.host.coherent_enumeration = self.coherent_enumeration;
        health.host.safe_prompt = self.safe_prompt;
        health.host.receipt_registration = self.receipt_registration;
        // More than two degraded lanes fold into one summary line; fewer keep
        // one line each (the first through the scheduler component).
        let folded = self.degraded_lanes.len() > LANE_LINES_BEFORE_FOLD;
        let scheduler_status = if folded {
            ComponentStatus::Degraded(folded_lanes_text(
                &self.degraded_lanes,
                self.log_path.as_deref(),
            ))
        } else {
            self.scheduler
        };
        let scheduler = component("scheduler", scheduler_status, &mut health.limitations);
        if !folded {
            for lane in self.degraded_lanes.iter().skip(1) {
                push(
                    &mut health.limitations,
                    format!("scheduler degraded: {}", lane.summary),
                );
            }
            if let (false, Some(path)) = (self.degraded_lanes.is_empty(), &self.log_path) {
                push(
                    &mut health.limitations,
                    format!(
                        "degraded: {}",
                        crate::daemon::remedy::remedy(
                            pointer_class(&self.degraded_lanes),
                            &crate::daemon::remedy::RemedyContext::LaneDegraded {
                                log: path.clone(),
                            },
                        )
                    ),
                );
            }
        }
        health.last_scheduler_tick_at = self.last_scheduler_tick_at;
        health.harness = v1_harness;
        for (name, status) in &self.harnesses {
            harness_line(name, status, &mut health.limitations, &mut health.notes);
        }
        for line in &self.harness_version_lines {
            push(&mut health.limitations, line.clone());
        }
        let cooperative_harness = self.harnesses.iter().any(|(_, status)| {
            matches!(
                status,
                HarnessStatus::ContractDeclared { .. }
                    | HarnessStatus::Cooperative { .. }
                    | HarnessStatus::Optimistic(_)
            )
        });
        if cooperative_harness {
            push(&mut health.notes, cooperative_receipt_line());
        }
        let cooperative_wake = self.safe_prompt == CapabilityState::Supported
            && self.current_execution != CapabilityState::Supported;
        if cooperative_wake {
            push(
                &mut health.notes,
                cooperative_wake_line_for(crate::harness::registry::builtins()),
            );
        }
        let wake_available = every_host_wakes.unwrap_or(
            self.safe_prompt == CapabilityState::Supported
                || self.current_execution == CapabilityState::Supported,
        );
        if !wake_available
            && self.current_execution != CapabilityState::Unknown
            && self.safe_prompt != CapabilityState::Unknown
        {
            push(&mut health.limitations, NO_WAKE_LINE.into());
        }
        if self.retirement.pending {
            health
                .limitations
                .push("retirement cleanup pending; inspect the exact seat for progress".into());
        }
        let lacking_evidence = self
            .binding_evidence
            .is_some_and(|report| report.still_lacking > 0);
        if let Some(report) = self
            .binding_evidence
            .filter(|report| report.backfilled > 0 || report.still_lacking > 0)
        {
            health
                .limitations
                .push(binding_evidence_health_line(report));
        }
        if self.transitions_refused > 0 {
            push(
                &mut health.notes,
                format!(
                    "reconciliation: the store refused {} seat transition(s) since boot; those \
                     seats were skipped, the others reconciled",
                    self.transitions_refused
                ),
            );
        }
        health.unresolved_seats = self.unresolved.as_ref().map(|summary| summary.count);
        let unresolved = self
            .unresolved
            .as_ref()
            .is_some_and(|summary| summary.count > 0);
        let (mut sample_at, mut sample_len) = (0, 0);
        if let Some(summary) = self.unresolved.as_ref().filter(|summary| summary.count > 0) {
            // At most 1 + UNRESOLVED_SEAT_SAMPLE lines; with every other
            // source present Health stays within its 16-line bound.
            health.limitations.push(unresolved_seats_line(summary));
            sample_at = health.limitations.len();
            health.limitations.extend(
                summary
                    .sample
                    .iter()
                    .take(crate::ports::UNRESOLVED_SEAT_SAMPLE)
                    .map(unresolved_seat_line),
            );
            sample_len = health.limitations.len() - sample_at;
        }
        if self.retirement.degraded {
            health.limitations.push(
                "retirement cleanup degraded; inspect the exact seat for the retained error".into(),
            );
        }
        // Facts that are fine whatever has been observed so far.
        let settled = health.database.state == ComponentState::Ready
            && health.schema.state == ComponentState::Ready
            && scheduler.state == ComponentState::Ready
            && self.degraded_lanes.is_empty()
            && !self.retirement.pending
            && !self.retirement.degraded
            && !lacking_evidence
            && !unresolved
            && self.harness_version_lines.is_empty();
        // Healthy is the designed operating mode, not native verification:
        // cooperative wake (or native current execution), cooperative or
        // supported harnesses, and no host receipt registration needed
        // (receipts are cooperative). Those facts are notes.
        let healthy = settled
            && health.host.reachability == ComponentState::Ready
            && health.host.coherent_enumeration == CapabilityState::Supported
            && wake_available
            && self.harnesses.iter().all(|(_, status)| status.acceptable());
        // Not healthy only because the host, a harness or a wake capability
        // has not been observed yet, before any reconciliation completed:
        // `starting` within the startup bound, explained as overdue after it.
        let awaiting_first_observation = !healthy
            && settled
            && self.last_reconciliation_at.is_none()
            && matches!(
                health.host.reachability,
                ComponentState::Ready | ComponentState::Unknown
            )
            && matches!(
                health.host.coherent_enumeration,
                CapabilityState::Supported | CapabilityState::Unknown
            )
            && (wake_available
                || self.safe_prompt == CapabilityState::Unknown
                || self.current_execution == CapabilityState::Unknown)
            && self
                .harnesses
                .iter()
                .all(|(_, status)| status.acceptable() || matches!(status, HarnessStatus::Unknown));
        match self.since_start_ms {
            Some(elapsed) if awaiting_first_observation && elapsed < STARTUP_GRACE_MS => {
                health.notes.insert(0, HEALTH_STARTING_NOTE.into());
            }
            Some(_) if awaiting_first_observation => {
                health
                    .limitations
                    .insert(0, FIRST_RECONCILIATION_OVERDUE_LINE.into());
                // No unresolved-seat sample exists here (`settled`), so the
                // sample range need not shift.
            }
            _ => {}
        }
        fit_budget(&mut health.limitations, sample_at, sample_len);
        fit_budget(&mut health.notes, 0, 0);
        health.state = if healthy {
            HealthState::Healthy
        } else {
            HealthState::Degraded
        };
        health
    }
}

#[cfg(test)]
#[path = "../../tests/daemon/health_budget.rs"]
mod health_budget;
#[cfg(test)]
#[path = "../../tests/daemon/health_harness_state.rs"]
mod health_harness_state;
#[cfg(test)]
#[path = "../../tests/daemon/health_hosts.rs"]
mod health_hosts;
#[cfg(test)]
#[path = "../../tests/daemon/health_optimistic.rs"]
mod health_optimistic;
#[cfg(test)]
#[path = "../../tests/daemon/health_starting.rs"]
mod health_starting;
