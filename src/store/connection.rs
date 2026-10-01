//! SQLite connection and decision transaction ownership.

use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, ffi};
use std::{ffi::c_void, ops::Deref, path::PathBuf, sync::Arc, time::Duration};

use super::schema;

pub struct StoreContext {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    /// The last writer open's binding-evidence verification result.
    binding_evidence: std::sync::Mutex<Option<crate::ports::BindingEvidenceStartup>>,
    /// Seats whose latest binding still lacked evidence after the last
    /// writer open. Only these are rechecked for Health; the set only
    /// shrinks, because every new live binding must carry evidence.
    binding_evidence_lacking: std::sync::Mutex<Vec<String>>,
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

impl StoreContext {
    pub fn new(path: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            path,
            clock,
            binding_evidence: std::sync::Mutex::new(None),
            binding_evidence_lacking: std::sync::Mutex::new(Vec::new()),
            #[cfg(test)]
            setup_busy_signal: std::sync::Mutex::new(None),
            #[cfg(test)]
            setup_end_signal: std::sync::Mutex::new(None),
            #[cfg(test)]
            rollback_journal: std::sync::atomic::AtomicBool::new(false),
        }
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
        let conn = Connection::open(&self.path).map_err(store_error)?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(store_error)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(store_error)?;
        schema::initialize(&conn)?;
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
        conn.pragma_update(None, "synchronous", "FULL")
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

    /// A dedicated connection, with its own interrupt target and query progress
    /// and busy callbacks. Both honor the live read-work budget.
    pub fn open_query(&self, budget: CallBudget) -> Result<QueryConnection, ApiError> {
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
        let conn = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| progress.map_sqlite_error(error))?;
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
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| progress.map_sqlite_error(error))?;
        conn.pragma_update(None, "query_only", "ON")
            .map_err(|error| progress.map_sqlite_error(error))?;
        schema::verify_query_connection(&conn).map_err(|error| progress.map_api_error(error))?;
        if let Some(error) = progress.budget_error() {
            return Err(error);
        }
        Ok(QueryConnection {
            conn,
            _progress: progress,
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
    conn: Connection,
    _progress: Box<QueryProgress>,
}

impl QueryConnection {
    pub fn check_budget(&self) -> Result<(), ApiError> {
        self._progress.budget_error().map_or(Ok(()), Err)
    }

    pub fn interrupt_handle(&self) -> rusqlite::InterruptHandle {
        self.conn.get_interrupt_handle()
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
        &self.conn
    }
}

impl Drop for QueryConnection {
    fn drop(&mut self) {
        unsafe {
            ffi::sqlite3_busy_handler(self.conn.handle(), None, std::ptr::null_mut());
            ffi::sqlite3_progress_handler(self.conn.handle(), 0, None, std::ptr::null_mut());
        }
    }
}

pub(crate) fn api_error(code: ErrorCode, detail: impl Into<String>) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
}

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
            _ => ErrorCode::StoreCorrupt,
        },
        _ => ErrorCode::StoreCorrupt,
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

#[cfg(test)]
#[path = "../../tests/store/schema.rs"]
mod tests;
