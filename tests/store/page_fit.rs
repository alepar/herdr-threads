use super::*;
use crate::{
    ports::{WakeRecoveryCandidate, WorkCandidate, WorkKind},
    protocol::{
        ids::{SeatId, ThreadId, WakeAttemptId},
        output::{ContinuationContext, OutputFormat, encode_selected},
        pagination::{Consistency, Page, StopReason},
        results::{
            ApiError, CommandResult, ContinuityStatus, InboxItem, SearchHit, SearchPage,
            SeatSummary, ThreadSummary,
        },
        time::UtcMillis,
    },
    store::{wake_recovery_page_at, work_page_at},
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn spec(format: OutputFormat) -> OutputSpec {
    OutputSpec {
        format,
        context: ContinuationContext::default(),
    }
}

/// A page whose cursor length never shrinks with the item index, as real
/// ordinal cursors do, so page length is monotone in the item count.
fn page<T>(items: Vec<T>, cursor: usize) -> Page<T> {
    let raw = format!("cur-{}", "x".repeat(20 + cursor / 50));
    Page {
        items,
        next_cursor: Some(raw.clone()),
        next_argv: Some(vec![
            "herdr-threads".into(),
            "page".into(),
            "--cursor".into(),
            raw,
        ]),
        high_water_ordinal: 7,
        scope_revision: None,
        has_more: true,
        stop_reason: StopReason::Rows,
        consistency: Consistency::BoundedLive,
    }
}

fn topic(rng: &mut Rng) -> String {
    // Quotes, newlines, backslashes and multi-byte characters change the
    // escaped (capped) form of a topic in both output formats.
    const ALPHABET: [char; 8] = ['a', 'b', ' ', '"', '\n', 'é', '\\', 'Z'];
    let len = match rng.below(4) {
        0 => rng.below(4),
        1 => rng.below(40),
        2 => rng.below(120),
        _ => rng.below(400),
    };
    (0..len)
        .map(|_| ALPHABET[rng.below(ALPHABET.len())])
        .collect()
}

fn thread(i: usize, rng: &mut Rng) -> ThreadSummary {
    ThreadSummary {
        last_activity: None,
        name: None,
        thread: ThreadId::new(format!("thread-{i}")),
        managed_owner: None,
        topic_data: topic(rng),
        topic_omitted: false,
        topic_detail_argv: None,
        archived: false,
        orphaned: false,
        message_count: i as u64,
        created_at: UtcMillis(1_000 + i as i64),
        ordinary_count: i as u64,
        system_count: 0,
        joined_count: 1,
    }
}

fn seat(i: usize, rng: &mut Rng) -> SeatSummary {
    SeatSummary {
        seat: SeatId::new(format!("seat-{}", "s".repeat(rng.below(60)))),
        continuity: ContinuityStatus::Resolved,
        target: None,
        generation: i as u64,
        created_at: UtcMillis(1_000 + i as i64),
        retired_at: None,
    }
}

fn inbox(i: usize, rng: &mut Rng) -> InboxItem {
    InboxItem {
        thread: ThreadId::new(format!("thread-{}-{i}", "t".repeat(rng.below(80)))),
        invitations: rng.below(5) as u64,
        invitations_has_more: rng.below(2) == 0,
        pending_receipts: rng.below(5000) as u64,
        pending_receipts_has_more: false,
        warnings: 0,
        warnings_has_more: false,
        pending_requirement: None,
    }
}

type Render<'a> = Box<dyn Fn(usize) -> Result<CommandResult, ApiError> + 'a>;

/// One converted site's page shape: the first `k` items with the continuation
/// cursor of item `k - 1` (item 0's for `k = 0`).
struct Shape<'a> {
    name: &'static str,
    len: usize,
    render: Render<'a>,
    render_one: Render<'a>,
}

fn shape<'a, T: Clone + 'a>(
    name: &'static str,
    items: &'a [T],
    wrap: impl Fn(Page<T>) -> CommandResult + Copy + 'a,
) -> Shape<'a> {
    Shape {
        name,
        len: items.len(),
        render: Box::new(move |k| Ok(wrap(page(items[..k].to_vec(), k.max(1) - 1)))),
        render_one: Box::new(move |i| Ok(wrap(page(vec![items[i].clone()], i)))),
    }
}

