//! Commit-change lane kicks (pacer nested spec D1): boundary types (ht-p03.39).
//! ht-p03.9.1 implements the table map, the commit hooks and the origin guard's
//! use at commit time; this module is the registration and origin boundary.
use crate::service::pacer::Pacer;
use std::cell::Cell;
use std::sync::{Arc, Mutex};

/// A daemon lane that can be kicked by a store commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lane {
    Deadlines,
    Wakes,
    Observation,
    Retention,
    AdmissionObserver,
    Archival,
}

impl Lane {
    pub const COUNT: usize = 6;
    pub const INTERNAL_ALL: [Lane; 6] = [
        Lane::Deadlines,
        Lane::Wakes,
        Lane::Observation,
        Lane::Retention,
        Lane::AdmissionObserver,
        Lane::Archival,
    ];
    pub const ALL: [Lane; Self::COUNT] = {
        let mut lanes = [Lane::Deadlines; Self::COUNT];
        let mut i = 0;
        while i < Self::COUNT {
            lanes[i] = Self::INTERNAL_ALL[i];
            i += 1;
        }
        lanes
    };

    pub fn name(self) -> &'static str {
        match self {
            Lane::Deadlines => "deadline",
            Lane::Wakes => "wake",
            Lane::Observation => "observation",
            Lane::Retention => "retention",
            Lane::AdmissionObserver => "admission-observer",
            Lane::Archival => "archival",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// A small set of lanes (bitset over [`Lane::ALL`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaneSet(u8);

impl LaneSet {
    pub const EMPTY: LaneSet = LaneSet(0);

    pub fn with(self, lane: Lane) -> Self {
        Self(self.0 | lane.bit())
    }
    pub fn without(self, lane: Lane) -> Self {
        Self(self.0 & !lane.bit())
    }
    pub fn contains(self, lane: Lane) -> bool {
        self.0 & lane.bit() != 0
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub(crate) fn to_bits(self) -> u8 {
        self.0
    }
    pub(crate) fn from_bits(bits: u8) -> Self {
        Self(bits & ((1 << Lane::INTERNAL_ALL.len()) - 1))
    }
    pub fn iter(self) -> impl Iterator<Item = Lane> {
        Lane::INTERNAL_ALL
            .into_iter()
            .filter(move |lane| self.contains(*lane))
    }
}

const ARCHIVAL_TABLES: &[&str] = &[
    "archival_instances",
    "channel_archival",
    "seat_archival",
    "channel_handoff_fences",
];
/// Tables whose commits wake the wake lane (spec D1).
const WAKE_TABLES: &[&str] = &[
    "wake_work",
    // Retained batching deadlines and their canonical clear triggers change
    // when a seat's ordinary attention can be reconsidered.
    "wake_batches",
    "seats",
    "occupant_bindings",
    "seat_availability",
    "warning_recipients",
    "warning_offer",
];
/// Tables whose commits wake the deadline lane (spec D1).
const DEADLINE_TABLES: &[&str] = &[
    "work_jobs",
    "warning_jobs",
    "retirements",
    "invitations",
    "invitation_cancellations",
    "invitation_rejections",
    "receipts",
    "receipt_state",
    // Catch-up rows move effective receipt deadlines (TRUST-POLICY A6).
    "catch_up",
];
/// Tables that explicitly kick no lane. `host_instances`,
/// `snapshot_generations` and `snapshot_targets` are classified 'none' by
/// design (observation admission, stage, seal and publish commits kick no
/// lane). A new table must be added to one list; the exhaustive
/// classification test fails until it is.
pub const KNOWN_UNMAPPED: &[&str] = &[
    "allocation_decisions",
    "delivery_observations",
    "digest_notice_offer",
    "digest_open_warning_recipients",
    "digest_open_warnings",
    "digest_pending_invitations",
    "digest_pending_manifest_receipts",
    "human_receipt_waivers",
    "human_receipt_reconciliation_bounds",
    "digest_programmatic_warnings",
    "filter_revisions",
    // Advisory diagnostics do not alter canonical state or schedule work.
    "harness_contract_diagnostics",
    "harness_contract_evidence_v2",
    "harness_runtime_identities",
    "harness_unattributed",
    "harness_unattributed_v2",
    "harness_version_evidence",
    "host_instances",
    // Passive inbox progress never schedules attention or deadlines.
    "lazy_recipients",
    "membership_intervals",
    "memberships",
    "messages",
    "observed_targets",
    "operations",
    "prepared_recipients",
    "prepared_unavailable_warnings",
    "recovery_baseline_releases",
    "recovery_baseline_targets",
    "recovery_holds",
    "requirement_episodes",
    "retirement_audits",
    "schema_identity",
    "send_manifests",
    "send_preparations",
    "service_authors",
    "service_notification_preparations",
    "service_notification_publications",
    "service_notification_recipients",
    "snapshot_generations",
    "snapshot_targets",
    // Open and clear writes also enqueue warning_jobs, which kick deadlines.
    "warning_conditions",
    "warning_close_sweeps",
    // Thread summaries (epic ht-1ip): derived data; a stored block's deadline
    // effect is written to `catch_up`, which kicks the deadline lane.
    "summary_blocks",
    "summary_items",
    "summary_job_durations",
    "summary_jobs",
    "summary_transitions",
    "threads",
    "sqlite_sequence",
];

/// Static table to lanes map (spec D1). Retention and AdmissionObserver map
/// no table. For Retention that is an explicit decision (ht-p03.12.6): it
/// runs on its 60 s safety tick and `has_more` re-runs, because pruning
/// is never urgent and a commit-driven kick would turn every publication
/// into a retention commit; its own commits kick no lane
/// ([`origin_discards_kicks`]). A table in neither mapped list nor [`KNOWN_UNMAPPED`] maps to
/// `EMPTY` and trips a debug assertion.
pub fn lanes_for_table(table: &str) -> LaneSet {
    let mut set = LaneSet::EMPTY;
    if ARCHIVAL_TABLES.contains(&table) {
        set = set.with(Lane::Archival);
    }
    if WAKE_TABLES.contains(&table) {
        set = set.with(Lane::Wakes);
    }
    if DEADLINE_TABLES.contains(&table) {
        set = set.with(Lane::Deadlines);
    }
    debug_assert!(
        set != LaneSet::EMPTY || KNOWN_UNMAPPED.contains(&table),
        "table {table} is not classified in service::kicks"
    );
    set
}

/// Every table named by the D1 map (test and audit support).
pub fn mapped_tables() -> impl Iterator<Item = &'static str> {
    WAKE_TABLES
        .iter()
        .chain(DEADLINE_TABLES)
        .chain(ARCHIVAL_TABLES)
        .copied()
}

/// Tables whose commits may change what a live mod watch channel shows or
/// whether it stays valid (spec D2): every attention source (ordinary and
/// lazy receipts, invitations and their cancellations, warnings, notices,
/// catch-up releases) and every channel-closing fact (binding, seat and
/// recovery-hold changes). A commit touching one of them wakes the mod
/// worker, which re-reads the seats with a live channel.
pub const MOD_NOTIFY_TABLES: &[&str] = &[
    "wake_work",
    "receipts",
    "receipt_state",
    "lazy_recipients",
    // Lazy rows become visible when their manifest publishes.
    "send_manifests",
    "invitations",
    "invitation_cancellations",
    "invitation_rejections",
    "warning_recipients",
    "warning_offer",
    // A warning is pending (digest backlog) from its job, before attribution.
    "warning_jobs",
    "warning_conditions",
    "digest_programmatic_warnings",
    "catch_up",
    "occupant_bindings",
    "seats",
    "recovery_holds",
    "service_notification_recipients",
];

/// True when a commit touching `table` must wake the mod worker.
pub fn notifies_mod(table: &str) -> bool {
    MOD_NOTIFY_TABLES.contains(&table)
}

/// Registry of lane Pacers. A kick to an unregistered lane is a no-op.
#[derive(Default)]
pub struct CommitKicks {
    pacers: Mutex<[Option<Arc<Pacer>>; 6]>,
    mod_observer: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl CommitKicks {
    fn slots(&self) -> std::sync::MutexGuard<'_, [Option<Arc<Pacer>>; 6]> {
        self.pacers.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers `lane`'s Pacer; called once per lane at worker start.
    pub fn register(&self, lane: Lane, pacer: Arc<Pacer>) {
        self.slots()[lane as usize] = Some(pacer);
    }

    /// Kicks the Pacer of every registered lane in `lanes`.
    pub fn kick(&self, lanes: LaneSet) {
        // Collect first so no Pacer lock is taken under the registry lock.
        let targets: Vec<Arc<Pacer>> = {
            let slots = self.slots();
            lanes
                .iter()
                .filter_map(|lane| slots[lane as usize].clone())
                .collect()
        };
        for pacer in targets {
            pacer.kick();
        }
    }

    /// Registers the observer a commit touching [`MOD_NOTIFY_TABLES`] calls
    /// (after the writer guard is released). It only wakes the mod worker; it
    /// must never do store work on the committing thread.
    pub fn set_mod_observer(&self, observer: Arc<dyn Fn() + Send + Sync>) {
        *self.mod_observer.lock().unwrap_or_else(|e| e.into_inner()) = Some(observer);
    }

    /// Calls the mod observer, outside any lock of this registry.
    pub fn notify_mod(&self) {
        let observer = self
            .mod_observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(observer) = observer {
            observer();
        }
    }

    pub fn registered(&self, lane: Lane) -> bool {
        self.slots()[lane as usize].is_some()
    }

    /// The Pacer registered for `lane` (test support).
    #[cfg(any(test, feature = "test-support"))]
    pub fn pacer(&self, lane: Lane) -> Option<Arc<Pacer>> {
        self.slots()[lane as usize].clone()
    }
}

thread_local! {
    static ORIGIN: Cell<Option<Lane>> = const { Cell::new(None) };
}

/// Commit-origin guard: set on lane-owned std threads only. Tokio workers and
/// `spawn_blocking` threads never hold one, so their commits count as request
/// origin.
#[must_use = "the origin applies only while the guard is held"]
pub struct LaneOrigin {
    previous: Option<Lane>,
}

pub fn enter_lane(lane: Lane) -> LaneOrigin {
    LaneOrigin {
        previous: ORIGIN.with(|origin| origin.replace(Some(lane))),
    }
}

impl Drop for LaneOrigin {
    fn drop(&mut self) {
        ORIGIN.with(|origin| origin.set(self.previous));
    }
}

/// The calling thread's lane origin; `None` means request origin.
pub fn current_origin() -> Option<Lane> {
    ORIGIN.with(Cell::get)
}

/// Commits sealed under these origins discard their pending kick set: deleting
/// superseded or settled rows cannot make a seat sendable or a deadline due.
pub fn origin_discards_kicks(origin: Option<Lane>) -> bool {
    matches!(origin, Some(Lane::Retention))
}

#[cfg(test)]
#[path = "../../tests/service/kicks_contract.rs"]
mod tests;
