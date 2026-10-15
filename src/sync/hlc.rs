//! Hybrid logical clock.
//!
//! Responsibility: this module owns the local tick, folding a remote stamp
//! under the 60 s skew cap, the capped resume from the self log, and the
//! packing of a stamp (48-bit Unix ms, 16-bit logical). It does not decide
//! deadlines: ACK and invitation deadlines use the deciding node's wall
//! `Clock`, never an HLC. An [`Hlc`] has no lock of its own and is never
//! locked outside Journal. New clock rules go here and nowhere else.
//!
//! Role of a stamp: it orders last-writer-wins registers (compared as the
//! tuple `(HlcStamp, NodeUuid)`) and is the origin's "sent at" time. It does
//! not order a log; the per-log counter does.

use crate::protocol::time::UtcMillis;

/// Largest Unix ms an [`HlcStamp`] can hold (48 bits).
const MS_LIMIT: u64 = 1 << 48;
const LOGICAL_BITS: u32 = 16;

/// A remote stamp whose ms is further ahead of the local wall than this is not
/// absorbed into the clock.
pub const MAX_ABSORB_MS: u64 = 60_000;

/// 48-bit Unix ms followed by a 16-bit logical counter, ordered as a `u64`.
///
/// Owns the bit packing; see the module doc for the clock's responsibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HlcStamp(u64);

impl HlcStamp {
    pub const ZERO: HlcStamp = HlcStamp(0);

    /// Builds a stamp; `ms` is clamped below 2^48.
    pub fn from_parts(ms: u64, logical: u16) -> Self {
        HlcStamp((ms.min(MS_LIMIT - 1) << LOGICAL_BITS) | u64::from(logical))
    }

    pub fn from_u64(raw: u64) -> Self {
        HlcStamp(raw)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn ms(self) -> u64 {
        self.0 >> LOGICAL_BITS
    }

    pub fn logical(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    fn next(self) -> Self {
        HlcStamp(self.0.saturating_add(1))
    }
}

/// Outcome of folding a remote stamp into the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Absorbed,
    /// The remote stamp is too far ahead of the local wall; the clock is unchanged.
    Skewed {
        ahead_ms: u64,
    },
}

/// A node's hybrid logical clock. Journal is its only mutator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hlc {
    last: HlcStamp,
}

/// Wall time as Unix ms clamped to `0..2^48`.
fn wall_ms(wall: UtcMillis) -> u64 {
    u64::try_from(wall.0).unwrap_or(0).min(MS_LIMIT - 1)
}

fn wall_stamp(wall: UtcMillis) -> HlcStamp {
    HlcStamp::from_parts(wall_ms(wall), 0)
}

impl Hlc {
    pub fn new() -> Self {
        Hlc {
            last: HlcStamp::ZERO,
        }
    }

    pub fn last(&self) -> HlcStamp {
        self.last
    }

    /// Restarts the clock from the largest stamp in the self log, capped at
    /// wall + 60 s so a stamp the running clock refused to absorb stays refused.
    pub fn resume(wall: UtcMillis, self_max: Option<HlcStamp>) -> Self {
        let capped_ms = (wall_ms(wall) + MAX_ABSORB_MS).min(MS_LIMIT - 1);
        let cap = HlcStamp::from_parts(capped_ms, 0);
        let from_log = self_max.map_or(HlcStamp::ZERO, |m| m.min(cap));
        Hlc {
            last: wall_stamp(wall).max(from_log),
        }
    }

    /// The next local stamp: `max(wall << 16, last + 1)`.
    pub fn tick(&mut self, wall: UtcMillis) -> HlcStamp {
        let next = wall_stamp(wall).max(self.last.next());
        self.last = next;
        next
    }

    /// A stamp above `floor` for an LWW write. `last` takes it only when the
    /// floor is within the skew cap; otherwise `last` is left exactly as it was.
    pub fn tick_above(&mut self, wall: UtcMillis, floor: HlcStamp) -> HlcStamp {
        let result = wall_stamp(wall).max(self.last.next()).max(floor.next());
        if floor.ms() <= wall_ms(wall) + MAX_ABSORB_MS {
            self.last = result;
        }
        result
    }

    /// Folds a remote stamp in when it is at most 60 s ahead of the wall.
    pub fn fold(&mut self, wall: UtcMillis, remote: HlcStamp) -> Fold {
        let wall = wall_ms(wall);
        if remote.ms() <= wall + MAX_ABSORB_MS {
            self.last = self.last.max(remote);
            Fold::Absorbed
        } else {
            Fold::Skewed {
                ahead_ms: remote.ms() - wall,
            }
        }
    }
}

