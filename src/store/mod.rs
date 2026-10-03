//! SQLite-backed production store dispatch.
pub mod attention;
pub mod catch_up;
pub mod connection;
pub mod control;
pub mod effective;
pub mod harness_evidence;
pub mod invitation_due;
pub mod materialization;
pub mod messages;
pub mod operator;
pub mod page_fit;
pub mod poke;
pub(crate) mod public_ids;
pub mod queries;
pub mod receipts;
pub mod retention;
pub mod schema;
pub mod seats;
pub mod service_controls;
pub mod service_events;
pub mod service_send;
pub mod service_substrate;
pub mod summary;
pub mod wake;
pub mod work;

use crate::service::kicks::{self, CommitKicks};
#[cfg(any(test, feature = "test-support"))]
use crate::service::kicks::{Lane, LaneSet};
use crate::{
    ports::{
        ClosureEvidence, DuePhase, DuePhaseCursor, DuePhaseProgress, DueScanProgress,
        DueScanRequest, DurableWorkAdmission, GuardedInvalidationTransition, GuardedSeatTransition,
        HostInvalidationFence, HostInvalidationReason, HostObservation, HostObservationAdmission,
        InvalidationSeatPage, OperationReadScope, OperatorRequest, PokeDue, PokeReceipt,
        PokeReservation, PriorLadder, PruneProgress, PublishedSnapshot, ReadContext,
        ReceiptSparseCursor, ReconciliationOutcome, RegisterAvailableRequest, RetirementJob,
        RetirementProgress, RetirementSummary, SendPreparationProgress, SnapshotCleanupProgress,
        SnapshotGenerationId, SnapshotHeader, SnapshotSeatPage, SnapshotStage,
        SnapshotStageProgress, StorePort, WakeCandidate, WakeOutcome, WakeRecoveryCandidate,
        WakeRecoveryOutcome, WakeRecoveryRequest, WakeReservation, WorkAdmission, WorkCandidate,
        WorkKind, WorkProgress,
    },
    protocol::{
        authority::{MutationPermit, OperatorActor},
        commands::{
            Command, DirectoryMembership, OperatorCommand, PermitMutation, ResolveSeat,
            RetirementJobsQuery, SendMessage, WarningsQuery,
        },
        ids::{RetirementJobId, SeatId, WakeAttemptId},
        pagination::{
            Consistency, Cursor, CursorDirection, CursorScope, Page, PageRequest, StopReason,
        },
        results::{ApiError, BoundedError, CommandResult, ErrorCode, RetirementStatus},
        time::{CallBudget, Clock, UtcMillis},
    },
};
use connection::{StoreContext, api_error, store_error};
use rusqlite::{Connection, OptionalExtension, ffi, params};
use std::{
    ffi::c_void,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};

/// Rows the catch-up stall scan ends per receipts due pass.
const CATCH_UP_STALL_SCAN_ROWS: u16 = 100;

#[derive(Debug, Clone)]
pub struct StoreSettings {
    pub invitation_default_ms: Option<u64>,
    pub message_limits: messages::MessageLimits,
    /// The elected daemon run's boot identity, supplied by the service factory.
    pub daemon_boot: Option<uuid::Uuid>,
    pub minimum_wake_delay_ms: u64,
    pub summary: crate::protocol::summary::SummarySettings,
}
impl Default for StoreSettings {
    fn default() -> Self {
        Self {
            invitation_default_ms: None,
            message_limits: messages::MessageLimits::default(),
            daemon_boot: None,
            minimum_wake_delay_ms: 30_000,
            summary: crate::protocol::summary::SummarySettings::default(),
        }
    }
}

/// One serialized domain writer. Every ordinary query opens an independent
/// query-only connection through StoreContext.
pub struct SqliteStore {
    context: StoreContext,
    instance: String,
    settings: StoreSettings,
    writer: Mutex<Connection>,
    /// Hook state of `writer`; declared after it so it outlives the connection.
    hooks: Box<connection::KickHooks>,
    kicks: Arc<CommitKicks>,
    /// Change mark (`retention::change_mark`) at which the last retention
    /// snapshot scan found no candidate; the scan is skipped while it holds.
    retention_idle: Mutex<Option<(u64, i64)>>,
    #[cfg(any(test, feature = "test-support"))]
    retention_cost: Mutex<Option<Arc<crate::test_support::isolation::CostCounter>>>,
    #[cfg(any(test, feature = "test-support"))]
    kick_sink: Mutex<Option<KickSink>>,
    #[cfg(any(test, feature = "test-support"))]
    kick_pause: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

#[cfg(any(test, feature = "test-support"))]
type KickSink = Arc<dyn Fn(LaneSet, Option<Lane>) + Send + Sync>;

/// One turn on the serialized domain writer. The mutex guard is private to
/// this type, so no caller can hold the writer outside a turn. Dropping the
/// turn takes the commits' sealed lane set while the guard is still held,
/// releases the guard, then kicks those lanes minus the committing thread's
/// own origin lane: taking the set after release would let another thread's
/// turn merge its tables into it and flush it minus its own origin.
pub struct WriterTurn<'a> {
    guard: Option<std::sync::MutexGuard<'a, Connection>>,
    store: &'a SqliteStore,
}
impl Deref for WriterTurn<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.guard.as_deref().expect("writer guard held until drop")
    }
}
impl DerefMut for WriterTurn<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.guard
            .as_deref_mut()
            .expect("writer guard held until drop")
    }
}
impl Drop for WriterTurn<'_> {
    fn drop(&mut self) {
        let sealed = self.store.hooks.take_sealed();
        self.store.hooks.settle_generation();
        drop(self.guard.take());
        #[cfg(any(test, feature = "test-support"))]
        {
            let pause = self.store.kick_pause.lock().ok().and_then(|p| p.clone());
            if let Some(pause) = pause {
                pause();
            }
        }
        let origin = kicks::current_origin();
        let lanes = origin.map_or(sealed, |lane| sealed.without(lane));
        if lanes.is_empty() {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            let sink = self.store.kick_sink.lock().ok().and_then(|s| s.clone());
            if let Some(sink) = sink {
                sink(lanes, origin);
            }
        }
        self.store.kicks.kick(lanes);
    }
}

struct WriterProgress<'a> {
    budget: &'a CallBudget,
    clock: &'a dyn Clock,
}
struct WriterProgressGuard<'a> {
    handle: *mut ffi::sqlite3,
    _state: Box<WriterProgress<'a>>,
}
unsafe extern "C" fn check_writer_progress(data: *mut c_void) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let state = unsafe { &*(data as *const WriterProgress<'_>) };
        i32::from(state.budget.is_exhausted(state.clock))
    }))
    .unwrap_or(1)
}
impl<'a> WriterProgressGuard<'a> {
    fn install(conn: &Connection, budget: &'a CallBudget, clock: &'a dyn Clock) -> Self {
        let state = Box::new(WriterProgress { budget, clock });
        let data = (&*state as *const WriterProgress<'_>).cast_mut().cast();
        let handle = unsafe { conn.handle() };
        unsafe { ffi::sqlite3_progress_handler(handle, 100, Some(check_writer_progress), data) };
        Self {
            handle,
            _state: state,
        }
    }
}
impl Drop for WriterProgressGuard<'_> {
    fn drop(&mut self) {
        unsafe { ffi::sqlite3_progress_handler(self.handle, 0, None, std::ptr::null_mut()) };
    }
}
impl SqliteStore {
    fn check_service_read_instance(
        &self,
        connection: &crate::ports::ServiceConnectionAuthority,
    ) -> Result<(), ApiError> {
        if connection.instance() != self.instance {
            return Err(api_error(
                ErrorCode::Unauthorized,
                "service instance mismatch",
            ));
        }
        Ok(())
    }

    /// Spec §9 join hint: a committed (or replayed) plain accept names the
    /// thread when it already holds one full summary chunk. Computed after the
    /// commit from a fresh read and never stored; a failed read means no hint,
    /// never a failed accept.
    fn with_join_hint(
        &self,
        result: CommandResult,
        thread: &crate::protocol::ids::ThreadId,
        budget: &CallBudget,
    ) -> CommandResult {
        let CommandResult::Accepted(mut accepted) = result else {
            return result;
        };
        let full = self
            .context
            .open_query(budget.clone())
            .and_then(|db| summary::thread_has_full_chunk(&db, thread, &self.settings.summary));
        if full.unwrap_or(false) {
            accepted.summary_available = Some(thread.clone());
        }
        CommandResult::Accepted(accepted)
    }

