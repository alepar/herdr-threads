//! A clock for fixtures whose subject is not elapsed time.

use crate::protocol::time::{Clock, MonoInstant, UtcMillis};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Real UTC; monotonic time runs at 1/100 of real speed. A production
/// wall-clock read budget (search's 100 ms) then spans 10 s of real time, so
/// a fixture that calibrates byte budgets is not decided by host load. The
/// time budgets themselves are covered by their own ticking-clock tests.
/// Waits on the monotonic clock still end, 100 times later.
pub struct SlowMonotonicClock {
    started: Instant,
}

impl SlowMonotonicClock {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl Default for SlowMonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SlowMonotonicClock {
    fn utc_now(&self) -> UtcMillis {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        UtcMillis(i64::try_from(millis).unwrap_or(i64::MAX))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(u64::try_from(self.started.elapsed().as_millis() / 100).unwrap_or(u64::MAX))
    }
}
