//! SQLite connection and decision transaction ownership.

use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use crate::service::kicks::{self, LaneSet};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, ffi};
use std::{
    ffi::{c_char, c_int, c_void},
    ops::Deref,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
    },
    time::Duration,
};

use super::schema::{self, NodeSeed};
use crate::node::NodeIdentity;
use crate::sync::facts::FactRegistry;
use crate::sync::hlc::Hlc;

/// Stamps the `herdr_hosts` row of `endpoint` as attached at `now`, creating it
/// for this node when it does not exist, and returns its `host_id`. Update
/// first and insert only when absent, so a re-attach never advances the
/// AUTOINCREMENT sequence.
pub(crate) fn upsert_host(conn: &Connection, endpoint: &str, now: i64) -> rusqlite::Result<i64> {
    use rusqlite::OptionalExtension;
    if let Some(host_id) = conn
        .query_row(
            "UPDATE herdr_hosts SET last_attached_at=?2 WHERE endpoint=?1 RETURNING host_id",
            rusqlite::params![endpoint, now],
            |row| row.get(0),
        )
        .optional()?
    {
        return Ok(host_id);
    }
    conn.query_row(
        "INSERT INTO herdr_hosts(node_id,endpoint,created_at,last_attached_at) \
         SELECT id,?1,?2,?2 FROM local_node RETURNING host_id",
        rusqlite::params![endpoint, now],
        |row| row.get(0),
    )
}

/// Decides, from the calling thread's lane origin, whether a store access
/// fails (test hook).
#[cfg(any(test, feature = "test-support"))]
pub type LaneFault =
    std::sync::Arc<dyn Fn(Option<crate::service::kicks::Lane>) -> Option<ApiError> + Send + Sync>;

/// Idle read-only query connections kept for reuse. A new connection parses
/// the whole schema on its first statement (milliseconds of CPU), and one
/// send's publication, materialization and wake each open query connections.
const IDLE_QUERY_CONNECTIONS: usize = 4;

type IdleQueries = Arc<std::sync::Mutex<Vec<Connection>>>;

/// Free space below which ingest stores nothing (`Headroom`): 256 MiB.
pub const INGEST_HEADROOM_BYTES: u64 = 256 * 1024 * 1024;

/// The journal's per-daemon runtime, installed once by `journal::open` into
/// [`StoreContext`].
///
/// Owns: the node's `Arc<NodeIdentity>`, the `Mutex<Hlc>`, the
/// `&'static FactRegistry`, the ingest headroom threshold, the incarnation
/// draw source and the "journal is open" gate (a store has a `SyncState` only
/// after `open`). Hides: whether a given store has a journal; callers ask
/// [`StoreContext::sync`], which answers `Unsupported "journal not open"` (a
/// programming error, distinct from every store and fact error) before open.
/// Must not: be locked outside `src/store/journal.rs` (only Journal ticks or
/// folds the Hlc, and only inside a writer transaction), or be replaced after
/// install (a registry upgrade is a reopen with a new `StoreContext`).
/// Role: read by Journal, ApplyEngine (registry) and the commit guard. New
/// journal runtime state (the own-history hold, ht-5fr.3.16) is a field here.
pub struct SyncState {
    pub(crate) identity: Arc<NodeIdentity>,
    pub(crate) hlc: std::sync::Mutex<Hlc>,
    pub(crate) registry: &'static FactRegistry,
    pub(crate) headroom_bytes: AtomicU64,
    /// Most `journal_forked` rows `serve_branch` walks before giving up.
    pub(crate) branch_walk_bound: AtomicU64,
    pub(crate) incarnation_rng: std::sync::Mutex<XorShift>,
    /// When (UTC ms) the hourly skew line of each log was last written.
    pub(crate) skew_logged: std::sync::Mutex<std::collections::HashMap<crate::sync::LogKey, i64>>,
    /// Set when entries may be waiting to apply (startup, an ingest that stored
    /// a pending row, an apply pass that hit its limit); cleared by the Apply
    /// lane before each kicked pass. Authoring leaves it clear: it applies
    /// inline, so a lane kick it causes finds nothing to do and takes no
    /// writer turn.
    pub(crate) apply_hint: Arc<AtomicBool>,
    /// While active, `apply_pass` applies only the active self log.
    pub(crate) own_history_hold: Arc<OwnHistoryHold>,
    /// Set by every `journal::begin_incarnation` after `open` (not by `open`'s
    /// own bootstrap): this daemon process has rotated its active log since it
    /// started. `restore_successor` needs it clear (incarnations design 6).
    pub(crate) rotated_since_start: AtomicBool,
}

impl SyncState {
    /// Whether this process has started a new incarnation since `open`.
    pub fn rotated_since_start(&self) -> bool {
        self.rotated_since_start.load(Ordering::Acquire)
    }

    /// The free-space floor below which ingest stores nothing.
    pub fn headroom_bytes(&self) -> u64 {
        self.headroom_bytes.load(Ordering::Relaxed)
    }

    /// The most `journal_forked` rows `serve_branch` walks before it answers
    /// `None`.
    pub fn branch_walk_bound(&self) -> u64 {
        self.branch_walk_bound.load(Ordering::Relaxed)
    }

    /// The hold SyncCoordinator and IncarnationsWorker set; shared by `Arc`.
    pub fn own_history_hold(&self) -> &Arc<OwnHistoryHold> {
        &self.own_history_hold
    }

    /// Marks entries as possibly waiting to apply.
    pub fn set_apply_hint(&self) {
        self.apply_hint.store(true, Ordering::Release);
    }

    /// Clears the hint and returns whether it was set.
    pub fn take_apply_hint(&self) -> bool {
        self.apply_hint.swap(false, Ordering::AcqRel)
    }
}

/// Longest the own-history hold filters: 5 minutes, in monotonic ms.
pub const OWN_HISTORY_HOLD_MAX: u64 = 5 * 60 * 1000;

/// Who holds the own-history hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldSource {
    /// SyncCoordinator: this node's links have not all been tried yet.
    Links,
    /// IncarnationsWorker: pulling back this node's own old incarnations.
    Branches,
}

#[derive(Default)]
struct HoldState {
    links: bool,
    branches: bool,
    started: Option<MonoInstant>,
    expiry_logged: bool,
}

impl HoldState {
    fn any(&self) -> bool {
        self.links || self.branches
    }
}

