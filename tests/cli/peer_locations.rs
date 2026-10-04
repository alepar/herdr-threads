use super::*;
use crate::{
    ports::LocalClient,
    protocol::{
        commands::Command,
        ids::*,
        pagination::{Consistency, Page, StopReason},
        results::{
            ApiError, CapabilityList, CommandResult, ContinuityStatus, MembershipStatus,
            Participant,
        },
        time::*,
    },
};
use std::cell::Cell;

struct Client {
    calls: std::sync::atomic::AtomicUsize,
    supported: bool,
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
fn labels(incarnation: &str) -> Vec<SeatHostLabels> {
    (0..3)
        .map(|i| SeatHostLabels {
            target: HostTargetId::new(format!("w1:p{i}")),
            terminal: format!("term{i}"),
            incarnation: Some(incarnation.into()),
            workspace_id: "w1".into(),
            workspace_label: Some("same \u{1b}[31mworkspace".into()),
            tab_id: "w1:t1".into(),
            tab_label: Some("same".into()),
            pane_label: Some("same\u{202e}".repeat(100)),
        })
        .collect()
}
#[test]
fn participant_locations_batch_once_escape_and_bound_hints_with_honest_fallbacks() {
    let client = Client {
        calls: std::sync::atomic::AtomicUsize::new(0),
        supported: true,
    };
    let snapshots = Cell::new(0);
    let ready = prepare(
        &result(),
        Some(ThreadId::new("t1")),
        &client,
        &CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        || {
            snapshots.set(snapshots.get() + 1);
            Ok(labels("server-old"))
        },
    );
    assert_eq!(client.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(snapshots.get(), 1);
    assert!(
        ready.rows[0].1.contains("w1:p0")
            && ready.rows[0].1.contains("w1:t1")
            && ready.rows[0].1.contains("w1")
    );
    assert!(!ready.rows[0].1.contains('\u{1b}') && !ready.rows[0].1.contains('\u{202e}'));
    assert!(ready.rows[0].1.len() < 700);
    assert!(ready.rows[1].1.contains("unresolved") && !ready.rows[1].1.contains("workspace"));
    assert!(ready.rows[2].1.contains("retired") && !ready.rows[2].1.contains("workspace"));
}
#[test]
fn participant_locations_restart_and_host_failure_do_not_label_reused_targets() {
    for failed in [false, true] {
        let client = Client {
            calls: std::sync::atomic::AtomicUsize::new(0),
            supported: true,
        };
        let ready = prepare(
            &result(),
            Some(ThreadId::new("t1")),
            &client,
            &CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
            || {
                if failed {
                    Err(ApiError::new(
                        crate::protocol::results::ErrorCode::HostUnavailable,
                        "offline",
                    ))
                } else {
                    Ok(labels("server-new"))
                }
            },
        );
        assert!(ready.rows[0].1.contains("w1:p0") && ready.rows[0].1.contains("unavailable"));
        assert!(!ready.rows[0].1.contains("workspace"));
    }
}
#[test]
fn participant_locations_older_daemon_skips_host_and_keeps_bounded_canonical_text() {
    let client = Client {
        calls: std::sync::atomic::AtomicUsize::new(0),
        supported: false,
    };
    let ready = prepare(
        &result(),
        Some(ThreadId::new("t1")),
        &client,
        &CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        || panic!("older daemon has no location lookup"),
    );
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
    let client = Client {
        calls: std::sync::atomic::AtomicUsize::new(0),
        supported: true,
    };
    let ready = prepare(
        &result(),
        Some(ThreadId::new("t1")),
        &client,
        &CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        || {
            let mut snapshot = labels("server-old");
            snapshot[0].workspace_id = "w\n\u{1b}".repeat(2000);
            snapshot[0].tab_id = "t\r\u{202e}".repeat(2000);
            Ok(snapshot)
        },
    );
    let hint = &ready.rows[0].1;
    assert!(!hint.contains(['\n', '\r', '\u{1b}', '\u{202e}']));
    assert!(hint.len() < 2000, "unbounded hint: {}", hint.len());
    assert!(hint.contains("w1:p0") && hint.contains("advisory"));
}
