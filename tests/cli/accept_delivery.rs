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
use std::{collections::VecDeque, sync::Mutex};

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn claim() -> CallerClaim {
    CallerClaim {
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
    pages: Mutex<VecDeque<Page<InboxBatchV2Item>>>,
    calls: Mutex<Vec<&'static str>>,
}
impl Script {
    fn new(pages: Vec<Page<InboxBatchV2Item>>) -> Self {
        Self {
            scope: true,
            live: false,
            fail_bookkeeping: false,
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
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
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
                if self.fail_bookkeeping {
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
        &|| CallBudget {
            deadline: MonoInstant(u64::MAX),
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
            .starts_with("herdr-threads: accepted; opening mail display not recorded"),
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
