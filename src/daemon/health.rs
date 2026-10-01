//! Health is assembled from injected observations without scanning durable work.
use crate::protocol::results::{
    CapabilityState, ComponentState, HarnessHealth, HarnessState, Health, HealthComponent,
    HealthSettings, HealthState,
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

/// One harness as the daemon observed it on its own `PATH` (the bounded
/// boot observation). Hook installation is per harness environment
/// (`$CLAUDE_CONFIG_DIR`, `$CODEX_HOME`), so `doctor`, run in that
/// environment, checks it; the daemon does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HarnessStatus {
    /// The observation has not completed.
    #[default]
    Unknown,
    /// No executable on the daemon's `PATH`: a note, never a degradation.
    NotInstalled(String),
    /// Present but not admitted (unobservable, unrecognized or a version no
    /// recipe covers): a limitation that degrades Health.
    Refused(String),
    /// Admitted by a recipe whose receipts are cooperative
    /// (`cooperative_top_level`). `live_unverified` marks a schema-matched
    /// admission, which stays listed as a limitation.
    Cooperative {
        detail: String,
        live_unverified: bool,
    },
    /// Admitted by a recipe that declares native-verified receipt.
    Supported(String),
}

impl HarnessStatus {
    pub fn state(&self) -> HarnessState {
        match self {
            Self::Unknown => HarnessState::Unknown,
            Self::NotInstalled(_) | Self::Refused(_) => HarnessState::Unsupported,
            Self::Cooperative { .. } => HarnessState::Cooperative,
            Self::Supported(_) => HarnessState::Supported,
        }
    }

