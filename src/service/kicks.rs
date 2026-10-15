//! Commit-change lane kicks (pacer nested spec D1): boundary types (ht-p03.39).
//! ht-p03.9.1 implements the table map, the commit hooks and the origin guard's
//! use at commit time; this module is the registration and origin boundary.
use crate::service::pacer::Pacer;
use std::cell::Cell;
use std::sync::{Arc, Mutex};

/// A daemon lane that can be kicked by a store commit.
///
/// Responsibility: the closed lane set. Owns lane identity, bits and names.
/// Does not own lane workers, or table classification beyond the table map
/// ([`lanes_for_table`]). A new lane is a variant here plus its table mapping.
/// Role: the one vocabulary commit kicks, Pacers and health share.
///
/// `Sync` is the peer-sync lane: it computes and pushes Have deltas to
/// subscribers on journal and sync-log commits, and reloads dial targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lane {
    Deadlines,
    Wakes,
    Observation,
    Retention,
    AdmissionObserver,
    Archival,
    /// Materializes journal entries into canonical state (ht-5fr.3).
    Apply,
    /// Peer sync: Have deltas, push to subscribers, dial-target reload (ht-5fr.5).
    Sync,
}

impl Lane {
    pub const COUNT: usize = 8;
    pub const INTERNAL_ALL: [Lane; Self::COUNT] = [
        Lane::Deadlines,
        Lane::Wakes,
        Lane::Observation,
        Lane::Retention,
        Lane::AdmissionObserver,
        Lane::Archival,
        Lane::Apply,
        Lane::Sync,
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
    /// The lanes that run a worker today, in `ALL` order. `Apply` and `Sync`
    /// are the last lanes and join when their workers register a Pacer
    /// (ht-5fr.3.15, ht-5fr.5).
    pub const WORKERS: [Lane; Self::COUNT - 2] = {
        let mut lanes = [Lane::Deadlines; Self::COUNT - 2];
        let mut i = 0;
        while i < Self::COUNT - 2 {
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
            Lane::Apply => "apply",
            Lane::Sync => "sync",
        }
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// A small set of lanes (bitset over [`Lane::ALL`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaneSet(u16);

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
    pub(crate) fn to_bits(self) -> u16 {
        self.0
    }
    pub(crate) fn from_bits(bits: u16) -> Self {
        Self(bits & ((1 << Lane::COUNT) - 1))
    }
    pub fn iter(self) -> impl Iterator<Item = Lane> {
        Lane::INTERNAL_ALL
            .into_iter()
            .filter(move |lane| self.contains(*lane))
    }
}

/// Tables whose commits wake the apply lane: journal rows await materialization.
const APPLY_TABLES: &[&str] = &["journal"];
/// Tables whose commits wake the sync lane: new journal rows and sync-log
/// changes alter the Have deltas pushed to peers, and a dial-target change
/// makes the Dialer reload its targets.
const SYNC_TABLES: &[&str] = &["journal", "sync_logs", "sync_dial_targets"];
const ARCHIVAL_TABLES: &[&str] = &[
    "archival_instances",
    "channel_archival",
    "seat_archival",
    "channel_handoff_fences",
    "bootstrap_handoffs",
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
    "receipt_warning_origins",
    "in_turn_offer",
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
    "receipt_deadline_backfill",
    "receipt_discovery_backfill",
    "receipt_due_sparse",
    // Catch-up rows move effective receipt deadlines (TRUST-POLICY A6).
    "catch_up",
];
/// Tables that explicitly kick no lane. `local_node`, `herdr_hosts`,
/// `node_directory`, `snapshot_generations` and `snapshot_targets` are classified 'none' by
/// design (observation admission, stage, seal and publish commits kick no
/// lane). A new table must be added to one list; the exhaustive
/// classification test fails until it is.
pub const KNOWN_UNMAPPED: &[&str] = &[
    // Pair identity is passive; its ordinary invitation/membership writes kick their own lanes.
    "direct_threads",
    // Fork repair state (ht-5fr.6.4): freezes kick Lane::Sync through the sync_logs mapping.
    "sync_forks",
    "sync_branches",
    "sync_reissues",
    // Immutable automatic identities accompany the archival fence parent.
    "automatic_handoff_companions",
    "automatic_bootstrap_parents",
    "automatic_bootstrap_keys",
    "automatic_bootstrap_attempts",
    "automatic_bootstrap_attachments",
    "automatic_bootstrap_reports",
    "automatic_bootstrap_recoveries",
    "automatic_handoff_keys",
    // Bootstrap proof/history transitions also update the archival barrier parent.
    "bootstrap_child_keys",
    "bootstrap_attempts",
    "bootstrap_recovery_decisions",
    "bootstrap_attachments",
    "bootstrap_reports",
    "allocation_decisions",
    // Per-binding read cursors (ht-54r.1, read by the per-binding inbox, ht-54r.7): a cursor
    // only moves forward over mail the binding already displayed, so a write retires unread
    // mail and never creates work for a lane.
    "binding_cursors",
    "delivery_observations",
    "digest_notice_offer",
    // Current presentation progress/outbox never schedules its own work.
    "invitation_presentation_consumers",
    "invitation_presentation_items",
    "presentation_confirming_invitations",
    "presentation_warning_fence",
    "presentation_warning_proofs",
    // Producer context refresh is notification-only, never native attention.
    "invitation_context_latest",
    "digest_open_warning_recipients",
    "digest_open_warnings",
    "digest_pending_invitations",
    "digest_pending_manifest_receipts",
    "human_receipt_waivers",
    "human_receipt_reconciliation_bounds",
    "digest_programmatic_warnings",
    "filter_revisions",
    // Immutable send-time cutovers are captured only at schema startup.
    "receipt_send_time_cutovers",
    // Advisory diagnostics do not alter canonical state or schedule work.
    "harness_contract_diagnostics",
    "harness_contract_evidence_v2",
    "harness_runtime_identities",
    "harness_unattributed",
    "harness_unattributed_v2",
    "harness_version_evidence",
    // SQL26 retained rows have no active lane consumer in this composition.
    "lazy_recipients",
    "herdr_hosts",
    "local_node",
    "node_directory",
    // Journal core (ht-5fr.3.4): `journal` maps to Apply and Sync, `sync_logs` to Sync.
    "journal_forked",
    // Reachability is read by queries, never by a lane (ht-5fr.5.4).
    "sync_node_contact",
    "thread_registers",
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
/// ([`kicks_for_commit`]). A table in neither mapped list nor [`KNOWN_UNMAPPED`] maps to
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
    if APPLY_TABLES.contains(&table) {
        set = set.with(Lane::Apply);
    }
    if SYNC_TABLES.contains(&table) {
        set = set.with(Lane::Sync);
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
        .chain(APPLY_TABLES)
        .chain(SYNC_TABLES)
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
    // A check-in that offered notices settles them (ht-j16.33).
    "digest_notice_offer",
    "catch_up",
    "occupant_bindings",
    "seats",
    "recovery_holds",
    "service_notification_recipients",
    "invitation_context_latest",
];

/// True when a commit touching `table` must wake the mod worker.
pub fn notifies_mod(table: &str) -> bool {
    MOD_NOTIFY_TABLES.contains(&table)
}

/// Registry of lane Pacers. A kick to an unregistered lane is a no-op.
#[derive(Default)]
pub struct CommitKicks {
    pacers: Mutex<[Option<Arc<Pacer>>; Lane::COUNT]>,
    mod_observer: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl CommitKicks {
    fn slots(&self) -> std::sync::MutexGuard<'_, [Option<Arc<Pacer>>; Lane::COUNT]> {
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

/// Which of the `sealed` lanes a commit under `origin` kicks.
///
/// Responsibility: the one commit-kick rule. Owns which sealed lanes a commit
/// kicks per origin. Does not run or schedule lanes. New origin rules go here.
/// Role: keeps Apply from kicking itself and Retention from kicking anything:
/// a Retention commit deletes superseded or settled rows, which cannot make a
/// seat sendable or a deadline due; an Apply commit is the materialization of
/// the journal rows that already kicked it.
pub fn kicks_for_commit(origin: Option<Lane>, sealed: LaneSet) -> LaneSet {
    match origin {
        Some(Lane::Retention) => LaneSet::EMPTY,
        Some(Lane::Apply) => sealed.without(Lane::Apply),
        _ => sealed,
    }
}

#[cfg(test)]
#[path = "../../tests/service/kicks_contract.rs"]
mod tests;
