//! Sender-visible wake outcome: after a send or invitation that carries priority-0 items, one
//! stderr line per local recipient whose harness has no idle wake at all (`wake=no_wake`), once per
//! (sender, seat) while it stays so. Only that lasting, registry-decided block warns at send time;
//! what a wake attempt met (`unreadable`, `focused`) depends on an attempt this send has not made
//! yet, so it is `doctor`'s. The line rides in the sender's own command output, so it is no item
//! and wakes nobody. Model-visible: `warning: <seat> wake=no_wake\n`, 23 bytes plus the seat id
//! (35 bytes for a 12-byte seat). Every shipped harness has an idle wake, so no production seat
//! produces it today and the skill does not explain it; the path stays as a harness-neutral
//! diagnostic for a harness without one (`synthetic_fourth` in test-support builds, covered end
//! to end in `tests/integration/config_smoke.rs`; `idle_wake_classification_of_the_builtins`
//! asks for the skill sentence back when a shipped harness loses its wake).

use crate::ports::LocalClient;
use crate::protocol::{
    authority::CallerClaim,
    capabilities::WAKE_STATUS_V1,
    commands::{Command, WakeStatusQuery},
    ids::{MessageId, SeatId},
    results::{CommandResult, SeatWakeStatus, WakeBlock},
    time::CallBudget,
};
use std::io::Write;

/// The warning lines for `rows`: a `no_wake` seat not yet reported to this sender.
pub(crate) fn lines(rows: &[SeatWakeStatus]) -> String {
    rows.iter()
        .filter(|row| row.first && row.wake == Some(WakeBlock::NoWake))
        .map(|row| format!("warning: {} wake=no_wake\n", row.seat.as_str()))
        .collect()
}

/// Best effort: a daemon without the capability, or any failure, prints nothing. `seats` are the
/// priority-0 recipients the CLI names (ACK recipients, an invitee, a p0 direct target);
/// `message` a published priority-0 thread send, whose delivery recipients the daemon finds.
pub(crate) fn show<C: LocalClient + ?Sized>(
    client: &C,
    budget: &CallBudget,
    claim: &CallerClaim,
    seats: Vec<SeatId>,
    message: Option<MessageId>,
    err: &mut dyn Write,
) {
    if (seats.is_empty() && message.is_none())
        || !client.supports_capability(WAKE_STATUS_V1, budget)
    {
        return;
    }
    let query = WakeStatusQuery {
        all: false,
        seats,
        message,
        observer: Some(claim.seat.clone()),
    };
    if let Ok(CommandResult::WakeStatus(rows)) = client.call(Command::WakeStatus(query), budget) {
        let _ = err.write_all(lines(&rows).as_bytes());
        let _ = err.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(seat: &str, wake: Option<WakeBlock>, first: bool) -> SeatWakeStatus {
        SeatWakeStatus {
            seat: SeatId::new(seat),
            harness: Some("h".into()),
            wake,
            first,
        }
    }

    /// Only a first-reported `no_wake` warns: attempt diagnostics and repeats never do.
    #[test]
    fn only_a_first_no_wake_is_a_warning_line() {
        let rows = [
            row("s1", Some(WakeBlock::NoWake), true),
            row("s2", Some(WakeBlock::NoWake), false),
            row("s3", Some(WakeBlock::Unreadable), true),
            row("s4", Some(WakeBlock::Focused), true),
            row("s5", Some(WakeBlock::Unknown), true),
            row("s6", None, true),
        ];
        assert_eq!(lines(&rows), "warning: s1 wake=no_wake\n");
    }
}
