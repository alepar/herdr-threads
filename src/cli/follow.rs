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
//! 0, also when that is learned right after a reconnect. Ctrl-C ends it with 0,
//! also in the middle of a call: the interrupt cancels the [`Cancellation`]
//! every call's budget carries, so a daemon that never answers cannot hold the
//! follower. A slow connect is a `Transient` failure (see
//! [`crate::client::error_class`]), retried quietly like a slow poll. Every
//! notice goes through [`escape_for_terminal`]; the fatal notice is printed
//! once and the run then exits with the error's status without a second
//! report.

use super::{
    RunError, connect,
    irc::{self, Full, Lookup, Nick, Style},
    output,
};
use crate::{
    daemon::paths::{InstancePaths, RuntimeContext},
    harness::context::{ContextJournal, Harness},
    host::{
        native::NativeCli,
        observation::{PaneName, SeatHostLabels},
    },
    ports::LocalClient,
    protocol::{
        capabilities::HISTORY_FULL_BODIES,
        commands::{
            BodyReadRequest, Command, FULL_BODY_FETCH_BYTES, HistoryQuery, HistoryRange,
            MessageQuery, ParticipantsQuery, SeatInspectQuery, SeatsQuery, ThreadQuery,
        },
        ids::{HostTargetId, SeatId, ThreadId},
        output::{OutputFormat, OutputSpec, selected_result},
        pagination::{MAX_PAGE_BYTES, MAX_PAGE_LIMIT, Page, PageRequest},
        results::{
            ApiError, CommandResult, ErrorCode, MessageContent, MessageKind, MessageSummary,
        },
        service::EventAuthor,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    view::escape::{Context, escape_for_terminal, is_unsafe_char, push_u4},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
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
const BODY_BYTES: u32 = FULL_BODY_FETCH_BYTES;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// How often the interrupt watcher looks at the signal flag; bounds how long
/// a Ctrl-C takes to reach a call that is waiting on the daemon.
const INTERRUPT_POLL: Duration = Duration::from_millis(10);

extern "C" fn on_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Ctrl-C (and SIGTERM) end the follow cleanly with status 0. Returns the
/// [`Cancellation`] the signal cancels: put it in every call budget.
fn install_interrupt() -> Cancellation {
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
    let cancel = Cancellation::default();
    watch_interrupt(&INTERRUPTED, cancel.clone());
    cancel
}

/// A signal handler may only touch atomics, so a thread turns the flag into a
/// real [`Cancellation`] (waking async waiters, which is what interrupts a
/// call in flight). It ends when the flag is seen or `cancel` is cancelled.
fn watch_interrupt(flag: &'static AtomicBool, cancel: Cancellation) {
    std::thread::spawn(move || {
        loop {
            if flag.load(Ordering::SeqCst) {
                cancel.cancel();
                return;
            }
            if cancel.wait_blocking(INTERRUPT_POLL) {
                return;
            }
        }
    });
}

/// Sleep, waking at once on Ctrl-C. Returns `false` when interrupted.
fn pause(total: Duration, cancel: &Cancellation) -> bool {
    !cancel.wait_blocking(total)
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

/// A seat's host target (when mapped) and binding generation.
type SeatBinding = (Option<HostTargetId>, u64);

/// Where a [`NickCache`] reads host pane names from.
pub(crate) trait PaneNameSource {
    fn pane_names(&self, budget: &CallBudget) -> Result<Vec<PaneName>, ApiError>;
    fn current_pane(&self, caller: &str, _budget: &CallBudget) -> Result<HostTargetId, ApiError> {
        Ok(HostTargetId::new(caller))
    }
    /// Older bounded fixtures can supply pane-only names; production supplies scope.
    fn seat_labels(&self, _budget: &CallBudget) -> Result<Option<Vec<SeatHostLabels>>, ApiError> {
        Ok(None)
    }
}

impl PaneNameSource for NativeCli {
    fn current_pane(&self, caller: &str, budget: &CallBudget) -> Result<HostTargetId, ApiError> {
        NativeCli::current_pane(self, caller, budget)
    }
    fn seat_labels(&self, budget: &CallBudget) -> Result<Option<Vec<SeatHostLabels>>, ApiError> {
        NativeCli::seat_labels(self, budget).map(Some)
    }
    fn pane_names(&self, budget: &CallBudget) -> Result<Vec<PaneName>, ApiError> {
        NativeCli::pane_names(self, budget)
    }
}

/// Seat nicks resolved from the service mapping, the host pane names and the
/// seat's local binding context, cached across polls.
pub(crate) struct NickCache {
    /// The clock every request budget is measured against: the same one the
    /// daemon client and the host adapter compare deadlines with.
    clock: Arc<dyn Clock>,
    host: Box<dyn PaneNameSource>,
    paths: InstancePaths,
    instance: uuid::Uuid,
    panes: Option<(Instant, Vec<PaneName>)>,
    labels: Option<Vec<SeatHostLabels>>,
    caller: Option<HostTargetId>,
    /// Original inherited locator; a qualified live target can change on a move.
    caller_hint: Option<String>,
    nicks: HashMap<SeatId, (Instant, Nick)>,
    /// The seats of each thread's last Participants page.
    participants: HashMap<ThreadId, (Instant, HashSet<SeatId>)>,
    /// Target and binding generation per seat from the last Seats page.
    mapped: Option<(Instant, HashMap<SeatId, SeatBinding>)>,
    /// While a history page is being resolved: whether its one pane-name
    /// snapshot has been read already. `None` outside a page (the follower).
    page_panes_read: Option<bool>,
    /// Cancelled by Ctrl-C; carried by every call budget this cache mints.
    cancel: Cancellation,
}

impl NickCache {
    pub(crate) fn new(
        context: &RuntimeContext,
        paths: &InstancePaths,
        instance: uuid::Uuid,
        clock: &Arc<dyn Clock>,
    ) -> Self {
        Self::with_pane_source(
            Box::new(NativeCli::new(
                context.host_endpoint.clone(),
                Arc::clone(clock),
            )),
            paths,
            instance,
            clock,
        )
    }

    /// A cache reading pane names from `host`: the production host adapter,
    /// or a counting fixture.
    fn with_pane_source(
        host: Box<dyn PaneNameSource>,
        paths: &InstancePaths,
        instance: uuid::Uuid,
        clock: &Arc<dyn Clock>,
    ) -> Self {
        Self {
            clock: Arc::clone(clock),
            host,
            paths: paths.clone(),
            instance,
            panes: None,
            labels: None,
            caller: None,
            caller_hint: None,
            nicks: HashMap::new(),
            participants: HashMap::new(),
            mapped: None,
            page_panes_read: None,
            cancel: Cancellation::default(),
        }
    }

    /// Explicit/inherited invoking pane; never UI focus. Its live scope comes
    /// from each refreshed host snapshot, so moves cannot be inferred from labels.
    pub(crate) fn with_caller(mut self, caller: Option<&str>) -> Self {
        let hint = caller
            .filter(|caller| !caller.is_empty())
            .map(str::to_owned);
        if self.caller_hint != hint {
            self.caller_hint = hint;
            self.caller = None;
            self.panes = None;
            self.page_panes_read = None;
            self.nicks.clear();
        }
        self
    }

    /// Make every call this cache issues give up when `cancel` fires.
    fn cancelled_by(mut self, cancel: &Cancellation) -> Self {
        self.cancel = cancel.clone();
        self
    }

    /// Whether the pane-name snapshot must be read again before it can name
    /// `targets`: absent, past its TTL, or (after two seconds) missing one.
    fn panes_stale<'t>(&self, targets: impl IntoIterator<Item = &'t str>) -> bool {
        self.panes.as_ref().is_none_or(|(at, panes)| {
            at.elapsed() > PANES_TTL
                || at.elapsed() > Duration::from_secs(2)
                    && targets
                        .into_iter()
                        .any(|target| !panes.iter().any(|pane| pane.target.as_str() == target))
        })
    }

    /// Re-read the pane names when stale for `targets`. Inside a history
    /// page this happens at most once, however many authors need it.
    fn refresh_panes<'t>(
        &mut self,
        targets: impl IntoIterator<Item = &'t str>,
        budget: &CallBudget,
    ) {
        if self.page_panes_read == Some(true) || !self.panes_stale(targets) {
            return;
        }
        if self.page_panes_read.is_some() {
            self.page_panes_read = Some(true);
        }
        // Re-resolve only with a snapshot refresh, from the original explicit
        // locator. A workspace move changes the qualified pane target; focus is
        // never consulted. A failed lookup stays retryable at the next refresh.
        let caller_budget = CallBudget {
            deadline: MonoInstant(
                budget
                    .deadline
                    .0
                    .min(self.clock.monotonic_now().0.saturating_add(750)),
            ),
            cancellation: budget.cancellation.clone(),
        };
        let caller = self
            .caller_hint
            .as_deref()
            .and_then(|hint| self.host.current_pane(hint, &caller_budget).ok());
        if self.caller != caller {
            self.caller = caller;
            self.nicks.clear();
        }
        match self.host.seat_labels(budget) {
            Ok(Some(labels)) => {
                if self.labels.as_ref() != Some(&labels) {
                    self.nicks.clear();
                }
                let panes = labels
                    .iter()
                    .map(|pane| PaneName {
                        target: pane.target.clone(),
                        label: pane.pane_label.clone(),
                        tab_label: pane.tab_label.clone(),
                        tab_pane_count: 0,
                    })
                    .collect();
                self.labels = Some(labels);
                self.panes = Some((Instant::now(), panes));
            }
            Ok(None) => {
                if let Ok(panes) = self.host.pane_names(budget) {
                    self.panes = Some((Instant::now(), panes));
                }
            }
            Err(_) => {
                // A host outage must not break durable history or retain stale
                // parent omissions after a move. Seat/target IDs remain usable.
                self.labels = None;
                self.panes = Some((Instant::now(), Vec::new()));
                self.nicks.clear();
            }
        }
    }

    fn pane_label(&mut self, target: &str, budget: &CallBudget) -> Option<String> {
        self.refresh_panes([target], budget);
        if let Some(labels) = &self.labels {
            let pane = labels.iter().find(|pane| pane.target.as_str() == target)?;
            let caller = self
                .caller
                .as_ref()
                .and_then(|target| labels.iter().find(|pane| pane.target == *target));
            return Some(irc::relative_pane_nick(pane, caller));
        }
        let (_, panes) = self.panes.as_ref()?;
        let pane = panes.iter().find(|pane| pane.target.as_str() == target)?;
        Some(
            pane.label
                .clone()
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| target.to_owned()),
        )
    }

    /// The harness of the seat's current binding, from its private local
    /// context (read only when it already exists; nothing is created).
    fn harness(&self, seat: &SeatId, generation: u64) -> Option<&'static str> {
        read_harness(&self.paths.instance_dir, self.instance, seat, generation)
    }

    fn fresh_nick(&self, seat: &SeatId) -> Option<&Nick> {
        self.nicks
            .get(seat)
            .filter(|(at, _)| at.elapsed() < NICK_TTL)
            .map(|(_, nick)| nick)
    }

    /// The nick of `seat` bound to `target` at `generation`, cached.
    fn build_nick(
        &mut self,
        seat: &SeatId,
        target: Option<&HostTargetId>,
        generation: u64,
    ) -> Nick {
        let mut nick = Nick::seat(seat);
        nick.harness = self.harness(seat, generation).map(str::to_owned);
        if matches!(nick.harness.as_deref(), Some("claude" | "codex")) {
            if let Some(target) = target {
                self.refresh_panes(
                    [target.as_str()],
                    &budget(self.clock.as_ref(), 2_000, &self.cancel),
                );
            }
            let pane = target.and_then(|target| {
                self.labels
                    .as_ref()?
                    .iter()
                    .find(|pane| pane.target == *target)
            });
            nick.name = irc::agent_seat_nick(seat, pane);
        } else if let Some(target) = target
            && let Some(label) = self.pane_label(
                target.as_str(),
                &budget(self.clock.as_ref(), 2_000, &self.cancel),
            )
        {
            nick.name = label;
        }
        self.nicks
            .insert(seat.clone(), (Instant::now(), nick.clone()));
        nick
    }

    /// Resolve the authors of one history page before it renders: one
    /// Participants page per thread (reused while fresh), one Seats page
    /// when a participant's binding is unknown, and one pane-name snapshot
    /// when stale. Only an author found in neither is left to
    /// [`Self::resolve`], which asks `SeatInspect` once and caches it.
    fn prefetch(
        &mut self,
        client: &dyn LocalClient,
        thread: &ThreadId,
        page: &Page<MessageSummary>,
        spec: &OutputSpec,
    ) {
        self.page_panes_read = Some(false);
        self.refresh_panes(
            std::iter::empty(),
            &budget(self.clock.as_ref(), 2_000, &self.cancel),
        );
        let mut missing: Vec<SeatId> = Vec::new();
        for summary in &page.items {
            let native = match &summary.event_author {
                Some(EventAuthor::Native(seat)) => Some(seat),
                _ => None,
            };
            for seat in native.into_iter().chain(summary.author.as_ref()) {
                if self.fresh_nick(seat).is_none() && !missing.contains(seat) {
                    missing.push(seat.clone());
                }
            }
        }
        if missing.is_empty() {
            return;
        }
        let first_page = PageRequest {
            cursor: None,
            limit: MAX_PAGE_LIMIT,
            max_bytes: MAX_PAGE_BYTES,
        };
        if self
            .participants
            .get(thread)
            .is_none_or(|(at, _)| at.elapsed() >= NICK_TTL)
            && let Ok(CommandResult::Participants(members)) = client.call_with_output(
                Command::Participants(ParticipantsQuery {
                    thread: thread.clone(),
                    page: first_page.clone(),
                    caller: None,
                }),
                spec,
                &budget(self.clock.as_ref(), 2_000, &self.cancel),
            )
        {
            let seats = members.items.into_iter().map(|row| row.seat).collect();
            self.participants
                .insert(thread.clone(), (Instant::now(), seats));
        }
        let Some((_, members)) = self.participants.get(thread) else {
            return;
        };
        let known: Vec<SeatId> = missing
            .into_iter()
            .filter(|seat| members.contains(seat))
            .collect();
        if known.is_empty() {
            return;
        }
        let unmapped = known.iter().any(|seat| {
            !self
                .mapped
                .as_ref()
                .is_some_and(|(at, map)| at.elapsed() < NICK_TTL && map.contains_key(seat))
        });
        if unmapped
            && let Ok(CommandResult::Seats(seats)) = client.call_with_output(
                Command::Seats(SeatsQuery {
                    page: first_page,
                    target: None,
                    include_retired: false,
                }),
                spec,
                &budget(self.clock.as_ref(), 2_000, &self.cancel),
            )
        {
            self.mapped = Some((
                Instant::now(),
                seats
                    .items
                    .into_iter()
                    .map(|row| (row.seat, (row.target, row.generation)))
                    .collect(),
            ));
        }
        let Some((_, map)) = &self.mapped else {
            return;
        };
        let bound: Vec<(SeatId, Option<HostTargetId>, u64)> = known
            .into_iter()
            .filter_map(|seat| {
                let (target, generation) = map.get(&seat)?.clone();
                Some((seat, target, generation))
            })
            .collect();
        let targets: Vec<String> = bound
            .iter()
            .filter_map(|(_, target, _)| target.as_ref().map(|target| target.as_str().to_owned()))
            .collect();
        let snapshot_budget = budget(self.clock.as_ref(), 2_000, &self.cancel);
        self.refresh_panes(targets.iter().map(String::as_str), &snapshot_budget);
        for (seat, target, generation) in bound {
            self.build_nick(&seat, target.as_ref(), generation);
        }
    }

    fn resolve(&mut self, client: &dyn LocalClient, seat: &SeatId, spec: &OutputSpec) -> Nick {
        self.refresh_panes(
            std::iter::empty(),
            &budget(self.clock.as_ref(), 2_000, &self.cancel),
        );
        if let Some(nick) = self.fresh_nick(seat) {
            return nick.clone();
        }
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
            &budget(self.clock.as_ref(), 2_000, &self.cancel),
        );
        if let Ok(CommandResult::SeatInspect(inspection)) = inspect {
            return self.build_nick(
                seat,
                inspection.summary.target.as_ref(),
                inspection.summary.generation,
            );
        }
        let nick = Nick::seat(seat);
        self.nicks
            .insert(seat.clone(), (Instant::now(), nick.clone()));
        nick
    }
}

