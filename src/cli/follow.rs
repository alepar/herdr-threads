//! `read THREAD --follow`, and the enriched human form of plain `read`.
//!
//! The follower prints the recent messages oldest first, then polls the
//! daemon for the messages after the last timeline sequence it printed
//! (`HistoryRange::After`, the cheapest exact "since" query) and prints only
//! those. It never redraws or clears the screen. It never ACKs, accepts or
//! sends: every call it makes is a read.
//!
//! Polling starts at one second and backs off while the thread is idle (up to
//! [`MAX_IDLE`]); any new message resets it. A failed poll is retried with
//! backoff against the freshly published endpoint. A lost daemon connection
//! (restart, stop) is reported once as a notice; a slow or busy poll is
//! retried quietly and reported only after [`QUIET_OUTAGE`] of continuous
//! failure. A daemon answer that is neither (refused, invalid, corrupt) ends
//! the follow with that error after [`DEFINITIVE_LIMIT`] polls in a row. An
//! archived thread is reported by its own archive notice and still followed
//! (it can be reopened); a thread that disappears ends the follow with status
//! 0, also when that is learned right after a reconnect. Ctrl-C ends it with 0.

use super::{
    RunError, connect,
    irc::{self, Full, Lookup, Nick, Style},
    output,
};
use crate::{
    client::local::LocalSocketClient,
    daemon::paths::{InstancePaths, RuntimeContext},
    harness::context::{ContextJournal, Harness},
    host::{native::NativeCli, observation::PaneName},
    ports::LocalClient,
    protocol::{
        commands::{
            BodyReadRequest, Command, HistoryQuery, HistoryRange, MessageQuery, SeatInspectQuery,
            ThreadQuery,
        },
        ids::SeatId,
        output::{OutputFormat, OutputSpec, selected_result},
        pagination::{MAX_PAGE_BYTES, MAX_PAGE_LIMIT, Page, PageRequest},
        results::{
            ApiError, CommandResult, ErrorCode, MessageContent, MessageKind, MessageSummary,
        },
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// First poll interval, and the interval right after any new message.
const MIN_IDLE: Duration = Duration::from_secs(1);
/// Longest poll interval while the thread is idle.
const MAX_IDLE: Duration = Duration::from_secs(4);
/// First retry delay after a failed poll.
const MIN_RETRY: Duration = Duration::from_millis(500);
/// Longest reconnect backoff while the daemon is unreachable.
const MAX_RECONNECT: Duration = Duration::from_secs(5);
/// Continuous slow or busy polling reported as a lost daemon only after this.
const QUIET_OUTAGE: Duration = Duration::from_secs(10);
/// Consecutive definitive daemon errors that end the follow.
const DEFINITIVE_LIMIT: u32 = 3;
/// How long a resolved nick is reused before it is looked up again.
const NICK_TTL: Duration = Duration::from_secs(60);
/// How long one host pane-name snapshot is reused.
const PANES_TTL: Duration = Duration::from_secs(15);
/// Body bytes fetched for a message whose preview was clipped.
const BODY_BYTES: u32 = 16_384;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Ctrl-C (and SIGTERM) end the follow cleanly with status 0.
fn install_interrupt() {
    // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_interrupt as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            on_interrupt as *const () as libc::sighandler_t,
        );
    }
}

fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// Sleep in small steps so Ctrl-C is noticed promptly. Returns `false` when
/// interrupted.
fn pause(total: Duration) -> bool {
    let end = Instant::now() + total;
    while Instant::now() < end {
        if interrupted() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50).min(end - Instant::now()));
    }
    !interrupted()
}

/// Columns of the terminal on stdout, or `$COLUMNS`, or 100.
fn terminal_width() -> usize {
    // SAFETY: TIOCGWINSZ only writes into the zeroed `winsize` we own.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_col >= 20 {
        return usize::from(size.ws_col);
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width >= 20)
        .unwrap_or(100)
}

/// The live human style: wrap at the terminal width, local time, and colors
/// only on a terminal (never with `NO_COLOR` or `TERM=dumb`).
pub(crate) fn live_style() -> Style {
    let terminal = output::stdout_is_terminal();
    let color = terminal
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        && std::env::var("TERM").map_or(true, |term| term != "dumb");
    Style {
        width: if terminal { terminal_width() } else { 100 },
        color,
        max_lines: 20,
        local_time: true,
    }
}

