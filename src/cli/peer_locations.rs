//! Advisory, batched participant locations for local text presentation.
//! Labels come from the daemon's per-host read, keyed `(host, target)`
//! (`ParticipantLocation.remote`, F6); the CLI reads no host itself.
use crate::{
    ports::LocalClient,
    protocol::{
        commands::{Command, ParticipantLocationsQuery},
        ids::{SeatId, ThreadId},
        results::{CommandResult, ContinuityStatus, ParticipantLocation},
        time::CallBudget,
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static HINTS: RefCell<HashMap<SeatId,String>> = RefCell::new(HashMap::new());
}
#[derive(Default)]
pub(crate) struct Prepared {
    pub rows: Vec<(SeatId, String)>,
    pub unavailable: bool,
}
/// Run `run` with location hints enabled: the claim has a host, so labels
/// can exist.
pub(crate) fn with_active<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE.with(|cell| cell.set(self.0));
        }
    }
    let _restore = Restore(ACTIVE.with(|cell| cell.replace(true)));
    run()
}
pub(crate) fn active() -> bool {
    ACTIVE.with(Cell::get)
}
pub(crate) fn with_hints<T>(ready: &Prepared, run: impl FnOnce() -> T) -> T {
    struct Restore(HashMap<SeatId, String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            HINTS.with(|cell| *cell.borrow_mut() = std::mem::take(&mut self.0));
        }
    }
    let hints = ready.rows.iter().cloned().collect();
    let _restore = Restore(HINTS.with(|cell| std::mem::replace(&mut *cell.borrow_mut(), hints)));
    run()
}
pub(crate) fn hint(seat: &SeatId) -> Option<String> {
    HINTS.with(|cell| cell.borrow().get(seat).cloned())
}

pub(crate) fn prepare<C: LocalClient + ?Sized>(
    result: &CommandResult,
    thread: Option<ThreadId>,
    client: &C,
    budget: &CallBudget,
) -> Prepared {
    let participants = match result {
        CommandResult::Participants(page) => &page.items,
        CommandResult::Thread(details) => &details.participants.items,
        _ => return Prepared::default(),
    };
    if participants.is_empty() {
        return Prepared::default();
    }
    let supported = matches!(client.call(Command::Capabilities,budget),Ok(CommandResult::Capabilities(list)) if list.capabilities.iter().any(|name|name==crate::protocol::capabilities::PARTICIPANT_LOCATIONS));
    if !supported {
        return Prepared {
            unavailable: true,
            ..Default::default()
        };
    }
    let Some(thread) = thread else {
        return Prepared {
            unavailable: true,
            ..Default::default()
        };
    };
    let seats = participants
        .iter()
        .map(|participant| participant.seat.clone())
        .collect();
    let mappings = match client.call(
        Command::ParticipantLocations(ParticipantLocationsQuery { thread, seats }),
        budget,
    ) {
        Ok(CommandResult::ParticipantLocations(mappings)) => mappings,
        _ => {
            return Prepared {
                unavailable: true,
                ..Default::default()
            };
        }
    };
    let mut mappings: HashMap<_, _> = mappings
        .into_iter()
        .map(|mapping| (mapping.seat.clone(), mapping))
        .collect();
    Prepared {
        rows: participants
            .iter()
            .map(|participant| {
                let location = mappings.remove(&participant.seat);
                let hint = location
                    .as_ref()
                    .map_or_else(|| "location unavailable".into(), format_location);
                (participant.seat.clone(), hint)
            })
            .collect(),
        unavailable: false,
    }
}
fn format_location(location: &ParticipantLocation) -> String {
    let target = location.target.as_ref();
    let target_text = target.map_or_else(
        || "target unavailable".into(),
        |target| target.as_str().to_owned(),
    );
    match location.continuity {
        ContinuityStatus::Retired => return format!("{target_text} retired; labels unavailable"),
        ContinuityStatus::Unresolved => {
            return format!("{target_text} unresolved; labels unavailable");
        }
        // A remote seat (ht-xg88): its home node's labels, last known once
        // its presence is not current; none from an older daemon or a node
        // without `seat.status.v1`.
        ContinuityStatus::Remote => {
            let labels = location.remote.as_ref().and_then(|d| {
                d.labels(|part| super::human::one_line(part, false, 64))
                    .map(|labels| (d, labels))
            });
            return match labels {
                Some((directory, labels)) => format!(
                    "{labels} [advisory{}]",
                    if directory.last_known() {
                        ", last known"
                    } else {
                        ""
                    }
                ),
                None => "remote; labels unavailable".into(),
            };
        }
        ContinuityStatus::Resolved => {}
    }
    // A local seat: the daemon's labels for its own host, only while the
    // seat still holds the pane (the daemon applies the terminal and
    // incarnation match). Without them every label is `-`.
    let shown = |part: &Option<String>| {
        part.as_deref().map_or_else(
            || "-".to_owned(),
            |part| super::human::one_line(part, false, 64),
        )
    };
    match location.remote.as_ref().filter(|d| d.has_labels()) {
        Some(d) => format!(
            "{} / {} / {target_text} ({}/{}/{}) [advisory]",
            shown(&d.workspace),
            shown(&d.tab),
            shown(&d.workspace),
            shown(&d.tab),
            shown(&d.pane),
        ),
        None => format!("- / - / {target_text} (-) [advisory]"),
    }
}

/// Advisory additions consume only the space left by the canonical encoding.
/// Existing rows, byte limits and continuation argv are retained exactly.
pub(crate) fn append_compact(mut base: Vec<u8>, ready: &Prepared, max_bytes: usize) -> Vec<u8> {
    let mut append = |line: String| {
        if base.len().saturating_add(line.len()) <= max_bytes {
            base.extend_from_slice(line.as_bytes());
        }
    };
    if ready.unavailable {
        append("location unavailable\n".into());
    }
    for (seat, location) in &ready.rows {
        append(format!("location {}: {location}\n", seat.as_str()));
    }
    base
}

#[cfg(test)]
#[path = "../../tests/cli/peer_locations.rs"]
mod tests;
