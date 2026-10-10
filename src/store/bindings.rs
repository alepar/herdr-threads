//! Session-keyed seat bindings (spec §1, §2). A binding is keyed by
//! (seat, harness, native session, agent id); a seat may hold many. Contract module (ht-54r.1):
//! minimal implementations against schema V29; ht-54r.3 (resolution), ht-54r.10 (liveness),
//! ht-54r.4 / ht-54r.10 (enroll hooks) and ht-54r.20 (lineage, receipts) build on it.
use crate::{
    protocol::{
        authority::{CallerClaim, LivenessFingerprint},
        ids::{AgentId, BindingId, NativeSessionId, SeatId},
        results::{ApiError, ErrorCode},
        time::UtcMillis,
    },
    store::{
        connection::{api_error, store_error},
        read_cursors::{self, SeedMode},
    },
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingRole {
    TopLevel,
    Subagent,
}
impl BindingRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TopLevel => "top_level",
            Self::Subagent => "subagent",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "top_level" => Some(Self::TopLevel),
            "subagent" => Some(Self::Subagent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Connected,
    Disconnected,
}
impl Liveness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "connected" => Some(Self::Connected),
            "disconnected" => Some(Self::Disconnected),
            _ => None,
        }
    }
}

/// Why a binding disconnected. A Rust vocabulary, not a SQL enum, so later beads add reasons
/// without a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    Migrated,
    ProcessExit,
    EndEvent,
    Superseded,
    SeatRetired,
    SeatUnresolved,
    Replaced,
}
impl DisconnectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Migrated => "migrated",
            Self::ProcessExit => crate::protocol::authority::LIVENESS_PROCESS_EXIT_PROVENANCE,
            Self::EndEvent => crate::protocol::authority::LIVENESS_END_EVENT_PROVENANCE,
            Self::Superseded => "superseded",
            Self::SeatRetired => "seat_retired",
            Self::SeatUnresolved => "seat_unresolved",
            Self::Replaced => "replaced",
        }
    }
}

/// The binding key (decision 3). `agent_id` None = top-level ('' in the DB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingKey {
    pub seat: SeatId,
    pub harness: String,
    pub native_session: NativeSessionId,
    pub agent_id: Option<AgentId>,
}
impl BindingKey {
    pub fn from_claim(claim: &CallerClaim) -> Self {
        Self {
            seat: claim.seat.clone(),
            harness: claim.harness.as_str().to_owned(),
            native_session: claim.native_session.clone(),
            agent_id: claim.agent_id.clone(),
        }
    }
    fn agent_column(&self) -> &str {
        self.agent_id.as_ref().map_or("", AgentId::as_str)
    }
}

/// Where the binding sits, supplied by the deciding transaction (ht-54r.3 fills it from A2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingPlacement {
    pub generation: u64,
    pub target_generation: u64,
    pub target: String,
    pub terminal: String,
    pub incarnation: String,
    pub host_boot: String,
    pub host_epoch: u64,
    pub execution: String,
    pub provenance: String,
    pub observed_at: UtcMillis,
    pub registered_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRow {
    pub ordinal: i64,
    pub binding_id: BindingId,
    pub seat: SeatId,
    pub harness: String,
    pub native_session: String,
    pub agent_id: String,
    pub role: BindingRole,
    pub parent_binding: Option<BindingId>,
    pub lineage_from: Option<BindingId>,
    pub liveness: Liveness,
    pub disconnected_at: Option<UtcMillis>,
    pub disconnect_reason: Option<String>,
    pub fingerprint: Option<LivenessFingerprint>,
    pub migrated: bool,
    pub migrated_session: Option<String>,
    pub generation: u64,
    pub provenance: String,
    pub ended_at: Option<UtcMillis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollTrigger {
    Lifecycle,
    PlainCall,
}

pub struct EnrollRequest<'a> {
    pub key: &'a BindingKey,
    pub role: BindingRole,
    pub parent: Option<&'a BindingId>,
    /// Lineage record (spec §1): the predecessor whose cursors the successor copies.
    pub lineage: Option<&'a BindingId>,
    /// Required for agents once ht-54r.3 lands (decision 9); the stub accepts None.
    pub fingerprint: Option<LivenessFingerprint>,
    pub placement: &'a BindingPlacement,
    pub trigger: EnrollTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollOutcome {
    Enrolled {
        binding: BindingRow,
        first_contact: bool,
    },
    Existing(BindingRow),
}

/// What an enroll hook sees: the binding (row and cursors already written).
pub struct EnrollEvent<'a> {
    pub binding: &'a BindingRow,
    pub trigger: EnrollTrigger,
    pub created: bool,
}
pub type EnrollHook = fn(&Connection, &EnrollEvent<'_>, UtcMillis) -> Result<(), ApiError>;
/// Enroll-hook registry, run in order after the row and its cursors exist. Later beads append:
/// ht-54r.4 `launch_placeholder_replace`, ht-54r.10 `same_pid_supersede`.
pub const ENROLL_HOOKS: &[(&str, EnrollHook)] = &[];

const COLUMNS: &str = "ordinal,binding_id,seat_id,harness,native_session,agent_id,role,parent_binding,lineage_from,liveness,disconnected_at,disconnect_reason,liveness_pid,liveness_start,migrated,migrated_session,generation,observation_provenance,ended_at";

fn corrupt(detail: &str) -> ApiError {
    api_error(ErrorCode::StoreCorrupt, detail.to_owned())
}

fn to_i64(value: u64, what: &str) -> Result<i64, ApiError> {
    i64::try_from(value)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, format!("{what} out of range")))
}

