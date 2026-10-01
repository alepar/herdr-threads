//! Validated duration values passed to store operations at creation time.

use crate::protocol::time::UtcMillis;

pub const DEFAULT_OBLIGATION_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeadlineDuration(i64);
impl DeadlineDuration {
    pub fn millis(millis: u64) -> Result<Self, &'static str> {
        if millis == 0 {
            return Err("duration must be positive");
        }
        Ok(Self(
            i64::try_from(millis).map_err(|_| "duration overflow")?,
        ))
    }
    pub fn seconds(seconds: u64) -> Result<Self, &'static str> {
        let millis = seconds.checked_mul(1_000).ok_or("duration overflow")?;
        Self::millis(millis)
    }
    pub fn as_millis(self) -> i64 {
        self.0
    }
    pub fn deadline_after(self, start: UtcMillis) -> Result<UtcMillis, &'static str> {
        start
            .0
            .checked_add(self.0)
            .map(UtcMillis)
            .ok_or("deadline overflow")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerTiming {
    pub invitation: DeadlineDuration,
    pub receipt: DeadlineDuration,
}
impl Default for SchedulerTiming {
    fn default() -> Self {
        let five_minutes =
            DeadlineDuration::seconds(DEFAULT_OBLIGATION_SECONDS).expect("fixed default");
        Self {
            invitation: five_minutes,
            receipt: five_minutes,
        }
    }
}
impl SchedulerTiming {
    pub fn new(invitation_millis: u64, receipt_millis: u64) -> Result<Self, &'static str> {
        Ok(Self {
            invitation: DeadlineDuration::millis(invitation_millis)?,
            receipt: DeadlineDuration::millis(receipt_millis)?,
        })
    }
    pub fn invitation_for_operation(
        self,
        override_millis: Option<u64>,
    ) -> Result<DeadlineDuration, &'static str> {
        override_millis
            .map(DeadlineDuration::millis)
            .unwrap_or(Ok(self.invitation))
    }
    pub fn receipt_for_operation(
        self,
        override_millis: Option<u64>,
    ) -> Result<DeadlineDuration, &'static str> {
        override_millis
            .map(DeadlineDuration::millis)
            .unwrap_or(Ok(self.receipt))
    }
}
