//! `peer prune`: forgetting dead node islands (TRUST-POLICY N16, 2.8.0).
//!
//! Responsibility (NodePrune)
//! - Owns: the candidate rule ([`evaluate`]), the tombstone write
//!   ([`apply`]) and the `sync_forgotten_nodes` reads ([`forgotten`],
//!   [`list`]).
//! - Hides: the island rule. Two nodes are joined when a journal entry of
//!   one needs an entity (`journal.needs`) another's entry creates
//!   (`journal.creates`); every replicated reference is such an edge, because
//!   only facts cross nodes and the journal is never compacted. A node is
//!   pruned only when every node of its island passes the per-node checks.
//! - Must not: delete anything, write a replicated table, or decide on the
//!   host label alone (it is a scope filter, never proof).
//! - Role: called by `service::sync_admin` in one read (dry run) or one
//!   operator write quantum (`--yes`); the forgotten set is read by the views
//!   and by the coordinator's pull policy.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, Transaction, params};

use super::connection::{api_error, store_error};
use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};

/// Why a node on the host is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeepReason {
    /// It has a live link now.
    Linked,
    /// An enabled dial target has learned it.
    DialTarget,
    /// Contact (or, with none, first sight) within `older_than`.
    Recent,
    /// Fork or open clone evidence names it.
    ForkEvidence,
    /// A peer-requested launch row names it.
    PeerRequest,
    /// Another log holds entries of an unknown layout, whose references are
    /// unknown.
    OpaqueEntries,
    /// Its island holds this node, which is not prunable (its uuid).
    ReferencedBy(NodeUuid),
}

impl KeepReason {
    pub fn text(&self, name: impl Fn(&NodeUuid) -> String) -> String {
        match self {
            Self::Linked => "linked".into(),
            Self::DialTarget => "dial_target".into(),
            Self::Recent => "recent".into(),
            Self::ForkEvidence => "fork_evidence".into(),
            Self::PeerRequest => "peer_request".into(),
            Self::OpaqueEntries => "opaque_entries".into(),
            Self::ReferencedBy(node) => format!("referenced_by {}", name(node)),
        }
    }
}

/// One node on the host and its verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub node: NodeUuid,
    pub tag: String,
    pub host: Option<String>,
    /// Last contact, else first sight (unix ms).
    pub last_seen_at: i64,
    pub logs: u64,
    pub entries: u64,
    /// `None`: prune.
    pub keep: Option<KeepReason>,
}

/// What a prune considers.
#[derive(Clone, Debug)]
pub struct PruneScope {
    pub now_ms: i64,
    /// The host label to scope to.
    pub host: String,
    pub older_than_ms: i64,
    /// Nodes with a live link now (the coordinator's snapshot).
    pub live: BTreeSet<NodeUuid>,
}

fn parse(text: &str) -> Result<NodeUuid, ApiError> {
    text.parse()
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed node uuid"))
}

fn uuids(conn: &Connection, sql: &str) -> Result<BTreeSet<NodeUuid>, ApiError> {
    let mut statement = conn.prepare(sql).map_err(store_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(store_error)?;
    let mut out = BTreeSet::new();
    for row in rows {
        out.insert(parse(&row.map_err(store_error)?)?);
    }
    Ok(out)
}

/// The forgotten nodes.
pub fn forgotten(conn: &Connection) -> Result<BTreeSet<NodeUuid>, ApiError> {
    uuids(conn, "SELECT node_uuid FROM sync_forgotten_nodes")
}

/// One tombstone, as `peer prune --list` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Forgotten {
    pub node: NodeUuid,
    pub tag: String,
    pub host: Option<String>,
    pub forgotten_at: i64,
    pub provenance: String,
}

/// Every tombstone, oldest first.
pub fn list(conn: &Connection) -> Result<Vec<Forgotten>, ApiError> {
    let mut statement = conn
        .prepare(
            "SELECT node_uuid,tag,host_label,forgotten_at,provenance FROM sync_forgotten_nodes \
             ORDER BY forgotten_at, tag, node_uuid",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (node, tag, host, forgotten_at, provenance) = row.map_err(store_error)?;
        out.push(Forgotten {
            node: parse(&node)?,
            tag,
            host,
            forgotten_at,
            provenance,
        });
    }
    Ok(out)
}

/// Union-find over node uuids.
#[derive(Default)]
struct Islands {
    parent: BTreeMap<NodeUuid, NodeUuid>,
}

impl Islands {
    fn root(&mut self, node: NodeUuid) -> NodeUuid {
        let mut at = node;
        while let Some(&up) = self.parent.get(&at) {
            if up == at {
                break;
            }
            at = up;
        }
        self.parent.insert(node, at);
        at
    }

    fn join(&mut self, a: NodeUuid, b: NodeUuid) {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra != rb {
            self.parent.insert(ra.max(rb), ra.min(rb));
        }
    }
}

/// The islands: nodes joined by a `needs` -> `creates` edge.
fn islands(conn: &Connection) -> Result<Islands, ApiError> {
    let mut creator: BTreeMap<String, NodeUuid> = BTreeMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT l.node_uuid,j.creates FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE j.creates IS NOT NULL ORDER BY j.ingest_seq",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(store_error)?;
        for row in rows {
            let (node, entity) = row.map_err(store_error)?;
            let node = parse(&node)?;
            creator.entry(entity).or_insert(node);
        }
    }
    let mut islands = Islands::default();
    let mut statement = conn
        .prepare(
            "SELECT l.node_uuid,j.needs FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE j.needs IS NOT NULL AND j.needs <> ''",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(store_error)?;
    for row in rows {
        let (node, needs) = row.map_err(store_error)?;
        let node = parse(&node)?;
        for entity in needs.split('\n') {
            if let Some(&owner) = creator.get(entity)
                && owner != node
            {
                islands.join(node, owner);
            }
        }
    }
    Ok(islands)
}

