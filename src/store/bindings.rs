//! Session-keyed seat bindings (spec §1, §2). A binding is keyed by
//! (seat, harness, native session, agent id); a seat may hold many. Contract module (ht-54r.1):
//! minimal implementations against schema V32; ht-54r.3 (resolution), ht-54r.10 (liveness),
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

impl BindingRow {
    /// True unless only plain calls have touched the binding (provenance
    /// `cooperative_env_session`, which no tool-boundary check-in rewrites). A liveness-ranking
    /// fact for the same-process rule, not a delivery or authority gate.
    pub fn lifecycle_claimed(&self) -> bool {
        self.provenance != crate::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE
    }
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
/// Enroll-hook registry, run in order after the row and its cursors exist.
pub const ENROLL_HOOKS: &[(&str, EnrollHook)] = &[
    ("launch_placeholder_replace", launch_placeholder_replace),
    ("same_pid_supersede", same_pid_supersede),
];

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

/// Refresh the fingerprint of an open binding; NEVER changes liveness (spec §1, r2). It is the
/// primitive: `plain_reconnect` decides whether a plain call also reconnects. Returns whether an
/// open binding was found.
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
            // Only a changed fingerprint writes: every agent call touches its binding, and an
            // unchanged row must not wake the lanes that watch `occupant_bindings`.
            "UPDATE occupant_bindings SET liveness_pid=?2, liveness_start=?3 WHERE binding_id=?1 \
             AND (liveness_pid IS NOT ?2 OR liveness_start IS NOT ?3)",
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

/// What a disconnect hook sees: the binding after the row changed.
pub struct DisconnectEvent<'a> {
    pub binding: &'a BindingRow,
    pub reason: DisconnectReason,
    pub at: UtcMillis,
}
pub type DisconnectHook = fn(&Connection, &DisconnectEvent<'_>) -> Result<(), ApiError>;
/// Side effects of a disconnect, run inside the deciding transaction after the row changed.
/// ht-54r.23 appended ("availability_recompute", ..); ht-54r.12 appended ("mod_channel_release", ..).
/// Human bindings (`harness='human'`) are exempt from the liveness sweep (TRUST-POLICY A4): a
/// person has no process fingerprint, and the seat stays human-held until an agent replaces it;
/// ht-54r.23's worker skips them.
pub const DISCONNECT_HOOKS: &[(&str, DisconnectHook)] = &[
    ("availability_recompute", availability_recompute),
    ("mod_channel_release", mod_channel_release),
];

/// The only disconnect transition (end event, pane close, re-check, session switch, supersede all
/// come through here). Connected -> disconnected with reason and time; idempotent. Runs
/// `DISCONNECT_HOOKS` only when the row actually changed; returns whether it did. Disconnection
/// never ends a binding, moves a seat or merges seats (TRUST-POLICY). A `Superseded` disconnect
/// never triggers fast-forward: fast-forward (ht-54r.8) runs only on the resuming binding's own
/// lifecycle reconnect.
pub fn disconnect(
    tx: &Connection,
    binding: &BindingId,
    reason: DisconnectReason,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    disconnect_with_hooks(tx, binding, reason, now, DISCONNECT_HOOKS)
}

pub(crate) fn disconnect_with_hooks(
    tx: &Connection,
    binding: &BindingId,
    reason: DisconnectReason,
    now: UtcMillis,
    hooks: &[(&str, DisconnectHook)],
) -> Result<bool, ApiError> {
    let changed = tx
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=?2, disconnect_reason=?3 \
             WHERE binding_id=?1 AND liveness='connected'",
            params![binding.as_str(), now.0, reason.as_str()],
        )
        .map_err(store_error)?;
    if changed == 0 {
        return Ok(false);
    }
    if !hooks.is_empty() {
        let row = get(tx, binding)?.ok_or_else(|| corrupt("disconnected binding vanished"))?;
        let event = DisconnectEvent {
            binding: &row,
            reason,
            at: now,
        };
        for (_, hook) in hooks {
            hook(tx, &event)?;
        }
    }
    Ok(true)
}

/// What brought a binding back to connected: enrollment, a lifecycle check-in, or a plain call
/// carrying a fingerprint (spec §3: any call the binding makes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectTrigger {
    Enrollment,
    Lifecycle,
    PlainCall,
}

/// The only connected transition: disconnected -> connected on an open binding (clears
/// disconnected_at/reason); the fingerprint is recorded or refreshed (None clears a stale one: an
/// old process is not evidence about the new one). Enrollment inserts rows already connected, so
/// this serves existing rows. Returns whether liveness changed.
pub fn connect(
    tx: &Connection,
    binding: &BindingId,
    fingerprint: Option<LivenessFingerprint>,
    _trigger: ConnectTrigger,
    _now: UtcMillis,
) -> Result<bool, ApiError> {
    let (pid, start) = match fingerprint {
        Some(f) => (
            Some(i64::from(f.pid)),
            Some(to_i64(f.start, "liveness start")?),
        ),
        None => (None, None),
    };
    let changed = tx
        .execute(
            "UPDATE occupant_bindings SET liveness='connected', disconnected_at=NULL, disconnect_reason=NULL, liveness_pid=?2, liveness_start=?3 \
             WHERE binding_id=?1 AND ended_at IS NULL AND liveness='disconnected'",
            params![binding.as_str(), pid, start],
        )
        .map_err(store_error)?;
    Ok(changed > 0)
}

