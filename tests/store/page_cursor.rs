use super::page_fit::{FitStop, PageFit};
use super::{
    LegacyWakePosition, PageKind, RecoveryPageCursor, WakePageCursor, WorkPageCursor,
    longest_cursor_bytes,
};
use crate::protocol::{
    ids::ThreadId,
    output::OutputSpec,
    pagination::{
        Consistency, Page, ReceiptAttentionCursorState, SeatAttentionCursorState, StopReason,
    },
    results::{CommandResult, ErrorCode, InboxItem},
};

const INSTANCE: &str = "00000000-0000-4000-8000-000000000000";

fn legacy_wake_cursor() -> WakePageCursor {
    WakePageCursor {
        after_ordinal: 1,
        high_water_ordinal: 2,
        legacy: Some(LegacyWakePosition {
            last_examined_key: "seat_1".into(),
            scope_revision: 4,
            attention: SeatAttentionCursorState {
                invitation_after_seq: 3,
                invitation_after_ordinal: 5,
                invitations_done: true,
                has_pending_invitation: true,
                invitation_frontier: Some((6, 7)),
                receipts: Some(ReceiptAttentionCursorState {
                    physical_after: 1,
                    manifest_after: 2,
                    physical_high_water: 3,
                    manifest_high_water: 4,
                    next_manifest: true,
                }),
                receipts_done: true,
                has_pending_receipt: true,
                receipt_frontier_seq: Some(8),
                physical_warning_after: 1,
                physical_warning_high_water: 2,
                manifest_warning_after: 3,
                manifest_warning_high_water: 4,
                next_manifest_warning: true,
                latest_warning_seq: Some(9),
                latest_warning_offset: Some(10),
            },
        }),
    }
}

#[test]
fn work_and_recovery_cursors_round_trip() {
    let work = WorkPageCursor {
        after_ordinal: 7,
        high_water_ordinal: 90,
    };
    assert_eq!(
        WorkPageCursor::decode(&work.encode(INSTANCE).unwrap(), INSTANCE).unwrap(),
        work
    );
    let rec = RecoveryPageCursor {
        after_seat_ordinal: 3,
        high_water_ordinal: 11,
    };
    assert_eq!(
        RecoveryPageCursor::decode(&rec.encode(INSTANCE).unwrap(), INSTANCE).unwrap(),
        rec
    );
}

#[test]
fn cursors_are_not_interchangeable_across_page_kinds() {
    let work = WorkPageCursor {
        after_ordinal: 1,
        high_water_ordinal: 2,
    }
    .encode(INSTANCE)
    .unwrap();
    assert_eq!(
        RecoveryPageCursor::decode(&work, INSTANCE)
            .unwrap_err()
            .code,
        ErrorCode::InvalidCursor
    );
    let legacy = legacy_wake_cursor().encode(INSTANCE).unwrap();
    assert_eq!(
        WorkPageCursor::decode(&legacy, INSTANCE).unwrap_err().code,
        ErrorCode::InvalidCursor
    );
}

#[test]
fn post_d2_wake_cursor_round_trips_without_legacy_fields() {
    let wake = WakePageCursor {
        after_ordinal: 5,
        high_water_ordinal: 9,
        legacy: None,
    };
    let raw = wake.encode(INSTANCE).unwrap();
    assert_eq!(
        WakePageCursor::decode_with(&raw, INSTANCE, true).unwrap(),
        wake
    );
}

#[test]
fn legacy_wake_fields_are_tolerated_only_by_the_explicit_off_decoder() {
    let legacy = legacy_wake_cursor();
    let raw = legacy.encode(INSTANCE).unwrap();
    assert_eq!(
        WakePageCursor::decode_with(&raw, INSTANCE, false).unwrap(),
        legacy
    );
    let switch = std::hint::black_box(super::WAKE_CURSOR_REJECTS_LEGACY_FIELDS);
    assert!(
        switch,
        "ht-p03.12.5 stops emitting the fields and switches this on"
    );
    assert_eq!(
        WakePageCursor::decode(&raw, INSTANCE).unwrap_err().code,
        ErrorCode::InvalidCursor
    );
}

#[test]
fn legacy_wake_fields_are_invalid_cursor_with_the_switch_on() {
    let raw = legacy_wake_cursor().encode(INSTANCE).unwrap();
    let error = WakePageCursor::decode_with(&raw, INSTANCE, true).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCursor);
}

struct Xorshift(u64);
impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// A value whose bit width is itself random, so every varint length occurs.
    fn wide(&mut self) -> u64 {
        let shift = self.next() % 64;
        self.next() >> shift
    }
    fn ordinals(&mut self) -> (u64, u64) {
        let (a, b) = (self.wide(), self.wide());
        (a.min(b), a.max(b))
    }
}

