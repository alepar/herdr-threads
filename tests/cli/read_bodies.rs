//! Human `read` body cost (ht-p03.12.8, spec D6 Wave 27 Bodies): the daemon
//! inlines complete bodies when `full_bodies` is sent, and the CLI fetches only
//! what is still clipped. Mounted from `src/cli/follow.rs`; counted with the
//! counting fake `LocalClient`.

use super::*;
use crate::{
    app::SystemClock,
    protocol::{
        capabilities::HISTORY_FULL_BODIES,
        ids::{MessageId, ThreadId},
        output::ContinuationContext,
        pagination::{Consistency, StopReason},
        results::MessageDetails,
        time::UtcMillis,
    },
    test_support::counting_client::{CallKind, CountingLocalClient, DaemonVintage},
};
use std::sync::Mutex;

/// Characters a daemon preview carries.
const SNIPPET: usize = 256;

fn thread() -> ThreadId {
    ThreadId::new("thread-Ab12Cd34")
}

fn body_for(sequence: u64, len: usize) -> String {
    // Distinct text per message so a mixed-up fetch shows in the transcript.
    format!("body-{sequence}-")
        .chars()
        .cycle()
        .take(len)
        .collect()
}

fn summary(sequence: u64, body: &str, inline: bool) -> MessageSummary {
    let clipped = !inline && body.chars().count() > SNIPPET;
    MessageSummary {
        message: MessageId::new(format!("msg-{sequence}")),
        thread: thread(),
        author: None,
        event_author: None,
        kind: MessageKind::Ordinary,
        sequence,
        created_at: UtcMillis(1_790_771_696_000 + i64::try_from(sequence).unwrap() * 1_000),
        actor_label: Some("alice".into()),
        preview_data: if inline || !clipped {
            body.to_owned()
        } else {
            body.chars().take(SNIPPET).collect()
        },
        preview_omitted: clipped,
        preview_detail_argv: None,
    }
}

/// The daemon: `bodies` are the thread's messages in order. A body longer
/// than `inline_limit` stays a clipped preview even when `full_bodies` is set
/// (a body the page budget cannot hold). Records the `full_bodies` flag of
/// every History request it sees.
struct Daemon {
    bodies: Vec<String>,
    inline_limit: usize,
    seen_full_bodies: Mutex<Vec<bool>>,
}

impl Daemon {
    fn answer(&self, command: &Command) -> Result<CommandResult, ApiError> {
        Ok(match command {
            Command::History(query) => {
                self.seen_full_bodies
                    .lock()
                    .unwrap()
                    .push(query.full_bodies);
                let items = self
                    .bodies
                    .iter()
                    .enumerate()
                    .map(|(i, body)| {
                        let inline = query.full_bodies && body.len() <= self.inline_limit;
                        summary(i as u64 + 1, body, inline)
                    })
                    .collect();
                CommandResult::History(Page {
                    items,
                    next_cursor: None,
                    next_argv: None,
                    high_water_ordinal: self.bodies.len() as u64,
                    scope_revision: None,
                    has_more: false,
                    stop_reason: StopReason::Complete,
                    consistency: Consistency::BoundedLive,
                })
            }
            Command::Message(query) => {
                let sequence: u64 = query
                    .message
                    .as_str()
                    .strip_prefix("msg-")
                    .and_then(|n| n.parse().ok())
                    .expect("known message id");
                let body = &self.bodies[sequence as usize - 1];
                CommandResult::Message(MessageDetails {
                    summary: summary(sequence, body, false),
                    content: MessageContent::Ordinary {
                        body_data: body.clone(),
                        body_offset: 0,
                        body_total_bytes: body.len() as u64,
                        body_complete: true,
                        body_next_cursor: None,
                        body_next_argv: None,
                    },
                })
            }
            _ => {
                return Err(ApiError::new(ErrorCode::Unsupported, "not served"));
            }
        })
    }
}

/// No pane lookup is expected: every author is an `actor_label`.
struct NoPanes;

impl PaneNameSource for NoPanes {
    fn pane_names(&self, _: &CallBudget) -> Result<Vec<PaneName>, ApiError> {
        Ok(Vec::new())
    }
}

struct Fixture {
    fake: Arc<CountingLocalClient>,
    cache: NickCache,
    daemon: Arc<Daemon>,
}