/// Spec §3 Presence: a seat is active iff it has an open connected binding, or its open binding is
/// a human binding and the seat is still resolved onto its target (the pane exists).
pub fn seat_active(db: &Connection, seat: &SeatId) -> Result<bool, ApiError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id \
         WHERE b.seat_id=?1 AND b.ended_at IS NULL AND \
         ((b.harness<>'human' AND b.liveness='connected') OR (b.harness='human' AND s.state='resolved')))",
        [seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Whether a seat is available or not, from the liveness of its bindings (spec §3 Presence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeatAvailability {
    Available,
    /// No open, connected, registered binding is confirmed; `since` is the latest moment a
    /// binding of the seat disconnected (the seat's creation when none ever did).
    Unavailable {
        since: UtcMillis,
    },
}

/// A seat is available iff it has at least one open, connected, registered binding that the
/// canonical observation confirms (`schema::effective_registered_availability`).
pub fn seat_availability(db: &Connection, seat: &SeatId) -> Result<SeatAvailability, ApiError> {
    if super::schema::effective_registered_availability(db, seat.as_str(), None)?.is_some() {
        return Ok(SeatAvailability::Available);
    }
    let since: i64 = db
        .query_row(
            "SELECT COALESCE((SELECT MAX(disconnected_at) FROM occupant_bindings WHERE seat_id=?1), \
             (SELECT created_at FROM seats WHERE id=?1), 0)",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(SeatAvailability::Unavailable {
        since: UtcMillis(since),
    })
}

/// Disconnect hook: when the disconnected binding was the seat's last confirmed connected
/// registered binding, the seat's unavailability episode opens (`seats.unavailability_open=1`).
/// A retired seat opens none; an already open episode is reused. Reconnect closes the episode in
/// the check-in that reconnects (`register_available`).
fn availability_recompute(db: &Connection, event: &DisconnectEvent<'_>) -> Result<(), ApiError> {
    let seat = &event.binding.seat;
    let retired: bool = db
        .query_row(
            "SELECT state='retired' FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if retired
        || super::schema::effective_registered_availability(db, seat.as_str(), None)?.is_some()
    {
        return Ok(());
    }
    super::schema::ensure_unavailability_episode(db, seat)?;
    Ok(())
}

/// Disconnect hook: the disconnected binding's mod channel must go (spec §3 Wake). The store
/// cannot reach the daemon's in-memory registry, so the hook bumps the seat's attention version
/// like any new-attention producer; the registry's observer pass then re-judges every entry of
/// the seat against its own binding and closes the disconnected one (`BindingChanged`, that
/// binding only). A human binding holds no channel.
fn mod_channel_release(db: &Connection, event: &DisconnectEvent<'_>) -> Result<(), ApiError> {
    if event.binding.harness == "human" {
        return Ok(());
    }
    super::wake::note_new_attention(db, &event.binding.seat)
}

/// A connected, open, non-human binding the liveness worker may judge, with what the same-process
/// sweep needs.
#[derive(Debug, Clone)]
pub struct LivenessCandidate {
    pub binding: BindingRow,
    pub instance: String,
    /// The binding's latest lifecycle claim (`observed_at`).
    pub claimed_at: i64,
}

/// One liveness verdict: disconnect `binding` for `reason` provided it is still connected with
/// exactly `fingerprint` (a reconnect with another process in between voids the verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivenessDecision {
    pub binding: BindingId,
    pub fingerprint: Option<LivenessFingerprint>,
    pub reason: DisconnectReason,
}

/// Connected open bindings of resolved seats the liveness check may judge. Human bindings are
/// never judged by process evidence (ht-54r.4). Unresolved and retired seats, and seats of an
/// instance under a recovery hold (open `recovery_holds` or an unclaimed baseline hold), are
/// skipped entirely: a Herdr restart never leads to a disconnect from here (the reconciler owns
/// those seats). `seat` narrows the read to one seat.
pub fn liveness_candidates(
    db: &Connection,
    seat: Option<&SeatId>,
) -> Result<Vec<LivenessCandidate>, ApiError> {
    let mut statement = db
        .prepare_cached(&format!(
            "SELECT {COLUMNS}, (SELECT instance_id FROM seats WHERE id=occupant_bindings.seat_id), observed_at \
             FROM occupant_bindings WHERE ended_at IS NULL AND liveness='connected' AND harness<>'human' \
             AND (?1 IS NULL OR seat_id=?1) \
             AND seat_id IN (SELECT s.id FROM seats s JOIN host_instances h ON h.id=s.instance_id \
                 WHERE s.state='resolved' AND COALESCE(h.baseline_hold_unclaimed,0)=0 \
                 AND NOT EXISTS(SELECT 1 FROM recovery_holds rh WHERE rh.instance_id=s.instance_id AND rh.released_at IS NULL)) \
             ORDER BY ordinal"
        ))
        .map_err(store_error)?;
    let rows = statement
        .query_map([seat.map(SeatId::as_str)], |r| {
            Ok((row(r)?, r.get::<_, String>(19)?, r.get::<_, i64>(20)?))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    rows.into_iter()
        .map(|(binding, instance, claimed_at)| {
            Ok(LivenessCandidate {
                binding: binding?,
                instance,
                claimed_at,
            })
        })
        .collect()
}

/// Applies liveness verdicts inside the caller's transaction, each through the sole disconnect
/// transition. A verdict whose binding is no longer a candidate (reconnected with another
/// fingerprint, seat now held, unresolved or retired) is dropped. Returns how many bindings
/// changed. Never ends a binding, moves a seat or merges seats.
pub fn apply_liveness(
    tx: &Connection,
    decisions: &[LivenessDecision],
    now: UtcMillis,
) -> Result<usize, ApiError> {
    if decisions.is_empty() {
        return Ok(0);
    }
    let current = liveness_candidates(tx, None)?;
    let mut changed = 0;
    for decision in decisions {
        let Some(candidate) = current.iter().find(|c| {
            c.binding.binding_id == decision.binding
                && c.binding.fingerprint == decision.fingerprint
        }) else {
            continue;
        };
        if decision.reason == DisconnectReason::Superseded
            && !is_superseded_now(&current, candidate)
        {
            continue;
        }
        if disconnect(tx, &decision.binding, decision.reason, now)? {
            changed += 1;
        }
    }
    Ok(changed)
}

/// The winner key of a same-process group of lifecycle-claimed bindings: the latest lifecycle
/// claim, ties by ordinal. The liveness worker's `service::liveness::classify` must use the same
/// ordering and the same membership filter (`lifecycle_claimed`); it carries its own copy.
fn supersede_rank(candidate: &LivenessCandidate) -> (i64, i64) {
    (candidate.claimed_at, candidate.binding.ordinal)
}

/// Re-runs the winner selection against the candidates read inside the write transaction: a
/// `Superseded` verdict was judged on an unlocked read, and the judged loser may have checked in
/// since (spec §3: "the most recent lifecycle claimer wins"). True only while `candidate` shares
/// its process with another top-level candidate that out-ranks it.
fn is_superseded_now(current: &[LivenessCandidate], candidate: &LivenessCandidate) -> bool {
    if candidate.binding.role != BindingRole::TopLevel
        || !candidate.binding.lifecycle_claimed()
        || candidate.binding.fingerprint.is_none()
    {
        return false;
    }
    let Some(winner) = current
        .iter()
        .filter(|c| {
            c.binding.role == BindingRole::TopLevel
                && c.binding.lifecycle_claimed()
                && c.instance == candidate.instance
                && c.binding.harness == candidate.binding.harness
                && c.binding.fingerprint == candidate.binding.fingerprint
        })
        .max_by_key(|c| supersede_rank(c))
    else {
        return false;
    };
    winner.binding.binding_id != candidate.binding.binding_id
}

/// Enroll hook: an enrollment *by a lifecycle check-in*, or a lifecycle check-in, of a top-level binding disconnects
/// (`Superseded`) every other open connected top-level binding of the same instance and harness
/// with the same liveness fingerprint (pid AND start: structural evidence of one process, so pid
/// reuse never matches). Most recent claimer wins. Subagents neither supersede nor are
/// superseded; a plain-call enrollment or a plain touch is not an event. Hooks run after cursor
/// seeding, so a lineage successor has copied its cursors before its predecessor is superseded.
pub fn same_pid_supersede(
    tx: &Connection,
    event: &EnrollEvent<'_>,
    now: UtcMillis,
) -> Result<(), ApiError> {
    let binding = event.binding;
    let Some(fp) = binding.fingerprint else {
        return Ok(());
    };
    // A `managed_launch` placeholder carries the started process's fingerprint but is no session:
    // it never supersedes a binding (the real session's check-in replaces it instead).
    if binding.role != BindingRole::TopLevel
        || binding.liveness != Liveness::Connected
        || binding.provenance == crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE
        || event.trigger != EnrollTrigger::Lifecycle
    {
        return Ok(());
    }
    let others = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE ended_at IS NULL AND liveness='connected' AND role='top_level' \
             AND harness=?1 AND liveness_pid=?2 AND liveness_start=?3 AND binding_id<>?4 \
             AND seat_id IN (SELECT id FROM seats WHERE instance_id=(SELECT instance_id FROM seats WHERE id=?5)) ORDER BY ordinal"
        ),
        &[
            &binding.harness,
            &i64::from(fp.pid),
            &to_i64(fp.start, "liveness start")?,
            &binding.binding_id.as_str(),
            &binding.seat.as_str(),
        ],
    )?;
    for other in &others {
        disconnect(tx, &other.binding_id, DisconnectReason::Superseded, now)?;
    }
    Ok(())
}

/// Enroll hook (TRUST-POLICY A4 `managed_launch`): the first top-level agent binding of harness
/// H that a lifecycle check-in enrolls or reconnects on seat S replaces every open
/// `managed_launch` placeholder of H on S: `disconnect(Replaced)` through the sole disconnect
/// transition, then `ended_at`. Bindings of other harnesses, real sessions and the human binding
/// are untouched; a plain call (a tool-boundary check-in enrolling an unseen session, or a touch)
/// never adopts a placeholder. The placeholder's own enrollment never replaces itself.
pub fn launch_placeholder_replace(
    tx: &Connection,
    event: &EnrollEvent<'_>,
    now: UtcMillis,
) -> Result<(), ApiError> {
    let binding = event.binding;
    if binding.role != BindingRole::TopLevel
        || binding.harness == "human"
        || binding.provenance == crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE
        || event.trigger != EnrollTrigger::Lifecycle
    {
        return Ok(());
    }
    let placeholders = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 AND ended_at IS NULL \
             AND observation_provenance=?3 AND binding_id<>?4 ORDER BY ordinal"
        ),
        &[
            &binding.seat.as_str(),
            &binding.harness,
            &crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE,
            &binding.binding_id.as_str(),
        ],
    )?;
    for placeholder in &placeholders {
        end_binding(tx, &placeholder.binding_id, DisconnectReason::Replaced, now)?;
    }
    Ok(())
}

/// Disconnects (idempotent, through the sole transition) and then ends one binding. Ending is
/// only ever done by an exclusivity or retirement rule, never on heuristic evidence.
fn end_binding(
    tx: &Connection,
    binding: &BindingId,
    reason: DisconnectReason,
    now: UtcMillis,
) -> Result<(), ApiError> {
    disconnect(tx, binding, reason, now)?;
    tx.execute(
        "UPDATE occupant_bindings SET ended_at=?2 WHERE binding_id=?1 AND ended_at IS NULL",
        params![binding.as_str(), now.0],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Human exclusivity (TRUST-POLICY A4, decision 14): an agent binding enrolled, reconnected or
/// touched on a seat ends the seat's open human binding (reason `replaced`). Returns how many
/// bindings it ended. A resume after this enrolls a fresh binding: ended rows are never reopened.
pub fn end_human_binding(
    tx: &Connection,
    seat: &SeatId,
    now: UtcMillis,
) -> Result<usize, ApiError> {
    let humans = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND harness='human' AND ended_at IS NULL ORDER BY ordinal"
        ),
        &[&seat.as_str()],
    )?;
    for human in &humans {
        end_binding(tx, &human.binding_id, DisconnectReason::Replaced, now)?;
    }
    Ok(humans.len())
}

/// A person's `me init` ends the seat's agent bindings (TRUST-POLICY A4): the disconnected ones
/// always, the connected ones only under the operator override (the caller refuses a connected
/// agent otherwise). `ended_at` is set; reason `replaced` is recorded for rows still connected.
pub fn end_agent_bindings(
    tx: &Connection,
    seat: &SeatId,
    now: UtcMillis,
) -> Result<usize, ApiError> {
    let agents = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND harness<>'human' AND ended_at IS NULL ORDER BY ordinal"
        ),
        &[&seat.as_str()],
    )?;
    for agent in &agents {
        end_binding(tx, &agent.binding_id, DisconnectReason::Replaced, now)?;
    }
    Ok(agents.len())
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

