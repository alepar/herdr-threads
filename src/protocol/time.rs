use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMillis(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonoInstant(pub u64);

pub trait Clock: Send + Sync {
    fn utc_now(&self) -> UtcMillis;
    fn monotonic_now(&self) -> MonoInstant;
}

type Waker = Arc<dyn Fn() + Send + Sync>;

struct CancelInner {
    flag: AtomicBool,
    notify: tokio::sync::Notify,
    lock: Mutex<()>,
    cv: Condvar,
    next_id: AtomicU64,
    wakers: Mutex<Vec<(u64, Waker)>>,
}

/// A shared cancellation token that can be polled ([`Self::is_cancelled`]),
/// awaited ([`Self::cancelled`]), waited on from a std thread
/// ([`Self::wait_blocking`]) or observed through a waker ([`Self::on_cancel`]).
/// `cancel()` is idempotent and wakes every kind of waiter; a waiter that
/// starts after `cancel()` returns immediately (no lost wakeup).
#[derive(Clone)]
pub struct Cancellation(Arc<CancelInner>);
impl Default for Cancellation {
    fn default() -> Self {
        Self(Arc::new(CancelInner {
            flag: AtomicBool::new(false),
            notify: tokio::sync::Notify::new(),
            lock: Mutex::new(()),
            cv: Condvar::new(),
            next_id: AtomicU64::new(0),
            wakers: Mutex::new(Vec::new()),
        }))
    }
}
impl std::fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Cancellation")
            .field(&self.is_cancelled())
            .finish()
    }
}

/// Dropping the subscription unregisters the waker.
pub struct CancelSubscription {
    inner: Option<(Arc<CancelInner>, u64)>,
}
impl Drop for CancelSubscription {
    fn drop(&mut self) {
        if let Some((inner, id)) = self.inner.take() {
            lock_ignore_poison(&inner.wakers).retain(|(i, _)| *i != id);
        }
    }
}

fn lock_ignore_poison<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Cancellation {
    pub fn cancel(&self) {
        let inner = &self.0;
        inner.flag.store(true, Ordering::Release);
        // Taking the lock orders this wake after any blocking waiter's flag
        // check, so a waiter either sees the flag or is already parked.
        drop(lock_ignore_poison(&inner.lock));
        inner.cv.notify_all();
        inner.notify.notify_waiters();
        let wakers = std::mem::take(&mut *lock_ignore_poison(&inner.wakers));
        for (_, waker) in wakers {
            waker();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::Acquire)
    }
    /// Resolves once cancelled; immediately if it already is.
    pub async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
    /// Blocks the calling thread until cancelled or `timeout` elapses.
    /// Returns true when cancelled.
    pub fn wait_blocking(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now().checked_add(timeout);
        let mut guard = lock_ignore_poison(&self.0.lock);
        loop {
            if self.is_cancelled() {
                return true;
            }
            let remaining = match deadline {
                Some(d) => {
                    let now = std::time::Instant::now();
                    if now >= d {
                        return false;
                    }
                    d - now
                }
                None => Duration::from_secs(3600),
            };
            guard = self
                .0
                .cv
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
    /// Registers `waker` to run once on cancellation (immediately when
    /// already cancelled). Dropping the returned subscription unregisters it.
    pub fn on_cancel(&self, waker: Arc<dyn Fn() + Send + Sync>) -> CancelSubscription {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut wakers = lock_ignore_poison(&self.0.wakers);
            if !self.is_cancelled() {
                wakers.push((id, waker));
                return CancelSubscription {
                    inner: Some((self.0.clone(), id)),
                };
            }
        }
        waker();
        CancelSubscription { inner: None }
    }
}

#[derive(Debug, Clone)]
pub struct CallBudget {
    pub deadline: MonoInstant,
    pub cancellation: Cancellation,
}
impl CallBudget {
    pub fn is_exhausted(&self, clock: &dyn Clock) -> bool {
        self.cancellation.is_cancelled() || clock.monotonic_now() >= self.deadline
    }
    /// The deadline half of [`Self::is_exhausted`] only. A caller that has
    /// already classified cancellation uses this for the deadline branch:
    /// `is_exhausted` re-reads the token, so a cancel landing between the two
    /// reads was misreported as deadline exhaustion.
    pub fn deadline_passed(&self, clock: &dyn Clock) -> bool {
        clock.monotonic_now() >= self.deadline
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/time.rs"]
mod tests;
