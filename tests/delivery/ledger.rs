use super::*;

fn id(n: usize) -> MessageId {
    MessageId::new(format!("m{n:05}"))
}
fn seat(n: usize) -> SeatId {
    SeatId::new(format!("s{n}"))
}
fn bind(n: usize) -> BindingId {
    BindingId::new(format!("b{n}"))
}
fn at(ms: i64) -> UtcMillis {
    UtcMillis(ms)
}

#[test]
fn records_and_looks_up_offers_per_seat_and_generation() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 4).unwrap();
    entry.record(&id(1), at(1_000), false);
    entry.record(&id(2), at(1_500), true);
    let entry = ledger.get(&bind(1)).unwrap();
    assert_eq!(entry.generation, 4);
    assert_eq!(entry.seat, seat(1));
    assert_eq!(entry.offers.len(), 2);
    assert_eq!(
        entry.offers[&id(1)],
        Offer {
            first_at: at(1_000),
            last_at: at(1_000),
            count: 1,
            truncated: false
        }
    );
    assert!(entry.offers[&id(2)].truncated);
    // Another seat has its own entry.
    assert!(ledger.get(&bind(2)).is_none());
    assert_eq!(
        ledger
            .binding_mut(&seat(2), &bind(2), 1)
            .unwrap()
            .offers
            .len(),
        0
    );
    assert_eq!(ledger.get(&bind(1)).unwrap().offers.len(), 2);
}

#[test]
fn same_generation_keeps_the_entry() {
    let mut ledger = OfferedLedger::new();
    ledger
        .binding_mut(&seat(1), &bind(1), 4)
        .unwrap()
        .record(&id(1), at(0), false);
    assert_eq!(
        ledger
            .binding_mut(&seat(1), &bind(1), 4)
            .unwrap()
            .offers
            .len(),
        1
    );
}

#[test]
fn a_rotated_generation_discards_the_seats_offers_and_flag() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 4).unwrap();
    entry.record(&id(1), at(0), false);
    entry.more_pending = true;
    entry.whole_read = Some(9);
    let entry = ledger.binding_mut(&seat(1), &bind(1), 5).unwrap();
    assert_eq!(entry.generation, 5);
    assert!(entry.offers.is_empty());
    assert!(!entry.more_pending);
    assert_eq!(entry.whole_read, None);
}

#[test]
fn due_rule_is_sixty_seconds_and_fewer_than_three_offers() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.record(&id(1), at(10_000), false);
    let offer = entry.offers[&id(1)];
    assert!(!offer.is_due(at(10_000 + REOFFER_AFTER_MS - 1)));
    assert!(offer.is_due(at(10_000 + REOFFER_AFTER_MS)));
    assert!(!entry.any_due(at(10_000 + REOFFER_AFTER_MS - 1)));
    assert!(entry.any_due(at(10_000 + REOFFER_AFTER_MS)));
    // The 60 s runs from the last showing, and the third showing parks it.
    entry.record(&id(1), at(70_000), false);
    assert!(!entry.offers[&id(1)].is_due(at(70_000 + REOFFER_AFTER_MS - 1)));
    assert!(entry.offers[&id(1)].is_due(at(130_000)));
    entry.record(&id(1), at(130_000), false);
    assert_eq!(entry.offers[&id(1)].count, MAX_OFFERS_PER_GENERATION);
    assert!(!entry.offers[&id(1)].is_due(at(10_000_000)));
    assert!(!entry.any_due(at(10_000_000)));
}

#[test]
fn recording_again_counts_and_keeps_the_first_sighting() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.record(&id(1), at(5), false);
    entry.record(&id(1), at(99), true);
    let offer = entry.offers[&id(1)];
    assert_eq!(
        (offer.first_at, offer.last_at, offer.count),
        (at(5), at(99), 2)
    );
    assert!(offer.truncated);
}