/// The seat's open top-level bindings, oldest first.
pub fn list_open_top_level(db: &Connection, seat: &SeatId) -> Result<Vec<BindingId>, ApiError> {
    Ok(query_rows(
        db,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND role='top_level' ORDER BY ordinal"
        ),
        &[&seat.as_str()],
    )?
    .into_iter()
    .map(|b| b.binding_id)
    .collect())
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

/// Decision 15 / decision 9. Human claims are exempt. `launch:` sessions are never a caller
/// (SessionClaimRequired), and so is a `plugin_context:` session. A missing fingerprint is LivenessFingerprintRequired when the claim would enroll,
/// reconnect or reopen a binding (a plain touch of an open binding needs none: `touch(None)`).
pub fn admit_agent_claim(claim: &CallerClaim, creates_or_connects: bool) -> Result<(), ApiError> {
    if claim.harness == crate::protocol::authority::Harness::Human {
        return Ok(());
    }
    let session = claim.native_session.as_str();
    let placeholder = session.is_empty()
        || session.starts_with(crate::protocol::authority::MANAGED_LAUNCH_PLACEHOLDER_PREFIX)
        || session.starts_with(PLUGIN_CONTEXT_PREFIX);
    if placeholder {
        return Err(api_error(
            ErrorCode::SessionClaimRequired,
            "agent call without a harness session id (an old hook or a pre-upgrade CLI in a \
             long-lived pane): restart the agent, or re-run `herdr-threads setup` if its hooks \
             are stale",
        ));
    }
    if creates_or_connects && claim.liveness.is_none() {
        return Err(api_error(
            ErrorCode::LivenessFingerprintRequired,
            "agent call without a liveness fingerprint",
        ));
    }
    Ok(())
}
/// Placeholder session prefix of a caller without a harness session id.
const PLUGIN_CONTEXT_PREFIX: &str = "plugin_context:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Existing {
        binding: BindingRow,
        /// True only for a lifecycle reconnect or a V32 re-adoption (the resolutions that may
        /// fast-forward); a plain-call reconnect reports false.
        reconnected: bool,
    },
    Enrolled {
        binding: BindingRow,
        first_contact: bool,
    },
    Reopened(BindingRow),
}
impl Resolution {
    pub fn binding(&self) -> &BindingRow {
        match self {
            Self::Existing { binding, .. }
            | Self::Enrolled { binding, .. }
            | Self::Reopened(binding) => binding,
        }
    }
}