/// Seat nicks resolved from the service mapping, the host pane names and the
/// seat's local binding context, cached across polls.
pub(crate) struct NickCache {
    /// The clock every request budget is measured against: the same one the
    /// daemon client and the host adapter compare deadlines with.
    clock: Arc<dyn Clock>,
    host: NativeCli,
    paths: InstancePaths,
    instance: uuid::Uuid,
    panes: Option<(Instant, Vec<PaneName>)>,
    nicks: HashMap<SeatId, (Instant, Nick)>,
}

impl NickCache {
    pub(crate) fn new(
        context: &RuntimeContext,
        paths: &InstancePaths,
        instance: uuid::Uuid,
        clock: &Arc<dyn Clock>,
    ) -> Self {
        Self {
            clock: Arc::clone(clock),
            host: NativeCli::new(context.host_endpoint.clone(), Arc::clone(clock)),
            paths: paths.clone(),
            instance,
            panes: None,
            nicks: HashMap::new(),
        }
    }

    fn pane_label(&mut self, target: &str, budget: &CallBudget) -> Option<String> {
        let stale = self.panes.as_ref().is_none_or(|(at, panes)| {
            at.elapsed() > PANES_TTL
                || !panes.iter().any(|pane| pane.target.as_str() == target)
                    && at.elapsed() > Duration::from_secs(2)
        });
        if stale && let Ok(panes) = self.host.pane_names(budget) {
            self.panes = Some((Instant::now(), panes));
        }
        let (_, panes) = self.panes.as_ref()?;
        let pane = panes.iter().find(|pane| pane.target.as_str() == target)?;
        pane.label
            .clone()
            .filter(|label| !label.is_empty())
            .or_else(|| {
                pane.tab_label
                    .clone()
                    .filter(|label| !label.is_empty() && pane.tab_pane_count == 1)
            })
    }

    /// The harness of the seat's current binding, from its private local
    /// context (read only when it already exists; nothing is created).
    fn harness(&self, seat: &SeatId, generation: u64) -> Option<&'static str> {
        let root = self
            .paths
            .instance_dir
            .canonicalize()
            .ok()?
            .join("contexts");
        let dir: PathBuf = root.join(format!("{:x}", Sha256::digest(seat.as_str().as_bytes())));
        if !dir.join("context.json").is_file() {
            return None;
        }
        let journal = ContextJournal::open(
            &dir,
            self.instance,
            seat.as_str(),
            Duration::from_millis(200),
        )
        .ok()?;
        let current = journal.current().ok()??;
        if current.seat != seat.as_str() || current.binding_generation != generation {
            return None;
        }
        Some(match current.harness {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Human => "human",
        })
    }

    fn resolve(&mut self, client: &dyn LocalClient, seat: &SeatId, spec: &OutputSpec) -> Nick {
        if let Some((at, nick)) = self.nicks.get(seat)
            && at.elapsed() < NICK_TTL
        {
            return nick.clone();
        }
        let mut nick = Nick::seat(seat);
        let inspect = client.call_with_output(
            Command::SeatInspect(SeatInspectQuery {
                seat: seat.clone(),
                page: PageRequest {
                    cursor: None,
                    limit: 1,
                    max_bytes: MAX_PAGE_BYTES,
                },
            }),
            spec,
            &budget(self.clock.as_ref(), 2_000),
        );
        if let Ok(CommandResult::SeatInspect(inspection)) = inspect {
            if let Some(target) = &inspection.summary.target
                && let Some(label) =
                    self.pane_label(target.as_str(), &budget(self.clock.as_ref(), 2_000))
            {
                nick.name = label;
            }
            nick.harness = self
                .harness(seat, inspection.summary.generation)
                .map(str::to_owned);
        }
        self.nicks
            .insert(seat.clone(), (Instant::now(), nick.clone()));
        nick
    }
}

/// A request budget of `millis` from now on `clock`. It must be the clock
/// the client compares the deadline with: a budget minted on a fresh clock
/// (which starts at zero) is already spent once the process has lived
/// longer than `millis`, and every later request fails before it is sent.
fn budget(clock: &dyn Clock, millis: u64) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(millis)),
        cancellation: Cancellation::default(),
    }
}