    pub fn new(
        context: StoreContext,
        instance: impl Into<String>,
        settings: StoreSettings,
    ) -> Result<Self, ApiError> {
        let instance = instance.into();
        if instance.is_empty()
            || instance.len() > 128
            || settings.message_limits.receipt_duration_ms <= 0
            || settings.message_limits.body_bytes == 0
            || settings.message_limits.body_bytes > messages::MAX_BODY_BYTES
            || settings
                .invitation_default_ms
                .is_some_and(|ms| ms == 0 || ms > i64::MAX as u64)
            || settings.minimum_wake_delay_ms < 30_000
            || settings.minimum_wake_delay_ms > i64::MAX as u64
            || settings
                .daemon_boot
                .as_ref()
                .is_some_and(uuid::Uuid::is_nil)
        {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "invalid store settings",
            ));
        }
        let mut writer = context.open_writer()?;
        // A store stuck with a hold nothing protects (reconciliation already
        // finished, no unresolved seat) is cleared once at daemon start.
        let tx = writer
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(store_error)?;
        seats::lift_baseline_hold_if_clear(&tx, &instance, context.clock().utc_now())?;
        tx.commit().map_err(store_error)?;
        Ok(Self {
            context,
            instance,
            settings,
            hooks: connection::KickHooks::install(&writer),
            writer: Mutex::new(writer),
            kicks: Arc::new(CommitKicks::default()),
            retention_idle: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            retention_cost: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            kick_sink: Mutex::new(None),
            #[cfg(any(test, feature = "test-support"))]
            kick_pause: Mutex::new(None),
        })
    }

    /// Replaces the lane registry commits kick through. The daemon passes the
    /// one registry its lanes register their Pacers with.
    #[must_use]
    pub fn with_commit_kicks(mut self, kicks: Arc<CommitKicks>) -> Self {
        self.kicks = kicks;
        self
    }

    /// Test hook: commits that changed rows, per origin lane name (`request`
    /// when no origin).
    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.hooks.commit_counts()
    }

    /// Test hook: counts the VM units of retention's read queries.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_retention_cost_counter(
        &self,
        counter: Option<Arc<crate::test_support::isolation::CostCounter>>,
    ) {
        *self.retention_cost.lock().unwrap() = counter;
    }

    #[cfg(any(test, feature = "test-support"))]
    fn cost_probe(&self) -> Option<Arc<crate::test_support::isolation::CostCounter>> {
        self.retention_cost.lock().ok().and_then(|c| c.clone())
    }

    /// Test hook: records every flushed kick (lanes, origin) after the writer
    /// guard is released and before the Pacers are kicked.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_kick_sink(&self, sink: Box<dyn Fn(LaneSet, Option<Lane>) + Send + Sync>) {
        *self.kick_sink.lock().unwrap() = Some(Arc::from(sink));
    }

    /// Test hook: runs in `WriterTurn::drop` between guard release and kick.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_kick_pause(&self, pause: Box<dyn Fn() + Send + Sync>) {
        *self.kick_pause.lock().unwrap() = Some(Arc::from(pause));
    }

    /// Test hook: fails store access (writer turns and query connections) on
    /// a thread whose lane origin the closure maps to an error.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_lane_fault(&self, fault: Option<connection::LaneFault>) {
        self.context.set_lane_fault(fault);
    }

    fn writer(&self, budget: &CallBudget) -> Result<WriterTurn<'_>, ApiError> {
        self.context.lane_fault_check()?;
        loop {
            self.live_budget(budget)?;
            match self.writer.try_lock() {
                Ok(guard) => {
                    self.live_budget(budget)?;
                    failpoint!(
                        "store.writer.acquired",
                        self.context.failpoint_scope(),
                        connection = &guard
                    );
                    return Ok(WriterTurn {
                        guard: Some(guard),
                        store: self,
                    });
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return Err(api_error(ErrorCode::StoreCorrupt, "writer lock poisoned"));
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    // Test-only barrier: this caller is waiting for the writer.
                    failpoint!("store.writer.contended", self.context.failpoint_scope());
                    let remaining = budget
                        .deadline
                        .0
                        .saturating_sub(self.context.clock().monotonic_now().0);
                    std::thread::sleep(std::time::Duration::from_millis(remaining.min(2)));
                }
            }
        }
    }

    fn live_budget(&self, budget: &CallBudget) -> Result<(), ApiError> {
        if budget.cancellation.is_cancelled() {
            Err(api_error(ErrorCode::Cancelled, "store call cancelled"))
        } else if budget.deadline_passed(self.context.clock()) {
            Err(api_error(
                ErrorCode::DeadlineExceeded,
                "store call deadline exceeded",
            ))
        } else {
            Ok(())
        }
    }
}

fn due_i64(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "due cursor position overflow"))
}
fn bounded_due_error(error: &ApiError) -> BoundedError {
    let mut detail = format!("{:?}: {}", error.code, error.detail);
    if detail.len() > crate::protocol::results::MAX_LAST_ERROR_BYTES {
        let mut end = crate::protocol::results::MAX_LAST_ERROR_BYTES;
        while !detail.is_char_boundary(end) {
            end -= 1;
        }
        detail.truncate(end);
    }
    BoundedError::parse(detail).expect("bounded due detail")
}
fn invitation_cursor(
    value: &DuePhaseCursor,
) -> Result<invitation_due::InvitationDueCursor, ApiError> {
    if value.receipt_sparse.is_some() || value.after_deadline.is_some() != (value.after_ordinal > 0)
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid invitation due cursor",
        ));
    }
    Ok(invitation_due::InvitationDueCursor {
        high_water_ordinal: due_i64(value.high_water_ordinal)?,
        after_deadline: value.after_deadline.map_or(i64::MIN, |at| at.0),
        after_ordinal: due_i64(value.after_ordinal)?,
    })
}
fn receipt_cursor(value: &DuePhaseCursor) -> Result<receipts::ReceiptDueCursor, ApiError> {
    let sparse = value.receipt_sparse.as_ref();
    if value.after_deadline.is_some() != (value.after_ordinal > 0) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid receipt due cursor",
        ));
    }
    let sparse_high_water_rowid = sparse
        .map(|c| due_i64(c.high_water_rowid))
        .transpose()?
        .unwrap_or(0);
    Ok(receipts::ReceiptDueCursor {
        high_water_ordinal: due_i64(value.high_water_ordinal)?,
        after_deadline: value.after_deadline.map(|at| at.0),
        after_ordinal: due_i64(value.after_ordinal)?,
        sparse_high_water_rowid,
        sparse_after_deadline: sparse.and_then(|c| c.after_deadline.map(|at| at.0)),
        sparse_after_message: sparse
            .and_then(|c| c.after_message.as_ref().map(|id| id.as_str().to_owned())),
        sparse_after_seat: sparse
            .and_then(|c| c.after_seat.as_ref().map(|id| id.as_str().to_owned())),
        next_sparse: sparse.is_some_and(|c| c.next_sparse),
        extension: receipts::ExtensionLapseCursor {
            through: value.extension_through.map(|at| at.0),
            after: value.extension_after.as_ref().map(|k| {
                (
                    k.until.0,
                    k.seat.as_str().to_owned(),
                    k.thread.as_str().to_owned(),
                )
            }),
        },
    })
}
fn exported_receipt_cursor(value: &receipts::ReceiptDueCursor) -> Result<DuePhaseCursor, ApiError> {
    Ok(DuePhaseCursor {
        high_water_ordinal: u64::try_from(value.high_water_ordinal)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative receipt high water"))?,
        after_deadline: value.after_deadline.map(UtcMillis),
        after_ordinal: u64::try_from(value.after_ordinal)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative receipt ordinal"))?,
        receipt_sparse: Some(ReceiptSparseCursor {
            high_water_rowid: u64::try_from(value.sparse_high_water_rowid)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative sparse high water"))?,
            after_deadline: value.sparse_after_deadline.map(UtcMillis),
            after_message: value
                .sparse_after_message
                .as_ref()
                .map(crate::protocol::ids::MessageId::new),
            after_seat: value.sparse_after_seat.as_ref().map(SeatId::new),
            next_sparse: value.next_sparse,
        }),
        extension_through: value.extension.through.map(UtcMillis),
        extension_after: value.extension.after.as_ref().map(|(until, seat, thread)| {
            crate::ports::ExtensionLapseKey {
                until: UtcMillis(*until),
                seat: SeatId::new(seat),
                thread: crate::protocol::ids::ThreadId::new(thread),
            }
        }),
    })
}

fn internal_page_budget_error(detail: &'static str, minimum: usize) -> ApiError {
    ApiError::invalid_budget(detail)
        .with_required_minimum_bytes(minimum.min(u32::MAX as usize) as u32)
}

/// Which internal discovery page a continuation cursor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    Work,
    Wake,
    Recovery,
}

/// On: wake discovery completes each seat within one call, so it never emits the
/// legacy mid-seat fields and a cursor carrying them is `InvalidCursor`.
pub const WAKE_CURSOR_REJECTS_LEGACY_FIELDS: bool = true;

fn encode_internal_cursor(
    instance: &str,
    scope: CursorScope,
    after_ordinal: u64,
    high_water_ordinal: u64,
    legacy: Option<&LegacyWakePosition>,
) -> Result<String, ApiError> {
    Cursor {
        instance: instance.into(),
        scope,
        scope_key: "all".into(),
        filter_digest: "all".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: legacy.map(|legacy| legacy.last_examined_key.clone()),
        after_ordinal,
        high_water_ordinal,
        scope_revision: legacy.map(|legacy| legacy.scope_revision),
        filter_revision: None,
        search: None,
        attention: legacy.map(|legacy| legacy.attention.clone()),
        inbox: None,
        binding: None,
    }
    .encode()
    .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))
}

fn decode_internal_cursor(
    raw: &str,
    instance: &str,
    scope: CursorScope,
) -> Result<Cursor, ApiError> {
    let cursor = Cursor::decode_for(
        raw,
        instance,
        scope,
        "all",
        "all",
        CursorDirection::Ascending,
        1,
    )
    .map_err(|why| api_error(ErrorCode::InvalidCursor, why))?;
    cursor
        .validate_for(instance, scope, "all", "all", CursorDirection::Ascending, 1)
        .map_err(|why| api_error(ErrorCode::InvalidCursor, why))?;
    if cursor.filter_revision.is_some() || cursor.search.is_some() || cursor.inbox.is_some() {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "cursor carries fields foreign to its scope",
        ));
    }
    Ok(cursor)
}

