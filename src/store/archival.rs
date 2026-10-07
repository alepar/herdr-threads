//! Bounded, conservative daemon lifecycle qualification. Host reads are supplied
//! by the worker outside the writer turn; this module performs canonical checks.
use super::{
    connection::{api_error, store_error},
    effective, schema,
};
use crate::protocol::{ids::ThreadId, results::ErrorCode, service::EventAuthor};
use crate::protocol::{results::ApiError, time::UtcMillis};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

/// Private certificates associate parser evidence with canonical bindings only;
/// they do not attest native execution or grant accountable authority (TRUST A3).
pub const CANONICAL_BINDING_COMPOSER: &str = "canonical_binding_composer";
fn eligible_composer(
    registry: &crate::harness::registry::Registry,
    harness: &str,
) -> Option<crate::harness::registry::AgentHarnessId> {
    registry.agent(harness).ok().filter(|id| {
        registry
            .by_id(*id)
            .ok()
            .and_then(|r| r.composer_policy())
            .is_some()
    })
}

pub const DEFAULT_AFTER_MS: u64 = 3_600_000;
pub const CADENCE_MS: i64 = 60_000;
pub const MAX_GAP_MS: i64 = 120_000;
pub const PAGE: usize = 32;

#[derive(Debug, Clone)]
pub struct Runtime {
    pub boot: String,
    pub mono: i64,
    pub utc: UtcMillis,
    pub after_ms: i64,
    pub host_generation: i64,
    pub coherent: bool,
    /// Latest permitted deciding instant for the worker's coherent host view.
    pub valid_until_mono: Option<i64>,
    /// Complete, revalidated readonly legacy-source coverage; None vetoes.
    pub legacy_source: Option<String>,
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub archived: Vec<String>,
    pub visited: usize,
    pub has_more: bool,
}

/// Each call owns one bounded deciding transaction. The caller must already
/// hold its fair background writer turn and install the store VM budget.
pub fn advance(db: &Connection, instance: &str, rt: &Runtime) -> Result<Progress, ApiError> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate).map_err(store_error)?;
    let result = advance_in(&tx, instance, rt)?;
    tx.commit().map_err(store_error)?;
    Ok(result)
}
pub(crate) fn advance_in(
    tx: &Transaction<'_>,
    instance: &str,
    rt: &Runtime,
) -> Result<Progress, ApiError> {
    advance_in_with_registry(tx, instance, rt, crate::harness::registry::builtins())
}
pub(crate) fn advance_in_with_registry(
    tx: &Transaction<'_>,
    instance: &str,
    rt: &Runtime,
    registry: &crate::harness::registry::Registry,
) -> Result<Progress, ApiError> {
    if rt.mono < 0 || rt.after_ms < 0 || rt.after_ms > i64::MAX - MAX_GAP_MS {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid archival timing",
        ));
    }
    let epoch = reconcile_runtime(tx, instance, rt)?;
    let mut progress = Progress::default();
    if rt.after_ms == 0 {
        return Ok(progress);
    }
    progress.has_more = seed(tx, instance, rt, epoch)?;
    if !rt.coherent || rt.legacy_source.is_none() {
        return Ok(progress);
    }
    if !validate_snapshot_page(tx, instance, rt)? {
        progress.has_more = true;
        return Ok(progress);
    }
    // One candidate per turn; large channels continue through indexed cursors.
    let candidate: Option<String> = tx.query_row(
        "SELECT thread_id FROM channel_archival WHERE instance_id=?1 AND enabled=1 AND due_mono<=?2 ORDER BY due_mono,thread_id LIMIT 1",
        params![instance,rt.mono], |r|r.get(0)).optional().map_err(store_error)?;
    if let Some(thread) = candidate {
        progress.visited = advance_channel(
            tx,
            instance,
            &thread,
            rt,
            epoch,
            &mut progress.archived,
            registry,
        )?;
        progress.has_more |= tx.query_row("SELECT EXISTS(SELECT 1 FROM channel_archival WHERE instance_id=?1 AND enabled=1 AND due_mono<=?2)",params![instance,rt.mono],|r|r.get::<_,bool>(0)).map_err(store_error)?;
    }
    Ok(progress)
}

