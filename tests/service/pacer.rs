use super::*;
use crate::protocol::time::UtcMillis;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct FakeClock(AtomicU64);
impl Clock for FakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
fn advance(clock: &FakeClock, pacer: &Pacer, ms: u64) {
    clock.0.fetch_add(ms, Ordering::SeqCst);
    pacer.clock_advanced();
}
fn setup() -> (Arc<FakeClock>, Cancellation, Pacer) {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let cancel = Cancellation::default();
    let pacer = Pacer::with_backoff("test", clock.clone(), cancel.clone(), Backoff::with_seed(3));
    (clock, cancel, pacer)
}
/// Blocks until the pacer has recorded `n` lane entries into a wait.
fn wait_idle(pacer: &Pacer, n: u64) {
    let start = Instant::now();
    while pacer.idle_events() < n {
        assert!(start.elapsed() < Duration::from_secs(5), "lane never idled");
        std::thread::sleep(Duration::from_millis(1));
    }
}
type Lane = mpsc::Receiver<(Wake, Instant)>;
fn spawn_lane(pacer: &Pacer, tick_ms: u64) -> Lane {
    let (tx, rx) = mpsc::channel();
    let p = pacer.clone();
    std::thread::spawn(move || {
        let w = p.wait_blocking(Duration::from_millis(tick_ms));
        let _ = tx.send((w, Instant::now()));
    });
    rx
}
fn expect_blocked(rx: &Lane) {
    assert!(
        rx.recv_timeout(Duration::from_millis(30)).is_err(),
        "lane returned early"
    );
}
fn expect_wake(rx: &Lane, since: Instant, want: Wake) {
    let (w, at) = rx.recv_timeout(Duration::from_secs(2)).expect("lane stuck");
    assert_eq!(w, want);
    assert!(
        at.saturating_duration_since(since) < Duration::from_millis(20),
        "woke {:?} after the event",
        at.saturating_duration_since(since)
    );
}

fn nominal(n: u32) -> f64 {
    (100.0 * 2f64.powi(n as i32 - 1)).min(30_000.0)
}

#[test]
fn backoff_sequence_doubles_to_the_cap() {
    let mut b = Backoff::with_seed(1);
    for n in 1..=12u32 {
        let now = MonoInstant(1_000);
        let at = b.on_failure(now);
        let delay = (at.0 - now.0) as f64;
        assert!(delay >= nominal(n) * 0.8 - 1.0, "n={n} delay={delay}");
        assert!(
            delay <= (nominal(n) * 1.2).min(30_000.0) + 1.0,
            "n={n} delay={delay}"
        );
        assert!(delay <= 30_000.0);
        assert_eq!(b.attempts(), n);
        assert_eq!(b.next_retry_at(), Some(at));
    }
    assert_eq!(nominal(10), 30_000.0);
}

#[test]
fn jitter_stays_within_twenty_percent() {
    for n in 1..=5u32 {
        let (mut lo, mut hi) = (false, false);
        for seed in 0..2_000u64 {
            let mut b = Backoff::with_seed(seed);
            for _ in 1..n {
                b.on_failure(MonoInstant(0));
            }
            let d = b.on_failure(MonoInstant(0)).0 as f64;
            let nom = nominal(n);
            assert!(d >= nom * 0.8 - 1.0 && d <= nom * 1.2 + 1.0, "n={n} d={d}");
            lo |= d < nom * 0.82;
            hi |= d > nom * 1.18;
        }
        assert!(lo && hi, "n={n}: jitter not spread across the band");
    }
}

#[test]
fn success_resets_the_schedule() {
    let mut b = Backoff::with_seed(5);
    for _ in 0..5 {
        b.on_failure(MonoInstant(0));
    }
    b.on_success();
    assert_eq!(b.attempts(), 0);
    assert_eq!(b.next_retry_at(), None);
    let d = b.on_failure(MonoInstant(0)).0;
    assert!(
        (80..=120).contains(&d),
        "first failure after reset delay {d}"
    );
    assert_eq!(b.attempts(), 1);
}

#[test]
fn standalone_backoff_matches_the_pacer() {
    let (clock, _c, pacer) = {
        let clock = Arc::new(FakeClock(AtomicU64::new(0)));
        let cancel = Cancellation::default();
        let p = Pacer::with_backoff("t", clock.clone(), cancel.clone(), Backoff::with_seed(7));
        (clock, cancel, p)
    };
    let mut b = Backoff::with_seed(7);
    // true = failure, false = success
    for (i, fail) in [true, true, true, false, true, true, true, true]
        .into_iter()
        .enumerate()
    {
        advance(&clock, &pacer, 37 * i as u64);
        let now = MonoInstant(clock.0.load(Ordering::SeqCst));
        if fail {
            assert_eq!(pacer.on_failure(), b.on_failure(now));
        } else {
            pacer.on_success();
            b.on_success();
        }
        assert_eq!(pacer.attempts(), b.attempts());
        assert_eq!(pacer.next_retry_at(), b.next_retry_at());
    }
}

#[test]
fn kick_wakes_a_blocked_lane_immediately() {
    let (_clock, _c, pacer) = setup();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    std::thread::sleep(Duration::from_millis(30));
    let kicked = Instant::now();
    pacer.kick();
    expect_wake(&rx, kicked, Wake::Kicked);
}