fn reject_positional_fields(cursor: &Cursor) -> Result<(), ApiError> {
    if cursor.last_examined_key.is_some()
        || cursor.scope_revision.is_some()
        || cursor.attention.is_some()
    {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "cursor carries fields foreign to its scope",
        ));
    }
    Ok(())
}

/// Work-discovery continuation (scope WorkJobs, ascending physical ordinal; unchanged by ht-p03.12.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkPageCursor {
    pub after_ordinal: u64,
    pub high_water_ordinal: u64,
}
impl WorkPageCursor {
    pub fn encode(&self, instance: &str) -> Result<String, ApiError> {
        encode_internal_cursor(
            instance,
            CursorScope::WorkJobs,
            self.after_ordinal,
            self.high_water_ordinal,
            None,
        )
    }
    pub fn decode(raw: &str, instance: &str) -> Result<Self, ApiError> {
        let cursor = decode_internal_cursor(raw, instance, CursorScope::WorkJobs)?;
        reject_positional_fields(&cursor)?;
        Ok(Self {
            after_ordinal: cursor.after_ordinal,
            high_water_ordinal: cursor.high_water_ordinal,
        })
    }
}

/// Wake-discovery continuation. Post-D2 shape: ordinals only. `legacy` is the
/// pre-D2 mid-seat position (`last_examined_key`, `scope_revision`, `attention`);
/// discovery never emits it and `decode` rejects it
/// (`WAKE_CURSOR_REJECTS_LEGACY_FIELDS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakePageCursor {
    pub after_ordinal: u64,
    pub high_water_ordinal: u64,
    pub legacy: Option<LegacyWakePosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyWakePosition {
    pub last_examined_key: String,
    pub scope_revision: u64,
    pub attention: crate::protocol::pagination::SeatAttentionCursorState,
}

impl WakePageCursor {
    pub fn encode(&self, instance: &str) -> Result<String, ApiError> {
        encode_internal_cursor(
            instance,
            CursorScope::WakeCandidates,
            self.after_ordinal,
            self.high_water_ordinal,
            self.legacy.as_ref(),
        )
    }
    pub fn decode(raw: &str, instance: &str) -> Result<Self, ApiError> {
        Self::decode_with(raw, instance, WAKE_CURSOR_REJECTS_LEGACY_FIELDS)
    }
    pub fn decode_with(raw: &str, instance: &str, reject_legacy: bool) -> Result<Self, ApiError> {
        let cursor = decode_internal_cursor(raw, instance, CursorScope::WakeCandidates)?;
        let any_legacy = cursor.last_examined_key.is_some()
            || cursor.scope_revision.is_some()
            || cursor.attention.is_some();
        if any_legacy && reject_legacy {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "legacy wake cursor fields are no longer accepted",
            ));
        }
        let legacy = match (
            cursor.last_examined_key,
            cursor.scope_revision,
            cursor.attention,
        ) {
            (None, None, None) => None,
            (Some(last_examined_key), Some(scope_revision), Some(attention)) => {
                Some(LegacyWakePosition {
                    last_examined_key,
                    scope_revision,
                    attention,
                })
            }
            _ => {
                return Err(api_error(
                    ErrorCode::InvalidCursor,
                    "invalid wake attention continuation",
                ));
            }
        };
        Ok(Self {
            after_ordinal: cursor.after_ordinal,
            high_water_ordinal: cursor.high_water_ordinal,
            legacy,
        })
    }
}

/// Wake-recovery continuation, ordered by seat ordinal (preserved by ht-p03.12.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPageCursor {
    pub after_seat_ordinal: u64,
    pub high_water_ordinal: u64,
}
impl RecoveryPageCursor {
    pub fn encode(&self, instance: &str) -> Result<String, ApiError> {
        encode_internal_cursor(
            instance,
            CursorScope::WakeRecovery,
            self.after_seat_ordinal,
            self.high_water_ordinal,
            None,
        )
    }
    pub fn decode(raw: &str, instance: &str) -> Result<Self, ApiError> {
        let cursor = decode_internal_cursor(raw, instance, CursorScope::WakeRecovery)?;
        reject_positional_fields(&cursor)?;
        Ok(Self {
            after_seat_ordinal: cursor.after_ordinal,
            high_water_ordinal: cursor.high_water_ordinal,
        })
    }
}

/// Upper bound on the encoded length of any continuation cursor of `kind`; the
/// base allowance for the page-fit estimate. Work and Recovery cursors are
/// bounded by their widest encoding: the payload carries `after` and
/// `high_water - after` as varints, so their total width peaks at
/// `after = 2^63`, `high_water = u64::MAX`, not at both `u64::MAX`. The Wake bound is
/// the same ordinal-only bound, because
/// `WAKE_CURSOR_REJECTS_LEGACY_FIELDS` rejects the wider legacy encoding.
pub fn longest_cursor_bytes(kind: PageKind) -> usize {
    static ORDINAL_ONLY: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let ordinal_only = *ORDINAL_ONLY.get_or_init(|| {
        let instance = "0".repeat(36);
        let widest = WorkPageCursor {
            after_ordinal: 1 << 63,
            high_water_ordinal: u64::MAX,
        };
        widest
            .encode(&instance)
            .map_or(crate::protocol::pagination::MAX_CURSOR_BYTES, |raw| {
                raw.len()
            })
    });
    match kind {
        PageKind::Work | PageKind::Recovery => ordinal_only,
        PageKind::Wake if WAKE_CURSOR_REJECTS_LEGACY_FIELDS => ordinal_only,
        PageKind::Wake => crate::protocol::pagination::MAX_CURSOR_BYTES,
    }
}

fn work_page_at(
    instance: &str,
    items: Vec<WorkCandidate>,
    after: u64,
    high_water: u64,
    stop: StopReason,
) -> Result<Page<WorkCandidate>, ApiError> {
    let has_more = after < high_water;
    let next_cursor = if has_more {
        Some(
            WorkPageCursor {
                after_ordinal: after,
                high_water_ordinal: high_water,
            }
            .encode(instance)?,
        )
    } else {
        None
    };
    let next_argv = next_cursor
        .as_ref()
        .map(|raw| vec!["work-jobs".into(), "--cursor".into(), raw.clone()]);
    Ok(Page {
        items,
        next_cursor,
        next_argv,
        high_water_ordinal: high_water,
        scope_revision: None,
        has_more,
        stop_reason: if has_more { stop } else { StopReason::Complete },
        consistency: Consistency::BoundedLive,
    })
}

fn wake_page_at(
    instance: &str,
    items: Vec<WakeCandidate>,
    after: u64,
    high_water: u64,
    stop: StopReason,
) -> Result<Page<WakeCandidate>, ApiError> {
    let has_more = after < high_water;
    let next_cursor = if has_more {
        Some(
            WakePageCursor {
                after_ordinal: after,
                high_water_ordinal: high_water,
                legacy: None,
            }
            .encode(instance)?,
        )
    } else {
        None
    };
    let next_argv = next_cursor
        .as_ref()
        .map(|raw| vec!["wake-candidates".into(), "--cursor".into(), raw.clone()]);
    Ok(Page {
        items,
        next_cursor,
        next_argv,
        high_water_ordinal: high_water,
        scope_revision: None,
        has_more,
        stop_reason: if has_more { stop } else { StopReason::Complete },
        consistency: Consistency::BoundedLive,
    })
}

fn wake_recovery_page_at(
    instance: &str,
    items: Vec<WakeRecoveryCandidate>,
    after: u64,
    high_water: u64,
    stop: StopReason,
) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
    let has_more = after < high_water;
    let next_cursor = if has_more {
        Some(
            RecoveryPageCursor {
                after_seat_ordinal: after,
                high_water_ordinal: high_water,
            }
            .encode(instance)?,
        )
    } else {
        None
    };
    let next_argv = next_cursor
        .as_ref()
        .map(|raw| vec!["wake-recovery".into(), "--cursor".into(), raw.clone()]);
    Ok(Page {
        items,
        next_cursor,
        next_argv,
        high_water_ordinal: high_water,
        scope_revision: None,
        has_more,
        stop_reason: if has_more { stop } else { StopReason::Complete },
        consistency: Consistency::BoundedLive,
    })
}

/// Rows one recovery walk query returns.
const WAKE_RECOVERY_WALK_LIMIT: usize = 100;

/// The recovery walk: reserved seats only, in seat-ordinal order (the recovery
/// cursor's order). Cost follows the reserved set, not settled or retired seats.
/// `CROSS JOIN` pins `wake_work` as the outer loop: with a plain join the planner
/// drives from `seats` in ordinal order and walks every seat.
pub(crate) const WAKE_RECOVERY_WALK_SQL: &str = "SELECT s.ordinal,s.id,w.reservation_id,w.reservation_boot FROM wake_work w INDEXED BY wake_work_reserved CROSS JOIN seats s ON s.id=w.seat_id WHERE w.reservation_id IS NOT NULL AND s.instance_id=?1 AND s.ordinal>?2 AND s.ordinal<=?3 ORDER BY s.ordinal LIMIT 100";