fn reconcile_runtime(tx: &Transaction<'_>, instance: &str, rt: &Runtime) -> Result<i64, ApiError> {
    tx.execute("INSERT OR IGNORE INTO archival_instances(instance_id,runtime_boot,policy_ms) VALUES(?1,?2,?3)",params![instance,rt.boot,rt.after_ms]).map_err(store_error)?;
    let (boot,epoch,mono,utc,policy,generation,coherent): (String,i64,Option<i64>,Option<i64>,i64,i64,Option<i64>) = tx.query_row(
        "SELECT runtime_boot,evidence_epoch,last_mono,last_utc,policy_ms,host_generation,coherent_since FROM archival_instances WHERE instance_id=?1",[instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(store_error)?;
    let gap = mono.map(|old| rt.mono.saturating_sub(old));
    let wall_delta = utc.map(|old| rt.utc.0.saturating_sub(old));
    let discontinuity = boot != rt.boot
        || policy != rt.after_ms
        || generation != rt.host_generation
        || gap.is_some_and(|g| !(0..=MAX_GAP_MS).contains(&g))
        || gap
            .zip(wall_delta)
            .is_some_and(|(m, w)| m.abs_diff(w) > 1_000)
        || (!rt.coherent && coherent.is_some());
    let epoch = epoch + i64::from(discontinuity);
    // Discovery high-water marks are not qualification evidence. Ordinary host
    // uncertainty/recovery must not rescan retained history. Due channels and
    // active samples reject stale evidence epochs lazily in their own turns.
    // A new/backward monotonic clock does require rebuilding old due times.
    let reset_discovery = boot != rt.boot || gap.is_some_and(|g| g < 0);
    tx.execute("UPDATE archival_instances SET runtime_boot=?2,evidence_epoch=?3,policy_ms=?4,last_mono=?5,last_utc=?6,host_generation=?7,coherent_since=?8,bootstrap_veto=?9,bootstrap_source=?10,seed_cursor=CASE WHEN ?12 THEN 0 ELSE seed_cursor END,seat_seed_cursor=CASE WHEN ?12 THEN 0 ELSE seat_seed_cursor END,snapshot_checked=CASE WHEN ?11 THEN 0 ELSE snapshot_checked END WHERE instance_id=?1",
        params![instance,rt.boot,epoch,rt.after_ms,rt.mono,rt.utc.0,rt.host_generation,
            if rt.coherent {Some(if discontinuity {rt.mono} else {coherent.unwrap_or(rt.mono)})} else {None},
            rt.legacy_source.is_none(),rt.legacy_source,discontinuity,reset_discovery]).map_err(store_error)?;
    Ok(epoch)
}

fn seed(tx: &Transaction<'_>, instance: &str, rt: &Runtime, epoch: i64) -> Result<bool, ApiError> {
    let (cursor, seat_cursor): (i64, i64) = tx
        .query_row(
            "SELECT seed_cursor,seat_seed_cursor FROM archival_instances WHERE instance_id=?1",
            [instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    let mut statement=tx.prepare("SELECT ordinal,id,archived FROM threads WHERE instance_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![instance, cursor, PAGE as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    for (ordinal, thread, archived) in &rows {
        tx.execute("INSERT OR IGNORE INTO channel_archival(thread_id,instance_id,enabled) VALUES(?1,?2,?3)",params![thread,instance,!archived]).map_err(store_error)?;
        tx.execute("UPDATE channel_archival SET runtime_boot=?2,evidence_epoch=?3,quiet_mono=?4,quiet_utc=?5,due_mono=?6,scan_revision=NULL,scan_phase=0 WHERE thread_id=?1 AND (runtime_boot IS NOT ?2 OR evidence_epoch IS NOT ?3)",params![thread,rt.boot,epoch,rt.mono,rt.utc.0,rt.mono.saturating_add(rt.after_ms)]).map_err(store_error)?;
        tx.execute(
            "UPDATE archival_instances SET seed_cursor=?2 WHERE instance_id=?1",
            params![instance, ordinal],
        )
        .map_err(store_error)?;
    }
    let mut statement=tx.prepare("SELECT ordinal,id FROM seats WHERE instance_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(store_error)?;
    let seats = statement
        .query_map(params![instance, seat_cursor, PAGE as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    for (ordinal, seat) in &seats {
        tx.execute(
            "INSERT OR IGNORE INTO seat_archival(seat_id,instance_id,next_mono) VALUES(?1,?2,0)",
            params![seat, instance],
        )
        .map_err(store_error)?;
        tx.execute("UPDATE seat_archival SET runtime_boot=?2,evidence_epoch=?3,idle_mono=NULL,samples=0,last_mono=NULL,observation_sequence=NULL,connection_epoch=NULL,next_mono=CASE WHEN next_mono IS NULL THEN NULL ELSE 0 END WHERE seat_id=?1 AND (runtime_boot IS NOT ?2 OR evidence_epoch IS NOT ?3)",params![seat,rt.boot,epoch]).map_err(store_error)?;
        tx.execute(
            "UPDATE archival_instances SET seat_seed_cursor=?2 WHERE instance_id=?1",
            params![instance, ordinal],
        )
        .map_err(store_error)?;
    }
    Ok(rows.len() == PAGE || seats.len() == PAGE)
}

fn defer(tx: &Transaction<'_>, thread: &str, rt: &Runtime) -> Result<(), ApiError> {
    tx.execute("UPDATE channel_archival SET scan_revision=NULL,scan_phase=0,scan_cursor=0,scan_key='',due_mono=?2 WHERE thread_id=?1",params![thread,rt.mono.saturating_add(CADENCE_MS)]).map_err(store_error)?;
    Ok(())
}
fn cheap_blocker(
    tx: &Connection,
    instance: &str,
    thread: &str,
    now: i64,
) -> Result<bool, ApiError> {
    tx.query_row("SELECT EXISTS(SELECT 1 FROM threads WHERE id=?2 AND (archived=1 OR managed_owner_author_id IS NOT NULL))
 OR EXISTS(SELECT 1 FROM requirement_episodes WHERE thread_id=?2 AND state IN ('pending','accepted'))
 OR EXISTS(SELECT 1 FROM catch_up WHERE thread_id=?2 AND state='active')
 OR EXISTS(SELECT 1 FROM summary_jobs WHERE thread_id=?2 AND block_id IS NULL AND lease_token IS NOT NULL AND lease_until>?3)
 OR EXISTS(SELECT 1 FROM channel_handoff_fences WHERE thread_id=?2 AND state='live')
 OR EXISTS(SELECT 1 FROM channel_handoff_fences WHERE instance_id=?1 AND thread_id IS NULL AND state='live')",params![instance,thread,now],|r|r.get(0)).map_err(store_error)
}

fn advance_channel(
    tx: &Transaction<'_>,
    instance: &str,
    thread: &str,
    rt: &Runtime,
    epoch: i64,
    archived: &mut Vec<String>,
    registry: &crate::harness::registry::Registry,
) -> Result<usize, ApiError> {
    type ChannelColumns = (
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        i64,
        i64,
        Option<i64>,
    );
    let (quiet,boot,saved_epoch,scan_revision,phase,cursor,expiry):ChannelColumns=tx.query_row("SELECT quiet_mono,runtime_boot,evidence_epoch,scan_revision,scan_phase,scan_cursor,scan_expiry FROM channel_archival WHERE thread_id=?1",[thread],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(store_error)?;
    if quiet.is_none() || boot.as_deref() != Some(rt.boot.as_str()) || saved_epoch != Some(epoch) {
        tx.execute("UPDATE channel_archival SET quiet_mono=?2,quiet_utc=?3,runtime_boot=?4,evidence_epoch=?5,due_mono=?6,scan_revision=NULL WHERE thread_id=?1",params![thread,rt.mono,rt.utc.0,rt.boot,epoch,rt.mono.saturating_add(rt.after_ms)]).map_err(store_error)?;
        return Ok(1);
    }
    let quiet = quiet.unwrap();
    let coherent_since: Option<i64> = tx
        .query_row(
            "SELECT coherent_since FROM archival_instances WHERE instance_id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if rt.mono.saturating_sub(quiet) < rt.after_ms
        || coherent_since.is_none_or(|since| rt.mono.saturating_sub(since) < rt.after_ms)
        || cheap_blocker(tx, instance, thread, rt.utc.0)?
    {
        defer(tx, thread, rt)?;
        return Ok(1);
    }
    let revision: i64 = tx
        .query_row(
            "SELECT mutation_revision FROM archival_instances WHERE instance_id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if scan_revision != Some(revision) || expiry.is_some_and(|e| e < rt.mono) {
        tx.execute("UPDATE channel_archival SET scan_revision=?2,scan_phase=0,scan_cursor=0,scan_key='',scan_expiry=NULL,receipt_physical_after=0,receipt_manifest_after=0,receipt_physical_high=NULL,receipt_manifest_high=NULL,receipt_next_manifest=0 WHERE thread_id=?1",params![thread,revision]).map_err(store_error)?;
        return Ok(1);
    }
    let (visited, exhausted, blocked) = match phase {
        0 => scan_members(tx, instance, thread, rt, epoch, cursor, registry)?,
        1 => scan_invitations(tx, thread, cursor)?,
        2 => scan_receipts(tx, thread)?,
        3 => scan_preparations(tx, thread)?,
        4 => scan_service_preparations(tx, thread, cursor)?,
        _ => {
            // Every bounded source was exhausted under this exact revision.
            // No host I/O, fanout or final membership rescan in this transaction.
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM archival_instances WHERE instance_id=?1 AND runtime_boot=?2 AND evidence_epoch=?3 AND mutation_revision=?4 AND bootstrap_veto=0 AND bootstrap_source=?5 AND policy_ms=?6)",params![instance,rt.boot,epoch,revision,rt.legacy_source,rt.after_ms],|r|r.get(0)).map_err(store_error)?;
            if !valid {
                defer(tx, thread, rt)?;
                return Ok(1);
            }
            tx.execute("UPDATE threads SET archived=1,updated_at=?2,directory_revision=directory_revision+1 WHERE id=?1 AND archived=0",params![thread,rt.utc.0]).map_err(store_error)?;
            let id = ThreadId::new(thread);
            let payload=serde_json::json!({"action":"automatic_archive","provenance":"daemon_lifecycle","reason":"quiet_and_no_protected_work","policy_version":1,"after_ms":rt.after_ms}).to_string();
            schema::append_attributed_event_once(
                tx,
                schema::EventInput {
                    thread: &id,
                    key: &format!("automatic_archive:{thread}:{}:{revision}", rt.boot),
                    kind: "info",
                    payload_json: &payload,
                    decision_at: rt.utc,
                    source_message: None,
                    source_invitation: None,
                },
                EventAuthor::BuiltIn,
            )?;
            schema::bump_filter_revision(tx, instance, "directory", "all")?;
            archived.push(thread.into());
            return Ok(1);
        }
    };
    if blocked {
        defer(tx, thread, rt)?;
    } else if exhausted {
        tx.execute("UPDATE channel_archival SET scan_phase=scan_phase+1,scan_cursor=0,scan_key='' WHERE thread_id=?1",[thread]).map_err(store_error)?;
    }
    Ok(visited)
}

fn scan_members(
    tx: &Transaction<'_>,
    instance: &str,
    thread: &str,
    rt: &Runtime,
    epoch: i64,
    cursor: i64,
    registry: &crate::harness::registry::Registry,
) -> Result<(usize, bool, bool), ApiError> {
    let mut statement=tx.prepare("SELECT ordinal,seat_id FROM memberships WHERE thread_id=?1 AND state='joined' AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![thread, cursor, PAGE as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    for (ordinal, seat) in &rows {
        let qualified:Option<(i64,String)>=tx.query_row("SELECT a.last_mono,b.harness FROM seat_archival a JOIN seats s ON s.id=a.seat_id JOIN occupant_bindings b ON b.seat_id=s.id AND b.ended_at IS NULL JOIN host_instances h ON h.id=s.instance_id WHERE a.seat_id=?1 AND s.instance_id=?2 AND s.state='resolved' AND s.generation=a.binding_generation AND s.target_generation=a.target_generation AND s.target_id=a.target_id AND b.generation=a.binding_generation AND b.registered_at IS NOT NULL AND b.harness=a.harness AND b.observation_provenance='cooperative_top_level' AND b.native_session=a.native_session AND b.execution_id=a.execution AND h.host_boot=a.host_boot AND h.host_epoch=a.host_epoch AND a.runtime_boot=?3 AND a.evidence_epoch=?4 AND a.samples>=2 AND a.idle_mono<=?5 AND a.last_mono>=?6 AND a.last_mono<=?7 AND NOT EXISTS(SELECT 1 FROM recovery_holds rh WHERE rh.instance_id=s.instance_id AND rh.target_id=s.target_id AND rh.released_at IS NULL)",params![seat,instance,rt.boot,epoch,rt.mono.saturating_sub(rt.after_ms),rt.mono.saturating_sub(MAX_GAP_MS),rt.mono],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
        let Some((last, harness)) = qualified else {
            return Ok((rows.len(), false, true));
        };
        if eligible_composer(registry, &harness).is_none() {
            return Ok((rows.len(), false, true));
        }
        // Structural snapshots may supersede an inspected current-target sample;
        // they cannot grant idle evidence, but a conflicting canonical identity
        // or newer nonidle read must invalidate it.
        if !sample_still_current(tx, instance, seat)? {
            return Ok((rows.len(), false, true));
        }
        tx.execute("UPDATE channel_archival SET scan_cursor=?2,scan_expiry=MIN(COALESCE(scan_expiry,?3),?3) WHERE thread_id=?1",params![thread,ordinal,last.saturating_add(MAX_GAP_MS)]).map_err(store_error)?;
    }
    Ok((rows.len(), rows.len() < PAGE, false))
}
fn sample_still_current(tx: &Connection, instance: &str, seat: &str) -> Result<bool, ApiError> {
    let (target,boot,epoch,generation,terminal,incarnation,sequence):(String,String,i64,i64,String,String,i64)=tx.query_row("SELECT target_id,host_boot,host_epoch,target_generation,terminal,incarnation,observation_sequence FROM seat_archival WHERE seat_id=?1",[seat],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(store_error)?;
    Ok(
        effective::effective_observation(tx, instance, &target)?.is_some_and(|o| {
            o.host_boot == boot
                && o.epoch == epoch
                && o.structural_generation == generation
                && o.terminal_id.as_deref() == Some(&terminal)
                && o.incarnation.as_deref() == Some(&incarnation)
                && (o.observation_sequence <= sequence
                    || o.ui_state == "idle"
                    || (o.source == effective::EffectiveObservationSource::Published
                        && o.ui_state == "unknown"))
        }),
    )
}
fn scan_invitations(
    tx: &Transaction<'_>,
    thread: &str,
    cursor: i64,
) -> Result<(usize, bool, bool), ApiError> {
    let mut statement=tx.prepare("SELECT d.ordinal,s.state FROM digest_pending_invitations d JOIN seats s ON s.id=d.seat_id WHERE d.thread_id=?1 AND d.ordinal>?2 ORDER BY d.ordinal LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![thread, cursor, PAGE as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    if let Some((ordinal, _)) = rows.last() {
        tx.execute(
            "UPDATE channel_archival SET scan_cursor=?2 WHERE thread_id=?1",
            params![thread, ordinal],
        )
        .map_err(store_error)?;
    }
    Ok((
        rows.len(),
        rows.len() < PAGE,
        rows.iter().any(|(_, state)| state != "retired"),
    ))
}
fn scan_receipts(tx: &Transaction<'_>, thread: &str) -> Result<(usize, bool, bool), ApiError> {
    let saved=tx.query_row("SELECT receipt_physical_after,receipt_manifest_after,receipt_physical_high,receipt_manifest_high,receipt_next_manifest FROM channel_archival WHERE thread_id=?1",[thread],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<i64>>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,bool>(4)?))).map_err(store_error)?;
    let position = saved
        .2
        .zip(saved.3)
        .map(
            |(physical_high_water, manifest_high_water)| effective::ReceiptScanPosition {
                physical_after: saved.0,
                manifest_after: saved.1,
                physical_high_water,
                manifest_high_water,
                next_manifest: saved.4,
            },
        );
    let page = effective::scan_effective_receipts(
        tx,
        &effective::ReceiptScanScope::Thread(thread.into()),
        position,
        PAGE as u16,
    )?;
    let p = page.position;
    tx.execute("UPDATE channel_archival SET receipt_physical_after=?2,receipt_manifest_after=?3,receipt_physical_high=?4,receipt_manifest_high=?5,receipt_next_manifest=?6 WHERE thread_id=?1",params![thread,p.physical_after,p.manifest_after,p.physical_high_water,p.manifest_high_water,p.next_manifest]).map_err(store_error)?;
    Ok((
        usize::from(page.visited),
        !page.has_more,
        page.items
            .iter()
            .any(|r| r.state == effective::EffectiveReceiptState::Pending),
    ))
}
fn scan_preparations(tx: &Transaction<'_>, thread: &str) -> Result<(usize, bool, bool), ApiError> {
    let key: String = tx
        .query_row(
            "SELECT scan_key FROM channel_archival WHERE thread_id=?1",
            [thread],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    // The status is a separate phase, retaining index order within each state.
    let mut statement=tx.prepare("SELECT id,status,EXISTS(SELECT 1 FROM send_manifests m WHERE m.preparation_id=p.id) FROM send_preparations p WHERE thread_id=?1 AND id>?2 ORDER BY id LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![thread, key, PAGE as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    if let Some((id, _, _)) = rows.last() {
        tx.execute(
            "UPDATE channel_archival SET scan_key=?2 WHERE thread_id=?1",
            params![thread, id],
        )
        .map_err(store_error)?;
    }
    Ok((
        rows.len(),
        rows.len() < PAGE,
        rows.iter()
            .any(|(_, status, committed)| status != "discarded" && !committed),
    ))
}
fn scan_service_preparations(
    tx: &Transaction<'_>,
    thread: &str,
    cursor: i64,
) -> Result<(usize, bool, bool), ApiError> {
    let mut statement=tx.prepare("SELECT ordinal,status FROM service_notification_preparations WHERE thread_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![thread, cursor, PAGE as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    if let Some((ordinal, _)) = rows.last() {
        tx.execute(
            "UPDATE channel_archival SET scan_cursor=?2 WHERE thread_id=?1",
            params![thread, ordinal],
        )
        .map_err(store_error)?;
    }
    Ok((
        rows.len(),
        rows.len() < PAGE,
        rows.iter()
            .any(|(_, status)| status == "building" || status == "sealed"),
    ))
}

/// An explicit producer contract: only the archival HostPort method may return
/// this composer-aware observation. Ordinary reads and snapshots are separate.
pub use crate::ports::ComposerObservation;

#[derive(Debug, Clone)]
pub struct ObservationTicket {
    claim: crate::protocol::authority::CallerClaim,
    activity_revision: i64,
    runtime_boot: String,
    evidence_epoch: i64,
    host_boot: crate::protocol::ids::HostBootId,
    host_epoch: u64,
}
impl ObservationTicket {
    pub fn host_context(
        &self,
        budget: crate::protocol::time::CallBudget,
    ) -> crate::ports::HostCallContext {
        crate::ports::HostCallContext {
            budget,
            expected_boot: Some(self.host_boot.clone()),
            expected_epoch: Some(self.host_epoch),
        }
    }
    pub fn target(&self) -> &crate::protocol::ids::HostTargetId {
        &self.claim.target
    }
    pub fn seat(&self) -> &str {
        self.claim.seat.as_str()
    }
}
#[derive(Debug)]
pub struct ObservationWork {
    pub ticket: Option<ObservationTicket>,
    pub has_more: bool,
}
/// Capture the exact accountable binding and activity epoch before host I/O.
pub fn observation_ticket(
    db: &Connection,
    instance: &str,
    seat: &str,
    rt: &Runtime,
) -> Result<Option<ObservationTicket>, ApiError> {
    observation_ticket_with_registry(db, instance, seat, rt, crate::harness::registry::builtins())
}
pub(crate) fn observation_ticket_with_registry(
    db: &Connection,
    instance: &str,
    seat: &str,
    rt: &Runtime,
    registry: &crate::harness::registry::Registry,
) -> Result<Option<ObservationTicket>, ApiError> {
    use crate::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        ids::*,
    };
    let row=db.query_row("SELECT s.target_id,b.generation,b.harness,b.native_session,b.execution_id,a.activity_revision,ai.runtime_boot,ai.evidence_epoch FROM seats s JOIN seat_archival a ON a.seat_id=s.id JOIN archival_instances ai ON ai.instance_id=s.instance_id JOIN occupant_bindings b ON b.seat_id=s.id AND b.ended_at IS NULL WHERE s.id=?1 AND s.instance_id=?2 AND s.state='resolved' AND b.registered_at IS NOT NULL AND b.observation_provenance='cooperative_top_level'",params![seat,instance],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?,r.get::<_,String>(6)?,r.get::<_,i64>(7)?))).optional().map_err(store_error)?;
    let Some((
        target,
        generation,
        harness,
        session,
        execution,
        activity_revision,
        runtime_boot,
        evidence_epoch,
    )) = row
    else {
        return Ok(None);
    };
    let Some(agent) = eligible_composer(registry, &harness) else {
        return Ok(None);
    };
    let claim = CallerClaim {
        instance: instance.into(),
        seat: SeatId::new(seat),
        binding_generation: generation as u64,
        role: CallerRole::TopLevel,
        harness: Harness::Agent(agent),
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new(execution),
        target: HostTargetId::new(target),
    };
    if runtime_boot != rt.boot {
        return Ok(None);
    }
    let mapping = match super::seats::cooperative_mapping(db, &claim, None) {
        Ok(mapping) => mapping,
        Err(_) => return Ok(None),
    };
    Ok(Some(ObservationTicket {
        host_boot: HostBootId::new(mapping.boot),
        host_epoch: mapping.epoch,
        claim,
        activity_revision,
        runtime_boot,
        evidence_epoch,
    }))
}

/// Reserve bounded observation work. Retained inactive seats are parked outside
/// the due index until a canonical seat/binding/membership change queues them.
/// Temporarily uncertain active seats remain scheduled; uncertainty is never idle.
pub fn next_observation(
    db: &Connection,
    instance: &str,
    rt: &Runtime,
) -> Result<Option<ObservationTicket>, ApiError> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate).map_err(store_error)?;
    let result = next_observation_in(&tx, instance, rt)?;
    tx.commit().map_err(store_error)?;
    Ok(result)
}
pub(crate) fn next_observation_in(
    tx: &Transaction<'_>,
    instance: &str,
    rt: &Runtime,
) -> Result<Option<ObservationTicket>, ApiError> {
    next_observation_in_with_registry(tx, instance, rt, crate::harness::registry::builtins())
}
pub(crate) fn next_observation_in_with_registry(
    tx: &Transaction<'_>,
    instance: &str,
    rt: &Runtime,
    registry: &crate::harness::registry::Registry,
) -> Result<Option<ObservationTicket>, ApiError> {
    let mut statement=tx.prepare("SELECT seat_id FROM seat_archival WHERE instance_id=?1 AND next_mono<=?2 ORDER BY next_mono,seat_id LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![instance, rt.mono, PAGE as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    drop(statement);
    let mut selected = None;
    for seat in rows {
        tx.execute(
            "UPDATE seat_archival SET next_mono=?2 WHERE seat_id=?1",
            params![seat, rt.mono.saturating_add(CADENCE_MS)],
        )
        .map_err(store_error)?;
        let active_harness:Option<String>=tx.query_row("SELECT b.harness FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.ended_at IS NULL WHERE s.id=?1 AND s.state='resolved' AND b.registered_at IS NOT NULL AND b.observation_provenance='cooperative_top_level' AND EXISTS(SELECT 1 FROM memberships INDEXED BY memberships_archival_joined_seat WHERE seat_id=s.id AND state='joined')",[&seat],|r|r.get(0)).optional().map_err(store_error)?;
        let active = active_harness
            .as_deref()
            .is_some_and(|h| eligible_composer(registry, h).is_some());
        if active {
            selected = observation_ticket_with_registry(tx, instance, &seat, rt, registry)?;
        }
        if selected.is_some() {
            break;
        }
        invalidate_sample(tx, &seat, rt)?;
        if !active {
            tx.execute(
                "UPDATE seat_archival SET next_mono=NULL WHERE seat_id=?1",
                [&seat],
            )
            .map_err(store_error)?;
        }
    }
    Ok(selected)
}

fn invalidate_sample(tx: &Connection, seat: &str, rt: &Runtime) -> Result<(), ApiError> {
    tx.execute("UPDATE archival_instances SET mutation_revision=mutation_revision+1 WHERE instance_id=(SELECT instance_id FROM seat_archival WHERE seat_id=?1 AND idle_mono IS NOT NULL)",[seat]).map_err(store_error)?;
    tx.execute(
        "UPDATE seat_archival SET idle_mono=NULL,samples=0,next_mono=?2 WHERE seat_id=?1",
        params![seat, rt.mono.saturating_add(CADENCE_MS)],
    )
    .map_err(store_error)?;
    Ok(())
}

/// A delayed positive read cannot erase an intervening action or binding change.
/// Invalid evidence resets the interval and invalidates any in-progress scan.
pub fn record_sample(
    db: &mut Connection,
    ticket: &ObservationTicket,
    rt: &Runtime,
    sample: &ComposerObservation,
) -> Result<bool, ApiError> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let result = record_sample_in(&tx, ticket, rt, sample)?;
    tx.commit().map_err(store_error)?;
    Ok(result)
}
pub(crate) fn record_sample_in(
    tx: &Transaction<'_>,
    ticket: &ObservationTicket,
    rt: &Runtime,
    sample: &ComposerObservation,
) -> Result<bool, ApiError> {
    record_sample_in_with_registry(tx, ticket, rt, sample, crate::harness::registry::builtins())
}
pub(crate) fn record_sample_in_with_registry(
    tx: &Transaction<'_>,
    ticket: &ObservationTicket,
    rt: &Runtime,
    sample: &ComposerObservation,
    registry: &crate::harness::registry::Registry,
) -> Result<bool, ApiError> {
    let claim = &ticket.claim;
    let epoch = reconcile_runtime(tx, &claim.instance, rt)?;
    let observation = &sample.0;
    let activity: i64 = tx
        .query_row(
            "SELECT activity_revision FROM seat_archival WHERE seat_id=?1",
            [claim.seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let proof = observation.verified_structural_proof();
    let mapping = super::seats::cooperative_mapping(tx, claim, None).ok();
    let current = effective::effective_observation(tx, &claim.instance, claim.target.as_str())?;
    let positive = rt.coherent
        && ticket.runtime_boot == rt.boot
        && ticket.evidence_epoch == epoch
        && ticket.activity_revision == activity
        && observation.ui == crate::ports::HostUiState::Idle
        && eligible_composer(registry,claim.harness.as_str()).is_some_and(|agent| {
            sample.1.as_ref().is_some_and(|e| {
                e.parser==agent && e.classification==crate::ports::ComposerClassification::Empty
                && e.basis==crate::ports::ComposerEvidenceBasis::RegisteredHostKindComposerRead
                // Exact static alias membership agrees with the real producer.
                && registry.by_host_kind(&e.reported_host_kind).is_some_and(|r|r.metadata().id==agent.as_str())
            })
        })
        && observation.occupant.as_ref().is_none_or(|occupant|occupant.harness==claim.harness)
        && tx.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND generation=?2 AND ended_at IS NULL AND registered_at IS NOT NULL AND observation_provenance='cooperative_top_level')",params![claim.seat.as_str(),claim.binding_generation as i64],|r|r.get::<_,bool>(0)).map_err(store_error)?
        && observation.completed_at_mono.0 <= rt.mono as u64
        && observation.started_at_mono.0 >= rt.mono.saturating_sub(5_000).max(0) as u64
        && proof.as_ref().zip(mapping.as_ref()).is_some_and(|(p, m)| {
            p.target() == &claim.target
                && p.host_boot().as_str() == m.boot
                && p.host_epoch() == m.epoch
                && p.target_generation() == m.revision
                && Some(p.terminal().as_str()) == m.terminal.as_deref()
                && Some(p.incarnation()) == m.incarnation.as_deref()
        })
        && current
            .as_ref()
            .is_some_and(|o| o.observation_sequence <= observation.observation_sequence as i64);
    if !positive {
        invalidate_sample(tx, claim.seat.as_str(), rt)?;
        return Ok(false);
    }
    let proof = proof.unwrap();
    type SampleColumns = (
        Option<i64>,
        Option<i64>,
        i64,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let (idle,last,samples,old_sequence,old_boot,old_epoch,old_binding,old_connection):SampleColumns=tx.query_row("SELECT idle_mono,last_mono,samples,observation_sequence,runtime_boot,evidence_epoch,binding_generation,connection_epoch FROM seat_archival WHERE seat_id=?1",[claim.seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(store_error)?;
    let same_producer = old_boot.as_deref() == Some(rt.boot.as_str())
        && old_epoch == Some(epoch)
        && old_connection == Some(proof.connection_epoch() as i64);
    // Connection epochs advance within one daemon. A late read from an older
    // connection cannot become a new producer merely by differing from it.
    if old_boot.as_deref() == Some(rt.boot.as_str())
        && old_connection.is_some_and(|old| old > proof.connection_epoch() as i64)
    {
        return Ok(false);
    }
    let continuing = same_producer
        && idle.is_some()
        && last.is_some_and(|last| rt.mono > last && rt.mono - last <= MAX_GAP_MS)
        && old_sequence.is_some_and(|sequence| observation.observation_sequence as i64 > sequence)
        && old_binding == Some(claim.binding_generation as i64);
    // A duplicate sample cannot extend or restart an interval positively.
    if same_producer
        && (last == Some(rt.mono)
            || old_sequence.is_some_and(|seq| seq >= observation.observation_sequence as i64))
    {
        return Ok(false);
    }
    if !continuing {
        invalidate_sample(tx, claim.seat.as_str(), rt)?;
    }
    tx.execute("UPDATE seat_archival SET runtime_boot=?2,evidence_epoch=?3,idle_mono=?4,idle_utc=CASE WHEN ?5 THEN idle_utc ELSE ?6 END,last_mono=?7,samples=?8,next_mono=?9,binding_generation=?10,target_generation=?11,target_id=?12,host_boot=?13,host_epoch=?14,terminal=?15,incarnation=?16,harness=?17,native_session=?18,execution=?19,observation_sequence=?20,connection_epoch=?21 WHERE seat_id=?1",params![claim.seat.as_str(),rt.boot,epoch,if continuing {idle.unwrap()} else {rt.mono},continuing,rt.utc.0,rt.mono,if continuing {samples.saturating_add(1)} else {1},rt.mono.saturating_add(CADENCE_MS),claim.binding_generation as i64,proof.target_generation() as i64,claim.target.as_str(),proof.host_boot().as_str(),proof.host_epoch() as i64,proof.terminal().as_str(),proof.incarnation(),claim.harness.as_str(),claim.native_session.as_str(),claim.execution.as_str(),observation.observation_sequence as i64,proof.connection_epoch() as i64]).map_err(store_error)?;
    Ok(true)
}

/// Publication may remove a target without updating any target row. Validate
/// the new published view in bounded positive-seat pages before trusting a
/// retained certificate. Matching Unknown structural snapshots do not reset
/// previously earned composer evidence or the canonical mutation revision.
fn validate_snapshot_page(
    tx: &Transaction<'_>,
    instance: &str,
    rt: &Runtime,
) -> Result<bool, ApiError> {
    let (current,saved,checked,mut cursor):(Option<String>,Option<String>,bool,String)=tx.query_row("SELECT h.active_snapshot_id,a.validating_snapshot,a.snapshot_checked,a.snapshot_cursor FROM archival_instances a JOIN host_instances h ON h.id=a.instance_id WHERE a.instance_id=?1",[instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(store_error)?;
    if current == saved && checked {
        return Ok(true);
    }
    if current != saved {
        cursor.clear();
        tx.execute("UPDATE archival_instances SET validating_snapshot=?2,snapshot_cursor='',snapshot_checked=0 WHERE instance_id=?1",params![instance,current]).map_err(store_error)?;
    }
    let mut statement=tx.prepare("SELECT seat_id FROM seat_archival WHERE instance_id=?1 AND idle_mono IS NOT NULL AND seat_id>?2 ORDER BY seat_id LIMIT ?3").map_err(store_error)?;
    let rows = statement
        .query_map(params![instance, cursor, PAGE as i64], |r| {
            r.get::<_, String>(0)
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    for seat in &rows {
        if !sample_still_current(tx, instance, seat)? {
            invalidate_sample(tx, seat, rt)?;
        }
    }
    let complete = rows.len() < PAGE;
    tx.execute(
        "UPDATE archival_instances SET snapshot_cursor=?2,snapshot_checked=?3 WHERE instance_id=?1",
        params![instance, rows.last().unwrap_or(&cursor), complete],
    )
    .map_err(store_error)?;
    Ok(complete)
}

/// Budgeted production entry: all SQL work remains rollbackable under the VM and
/// busy/cancellation guards until StoreContext accepts the deciding transaction.
/// Apply is constant-time; fsync is the existing accepted storage boundary.
pub(crate) fn budgeted<R>(
    store: &super::SqliteStore,
    runtime: &Runtime,
    budget: &crate::protocol::time::CallBudget,
    work: impl FnOnce(&Transaction<'_>, &Runtime) -> Result<(R, Option<i64>), ApiError>,
) -> Result<R, ApiError> {
    let mut writer = store.writer(budget)?;
    store.context.execute_budgeted_decision(
        &mut writer,
        budget,
        |tx| {
            let mut rt = runtime.clone();
            rt.mono = i64::try_from(store.context.clock().monotonic_now().0).map_err(|_| {
                api_error(ErrorCode::InvalidRequest, "archival monotonic clock range")
            })?;
            rt.utc = store.context.clock().utc_now();
            if rt.after_ms < 0 || rt.after_ms > i64::MAX - MAX_GAP_MS {
                return Err(api_error(
                    ErrorCode::InvalidRequest,
                    "archival policy range",
                ));
            }
            let (result, expiry) = work(tx, &rt)?;
            let expiry = expiry.into_iter().chain(rt.valid_until_mono).min();
            Ok((result, expiry))
        },
        |_, decision, (result, expiry)| {
            if expiry.is_some_and(|expires| expires < 0 || decision.monotonic.0 > expires as u64) {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "archival evidence expired before decision",
                ));
            }
            Ok(result)
        },
    )
}
pub(crate) fn store_pass(
    store: &super::SqliteStore,
    rt: &Runtime,
    hints: &[crate::archival_legacy::Hint],
    budget: &crate::protocol::time::CallBudget,
) -> Result<Progress, ApiError> {
    if hints.len() > crate::archival_legacy::ENTRIES_PER_PAGE {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "archival hint page too large",
        ));
    }
    budgeted(store, rt, budget, |tx, rt| {
        for hint in hints {
            if hint.identity.claim.instance != store.instance {
                return Err(api_error(ErrorCode::InvalidRequest, "foreign legacy hint"));
            }
            super::handoff::import_hint(
                tx,
                &hint.identity,
                hint.progress_thread.as_ref(),
                rt.legacy_source
                    .as_deref()
                    .unwrap_or("partial-readonly-source"),
                rt.utc,
            )?;
        }
        let progress = advance_in(tx, &store.instance, rt)?;
        let expiry = if let Some(thread) = progress.archived.first() {
            tx.query_row(
                "SELECT scan_expiry FROM channel_archival WHERE thread_id=?1",
                [thread],
                |r| r.get::<_, Option<i64>>(0),
            )
            .map_err(store_error)?
        } else {
            None
        };
        Ok((progress, expiry))
    })
}
pub(crate) fn store_next(
    store: &super::SqliteStore,
    rt: &Runtime,
    budget: &crate::protocol::time::CallBudget,
) -> Result<ObservationWork, ApiError> {
    budgeted(store, rt, budget, |tx, rt| {
        reconcile_runtime(tx, &store.instance, rt)?;
        let ticket = next_observation_in(tx, &store.instance, rt)?;
        let has_more = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM seat_archival WHERE instance_id=?1 AND next_mono<=?2)",
                params![store.instance, rt.mono],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        Ok((ObservationWork { ticket, has_more }, None))
    })
}
pub(crate) fn store_sample(
    store: &super::SqliteStore,
    rt: &Runtime,
    ticket: &ObservationTicket,
    sample: Option<&ComposerObservation>,
    budget: &crate::protocol::time::CallBudget,
) -> Result<bool, ApiError> {
    if ticket.claim.instance != store.instance {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "foreign archival sample ticket",
        ));
    }
    budgeted(store, rt, budget, |tx, rt| {
        Ok((
            match sample {
                Some(sample) => record_sample_in(tx, ticket, rt, sample),
                None => {
                    reconcile_runtime(tx, &store.instance, rt)?;
                    invalidate_sample(tx, ticket.seat(), rt)?;
                    Ok(false)
                }
            }?,
            None,
        ))
    })
}
