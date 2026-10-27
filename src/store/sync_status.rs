//! Read APIs behind `sync status`, `sync verify`, `sync unapplied` and
//! `peer list` (ht-5fr.8), and the store doors the operator commands write
//! through.
//!
//! Responsibility
//! - Owns: the per-log counts (contiguous counter, head prefix,
//!   `applied_through`, parked and failed counts, freeze, skew), the bounded
//!   page of `parked_unknown`/`parked_dep`/`failed` journal entries, the
//!   `peer list` rows, the node-name lookup (`uuid` or tag) the operator
//!   commands share, and `sync verify` (the journal startup invariants, the
//!   I2/I3 checks of `store::incarnations` and the hash-chain linkage of every
//!   stored log, plus the J3 recomputation of each stored entry's hash).
//!   Also the two `SqliteStore` doors the operator commands need:
//!   an operator writer quantum and a bounded `ReplayScope::Unapplied` replay.
//! - Hides: the table shapes behind those reads and the partial index
//!   `journal_unapplied` they lean on.
//! - Must not: write a replicated table itself, decide a repair, or hold
//!   coordinator state; the coordinator snapshot is joined by the service
//!   (`service::sync_admin`).
//! - Role: pure functions over a query connection, plus the store doors.
//!   Read-only by construction: nothing here opens a transaction that writes.

use std::collections::BTreeMap;

use rusqlite::{Connection, Transaction, params};
use sha2::{Digest, Sha256};

use crate::node::{NodeTag, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::sync_admin::{
    LogRow, NodeRow, PeerRow, UnappliedPage, UnappliedRow, ViolationRow,
};
use crate::protocol::time::CallBudget;
use crate::store::apply::{self, ApplyLimits, ApplyState, ReplayScope};
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::{node_directory, sync_contacts, sync_dial_targets, sync_peer_status};
use crate::sync::envelope::StoredEntry;
use crate::sync::journal_port::ApplyProgress;
use crate::sync::{BatchLimits, LogKey};

/// Violations `sync verify` lists per check before it stops listing.
const MAX_VIOLATIONS: usize = 16;

/// A node's display name: its tag when the directory knows it, else the
/// first 8 hex characters of its uuid.
pub fn node_label(tag: Option<&str>, uuid: &str) -> String {
    tag.map_or_else(|| uuid.get(..8).unwrap_or(uuid).to_owned(), str::to_owned)
}

/// Resolves an operator-typed node name: a 32-hex uuid, or a tag the
/// directory knows. `InvalidRequest` for an unknown or ambiguous name.
pub fn resolve_node(conn: &Connection, name: &str) -> Result<NodeUuid, ApiError> {
    let invalid = |detail: &str| api_error(ErrorCode::InvalidRequest, detail);
    if let Ok(uuid) = name.parse::<NodeUuid>() {
        return Ok(uuid);
    }
    let tag: NodeTag = name
        .parse()
        .map_err(|_| invalid("node must be a 32-hex uuid or a 3-character tag"))?;
    let mut found = node_directory::colliding(conn, &tag).map_err(store_error)?;
    match found.len() {
        0 => Err(invalid("unknown node")),
        1 => Ok(found.remove(0)),
        _ => Err(invalid("ambiguous node tag; use the uuid")),
    }
}

/// The per-log rows with their keys, own logs first, at most `limit`; the
/// second value counts the logs left out. `lag` is left for the service to
/// fill from the coordinator.
pub fn per_log(
    conn: &Connection,
    node: Option<NodeUuid>,
    limit: usize,
) -> Result<(Vec<(LogKey, LogRow)>, u64), ApiError> {
    let filter = node.map(|node| node.to_string());
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_logs WHERE (?1 IS NULL OR node_uuid=?1)",
            [&filter],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let mut unapplied = std::collections::BTreeMap::<(i64, String), u64>::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT log_ix,apply_state,COUNT(*) FROM journal \
                 WHERE apply_state <> 'applied' AND apply_state IN ('parked_unknown','parked_dep','parked_layout','failed') \
                 GROUP BY log_ix,apply_state",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(store_error)?;
        for row in rows {
            let (log_ix, state, count) = row.map_err(store_error)?;
            unapplied.insert((log_ix, state), u64::try_from(count).unwrap_or(0));
        }
    }
    // Parked rows of a layout this build has a parser for: promotion failed.
    let mut cannot_promote = std::collections::BTreeMap::<i64, u64>::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT log_ix,substr(raw,1,8),COUNT(*) FROM journal \
                 WHERE apply_state='parked_layout' GROUP BY log_ix,substr(raw,1,8)",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(store_error)?;
        for row in rows {
            let (log_ix, magic, count) = row.map_err(store_error)?;
            let understood = <[u8; 8]>::try_from(magic.as_slice())
                .ok()
                .is_some_and(|m| crate::sync::envelope::understood_layout(&m).is_some());
            if understood {
                *cannot_promote.entry(log_ix).or_default() += u64::try_from(count).unwrap_or(0);
            }
        }
    }
    let mut statement = conn
        .prepare(
            "SELECT l.log_ix,l.node_uuid,l.incarnation,l.is_self,l.contiguous_counter,l.head_hash, \
             l.applied_through,l.frozen_at_counter,l.skew_ahead_ms,d.tag,l.blocked_on \
             FROM sync_logs l LEFT JOIN node_directory d ON d.node_uuid=l.node_uuid \
             WHERE (?1 IS NULL OR l.node_uuid=?1) \
             ORDER BY l.is_self DESC,l.node_uuid,l.incarnation LIMIT ?2",
        )
        .map_err(store_error)?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = statement
        .query_map(params![filter, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Vec<u8>>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (
            log_ix,
            node,
            incarnation,
            is_self,
            counter,
            head,
            applied,
            frozen,
            skew,
            tag,
            blocked_on,
        ) = row.map_err(store_error)?;
        let count = |state: &str| {
            unapplied
                .get(&(log_ix, state.to_owned()))
                .copied()
                .unwrap_or(0)
        };
        let key = LogKey {
            node: node
                .parse()
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed node uuid"))?,
            incarnation,
        };
        out.push((
            key,
            LogRow {
                node: node_label(tag.as_deref(), &node),
                incarnation,
                is_self,
                counter: u64::try_from(counter).unwrap_or(0),
                head: head.iter().take(4).map(|b| format!("{b:02x}")).collect(),
                applied_through: u64::try_from(applied).unwrap_or(0),
                parked_unknown: count("parked_unknown"),
                parked_dep: count("parked_dep"),
                parked_layout: count("parked_layout"),
                cannot_promote: cannot_promote.get(&log_ix).copied().unwrap_or(0),
                failed: count("failed"),
                frozen_at: frozen.and_then(|k| u64::try_from(k).ok()),
                lag: None,
                skew_ahead_ms: u64::try_from(skew).unwrap_or(0),
                blocked_on,
            },
        ));
    }
    let omitted = u64::try_from(total)
        .unwrap_or(0)
        .saturating_sub(out.len() as u64);
    Ok((out, omitted))
}

