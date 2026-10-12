//! The offered ledger of hook-pull delivery (delivery-policy DP5): which ids a
//! `DeliveryPull` showed to which binding, how often and when. Each binding (one
//! native session of a seat) has its own entry, so two live bindings of one seat
//! never touch each other's offers. Pure data with explicit bounds; the daemon keeps one behind a `Mutex`
//! (`service::delivery_pull`) and never holds the lock across a store read.

use crate::ports::NextPull;
use crate::protocol::{
    ids::{BindingId, InvitationId, MessageId, SeatId},
    time::UtcMillis,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Bindings the ledger remembers; the least recently touched is evicted first.
pub const MAX_LEDGER_BINDINGS: usize = 4096;
/// Offers kept per binding. A full table evicts nothing (an evicted pending id would be offered afresh): a new id
/// waits, pending, for `inbox` until a confirm, a settle or a whole read frees a slot.
pub const MAX_OFFERS_PER_BINDING: usize = 256;
/// Independent per-binding invitation display cache, not receipt offers or wake work.
pub const MAX_INVITATION_DISPLAYS_PER_BINDING: usize = 1024;
/// An unconfirmed id is offered again only after this long.
pub const REOFFER_AFTER_MS: i64 = 60_000;
/// An id is offered at most this many times per binding generation.
pub const MAX_OFFERS_PER_GENERATION: u8 = 3;

/// One id shown to a binding generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    pub first_at: UtcMillis,
    pub last_at: UtcMillis,
    /// Recorded times shown (1..=255). Admission separately enforces the reoffer ceiling.
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

/// Scope of a finite due-key traversal; no canonical receipt authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdinaryDueScope {
    pub execution: crate::protocol::ids::ExecutionId,
    pub ordinary_version: u64,
    pub passive_generation: u64,
    pub render_key: [u8; 32],
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrdinaryDueCache {
    pub scope: Option<OrdinaryDueScope>,
    pub deferred: BTreeMap<MessageId, Offer>,
}

/// A bounded hint snapshot/delta. The incarnation and full stamps fence merges
/// against V1 recording, generation replacement and same-generation LRU recreation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrdinaryDueSnapshot {
    pub incarnation: [u8; 16],
    pub generation: u64,
    pub binding_id: BindingId,
    pub stamps: BTreeMap<MessageId, Offer>,
    pub cache: OrdinaryDueCache,
}

fn consumer_index(consumer: crate::protocol::delivery::PresentationConsumer) -> usize {
    match consumer {
        crate::protocol::delivery::PresentationConsumer::Hook => 0,
        crate::protocol::delivery::PresentationConsumer::Watch => 1,
    }
}

/// What one binding was offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingOffers {
    pub binding_id: BindingId,
    /// The seat the binding belongs to.
    pub seat: SeatId,
    pub generation: u64,
    /// The last page stopped at a bound, so more is pending behind it.
    pub more_pending: bool,
    /// The last pull left informational notices pending (a full notice page, a refused settle or a failed
    /// notice read). It defeats the `Unchanged` reply so the next pull carries them, but it is never wake
    /// work: notices never wake, so [`Self::next_pull`] ignores it.
    pub notices_pending: bool,
    /// The seat's wake attention counter at the last pull of this process that read the whole pending set, or
    /// stopped short over the set that read saw (see [`Self::whole_read`]). `None` on a new entry (a new process, an
    /// evicted seat and a rotated generation all start there).
    pub read_attention: Option<u64>,
    /// The pending-set version (the seat fingerprint's attention version without notices) at this process's last
    /// pull that read the whole pending set. While it is current a full table can neither free a slot nor take a
    /// new id, so a pull reads only as far as its due offers. `None` on a new entry.
    pub whole_read: Option<u64>,
    pub offers: BTreeMap<MessageId, Offer>,
    pub whole_invitations: BTreeMap<InvitationId, Option<u64>>,
    pub truncated_invitations: BTreeMap<InvitationId, (Option<u64>, Offer)>,
    /// Passive page continuation, ignored by native wake outlook.
    pub invitations_pending: bool,
    presentation_incarnation: [u8; 16],
    ordinary_due: [OrdinaryDueCache; 2],
    touched: u64,
}

impl BindingOffers {
    fn prune_due(&mut self) {
        for cache in &mut self.ordinary_due {
            cache
                .deferred
                .retain(|id, stamp| self.offers.get(id) == Some(stamp));
        }
    }