// Kills: evicting a still-pending offer at the cap (main review Important 2).
#[test]
fn a_full_offer_table_keeps_every_offer_and_records_no_new_id() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    for n in 0..MAX_OFFERS_PER_BINDING {
        assert!(entry.record(&id(n), at(n as i64), false));
    }
    assert!(entry.is_full());
    assert!(
        !entry.record(&id(MAX_OFFERS_PER_BINDING), at(1_000), false),
        "no slot for a new id"
    );
    assert_eq!(entry.offers.len(), MAX_OFFERS_PER_BINDING);
    assert!(
        entry.offers.contains_key(&id(0)),
        "the oldest offer is kept"
    );
    assert!(!entry.offers.contains_key(&id(MAX_OFFERS_PER_BINDING)));
    assert!(
        entry.record(&id(0), at(REOFFER_AFTER_MS), false),
        "a known id is still recorded"
    );
    assert_eq!(entry.offers[&id(0)].count, 2);
    entry.remove(&id(1));
    assert!(
        entry.record(&id(MAX_OFFERS_PER_BINDING), at(2_000), false),
        "a freed slot takes a new id"
    );
}

#[test]
fn seats_are_bounded_and_the_least_recently_touched_goes_first() {
    let mut ledger = OfferedLedger::new();
    for n in 0..MAX_LEDGER_BINDINGS {
        ledger
            .binding_mut(&seat(n), &bind(n), 1)
            .unwrap()
            .record(&id(0), at(0), false);
    }
    assert_eq!(ledger.binding_count(), MAX_LEDGER_BINDINGS);
    // Touch seat 0 so seat 1 is now the least recently touched.
    ledger.binding_mut(&seat(0), &bind(0), 1).unwrap();
    ledger
        .binding_mut(&seat(MAX_LEDGER_BINDINGS), &bind(MAX_LEDGER_BINDINGS), 1)
        .unwrap();
    assert_eq!(ledger.binding_count(), MAX_LEDGER_BINDINGS);
    assert!(ledger.get(&bind(0)).is_some(), "a touched seat survives");
    assert!(
        ledger.get(&bind(1)).is_none(),
        "the stalest seat is evicted"
    );
    assert!(ledger.get(&bind(MAX_LEDGER_BINDINGS)).is_some());
}

#[test]
fn retain_pending_drops_ids_no_longer_pending_and_remove_forgets_one() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    for n in 1..=3 {
        entry.record(&id(n), at(0), false);
    }
    entry.retain_pending(&BTreeSet::from([id(1), id(3), id(9)]));
    assert_eq!(
        entry.offers.keys().cloned().collect::<Vec<_>>(),
        vec![id(1), id(3)]
    );
    entry.remove(&id(1));
    assert_eq!(
        entry.offers.keys().cloned().collect::<Vec<_>>(),
        vec![id(3)]
    );
}

#[test]
fn next_pull_is_unknown_without_an_entry_of_that_generation() {
    let mut ledger = OfferedLedger::new();
    assert_eq!(
        ledger.next_pull(&seat(1), &[bind(1)], 1, 7, at(0)),
        NextPull::Unknown
    );
    assert!(!ledger.holds_parked(&seat(1), &[bind(1)], 1));
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.read_attention = Some(7);
    for _ in 0..MAX_OFFERS_PER_GENERATION {
        entry.record(&id(1), at(0), false);
    }
    assert_eq!(
        ledger.next_pull(&seat(1), &[bind(1)], 2, 7, at(0)),
        NextPull::Unknown
    );
    assert!(!ledger.holds_parked(&seat(1), &[bind(1)], 2));
    assert!(
        ledger.holds_parked(&seat(1), &[bind(1)], 1),
        "the control: generation 1 does hold one"
    );
}

#[test]
fn next_pull_delivers_until_a_whole_read_matches_the_attention_version() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    assert_eq!(entry.next_pull(7, at(0)), NextPull::Delivers, "never read");
    entry.read_attention = Some(7);
    assert_eq!(entry.next_pull(7, at(0)), NextPull::Nothing, "the control");
    assert_eq!(
        entry.next_pull(8, at(0)),
        NextPull::Delivers,
        "new attention"
    );
    entry.more_pending = true;
    assert_eq!(
        entry.next_pull(7, at(0)),
        NextPull::Delivers,
        "more behind a page"
    );
}