fn wake_recovery_page(
    context: &StoreContext,
    instance: &str,
    elected_boot: Option<uuid::Uuid>,
    page: PageRequest,
    budget: &CallBudget,
) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
    page.validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let Some(current_boot) = elected_boot else {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "wake recovery requires elected daemon boot",
        ));
    };
    let cursor = page
        .cursor
        .as_ref()
        .map(|raw| RecoveryPageCursor::decode(raw, instance))
        .transpose()?;
    let db = context.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|error| db.map_error(error))?;
    let high_water = match &cursor {
        Some(cursor) => cursor.high_water_ordinal,
        None => {
            let raw: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE instance_id=?1",
                    [instance],
                    |row| row.get(0),
                )
                .map_err(|error| db.map_error(error))?;
            u64::try_from(raw)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat high water"))?
        }
    };
    let mut after = cursor
        .as_ref()
        .map_or(0, |cursor| cursor.after_seat_ordinal);
    let mut items = Vec::new();
    let mut positions = Vec::new();
    let mut visited = 0usize;
    let mut stop = StopReason::Complete;
    let mut statement = db
        .prepare(WAKE_RECOVERY_WALK_SQL)
        .map_err(|error| db.map_error(error))?;
    let mut rows = statement
        .query(params![instance, due_i64(after)?, due_i64(high_water)?])
        .map_err(|error| db.map_error(error))?;
    while let Some(row) = rows.next().map_err(|error| db.map_error(error))? {
        if budget.is_exhausted(context.clock()) {
            return Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "wake recovery scan budget exhausted",
            ));
        }
        let ordinal: i64 = row.get(0).map_err(store_error)?;
        let seat: String = row.get(1).map_err(store_error)?;
        let attempt: Option<String> = row.get(2).map_err(store_error)?;
        let prior_boot: Option<String> = row.get(3).map_err(store_error)?;
        match (attempt, prior_boot) {
            (Some(attempt), Some(prior_boot)) => {
                let parsed = uuid::Uuid::parse_str(&prior_boot).map_err(|_| {
                    api_error(
                        ErrorCode::StoreCorrupt,
                        "invalid persisted wake daemon boot",
                    )
                })?;
                if parsed != current_boot {
                    if items.len() >= usize::from(page.limit) {
                        stop = StopReason::Rows;
                        break;
                    }
                    positions.push((
                        after,
                        u64::try_from(ordinal).map_err(|_| {
                            api_error(ErrorCode::StoreCorrupt, "negative seat ordinal")
                        })?,
                    ));
                    items.push(WakeRecoveryCandidate {
                        seat: SeatId::new(seat),
                        attempt: WakeAttemptId::new(attempt),
                        prior_daemon_boot: parsed,
                    });
                }
            }
            _ => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "incomplete persisted wake reservation",
                ));
            }
        }
        after = u64::try_from(ordinal)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat ordinal"))?;
        visited += 1;
    }
    // The walk reads reserved seats only, so a short result means the whole
    // range up to the high water has been covered, not that the last reserved
    // seat was the high-water seat.
    if visited < WAKE_RECOVERY_WALK_LIMIT && stop == StopReason::Complete {
        after = high_water;
    }
    if after < high_water && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    page_fit::fit_internal(
        page.max_bytes as usize,
        items,
        &positions,
        |items| wake_recovery_page_at(instance, items, after, high_water, stop),
        |items, before| {
            wake_recovery_page_at(instance, items, before, high_water, StopReason::Bytes)
        },
        "wake recovery page cannot fit",
    )
}

fn pending_work_page(
    context: &StoreContext,
    instance: &str,
    page: PageRequest,
    budget: &CallBudget,
) -> Result<Page<WorkCandidate>, ApiError> {
    page.validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let cursor = page
        .cursor
        .as_ref()
        .map(|raw| WorkPageCursor::decode(raw, instance))
        .transpose()?;
    let db = context.open_query(budget.clone())?;
    pending_work_page_on(context, &db, instance, page, cursor, budget)
}

fn pending_work_page_on(
    context: &StoreContext,
    db: &connection::QueryConnection,
    instance: &str,
    page: PageRequest,
    cursor: Option<WorkPageCursor>,
    budget: &CallBudget,
) -> Result<Page<WorkCandidate>, ApiError> {
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|e| db.map_error(e))?;
    let high_water = match &cursor {
        Some(c) => c.high_water_ordinal,
        None => {
            let value: i64 = db
                .query_row("SELECT COALESCE(MAX(ordinal),0) FROM work_jobs", [], |r| {
                    r.get(0)
                })
                .map_err(|e| db.map_error(e))?;
            u64::try_from(value)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative work high water"))?
        }
    };
    let mut after = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut item_positions = Vec::new();
    let mut visited = 0usize;
    let mut stop = StopReason::Complete;
    let mut statement=db.prepare("SELECT ordinal,id,kind,position,high_water,status FROM work_jobs INDEXED BY work_jobs_live WHERE status IN ('pending','failed') AND ordinal>?1 AND ordinal<=?2 ORDER BY ordinal LIMIT 100").map_err(|e|db.map_error(e))?;
    let mut rows = statement
        .query(params![due_i64(after)?, due_i64(high_water)?])
        .map_err(|e| db.map_error(e))?;
    while let Some(row) = rows.next().map_err(|e| db.map_error(e))? {
        if budget.is_exhausted(context.clock()) {
            return Err(api_error(
                ErrorCode::ReadBudgetExhausted,
                "work discovery budget exhausted",
            ));
        }
        let ordinal: i64 = row.get(0).map_err(store_error)?;
        let id: String = row.get(1).map_err(store_error)?;
        let kind: String = row.get(2).map_err(store_error)?;
        let position: i64 = row.get(3).map_err(store_error)?;
        let high: i64 = row.get(4).map_err(store_error)?;
        let status: String = row.get(5).map_err(store_error)?;
        debug_assert!(
            status == "pending" || status == "failed",
            "work_jobs_live yielded a {status} job"
        );
        if items.len() >= usize::from(page.limit) {
            stop = StopReason::Rows;
            break;
        }
        let kind = match kind.as_str() {
            "warning_attribution" => WorkKind::WarningAttribution,
            "send_attention" => WorkKind::SendAttention,
            "receipt_timer_materialization" => WorkKind::ReceiptTimerMaterialization,
            "preparation_cleanup" => WorkKind::PreparationCleanup,
            _ => return Err(api_error(ErrorCode::StoreCorrupt, "invalid work kind")),
        };
        item_positions.push((
            after,
            u64::try_from(ordinal)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative work ordinal"))?,
        ));
        items.push(WorkCandidate {
            id,
            kind,
            position: u64::try_from(position)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative work position"))?,
            high_water: u64::try_from(high)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative work bound"))?,
            has_more: position < high,
        });
        after = u64::try_from(ordinal)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative work ordinal"))?;
        visited += 1;
    }
    if visited == 100 && after < high_water && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    if visited < 100 && stop == StopReason::Complete {
        // The live-row scan ran dry before the cap: nothing live remains up to
        // the high-water, so the cursor advances past the skipped dead rows.
        after = high_water;
    }
    if after < high_water && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    page_fit::fit_internal(
        page.max_bytes as usize,
        items,
        &item_positions,
        |items| work_page_at(instance, items, after, high_water, stop),
        |items, before| work_page_at(instance, items, before, high_water, StopReason::Bytes),
        "work page cannot fit",
    )
}

/// The seat walk of wake discovery: live (`seats_live_ordinal`) seats in ordinal
/// order, excluding any seat whose current occupant is human (Wave 18).
pub(crate) const WAKE_SEAT_WALK: &str = "SELECT s.ordinal,s.id FROM seats s INDEXED BY seats_live_ordinal WHERE s.instance_id=?1 AND s.state!='retired' AND s.ordinal>?2 AND s.ordinal<=?3 AND NOT EXISTS (SELECT 1 FROM occupant_bindings INDEXED BY occupant_bindings_current WHERE seat_id=s.id AND ended_at IS NULL AND harness='human') ORDER BY s.ordinal LIMIT 1";

/// What one discovery pass found: the candidates, the `(before, ordinal)` position
/// of each, the last seat ordinal passed, and why the pass stopped.
pub(crate) struct WakeDiscovery {
    pub items: Vec<WakeCandidate>,
    pub item_positions: Vec<(u64, u64)>,
    pub after: u64,
    pub stop: StopReason,
}

