//! The merge's ordering classes and caller subsets.
//!
//! `OrderClass`: what the merge compares heads across. Owns the class membership of the five
//! `Source`s (Lazy -> DeliveryRows; Invitations, Unread, Receipts -> Deadline; Warnings ->
//! Warnings) and the per-class ensure-head, head-key, advance and done operations. A new ordering
//! strategy is a new arm here. Must not: compare keys (`DeliveryOrderKey` does) or hold rows
//! across requests.
//!
//! `PageSubset`: which classes and walks a caller sees, applied at capture and after cursor decode
//! only. A new caller is one new variant. Must not: add per-subset branches inside the merge or
//! the classes. Callers never set done flags or test `ordinary_only`/thread scope to hide classes
//! (`ordinary_only` survives only as private presentation eligibility).
//!
//! `Merge`: the per-request head cache. Heads live in memory for one request; only positions
//! round-trip through the cursor, and a peeked-but-unemitted head is re-peeked next page.

use super::deadline_window::{DeadlineItem, DeadlineRef, DeadlineWindow, WINDOW_VISITS};
use super::*;
use crate::protocol::{
    pagination::DeliveryRowsPosition,
    priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
};
use crate::store::{
    catch_up::HoldCache,
    lazy_delivery::{Recipient, RowFilter, seek_pending},
};

/// Visits one window collection may overshoot `WINDOW_VISITS` by (one walk step).
const WINDOW_SLACK: usize = 8;

/// What the merge compares heads across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OrderClass {
    DeliveryRows,
    Deadline,
    Warnings,
}

impl OrderClass {
    /// The collection order: the bounded deadline window first, then the delivery-row seek with
    /// the remaining work, then the warning scan (whose 2 visits the row seek leaves in reserve).
    pub(super) const ALL: [OrderClass; 3] = [
        OrderClass::Deadline,
        OrderClass::DeliveryRows,
        OrderClass::Warnings,
    ];

    pub(super) fn of(source: Source) -> Self {
        match source {
            Source::Lazy => Self::DeliveryRows,
            Source::Invitations | Source::Unread | Source::Receipts => Self::Deadline,
            Source::Warnings => Self::Warnings,
        }
    }

    pub(super) fn done(self, state: &State) -> bool {
        match self {
            Self::DeliveryRows => state.rows.done,
            Self::Deadline => state.deadline.done,
            Self::Warnings => state.warnings.done,
        }
    }
}

/// Which classes and walks a caller sees.
#[derive(Debug, Clone, Copy)]
pub(super) enum PageSubset<'a> {
    /// CLI inbox, delivery pull and watch: every class.
    All,
    /// The private ordinary presentation: delivery rows and the receipt-bearing walks (binding
    /// Unread and Receipts), no invitations and no warnings.
    Presentation,
    /// Accept's thread-scoped display: the thread's delivery rows only.
    ThreadScoped(&'a ThreadId),
    /// A hook's inbox check-in page: every class, with the catch-up hold applied to the deadline
    /// class' ordinary messages as in the attention digest.
    Pushed,
}

impl PageSubset<'_> {
    /// Mark the classes and walks this subset never shows as finished. Idempotent.
    pub(super) fn apply(&self, state: &mut State, reader: &Reader) {
        match self {
            Self::All | Self::Pushed => {}
            Self::Presentation => {
                state.warnings.done = true;
                let invitations = state.deadline.high.invitation;
                state.deadline.window_start.invitation_after = invitations;
                if let Some(end) = &mut state.deadline.window_end {
                    end.invitation_after = invitations;
                }
            }
            Self::ThreadScoped(_) => {
                state.warnings.done = true;
                state.deadline.done = true;
                // A claim that names no binding of the seat would complete in the pre-binding
                // seat-wide form, whose cursor this page cannot bound: it shows no row.
                if matches!(reader, Reader::Unknown) {
                    state.rows.done = true;
                }
            }
        }
    }

    pub(super) fn private(&self) -> bool {
        matches!(self, Self::Presentation)
    }

    fn pushed(&self) -> bool {
        matches!(self, Self::Pushed)
    }
}

