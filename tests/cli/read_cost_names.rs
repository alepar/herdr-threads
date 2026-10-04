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
    assert!(text.contains(target(8).as_str()), "{text}");
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

#[test]
fn relative_nick_missing_pane_label_retains_pane_id() {
    let mut fx = fixture(standard_daemon(hundred_messages()));
    fx.cache.panes = Some((
        Instant::now(),
        vec![PaneName {
            target: target(8),
            label: None,
            tab_label: Some("tryout".into()),
            tab_pane_count: 1,
        }],
    ));
    let shown = fx.cache.pane_label(
        target(8).as_str(),
        &budget(fx.cache.clock.as_ref(), 2000, &Cancellation::default()),
    );
    assert_eq!(
        shown.as_deref(),
        Some(target(8).as_str()),
        "relative nicks must retain the pane component instead of substituting a tab alias"
    );
}

struct ScopedPanes {
    labels: Arc<Mutex<Option<Vec<SeatHostLabels>>>>,
}
impl PaneNameSource for ScopedPanes {
    fn pane_names(&self, _: &CallBudget) -> Result<Vec<PaneName>, ApiError> {
        panic!("scope source must use one snapshot")
    }
    fn seat_labels(&self, _: &CallBudget) -> Result<Option<Vec<SeatHostLabels>>, ApiError> {
        self.labels
            .lock()
            .unwrap()
            .clone()
            .map(Some)
            .ok_or_else(|| ApiError::host_unavailable("host offline"))
    }
}
fn scoped_labels(index: usize, workspace: &str, tab: &str) -> SeatHostLabels {
    SeatHostLabels {
        terminal: "test-terminal".into(),
        incarnation: None,
        target: target(index),
        workspace_id: workspace.into(),
        workspace_label: Some("project".into()),
        tab_id: tab.into(),
        tab_label: Some("tryout".into()),
        pane_label: Some("alice".into()),
    }
}
#[test]
fn relative_nick_scope_uses_ids_refreshes_caller_move_and_survives_host_failure() {
    let mut fx = fixture(standard_daemon(hundred_messages()));
    let labels = Arc::new(Mutex::new(Some(vec![
        scoped_labels(0, "w1", "t1"),
        scoped_labels(1, "w1", "t2"),
        scoped_labels(2, "w2", "t1"),
    ])));
    fx.cache.host = Box::new(ScopedPanes {
        labels: Arc::clone(&labels),
    });
    fx.cache = fx.cache.with_caller(Some(target(0).as_str()));
    let shown = read_page(&mut fx);
    assert!(shown.contains("<alice>"), "{shown}");
    assert!(shown.contains("<tryout/alice>"), "{shown}");
    assert!(
        shown.contains("<project/tryout/alice>"),
        "equal labels must not erase distinct canonical parents: {shown}"
    );
    let mut moved = labels.lock().unwrap();
    moved.as_mut().unwrap()[0].workspace_id = "w2".into();
    drop(moved);
    fx.cache.panes = None;
    let shown = read_page(&mut fx);
    assert!(shown.contains("<project/tryout/alice>"));
    assert_eq!(fx.cache.nicks.get(&seat(2)).unwrap().1.name, "alice");
    *labels.lock().unwrap() = None;
    fx.cache.panes = None;
    let shown = read_page(&mut fx);
    assert!(shown.contains("message number 100"));
    assert!(!shown.contains("<project/tryout/alice>"));
}
#[test]
fn relative_nick_missing_scope_labels_and_event_recipient_keep_ids_and_readonly() {
    let mut warning = message(1, seat(0));
    warning.kind = MessageKind::Warn;
    warning.preview_data = format!(
        "{{\"obligation\":\"receipt\",\"seat\":\"{}\"}}",
        seat(1).as_str()
    );
    let mut fx = fixture(standard_daemon(vec![warning]));
    let mut recipient = scoped_labels(1, "w1", "t2");
    recipient.tab_label = None;
    recipient.pane_label = None;
    let labels = Arc::new(Mutex::new(Some(vec![
        scoped_labels(0, "w1", "t1"),
        recipient,
    ])));
    fx.cache.host = Box::new(ScopedPanes { labels });
    fx.cache = fx.cache.with_caller(Some(target(0).as_str()));
    let shown = read_page(&mut fx);
    assert!(
        shown.contains(&format!("t2/{} is overdue", target(1).as_str())),
        "{shown}"
    );
    assert_eq!(fx.fake.calls(CallKind::History), 1);
    assert!(fx.fake.calls(CallKind::SeatInspect) <= 1);
    assert_eq!(fx.fake.calls(CallKind::Message), 0);
}

