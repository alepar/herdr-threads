//! Component boundaries and field ownership.
//!
//! The store owns durable identities, membership, obligations, transaction decision UTC,
//! retirement cutovers, pagination positions, idempotency records and wake reservations.
//! The host owns native target observations, boot/epoch/generation and prompt submission.
//! Caller verification owns the interpretation of harness evidence and issues internal
//! payload-bound permits. The local socket owns kernel peer credentials. Notification
//! owns volatile monotonic wake guards; clients own only claims, operation keys and
//! requested page/output bounds. No client supplies a decision timestamp or actor.

use serde::Serialize;

use crate::protocol::ids::ServiceAuthorId;
use crate::protocol::{
    authority::{
        CallerClaim, DecisionFence, Harness, MAX_PERMIT_MILLIS, MutationPermit, ObligationRef,
        OperatorActor,
    },
    commands::{
        CheckIn, Command, ContinuityCheckIn, OperatorCommand, OperatorFreshSeat,
        OperatorOrphanInvite, OperatorRebind, OperatorReplace, OperatorRetire, PermitMutation,
        ResolveSeat, SendMessage,
    },
    ids::*,
    output::OutputSpec,
    pagination::{Page, PageRequest},
    results::{ApiError, BoundedError, CommandResult, RetirementStatus},
    service::{ServiceOperation, ServiceResult},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

/// Server-private connection identity. It has no wire decoder; the returned
/// registration author/boot/generation are diagnostic values, not this handle.
#[derive(Debug, PartialEq, Eq)]
pub struct ServiceConnectionAuthority {
    instance: String,
    boot: String,
    generation: u64,
    author: ServiceAuthorId,
}
impl ServiceConnectionAuthority {
    /// Called only by the registered transport after peer/instance validation.
    pub(crate) fn new(
        instance: String,
        boot: String,
        generation: u64,
        author: ServiceAuthorId,
    ) -> Self {
        Self {
            instance,
            boot,
            generation,
            author,
        }
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn boot(&self) -> &str {
        &self.boot
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn author(&self) -> &ServiceAuthorId {
        &self.author
    }
}

/// Only `ServiceDecisionTransaction::begin` creates this after SQLite has
/// acquired its IMMEDIATE transaction. It cannot be constructed by consumers.
pub struct ServiceWriteTransactionProof(());

pub trait ServiceDecisionGuard {
    fn author(&self) -> &ServiceAuthorId;
}

/// Shared by transport revocation and store mutation. Acquire the database
/// write lock first, then `decision_guard`; revocation takes this gate alone.
/// No host call or network I/O runs under the decision guard. The store may
/// commit one bounded preparation quantum while holding it, then releases it
/// before admitting another quantum or the final publication decision.
pub trait ServiceAuthorityGate: Send + Sync {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
    ) -> Result<ServiceConnectionAuthority, crate::protocol::results::ApiError>;
    fn decision_guard<'a>(
        &'a self,
        proof: &ServiceWriteTransactionProof,
        connection: &ServiceConnectionAuthority,
    ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, crate::protocol::results::ApiError>;
    /// Compare and revoke this exact generation. Late cleanup cannot evict a successor.
    fn revoke_exact(&self, connection: &ServiceConnectionAuthority) -> bool;
}

#[cfg(test)]
mod service_decision_contract_tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex, MutexGuard},
    };

    struct Gate {
        path: PathBuf,
        author: ServiceAuthorId,
        serial: Mutex<()>,
        released_before_guard_drop: Arc<Mutex<Vec<bool>>>,
    }
    struct Guard<'a> {
        path: &'a PathBuf,
        author: &'a ServiceAuthorId,
        _serial: MutexGuard<'a, ()>,
        observations: Arc<Mutex<Vec<bool>>>,
    }
    impl ServiceDecisionGuard for Guard<'_> {
        fn author(&self) -> &ServiceAuthorId {
            self.author
        }
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            let mut probe = Connection::open(self.path).unwrap();
            probe.busy_timeout(std::time::Duration::ZERO).unwrap();
            let available = probe
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .is_ok();
            self.observations.lock().unwrap().push(available);
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _proof: &ServiceWriteTransactionProof,
            _connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            let serial = self.serial.lock().unwrap();
            let mut probe = Connection::open(&self.path).unwrap();
            probe.busy_timeout(std::time::Duration::ZERO).unwrap();
            assert!(
                probe
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .is_err(),
                "SQLite write ownership must precede authority acquisition"
            );
            Ok(Box::new(Guard {
                path: &self.path,
                author: &self.author,
                _serial: serial,
                observations: self.released_before_guard_drop.clone(),
            }))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            let _serial = self.serial.lock().unwrap();
            true
        }
    }

    #[test]
    fn actual_sqlite_transaction_ends_before_authority_guard_on_every_exit() {
        let path = std::env::temp_dir().join(format!(
            "herdr-service-decision-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut writer = Connection::open(&path).unwrap();
        writer
            .execute_batch("CREATE TABLE decisions (value INTEGER NOT NULL)")
            .unwrap();
        let observations = Arc::new(Mutex::new(Vec::new()));
        let gate = Gate {
            path: path.clone(),
            author: ServiceAuthorId::new("graph"),
            serial: Mutex::new(()),
            released_before_guard_drop: observations.clone(),
        };
        let connection = gate
            .register("instance", "boot", ServiceAuthorId::new("graph"))
            .unwrap();

        let committed = ServiceDecisionTransaction::begin(&mut writer, &gate, &connection).unwrap();
        assert_eq!(committed.author(), &ServiceAuthorId::new("graph"));
        committed
            .transaction()
            .execute("INSERT INTO decisions VALUES (1)", [])
            .unwrap();
        committed.commit().unwrap();

        let rolled_back =
            ServiceDecisionTransaction::begin(&mut writer, &gate, &connection).unwrap();
        rolled_back
            .transaction()
            .execute("INSERT INTO decisions VALUES (2)", [])
            .unwrap();
        rolled_back.rollback().unwrap();

        let failed: Result<(), &'static str> = {
            let pending =
                ServiceDecisionTransaction::begin(&mut writer, &gate, &connection).unwrap();
            pending
                .transaction()
                .execute("INSERT INTO decisions VALUES (3)", [])
                .unwrap();
            Err("application error before decision")
        };
        assert_eq!(failed, Err("application error before decision"));
        assert_eq!(
            writer
                .query_row("SELECT COUNT(*) FROM decisions", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(*observations.lock().unwrap(), vec![true, true, true]);
        drop(writer);
        std::fs::remove_file(path).unwrap();
    }
}

/// A SQLite BEGIN failure occurs before the authority gate is touched.
#[derive(Debug)]
pub enum ServiceDecisionStartError {
    Database(rusqlite::Error),
    Authority(ApiError),
}

/// Owns both the actual SQLite write transaction and authority guard. SQLite
/// BEGIN IMMEDIATE runs first; `commit`, `rollback`, and the drop/error path
/// finish the transaction while the authority guard is still held.
pub struct ServiceDecisionTransaction<'db, 'gate> {
    transaction: Option<Transaction<'db>>,
    authority: Box<dyn ServiceDecisionGuard + 'gate>,
}
impl<'db, 'gate> ServiceDecisionTransaction<'db, 'gate> {
    pub fn begin(
        writer: &'db mut Connection,
        gate: &'gate dyn ServiceAuthorityGate,
        connection: &ServiceConnectionAuthority,
    ) -> Result<Self, ServiceDecisionStartError> {
        let transaction = writer
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(ServiceDecisionStartError::Database)?;
        let proof = ServiceWriteTransactionProof(());
        let authority = gate
            .decision_guard(&proof, connection)
            .map_err(ServiceDecisionStartError::Authority)?;
        Ok(Self {
            transaction: Some(transaction),
            authority,
        })
    }

    pub fn transaction(&self) -> &Transaction<'db> {
        self.transaction
            .as_ref()
            .expect("decision transaction is open")
    }

    pub fn author(&self) -> &ServiceAuthorId {
        self.authority.author()
    }

    pub fn commit(mut self) -> rusqlite::Result<()> {
        self.transaction
            .take()
            .expect("decision transaction is open")
            .commit()
    }

    pub fn rollback(mut self) -> rusqlite::Result<()> {
        self.transaction
            .take()
            .expect("decision transaction is open")
            .rollback()
    }
}
impl Drop for ServiceDecisionTransaction<'_, '_> {
    fn drop(&mut self) {
        if let Some(transaction) = self.transaction.take() {
            let _ = transaction.rollback();
        }
        // `authority` is dropped after this function returns.
    }
}

pub const RETIREMENT_MAX_UNITS: usize = 16;
pub const RETIREMENT_WORK_MILLIS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadContext {
    pub instance: String,
    pub output: OutputSpec,
    /// Trusted dispatch scope for operation-status lookup; never from wire.
    pub operation_scope: Option<OperationReadScope>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationReadScope {
    Seat(SeatId),
    Operator(OperatorActor),
    ServiceAllocation,
}
impl OperationReadScope {
    pub fn actor_scope(&self, instance: &str) -> String {
        match self {
            Self::Seat(seat) => format!("seat:{}", seat.as_str()),
            Self::Operator(actor) => actor.operation_scope(instance),
            Self::ServiceAllocation => format!("service-allocation:{instance}"),
        }
    }
}
impl ReadContext {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.instance.is_empty() || self.instance.len() > 128 {
            return Err("invalid read instance");
        }
        self.output.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterAvailableRequest {
    pub command: CheckIn,
    /// Trusted instance and selected-output bounds for the transaction-local offer.
    pub read: ReadContext,
    /// Set only when the local account explicitly overrides the agent-to-human
    /// binding guard (TRUST-POLICY A4); the daemon audits it.
    pub operator: Option<OperatorActor>,
}

/// Service input for a cooperative permit. The digest must cover the exact
/// command context and payload; `check_in_mode` is present only for CheckIn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooperativePermitRequest {
    pub claim: CallerClaim,
    pub operation: OperationId,
    pub obligation: ObligationRef,
    pub payload_hash: [u8; 32],
    pub check_in_mode: Option<crate::protocol::commands::CheckInMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuePhase {
    Invitations,
    Receipts,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuePhaseCursor {
    pub high_water_ordinal: u64,
    pub after_deadline: Option<UtcMillis>,
    pub after_ordinal: u64,
    /// Receipt phase only: the independently retained sparse materialized
    /// receipt index position. Invitation cursors leave this absent.
    pub receipt_sparse: Option<ReceiptSparseCursor>,
    /// Receipt phase only: every catch-up row with `extension_until` at or
    /// before this was rechecked by a completed walk (spec §8). Absent means
    /// the recheck starts from the beginning.
    pub extension_through: Option<UtcMillis>,
    /// Receipt phase only: keyset position inside the recheck walk in
    /// progress. Absent means the walk starts from the beginning.
    pub extension_after: Option<ExtensionLapseKey>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionLapseKey {
    pub until: UtcMillis,
    pub seat: SeatId,
    pub thread: ThreadId,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptSparseCursor {
    pub high_water_rowid: u64,
    pub after_deadline: Option<UtcMillis>,
    pub after_message: Option<MessageId>,
    pub after_seat: Option<SeatId>,
    pub next_sparse: bool,
}
impl ReceiptSparseCursor {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.after_message.is_some() != self.after_seat.is_some() {
            Err("incomplete sparse receipt cursor key")
        } else if self.after_message.is_some() != self.after_deadline.is_some() {
            Err("incomplete sparse receipt deadline key")
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueScanState {
    pub invitations: Option<DuePhaseCursor>,
    pub receipts: Option<DuePhaseCursor>,
    pub next_phase: DuePhase,
}
impl Default for DueScanState {
    fn default() -> Self {
        Self {
            invitations: None,
            receipts: None,
            next_phase: DuePhase::Invitations,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueScanRequest {
    pub state: DueScanState,
    pub max_candidates: u16,
    /// Driver-owned backoff can defer either scanner without losing its state.
    pub run_invitations: bool,
    pub run_receipts: bool,
}
impl DueScanRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.run_invitations && !self.run_receipts {
            Err("at least one due phase must be admitted")
        } else if self.max_candidates == 0 || self.max_candidates > 100 {
            Err("invalid due candidate bound")
        } else if self
            .state
            .invitations
            .as_ref()
            .is_some_and(|cursor| cursor.receipt_sparse.is_some())
        {
            Err("invitation cursor carries receipt state")
        } else if let Some(sparse) = self
            .state
            .receipts
            .as_ref()
            .and_then(|cursor| cursor.receipt_sparse.as_ref())
        {
            sparse.validate()
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuePhaseProgress {
    Complete,
    More,
    /// The phase's error code (Health-safe) and its bounded private detail
    /// (diagnostics only; never rendered into Health).
    Failed(crate::protocol::results::ErrorCode, BoundedError),
    Skipped,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueScanProgress {
    pub state: DueScanState,
    pub examined_candidates: u16,
    pub warnings_added: u16,
    pub invitations: DuePhaseProgress,
    pub receipts: DuePhaseProgress,
    pub has_more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WorkKind {
    WarningAttribution,
    SendAttention,
    ReceiptTimerMaterialization,
    PreparationCleanup,
}
/// One bounded hidden send-preparation step. `Ready` is not a publish permit;
/// the service must obtain fresh caller proof for the final deciding mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
// Allowed: a transient per-step result; boxing CommandResult would change the port API.
#[allow(clippy::large_enum_variant)]
pub enum SendPreparationProgress {
    Committed(CommandResult),
    More { visited: u8, preparation_id: String },
    Ready { visited: u8, preparation_id: String },
}
impl SendPreparationProgress {
    pub fn visited(&self) -> u8 {
        match self {
            Self::Committed(_) => 0,
            Self::More { visited, .. } | Self::Ready { visited, .. } => *visited,
        }
    }
    /// Captured hidden preparation generation for request-exit abandonment.
    /// Re-looking up by operation key could discard a successor rebuild.
    pub fn preparation_id(&self) -> Option<&str> {
        match self {
            Self::Committed(_) => None,
            Self::More { preparation_id, .. } | Self::Ready { preparation_id, .. } => {
                Some(preparation_id)
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkCandidate {
    pub id: String,
    pub kind: WorkKind,
    pub position: u64,
    pub high_water: u64,
    pub has_more: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkProgress {
    pub completed_units: u64,
    /// Units committed by this call. Committed progress without a retained
    /// error clears the job's failure state.
    pub processed_this_turn: u8,
    pub has_more: bool,
    pub next_position: u64,
    pub last_error: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableWorkAdmission {
    pub max_units: u8,
}
impl DurableWorkAdmission {
    pub fn new(max_units: u8) -> Result<Self, &'static str> {
        if max_units == 0 || max_units > 16 {
            Err("invalid work admission")
        } else {
            Ok(Self { max_units })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkAdmission {
    Foreground,
    Background,
}

/// The retirement cutover is loaded from storage for each cleanup transaction.
/// Wire requests cannot choose this time basis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeBasis {
    Decision,
    Retirement(RetirementJobId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementJob {
    pub id: RetirementJobId,
    pub seat: SeatId,
    pub retired_at: UtcMillis,
    pub phase: RetirementPhase,
    pub after_ordinal: Option<u64>,
    pub high_water_ordinal: u64,
    pub processed_units: u64,
}

/// Result of the writer's startup binding-evidence verification: bindings
/// whose terminal/incarnation evidence was backfilled from the seat's own
/// matching structural proof, and latest bindings of non-retired seats that
/// still lack it (fail-closed: after the next host invalidation such a seat
/// needs operator repair).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindingEvidenceStartup {
    pub backfilled: u64,
    pub still_lacking: u64,
}

/// Seats in `unresolved` state for Health and doctor (daemon design: Health
/// reports unresolved identity counts). Counted by the
/// `seats_instance_state_ordinal` index, so the work is proportional to the
/// unresolved seats only, never to history; `sample` names at most
/// [`UNRESOLVED_SEAT_SAMPLE`] of them, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnresolvedSeatSummary {
    pub count: u64,
    pub sample: Vec<UnresolvedSeatSample>,
}
pub const UNRESOLVED_SEAT_SAMPLE: usize = 3;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedSeatSample {
    pub seat: SeatId,
    pub target: Option<HostTargetId>,
    pub reason: Option<UnresolvedReason>,
}

/// Indexed scalar observation for Health; exact errors remain on seat inspect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetirementSummary {
    pub pending: bool,
    pub degraded: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementPhase {
    Warnings,
    Recipients,
    ThreadAudits,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementProgress {
    pub job: RetirementJobId,
    pub processed_this_turn: u8,
    pub processed_total: u64,
    pub complete: bool,
    pub warning_history_complete: bool,
    pub last_error: Option<BoundedError>,
}

/// Closure proof comes from a coherent, unchanged host-incarnation enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureEvidence {
    pub host_boot: HostBootId,
    pub epoch: u64,
    pub target: HostTargetId,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostUiState {
    Idle,
    ActiveTurn,
    ApprovalOrQuestion,
    HumanInput,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOccupant {
    pub harness: Harness,
    pub session: NativeSessionId,
    pub execution: ExecutionId,
    pub is_top_level: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationProvenance {
    FreshCurrentTarget,
    CoherentEnumeration,
    UncharacterizedCache,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralOccupancy {
    EmptyShell,
    Occupied,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    NativeCurrentTarget,
    NativeInvocation,
    CoherentEnumeration,
}
impl EvidenceKind {
    pub fn structural_storage_spelling(self) -> Option<&'static str> {
        match self {
            Self::NativeCurrentTarget => Some("native_current_target"),
            Self::CoherentEnumeration => Some("coherent_enumeration"),
            Self::NativeInvocation => None,
        }
    }
    pub(crate) fn from_structural_storage(value: &str) -> Option<Self> {
        match value {
            "native_current_target" => Some(Self::NativeCurrentTarget),
            "coherent_enumeration" => Some(Self::CoherentEnumeration),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncarnationEvidence {
    Unknown,
    Verified {
        identity: String,
        evidence_kind: EvidenceKind,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionEvidence {
    Unknown,
    Verified {
        execution: ExecutionId,
        evidence_kind: EvidenceKind,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostObservation {
    pub target: HostTargetId,
    pub host_boot: HostBootId,
    pub epoch: u64,
    pub generation: u64,
    pub observed_at_utc: UtcMillis,
    pub observed_at_mono: MonoInstant,
    pub provenance: ObservationProvenance,
    pub occupant: Option<NativeOccupant>,
    pub ui: HostUiState,
    /// Herdr's pane focus at observation time; spec §10 skips pokes into a
    /// focused pane.
    pub focused: bool,
    pub terminal: Option<TerminalId>,
    pub occupancy: StructuralOccupancy,
    pub incarnation: IncarnationEvidence,
    pub execution: ExecutionEvidence,
    pub call_id: HostCallId,
    pub connection_epoch: u64,
    pub observation_sequence: u64,
    pub started_at_mono: MonoInstant,
    pub completed_at_mono: MonoInstant,
}
/// Durable structural continuity from one fresh, explicit host target read.
/// It says nothing about the current native execution or receipt registration.
/// HostPort must bind CoherentEnumeration evidence to this same target, boot,
/// epoch, connection, terminal and ordered observation; copying a cached
/// enumeration identity into a fresh-looking row violates the producer contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableStructuralProof {
    target: HostTargetId,
    terminal: TerminalId,
    incarnation: String,
    source: EvidenceKind,
    host_boot: HostBootId,
    host_epoch: u64,
    connection_epoch: u64,
    observation_sequence: u64,
    target_generation: u64,
}
impl DurableStructuralProof {
    // Allowed: validating constructor: one argument per persisted column.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_persisted(
        target: HostTargetId,
        terminal: TerminalId,
        incarnation: String,
        source: EvidenceKind,
        host_boot: HostBootId,
        host_epoch: u64,
        connection_epoch: u64,
        observation_sequence: u64,
        target_generation: u64,
    ) -> Result<Self, &'static str> {
        if incarnation.is_empty()
            || incarnation.len() > 128
            || !incarnation.bytes().all(|b| b.is_ascii_graphic())
            || target.as_str().is_empty()
            || terminal.as_str().is_empty()
            || host_boot.as_str().is_empty()
            || matches!(source, EvidenceKind::NativeInvocation)
            || observation_sequence == 0
            || host_epoch > i64::MAX as u64
            || connection_epoch > i64::MAX as u64
            || observation_sequence > i64::MAX as u64
            || target_generation > i64::MAX as u64
        {
            return Err("invalid durable structural proof");
        }
        Ok(Self {
            target,
            terminal,
            incarnation,
            source,
            host_boot,
            host_epoch,
            connection_epoch,
            observation_sequence,
            target_generation,
        })
    }
    pub fn target(&self) -> &HostTargetId {
        &self.target
    }
    pub fn terminal(&self) -> &TerminalId {
        &self.terminal
    }
    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }
    pub fn source(&self) -> EvidenceKind {
        self.source
    }
    pub fn source_spelling(&self) -> &'static str {
        self.source
            .structural_storage_spelling()
            .expect("qualified structural proof")
    }
    pub fn host_boot(&self) -> &HostBootId {
        &self.host_boot
    }
    pub fn host_epoch(&self) -> u64 {
        self.host_epoch
    }
    pub fn connection_epoch(&self) -> u64 {
        self.connection_epoch
    }
    pub fn observation_sequence(&self) -> u64 {
        self.observation_sequence
    }
    pub fn target_generation(&self) -> u64 {
        self.target_generation
    }
}
impl HostObservation {
    pub fn is_fresh_structure(&self) -> bool {
        self.provenance == ObservationProvenance::FreshCurrentTarget
            && self.completed_at_mono >= self.started_at_mono
    }
    pub fn has_verified_execution(&self) -> bool {
        self.is_fresh_structure()
            && matches!(&self.incarnation, IncarnationEvidence::Verified { identity, .. }
                if !identity.is_empty() && identity.len() <= 128 && identity.bytes().all(|b| b.is_ascii_graphic()))
            && matches!(self.execution, ExecutionEvidence::Verified { .. })
    }
    pub fn verified_structural_proof(&self) -> Option<DurableStructuralProof> {
        if !self.is_fresh_structure()
            || self.call_id.as_str().is_empty()
            || self.observed_at_mono < self.started_at_mono
            || self.observed_at_mono > self.completed_at_mono
        {
            return None;
        }
        let terminal = self.terminal.clone()?;
        let IncarnationEvidence::Verified {
            identity,
            evidence_kind,
        } = &self.incarnation
        else {
            return None;
        };
        DurableStructuralProof::from_persisted(
            self.target.clone(),
            terminal,
            identity.clone(),
            *evidence_kind,
            self.host_boot.clone(),
            self.epoch,
            self.connection_epoch,
            self.observation_sequence,
            self.generation,
        )
        .ok()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumerationEvidence {
    Partial,
    CompleteUnverified,
    CoherentVerified,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSnapshot {
    pub boot: HostBootId,
    pub epoch: u64,
    /// Trusted coherent enumeration event sequence, including empty snapshots.
    /// Comparable only within this boot and epoch.
    pub observation_sequence: u64,
    pub complete: bool,
    pub enumeration: EnumerationEvidence,
    pub incarnation: IncarnationEvidence,
    pub targets: Vec<HostObservation>,
}
impl HostSnapshot {
    pub fn has_coherent_order(&self) -> bool {
        self.observation_sequence > 0
            && self.observation_sequence <= i64::MAX as u64
            && self.targets.iter().all(|target| {
                target.host_boot == self.boot
                    && target.epoch == self.epoch
                    && target.observation_sequence > 0
                    && target.observation_sequence <= self.observation_sequence
            })
    }
    pub fn authorizes_absence_closure(&self) -> bool {
        self.complete
            && self.has_coherent_order()
            && self.enumeration == EnumerationEvidence::CoherentVerified
            && matches!(&self.incarnation, IncarnationEvidence::Verified { identity, evidence_kind: EvidenceKind::CoherentEnumeration }
                if !identity.is_empty() && identity.len() <= 128 && identity.bytes().all(|b| b.is_ascii_graphic()))
    }
}

/// A verified complete capture, with its target vector deliberately omitted.
/// The store issues the durable stage ID and captures publication fences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostObservationAdmission {
    pub(crate) instance: String,
    pub(crate) sequence: u64,
    pub(crate) expected_active: Option<SnapshotGenerationId>,
    pub(crate) expected_boot: Option<HostBootId>,
    pub(crate) expected_epoch: u64,
    pub(crate) lifecycle_revision: u64,
    pub(crate) invalidation_revision: u64,
}
impl HostObservationAdmission {
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostInvalidationReason {
    PartialEnumeration,
    UnknownIncarnation,
    HostUnavailable,
    CoherenceLost,
    /// A coherent capture could not be durably staged or published. The
    /// admission fence still prevents an older failed apply from invalidating
    /// a newer successful publication.
    PublicationFailed,
}
impl HostInvalidationReason {
    /// Herdr did not answer (timeout, unreachable) or the daemon could not
    /// stage what it captured: missing evidence, not evidence of change. Such
    /// a failure writes no invalidation, so seats, bindings and the published
    /// view stay frozen until Herdr answers (TRUST-POLICY C4).
    pub fn is_unavailability(self) -> bool {
        matches!(self, Self::HostUnavailable | Self::PublicationFailed)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostInvalidationFence {
    pub(crate) admission: HostObservationAdmission,
    pub(crate) invalidation_revision: u64,
}
impl HostInvalidationFence {
    pub fn instance(&self) -> &str {
        self.admission.instance()
    }
    pub fn admission_sequence(&self) -> u64 {
        self.admission.sequence()
    }
    pub fn invalidation_revision(&self) -> u64 {
        self.invalidation_revision
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedInvalidationTransition {
    pub fence: HostInvalidationFence,
    pub seat: SeatId,
    pub expected_binding_generation: u64,
    pub expected_target: Option<HostTargetId>,
    pub expected_terminal: Option<TerminalId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub admission: HostObservationAdmission,
    pub instance: String,
    pub boot: HostBootId,
    pub epoch: u64,
    pub observation_sequence: u64,
    pub incarnation: String,
    pub expected_targets: u64,
}
impl SnapshotHeader {
    pub fn from_captured(
        admission: HostObservationAdmission,
        snapshot: &HostSnapshot,
    ) -> Result<Self, &'static str> {
        if admission.instance.is_empty()
            || admission.instance.len() > 128
            || admission.sequence == 0
            || admission.sequence > i64::MAX as u64
            || !snapshot.authorizes_absence_closure()
        {
            return Err("snapshot lacks complete coherent authority");
        }
        if snapshot.epoch > i64::MAX as u64 {
            return Err("snapshot epoch exceeds durable range");
        }
        let expected_targets = u64::try_from(snapshot.targets.len())
            .map_err(|_| "snapshot target count exceeds durable range")?;
        if expected_targets > i64::MAX as u64 {
            return Err("snapshot target count exceeds durable range");
        }
        let IncarnationEvidence::Verified { identity, .. } = &snapshot.incarnation else {
            return Err("snapshot incarnation is unverified");
        };
        Ok(Self {
            instance: admission.instance.clone(),
            admission,
            boot: snapshot.boot.clone(),
            epoch: snapshot.epoch,
            observation_sequence: snapshot.observation_sequence,
            incarnation: identity.clone(),
            expected_targets,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotGenerationId(String);
impl SnapshotGenerationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub(crate) fn store_issued(id: String) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotStage {
    pub id: SnapshotGenerationId,
    pub expected_targets: u64,
    pub staged_targets: u64,
    pub sealed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotStageProgress {
    pub stage: SnapshotStage,
    /// Counted physical target rows visited in this writer turn, at most 16.
    pub visited: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotCleanupProgress {
    pub stage: SnapshotGenerationId,
    pub visited: u8,
    pub complete: bool,
}
/// What one retention pass deleted (`store::retention::prune_once`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneProgress {
    /// Snapshot generations deleted.
    pub generations: u32,
    /// Snapshot target rows deleted.
    pub targets: u32,
    /// Completed work jobs deleted.
    pub jobs: u32,
    /// A transaction stopped on its row or time budget with work left.
    pub has_more: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedSnapshot {
    pub id: SnapshotGenerationId,
    pub instance: String,
    pub boot: HostBootId,
    pub epoch: u64,
    pub observation_sequence: u64,
    pub incarnation: String,
    pub target_count: u64,
    /// Changes when a host lifecycle event invalidates this observation.
    pub invalidation_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotTargetMatch {
    pub target: HostTargetId,
    pub terminal: Option<TerminalId>,
    pub structural_generation: u64,
    pub observation_sequence: u64,
    /// Present only when the staged target itself carried qualified fresh
    /// incarnation evidence. The generation identity alone cannot invent it.
    pub connection_epoch: Option<u64>,
    pub incarnation_source: Option<EvidenceKind>,
    pub occupancy: StructuralOccupancy,
    pub verified_execution: Option<ExecutionId>,
    pub top_level_occupant: bool,
}
impl SnapshotTargetMatch {
    /// Positive structural evidence that the target no longer holds a
    /// top-level occupant: an observed empty shell, or an observed occupant
    /// that is not top-level. `Unknown` occupancy (what the production Herdr
    /// adapter reports) and an occupied top-level target without execution
    /// proof are absence of evidence, never evidence of absence: errors and
    /// unknown views cannot retire or unseat an occupant (seat-identity
    /// design; root adoption wave2-fix1 (b)).
    pub fn shows_occupant_absent(&self) -> bool {
        match self.occupancy {
            StructuralOccupancy::EmptyShell => true,
            StructuralOccupancy::Occupied => !self.top_level_occupant,
            StructuralOccupancy::Unknown => false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSavedSeat {
    pub ordinal: u64,
    pub seat: SeatId,
    pub state: SeatState,
    /// Durable cause of unresolved state. None is an older/unknown cause and
    /// never authorizes automatic reconfirmation.
    pub unresolved_reason: Option<UnresolvedReason>,
    /// Frozen pre-invalidation evidence, looked up by the retained published
    /// generation saved on this seat. None denies prelaunch reconfirmation.
    pub prior_published_observation: Option<PriorPublishedTarget>,
    /// Allocation/repair structural identity retained even with no occupant.
    pub structural_proof: Option<DurableStructuralProof>,
    pub target: Option<HostTargetId>,
    pub terminal: Option<TerminalId>,
    pub binding_generation: u64,
    pub binding_execution: Option<ExecutionId>,
    /// Current registered binding only. `binding_execution` above may be
    /// historical evidence for an unresolved seat after invalidation.
    pub active_binding_execution: Option<ExecutionId>,
    /// Host epoch of the open registered binding, only when that binding
    /// carries the same host boot and incarnation as the seat's structural
    /// proof. A lagging epoch is what C4 carries forward.
    pub bound_epoch: Option<u64>,
    pub bound_boot: Option<HostBootId>,
    pub bound_incarnation: Option<String>,
    /// Reconfirmation evidence stored on the seat's latest occupant binding,
    /// live or ended: present only when that row carries both its terminal
    /// and its verified host incarnation. Unlike `bound_incarnation` it never
    /// falls back to structural or published provenance.
    pub latest_binding_evidence: Option<BindingEvidence>,
    /// Indexed lookup by terminal, with target lookup for no-terminal cases.
    pub observed_match: Option<SnapshotTargetMatch>,
}
/// The terminal and verified host incarnation recorded on an occupant binding
/// by the observation that proved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingEvidence {
    pub terminal: TerminalId,
    pub host_boot: HostBootId,
    pub incarnation: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorPublishedTarget {
    pub generation_id: SnapshotGenerationId,
    pub host_boot: HostBootId,
    pub host_epoch: u64,
    pub incarnation: String,
    pub prior_binding_generation: u64,
    pub target: SnapshotTargetMatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatState {
    Resolved,
    Unresolved,
    Retired,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedReason {
    HostInvalidation,
    Other,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSeatPage {
    pub publication: PublishedSnapshot,
    pub high_water_ordinal: u64,
    pub after_ordinal: u64,
    pub seats: Vec<SnapshotSavedSeat>,
    pub visited: u8,
    pub has_more: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidationSeatPage {
    pub fence: HostInvalidationFence,
    pub high_water_ordinal: u64,
    pub after_ordinal: u64,
    /// Indexed saved-seat scalars only; observed_match is always None because
    /// denied/partial captures cannot authorize target absence or continuity.
    pub seats: Vec<SnapshotSavedSeat>,
    pub visited: u8,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedSeatTransition {
    pub publication: PublishedSnapshot,
    pub seat: SeatId,
    pub expected_binding_generation: u64,
    pub expected_target: Option<HostTargetId>,
    pub expected_terminal: Option<TerminalId>,
    pub action: ReconciliationAction,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationAction {
    /// Structural reconfirmation of a host-invalidated seat (seat-identity
    /// rule 1: an exact live terminal match in verified unchanged host
    /// context retains the seat). The new coherent publication shows the same
    /// terminal ID in the same verified host boot and incarnation as the
    /// evidence stored on the seat's latest binding or, for a seat that was
    /// resolved but never had any binding, as the seat's own verified
    /// structural proof. No occupancy or
    /// execution evidence is required: Herdr 0.9.1 reports both as Unknown.
    /// It restores only the seat, never a revoked registration (an open
    /// cooperative binding is carried forward per C4, and the instance-wide
    /// baseline hold lifts once no unresolved seat remains, TRUST-POLICY
    /// C1-C3); otherwise the pane's agent registers by its next lifecycle
    /// check-in at the new generation.
    ReconfirmStructure {
        target: HostTargetId,
        terminal: TerminalId,
    },
    /// C4: a resolved seat structurally reconfirmed in a newer host epoch of
    /// the same boot and incarnation keeps its open binding; the binding's
    /// host epoch and target generation move forward in place; nothing else
    /// changes.
    CarryForward {
        target: HostTargetId,
        terminal: TerminalId,
    },
    Move {
        target: HostTargetId,
        terminal: TerminalId,
    },
    MarkUnresolved,
    BeginRetirement {
        absent_target: HostTargetId,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationOutcome {
    Applied,
    Unchanged,
    Stale,
    RetirementStarted(RetirementJob),
}

/// The service's recovery baseline classifies a target independently of caller claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDisposition {
    UnambiguousUnclaimed,
    CreatedAfterBaseline,
    HeldForRepair,
    AlreadyOwned,
    Unknown,
}
/// One explicit ordinary target read and its store-issued observation admission.
/// This guard carries structural continuity only, with no native caller authority.
#[derive(Debug)]
pub struct OrdinaryResolutionGuard {
    operation: OperationId,
    call_id: HostCallId,
    proof: DurableStructuralProof,
    admission: HostObservationAdmission,
}
impl OrdinaryResolutionGuard {
    pub(crate) fn try_new(
        request: &ResolveSeat,
        observation: HostObservation,
        admission: &HostObservationAdmission,
    ) -> Result<Self, &'static str> {
        if request.target != observation.target
            || request.operation.as_str().is_empty()
            || observation.provenance != ObservationProvenance::FreshCurrentTarget
            || admission.sequence() == 0
        {
            return Err("resolution needs a matching explicit current-target read");
        }
        let proof = observation
            .verified_structural_proof()
            .ok_or("resolution lacks qualified structural proof")?;
        Ok(Self {
            operation: request.operation.clone(),
            call_id: observation.call_id,
            proof,
            admission: admission.clone(),
        })
    }
    pub fn call_id(&self) -> &HostCallId {
        &self.call_id
    }
    pub fn structural_proof(&self) -> &DurableStructuralProof {
        &self.proof
    }
    pub fn admission(&self) -> &HostObservationAdmission {
        &self.admission
    }
    pub fn operation(&self) -> &OperationId {
        &self.operation
    }
}
#[derive(Debug)]
// Allowed: a transient per-resolution value; boxing the guard would change the port API.
#[allow(clippy::large_enum_variant)]
pub enum OrdinaryResolutionAttempt {
    /// Exact historical result only; no observation, allocation or currentness.
    ReplayOnly,
    Observed(OrdinaryResolutionGuard),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrdinaryResolutionOutcome {
    NeedsObservation,
    /// Historical on replay. Managed launch still requires a fresh final check.
    Resolved(SeatId),
}
/// A check-only decision from the second immediate prelaunch observation.
#[derive(Debug)]
pub struct ResolvedTargetCheck {
    pub expected_seat: SeatId,
    pub guard: OrdinaryResolutionGuard,
}

/// Per-request host evidence for explicit operator repair. This grants no
/// native caller, receipt, or checkpoint authority.
#[derive(Debug)]
pub struct OperatorTargetGuard {
    instance: String,
    target: HostTargetId,
    command: OperatorCommand,
    host_boot: HostBootId,
    epoch: u64,
    generation: u64,
    observed_at: MonoInstant,
    structural_proof: Option<DurableStructuralProof>,
    consumed: bool,
}
impl OperatorTargetGuard {
    pub(crate) fn try_new(
        instance: &str,
        command: &OperatorCommand,
        observation: HostObservation,
    ) -> Result<Self, &'static str> {
        let target = match command {
            OperatorCommand::Rebind(command) => &command.target,
            OperatorCommand::FreshSeat(command) => &command.target,
            OperatorCommand::Replace(command) => &command.target,
            OperatorCommand::OrphanInvite(_) => return Err("orphan invite has no target guard"),
            OperatorCommand::Retire(_) => return Err("retire has no target guard"),
        };
        if instance.is_empty() || observation.target != *target {
            return Err("operator target does not match fresh observation");
        }
        if observation.provenance != ObservationProvenance::FreshCurrentTarget {
            return Err("operator target lacks a fresh current observation");
        }
        Ok(Self {
            structural_proof: observation.verified_structural_proof(),
            instance: instance.into(),
            target: observation.target,
            command: command.clone(),
            host_boot: observation.host_boot,
            epoch: observation.epoch,
            generation: observation.generation,
            observed_at: observation.observed_at_mono,
            consumed: false,
        })
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn target(&self) -> &HostTargetId {
        &self.target
    }
    pub fn host_boot(&self) -> &HostBootId {
        &self.host_boot
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn structural_proof(&self) -> Option<&DurableStructuralProof> {
        self.structural_proof.as_ref()
    }
    fn consume(
        &mut self,
        instance: &str,
        command: &OperatorCommand,
        fence: &DecisionFence,
    ) -> Result<(), &'static str> {
        if self.consumed {
            return Err("operator target guard already consumed");
        }
        if instance != self.instance || command != &self.command {
            return Err("operator request does not match target guard");
        }
        if fence.known_invalidated
            || fence.host_boot != self.host_boot
            || fence.host_epoch != self.epoch
            || fence.target_generation != self.generation
        {
            return Err("operator target changed");
        }
        if fence.now.0 < self.observed_at.0 || fence.now.0 - self.observed_at.0 > MAX_PERMIT_MILLIS
        {
            return Err("operator target observation expired");
        }
        self.consumed = true;
        Ok(())
    }
}

#[derive(Debug)]
pub enum OperatorRequest {
    Rebind(OperatorRebind, OperatorTargetGuard),
    FreshSeat(OperatorFreshSeat, OperatorTargetGuard),
    OrphanInvite(OperatorOrphanInvite),
    Retire(OperatorRetire),
    Replace(OperatorReplace, OperatorTargetGuard),
}
impl OperatorRequest {
    /// Consume target evidence against the command being decided and the store's
    /// instance identity. The orphan-invite command has no target evidence.
    pub fn consume_for_decision(
        &mut self,
        instance: &str,
        fence: &DecisionFence,
    ) -> Result<(), &'static str> {
        match self {
            Self::Rebind(command, guard) => {
                guard.consume(instance, &OperatorCommand::Rebind(command.clone()), fence)
            }
            Self::FreshSeat(command, guard) => guard.consume(
                instance,
                &OperatorCommand::FreshSeat(command.clone()),
                fence,
            ),
            Self::Replace(command, guard) => {
                guard.consume(instance, &OperatorCommand::Replace(command.clone()), fence)
            }
            Self::OrphanInvite(_) | Self::Retire(_) => Ok(()),
        }
    }
}

/// Per-request host evidence for a cooperative continuity reattachment
/// (TRUST-POLICY C1). Like [`OperatorTargetGuard`] it proves the target's
/// current structural observation only; it grants no caller authority and the
/// Herdr `agent_session` diagnostic is never part of it.
#[derive(Debug)]
pub struct ContinuityTargetGuard {
    instance: String,
    target: HostTargetId,
    command: ContinuityCheckIn,
    host_boot: HostBootId,
    epoch: u64,
    generation: u64,
    observed_at: MonoInstant,
    structural_proof: Option<DurableStructuralProof>,
    consumed: bool,
}
impl ContinuityTargetGuard {
    pub(crate) fn try_new(
        instance: &str,
        command: &ContinuityCheckIn,
        observation: HostObservation,
    ) -> Result<Self, &'static str> {
        if instance.is_empty() || observation.target != command.target {
            return Err("continuity target does not match fresh observation");
        }
        if observation.provenance != ObservationProvenance::FreshCurrentTarget {
            return Err("continuity target lacks a fresh current observation");
        }
        Ok(Self {
            structural_proof: observation.verified_structural_proof(),
            instance: instance.into(),
            target: observation.target,
            command: command.clone(),
            host_boot: observation.host_boot,
            epoch: observation.epoch,
            generation: observation.generation,
            observed_at: observation.observed_at_mono,
            consumed: false,
        })
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn target(&self) -> &HostTargetId {
        &self.target
    }
    pub fn host_boot(&self) -> &HostBootId {
        &self.host_boot
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn structural_proof(&self) -> Option<&DurableStructuralProof> {
        self.structural_proof.as_ref()
    }
    pub fn consume(
        &mut self,
        instance: &str,
        command: &ContinuityCheckIn,
        fence: &DecisionFence,
    ) -> Result<(), &'static str> {
        if self.consumed {
            return Err("continuity target guard already consumed");
        }
        if instance != self.instance || command != &self.command {
            return Err("continuity request does not match target guard");
        }
        if fence.known_invalidated
            || fence.host_boot != self.host_boot
            || fence.host_epoch != self.epoch
            || fence.target_generation != self.generation
        {
            return Err("continuity target changed");
        }
        if fence.now.0 < self.observed_at.0 || fence.now.0 - self.observed_at.0 > MAX_PERMIT_MILLIS
        {
            return Err("continuity target observation expired");
        }
        self.consumed = true;
        Ok(())
    }
}

/// The decision input of [`StorePort::decide_continuity`]. `diagnostic` is
/// one of `match`, `mismatch`, `absent`, `read_error` and is only recorded.
#[derive(Debug)]
pub struct ContinuityRequest {
    pub command: ContinuityCheckIn,
    pub guard: ContinuityTargetGuard,
    pub diagnostic: &'static str,
}

/// Herdr's per-pane agent record for exactly one pane. TRUST-POLICY C1/A4:
/// Herdr's agent field may only suggest; it never moves a seat, allocates one
/// or ends a binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneAgentObservation {
    /// Herdr's detection-based agent kind (for example `claude`, `codex`).
    pub kind: Option<String>,
    /// Herdr integration report; diagnostic value only.
    pub agent_session: Option<String>,
}

/// A recognized idle native occupant may receive a recovery hint before check-in.
/// This is separate from a cooperative check-in, which starts receipt availability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeWakeTarget {
    pub seat: SeatId,
    pub target: HostTargetId,
    pub host_boot: HostBootId,
    pub generation: u64,
    pub terminal: TerminalId,
    pub incarnation: String,
    pub basis: WakeTargetBasis,
    pub epoch: u64,
    pub observation_sequence: u64,
    /// The seat's open binding harness (`claude` or `codex`); the host
    /// adapter refuses a wake unless Herdr's detected agent kind equals it
    /// (TRUST-POLICY A4). The host cannot know it, so adapters set `None` and
    /// the notification dispatcher fills it from the reservation. `None`
    /// means the seat has no open binding: the recognized-kind rule applies.
    pub bound_harness: Option<String>,
}

/// What identifies the occupant a wake prompt may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeTargetBasis {
    /// A verified current native session/execution from the fresh read.
    VerifiedOccupant {
        session: NativeSessionId,
        execution: ExecutionId,
    },
    /// Cooperative native policy: the adapter cannot verify the current
    /// execution. The target is the same terminal in the same verified host
    /// incarnation, and the adapter itself rechecks immediately before
    /// submission that the host reports a recognized, top-level harness agent
    /// in that terminal that is idle (or finished its turn) and not blocked.
    /// A prompt is a generic hint only: it never registers, receives or
    /// settles anything.
    CooperativeAgent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    Submitted,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredHook {
    pub scope: String,
    pub path: String,
    pub fingerprint: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLaunchRequest {
    pub seat: SeatId,
    pub target: HostTargetId,
    pub harness: Harness,
    /// Native agent arguments passed verbatim after Herdr's `agent start ... --`.
    /// The host adapter supplies the Herdr executable, kind, and explicit target.
    pub argv: Vec<String>,
    pub configured_hook: ConfiguredHook,
    pub expected_terminal: TerminalId,
    pub expected_generation: u64,
    pub expected_incarnation: String,
    /// Readable name wanted for the Herdr agent: `launch --name`, else the
    /// target pane's Herdr label. Raw; [`Self::agent_name`] sanitizes it and
    /// falls back to the short seat id.
    pub name_hint: Option<String>,
}

/// Longest Herdr agent name: names match `[a-z][a-z0-9_-]{0,31}`.
pub const HERDR_AGENT_NAME_MAX: usize = 32;

/// `raw` mapped onto Herdr's agent-name rule `[a-z][a-z0-9_-]{0,31}`:
/// ASCII-lowercased, every other character a single `-`, leading characters
/// before the first letter dropped, cut to 32 bytes, with no trailing `-`/`_`.
/// `None` when nothing usable is left.
pub fn sanitize_agent_name(raw: &str) -> Option<String> {
    let mut name = String::new();
    for c in raw.chars() {
        let c = c.to_ascii_lowercase();
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if (name.is_empty() && !c.is_ascii_lowercase()) || (c == '-' && name.ends_with('-')) {
            continue;
        }
        name.push(c);
        if name.len() == HERDR_AGENT_NAME_MAX {
            break;
        }
    }
    let name = name.trim_end_matches(['-', '_']);
    (!name.is_empty()).then(|| name.to_owned())
}

/// Short readable seat id: the seat's alphanumeric characters after its
/// `seat-` prefix, lowercased, at most 8 (`seat-k3Fq9a2B` -> `k3fq9a2b`).
pub fn short_seat(seat: &SeatId) -> String {
    let seat_str = seat.as_str();
    let rest = seat_str
        .strip_prefix("seat-")
        .or_else(|| seat_str.strip_prefix("seat_"))
        .unwrap_or(seat_str);
    let short: String = rest
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .take(8)
        .collect();
    if short.is_empty() {
        seat_digest(seat)
    } else {
        short
    }
}

fn seat_digest(seat: &SeatId) -> String {
    let digest = Sha256::digest(seat.as_str().as_bytes());
    digest.iter().take(3).map(|b| format!("{b:02x}")).collect()
}

/// The Herdr agent names one launch of `seat` may submit, in order: the
/// sanitized `hint` (`launch --name`, else the pane label), else
/// `seat-<short seat id>`; then, used once only after Herdr refuses the first
/// as `agent_name_taken`, that name cut to fit plus `-<short seat id>` (a
/// seat digest when the name already ends with that id).
pub fn launch_agent_names(seat: &SeatId, hint: Option<&str>) -> [String; 2] {
    let short = short_seat(seat);
    let first = hint
        .and_then(sanitize_agent_name)
        .unwrap_or_else(|| format!("seat-{short}"));
    let suffix = if first.ends_with(&short) {
        seat_digest(seat)
    } else {
        short
    };
    let keep = HERDR_AGENT_NAME_MAX - 1 - suffix.len();
    let base = first[..first.len().min(keep)].trim_end_matches(['-', '_']);
    let retry = format!("{base}-{suffix}");
    [first, retry]
}

impl NativeLaunchRequest {
    /// The Herdr agent name launch asks for first. Launch correlates
    /// Herdr's responses with the exact name it submitted.
    pub fn agent_name(&self) -> String {
        let [first, _] = self.agent_name_candidates();
        first
    }

    /// Every name one launch may submit, in order (see
    /// [`launch_agent_names`]); a correlated startup carries one of them.
    pub fn agent_name_candidates(&self) -> [String; 2] {
        launch_agent_names(&self.seat, self.name_hint.as_deref())
    }

    /// Largest single native argument (an initial prompt is usually the biggest).
    pub const MAX_ARG_BYTES: usize = 16 * 1024;
    /// Largest total of all native arguments.
    pub const MAX_ARGV_BYTES: usize = 32 * 1024;

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.argv.len() > 64 {
            return Err("invalid native launch request: more than 64 native arguments");
        }
        if self
            .argv
            .iter()
            .any(|arg| arg.is_empty() || arg.contains('\0'))
        {
            return Err(
                "invalid native launch request: an empty native argument or one containing NUL",
            );
        }
        // Herdr starts the agent by typing its command line into the pane's
        // shell, so a line break would submit a truncated command and the start
        // is never confirmed (observed: outcome_unknown with nothing started).
        if self
            .argv
            .iter()
            .any(|arg| arg.contains('\n') || arg.contains('\r'))
        {
            return Err(
                "invalid native launch request: a native argument contains a line break; Herdr starts agents through the pane's shell, so pass the prompt on one line (or put it in a file and ask the agent to read it)",
            );
        }
        if self.argv.iter().any(|arg| arg.len() > Self::MAX_ARG_BYTES) {
            return Err(
                "invalid native launch request: a native argument is over 16 KiB (put a long prompt in a file and ask the agent to read it)",
            );
        }
        if self.argv.iter().map(String::len).sum::<usize>() > Self::MAX_ARGV_BYTES {
            return Err("invalid native launch request: native arguments total over 32 KiB");
        }
        if self.expected_incarnation.is_empty()
            || self.expected_incarnation.len() > 128
            || self.configured_hook.scope.is_empty()
            || self.configured_hook.scope.len() > 128
            || self.configured_hook.path.is_empty()
            || self.configured_hook.path.len() > 1024
            || self.configured_hook.fingerprint.is_empty()
            || self.configured_hook.fingerprint.len() > 256
        {
            return Err("invalid native launch request: internal host or hook fields out of range");
        }
        Ok(())
    }
}
/// A trusted host adapter's correlation of one submitted start request with a
/// ready `agent_started` response. This is startup evidence only; it proves no
/// current native execution, registration, receipt, or ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelatedStartup {
    pub seat: SeatId,
    pub agent_name: String,
    pub harness: Harness,
    pub target: HostTargetId,
    pub terminal: TerminalId,
    pub expected_generation: u64,
    pub expected_incarnation: String,
    pub argv: Vec<String>,
    pub host_boot: HostBootId,
    pub epoch: u64,
    pub submitted_at_mono: MonoInstant,
    pub completed_at_mono: MonoInstant,
}
impl CorrelatedStartup {
    pub fn matches_request(
        &self,
        request: &NativeLaunchRequest,
        context: &HostCallContext,
    ) -> bool {
        self.seat == request.seat
            && request.agent_name_candidates().contains(&self.agent_name)
            && self.harness == request.harness
            && self.target == request.target
            && self.terminal == request.expected_terminal
            && self.expected_generation == request.expected_generation
            && self.expected_incarnation == request.expected_incarnation
            && self.argv == request.argv
            && context.expected_boot.as_ref() == Some(&self.host_boot)
            && context.expected_epoch == Some(self.epoch)
            && self.submitted_at_mono <= self.completed_at_mono
            && self.completed_at_mono < context.budget.deadline
            && !context.budget.cancellation.is_cancelled()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
// Allowed: a transient launch result; boxing would change the port API.
#[allow(clippy::large_enum_variant)]
pub enum NativeLaunchOutcome {
    ObservedStartup {
        correlation: CorrelatedStartup,
        diagnostic: HostObservation,
    },
    OutcomeUnknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeLaunchCapability {
    HostGuardedStart,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeReservation {
    pub attempt: WakeAttemptId,
    /// Boot of the daemon that owns this durable attempt, distinct from host_boot.
    pub daemon_boot: uuid::Uuid,
    pub seat: SeatId,
    pub reasons: Vec<String>,
    pub retained_effective_delay_ms: u64,
    pub lease_until: MonoInstant,
    pub retained_minimum_delay_ms: u64,
    pub reserved_at_utc: UtcMillis,
    pub host_boot: HostBootId,
    pub host_epoch: u64,
    pub target: HostTargetId,
    pub target_generation: u64,
    /// Completed canonical attention proof accepted by the reserving store
    /// transaction. The final store fence rechecks its scalar revision and
    /// outage marker without repeating an unbounded scan.
    pub attention_witness: WakeAttentionWitness,
    /// The reserving transaction records occupant identity or an explicit
    /// recognized recovery-hint classification; a queued candidate is not authority.
    pub authority: ReservedWakeAuthority,
}

/// A historical active attempt discovered by a bounded physical seat-ordinal
/// scan. Eligibility and attention are deliberately absent: recovery must run
/// even for a retired or currently unsafe seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WakeRecoveryCandidate {
    pub seat: SeatId,
    pub attempt: WakeAttemptId,
    pub prior_daemon_boot: uuid::Uuid,
}

/// The store compares the elected boot with its own owner settings; the value
/// carried here is never authority by itself or a wire/client input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeRecoveryRequest {
    pub instance: String,
    pub seat: SeatId,
    pub attempt: WakeAttemptId,
    pub prior_daemon_boot: uuid::Uuid,
    pub elected_boot: uuid::Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeRecoveryOutcome {
    /// Exact old-boot active attempt was durably settled as uncertain.
    Recovered,
    /// This exact attempt is already retained as settled history.
    AlreadySettled,
    /// A different/current attempt or owner fence won; no state changed.
    Stale,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReservedWakeAuthority {
    Registered {
        binding_generation: u64,
        execution: ExecutionId,
    },
    RecoveryHint {
        execution: ExecutionId,
    },
    /// Cooperative native policy (the D2 reasoning): Unknown current
    /// execution never blocks structural identity. The seat's resolved target
    /// shows no verified execution, but a terminal in a verified server
    /// incarnation; a live binding, when present, names that same terminal.
    /// Eligibility of the occupant itself (recognized idle harness, never a
    /// shell, unknown harness or blocked UI) is decided by the host adapter's
    /// recheck immediately before prompting, never here.
    Cooperative {
        terminal: TerminalId,
        incarnation: String,
        binding_generation: Option<u64>,
        /// The open binding's harness, when the seat has one.
        harness: Option<String>,
    },
}
impl WakeReservation {
    /// Cooperative structural recheck against a new host read: the same
    /// target, boot, epoch, generation, terminal and verified server
    /// incarnation, from a fresh current-target read with no positive
    /// evidence of an empty shell, an active turn or blocked UI. Typed
    /// composer input (`HumanInput`) never refuses an ordinary wake
    /// (TRUST-POLICY A4), as before the composer reader existed.
    /// A verified execution is never downgraded to this path.
    pub fn matches_cooperative_identity(&self, observation: &HostObservation) -> bool {
        self.matches_cooperative_structure(observation)
            && !matches!(
                observation.ui,
                HostUiState::ActiveTurn | HostUiState::ApprovalOrQuestion
            )
    }
    /// The structural half of [`Self::matches_cooperative_identity`], without
    /// the UI exclusions: a soft-deadline poke decides the UI state itself
    /// (`poke_eligibility`) and may act in an active turn or typed input where
    /// the harness recipe declares it.
    pub fn matches_cooperative_structure(&self, observation: &HostObservation) -> bool {
        let ReservedWakeAuthority::Cooperative {
            terminal,
            incarnation,
            ..
        } = &self.authority
        else {
            return false;
        };
        observation.target == self.target
            && observation.host_boot == self.host_boot
            && observation.epoch == self.host_epoch
            && observation.generation == self.target_generation
            && observation.provenance == ObservationProvenance::FreshCurrentTarget
            && observation.terminal.as_ref() == Some(terminal)
            && matches!(&observation.incarnation, IncarnationEvidence::Verified {
                identity,
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            } if identity == incarnation)
            && !matches!(observation.execution, ExecutionEvidence::Verified { .. })
            && observation.occupancy != StructuralOccupancy::EmptyShell
    }
    /// Structural and execution recheck against a new host read. Store-side
    /// binding generation and live reservation CAS are separate requirements.
    pub fn matches_fresh_identity(&self, observation: &HostObservation) -> bool {
        if observation.target != self.target
            || observation.host_boot != self.host_boot
            || observation.epoch != self.host_epoch
            || observation.generation != self.target_generation
            || !observation.has_verified_execution()
        {
            return false;
        }
        let expected = match &self.authority {
            ReservedWakeAuthority::Registered { execution, .. }
            | ReservedWakeAuthority::RecoveryHint { execution } => execution,
            ReservedWakeAuthority::Cooperative { .. } => return false,
        };
        matches!(&observation.execution, ExecutionEvidence::Verified { execution, .. } if execution==expected)
            && matches!(&observation.occupant,Some(occupant) if occupant.is_top_level && &occupant.execution==expected)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WakeCandidate {
    pub seat: SeatId,
    /// Present only after all bounded effective attention sources complete.
    pub attention_witness: Option<WakeAttentionWitness>,
    pub effectively_retired: bool,
    pub continuity_resolved: bool,
    pub binding_generation: Option<u64>,
    pub binding_execution: Option<ExecutionId>,
    pub target: Option<HostTargetId>,
    pub reason_bits: u64,
    pub has_pending_invitation: bool,
    pub has_pending_receipt: bool,
    pub actionable_warning_generation: Option<u64>,
    pub actionable_warning_seq: Option<u64>,
    pub warning_offer: Option<WarningOfferFrontier>,
    pub attention_version: u64,
    pub checkpoint_version: u64,
    pub retry_step: u32,
    pub reservation_id: Option<WakeAttemptId>,
    pub reservation_boot: Option<HostBootId>,
    pub last_reservation_id: Option<WakeAttemptId>,
    pub last_reservation_boot: Option<HostBootId>,
    /// Immutable logical maxima frozen by the last reservation, retained after
    /// settlement and across daemon boots.
    pub last_reserved_frontier: LogicalAttentionFrontier,
    pub minimum_delay_ms: u64,
    pub effective_delay_ms: u64,
    pub last_outcome: Option<String>,
    /// UTC of the last reservation, retained so a Refused completion can
    /// restore the pre-reservation ladder row exactly (ht-p03.9.3).
    pub last_reserved_at_utc: Option<UtcMillis>,
}
/// Opaque internal proof that one store read completed the canonical effective
/// attention scan at a captured instance decision revision. No wire decoder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WakeAttentionWitness {
    pub(crate) instance: String,
    pub(crate) seat: SeatId,
    pub(crate) decision_seq: u64,
    pub(crate) pending_invitation: bool,
    pub(crate) pending_receipt: bool,
    pub(crate) latest_warning_seq: Option<u64>,
    /// Lazy outage bookkeeping may change without a decision sequence bump.
    pub(crate) unavailability_episode: u64,
    pub(crate) unavailability_open: bool,
    /// Complete logical publication maxima at the same decision snapshot.
    /// Projection cannot advance these immutable keys.
    pub(crate) frontier: LogicalAttentionFrontier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct LogicalPublicationKey {
    pub decision_seq: u64,
    /// Zero for one-event decisions. Batch warnings use their canonical
    /// immutable positive offset within the decision.
    pub event_offset: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LogicalAttentionFrontier {
    pub invitation: Option<LogicalPublicationKey>,
    pub addressed_receipt: Option<LogicalPublicationKey>,
    pub actionable_warning: Option<LogicalPublicationKey>,
}
impl LogicalAttentionFrontier {
    pub fn advanced_beyond(&self, prior: &Self) -> bool {
        self.invitation > prior.invitation
            || self.addressed_receipt > prior.addressed_receipt
            || self.actionable_warning > prior.actionable_warning
    }
}

#[cfg(test)]
mod logical_attention_frontier_tests {
    use super::*;
    #[test]
    fn second_same_kind_publication_advances_without_projection_bump() {
        let prior = LogicalAttentionFrontier {
            addressed_receipt: Some(LogicalPublicationKey {
                decision_seq: 10,
                event_offset: 0,
            }),
            ..Default::default()
        };
        assert!(!prior.advanced_beyond(&prior));
        let next = LogicalAttentionFrontier {
            addressed_receipt: Some(LogicalPublicationKey {
                decision_seq: 11,
                event_offset: 0,
            }),
            ..prior
        };
        assert!(next.advanced_beyond(&prior));
        assert!(!prior.advanced_beyond(&next));
        let warning_batch = LogicalAttentionFrontier {
            actionable_warning: Some(LogicalPublicationKey {
                decision_seq: 11,
                event_offset: 2,
            }),
            ..next
        };
        assert!(warning_batch.advanced_beyond(&next));
    }
}
impl WakeAttentionWitness {
    // Allowed: validating constructor: one argument per attention field.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_complete(
        instance: String,
        seat: SeatId,
        decision_seq: u64,
        pending_invitation: bool,
        pending_receipt: bool,
        latest_warning_seq: Option<u64>,
        unavailability_episode: u64,
        unavailability_open: bool,
        frontier: LogicalAttentionFrontier,
    ) -> Self {
        Self {
            instance,
            seat,
            decision_seq,
            pending_invitation,
            pending_receipt,
            latest_warning_seq,
            unavailability_episode,
            unavailability_open,
            frontier,
        }
    }
    pub fn frontier(&self) -> LogicalAttentionFrontier {
        self.frontier
    }
    pub fn decision_seq(&self) -> u64 {
        self.decision_seq
    }
    pub fn instance(&self) -> &str {
        &self.instance
    }
    pub fn valid_at(
        &self,
        instance: &str,
        seat: &SeatId,
        decision_seq: u64,
        unavailability_episode: u64,
        unavailability_open: bool,
    ) -> bool {
        self.instance == instance
            && &self.seat == seat
            && self.decision_seq == decision_seq
            && self.unavailability_episode == unavailability_episode
            && self.unavailability_open == unavailability_open
    }
    pub fn valid_for(
        &self,
        candidate: &WakeCandidate,
        instance: &str,
        decision_seq: u64,
        unavailability_episode: u64,
        unavailability_open: bool,
    ) -> bool {
        self.valid_at(
            instance,
            &candidate.seat,
            decision_seq,
            unavailability_episode,
            unavailability_open,
        ) && self.pending_invitation == candidate.has_pending_invitation
            && self.pending_receipt == candidate.has_pending_receipt
            && self.latest_warning_seq == candidate.actionable_warning_seq
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WarningOfferFrontier {
    pub generation: u64,
    pub execution: ExecutionId,
    pub offered_through_seq: u64,
}
impl WakeCandidate {
    pub fn warning_offered_for_current_occupant(&self) -> bool {
        match (
            self.binding_generation,
            self.binding_execution.as_ref(),
            self.actionable_warning_seq,
            self.warning_offer.as_ref(),
        ) {
            (Some(generation), Some(execution), Some(seq), Some(offer)) => {
                offer.generation == generation
                    && &offer.execution == execution
                    && seq <= offer.offered_through_seq
            }
            _ => false,
        }
    }
    pub fn has_actionable_work(&self) -> bool {
        !self.effectively_retired
            && (self.has_pending_invitation
                || self.has_pending_receipt
                || (self.actionable_warning_seq.is_some()
                    && !self.warning_offered_for_current_occupant()))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeOutcome {
    /// The prompt was delivered and verified submitted.
    Submitted,
    /// The prompt may or may not have been delivered. Also the outcome of a
    /// prompt delivered to the pane but still unsent after the single
    /// submit-key retry (ht-p03.41): it keeps the advanced ladder step, is
    /// never re-sent in a loop, and stores this same last_outcome string.
    OutcomeUnknown,
    Unsafe,
    Unavailable,
    TimedOut,
    Cancelled,
    /// The wake was refused before `submit_prompt` was called: nothing was
    /// sent, so the reminder ladder does not climb (pacer spec D5). The seat
    /// retries on a per-seat refusal backoff instead.
    Refused(RefusalCause),
}

/// Which physical receipt row holds a receipt's `soft_poked_at` (spec §10):
/// legacy `receipts` rows, or the `receipt_state` row a send manifest
/// materializes. The store reads it through the effective receipt projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PokeSource {
    Receipts,
    ReceiptState,
}

/// One receipt past its soft point and before its effective deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokeReceipt {
    pub message: MessageId,
    pub seat: SeatId,
    pub thread: ThreadId,
    pub source: PokeSource,
    /// `receipts::effective_deadline` when the store selected it.
    pub effective_deadline: i64,
}

/// A seat's due soft pokes, ordered by effective deadline then message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokeDue {
    pub seat: SeatId,
    pub receipts: Vec<PokeReceipt>,
}

/// A committed poke reservation: the wake slot (same columns, reason
/// `soft_deadline`) plus the receipts still due when it committed. Only these
/// are marked when the host accepts the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokeReservation {
    pub reservation: WakeReservation,
    pub receipts: Vec<PokeReceipt>,
}

/// The prompt a poke submits and the receipts it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokePlan {
    pub text: String,
    pub receipts: Vec<PokeReceipt>,
}

/// `PokeOnly` is a scheduled soft-deadline poke: no eligible state, no prompt.
/// `WithWake` rides an ordinary wake that is due for the same seat: the poke
/// text replaces the marker only when the poke itself is eligible to submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PokeMode {
    PokeOnly,
    WithWake,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PokeAttempt {
    pub outcome: WakeOutcome,
    /// The accepted prompt carried the poke text.
    pub poked: bool,
    /// A diagnostic the attempt kept, for example a failed composer restore
    /// (which carries the saved text so a person can recover it).
    pub diagnostic: Option<String>,
}

/// Composer-stash result (spec §10 HumanInput path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerStash {
    /// The adapter cannot stash typed input; the poke is skipped.
    Unsupported,
    /// The typed text, to be restored after the poke prompt.
    Saved(String),
    Failed(String),
}

/// Evidence-backed poke capabilities per harness. The default reports none, so
/// ActiveTurn and HumanInput pokes are skipped until a recipe declares them.
pub trait PokeCapabilitySource: Send + Sync {
    fn capabilities(&self, _harness: Harness) -> crate::harness::recipe::PokeCapabilities {
        crate::harness::recipe::PokeCapabilities::NONE
    }
}
/// No capability evidence anywhere: every capability is `Unsupported`.
pub struct NoPokeCapabilities;
impl PokeCapabilitySource for NoPokeCapabilities {}

/// Why a pre-send wake refusal happened. Stored with the existing
/// `unsafe` / `unavailable` / `timed_out` last_outcome strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCause {
    Unsafe,
    Unavailable,
    TimedOut,
}

/// The pre-reservation ladder row, carried from the reserved candidate so a
/// `Refused` completion can restore it in the same fenced UPDATE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorLadder {
    pub retry_step: u32,
    pub minimum_delay_ms: u64,
    pub effective_delay_ms: u64,
    pub last_reservation_id: Option<WakeAttemptId>,
    pub last_reservation_boot: Option<HostBootId>,
    pub last_reserved_at_utc: Option<UtcMillis>,
    pub last_reserved_frontier: LogicalAttentionFrontier,
}

impl PriorLadder {
    pub fn from_candidate(candidate: &WakeCandidate) -> Self {
        Self {
            retry_step: candidate.retry_step,
            minimum_delay_ms: candidate.minimum_delay_ms,
            effective_delay_ms: candidate.effective_delay_ms,
            last_reservation_id: candidate.last_reservation_id.clone(),
            last_reservation_boot: candidate.last_reservation_boot.clone(),
            last_reserved_at_utc: candidate.last_reserved_at_utc,
            last_reserved_frontier: candidate.last_reserved_frontier,
        }
    }
}

/// Store implementations inject a clock at construction and sample UTC inside each
/// deciding write transaction, after validation and lock/queue waits. Mutation and due
/// methods have no caller-provided UTC decision time.
pub trait StorePort: Send + Sync {
    fn clock(&self) -> &dyn Clock;
    /// Called after the live authority slot is revoked; never holds its guard.
    fn audit_service_disconnect(
        &self,
        boot: &str,
        generation: u64,
        peer: crate::protocol::authority::PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
    /// The registered transport supplies the server-private connection. The
    /// store takes its DB write lock before `gate.decision_guard` and retains
    /// that guard through commit/rollback, including exact-key replay. The
    /// elected service passes its writer so each notification preparation and
    /// publication quantum is admitted separately and native writes can make
    /// progress between them; `None` takes the store's own writer lock only.
    fn service_operation(
        &self,
        operation: ServiceOperation,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
        admission: Option<&crate::service::fair_writer::FairWriter>,
    ) -> Result<ServiceResult, ApiError>;
    fn query(
        &self,
        command: &Command,
        read: &ReadContext,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    fn mutate(
        &self,
        command: PermitMutation,
        permit: MutationPermit,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// Hidden bounded staging keyed by the request's durable operation identity.
    /// Caller authority is rechecked at final `mutate` publication.
    fn prepare_send_step(
        &self,
        request: &SendMessage,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SendPreparationProgress, ApiError>;
    /// Runs under a fresh service budget after a request stops. The expected
    /// preparation ID fences cleanup against a same-key successor rebuild.
    fn abandon_send_preparation(
        &self,
        expected_preparation_id: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
    /// Resolve under the ordinary allocation operation scope, replaying before
    /// present-state validation. A resolved result alone grants no launch authority.
    fn resolve_seat(
        &self,
        request: ResolveSeat,
        attempt: OrdinaryResolutionAttempt,
        budget: &CallBudget,
    ) -> Result<OrdinaryResolutionOutcome, ApiError>;
    /// Never replays, allocates or replaces the expected seat.
    fn check_resolved_target(
        &self,
        check: ResolvedTargetCheck,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
    /// Payload-bound historical operator result; a miss has no durable effects.
    fn replay_operator(
        &self,
        command: OperatorCommand,
        actor: OperatorActor,
        budget: &CallBudget,
    ) -> Result<Option<CommandResult>, ApiError>;
    fn mutate_operator(
        &self,
        command: OperatorRequest,
        actor: OperatorActor,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// Payload-bound historical continuity result; a miss has no durable effects.
    fn replay_continuity(
        &self,
        command: ContinuityCheckIn,
        budget: &CallBudget,
    ) -> Result<Option<CommandResult>, ApiError>;
    /// TRUST-POLICY C1: reattach the unique unresolved seat whose last binding
    /// carries the resumed session id.
    fn decide_continuity(
        &self,
        request: ContinuityRequest,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// TRUST-POLICY A3 `managed_launch` (ht-5n6): open an unregistered
    /// occupant binding for a correlated managed launch on a seat with no
    /// open binding, decided against the effective observation.
    fn record_managed_launch(
        &self,
        command: crate::protocol::commands::RecordManagedLaunch,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// Local durable validation only; implementations must not invent native proof.
    fn issue_cooperative_permit(
        &self,
        request: CooperativePermitRequest,
        budget: &CallBudget,
    ) -> Result<MutationPermit, ApiError>;

    fn register_available(
        &self,
        request: RegisterAvailableRequest,
        permit: MutationPermit,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// Read-only: the host epoch persisted by the last accepted publication
    /// for this instance (0 when none). Used only to start a new epoch above
    /// it at daemon boot; it grants no observation authority.
    fn persisted_host_epoch(&self, _instance: &str, _budget: &CallBudget) -> Result<u64, ApiError>;
    /// Reserve durable ordering before a host capture. This sequence fences
    /// late failed reads; it does not certify coherent enumeration.
    fn begin_host_observation(
        &self,
        instance: &str,
        budget: &CallBudget,
    ) -> Result<HostObservationAdmission, ApiError>;
    /// Admit one trusted explicit-target read after host I/O, under the same
    /// store-issued ordering/failure fence as coherent snapshot publication.
    /// A stale or mismatched admission returns false without changing durable
    /// proof. This never clears recovery holds or grants native authority.
    fn publish_current_target_observation(
        &self,
        admission: &HostObservationAdmission,
        observation: &HostObservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError>;
    /// A failed/partial trusted capture invalidates effective availability in
    /// one scalar write. Stale admissions return None without effects.
    fn invalidate_host_observation(
        &self,
        admission: &HostObservationAdmission,
        reason: HostInvalidationReason,
        budget: &CallBudget,
    ) -> Result<Option<HostInvalidationFence>, ApiError>;
    /// Bounded projection after invalidation; cannot infer target absence.
    fn mark_unresolved_from_invalidation(
        &self,
        transition: GuardedInvalidationTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError>;
    /// Bounded saved-seat keyset under the exact accepted invalidation. A
    /// newer host decision returns CursorStale before old rows are projected.
    fn saved_seats_page_for_invalidation(
        &self,
        fence: &HostInvalidationFence,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<InvalidationSeatPage, ApiError>;
    /// The stage is invisible until sealed and atomically published.
    fn begin_snapshot_stage(
        &self,
        header: SnapshotHeader,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError>;
    /// Exactly the next contiguous slice, at most 16 counted target rows.
    fn stage_snapshot_targets(
        &self,
        stage: &SnapshotGenerationId,
        offset: u64,
        targets: &[HostObservation],
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotStageProgress, ApiError>;
    fn seal_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<SnapshotStage, ApiError>;
    /// One constant-size active-pointer decision with lifecycle/recovery CAS.
    fn publish_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        budget: &CallBudget,
    ) -> Result<PublishedSnapshot, ApiError>;
    /// Discard/cleanup advances in at most 16 counted physical rows per call.
    fn discard_snapshot_stage(
        &self,
        stage: &SnapshotGenerationId,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<SnapshotCleanupProgress, ApiError>;
    /// One retention pass: one bounded snapshot transaction, then one bounded
    /// work-job transaction (spec D3). Opens no write transaction when no row
    /// qualifies.
    fn prune_retention(&self, budget: &CallBudget) -> Result<PruneProgress, ApiError>;
    /// Records one hook payload's harness evidence (ht-xoc.4): upserts the
    /// (harness, version, contract id) row and applies the outcome.
    fn record_harness_evidence(
        &self,
        record: &crate::store::harness_evidence::EvidenceRecord<'_>,
        budget: &CallBudget,
    ) -> Result<crate::store::harness_evidence::Recorded, ApiError>;
    fn harness_evidence(
        &self,
        harness: &str,
        version: &str,
        contract_id: &str,
        budget: &CallBudget,
    ) -> Result<Option<crate::store::harness_evidence::EvidenceRow>, ApiError>;
    /// Rows of both harnesses with `last_seen_at >= since_ms`.
    fn harness_evidence_since(
        &self,
        since_ms: u64,
        budget: &CallBudget,
    ) -> Result<Vec<crate::store::harness_evidence::EvidenceRow>, ApiError>;
    /// Every row of one harness (doctor and "newest verified here").
    fn harness_evidence_all(
        &self,
        harness: &str,
        budget: &CallBudget,
    ) -> Result<Vec<crate::store::harness_evidence::EvidenceRow>, ApiError>;
    /// Keeps the latest reason a payload of `harness` was unattributable.
    fn record_unattributed(
        &self,
        harness: &str,
        reason: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
    fn last_unattributed(
        &self,
        harness: &str,
        budget: &CallBudget,
    ) -> Result<Option<(String, u64)>, ApiError>;
    /// A changed active snapshot returns CursorStale before any old row is read.
    fn saved_seats_page(
        &self,
        published: &SnapshotGenerationId,
        after_ordinal: u64,
        high_water_ordinal: Option<u64>,
        limit: u8,
        budget: &CallBudget,
    ) -> Result<SnapshotSeatPage, ApiError>;
    /// Each decision rechecks publication, invalidation and saved-seat CAS.
    fn apply_reconciliation_transition(
        &self,
        transition: GuardedSeatTransition,
        budget: &CallBudget,
    ) -> Result<ReconciliationOutcome, ApiError>;
    /// Persist that the saved-seat pass of `published` ended with no refused
    /// transition and lift the restore hold when nothing is left to protect.
    /// Returns whether the marker was written (false for a stale publication).
    fn record_reconciliation_pass(
        &self,
        published: &PublishedSnapshot,
        budget: &CallBudget,
    ) -> Result<bool, ApiError>;
    fn due_obligations(
        &self,
        request: DueScanRequest,
        budget: &CallBudget,
    ) -> Result<DueScanProgress, ApiError>;
    fn begin_retirement(
        &self,
        seat: SeatId,
        proof: ClosureEvidence,
        budget: &CallBudget,
    ) -> Result<RetirementJob, ApiError>;
    fn advance_retirement(
        &self,
        job: RetirementJobId,
        admission: WorkAdmission,
        budget: &CallBudget,
    ) -> Result<RetirementProgress, ApiError>;
    fn pending_retirement_jobs(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<RetirementStatus>, ApiError>;
    /// The startup binding-evidence verification result of this store's
    /// writer. None when the implementation has no durable writer.
    fn binding_evidence_startup(&self) -> Option<BindingEvidenceStartup>;
    /// The startup result with `still_lacking` rechecked against the store
    /// now (wave-2 fix2 (b)): a seat whose agent re-registered with evidence,
    /// or that was retired, no longer counts. Only the seats found lacking at
    /// startup are rechecked, each by one indexed lookup.
    fn binding_evidence_current(
        &self,
        budget: &CallBudget,
    ) -> Result<Option<BindingEvidenceStartup>, ApiError>;
    fn unresolved_seat_summary(
        &self,
        budget: &CallBudget,
    ) -> Result<UnresolvedSeatSummary, ApiError>;
    fn retirement_summary(&self, _budget: &CallBudget) -> Result<RetirementSummary, ApiError>;
    /// Thread summary protocol (spec §4): plan, lease and assemble.
    fn summary(
        &self,
        _request: &crate::protocol::summary::SummaryRequest,
        _budget: &CallBudget,
    ) -> Result<crate::protocol::summary::SummaryOutcome, ApiError> {
        Err(ApiError::unsupported("thread summaries are unavailable"))
    }
    /// Fetch a leased job's bundle; the fetch starts the lease clock.
    fn summary_job(
        &self,
        _request: &crate::protocol::summary::SummaryJobRequest,
        _budget: &CallBudget,
    ) -> Result<crate::protocol::summary::SummaryJobOutcome, ApiError> {
        Err(ApiError::unsupported("thread summaries are unavailable"))
    }
    /// Validate and store a job submission (idempotent per job and token).
    fn summary_submit(
        &self,
        _request: &crate::protocol::summary::SummarySubmitRequest,
        _budget: &CallBudget,
    ) -> Result<crate::protocol::summary::SubmitOutcome, ApiError> {
        Err(ApiError::unsupported("thread summaries are unavailable"))
    }
    fn wake_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeCandidate>, ApiError>;
    fn pending_work(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WorkCandidate>, ApiError>;
    fn advance_work(
        &self,
        job: &str,
        admission: DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<WorkProgress, ApiError>;
    fn reserve_wake(
        &self,
        candidate: &WakeCandidate,
        budget: &CallBudget,
    ) -> Result<Option<WakeReservation>, ApiError>;
    /// Indexed immutable-seat-ordinal page of active reservations, including
    /// zero-reason, unsafe and retired historical rows. This is owner recovery,
    /// never prompt eligibility.
    fn wake_recovery_candidates(
        &self,
        page: PageRequest,
        budget: &CallBudget,
    ) -> Result<Page<WakeRecoveryCandidate>, ApiError>;
    /// Compare exact instance/seat/attempt/prior boot and current elected owner
    /// boot. Record uncertainty while preserving frozen delay and history.
    fn recover_wake_reservation(
        &self,
        request: WakeRecoveryRequest,
        budget: &CallBudget,
    ) -> Result<WakeRecoveryOutcome, ApiError>;
    /// Final short store fence after a fresh host read and before prompt I/O.
    /// Rechecks live attempt/daemon boot, seat/target/binding identity, and
    /// effective actionable work. No SQL writer is held during prompt I/O.
    fn validate_wake_reservation(
        &self,
        reservation: &WakeReservation,
        budget: &CallBudget,
    ) -> Result<bool, ApiError>;
    /// Returns whether the fenced `refused_restore` matched. It is `false`
    /// for every non-Refused outcome and when a path cleared the reservation
    /// between reserve and completion (pacer D5 fence miss: accepted, the
    /// durable step stays advanced by one).
    fn complete_wake(
        &self,
        attempt: WakeAttemptId,
        outcome: WakeOutcome,
        refused_restore: Option<&PriorLadder>,
        budget: &CallBudget,
    ) -> Result<bool, ApiError>;
    /// Seats with receipts past their soft point (spec §10), at most `limit`
    /// seats, evaluated lazily from each receipt's effective deadline.
    fn poke_candidates(&self, _limit: u16, _budget: &CallBudget) -> Result<Vec<PokeDue>, ApiError> {
        Err(unsupported_poke())
    }
    /// Takes the seat's single wake reservation slot for a poke (reason
    /// `soft_deadline`). `None`: the slot is busy or nothing is still due.
    fn reserve_poke(
        &self,
        _due: &PokeDue,
        _budget: &CallBudget,
    ) -> Result<Option<PokeReservation>, ApiError> {
        Err(unsupported_poke())
    }
    /// Settles a poke (or a wake that carried poke text) and, only on
    /// `Submitted`, sets `soft_poked_at` on exactly `receipts` in the same
    /// transaction. Any other outcome marks nothing.
    fn complete_poke(
        &self,
        _attempt: WakeAttemptId,
        _outcome: WakeOutcome,
        _receipts: &[PokeReceipt],
        _budget: &CallBudget,
    ) -> Result<(), ApiError> {
        Err(unsupported_poke())
    }
    /// Seat-scoped due pokes, for an ordinary wake that can carry the poke text.
    fn poke_for_wake(
        &self,
        _seat: &SeatId,
        _budget: &CallBudget,
    ) -> Result<Option<PokeDue>, ApiError> {
        Err(unsupported_poke())
    }
}

fn unsupported_poke() -> ApiError {
    ApiError::unsupported("soft-deadline pokes are unavailable")
}

/// Every call has an absolute monotonic deadline and cancellation token. Host calls
/// run outside the domain writer. Observation results carry their actual provenance.
/// What the pane's agent composer shows after a wake prompt was sent (Wave 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentComposerState {
    /// The composer is empty / the agent is working: the prompt was submitted.
    Submitted,
    /// The composer still holds the sent prompt text (not submitted).
    HoldingPrompt,
    /// The adapter cannot tell (no screen read, unsupported harness).
    Unknown,
}

pub trait HostPort: Send + Sync {
    /// Adapter-owned, verified support; callers cannot assert a launch capability.
    fn native_launch_capability(&self) -> NativeLaunchCapability;
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError>;
    /// [`Self::observe_current_target`] for an attempt that carries a
    /// soft-deadline poke: the one place the adapter may spend a composer read
    /// to classify the UI (`Idle`, `HumanInput`, `ActiveTurn`). An ordinary
    /// wake never calls this, so it decides on Herdr's agent status and the
    /// structural identity alone (TRUST-POLICY A4). The default is the plain
    /// observation.
    fn observe_current_target_for_poke(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.observe_current_target(target, context)
    }
    fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError>;
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget>;
    /// The target a soft-deadline poke may reach. A poke decides the UI state
    /// itself (`poke_eligibility`) and may act in an active turn or typed
    /// input where a recipe declares it, so the default is the wake target;
    /// an adapter whose wake target excludes those UI states overrides it.
    fn safe_poke_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        self.safe_wake_target(seat, observation)
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError>;
    /// A soft-deadline poke into a running turn (spec §10 `poke_during_turn`):
    /// the prompt is queued into the current turn, so the adapter's recheck
    /// accepts a `working` agent for this call only. Ordinary wakes keep
    /// `submit_prompt`'s idle/done recheck. Adapters without the mode submit
    /// as an ordinary wake, which refuses a working agent.
    fn submit_prompt_during_turn(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        context: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.submit_prompt(target, text, context)
    }
    fn launch_native(
        &self,
        request: NativeLaunchRequest,
        context: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError>;
    /// Read the pane's agent composer state after a wake send (Wave 28 verification).
    fn pane_agent_state(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError>;
    /// Send exactly one submit key (Enter) to the wake target's composer. Used
    /// once per wake drive when the prompt was left unsent (Wave 28).
    fn send_submit_key(
        &self,
        target: &SafeWakeTarget,
        context: &HostCallContext,
    ) -> Result<(), ApiError>;
    /// Herdr's agent record for one pane. `Ok(None)`: Herdr answered and
    /// reports no agent; `Err`: the read failed. Adapters without the route
    /// report "no agent".
    fn observe_pane_agent(
        &self,
        _target: &HostTargetId,
        _context: &HostCallContext,
    ) -> Result<Option<PaneAgentObservation>, ApiError> {
        Ok(None)
    }
    /// Elected composition calls this once, before any observation, with the
    /// host epoch the previous daemon boot persisted. Daemon restart creates a
    /// new, higher connection epoch so a same-incarnation capture orders after
    /// every durable publication. Adapters without an epoch ignore it.
    fn resume_after_epoch(&self, _persisted: u64) {}
    /// Adapter-owned platform capability to witness the host server
    /// incarnation on response connections. `Unsupported` is a static fact
    /// (Health reports it without waiting for a capture); `Unknown` leaves
    /// Health to the evidence the observation lane actually observes. An
    /// adapter can never assert `Supported` here: only a verified coherent
    /// publication establishes it.
    fn incarnation_witness(&self) -> crate::protocol::results::CapabilityState {
        crate::protocol::results::CapabilityState::Unknown
    }
    /// Adapter-owned static capability to submit a wake prompt only after its
    /// own bounded recheck of the target. Health reports it as `safe_prompt`.
    fn safe_prompt_capability(&self) -> crate::protocol::results::CapabilityState {
        crate::protocol::results::CapabilityState::Unsupported
    }
    /// Composer-stash hook for a poke into typed input (spec §10). A recipe
    /// that declares `composer_stash` from captured evidence overrides it; the
    /// inert default reports `Unsupported`, which skips the poke before any
    /// prompt is submitted.
    fn stash_composer(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<ComposerStash, ApiError> {
        Ok(ComposerStash::Unsupported)
    }
    /// Restores text saved by `stash_composer` after the poke prompt.
    fn restore_composer(
        &self,
        _target: &SafeWakeTarget,
        _saved: &str,
        _context: &HostCallContext,
    ) -> Result<(), ApiError> {
        Err(unsupported_poke())
    }
}
#[derive(Debug, Clone)]
pub struct HostCallContext {
    pub budget: CallBudget,
    pub expected_boot: Option<HostBootId>,
    pub expected_epoch: Option<u64>,
}

pub trait NotificationPort: Send + Sync {
    fn attempt_wake(
        &self,
        reservation: WakeReservation,
        context: &HostCallContext,
    ) -> Result<WakeOutcome, ApiError>;
    /// A soft-deadline poke (spec §10) through the same reservation checks as
    /// a wake. The default cannot poke: `PokeOnly` is unavailable and
    /// `WithWake` is an ordinary wake.
    fn attempt_poke(
        &self,
        reservation: WakeReservation,
        plan: &PokePlan,
        mode: PokeMode,
        caps: &dyn PokeCapabilitySource,
        context: &HostCallContext,
    ) -> Result<PokeAttempt, ApiError> {
        let _ = (plan, caps);
        match mode {
            PokeMode::PokeOnly => Ok(PokeAttempt {
                outcome: WakeOutcome::Unavailable,
                poked: false,
                diagnostic: None,
            }),
            PokeMode::WithWake => {
                self.attempt_wake(reservation, context)
                    .map(|outcome| PokeAttempt {
                        outcome,
                        poked: false,
                        diagnostic: None,
                    })
            }
        }
    }
    /// The post-send verification of the last wake attempt for `seat`, taken
    /// once (ht-p03.30). Notifiers that do not verify report nothing.
    fn take_verification(
        &self,
        _seat: &SeatId,
    ) -> Option<crate::scheduler::SubmissionVerification> {
        None
    }
}

/// Local client and service share the same typed command/result envelope.
pub trait LocalClient: Send + Sync {
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError>;
    /// A selected read must carry its output context through the transport so
    /// server-generated continuation argv uses the same selectors.
    fn call_with_output(
        &self,
        command: Command,
        output: &crate::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// Outer error: not sent or outcome unknown (the durable intent stays
    /// pending). Inner `Err`: the daemon's correlated, definitive rejection.
    /// The default cannot tell them apart, so every error is treated as
    /// uncertain; transports that can distinguish them override this.
    fn call_definitive(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<Result<CommandResult, ApiError>, ApiError> {
        self.call(command, budget).map(Ok)
    }
}
pub trait LocalService: Send + Sync {
    /// Operator recovery routes through the ordinary same-UID transport path.
    fn service_control(
        &self,
        command: Command,
        peer: crate::protocol::authority::PeerIdentity,
        instance: &str,
        boot: &str,
        gate: &crate::service::live_gate::LiveServiceGate,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;

    fn audit_service_disconnect(
        &self,
        boot: &str,
        generation: u64,
        peer: crate::protocol::authority::PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(), ApiError>;
    /// The connection and gate are supplied only by the registered UDS transport.
    fn service_operation(
        &self,
        operation: ServiceOperation,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
    ) -> Result<ServiceResult, ApiError>;
    fn handle(
        &self,
        command: Command,
        peer: crate::protocol::authority::PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
    /// Presentation affects bounded reads only. The transport still supplies
    /// peer identity and the elected instance; output is never authority.
    fn handle_with_output(
        &self,
        command: Command,
        peer: crate::protocol::authority::PeerIdentity,
        budget: &CallBudget,
        output: &crate::protocol::output::OutputSpec,
    ) -> Result<CommandResult, ApiError>;
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    use crate::protocol::commands::ResolveSeat;

    /// ht-p03.3: no port method may fall back to `Unsupported`, and no
    /// `_admitted` duplicate entry point remains.
    #[test]
    fn ports_have_no_unsupported_defaults_or_admitted_duplicates() {
        let src = include_str!("ports.rs");
        // Production text only: drop every top-level `#[cfg(test)] mod ... { }` block.
        let mut production = String::new();
        let mut in_test_mod = false;
        let mut pending_cfg = false;
        for line in src.lines() {
            if in_test_mod {
                if line == "}" {
                    in_test_mod = false;
                }
                continue;
            }
            if line == "#[cfg(test)]" {
                pending_cfg = true;
                continue;
            }
            if pending_cfg {
                pending_cfg = false;
                if line.starts_with("mod ") {
                    in_test_mod = true;
                    continue;
                }
            }
            production.push_str(line);
            production.push('\n');
        }
        assert!(
            !production.contains("_admitted"),
            "fold _admitted entry points"
        );
        for (at, _) in production.match_indices("ErrorCode::Unsupported") {
            let preceding = &production[..at];
            let trait_open = preceding.rfind("\npub trait ");
            let impl_open = preceding.rfind("\nimpl");
            assert!(
                impl_open > trait_open,
                "trait default returns Unsupported near byte {at}"
            );
        }
    }

    #[test]
    fn operator_operation_scope_is_instance_qualified() {
        let actor = OperatorActor::from_peer(
            crate::protocol::authority::PeerIdentity::from_kernel(501),
            501,
        )
        .unwrap();
        let scope = OperationReadScope::Operator(actor);
        assert_ne!(
            scope.actor_scope("instance-a"),
            scope.actor_scope("instance-b")
        );
        assert!(
            scope
                .actor_scope("instance-a")
                .starts_with("operator:instance-a:")
        );
    }

    #[test]
    fn stage_header_requires_complete_verified_coherent_snapshot() {
        let mut snapshot = HostSnapshot {
            boot: HostBootId::new("boot"),
            epoch: 1,
            observation_sequence: 1,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: Vec::new(),
        };
        let admission = HostObservationAdmission {
            instance: "instance".into(),
            sequence: 1,
            expected_active: None,
            expected_boot: None,
            expected_epoch: 0,
            lifecycle_revision: 0,
            invalidation_revision: 0,
        };
        let header = SnapshotHeader::from_captured(admission.clone(), &snapshot).unwrap();
        assert_eq!(header.expected_targets, 0);
        snapshot.complete = false;
        assert!(SnapshotHeader::from_captured(admission.clone(), &snapshot).is_err());
        snapshot.complete = true;
        snapshot.epoch = (i64::MAX as u64) + 1;
        assert!(SnapshotHeader::from_captured(admission, &snapshot).is_err());
    }

    #[test]
    fn wake_reservation_rechecks_committed_target_and_binding_identity() {
        let observation = HostObservation {
            focused: false,
            target: HostTargetId::new("pane"),
            host_boot: HostBootId::new("boot"),
            epoch: 3,
            generation: 4,
            observed_at_utc: UtcMillis(1),
            observed_at_mono: MonoInstant(1),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: Some(NativeOccupant {
                harness: Harness::Codex,
                session: NativeSessionId::new("n"),
                execution: ExecutionId::new("e"),
                is_top_level: true,
            }),
            ui: HostUiState::Idle,
            terminal: None,
            occupancy: StructuralOccupancy::Occupied,
            incarnation: IncarnationEvidence::Verified {
                identity: "inc".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Verified {
                execution: ExecutionId::new("e"),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            call_id: HostCallId::new("c"),
            connection_epoch: 1,
            observation_sequence: 5,
            started_at_mono: MonoInstant(1),
            completed_at_mono: MonoInstant(1),
        };
        let reservation = WakeReservation {
            attempt: WakeAttemptId::new("a"),
            daemon_boot: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
            seat: SeatId::new("s"),
            reasons: vec![],
            retained_effective_delay_ms: 1,
            lease_until: MonoInstant(2),
            retained_minimum_delay_ms: 1,
            reserved_at_utc: UtcMillis(1),
            host_boot: HostBootId::new("boot"),
            host_epoch: 3,
            target: HostTargetId::new("pane"),
            target_generation: 4,
            attention_witness: WakeAttentionWitness::from_complete(
                "i".into(),
                SeatId::new("s"),
                7,
                true,
                false,
                None,
                1,
                true,
                LogicalAttentionFrontier::default(),
            ),
            authority: ReservedWakeAuthority::Registered {
                binding_generation: 2,
                execution: ExecutionId::new("e"),
            },
        };
        assert!(reservation.matches_fresh_identity(&observation));
        let mut changed = observation.clone();
        changed.target = HostTargetId::new("other");
        assert!(!reservation.matches_fresh_identity(&changed));
        changed = observation.clone();
        changed.epoch = 4;
        assert!(!reservation.matches_fresh_identity(&changed));
        changed = observation;
        changed.occupant.as_mut().unwrap().execution = ExecutionId::new("successor");
        assert!(!reservation.matches_fresh_identity(&changed));
    }

    #[test]
    fn completed_wake_attention_witness_rejects_wrong_scope_or_revision() {
        let candidate = WakeCandidate {
            seat: SeatId::new("s"),
            attention_witness: None,
            effectively_retired: false,
            continuity_resolved: true,
            binding_generation: Some(1),
            binding_execution: Some(ExecutionId::new("e")),
            target: Some(HostTargetId::new("p")),
            reason_bits: 0,
            has_pending_invitation: true,
            has_pending_receipt: false,
            actionable_warning_generation: None,
            actionable_warning_seq: None,
            warning_offer: None,
            attention_version: 0,
            checkpoint_version: 0,
            retry_step: 0,
            reservation_id: None,
            reservation_boot: None,
            last_reservation_id: None,
            last_reservation_boot: None,
            last_reserved_frontier: LogicalAttentionFrontier::default(),
            minimum_delay_ms: 0,
            effective_delay_ms: 0,
            last_outcome: None,
            last_reserved_at_utc: None,
        };
        let witness = WakeAttentionWitness::from_complete(
            "i".into(),
            SeatId::new("s"),
            7,
            true,
            false,
            None,
            1,
            true,
            LogicalAttentionFrontier::default(),
        );
        assert!(witness.valid_for(&candidate, "i", 7, 1, true));
        // Reservation bookkeeping leaves decision_seq unchanged. The final
        // short store fence can accept this same proof, then reject any
        // attention decision or outage marker change without rescanning.
        assert!(witness.valid_at("i", &SeatId::new("s"), 7, 1, true));
        assert!(!witness.valid_at("i", &SeatId::new("s"), 8, 1, true));
        assert!(!witness.valid_at("i", &SeatId::new("s"), 7, 2, true));
        assert!(!witness.valid_at("i", &SeatId::new("s"), 7, 1, false));
        assert!(!witness.valid_at("i", &SeatId::new("other"), 7, 1, true));
        assert!(!witness.valid_for(&candidate, "other", 7, 1, true));
        assert!(!witness.valid_for(&candidate, "i", 8, 1, true));
        assert!(!witness.valid_for(&candidate, "i", 7, 2, true));
        assert!(!witness.valid_for(&candidate, "i", 7, 1, false));
        let mut other = candidate;
        other.seat = SeatId::new("other");
        assert!(!witness.valid_for(&other, "i", 7, 1, true));
        other.seat = SeatId::new("s");
        other.has_pending_invitation = false;
        assert!(!witness.valid_for(&other, "i", 7, 1, true));
    }

    #[test]
    fn operator_repair_requires_fresh_matching_target_but_orphan_invite_does_not() {
        use crate::protocol::commands::{OperatorFreshSeat, OperatorOrphanInvite, OperatorRebind};
        let observation = HostObservation {
            focused: false,
            target: HostTargetId::new("p1"),
            host_boot: HostBootId::new("b1"),
            epoch: 4,
            generation: 2,
            observed_at_utc: UtcMillis(1),
            observed_at_mono: MonoInstant(1),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: None,
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Unknown,
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("test-call"),
            connection_epoch: 0,
            observation_sequence: 0,
            started_at_mono: MonoInstant(1),
            completed_at_mono: MonoInstant(1),
        };
        let fresh = OperatorFreshSeat {
            target: HostTargetId::new("p1"),
            operation: OperationId::new("o1"),
        };
        let guard = OperatorTargetGuard::try_new(
            "i1",
            &OperatorCommand::FreshSeat(fresh.clone()),
            observation.clone(),
        )
        .unwrap();
        assert_eq!(guard.target(), &fresh.target);
        assert_eq!(guard.generation(), 2);
        assert!(matches!(
            OperatorRequest::FreshSeat(fresh.clone(), guard),
            OperatorRequest::FreshSeat(..)
        ));
        let rebind = OperatorRebind {
            seat: SeatId::new("s1"),
            target: HostTargetId::new("p2"),
            operation: OperationId::new("o2"),
        };
        assert!(
            OperatorTargetGuard::try_new(
                "i1",
                &OperatorCommand::Rebind(rebind),
                observation.clone()
            )
            .is_err()
        );
        assert!(
            OperatorTargetGuard::try_new(
                "i1",
                &OperatorCommand::FreshSeat(fresh.clone()),
                HostObservation {
                    provenance: ObservationProvenance::UncharacterizedCache,
                    ..observation
                }
            )
            .is_err()
        );
        let orphan = OperatorOrphanInvite {
            thread: ThreadId::new("t1"),
            seat: SeatId::new("s1"),
            deadline_millis: None,
            operation: OperationId::new("o3"),
        };
        assert!(
            OperatorTargetGuard::try_new(
                "i1",
                &OperatorCommand::OrphanInvite(orphan.clone()),
                operator_observation("p1")
            )
            .is_err()
        );
        assert!(matches!(
            OperatorRequest::OrphanInvite(orphan),
            OperatorRequest::OrphanInvite(..)
        ));
    }

    fn operator_observation(target: &str) -> HostObservation {
        HostObservation {
            focused: false,
            target: HostTargetId::new(target),
            host_boot: HostBootId::new("b1"),
            epoch: 4,
            generation: 2,
            observed_at_utc: UtcMillis(1),
            observed_at_mono: MonoInstant(100),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: None,
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Unknown,
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("test-call"),
            connection_epoch: 0,
            observation_sequence: 0,
            started_at_mono: MonoInstant(100),
            completed_at_mono: MonoInstant(100),
        }
    }

    fn operator_fence() -> DecisionFence {
        DecisionFence {
            now: MonoInstant(350),
            host_boot: HostBootId::new("b1"),
            host_epoch: 4,
            target_generation: 2,
            binding_generation: 0,
            known_invalidated: false,
        }
    }

    #[test]
    fn operator_request_consumes_matching_evidence_once_at_age_limit() {
        let target = HostTargetId::new("p1");
        let command = OperatorFreshSeat {
            target,
            operation: OperationId::new("o1"),
        };
        let guard = OperatorTargetGuard::try_new(
            "i1",
            &OperatorCommand::FreshSeat(command.clone()),
            operator_observation("p1"),
        )
        .unwrap();
        let mut request = OperatorRequest::FreshSeat(command, guard);
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_ok()
        );
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_err()
        );
    }

    #[test]
    fn operator_request_rejects_wrong_command_target_or_instance_without_consuming() {
        let target = HostTargetId::new("p1");
        let original = OperatorRebind {
            seat: SeatId::new("s1"),
            target: target.clone(),
            operation: OperationId::new("o1"),
        };
        let guard = OperatorTargetGuard::try_new(
            "i1",
            &OperatorCommand::Rebind(original.clone()),
            operator_observation("p1"),
        )
        .unwrap();
        let mut request = OperatorRequest::Rebind(
            OperatorRebind {
                seat: SeatId::new("s1"),
                target: HostTargetId::new("p2"),
                operation: OperationId::new("o1"),
            },
            guard,
        );
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_err()
        );
        if let OperatorRequest::Rebind(command, _) = &mut request {
            command.target = target;
        }
        assert!(
            request
                .consume_for_decision("i2", &operator_fence())
                .is_err()
        );
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_ok()
        );
    }

    #[test]
    fn operator_request_rejects_different_command_on_same_target() {
        let target = HostTargetId::new("p1");
        let original = OperatorRebind {
            seat: SeatId::new("s1"),
            target: target.clone(),
            operation: OperationId::new("o1"),
        };
        let guard = OperatorTargetGuard::try_new(
            "i1",
            &OperatorCommand::Rebind(original),
            operator_observation("p1"),
        )
        .unwrap();
        let mut request = OperatorRequest::Rebind(
            OperatorRebind {
                seat: SeatId::new("s1"),
                target: target.clone(),
                operation: OperationId::new("o2"),
            },
            guard,
        );
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_err()
        );
        if let OperatorRequest::Rebind(command, _) = &mut request {
            command.operation = OperationId::new("o1");
            command.seat = SeatId::new("s2");
        }
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_err()
        );
        if let OperatorRequest::Rebind(command, _) = &mut request {
            command.seat = SeatId::new("s1");
        }
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_ok()
        );
    }

    #[test]
    fn operator_request_rejects_changed_or_expired_decision_without_consuming() {
        let target = HostTargetId::new("p1");
        let command = OperatorFreshSeat {
            target,
            operation: OperationId::new("o1"),
        };
        let guard = OperatorTargetGuard::try_new(
            "i1",
            &OperatorCommand::FreshSeat(command.clone()),
            operator_observation("p1"),
        )
        .unwrap();
        let mut request = OperatorRequest::FreshSeat(command, guard);
        let good = operator_fence();
        for changed in [
            DecisionFence {
                host_boot: HostBootId::new("b2"),
                ..good.clone()
            },
            DecisionFence {
                host_epoch: 5,
                ..good.clone()
            },
            DecisionFence {
                target_generation: 3,
                ..good.clone()
            },
            DecisionFence {
                known_invalidated: true,
                ..good.clone()
            },
            DecisionFence {
                now: MonoInstant(99),
                ..good.clone()
            },
            DecisionFence {
                now: MonoInstant(good.now.0 + 1),
                ..good.clone()
            },
        ] {
            assert!(request.consume_for_decision("i1", &changed).is_err());
        }
        assert!(request.consume_for_decision("i1", &good).is_ok());
    }

    #[test]
    fn operator_request_allows_unoccupied_target_repair() {
        let target = HostTargetId::new("p1");
        let command = OperatorFreshSeat {
            target,
            operation: OperationId::new("o1"),
        };
        let guard = OperatorTargetGuard::try_new(
            "i1",
            &OperatorCommand::FreshSeat(command.clone()),
            operator_observation("p1"),
        )
        .unwrap();
        let mut request = OperatorRequest::FreshSeat(command, guard);
        assert!(
            request
                .consume_for_decision("i1", &operator_fence())
                .is_ok()
        );
    }

    #[test]
    fn structural_proof_needs_fresh_ordered_verified_evidence() {
        let request = ResolveSeat {
            target: HostTargetId::new("p1"),
            operation: OperationId::new("op1"),
        };
        let observation = HostObservation {
            focused: false,
            target: request.target.clone(),
            host_boot: HostBootId::new("b1"),
            epoch: 4,
            generation: 2,
            observed_at_utc: UtcMillis(1),
            observed_at_mono: MonoInstant(1),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: Some(TerminalId::new("term-p1")),
            occupancy: StructuralOccupancy::Unknown,
            incarnation: IncarnationEvidence::Verified {
                identity: "inc-1".into(),
                evidence_kind: EvidenceKind::NativeCurrentTarget,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("test-call"),
            connection_epoch: 0,
            observation_sequence: 1,
            started_at_mono: MonoInstant(1),
            completed_at_mono: MonoInstant(1),
        };
        let proof = observation.verified_structural_proof().unwrap();
        assert_eq!(proof.terminal().as_str(), "term-p1");
        assert_eq!(proof.incarnation(), "inc-1");
        let stale = HostObservation {
            provenance: ObservationProvenance::UncharacterizedCache,
            ..observation.clone()
        };
        assert!(stale.verified_structural_proof().is_none());
        let unproven = HostObservation {
            incarnation: IncarnationEvidence::Unknown,
            ..observation.clone()
        };
        assert!(unproven.verified_structural_proof().is_none());
        let invocation_only = HostObservation {
            incarnation: IncarnationEvidence::Verified {
                identity: "inc-1".into(),
                evidence_kind: EvidenceKind::NativeInvocation,
            },
            ..observation.clone()
        };
        assert!(invocation_only.verified_structural_proof().is_none());
        let coherent_bound = HostObservation {
            incarnation: IncarnationEvidence::Verified {
                identity: "inc-1".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            ..observation.clone()
        };
        assert_eq!(
            coherent_bound
                .verified_structural_proof()
                .unwrap()
                .source_spelling(),
            "coherent_enumeration"
        );
        let missing_order = HostObservation {
            observation_sequence: 0,
            ..observation.clone()
        };
        assert!(missing_order.verified_structural_proof().is_none());
        let outside_call = HostObservation {
            observed_at_mono: MonoInstant(2),
            ..observation.clone()
        };
        assert!(outside_call.verified_structural_proof().is_none());
        let occupied = HostObservation {
            occupant: Some(NativeOccupant {
                harness: Harness::Codex,
                session: NativeSessionId::new("n1"),
                execution: ExecutionId::new("e1"),
                is_top_level: true,
            }),
            ..observation
        };
        assert!(occupied.verified_structural_proof().is_some());
    }
}

#[cfg(test)]
mod contract_adapter_tests {
    use super::*;
    use crate::protocol::{
        authority::PeerIdentity, commands::ResolveSeat, results::Health, time::Cancellation,
    };

    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeClock;
    impl Clock for FakeClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(500)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    struct Adapter {
        clock: FakeClock,
        observations: AtomicUsize,
    }
    fn unsupported() -> ApiError {
        ApiError::unsupported("inert contract adapter")
    }
    fn health() -> CommandResult {
        let mut health = Health::unknown(
            "00000000-0000-4000-8000-000000000001".into(),
            "00000000-0000-4000-8000-000000000002".into(),
            "test".into(),
            1,
        );
        health.limitations.push("inert".into());
        CommandResult::Health(health)
    }
    impl Adapter {
        fn query(&self, command: &Command) -> Result<CommandResult, ApiError> {
            if matches!(command, Command::Health) {
                Ok(health())
            } else {
                Err(unsupported())
            }
        }
    }
    impl HostPort for Adapter {
        fn native_launch_capability(&self) -> NativeLaunchCapability {
            NativeLaunchCapability::Unsupported
        }
        fn observe_current_target(
            &self,
            target: &HostTargetId,
            _context: &HostCallContext,
        ) -> Result<HostObservation, ApiError> {
            let call = self.observations.fetch_add(1, Ordering::SeqCst);
            Ok(HostObservation {
                focused: false,
                target: target.clone(),
                host_boot: HostBootId::new("b1"),
                epoch: 4,
                generation: 7,
                observed_at_utc: UtcMillis(500),
                observed_at_mono: MonoInstant(100 + call as u64),
                provenance: ObservationProvenance::FreshCurrentTarget,
                occupant: (call > 0).then(|| NativeOccupant {
                    harness: Harness::Codex,
                    session: NativeSessionId::new("n1"),
                    execution: ExecutionId::new("e1"),
                    is_top_level: true,
                }),
                ui: HostUiState::Idle,
                terminal: Some(TerminalId::new("term-p1")),
                occupancy: StructuralOccupancy::Unknown,
                incarnation: IncarnationEvidence::Verified {
                    identity: "inc-1".into(),
                    evidence_kind: EvidenceKind::NativeCurrentTarget,
                },
                execution: ExecutionEvidence::Unknown,
                call_id: HostCallId::new("test-call"),
                connection_epoch: 0,
                observation_sequence: 1,
                started_at_mono: MonoInstant(100 + call as u64),
                completed_at_mono: MonoInstant(100 + call as u64),
            })
        }
        fn enumerate_targets(&self, _context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
            Err(unsupported())
        }
        fn safe_wake_target(
            &self,
            _seat: &SeatId,
            _observation: &HostObservation,
        ) -> Option<SafeWakeTarget> {
            None
        }
        fn submit_prompt(
            &self,
            _target: &SafeWakeTarget,
            _text: &str,
            _context: &HostCallContext,
        ) -> Result<PromptOutcome, ApiError> {
            Err(unsupported())
        }
        fn pane_agent_state(
            &self,
            _target: &SafeWakeTarget,
            _context: &HostCallContext,
        ) -> Result<crate::ports::AgentComposerState, ApiError> {
            Ok(crate::ports::AgentComposerState::Submitted)
        }
        fn send_submit_key(
            &self,
            _target: &SafeWakeTarget,
            _context: &HostCallContext,
        ) -> Result<(), ApiError> {
            Err(unsupported())
        }

        fn launch_native(
            &self,
            _request: NativeLaunchRequest,
            _context: &HostCallContext,
        ) -> Result<NativeLaunchOutcome, ApiError> {
            Err(unsupported())
        }
    }
    impl NotificationPort for Adapter {
        fn attempt_wake(
            &self,
            reservation: WakeReservation,
            _context: &HostCallContext,
        ) -> Result<WakeOutcome, ApiError> {
            if reservation.seat.as_str() == "s1" {
                Ok(WakeOutcome::Submitted)
            } else {
                Err(unsupported())
            }
        }
    }
    impl LocalClient for Adapter {
        crate::default_output_local_client!();
        fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            self.query(&command)
        }
    }
    impl LocalService for Adapter {
        crate::unserved_local_service_routes!(
            service_control,
            audit_service_disconnect,
            service_operation,
            handle_with_output
        );
        fn handle(
            &self,
            command: Command,
            peer: PeerIdentity,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            if peer.effective_uid() != 501 {
                return Err(unsupported());
            }
            self.query(&command)
        }
    }

    #[test]
    fn all_ports_accept_typed_adapter_calls() {
        let adapter = Adapter {
            clock: FakeClock,
            observations: AtomicUsize::new(0),
        };
        let budget = CallBudget {
            deadline: MonoInstant(200),
            cancellation: Cancellation::default(),
        };
        let context = HostCallContext {
            budget: budget.clone(),
            expected_boot: Some(HostBootId::new("b1")),
            expected_epoch: Some(4),
        };
        let request = ResolveSeat {
            target: HostTargetId::new("p1"),
            operation: OperationId::new("op1"),
        };
        let observation = adapter
            .observe_current_target(&request.target, &context)
            .unwrap();
        assert_eq!(observation.target, request.target);
        let seat = SeatId::new("s1");
        assert_eq!(adapter.clock.utc_now(), UtcMillis(500));
        let reservation = WakeReservation {
            attempt: WakeAttemptId::new("w1"),
            daemon_boot: uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
            seat: seat.clone(),
            reasons: vec!["pending".into()],
            retained_effective_delay_ms: 1000,
            lease_until: MonoInstant(200),
            retained_minimum_delay_ms: 500,
            reserved_at_utc: UtcMillis(500),
            host_boot: observation.host_boot,
            host_epoch: observation.epoch,
            target: observation.target,
            target_generation: 7,
            attention_witness: WakeAttentionWitness::from_complete(
                "i".into(),
                seat.clone(),
                7,
                true,
                false,
                None,
                1,
                true,
                LogicalAttentionFrontier::default(),
            ),
            authority: ReservedWakeAuthority::Registered {
                binding_generation: 1,
                execution: ExecutionId::new("e1"),
            },
        };
        assert_eq!(
            adapter.attempt_wake(reservation, &context).unwrap(),
            WakeOutcome::Submitted
        );
        assert!(matches!(
            LocalClient::call(&adapter, Command::Health, &budget),
            Ok(CommandResult::Health(_))
        ));
        assert!(matches!(
            adapter.handle(Command::Health, PeerIdentity::from_kernel(501), &budget),
            Ok(CommandResult::Health(_))
        ));
    }
}