/// The harness of `seat`'s current binding, from its private local context
/// under `instance_dir` (read only when it already exists; nothing is
/// created). This runs while a message is being rendered, so it reads a
/// lock-free snapshot ([`ContextJournal::current_snapshot`]): it never waits
/// on a check-in that holds `context.lock`, and never delays one.
fn read_harness(
    instance_dir: &std::path::Path,
    instance: uuid::Uuid,
    seat: &SeatId,
    generation: u64,
) -> Option<&'static str> {
    let root = instance_dir.canonicalize().ok()?.join("contexts");
    let dir: PathBuf = root.join(format!("{:x}", Sha256::digest(seat.as_str().as_bytes())));
    if !dir.join("context.json").is_file() {
        return None;
    }
    let journal =
        ContextJournal::open(&dir, instance, seat.as_str(), Duration::from_millis(200)).ok()?;
    let current = journal.current_snapshot().ok()??;
    if current.seat != seat.as_str() || current.binding_generation != generation {
        return None;
    }
    Some(match current.harness {
        Harness::Claude => "claude",
        Harness::Codex => "codex",
        Harness::Human => "human",
    })
}

/// A request budget of `millis` from now on `clock`. It must be the clock
/// the client compares the deadline with: a budget minted on a fresh clock
/// (which starts at zero) is already spent once the process has lived
/// longer than `millis`, and every later request fails before it is sent.
/// The call gives up as soon as `cancel` fires (Ctrl-C).
fn budget(clock: &dyn Clock, millis: u64, cancel: &Cancellation) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(millis)),
        cancellation: cancel.clone(),
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
                &budget(self.cache.clock.as_ref(), 5_000, &self.cache.cancel),
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
/// Asks the daemon to inline complete bodies only when it advertises
/// `HISTORY_FULL_BODIES`; an older daemon gets today's request and the
/// per-preview body fetches.
pub(crate) fn render_history(
    client: &crate::client::local::LocalSocketClient,
    query: HistoryQuery,
    spec: &OutputSpec,
    cache: &mut NickCache,
    writer: &mut dyn Write,
) -> Result<(), RunError> {
    let full_bodies = client
        .capabilities(&budget(cache.clock.as_ref(), 5_000, &cache.cancel))
        .supports(HISTORY_FULL_BODIES);
    render_history_with(
        client,
        full_bodies,
        query,
        spec,
        cache,
        writer,
        &live_style(),
    )
}

