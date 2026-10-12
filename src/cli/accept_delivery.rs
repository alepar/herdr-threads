//! The joined thread's opening lazy mail in a text `accept`'s own output (ht-br0).
//!
//! An accept moves the seat into Joined. Exact active direct-invitation acceptance keeps each own
//! cursor and inserts missing open-binding cursors at the invitation's event floor, so the pair's
//! opener is unread in the accepting binding's own stream; a generic accept runs the join
//! frontier, which marks undisplayed lazy rows past the seat's furthest cursor seat-pending
//! (TRUST-POLICY). A harness with no post-tool delivery (Codex, say) would otherwise see them only
//! at its next hook or wake, so an accept that is the last tool call of a turn would leave the
//! opener unseen that turn. The caller's own text accept therefore appends one thread-scoped page
//! of its own text inbox: the same rows `inbox` prints under the same lazy eligibility, only lazy
//! rows of the accepted thread that lie below the binding's first unread ordinary row there (the
//! display walk's stop, so every row shown whole is passed and never shown again), within the
//! default inbox bounds, with one `next:` line naming `inbox` when more remains. It is the
//! caller's default text-inbox display (A8), under the same claim: the page is written and
//! flushed first, then the binding's cursors advance over what it showed whole and the lazy rows
//! complete exactly as `inbox` completes them (no new provenance). Ordinary, receipt, invitation
//! and warning rows are not part of the page, so nothing is ACKed here.
//!
//! A live mod channel appends the binding's lazy rows to the transcript itself, possibly while
//! this accept is still running. The accept then gives the channel a bounded grace
//! ([`CHANNEL_GRACE`]): it peeks at the scoped page (read-only: nothing shown, nothing recorded)
//! until every row of the first peek has been completed by the channel, and prints nothing. When
//! the grace runs out it shows and settles the remaining rows itself, as without a channel; a
//! late channel append may then repeat them (at least once).
//!
//! The accept is already committed and reported when this runs: nothing here changes its result
//! or exit status. A failure before any output prints nothing (the rows stay pending for the
//! next hook, wake or `inbox`); a bookkeeping failure after the page was shown is one diagnostics
//! line, and the rows are offered again (at least once).
use crate::{
    cli::{RunError, journal::Journal, lazy_display, output::Presentation},
    delivery::routing::{AcceptOutputPage, accept_output_page},
    ports::LocalClient,
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        capabilities::{BINDING_CURSORS, INBOX_BATCH_V2, INBOX_THREAD_SCOPE},
        commands::{AdvanceCursors, Command, InboxQuery, MAX_BATCH_ITEMS},
        ids::{MessageId, OperationId, ThreadId},
        output::{OutputFormat, OutputSpec},
        pagination::{Page, PageRequest},
        results::{ApiError, CommandResult, InboxBatchV2Item},
        time::{CallBudget, Clock},
    },
};
use std::{
    io::Write,
    time::{Duration, Instant},
};

/// How long a text accept waits for a live mod channel to complete the accepted thread's lazy
/// rows, and how often it peeks. The channel's path is a daemon push to the `watch` child, one
/// in-process transcript append by the mod and one `watch ack` process with one daemon call:
/// normally well under a second. Two seconds covers a loaded machine while bounding the extra
/// latency of the accept's tool call; 100 ms peeks (at most 20 bounded read-only queries) keep
/// the added delay after the channel's completion small.
pub const CHANNEL_GRACE: ChannelGrace = ChannelGrace {
    bound: Duration::from_secs(2),
    interval: Duration::from_millis(100),
};

/// Test builds only: overrides [`CHANNEL_GRACE`]'s bound in milliseconds (no slow tests).
#[cfg(feature = "test-support")]
pub const CHANNEL_GRACE_ENV: &str = "HT_TEST_ACCEPT_CHANNEL_GRACE_MS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelGrace {
    pub bound: Duration,
    pub interval: Duration,
}

impl ChannelGrace {
    /// The production grace; a test-support build honours `HT_TEST_ACCEPT_CHANNEL_GRACE_MS`.
    pub fn configured() -> Self {
        #[cfg(feature = "test-support")]
        if let Some(ms) = std::env::var(CHANNEL_GRACE_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            return Self {
                bound: Duration::from_millis(ms),
                interval: CHANNEL_GRACE.interval.min(Duration::from_millis(ms.max(1))),
            };
        }
        CHANNEL_GRACE
    }
}

/// One accept's display request.
#[derive(Clone, Copy)]
pub struct AcceptPage<'a> {
    /// The caller's presentation; `--machine` shows nothing, `--human` carries into `next:`.
    pub presentation: Presentation,
    pub thread: &'a ThreadId,
    pub claim: &'a CallerClaim,
    pub output: &'a OutputSpec,
    pub journal: &'a Journal,
    pub grace: ChannelGrace,
}

/// Show the accepted thread's pending lazy rows after the accept result was written and flushed.
/// A failure is one line on `diagnostics`; it never changes the accept's result.
pub fn show<C: LocalClient + ?Sized, W: Write>(
    request: &AcceptPage<'_>,
    client: &C,
    clock: &dyn Clock,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
    diagnostics: &mut dyn Write,
) {
    if let Err(error) = run(request, client, clock, budget, writer) {
        let error = error.to_string().replace(['\n', '\r'], " ");
        let _ = writeln!(
            diagnostics,
            "herdr-threads: accepted; opening mail display not recorded, it stays pending: {error}"
        );
    }
}

