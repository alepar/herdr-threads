use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

#[test]
fn cancel_before_wait_returns_immediately() {
    let c = Cancellation::default();
    c.cancel();
    let start = Instant::now();
    assert!(c.wait_blocking(Duration::from_secs(5)));
    assert!(start.elapsed() < Duration::from_millis(20));
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_before_async_wait_completes_at_once() {
    let c = Cancellation::default();
    c.cancel();
    tokio::time::timeout(Duration::from_millis(20), c.cancelled())
        .await
        .expect("cancelled() must complete at once");
}

#[test]
fn cancel_during_blocking_wait_wakes_it() {
    let c = Cancellation::default();
    let c2 = c.clone();
    let waiter = std::thread::spawn(move || {
        let r = c2.wait_blocking(Duration::from_secs(10));
        (r, Instant::now())
    });
    std::thread::sleep(Duration::from_millis(50));
    let cancelled_at = Instant::now();
    c.cancel();
    let (r, woke) = waiter.join().unwrap();
    assert!(r);
    assert!(woke.duration_since(cancelled_at) < Duration::from_millis(20));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_from_another_thread_wakes_async_waiter() {
    let c = Cancellation::default();
    let c2 = c.clone();
    let start = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        c2.cancel();
    });
    tokio::time::timeout(Duration::from_secs(2), c.cancelled())
        .await
        .expect("async waiter must wake");
    assert!(start.elapsed() < Duration::from_millis(30 + 20));
}

#[test]
fn wait_blocking_times_out_false_when_not_cancelled() {
    let c = Cancellation::default();
    let start = Instant::now();
    assert!(!c.wait_blocking(Duration::from_millis(50)));
    assert!(start.elapsed() >= Duration::from_millis(50));
    assert!(!c.is_cancelled());
}

#[test]
fn stress_no_lost_wakeup() {
    for i in 0..1000u64 {
        let c = Cancellation::default();
        let c2 = c.clone();
        let waiter = std::thread::spawn(move || c2.wait_blocking(Duration::from_secs(1)));
        // Vary the cancel point: before, during or after the waiter enters.
        for _ in 0..(i % 50) * 20 {
            std::hint::spin_loop();
        }
        if i % 7 == 0 {
            std::thread::yield_now();
        }
        c.cancel();
        let start = Instant::now();
        assert!(waiter.join().unwrap(), "iteration {i} lost the wakeup");
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}

#[test]
fn subscriber_runs_once_and_unsubscribes_on_drop() {
    let c = Cancellation::default();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let _kept = c.on_cancel(Arc::new(move || {
        h.fetch_add(1, Ordering::SeqCst);
    }));
    let h2 = hits.clone();
    let dropped = c.on_cancel(Arc::new(move || {
        h2.fetch_add(100, Ordering::SeqCst);
    }));
    drop(dropped);
    c.cancel();
    c.cancel();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // Subscribing after cancellation runs the waker immediately.
    let h3 = hits.clone();
    let _late = c.on_cancel(Arc::new(move || {
        h3.fetch_add(10, Ordering::SeqCst);
    }));
    assert_eq!(hits.load(Ordering::SeqCst), 11);
}
