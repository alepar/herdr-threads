//! ht-p03.11: rate-limited lane logging on a fake clock. Each test names the
//! mutation it kills.
use super::*;
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::{MonoInstant, UtcMillis},
};
use crate::service::{kicks::Lane, workers::WorkerStatus};
use std::sync::atomic::{AtomicU64, Ordering};

struct FakeClock(AtomicU64);
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}

fn limiter() -> (
    Arc<FakeClock>,
    Arc<RateLimitedLaneLog>,
    Arc<Mutex<Vec<String>>>,
) {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&lines);
    let log = Arc::new(RateLimitedLaneLog::new(
        clock.clone(),
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned())),
    ));
    (clock, log, lines)
}

fn at(clock: &FakeClock, ms: u64) {
    clock.0.store(ms, Ordering::SeqCst);
}

/// Kills: logging every failure (100 lines), never summarizing (1 line), a
/// summary that forgets the count, and a window that never restarts.
#[test]
fn first_occurrence_then_one_summary_per_window() {
    let (clock, log, lines) = limiter();
    let error = ApiError::new(ErrorCode::HostUnavailable, "herdr down");
    for i in 0..100u64 {
        at(&clock, i * 95_000 / 99);
        log.record_with_attempt(Lane::Observation, &error, 4);
    }
    let lines = lines.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "lane observation: HostUnavailable: herdr down".to_owned(),
            "lane observation: HostUnavailable repeated 32 times in the last 30s (attempt 4)"
                .to_owned(),
            "lane observation: HostUnavailable repeated 32 times in the last 30s (attempt 4)"
                .to_owned(),
            "lane observation: HostUnavailable repeated 32 times in the last 30s (attempt 4)"
                .to_owned(),
        ]
    );
}

/// Kills: keying the limiter on the lane alone (a new code would be hidden)
/// or on the code alone (another lane's first line would be hidden).
#[test]
fn different_codes_get_their_own_first_line() {
    let (clock, log, lines) = limiter();
    at(&clock, 10);
    log.record(Lane::Wakes, &ApiError::new(ErrorCode::StoreBusy, "busy"));
    log.record(Lane::Wakes, &ApiError::new(ErrorCode::StoreBusy, "busy"));
    log.record(Lane::Wakes, &ApiError::new(ErrorCode::StoreCorrupt, "bad"));
    log.record(
        Lane::Retention,
        &ApiError::new(ErrorCode::StoreBusy, "busy\nsecond line"),
    );
    assert_eq!(
        lines.lock().unwrap().clone(),
        vec![
            "lane wake: StoreBusy: busy".to_owned(),
            "lane wake: StoreCorrupt: bad".to_owned(),
            "lane retention: StoreBusy: busy second line".to_owned(),
        ]
    );
}

/// Kills: a hook-parse entry that bypasses the limiter, and one that shares a
/// key with another harness.
#[test]
fn hook_parse_failures_are_rate_limited_too() {
    let (clock, log, lines) = limiter();
    for i in 0..61u64 {
        at(&clock, i * 1_000);
        log.record_hook_parse_failure("claude", "unexpected eof");
    }
    log.record_hook_parse_failure("codex", "bad json");
    assert_eq!(
        lines.lock().unwrap().clone(),
        vec![
            "hook claude: parse_failure: unexpected eof".to_owned(),
            "hook claude: parse_failure repeated 30 times in the last 30s (attempt 0)".to_owned(),
            "hook claude: parse_failure repeated 30 times in the last 30s (attempt 0)".to_owned(),
            "hook codex: parse_failure: bad json".to_owned(),
        ]
    );
}

/// Kills: a lane (deadline, wake, retention, admission observer, observation)
/// whose `WorkerStatus` is not wired to the logger, and a status that
/// forgets the Pacer attempt count.
#[test]
fn injected_failure_in_each_lane_lands_rate_limited() {
    use crate::service::pacer::Pacer;
    let (clock, log, lines) = limiter();
    for lane in Lane::ALL {
        let status = WorkerStatus::default();
        status.set_error_log(log.clone());
        let pacer = Arc::new(Pacer::new(
            lane.name(),
            clock.clone(),
            crate::protocol::time::Cancellation::default(),
        ));
        status.attach_pacer(Arc::clone(&pacer));
        pacer.on_failure();
        let error = ApiError::new(ErrorCode::StoreBusy, "injected");
        status.record_failure(lane, &error);
        at(&clock, clock.0.load(Ordering::SeqCst) + 1_000);
        status.record_failure(lane, &error);
        at(&clock, clock.0.load(Ordering::SeqCst) + 31_000);
        status.record_failure(lane, &error);
    }
    let lines = lines.lock().unwrap().clone();
    for lane in Lane::ALL {
        let first = format!("lane {}: StoreBusy: injected", lane.name());
        let summary = format!(
            "lane {}: StoreBusy repeated 2 times in the last 30s (attempt 1)",
            lane.name()
        );
        assert_eq!(
            lines.iter().filter(|line| **line == first).count(),
            1,
            "{lines:?}"
        );
        assert_eq!(
            lines.iter().filter(|line| **line == summary).count(),
            1,
            "{lines:?}"
        );
    }
    assert_eq!(lines.len(), 10, "{lines:?}");
}

/// Kills: an unthrottled verification line (one per wake), a limiter shared
/// between `unsubmitted` and `not_checked` (the second code's first line would
/// be hidden), and a log that reports `Verified` / `Retried` wakes.
#[test]
fn wake_verification_lines_are_rate_limited() {
    use crate::scheduler::SubmissionVerification::{NotChecked, Retried, Unsubmitted, Verified};
    let (clock, log, lines) = limiter();
    let seat = crate::protocol::ids::SeatId::new("seat_1");
    for ms in [0, 10_000, 20_000] {
        at(&clock, ms);
        log.record_wake_verification(&seat, Unsubmitted);
    }
    at(&clock, 31_000);
    log.record_wake_verification(&seat, Unsubmitted);
    log.record_wake_verification(&seat, NotChecked);
    log.record_wake_verification(&seat, Verified);
    log.record_wake_verification(&seat, Retried);
    assert_eq!(
        lines.lock().unwrap().clone(),
        vec![
            "lane wakes: verification unsubmitted: seat seat_1: wake prompt unsubmitted after one submit-key retry".to_owned(),
            "lane wakes: verification unsubmitted repeated 3 times in the last 30s (attempt 0)".to_owned(),
            "lane wakes: verification not_checked: seat seat_1: wake prompt submission not checked".to_owned(),
        ]
    );
}