/// Makes `apply_pass` wait with every log but the active self log.
///
/// Responsibility: owns the two hold flags ([`HoldSource`]), the hold's start
/// instant and the 5-minute cap ([`OWN_HISTORY_HOLD_MAX`]). Why: catch-up
/// suppression ([`crate::store::apply::ApplyContext::delivery_suppressed`]) is settled-only, so a
/// message may be offered only after the settlement that would suppress it
/// has been ingested. After a wipe or restore this node's own settlements sit
/// in its own old incarnations, which it pulls back from peers; applying a
/// foreign or own-old head before that pull finishes would redeliver settled
/// history (root spec section 7, journal-core step 3a). So while a flag is set
/// and the hold is younger than the cap, those heads stay `pending` (no
/// `blocked_on` write). Guarantees: a fresh node with no flag set never waits,
/// and no hold outlasts 5 minutes, so a silent peer cannot stall apply
/// forever (a settlement arriving after the cap cannot suppress; accepted
/// limit in TRUST-POLICY). The start instant is set when the first flag sets
/// while both are clear and re-arms only after both have cleared. Must not:
/// touch the store. Role: set by SyncCoordinator and IncarnationsWorker,
/// read by `apply_pass` and by the FactRoute restore-window authoring hold
/// ([`active`](Self::active)). When the last flag clears or the cap expires
/// it sets the shared apply hint and runs the kick hook (a `Lane::Apply`
/// kick), so held entries apply without waiting for the safety tick.
pub struct OwnHistoryHold {
    state: std::sync::Mutex<HoldState>,
    apply_hint: Arc<AtomicBool>,
    kick: std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    expiry_logs: AtomicU64,
}

impl OwnHistoryHold {
    /// A hold that sets `apply_hint` when it lifts. `SyncState` builds the
    /// daemon's one; a coordinator test rig builds its own.
    pub fn new(apply_hint: Arc<AtomicBool>) -> Self {
        Self {
            state: std::sync::Mutex::default(),
            apply_hint,
            kick: std::sync::Mutex::new(None),
            #[cfg(test)]
            expiry_logs: AtomicU64::new(0),
        }
    }

    /// Installs the callback that kicks `Lane::Apply` when the hold lifts.
    pub fn set_release_kick(&self, kick: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut slot) = self.kick.lock() {
            *slot = Some(kick);
        }
    }

    /// Sets or clears `source`'s flag at `now`.
    pub fn set(&self, source: HoldSource, on: bool, now: MonoInstant) {
        let released = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let was = state.any();
            match source {
                HoldSource::Links => state.links = on,
                HoldSource::Branches => state.branches = on,
            }
            if !was && state.any() {
                state.started = Some(now);
                state.expiry_logged = false;
            }
            let released = was && !state.any();
            if released {
                state.started = None;
            }
            released
        };
        if released {
            self.release();
        }
    }

    /// True while a flag is set and the hold is younger than the cap. The
    /// call that first sees the cap passed logs once and kicks the Apply lane.
    pub fn active(&self, now: MonoInstant) -> bool {
        let expired = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(started) = state.started.filter(|_| state.any()) else {
                return false;
            };
            if now.0.saturating_sub(started.0) < OWN_HISTORY_HOLD_MAX {
                return true;
            }
            !std::mem::replace(&mut state.expiry_logged, true)
        };
        if expired {
            eprintln!("sync: own-history hold expired after 5 min; applying foreign entries");
            #[cfg(test)]
            self.expiry_logs.fetch_add(1, Ordering::SeqCst);
            self.release();
        }
        false
    }

    #[cfg(test)]
    pub(crate) fn expiry_logs(&self) -> u64 {
        self.expiry_logs.load(Ordering::SeqCst)
    }

    fn release(&self) {
        self.apply_hint.store(true, Ordering::Release);
        let kick = self.kick.lock().ok().and_then(|slot| slot.clone());
        if let Some(kick) = kick {
            kick();
        }
    }
}

/// A tiny xorshift64* generator behind incarnation draws.
///
/// Unless fixed by [`StoreContext::seed_incarnations`] it re-seeds its state
/// from `getrandom` at every draw, so a memory-preserving copy of a running
/// daemon (a VM snapshot, a restored image) does not repeat the draw of its
/// original.
#[derive(Clone, Debug)]
pub struct XorShift {
    state: u64,
    reseed: bool,
}

impl XorShift {
    /// A generator that re-seeds from the OS at every draw.
    pub fn from_os() -> Self {
        Self {
            state: 0,
            reseed: true,
        }
    }

    /// A reproducible sequence from `seed`; never re-seeds.
    pub fn fixed(seed: u64) -> Self {
        Self {
            state: seed | 1,
            reseed: false,
        }
    }

    fn next_u64(&mut self) -> u64 {
        if self.reseed {
            let mut bytes = [0u8; 8];
            // A failing OS source falls back to the current state and time
            // rather than a fixed value.
            if getrandom::fill(&mut bytes).is_ok() {
                self.state = u64::from_le_bytes(bytes) | 1;
            } else {
                self.state ^= std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0x9e37_79b9, |d| d.as_nanos() as u64)
                    | 1;
            }
        }
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// An incarnation drawn uniformly from `1..=u32::MAX`.
    pub fn draw_incarnation(&mut self) -> u32 {
        loop {
            let value = (self.next_u64() >> 32) as u32;
            if value != 0 {
                return value;
            }
        }
    }
}

pub struct StoreContext {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    /// The store's view of attached hosts (see [`StoreContext::hosts`]).
    hosts: Arc<super::hosts::HostIndex>,
    /// The identity a fresh store is created with; an existing store keeps its
    /// own. Production sets it from the loaded node key; test builds default to
    /// the fixed test node.
    node_seed: Option<NodeSeed>,
    /// Test hook: the key seed `SqliteStore::new` opens the journal with, so
    /// every test store runs against an opened journal. Test builds default to
    /// the fixed test node; `with_node_seed` (production identity) clears it,
    /// because the daemon calls `journal::open` itself with its own key.
    #[cfg(any(test, feature = "test-support"))]
    auto_journal: Option<[u8; 32]>,
    idle_queries: IdleQueries,
    /// The last writer open's binding-evidence verification result.
    binding_evidence: std::sync::Mutex<Option<crate::ports::BindingEvidenceStartup>>,
    /// Set once the daemon's early journal-open writer produced the startup
    /// report: later writer opens (the store's own) no longer replace it, so
    /// the report keeps the backfill the first open did.
    binding_evidence_pinned: AtomicBool,
    /// Seats whose latest binding still lacked evidence after the last
    /// writer open. Only these are rechecked for Health; the set only
    /// shrinks, because every new live binding must carry evidence.
    binding_evidence_lacking: std::sync::Mutex<Vec<String>>,
    /// Per hidden snapshot stage this process is staging: the captured active
    /// publication and how many leading staged targets equal that
    /// publication's rows (see `seats::stage_snapshot_targets`). It can only
    /// spare a send-eligibility bump: publication bumps whenever the entry is
    /// missing or does not cover every target (restart, eviction, another
    /// process staging, a differing target).
    snapshot_stage_matches: std::sync::Mutex<Vec<SnapshotStageMatch>>,
    /// The journal runtime, installed once by `journal::open` (see
    /// [`SyncState`]).
    sync: OnceLock<SyncState>,
    /// Test hook: the fixed incarnation sequence a later `journal::open` (or
    /// the open `SyncState`) draws from (see `seed_incarnations`).
    #[cfg(any(test, feature = "test-support"))]
    incarnation_seed: std::sync::Mutex<Option<u64>>,
    /// Test hook: every writer open attaches the fixture Herdr host (see
    /// `with_test_host`).
    #[cfg(any(test, feature = "test-support"))]
    test_host: bool,
    /// Test hook: the server every writer open attaches and publishes (see
    /// `with_attached_host`).
    #[cfg(any(test, feature = "test-support"))]
    attached_host: Option<crate::protocol::ids::HostLocator>,
    /// Test hook: fails store access on lane-owned threads (see `set_lane_fault`).
    #[cfg(any(test, feature = "test-support"))]
    lane_fault: std::sync::Mutex<Option<LaneFault>>,
    /// Test hook: writers commit with `synchronous=NORMAL` (see
    /// `relax_commit_durability`).
    #[cfg(any(test, feature = "test-support"))]
    relaxed_durability: AtomicBool,
    #[cfg(test)]
    setup_busy_signal: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    setup_end_signal: std::sync::Mutex<Option<std::sync::mpsc::Sender<Option<ErrorCode>>>>,
    #[cfg(test)]
    rollback_journal: std::sync::atomic::AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecisionInstant {
    pub utc: UtcMillis,
    pub monotonic: MonoInstant,
}

/// Test-support builds only: a process started with this set to `1` (every
/// child tagged by `test_support::spawn`, so test-spawned CLIs and daemons)
/// commits with `synchronous=NORMAL`, as do the library's own unit tests
/// (see `require_commit_durability`). Hundreds of test daemons each fsyncing
/// every WAL commit made the suite disk-bound on a loaded machine; a test
/// cannot observe the difference, which only matters on power loss.
/// Production builds never compile it and always commit with `FULL`.
#[cfg(any(test, feature = "test-support"))]
pub const TEST_RELAXED_DURABILITY_ENV: &str = "HT_TEST_RELAXED_DURABILITY";

/// Process-local evidence that a hidden snapshot stage repeats its captured
/// active publication so far: `matched` leading staged targets each equal the
/// active publication's row for the same target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotStageMatch {
    pub stage: String,
    pub captured_active: String,
    pub matched: i64,
}