    pub fn due_snapshot(
        &mut self,
        consumer: crate::protocol::delivery::PresentationConsumer,
    ) -> OrdinaryDueSnapshot {
        self.prune_due();
        OrdinaryDueSnapshot {
            incarnation: self.presentation_incarnation,
            generation: self.generation,
            binding_id: self.binding_id.clone(),
            stamps: self.offers.clone(),
            cache: self.ordinary_due[consumer_index(consumer)].clone(),
        }
    }

    pub fn apply_due_update(
        &mut self,
        consumer: crate::protocol::delivery::PresentationConsumer,
        mut update: OrdinaryDueSnapshot,
    ) {
        self.prune_due();
        if update.incarnation != self.presentation_incarnation
            || update.generation != self.generation
            || update.binding_id != self.binding_id
        {
            return;
        }
        update.cache.deferred.retain(|id, stamp| {
            self.offers.get(id) == Some(stamp) && update.stamps.get(id) == Some(stamp)
        });
        self.ordinary_due[consumer_index(consumer)] = update.cache;
    }

    /// True when some offered id is due to be shown again.
    pub fn any_due(&self, now: UtcMillis) -> bool {
        self.offers.values().any(|offer| offer.is_due(now))
    }

    /// True when the table holds [`MAX_OFFERS_PER_BINDING`] offers: no new id can be recorded.
    pub fn is_full(&self) -> bool {
        self.offers.len() >= MAX_OFFERS_PER_BINDING
    }

    /// What the binding's next pull would show at `now`, given the wake attention counter. Leftover informational
    /// notices (`notices_pending`) are not counted: they never wake.
    pub fn next_pull(&self, attention_version: u64, now: UtcMillis) -> NextPull {
        if self.read_attention != Some(attention_version) || self.more_pending || self.any_due(now)
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

    /// True when some offered id was shown the maximum number of times, or the table is full (new ids are not
    /// shown): either waits for `inbox`.
    pub fn holds_parked(&self) -> bool {
        self.is_full()
            || self
                .offers
                .values()
                .any(|offer| offer.count >= MAX_OFFERS_PER_GENERATION)
    }

    /// Record that `id` was shown at `now` (count + 1; the first sight sets `first_at`) and return true. A full
    /// table records no new id and evicts none: it returns false.
    pub fn record(&mut self, id: &MessageId, now: UtcMillis, truncated: bool) -> bool {
        if self.is_full() && !self.offers.contains_key(id) {
            return false;
        }
        for cache in &mut self.ordinary_due {
            cache.deferred.remove(id);
        }
        let offer = self.offers.entry(id.clone()).or_insert(Offer {
            first_at: now,
            last_at: now,
            count: 0,
            truncated,
        });
        offer.count = offer.count.saturating_add(1);
        offer.last_at = now;
        offer.truncated = truncated;
        true
    }

    /// Forget an id (settled, or already settled).
    pub fn remove(&mut self, id: &MessageId) {
        self.offers.remove(id);
        for cache in &mut self.ordinary_due {
            cache.deferred.remove(id);
        }
    }

    /// Drop every offer whose id is not in `pending`.
    pub fn retain_pending(&mut self, pending: &BTreeSet<MessageId>) {
        self.offers.retain(|id, _| pending.contains(id));
        self.prune_due();
    }
}

/// All bindings' offers.
#[derive(Debug, Default)]
pub struct OfferedLedger {
    bindings: HashMap<BindingId, BindingOffers>,
    /// The bindings that have an entry, per seat (for the outlook of a source without per-binding information).
    seats: HashMap<SeatId, BTreeSet<BindingId>>,
    clock: u64,
}

impl OfferedLedger {
    pub fn new() -> Self {
        Self::default()
    }

    fn drop_binding(&mut self, binding: &BindingId) {
        if let Some(entry) = self.bindings.remove(binding)
            && let Some(set) = self.seats.get_mut(&entry.seat)
        {
            set.remove(binding);
            if set.is_empty() {
                self.seats.remove(&entry.seat);
            }
        }
    }