#[test]
fn relative_nick_follow_refreshes_live_scope_without_changing_machine_ids() {
    let mut fx = fixture(standard_daemon(hundred_messages()));
    let labels = Arc::new(Mutex::new(Some(vec![
        scoped_labels(0, "w1", "t1"),
        scoped_labels(1, "w1", "t2"),
    ])));
    fx.cache.host = Box::new(ScopedPanes {
        labels: Arc::clone(&labels),
    });
    fx.cache = fx.cache.with_caller(Some(target(0).as_str()));
    let spec = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext::default(),
    };
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut printer = Printer {
        form: Form::Human,
        style: Style::plain(),
        no_system: false,
        writer: &mut out,
        errors: &mut errors,
    };
    printer
        .message(&message(1, seat(1)), fx.fake.as_ref(), &mut fx.cache, &spec)
        .unwrap();
    labels.lock().unwrap().as_mut().unwrap()[0].tab_id = "t2".into();
    fx.cache.panes = None;
    printer
        .message(&message(2, seat(1)), fx.fake.as_ref(), &mut fx.cache, &spec)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.lines().nth(1).unwrap().contains("<alice>"),
        "follow must refresh cached relative scope: {text}"
    );
    let mut out = Vec::new();
    let mut printer = Printer {
        form: Form::Lines,
        style: Style::plain(),
        no_system: false,
        writer: &mut out,
        errors: &mut errors,
    };
    let before = fx.fake.calls(CallKind::SeatInspect);
    printer
        .message(&message(3, seat(1)), fx.fake.as_ref(), &mut fx.cache, &spec)
        .unwrap();
    assert!(String::from_utf8(out).unwrap().contains(seat(1).as_str()));
    assert_eq!(fx.fake.calls(CallKind::SeatInspect), before);
}