/// A bounded, ordered page of unapplied entries after `after` (an
/// `ingest_seq`), with the per-state totals. Ordered by `ingest_seq`, the
/// order a replay attempts them in.
pub fn unapplied_page(
    conn: &Connection,
    after: Option<u64>,
    limit: u16,
) -> Result<UnappliedPage, ApiError> {
    let after = i64::try_from(after.unwrap_or(0)).unwrap_or(i64::MAX);
    let mut statement = conn
        .prepare(
            // Three states: a `parked_layout` row has no kind to list (it is
            // counted per log in `per_log`).
            "SELECT j.ingest_seq,l.node_uuid,l.incarnation,j.counter,j.kind,j.apply_state, \
             j.parked_on,j.apply_error,d.tag \
             FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             LEFT JOIN node_directory d ON d.node_uuid=l.node_uuid \
             WHERE j.apply_state <> 'applied' \
               AND j.apply_state IN ('parked_unknown','parked_dep','failed') \
               AND j.ingest_seq > ?1 \
             ORDER BY j.ingest_seq LIMIT ?2",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map(params![after, i64::from(limit) + 1], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(store_error)?;
    let mut page = Vec::new();
    for row in rows {
        let (seq, node, incarnation, counter, kind, state, parked_on, error, tag) =
            row.map_err(store_error)?;
        let detail = match state.as_str() {
            "parked_dep" => parked_on.map(|entity| format!("on {entity}")),
            "failed" => error,
            _ => None,
        };
        page.push(UnappliedRow {
            ingest_seq: u64::try_from(seq).unwrap_or(0),
            log: format!("{}/{incarnation}", node_label(tag.as_deref(), &node)),
            counter: u64::try_from(counter).unwrap_or(0),
            kind,
            state,
            detail,
        });
    }
    let more = page.len() > usize::from(limit);
    page.truncate(usize::from(limit));
    let next_after = more
        .then(|| page.last().map(|row| row.ingest_seq))
        .flatten();
    let count = |state: &str| -> Result<u64, ApiError> {
        conn.query_row(
            "SELECT COUNT(*) FROM journal WHERE apply_state <> 'applied' AND apply_state=?1",
            [state],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| u64::try_from(n).unwrap_or(0))
        .map_err(store_error)
    };
    Ok(UnappliedPage {
        rows: page,
        parked_unknown: count("parked_unknown")?,
        parked_dep: count("parked_dep")?,
        failed: count("failed")?,
        next_after,
    })
}

/// Every dial target, enabled or not, with its learned node's name and
/// reachability at `now_ms`.
pub fn peers(conn: &Connection, now_ms: u64) -> Result<Vec<PeerRow>, ApiError> {
    let mut out = Vec::new();
    for target in sync_dial_targets::list(conn)? {
        let (node, reach) = match target.learned_node {
            Some(uuid) => {
                let uuid_text = uuid.to_string();
                let tag = node_directory::get(conn, uuid)
                    .map_err(store_error)?
                    .map(|row| row.tag.to_string());
                let reach = match sync_contacts::reach(conn, &uuid, now_ms)? {
                    sync_contacts::NodeReach::Reachable => "reachable",
                    sync_contacts::NodeReach::Unreachable { .. } => "unreachable",
                };
                (Some(node_label(tag.as_deref(), &uuid_text)), reach)
            }
            None => (None, "unknown"),
        };
        out.push(PeerRow {
            target_id: target.target_id.0,
            ssh_dest: target.ssh_dest,
            source: target.source.as_str().to_owned(),
            enabled: target.enabled,
            state: target.state.as_str().to_owned(),
            stopped_reason: target.stopped.map(|(reason, _)| reason.as_str().to_owned()),
            remote_command: target.remote_command,
            node,
            reach: reach.to_owned(),
            attempts: target.attempts,
            last_error: target.last_error,
        });
    }
    Ok(out)
}

/// A node's live link, as the coordinator reports it (the daemon's view;
/// absent when the node has no active link or the coordinator did not
/// answer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveNode {
    pub node: NodeUuid,
    pub linked_since: i64,
    pub last_rx: i64,
    pub app_seen_at: i64,
    pub version: Option<String>,
    pub tx: Option<(u64, u64)>,
    pub rx: Option<(u64, u64)>,
    /// Which of `app`, `tx`, `rx` stalled.
    pub stalled: Vec<&'static str>,
}

/// The node-status rule (ht-xg88), the only place it is decided. First match
/// wins: unreachable by `sync_contacts::reach`; `stalled` while its active
/// link stalled; `linked` with an active link; else `reachable` (relayed).
pub fn node_state(reach: sync_contacts::NodeReach, live: Option<&LiveNode>) -> &'static str {
    match (reach, live) {
        (sync_contacts::NodeReach::Unreachable { .. }, _) => "unreachable",
        (_, Some(live)) if !live.stalled.is_empty() => "stalled",
        (_, Some(_)) => "linked",
        (_, None) => "reachable",
    }
}