pub struct ResolveRequest<'a> {
    pub claim: &'a CallerClaim,
    /// A lifecycle check-in: the only claim that (re)connects and supersedes (r2).
    pub lifecycle: bool,
    pub placement: &'a BindingPlacement,
    /// Lineage predecessor hint (spec §1), honoured only when [`validate_lineage`] holds.
    /// `seats::resolve_cooperative_with_lineage` passes it for a Hermes `parent_session_id` hint.
    pub lineage: Option<&'a BindingId>,
}

/// The open top-level binding (seat, harness, native_session, agent '') a subagent claim attaches to.
pub fn resolve_parent(
    db: &Connection,
    claim: &CallerClaim,
) -> Result<Option<BindingRow>, ApiError> {
    let mut key = BindingKey::from_claim(claim);
    key.agent_id = None;
    match_claim(db, &key)
}

/// Attach a subagent binding to its (open) parent. Attachment only; it never ends a binding.
pub fn attach_subagent(
    tx: &Connection,
    claim: &CallerClaim,
    parent: &BindingRow,
    placement: &BindingPlacement,
    now: UtcMillis,
) -> Result<EnrollOutcome, ApiError> {
    attach_subagent_with_hooks(tx, claim, parent, placement, now, ENROLL_HOOKS)
}

fn attach_subagent_with_hooks(
    tx: &Connection,
    claim: &CallerClaim,
    parent: &BindingRow,
    placement: &BindingPlacement,
    now: UtcMillis,
    hooks: &[(&str, EnrollHook)],
) -> Result<EnrollOutcome, ApiError> {
    if claim.agent_id.is_none() {
        return Err(subagent_shape_error());
    }
    enroll_with_hooks(
        tx,
        &EnrollRequest {
            key: &BindingKey::from_claim(claim),
            role: BindingRole::Subagent,
            parent: Some(&parent.binding_id),
            lineage: None,
            fingerprint: claim.liveness,
            placement,
            trigger: EnrollTrigger::PlainCall,
        },
        now,
        hooks,
    )
}