/// Rows stay raw-checked: a value outside the stored vocabulary is corruption, not a panic.
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<BindingRow, ApiError>> {
    let id: String = r.get(1)?;
    let seat: String = r.get(2)?;
    let role: String = r.get(6)?;
    let parent: Option<String> = r.get(7)?;
    let lineage: Option<String> = r.get(8)?;
    let liveness: String = r.get(9)?;
    let disconnected_at: Option<i64> = r.get(10)?;
    let pid: Option<i64> = r.get(12)?;
    let start: Option<i64> = r.get(13)?;
    let generation: i64 = r.get(16)?;
    let ended_at: Option<i64> = r.get(18)?;
    let ordinal: i64 = r.get(0)?;
    let harness: String = r.get(3)?;
    let native_session: String = r.get(4)?;
    let agent_id: String = r.get(5)?;
    let reason: Option<String> = r.get(11)?;
    let migrated: i64 = r.get(14)?;
    let migrated_session: Option<String> = r.get(15)?;
    let provenance: String = r.get(17)?;
    let build = || -> Result<BindingRow, ApiError> {
        let fingerprint = match (pid, start) {
            (Some(pid), Some(start)) => Some(LivenessFingerprint {
                pid: u32::try_from(pid).map_err(|_| corrupt("invalid binding fingerprint pid"))?,
                start: u64::try_from(start)
                    .map_err(|_| corrupt("invalid binding fingerprint start"))?,
            }),
            (None, None) => None,
            _ => return Err(corrupt("partial binding fingerprint")),
        };
        Ok(BindingRow {
            ordinal,
            binding_id: BindingId::parse(id).map_err(|_| corrupt("invalid binding id"))?,
            seat: SeatId::parse(seat).map_err(|_| corrupt("invalid binding seat"))?,
            harness,
            native_session,
            agent_id,
            role: BindingRole::parse(&role).ok_or_else(|| corrupt("invalid binding role"))?,
            parent_binding: parent
                .map(|v| BindingId::parse(v).map_err(|_| corrupt("invalid parent binding")))
                .transpose()?,
            lineage_from: lineage
                .map(|v| BindingId::parse(v).map_err(|_| corrupt("invalid lineage binding")))
                .transpose()?,
            liveness: Liveness::parse(&liveness)
                .ok_or_else(|| corrupt("invalid binding liveness"))?,
            disconnected_at: disconnected_at.map(UtcMillis),
            disconnect_reason: reason,
            fingerprint,
            migrated: migrated != 0,
            migrated_session,
            generation: u64::try_from(generation)
                .map_err(|_| corrupt("invalid binding generation"))?,
            provenance,
            ended_at: ended_at.map(UtcMillis),
        })
    };
    Ok(build())
}