fn render_history_with(
    client: &dyn LocalClient,
    full_bodies: bool,
    mut query: HistoryQuery,
    spec: &OutputSpec,
    cache: &mut NickCache,
    writer: &mut dyn Write,
    style: &Style,
) -> Result<(), RunError> {
    let thread = query.thread.clone();
    query.full_bodies = full_bodies;
    let result = client.call_with_output(
        Command::History(query),
        spec,
        &budget(cache.clock.as_ref(), 5_000, &cache.cancel),
    )?;
    let CommandResult::History(page) = selected_result(&result, spec) else {
        return Err(RunError::Api(ApiError::store_corrupt(
            "daemon returned no history page",
        )));
    };
    let lookup_spec = text_spec(spec);
    cache.prefetch(client, &thread, &page, &lookup_spec);
    let mut lookup = LiveLookup {
        client,
        cache,
        spec: &lookup_spec,
    };
    let text = irc::render_page(&page, &mut lookup, style);
    lookup.cache.page_panes_read = None;
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
    /// Where notices go for the machine form (stderr in a real run).
    errors: &'a mut dyn Write,
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
            Form::Lines => writeln!(
                self.errors,
                "herdr-threads: {}",
                escape_for_terminal(text, Context::SingleLine)
            ),
        }
    }
}