struct MovingCallerState {
    current: Option<HostTargetId>,
    labels: Vec<SeatHostLabels>,
    hints: Vec<String>,
    snapshots: usize,
}
struct MovingCallerPanes(Arc<Mutex<MovingCallerState>>);
impl PaneNameSource for MovingCallerPanes {
    fn pane_names(&self, _: &CallBudget) -> Result<Vec<PaneName>, ApiError> {
        panic!("scoped snapshot required")
    }
    fn current_pane(&self, hint: &str, _: &CallBudget) -> Result<HostTargetId, ApiError> {
        let mut state = self.0.lock().unwrap();
        state.hints.push(hint.into());
        state
            .current
            .clone()
            .ok_or_else(|| ApiError::host_unavailable("caller lookup outage"))
    }
    fn seat_labels(&self, _: &CallBudget) -> Result<Option<Vec<SeatHostLabels>>, ApiError> {
        let mut state = self.0.lock().unwrap();
        state.snapshots += 1;
        Ok(Some(state.labels.clone()))
    }
}
fn moving_caller_fixture(
    current: Option<HostTargetId>,
    labels: Vec<SeatHostLabels>,
    author_target: HostTargetId,
) -> (Fixture, Arc<Mutex<MovingCallerState>>) {
    let mut daemon = standard_daemon(hundred_messages());
    daemon.seats[1].target = Some(author_target);
    let mut fx = fixture(daemon);
    let state = Arc::new(Mutex::new(MovingCallerState {
        current,
        labels,
        hints: Vec::new(),
        snapshots: 0,
    }));
    fx.cache.host = Box::new(MovingCallerPanes(Arc::clone(&state)));
    fx.cache = fx.cache.with_caller(Some("w1:p0"));
    (fx, state)
}
#[test]
fn relative_nick_live_caller_qualified_id_move_reuses_original_hint_with_bounded_lookups() {
    let mut author = scoped_labels(1, "w2", "t2");
    author.target = HostTargetId::new("w2:p1");
    let (mut fx, state) = moving_caller_fixture(
        Some(target(0)),
        vec![scoped_labels(0, "w1", "t1"), author.clone()],
        author.target,
    );
    read_page(&mut fx);
    assert_eq!(
        fx.cache.nicks.get(&seat(1)).unwrap().1.name,
        "project/tryout/alice"
    );
    read_page(&mut fx);
    assert_eq!(
        state.lock().unwrap().hints.len(),
        1,
        "cached100-message reads must not re-resolve per message/page"
    );
    let mut caller = scoped_labels(0, "w2", "t2");
    caller.target = HostTargetId::new("w2:p0");
    {
        let mut state = state.lock().unwrap();
        state.current = Some(caller.target.clone());
        state.labels[0] = caller;
    }
    fx.cache.panes = None;
    read_page(&mut fx);
    assert_eq!(
        fx.cache.nicks.get(&seat(1)).unwrap().1.name,
        "alice",
        "cross-workspace move changes qualified caller ID"
    );
    let state = state.lock().unwrap();
    assert_eq!(state.hints, vec!["w1:p0", "w1:p0"]);
    assert_eq!(state.snapshots, 2);
}
#[test]
fn relative_nick_live_caller_initial_outage_recovers_with_unchanged_labels() {
    let (mut fx, state) = moving_caller_fixture(
        None,
        vec![scoped_labels(0, "w1", "t1"), scoped_labels(1, "w1", "t1")],
        target(1),
    );
    read_page(&mut fx);
    assert_eq!(
        fx.cache.nicks.get(&seat(1)).unwrap().1.name,
        "project/tryout/alice"
    );
    state.lock().unwrap().current = Some(target(0));
    fx.cache.panes = None;
    read_page(&mut fx);
    assert_eq!(
        fx.cache.nicks.get(&seat(1)).unwrap().1.name,
        "alice",
        "initial caller lookup failure must recover at bounded refresh"
    );
    let state = state.lock().unwrap();
    assert_eq!(state.hints, vec!["w1:p0", "w1:p0"]);
    assert_eq!(state.snapshots, 2);
}
#[test]
fn relative_nick_live_caller_target_change_invalidates_nicks_even_with_unchanged_snapshot() {
    let mut author = scoped_labels(1, "w2", "t2");
    author.target = HostTargetId::new("w2:p1");
    let mut next = scoped_labels(0, "w2", "t2");
    next.target = HostTargetId::new("w2:p0");
    let (mut fx, state) = moving_caller_fixture(
        Some(target(0)),
        vec![scoped_labels(0, "w1", "t1"), next.clone(), author.clone()],
        author.target,
    );
    read_page(&mut fx);
    assert_eq!(
        fx.cache.nicks.get(&seat(1)).unwrap().1.name,
        "project/tryout/alice"
    );
    state.lock().unwrap().current = Some(next.target);
    fx.cache.panes = None;
    read_page(&mut fx);
    assert_eq!(
        fx.cache.nicks.get(&seat(1)).unwrap().1.name,
        "alice",
        "live caller alone must invalidate cached parent omissions"
    );
    read_page(&mut fx);
    let state = state.lock().unwrap();
    assert_eq!(state.hints, vec!["w1:p0", "w1:p0"]);
    assert_eq!(state.snapshots, 2);
}