/// Stages one process tracks at once. The observation lane stages one at a
/// time; an overflow forgets every entry, which only costs bumps.
const SNAPSHOT_STAGE_MATCHES: usize = 8;

impl StoreContext {
    /// Records the committed outcome of one staged slice. `slice_matched` is
    /// whether every target of the slice equals the captured active
    /// publication's row; `offset..staged` is the slice. An entry survives
    /// only while it covers a contiguous all-matching prefix from offset 0.
    pub(crate) fn note_snapshot_stage_slice(
        &self,
        stage: &str,
        captured_active: Option<&str>,
        offset: i64,
        staged: i64,
        slice_matched: bool,
    ) {
        let Ok(mut entries) = self.snapshot_stage_matches.lock() else {
            return;
        };
        let prior = entries.iter().position(|entry| entry.stage == stage);
        let prior = prior.map(|index| entries.swap_remove(index));
        let Some(captured_active) = captured_active.filter(|_| slice_matched) else {
            return;
        };
        let continues = offset == 0
            || prior.is_some_and(|prior| {
                prior.captured_active == captured_active && prior.matched == offset
            });
        if !continues {
            return;
        }
        if entries.len() >= SNAPSHOT_STAGE_MATCHES {
            entries.clear();
        }
        entries.push(SnapshotStageMatch {
            stage: stage.to_owned(),
            captured_active: captured_active.to_owned(),
            matched: staged,
        });
    }

    /// Removes and returns the stage's match evidence. Publication and
    /// discard both end a stage's tracking.
    pub(crate) fn take_snapshot_stage_match(&self, stage: &str) -> Option<SnapshotStageMatch> {
        let mut entries = self.snapshot_stage_matches.lock().ok()?;
        let index = entries.iter().position(|entry| entry.stage == stage)?;
        Some(entries.swap_remove(index))
    }

    pub fn new(path: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self::with_host_index(path, clock, Arc::default())
    }

    /// [`StoreContext::new`] sharing `hosts` with `HostSessions`, the only
    /// writer of the index. `new` gives the context a private, empty index.
    pub fn with_host_index(
        path: PathBuf,
        clock: Arc<dyn Clock>,
        hosts: Arc<super::hosts::HostIndex>,
    ) -> Self {
        Self {
            path,
            clock,
            hosts,
            #[cfg(any(test, feature = "test-support"))]
            node_seed: Some(crate::test_support::test_node_seed()),
            #[cfg(not(any(test, feature = "test-support")))]
            node_seed: None,
            #[cfg(any(test, feature = "test-support"))]
            auto_journal: Some(crate::test_support::TEST_NODE_SEED),
            idle_queries: Arc::default(),
            sync: OnceLock::new(),
            #[cfg(any(test, feature = "test-support"))]
            incarnation_seed: std::sync::Mutex::new(None),
            binding_evidence: std::sync::Mutex::new(None),
            binding_evidence_pinned: AtomicBool::new(false),
            binding_evidence_lacking: std::sync::Mutex::new(Vec::new()),
            snapshot_stage_matches: std::sync::Mutex::new(Vec::new()),
            #[cfg(any(test, feature = "test-support"))]
            test_host: false,
            #[cfg(any(test, feature = "test-support"))]
            attached_host: None,
            #[cfg(any(test, feature = "test-support"))]
            lane_fault: std::sync::Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            relaxed_durability: AtomicBool::new(
                cfg!(test)
                    || std::env::var_os(TEST_RELAXED_DURABILITY_ENV).is_some_and(|v| v == "1"),
            ),
            #[cfg(test)]
            setup_busy_signal: std::sync::Mutex::new(None),
            #[cfg(test)]
            setup_end_signal: std::sync::Mutex::new(None),
            #[cfg(test)]
            rollback_journal: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The directory holding the database file, for the free-space probe.
    pub(crate) fn database_dir(&self) -> PathBuf {
        match self.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    /// Test hook: the free-space floor below which ingest stores nothing
    /// (`u64::MAX` makes every ingest `Headroom`). A no-op before
    /// `journal::open`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_ingest_headroom(&self, bytes: u64) {
        if let Some(state) = self.sync.get() {
            state.headroom_bytes.store(bytes, Ordering::Relaxed);
        }
    }

    /// Test hook: the most `journal_forked` rows `serve_branch` walks before it
    /// answers `None`. A no-op before `journal::open`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_branch_walk_bound(&self, steps: u64) {
        if let Some(state) = self.sync.get() {
            state.branch_walk_bound.store(steps, Ordering::Relaxed);
        }
    }

    /// The journal runtime, or `Unsupported "journal not open"` before
    /// `journal::open` installed it.
    pub fn sync(&self) -> Result<&SyncState, ApiError> {
        self.sync
            .get()
            .ok_or_else(|| api_error(ErrorCode::Unsupported, "journal not open"))
    }

    /// Installs the journal runtime; `false` when one is already installed
    /// (it is never replaced). Only `journal::open` calls this.
    pub(crate) fn install_sync(&self, state: SyncState) -> bool {
        self.sync.set(state).is_ok()
    }

    /// Whether `journal::open` has installed the journal runtime.
    pub fn journal_open(&self) -> bool {
        self.sync.get().is_some()
    }

    /// The incarnation source a new `SyncState` starts with: re-seeding from
    /// the OS, unless a test fixed the sequence.
    pub(crate) fn new_incarnation_rng(&self) -> XorShift {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(seed) = self.incarnation_seed.lock().ok().and_then(|slot| *slot) {
            return XorShift::fixed(seed);
        }
        XorShift::from_os()
    }

    /// Test hook: fixes the incarnation sequence to `seed` and turns
    /// re-seeding off, for this context before or after `journal::open`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_incarnations(&self, seed: u64) {
        if let Ok(mut slot) = self.incarnation_seed.lock() {
            *slot = Some(seed);
        }
        if let Some(state) = self.sync.get()
            && let Ok(mut rng) = state.incarnation_rng.lock()
        {
            *rng = XorShift::fixed(seed);
        }
    }