/// One message summary as a single JSON line, with C1 controls and Unicode
/// line separators escaped so a peer cannot drive the terminal.
pub(crate) fn json_line(summary: &MessageSummary) -> String {
    let raw = serde_json::to_string(summary).unwrap_or_default();
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if !ch.is_ascii_control() && is_unsafe_char(ch) {
            push_u4(ch, &mut out);
        } else {
            out.push(ch);
        }
    }
    out
}

fn history(
    client: &dyn LocalClient,
    thread: &crate::protocol::ids::ThreadId,
    initial: Option<HistoryRange>,
    cursor: Option<String>,
    limit: u16,
    spec: &OutputSpec,
    budget: &CallBudget,
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
            full_bodies: false,
        }),
        spec,
        budget,
    )?;
    match selected_result(&result, spec) {
        CommandResult::History(page) => Ok(page),
        _ => Err(ApiError::store_corrupt("daemon returned no history page")),
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
    // One slow connect is the daemon being slow, not gone: retry it quietly.
    if crate::client::error_class(error) == Some(crate::protocol::results::ErrorClass::Transient)
        && error.code == ErrorCode::HostUnavailable
    {
        return FailureClass::Transient;
    }
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
        | ErrorCode::InstanceMismatch
        | ErrorCode::DaemonBootChanged => FailureClass::Transient,
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

/// What the loop does after [`failed_poll`] has told the person what it must.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Wait, reconnect and poll again.
    Retry,
    /// End the follow with status 0 (thread gone, or the output closed).
    Stop,
}

