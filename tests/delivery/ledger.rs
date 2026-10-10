use super::*;

fn id(n: usize) -> MessageId {
    MessageId::new(format!("m{n:05}"))
}
fn seat(n: usize) -> SeatId {
    SeatId::new(format!("s{n}"))
}
fn at(ms: i64) -> UtcMillis {
    UtcMillis(ms)
}

#[test]
fn records_and_looks_up_offers_per_seat_and_generation() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 4, "native-a");
    entry.record(&id(1), at(1_000), false);
    entry.record(&id(2), at(1_500), true);
    let entry = ledger.get(&seat(1)).unwrap();
    assert_eq!(entry.generation, 4);
    assert_eq!(entry.native_session, "native-a");
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
    assert!(ledger.get(&seat(2)).is_none());
    assert_eq!(ledger.seat_mut(&seat(2), 1, "x").offers.len(), 0);
    assert_eq!(ledger.get(&seat(1)).unwrap().offers.len(), 2);
}

#[test]
fn same_generation_keeps_the_entry() {
    let mut ledger = OfferedLedger::new();
    ledger
        .seat_mut(&seat(1), 4, "n")
        .record(&id(1), at(0), false);
    assert_eq!(ledger.seat_mut(&seat(1), 4, "n").offers.len(), 1);
}

#[test]
fn a_rotated_generation_discards_the_seats_offers_and_flag() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 4, "n");
    entry.record(&id(1), at(0), false);
    entry.more_pending = true;
    let entry = ledger.seat_mut(&seat(1), 5, "n");
    assert_eq!(entry.generation, 5);
    assert!(entry.offers.is_empty());
    assert!(!entry.more_pending);
}

#[test]
fn due_rule_is_sixty_seconds_and_fewer_than_three_offers() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 1, "n");
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
    let entry = ledger.seat_mut(&seat(1), 1, "n");
    entry.record(&id(1), at(5), false);
    entry.record(&id(1), at(99), true);
    let offer = entry.offers[&id(1)];
    assert_eq!(
        (offer.first_at, offer.last_at, offer.count),
        (at(5), at(99), 2)
    );
    assert!(offer.truncated);
}

#[test]
fn offers_per_seat_are_bounded_and_the_oldest_goes_first() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 1, "n");
    for n in 0..MAX_OFFERS_PER_SEAT + 1 {
        entry.record(&id(n), at(n as i64), false);
    }
    assert_eq!(entry.offers.len(), MAX_OFFERS_PER_SEAT);
    assert!(
        !entry.offers.contains_key(&id(0)),
        "the oldest offer is evicted"
    );
    assert!(entry.offers.contains_key(&id(1)));
    assert!(entry.offers.contains_key(&id(MAX_OFFERS_PER_SEAT)));
}

#[test]
fn seats_are_bounded_and_the_least_recently_touched_goes_first() {
    let mut ledger = OfferedLedger::new();
    for n in 0..MAX_LEDGER_SEATS {
        ledger
            .seat_mut(&seat(n), 1, "n")
            .record(&id(0), at(0), false);
    }
    assert_eq!(ledger.seat_count(), MAX_LEDGER_SEATS);
    // Touch seat 0 so seat 1 is now the least recently touched.
    ledger.seat_mut(&seat(0), 1, "n");
    ledger.seat_mut(&seat(MAX_LEDGER_SEATS), 1, "n");
    assert_eq!(ledger.seat_count(), MAX_LEDGER_SEATS);
    assert!(ledger.get(&seat(0)).is_some(), "a touched seat survives");
    assert!(
        ledger.get(&seat(1)).is_none(),
        "the stalest seat is evicted"
    );
    assert!(ledger.get(&seat(MAX_LEDGER_SEATS)).is_some());
}

#[test]
fn retain_pending_drops_ids_no_longer_pending_and_remove_forgets_one() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 1, "n");
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