/// The discovery loop of `wake_candidates_page`, inside the caller's read
/// transaction: walk live non-human seats after `after` up to `high_water`,
/// probe each for pending rows, examine only seats with a hit (one unit of the
/// 100-unit cap each) and emit those whose candidate has actionable work.
pub(crate) fn discover_wake_candidates(
    db: &Connection,
    instance: &str,
    mut after: u64,
    high_water: u64,
    limit: usize,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<WakeDiscovery, ApiError> {
    let mut items = Vec::new();
    let mut item_positions = Vec::new();
    let mut examined = 0u16;
    let mut stop = StopReason::Complete;
    while examined < 100 {
        check_budget()?;
        let next: Option<(i64, String)> = db
            .prepare_cached(WAKE_SEAT_WALK)
            .map_err(store_error)?
            .query_row(
                params![instance, due_i64(after)?, due_i64(high_water)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        let Some((ordinal, seat_raw)) = next else {
            // No live seat remains up to the high water (trailing retired or
            // human seats are never visited): the walk is complete.
            after = high_water;
            break;
        };
        let ordinal = u64::try_from(ordinal)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat ordinal"))?;
        if !attention::seat_has_pending_rows(db, &seat_raw)? {
            after = ordinal;
            continue;
        }
        examined += 1;
        let wake_attention = attention::wake_seat_attention(db, &seat_raw)?;
        let candidate = wake::load_candidate(
            db,
            instance,
            &SeatId::new(&seat_raw),
            &wake_attention.attention,
            wake_attention.decision_seq,
        )?;
        if candidate.has_actionable_work() {
            if items.len() >= limit {
                stop = StopReason::Rows;
                break;
            }
            item_positions.push((after, ordinal));
            items.push(candidate);
        }
        after = ordinal;
    }
    if after < high_water && stop == StopReason::Complete {
        stop = StopReason::Work;
    }
    Ok(WakeDiscovery {
        items,
        item_positions,
        after,
        stop,
    })
}

fn wake_candidates_page(
    context: &StoreContext,
    instance: &str,
    page: PageRequest,
    budget: &CallBudget,
) -> Result<Page<WakeCandidate>, ApiError> {
    page.validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let cursor = page
        .cursor
        .as_ref()
        .map(|raw| WakePageCursor::decode(raw, instance))
        .transpose()?;
    let db = context.open_query(budget.clone())?;
    db.execute_batch("BEGIN DEFERRED")
        .map_err(|e| db.map_error(e))?;
    let high_water = if let Some(cursor) = &cursor {
        cursor.high_water_ordinal
    } else {
        let value: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE instance_id=?1",
                [instance],
                |r| r.get(0),
            )
            .map_err(|e| db.map_error(e))?;
        u64::try_from(value)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat high water"))?
    };
    let WakeDiscovery {
        items,
        item_positions,
        after,
        stop,
    } = discover_wake_candidates(
        &db,
        instance,
        cursor.as_ref().map_or(0, |c| c.after_ordinal),
        high_water,
        usize::from(page.limit),
        &|| {
            if budget.is_exhausted(context.clock()) {
                return Err(api_error(
                    ErrorCode::ReadBudgetExhausted,
                    "wake discovery budget exhausted",
                ));
            }
            Ok(())
        },
    )?;
    page_fit::fit_internal(
        page.max_bytes as usize,
        items,
        &item_positions,
        |items| wake_page_at(instance, items, after, high_water, stop),
        |items, before| wake_page_at(instance, items, before, high_water, StopReason::Bytes),
        "wake candidate page cannot fit",
    )
}

impl StorePort for SqliteStore {
    fn clock(&self) -> &dyn Clock {
        self.context.clock()
    }

    fn audit_service_disconnect(
        &self,
        boot: &str,
        generation: u64,
        peer: crate::protocol::authority::PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        use sha2::{Digest, Sha256};
        if generation == 0
            || self
                .settings
                .daemon_boot
                .is_some_and(|current| current.to_string() != boot)
        {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "invalid service recovery audit target",
            ));
        }
        let scope = format!("service-recovery-audit:{}", self.instance);
        let key = format!("{boot}:{generation}");
        let payload = serde_json::json!({
            "event": "operator_service_disconnect",
            "instance": self.instance,
            "daemon_boot": boot,
            "connection_generation": generation,
            "actor": format!("operator:local-user:{}", peer.effective_uid()),
        });
        let json = serde_json::to_string(&payload).map_err(|_| {
            api_error(
                ErrorCode::StoreCorrupt,
                "cannot encode service recovery audit",
            )
        })?;
        let digest = Sha256::digest(json.as_bytes());
        // Reserve time in the ordinary call for reporting a failed audit.
        let audit_budget = CallBudget {
            deadline: crate::protocol::time::MonoInstant(
                self.context
                    .clock()
                    .monotonic_now()
                    .0
                    .saturating_add(1_000)
                    .min(budget.deadline.0.saturating_sub(100)),
            ),
            cancellation: budget.cancellation.clone(),
        };
        let mut writer = self.writer(&audit_budget)?;
        self.context.execute_budgeted_decision(
            &mut writer,
            &audit_budget,
            |_| Ok(()),
            |tx, decision, ()| {
                tx.execute(
                    "INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES (?1,?2,?3,?4,?5)",
                    params![scope, key, digest.as_slice(), json, decision.utc.0],
                ).map_err(store_error)?;
                Ok(())
            },
        )
    }

    fn service_operation(
        &self,
        operation: crate::protocol::service::ServiceOperation,
        connection: &crate::ports::ServiceConnectionAuthority,
        gate: &dyn crate::ports::ServiceAuthorityGate,
        budget: &CallBudget,
        admission: Option<&crate::service::fair_writer::FairWriter>,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        operation
            .validate()
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
        match operation {
            crate::protocol::service::ServiceOperation::Send(ref request) => {
                return self.service_send_with_admission(
                    request,
                    connection,
                    gate,
                    budget,
                    admission,
                    |_, _| {},
                );
            }
            // Reads run on the reader path: no writer, no admission turn, no
            // authority guard, nothing mutated.
            crate::protocol::service::ServiceOperation::History(ref q) => {
                self.check_service_read_instance(connection)?;
                return match queries::query_with_output(
                    &self.context,
                    &self.instance,
                    &Command::History(crate::protocol::commands::HistoryQuery {
                        thread: q.thread.clone(),
                        page: q.page.clone(),
                        initial: q.initial.clone(),
                        full_bodies: false,
                    }),
                    &crate::protocol::output::OutputSpec::default(),
                    budget,
                )? {
                    CommandResult::History(page) => {
                        Ok(crate::protocol::service::ServiceResult::History(page))
                    }
                    _ => Err(api_error(ErrorCode::StoreCorrupt, "unexpected read result")),
                };
            }
            crate::protocol::service::ServiceOperation::Receipts(ref q) => {
                self.check_service_read_instance(connection)?;
                return match queries::service_receipts(
                    &self.context,
                    &self.instance,
                    connection.author(),
                    &crate::protocol::commands::DeliveryInspectQuery {
                        message: q.message.clone(),
                        page: q.page.clone(),
                    },
                    budget,
                )? {
                    CommandResult::DeliveryInspect(inspection) => Ok(
                        crate::protocol::service::ServiceResult::Receipts(inspection),
                    ),
                    _ => Err(api_error(ErrorCode::StoreCorrupt, "unexpected read result")),
                };
            }
            _ => {}
        }
        if let crate::protocol::service::ServiceOperation::Notify(ref request) = operation {
            return self.service_notify_with_admission(
                request,
                connection,
                gate,
                budget,
                admission,
                |_, _| {},
            );
        }
        let _turn = admission
            .map(|lane| lane.enter_foreground(budget, self.context.clock()))
            .transpose()?;
        let mut writer = self.writer(budget)?;
        service_controls::operate(
            &self.context,
            &mut writer,
            &self.instance,
            operation,
            connection,
            gate,
            budget,
            self.settings.invitation_default_ms,
        )
    }

    fn query(
        &self,
        command: &Command,
        read: &ReadContext,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        read.validate()
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
        if read.instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "read instance mismatch",
            ));
        }
        if let Command::OperationStatus(status) = command {
            let scope = read.operation_scope.as_ref().ok_or_else(|| {
                api_error(ErrorCode::Unauthorized, "trusted operation scope required")
            })?;
            let result = queries::query_operation_status(
                &self.context,
                &self.instance,
                scope,
                status,
                budget,
            )?;
            crate::protocol::output::encode_selected(&result, &read.output)?;
            return Ok(result);
        }
        if let Command::HotThreads(q) = command {
            let result = queries::hot_threads(
                &self.context,
                &self.instance,
                q,
                self.settings.summary.hot_window_ms,
                budget,
            )?;
            crate::protocol::output::encode_selected(&result, &read.output)?;
            return Ok(result);
        }
        let mut selected = command.clone();
        let needs_seat = matches!(&selected,Command::Inbox(q) if q.seat.is_none())
            || matches!(&selected,Command::Directory(q) if q.membership.is_none() && q.membership_filter!=DirectoryMembership::All);
        if needs_seat {
            let Some(OperationReadScope::Seat(seat)) = &read.operation_scope else {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "selected seat required for this query",
                ));
            };
            let db = self.context.open_query(budget.clone())?;
            let owned: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
                    params![seat.as_str(), self.instance],
                    |r| r.get(0),
                )
                .map_err(|e| db.map_error(e))?;
            if !owned {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    "selected seat does not belong to instance",
                ));
            }
            match &mut selected {
                Command::Inbox(q) => q.seat = Some(seat.clone()),
                Command::Directory(q) => q.membership = Some(seat.clone()),
                _ => unreachable!(),
            }
        }
        queries::query_with_output(
            &self.context,
            &self.instance,
            &selected,
            &read.output,
            budget,
        )
    }

    fn mutate(
        &self,
        command: PermitMutation,
        permit: MutationPermit,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if permit.claim().instance != self.instance {
            return Err(api_error(
                ErrorCode::CallerUnverified,
                "cooperative permit instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        match command {
            PermitMutation::CheckIn(_) => Err(api_error(
                ErrorCode::InvalidRequest,
                "check-in requires verified registration and read context",
            )),
            PermitMutation::CreateThread(v) => {
                control::create_thread(&self.context, &mut writer, budget, &v, permit)
            }
            PermitMutation::Invite(v) => control::invite(
                &self.context,
                &mut writer,
                budget,
                &v,
                permit,
                self.settings.invitation_default_ms,
            ),
            PermitMutation::Accept(v) => {
                let result = control::accept(&self.context, &mut writer, budget, &v, permit)?;
                Ok(self.with_join_hint(result, &v.thread, budget))
            }
            PermitMutation::AcceptRequired(v) => {
                control::accept_required(&self.context, &mut writer, budget, &v, permit)
            }
            PermitMutation::SendMessage(v) => {
                let mut permit = permit;
                messages::publish_send(&self.context, &mut writer, &v, &mut permit, budget, || {
                    self.settings.message_limits.body_bytes
                })
            }
            PermitMutation::Ack(v) => {
                let mut permit = permit;
                receipts::ack(&self.context, &mut writer, budget, &v, &mut permit)
            }
            PermitMutation::Leave(v) => {
                control::leave(&self.context, &mut writer, budget, &v, permit)
            }
            PermitMutation::SetTopic(v) => {
                control::set_topic(&self.context, &mut writer, budget, &v, permit)
            }
            PermitMutation::Archive(v) => {
                control::archive(&self.context, &mut writer, budget, &v, permit)
            }
            PermitMutation::Reopen(v) => {
                control::reopen(&self.context, &mut writer, budget, &v, permit)
            }
        }
    }
    fn prepare_send_step(
        &self,
        request: &SendMessage,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SendPreparationProgress, ApiError> {
        if request.claim.instance != self.instance {
            return Err(api_error(
                ErrorCode::CallerUnverified,
                "cooperative send instance mismatch",
            ));
        }

        let mut writer = self.writer(budget)?;
        messages::prepare_send_step(
            &self.context,
            &mut writer,
            request,
            self.settings.message_limits,
            budget,
            admission,
        )
    }
    fn abandon_send_preparation(
        &self,
        expected_preparation_id: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let mut writer = self.writer(budget)?;
        messages::abandon_send_preparation(&mut writer, expected_preparation_id)?;
        Ok(())
    }
    fn resolve_seat(
        &self,
        request: ResolveSeat,
        attempt: crate::ports::OrdinaryResolutionAttempt,
        budget: &CallBudget,
    ) -> Result<crate::ports::OrdinaryResolutionOutcome, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::resolve_seat(
            &self.context,
            &mut writer,
            &self.instance,
            request,
            attempt,
            budget,
        )
    }
    fn check_resolved_target(
        &self,
        check: crate::ports::ResolvedTargetCheck,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let mut writer = self.writer(budget)?;
        seats::check_resolved_target(&self.context, &mut writer, &self.instance, check, budget)
    }
    fn replay_operator(
        &self,
        command: OperatorCommand,
        actor: OperatorActor,
        budget: &CallBudget,
    ) -> Result<Option<CommandResult>, ApiError> {
        self.live_budget(budget)?;
        let result = operator::replay(&self.context, &self.instance, &command, &actor, budget)?;
        self.live_budget(budget)?;
        Ok(result)
    }
    fn mutate_operator(
        &self,
        command: OperatorRequest,
        actor: OperatorActor,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::mutate_operator(
            &self.context,
            &mut writer,
            &self.instance,
            command,
            actor,
            self.settings.invitation_default_ms,
        )
    }
    fn replay_continuity(
        &self,
        command: crate::protocol::commands::ContinuityCheckIn,
        budget: &CallBudget,
    ) -> Result<Option<CommandResult>, ApiError> {
        self.live_budget(budget)?;
        let result = seats::replay_continuity(&self.context, &self.instance, &command, budget)?;
        self.live_budget(budget)?;
        Ok(result)
    }
    fn decide_continuity(
        &self,
        request: crate::ports::ContinuityRequest,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::decide_continuity(&self.context, &mut writer, &self.instance, request)
    }
    fn record_managed_launch(
        &self,
        command: crate::protocol::commands::RecordManagedLaunch,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::record_managed_launch(&self.context, &mut writer, &self.instance, &command)
    }
    fn issue_cooperative_permit(
        &self,
        request: crate::ports::CooperativePermitRequest,
        budget: &CallBudget,
    ) -> Result<MutationPermit, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        let tx = db.unchecked_transaction().map_err(store_error)?;
        let permit =
            seats::issue_cooperative_permit(&self.context, &tx, &self.instance, request, budget)?;
        tx.rollback().map_err(store_error)?;
        self.live_budget(budget)?;
        Ok(permit)
    }

    fn register_available(
        &self,
        request: RegisterAvailableRequest,
        permit: MutationPermit,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        request
            .read
            .validate()
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
        if request.read.instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "registration instance mismatch",
            ));
        }
        if permit.claim().instance != self.instance
            || request.command.claim.instance != self.instance
        {
            return Err(api_error(
                ErrorCode::CallerUnverified,
                "cooperative registration instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        let result = seats::register_available(
            &self.context,
            &mut writer,
            &request.command,
            request.operator.as_ref(),
            permit,
            budget,
            |tx, seat, seq| {
                let _progress = WriterProgressGuard::install(tx, budget, self.context.clock());
                let page = PageRequest {
                    max_bytes: 8_000,
                    ..PageRequest::default()
                };
                let inbox = queries::inbox_in_transaction(
                    tx,
                    &self.instance,
                    seat,
                    &page,
                    &request.read.output,
                    budget,
                    self.context.clock(),
                )?;
                let (warning_count, warning_count_has_more) =
                    queries::pending_warning_count_in_transaction(
                        tx,
                        &self.instance,
                        seat,
                        budget,
                        self.context.clock(),
                    )?;
                let warnings = queries::warnings_in_transaction(
                    tx,
                    &self.instance,
                    &WarningsQuery {
                        seat: seat.clone(),
                        page,
                    },
                    &request.read.output,
                )?;
                let CommandResult::Warnings(warnings) = warnings else {
                    return Err(api_error(
                        ErrorCode::StoreCorrupt,
                        "warning query returned wrong result",
                    ));
                };
                let mut returned_context = request.command.claim.clone();
                returned_context.instance = self.instance.clone();
                returned_context.seat = seat.clone();
                let generation: i64 = tx
                    .query_row(
                        "SELECT generation FROM seats WHERE id=?1",
                        [seat.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                returned_context.binding_generation = u64::try_from(generation).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative binding generation")
                })?;
                // The capped page of programmatic notices above the current
                // occupant's offered frontier, oldest first; the writer
                // settles exactly the prefix this offer carries.
                let mut notices = crate::store::attention::notice_offer_page(
                    tx,
                    seat.as_str(),
                    crate::protocol::results::MAX_NOTICE_PAGE_ITEMS + 1,
                )?
                .into_iter()
                .map(|offered| offered.notice)
                .collect::<Vec<_>>();
                let mut notices_has_more =
                    notices.len() > crate::protocol::results::MAX_NOTICE_PAGE_ITEMS;
                notices.truncate(crate::protocol::results::MAX_NOTICE_PAGE_ITEMS);
                let mut offer = crate::protocol::results::CheckInResult {
                    context_disposition:
                        crate::protocol::results::CheckInContextDisposition::Current,
                    context: returned_context,
                    seat: seat.clone(),
                    offered_through: Some(seq.to_string()),
                    warning_count,
                    warning_count_has_more,
                    warnings,
                    notices: crate::protocol::results::NoticeOffer::default(),
                    inbox,
                };
                // Trim the notice page (never the rest of the offer) to the
                // selected output bound: only notices the offer carries settle.
                loop {
                    offer.notices = crate::protocol::results::NoticeOffer {
                        items: notices.clone(),
                        has_more: notices_has_more,
                    };
                    let encoded = crate::protocol::output::encode_selected(
                        &CommandResult::CheckedIn(offer.clone()),
                        &request.read.output,
                    )?;
                    if encoded.len().saturating_add(3) <= PageRequest::default().max_bytes as usize
                    {
                        break;
                    }
                    if notices.pop().is_none() {
                        return Err(api_error(
                            ErrorCode::InvalidBudget,
                            "complete check-in offer exceeds selected output bound",
                        ));
                    }
                    notices_has_more = true;
                }
                Ok(offer)
            },
        );
        match result {
            Err(error)
                if error.code == ErrorCode::Cancelled
                    && !budget.cancellation.is_cancelled()
                    && budget.deadline_passed(self.context.clock()) =>
            {
                Err(api_error(
                    ErrorCode::ReadBudgetExhausted,
                    "registration offer read budget exhausted",
                ))
            }
            other => other,
        }
    }
    fn persisted_host_epoch(&self, instance: &str, budget: &CallBudget) -> Result<u64, ApiError> {
        if instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "host instance mismatch",
            ));
        }
        let writer = self.writer(budget)?;
        seats::persisted_host_epoch(&writer, instance)
    }
    fn begin_host_observation(
        &self,
        instance: &str,
        budget: &CallBudget,
    ) -> Result<HostObservationAdmission, ApiError> {
        if instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "host instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::begin_host_observation(&self.context, &mut writer, instance, budget)
    }
    fn publish_current_target_observation(
        &self,
        admission: &HostObservationAdmission,
        observation: &HostObservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        if admission.instance() != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "host admission instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::publish_current_target_observation(
            &self.context,
            &mut writer,
            admission,
            observation,
            budget,
        )
    }
    fn invalidate_host_observation(
        &self,
        admission: &HostObservationAdmission,
        reason: HostInvalidationReason,
        budget: &CallBudget,
    ) -> Result<Option<HostInvalidationFence>, ApiError> {
        if admission.instance() != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "host admission instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::invalidate_host_observation(&self.context, &mut writer, admission, reason, budget)
    }
    fn mark_unresolved_from_invalidation(
        &self,
        transition: GuardedInvalidationTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError> {
        if transition.fence.instance() != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "host invalidation instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::mark_unresolved_from_invalidation(&self.context, &mut writer, transition, budget)
    }
    fn saved_seats_page_for_invalidation(
        &self,
        fence: &HostInvalidationFence,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<InvalidationSeatPage, ApiError> {
        if fence.instance() != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "host invalidation instance mismatch",
            ));
        }
        let db = self.context.open_query(budget.clone())?;
        seats::saved_seats_page_for_invalidation(
            &self.context,
            &db,
            fence,
            after_ordinal,
            high_water_ordinal,
            limit,
            budget,
        )
    }
    fn begin_snapshot_stage(
        &self,
        header: SnapshotHeader,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError> {
        if header.instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "snapshot instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::begin_snapshot_stage(&self.context, &mut writer, header, budget)
    }
    fn stage_snapshot_targets(
        &self,
        stage: &SnapshotGenerationId,
        offset: u64,
        targets: &[HostObservation],
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotStageProgress, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::stage_snapshot_targets(
            &self.context,
            &mut writer,
            stage,
            offset,
            targets,
            admission,
            budget,
        )
    }
    fn seal_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::seal_snapshot_stage(&self.context, &mut writer, stage, budget)
    }
    fn publish_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<PublishedSnapshot, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::publish_snapshot_stage(&self.context, &mut writer, stage, budget)
    }
    fn discard_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotCleanupProgress, ApiError> {
        let mut writer = self.writer(budget)?;
        seats::discard_snapshot_stage(&self.context, &mut writer, stage, admission, budget)
    }
    fn prune_retention(&self, budget: &CallBudget) -> Result<PruneProgress, ApiError> {
        retention::prune_once(self, budget)
    }
    fn record_harness_evidence(
        &self,
        record: &harness_evidence::EvidenceRecord<'_>,
        budget: &CallBudget,
    ) -> Result<harness_evidence::Recorded, ApiError> {
        let mut writer = self.writer(budget)?;
        harness_evidence::record(&self.context, &mut writer, record)
    }
    fn harness_evidence(
        &self,
        harness: &str,
        version: &str,
        contract_id: &str,
        budget: &CallBudget,
    ) -> Result<Option<harness_evidence::EvidenceRow>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        harness_evidence::get(&db, harness, version, contract_id)
    }
    fn harness_evidence_since(
        &self,
        since_ms: u64,
        budget: &CallBudget,
    ) -> Result<Vec<harness_evidence::EvidenceRow>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        harness_evidence::since(&db, since_ms)
    }
    fn harness_evidence_all(
        &self,
        harness: &str,
        budget: &CallBudget,
    ) -> Result<Vec<harness_evidence::EvidenceRow>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        harness_evidence::all(&db, harness)
    }
    fn record_unattributed(
        &self,
        harness: &str,
        reason: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let mut writer = self.writer(budget)?;
        harness_evidence::record_unattributed(&self.context, &mut writer, harness, reason)
    }
    fn last_unattributed(
        &self,
        harness: &str,
        budget: &CallBudget,
    ) -> Result<Option<(String, u64)>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        harness_evidence::last_unattributed(&db, harness)
    }
    fn saved_seats_page(
        &self,
        published: &SnapshotGenerationId,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<SnapshotSeatPage, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        let page = seats::saved_seats_page(
            &self.context,
            &db,
            published,
            after_ordinal,
            high_water_ordinal,
            limit,
            budget,
        )?;
        if page.publication.instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "snapshot page instance mismatch",
            ));
        }
        Ok(page)
    }
    fn apply_reconciliation_transition(
        &self,
        transition: GuardedSeatTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError> {
        if transition.publication.instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "reconciliation instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::apply_reconciliation_transition(&self.context, &mut writer, transition, budget)
    }
    fn record_reconciliation_pass(
        &self,
        published: &PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        if published.instance != self.instance {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "reconciliation instance mismatch",
            ));
        }
        let mut writer = self.writer(budget)?;
        seats::record_reconciliation_pass(&self.context, &mut writer, published, budget)
    }
    fn due_obligations(
        &self,
        request: DueScanRequest,
        budget: &CallBudget,
    ) -> Result<DueScanProgress, ApiError> {
        request
            .validate()
            .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
        if let Some(cursor) = &request.state.invitations {
            invitation_cursor(cursor)?;
        }
        if let Some(cursor) = &request.state.receipts {
            receipt_cursor(cursor)?;
        }
        let mut writer = self.writer(budget)?;
        let mut state = request.state.clone();
        let first = if request.run_invitations && request.run_receipts {
            state.next_phase
        } else if request.run_invitations {
            DuePhase::Invitations
        } else {
            DuePhase::Receipts
        };
        state.next_phase = match first {
            DuePhase::Invitations => DuePhase::Receipts,
            DuePhase::Receipts => DuePhase::Invitations,
        };
        let (first_cap, second_cap) = if request.run_invitations && request.run_receipts {
            (
                request.max_candidates.div_ceil(2),
                request.max_candidates / 2,
            )
        } else {
            (request.max_candidates, 0)
        };
        let mut examined = 0u16;
        let mut warnings = 0u16;
        let mut invitation_phase = DuePhaseProgress::Skipped;
        let mut receipt_phase = DuePhaseProgress::Skipped;
        for (index, phase) in [first, state.next_phase].into_iter().enumerate() {
            let cap = if index == 0 { first_cap } else { second_cap };
            if cap == 0 {
                continue;
            }
            match phase {
                DuePhase::Invitations if request.run_invitations => {
                    let cursor = state
                        .invitations
                        .as_ref()
                        .map(invitation_cursor)
                        .transpose()?;
                    match invitation_due::scan_invitation_due_batch(
                        &self.context,
                        &mut writer,
                        cursor,
                        cap,
                    ) {
                        Ok(batch) => {
                            examined += batch.inspected;
                            warnings += batch.warnings_added;
                            state.invitations = batch.next.map(|next| DuePhaseCursor {
                                high_water_ordinal: next.high_water_ordinal as u64,
                                after_deadline: Some(UtcMillis(next.after_deadline)),
                                after_ordinal: next.after_ordinal as u64,
                                receipt_sparse: None,
                                extension_through: None,
                                extension_after: None,
                            });
                            invitation_phase = if state.invitations.is_some() {
                                DuePhaseProgress::More
                            } else {
                                DuePhaseProgress::Complete
                            };
                        }
                        Err(error) => {
                            invitation_phase = DuePhaseProgress::Failed(
                                error.code.clone(),
                                bounded_due_error(&error),
                            )
                        }
                    }
                }
                DuePhase::Receipts if request.run_receipts => {
                    let mut cursor = state
                        .receipts
                        .as_ref()
                        .map(receipt_cursor)
                        .transpose()?
                        .unwrap_or_default();
                    // Catch-up stall scan (spec §7): rows whose extension lapsed
                    // end `stalled` and release what they held, in their own
                    // short decision, before the receipt scan.
                    let stalled = self.context.execute_decision(
                        &mut writer,
                        |_| Ok(()),
                        |tx, at, ()| catch_up::stall_scan(tx, at.utc, CATCH_UP_STALL_SCAN_ROWS),
                    );
                    let scanned = match stalled {
                        Ok(ended) => {
                            examined += ended;
                            receipts::scan_due(&self.context, &mut writer, cap, &mut cursor)
                                .and_then(|mut due| {
                                    // Extension lapses (spec §8): scan_due skips
                                    // candidates whose effective deadline is
                                    // future; the lapse itself is found here.
                                    let lapses = receipts::scan_extension_lapses(
                                        &self.context,
                                        &mut writer,
                                        &mut cursor,
                                    )?;
                                    due.warnings = due.warnings.saturating_add(lapses.warnings);
                                    due.inspected = due.inspected.saturating_add(lapses.inspected);
                                    due.more |= lapses.more;
                                    Ok(due)
                                })
                        }
                        Err(error) => Err(error),
                    };
                    match scanned {
                        Ok(result) => {
                            examined += result.inspected;
                            warnings += result.warnings;
                            state.receipts = if result.more || cursor.extension.through.is_some() {
                                Some(exported_receipt_cursor(&cursor)?)
                            } else {
                                None
                            };
                            receipt_phase = if result.more {
                                DuePhaseProgress::More
                            } else {
                                DuePhaseProgress::Complete
                            };
                        }
                        Err(error) => {
                            receipt_phase = DuePhaseProgress::Failed(
                                error.code.clone(),
                                bounded_due_error(&error),
                            )
                        }
                    }
                }
                _ => {}
            }
        }
        let has_more = matches!(
            invitation_phase,
            DuePhaseProgress::More | DuePhaseProgress::Failed(..)
        ) || matches!(
            receipt_phase,
            DuePhaseProgress::More | DuePhaseProgress::Failed(..)
        ) || (request.run_invitations && request.run_receipts && second_cap == 0);
        Ok(DueScanProgress {
            state,
            examined_candidates: examined,
            warnings_added: warnings,
            invitations: invitation_phase,
            receipts: receipt_phase,
            has_more,
        })
    }
    fn begin_retirement(
        &self,
        seat: SeatId,
        proof: ClosureEvidence,
        budget: &CallBudget,
    ) -> Result<RetirementJob, ApiError> {
        let mut writer = self.writer(budget)?;
        control::begin_retirement(&self.context, &mut writer, seat, proof)
    }
    fn advance_retirement(
        &self,
        job: RetirementJobId,
        admission: WorkAdmission,
        budget: &CallBudget,
    ) -> Result<RetirementProgress, ApiError> {
        let mut writer = self.writer(budget)?;
        control::advance_retirement(&self.context, &mut writer, job, admission, budget)
    }
    fn pending_retirement_jobs(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError> {
        let result = queries::query_with_output(
            &self.context,
            &self.instance,
            &Command::RetirementJobs(RetirementJobsQuery { page }),
            &crate::protocol::output::OutputSpec::default(),
            budget,
        )?;
        match result {
            CommandResult::RetirementJobs(page) => Ok(page),
            _ => Err(api_error(
                ErrorCode::StoreCorrupt,
                "retirement query returned wrong result",
            )),
        }
    }
    fn binding_evidence_startup(&self) -> Option<crate::ports::BindingEvidenceStartup> {
        self.context.binding_evidence_startup()
    }
    fn binding_evidence_current(
        &self,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::BindingEvidenceStartup>, ApiError> {
        self.context.binding_evidence_current(budget)
    }
    fn unresolved_seat_summary(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::ports::UnresolvedSeatSummary, ApiError> {
        queries::unresolved_seat_summary(&self.context, &self.instance, budget)
    }
    fn retirement_summary(&self, budget: &CallBudget) -> Result<RetirementSummary, ApiError> {
        queries::retirement_summary(&self.context, &self.instance, budget)
    }
    fn summary(
        &self,
        request: &crate::protocol::summary::SummaryRequest,
        budget: &CallBudget,
    ) -> Result<crate::protocol::summary::SummaryOutcome, ApiError> {
        let mut writer = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut writer,
            budget,
            |_| Ok(()),
            |tx, decision, ()| {
                summary::summary(
                    tx,
                    &self.instance,
                    request,
                    &self.settings.summary,
                    decision.utc,
                )
            },
        )
    }
    fn summary_job(
        &self,
        request: &crate::protocol::summary::SummaryJobRequest,
        budget: &CallBudget,
    ) -> Result<crate::protocol::summary::SummaryJobOutcome, ApiError> {
        let mut writer = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut writer,
            budget,
            |_| Ok(()),
            |tx, decision, ()| {
                summary::summary_job(
                    tx,
                    &self.instance,
                    request,
                    &self.settings.summary,
                    decision.utc,
                )
            },
        )
    }
    fn summary_submit(
        &self,
        request: &crate::protocol::summary::SummarySubmitRequest,
        budget: &CallBudget,
    ) -> Result<crate::protocol::summary::SubmitOutcome, ApiError> {
        let mut writer = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut writer,
            budget,
            |_| Ok(()),
            |tx, decision, ()| {
                summary::summary_submit(
                    tx,
                    &self.instance,
                    request,
                    &self.settings.summary,
                    decision.utc,
                )
            },
        )
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError> {
        wake_candidates_page(&self.context, &self.instance, page, budget)
    }
    fn pending_work(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WorkCandidate>, ApiError> {
        pending_work_page(&self.context, &self.instance, page, budget)
    }
    fn advance_work(
        &self,
        job: &str,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<WorkProgress, ApiError> {
        let mut writer = self.writer(budget)?;
        materialization::advance_work(&mut writer, job, admission, budget, self.context.clock())
    }
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError> {
        let Some(daemon_boot) = self.settings.daemon_boot else {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "elected daemon boot required",
            ));
        };
        let mut writer = self.writer(budget)?;
        wake::reserve(
            &self.context,
            &mut writer,
            &self.instance,
            candidate,
            daemon_boot,
            self.settings.minimum_wake_delay_ms,
        )
    }
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
        wake_recovery_page(
            &self.context,
            &self.instance,
            self.settings.daemon_boot,
            page,
            budget,
        )
    }
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError> {
        let mut writer = self.writer(budget)?;
        wake::recover_abandoned(
            &self.context,
            &mut writer,
            &self.instance,
            self.settings.daemon_boot,
            &request,
        )
    }
    fn validate_wake_reservation(
        &self,
        reservation: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        db.execute_batch("BEGIN DEFERRED")
            .map_err(|e| db.map_error(e))?;
        wake::validate_reservation(&db, &self.instance, reservation)
    }
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        let Some(daemon_boot) = &self.settings.daemon_boot else {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "elected daemon boot required",
            ));
        };
        let mut writer = self.writer(budget)?;
        wake::complete(
            &self.context,
            &mut writer,
            &attempt,
            daemon_boot,
            outcome,
            refused_restore,
            budget,
        )
    }
    fn poke_candidates(&self, limit: u16, budget: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        db.execute_batch("BEGIN DEFERRED")
            .map_err(|e| db.map_error(e))?;
        poke::due_pokes(
            &db,
            self.context.clock().utc_now().0,
            &self.settings.summary,
            limit,
        )
    }
    fn poke_for_wake(
        &self,
        seat: &SeatId,
        budget: &CallBudget,
    ) -> Result<Option<PokeDue>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        db.execute_batch("BEGIN DEFERRED")
            .map_err(|e| db.map_error(e))?;
        poke::due_pokes_for_seat(
            &db,
            seat,
            self.context.clock().utc_now().0,
            &self.settings.summary,
        )
    }
    fn reserve_poke(
        &self,
        due: &PokeDue,
        budget: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        let Some(daemon_boot) = self.settings.daemon_boot else {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "elected daemon boot required",
            ));
        };
        let mut writer = self.writer(budget)?;
        wake::reserve_poke(
            &self.context,
            &mut writer,
            &self.instance,
            due,
            daemon_boot,
            &self.settings.summary,
        )
    }
    fn complete_poke(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        receipts: &[PokeReceipt],
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        let Some(daemon_boot) = &self.settings.daemon_boot else {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "elected daemon boot required",
            ));
        };
        let mut writer = self.writer(budget)?;
        // A poke reservation never advanced the wake ladder, so there is no
        // prior ladder row to restore on a refusal.
        wake::complete_with_pokes(
            &self.context,
            &mut writer,
            &attempt,
            daemon_boot,
            outcome,
            None,
            budget,
            receipts,
        )
        .map(|_| ())
    }
}

