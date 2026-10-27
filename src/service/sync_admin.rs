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
    EvidenceRow, LinkRow, MAX_STATUS_LOGS, RepairRow, ReplayReply, StatusReply, SyncCommand,
    SyncReply,
};
use crate::protocol::time::{CallBudget, Cancellation, Clock};
use crate::service::fair_writer::FairWriter;
use crate::store::SqliteStore;
use crate::store::incarnations::{self, ForkOrigin};
use crate::store::sync_status::{self, node_label, resolve_node};
use crate::store::{node_directory, sync_dial_targets};
use crate::sync::LogKey;
use crate::sync::coordinator::{LinkMode, LinkStatus, LogFn, Stall, SyncHandle};
use crate::sync::incarnations::IncarnationsSink;
use crate::sync::resync::{self, RequestEnv};

/// Appended to the two-copies remedy: the dial targets a `node_collision`
/// stopped are not restarted by the clearance.
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
                // linked: the stored rows give last-known values only.
                let live: Vec<sync_status::LiveNode> = self
                    .runtime
                    .block_on(self.sync.snapshot(budget))
                    .map(|snapshot| live_nodes(&snapshot.links))
                    .unwrap_or_default();
                let (rows, nodes) = self.store.sync_read(budget, |conn, _| {
                    Ok((
                        sync_status::peers(conn, now)?,
                        sync_status::nodes(conn, now, &live)?,
                    ))
                })?;
                Ok(SyncReply::Peers {
                    rows,
                    discovery: self.discovery_state(),
                    nodes,
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
        let (logs, omitted, repair, directory, targets) =
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
                Ok((logs, omitted, repair, directory, targets))
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
                            .and_then(|target| target.last_error.clone())
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
                    app_rx: Some(link.app_rx.0),
                    subscriptions: link.subscriptions,
                    peer_lag: link.tx.map(|(acked, head)| head.saturating_sub(acked)),
                    stalled: stall_names(link.stalled)
                        .iter()
                        .map(|s| (*s).to_owned())
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

/// `app`, `tx`, `rx`: the parts of a link that stalled, in that order.
fn stall_names(stall: Stall) -> Vec<&'static str> {
    [("app", stall.app), ("tx", stall.tx), ("rx", stall.rx)]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect()
}

/// One [`sync_status::LiveNode`] per node with an active, open link.
fn live_nodes(links: &[LinkStatus]) -> Vec<sync_status::LiveNode> {
    links
        .iter()
        .filter(|link| link.mode == LinkMode::Active)
        .map(|link| sync_status::LiveNode {
            node: link.node,
            linked_since: link.since.0,
            last_rx: link.last_rx.0,
            app_seen_at: link.app_rx.0,
            version: link.peer_version.clone(),
            tx: link.tx,
            rx: link.rx,
            stalled: stall_names(link.stalled),
        })
        .collect()
}