#[test]
fn next_pull_waits_for_the_earliest_due_reoffer() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.record(&id(1), at(0), false);
    entry.record(&id(2), at(10_000), false);
    entry.read_attention = Some(7);
    assert_eq!(
        entry.next_pull(7, at(1_000)),
        NextPull::ReofferAt(at(REOFFER_AFTER_MS))
    );
    assert_eq!(entry.next_pull(7, at(REOFFER_AFTER_MS)), NextPull::Delivers);
}

#[test]
fn pending_notices_are_not_wake_work() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.read_attention = Some(7);
    entry.notices_pending = true;
    assert_eq!(
        entry.next_pull(7, at(0)),
        NextPull::Nothing,
        "notices alone never wake"
    );
    entry.record(&id(1), at(0), false);
    assert_eq!(
        entry.next_pull(7, at(10_000)),
        NextPull::ReofferAt(at(REOFFER_AFTER_MS)),
        "notices do not bring a held re-offer forward"
    );
    let entry = ledger.binding_mut(&seat(1), &bind(1), 2).unwrap();
    assert!(
        !entry.notices_pending,
        "a rotated generation starts without the flag"
    );
}

#[test]
fn next_pull_is_nothing_once_every_offer_is_parked() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.read_attention = Some(7);
    assert_eq!(entry.next_pull(7, at(0)), NextPull::Nothing);
    assert!(!entry.holds_parked());
    for _ in 0..MAX_OFFERS_PER_GENERATION {
        entry.record(&id(1), at(0), false);
    }
    assert_eq!(
        entry.next_pull(7, at(10 * REOFFER_AFTER_MS)),
        NextPull::Nothing
    );
    assert!(entry.holds_parked());
}

// Kills: a full table answering Delivers forever (a rule that existed only for eviction) instead of parking.
#[test]
fn a_full_offer_table_parks_for_inbox_instead_of_delivering() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.read_attention = Some(7);
    for n in 0..MAX_OFFERS_PER_BINDING {
        entry.record(&id(n), at(0), false);
    }
    assert_eq!(
        entry.next_pull(7, at(1)),
        NextPull::ReofferAt(at(REOFFER_AFTER_MS))
    );
    assert!(entry.holds_parked(), "new ids wait for inbox");
    assert!(ledger.holds_parked(&seat(1), &[bind(1)], 1));
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.remove(&id(0));
    assert!(
        !entry.holds_parked(),
        "the control: one below the bound, no id at three showings"
    );
}

// Kills: an older-generation request wiping the newer generation's offers (main review, ledger.rs binding_mut).
#[test]
fn an_older_generation_never_replaces_a_newer_entry() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 5).unwrap();
    entry.record(&id(1), at(0), false);
    entry.read_attention = Some(7);
    assert!(
        ledger.binding_mut(&seat(1), &bind(1), 4).is_none(),
        "a stale generation gets no entry"
    );
    let entry = ledger.get(&bind(1)).unwrap();
    assert_eq!(entry.generation, 5);
    assert_eq!(entry.offers.len(), 1);
    assert_eq!(entry.read_attention, Some(7));
    let entry = ledger.binding_mut(&seat(1), &bind(1), 6).unwrap();
    assert!(
        entry.offers.is_empty(),
        "a newer generation still replaces the entry"
    );
}

#[test]
fn next_pull_never_touches_the_lru_order() {
    let mut ledger = OfferedLedger::new();
    for n in 0..MAX_LEDGER_BINDINGS {
        ledger.binding_mut(&seat(n), &bind(n), 1).unwrap();
    }
    // Asking about seat 0 (the oldest) must not refresh it.
    ledger.next_pull(&seat(0), &[bind(0)], 1, 7, at(0));
    ledger.holds_parked(&seat(0), &[bind(0)], 1);
    ledger
        .binding_mut(&seat(MAX_LEDGER_BINDINGS), &bind(MAX_LEDGER_BINDINGS), 1)
        .unwrap();
    assert!(
        ledger.get(&bind(0)).is_none(),
        "seat 0 is still the stalest"
    );
    assert!(ledger.get(&bind(1)).is_some());
}