#[cfg(test)]
#[path = "../../tests/store/discovery_cost.rs"]
mod discovery_cost_tests;
#[cfg(test)]
#[path = "../../tests/store/page_cursor_seam.rs"]
mod page_cursor_seam_tests;
#[cfg(test)]
#[path = "../../tests/store/page_cursor.rs"]
mod page_cursor_tests;
#[cfg(test)]
#[path = "../../tests/store/service_reads.rs"]
mod service_reads_tests;
#[cfg(test)]
#[path = "../../tests/store/facade.rs"]
mod tests;
#[cfg(test)]
#[path = "../../tests/store/wake.rs"]
mod wake_tests;

#[cfg(test)]
#[path = "../../tests/store/writer_budget.rs"]
mod writer_budget_tests;

#[cfg(test)]
#[path = "../../tests/store/commit_kicks.rs"]
mod commit_kicks_tests;

#[cfg(test)]
#[path = "../../tests/store/cooperative_checkin.rs"]
mod cooperative_checkin_tests;

#[cfg(test)]
#[path = "../../tests/store/wake_discovery_cost.rs"]
mod discovery_cost;

/// Canonical service-local permit input for every accountable cooperative route.
/// Accept freezes its current InvitationId inside the issuer, preserving episode
/// races. The unresolved request marker can never authorize a new mutation.
pub fn cooperative_permit_request(
    command: &PermitMutation,
) -> Result<crate::ports::CooperativePermitRequest, ApiError> {
    use crate::protocol::authority::ObligationRef;
    let (claim, operation, obligation, payload_hash, check_in_mode) = match command {
        PermitMutation::CheckIn(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::CheckIn(v.claim.seat.clone()),
            schema::canonical_digest(&seats::check_in_payload(v))?,
            Some(v.mode),
        ),
        PermitMutation::CreateThread(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::CheckIn(v.claim.seat.clone()),
            control::cooperative_payload_hash("create_thread", v)?,
            None,
        ),
        PermitMutation::Invite(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Control(v.thread.clone()),
            control::cooperative_payload_hash("invite", v)?,
            None,
        ),
        PermitMutation::Accept(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::AcceptCurrent(v.thread.clone()),
            control::cooperative_payload_hash("accept", v)?,
            None,
        ),
        PermitMutation::AcceptRequired(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Invitation(v.invitation.clone()),
            control::cooperative_payload_hash("accept_required", v)?,
            None,
        ),
        PermitMutation::SendMessage(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Control(v.thread.clone()),
            schema::canonical_digest(&messages::send_payload(v))?,
            None,
        ),
        PermitMutation::Ack(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::CheckIn(v.claim.seat.clone()),
            schema::canonical_digest(&receipts::ack_payload(v))?,
            None,
        ),
        PermitMutation::Leave(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Control(v.thread.clone()),
            control::cooperative_payload_hash("leave", v)?,
            None,
        ),
        PermitMutation::SetTopic(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Control(v.thread.clone()),
            control::cooperative_payload_hash("set_topic", v)?,
            None,
        ),
        PermitMutation::Archive(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Control(v.thread.clone()),
            control::cooperative_payload_hash("archive", v)?,
            None,
        ),
        PermitMutation::Reopen(v) => (
            v.claim.clone(),
            v.operation.clone(),
            ObligationRef::Control(v.thread.clone()),
            control::cooperative_payload_hash("reopen", v)?,
            None,
        ),
    };
    if claim.instance.is_empty() {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative instance required",
        ));
    }
    Ok(crate::ports::CooperativePermitRequest {
        claim,
        operation,
        obligation,
        payload_hash,
        check_in_mode,
    })
}