/// Every known peer node at `now_ms`: its state by [`node_state`], the live
/// values of its active link where `live` has one, else the last-known ones
/// from `sync_peer_status`.
pub fn nodes(conn: &Connection, now_ms: u64, live: &[LiveNode]) -> Result<Vec<NodeRow>, ApiError> {
    let known: Vec<String> = {
        let mut statement = conn
            .prepare_cached(
                "SELECT node_uuid FROM node_directory \
                 WHERE node_uuid NOT IN (SELECT id FROM local_node) ORDER BY tag, node_uuid",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(store_error)?;
        rows.collect::<Result<_, _>>().map_err(store_error)?
    };
    let stored: BTreeMap<NodeUuid, sync_peer_status::PeerStatus> = sync_peer_status::list(conn)?
        .into_iter()
        .map(|row| (row.node, row))
        .collect();
    let mut clones = conn
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM sync_clone_evidence \
             WHERE subject = ?1 AND cleared_at IS NULL)",
        )
        .map_err(store_error)?;
    let at = |v: u64| i64::try_from(v).ok();
    let mut out = Vec::new();
    for text in known {
        let uuid: NodeUuid = text
            .parse()
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed node uuid"))?;
        let tag = node_directory::get(conn, uuid)
            .map_err(store_error)?
            .map(|row| row.tag.to_string());
        let reach = sync_contacts::reach(conn, &uuid, now_ms)?;
        let contact = sync_contacts::get(conn, &uuid)?;
        let live = live.iter().find(|l| l.node == uuid);
        let last = stored.get(&uuid);
        let clone_suspected: bool = clones
            .query_row([&text], |row| row.get(0))
            .map_err(store_error)?;
        out.push(NodeRow {
            node: node_label(tag.as_deref(), &text),
            state: node_state(reach, live).to_owned(),
            stalled: live
                .map(|l| l.stalled.iter().map(|s| (*s).to_owned()).collect())
                .unwrap_or_default(),
            last_contact_at: live
                .map(|l| l.last_rx)
                .or_else(|| contact.and_then(|c| at(c.last_contact_at))),
            app_seen_at: live
                .map(|l| l.app_seen_at)
                .or_else(|| last.and_then(|r| r.app_seen_at.and_then(at))),
            linked_since: live.map(|l| l.linked_since),
            version: live
                .and_then(|l| l.version.clone())
                .or_else(|| last.and_then(|r| r.version.clone())),
            tx: live.and_then(|l| l.tx).or_else(|| last.and_then(|r| r.tx)),
            rx: live.and_then(|l| l.rx).or_else(|| last.and_then(|r| r.rx)),
            clone_suspected,
        });
    }
    Ok(out)
}

