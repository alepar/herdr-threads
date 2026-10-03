//! Seam: wake, work and recovery pages cut by the PageFit tails round-trip their
//! post-D2 continuation cursors across pages, match the greedy oracle at every
//! page boundary, and a pre-rewrite wake cursor is rejected (ht-p03.12.11).
use super::*;
use crate::ports::{WakeCandidate, WakeRecoveryCandidate, WorkCandidate};
use crate::protocol::{
    pagination::{
        MAX_PAGE_BYTES, Page, PageRequest, ReceiptAttentionCursorState, SeatAttentionCursorState,
        StopReason,
    },
    results::ErrorCode,
    time::{Cancellation, MonoInstant, UtcMillis},
};
use std::sync::Arc;

const INSTANCE: &str = "i";
const OLD_BOOT: &str = "00000000-0000-0000-0000-000000000001";
const CURRENT_BOOT: &str = "00000000-0000-0000-0000-000000000002";

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1_000)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000_000),
        cancellation: Cancellation::default(),
    }
}

struct Fixture {
    path: std::path::PathBuf,
    context: StoreContext,
}

impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("herdr-page-seam-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let db = context.open_writer().unwrap();
        db.execute_batch(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1000000);\
             INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);",
        )
        .unwrap();
        Self { path, context }
    }
    fn writer(&self) -> Connection {
        self.context.open_writer().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

fn seed_seats(db: &Connection, count: u64) {
    db.execute_batch(&format!(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{count}) \
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) \
         SELECT 'seat-'||x,'i','resolved','native','pane-'||x,1,1,0 FROM n"
    ))
    .unwrap();
}