// Kills: one entry per seat, so a second binding's pull replaced the first's offers, count and spacing.
#[test]
fn two_bindings_of_one_seat_keep_separate_offers_counts_and_spacing() {
    let mut ledger = OfferedLedger::new();
    let (a, b) = (bind(1), bind(2));
    let entry = ledger.binding_mut(&seat(1), &a, 1).unwrap();
    entry.record(&id(1), at(0), false);
    entry.record(&id(1), at(REOFFER_AFTER_MS), false);
    let entry = ledger.binding_mut(&seat(1), &b, 1).unwrap();
    assert!(entry.offers.is_empty(), "B starts empty");
    entry.record(&id(1), at(2 * REOFFER_AFTER_MS), false);
    let entry = ledger.binding_mut(&seat(1), &a, 1).unwrap();
    assert_eq!(entry.offers[&id(1)].count, 2, "A's count survives B's pull");
    assert_eq!(entry.offers[&id(1)].last_at, at(REOFFER_AFTER_MS));
    assert_eq!(ledger.get(&b).unwrap().offers[&id(1)].count, 1);
    assert_eq!(
        ledger.get(&b).unwrap().offers[&id(1)].first_at,
        at(2 * REOFFER_AFTER_MS)
    );
    assert_eq!(ledger.binding_count(), 2);
    // Each has its own 256-id table.
    let entry = ledger.binding_mut(&seat(1), &a, 1).unwrap();
    for n in 2..=MAX_OFFERS_PER_BINDING {
        assert!(entry.record(&id(n), at(0), false));
    }
    assert!(entry.is_full());
    assert!(!ledger.get(&b).unwrap().is_full(), "B's table is not A's");
}

#[test]
fn a_rotated_generation_discards_only_that_bindings_offers() {
    let mut ledger = OfferedLedger::new();
    ledger
        .binding_mut(&seat(1), &bind(1), 1)
        .unwrap()
        .record(&id(1), at(0), false);
    ledger
        .binding_mut(&seat(1), &bind(2), 1)
        .unwrap()
        .record(&id(1), at(0), false);
    assert!(
        ledger
            .binding_mut(&seat(1), &bind(1), 2)
            .unwrap()
            .offers
            .is_empty()
    );
    assert_eq!(ledger.get(&bind(2)).unwrap().offers.len(), 1);
}

#[test]
fn a_newer_generation_entry_is_not_touched() {
    let mut ledger = OfferedLedger::new();
    ledger
        .binding_mut(&seat(1), &bind(1), 5)
        .unwrap()
        .record(&id(1), at(0), false);
    assert!(ledger.binding_mut(&seat(1), &bind(1), 4).is_none());
    assert_eq!(ledger.get(&bind(1)).unwrap().offers.len(), 1);
    assert_eq!(ledger.get(&bind(1)).unwrap().generation, 5);
}