fn encoded(result: &CommandResult, output: &OutputSpec) -> Vec<u8> {
    encode_selected(result, output).unwrap()
}

/// The encoded length of every growing prefix (0..=len) of a shape's page.
/// It does not depend on the budget, so each shape encodes its prefixes once
/// and the oracle reads them for every budget.
fn prefix_lengths(shape: &Shape<'_>, output: &OutputSpec) -> Vec<usize> {
    (0..=shape.len)
        .map(|count| encoded(&(shape.render)(count).unwrap(), output).len())
        .collect()
}

/// The pre-D5 loop, kept as the oracle: the largest count whose page fits,
/// walking every growing prefix (`prefix_lengths`) up to the first that does not.
fn greedy_oracle(
    shape: &Shape<'_>,
    output: &OutputSpec,
    prefixes: &[usize],
    max: usize,
) -> (usize, Vec<u8>) {
    let mut accepted = 0;
    for (count, &len) in prefixes.iter().enumerate() {
        if len > max {
            break;
        }
        accepted = count;
    }
    let bytes = encoded(&(shape.render)(accepted).unwrap(), output);
    (accepted, bytes)
}

fn shapes<'a>(
    threads: &'a [ThreadSummary],
    seats: &'a [SeatSummary],
    inboxes: &'a [InboxItem],
    hits: &'a [SearchHit],
) -> Vec<Shape<'a>> {
    vec![
        shape("directory", threads, CommandResult::Directory),
        shape("seats", seats, CommandResult::Seats),
        shape("inbox", inboxes, CommandResult::Inbox),
        shape("search", hits, |matches| {
            CommandResult::Search(SearchPage {
                matches,
                examined_candidates: 3,
                examined_utf8_bytes: 99,
            })
        }),
    ]
}

fn fixtures(
    rng: &mut Rng,
    n: usize,
) -> (
    Vec<ThreadSummary>,
    Vec<SeatSummary>,
    Vec<InboxItem>,
    Vec<SearchHit>,
) {
    let threads: Vec<_> = (0..n).map(|i| thread(i, rng)).collect();
    let seats: Vec<_> = (0..n).map(|i| seat(i, rng)).collect();
    let inboxes: Vec<_> = (0..n).map(|i| inbox(i, rng)).collect();
    let hits = threads.iter().cloned().map(SearchHit::Topic).collect();
    (threads, seats, inboxes, hits)
}

#[test]
fn differential_matches_greedy_for_every_site() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for format in [OutputFormat::Json, OutputFormat::Text] {
        let output = spec(format);
        for trial in 0..5 {
            let n = if trial == 0 { 300 } else { rng.below(150) };
            let (threads, seats, inboxes, hits) = fixtures(&mut rng, n);
            for shape in shapes(&threads, &seats, &inboxes, &hits) {
                let prefixes = prefix_lengths(&shape, &output);
                let full = encoded(&(shape.render)(shape.len).unwrap(), &output).len();
                let single = if shape.len > 0 {
                    encoded(&(shape.render)(1).unwrap(), &output).len()
                } else {
                    0
                };
                // Budgets across the whole range: below one item, around one
                // item, anywhere inside the page, and past the whole page.
                let maxes = [
                    0,
                    single.saturating_sub(1),
                    single,
                    rng.below(full + 1),
                    rng.below(full + 1),
                    full.saturating_sub(1),
                    full,
                    full + rng.below(200),
                ];
                for max in maxes {
                    let (want, want_bytes) = greedy_oracle(&shape, &output, &prefixes, max);
                    let mut fit = PageFit::for_command(&output, max);
                    let exact = fit
                        .fit_with(&vec![(); shape.len], &shape.render, &shape.render_one)
                        .unwrap();
                    assert_eq!(
                        exact.accepted, want,
                        "{} {format:?} n={n} max={max}",
                        shape.name
                    );
                    assert_eq!(
                        encoded(&(shape.render)(exact.accepted).unwrap(), &output),
                        want_bytes,
                        "{} {format:?} n={n} max={max}",
                        shape.name
                    );
                    let default = fit.fit(&vec![(); shape.len], &shape.render).unwrap();
                    assert_eq!(
                        default.accepted, want,
                        "default fit {} {format:?} n={n} max={max}",
                        shape.name
                    );
                    assert_eq!(
                        exact.stop,
                        if want == shape.len {
                            FitStop::Rows
                        } else {
                            FitStop::Bytes
                        }
                    );
                }
            }
        }
    }
}

