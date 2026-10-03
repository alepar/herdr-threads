//! Human `read` names cost (ht-p03.12.7, spec D6 Wave 27): author names are
//! resolved once per page, not once per author. Mounted from
//! `src/cli/follow.rs`; counted with the counting fake `LocalClient`.

use super::*;
use crate::{
    app::SystemClock,
    protocol::{
        ids::{HostTargetId, MessageId, ThreadId},
        output::{ContinuationContext, OutputFormat},
        pagination::{Consistency, StopReason},
        results::{
            ContinuityStatus, MappingStatus, MembershipStatus, Participant, SeatInspection,
            SeatSummary,
        },
        time::UtcMillis,
    },
    test_support::counting_client::{CallKind, CountingLocalClient, DaemonVintage},
};
use std::sync::Mutex;

const AUTHORS: usize = 10;

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
        high_water_ordinal: 1,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

fn seat_summary(seat: SeatId, target: Option<HostTargetId>) -> SeatSummary {
    SeatSummary {
        seat,
        continuity: ContinuityStatus::Resolved,
        target,
        generation: 1,
        created_at: UtcMillis(1_790_771_000_000),
        retired_at: None,
    }
}

fn participant(seat: SeatId) -> Participant {
    Participant {
        seat,
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

fn message(sequence: u64, author: SeatId) -> MessageSummary {
    MessageSummary {
        message: MessageId::new(format!("msg-{sequence}")),
        thread: thread(),
        author: Some(author),
        event_author: None,
        author_role: None,
        relays_user: false,
        author_role_backfilled: false,
        kind: MessageKind::Ordinary,
        sequence,
        // 2026-09-30 12:34:56Z
        created_at: UtcMillis(1_790_771_696_000 + i64::try_from(sequence).unwrap() * 1_000),
        actor_label: None,
        preview_data: format!("message number {sequence}"),
        preview_omitted: false,
        preview_detail_argv: None,
    }
}

/// Authors 0..8 own a pane with a label, author 8 a pane whose single-pane
/// tab carries the label, author 9 a pane with no name at all.
fn panes() -> Vec<PaneName> {
    (0..AUTHORS)
        .map(|index| PaneName {
            target: target(index),
            label: (index < 8).then(|| format!("pane-name-{index}")),
            tab_label: (index == 8).then(|| "tab-name-8".to_owned()),
            tab_pane_count: 1,
        })
        .collect()
}

struct CountingPanes {
    fake: Arc<CountingLocalClient>,
    panes: Vec<PaneName>,
}

impl PaneNameSource for CountingPanes {
    fn pane_names(&self, _: &CallBudget) -> Result<Vec<PaneName>, ApiError> {
        self.fake.record_pane_names();
        Ok(self.panes.clone())
    }
}

/// The daemon: `messages` is the history page, `seats` the mapped seats and
/// `participants` the thread's members. A seat in neither is only
/// answerable through `SeatInspect`.
struct Daemon {
    messages: Vec<MessageSummary>,
    seats: Vec<SeatSummary>,
    participants: Vec<SeatId>,
    /// Seats `SeatInspect` knows beyond the list pages (retired or left).
    inspectable: Vec<SeatSummary>,
    history_calls: Mutex<u32>,
}

impl Daemon {
    fn answer(&self, command: &Command) -> Result<CommandResult, ApiError> {
        Ok(match command {
            Command::History(_) => {
                *self.history_calls.lock().unwrap() += 1;
                CommandResult::History(page(self.messages.clone()))
            }
            Command::Participants(_) => CommandResult::Participants(page(
                self.participants.iter().cloned().map(participant).collect(),
            )),
            Command::Seats(_) => CommandResult::Seats(page(self.seats.clone())),
            Command::SeatInspect(query) => {
                let summary = self
                    .seats
                    .iter()
                    .chain(&self.inspectable)
                    .find(|summary| summary.seat == query.seat)
                    .cloned()
                    .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "no such seat"))?;
                CommandResult::SeatInspect(SeatInspection {
                    mapping: MappingStatus {
                        state: summary.continuity,
                        target: summary.target.clone(),
                        detail_argv: None,
                    },
                    summary,
                    hold: None,
                    retirement: None,
                    open_binding: None,
                    history: page(Vec::new()),
                })
            }
            _ => {
                return Err(ApiError::new(ErrorCode::Unsupported, "not served"));
            }
        })
    }
}

struct Fixture {
    fake: Arc<CountingLocalClient>,
    cache: NickCache,
    daemon: Arc<Daemon>,
}