/// `sync verify`: the journal startup invariants, the I2/I3 checks, the
/// hash-chain linkage of every stored log and (J3) a recomputation of each
/// stored entry's hash from its canonical bytes. Empty means healthy.
///
/// The linkage check compares each entry's `prev_hash` with the stored hash
/// of the entry before it. The J3 walk is linear in the journal size and runs
/// only on this operator read; it covers the main chains (`sync_logs`), not
/// `journal_forked` rows.
pub fn verify(conn: &Connection) -> Result<Vec<ViolationRow>, ApiError> {
    let mut out = Vec::new();
    match crate::store::journal::verify_invariants(conn) {
        Ok(()) => {}
        Err(error) if error.code == ErrorCode::StoreCorrupt => out.push(ViolationRow {
            invariant: "J1".to_owned(),
            log: None,
            counter: None,
            detail: error.detail,
        }),
        Err(error) => return Err(error),
    }
    for violation in crate::store::incarnations::check_invariants(conn)? {
        out.push(ViolationRow {
            invariant: format!("{:?}", violation.invariant),
            log: Some(format!(
                "{}/{}",
                violation.log.node.to_string().get(..8).unwrap_or_default(),
                violation.log.incarnation
            )),
            counter: Some(violation.counter),
            detail: violation.detail,
        });
    }
    let mut statement = conn
        .prepare(
            // Layout-agnostic: linkage reads only the envelope columns.
            "SELECT l.node_uuid,l.incarnation,j.counter FROM journal j \
                 JOIN sync_logs l ON l.log_ix=j.log_ix \
                 LEFT JOIN journal p ON p.log_ix=j.log_ix AND p.counter=j.counter-1 \
                 WHERE j.counter > 1 AND (p.hash IS NULL OR p.hash <> j.prev_hash) \
                 ORDER BY l.log_ix,j.counter LIMIT ?1",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([i64::try_from(MAX_VIOLATIONS).unwrap_or(16)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(store_error)?;
    for row in rows {
        let (node, incarnation, counter) = row.map_err(store_error)?;
        out.push(ViolationRow {
            invariant: "J2".to_owned(),
            log: Some(format!("{}/{incarnation}", node.get(..8).unwrap_or(&node))),
            counter: Some(u64::try_from(counter).unwrap_or(0)),
            detail: format!(
                "prev_hash does not match the stored hash at counter {}",
                counter - 1
            ),
        });
    }
    rehash_entries(conn, &mut out)?;
    Ok(out)
}

/// J3: recomputes every stored entry's hash (a known entry from its canonical
/// bytes, an opaque one as SHA-256 of its raw bytes, never as HTJRNL01);
/// stops listing at [`MAX_VIOLATIONS`] rows.
fn rehash_entries(conn: &Connection, out: &mut Vec<ViolationRow>) -> Result<(), ApiError> {
    let mut logs = Vec::new();
    {
        let mut statement = conn
            .prepare("SELECT node_uuid,incarnation FROM sync_logs ORDER BY log_ix")
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))
            .map_err(store_error)?;
        for row in rows {
            logs.push(row.map_err(store_error)?);
        }
    }
    let mut listed = 0usize;
    'logs: for (node, incarnation) in logs {
        let Ok(uuid) = node.parse::<NodeUuid>() else {
            continue;
        };
        let log = LogKey {
            node: uuid,
            incarnation,
        };
        let name = format!("{}/{incarnation}", node.get(..8).unwrap_or(&node));
        let mut from = 1u64;
        loop {
            let batch = match crate::store::journal::serve_entries(
                conn,
                &log,
                from,
                BatchLimits::default(),
            ) {
                Ok(batch) => batch,
                Err(error) => {
                    out.push(ViolationRow {
                        invariant: "J3".to_owned(),
                        log: Some(name),
                        counter: Some(from),
                        detail: error.detail,
                    });
                    listed += 1;
                    if listed >= MAX_VIOLATIONS {
                        break 'logs;
                    }
                    continue 'logs;
                }
            };
            let Some(last) = batch.last().map(|entry| match entry {
                StoredEntry::Known(e) => e.counter,
                StoredEntry::Opaque(o) => o.counter(),
            }) else {
                break;
            };
            for entry in &batch {
                let (counter, detail) = match entry {
                    StoredEntry::Known(e) if e.compute_hash() != e.hash => {
                        (e.counter, "hash does not match the entry's canonical bytes")
                    }
                    StoredEntry::Opaque(o)
                        if <[u8; 32]>::from(Sha256::digest(o.raw())) != o.hash() =>
                    {
                        (o.counter(), "hash does not match the entry's raw bytes")
                    }
                    _ => continue,
                };
                out.push(ViolationRow {
                    invariant: "J3".to_owned(),
                    log: Some(name.clone()),
                    counter: Some(counter),
                    detail: detail.to_owned(),
                });
                listed += 1;
                if listed >= MAX_VIOLATIONS {
                    break 'logs;
                }
            }
            from = last.saturating_add(1);
        }
    }
    Ok(())
}

