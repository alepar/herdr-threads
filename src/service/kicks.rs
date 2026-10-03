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
}

impl Lane {
    pub const ALL: [Lane; 5] = [
        Lane::Deadlines,
        Lane::Wakes,
        Lane::Observation,
        Lane::Retention,
        Lane::AdmissionObserver,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Lane::Deadlines => "deadline",
            Lane::Wakes => "wake",
            Lane::Observation => "observation",
            Lane::Retention => "retention",
            Lane::AdmissionObserver => "admission-observer",
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
        Self(bits & ((1 << Lane::ALL.len()) - 1))
    }
    pub fn iter(self) -> impl Iterator<Item = Lane> {
        Lane::ALL
            .into_iter()
            .filter(move |lane| self.contains(*lane))
    }
}

/// Tables whose commits wake the wake lane (spec D1).
const WAKE_TABLES: &[&str] = &[
    "wake_work",
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
    "receipts",
    "receipt_state",
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
    "digest_programmatic_warnings",
    "filter_revisions",
    "harness_unattributed",
    "harness_version_evidence",
    "host_instances",
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
    WAKE_TABLES.iter().chain(DEADLINE_TABLES).copied()
}

/// Registry of lane Pacers. A kick to an unregistered lane is a no-op.
#[derive(Default)]
pub struct CommitKicks {
    pacers: Mutex<[Option<Arc<Pacer>>; 5]>,
}

impl CommitKicks {
    fn slots(&self) -> std::sync::MutexGuard<'_, [Option<Arc<Pacer>>; 5]> {
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
