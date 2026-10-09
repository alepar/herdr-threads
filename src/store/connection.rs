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
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use super::schema;

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

pub struct StoreContext {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    idle_queries: IdleQueries,
    /// The last writer open's binding-evidence verification result.
    binding_evidence: std::sync::Mutex<Option<crate::ports::BindingEvidenceStartup>>,
    /// Seats whose latest binding still lacked evidence after the last
    /// writer open. Only these are rechecked for Health; the set only
    /// shrinks, because every new live binding must carry evidence.
    binding_evidence_lacking: std::sync::Mutex<Vec<String>>,
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
/// commits with `synchronous=NORMAL`. Hundreds of test daemons each fsyncing
/// every WAL commit made the suite disk-bound on a loaded machine; a test
/// cannot observe the difference, which only matters on power loss.
/// Production builds never compile it and always commit with `FULL`.
#[cfg(any(test, feature = "test-support"))]
pub const TEST_RELAXED_DURABILITY_ENV: &str = "HT_TEST_RELAXED_DURABILITY";

impl StoreContext {
    pub fn new(path: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            path,
            clock,
            idle_queries: Arc::default(),
            binding_evidence: std::sync::Mutex::new(None),
            binding_evidence_lacking: std::sync::Mutex::new(Vec::new()),
            #[cfg(any(test, feature = "test-support"))]
            lane_fault: std::sync::Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            relaxed_durability: AtomicBool::new(
                std::env::var_os(TEST_RELAXED_DURABILITY_ENV).is_some_and(|v| v == "1"),
            ),
            #[cfg(test)]
            setup_busy_signal: std::sync::Mutex::new(None),
            #[cfg(test)]
            setup_end_signal: std::sync::Mutex::new(None),
            #[cfg(test)]
            rollback_journal: std::sync::atomic::AtomicBool::new(false),
        }
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

    pub fn open_writer(&self) -> Result<Connection, ApiError> {
        #[cfg(test)]
        fresh_schema_template::seed(&self.path);
        let conn = Connection::open(&self.path).map_err(store_error)?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(store_error)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(store_error)?;
        schema::initialize(&conn, || self.clock.utc_now())?;
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
        if let Ok(mut slot) = self.binding_evidence.lock() {
            *slot = Some(report);
        }
        if let Ok(mut slot) = self.binding_evidence_lacking.lock() {
            *slot = lacking;
        }
        Ok(conn)
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
mod tests;

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
            schema::LATEST_VERSION,
            meta.len(),
            built.as_nanos()
        ));
        if path.is_file() {
            return Some(path);
        }
        let staging = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let clock_read = std::cell::Cell::new(false);
        let built = Connection::open(&staging).ok().and_then(|conn| {
            schema::initialize(&conn, || {
                clock_read.set(true);
                UtcMillis(0)
            })
            .ok()
        });
        // A creation path that reads the clock is not reproducible from a
        // template: keep ordinary creation.
        let renamed =
            built.is_some() && !clock_read.get() && std::fs::rename(&staging, &path).is_ok();
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
    pending: AtomicU8,
    sealed: AtomicU8,
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
        if !kicks::origin_discards_kicks(origin) {
            self.sealed.fetch_or(pending, Ordering::SeqCst);
            if mod_pending {
                self.mod_sealed.store(true, Ordering::SeqCst);
            }
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
