// ht-br0: the text accept's thread-scoped page against a scripted daemon: who gets no page, the
// rewritten `next:` line, the live-channel grace and a post-display bookkeeping failure.
use super::*;
use crate::protocol::{
    attention::{AttentionClass, AttentionDigest, AttentionToken},
    ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId},
    output::ContinuationContext,
    pagination::{Consistency, StopReason},
    results::ErrorCode,
    time::{MonoInstant, UtcMillis},
};
use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
    time::Instant,
};

/// Real monotonic milliseconds: the channel grace and the scripted transport's budget checks
/// run against wall time.
struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        static START: OnceLock<Instant> = OnceLock::new();
        MonoInstant(1 + START.get_or_init(Instant::now).elapsed().as_millis() as u64)
    }
}

fn claim() -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
        target: HostTargetId::new("w1:p1"),
        agent_id: None,
        liveness: None,
    }
}

fn lazy(id: &str, body: &str) -> InboxBatchV2Item {
    InboxBatchV2Item::LazyMessage {
        thread: ThreadId::new("t"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: Some(SeatId::new("peer")),
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: body.into(),
        body_start: 0,
        body_end: body.len() as u64,
        body_len: body.len() as u64,
        wakes: false,
    }
}

fn page(items: Vec<InboxBatchV2Item>, has_more: bool) -> Page<InboxBatchV2Item> {
    Page {
        items,
        next_cursor: has_more.then(|| "v2:more".into()),
        next_argv: None,
        high_water_ordinal: 1,
        scope_revision: None,
        has_more,
        stop_reason: if has_more {
            StopReason::Rows
        } else {
            StopReason::Complete
        },
        consistency: Consistency::BoundedLive,
    }
}

/// A scripted daemon. `pages` answers the scoped reads in order (the last one repeats);
/// `scope` says whether it advertises `inbox.thread_scope_v1`.
struct Script {
    scope: bool,
    live: bool,
    fail_bookkeeping: bool,
    /// Each page read takes this long, as a slow daemon bounded by the call budget: a budget
    /// that ends first cuts the read off at its deadline with `deadline_exceeded`.
    page_delay: Duration,
    fail_advance_only: bool,
    /// The page read with this 1-based index is refused at once (a canonical refusal, not a
    /// timeout); 0 refuses none.
    refuse_read: usize,
    /// Each page read's (call time, budget deadline), in TestClock milliseconds.
    page_budgets: Mutex<Vec<(u64, u64)>>,
    pages: Mutex<VecDeque<Page<InboxBatchV2Item>>>,
    calls: Mutex<Vec<&'static str>>,
}
impl Script {
    fn new(pages: Vec<Page<InboxBatchV2Item>>) -> Self {
        Self {
            scope: true,
            live: false,
            fail_bookkeeping: false,
            page_delay: Duration::ZERO,
            fail_advance_only: false,
            refuse_read: 0,
            page_budgets: Mutex::new(vec![]),
            pages: Mutex::new(pages.into()),
            calls: Mutex::new(vec![]),
        }
    }
    fn count(&self, name: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == name)
            .count()
    }
}
impl LocalClient for Script {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        name != INBOX_THREAD_SCOPE || self.scope
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        let busy = || Err(ApiError::new(ErrorCode::StoreBusy, "scripted busy"));
        match command {
            Command::AttentionDigest(_) => {
                self.calls.lock().unwrap().push("digest");
                let none = || AttentionClass {
                    count: 0,
                    items: vec![],
                    has_more: false,
                    count_has_more: false,
                };
                Ok(CommandResult::AttentionDigest(AttentionDigest {
                    version: 1,
                    seat: SeatId::new("s"),
                    token: AttentionToken::default(),
                    invitations: none(),
                    receipts: none(),
                    warnings: none(),
                    unavailability_open: false,
                    lazy: None,
                    mail: None,
                    mod_channel_live: self.live,
                }))
            }
            Command::InboxBatchV2(query) => {
                self.calls.lock().unwrap().push("page");
                assert_eq!(query.thread, Some(ThreadId::new("t")));
                self.page_budgets
                    .lock()
                    .unwrap()
                    .push((TestClock.monotonic_now().0, budget.deadline.0));
                if self.page_budgets.lock().unwrap().len() == self.refuse_read {
                    return busy();
                }
                if !self.page_delay.is_zero() {
                    let left = budget
                        .deadline
                        .0
                        .saturating_sub(TestClock.monotonic_now().0);
                    let delay = u64::try_from(self.page_delay.as_millis()).unwrap();
                    std::thread::sleep(Duration::from_millis(left.min(delay)));
                    if left < delay {
                        return Err(ApiError::new(
                            ErrorCode::DeadlineExceeded,
                            "scripted slow read",
                        ));
                    }
                }
                assert!(query.page.cursor.is_none());
                let mut pages = self.pages.lock().unwrap();
                let next = if pages.len() > 1 {
                    pages.pop_front().unwrap()
                } else {
                    pages.front().cloned().unwrap()
                };
                Ok(CommandResult::InboxBatchV2(next))
            }
            Command::AdvanceCursors(advance) => {
                self.calls.lock().unwrap().push("advance");
                if self.fail_bookkeeping || self.fail_advance_only {
                    return busy();
                }
                Ok(CommandResult::CursorsAdvanced(advance.displayed))
            }
            Command::CompleteInboxDelivery(complete) => {
                self.calls.lock().unwrap().push("complete");
                if self.fail_bookkeeping {
                    return busy();
                }
                Ok(CommandResult::InboxDeliveryCompleted(complete.messages))
            }
            other => panic!("unexpected command {other:?}"),
        }
    }
}