fn subagent_shape_error() -> ApiError {
    api_error(
        ErrorCode::InvalidRequest,
        "subagent claims carry an agent id".to_owned(),
    )
}

/// The claim of a Hermes delegate child (spec §1 Hermes row): the parent's claim retargeted at the
/// child session, which is also its agent id, with the subagent role.
pub fn registered_child_claim(
    parent: &CallerClaim,
    child: &NativeSessionId,
) -> Result<CallerClaim, ApiError> {
    let agent = AgentId::parse(child.as_str().to_owned()).map_err(|_| subagent_shape_error())?;
    Ok(CallerClaim {
        role: crate::protocol::authority::CallerRole::Subagent,
        native_session: child.clone(),
        agent_id: Some(agent),
        ..parent.clone()
    })
}

/// The open Hermes subagent binding registered for the child session `key` names
/// (`(seat, harness, X, agent_id = X)`), if any. Only a harness whose `BindingPolicy` registers
/// child sessions does so.
fn registered_child(db: &Connection, key: &BindingKey) -> Result<Option<BindingRow>, ApiError> {
    if !crate::harness::registry::builtins()
        .binding_policy(&key.harness)
        .registered_child_sessions
        || key.agent_id.is_some()
    {
        return Ok(None);
    }
    Ok(match_claim(
        db,
        &BindingKey {
            agent_id: AgentId::parse(key.native_session.as_str().to_owned()).ok(),
            ..key.clone()
        },
    )?
    .filter(|binding| binding.role == BindingRole::Subagent))
}

/// A subagent's stop event (spec §3): disconnects the open subagent binding `key` names. Returns
/// whether a binding changed; an unknown, ended or already disconnected binding is a no-op. It
/// never ends the binding (TRUST-POLICY).
pub fn end_event_subagent(
    tx: &Connection,
    key: &BindingKey,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    match match_claim(tx, key)? {
        Some(binding) if binding.role == BindingRole::Subagent => Ok(u32::from(disconnect(
            tx,
            &binding.binding_id,
            DisconnectReason::EndEvent,
            now,
        )?)),
        _ => Ok(0),
    }
}

