//! Handoff out of and into a named seat (SA14, M11), in-crate from `src/cli/handoff.rs`.

use super::*;
use crate::{
    cli::caller_location::claim_recovery_env,
    protocol::{
        ids::{HostLocator, NAMED_HOST_LOCATOR},
        pagination::{Consistency, Page, StopReason},
        results::{ContinuityStatus, SeatSummary},
        time::{CallBudget, UtcMillis},
    },
};
use std::sync::Mutex;

fn named_locator() -> HostLocator {
    HostLocator::parse(NAMED_HOST_LOCATOR).unwrap()
}

struct SeatsClient {
    /// The seat the daemon answers for `target` on the named host, if any.
    named: Option<(&'static str, &'static str)>,
    sent: Mutex<Vec<Command>>,
}

impl LocalClient for SeatsClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.sent.lock().unwrap().push(command.clone());
        let Command::Seats(query) = command else {
            return Err(ApiError::new(ErrorCode::Unsupported, "only seat lookups"));
        };
        let items = match (self.named, &query.host, &query.target) {
            (Some((seat, target)), Some(host), Some(asked))
                if host.is_named_host() && asked.as_str() == target =>
            {
                vec![SeatSummary {
                    seat: SeatId::new(seat),
                    continuity: ContinuityStatus::Resolved,
                    target: Some(HostTargetId::new(target)),
                    generation: 1,
                    created_at: UtcMillis(0),
                    retired_at: None,
                    active: true,
                    bindings: Vec::new(),
                    availability: None,
                    directory: None,
                }]
            }
            _ => Vec::new(),
        };
        Ok(CommandResult::Seats(Page {
            items,
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        }))
    }
}

fn spec() -> OutputSpec {
    OutputSpec {
        format: crate::protocol::output::OutputFormat::Text,
        context: Default::default(),
    }
}

fn clock() -> crate::cli::SystemClock {
    crate::cli::SystemClock::new()
}

// Kills: a handoff into a named seat reaching the Herdr launch (a target created or any
// non-lookup command sent), and an M11 text other than the pinned bytes.
#[test]
fn handoff_into_named_seat_refused_m11() {
    let client = SeatsClient {
        named: Some(("snNamed000001", "reviewer")),
        sent: Mutex::new(Vec::new()),
    };
    let error = refuse_named_target(&client, &spec(), &HostTargetId::new("reviewer"), &clock())
        .unwrap_err();
    let RunError::Api(api) = error else {
        panic!("{error:?}")
    };
    assert_eq!(api.code, ErrorCode::InvalidRequest);
    assert_eq!(format!("{}\n", api.detail), include_str!("golden/m11.txt"));
    let sent = client.sent.lock().unwrap();
    assert!(
        sent.iter().all(|c| matches!(c, Command::Seats(_))),
        "only lookups, no target created: {sent:?}"
    );
}

// Kills: a refusal for a pane target, and for a name no named seat holds (the Herdr launch
// reports those as before).
#[test]
fn handoff_into_other_targets_is_not_refused() {
    let client = SeatsClient {
        named: Some(("snNamed000001", "reviewer")),
        sent: Mutex::new(Vec::new()),
    };
    for target in ["w1:p2", "nobody"] {
        refuse_named_target(&client, &spec(), &HostTargetId::new(target), &clock())
            .unwrap_or_else(|e| panic!("{target}: {e:?}"));
    }
}

// Kills: a recovery hint that sends a named caller's retry out of the pane (the `HERDR_PANE_ID`
// assignment with the name as a pane id), a named hint that does not name the seat, and a
// changed Pane-site hint.
#[test]
fn named_recovery_hint_round_trip() {
    let hint = claim_recovery_env(&named_locator(), "reviewer");
    assert_eq!(hint, vec!["HERDR_THREADS_SEAT_NAME=reviewer".to_owned()]);
    // Running the hint resolves to the same seat: the env it sets locates a Named caller with
    // that name.
    let env = crate::cli::caller_location::CallerEnv::from_lookup(|name| {
        hint.iter()
            .find_map(|word| word.strip_prefix(&format!("{name}=")))
            .map(std::ffi::OsString::from)
    });
    assert_eq!(
        env.named().map(|n| n.as_str().to_owned()),
        Some("reviewer".into())
    );
    let host = HostLocator::parse("/run/herdr/herdr.sock").unwrap();
    assert_eq!(
        claim_recovery_env(&host, "w1:p2"),
        vec!["HERDR_PANE_ID=w1:p2".to_owned()]
    );
    // The claim's host decides, not the shape of the target: a name-shaped target at a Herdr host
    // is still a pane.
    assert_eq!(
        claim_recovery_env(&host, "reviewer"),
        vec!["HERDR_PANE_ID=reviewer".to_owned()]
    );
}

// Kills: resolving the Herdr endpoint eagerly (a Pane caller paying the `herdr` query), a
// named-host claim path taken as a Herdr server, and a named caller left with no endpoint.
#[test]
fn handoff_from_named_seat_resolves_herdr_endpoint_lazily() {
    let calls = std::cell::Cell::new(0);
    let lazy = || {
        calls.set(calls.get() + 1);
        Ok(std::path::PathBuf::from("/run/herdr-test/herdr.sock"))
    };
    let pane = herdr_endpoint(Some(std::path::Path::new("/run/herdr/herdr.sock")), lazy).unwrap();
    assert_eq!(pane, std::path::Path::new("/run/herdr/herdr.sock"));
    assert_eq!(
        calls.get(),
        0,
        "a pane caller's endpoint is never looked up"
    );
    for none in [None, Some(std::path::Path::new(NAMED_HOST_LOCATOR))] {
        let host = herdr_endpoint(none, lazy).unwrap();
        assert_eq!(host, std::path::Path::new("/run/herdr-test/herdr.sock"));
    }
    assert_eq!(calls.get(), 2);
}