struct Run {
    out: String,
    diagnostics: String,
}

fn run_with(
    script: &Script,
    claim: &CallerClaim,
    presentation: Presentation,
    grace: ChannelGrace,
) -> Run {
    let root = crate::test_support::isolation::TestIsolation::new("accept-delivery");
    let journal = Journal::open(root.state_root().join("intents")).unwrap();
    let output = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some("/state".into()),
            host: Some("/host.sock".into()),
        },
    };
    let thread = ThreadId::new("t");
    let mut out = Vec::new();
    let mut diagnostics = Vec::new();
    show(
        &AcceptPage {
            presentation,
            thread: &thread,
            claim,
            output: &output,
            journal: &journal,
            grace,
        },
        script,
        &TestClock,
        // The production per-call budget: 5 s from each call.
        &|| CallBudget {
            deadline: MonoInstant(TestClock.monotonic_now().0 + 5_000),
            cancellation: Default::default(),
        },
        &mut out,
        &mut diagnostics,
    );
    Run {
        out: String::from_utf8(out).unwrap(),
        diagnostics: String::from_utf8(diagnostics).unwrap(),
    }
}

fn run(script: &Script) -> Run {
    run_with(script, &claim(), Presentation::Auto, CHANNEL_GRACE)
}

fn fast_grace(bound_ms: u64) -> ChannelGrace {
    ChannelGrace {
        bound: Duration::from_millis(bound_ms),
        interval: Duration::from_millis(2),
    }
}

#[test]
fn a_subagent_caller_prints_no_page_and_reads_nothing() {
    let script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    let mut subagent = claim();
    subagent.role = CallerRole::Subagent;
    subagent.agent_id = Some(crate::protocol::ids::AgentId::new("agent-1"));
    let run = run_with(&script, &subagent, Presentation::Auto, CHANNEL_GRACE);
    assert_eq!(run.out, "");
    assert_eq!(run.diagnostics, "");
    assert!(script.calls.lock().unwrap().is_empty());
}

#[test]
fn a_daemon_without_thread_scope_gets_no_page_read_and_no_bookkeeping() {
    let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    script.scope = false;
    let run = run(&script);
    assert_eq!(run.out, "");
    assert_eq!(run.diagnostics, "");
    assert!(script.calls.lock().unwrap().is_empty());
}

#[test]
fn a_page_with_more_prints_one_plain_inbox_next_line_with_the_callers_presentation() {
    for (presentation, flag) in [
        (Presentation::Auto, None),
        (Presentation::Human, Some("--human")),
    ] {
        let script = Script::new(vec![page(vec![lazy("m1", "opener body")], true)]);
        let run = run_with(&script, &claim(), presentation, CHANNEL_GRACE);
        assert!(run.out.contains("opener body"), "{}", run.out);
        let next: Vec<&str> = run
            .out
            .lines()
            .filter(|line| line.starts_with("next:"))
            .collect();
        assert_eq!(next.len(), 1, "{}", run.out);
        let argv: Vec<&str> = next[0]["next:".len()..].split_whitespace().collect();
        let mut expected = vec![
            "herdr-threads",
            "--state-dir",
            "/state",
            "--host-endpoint",
            "/host.sock",
        ];
        expected.extend(flag);
        expected.push("inbox");
        assert_eq!(argv, expected, "{}", run.out);
        assert!(!run.out.contains("v2:more"), "{}", run.out);
        assert_eq!(script.count("advance"), 1);
        assert_eq!(script.count("complete"), 1);
    }
}

#[test]
fn a_bookkeeping_failure_after_display_is_one_diagnostics_line() {
    let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    script.fail_bookkeeping = true;
    let run = run(&script);
    assert!(run.out.contains("opener body"), "{}", run.out);
    assert_eq!(run.diagnostics.lines().count(), 1, "{}", run.diagnostics);
    assert!(
        run.diagnostics
            .starts_with(
                "herdr-threads: accepted; opening mail display bookkeeping failed, rows may be offered again: "
            ),
        "{}",
        run.diagnostics
    );
    assert_eq!(script.count("advance"), 1);
    assert!(script.count("complete") >= 1);
}