impl Default for Hlc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(ms: i64) -> UtcMillis {
        UtcMillis(ms)
    }

    #[test]
    fn tick_strictly_increases_when_wall_steps_back() {
        let mut hlc = Hlc::new();
        let a = hlc.tick(w(1_000));
        let b = hlc.tick(w(500));
        let c = hlc.tick(w(500));
        assert_eq!(a, HlcStamp::from_parts(1_000, 0));
        assert_eq!(b, HlcStamp::from_parts(1_000, 1));
        assert_eq!(c, HlcStamp::from_parts(1_000, 2));
        let d = hlc.tick(w(2_000));
        assert_eq!(d, HlcStamp::from_parts(2_000, 0));
    }

    #[test]
    fn logical_carries_at_ffff() {
        let mut hlc = Hlc {
            last: HlcStamp::from_parts(7, 0xFFFF),
        };
        let next = hlc.tick(w(7));
        assert_eq!(next, HlcStamp::from_parts(8, 0));
        assert_eq!((next.ms(), next.logical()), (8, 0));
    }

    #[test]
    fn tick_saturates_at_u64_max() {
        let mut hlc = Hlc {
            last: HlcStamp::from_u64(u64::MAX),
        };
        assert_eq!(hlc.tick(w(1)).as_u64(), u64::MAX);
    }

    #[test]
    fn fold_absorbs_at_60000_and_skews_at_60001() {
        let mut hlc = Hlc::new();
        let at_cap = HlcStamp::from_parts(1_000 + 60_000, 3);
        assert_eq!(hlc.fold(w(1_000), at_cap), Fold::Absorbed);
        assert_eq!(hlc.last(), at_cap);

        let mut hlc = Hlc::new();
        let beyond = HlcStamp::from_parts(1_000 + 60_001, 0);
        assert_eq!(
            hlc.fold(w(1_000), beyond),
            Fold::Skewed { ahead_ms: 60_001 }
        );
        assert_eq!(hlc.last(), HlcStamp::ZERO);
    }

    #[test]
    fn fold_never_lowers_last() {
        let mut hlc = Hlc {
            last: HlcStamp::from_parts(5_000, 9),
        };
        assert_eq!(
            hlc.fold(w(5_000), HlcStamp::from_parts(10, 0)),
            Fold::Absorbed
        );
        assert_eq!(hlc.last(), HlcStamp::from_parts(5_000, 9));
    }

    #[test]
    fn tick_above_returns_floor_plus_one_and_keeps_last_beyond_cap() {
        let before = HlcStamp::from_parts(1_000, 4);
        let mut hlc = Hlc { last: before };
        let floor = HlcStamp::from_parts(1_000 + 60_001, 7);
        let got = hlc.tick_above(w(1_000), floor);
        assert_eq!(got, HlcStamp::from_parts(1_000 + 60_001, 8));
        assert_eq!(hlc.last(), before);
    }

    #[test]
    fn tick_above_commits_within_cap() {
        let mut hlc = Hlc::new();
        let floor = HlcStamp::from_parts(1_000 + 60_000, 0xFFFF);
        let got = hlc.tick_above(w(1_000), floor);
        assert_eq!(got, HlcStamp::from_parts(1_000 + 60_001, 0));
        assert_eq!(hlc.last(), got);
    }

    #[test]
    fn tick_above_low_floor_acts_as_tick() {
        let mut hlc = Hlc::new();
        let got = hlc.tick_above(w(1_000), HlcStamp::from_parts(3, 0));
        assert_eq!(got, HlcStamp::from_parts(1_000, 0));
        assert_eq!(hlc.last(), got);
    }

    #[test]
    fn resume_caps_self_max_at_wall_plus_60s() {
        let far = HlcStamp::from_parts(1_000 + 600_000, 5);
        assert_eq!(
            Hlc::resume(w(1_000), Some(far)).last(),
            HlcStamp::from_parts(1_000 + 60_000, 0)
        );
        let near = HlcStamp::from_parts(1_000 + 30_000, 5);
        assert_eq!(Hlc::resume(w(1_000), Some(near)).last(), near);
        let old = HlcStamp::from_parts(10, 5);
        assert_eq!(
            Hlc::resume(w(1_000), Some(old)).last(),
            HlcStamp::from_parts(1_000, 0)
        );
    }

    #[test]
    fn resume_without_self_max_is_wall() {
        assert_eq!(
            Hlc::resume(w(1_000), None).last(),
            HlcStamp::from_parts(1_000, 0)
        );
    }

    #[test]
    fn wall_clamped_to_48_bits() {
        let mut hlc = Hlc::new();
        let max_ms = (1u64 << 48) - 1;
        assert_eq!(hlc.tick(w(i64::MAX)), HlcStamp::from_parts(max_ms, 0));
        assert_eq!(HlcStamp::from_parts(u64::MAX, 0).ms(), max_ms);
        let mut hlc = Hlc::new();
        assert_eq!(hlc.tick(w(-5)), HlcStamp::from_parts(0, 1));
        assert_eq!(
            Hlc::resume(w(i64::MAX), None).last(),
            HlcStamp::from_parts(max_ms, 0)
        );
    }

    #[test]
    fn parts_round_trip() {
        let s = HlcStamp::from_parts(123_456, 0xABCD);
        assert_eq!((s.ms(), s.logical()), (123_456, 0xABCD));
        assert_eq!(HlcStamp::from_u64(s.as_u64()), s);
    }
}
