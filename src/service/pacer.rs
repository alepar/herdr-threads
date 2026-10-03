//! Wakeup-and-backoff primitive for service lanes.
//!
//! A [`Pacer`] gives a lane one place to block: it wakes on a [`Pacer::kick`],
//! on cancellation, when a failure backoff comes due, or at a safety tick.
//! [`Backoff`] is the standalone schedule (100 ms x 2^(n-1), capped at 30 s,
//! +-20 % jitter, reset on success) so per-seat instances can be kept without a
//! Pacer. All time is read from the injected [`Clock`], so tests drive a fake
//! clock and call [`Pacer::clock_advanced`] after moving it.
//!
//! Wait precedence, checked under the Pacer mutex: `Cancelled`, then an
//! explicit kick ([`Pacer::kick_explicit`]: `Kicked`, even while backing off),
//! then (while failures are outstanding) `RetryDue` once `now >=
//! next_retry_at`, then `Kicked` for an ordinary latched kick, then `Tick` once
//! `now >= call_start + tick`. The tick is measured from the start of each wait
//! call, so a `Kicked` return followed by a new call always waits the full
//! tick. While backing off (`attempts > 0 && now < next_retry_at`) ordinary
//! kicks stay latched and the safety tick is ignored, so neither shortens a
//! backoff. An explicit kick does end the wait but leaves the schedule
//! untouched: a pass it starts that fails records the next backoff step as
//! usual. The `RetryDue` and explicit `Kicked` returns clear the latched
//! kicks, because that pass drains the work anyway.
use crate::protocol::time::{CancelSubscription, Cancellation, Clock, MonoInstant};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Duration;

const BASE_MS: u64 = 100;
const CAP_MS: u64 = 30_000;
const JITTER: f64 = 0.2;

/// Why a wait returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    Kicked,
    RetryDue,
    Tick,
    Cancelled,
}