    /// The binding's entry for the canonical binding `generation`, created when missing. An entry of an older
    /// generation is discarded first (a rotated binding discards its offered set); an entry of a newer generation
    /// is left untouched and the answer is `None` (a request that read the binding before a rotation must not wipe
    /// its successor's offers). A binding id is one native session, so no other binding's entry is ever touched
    /// (a `/clear` is a new binding id). A new entry over [`MAX_LEDGER_BINDINGS`] evicts the least recently
    /// touched binding.
    pub fn binding_mut(
        &mut self,
        seat: &SeatId,
        binding: &BindingId,
        generation: u64,
    ) -> Option<&mut BindingOffers> {
        if self
            .bindings
            .get(binding)
            .is_some_and(|entry| entry.generation > generation)
        {
            return None;
        }
        self.clock += 1;
        let touched = self.clock;
        if self
            .bindings
            .get(binding)
            .is_some_and(|entry| entry.generation != generation || entry.seat != *seat)
        {
            self.drop_binding(binding);
        }
        if !self.bindings.contains_key(binding) {
            if self.bindings.len() >= MAX_LEDGER_BINDINGS
                && let Some(oldest) = self
                    .bindings
                    .iter()
                    .min_by_key(|(_, entry)| entry.touched)
                    .map(|(id, _)| id.clone())
            {
                self.drop_binding(&oldest);
            }
            self.seats
                .entry(seat.clone())
                .or_default()
                .insert(binding.clone());
            self.bindings.insert(
                binding.clone(),
                BindingOffers {
                    binding_id: binding.clone(),
                    seat: seat.clone(),
                    generation,
                    more_pending: false,
                    notices_pending: false,
                    read_attention: None,
                    whole_read: None,
                    offers: BTreeMap::new(),
                    whole_invitations: BTreeMap::new(),
                    truncated_invitations: BTreeMap::new(),
                    invitations_pending: false,
                    presentation_incarnation: *uuid::Uuid::new_v4().as_bytes(),
                    ordinary_due: Default::default(),
                    touched,
                },
            );
        }
        let entry = self
            .bindings
            .get_mut(binding)
            .expect("the entry was inserted above");
        entry.touched = touched;
        Some(entry)
    }

    /// The binding's entry, if any, without touching it.
    pub fn get(&self, binding: &BindingId) -> Option<&BindingOffers> {
        self.bindings.get(binding)
    }

    /// The entries an outlook question counts: each target's entry of this seat and generation (`Err(())` when a
    /// target has none), or, with no targets, every entry of the seat with that generation (`Err(())` when there
    /// is none).
    fn counted(
        &self,
        seat: &SeatId,
        targets: &[BindingId],
        generation: u64,
    ) -> Result<Vec<&BindingOffers>, ()> {
        let matching =
            |entry: &&BindingOffers| entry.seat == *seat && entry.generation == generation;
        if targets.is_empty() {
            let entries: Vec<&BindingOffers> = self
                .seats
                .get(seat)
                .into_iter()
                .flatten()
                .filter_map(|binding| self.bindings.get(binding))
                .filter(matching)
                .collect();
            return if entries.is_empty() {
                Err(())
            } else {
                Ok(entries)
            };
        }
        targets
            .iter()
            .map(|target| self.bindings.get(target).filter(matching).ok_or(()))
            .collect()
    }

    /// What the next pull of the bindings a wake would reach shows (`targets`; empty when the source has no
    /// per-binding information, then every entry of the seat counts). `Delivers` if any counted entry delivers,
    /// else `ReofferAt` the earliest re-offer, else `Nothing`. A target with no entry of this generation has not
    /// pulled in this process, so its next pull may deliver: the answer is `Unknown`. A superseded binding is
    /// never a target, so its stale entry cannot hold a wake back; with no targets it still counts (the exposure
    /// the seat-keyed entry had). Read-only: never creates an entry and never touches the LRU order.
    pub fn next_pull(
        &self,
        seat: &SeatId,
        targets: &[BindingId],
        generation: u64,
        attention_version: u64,
        now: UtcMillis,
    ) -> NextPull {
        let Ok(entries) = self.counted(seat, targets, generation) else {
            return NextPull::Unknown;
        };
        let mut earliest: Option<UtcMillis> = None;
        for entry in entries {
            match entry.next_pull(attention_version, now) {
                NextPull::Delivers => return NextPull::Delivers,
                NextPull::ReofferAt(at) => earliest = Some(earliest.map_or(at, |e| e.min(at))),
                NextPull::Nothing | NextPull::Unknown => {}
            }
        }
        earliest.map_or(NextPull::Nothing, NextPull::ReofferAt)
    }

    /// True if any counted entry holds a parked id or is full (see [`Self::next_pull`] for which entries count).
    /// A target with no entry counts as false. Read-only.
    pub fn holds_parked(&self, seat: &SeatId, targets: &[BindingId], generation: u64) -> bool {
        let matching =
            |entry: &&BindingOffers| entry.seat == *seat && entry.generation == generation;
        if targets.is_empty() {
            return self
                .seats
                .get(seat)
                .into_iter()
                .flatten()
                .filter_map(|binding| self.bindings.get(binding))
                .filter(matching)
                .any(BindingOffers::holds_parked);
        }
        targets
            .iter()
            .filter_map(|target| self.bindings.get(target))
            .filter(matching)
            .any(BindingOffers::holds_parked)
    }

    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }
}

#[cfg(test)]
#[path = "../../tests/delivery/ledger.rs"]
mod tests;