fn counted<R>(work: impl FnOnce() -> R) -> (R, usize, usize) {
    ENCODES.with(|c| c.set(0));
    ENCODED_BYTES.with(|c| c.set(0));
    let result = work();
    (
        result,
        ENCODES.with(|c| c.get()),
        ENCODED_BYTES.with(|c| c.get()),
    )
}

fn ceil_log2(n: usize) -> usize {
    (usize::BITS - n.saturating_sub(1).leading_zeros()) as usize
}

#[test]
fn encode_calls_are_bounded() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let output = spec(OutputFormat::Json);
    for n in [1usize, 2, 3, 7, 10, 50, 100, 300, 1000] {
        let (threads, seats, inboxes, hits) = fixtures(&mut rng, n);
        for shape in shapes(&threads, &seats, &inboxes, &hits) {
            let full = encoded(&(shape.render)(n).unwrap(), &output).len();
            for max in [full / 4, full / 2, full * 9 / 10, full, full + 50] {
                let (fit, calls, _) = counted(|| {
                    PageFit::for_command(&output, max)
                        .fit_with(&vec![(); n], &shape.render, &shape.render_one)
                        .unwrap()
                });
                let bound = n + 2 * ceil_log2(n) + 2;
                assert!(
                    calls <= bound,
                    "{} n={n} max={max}: {calls} encodes for accepted {} (bound {bound})",
                    shape.name,
                    fit.accepted
                );
            }
        }
    }
}

#[test]
fn encoded_bytes_are_linear() {
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    let output = spec(OutputFormat::Json);
    let c = 4;
    let mut totals = Vec::new();
    for n in [100usize, 1000] {
        let threads: Vec<_> = (0..n)
            .map(|i| {
                let mut t = thread(i, &mut rng);
                t.topic_data = "topic words ".repeat(1 + rng.below(8));
                t
            })
            .collect();
        let shape = shape("directory", &threads, CommandResult::Directory);
        // Half of the items fit: the page is as large as the budget.
        let full = encoded(&(shape.render)(n).unwrap(), &output).len();
        let max = full / 2;
        let base = encoded(&(shape.render)(0).unwrap(), &output).len();
        let (fit, _, bytes) = counted(|| {
            PageFit::for_command(&output, max)
                .fit_with(&vec![(); n], &shape.render, &shape.render_one)
                .unwrap()
        });
        assert!(fit.accepted > 0 && fit.accepted < n);
        let limit = c * (n * base + max * (1 + ceil_log2(n)));
        assert!(
            bytes <= limit,
            "n={n}: {bytes} bytes encoded exceeds {limit} (base {base}, page {max})"
        );
        totals.push(bytes);
    }
    // A 10x larger page costs about 10-25x (quadratic fitting would cost ~100x).
    assert!(
        totals[1] < totals[0] * 40,
        "bytes grew {} -> {} for a 10x page",
        totals[0],
        totals[1]
    );
}

#[test]
fn single_item_too_large_keeps_the_error() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let output = spec(OutputFormat::Json);
    let threads: Vec<_> = (0..5).map(|i| thread(i, &mut rng)).collect();
    let shape = shape("directory", &threads, CommandResult::Directory);
    let one = encoded(&(shape.render)(1).unwrap(), &output).len();
    let fit = PageFit::for_command(&output, one - 1)
        .fit_with(&threads, &shape.render, &shape.render_one)
        .unwrap();
    // The caller's InvalidBudget carries the exact bytes the first item needs.
    assert_eq!(fit.accepted, 0);
    assert_eq!(fit.stop, FitStop::Bytes);
    assert_eq!(fit.required_minimum, Some(one as u32));
    // Exactly that budget admits the first item (and not the second).
    let fit = PageFit::for_command(&output, one)
        .fit_with(&threads, &shape.render, &shape.render_one)
        .unwrap();
    assert_eq!(fit.accepted, 1);
    assert_eq!(fit.required_minimum, None);
    // No items is not an error: nothing to place.
    let none = PageFit::for_command(&output, 0)
        .fit_with(&Vec::<()>::new(), &shape.render, &shape.render_one)
        .unwrap();
    assert_eq!(
        (none.accepted, none.stop, none.required_minimum),
        (0, FitStop::Rows, None)
    );
}