    /// The store's read view of attached hosts.
    ///
    /// Owns: the one answer to "is this claim's Herdr server attached, and
    /// what is its `HostId`" (`HostIndex::attached_id`; a miss is
    /// `HostUnavailable`). Does not own: the `herdr_hosts` rows, a host's
    /// session or reachability, or any per-seat routing (the seat row's
    /// `host_id` does that). Role: replaces the single-host seam.
    /// Only `cooperative_claim` and C1 continuity resolve a locator through
    /// it; every other query takes its host from the seat row or from the
    /// worker that owns the host. Read-only here; only `HostSessions` (and
    /// the interim single-host composition) publishes into it.
    pub fn hosts(&self) -> &Arc<super::hosts::HostIndex> {
        &self.hosts
    }

    /// The identity a fresh store file is created with. An existing store
    /// keeps the identity it was created with; a fresh store with no seed is an
    /// error, never a silent default.
    #[must_use]
    pub fn with_node_seed(mut self, seed: NodeSeed) -> Self {
        self.node_seed = Some(seed);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.auto_journal = None;
        }
        self
    }

    /// Test hook: a store whose node is the one `NodeIdentity::from_seed(seed)`
    /// names, with the journal opened under that key by `SqliteStore::new`, so
    /// `local_node.id` and the journal identity agree. For multi-store and
    /// oracle tests that need a distinct identity.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_journal_seed(mut self, seed: [u8; 32]) -> Self {
        let uuid = crate::node::NodeIdentity::from_seed(seed).uuid();
        self.node_seed = Some(NodeSeed {
            uuid,
            tag: crate::node::tag_for(uuid, 0),
            tag_k: 0,
        });
        self.auto_journal = Some(seed);
        self
    }

    /// The key seed `SqliteStore::new` auto-opens the journal with, if any.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn auto_journal(&self) -> Option<[u8; 32]> {
        self.auto_journal
    }

    /// Test hook: a store whose Herdr host is the fixture host
    /// (`test_support::TEST_HOST_ENDPOINT`). Every writer open upserts its
    /// `herdr_hosts` row and publishes it into the context's [`HostIndex`]
    /// under `test_support::test_host_locator()`, so a fixture sets host
    /// columns with `UPDATE herdr_hosts` and never inserts the row. A store
    /// built without it has no attached host (`HostUnavailable`).
    ///
    /// [`HostIndex`]: super::hosts::HostIndex
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_test_host(mut self) -> Self {
        self.test_host = true;
        // Published at once (the first row of a fresh store is `test_host_id()`), so a context
        // that is handed a connection some other opener created still resolves the fixture host.
        self.hosts.publish(
            crate::test_support::test_host_locator(),
            crate::test_support::test_host_id(),
        );
        self
    }

    /// Test hook: a store whose Herdr host is the server at `locator`, for a fixture whose claims
    /// name that server instead of the fixture host. Every writer open upserts its
    /// `herdr_hosts` row and publishes it into the context's [`HostIndex`] under `locator`, as the
    /// daemon does (the row, then the index entry); the order of builder calls does not matter.
    ///
    /// [`HostIndex`]: super::hosts::HostIndex
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_attached_host(mut self, locator: &crate::protocol::ids::HostLocator) -> Self {
        self.attached_host = Some(locator.clone());
        self
    }

    /// Test hook: the fixture host is published as id 1, the id the first
    /// `herdr_hosts` row of a fresh store gets, before that row exists. A test
    /// inserts the row later (see `test_support::attach_test_host_to_connection`)
    /// to model the moment a host is first admitted.
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn with_pending_test_host(self) -> Self {
        self.hosts.publish(
            crate::test_support::test_host_locator(),
            crate::ports::HostId::from_row(1),
        );
        self
    }

    /// Attaches the Herdr server at `endpoint`: upserts its `herdr_hosts` row
    /// for this node (stamping `last_attached_at`) and returns its id. It
    /// creates and verifies the store when it is new but opens no writer, so
    /// the writer's startup invariant report is still produced once, by the
    /// store that opens next. It does not publish into the [`HostIndex`]: that
    /// is the composition's step, once the host's workers run.
    ///
    /// [`HostIndex`]: super::hosts::HostIndex
    pub fn attach_host(&self, endpoint: &str) -> Result<i64, ApiError> {
        let conn = self.open_initialized()?;
        let now = self.clock.utc_now().0;
        upsert_host(&conn, endpoint, now).map_err(store_error)
    }

    /// Attaches the remembered host: the `herdr_hosts` row with the greatest
    /// `last_attached_at` whose endpoint satisfies `is_live` (stamped), or
    /// `None` when no row qualifies and nothing is attached. Returns the host id and its endpoint.
    pub fn attach_remembered(
        &self,
        is_live: &dyn Fn(&str) -> bool,
    ) -> Result<Option<(i64, String)>, ApiError> {
        let conn = self.open_initialized()?;
        let mut statement = conn
            .prepare(
                "SELECT endpoint FROM herdr_hosts ORDER BY last_attached_at DESC, host_id DESC",
            )
            .map_err(store_error)?;
        let endpoints = statement
            .query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(store_error)?;
        drop(statement);
        drop(conn);
        let Some(endpoint) = endpoints.into_iter().find(|endpoint| is_live(endpoint)) else {
            return Ok(None);
        };
        let host_id = self.attach_host(&endpoint)?;
        Ok(Some((host_id, endpoint)))
    }

    /// Test hook: installs (or clears) the lane fault `lane_fault_check` consults.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_lane_fault(&self, fault: Option<LaneFault>) {
        *self.lane_fault.lock().unwrap() = fault;
    }

    /// Test hook for latency fixtures: writers opened afterwards commit with
    /// `synchronous=NORMAL` (no fsync per WAL commit) instead of `FULL`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn relax_commit_durability(&self) {
        self.relaxed_durability.store(true, Ordering::SeqCst);
    }

    /// Unit-test builds relax every store by default (each in-process store
    /// test's fsyncs made the library suite disk-bound under load); a test
    /// that asserts the production `FULL` setting opts back in here.
    #[cfg(test)]
    pub(crate) fn require_commit_durability(&self) {
        self.relaxed_durability.store(false, Ordering::SeqCst);
    }

    /// Fails with the installed fault's error for the calling thread's lane
    /// origin. Called where a lane's store access begins (writer turn, query
    /// connection); a no-op in ordinary builds.
    pub(super) fn lane_fault_check(&self) -> Result<(), ApiError> {
        #[cfg(any(test, feature = "test-support"))]
        {
            let fault = self.lane_fault.lock().ok().and_then(|f| f.clone());
            if let Some(error) = fault.and_then(|f| f(crate::service::kicks::current_origin())) {
                return Err(error);
            }
        }
        Ok(())
    }

    /// Test hook: signal once when the next query connection first waits on a
    /// SQLite lock, so tests can act while the read is provably blocked.
    #[cfg(test)]
    pub(crate) fn signal_next_query_busy(&self, signal: std::sync::mpsc::Sender<()>) {
        *self.setup_busy_signal.lock().unwrap() = Some(signal);
    }

    /// Test hook: when the next query connection is dropped, send its typed
    /// budget outcome (`Cancelled`, `DeadlineExceeded`, or `None` if live).
    #[cfg(test)]
    pub(crate) fn signal_next_query_end(&self, signal: std::sync::mpsc::Sender<Option<ErrorCode>>) {
        *self.setup_end_signal.lock().unwrap() = Some(signal);
    }

    /// Test hook: writers keep rollback journaling instead of WAL, so a
    /// competing `BEGIN EXCLUSIVE` can block query connections.
    #[cfg(test)]
    pub(crate) fn use_rollback_journal(&self) {
        self.rollback_journal
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// Failpoint scope: one database file, so parallel tests stay isolated.
    #[cfg(any(test, feature = "test-support"))]
    pub fn failpoint_scope(&self) -> String {
        self.path.display().to_string()
    }

    /// A connection to the store, created and verified when it is new: foreign
    /// keys on, `schema::initialize` run. The shared
    /// start of the writer open and the host attach.
    fn open_initialized(&self) -> Result<Connection, ApiError> {
        #[cfg(test)]
        if self.node_seed == Some(crate::test_support::test_node_seed()) {
            fresh_schema_template::seed(&self.path);
        }
        let conn = Connection::open(&self.path).map_err(store_error)?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(store_error)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(store_error)?;
        schema::initialize_with(&conn, || self.clock.utc_now(), self.node_seed.as_ref())?;
        #[cfg(any(test, feature = "test-support"))]
        if self.test_host {
            crate::test_support::attach_test_host(self, &conn);
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(locator) = &self.attached_host {
            let id = upsert_host(&conn, locator.as_str(), self.clock.utc_now().0)
                .map_err(store_error)?;
            self.hosts
                .publish(locator.clone(), crate::ports::HostId::from_row(id));
        }
        Ok(conn)
    }

    pub fn open_writer(&self) -> Result<Connection, ApiError> {
        let conn = self.open_initialized()?;
        #[cfg(test)]
        let journal_mode = if self
            .rollback_journal
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            "DELETE"
        } else {
            "WAL"
        };
        #[cfg(not(test))]
        let journal_mode = "WAL";
        conn.pragma_update(None, "journal_mode", journal_mode)
            .map_err(store_error)?;
        #[cfg(any(test, feature = "test-support"))]
        let synchronous = if self.relaxed_durability.load(Ordering::SeqCst) {
            "NORMAL"
        } else {
            "FULL"
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let synchronous = "FULL";
        conn.pragma_update(None, "synchronous", synchronous)
            .map_err(store_error)?;
        conn.pragma_update(None, "journal_size_limit", 67_108_864_i64)
            .map_err(store_error)?;
        let (backfilled, still_lacking) = schema::guard_binding_evidence(&conn)?;
        let report = crate::ports::BindingEvidenceStartup {
            backfilled: backfilled as u64,
            still_lacking: u64::try_from(still_lacking).unwrap_or(0),
        };
        let lacking = if still_lacking > 0 {
            schema::binding_evidence_lacking_seats(&conn)?
        } else {
            Vec::new()
        };
        if !self.binding_evidence_pinned.load(Ordering::SeqCst) {
            if let Ok(mut slot) = self.binding_evidence.lock() {
                *slot = Some(report);
            }
            if let Ok(mut slot) = self.binding_evidence_lacking.lock() {
                *slot = lacking;
            }
        }
        Ok(conn)
    }

    /// Keeps the binding-evidence report of the writer opens so far as the
    /// startup report. The daemon calls it after the early writer it opens
    /// for `journal::open`, so the store's own later open (which has nothing
    /// left to backfill) does not replace the report of the first open.
    pub fn pin_binding_evidence(&self) {
        self.binding_evidence_pinned.store(true, Ordering::SeqCst);
    }

    /// The startup report with `still_lacking` recomputed now: each seat
    /// found lacking at startup is rechecked by one indexed lookup, and the
    /// healed ones (re-registered with evidence, or retired) are dropped.
    /// None before any writer was opened.
    pub fn binding_evidence_current(
        &self,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::BindingEvidenceStartup>, ApiError> {
        let Some(startup) = self.binding_evidence_startup() else {
            return Ok(None);
        };
        let remembered = self
            .binding_evidence_lacking
            .lock()
            .map(|slot| slot.clone())
            .unwrap_or_default();
        if remembered.is_empty() {
            return Ok(Some(crate::ports::BindingEvidenceStartup {
                backfilled: startup.backfilled,
                still_lacking: 0,
            }));
        }
        let db = self.open_query(budget.clone())?;
        let mut still = Vec::with_capacity(remembered.len());
        for seat in remembered {
            db.check_budget()?;
            if schema::binding_evidence_lacking(&db, &seat).map_err(|error| db.map_error(error))? {
                still.push(seat);
            }
        }
        db.check_budget()?;
        let still_lacking = still.len() as u64;
        if let Ok(mut slot) = self.binding_evidence_lacking.lock() {
            slot.retain(|seat| still.contains(seat));
        }
        Ok(Some(crate::ports::BindingEvidenceStartup {
            backfilled: startup.backfilled,
            still_lacking,
        }))
    }

    /// The binding-evidence verification result of the last writer open:
    /// how many legacy bindings were backfilled and how many still lack
    /// reconfirmation evidence. None before any writer was opened.
    pub fn binding_evidence_startup(&self) -> Option<crate::ports::BindingEvidenceStartup> {
        self.binding_evidence.lock().ok().and_then(|slot| *slot)
    }

    /// A connection used by one caller at a time, with query progress and busy
    /// callbacks for this call only. Both honor the live read-work budget. An
    /// idle connection left by an earlier call is reused when one is kept.
    pub fn open_query(&self, budget: CallBudget) -> Result<QueryConnection, ApiError> {
        self.lane_fault_check()?;
        if let Some(error) = query_budget_error(&budget, self.clock()) {
            return Err(error);
        }
        let progress = Box::new(QueryProgress {
            budget,
            clock: Arc::clone(&self.clock),
            #[cfg(test)]
            busy_signal: std::sync::Mutex::new(self.setup_busy_signal.lock().unwrap().take()),
            #[cfg(test)]
            end_signal: std::sync::Mutex::new(self.setup_end_signal.lock().unwrap().take()),
        });
        let reused = self
            .idle_queries
            .lock()
            .ok()
            .and_then(|mut idle| idle.pop());
        let fresh = reused.is_none();
        let conn = match reused {
            Some(conn) => conn,
            None => Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|error| progress.map_sqlite_error(error))?,
        };
        // Both callbacks borrow the same stable Box allocation. Install the
        // busy callback before any schema or PRAGMA statement can wait on a lock.
        let callback_data = (&*progress as *const QueryProgress).cast_mut().cast();
        let busy_result = unsafe {
            ffi::sqlite3_busy_handler(conn.handle(), Some(check_query_busy), callback_data)
        };
        if busy_result != ffi::SQLITE_OK {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "cannot install query busy handler",
            ));
        }
        unsafe {
            ffi::sqlite3_progress_handler(
                conn.handle(),
                100,
                Some(check_query_progress),
                callback_data,
            );
        }
        if fresh {
            conn.pragma_update(None, "foreign_keys", "ON")
                .map_err(|error| progress.map_sqlite_error(error))?;
            conn.pragma_update(None, "query_only", "ON")
                .map_err(|error| progress.map_sqlite_error(error))?;
        }
        schema::verify_query_connection(&conn).map_err(|error| progress.map_api_error(error))?;
        if let Some(error) = progress.budget_error() {
            return Err(error);
        }
        Ok(QueryConnection {
            conn: Some(conn),
            _progress: progress,
            idle: Arc::clone(&self.idle_queries),
        })
    }

    /// Budget-aware predecision work on the serialized domain writer. Install
    /// callbacks before BEGIN IMMEDIATE, validate, then check the live budget
    /// immediately before accepting the sampled decision. After acceptance,
    /// caller cancellation cannot replace the apply/commit storage outcome.
    ///
    /// This does not bound physical commit/fsync time. Older execute_decision
    /// consumers retain their existing semantics until deliberately migrated.
    pub fn execute_budgeted_decision<P, R>(
        &self,
        conn: &mut Connection,
        budget: &CallBudget,
        validate: impl FnOnce(&Transaction<'_>) -> Result<P, ApiError>,
        apply: impl FnOnce(&Transaction<'_>, DecisionInstant, P) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        self.execute_budgeted_decision_with_constraints(conn, budget, None, validate, apply)
    }

    /// The current call and permit issuance retain independent cancellation
    /// tokens. Both constrain predecision work; accepted storage outcomes stay
    /// independent of subsequent cancellation, as in the single-budget API.
    pub fn execute_budgeted_decision_with_constraints<P, R>(
        &self,
        conn: &mut Connection,
        budget: &CallBudget,
        issuance: Option<&CallBudget>,
        validate: impl FnOnce(&Transaction<'_>) -> Result<P, ApiError>,
        apply: impl FnOnce(&Transaction<'_>, DecisionInstant, P) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let mut guard = WriterBudgetGuard::install(conn, budget, issuance, self.clock())?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| guard.map_error(store_error(error)))?;
        let prepared = match validate(&tx) {
            Ok(prepared) => prepared,
            Err(error) => {
                let error = guard.map_error(error);
                // Rollback must remain possible after interruption. The guard
                // outlives the transaction and clears callbacks before its drop.
                guard.disable();
                return Err(error);
            }
        };
        let decision = DecisionInstant {
            utc: self.clock.utc_now(),
            monotonic: self.clock.monotonic_now(),
        };
        if let Some(error) = guard.budget_error() {
            guard.disable();
            return Err(error);
        }
        guard.disable();
        let result = apply(&tx, decision, prepared)?;
        super::journal::check_self_applied(&tx)?;
        tx.commit().map_err(store_error)?;
        Ok(result)
    }

    /// Validation runs under BEGIN IMMEDIATE, before either clock sample.
    /// The closure's result is visible only after the transaction commits.
    pub fn execute_decision<P, R>(
        &self,
        conn: &mut Connection,
        validate: impl FnOnce(&Transaction<'_>) -> Result<P, ApiError>,
        apply: impl FnOnce(&Transaction<'_>, DecisionInstant, P) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let prepared = validate(&tx)?;
        let decision = DecisionInstant {
            utc: self.clock.utc_now(),
            monotonic: self.clock.monotonic_now(),
        };
        let result = apply(&tx, decision, prepared)?;
        super::journal::check_self_applied(&tx)?;
        tx.commit().map_err(store_error)?;
        Ok(result)
    }
}

// Only StoreContext installs this guard. The stable boxed callback state lives
// until all callbacks are removed, including BEGIN failures and rollback paths.
struct WriterBudgetState<'a> {
    budget: &'a CallBudget,
    issuance: Option<&'a CallBudget>,
    clock: &'a dyn Clock,
}
impl WriterBudgetState<'_> {
    fn budget_error(&self) -> Option<ApiError> {
        writer_budget_error(self.budget, self.clock).or_else(|| {
            self.issuance
                .and_then(|budget| writer_budget_error(budget, self.clock))
        })
    }
    fn deadline(&self) -> MonoInstant {
        self.issuance.map_or(self.budget.deadline, |budget| {
            budget.deadline.min(self.budget.deadline)
        })
    }
}
struct WriterBudgetGuard<'a> {
    handle: *mut ffi::sqlite3,
    state: Box<WriterBudgetState<'a>>,
    enabled: bool,
}
impl<'a> WriterBudgetGuard<'a> {
    fn install(
        conn: &Connection,
        budget: &'a CallBudget,
        issuance: Option<&'a CallBudget>,
        clock: &'a dyn Clock,
    ) -> Result<Self, ApiError> {
        let state = Box::new(WriterBudgetState {
            budget,
            issuance,
            clock,
        });
        if let Some(error) = state.budget_error() {
            return Err(error);
        }
        let handle = unsafe { conn.handle() };
        let data = (&*state as *const WriterBudgetState<'_>).cast_mut().cast();
        let result = unsafe { ffi::sqlite3_busy_handler(handle, Some(check_writer_busy), data) };
        if result != ffi::SQLITE_OK {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "cannot install writer busy handler",
            ));
        }
        unsafe {
            ffi::sqlite3_progress_handler(handle, 100, Some(check_writer_budget_progress), data)
        };
        Ok(Self {
            handle,
            state,
            enabled: true,
        })
    }
    fn budget_error(&self) -> Option<ApiError> {
        self.state.budget_error()
    }
    fn map_error(&self, error: ApiError) -> ApiError {
        if matches!(error.code, ErrorCode::StoreBusy | ErrorCode::Cancelled) {
            self.budget_error().unwrap_or(error)
        } else {
            error
        }
    }
    fn disable(&mut self) {
        if self.enabled {
            unsafe {
                ffi::sqlite3_progress_handler(self.handle, 0, None, std::ptr::null_mut());
                // Restores the writer's normal postdecision busy policy and
                // replaces the busy callback, removing its borrowed pointer.
                ffi::sqlite3_busy_timeout(self.handle, 2000);
            }
            self.enabled = false;
        }
    }
}
impl Drop for WriterBudgetGuard<'_> {
    fn drop(&mut self) {
        self.disable();
    }
}
fn writer_budget_error(budget: &CallBudget, clock: &dyn Clock) -> Option<ApiError> {
    if budget.cancellation.is_cancelled() {
        Some(api_error(ErrorCode::Cancelled, "store call cancelled"))
    } else if clock.monotonic_now() >= budget.deadline {
        Some(api_error(
            ErrorCode::DeadlineExceeded,
            "store call deadline exceeded",
        ))
    } else {
        None
    }
}
unsafe extern "C" fn check_writer_busy(pointer: *mut c_void, _count: i32) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &*(pointer as *const WriterBudgetState<'_>) };
        if state.budget_error().is_some() {
            return 0;
        }
        let remaining = state
            .deadline()
            .0
            .saturating_sub(state.clock.monotonic_now().0);
        std::thread::sleep(Duration::from_millis(remaining.min(2)));
        i32::from(state.budget_error().is_none())
    }))
    .unwrap_or(0)
}
unsafe extern "C" fn check_writer_budget_progress(pointer: *mut c_void) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &*(pointer as *const WriterBudgetState<'_>) };
        i32::from(state.budget_error().is_some())
    }))
    .unwrap_or(1)
}