/// Apply one failed poll to the outage state and print the notice it calls
/// for. Each notice is printed at most once per outage, and a fatal one ends
/// the run with [`RunError::Exit`]: the notice is the report, so the caller's
/// top level does not print the error a second time.
fn failed_poll(
    outage: &mut Outage,
    error: &ApiError,
    now: Instant,
    thread: &crate::protocol::ids::ThreadId,
    printer: &mut Printer<'_>,
) -> Result<Step, RunError> {
    match outage.failed(error, now) {
        Verdict::Gone => {
            write_result(printer.notice(&format!(
                "thread {} no longer exists; stopped following",
                thread.as_str()
            )))?;
            Ok(Step::Stop)
        }
        Verdict::Fatal => {
            let remedy = error
                .restart_argv
                .as_ref()
                .map_or_else(String::new, |argv| format!("; restart: {}", argv.join(" ")));
            write_result(printer.notice(&format!(
                "stopped following: the daemon keeps refusing ({}){remedy}",
                error.detail
            )))?;
            Err(RunError::Exit(super::exit::api_exit_code(&error.code)))
        }
        Verdict::Announce => {
            let shown = write_result(
                printer.notice(&format!("lost the daemon ({}); reconnecting", error.detail)),
            )?;
            Ok(if shown { Step::Retry } else { Step::Stop })
        }
        Verdict::Quiet => Ok(Step::Retry),
    }
}