fn scoped_page<C: LocalClient + ?Sized>(
    request: &AcceptPage<'_>,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
) -> Option<Page<InboxBatchV2Item>> {
    match client.call_with_output(
        Command::InboxBatchV2(InboxQuery {
            seat: Some(request.claim.seat.clone()),
            page: PageRequest::default(),
            claim: Some(request.claim.clone()),
            thread: Some(request.thread.clone()),
        }),
        request.output,
        &budget(),
    ) {
        Ok(CommandResult::InboxBatchV2(page)) => Some(page),
        _ => None,
    }
}

fn message_ids(page: &Page<InboxBatchV2Item>) -> Vec<MessageId> {
    page.items
        .iter()
        .filter_map(|item| match item {
            InboxBatchV2Item::LazyMessage { message, .. }
            | InboxBatchV2Item::Message { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

/// Whether the live channel completed every row of the first peek within the grace. A failed
/// peek counts as completed: nothing is shown and the rows stay pending for the channel.
fn channel_completes<C: LocalClient + ?Sized>(
    request: &AcceptPage<'_>,
    client: &C,
    budget: &dyn Fn() -> CallBudget,
) -> bool {
    let Some(first) = scoped_page(request, client, budget) else {
        return true;
    };
    let offered = message_ids(&first);
    if offered.is_empty() {
        return true;
    }
    let deadline = Instant::now() + request.grace.bound;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(request.grace.interval.min(deadline - now));
        let Some(page) = scoped_page(request, client, budget) else {
            return true;
        };
        if !message_ids(&page).iter().any(|id| offered.contains(id)) {
            return true;
        }
    }
}

fn run<C: LocalClient + ?Sized, W: Write>(
    request: &AcceptPage<'_>,
    client: &C,
    clock: &dyn Clock,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
) -> Result<(), RunError> {
    let AcceptPage {
        presentation,
        claim,
        output,
        journal,
        ..
    } = *request;
    if presentation == Presentation::Machine
        || output.format != OutputFormat::Text
        || claim.harness == Harness::Human
        || claim.role != CallerRole::TopLevel
    {
        return Ok(());
    }
    let probe = budget();
    if ![INBOX_BATCH_V2, BINDING_CURSORS, INBOX_THREAD_SCOPE]
        .into_iter()
        .all(|name| client.supports_capability(name, &probe))
    {
        return Ok(());
    }
    // A failed digest read shows nothing.
    let Ok(digest) =
        crate::harness::bridge::read_digest_as(client, &claim.seat, Some(claim), &budget())
    else {
        return Ok(());
    };
    if accept_output_page(digest.mod_channel_live) == AcceptOutputPage::AfterChannelGrace
        && channel_completes(request, client, budget)
    {
        return Ok(());
    }
    let max_bytes = PageRequest::default().max_bytes;
    let Some(mut page) = scoped_page(request, client, budget) else {
        return Ok(());
    };
    if page.items.is_empty() {
        return Ok(());
    }
    // The daemon's continuation cursor does not carry the thread scope: what remains is read
    // with the caller's plain `inbox` (same selectors and presentation), which lists these rows
    // until they are displayed. Rows this page shows whole are passed by its cursor advance, so
    // that `inbox` does not show them again.
    if page.has_more {
        let mut argv = vec!["herdr-threads".to_owned()];
        if let Some(state_dir) = &output.context.state_dir {
            argv.extend(["--state-dir".into(), state_dir.clone()]);
        }
        if let Some(host) = &output.context.host {
            argv.extend(["--host-endpoint".into(), host.clone()]);
        }
        if presentation == Presentation::Human {
            argv.push("--human".into());
        }
        argv.push("inbox".into());
        page.next_argv = Some(argv);
    }
    let result = CommandResult::InboxBatchV2(page);
    let candidates = lazy_display::write_page(
        &result,
        output,
        max_bytes,
        writer,
        journal,
        claim,
        lazy_display::DisplaySelection::OwnDefaultText,
    )?;
    if !candidates.ordinary.is_empty() {
        return Err(RunError::Api(ApiError::store_corrupt(
            "thread-scoped inbox page carried an ordinary message",
        )));
    }
    // Display before bookkeeping: the page was flushed; the binding's cursors move over what it
    // showed whole, then the lazy rows complete (as `inbox` does). A failure of either leaves the
    // rows to be offered again; it is reported once, after both were attempted.
    let mut advance_failed = None;
    for batch in candidates.displayed.chunks(MAX_BATCH_ITEMS) {
        if let Err(error) = client.call(
            Command::AdvanceCursors(AdvanceCursors {
                claim: claim.clone(),
                operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
                displayed: batch.to_vec(),
                in_turn: false,
                invitations: Vec::new(),
            }),
            &budget(),
        ) {
            advance_failed.get_or_insert(error);
        }
    }
    lazy_display::settle(
        candidates,
        journal,
        clock.utc_now().0,
        |command| client.call(command, &budget()),
        output,
    )?;
    match advance_failed {
        Some(error) => Err(RunError::Api(error)),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "../../tests/cli/accept_delivery.rs"]
mod tests;