// Kills: an eviction that leaves the seat index pointing at an evicted binding (a stale entry in the
// empty-targets outlook) or that counts seats.
#[test]
fn eviction_counts_bindings_and_keeps_the_seat_index_in_step() {
    let mut ledger = OfferedLedger::new();
    // Every binding belongs to one seat.
    for n in 0..MAX_LEDGER_BINDINGS {
        let entry = ledger.binding_mut(&seat(1), &bind(n), 1).unwrap();
        entry.read_attention = Some(7);
        entry.record(&id(0), at(0), false);
    }
    assert_eq!(ledger.binding_count(), MAX_LEDGER_BINDINGS);
    ledger.binding_mut(&seat(1), &bind(0), 1).unwrap();
    ledger
        .binding_mut(&seat(1), &bind(MAX_LEDGER_BINDINGS), 1)
        .unwrap();
    assert_eq!(ledger.binding_count(), MAX_LEDGER_BINDINGS);
    assert!(ledger.get(&bind(0)).is_some(), "a touched binding survives");
    assert!(ledger.get(&bind(1)).is_none(), "the stalest is evicted");
    assert_eq!(
        ledger.seats[&seat(1)].len(),
        MAX_LEDGER_BINDINGS,
        "the seat index lost the evicted binding and gained the new one"
    );
    assert!(!ledger.seats[&seat(1)].contains(&bind(1)));
    assert!(ledger.seats[&seat(1)].contains(&bind(MAX_LEDGER_BINDINGS)));
    // A generation change keeps exactly one index entry for the binding.
    let mut ledger = OfferedLedger::new();
    ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    ledger.binding_mut(&seat(1), &bind(1), 2).unwrap();
    assert_eq!(ledger.seats[&seat(1)].len(), 1);
}

#[test]
fn outlook_over_targets() {
    let mut ledger = OfferedLedger::new();
    let (a, b) = (bind(1), bind(2));
    let entry = ledger.binding_mut(&seat(1), &a, 1).unwrap();
    entry.read_attention = Some(7);
    entry.record(&id(1), at(0), false);
    let entry = ledger.binding_mut(&seat(1), &b, 1).unwrap();
    entry.read_attention = Some(7);
    entry.record(&id(1), at(10_000), false);
    let now = at(20_000);
    let both = [a.clone(), b.clone()];
    assert_eq!(
        ledger.next_pull(&seat(1), &both, 1, 7, now),
        NextPull::ReofferAt(at(REOFFER_AFTER_MS)),
        "the earliest of the targets"
    );
    assert_eq!(
        ledger.next_pull(&seat(1), std::slice::from_ref(&b), 1, 7, now),
        NextPull::ReofferAt(at(10_000 + REOFFER_AFTER_MS))
    );
    assert_eq!(
        ledger.next_pull(&seat(1), &both, 1, 7, at(REOFFER_AFTER_MS)),
        NextPull::Delivers,
        "one due target delivers"
    );
    assert_eq!(
        ledger.next_pull(&seat(1), &[a.clone(), bind(3)], 1, 7, now),
        NextPull::Unknown,
        "a target that never pulled may deliver"
    );
    assert_eq!(
        ledger.next_pull(&seat(1), &[], 1, 7, now),
        NextPull::ReofferAt(at(REOFFER_AFTER_MS)),
        "no targets: the seat's entries"
    );
    assert_eq!(
        ledger.next_pull(&seat(1), &[], 2, 7, now),
        NextPull::Unknown,
        "another generation is ignored"
    );
    assert_eq!(
        ledger.next_pull(&seat(2), &[], 1, 7, now),
        NextPull::Unknown,
        "another seat is ignored"
    );
    assert_eq!(
        ledger.next_pull(&seat(2), &both, 1, 7, now),
        NextPull::Unknown,
        "targets of another seat are ignored"
    );
    // holds_parked: true iff a counted entry is parked or full.
    assert!(!ledger.holds_parked(&seat(1), &both, 1));
    assert!(!ledger.holds_parked(&seat(1), &[], 1));
    let entry = ledger.binding_mut(&seat(1), &b, 1).unwrap();
    for _ in 0..MAX_OFFERS_PER_GENERATION {
        entry.record(&id(2), at(0), false);
    }
    assert!(ledger.holds_parked(&seat(1), &both, 1));
    assert!(ledger.holds_parked(&seat(1), std::slice::from_ref(&b), 1));
    assert!(ledger.holds_parked(&seat(1), &[], 1));
    assert!(
        !ledger.holds_parked(&seat(1), std::slice::from_ref(&a), 1),
        "A holds nothing parked"
    );
    assert!(
        !ledger.holds_parked(&seat(1), &[a, bind(3)], 1),
        "a missing entry counts as false"
    );
    assert!(
        !ledger.holds_parked(&seat(1), &both, 2),
        "another generation"
    );
    // Full counts as parked.
    let entry = ledger.binding_mut(&seat(3), &bind(30), 1).unwrap();
    for n in 0..MAX_OFFERS_PER_BINDING {
        entry.record(&id(n), at(0), false);
    }
    assert!(ledger.holds_parked(&seat(3), &[bind(30)], 1));
}