/// Every not yet forgotten node whose host label is `scope.host`, with its
/// verdict. Read-only.
pub fn evaluate(conn: &Connection, scope: &PruneScope) -> Result<Vec<Candidate>, ApiError> {
    let local = uuids(conn, "SELECT id FROM local_node")?;
    let forgotten = forgotten(conn)?;
    let dial = uuids(
        conn,
        "SELECT learned_node_uuid FROM sync_dial_targets \
         WHERE enabled=1 AND learned_node_uuid IS NOT NULL",
    )?;
    let fork = uuids(
        conn,
        "SELECT l.node_uuid FROM sync_forks f JOIN sync_logs l ON l.log_ix=f.log_ix \
         UNION SELECT subject FROM sync_clone_evidence WHERE cleared_at IS NULL",
    )?;
    let requests = uuids(conn, "SELECT node_uuid FROM peer_requests")?;
    let opaque = uuids(
        conn,
        "SELECT DISTINCT l.node_uuid FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
         WHERE j.apply_state='parked_layout'",
    )?;
    let mut statement = conn
        .prepare(
            "SELECT d.node_uuid,d.tag,d.host_label, \
             COALESCE((SELECT last_contact_at FROM sync_node_contact c WHERE c.node_uuid=d.node_uuid), \
                      d.first_seen_at), \
             (SELECT COUNT(*) FROM sync_logs l WHERE l.node_uuid=d.node_uuid), \
             (SELECT COUNT(*) FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
              WHERE l.node_uuid=d.node_uuid) \
             FROM node_directory d ORDER BY d.tag, d.node_uuid",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(store_error)?;
    // Every directory node: its own verdict before the island rule. A node
    // off the host, or the local node, is never prunable and so blocks its
    // island.
    let mut all: Vec<(Candidate, bool)> = Vec::new();
    for row in rows {
        let (node, tag, host, last_seen_at, logs, entries) = row.map_err(store_error)?;
        let node = parse(&node)?;
        if forgotten.contains(&node) {
            continue;
        }
        let on_host = host.as_deref() == Some(scope.host.as_str()) && !local.contains(&node);
        let keep = if scope.live.contains(&node) {
            Some(KeepReason::Linked)
        } else if dial.contains(&node) {
            Some(KeepReason::DialTarget)
        } else if scope.now_ms.saturating_sub(last_seen_at) < scope.older_than_ms {
            Some(KeepReason::Recent)
        } else if fork.contains(&node) {
            Some(KeepReason::ForkEvidence)
        } else if requests.contains(&node) {
            Some(KeepReason::PeerRequest)
        } else {
            None
        };
        all.push((
            Candidate {
                node,
                tag,
                host,
                last_seen_at,
                logs: u64::try_from(logs).unwrap_or(0),
                entries: u64::try_from(entries).unwrap_or(0),
                keep,
            },
            on_host,
        ));
    }
    let blocked_by_opaque = opaque.iter().any(|node| {
        !all.iter().any(|(candidate, on_host)| {
            *on_host && candidate.keep.is_none() && candidate.node == *node
        })
    });
    let mut islands = islands(conn)?;
    // The first blocker of each island, in directory order.
    let mut blocker: BTreeMap<NodeUuid, NodeUuid> = BTreeMap::new();
    for (candidate, on_host) in &all {
        if !*on_host || candidate.keep.is_some() {
            let root = islands.root(candidate.node);
            blocker.entry(root).or_insert(candidate.node);
        }
    }
    // The local node, and any log owner the directory does not know, block
    // too.
    let listed: BTreeSet<NodeUuid> = all.iter().map(|(candidate, _)| candidate.node).collect();
    let owners = uuids(conn, "SELECT DISTINCT node_uuid FROM sync_logs")?;
    for node in local.iter().chain(
        owners
            .iter()
            .filter(|n| !listed.contains(n) && !forgotten.contains(n)),
    ) {
        let root = islands.root(*node);
        blocker.entry(root).or_insert(*node);
    }
    let mut out = Vec::new();
    for (mut candidate, on_host) in all {
        if !on_host {
            continue;
        }
        if candidate.keep.is_none() {
            let root = islands.root(candidate.node);
            if let Some(&by) = blocker.get(&root) {
                candidate.keep = Some(KeepReason::ReferencedBy(by));
            } else if blocked_by_opaque {
                candidate.keep = Some(KeepReason::OpaqueEntries);
            }
        }
        out.push(candidate);
    }
    Ok(out)
}

/// Evaluates in `tx` and writes a tombstone for every prunable node, with
/// `provenance`. Returns the evaluation.
pub fn apply(
    tx: &Transaction<'_>,
    scope: &PruneScope,
    provenance: &str,
) -> Result<Vec<Candidate>, ApiError> {
    let candidates = evaluate(tx, scope)?;
    for candidate in candidates.iter().filter(|c| c.keep.is_none()) {
        tx.execute(
            "INSERT OR IGNORE INTO sync_forgotten_nodes(node_uuid,tag,host_label,forgotten_at,provenance) \
             VALUES (?1,?2,?3,?4,?5)",
            params![
                candidate.node.to_string(),
                candidate.tag,
                candidate.host,
                scope.now_ms,
                provenance
            ],
        )
        .map_err(store_error)?;
    }
    Ok(candidates)
}
