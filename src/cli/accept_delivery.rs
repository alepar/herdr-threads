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
//! rows of the accepted thread, within the default inbox
//! bounds, with one `next:` line naming `inbox` when more remains. It is the caller's default
//! text-inbox display (A8), under the same claim: the page is written and flushed first, then
//! the binding's cursors advance over what it showed whole and the lazy rows complete exactly as
//! `inbox` completes them (no new provenance). Ordinary, receipt, invitation and warning rows are
//! not part of the page, so nothing is ACKed here.
//!
//! The accept is already committed and reported when this runs: nothing here changes its result
//! or exit status. A failure before any output prints nothing (the rows stay pending for the
//! next hook, wake or `inbox`); a bookkeeping failure after the page was shown is one stderr
//! line, and the rows are offered again (at least once).
use crate::{
    cli::{RunError, journal::Journal, lazy_display},
    ports::LocalClient,
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        capabilities::{BINDING_CURSORS, INBOX_BATCH_V2, INBOX_THREAD_SCOPE},
        commands::{AdvanceCursors, Command, InboxQuery, MAX_BATCH_ITEMS},
        ids::{OperationId, ThreadId},
        output::{OutputFormat, OutputSpec},
        pagination::PageRequest,
        results::{ApiError, CommandResult},
        time::{CallBudget, Clock},
    },
};
use std::io::Write;

/// Show the accepted thread's pending lazy rows after the accept result was written and flushed.
/// `text` is the caller's own text presentation (text format, not `--machine`).
#[allow(clippy::too_many_arguments)]
pub fn show<C: LocalClient + ?Sized, W: Write>(
    text: bool,
    thread: &ThreadId,
    claim: &CallerClaim,
    output: &OutputSpec,
    journal: &Journal,
    client: &C,
    clock: &dyn Clock,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
) {
    if let Err(error) = run(
        text, thread, claim, output, journal, client, clock, budget, writer,
    ) {
        eprintln!(
            "herdr-threads: accepted; opening mail display not recorded, it stays pending: {error}"
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn run<C: LocalClient + ?Sized, W: Write>(
    text: bool,
    thread: &ThreadId,
    claim: &CallerClaim,
    output: &OutputSpec,
    journal: &Journal,
    client: &C,
    clock: &dyn Clock,
    budget: &dyn Fn() -> CallBudget,
    writer: &mut W,
) -> Result<(), RunError> {
    if !text
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
    // A live mod channel delivers the binding's lazy rows itself; a failed read shows nothing.
    let Ok(digest) =
        crate::harness::bridge::read_digest_as(client, &claim.seat, Some(claim), &budget())
    else {
        return Ok(());
    };
    if !crate::delivery::routing::accept_output_page(digest.mod_channel_live) {
        return Ok(());
    }
    let page = PageRequest::default();
    let max_bytes = page.max_bytes;
    let Ok(CommandResult::InboxBatchV2(mut page)) = client.call_with_output(
        Command::InboxBatchV2(InboxQuery {
            seat: Some(claim.seat.clone()),
            page,
            claim: Some(claim.clone()),
            thread: Some(thread.clone()),
        }),
        output,
        &budget(),
    ) else {
        return Ok(());
    };
    if page.items.is_empty() {
        return Ok(());
    }
    // The daemon's continuation cursor does not carry the thread scope: what remains is read
    // with the caller's plain `inbox`, which lists these rows until they are displayed.
    if page.has_more {
        let mut argv = vec!["herdr-threads".to_owned()];
        if let Some(state_dir) = &output.context.state_dir {
            argv.extend(["--state-dir".into(), state_dir.clone()]);
        }
        if let Some(host) = &output.context.host {
            argv.extend(["--host-endpoint".into(), host.clone()]);
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
    // showed whole, then the lazy rows complete (as `inbox` does). A failed advance only means
    // the rows are offered again.
    for batch in candidates.displayed.chunks(MAX_BATCH_ITEMS) {
        let _ = client.call(
            Command::AdvanceCursors(AdvanceCursors {
                claim: claim.clone(),
                operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
                displayed: batch.to_vec(),
                in_turn: false,
                invitations: Vec::new(),
            }),
            &budget(),
        );
    }
    lazy_display::settle(
        candidates,
        journal,
        clock.utc_now().0,
        |command| client.call(command, &budget()),
        output,
    )
}