fn deferred_update(
    entry: &mut BindingOffers,
    consumer: crate::protocol::delivery::PresentationConsumer,
) -> OrdinaryDueSnapshot {
    let mut update = entry.due_snapshot(consumer);
    update.cache.scope = Some(OrdinaryDueScope {
        execution: crate::protocol::ids::ExecutionId::new("execution"),
        ordinary_version: 1,
        passive_generation: 1,
        render_key: [1; 32],
    });
    update.cache.deferred = update.stamps.clone();
    update
}

// Kills: stale misses suppressing a newer V1 offer or crossing consumer/session boundaries.
#[test]
fn due_cache_filters_full_stamps_and_isolates_consumer_and_session() {
    use crate::protocol::delivery::PresentationConsumer::{Hook, Watch};
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(1), &bind(1), 1).unwrap();
    entry.record(&id(1), at(0), false);
    entry.record(&id(2), at(0), false);
    let old = deferred_update(entry, Hook);
    entry.record(&id(1), at(60_000), true);
    entry.apply_due_update(Hook, old.clone());
    assert_eq!(
        entry
            .due_snapshot(Hook)
            .cache
            .deferred
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![id(2)]
    );
    assert!(entry.due_snapshot(Watch).cache.deferred.is_empty());
    entry.remove(&id(2));
    assert!(entry.due_snapshot(Hook).cache.deferred.is_empty());
    entry.apply_due_update(Hook, old.clone());
    assert!(entry.due_snapshot(Hook).cache.deferred.is_empty());
    let current = deferred_update(entry, Watch);
    entry.apply_due_update(Watch, current);
    assert_eq!(entry.due_snapshot(Watch).cache.deferred.len(), 1);
    let entry = ledger.binding_mut(&seat(1), &bind(2), 1).unwrap();
    entry.apply_due_update(Hook, old);
    assert!(entry.due_snapshot(Hook).cache.deferred.is_empty());
    assert!(entry.due_snapshot(Watch).cache.deferred.is_empty());
}

// Kills: generation and same-generation LRU ABA applying an obsolete cache delta.
#[test]
fn due_cache_recreation_and_retention_never_reuse_an_old_incarnation() {
    use crate::protocol::delivery::PresentationConsumer::Hook;
    let mut ledger = OfferedLedger::new();
    let entry = ledger.binding_mut(&seat(0), &bind(0), 1).unwrap();
    entry.record(&id(1), at(0), false);
    let old = deferred_update(entry, Hook);
    entry.apply_due_update(Hook, old.clone());
    entry.retain_pending(&BTreeSet::new());
    assert!(entry.due_snapshot(Hook).cache.deferred.is_empty());
    for n in 1..=MAX_LEDGER_BINDINGS {
        ledger.binding_mut(&seat(n), &bind(n), 1).unwrap();
    }
    let entry = ledger.binding_mut(&seat(0), &bind(0), 1).unwrap();
    entry.record(&id(1), at(0), false);
    entry.apply_due_update(Hook, old.clone());
    assert!(entry.due_snapshot(Hook).cache.deferred.is_empty());
    assert_ne!(entry.due_snapshot(Hook).incarnation, old.incarnation);
    let current = deferred_update(entry, Hook);
    let entry = ledger.binding_mut(&seat(0), &bind(0), 2).unwrap();
    entry.record(&id(1), at(0), false);
    entry.apply_due_update(Hook, current);
    assert!(entry.due_snapshot(Hook).cache.deferred.is_empty());
}
