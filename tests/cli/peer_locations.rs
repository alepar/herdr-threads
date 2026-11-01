use super::*;
use crate::{
    ports::LocalClient,
    protocol::{
        commands::Command,
        ids::*,
        pagination::{Consistency, Page, StopReason},
        results::{
            ApiError, CapabilityList, CommandResult, ContinuityStatus, MembershipStatus,
            Participant, SeatDirectory,
        },
        time::*,
    },
};

struct Client {
    calls: std::sync::atomic::AtomicUsize,
    supported: bool,
    /// The daemon's workspace, tab and pane labels for the resolved seat;
    /// `None` is a daemon that cannot read the host (or predates the read).
    labels: Option<(String, String, String)>,
}
fn entry(labels: &(String, String, String)) -> SeatDirectory {
    SeatDirectory {
        node: "n".into(),
        machine: None,
        state: "idle".into(),
        since: None,
        harness: None,
        activity_at: None,
        seen_at: None,
        workspace: Some(labels.0.clone()),
        tab: Some(labels.1.clone()),
        pane: Some(labels.2.clone()),
        name: None,
    }
}
fn same_labels() -> Option<(String, String, String)> {
    Some((
        "same \u{1b}[31mworkspace".into(),
        "same".into(),
        "same\u{202e}".repeat(100),
    ))
}
impl LocalClient for Client {
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match command {
            Command::Capabilities => Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: if self.supported {
                    vec![crate::protocol::capabilities::PARTICIPANT_LOCATIONS.into()]
                } else {
                    vec![]
                },
            })),
            Command::ParticipantLocations(q) => {
                assert_eq!(q.seats.len(), 3);
                Ok(CommandResult::ParticipantLocations(
                    q.seats
                        .into_iter()
                        .enumerate()
                        .map(|(i, seat)| ParticipantLocation {
                            seat,
                            target: Some(HostTargetId::new(format!("w1:p{i}"))),
                            continuity: match i {
                                1 => ContinuityStatus::Unresolved,
                                2 => ContinuityStatus::Retired,
                                _ => ContinuityStatus::Resolved,
                            },
                            terminal: Some(format!("term{i}")),
                            incarnation: Some("server-old".into()),
                            remote: (i == 0).then(|| self.labels.as_ref().map(entry)).flatten(),
                        })
                        .collect(),
                ))
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
fn result() -> CommandResult {
    CommandResult::Participants(Page {
        items: (0..3)
            .map(|i| Participant {
                seat: SeatId::new(format!("s{i}")),
                is_self: false,
                requirement: None,
                episode: 1,
                joined: true,
                retired: false,
                physical_state: MembershipStatus::Joined,
                effective_state: MembershipStatus::Joined,
                joined_at: None,
                left_at: None,
                retirement_cutover: None,
                cleanup_state: None,
                accepted_invitation: None,
                active: false,
                bindings: Vec::new(),
                availability: None,
                home_unreachable: None,
                directory: None,
            })
            .collect(),
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 3,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    })
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}
fn client(supported: bool, labels: Option<(String, String, String)>) -> Client {
    Client {
        calls: std::sync::atomic::AtomicUsize::new(0),
        supported,
        labels,
    }
}
#[test]
fn participant_locations_batch_once_escape_and_bound_hints_with_honest_fallbacks() {
    let client = client(true, same_labels());
    let ready = prepare(&result(), Some(ThreadId::new("t1")), &client, &budget());
    assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(
        ready.rows[0].1.contains("w1:p0") && ready.rows[0].1.contains("same"),
        "{}",
        ready.rows[0].1
    );
    assert!(!ready.rows[0].1.contains('\u{1b}') && !ready.rows[0].1.contains('\u{202e}'));
    assert!(ready.rows[0].1.len() < 700);
    assert!(ready.rows[1].1.contains("unresolved") && !ready.rows[1].1.contains("workspace"));
    assert!(ready.rows[2].1.contains("retired") && !ready.rows[2].1.contains("workspace"));
}
// Kills: a resolved seat the daemon sent no labels for (host down, a reused
// target, a daemon that predates the read) shown with another pane's labels
// or with invented ones instead of `-`.
#[test]
fn participant_locations_without_daemon_labels_show_dash() {
    let client = client(true, None);
    let ready = prepare(&result(), Some(ThreadId::new("t1")), &client, &budget());
    assert_eq!(ready.rows[0].1, "- / - / w1:p0 (-) [advisory]");
}
#[test]
fn participant_locations_older_daemon_skips_host_and_keeps_bounded_canonical_text() {
    let client = client(false, None);
    let ready = prepare(&result(), Some(ThreadId::new("t1")), &client, &budget());
    assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(ready.unavailable);
    let base = b"participants\ns0 joined\ns1 joined\ns2 joined\n".to_vec();
    let compact = append_compact(base.clone(), &ready, base.len() + 24);
    assert!(compact.len() <= base.len() + 24);
    assert!(compact.starts_with(&base));
    assert_eq!(append_compact(base.clone(), &ready, base.len()), base);
}

#[test]
fn participant_locations_host_parent_ids_are_escaped_and_bounded() {
    let client = client(
        true,
        Some((
            "w\n\u{1b}".repeat(2000),
            "t\r\u{202e}".repeat(2000),
            "p".into(),
        )),
    );
    let ready = prepare(&result(), Some(ThreadId::new("t1")), &client, &budget());
    let hint = &ready.rows[0].1;
    assert!(!hint.contains(['\n', '\r', '\u{1b}', '\u{202e}']));
    assert!(hint.len() < 2000, "unbounded hint: {}", hint.len());
    assert!(hint.contains("w1:p0") && hint.contains("advisory"));
}

#[path = "host_topologies.rs"]
mod host_topologies;
