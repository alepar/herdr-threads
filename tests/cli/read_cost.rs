//! B1 read cost (ht-p03.12.3, W6-R1): `show` and `participants` with a caller
//! pane use one daemon connection and one seat page plus the read itself.

use crate::{
    app::SystemClock,
    cli::{LazyConnection, RunError, commands::parse_argv, run_caller_scoped},
    daemon::paths::InstancePaths,
    ports::LocalClient,
    protocol::{
        commands::Command,
        ids::{HostTargetId, SeatId, ThreadId},
        output::OutputSpec,
        pagination::{Consistency, Page, StopReason},
        results::{
            ApiError, CommandResult, ContinuityStatus, ErrorCode, MembershipStatus, Participant,
            SeatSummary, ThreadDetails, ThreadSummary,
        },
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    test_support::counting_client::{CallKind, CountingLocalClient, DaemonVintage},
};
use std::sync::{Arc, Mutex};

/// The fake behind a handle the lazy connection can own while the test keeps
/// reading its counters.
struct Shared(Arc<CountingLocalClient>);

impl LocalClient for Shared {
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.0.call(command, budget)
    }

    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.0.call_with_output(command, output, budget)
    }
}

const PANE: &str = "pane-7";

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

fn seat(name: &str) -> SeatSummary {
    seat_in(name, ContinuityStatus::Resolved)
}

fn seat_in(name: &str, continuity: ContinuityStatus) -> SeatSummary {
    SeatSummary {
        seat: SeatId::new(name),
        continuity,
        target: Some(HostTargetId::new(PANE)),
        generation: 1,
        created_at: UtcMillis(0),
        retired_at: None,
    }
}

fn participants() -> Page<Participant> {
    page(vec![Participant {
        seat: SeatId::new("seat-a"),
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
    }])
}

fn thread_details() -> ThreadDetails {
    ThreadDetails {
        summary: ThreadSummary {
            last_activity: None,
            name: None,
            thread: ThreadId::new("t1"),
            managed_owner: None,
            topic_data: "topic".into(),
            topic_omitted: false,
            topic_detail_argv: None,
            archived: false,
            orphaned: false,
            message_count: 0,
            created_at: UtcMillis(0),
            ordinary_count: 0,
            system_count: 0,
            joined_count: 1,
        },
        goal_data: String::new(),
        created_at: UtcMillis(0),
        participant_count: 1,
        participants: participants(),
        pending_receipt_count: 0,
        pending_receipts_argv: vec![],
    }
}

struct Run {
    fake: Arc<CountingLocalClient>,
    seen: Arc<Mutex<Vec<Command>>>,
    result: Result<Vec<u8>, RunError>,
}