fn ordinals(db: &Connection, table: &str, id_column: &str) -> Vec<(String, u64)> {
    let mut statement = db
        .prepare(&format!(
            "SELECT {id_column},ordinal FROM {table} ORDER BY ordinal"
        ))
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

type Run<T> = Box<dyn Fn(Option<String>, u16, u32) -> Result<Page<T>, ApiError>>;
type PageAt<T> = fn(Vec<T>, u64, u64, StopReason) -> Result<Page<T>, ApiError>;

/// One page kind under test: how to run the production page function, how the
/// page builder shapes a page (the oracle's page), how to name an item and read
/// a cursor back.
struct Subject<T> {
    name: &'static str,
    kind: PageKind,
    /// The ordinal of every item the walk can return, by item id.
    ordinals: Vec<(String, u64)>,
    /// The ordinal of every row the walk passes, candidate or not: a cut
    /// cursor sits after the last row passed before the first dropped item.
    visited: Vec<u64>,
    run: Run<T>,
    page_at: PageAt<T>,
    id: fn(&T) -> String,
    /// `(after, high_water)` of an emitted cursor, decoded post-D2.
    decode: fn(&str) -> Result<(u64, u64), ApiError>,
}

fn json_len<T: serde::Serialize>(page: &Page<T>) -> usize {
    serde_json::to_vec(page).unwrap().len()
}

fn ordinal_of<T>(subject: &Subject<T>, item: &T) -> u64 {
    let id = (subject.id)(item);
    subject
        .ordinals
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .unwrap_or_else(|| panic!("{}: unknown item {id}", subject.name))
        .1
}

/// The pre-D5 greedy boundary, over the unpaged remainder `all`: the largest
/// prefix whose fixed JSON page fits `max`, the page after it cut at the last
/// row passed before the first dropped item (one item when even the first does
/// not fit).
fn oracle<T: serde::Serialize + Clone>(
    subject: &Subject<T>,
    all: &[T],
    after: u64,
    high_water: u64,
    max: usize,
) -> (usize, Page<T>) {
    let before = |k: usize| {
        subject
            .visited
            .iter()
            .copied()
            .filter(|ordinal| *ordinal < ordinal_of(subject, &all[k]) && *ordinal > after)
            .max()
            .unwrap_or(after)
    };
    let page_of = |k: usize| {
        if k == all.len() {
            (subject.page_at)(all.to_vec(), high_water, high_water, StopReason::Complete)
        } else {
            (subject.page_at)(all[..k].to_vec(), before(k), high_water, StopReason::Bytes)
        }
        .unwrap()
    };
    let mut accepted = 0;
    for k in 0..=all.len() {
        if json_len(&page_of(k)) > max {
            break;
        }
        accepted = k;
    }
    let accepted = accepted.max(1);
    (accepted, page_of(accepted))
}

fn walk_and_check<T: serde::Serialize + Clone + PartialEq + std::fmt::Debug>(
    subject: &Subject<T>,
    min_pages: usize,
) {
    let name = subject.name;
    let unpaged = (subject.run)(None, 100, MAX_PAGE_BYTES).unwrap();
    assert!(
        unpaged.next_cursor.is_none(),
        "{name}: the unpaged walk must finish in one page"
    );
    let want: Vec<String> = unpaged
        .items
        .iter()
        .map(|item| (subject.id)(item))
        .collect();
    assert_eq!(
        want,
        subject
            .ordinals
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        "{name}: the unpaged walk visits every seeded item in ordinal order"
    );
    let single = json_len(&(subject.run)(None, 1, MAX_PAGE_BYTES).unwrap());
    let marginal = json_len(&(subject.run)(None, 2, MAX_PAGE_BYTES).unwrap()) - single;
    let cursor_cap = longest_cursor_bytes(subject.kind);

    for max in [
        single + marginal * 2 + marginal / 2,
        single + marginal * 5 + marginal / 2,
    ] {
        let max_bytes = u32::try_from(max).unwrap();
        let mut cursor: Option<String> = None;
        let mut seen: Vec<String> = Vec::new();
        let mut pages = 0usize;
        loop {
            pages += 1;
            assert!(pages <= want.len() + 1, "{name}: cursor does not advance");
            let page = (subject.run)(cursor.clone(), 100, max_bytes).unwrap();
            // The oracle runs from the same continuation over the unpaged remainder.
            let remainder = (subject.run)(cursor.clone(), 100, MAX_PAGE_BYTES).unwrap();
            assert!(
                remainder.next_cursor.is_none(),
                "{name}: remainder in one page"
            );
            let after = cursor
                .as_deref()
                .map_or(0, |raw| (subject.decode)(raw).unwrap().0);
            let (accepted, expected) = oracle(
                subject,
                &remainder.items,
                after,
                unpaged.high_water_ordinal,
                max,
            );
            assert_eq!(
                page.items,
                remainder.items[..accepted],
                "{name} max={max} page {pages}: boundary differs from the greedy oracle"
            );
            assert_eq!(
                page.next_cursor, expected.next_cursor,
                "{name} page {pages}"
            );
            assert_eq!(
                page.stop_reason, expected.stop_reason,
                "{name} page {pages}"
            );
            assert_eq!(json_len(&page), json_len(&expected), "{name} page {pages}");
            assert!(json_len(&page) <= max, "{name} page {pages} over budget");
            seen.extend(page.items.iter().map(|item| (subject.id)(item)));
            let Some(next) = page.next_cursor.clone() else {
                break;
            };
            assert!(
                next.len() <= cursor_cap,
                "{name}: cursor of {} bytes exceeds the {cursor_cap}-byte bound",
                next.len()
            );
            let (next_after, next_high) = (subject.decode)(&next).unwrap();
            let last = ordinal_of(
                subject,
                page.items.last().expect("a cut page keeps an item"),
            );
            let first_dropped = ordinal_of(subject, &remainder.items[page.items.len()]);
            assert!(
                (last..first_dropped).contains(&next_after),
                "{name}: cursor {next_after} must resume between {last} and {first_dropped}"
            );
            assert_eq!(
                next_high, unpaged.high_water_ordinal,
                "{name}: high water is frozen"
            );
            cursor = Some(next);
        }
        assert!(
            pages >= min_pages,
            "{name} max={max}: only {pages} pages, the byte budget must cut the walk"
        );
        assert_eq!(
            seen, want,
            "{name} max={max}: pages union equals the unpaged walk"
        );
    }
}

fn page_request(cursor: Option<String>, limit: u16, max_bytes: u32) -> PageRequest {
    PageRequest {
        cursor,
        limit,
        max_bytes,
    }
}

#[test]
fn wake_pages_round_trip_cursors_and_match_the_oracle() {
    let fixture = Fixture::new();
    let db = fixture.writer();
    seed_seats(&db, 48);
    db.execute_batch(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) \
         SELECT 'inv-'||id,'t',id,1,'pending',0,1,100,100 FROM seats WHERE ordinal%3!=0",
    )
    .unwrap();
    let all_seats = ordinals(&db, "seats", "id");
    let visited: Vec<u64> = all_seats.iter().map(|(_, ordinal)| *ordinal).collect();
    // Seats without pending rows are passed but never returned.
    let ordinals: Vec<(String, u64)> = all_seats
        .into_iter()
        .filter(|(_, ordinal)| ordinal % 3 != 0)
        .collect();
    drop(db);
    let subject = Subject {
        name: "wake",
        kind: PageKind::Wake,
        ordinals,
        visited,
        run: Box::new(move |cursor, limit, max| {
            wake_candidates_page(
                &fixture.context,
                INSTANCE,
                page_request(cursor, limit, max),
                &budget(),
            )
        }),
        page_at: |items: Vec<WakeCandidate>, after, high, stop| {
            wake_page_at(INSTANCE, items, after, high, stop)
        },
        id: |item| item.seat.as_str().to_owned(),
        decode: |raw| {
            let cursor = WakePageCursor::decode(raw, INSTANCE)?;
            assert!(
                cursor.legacy.is_none(),
                "post-D2 cursors carry no legacy fields"
            );
            Ok((cursor.after_ordinal, cursor.high_water_ordinal))
        },
    };
    walk_and_check(&subject, 4);
}

