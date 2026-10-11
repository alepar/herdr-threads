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
    let entry = ledger.seat_mut(&seat(1), 4, "native-a").unwrap();
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
    assert_eq!(ledger.seat_mut(&seat(2), 1, "x").unwrap().offers.len(), 0);
    assert_eq!(ledger.get(&seat(1)).unwrap().offers.len(), 2);
}

#[test]
fn same_generation_keeps_the_entry() {
    let mut ledger = OfferedLedger::new();
    ledger
        .seat_mut(&seat(1), 4, "n")
        .unwrap()
        .record(&id(1), at(0), false);
    assert_eq!(ledger.seat_mut(&seat(1), 4, "n").unwrap().offers.len(), 1);
}

#[test]
fn a_rotated_generation_discards_the_seats_offers_and_flag() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 4, "n").unwrap();
    entry.record(&id(1), at(0), false);
    entry.more_pending = true;
    entry.whole_read = Some(9);
    let entry = ledger.seat_mut(&seat(1), 5, "n").unwrap();
    assert_eq!(entry.generation, 5);
    assert!(entry.offers.is_empty());
    assert!(!entry.more_pending);
    assert_eq!(entry.whole_read, None);
}

#[test]
fn due_rule_is_sixty_seconds_and_fewer_than_three_offers() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
    for n in 0..MAX_OFFERS_PER_SEAT {
        assert!(entry.record(&id(n), at(n as i64), false));
    }
    assert!(entry.is_full());
    assert!(
        !entry.record(&id(MAX_OFFERS_PER_SEAT), at(1_000), false),
        "no slot for a new id"
    );
    assert_eq!(entry.offers.len(), MAX_OFFERS_PER_SEAT);
    assert!(
        entry.offers.contains_key(&id(0)),
        "the oldest offer is kept"
    );
    assert!(!entry.offers.contains_key(&id(MAX_OFFERS_PER_SEAT)));
    assert!(
        entry.record(&id(0), at(REOFFER_AFTER_MS), false),
        "a known id is still recorded"
    );
    assert_eq!(entry.offers[&id(0)].count, 2);
    entry.remove(&id(1));
    assert!(
        entry.record(&id(MAX_OFFERS_PER_SEAT), at(2_000), false),
        "a freed slot takes a new id"
    );
}

#[test]
fn seats_are_bounded_and_the_least_recently_touched_goes_first() {
    let mut ledger = OfferedLedger::new();
    for n in 0..MAX_LEDGER_SEATS {
        ledger
            .seat_mut(&seat(n), 1, "n")
            .unwrap()
            .record(&id(0), at(0), false);
    }
    assert_eq!(ledger.seat_count(), MAX_LEDGER_SEATS);
    // Touch seat 0 so seat 1 is now the least recently touched.
    ledger.seat_mut(&seat(0), 1, "n").unwrap();
    ledger.seat_mut(&seat(MAX_LEDGER_SEATS), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    assert_eq!(ledger.next_pull(&seat(1), 1, 7, at(0)), NextPull::Unknown);
    assert!(!ledger.holds_parked(&seat(1), 1));
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
    entry.read_attention = Some(7);
    for _ in 0..MAX_OFFERS_PER_GENERATION {
        entry.record(&id(1), at(0), false);
    }
    assert_eq!(ledger.next_pull(&seat(1), 2, 7, at(0)), NextPull::Unknown);
    assert!(!ledger.holds_parked(&seat(1), 2));
    assert!(
        ledger.holds_parked(&seat(1), 1),
        "the control: generation 1 does hold one"
    );
}

#[test]
fn next_pull_delivers_until_a_whole_read_matches_the_attention_version() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 2, "n").unwrap();
    assert!(
        !entry.notices_pending,
        "a rotated generation starts without the flag"
    );
}

#[test]
fn next_pull_is_nothing_once_every_offer_is_parked() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
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
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
    entry.read_attention = Some(7);
    for n in 0..MAX_OFFERS_PER_SEAT {
        entry.record(&id(n), at(0), false);
    }
    assert_eq!(
        entry.next_pull(7, at(1)),
        NextPull::ReofferAt(at(REOFFER_AFTER_MS))
    );
    assert!(entry.holds_parked(), "new ids wait for inbox");
    assert!(ledger.holds_parked(&seat(1), 1));
    let entry = ledger.seat_mut(&seat(1), 1, "n").unwrap();
    entry.remove(&id(0));
    assert!(
        !entry.holds_parked(),
        "the control: one below the bound, no id at three showings"
    );
}

// Kills: an older-generation request wiping the newer generation's offers (main review, ledger.rs seat_mut).
#[test]
fn an_older_generation_never_replaces_a_newer_entry() {
    let mut ledger = OfferedLedger::new();
    let entry = ledger.seat_mut(&seat(1), 5, "new").unwrap();
    entry.record(&id(1), at(0), false);
    entry.read_attention = Some(7);
    assert!(
        ledger.seat_mut(&seat(1), 4, "old").is_none(),
        "a stale generation gets no entry"
    );
    let entry = ledger.get(&seat(1)).unwrap();
    assert_eq!(
        (entry.generation, entry.native_session.as_str()),
        (5, "new")
    );
    assert_eq!(entry.offers.len(), 1);
    assert_eq!(entry.read_attention, Some(7));
    let entry = ledger.seat_mut(&seat(1), 6, "newer").unwrap();
    assert!(
        entry.offers.is_empty(),
        "a newer generation still replaces the entry"
    );
}

#[test]
fn next_pull_never_touches_the_lru_order() {
    let mut ledger = OfferedLedger::new();
    for n in 0..MAX_LEDGER_SEATS {
        ledger.seat_mut(&seat(n), 1, "n").unwrap();
    }
    // Asking about seat 0 (the oldest) must not refresh it.
    ledger.next_pull(&seat(0), 1, 7, at(0));
    ledger.holds_parked(&seat(0), 1);
    ledger.seat_mut(&seat(MAX_LEDGER_SEATS), 1, "n").unwrap();
    assert!(
        ledger.get(&seat(0)).is_none(),
        "seat 0 is still the stalest"
    );
    assert!(ledger.get(&seat(1)).is_some());
}
