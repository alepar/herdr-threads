//! The daemon side of the operator and status CLI for sync (ht-5fr.8).
//!
//! Responsibility (SyncAdmin)
//! - Owns: answering one [`SyncCommand`]: `peer add|forget|list`, `sync status`
//!   (store rows joined with the coordinator snapshot and the incarnations
//!   repair status), `sync verify`, `sync unapplied list|replay`, `sync resync`
//!   and `sync clone-cleared`. For the last it commits
//!   `store::incarnations::clear_clone_evidence` with the caller's
//!   `operator:local-user:<uid>` label and then signals the incarnations
//!   worker through the sink, so a held restore rotation runs.
//! - Hides: the store doors, the `FairWriter` turn each write takes and the
//!   runtime handle that lets the blocking handler await the coordinator.
//! - Must not: authenticate the caller (dispatch admits only the elected
//!   owner's peer UID and hands over the [`OperatorActor`]), write a
//!   replicated table outside the apply engine's replay, or decide a repair.
//! - Role: composed by `app.rs` next to the sync runtime and handed to
//!   `DomainService`; the pure reads live in `store::sync_status`.
//!
//! Every mutating command needs the actor: without one it is `Unauthorized`.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::daemon::paths::NodePaths;
use crate::protocol::authority::OperatorActor;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::sync_admin::{
    EventRow, EventsPage, EvidenceRow, LinkRow, MAX_PRUNE_ROWS, MAX_STATUS_LOGS, PruneReply,
    PruneRow, RepairRow, ReplayReply, StatusReply, SyncCommand, SyncReply,
};
use crate::protocol::time::{CallBudget, Cancellation, Clock};
use crate::service::fair_writer::FairWriter;
use crate::store::SqliteStore;
use crate::store::incarnations::{self, ForkOrigin};
use crate::store::sync_status::{self, node_label, resolve_node};
use crate::store::{node_directory, node_prune, sync_dial_targets, sync_link_events};
use crate::sync::LogKey;
use crate::sync::coordinator::{LinkMode, LinkStatus, LogFn, SyncHandle};
use crate::sync::incarnations::IncarnationsSink;
use crate::sync::resync::{self, RequestEnv};

/// Appended to the two-copies remedy: the dial targets a `node_collision`
/// stopped are not restarted by the clearance.
/// How long `peer list` waits for the coordinator's live links (ms) before
/// it shows the stored last-known rows alone.
const PEER_LIST_SNAPSHOT_MS: u64 = 2_000;
/// How long `resolve address` waits for the home node's answer (spec 4.3).
pub const REMOTE_RESOLVE_WAIT_MS: u64 = 10_000;
/// How long it then waits for the named seat to replicate here.
pub const REPLICA_WAIT_MS: u64 = 5_000;
/// The daemon transport's bound for one `resolve address` request: both
/// waits plus margin (the ordinary 5 s bound would cut them off).
pub const REMOTE_ADDRESS_DAEMON_BOUND_MS: u64 = REMOTE_RESOLVE_WAIT_MS + REPLICA_WAIT_MS + 2_000;
/// Known machines an unknown-machine answer lists.
const KNOWN_MACHINES_LISTED: usize = 8;
const REMEDY_RE_ADD: &str = "; then `herdr-threads peer add` any target stopped by node_collision";

/// See the module documentation.
pub struct SyncAdmin {
    pub store: Arc<SqliteStore>,
    pub sync: SyncHandle,
    pub sink: Arc<IncarnationsSink>,
    pub clock: Arc<dyn Clock>,
    pub paths: NodePaths,
    /// The token the wire `Stop` cancels (the daemon's graceful stop).
    pub shutdown: Cancellation,
    pub writer: Arc<FairWriter>,
    pub log: LogFn,
    /// The daemon's runtime: the handler runs on a blocking thread and awaits
    /// the coordinator through it.
    pub runtime: tokio::runtime::Handle,
    /// The discovery worker's state; `None` when the daemon runs none.
    pub discovery: Option<Arc<crate::sync::discovery::DiscoveryStatus>>,
}

impl SyncAdmin {
    fn discovery_state(&self) -> Option<String> {
        self.discovery
            .as_ref()
            .map(|status| status.get().as_str().to_owned())
    }

