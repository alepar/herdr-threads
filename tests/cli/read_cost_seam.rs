//! B1 read-cost seam (ht-p03.12.13): one human read session (`show`,
//! `participants`, a 100-message `read` with clipped previews) through the
//! counting fake `LocalClient` makes the per-invocation counts the leaves
//! claim: ht-p03.12.3 (single-connection self marker), ht-p03.12.7 (batched
//! names) and ht-p03.12.8 (`full_bodies`). Mounted from `src/cli/follow.rs`
//! because the human read renderer and the nick cache are private to it.

use super::*;
use crate::{
    app::SystemClock,
    cli::{CliAction, LazyConnection, commands::parse_argv, run_caller_scoped},
    protocol::{
        ids::MessageId,
        pagination::{Consistency, StopReason},
        results::{
            ContinuityStatus, MembershipStatus, Participant, SeatSummary, ThreadDetails,
            ThreadSummary,
        },
        time::{MonoInstant, UtcMillis},
    },
    test_support::counting_client::{CallKind, CountingLocalClient, DaemonVintage},
};
use std::sync::Mutex;

const AUTHORS: usize = 10;
const MESSAGES: u64 = 100;
const SNIPPET: usize = 256;
/// The pane the human types in; it hosts `seat(0)`.
const CALLER_PANE: &str = "w1:p0";

fn thread() -> ThreadId {
    ThreadId::new("thread-Ab12Cd34")
}

fn seat(index: usize) -> SeatId {
    SeatId::new(format!("seat-Author{index:02}"))
}

fn target(index: usize) -> HostTargetId {
    HostTargetId::new(format!("w1:p{index}"))
}

fn page<T>(items: Vec<T>) -> Page<T> {
    Page {
        items,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: MESSAGES,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

fn seat_summary(index: usize) -> SeatSummary {
    SeatSummary {
        seat: seat(index),
        continuity: ContinuityStatus::Resolved,
        target: Some(target(index)),
        generation: 1,
        created_at: UtcMillis(1_790_771_000_000),
        retired_at: None,
    }
}

fn participant(index: usize) -> Participant {
    Participant {
        seat: seat(index),
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
    }
}

/// Every tenth message is longer than a preview; its body still fits the page.
fn body(sequence: u64) -> String {
    let len = if sequence.is_multiple_of(10) { 600 } else { 40 };
    format!("body-{sequence}-")
        .chars()
        .cycle()
        .take(len)
        .collect()
}

fn message(sequence: u64, full_bodies: bool) -> MessageSummary {
    let body = body(sequence);
    let clipped = !full_bodies && body.chars().count() > SNIPPET;
    MessageSummary {
        message: MessageId::new(format!("msg-{sequence}")),
        thread: thread(),
        author: Some(seat(usize::try_from(sequence).unwrap() % AUTHORS)),
        event_author: None,
        kind: MessageKind::Ordinary,
        sequence,
        created_at: UtcMillis(1_790_771_696_000 + i64::try_from(sequence).unwrap() * 1_000),
        actor_label: None,
        preview_data: if clipped {
            body.chars().take(SNIPPET).collect()
        } else {
            body
        },
        preview_omitted: clipped,
        preview_detail_argv: None,
    }
}

fn thread_details() -> ThreadDetails {
    ThreadDetails {
        summary: ThreadSummary {
            thread: thread(),
            managed_owner: None,
            topic_data: "topic".into(),
            topic_omitted: false,
            topic_detail_argv: None,
            archived: false,
            orphaned: false,
            message_count: MESSAGES,
            created_at: UtcMillis(0),
            ordinary_count: MESSAGES,
            system_count: 0,
            joined_count: u64::try_from(AUTHORS).unwrap(),
        },
        goal_data: String::new(),
        created_at: UtcMillis(0),
        participant_count: u64::try_from(AUTHORS).unwrap(),
        participants: page((0..AUTHORS).map(participant).collect()),
        pending_receipt_count: 0,
        pending_receipts_argv: vec![],
    }
}

/// The Current-vintage daemon behind every command of the session. Records
/// the `full_bodies` flag of every History request.
fn answer(
    command: &Command,
    seen_full_bodies: &Mutex<Vec<bool>>,
) -> Result<CommandResult, ApiError> {
    Ok(match command {
        Command::Seats(query) => {
            let seats = (0..AUTHORS)
                .filter(|index| query.target.as_ref().is_none_or(|t| *t == target(*index)))
                .map(seat_summary)
                .collect();
            CommandResult::Seats(page(seats))
        }
        Command::Participants(_) => {
            CommandResult::Participants(page((0..AUTHORS).map(participant).collect()))
        }
        Command::Thread(_) => CommandResult::Thread(thread_details()),
        Command::History(query) => {
            seen_full_bodies.lock().unwrap().push(query.full_bodies);
            CommandResult::History(page(
                (1..=MESSAGES)
                    .map(|n| message(n, query.full_bodies))
                    .collect(),
            ))
        }
        other => {
            return Err(ApiError::new(
                ErrorCode::Unsupported,
                format!("unexpected {other:?}"),
            ));
        }
    })
}

/// The fake behind a handle the lazy connection owns while the test keeps
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

struct CountingPanes(Arc<CountingLocalClient>);

impl PaneNameSource for CountingPanes {
    fn pane_names(&self, _: &CallBudget) -> Result<Vec<PaneName>, ApiError> {
        self.0.record_pane_names();
        Ok((0..AUTHORS)
            .map(|index| PaneName {
                target: target(index),
                label: Some(format!("pane-name-{index}")),
                tab_label: None,
                tab_pane_count: 1,
            })
            .collect())
    }
}

/// One CLI invocation: a fresh counting fake (so every command is counted on
/// its own) and one lazy connection, opened where production opens it.
struct Invocation {
    fake: Arc<CountingLocalClient>,
    seen_full_bodies: Arc<Mutex<Vec<bool>>>,
    paths: InstancePaths,
    clock: Arc<dyn Clock>,
}

impl Invocation {
    fn new() -> Self {
        let seen_full_bodies = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen_full_bodies);
        let fake = Arc::new(CountingLocalClient::scripted(
            move |command| answer(command, &log),
            DaemonVintage::Current,
        ));
        let root = std::env::temp_dir().join(format!("read-cost-seam-{}", uuid::Uuid::new_v4()));
        let paths = InstancePaths {
            instance_dir: root.clone(),
            socket_path: root.join("sock"),
            lock_path: root.join("lock"),
            descriptor_path: root.join("descriptor"),
            locator_path: root.join("locator"),
            namespace_path: root.join("namespace"),
            database_path: root.join("db"),
            locator: "test".into(),
        };
        Self {
            fake,
            seen_full_bodies,
            paths,
            clock: Arc::new(SystemClock::new()),
        }
    }

    fn connection(
        &self,
    ) -> LazyConnection<Shared, impl Fn() -> Result<(uuid::Uuid, Shared), RunError>> {
        let fake = Arc::clone(&self.fake);
        LazyConnection::new(move || {
            fake.connect();
            Ok((uuid::Uuid::nil(), Shared(Arc::clone(&fake))))
        })
    }

    /// `show` or `participants` as `herdr-threads` runs them from a pane.
    fn run_scoped(&self, argv: &[&str]) -> String {
        let budget = || CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        };
        let parsed = parse_argv(argv.iter().copied()).expect("argv parses");
        let connection = self.connection();
        let mut out = Vec::new();
        // No pane-agent read reaches a host here (agent contexts only).
        let runtime = crate::daemon::paths::RuntimeContext {
            state_dir: self.paths.instance_dir.clone(),
            host_endpoint: self.paths.instance_dir.join("host.sock"),
            herdr_bin: None,
        };
        let rest = run_caller_scoped(
            parsed,
            Some(CALLER_PANE),
            &runtime,
            &self.paths,
            &connection,
            &self.clock,
            &budget,
            &mut out,
        )
        .expect("the read succeeds");
        assert!(rest.is_none(), "{argv:?} ran inside the scoped path");
        String::from_utf8(out).unwrap()
    }

    /// Human `read`: the IRC transcript, as `run` renders it.
    fn run_read(&self, argv: &[&str]) -> String {
        let parsed = parse_argv(argv.iter().copied()).expect("argv parses");
        let CliAction::Wire(Command::History(query)) = parsed.action else {
            panic!("read parses to a history query");
        };
        let connection = self.connection();
        let (_, client) = connection.get().expect("connects");
        let mut cache = NickCache::with_pane_source(
            Box::new(CountingPanes(Arc::clone(&self.fake))),
            &self.paths,
            uuid::Uuid::nil(),
            &self.clock,
        );
        let mut out = Vec::new();
        render_history_with(
            client,
            self.fake.capabilities().supports(HISTORY_FULL_BODIES),
            query,
            &parsed.output,
            &mut cache,
            &mut out,
            &Style::plain(),
        )
        .expect("history renders");
        String::from_utf8(out).unwrap()
    }
}