#[test]
fn a_live_channel_that_completes_within_the_grace_leaves_the_accept_silent() {
    let mut script = Script::new(vec![
        page(vec![lazy("m1", "opener body")], false),
        page(vec![lazy("m1", "opener body")], false),
        page(vec![], false),
    ]);
    script.live = true;
    let run = run_with(&script, &claim(), Presentation::Auto, fast_grace(5_000));
    assert_eq!(run.out, "");
    assert_eq!(run.diagnostics, "");
    assert_eq!(script.count("page"), 3, "peeks until the channel completed");
    assert_eq!(script.count("advance"), 0);
    assert_eq!(script.count("complete"), 0);
}

#[test]
fn a_live_channel_that_misses_the_grace_leaves_the_accept_to_show_and_settle() {
    let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    script.live = true;
    let run = run_with(&script, &claim(), Presentation::Auto, fast_grace(20));
    assert_eq!(run.out.matches("opener body").count(), 1, "{}", run.out);
    assert_eq!(run.diagnostics, "");
    assert_eq!(script.count("advance"), 1);
    assert_eq!(script.count("complete"), 1);
}

/// Main review I1: a slow but bounded daemon must not stretch the grace. The grace runs from
/// before the first peek and every peek's budget ends by the grace's end, so no read can outlast
/// it; the read cut off there falls back to printing, and the fallback's own read has the
/// caller's fresh budget. Asserted on the budgets the scripted daemon received, not wall time.
#[test]
fn a_slow_daemon_cannot_stretch_the_channel_grace() {
    let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    script.live = true;
    script.page_delay = Duration::from_millis(300);
    let grace = ChannelGrace {
        bound: Duration::from_millis(400),
        interval: Duration::from_millis(50),
    };
    let run = run_with(&script, &claim(), Presentation::Auto, grace);
    let budgets = script.page_budgets.lock().unwrap().clone();
    let (fallback, peeks) = budgets.split_last().expect("peeks and a fallback read");
    // The grace starts just before the first peek, so it ends by that peek's call time + bound.
    let grace_end = peeks[0].0 + 400;
    for (_, deadline) in peeks {
        assert!(
            *deadline <= grace_end,
            "peek budget past the grace: {budgets:?}"
        );
    }
    assert!(
        fallback.1 >= fallback.0 + 4_000,
        "fallback keeps a fresh budget: {budgets:?}"
    );
    assert_eq!(run.out.matches("opener body").count(), 1, "{}", run.out);
    assert_eq!(run.diagnostics, "");
    assert_eq!(script.count("advance"), 1);
}

/// The first peek alone outlasting the grace is the grace running out (fallback), not a refusal:
/// its budget already ends with the grace.
#[test]
fn a_first_peek_cut_off_by_the_grace_falls_back_to_printing() {
    let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    script.live = true;
    script.page_delay = Duration::from_millis(250);
    let grace = ChannelGrace {
        bound: Duration::from_millis(100),
        interval: Duration::from_millis(20),
    };
    let run = run_with(&script, &claim(), Presentation::Auto, grace);
    let budgets = script.page_budgets.lock().unwrap().clone();
    assert_eq!(
        budgets.len(),
        2,
        "one cut-off peek, then the fallback's read: {budgets:?}"
    );
    assert!(
        budgets[0].1 <= budgets[0].0 + 100,
        "first peek capped at the grace: {budgets:?}"
    );
    assert!(
        budgets[1].1 >= budgets[1].0 + 4_000,
        "fallback keeps a fresh budget: {budgets:?}"
    );
    assert_eq!(run.out.matches("opener body").count(), 1, "{}", run.out);
}

/// A peek the daemon refuses inside the grace shows nothing: the rows stay with the channel.
/// Both the first peek and a later one (after the first offered the opener).
#[test]
fn a_peek_refused_inside_the_grace_shows_nothing() {
    for refused in [1, 2] {
        let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
        script.live = true;
        script.refuse_read = refused;
        let run = run_with(&script, &claim(), Presentation::Auto, fast_grace(5_000));
        assert_eq!(run.out, "", "refused peek {refused}");
        assert_eq!(run.diagnostics, "", "refused peek {refused}");
        assert_eq!(script.count("page"), refused, "no read after the refusal");
        assert_eq!(script.count("advance") + script.count("complete"), 0);
    }
}

/// Main review M1: when only AdvanceCursors fails and completion succeeds, the diagnostics line
/// must not claim the rows stay pending.
#[test]
fn an_advance_failure_after_successful_completion_does_not_claim_pending() {
    let mut script = Script::new(vec![page(vec![lazy("m1", "opener body")], false)]);
    script.fail_advance_only = true;
    let run = run(&script);
    assert_eq!(run.diagnostics.lines().count(), 1, "{}", run.diagnostics);
    assert!(
        !run.diagnostics.contains("stays pending"),
        "{}",
        run.diagnostics
    );
    assert!(
        run.diagnostics.contains("may be offered again"),
        "{}",
        run.diagnostics
    );
    assert_eq!(script.count("complete"), 1);
}