#[test]
fn work_pages_round_trip_cursors_and_match_the_oracle() {
    let fixture = Fixture::new();
    let db = fixture.writer();
    // Completed jobs between the live ones: the cut cursor still names the last
    // returned job, and the walk skips the dead rows.
    db.execute_batch(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<60) \
         INSERT INTO work_jobs(id,kind,subject_id,position,high_water,status) \
         SELECT 'job-'||x,'warning_attribution','subject-'||x,0,4,IIF(x%5=0,'complete',IIF(x%2=0,'failed','pending')) FROM n",
    )
    .unwrap();
    let live: Vec<(String, u64)> = ordinals(&db, "work_jobs", "id")
        .into_iter()
        .filter(|(id, _)| {
            let n: u32 = id.trim_start_matches("job-").parse().unwrap();
            !n.is_multiple_of(5)
        })
        .collect();
    drop(db);
    let subject = Subject {
        name: "work",
        kind: PageKind::Work,
        visited: live.iter().map(|(_, ordinal)| *ordinal).collect(),
        ordinals: live,
        run: Box::new(move |cursor, limit, max| {
            pending_work_page(
                &fixture.context,
                INSTANCE,
                page_request(cursor, limit, max),
                &budget(),
            )
        }),
        page_at: |items: Vec<WorkCandidate>, after, high, stop| {
            work_page_at(INSTANCE, items, after, high, stop)
        },
        id: |item| item.id.clone(),
        decode: |raw| {
            let cursor = WorkPageCursor::decode(raw, INSTANCE)?;
            Ok((cursor.after_ordinal, cursor.high_water_ordinal))
        },
    };
    walk_and_check(&subject, 4);
}

#[test]
fn recovery_pages_round_trip_cursors_and_match_the_oracle() {
    let fixture = Fixture::new();
    let db = fixture.writer();
    seed_seats(&db, 72);
    // Every seat with a prior-boot reservation is a candidate; every third seat
    // holds the current boot's reservation and is skipped, and the rest hold
    // none, so the walk's cut cursors sit between reserved seats.
    db.execute_batch(&format!(
        "INSERT INTO wake_work(seat_id,reservation_id,reservation_boot) \
         SELECT id,'attempt-'||ordinal,IIF(ordinal%3=0,'{CURRENT_BOOT}','{OLD_BOOT}') FROM seats WHERE ordinal%4!=0"
    ))
    .unwrap();
    let candidates: Vec<(String, u64)> = {
        let mut statement = db
            .prepare(&format!(
                "SELECT s.id,s.ordinal FROM seats s JOIN wake_work w ON w.seat_id=s.id \
                 WHERE w.reservation_boot='{OLD_BOOT}' ORDER BY s.ordinal"
            ))
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as u64)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let visited: Vec<u64> = {
        let mut statement = db
            .prepare("SELECT s.ordinal FROM seats s JOIN wake_work w ON w.seat_id=s.id WHERE w.reservation_id IS NOT NULL ORDER BY s.ordinal")
            .unwrap();
        statement
            .query_map([], |row| Ok(row.get::<_, i64>(0)? as u64))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    drop(db);
    let elected = uuid::Uuid::parse_str(CURRENT_BOOT).unwrap();
    let subject = Subject {
        name: "recovery",
        kind: PageKind::Recovery,
        ordinals: candidates,
        visited,
        run: Box::new(move |cursor, limit, max| {
            wake_recovery_page(
                &fixture.context,
                INSTANCE,
                Some(elected),
                page_request(cursor, limit, max),
                &budget(),
            )
        }),
        page_at: |items: Vec<WakeRecoveryCandidate>, after, high, stop| {
            wake_recovery_page_at(INSTANCE, items, after, high, stop)
        },
        id: |item| item.seat.as_str().to_owned(),
        decode: |raw| {
            let cursor = RecoveryPageCursor::decode(raw, INSTANCE)?;
            Ok((cursor.after_seat_ordinal, cursor.high_water_ordinal))
        },
    };
    walk_and_check(&subject, 4);
}

/// A continuation as the pre-rewrite wake discovery emitted it: mid-seat position
/// with `last_examined_key`, `scope_revision` and `attention`.
fn pre_rewrite_wake_cursor() -> String {
    WakePageCursor {
        after_ordinal: 1,
        high_water_ordinal: 2,
        legacy: Some(LegacyWakePosition {
            last_examined_key: "seat-1".into(),
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
    .encode(INSTANCE)
    .unwrap()
}

#[test]
fn pre_rewrite_wake_cursor_is_rejected() {
    let fixture = Fixture::new();
    seed_seats(&fixture.writer(), 3);
    let raw = pre_rewrite_wake_cursor();
    let error = wake_candidates_page(
        &fixture.context,
        INSTANCE,
        page_request(Some(raw.clone()), 100, 16_384),
        &budget(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidCursor);
    assert_eq!(
        WakePageCursor::decode(&raw, INSTANCE).unwrap_err().code,
        ErrorCode::InvalidCursor
    );
    // The same position without the removed fields is a valid continuation.
    let current = WakePageCursor {
        after_ordinal: 1,
        high_water_ordinal: 3,
        legacy: None,
    }
    .encode(INSTANCE)
    .unwrap();
    let page = wake_candidates_page(
        &fixture.context,
        INSTANCE,
        page_request(Some(current), 100, 16_384),
        &budget(),
    )
    .unwrap();
    assert!(page.next_cursor.is_none());
}