/// [`Lookup`] backed by the daemon (bodies, seat mappings) and the cache.
struct LiveLookup<'a> {
    client: &'a dyn LocalClient,
    cache: &'a mut NickCache,
    spec: &'a OutputSpec,
}

impl Lookup for LiveLookup<'_> {
    fn nick(&mut self, seat: &SeatId) -> Nick {
        self.cache.resolve(self.client, seat, self.spec)
    }

    fn full(&mut self, summary: &MessageSummary) -> Option<Full> {
        let result = self
            .client
            .call_with_output(
                Command::Message(MessageQuery {
                    message: summary.message.clone(),
                    body: BodyReadRequest {
                        cursor: None,
                        offset: None,
                        max_bytes: BODY_BYTES,
                    },
                }),
                self.spec,
                &budget(self.cache.clock.as_ref(), 5_000),
            )
            .ok()?;
        let CommandResult::Message(details) = result else {
            return None;
        };
        Some(match details.content {
            MessageContent::Ordinary {
                body_data,
                body_complete,
                body_next_argv,
                ..
            } => Full::Text {
                data: body_data,
                complete: body_complete,
                more_argv: body_next_argv,
            },
            MessageContent::System { event, .. } => Full::Event(event.event_json),
        })
    }
}

/// The text output spec the follower uses for its own lookups.
fn text_spec(spec: &OutputSpec) -> OutputSpec {
    OutputSpec {
        format: OutputFormat::Text,
        context: spec.context.clone(),
    }
}

/// Human `read`: the IRC transcript with nicks and full bodies resolved.
pub(crate) fn render_history(
    client: &LocalSocketClient,
    query: HistoryQuery,
    spec: &OutputSpec,
    cache: &mut NickCache,
    writer: &mut dyn Write,
) -> Result<(), RunError> {
    let result = client.call_with_output(
        Command::History(query),
        spec,
        &budget(cache.clock.as_ref(), 5_000),
    )?;
    let CommandResult::History(page) = selected_result(&result, spec) else {
        return Err(RunError::Api(ApiError {
            code: ErrorCode::StoreCorrupt,
            detail: "daemon returned no history page".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }));
    };
    let lookup_spec = text_spec(spec);
    let mut lookup = LiveLookup {
        client,
        cache,
        spec: &lookup_spec,
    };
    let text = irc::render_page(&page, &mut lookup, &live_style());
    writer.write_all(text.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// How the follower prints records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    Human,
    /// One JSON object (a message summary) per line.
    Lines,
}

struct Printer<'a> {
    form: Form,
    style: Style,
    no_system: bool,
    writer: &'a mut dyn Write,
}

impl Printer<'_> {
    fn message(
        &mut self,
        summary: &MessageSummary,
        client: &dyn LocalClient,
        cache: &mut NickCache,
        spec: &OutputSpec,
    ) -> io::Result<()> {
        if self.no_system && summary.kind != MessageKind::Ordinary {
            return Ok(());
        }
        let text = match self.form {
            Form::Human => {
                let lookup_spec = text_spec(spec);
                let mut lookup = LiveLookup {
                    client,
                    cache,
                    spec: &lookup_spec,
                };
                irc::render_message(summary, &mut lookup, &self.style)
            }
            Form::Lines => {
                let mut line = json_line(summary);
                line.push('\n');
                line
            }
        };
        self.writer.write_all(text.as_bytes())?;
        self.writer.flush()
    }

    /// A follower notice: an IRC `-!-` line for a person, stderr otherwise.
    fn notice(&mut self, text: &str) -> io::Result<()> {
        match self.form {
            Form::Human => {
                let now = crate::protocol::time::UtcMillis(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0)),
                );
                let line = irc::notice(text, Some(now), &self.style);
                self.writer.write_all(line.as_bytes())?;
                self.writer.flush()
            }
            Form::Lines => {
                eprintln!("herdr-threads: {text}");
                Ok(())
            }
        }
    }
}