struct QueryProgress {
    budget: CallBudget,
    clock: Arc<dyn Clock>,
    #[cfg(test)]
    busy_signal: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    end_signal: std::sync::Mutex<Option<std::sync::mpsc::Sender<Option<ErrorCode>>>>,
}

#[cfg(test)]
impl Drop for QueryProgress {
    fn drop(&mut self) {
        if let Some(signal) = self.end_signal.get_mut().ok().and_then(Option::take) {
            let _ = signal.send(self.budget_error().map(|error| error.code));
        }
    }
}

fn query_budget_error(budget: &CallBudget, clock: &dyn Clock) -> Option<ApiError> {
    if budget.cancellation.is_cancelled() {
        Some(api_error(ErrorCode::Cancelled, "read cancelled"))
    } else if clock.monotonic_now() >= budget.deadline {
        Some(api_error(
            ErrorCode::DeadlineExceeded,
            "read budget exhausted",
        ))
    } else {
        None
    }
}

impl QueryProgress {
    fn budget_error(&self) -> Option<ApiError> {
        query_budget_error(&self.budget, self.clock.as_ref())
    }

    fn map_api_error(&self, error: ApiError) -> ApiError {
        if matches!(error.code, ErrorCode::StoreBusy | ErrorCode::Cancelled) {
            self.budget_error().unwrap_or(error)
        } else {
            error
        }
    }