type InternalBuild<'a, T> = &'a dyn Fn(Vec<T>, u64, StopReason) -> Result<Page<T>, ApiError>;

/// The pre-D5 internal-page tail (pop and re-encode), kept as the oracle.
fn old_internal_tail<T: serde::Serialize + Clone>(
    max_bytes: usize,
    items: Vec<T>,
    mut positions: Vec<(u64, u64)>,
    final_after: u64,
    stop: StopReason,
    build: InternalBuild<'_, T>,
    detail: &'static str,
) -> Result<Page<T>, ApiError> {
    let mut result = build(items, final_after, stop)?;
    loop {
        let measured = serde_json::to_vec(&result).unwrap().len();
        if measured <= max_bytes {
            return Ok(result);
        }
        if result.items.len() <= 1 {
            if result.items.len() == 1 {
                let (_, ordinal) = positions[0];
                let single = build(result.items, ordinal, StopReason::Bytes)?;
                let minimum = serde_json::to_vec(&single).unwrap().len();
                if minimum <= max_bytes {
                    return Ok(single);
                }
                return Err(crate::store::internal_page_budget_error(detail, minimum));
            }
            return Err(crate::store::internal_page_budget_error(detail, measured));
        }
        let (before, _) = positions.pop().unwrap();
        result.items.pop();
        result = build(result.items, before, StopReason::Bytes)?;
    }
}

fn summarize<T: serde::Serialize>(result: &Result<Page<T>, ApiError>) -> String {
    match result {
        Ok(page) => serde_json::to_string(page).unwrap(),
        Err(e) => format!("{:?}|{}|{:?}", e.code, e.detail, e.required_minimum_bytes),
    }
}

fn positions(rng: &mut Rng, n: usize) -> (Vec<(u64, u64)>, u64, u64) {
    let mut after = rng.below(5) as u64;
    let mut out = Vec::new();
    for _ in 0..n {
        let ordinal = after + 1 + rng.below(4) as u64;
        out.push((after, ordinal));
        after = ordinal;
    }
    let final_after = after + rng.below(3) as u64;
    (
        out,
        final_after,
        final_after + 1 + rng.below(2) as u64 * 1_000_000,
    )
}