/// Capped exponential backoff with jitter. `nominal(n) = min(100 ms * 2^(n-1),
/// 30 s)` for the n-th consecutive failure; the delay is `nominal * (1 + j)`
/// with `j` uniform in `[-0.2, 0.2]`, clamped to the 30 s cap.
#[derive(Debug, Clone)]
pub struct Backoff {
    attempts: u32,
    next_retry_at: Option<MonoInstant>,
    rng: u64,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    pub fn new() -> Self {
        let id = uuid::Uuid::new_v4().as_u128();
        Self::with_seed((id as u64) ^ ((id >> 64) as u64))
    }
    /// Deterministic jitter, for tests.
    pub fn with_seed(seed: u64) -> Self {
        // xorshift64 must not be seeded with zero; scramble so small seeds differ.
        let rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        Self {
            attempts: 0,
            next_retry_at: None,
            rng: if rng == 0 { 0x2545_F491_4F6C_DD1D } else { rng },
        }
    }
    fn next_unit(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Records a failure at `now` and returns the time of the next retry.
    pub fn on_failure(&mut self, now: MonoInstant) -> MonoInstant {
        self.attempts = self.attempts.saturating_add(1);
        let shift = (self.attempts - 1).min(20);
        let nominal = (BASE_MS << shift).min(CAP_MS) as f64;
        let j = (self.next_unit() * 2.0 - 1.0) * JITTER;
        let delay = ((nominal * (1.0 + j)).round() as u64).min(CAP_MS);
        let at = MonoInstant(now.0.saturating_add(delay));
        self.next_retry_at = Some(at);
        at
    }
    pub fn on_success(&mut self) {
        self.attempts = 0;
        self.next_retry_at = None;
    }
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
    pub fn next_retry_at(&self) -> Option<MonoInstant> {
        self.next_retry_at
    }
}

struct State {
    kick_latched: bool,
    explicit_kick: bool,
    backoff: Backoff,
    idle_events: u64,
    /// Wait-loop evaluations and the ones that ended a wait; test-support
    /// observers use them to step a fake clock in lockstep with the lane.
    evaluations: u64,
    wakes: u64,
}

#[cfg(any(test, feature = "test-support"))]
type IdleHook = Box<dyn Fn(u64) + Send + Sync>;

struct Inner {
    lane: &'static str,
    clock: Arc<dyn Clock>,
    cancel: Cancellation,
    state: Mutex<State>,
    cv: Condvar,
    notify: tokio::sync::Notify,
    #[cfg(any(test, feature = "test-support"))]
    idle_hook: Mutex<Option<IdleHook>>,
    subscription: Mutex<Option<CancelSubscription>>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn wake_all(&self) {
        self.cv.notify_all();
        self.notify.notify_waiters();
    }
    /// Counts one lane entry into a wait and runs the idle hook.
    fn record_idle(&self) {
        let count = {
            let mut st = self.lock();
            st.idle_events += 1;
            st.idle_events
        };
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = self
            .idle_hook
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            hook(count);
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = count;
    }
    /// Decides the wake reason, or how long (clock ms) until the next time-based one.
    fn evaluate(&self, st: &mut State, deadline: u64) -> Result<Wake, u64> {
        st.evaluations += 1;
        let decision = self.decide(st, deadline);
        if decision.is_ok() {
            st.wakes += 1;
        }
        decision
    }
    fn decide(&self, st: &mut State, deadline: u64) -> Result<Wake, u64> {
        if self.cancel.is_cancelled() {
            return Ok(Wake::Cancelled);
        }
        if st.explicit_kick {
            st.explicit_kick = false;
            st.kick_latched = false;
            return Ok(Wake::Kicked);
        }
        let now = self.clock.monotonic_now().0;
        if st.backoff.attempts() > 0 {
            let due = st.backoff.next_retry_at().map_or(0, |t| t.0);
            if now >= due {
                st.kick_latched = false;
                st.explicit_kick = false;
                return Ok(Wake::RetryDue);
            }
            return Err(due - now);
        }
        if st.kick_latched {
            st.kick_latched = false;
            return Ok(Wake::Kicked);
        }
        if now >= deadline {
            return Ok(Wake::Tick);
        }
        Err(deadline - now)
    }
}

/// Per-lane wakeup primitive; cheap to clone (shared state).
#[derive(Clone)]
pub struct Pacer(Arc<Inner>);

impl Pacer {
    pub fn new(lane: &'static str, clock: Arc<dyn Clock>, cancel: Cancellation) -> Self {
        Self::with_backoff(lane, clock, cancel, Backoff::new())
    }