fn fixture(daemon: Daemon, vintage: DaemonVintage) -> Fixture {
    let daemon = Arc::new(daemon);
    let script = Arc::clone(&daemon);
    let fake = Arc::new(CountingLocalClient::scripted(
        move |command| script.answer(command),
        vintage,
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
    // Every author is an `actor_label`, so no seat or pane lookup happens.
    let cache = NickCache::with_pane_source(Box::new(NoPanes), &paths, uuid::Uuid::nil(), &clock);
    Fixture {
        fake,
        cache,
        daemon,
    }
}

/// 100 messages; five of them (every twentieth) are longer than a preview.
fn hundred_with_five_clipped() -> Vec<String> {
    (1..=100u64)
        .map(|n| {
            let len = if n % 20 == 0 {
                300 + 100 * (n as usize / 20)
            } else {
                40
            };
            body_for(n, len)
        })
        .collect()
}

fn daemon(bodies: Vec<String>, inline_limit: usize) -> Daemon {
    Daemon {
        bodies,
        inline_limit,
        seen_full_bodies: Mutex::new(Vec::new()),
    }
}

/// Reads the page the way `render_history` does for the fake's vintage: the
/// capability decides whether `full_bodies` is sent.
fn read_page(fixture: &mut Fixture) -> Result<String, RunError> {
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
        fixture.fake.capabilities().supports(HISTORY_FULL_BODIES),
        query,
        &spec,
        &mut fixture.cache,
        &mut out,
        &Style::plain(),
    )?;
    Ok(String::from_utf8(out).unwrap())
}

/// Kills: fetching one body per clipped preview even when the daemon inlined
/// it (5 Message calls), or not asking for `full_bodies` at all.
#[test]
fn hundred_message_page_with_five_clipped_makes_one_history_and_no_message_calls() {
    let bodies = hundred_with_five_clipped();
    let mut fx = fixture(daemon(bodies.clone(), usize::MAX), DaemonVintage::Current);
    let text = read_page(&mut fx).expect("renders");
    assert_eq!(fx.fake.calls(CallKind::History), 1);
    assert_eq!(fx.fake.calls(CallKind::Message), 0);
    assert_eq!(*fx.daemon.seen_full_bodies.lock().unwrap(), vec![true]);
    // The longest body is whole in the transcript, not a clipped preview.
    let longest = &bodies[99];
    assert!(longest.len() > SNIPPET);
    let unwrapped: String = text.split_whitespace().collect();
    assert!(unwrapped.contains(longest.as_str()), "{text}");
}

/// Kills: re-fetching bodies the daemon did inline, or fetching an oversize
/// body more than once (or not at all, which would print a clipped preview).
#[test]
fn single_oversize_body_costs_one_message_call() {
    let mut bodies = hundred_with_five_clipped();
    // The page budget cannot hold this one body, so the daemon keeps its
    // clipped preview; the other four long bodies are inlined.
    bodies[39] = body_for(40, 5_000);
    let mut fx = fixture(daemon(bodies.clone(), 2_000), DaemonVintage::Current);
    let text = read_page(&mut fx).expect("renders");
    assert_eq!(fx.fake.calls(CallKind::History), 1);
    assert_eq!(fx.fake.calls(CallKind::Message), 1);
    let unwrapped: String = text.split_whitespace().collect();
    // Past the preview and into the fetched body (the transcript folds the
    // rest of a 5_000 character body at the style's line limit).
    assert!(
        unwrapped.contains(&bodies[39][..600]),
        "fetched body is shown past the preview"
    );
}

/// Kills: sending `full_bodies` to a daemon that did not advertise it (an old
/// daemon rejects the unknown field), or dropping the per-preview fetches the
/// old daemon still needs, or changing the rendered bytes on either path.
#[test]
fn older_daemon_gets_no_full_bodies_and_identical_bytes() {
    let bodies = hundred_with_five_clipped();
    let mut older = fixture(daemon(bodies.clone(), usize::MAX), DaemonVintage::Older);
    let old_text = read_page(&mut older).expect("renders");
    assert_eq!(*older.daemon.seen_full_bodies.lock().unwrap(), vec![false]);
    assert_eq!(older.fake.calls(CallKind::History), 1);
    assert_eq!(
        older.fake.calls(CallKind::Message),
        5,
        "one fetch per preview"
    );
    let mut current = fixture(daemon(bodies, usize::MAX), DaemonVintage::Current);
    let new_text = read_page(&mut current).expect("renders");
    assert_eq!(old_text, new_text);
}

/// Kills: swallowing an `InvalidRequest` from a daemon that advertised the
/// capability (for example by retrying without the field), which would hide a
/// real protocol mismatch behind a silent fallback.
#[test]
fn invalid_request_from_an_advertising_daemon_is_surfaced() {
    let fake = CountingLocalClient::scripted(
        |command| match command {
            Command::History(_) => Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "unknown field `full_bodies`",
            )),
            _ => unreachable!("only History is sent"),
        },
        DaemonVintage::Current,
    );
    let mut fx = fixture(daemon(Vec::new(), 0), DaemonVintage::Current);
    fx.fake = Arc::new(fake);
    let error = read_page(&mut fx).expect_err("the error is surfaced");
    assert!(
        matches!(&error, RunError::Api(api) if api.code == ErrorCode::InvalidRequest),
        "{error:?}"
    );
    assert_eq!(fx.fake.calls(CallKind::History), 1, "no silent retry");
    assert_eq!(fx.fake.calls(CallKind::Message), 0);
}

/// Differential golden: the transcript bytes are what the per-preview fetch
/// path (an older daemon) printed before `full_bodies` existed. `HT_BLESS=1`
/// rewrites it from the older-daemon path, never from the `full_bodies` path.
#[test]
fn rendered_transcript_is_unchanged_with_full_bodies() {
    let bodies = hundred_with_five_clipped();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/cli/golden/read_transcript_bodies.txt");
    if std::env::var_os("HT_BLESS").is_some_and(|value| value == "1") {
        let mut older = fixture(daemon(bodies, usize::MAX), DaemonVintage::Older);
        std::fs::write(&path, read_page(&mut older).unwrap()).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "no golden at {} ({error}); HT_BLESS=1 writes it",
            path.display()
        )
    });
    let mut current = fixture(daemon(bodies.clone(), usize::MAX), DaemonVintage::Current);
    assert_eq!(
        read_page(&mut current).unwrap(),
        expected,
        "full_bodies transcript differs from the golden (HT_BLESS=1 rewrites)"
    );
    let mut older = fixture(daemon(bodies, usize::MAX), DaemonVintage::Older);
    assert_eq!(read_page(&mut older).unwrap(), expected);
}