#[test]
fn internal_pages_match_the_old_tail() {
    let instance = "11111111-2222-3333-4444-555555555555";
    let mut rng = Rng(0x8CB9_2BA7_2F3D_8DD7);
    for trial in 0..60 {
        let n = if trial == 0 { 100 } else { rng.below(101) };
        let (pos, final_after, high) = positions(&mut rng, n);
        let work: Vec<WorkCandidate> = (0..n)
            .map(|i| WorkCandidate {
                id: format!("job-{}", "j".repeat(rng.below(40))),
                kind: WorkKind::WarningAttribution,
                position: i as u64,
                high_water: 10 + i as u64,
                has_more: i % 2 == 0,
            })
            .collect();
        let recovery: Vec<WakeRecoveryCandidate> = (0..n)
            .map(|i| WakeRecoveryCandidate {
                seat: SeatId::new(format!("seat-{}", "s".repeat(rng.below(40)))),
                attempt: WakeAttemptId::new(format!("attempt-{i}")),
                prior_daemon_boot: uuid::Uuid::from_u128(rng.next() as u128),
            })
            .collect();
        let stop = if trial % 2 == 0 {
            StopReason::Work
        } else {
            StopReason::Rows
        };
        let build_work = |items: Vec<WorkCandidate>, after: u64, stop: StopReason| {
            work_page_at(instance, items, after, high, stop)
        };
        let build_recovery = |items: Vec<WakeRecoveryCandidate>, after: u64, stop: StopReason| {
            wake_recovery_page_at(instance, items, after, high, stop)
        };
        let full_work = serde_json::to_vec(&build_work(work.clone(), final_after, stop).unwrap())
            .unwrap()
            .len();
        let full_recovery =
            serde_json::to_vec(&build_recovery(recovery.clone(), final_after, stop).unwrap())
                .unwrap()
                .len();
        for step in 0..8 {
            let work_max = [
                0,
                1,
                rng.below(full_work + 1),
                rng.below(full_work + 1),
                full_work / 3,
                full_work - full_work / 5,
                full_work,
                full_work + 10,
            ][step];
            let want = old_internal_tail(
                work_max,
                work.clone(),
                pos.clone(),
                final_after,
                stop,
                &build_work,
                "work page cannot fit",
            );
            let got = crate::store::page_fit::fit_internal(
                work_max,
                work.clone(),
                &pos,
                |items| build_work(items, final_after, stop),
                |items, before| build_work(items, before, StopReason::Bytes),
                "work page cannot fit",
            );
            assert_eq!(
                summarize(&got),
                summarize(&want),
                "work n={n} max={work_max}"
            );

            let recovery_max = [
                0,
                1,
                rng.below(full_recovery + 1),
                rng.below(full_recovery + 1),
                full_recovery / 3,
                full_recovery - full_recovery / 5,
                full_recovery,
                full_recovery + 10,
            ][step];
            let want = old_internal_tail(
                recovery_max,
                recovery.clone(),
                pos.clone(),
                final_after,
                stop,
                &build_recovery,
                "wake recovery page cannot fit",
            );
            let got = crate::store::page_fit::fit_internal(
                recovery_max,
                recovery.clone(),
                &pos,
                |items| build_recovery(items, final_after, stop),
                |items, before| build_recovery(items, before, StopReason::Bytes),
                "wake recovery page cannot fit",
            );
            assert_eq!(
                summarize(&got),
                summarize(&want),
                "recovery n={n} max={recovery_max}"
            );
        }
    }
}

#[test]
fn internal_encode_calls_are_bounded() {
    let instance = "11111111-2222-3333-4444-555555555555";
    let mut rng = Rng(0xF1EA_5EED_0BAD_CAFE);
    for n in [1usize, 2, 5, 10, 40, 100] {
        let (pos, final_after, high) = positions(&mut rng, n);
        let work: Vec<WorkCandidate> = (0..n)
            .map(|i| WorkCandidate {
                id: format!("job-{}", "j".repeat(rng.below(40))),
                kind: WorkKind::SendAttention,
                position: i as u64,
                high_water: 10 + i as u64,
                has_more: false,
            })
            .collect();
        let build = |items: Vec<WorkCandidate>, after: u64, stop: StopReason| {
            work_page_at(instance, items, after, high, stop)
        };
        let full = serde_json::to_vec(&build(work.clone(), final_after, StopReason::Work).unwrap())
            .unwrap()
            .len();
        // Budgets that admit at least the first item: a page that cannot fit
        // any item takes one more encode for the single-item fallback.
        let first = serde_json::to_vec(
            &if n == 1 {
                build(work.clone(), final_after, StopReason::Work)
            } else {
                build(work[..1].to_vec(), pos[1].0, StopReason::Bytes)
            }
            .unwrap(),
        )
        .unwrap()
        .len();
        for max in [full / 4, full / 2, full * 9 / 10, full] {
            if max < first {
                continue;
            }
            let (page, calls, _) = counted(|| {
                crate::store::page_fit::fit_internal(
                    max,
                    work.clone(),
                    &pos,
                    |items| build(items, final_after, StopReason::Work),
                    |items, before| build(items, before, StopReason::Bytes),
                    "work page cannot fit",
                )
            });
            let bound = n + 2 * ceil_log2(n) + 2;
            assert!(
                calls <= bound,
                "n={n} max={max}: {calls} encodes (bound {bound})"
            );
            assert!(!page.unwrap().items.is_empty());
        }
    }
}