pub(super) struct WarningHead {
    key: DeliveryOrderKey,
    item: Item,
    /// The scan position just past this warning.
    after: (i64, i64),
}

/// The merge's per-request state: the immutable request context, the work charged so far and the
/// heads currently held.
pub(super) struct Merge<'a> {
    db: &'a Connection,
    instance: &'a str,
    seat: &'a SeatId,
    reading: Option<&'a BindingId>,
    current_agent: bool,
    filter: RowFilter<'a>,
    private: bool,
    hold: Option<HoldCache<'a>>,
    pub(super) visited: usize,
    pub(super) lazy_scope_probes: usize,
    row: Option<(DeliveryRowsPosition, Recipient)>,
    window: Option<DeadlineWindow>,
    warning: Option<WarningHead>,
}

pub(super) fn row_filter<'a>(
    db: &Connection,
    subset: &PageSubset<'a>,
    reader: &'a Reader,
    seat: &SeatId,
) -> Result<RowFilter<'a>, ApiError> {
    Ok(match (subset, reader) {
        (PageSubset::Presentation, Reader::Bound(binding)) => {
            RowFilter::PrivatePresentation(binding)
        }
        (PageSubset::ThreadScoped(thread), Reader::Bound(binding)) => {
            let stop =
                crate::store::read_cursors::first_ordinary_walk_stop(db, binding, seat, thread)?
                    .map_or(Ok(i64::MAX), i64::try_from)
                    .map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "message sequence out of range")
                    })?;
            RowFilter::ThreadBinding {
                binding,
                thread,
                stop,
            }
        }
        (PageSubset::ThreadScoped(thread), _) => RowFilter::Thread(thread),
        (_, Reader::Bound(binding)) => RowFilter::Binding(binding),
        _ => RowFilter::Seat,
    })
}

pub(super) fn row_key(position: DeliveryRowsPosition) -> DeliveryOrderKey {
    DeliveryOrderKey {
        priority: position.priority,
        deadline: None,
        decision_seq: position.decision_seq,
        rank: SourceRank::DeliveryRow,
        ordinal: position.ordinal,
    }
}