    fn map_sqlite_error(&self, error: rusqlite::Error) -> ApiError {
        self.map_api_error(store_error(error))
    }
}

unsafe extern "C" fn check_query_busy(pointer: *mut c_void, count: i32) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let progress = unsafe { &*(pointer as *const QueryProgress) };
        #[cfg(test)]
        if count == 0
            && let Some(signal) = progress
                .busy_signal
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
        {
            let _ = signal.send(());
        }
        #[cfg(not(test))]
        let _ = count;
        if progress.budget.is_exhausted(progress.clock.as_ref()) {
            return 0;
        }
        let remaining = progress
            .budget
            .deadline
            .0
            .saturating_sub(progress.clock.monotonic_now().0);
        std::thread::sleep(Duration::from_millis(remaining.min(2)));
        i32::from(!progress.budget.is_exhausted(progress.clock.as_ref()))
    }))
    .unwrap_or(0)
}

unsafe extern "C" fn check_query_progress(pointer: *mut c_void) -> i32 {
    // Clock implementations are supplied by the daemon. An accidental panic
    // must not unwind through SQLite's C callback.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let progress = unsafe { &*(pointer as *const QueryProgress) };
        i32::from(progress.budget.is_exhausted(progress.clock.as_ref()))
    }))
    .unwrap_or(1)
}