#[test]
fn longest_cursor_bytes_bounds_every_encoded_cursor() {
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    let mut widest = [0usize; 3];
    let mut record = |kind: PageKind, raw: &str| {
        let slot = kind as usize;
        widest[slot] = widest[slot].max(raw.len());
        assert!(
            raw.len() <= longest_cursor_bytes(kind),
            "{kind:?} cursor of {} bytes exceeds bound {}",
            raw.len(),
            longest_cursor_bytes(kind)
        );
    };
    let mut ordinal_pairs = vec![
        (0, 0),
        (0, u64::MAX),
        (u64::MAX, u64::MAX),
        (1, u64::MAX),
        (1 << 63, u64::MAX),
    ];
    ordinal_pairs.extend((0..10_000).map(|_| rng.ordinals()));
    for (after, high) in ordinal_pairs {
        let work = WorkPageCursor {
            after_ordinal: after,
            high_water_ordinal: high,
        };
        record(PageKind::Work, &work.encode(INSTANCE).unwrap());
        let recovery = RecoveryPageCursor {
            after_seat_ordinal: after,
            high_water_ordinal: high,
        };
        record(PageKind::Recovery, &recovery.encode(INSTANCE).unwrap());
        let wake = WakePageCursor {
            after_ordinal: after,
            high_water_ordinal: high,
            legacy: None,
        };
        record(PageKind::Wake, &wake.encode(INSTANCE).unwrap());
    }
    // The ordinal-only bound is tight (the widest ordinal cursor reaches it), and
    // the wake bound is the same ordinal-only bound once legacy fields are rejected.
    assert_eq!(
        widest[PageKind::Work as usize],
        longest_cursor_bytes(PageKind::Work)
    );
    assert_eq!(
        widest[PageKind::Recovery as usize],
        longest_cursor_bytes(PageKind::Recovery)
    );
    assert_eq!(
        widest[PageKind::Wake as usize],
        longest_cursor_bytes(PageKind::Wake)
    );
    assert_eq!(
        longest_cursor_bytes(PageKind::Wake),
        longest_cursor_bytes(PageKind::Work)
    );
}

fn inbox_item(thread: &str) -> InboxItem {
    InboxItem {
        thread: ThreadId::new(thread),
        invitations: 1,
        invitations_has_more: false,
        pending_receipts: 2,
        pending_receipts_has_more: false,
        warnings: 0,
        warnings_has_more: false,
        pending_requirement: None,
    }
}

fn inbox_page(items: Vec<InboxItem>) -> CommandResult {
    CommandResult::Inbox(Page {
        items,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    })
}

#[test]
fn page_fit_default_is_the_greedy_prefix() {
    let output = OutputSpec::default();
    let items: Vec<InboxItem> = (0..8)
        .map(|index| inbox_item(&format!("thread-{}", "x".repeat(10 + index * 7))))
        .collect();
    let render = |count: usize| Ok(inbox_page(items[..count].to_vec()));
    let len_of = |count: usize| {
        crate::protocol::output::encode_selected(&render(count).unwrap(), &output)
            .unwrap()
            .len()
    };
    let manual = |max: usize| {
        (0..=items.len())
            .take_while(|count| len_of(*count) <= max)
            .last()
    };

    // Every budget from "nothing fits" to "everything fits" agrees with the
    // byte-by-byte computation.
    for max in (len_of(0).saturating_sub(1))..=len_of(items.len()) + 1 {
        let mut fit = PageFit::new(&output, max, PageKind::Work);
        let got = fit.fit(&items, render).unwrap();
        match manual(max) {
            None => {
                // not even the empty page fits
                assert_eq!(got.accepted, 0);
                assert_eq!(got.stop, FitStop::Bytes);
            }
            Some(expected) => {
                assert_eq!(got.accepted, expected, "max {max}");
                let stop = if expected == items.len() {
                    FitStop::Rows
                } else {
                    FitStop::Bytes
                };
                assert_eq!(got.stop, stop, "max {max}");
                assert_eq!(
                    got.required_minimum,
                    (expected == 0).then(|| len_of(1) as u32),
                    "max {max}"
                );
            }
        }
    }
    // Exactly three rows fit.
    let mut fit = PageFit::new(&output, len_of(3), PageKind::Work);
    let got = fit.fit(&items, render).unwrap();
    assert_eq!((got.accepted, got.stop), (3, FitStop::Bytes));
    assert_eq!(got.required_minimum, None);

    for kind in [PageKind::Work, PageKind::Wake, PageKind::Recovery] {
        assert_eq!(
            PageFit::new(&output, 0, kind).cursor_base_bytes(),
            longest_cursor_bytes(kind)
        );
    }
}