impl<'a> Merge<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        db: &'a Connection,
        instance: &'a str,
        seat: &'a SeatId,
        reading: Option<&'a BindingId>,
        current_agent: bool,
        filter: RowFilter<'a>,
        subset: &PageSubset<'_>,
        visited: usize,
    ) -> Self {
        Self {
            db,
            instance,
            seat,
            reading,
            current_agent,
            filter,
            private: subset.private(),
            hold: subset.pushed().then(|| HoldCache::new(seat.as_str())),
            visited,
            lazy_scope_probes: 0,
            row: None,
            window: None,
            warning: None,
        }
    }

    /// Make every unfinished class hold a head or be done. `false`: the page's work ran out first
    /// (the classes' positions already carry the progress made).
    pub(super) fn ensure(&mut self, state: &mut State) -> Result<bool, ApiError> {
        for class in OrderClass::ALL {
            if !self.ensure_class(class, state)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn ensure_class(
        &mut self,
        class: OrderClass,
        state: &mut State,
    ) -> Result<bool, ApiError> {
        match class {
            OrderClass::DeliveryRows => self.ensure_rows(state),
            OrderClass::Deadline => self.ensure_deadline(state),
            OrderClass::Warnings => self.ensure_warnings(state),
        }
    }

    fn ensure_rows(&mut self, state: &mut State) -> Result<bool, ApiError> {
        if state.rows.done || self.row.is_some() {
            return Ok(true);
        }
        let reserve = if state.warnings.done { 0 } else { 2 };
        let remaining = CANDIDATE_LIMIT
            .saturating_sub(self.visited)
            .saturating_sub(reserve);
        if remaining == 0 {
            return Ok(false);
        }
        let seek = seek_pending(
            self.db,
            self.instance,
            self.seat,
            self.filter,
            state.rows.after,
            state.publication_decision_high_water,
            remaining,
        )?;
        self.visited += seek.visited;
        if self.private {
            // Every rejected or accepted candidate costs the private eligibility test's probes.
            self.lazy_scope_probes += 3 * seek.visited;
        }
        match seek.row {
            Some(row) => {
                self.row = Some(row);
                Ok(true)
            }
            None if seek.exhausted => {
                state.rows.done = true;
                Ok(true)
            }
            None => {
                state.rows.after = seek.position;
                Ok(false)
            }
        }
    }

    fn ensure_deadline(&mut self, state: &mut State) -> Result<bool, ApiError> {
        loop {
            if self.window.as_ref().is_some_and(|w| w.head().is_some()) {
                return Ok(true);
            }
            if let Some(window) = self.window.take() {
                window.persist(&mut state.deadline);
            }
            if state.deadline.done {
                return Ok(true);
            }
            if CANDIDATE_LIMIT.saturating_sub(self.visited) < WINDOW_VISITS + WINDOW_SLACK {
                return Ok(false);
            }
            self.window = Some(DeadlineWindow::collect(
                self.db,
                self.seat,
                self.reading,
                self.current_agent,
                &state.deadline,
                state.publication_decision_high_water,
                state.last.as_ref(),
                &mut self.visited,
                &mut self.hold,
            )?);
        }
    }

    fn ensure_warnings(&mut self, state: &mut State) -> Result<bool, ApiError> {
        loop {
            if state.warnings.done || self.warning.is_some() {
                return Ok(true);
            }
            if CANDIDATE_LIMIT.saturating_sub(self.visited) < 2 {
                return Ok(false);
            }
            let slice = scan_global_logical_candidates(
                self.db,
                self.instance,
                GlobalLogicalKinds::Warnings,
                Some(GlobalLogicalPosition {
                    after_decision_seq: state.warnings.physical_after,
                    after_event_offset: state.warnings.manifest_after,
                    high_water_decision_seq: state.publication_decision_high_water as i64,
                }),
                2,
            )?;
            self.visited += slice.visited as usize;
            let after = (
                slice.position.after_decision_seq,
                slice.position.after_event_offset,
            );
            let Some(candidate) = slice.candidates.into_iter().next() else {
                state.warnings.done = true;
                return Ok(true);
            };
            match warning_item(
                self.db,
                self.seat,
                self.reading,
                &candidate,
                state.warnings.offered,
            )? {
                Some(item) => {
                    self.warning = Some(WarningHead {
                        key: DeliveryOrderKey {
                            priority: DeliveryPriority::p0(),
                            deadline: None,
                            decision_seq: candidate.decision_seq.max(0) as u64,
                            rank: SourceRank::Warning,
                            ordinal: candidate.event_offset.max(0) as u64,
                        },
                        item,
                        after,
                    });
                }
                None => {
                    // A rejected candidate is passed for good.
                    (state.warnings.physical_after, state.warnings.manifest_after) = after;
                }
            }
        }
    }

    /// The smallest head across the classes.
    pub(super) fn pick(&self) -> Option<(OrderClass, DeliveryOrderKey)> {
        OrderClass::ALL
            .into_iter()
            .filter_map(|class| self.head_key(class).map(|key| (class, key)))
            .min_by_key(|(_, key)| *key)
    }

    fn head_key(&self, class: OrderClass) -> Option<DeliveryOrderKey> {
        match class {
            OrderClass::DeliveryRows => self.row.as_ref().map(|(position, _)| row_key(*position)),
            OrderClass::Deadline => self
                .window
                .as_ref()
                .and_then(|w| w.head())
                .map(|item| item.key),
            OrderClass::Warnings => self.warning.as_ref().map(|w| w.key),
        }
    }

    /// A deadline-class body continuation is emitted straight from its message: drop that
    /// message's item from the held window (advancing past it when it is the head). Returns the
    /// head's key when it was the head.
    pub(super) fn settle_body(
        &mut self,
        state: &mut State,
        message: &MessageId,
    ) -> Option<DeliveryOrderKey> {
        let window = self.window.as_mut()?;
        let key = window.take_message(message);
        if key.is_some() {
            window.persist(&mut state.deadline);
        }
        key
    }

    /// Whether a deadline-class body message is acknowledgeable by the reader: an agent reader
    /// and a receipt still pending within the publication bound.
    pub(super) fn body_ack(&self, message: &MessageId, decision: u64) -> Result<bool, ApiError> {
        Ok(self.current_agent
            && crate::store::effective::effective_receipt(
                self.db,
                message.as_str(),
                self.seat.as_str(),
            )?
            .is_some_and(|r| {
                r.state == EffectiveReceiptState::Pending
                    && r.decision_seq.unwrap_or(0) <= decision as i64
            }))
    }

    /// The inbox item of a resumed deadline-class body, hydrated from its message at `position`.
    pub(super) fn body_item(
        &self,
        message: &MessageId,
        ack: bool,
        position: &InboxBatchV2BodyPosition,
    ) -> Result<Option<Item>, ApiError> {
        hydrate_message(
            self.db,
            message.as_str(),
            false,
            ack,
            self.seat,
            Some(position),
        )
    }

    /// The window head, when the Deadline class holds one.
    pub(super) fn window_head(&self) -> Option<&DeadlineItem> {
        self.window.as_ref().and_then(|w| w.head())
    }

    /// The inbox item of `class`'s head and the `Source` that owns its body, with `position`
    /// resuming a partly shown body. `None`: the head no longer has an item (it is passed).
    pub(super) fn item(
        &self,
        class: OrderClass,
        state: &State,
        position: Option<&InboxBatchV2BodyPosition>,
    ) -> Result<Option<(Item, Source)>, ApiError> {
        let seat = self.seat;
        Ok(match class {
            OrderClass::DeliveryRows => {
                // A resumed body has no peeked head: the cursor body names the message.
                let message = match (&self.row, position) {
                    (Some((_, recipient)), _) => recipient.message.as_str(),
                    (None, Some(body)) => body.message.as_str(),
                    (None, None) => unreachable!("delivery row head or resumed body"),
                };
                hydrate_message(self.db, message, true, false, seat, position)?
                    .map(|item| (item, Source::Lazy))
            }
            OrderClass::Warnings => self
                .warning
                .as_ref()
                .map(|w| (w.item.clone(), Source::Warnings)),
            OrderClass::Deadline => {
                let head = self.window_head().expect("deadline head");
                match &head.item {
                    DeadlineRef::Invitation { ordinal, .. } => {
                        let step = next_invitation(
                            self.db,
                            seat,
                            ordinal - 1,
                            *ordinal,
                            state.publication_decision_high_water,
                        )?;
                        match step {
                            Some(step) => invitation_item(self.db, seat, step)?
                                .map(|item| (item, Source::Invitations)),
                            None => None,
                        }
                    }
                    DeadlineRef::Unread { message, ack, .. } => {
                        hydrate_message(self.db, message.as_str(), false, *ack, seat, position)?
                            .map(|item| (item, Source::Unread))
                    }
                    DeadlineRef::Receipt { message, ack, .. } => {
                        hydrate_message(self.db, message.as_str(), false, *ack, seat, position)?
                            .map(|item| (item, Source::Receipts))
                    }
                }
            }
        })
    }

    /// Move `class` past its emitted head `key` (a delivery-row key also names the row position
    /// when the head is a resumed body that was never peeked).
    pub(super) fn advance(&mut self, class: OrderClass, state: &mut State, key: DeliveryOrderKey) {
        match class {
            OrderClass::DeliveryRows => {
                self.row = None;
                state.rows.after = Some(DeliveryRowsPosition {
                    priority: key.priority,
                    decision_seq: key.decision_seq,
                    ordinal: key.ordinal,
                });
            }
            OrderClass::Deadline => {
                if let Some(window) = self.window.as_mut() {
                    window.advance();
                    window.persist(&mut state.deadline);
                }
            }
            OrderClass::Warnings => {
                if let Some(head) = self.warning.take() {
                    (state.warnings.physical_after, state.warnings.manifest_after) = head.after;
                }
            }
        }
    }

    /// The cursor state of a page that ends inside `class`'s head body: `next` is the state
    /// before the head, which the deadline window must re-collect exactly.
    pub(super) fn pin_partial(&self, class: OrderClass, next: &mut State) {
        if class == OrderClass::Deadline
            && let Some(window) = &self.window
        {
            window.persist_partial(&mut next.deadline);
        }
    }
}