#[test]
fn retry_due_wakes_at_next_retry_at() {
    let (clock, _c, pacer) = setup();
    let at = pacer.on_failure().0;
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    advance(&clock, &pacer, at - 1);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, 1);
    expect_wake(&rx, t, Wake::RetryDue);
}

#[test]
fn tick_fires_at_the_safety_tick() {
    let (clock, _c, pacer) = setup();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    advance(&clock, &pacer, 4_999);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, 1);
    expect_wake(&rx, t, Wake::Tick);
}

#[test]
fn cancel_wakes_and_stops() {
    let (_clock, cancel, pacer) = setup();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    let t = Instant::now();
    cancel.cancel();
    expect_wake(&rx, t, Wake::Cancelled);
    let t = Instant::now();
    assert_eq!(pacer.wait_blocking(Duration::from_secs(5)), Wake::Cancelled);
    assert!(t.elapsed() < Duration::from_millis(20));
}

#[test]
fn kicked_return_restarts_the_full_tick() {
    let (clock, _c, pacer) = setup();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    advance(&clock, &pacer, 3_000);
    pacer.kick();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap().0,
        Wake::Kicked
    );
    // New call at fake t=3000: the old tick would end at 5000.
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 2);
    advance(&clock, &pacer, 2_000);
    expect_blocked(&rx);
    advance(&clock, &pacer, 2_999);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, 1);
    expect_wake(&rx, t, Wake::Tick);
}

#[test]
fn kick_during_backoff_is_latched_not_shortening() {
    let (clock, _c, pacer) = setup();
    let at = pacer.on_failure().0;
    pacer.kick();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    expect_blocked(&rx);
    advance(&clock, &pacer, at - 1);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, 1);
    expect_wake(&rx, t, Wake::RetryDue);
    // The RetryDue return drained the latched kick.
    pacer.on_success();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 2);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, 5_000);
    expect_wake(&rx, t, Wake::Tick);
}

#[test]
fn tick_does_not_cut_a_backoff_short() {
    let (clock, _c, pacer) = setup();
    let mut at = 0;
    for _ in 0..9 {
        at = pacer.on_failure().0;
    }
    assert!(at >= 20_000, "9th failure nominal is 25.6 s, got {at}");
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    advance(&clock, &pacer, 5_000);
    expect_blocked(&rx);
    advance(&clock, &pacer, at - 5_001);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, 1);
    expect_wake(&rx, t, Wake::RetryDue);
}

#[test]
fn idle_hook_fires_once_per_drain() {
    let (_clock, _c, pacer) = setup();
    let seen = Arc::new(Mutex::new(Vec::<u64>::new()));
    let s = seen.clone();
    pacer.set_idle_hook(Box::new(move |n| s.lock().unwrap().push(n)));
    for i in 1..=3u64 {
        let rx = spawn_lane(&pacer, 5_000);
        wait_idle(&pacer, i);
        pacer.kick();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap().0,
            Wake::Kicked
        );
    }
    assert_eq!(*seen.lock().unwrap(), vec![1, 2, 3]);
    assert_eq!(pacer.idle_events(), 3);
}

#[tokio::test]
async fn async_wait_kick_and_cancel() {
    let (_clock, cancel, pacer) = setup();
    let p = pacer.clone();
    let task = tokio::spawn(async move { p.wait(Duration::from_secs(5)).await });
    while pacer.idle_events() < 1 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    pacer.kick();
    let w = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w, Wake::Kicked);
    let p = pacer.clone();
    let task = tokio::spawn(async move { p.wait(Duration::from_secs(5)).await });
    while pacer.idle_events() < 2 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    cancel.cancel();
    let w = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(w, Wake::Cancelled);
}

#[test]
fn explicit_kick_during_backoff_runs_a_pass_and_keeps_the_schedule() {
    let (clock, _c, pacer) = setup();
    let at = pacer.on_failure().0;
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    expect_blocked(&rx);
    let t = Instant::now();
    pacer.kick_explicit();
    expect_wake(&rx, t, Wake::Kicked);
    // The schedule is untouched: still one attempt, same retry time.
    assert_eq!(pacer.attempts(), 1);
    assert_eq!(pacer.next_retry_at().map(|m| m.0), Some(at));
    // The explicit latch is consumed: the next wait blocks until RetryDue.
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 2);
    expect_blocked(&rx);
    let t = Instant::now();
    advance(&clock, &pacer, at);
    expect_wake(&rx, t, Wake::RetryDue);
}

#[test]
fn explicit_kick_also_consumes_an_ordinary_latched_kick() {
    let (_clock, _c, pacer) = setup();
    pacer.on_failure();
    pacer.kick();
    pacer.kick_explicit();
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 1);
    expect_wake(&rx, Instant::now(), Wake::Kicked);
    let rx = spawn_lane(&pacer, 5_000);
    wait_idle(&pacer, 2);
    expect_blocked(&rx); // no second Kicked from the ordinary latch
}

#[test]
fn cancel_beats_an_explicit_kick() {
    let (_clock, cancel, pacer) = setup();
    pacer.on_failure();
    pacer.kick_explicit();
    cancel.cancel();
    assert_eq!(pacer.wait_blocking(Duration::from_secs(5)), Wake::Cancelled);
}

#[tokio::test]
async fn async_explicit_kick_during_backoff_wakes() {
    let (_clock, _c, pacer) = setup();
    pacer.on_failure();
    pacer.kick_explicit();
    assert_eq!(pacer.wait(Duration::from_secs(5)).await, Wake::Kicked);
}