    /// Whether this harness leaves Health `healthy`: cooperative or
    /// supported, or simply not installed here.
    fn acceptable(&self) -> bool {
        matches!(
            self,
            Self::NotInstalled(_) | Self::Cooperative { .. } | Self::Supported(_)
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetirementHealth {
    /// A persisted cleanup cursor exists; it supplies no exact backlog count.
    pub pending: bool,
    pub degraded: bool,
}

pub struct HealthInputs {
    pub instance: Uuid,
    pub boot: Uuid,
    pub database: ComponentStatus,
    pub schema: ComponentStatus,
    pub host: ComponentStatus,
    pub host_version: Option<String>,
    pub current_execution: CapabilityState,
    pub coherent_enumeration: CapabilityState,
    pub safe_prompt: CapabilityState,
    pub receipt_registration: CapabilityState,
    pub scheduler: ComponentStatus,
    /// Completion time of the deadline scheduler's last error-free pass;
    /// None until one completed in this daemon boot.
    pub last_scheduler_tick_at: Option<UtcMillis>,
    /// The installed `codex` on the daemon's `PATH` and its admission line
    /// (listed recipe, schema-matched and live-unverified, or refused).
    pub codex: HarnessStatus,
    /// The installed `claude` on the daemon's `PATH` and its admission.
    pub claude: HarnessStatus,
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
pub const COOPERATIVE_WAKE_LINE: &str = "wake cooperative: prompts only Herdr's detected idle/done claude or codex agent in the seat's terminal, rechecked immediately before submission; native execution and composer contents are unverified";

/// Health's note stating the cooperative receipt basis while a harness is
/// `cooperative`: an admitted recipe, but no native-verified model receipt
/// (no recipe proves one). accept/ACK work through the cooperative caller
/// contract, recorded as `cooperative_top_level` provenance and demonstrated
/// live only on the versions named here (docs/validation/report.md).
pub const COOPERATIVE_RECEIPT_LINE: &str = "receipt cooperative: harness cooperative means an admitted recipe without native-verified receipt; model-issued accept/ACK is recorded as cooperative_top_level, shown live on claude 2.1.285-2.1.286 and codex 0.159.2 (schema-matched, live-unverified)";

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

fn push(lines: &mut Vec<String>, line: String) {
    if lines.len() < 16 {
        lines.push(bounded(&line, 256));
    }
}

/// The harness's limitation or note line.
fn harness_line(
    name: &str,
    status: &HarnessStatus,
    limitations: &mut Vec<String>,
    notes: &mut Vec<String>,
) {
    match status {
        HarnessStatus::Unknown => push(
            limitations,
            format!("harness {name} unknown: the installed version has not been observed yet"),
        ),
        HarnessStatus::NotInstalled(detail) => {
            push(notes, format!("harness {name} not installed: {detail}"))
        }
        HarnessStatus::Refused(detail) => {
            push(limitations, format!("harness {name} unsupported: {detail}"))
        }
        HarnessStatus::Cooperative {
            detail,
            live_unverified,
        } => {
            let line = format!("harness {name}: {detail}");
            if *live_unverified {
                push(limitations, line);
            } else {
                push(notes, line);
            }
        }
        HarnessStatus::Supported(detail) => push(notes, format!("harness {name}: {detail}")),
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
    pub fn unknown(instance: Uuid, boot: Uuid) -> Self {
        Self {
            instance,
            boot,
            database: ComponentStatus::Unknown,
            schema: ComponentStatus::Unknown,
            host: ComponentStatus::Unknown,
            host_version: None,
            current_execution: CapabilityState::Unknown,
            coherent_enumeration: CapabilityState::Unknown,
            safe_prompt: CapabilityState::Unknown,
            receipt_registration: CapabilityState::Unknown,
            scheduler: ComponentStatus::Unknown,
            last_scheduler_tick_at: None,
            codex: HarnessStatus::Unknown,
            claude: HarnessStatus::Unknown,
            retirement: RetirementHealth::default(),
            settings: None,
            last_reconciliation_at: None,
            binding_evidence: None,
            unresolved: None,
        }
    }

    pub fn assemble(self) -> Health {
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
        health.host.version = self
            .host_version
            .as_deref()
            .map(|value| bounded(value, 128));
        health.host.current_execution = self.current_execution;
        health.host.coherent_enumeration = self.coherent_enumeration;
        health.host.safe_prompt = self.safe_prompt;
        health.host.receipt_registration = self.receipt_registration;
        let scheduler = component("scheduler", self.scheduler, &mut health.limitations);
        health.last_scheduler_tick_at = self.last_scheduler_tick_at;
        health.harness = HarnessHealth {
            codex: self.codex.state(),
            claude: self.claude.state(),
        };
        for (name, status) in [("claude", &self.claude), ("codex", &self.codex)] {
            harness_line(name, status, &mut health.limitations, &mut health.notes);
        }
        let cooperative_harness = [&self.claude, &self.codex]
            .iter()
            .any(|status| matches!(status, HarnessStatus::Cooperative { .. }));
        if cooperative_harness {
            push(&mut health.notes, COOPERATIVE_RECEIPT_LINE.into());
        }
        let cooperative_wake = self.safe_prompt == CapabilityState::Supported
            && self.current_execution != CapabilityState::Supported;
        if cooperative_wake {
            push(&mut health.notes, COOPERATIVE_WAKE_LINE.into());
        }
        let wake_available = self.safe_prompt == CapabilityState::Supported
            || self.current_execution == CapabilityState::Supported;
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
        health.unresolved_seats = self.unresolved.as_ref().map(|summary| summary.count);
        let unresolved = self
            .unresolved
            .as_ref()
            .is_some_and(|summary| summary.count > 0);
        if let Some(summary) = self.unresolved.as_ref().filter(|summary| summary.count > 0) {
            // At most 1 + UNRESOLVED_SEAT_SAMPLE lines; with every other
            // source present Health stays within its 16-line bound.
            health.limitations.push(unresolved_seats_line(summary));
            health.limitations.extend(
                summary
                    .sample
                    .iter()
                    .take(crate::ports::UNRESOLVED_SEAT_SAMPLE)
                    .map(unresolved_seat_line),
            );
        }
        if self.retirement.degraded {
            health.limitations.push(
                "retirement cleanup degraded; inspect the exact seat for the retained error".into(),
            );
        }
        // Healthy is the designed operating mode, not native verification:
        // cooperative wake (or native current execution), cooperative or
        // supported harnesses, and no host receipt registration needed
        // (receipts are cooperative). Those facts are notes.
        health.state = if health.database.state == ComponentState::Ready
            && health.schema.state == ComponentState::Ready
            && health.host.reachability == ComponentState::Ready
            && scheduler.state == ComponentState::Ready
            && health.host.coherent_enumeration == CapabilityState::Supported
            && wake_available
            && self.claude.acceptable()
            && self.codex.acceptable()
            && !self.retirement.pending
            && !self.retirement.degraded
            && !lacking_evidence
            && !unresolved
        {
            HealthState::Healthy
        } else {
            HealthState::Degraded
        };
        health
    }
}
