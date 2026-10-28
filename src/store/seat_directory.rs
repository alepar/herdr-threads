//! One seat's directory entry (ht-xg88 phase 3).
//!
//! Responsibility (seat_directory)
//! - Owns: [`directory`], the entry `seat list`, `thread participants`,
//!   participant locations and `seat inspect` show, and [`matches`], the
//!   `seat list` filter.
//! - Hides: that a local seat's entry comes from the local binding and
//!   observation (`local_seat_status`) and a remote seat's from its home
//!   node's snapshot through the one freshness rule
//!   (`remote_seat_status::presence_of`).
//! - Must not: feed any decision. Every value is a display hint (TRUST-POLICY
//!   C1, N13). A local seat's entry carries no labels (the CLI shows its own
//!   host read); `--label` matches it by the daemon's host read, the one the
//!   seat publisher makes (ht-hldp).
//! - Role: read by `store::queries`.

use rusqlite::{Connection, OptionalExtension};

use super::connection::store_error;
use super::local_seat_status::TargetLabels;
use super::remote_seat_status::{self, RemotePresence};
use super::sync_status::node_label;
use super::{local_seat_status, node_directory, sync_contacts};
use crate::node::NodeUuid;
use crate::protocol::commands::SeatFilter;
use crate::protocol::results::{ApiError, SeatDirectory};
use crate::protocol::time::UtcMillis;

fn at(ms: u64) -> UtcMillis {
    UtcMillis(i64::try_from(ms).unwrap_or(i64::MAX))
}

/// `seat`'s entry at `now`; `None` for an unknown seat.
pub fn directory(
    db: &Connection,
    seat: &str,
    now: UtcMillis,
) -> Result<Option<Box<SeatDirectory>>, ApiError> {
    let row: Option<(String, bool)> = db
        .query_row(
            "SELECT home_node, is_local FROM seats WHERE id=?1",
            [seat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((home, is_local)) = row else {
        return Ok(None);
    };
    let home_uuid: Option<NodeUuid> = home.parse().ok();
    let node_row = match home_uuid {
        Some(uuid) => node_directory::get(db, uuid).map_err(store_error)?,
        None => None,
    };
    let node = node_label(
        node_row.as_ref().map(|r| r.tag.to_string()).as_deref(),
        &home,
    );
    let machine = node_row.and_then(|r| r.host_label);
    if is_local {
        let Some(local) = local_seat_status::local_seat(db, seat)? else {
            return Ok(None);
        };
        return Ok(Some(Box::new(SeatDirectory {
            node,
            machine,
            state: local_seat_status::presence(&local).to_owned(),
            since: None,
            harness: local.harness,
            activity_at: local.last_transition_at.map(UtcMillis),
            seen_at: None,
            workspace: None,
            tab: None,
            pane: None,
        })));
    }
    let now_ms = u64::try_from(now.0).unwrap_or(0);
    let stored = remote_seat_status::get(db, seat)?;
    let reach = match home_uuid {
        Some(uuid) => sync_contacts::reach(db, &uuid, now_ms)?,
        None => sync_contacts::NodeReach::Unreachable { since: None },
    };
    let (state, since, activity) =
        match remote_seat_status::presence_of(stored.as_ref(), reach, false, now_ms) {
            RemotePresence::Fresh {
                presence,
                activity_at,
            } => (presence, None, activity_at),
            RemotePresence::Unreachable { since } => ("unreachable".to_owned(), since, None),
            RemotePresence::Stale { built_at } => ("stale".to_owned(), Some(built_at), None),
            RemotePresence::CloneSuspected => ("clone_suspected".to_owned(), None, None),
            RemotePresence::Unknown => ("-".to_owned(), None, None),
        };
    let stored_activity = stored.as_ref().and_then(|row| row.seat.activity_at);
    Ok(Some(Box::new(SeatDirectory {
        node,
        machine,
        state,
        since: since.map(at),
        harness: stored.as_ref().and_then(|row| row.seat.harness.clone()),
        // Last known even when the presence is not current.
        activity_at: activity.or(stored_activity).map(at),
        seen_at: stored.as_ref().map(|row| at(row.built_at)),
        workspace: stored.as_ref().and_then(|row| row.seat.workspace.clone()),
        tab: stored.as_ref().and_then(|row| row.seat.tab.clone()),
        pane: stored.as_ref().and_then(|row| row.seat.pane.clone()),
    })))
}

/// Whether `entry` (of a local seat when `local`, whose current host labels
/// are `host_labels`) passes `filter`: every set criterion,
/// case-insensitively for names and labels.
pub fn matches(
    filter: &SeatFilter,
    entry: Option<&SeatDirectory>,
    local: bool,
    host_labels: Option<&TargetLabels>,
) -> bool {
    if filter.is_empty() {
        return true;
    }
    if filter.local && !local {
        return false;
    }
    let Some(entry) = entry else {
        return !(filter.node.is_some()
            || filter.machine.is_some()
            || filter.harness.is_some()
            || filter.label.is_some());
    };
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    if filter
        .node
        .as_deref()
        .is_some_and(|node| !eq(node, &entry.node))
    {
        return false;
    }
    if let Some(machine) = &filter.machine
        && !eq(machine, &entry.node)
        && !entry.machine.as_deref().is_some_and(|m| eq(machine, m))
    {
        return false;
    }
    if let Some(harness) = &filter.harness
        && !entry.harness.as_deref().is_some_and(|h| eq(harness, h))
    {
        return false;
    }
    if let Some(label) = &filter.label {
        let needle = label.to_lowercase();
        let host = host_labels
            .into_iter()
            .flat_map(|l| [&l.workspace, &l.tab, &l.pane]);
        let hit = [&entry.workspace, &entry.tab, &entry.pane]
            .into_iter()
            .flatten()
            .chain(host)
            .any(|text| text.to_lowercase().contains(&needle));
        if !hit {
            return false;
        }
    }
    true
}