/// Cancels the wrapped [`Cancellation`] when dropped.
struct StopOnDrop(Cancellation);
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
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
    caller_pane: Option<&str>,
    spec: &OutputSpec,
    context: &RuntimeContext,
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut dyn Write,
) -> Result<(), RunError> {
    let cancel = install_interrupt();
    // Stops the interrupt watcher on every way out of the follow.
    let _stop_watcher = StopOnDrop(cancel.clone());
    let form = if output::human_active() {
        Form::Human
    } else {
        Form::Lines
    };
    let (instance, _, mut client) = connect(paths, clock)?;
    let mut cache = NickCache::new(context, paths, instance, clock)
        .with_caller(if form == Form::Human {
            caller_pane
        } else {
            None
        })
        .cancelled_by(&cancel);
    let mut stderr = io::stderr();
    let mut printer = Printer {
        form,
        style: if form == Form::Human {
            live_style()
        } else {
            Style::plain()
        },
        no_system: request.no_system,
        writer,
        errors: &mut stderr,
    };
    let thread = &request.thread;

    // The opening page: the recent tail (oldest first) or everything after
    // an explicit sequence. Errors here are ordinary command errors.
    let mut last = match request.after {
        Some(after) => after,
        None => {
            let page = history(
                &client,
                thread,
                Some(HistoryRange::Recent {
                    count: request.recent.max(1),
                }),
                None,
                request.recent.max(1),
                spec,
                &budget(clock.as_ref(), 5_000, &cancel),
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
                    &budget(clock.as_ref(), 5_000, &cancel),
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
        if cancel.is_cancelled() {
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
                thread,
                initial,
                cursor.take(),
                MAX_PAGE_LIMIT,
                spec,
                &budget(clock.as_ref(), 5_000, &cancel),
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
                    if page.has_more && page.next_cursor.is_some() && !cancel.is_cancelled() {
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
                if !pause(idle, &cancel) {
                    return Ok(());
                }
            }
            Err(error) => {
                // Ctrl-C cancelled the call: that is the end, not an outage.
                if cancel.is_cancelled() {
                    return Ok(());
                }
                match failed_poll(&mut outage, &error, Instant::now(), thread, &mut printer)? {
                    Step::Retry => {}
                    Step::Stop => return Ok(()),
                }
                if !pause(retry_wait, &cancel) {
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
        ApiError::new(code, "detail")
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
        let budget = budget(&Late, 5_000, &Cancellation::default());
        assert_eq!(budget.deadline, MonoInstant(65_000));
        assert!(!budget.is_exhausted(&Late));
    }
}

#[cfg(test)]
#[path = "../../tests/cli/follow_retry.rs"]
mod follow_retry;

#[cfg(test)]
#[path = "../../tests/cli/read_bodies.rs"]
mod read_bodies;
#[cfg(test)]
#[path = "../../tests/cli/read_cost_names.rs"]
mod read_cost_names;
#[cfg(test)]
#[path = "../../tests/cli/read_cost_seam.rs"]
mod read_cost_seam;