/// A top-level session's end event (spec §3): disconnects the open top-level binding `key` names
/// and every open subagent binding attached to it. Returns how many changed; an unknown binding
/// is a no-op. It never ends a binding (TRUST-POLICY).
pub fn end_event_session(
    tx: &Connection,
    key: &BindingKey,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    let Some(root) = match_claim(tx, key)? else {
        return Ok(0);
    };
    if root.role != BindingRole::TopLevel {
        return Ok(0);
    }
    let mut changed = u32::from(disconnect(
        tx,
        &root.binding_id,
        DisconnectReason::EndEvent,
        now,
    )?);
    let children = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE parent_binding=?1 AND ended_at IS NULL AND liveness='connected' ORDER BY ordinal"
        ),
        &[&root.binding_id.as_str()],
    )?;
    for child in &children {
        changed += u32::from(disconnect(
            tx,
            &child.binding_id,
            DisconnectReason::EndEvent,
            now,
        )?);
    }
    Ok(changed)
}

/// Ends every open binding of the seat: connected ones go through `disconnect(reason)` first (so
/// ht-54r.10's side-effect hooks run), then `ended_at` is set. Used by unresolve here, and by
/// retirement (ht-54r.14). Human exclusivity uses `end_human_binding` / `end_agent_bindings`.
pub fn end_open_bindings(
    tx: &Connection,
    seat: &SeatId,
    reason: DisconnectReason,
    now: UtcMillis,
) -> Result<usize, ApiError> {
    let open = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL ORDER BY ordinal"
        ),
        &[&seat.as_str()],
    )?;
    for binding in &open {
        disconnect(tx, &binding.binding_id, reason, now)?;
    }
    tx.execute(
        "UPDATE occupant_bindings SET ended_at=?2 WHERE seat_id=?1 AND ended_at IS NULL",
        params![seat.as_str(), now.0],
    )
    .map_err(store_error)?;
    Ok(open.len())
}

/// Spec §1 Lineage. The predecessor must be a top_level binding on the same seat with the same
/// harness and the same process fingerprint (pid and start) as the successor; otherwise the hint
/// is ignored (fresh seed) - never trusted blindly. Enrolling successor: `resolve_claim` with
/// `ResolveRequest.lineage` seeds via SeedMode::Lineage (copy, then fill). Already-enrolled
/// successor (CLI-first, accepted limit): `link_lineage` sets lineage_from and copies with MAX,
/// so no cursor moves backwards.
pub fn validate_lineage(
    db: &Connection,
    successor_claim: &CallerClaim,
    predecessor: &BindingId,
) -> Result<bool, ApiError> {
    let Some(fp) = successor_claim.liveness else {
        return Ok(false);
    };
    if successor_claim.is_subagent() {
        return Ok(false);
    }
    let Some(prev) = get(db, predecessor)? else {
        return Ok(false);
    };
    Ok(prev.role == BindingRole::TopLevel
        && prev.seat == successor_claim.seat
        && prev.harness == successor_claim.harness.as_str()
        && prev.fingerprint == Some(fp))
}

/// Records `predecessor` as the lineage of an already-enrolled `successor` and copies the
/// predecessor's cursors with MAX (never backwards). The caller validated the pair with
/// [`validate_lineage`]. A successor keeps the first lineage recorded; a repeat only re-copies.
/// Returns the number of cursors written.
pub fn link_lineage(
    tx: &Connection,
    successor: &BindingId,
    predecessor: &BindingId,
) -> Result<usize, ApiError> {
    tx.execute(
        "UPDATE occupant_bindings SET lineage_from=?2 WHERE binding_id=?1 AND lineage_from IS NULL",
        params![successor.as_str(), predecessor.as_str()],
    )
    .map_err(store_error)?;
    read_cursors::copy_lineage(tx, predecessor, successor)
}

pub fn resolve_claim(
    tx: &Connection,
    request: &ResolveRequest<'_>,
    now: UtcMillis,
) -> Result<Resolution, ApiError> {
    resolve_claim_with_hooks(tx, request, now, ENROLL_HOOKS)
}