    pub fn with_backoff(
        lane: &'static str,
        clock: Arc<dyn Clock>,
        cancel: Cancellation,
        backoff: Backoff,
    ) -> Self {
        let inner = Arc::new(Inner {
            lane,
            clock,
            cancel: cancel.clone(),
            state: Mutex::new(State {
                kick_latched: false,
                explicit_kick: false,
                backoff,
                idle_events: 0,
                evaluations: 0,
                wakes: 0,
            }),
            cv: Condvar::new(),
            notify: tokio::sync::Notify::new(),
            #[cfg(any(test, feature = "test-support"))]
            idle_hook: Mutex::new(None),
            subscription: Mutex::new(None),
        });
        let weak: Weak<Inner> = Arc::downgrade(&inner);
        let sub = cancel.on_cancel(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                // Taking the state mutex orders the wake after a waiter's flag check.
                drop(inner.lock());
                inner.wake_all();
            }
        }));
        *inner.subscription.lock().unwrap_or_else(|e| e.into_inner()) = Some(sub);
        Self(inner)
    }

    pub fn lane(&self) -> &'static str {
        self.0.lane
    }

    pub fn cancellation(&self) -> &Cancellation {
        &self.0.cancel
    }

    /// Latches a kick and wakes a waiting lane. Kicks coalesce.
    pub fn kick(&self) {
        self.0.lock().kick_latched = true;
        self.0.wake_all();
    }

    /// An explicit kick (an operator or explicit target capture): ends any
    /// wait with `Wake::Kicked`, including a backoff wait, without touching the
    /// backoff schedule. A pass it starts that fails records the next backoff
    /// step as usual, so it never adds more than one pass per step. Commit
    /// kicks use [`Self::kick`], which never shortens a backoff.
    pub fn kick_explicit(&self) {
        self.0.lock().explicit_kick = true;
        self.0.wake_all();
    }

    /// Records a failure; returns the time of the next retry.
    pub fn on_failure(&self) -> MonoInstant {
        let now = self.0.clock.monotonic_now();
        self.0.lock().backoff.on_failure(now)
    }

    /// Records a success: attempts back to 0, no retry pending.
    pub fn on_success(&self) {
        self.0.lock().backoff.on_success();
    }

    pub fn attempts(&self) -> u32 {
        self.0.lock().backoff.attempts()
    }

    /// The Pacer's clock reading, for rendering `next_retry_at`.
    pub fn now(&self) -> MonoInstant {
        self.0.clock.monotonic_now()
    }

    pub fn next_retry_at(&self) -> Option<MonoInstant> {
        self.0.lock().backoff.next_retry_at()
    }

    /// Wakes waiters to re-read the clock without counting as a kick. Tests
    /// call this after advancing a fake clock; it is a no-op in effect for a
    /// real clock.
    pub fn clock_advanced(&self) {
        drop(self.0.lock());
        self.0.wake_all();
    }

    /// Number of times the lane has entered a wait (finished a pass).
    pub fn idle_events(&self) -> u64 {
        self.0.lock().idle_events
    }

    /// Wait-loop evaluations so far (each re-reads the clock).
    #[cfg(any(test, feature = "test-support"))]
    pub fn evaluations(&self) -> u64 {
        self.0.lock().evaluations
    }

    /// Waits that ended with a wake reason so far. A lane that is blocked in
    /// its wait has `idle_events() == wakes() + 1` (its boot pass runs before
    /// the first wait), so a mismatch means a pass is still running.
    #[cfg(any(test, feature = "test-support"))]
    pub fn wakes(&self) -> u64 {
        self.0.lock().wakes
    }

    /// Registers a hook run on every idle event with the running count.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_idle_hook(&self, hook: IdleHook) {
        *self.0.idle_hook.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    fn call_deadline(&self, tick: Duration) -> u64 {
        let start = self.0.clock.monotonic_now().0;
        start.saturating_add(u64::try_from(tick.as_millis()).unwrap_or(u64::MAX))
    }

    /// Blocks the calling (std) thread until a [`Wake`] reason applies.
    pub fn wait_blocking(&self, tick: Duration) -> Wake {
        let deadline = self.call_deadline(tick);
        self.0.record_idle();
        let mut st = self.0.lock();
        loop {
            match self.0.evaluate(&mut st, deadline) {
                Ok(w) => return w,
                Err(remaining_ms) => {
                    let timeout = Duration::from_millis(remaining_ms.max(1));
                    st = self
                        .0
                        .cv
                        .wait_timeout(st, timeout)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
            }
        }
    }

    /// Async twin of [`Self::wait_blocking`] with the same precedence.
    pub async fn wait(&self, tick: Duration) -> Wake {
        let deadline = self.call_deadline(tick);
        self.0.record_idle();
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let remaining_ms = {
                let mut st = self.0.lock();
                match self.0.evaluate(&mut st, deadline) {
                    Ok(w) => return w,
                    Err(r) => r,
                }
            };
            tokio::select! {
                _ = &mut notified => {}
                _ = self.0.cancel.cancelled() => {}
                _ = tokio::time::sleep(Duration::from_millis(remaining_ms.max(1))) => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/service/pacer.rs"]
mod tests;