/// The store doors the operator commands use. Callers hold a `FairWriter`
/// turn.
impl crate::store::SqliteStore {
    /// Runs `read` on a fresh query connection inside a deferred
    /// transaction, so it sees one snapshot.
    pub fn sync_read<R>(
        &self,
        budget: &CallBudget,
        read: impl FnOnce(&Connection, &StoreContext) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let db = self.context().open_query(budget.clone())?;
        db.execute_batch("BEGIN DEFERRED")
            .map_err(|e| db.map_error(e))?;
        read(&db, self.context())
    }

    /// One operator write (`peer add`/`forget`, `sync clone-cleared`) as one
    /// budgeted decision on the store's writer.
    pub fn sync_operator_quantum<R>(
        &self,
        budget: &CallBudget,
        apply: impl FnOnce(&Transaction<'_>, &StoreContext, DecisionInstant) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let mut writer = self.writer(budget)?;
        self.context().execute_budgeted_decision(
            &mut writer,
            budget,
            |_| Ok(()),
            |tx, decision, ()| apply(tx, self.context(), decision),
        )
    }

    /// One bounded `ReplayScope::Unapplied` pass over every unapplied state
    /// (`sync unapplied replay`).
    pub fn replay_unapplied(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
        const STATES: [ApplyState; 3] = [
            ApplyState::ParkedUnknown,
            ApplyState::ParkedDep,
            ApplyState::Failed,
        ];
        self.sync_operator_quantum(budget, |tx, ctx, decision| {
            apply::replay(
                tx,
                ctx,
                decision,
                ReplayScope::Unapplied { states: &STATES },
                ApplyLimits::default(),
            )
        })
    }
}

#[cfg(test)]
#[path = "../../tests/store/sync_status.rs"]
mod tests;
