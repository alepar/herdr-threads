//! The offered ledger of hook-pull delivery (delivery-policy DP5): which ids a
//! `DeliveryPull` showed to which binding generation of a seat, how often and
//! when. Pure data with explicit bounds; the daemon keeps one behind a `Mutex`
//! (`service::delivery_pull`) and never holds the lock across a store read.

use crate::ports::NextPull;
use crate::protocol::{ids::MessageId, ids::SeatId, time::UtcMillis};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Seats the ledger remembers; the least recently touched is evicted first.
pub const MAX_LEDGER_SEATS: usize = 4096;
/// Offers kept per seat; the oldest is evicted first.
pub const MAX_OFFERS_PER_SEAT: usize = 256;
/// An unconfirmed id is offered again only after this long.
pub const REOFFER_AFTER_MS: i64 = 60_000;
/// An id is offered at most this many times per binding generation.
pub const MAX_OFFERS_PER_GENERATION: u8 = 3;

/// One id shown to a binding generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    pub first_at: UtcMillis,
    pub last_at: UtcMillis,
    /// Times shown (1..=[`MAX_OFFERS_PER_GENERATION`]).
    pub count: u8,
    /// Shown cut at the body limit; never confirmable.
    pub truncated: bool,
}

impl Offer {
    /// Whether the id may be shown again at `now`:
    /// `now - last_at >= REOFFER_AFTER_MS && count < MAX_OFFERS_PER_GENERATION`.
    pub fn is_due(&self, now: UtcMillis) -> bool {
        now.0.saturating_sub(self.last_at.0) >= REOFFER_AFTER_MS
            && self.count < MAX_OFFERS_PER_GENERATION
    }
}

/// What one seat's current binding generation was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatOffers {
    pub generation: u64,
    pub native_session: String,
    /// The last page stopped at a bound, so more is pending behind it.
    pub more_pending: bool,
    /// The seat's wake attention counter at the last pull of this process that read the whole pending set. `None` on
    /// a new entry (a new process, an evicted seat and a rotated generation all start there).
    pub read_attention: Option<u64>,
    pub offers: BTreeMap<MessageId, Offer>,
    touched: u64,
}

impl SeatOffers {
    /// True when some offered id is due to be shown again.
    pub fn any_due(&self, now: UtcMillis) -> bool {
        self.offers.values().any(|offer| offer.is_due(now))
    }

    /// What the seat's next pull would show at `now`, given the wake attention counter.
    pub fn next_pull(&self, attention_version: u64, now: UtcMillis) -> NextPull {
        if self.read_attention != Some(attention_version)
            || self.more_pending
            // An evicted pending id would be shown again.
            || self.offers.len() >= MAX_OFFERS_PER_SEAT
            || self.any_due(now)
        {
            return NextPull::Delivers;
        }
        self.offers
            .values()
            .filter(|offer| offer.count < MAX_OFFERS_PER_GENERATION)
            .map(|offer| offer.last_at.0.saturating_add(REOFFER_AFTER_MS))
            .min()
            .map_or(NextPull::Nothing, |at| NextPull::ReofferAt(UtcMillis(at)))
    }

    /// True when some offered id was shown the maximum number of times (it waits for `inbox`).
    pub fn holds_parked(&self) -> bool {
        self.offers
            .values()
            .any(|offer| offer.count >= MAX_OFFERS_PER_GENERATION)
    }

    /// Record that `id` was shown at `now` (count + 1; the first sight sets
    /// `first_at`). Over [`MAX_OFFERS_PER_SEAT`] the oldest offer goes.
    pub fn record(&mut self, id: &MessageId, now: UtcMillis, truncated: bool) {
        let offer = self.offers.entry(id.clone()).or_insert(Offer {
            first_at: now,
            last_at: now,
            count: 0,
            truncated,
        });
        offer.count = offer.count.saturating_add(1);
        offer.last_at = now;
        offer.truncated = truncated;
        while self.offers.len() > MAX_OFFERS_PER_SEAT {
            let oldest = self
                .offers
                .iter()
                .min_by_key(|(id, offer)| (offer.first_at, (*id).clone()))
                .map(|(id, _)| id.clone());
            match oldest {
                Some(oldest) => self.offers.remove(&oldest),
                None => break,
            };
        }
    }

    /// Forget an id (settled, or already settled).
    pub fn remove(&mut self, id: &MessageId) {
        self.offers.remove(id);
    }

    /// Drop every offer whose id is not in `pending`.
    pub fn retain_pending(&mut self, pending: &BTreeSet<MessageId>) {
        self.offers.retain(|id, _| pending.contains(id));
    }
}

/// All seats' offers.
#[derive(Debug, Default)]
pub struct OfferedLedger {
    seats: HashMap<SeatId, SeatOffers>,
    clock: u64,
}

impl OfferedLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// The seat's entry for the canonical binding `generation`, created when
    /// missing. An entry of another generation is discarded first (a rotated
    /// generation discards its offered set), and a new entry over
    /// [`MAX_LEDGER_SEATS`] evicts the least recently touched seat.
    pub fn seat_mut(
        &mut self,
        seat: &SeatId,
        generation: u64,
        native_session: &str,
    ) -> &mut SeatOffers {
        self.clock += 1;
        let touched = self.clock;
        if self
            .seats
            .get(seat)
            .is_some_and(|entry| entry.generation != generation)
        {
            self.seats.remove(seat);
        }
        if !self.seats.contains_key(seat) {
            if self.seats.len() >= MAX_LEDGER_SEATS
                && let Some(oldest) = self
                    .seats
                    .iter()
                    .min_by_key(|(_, entry)| entry.touched)
                    .map(|(seat, _)| seat.clone())
            {
                self.seats.remove(&oldest);
            }
            self.seats.insert(
                seat.clone(),
                SeatOffers {
                    generation,
                    native_session: native_session.to_owned(),
                    more_pending: false,
                    read_attention: None,
                    offers: BTreeMap::new(),
                    touched,
                },
            );
        }
        let entry = self
            .seats
            .get_mut(seat)
            .expect("the entry was inserted above");
        entry.touched = touched;
        entry.native_session = native_session.to_owned();
        entry
    }

    /// The seat's entry, if any, without touching it.
    pub fn get(&self, seat: &SeatId) -> Option<&SeatOffers> {
        self.seats.get(seat)
    }

    /// Read-only: never creates an entry and never touches the LRU order. Another generation, or none, is `Unknown`.
    pub fn next_pull(
        &self,
        seat: &SeatId,
        generation: u64,
        attention_version: u64,
        now: UtcMillis,
    ) -> NextPull {
        self.seats
            .get(seat)
            .filter(|entry| entry.generation == generation)
            .map_or(NextPull::Unknown, |entry| {
                entry.next_pull(attention_version, now)
            })
    }

    /// Read-only, like [`Self::next_pull`].
    pub fn holds_parked(&self, seat: &SeatId, generation: u64) -> bool {
        self.seats
            .get(seat)
            .is_some_and(|entry| entry.generation == generation && entry.holds_parked())
    }

    pub fn seat_count(&self) -> usize {
        self.seats.len()
    }
}

#[cfg(test)]
#[path = "../../tests/delivery/ledger.rs"]
mod tests;