    /// Answers `command`. `actor` is the admitted operator for a mutating
    /// command and `None` for a read.
    pub fn run(
        &self,
        command: SyncCommand,
        actor: Option<&OperatorActor>,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        command
            .validate()
            .map_err(|detail| ApiError::new(ErrorCode::InvalidRequest, detail))?;
        let operator = match (command.is_mutating(), actor) {
            (true, None) => {
                return Err(ApiError::new(
                    ErrorCode::Unauthorized,
                    "operator peer does not match elected owner",
                ));
            }
            (_, actor) => actor.map(OperatorActor::audit_label),
        };
        let label = operator.as_deref().unwrap_or_default();
        match command {
            SyncCommand::PeerAdd {
                ssh_dest,
                remote_command,
            } => self.peer_add(&ssh_dest, remote_command.as_deref(), label, budget),
            SyncCommand::PeerForget { target } => self.peer_forget(&target, label, budget),
            SyncCommand::PeerList => {
                let now = u64::try_from(self.clock.utc_now().0).unwrap_or(0);
                // Without an answer from the coordinator no node is shown as
                // linked: the stored rows give last-known values only. The ask
                // gets its own short deadline, so a wedged coordinator still
                // leaves the budget for the store read.
                let ask = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        budget.deadline.0.min(
                            self.clock
                                .monotonic_now()
                                .0
                                .saturating_add(PEER_LIST_SNAPSHOT_MS),
                        ),
                    ),
                    cancellation: budget.cancellation.clone(),
                };
                let live: Vec<sync_status::LiveNode> = self
                    .runtime
                    .block_on(self.sync.snapshot(&ask))
                    .map(|snapshot| live_nodes(&snapshot.links))
                    .unwrap_or_default();
                let (rows, nodes, forgotten) = self.store.sync_read(budget, |conn, _| {
                    Ok((
                        sync_status::peers(conn, now)?,
                        sync_status::nodes(conn, now, &live)?,
                        sync_status::forgotten_count(conn)?,
                    ))
                })?;
                Ok(SyncReply::Peers {
                    rows,
                    discovery: self.discovery_state(),
                    nodes,
                    forgotten,
                })
            }
            SyncCommand::Status { log } => self.status(log.as_deref(), budget),
            SyncCommand::Verify => {
                let violations = self
                    .store
                    .sync_read(budget, |conn, _| sync_status::verify(conn))?;
                Ok(SyncReply::Verified { violations })
            }
            SyncCommand::UnappliedList { after, limit } => {
                let page = self.store.sync_read(budget, |conn, _| {
                    sync_status::unapplied_page(conn, after, limit)
                })?;
                Ok(SyncReply::Unapplied(page))
            }
            SyncCommand::UnappliedReplay => self.unapplied_replay(label, budget),
            SyncCommand::Resync { from, logs } => self.resync(&from, &logs, label, budget),
            SyncCommand::CloneCleared => self.clone_cleared(label, budget),
            SyncCommand::ResolveAddress { machine, target } => {
                self.resolve_address(&machine, target, budget)
            }
            SyncCommand::CheckRemoteSeat { seat } => self.check_remote_seat(&seat, budget),
            SyncCommand::RemoteLaunch { machine, request } => {
                self.remote_launch(&machine, Some(*request), budget)
            }
            SyncCommand::ResolveMachine { machine } => self.remote_launch(&machine, None, budget),
            SyncCommand::PeerEvents {
                node,
                before,
                limit,
            } => self.peer_events(node.as_deref(), before, limit, budget),
            SyncCommand::PeerPrune {
                host,
                older_than_ms,
                apply,
                list,
            } => self.peer_prune(host, older_than_ms, apply, list, label, budget),
        }
    }

    fn note(&self, operator: &str, what: &str) {
        (self.log)(&format!("sync: {operator}: {what}"));
    }

    fn peer_add(
        &self,
        ssh_dest: &str,
        remote_command: Option<&str>,
        operator: &str,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
        let now = self.clock.utc_now().0;
        let id = self.store.sync_operator_quantum(budget, |tx, _, _| {
            sync_dial_targets::add_explicit(tx, ssh_dest, remote_command, now)
        })?;
        self.sync.reload_targets();
        self.note(operator, &format!("peer add {ssh_dest}"));
        Ok(SyncReply::PeerAdded {
            target_id: id.0,
            ssh_dest: ssh_dest.to_owned(),
        })
    }

    fn peer_forget(
        &self,
        target: &str,
        operator: &str,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
        let now = self.clock.utc_now().0;
        let disabled = self.store.sync_operator_quantum(budget, |tx, _, _| {
            let mut disabled = sync_dial_targets::forget(tx, target, now)?;
            if disabled == 0
                && let Ok(node) = resolve_node(tx, target)
            {
                disabled = sync_dial_targets::forget(tx, &node.to_string(), now)?;
            }
            Ok(disabled)
        })?;
        if disabled == 0 {
            return Err(ApiError::new(
                ErrorCode::NotFound,
                "no enabled dial target matches",
            ));
        }
        self.sync.reload_targets();
        self.note(operator, &format!("peer forget {target}"));
        Ok(SyncReply::PeerForgotten {
            disabled: u32::try_from(disabled).unwrap_or(u32::MAX),
        })
    }

    fn status(&self, log: Option<&str>, budget: &CallBudget) -> Result<SyncReply, ApiError> {
        let (logs, omitted, repair, directory, targets, forgotten) =
            self.store.sync_read(budget, |conn, ctx| {
                let node = log.map(|name| resolve_node(conn, name)).transpose()?;
                let (logs, omitted) = sync_status::per_log(conn, node, MAX_STATUS_LOGS)?;
                let repair = incarnations::status(conn, ctx)?;
                // Names for the evidence subjects and the repair logs.
                let mut directory = BTreeMap::new();
                let mut want = Vec::new();
                for row in &repair.clone_evidence {
                    want.push(row.subject);
                }
                for repaired in &repair.logs {
                    want.push(repaired.log.node);
                }
                for node in want {
                    if let Some(row) = node_directory::get(conn, node)
                        .map_err(crate::store::connection::store_error)?
                    {
                        directory.insert(node, row.tag.to_string());
                    }
                }
                let targets = sync_dial_targets::list(conn)?;
                let forgotten = sync_status::forgotten_count(conn)?;
                Ok((logs, omitted, repair, directory, targets, forgotten))
            })?;
        let name = |node: &crate::node::NodeUuid| {
            node_label(directory.get(node).map(String::as_str), &node.to_string())
        };
        let log_name = |key: &LogKey| format!("{}/{}", name(&key.node), key.incarnation);

        let snapshot = self.runtime.block_on(self.sync.snapshot(budget));
        let coordinator_unavailable = snapshot.is_err();
        let snapshot = snapshot.unwrap_or_default();

        let logs = logs
            .into_iter()
            .map(|(key, mut row)| {
                row.lag = snapshot
                    .advertised
                    .get(&key)
                    .map(|advertised| advertised.saturating_sub(row.counter));
                row
            })
            .collect();
        let links = snapshot
            .links
            .iter()
            .map(|link| {
                let last_error = link.last_error.clone().or_else(|| {
                    link.dest.as_ref().and_then(|dest| {
                        targets
                            .iter()
                            .find(|target| &target.ssh_dest == dest)
                            .and_then(|target| target.current_error().map(str::to_owned))
                    })
                });
                LinkRow {
                    node: link.tag.to_string(),
                    mode: match link.mode {
                        LinkMode::Active => "active",
                        LinkMode::Standby => "standby",
                    }
                    .to_owned(),
                    role: format!("{:?}", link.role).to_lowercase(),
                    rx_logs: link.rx_logs,
                    unsolicited: link.unsolicited,
                    withheld_logs: link.withheld_logs,
                    dest: link.dest.clone(),
                    last_error,
                    since: Some(link.since.0),
                    last_rx: Some(link.last_rx.0),
                    app_rx: link.app_rx.map(|at| at.0),
                    subscriptions: link.subscriptions,
                    peer_lag: link.tx.map(|(acked, head)| head.saturating_sub(acked)),
                    stalled: link
                        .stalled
                        .names()
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                }
            })
            .collect();
        let repairs = repair
            .logs
            .iter()
            .map(|repaired| RepairRow {
                log: log_name(&repaired.log),
                frozen_at: repaired.frozen_at,
                origin: repaired.origin.map(|origin| {
                    match origin {
                        ForkOrigin::Evidence => "evidence",
                        ForkOrigin::Declared => "declared",
                    }
                    .to_owned()
                }),
                evidence: repaired.evidence_count,
                branches: u32::try_from(repaired.branches.len()).unwrap_or(u32::MAX),
                reissued: repaired.reissued,
                pending: repaired.pending,
                cannot_reissue: repaired.cannot_reissue,
            })
            .collect();
        let evidence = repair
            .clone_evidence
            .iter()
            .filter(|row| row.cleared_at.is_none())
            .map(|row| EvidenceRow {
                ev_ix: row.ev_ix,
                subject: name(&row.subject),
                kind: row.kind.clone(),
                log: row.log.as_ref().map(log_name),
                detail: row.detail.clone(),
                detected_at: row.detected_at,
                last_seen_at: row.last_seen_at,
            })
            .collect();
        let remedies = repair
            .remedy
            .map(|remedy| {
                vec![
                    format!("{}{REMEDY_RE_ADD}", remedy.two_copies),
                    remedy.single_copy.to_owned(),
                ]
            })
            .unwrap_or_default();
        Ok(SyncReply::Status(Box::new(StatusReply {
            logs,
            logs_omitted: omitted,
            links,
            paused_logs: snapshot.paused_logs.iter().map(&log_name).collect(),
            repairs,
            evidence,
            own_unknown_newer: repair.own_unknown_newer.iter().map(&log_name).collect(),
            lineage_branches: repair.lineage_branches.iter().map(&log_name).collect(),
            restore_successor: repair.restore_successor.as_ref().map(log_name),
            remedies,
            store_bytes: resync::database_bytes(&self.paths),
            headroom_until: snapshot.headroom_until.map(|until| until.0),
            coordinator_unavailable,
            discovery: self.discovery_state(),
            forgotten,
        })))
    }

    fn unapplied_replay(&self, operator: &str, budget: &CallBudget) -> Result<SyncReply, ApiError> {
        let _turn = self.writer.enter_background(budget, self.clock.as_ref())?;
        let progress = self.store.replay_unapplied(budget)?;
        self.note(operator, "unapplied replay");
        Ok(SyncReply::Replayed(ReplayReply {
            attempted: progress.replayed as u64,
            applied: progress.applied as u64,
            has_more: progress.has_more,
        }))
    }

    fn resync(
        &self,
        from: &str,
        logs: &[String],
        operator: &str,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        let (node, selected) = self.store.sync_read(budget, |conn, _| {
            let node = resolve_node(conn, from)?;
            let selected = logs
                .iter()
                .map(|name| resolve_node(conn, name))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((node, selected))
        })?;
        let env = RequestEnv {
            clock: &self.clock,
            shutdown: &self.shutdown,
        };
        self.note(operator, &format!("resync from {from}"));
        self.runtime.block_on(resync::request(
            self.store.as_ref(),
            &self.paths,
            &self.sync,
            node,
            (!selected.is_empty()).then_some(selected),
            operator,
            &env,
        ))?;
        Ok(SyncReply::ResyncRequested {
            from: from.to_owned(),
        })
    }

    fn peer_events(
        &self,
        node: Option<&str>,
        before: Option<i64>,
        limit: u16,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        let node_filter = node;
        let page = self.store.sync_read(budget, |conn, _| {
            let node = node.map(|name| resolve_node(conn, name)).transpose()?;
            let (rows, more) = sync_link_events::page(conn, node, before, limit)?;
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let tag = match row.node.as_deref().map(str::parse) {
                    Some(Ok(uuid)) => node_directory::get(conn, uuid)
                        .map_err(crate::store::connection::store_error)?
                        .map(|dir| dir.tag.to_string()),
                    _ => None,
                };
                out.push(EventRow {
                    seq: row.seq,
                    at: row.at,
                    last_at: row.last_at,
                    n: row.n,
                    node: row
                        .node
                        .as_deref()
                        .map(|uuid| node_label(tag.as_deref(), uuid)),
                    dest: row.dest,
                    kind: row.kind,
                    origin: row.origin,
                    detail: row.detail,
                });
            }
            let next_before = more.then(|| out.last().map(|row| row.seq)).flatten();
            Ok(EventsPage {
                rows: out,
                next_before,
                node: node_filter.map(str::to_owned),
                limit,
            })
        })?;
        Ok(SyncReply::Events(page))
    }

    /// `peer prune` (N16): a dry run, the tombstone write (`apply`) or the
    /// tombstone list.
    fn peer_prune(
        &self,
        host: Option<String>,
        older_than_ms: u64,
        apply: bool,
        list: bool,
        operator: &str,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        if list {
            let rows = self
                .store
                .sync_read(budget, |conn, _| node_prune::list(conn))?;
            let total = rows.len();
            let rows: Vec<PruneRow> = rows
                .into_iter()
                .take(MAX_PRUNE_ROWS)
                .map(|row| PruneRow {
                    node: row.tag,
                    uuid: row.node.to_string(),
                    host: row.host,
                    verdict: "forgotten".into(),
                    reason: Some(row.provenance),
                    at: row.forgotten_at,
                    logs: 0,
                    entries: 0,
                })
                .collect();
            return Ok(SyncReply::Pruned(PruneReply {
                list: true,
                omitted: (total - rows.len()) as u64,
                prunable: total as u64,
                rows,
                ..PruneReply::default()
            }));
        }
        // Every node with any link now is live, whatever its mode. Without
        // the coordinator's answer a dry run still runs on the stored
        // contact times; a write does not.
        let live: std::collections::BTreeSet<crate::node::NodeUuid> =
            match self.runtime.block_on(self.sync.snapshot(budget)) {
                Ok(snapshot) => snapshot.links.iter().map(|link| link.node).collect(),
                Err(e) if apply => return Err(e),
                Err(_) => Default::default(),
            };
        let host = match host {
            Some(host) => host,
            None => self.store.sync_read(budget, |conn, _| {
                conn.query_row(
                    "SELECT d.host_label FROM node_directory d JOIN local_node n ON n.id=d.node_uuid",
                    [],
                    |r| r.get::<_, Option<String>>(0),
                )
                .map_err(crate::store::connection::store_error)?
                .ok_or_else(|| {
                    ApiError::new(
                        ErrorCode::InvalidRequest,
                        "this node has no host label; pass --host",
                    )
                })
            })?,
        };
        let scope = node_prune::PruneScope {
            now_ms: self.clock.utc_now().0,
            host: host.clone(),
            older_than_ms: i64::try_from(older_than_ms).unwrap_or(i64::MAX),
            live,
        };
        let (candidates, names) = if apply {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store.sync_operator_quantum(budget, |tx, _, _| {
                let candidates = node_prune::apply(tx, &scope, operator)?;
                Ok((candidates, prune_names(tx)?))
            })?
        } else {
            self.store.sync_read(budget, |conn, _| {
                Ok((node_prune::evaluate(conn, &scope)?, prune_names(conn)?))
            })?
        };
        let prunable = candidates.iter().filter(|c| c.keep.is_none()).count() as u64;
        let kept = candidates.len() as u64 - prunable;
        if apply {
            for candidate in candidates.iter().filter(|c| c.keep.is_none()) {
                self.note(
                    operator,
                    &format!("peer prune forgot {} ({})", candidate.tag, candidate.node),
                );
            }
            // The coordinator reads the forgotten set with its next local
            // vector; a round asks for one.
            self.sync
                .signals()
                .request_round(self.clock.monotonic_now());
        }
        let total = candidates.len();
        // Prunable rows first, then kept ones, each in directory order.
        let mut ordered: Vec<_> = candidates.iter().filter(|c| c.keep.is_none()).collect();
        ordered.extend(candidates.iter().filter(|c| c.keep.is_some()));
        let rows: Vec<PruneRow> = ordered
            .into_iter()
            .take(MAX_PRUNE_ROWS)
            .map(|c| PruneRow {
                node: c.tag.clone(),
                uuid: c.node.to_string(),
                host: c.host.clone(),
                verdict: match (&c.keep, apply) {
                    (Some(_), _) => "keep",
                    (None, true) => "pruned",
                    (None, false) => "prune",
                }
                .into(),
                reason: c.keep.as_ref().map(|reason| {
                    reason.text(|node| {
                        node_label(names.get(node).map(String::as_str), &node.to_string())
                    })
                }),
                at: c.last_seen_at,
                logs: c.logs,
                entries: c.entries,
            })
            .collect();
        Ok(SyncReply::Pruned(PruneReply {
            host,
            older_than_ms,
            applied: apply,
            list: false,
            omitted: (total - rows.len()) as u64,
            rows,
            prunable,
            kept,
        }))
    }

    fn clone_cleared(&self, operator: &str, budget: &CallBudget) -> Result<SyncReply, ApiError> {
        let cleared = {
            let _turn = self.writer.enter_foreground(budget, self.clock.as_ref())?;
            self.store
                .sync_operator_quantum(budget, |tx, _, decision| {
                    incarnations::clear_clone_evidence(tx, operator, decision.utc)
                })?
        };
        // The worker re-derives its held repairs, forgets its Have memos and
        // asks for a round. Always signalled: a clearance committed earlier
        // whose signal was lost must not leave a repair held.
        self.sink.notify_clone_cleared();
        self.note(operator, &format!("clone-cleared ({cleared} rows)"));
        Ok(SyncReply::CloneCleared { cleared })
    }
}