fn fixture(daemon: Daemon) -> Fixture {
    let daemon = Arc::new(daemon);
    let script = Arc::clone(&daemon);
    let fake = Arc::new(CountingLocalClient::scripted(
        move |command| script.answer(command),
        DaemonVintage::Current,
    ));
    let root = std::path::PathBuf::from("/nonexistent-herdr-threads-read-cost");
    let paths = InstancePaths {
        instance_dir: root.clone(),
        socket_path: root.join("s"),
        lock_path: root.join("l"),
        descriptor_path: root.join("d"),
        locator_path: root.join("loc"),
        namespace_path: root.join("n"),
        database_path: root.join("db"),
        locator: "test".into(),
    };
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let cache = NickCache::with_pane_source(
        Box::new(CountingPanes {
            fake: Arc::clone(&fake),
            panes: panes(),
        }),
        &paths,
        uuid::Uuid::nil(),
        &clock,
    );
    Fixture {
        fake,
        cache,
        daemon,
    }
}

fn standard_daemon(messages: Vec<MessageSummary>) -> Daemon {
    Daemon {
        messages,
        seats: (0..AUTHORS)
            .map(|index| seat_summary(seat(index), Some(target(index))))
            .collect(),
        participants: (0..AUTHORS).map(seat).collect(),
        inspectable: Vec::new(),
        history_calls: Mutex::new(0),
    }
}

fn hundred_messages() -> Vec<MessageSummary> {
    (0..100)
        .map(|n| message(n + 1, seat(usize::try_from(n).unwrap() % AUTHORS)))
        .collect()
}

fn read_page(fixture: &mut Fixture) -> String {
    let query = HistoryQuery {
        thread: thread(),
        page: PageRequest {
            cursor: None,
            limit: 100,
            max_bytes: MAX_PAGE_BYTES,
        },
        initial: None,
        full_bodies: false,
    };
    let spec = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext::default(),
    };
    let mut out = Vec::new();
    render_history_with(
        fixture.fake.as_ref(),
        false,
        query,
        &spec,
        &mut fixture.cache,
        &mut out,
        &Style::plain(),
    )
    .expect("history renders");
    String::from_utf8(out).unwrap()
}

/// Kills: resolving one `SeatInspect` and one `pane_names` per author (the
/// 10-author page cost 10 + 10 calls), or skipping the Participants page.
#[test]
fn hundred_message_page_resolves_names_once() {
    let mut fx = fixture(standard_daemon(hundred_messages()));
    let text = read_page(&mut fx);
    assert!(text.contains("pane-name-0"), "{text}");
    assert!(text.contains("tab-name-8"), "{text}");
    assert_eq!(fx.fake.calls(CallKind::SeatInspect), 0);
    assert!(fx.fake.calls(CallKind::Participants) <= 1);
    assert!(fx.fake.calls(CallKind::PaneNames) <= 1);
    assert_eq!(
        fx.fake.calls(CallKind::PaneNames),
        1,
        "the page did read the panes"
    );
    assert_eq!(fx.fake.calls(CallKind::History), 1);

    // A second page of the same thread reuses the cache entirely.
    read_page(&mut fx);
    assert_eq!(fx.fake.calls(CallKind::SeatInspect), 0);
    assert!(fx.fake.calls(CallKind::Participants) <= 1);
    assert!(fx.fake.calls(CallKind::PaneNames) <= 1);
}

/// Kills: not caching the `SeatInspect` result across pages (a second page
/// would inspect again), and inspecting an author that is a participant.
#[test]
fn unknown_author_costs_one_seat_inspect_and_is_cached() {
    let stranger = SeatId::new("seat-Stranger99");
    let mut daemon = standard_daemon(hundred_messages());
    daemon.messages.push(message(101, stranger.clone()));
    daemon.inspectable = vec![seat_summary(stranger, Some(target(3)))];
    let mut fx = fixture(daemon);
    let text = read_page(&mut fx);
    assert_eq!(fx.fake.calls(CallKind::SeatInspect), 1, "{text}");
    assert!(fx.fake.calls(CallKind::PaneNames) <= 1);
    read_page(&mut fx);
    assert_eq!(
        fx.fake.calls(CallKind::SeatInspect),
        1,
        "the result is cached"
    );
    assert_eq!(fx.daemon.history_calls.lock().unwrap().to_owned(), 2);
}

/// Differential golden: the transcript bytes are what the per-author
/// `SeatInspect` path produced. `HT_BLESS=1` rewrites it; regenerate it from
/// the current tip, never from this leaf's own output.
#[test]
fn rendered_transcript_is_unchanged() {
    let mut fx = fixture(standard_daemon(hundred_messages()));
    let actual = read_page(&mut fx);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/cli/golden/read_transcript_names.txt");
    if std::env::var_os("HT_BLESS").is_some_and(|value| value == "1") {
        std::fs::write(&path, &actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no golden at {} ({error}); HT_BLESS=1 writes it",
            path.display()
        )
    });
    assert_eq!(
        actual, expected,
        "transcript bytes changed (HT_BLESS=1 rewrites)"
    );
}