/// Rewrites the placement columns of an open binding from the deciding mapping.
fn refresh_placement(
    tx: &Connection,
    binding: &BindingId,
    place: &BindingPlacement,
    provenance: Option<&str>,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE occupant_bindings SET generation=?2,target_generation=?3,target_id=?4,terminal_id=?5,incarnation=?6,host_boot=?7,host_epoch=?8,execution_id=?9,observation_provenance=COALESCE(?10,observation_provenance),observed_at=?11,registered_at=?12 \
         WHERE binding_id=?1",
        params![
            binding.as_str(),
            to_i64(place.generation, "generation")?,
            to_i64(place.target_generation, "target generation")?,
            place.target,
            place.terminal,
            place.incarnation,
            place.host_boot,
            to_i64(place.host_epoch, "host epoch")?,
            place.execution,
            provenance,
            place.observed_at.0,
            place.registered_at.map(|t| t.0),
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// The binding resolution of one cooperative claim, inside the caller's deciding transaction
/// (spec §1, §2 Enrollment). The only bindings it ends are exclusivity ends (TRUST-POLICY A4): any
/// agent claim (enroll, reconnect, reopen or plain touch) ends the seat's open human binding, and
/// the enroll hooks end a launch placeholder.
pub(crate) fn resolve_claim_with_hooks(
    tx: &Connection,
    request: &ResolveRequest<'_>,
    now: UtcMillis,
    hooks: &[(&str, EnrollHook)],
) -> Result<Resolution, ApiError> {
    let resolution = resolve_claim_inner(tx, request, now, hooks)?;
    if request.claim.harness != crate::protocol::authority::Harness::Human {
        end_human_binding(tx, &request.claim.seat, now)?;
    }
    Ok(resolution)
}

/// A plain (non-lifecycle) call by an open binding: reconnect it when it is disconnected, its seat
/// is resolved and the call carries a liveness fingerprint (spec §3), else touch it. A plain
/// reconnect records the fingerprint and keeps the lifecycle claim time, placement, provenance and
/// cursors; it runs no enroll hook, so it never supersedes or replaces a placeholder. Returns
/// whether the binding reconnected.
fn plain_reconnect(
    tx: &Connection,
    row: &BindingRow,
    claim: &CallerClaim,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    if row.liveness == Liveness::Disconnected
        && claim.liveness.is_some()
        && seat_is_resolved(tx, &row.seat)?
        && connect(
            tx,
            &row.binding_id,
            claim.liveness,
            ConnectTrigger::PlainCall,
            now,
        )?
    {
        return Ok(true);
    }
    touch(tx, &row.binding_id, claim.liveness, now)?;
    Ok(false)
}

fn resolve_claim_inner(
    tx: &Connection,
    request: &ResolveRequest<'_>,
    now: UtcMillis,
    hooks: &[(&str, EnrollHook)],
) -> Result<Resolution, ApiError> {
    let claim = request.claim;
    let human = claim.harness == crate::protocol::authority::Harness::Human;
    let subagent = claim.role == crate::protocol::authority::CallerRole::Subagent;
    if claim.agent_id.is_some() != subagent {
        return Err(subagent_shape_error());
    }
    let key = BindingKey::from_claim(claim);
    // A Hermes delegate child runs `herdr-threads` under its own session id and no agent id; its
    // start event registered that session as a subagent binding of the parent (spec §1 Hermes
    // row), so the claim resolves to it (role subagent) instead of enrolling a top-level one.
    if !human
        && !subagent
        && let Some(child) = registered_child(tx, &key)?
    {
        plain_reconnect(tx, &child, claim, now)?;
        let binding = get(tx, &child.binding_id)?
            .ok_or_else(|| corrupt("touched registered child vanished"))?;
        return Ok(Resolution::Existing {
            binding,
            reconnected: false,
        });
    }
    let found = match_claim(tx, &key)?;
    // A person's binding comes only from the person's own lifecycle check-in (`me init`, with
    // its A4 guards); a plain human call never enrolls one.
    if human && !request.lifecycle && found.is_none() {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative occupant context changed".to_owned(),
        ));
    }
    let readopts = found.as_ref().is_some_and(is_migrated_disconnected);
    let creates_or_connects = request.lifecycle || found.is_none() || readopts;
    if !human {
        admit_agent_claim(claim, creates_or_connects)?;
    }
    let harness = claim.harness;
    let top_provenance = if request.lifecycle || human {
        harness.cooperative_provenance()
    } else {
        crate::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE
    };
    let sub_provenance = if crate::harness::registry::builtins()
        .binding_policy(harness.as_str())
        .hook_agent_id
    {
        crate::protocol::authority::COOPERATIVE_HOOK_AGENT_PROVENANCE
    } else {
        crate::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE
    };
    let placed = |provenance: &str| BindingPlacement {
        provenance: provenance.to_owned(),
        ..request.placement.clone()
    };
    if let Some(existing) = found {
        if !request.lifecycle && readopts {
            // Re-adoption: the plain-call reconnect of a V32-migrated binding, which also
            // refreshes its placement and seeds `Readopt` cursors (TRUST-POLICY A4). Structural
            // match: the stored session is the claim's key.
            connect(
                tx,
                &existing.binding_id,
                claim.liveness,
                ConnectTrigger::Lifecycle,
                now,
            )?;
            let provenance = (existing.role == BindingRole::TopLevel).then_some(top_provenance);
            refresh_placement(tx, &existing.binding_id, request.placement, provenance)?;
            read_cursors::seed(tx, &existing.binding_id, SeedMode::Readopt)?;
            let binding = get(tx, &existing.binding_id)?
                .ok_or_else(|| corrupt("readopted binding vanished"))?;
            run_enroll_hooks(
                tx,
                &EnrollEvent {
                    binding: &binding,
                    trigger: EnrollTrigger::PlainCall,
                    created: false,
                },
                now,
                hooks,
            )?;
            return Ok(Resolution::Existing {
                binding,
                reconnected: true,
            });
        }
        if !request.lifecycle {
            plain_reconnect(tx, &existing, claim, now)?;
            let binding = get(tx, &existing.binding_id)?
                .ok_or_else(|| corrupt("touched binding vanished"))?;
            return Ok(Resolution::Existing {
                binding,
                reconnected: false,
            });
        }
        let reconnected = existing.liveness == Liveness::Disconnected
            && connect(
                tx,
                &existing.binding_id,
                claim.liveness,
                ConnectTrigger::Lifecycle,
                now,
            )?;
        if !reconnected {
            touch(tx, &existing.binding_id, claim.liveness, now)?;
        }
        let provenance = (existing.role == BindingRole::TopLevel).then_some(top_provenance);
        refresh_placement(tx, &existing.binding_id, request.placement, provenance)?;
        let binding = get(tx, &existing.binding_id)?
            .ok_or_else(|| corrupt("reconnected binding vanished"))?;
        run_enroll_hooks(
            tx,
            &EnrollEvent {
                binding: &binding,
                trigger: EnrollTrigger::Lifecycle,
                created: false,
            },
            now,
            hooks,
        )?;
        return Ok(Resolution::Existing {
            binding,
            reconnected,
        });
    }
    // No open row. A session unresolved with its seat and seen again after the rebind reopens
    // its own binding; rows ended any other way are never reopened.
    let ended = query_rows(
        tx,
        &format!(
            "SELECT {COLUMNS} FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 AND native_session=?3 AND agent_id=?4 AND ended_at IS NOT NULL ORDER BY ordinal DESC LIMIT 1"
        ),
        &[
            &key.seat.as_str(),
            &key.harness,
            &key.native_session.as_str(),
            &key.agent_column(),
        ],
    )?
    .into_iter()
    .next();
    if let Some(ended) = ended
        && ended.disconnect_reason.as_deref() == Some(DisconnectReason::SeatUnresolved.as_str())
        && seat_is_resolved(tx, &key.seat)?
    {
        tx.execute(
            "UPDATE occupant_bindings SET ended_at=NULL WHERE binding_id=?1",
            [ended.binding_id.as_str()],
        )
        .map_err(store_error)?;
        connect(
            tx,
            &ended.binding_id,
            claim.liveness,
            ConnectTrigger::Lifecycle,
            now,
        )?;
        let provenance = if subagent { None } else { Some(top_provenance) };
        refresh_placement(tx, &ended.binding_id, request.placement, provenance)?;
        read_cursors::seed(tx, &ended.binding_id, SeedMode::Readopt)?;
        let binding =
            get(tx, &ended.binding_id)?.ok_or_else(|| corrupt("reopened binding vanished"))?;
        run_enroll_hooks(
            tx,
            &EnrollEvent {
                binding: &binding,
                trigger: if request.lifecycle {
                    EnrollTrigger::Lifecycle
                } else {
                    EnrollTrigger::PlainCall
                },
                created: false,
            },
            now,
            hooks,
        )?;
        return Ok(Resolution::Reopened(binding));
    }
    let trigger = if request.lifecycle {
        EnrollTrigger::Lifecycle
    } else {
        EnrollTrigger::PlainCall
    };
    let outcome = if subagent {
        let parent = match resolve_parent(tx, claim)? {
            Some(parent) => parent,
            None => {
                let parent_key = BindingKey {
                    agent_id: None,
                    ..key.clone()
                };
                let parent_place =
                    placed(crate::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE);
                match enroll_with_hooks(
                    tx,
                    &EnrollRequest {
                        key: &parent_key,
                        role: BindingRole::TopLevel,
                        parent: None,
                        lineage: None,
                        fingerprint: claim.liveness,
                        placement: &parent_place,
                        trigger: EnrollTrigger::PlainCall,
                    },
                    now,
                    hooks,
                )? {
                    EnrollOutcome::Enrolled { binding, .. } | EnrollOutcome::Existing(binding) => {
                        binding
                    }
                }
            }
        };
        attach_subagent_with_hooks(tx, claim, &parent, &placed(sub_provenance), now, hooks)?
    } else {
        let place = placed(top_provenance);
        enroll_with_hooks(
            tx,
            &EnrollRequest {
                key: &key,
                role: BindingRole::TopLevel,
                parent: None,
                lineage: match request.lineage {
                    Some(predecessor) if validate_lineage(tx, claim, predecessor)? => {
                        Some(predecessor)
                    }
                    _ => None,
                },
                fingerprint: claim.liveness,
                placement: &place,
                trigger,
            },
            now,
            hooks,
        )?
    };
    Ok(match outcome {
        EnrollOutcome::Enrolled {
            binding,
            first_contact,
        } => Resolution::Enrolled {
            binding,
            first_contact,
        },
        EnrollOutcome::Existing(binding) => Resolution::Existing {
            binding,
            reconnected: false,
        },
    })
}

/// A binding the V32 migration created and nothing has reconnected yet.
fn is_migrated_disconnected(binding: &BindingRow) -> bool {
    binding.migrated
        && binding.liveness == Liveness::Disconnected
        && binding.disconnect_reason.as_deref() == Some(DisconnectReason::Migrated.as_str())
}

fn seat_is_resolved(db: &Connection, seat: &SeatId) -> Result<bool, ApiError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND state='resolved')",
        [seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

#[cfg(test)]
#[path = "../../tests/store/bindings.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "../../tests/store/liveness_apply.rs"]
mod liveness_apply_tests;