fn run(argv: &[&str], seats: Vec<SeatSummary>) -> Run {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let fake = Arc::new(CountingLocalClient::scripted(
        move |command: &Command| {
            log.lock().unwrap().push(command.clone());
            match command {
                Command::Seats(q) => {
                    let start: usize = q.page.cursor.as_deref().map_or(0, |c| c.parse().unwrap());
                    let end = (start + q.page.limit as usize).min(seats.len());
                    let mut out = page(seats[start..end].to_vec());
                    out.next_cursor = (end < seats.len()).then(|| end.to_string());
                    out.has_more = out.next_cursor.is_some();
                    if out.has_more {
                        out.stop_reason = StopReason::Rows;
                    }
                    Ok(CommandResult::Seats(out))
                }
                Command::Participants(_) => Ok(CommandResult::Participants(participants())),
                Command::Thread(_) => Ok(CommandResult::Thread(thread_details())),
                other => Err(ApiError::unsupported(format!("unexpected {other:?}"))),
            }
        },
        DaemonVintage::Current,
    ));
    let root = std::env::temp_dir().join(format!("read-cost-unused-{}", uuid::Uuid::new_v4()));
    let paths = InstancePaths {
        instance_dir: root.clone(),
        socket_path: root.join("sock"),
        lock_path: root.join("lock"),
        descriptor_path: root.join("descriptor"),
        locator_path: root.join("locator"),
        namespace_path: root.join("namespace"),
        database_path: root.join("db"),
        locator: String::new(),
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    // The one place this test's CLI opens a connection.
    let connection = LazyConnection::new(|| {
        fake.connect();
        Ok((uuid::Uuid::nil(), Shared(Arc::clone(&fake))))
    });
    let budget = || CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let parsed = parse_argv(argv.iter().copied()).expect("argv parses");
    let mut out = Vec::new();
    let runtime = crate::daemon::paths::RuntimeContext {
        state_dir: root.clone(),
        host_endpoint: root.join("host.sock"),
        herdr_bin: None,
    };
    let result = run_caller_scoped(
        parsed,
        Some(PANE),
        &runtime,
        &paths,
        &connection,
        &clock,
        &budget,
        &mut out,
    )
    .map(|_| out);
    drop(connection);
    Run { fake, seen, result }
}

fn seats_query(seen: &Mutex<Vec<Command>>) -> crate::protocol::commands::SeatsQuery {
    seen.lock()
        .unwrap()
        .iter()
        .find_map(|command| match command {
            Command::Seats(query) => Some(query.clone()),
            _ => None,
        })
        .expect("a seats query was sent")
}

fn caller_sent(seen: &Mutex<Vec<Command>>) -> Option<SeatId> {
    seen.lock()
        .unwrap()
        .iter()
        .find_map(|command| match command {
            Command::Participants(q) => q.caller.clone(),
            Command::Thread(q) => q.caller.clone(),
            _ => None,
        })
}

#[test]
fn show_and_participants_use_one_connection_and_two_calls() {
    for (argv, read) in [
        (
            &["herdr-threads", "thread", "show", "t1"][..],
            CallKind::Other,
        ),
        (
            &["herdr-threads", "participants", "t1"][..],
            CallKind::Participants,
        ),
    ] {
        let run = run(argv, vec![seat("seat-a")]);
        run.result.as_ref().expect("read succeeds");
        assert_eq!(run.fake.connections(), 1, "{argv:?}");
        assert_eq!(run.fake.calls(CallKind::Seats), 1, "{argv:?}");
        assert_eq!(
            run.fake.total_calls(),
            2,
            "{argv:?}: one seats page plus the read"
        );
        if read != CallKind::Other {
            assert_eq!(run.fake.calls(read), 1, "{argv:?}");
        }
        let query = seats_query(&run.seen);
        assert_eq!(query.target, Some(HostTargetId::new(PANE)));
        assert_eq!(query.page.limit, crate::cli::PANE_SEAT_PAGE_LIMIT);
        // The seat found on that one page marks the caller's own row.
        assert_eq!(
            caller_sent(&run.seen),
            Some(SeatId::new("seat-a")),
            "{argv:?}"
        );
    }
}

#[test]
fn pane_mapped_to_two_seats_still_errors() {
    // A seat-defaulting read refuses an ambiguous pane.
    let run = run(
        &["herdr-threads", "inbox"],
        vec![seat("seat-a"), seat("seat-b")],
    );
    let Err(RunError::Api(error)) = &run.result else {
        panic!(
            "expected an api error, got {:?}",
            run.result.as_ref().map(|_| ())
        );
    };
    assert_eq!(error.code, ErrorCode::TargetUnresolved);
    assert!(
        error.detail.contains("maps to more than one seat"),
        "{}",
        error.detail
    );
    assert_eq!(run.fake.connections(), 1);
    assert_eq!(run.fake.total_calls(), 1, "no read after the refusal");
    // `show` only marks the caller's row: an ambiguous pane leaves it unmarked.
    let show = run_show_ambiguous();
    assert_eq!(caller_sent(&show.seen), None);
    assert_eq!(show.fake.connections(), 1);
}

fn run_show_ambiguous() -> Run {
    run(
        &["herdr-threads", "thread", "show", "t1"],
        vec![seat("seat-a"), seat("seat-b")],
    )
}

// Kills: one limit-2 seat page, so two older unresolved seats on a reused
// pane id truncate the resolved seat away (final review S5).
#[test]
fn resolved_seat_behind_two_unresolved_seats_is_found() {
    let seats = vec![
        seat_in("seat-old1", ContinuityStatus::Unresolved),
        seat_in("seat-old2", ContinuityStatus::Unresolved),
        seat("seat-new"),
    ];
    // show marks the caller's own row with the pane's resolved seat
    let show = run(&["herdr-threads", "thread", "show", "t1"], seats.clone());
    show.result.as_ref().expect("read succeeds");
    assert_eq!(caller_sent(&show.seen), Some(SeatId::new("seat-new")));
    // a seat-defaulting read finds it too (no "no seat for pane" refusal)
    let inbox = run(&["herdr-threads", "inbox"], seats);
    assert!(
        !matches!(&inbox.result, Err(RunError::Api(e)) if e.code == ErrorCode::TargetUnresolved),
        "{:?}",
        inbox.result.as_ref().map(|_| ())
    );
}

// Kills: paging that stops at PAGE_LIMIT, so a resolved seat behind a full
// page of unresolved ones is never read.
#[test]
fn resolved_seat_past_a_full_page_of_unresolved_seats_is_found() {
    let mut seats: Vec<SeatSummary> = (0..8)
        .map(|i| seat_in(&format!("seat-old{i}"), ContinuityStatus::Unresolved))
        .collect();
    seats.push(seat("seat-new"));
    let show = run(&["herdr-threads", "thread", "show", "t1"], seats);
    show.result.as_ref().expect("read succeeds");
    assert_eq!(caller_sent(&show.seen), Some(SeatId::new("seat-new")));
    assert_eq!(show.fake.calls(CallKind::Seats), 2);
}

// Kills: an ambiguity check that only sees the first page.
#[test]
fn two_resolved_seats_behind_unresolved_ones_are_ambiguous() {
    let seats = vec![
        seat_in("seat-old1", ContinuityStatus::Unresolved),
        seat_in("seat-old2", ContinuityStatus::Unresolved),
        seat("seat-a"),
        seat_in("seat-old3", ContinuityStatus::Unresolved),
        seat("seat-b"),
    ];
    let run = run(&["herdr-threads", "inbox"], seats);
    let Err(RunError::Api(error)) = &run.result else {
        panic!(
            "expected ambiguity, got {:?}",
            run.result.as_ref().map(|_| ())
        );
    };
    assert_eq!(error.code, ErrorCode::TargetUnresolved);
    assert!(
        error.detail.contains("maps to more than one seat"),
        "{}",
        error.detail
    );
}
