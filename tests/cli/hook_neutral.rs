//! Hook runner behavior that is decided by the invoking hook's registration, not by a
//! harness name (core-sweep D10): the pane seat lookup and the parse-failure report.
use super::*;
use crate::protocol::{
    commands::Command,
    ids::SeatId,
    pagination::{Consistency, Page, StopReason},
    results::{ContinuityStatus, MappingStatus, SeatInspection, SeatSummary},
    time::UtcMillis,
};

fn page<T>(items: Vec<T>) -> Page<T> {
    Page {
        items,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

struct Seats(Vec<SeatSummary>);
impl LocalClient for Seats {
    crate::default_output_local_client!();
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        match command {
            Command::Seats(_) => Ok(CommandResult::Seats(page(self.0.clone()))),
            Command::SeatInspect(q) => {
                let summary = self.0.iter().find(|s| s.seat == q.seat).unwrap().clone();
                Ok(CommandResult::SeatInspect(SeatInspection {
                    mapping: MappingStatus {
                        state: summary.continuity,
                        target: summary.target.clone(),
                        detail_argv: None,
                    },
                    summary,
                    hold: None,
                    retirement: None,
                    open_binding: None,
                    history: page(vec![]),
                }))
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

fn seat(continuity: ContinuityStatus) -> SeatSummary {
    SeatSummary {
        seat: SeatId::new("s1"),
        continuity,
        target: Some(HostTargetId::new("pane")),
        generation: 7,
        created_at: UtcMillis(0),
        retired_at: None,
        active: true,
        availability: None,
        bindings: Vec::new(),
    }
}

fn pick(harness: Harness, seats: Vec<SeatSummary>) -> Result<PaneSeat, Failure> {
    find_seat_for(
        harness,
        &Seats(seats),
        &HostTargetId::new("pane"),
        Instant::now() + Duration::from_secs(5),
        &SystemClock::new(),
    )
}

/// Kills: a lookup pinned to one harness (a Codex or third-harness hook would
/// then resolve through another harness's selection), or one that changes
/// what a Claude hook finds.
#[test]
fn every_hooks_own_harness_finds_its_resolved_binding() {
    let registry = crate::harness::registry::builtins();
    for id in ["claude", "codex", "hermes"] {
        let harness: Harness =
            crate::harness::registry::OccupantHarness::Agent(registry.agent(id).unwrap()).into();
        match pick(harness, vec![seat(ContinuityStatus::Resolved)]) {
            Ok(PaneSeat::Resolved(found, generation)) => {
                assert_eq!((found.as_str(), generation), ("s1", 7), "{id}");
            }
            _ => panic!("{id}: the resolved binding was not found"),
        }
        assert!(
            matches!(pick(harness, vec![]), Ok(PaneSeat::Unowned)),
            "{id}: no seat on the pane"
        );
        assert!(
            matches!(
                pick(harness, vec![seat(ContinuityStatus::Unresolved)]),
                Ok(PaneSeat::HeldOrUnresolved(_))
            ),
            "{id}: an unresolved mapping is held, never allocated around"
        );
    }
}

/// Kills: a report for an adapter outside the v1 evidence wire (the daemon
/// would refuse its harness name), a report named by anything but the
/// registered id, and a report suppressed for a v1 adapter.
#[test]
fn parse_failure_report_follows_the_legacy_contract_predicate() {
    use crate::test_support::counting_client::{CallKind, CountingLocalClient, DaemonVintage};
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Cancellation::default(),
    };
    let registry = crate::harness::registry::builtins();
    for (id, sent) in [("claude", true), ("codex", true), ("hermes", false)] {
        let harness: Harness =
            crate::harness::registry::OccupantHarness::Agent(registry.agent(id).unwrap()).into();
        assert_eq!(
            registry
                .by_id(registry.agent(id).unwrap())
                .unwrap()
                .legacy_contract_id()
                .is_some(),
            sent,
            "{id}: the predicate this test pins"
        );
        let client = CountingLocalClient::scripted(
            move |command| match command {
                Command::HookParseFailure(report) => {
                    assert_eq!(report.harness, id);
                    Ok(CommandResult::HookParseFailureRecorded)
                }
                other => panic!("unexpected command {other:?}"),
            },
            DaemonVintage::Current,
        );
        assert_eq!(
            report_parse_failure(
                &client,
                &client.capabilities(),
                harness,
                &ContextError::Invalid,
                &budget
            ),
            sent,
            "{id}"
        );
        assert_eq!(
            client.calls(CallKind::HookParseFailure),
            u32::from(sent),
            "{id}"
        );
    }
}

// Adapter completion is gated by both real writer operations, not by preparation.
#[test]
fn adapter_completion_requires_successful_stdout_write_and_flush() {
    struct Writer {
        fail_write: bool,
        fail_flush: bool,
        bytes: Vec<u8>,
        flushed: bool,
    }
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail_write {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_flush {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.flushed = true;
            Ok(())
        }
    }
    for (fail_write, fail_flush, completed) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut writer = Writer {
            fail_write,
            fail_flush,
            bytes: vec![],
            flushed: false,
        };
        let delivered = write_and_flush(b"native hook output", &mut writer);
        let mut calls = 0;
        complete_after_emit(delivered, || {
            assert!(writer.flushed);
            assert_eq!(writer.bytes, b"native hook output");
            calls += 1;
        });
        assert_eq!(calls, usize::from(completed));
    }
}

#[test]
fn default_hook_step_completion_is_inert() {
    struct Existing;
    impl HookStep for Existing {
        fn run(&self, _: &HookStepContext<'_>) -> Vec<u8> {
            b"old native bytes".to_vec()
        }
    }
    struct NoCalls;
    impl HookStepDaemon for NoCalls {
        fn call(&self, _: Command) -> Result<CommandResult, ApiError> {
            panic!("default completion must be inert")
        }
    }
    // The default completion accepts no authority or state from its payload.
    Existing.complete(b"opaque", &NoCalls, Path::new("/unused"), Instant::now());
}