/// Kills: a leaf's saving lost at the seam. `show`/`participants` opening a
/// second connection or paging seats twice (leaf 12.3); `read` resolving a name
/// per author or fetching a body per clipped preview (leaves 12.7, 12.8),
/// or not sending `full_bodies` to a daemon that advertises it.
#[test]
fn one_human_read_session_meets_every_leaf_bound() {
    for argv in [
        &["herdr-threads", "thread", "show", "thread-Ab12Cd34"][..],
        &["herdr-threads", "participants", "thread-Ab12Cd34"][..],
    ] {
        let run = Invocation::new();
        run.run_scoped(argv);
        assert_eq!(run.fake.connections(), 1, "{argv:?}: one connection");
        assert!(
            run.fake.total_calls() <= 2,
            "{argv:?}: at most one seats page plus the read, got {}",
            run.fake.total_calls()
        );
        assert_eq!(run.fake.calls(CallKind::Seats), 1, "{argv:?}");
    }

    let run = Invocation::new();
    let text = run.run_read(&[
        "herdr-threads",
        "read",
        "thread-Ab12Cd34",
        "--recent",
        "100",
    ]);
    assert_eq!(run.fake.connections(), 1, "read: one connection");
    assert_eq!(run.fake.calls(CallKind::History), 1, "read: one History");
    assert_eq!(run.fake.calls(CallKind::Message), 0, "read: no Message");
    assert!(run.fake.calls(CallKind::Participants) <= 1);
    assert_eq!(run.fake.calls(CallKind::PaneNames), 1, "one pane snapshot");
    assert_eq!(run.fake.calls(CallKind::SeatInspect), 0);
    assert_eq!(*run.seen_full_bodies.lock().unwrap(), vec![true]);
    // Every clipped preview was inlined whole, and every author is named.
    let unwrapped: String = text.split_whitespace().collect();
    assert!(unwrapped.contains(&body(100)), "{text}");
    assert!(text.contains("pane-name-3"), "{text}");
}