pub struct QueryConnection {
    conn: Option<Connection>,
    _progress: Box<QueryProgress>,
    idle: IdleQueries,
}

impl QueryConnection {
    pub fn check_budget(&self) -> Result<(), ApiError> {
        self._progress.budget_error().map_or(Ok(()), Err)
    }

    /// A busy handler returns SQLITE_BUSY when cancellation or the budget ends;
    /// convert that result using the same connection's live budget state.
    pub fn map_error(&self, error: rusqlite::Error) -> ApiError {
        self._progress.map_sqlite_error(error)
    }
}

impl Deref for QueryConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn
            .as_ref()
            .expect("query connection is present until drop")
    }
}

impl Drop for QueryConnection {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        unsafe {
            ffi::sqlite3_busy_handler(conn.handle(), None, std::ptr::null_mut());
            ffi::sqlite3_progress_handler(conn.handle(), 0, None, std::ptr::null_mut());
        }
        // Only a connection with no open transaction goes back, so the next
        // call starts its own read snapshot.
        if conn.is_autocommit()
            && let Ok(mut idle) = self.idle.lock()
            && idle.len() < IDLE_QUERY_CONNECTIONS
        {
            idle.push(conn);
        }
    }
}

pub(crate) fn api_error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError::new(code, detail)
}

/// Map a SQLite error to the four-way taxonomy (root §B3 D3). Only
/// SQLITE_CORRUPT / SQLITE_NOTADB (and a failed integrity check, reported by
/// `check_integrity`) are Corrupt. Every unlisted code, and every non-SQLite
/// failure, is Transient with the error as detail, never Corrupt. ht-p03.22's
/// stored class is not merged yet: `StoreBusy` defaults to Transient.
pub(crate) fn store_error(error: rusqlite::Error) -> ApiError {
    let code = match &error {
        rusqlite::Error::SqliteFailure(err, _) => match err.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                ErrorCode::StoreBusy
            }
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                ErrorCode::StoreCorrupt
            }
            // Disk full is actionable and distinct from damaged history.
            rusqlite::ErrorCode::DiskFull => ErrorCode::StoreFull,
            rusqlite::ErrorCode::ConstraintViolation => ErrorCode::Conflict,
            rusqlite::ErrorCode::OperationInterrupted => ErrorCode::Cancelled,
            _ => return unclassified(&error),
        },
        _ => return unclassified(&error),
    };
    if code == ErrorCode::StoreFull {
        return api_error(
            code,
            format!(
                "SQLite: {error}; free disk space and retry (nothing was accepted, history is intact)"
            ),
        );
    }
    api_error(code, format!("SQLite: {error}"))
}

fn unclassified(error: &rusqlite::Error) -> ApiError {
    api_error(
        ErrorCode::StoreBusy,
        format!("SQLite (unclassified, transient): {error}"),
    )
}

#[cfg(test)]
#[path = "../../tests/store/schema.rs"]
pub(crate) mod tests;