#[cfg(test)]
#[path = "../../tests/service/sync_admin.rs"]
mod tests;

/// Every directory node's tag, for the `referenced_by` names.
fn prune_names(
    conn: &rusqlite::Connection,
) -> Result<BTreeMap<crate::node::NodeUuid, String>, ApiError> {
    Ok(node_directory::all(conn)
        .map_err(crate::store::connection::store_error)?
        .into_iter()
        .map(|row| (row.node_uuid, row.tag.to_string()))
        .collect())
}

/// One [`sync_status::LiveNode`] per node with an active link.
fn live_nodes(links: &[LinkStatus]) -> Vec<sync_status::LiveNode> {
    links
        .iter()
        .filter(|link| link.mode == LinkMode::Active)
        .map(|link| sync_status::LiveNode {
            node: link.node,
            linked_since: link.since.0,
            last_rx: link.last_rx.0,
            app_seen_at: link.app_rx.map(|at| at.0),
            version: link.peer_version.clone(),
            tx: link.tx,
            rx: link.rx,
            stalled: link.stalled,
        })
        .collect()
}

impl SyncAdmin {
    /// `@MACHINE` resolution (spec D1, 4.3): the machine against the node
    /// directory, then the location by the home node's daemon over its link.
    /// A read: nothing is written or attributed here or there.
    fn resolve_address(
        &self,
        machine: &str,
        target: crate::sync::frame::AddrTarget,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        use crate::protocol::sync_admin::{AddressReply, RemoteAddress};
        use crate::sync::coordinator::remote_addr::RemoteAnswer;
        use crate::sync::frame::AddrOutcome;
        let (rows, own) = self.store.sync_read(budget, |conn, _| {
            let rows = sync_status::machine_rows(conn)?;
            let own: String = conn
                .query_row("SELECT id FROM local_node", [], |r| r.get(0))
                .map_err(crate::store::connection::store_error)?;
            Ok((rows, own))
        })?;
        let found = crate::protocol::address::match_machine(machine, &rows);
        let node = match found.as_slice() {
            [] => {
                return Ok(SyncReply::Address(AddressReply::UnknownMachine {
                    known: rows.iter().take(KNOWN_MACHINES_LISTED).cloned().collect(),
                }));
            }
            [one] => (*one).clone(),
            many => {
                return Ok(SyncReply::Address(AddressReply::AmbiguousMachine {
                    matches: many.iter().map(|row| (*row).clone()).collect(),
                }));
            }
        };
        if node.uuid == own {
            return self.self_address(&target, &rows, budget);
        }
        let uuid: crate::node::NodeUuid = node
            .uuid
            .parse()
            .map_err(|_| ApiError::new(ErrorCode::StoreCorrupt, "malformed node directory uuid"))?;
        let remaining = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        let wait = std::time::Duration::from_millis(remaining.min(REMOTE_RESOLVE_WAIT_MS));
        let exact = match &target {
            crate::sync::frame::AddrTarget::Seat { location }
                if crate::protocol::ids::SeatId::parse(location).is_ok() =>
            {
                Some(location.clone())
            }
            _ => None,
        };
        let answer = self.runtime.block_on(self.sync.resolve_remote(
            uuid,
            target,
            wait,
            &budget.cancellation,
        ));
        let answer = match answer {
            None => RemoteAddress::Timeout {
                secs: wait.as_secs(),
            },
            Some(RemoteAnswer::NotLinked | RemoteAnswer::LinkLost) => {
                let last = self.store.sync_read(budget, |conn, _| {
                    crate::store::sync_contacts::get(conn, &uuid)
                })?;
                RemoteAddress::Unreachable {
                    last_contact_at: last.and_then(|c| i64::try_from(c.last_contact_at).ok()),
                }
            }
            Some(RemoteAnswer::NoCap { cap, version }) => RemoteAddress::NoCap { version, cap },
            Some(RemoteAnswer::Answered(outcome)) => match &outcome {
                AddrOutcome::Seat { seat, .. }
                    if !self.wait_replicated(seat, &uuid, REPLICA_WAIT_MS, budget)? =>
                {
                    RemoteAddress::NotReplicated { seat: seat.clone() }
                }
                _ => RemoteAddress::Answered { outcome },
            },
        };
        // N15: an exact seat ID this node holds as that node's nonretired seat
        // stands on replicated state when the node cannot answer.
        let answer = match (&answer, exact) {
            (
                RemoteAddress::Unreachable { .. }
                | RemoteAddress::NoCap { .. }
                | RemoteAddress::Timeout { .. },
                Some(seat),
            ) if self.holds_remote_seat(&seat, &uuid, budget)? => RemoteAddress::Replicated {
                seat,
                reason: match &answer {
                    RemoteAddress::Unreachable { .. } => "unreachable".into(),
                    RemoteAddress::NoCap { .. } => "no addr.resolve.v1".into(),
                    _ => "no answer in time".into(),
                },
            },
            _ => answer,
        };
        Ok(SyncReply::Address(AddressReply::Remote { node, answer }))
    }