/// One message summary as a single JSON line, with C1 controls and Unicode
/// line separators escaped so a peer cannot drive the terminal.
pub(crate) fn json_line(summary: &MessageSummary) -> String {
    let raw = serde_json::to_string(summary).unwrap_or_default();
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '\u{007f}'..='\u{009f}' | '\u{2028}' | '\u{2029}') {
            out.push_str(&format!("\\u{:04x}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

fn history(
    client: &dyn LocalClient,
    clock: &dyn Clock,
    thread: &crate::protocol::ids::ThreadId,
    initial: Option<HistoryRange>,
    cursor: Option<String>,
    limit: u16,
    spec: &OutputSpec,
) -> Result<Page<MessageSummary>, ApiError> {
    let result = client.call_with_output(
        Command::History(HistoryQuery {
            thread: thread.clone(),
            page: PageRequest {
                cursor,
                limit,
                max_bytes: MAX_PAGE_BYTES,
            },
            initial,
        }),
        spec,
        &budget(clock, 5_000),
    )?;
    match selected_result(&result, spec) {
        CommandResult::History(page) => Ok(page),
        _ => Err(ApiError {
            code: ErrorCode::StoreCorrupt,
            detail: "daemon returned no history page".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }),
    }
}

/// How one failed poll is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureClass {
    /// The daemon connection itself is gone (refused, absent, cut off).
    ConnectionLost,
    /// The poll was slow or the daemon busy; the connection may be fine.
    Transient,
    /// The thread no longer exists.
    Gone,
    /// A definitive daemon answer that retrying cannot be expected to change.
    Definitive,
}

fn classify(error: &ApiError) -> FailureClass {
    match error.code {
        ErrorCode::NotFound => FailureClass::Gone,
        ErrorCode::HostUnavailable | ErrorCode::TransportDenied => FailureClass::ConnectionLost,
        ErrorCode::DeadlineExceeded
        | ErrorCode::UnknownOutcome
        | ErrorCode::Cancelled
        | ErrorCode::StoreBusy
        | ErrorCode::ServiceBusy
        | ErrorCode::ReadBudgetExhausted
        | ErrorCode::CursorStale
        | ErrorCode::InvalidCursor
        | ErrorCode::InstanceMismatch => FailureClass::Transient,
        _ => FailureClass::Definitive,
    }
}

/// What the follower does after one failed poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Retry without printing anything.
    Quiet,
    /// Print the one "lost the daemon" notice, then retry.
    Announce,
    /// The thread disappeared: end the follow with status 0.
    Gone,
    /// End the follow with this error.
    Fatal,
}

/// The follower's view of an ongoing run of failed polls.
#[derive(Debug, Default)]
struct Outage {
    since: Option<Instant>,
    announced: bool,
    definitive: u32,
}

impl Outage {
    fn failed(&mut self, error: &ApiError, now: Instant) -> Verdict {
        let class = classify(error);
        if class == FailureClass::Gone {
            return Verdict::Gone;
        }
        let since = *self.since.get_or_insert(now);
        if class == FailureClass::Definitive {
            self.definitive += 1;
            if self.definitive >= DEFINITIVE_LIMIT {
                return Verdict::Fatal;
            }
        } else {
            self.definitive = 0;
        }
        let announce = class == FailureClass::ConnectionLost
            || now.saturating_duration_since(since) >= QUIET_OUTAGE;
        if announce && !self.announced {
            self.announced = true;
            return Verdict::Announce;
        }
        Verdict::Quiet
    }

    /// A poll succeeded. Returns whether a lost daemon had been announced
    /// (and so a "reconnected" notice is owed).
    fn recovered(&mut self) -> bool {
        let announced = self.announced;
        *self = Self::default();
        announced
    }
}

/// A write to a closed pipe (`| head`) ends the follow quietly.
fn write_result(result: io::Result<()>) -> Result<bool, RunError> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Run `read THREAD --follow` until Ctrl-C, a closed output, or the thread
/// disappearing.
pub(crate) fn run(
    request: &super::commands::FollowRequest,
    spec: &OutputSpec,
    context: &RuntimeContext,
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut dyn Write,
) -> Result<(), RunError> {
    install_interrupt();
    let form = if output::human_active() {
        Form::Human
    } else {
        Form::Lines
    };
    let (instance, _, mut client) = connect(paths, clock)?;
    let mut cache = NickCache::new(context, paths, instance, clock);
    let mut printer = Printer {
        form,
        style: if form == Form::Human {
            live_style()
        } else {
            Style::plain()
        },
        no_system: request.no_system,
        writer,
    };
    let thread = &request.thread;

    // The opening page: the recent tail (oldest first) or everything after
    // an explicit sequence. Errors here are ordinary command errors.
    let mut last = match request.after {
        Some(after) => after,
        None => {
            let page = history(
                &client,
                clock.as_ref(),
                thread,
                Some(HistoryRange::Recent {
                    count: request.recent.max(1),
                }),
                None,
                request.recent.max(1),
                spec,
            )?;
            if form == Form::Human {
                let topic = match client.call_with_output(
                    Command::Thread(ThreadQuery {
                        thread: thread.clone(),
                        page: PageRequest {
                            cursor: None,
                            limit: 1,
                            max_bytes: MAX_PAGE_BYTES,
                        },
                        caller: None,
                    }),
                    &text_spec(spec),
                    &budget(clock.as_ref(), 5_000),
                ) {
                    Ok(CommandResult::Thread(details)) => {
                        let mut text = format!(
                            "Topic for {}: {}",
                            thread.as_str(),
                            details.summary.topic_data
                        );
                        if details.summary.archived {
                            text.push_str(" (archived)");
                        }
                        text
                    }
                    _ => format!("Following {}", thread.as_str()),
                };
                if !write_result(printer.notice(&format!("{topic} (Ctrl-C to stop)")))? {
                    return Ok(());
                }
            }
            let mut items = page.items;
            items.sort_by_key(|summary| summary.sequence);
            if request.recent > 0 {
                for summary in &items {
                    if !write_result(printer.message(summary, &client, &mut cache, spec))? {
                        return Ok(());
                    }
                }
            }
            items
                .last()
                .map_or(0, |summary| summary.sequence)
                .max(page.high_water_ordinal)
        }
    };

    let mut idle = MIN_IDLE;
    let mut outage = Outage::default();
    let mut retry_wait = MIN_RETRY;
    loop {
        if interrupted() {
            return Ok(());
        }
        // Drain everything after `last`, following page continuations.
        let mut cursor: Option<String> = None;
        let mut printed = false;
        let outcome = loop {
            let initial = cursor
                .is_none()
                .then_some(HistoryRange::After { sequence: last });
            match history(
                &client,
                clock.as_ref(),
                thread,
                initial,
                cursor.take(),
                MAX_PAGE_LIMIT,
                spec,
            ) {
                Ok(page) => {
                    let mut items = page.items;
                    items.sort_by_key(|summary| summary.sequence);
                    for summary in &items {
                        if summary.sequence <= last {
                            continue;
                        }
                        if !write_result(printer.message(summary, &client, &mut cache, spec))? {
                            return Ok(());
                        }
                        last = summary.sequence;
                        printed = true;
                    }
                    if page.has_more && page.next_cursor.is_some() && !interrupted() {
                        cursor = page.next_cursor;
                        continue;
                    }
                    break Ok(());
                }
                Err(error) => break Err(error),
            }
        };
        match outcome {
            Ok(()) => {
                retry_wait = MIN_RETRY;
                if outage.recovered() && !write_result(printer.notice("reconnected to the daemon"))?
                {
                    return Ok(());
                }
                idle = if printed {
                    MIN_IDLE
                } else {
                    (idle.mul_f32(1.5)).min(MAX_IDLE)
                };
                if !pause(idle) {
                    return Ok(());
                }
            }
            Err(error) => {
                match outage.failed(&error, Instant::now()) {
                    Verdict::Gone => {
                        write_result(printer.notice(&format!(
                            "thread {} no longer exists; stopped following",
                            thread.as_str()
                        )))?;
                        return Ok(());
                    }
                    Verdict::Fatal => {
                        write_result(printer.notice(&format!(
                            "stopped following: the daemon keeps refusing ({})",
                            error.detail
                        )))?;
                        return Err(RunError::Api(error));
                    }
                    Verdict::Announce => {
                        if !write_result(
                            printer.notice(&format!(
                                "lost the daemon ({}); reconnecting",
                                error.detail
                            )),
                        )? {
                            return Ok(());
                        }
                    }
                    Verdict::Quiet => {}
                }
                if !pause(retry_wait) {
                    return Ok(());
                }
                retry_wait = retry_wait.mul_f32(1.5).min(MAX_RECONNECT);
                // A restarted daemon publishes a new endpoint and boot.
                if let Ok((instance, _, fresh)) = connect(paths, clock) {
                    client = fresh;
                    cache.instance = instance;
                }
            }
        }
    }
}

#[cfg(test)]
mod outage_tests {
    use super::*;

    fn api(code: ErrorCode) -> ApiError {
        ApiError {
            code,
            detail: "detail".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }
    }

    #[test]
    fn one_slow_poll_is_retried_quietly() {
        let mut outage = Outage::default();
        let start = Instant::now();
        assert_eq!(
            outage.failed(&api(ErrorCode::DeadlineExceeded), start),
            Verdict::Quiet
        );
        assert!(!outage.recovered(), "nothing announced, nothing to undo");
    }

    #[test]
    fn continuous_slow_polls_are_announced_once_after_the_quiet_window() {
        let mut outage = Outage::default();
        let start = Instant::now();
        for (at, code) in [
            (0, ErrorCode::DeadlineExceeded),
            (3, ErrorCode::UnknownOutcome),
            (9, ErrorCode::StoreBusy),
        ] {
            assert_eq!(
                outage.failed(&api(code), start + Duration::from_secs(at)),
                Verdict::Quiet
            );
        }
        assert_eq!(
            outage.failed(&api(ErrorCode::DeadlineExceeded), start + QUIET_OUTAGE),
            Verdict::Announce
        );
        assert_eq!(
            outage.failed(&api(ErrorCode::DeadlineExceeded), start + QUIET_OUTAGE * 2),
            Verdict::Quiet
        );
        assert!(outage.recovered(), "a reconnected notice is owed");
        assert_eq!(
            outage.failed(&api(ErrorCode::DeadlineExceeded), start + QUIET_OUTAGE * 3),
            Verdict::Quiet,
            "a fresh outage starts its own quiet window"
        );
    }

    #[test]
    fn a_lost_connection_is_announced_at_once() {
        let mut outage = Outage::default();
        let start = Instant::now();
        assert_eq!(
            outage.failed(&api(ErrorCode::HostUnavailable), start),
            Verdict::Announce
        );
        assert_eq!(
            outage.failed(&api(ErrorCode::HostUnavailable), start),
            Verdict::Quiet
        );
    }

    #[test]
    fn a_vanished_thread_ends_the_follow_even_after_a_reconnect() {
        let mut outage = Outage::default();
        let start = Instant::now();
        assert_eq!(
            outage.failed(&api(ErrorCode::HostUnavailable), start),
            Verdict::Announce
        );
        assert_eq!(
            outage.failed(&api(ErrorCode::NotFound), start + Duration::from_secs(1)),
            Verdict::Gone
        );
    }

    #[test]
    fn persistent_definitive_errors_end_the_follow() {
        let mut outage = Outage::default();
        let start = Instant::now();
        for _ in 1..DEFINITIVE_LIMIT {
            assert_eq!(
                outage.failed(&api(ErrorCode::Unauthorized), start),
                Verdict::Quiet
            );
        }
        assert_eq!(
            outage.failed(&api(ErrorCode::Unauthorized), start),
            Verdict::Fatal
        );
        // A transport failure in between resets the definitive run.
        let mut outage = Outage::default();
        outage.failed(&api(ErrorCode::StoreCorrupt), start);
        outage.failed(&api(ErrorCode::StoreCorrupt), start);
        outage.failed(&api(ErrorCode::DeadlineExceeded), start);
        assert_eq!(
            outage.failed(&api(ErrorCode::StoreCorrupt), start),
            Verdict::Quiet
        );
    }

    #[test]
    fn budgets_are_measured_on_the_callers_clock() {
        struct Late;
        impl Clock for Late {
            fn utc_now(&self) -> crate::protocol::time::UtcMillis {
                crate::protocol::time::UtcMillis(0)
            }
            fn monotonic_now(&self) -> MonoInstant {
                MonoInstant(60_000)
            }
        }
        let budget = budget(&Late, 5_000);
        assert_eq!(budget.deadline, MonoInstant(65_000));
        assert!(!budget.is_exhausted(&Late));
    }
}