fn query_rows(
    db: &Connection,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<BindingRow>, ApiError> {
    let mut statement = db.prepare_cached(sql).map_err(store_error)?;
    let rows = statement
        .query_map(args, row)
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    rows.into_iter().collect()
}

pub fn enroll(
    tx: &Connection,
    request: &EnrollRequest<'_>,
    now: UtcMillis,
) -> Result<EnrollOutcome, ApiError> {
    enroll_with_hooks(tx, request, now, ENROLL_HOOKS)
}

/// Idempotent per key inside the caller's transaction (the partial unique index backs it).
pub(crate) fn enroll_with_hooks(
    tx: &Connection,
    request: &EnrollRequest<'_>,
    now: UtcMillis,
    hooks: &[(&str, EnrollHook)],
) -> Result<EnrollOutcome, ApiError> {
    if let Some(existing) = match_claim(tx, request.key)? {
        return Ok(EnrollOutcome::Existing(existing));
    }
    let place = request.placement;
    let key = request.key;
    let (pid, start) = match request.fingerprint {
        Some(f) => (
            Some(i64::from(f.pid)),
            Some(to_i64(f.start, "liveness start")?),
        ),
        None => (None, None),
    };
    tx.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,role,parent_binding,agent_id,lineage_from,liveness_pid,liveness_start) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        params![
            key.seat.as_str(),
            to_i64(place.generation, "generation")?,
            to_i64(place.target_generation, "target generation")?,
            place.target,
            place.terminal,
            place.incarnation,
            place.host_boot,
            to_i64(place.host_epoch, "host epoch")?,
            key.harness,
            key.native_session.as_str(),
            place.execution,
            place.provenance,
            place.observed_at.0,
            place.registered_at.map(|t| t.0),
            request.role.as_str(),
            request.parent.map(BindingId::as_str),
            key.agent_column(),
            request.lineage.map(BindingId::as_str),
            pid,
            start,
        ],
    )
    .map_err(store_error)?;
    let ordinal = tx.last_insert_rowid();
    let binding = query_rows(
        tx,
        &format!("SELECT {COLUMNS} FROM occupant_bindings WHERE ordinal=?1"),
        &[&ordinal],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| corrupt("enrolled binding vanished"))?;
    let mode = match request.lineage {
        Some(p) => SeedMode::Lineage(p.clone()),
        None => SeedMode::Fresh,
    };
    let seeded = read_cursors::seed(tx, &binding.binding_id, mode)?;
    run_enroll_hooks(
        tx,
        &EnrollEvent {
            binding: &binding,
            trigger: request.trigger,
            created: true,
        },
        now,
        hooks,
    )?;
    Ok(EnrollOutcome::Enrolled {
        binding,
        first_contact: seeded.first_contact,
    })
}

/// Also called by ht-54r.3 on a lifecycle reconnect (`created: false`).
pub fn run_enroll_hooks(
    tx: &Connection,
    event: &EnrollEvent<'_>,
    now: UtcMillis,
    hooks: &[(&str, EnrollHook)],
) -> Result<(), ApiError> {
    for (_, hook) in hooks {
        hook(tx, event, now)?;
    }
    Ok(())
}

/// The open binding (ended_at IS NULL) holding this key, if any.
pub fn match_claim(db: &Connection, key: &BindingKey) -> Result<Option<BindingRow>, ApiError> {
    Ok(query_rows(
        db,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 AND native_session=?3 AND agent_id=?4 AND ended_at IS NULL"
        ),
        &[
            &key.seat.as_str(),
            &key.harness,
            &key.native_session.as_str(),
            &key.agent_column(),
        ],
    )?
    .into_iter()
    .next())
}

/// Refresh the fingerprint of an open binding; NEVER changes liveness (spec §1, r2). Returns
/// whether an open binding was found.
pub fn touch(
    tx: &Connection,
    binding: &BindingId,
    fingerprint: Option<LivenessFingerprint>,
    _now: UtcMillis,
) -> Result<bool, ApiError> {
    let found: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE binding_id=?1 AND ended_at IS NULL)",
            [binding.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if let (true, Some(f)) = (found, fingerprint) {
        tx.execute(
            "UPDATE occupant_bindings SET liveness_pid=?2, liveness_start=?3 WHERE binding_id=?1",
            params![
                binding.as_str(),
                i64::from(f.pid),
                to_i64(f.start, "liveness start")?
            ],
        )
        .map_err(store_error)?;
    }
    Ok(found)
}

/// Minimal disconnect: connected -> disconnected with reason and time. Idempotent; returns
/// whether it changed the row. ht-54r.10 turns this into the sole disconnect transition (with
/// side-effect hooks); keep the signature.
pub fn disconnect(
    tx: &Connection,
    binding: &BindingId,
    reason: DisconnectReason,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    let changed = tx
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=?2, disconnect_reason=?3 \
             WHERE binding_id=?1 AND liveness='connected'",
            params![binding.as_str(), now.0, reason.as_str()],
        )
        .map_err(store_error)?;
    Ok(changed > 0)
}

/// Open, connected bindings of the seat, in ordinal order.
pub fn list_connected(db: &Connection, seat: &SeatId) -> Result<Vec<BindingRow>, ApiError> {
    query_rows(
        db,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND liveness='connected' ORDER BY ordinal"
        ),
        &[&seat.as_str()],
    )
}

pub fn parent_of(db: &Connection, binding: &BindingId) -> Result<Option<BindingId>, ApiError> {
    let parent: Option<Option<String>> = db
        .query_row(
            "SELECT parent_binding FROM occupant_bindings WHERE binding_id=?1",
            [binding.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    parent
        .flatten()
        .map(|v| BindingId::parse(v).map_err(|_| corrupt("invalid parent binding")))
        .transpose()
}

pub fn get(db: &Connection, binding: &BindingId) -> Result<Option<BindingRow>, ApiError> {
    Ok(query_rows(
        db,
        &format!("SELECT {COLUMNS} FROM occupant_bindings WHERE binding_id=?1"),
        &[&binding.as_str()],
    )?
    .into_iter()
    .next())
}

#[cfg(test)]
#[path = "../../tests/store/bindings.rs"]
pub(crate) mod tests;