    /// Whether this node holds `seat` as `home`'s nonretired remote seat.
    fn holds_remote_seat(
        &self,
        seat: &str,
        home: &crate::node::NodeUuid,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        use rusqlite::OptionalExtension;
        self.store.sync_read(budget, |conn, _| {
            conn.query_row(
                "SELECT 1 FROM seats WHERE id=?1 AND home_node=?2 AND is_local=0 AND state='remote'",
                [seat, home.to_string().as_str()],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
            .map_err(crate::store::connection::store_error)
        })
    }

    /// N15: forward a launch request to `machine` and report its state there.
    /// A `launched` state is reported only once the seat replicated here as
    /// that machine's.
    fn remote_launch(
        &self,
        machine: &str,
        request: Option<crate::sync::frame::LaunchRequest>,
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        use crate::protocol::sync_admin::{LaunchReply, RemoteLaunchAnswer};
        use crate::sync::coordinator::remote_launch::LaunchAnswer;
        use crate::sync::frame::LaunchState;
        let (rows, own) = self.store.sync_read(budget, |conn, _| {
            let rows = sync_status::machine_rows(conn)?;
            let own: String = conn
                .query_row("SELECT id FROM local_node", [], |r| r.get(0))
                .map_err(crate::store::connection::store_error)?;
            Ok((rows, own))
        })?;
        let found = crate::protocol::address::match_machine(machine, &rows);
        let node = match found.as_slice() {
            [] => {
                return Ok(SyncReply::Launch(LaunchReply::UnknownMachine {
                    known: rows.iter().take(KNOWN_MACHINES_LISTED).cloned().collect(),
                }));
            }
            [one] => (*one).clone(),
            many => {
                return Ok(SyncReply::Launch(LaunchReply::AmbiguousMachine {
                    matches: many.iter().map(|row| (*row).clone()).collect(),
                }));
            }
        };
        if node.uuid == own {
            return Ok(SyncReply::Launch(LaunchReply::SelfNode));
        }
        let uuid: crate::node::NodeUuid = node
            .uuid
            .parse()
            .map_err(|_| ApiError::new(ErrorCode::StoreCorrupt, "malformed node directory uuid"))?;
        let Some(request) = request else {
            return Ok(SyncReply::Launch(LaunchReply::Remote {
                node,
                answer: RemoteLaunchAnswer::Unknown,
            }));
        };
        let remaining = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        let wait = std::time::Duration::from_millis(remaining.min(REMOTE_RESOLVE_WAIT_MS));
        let answer = self.runtime.block_on(self.sync.launch_remote(
            uuid,
            request,
            wait,
            &budget.cancellation,
        ));
        let answer = match answer {
            None => RemoteLaunchAnswer::Timeout {
                secs: wait.as_secs(),
            },
            Some(LaunchAnswer::NotLinked | LaunchAnswer::LinkLost) => {
                let last = self.store.sync_read(budget, |conn, _| {
                    crate::store::sync_contacts::get(conn, &uuid)
                })?;
                RemoteLaunchAnswer::Unreachable {
                    last_contact_at: last.and_then(|c| i64::try_from(c.last_contact_at).ok()),
                }
            }
            Some(LaunchAnswer::NoCap { version }) => RemoteLaunchAnswer::NoCap { version },
            Some(LaunchAnswer::Answered(state)) => match &state {
                LaunchState::Launched { seat, .. }
                    if !self.wait_replicated(seat, &uuid, REPLICA_WAIT_MS, budget)? =>
                {
                    RemoteLaunchAnswer::NotReplicated { seat: seat.clone() }
                }
                _ => RemoteLaunchAnswer::Answered { state },
            },
        };
        Ok(SyncReply::Launch(LaunchReply::Remote { node, answer }))
    }

    /// N15: ask `seat`'s home node whether it can take a handoff now. Only a
    /// seat this node holds as remote and nonretired is asked about.
    fn check_remote_seat(&self, seat: &str, budget: &CallBudget) -> Result<SyncReply, ApiError> {
        use rusqlite::OptionalExtension;
        let home: Option<String> = self.store.sync_read(budget, |conn, _| {
            conn.query_row(
                "SELECT home_node FROM seats WHERE id=?1 AND is_local=0 AND state='remote'",
                [seat],
                |r| r.get(0),
            )
            .optional()
            .map_err(crate::store::connection::store_error)
        })?;
        let Some(home) = home else {
            return Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "seat is not a nonretired remote seat here",
            ));
        };
        self.resolve_address(
            &home,
            crate::sync::frame::AddrTarget::Check {
                seat: seat.to_owned(),
            },
            budget,
        )
    }

    /// `LOCATION@<this node>`: resolved locally by the caller, unless it is
    /// the exact ID of a seat this node does not home (retired included).
    fn self_address(
        &self,
        target: &crate::sync::frame::AddrTarget,
        rows: &[crate::protocol::sync_admin::MachineRow],
        budget: &CallBudget,
    ) -> Result<SyncReply, ApiError> {
        use crate::protocol::sync_admin::AddressReply;
        use rusqlite::OptionalExtension;
        let crate::sync::frame::AddrTarget::Seat { location } = target else {
            return Ok(SyncReply::Address(AddressReply::SelfNode));
        };
        if crate::protocol::ids::SeatId::parse(location).is_err() {
            return Ok(SyncReply::Address(AddressReply::SelfNode));
        }
        let home: Option<String> = self.store.sync_read(budget, |conn, _| {
            conn.query_row(
                "SELECT home_node FROM seats WHERE id=?1 AND is_local=0",
                [location.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(crate::store::connection::store_error)
        })?;
        Ok(SyncReply::Address(match home {
            None => AddressReply::SelfNode,
            Some(home) => AddressReply::SeatElsewhere {
                seat: location.clone(),
                home: rows.iter().find(|row| row.uuid == home).cloned(),
            },
        }))
    }

    /// Whether `seat` is (or within [`REPLICA_WAIT_MS`] becomes) a seat this
    /// node holds as `home`'s (TRUST-POLICY N14).
    fn wait_replicated(
        &self,
        seat: &str,
        home: &crate::node::NodeUuid,
        wait_ms: u64,
        budget: &CallBudget,
    ) -> Result<bool, ApiError> {
        use rusqlite::OptionalExtension;
        const POLL_MS: u64 = 100;
        let start = self.clock.monotonic_now().0;
        // Bounded by count too, so a stalled clock cannot keep it polling.
        for _ in 0..=wait_ms / POLL_MS {
            let held = self.store.sync_read(budget, |conn, _| {
                conn.query_row(
                    "SELECT 1 FROM seats WHERE id=?1 AND home_node=?2 AND is_local=0",
                    [seat, home.to_string().as_str()],
                    |_| Ok(()),
                )
                .optional()
                .map(|found| found.is_some())
                .map_err(crate::store::connection::store_error)
            })?;
            let now = self.clock.monotonic_now().0;
            if held || now.saturating_sub(start) >= wait_ms || now >= budget.deadline.0 {
                return Ok(held);
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        }
        Ok(false)
    }
}
