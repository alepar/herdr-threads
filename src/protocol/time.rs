use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UtcMillis(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonoInstant(pub u64);

pub trait Clock: Send + Sync {
    fn utc_now(&self) -> UtcMillis;
    fn monotonic_now(&self) -> MonoInstant;
}

#[derive(Debug, Clone)]
pub struct Cancellation(Arc<AtomicBool>);
impl Default for Cancellation {
    fn default() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
}
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
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