/// Unit tests run one process per test and open hundreds of fresh stores;
/// replaying every migration costs each ~60 ms of CPU (schema 27's table
/// rebuilds alone ~17 ms). A writer whose store file does not exist yet is
/// seeded with this test binary's fresh-schema template, built once by the
/// production fresh-creation path; `schema::initialize` then verifies it as
/// it verifies any existing store. Existing files, in-memory databases and
/// direct `schema::initialize` calls are untouched, and any seeding failure
/// falls back to ordinary creation.
#[cfg(test)]
mod fresh_schema_template {
    use super::schema;
    use crate::protocol::time::UtcMillis;
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    pub(super) fn seed(path: &Path) {
        if std::fs::symlink_metadata(path).is_ok() {
            return;
        }
        let (Some(template), Some(name)) = (template(), path.file_name()) else {
            return;
        };
        let mut staging = name.to_os_string();
        staging.push(format!(".{}.seed", uuid::Uuid::new_v4()));
        let staging = path.with_file_name(staging);
        if std::fs::copy(&template, &staging).is_ok() {
            // Never replaces a store another opener created meanwhile.
            let _ = std::fs::hard_link(&staging, path);
        }
        let _ = std::fs::remove_file(&staging);
    }

    /// Keyed by this binary's identity, so a rebuild never reuses a template
    /// produced by different schema code.
    fn template() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        let built = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let path = exe.with_file_name(format!(
            "{}.schema-v{}-{}-{}.db",
            exe.file_name()?.to_str()?,
            schema::NODE_SCHEMA_VERSION,
            meta.len(),
            built.as_nanos()
        ));
        if path.is_file() {
            return Some(path);
        }
        let staging = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        // The template holds the fixed test node and `created_at = 0`; a
        // store created through it differs from ordinary creation only in the
        // node row's creation time, which nothing reads.
        let built = Connection::open(&staging).ok().and_then(|conn| {
            schema::initialize(
                &conn,
                || UtcMillis(0),
                &crate::test_support::test_node_seed(),
            )
            .ok()
        });
        let renamed = built.is_some() && std::fs::rename(&staging, &path).is_ok();
        let _ = std::fs::remove_file(&staging);
        renamed.then_some(path)
    }
}

/// Per-connection commit-change state for the domain writer (spec D1). The
/// update hook ORs the changed table's lane set into `pending`, the commit
/// hook seals it, the rollback hook clears it. Every access happens on the
/// thread that holds the writer mutex; the atomics only make the state
/// `Sync`, the mutex provides the ordering.
#[derive(Default)]
pub(super) struct KickHooks {
    pending: AtomicU16,
    sealed: AtomicU16,
    /// A table of `kicks::MOD_NOTIFY_TABLES` changed in the open / last
    /// commit; sealed separately from the lane set (spec D2).
    mod_pending: AtomicBool,
    mod_sealed: AtomicBool,
    row_changed: AtomicBool,
    /// A row-changing commit has happened since the last `settle_generation`.
    dirty: AtomicBool,
    /// Count of writer turns that committed row changes, advanced only when
    /// the turn ends (the committed rows are then visible to every other
    /// connection). Retention reads it to skip a rescan of unchanged tables.
    generation: std::sync::atomic::AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    commits: [std::sync::atomic::AtomicU64; kicks::Lane::INTERNAL_ALL.len() + 1],
}

impl KickHooks {
    /// Installs the update, commit and rollback hooks on the writer
    /// connection. The returned box is the hooks' user data: it must outlive
    /// `conn` (declare it after the connection in the owning struct).
    pub(super) fn install(conn: &Connection) -> Box<Self> {
        let state = Box::new(Self::default());
        let data: *mut c_void = (&*state as *const Self).cast_mut().cast();
        unsafe {
            let handle = conn.handle();
            ffi::sqlite3_update_hook(handle, Some(kick_update_hook), data);
            ffi::sqlite3_commit_hook(handle, Some(kick_commit_hook), data);
            ffi::sqlite3_rollback_hook(handle, Some(kick_rollback_hook), data);
        }
        state
    }

    /// Takes the sealed set. Called while the writer guard is still held.
    pub(super) fn take_sealed(&self) -> LaneSet {
        LaneSet::from_bits(self.sealed.swap(0, Ordering::SeqCst))
    }

    /// Takes the sealed mod-notify bit. Called while the writer guard is held.
    pub(super) fn take_mod_sealed(&self) -> bool {
        self.mod_sealed.swap(false, Ordering::SeqCst)
    }

    /// Publishes the turn's committed changes as a new generation. Called
    /// while the writer guard is still held, after the turn's last commit.
    pub(super) fn settle_generation(&self) {
        if self.dirty.swap(false, Ordering::SeqCst) {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The writer's change generation: unchanged means no committed row change
    /// since it was last read.
    pub(super) fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Commits that changed at least one row, keyed by origin lane name or
    /// `request` for no origin.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn commit_counts(&self) -> std::collections::BTreeMap<String, u64> {
        let mut counts = std::collections::BTreeMap::new();
        for (index, lane) in kicks::Lane::ALL.iter().enumerate() {
            counts.insert(
                lane.name().to_string(),
                self.commits[index].load(Ordering::SeqCst),
            );
        }
        counts.insert(
            "request".to_string(),
            self.commits[kicks::Lane::INTERNAL_ALL.len()].load(Ordering::SeqCst),
        );
        counts
    }

    fn on_update(&self, table: &str) {
        self.row_changed.store(true, Ordering::SeqCst);
        self.pending
            .fetch_or(kicks::lanes_for_table(table).to_bits(), Ordering::SeqCst);
        if kicks::notifies_mod(table) {
            self.mod_pending.store(true, Ordering::SeqCst);
        }
    }

    fn on_commit(&self) {
        let pending = self.pending.swap(0, Ordering::SeqCst);
        let mod_pending = self.mod_pending.swap(false, Ordering::SeqCst);
        if !self.row_changed.swap(false, Ordering::SeqCst) {
            return;
        }
        self.dirty.store(true, Ordering::SeqCst);
        let origin = kicks::current_origin();
        #[cfg(any(test, feature = "test-support"))]
        self.commits[origin.map_or(kicks::Lane::INTERNAL_ALL.len(), |lane| lane as usize)]
            .fetch_add(1, Ordering::SeqCst);
        let kicked = kicks::kicks_for_commit(origin, LaneSet::from_bits(pending));
        self.sealed.fetch_or(kicked.to_bits(), Ordering::SeqCst);
        // Mod notification follows the origin guard only for Retention.
        if mod_pending && origin != Some(kicks::Lane::Retention) {
            self.mod_sealed.store(true, Ordering::SeqCst);
        }
    }

    fn on_rollback(&self) {
        self.pending.store(0, Ordering::SeqCst);
        self.mod_pending.store(false, Ordering::SeqCst);
        self.row_changed.store(false, Ordering::SeqCst);
    }
}

unsafe extern "C" fn kick_update_hook(
    data: *mut c_void,
    _op: c_int,
    _database: *const c_char,
    table: *const c_char,
    _rowid: i64,
) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &*(data as *const KickHooks) };
        let table = unsafe { std::ffi::CStr::from_ptr(table) }.to_string_lossy();
        state.on_update(&table);
    }));
}

unsafe extern "C" fn kick_commit_hook(data: *mut c_void) -> c_int {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        unsafe { &*(data as *const KickHooks) }.on_commit();
    }));
    0
}

unsafe extern "C" fn kick_rollback_hook(data: *mut c_void) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        unsafe { &*(data as *const KickHooks) }.on_rollback();
    }));
}
