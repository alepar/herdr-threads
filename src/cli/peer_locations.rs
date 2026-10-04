//! Advisory, batched participant locations for local text presentation.
use crate::{
    host::{native::NativeCli, observation::SeatHostLabels},
    ports::LocalClient,
    protocol::{
        commands::{Command, ParticipantLocationsQuery},
        ids::{SeatId, ThreadId},
        results::{ApiError, CommandResult, ContinuityStatus, ParticipantLocation},
        time::CallBudget,
    },
};
use std::{cell::RefCell, collections::HashMap};

thread_local! {
    static SOURCE: RefCell<Option<NativeCli>> = const { RefCell::new(None) };
    static HINTS: RefCell<HashMap<SeatId,String>> = RefCell::new(HashMap::new());
}
#[derive(Default)]
pub(crate) struct Prepared {
    pub rows: Vec<(SeatId, String)>,
    pub unavailable: bool,
}
pub(crate) fn with_source<T>(source: NativeCli, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<NativeCli>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SOURCE.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SOURCE.with(|cell| cell.borrow_mut().replace(source)));
    run()
}
pub(crate) fn active() -> bool {
    SOURCE.with(|cell| cell.borrow().is_some())
}
pub(crate) fn snapshot(budget: &CallBudget) -> Result<Vec<SeatHostLabels>, ApiError> {
    SOURCE.with(|cell| {
        cell.borrow()
            .as_ref()
            .ok_or_else(|| {
                ApiError::new(
                    crate::protocol::results::ErrorCode::HostUnavailable,
                    "host label source unavailable",
                )
            })?
            .participant_labels(budget)
    })
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
    read_snapshot: impl FnOnce() -> Result<Vec<SeatHostLabels>, ApiError>,
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
    let labels = read_snapshot().unwrap_or_default();
    let labels: HashMap<_, _> = labels
        .into_iter()
        .map(|label| (label.target.clone(), label))
        .collect();
    let mut mappings: HashMap<_, _> = mappings
        .into_iter()
        .map(|mapping| (mapping.seat.clone(), mapping))
        .collect();
    Prepared {
        rows: participants
            .iter()
            .map(|participant| {
                let location = mappings.remove(&participant.seat);
                let hint = location.as_ref().map_or_else(
                    || "location unavailable".into(),
                    |location| format_location(location, &labels),
                );
                (participant.seat.clone(), hint)
            })
            .collect(),
        unavailable: false,
    }
}
fn format_location(
    location: &ParticipantLocation,
    labels: &HashMap<crate::protocol::ids::HostTargetId, SeatHostLabels>,
) -> String {
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
        ContinuityStatus::Resolved => {}
    }
    let current = target
        .and_then(|target| labels.get(target))
        .filter(|label| {
            location.terminal.as_deref() == Some(label.terminal.as_str())
                && location.incarnation.as_ref().is_some()
                && location.incarnation == label.incarnation
        });
    match current {
        Some(current) => format!(
            "{} / {} / {} ({}) [advisory]",
            super::human::one_line(&current.workspace_id, false, 64),
            super::human::one_line(&current.tab_id, false, 64),
            target_text,
            super::irc::relative_pane_nick(current, None)
        ),
        None => format!("{target_text} resolved; labels unavailable"),
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
