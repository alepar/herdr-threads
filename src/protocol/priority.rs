//! The delivery-priority value types of the message-priority feature
//! (`docs/superpowers/runs/2026-10-13-message-priority-delivery/`).
//!
//! Naming rule: this is the *delivery* priority, a sender-chosen notification urgency (0 most
//! urgent). The person-authored "priority" of summaries and catch-up (`is_priority`) is a separate
//! axis; where the two meet, code says `delivery_priority`.

use crate::protocol::commands::{DeliveryMode, WakeOn};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::UtcMillis;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// A message's immutable notification priority, 0 (most urgent) ..= 9.
/// Owns: the range, the default (1), the fixed p0 for invitations, ACK requests and actionable warnings,
/// the legacy mapping both ways (`ordinary` <-> 0, `lazy` <-> 1; p >= 1 -> `lazy`, lossy when p > 1),
/// and the `[p<N>]` marker (only when N != 1).
/// Does not: decide wake (NudgeThreshold does) or create receipts/deadlines (send preparation does,
/// for explicit ACK recipients only).
/// New legacy spellings are added here only; callers never match on `DeliveryMode` to decide anything.
/// Not the person-authored summary/catch-up priority (`is_priority`); callers there say `delivery_priority`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct DeliveryPriority(u8);

/// A priority rendered as an old daemon's two-valued mode. An old daemon only knows two modes;
/// `lossy` tells the client to say so once on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMode {
    pub mode: DeliveryMode,
    pub lossy: bool,
}

const MARKERS: [&str; 10] = [
    "[p0]", "", "[p2]", "[p3]", "[p4]", "[p5]", "[p6]", "[p7]", "[p8]", "[p9]",
];

impl DeliveryPriority {
    pub const MAX: u8 = 9;
    pub const DEFAULT: Self = Self(1);

    pub const fn p0() -> Self {
        Self(0)
    }

    pub fn new(value: u8) -> Result<Self, &'static str> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err("delivery priority must be 0..=9")
        }
    }

    /// ASCII digits only, then [`Self::new`].
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err("delivery priority must be 0..=9");
        }
        let value: u8 = text
            .parse()
            .map_err(|_| "delivery priority must be 0..=9")?;
        Self::new(value)
    }

    pub fn from_stored(value: i64) -> Result<Self, ApiError> {
        u8::try_from(value)
            .ok()
            .and_then(|v| Self::new(v).ok())
            .ok_or_else(|| {
                ApiError::new(ErrorCode::StoreCorrupt, "invalid stored delivery priority")
            })
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub fn from_mode(mode: DeliveryMode) -> Self {
        match mode {
            DeliveryMode::Ordinary => Self(0),
            DeliveryMode::Lazy => Self::DEFAULT,
        }
    }

    pub fn to_mode(self) -> LegacyMode {
        match self.0 {
            0 => LegacyMode {
                mode: DeliveryMode::Ordinary,
                lossy: false,
            },
            1 => LegacyMode {
                mode: DeliveryMode::Lazy,
                lossy: false,
            },
            _ => LegacyMode {
                mode: DeliveryMode::Lazy,
                lossy: true,
            },
        }
    }

    /// `Some("[pN]")` (4 bytes) iff N != 1.
    pub fn marker(self) -> Option<&'static str> {
        let text = MARKERS[usize::from(self.0)];
        (!text.is_empty()).then_some(text)
    }

    pub fn is_default(&self) -> bool {
        *self == Self::DEFAULT
    }
}

impl Default for DeliveryPriority {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<u8> for DeliveryPriority {
    type Error = &'static str;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<DeliveryPriority> for u8 {
    fn from(value: DeliveryPriority) -> Self {
        value.0
    }
}

/// The per-seat nudge threshold T (-1..=9, default 0): an idle seat wakes on a pending item iff its
/// priority <= T; a negative T (stored as -1) admits every item.
/// Owns: the single wake predicate `admits`, clamping negative input to -1, and the `wake-on` alias mapping
/// (`all` <-> -1, `nudge` <-> 0; other values are not expressible to an old daemon).
/// Does not: get cached or frozen at send; every wake decision reads the seat's current value.
/// Per-thread overrides (out of scope) go into the code that looks T up, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct NudgeThreshold(i8);

impl NudgeThreshold {
    pub const ALL: Self = Self(-1);
    pub const DEFAULT: Self = Self(0);
    pub const MAX: i8 = 9;

    /// Any negative value clamps to -1; above 9 is an error.
    pub fn new(value: i64) -> Result<Self, &'static str> {
        if value < 0 {
            Ok(Self::ALL)
        } else if value > i64::from(Self::MAX) {
            Err("nudge threshold must be <= 9")
        } else {
            Ok(Self(value as i8))
        }
    }

    /// Optional leading `-`, then ASCII digits.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err("nudge threshold must be an integer");
        }
        let magnitude = digits.parse::<i64>().unwrap_or(i64::MAX);
        Self::new(if negative { -magnitude } else { magnitude })
    }

    /// Exactly -1..=9.
    pub fn from_stored(value: i64) -> Result<Self, ApiError> {
        if (-1..=i64::from(Self::MAX)).contains(&value) {
            Ok(Self(value as i8))
        } else {
            Err(ApiError::new(
                ErrorCode::StoreCorrupt,
                "invalid stored nudge threshold",
            ))
        }
    }

    pub const fn get(self) -> i8 {
        self.0
    }

    /// The one wake predicate: `priority <= T`, or any priority when T is negative.
    pub fn admits(self, priority: DeliveryPriority) -> bool {
        self.0 < 0 || priority.get() as i8 <= self.0
    }

    pub fn from_wake_on(value: WakeOn) -> Self {
        match value {
            WakeOn::All => Self::ALL,
            WakeOn::Nudge => Self::DEFAULT,
        }
    }

    /// Exact alias, for the new-client/old-daemon fallback; `None` when T is not expressible.
    pub fn to_wake_on(self) -> Option<WakeOn> {
        match self.0 {
            -1 => Some(WakeOn::All),
            0 => Some(WakeOn::Nudge),
            _ => None,
        }
    }

    /// Lossy report for an old client's `WakeOnQuery`: `All` iff T < 0.
    pub fn legacy_wake_on(self) -> WakeOn {
        if self.0 < 0 {
            WakeOn::All
        } else {
            WakeOn::Nudge
        }
    }
}

impl Default for NudgeThreshold {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl TryFrom<i64> for NudgeThreshold {
    type Error = &'static str;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<NudgeThreshold> for i64 {
    fn from(value: NudgeThreshold) -> Self {
        i64::from(value.0)
    }
}

/// Fixed tiebreak between pending-item sources. A new pending-item source gets a rank here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceRank {
    Invitation = 0,
    Warning = 1,
    /// Explicit-ACK receipts and pre-upgrade binding-unread ordinary rows.
    Receipt = 2,
    DeliveryRow = 3,
}

impl SourceRank {
    fn from_bits(bits: u8) -> Self {
        match bits & 3 {
            0 => Self::Invitation,
            1 => Self::Warning,
            2 => Self::Receipt,
            _ => Self::DeliveryRow,
        }
    }
}

/// The total presentation order of one pending item: priority asc, deadline asc with none last,
/// publication `decision_seq` asc, source rank, per-source ordinal.
/// Owns: the tuple, the "no deadline last" rule, the source-rank table and the continuation encoding.
/// Does not: run queries; each source's indexed seek yields rows in key order, the key only compares heads.
/// Callers never sort pending items by individual fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryOrderKey {
    pub priority: DeliveryPriority,
    pub deadline: Option<UtcMillis>,
    pub decision_seq: u64,
    pub rank: SourceRank,
    pub ordinal: u64,
}

impl Ord for DeliveryOrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| match (self.deadline, other.deadline) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            })
            .then_with(|| self.decision_seq.cmp(&other.decision_seq))
            .then_with(|| self.rank.cmp(&other.rank))
            .then_with(|| self.ordinal.cmp(&other.ordinal))
    }
}

impl PartialOrd for DeliveryOrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

const MALFORMED: &str = "malformed delivery order key";

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(input: &mut &[u8]) -> Result<u64, &'static str> {
    let mut value = 0u64;
    for index in 0..10 {
        let (&byte, rest) = input.split_first().ok_or(MALFORMED)?;
        *input = rest;
        let bits = u64::from(byte & 0x7f);
        if index == 9 && bits > 1 {
            return Err(MALFORMED);
        }
        value |= bits << (7 * index);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(MALFORMED)
}

impl DeliveryOrderKey {
    /// Header byte: priority (bits 0-3) | rank (bits 4-5) | deadline-present (bit 6); bit 7 is 0.
    /// Then the deadline (zigzag LEB128, if present), `decision_seq` and `ordinal` (LEB128).
    pub fn write_compact(&self, out: &mut Vec<u8>) {
        let header = self.priority.get()
            | ((self.rank as u8) << 4)
            | (u8::from(self.deadline.is_some()) << 6);
        out.push(header);
        if let Some(UtcMillis(v)) = self.deadline {
            write_varint(out, ((v << 1) ^ (v >> 63)) as u64);
        }
        write_varint(out, self.decision_seq);
        write_varint(out, self.ordinal);
    }

    /// Parses one key and advances `input` past exactly the consumed bytes; rejects noncanonical
    /// encodings.
    pub fn read_compact(input: &mut &[u8]) -> Result<Self, &'static str> {
        let start = *input;
        let (&header, rest) = input.split_first().ok_or(MALFORMED)?;
        *input = rest;
        if header & 0x80 != 0 {
            return Err(MALFORMED);
        }
        let priority = DeliveryPriority::new(header & 0x0f).map_err(|_| MALFORMED)?;
        let rank = SourceRank::from_bits(header >> 4);
        let deadline = if header & 0x40 != 0 {
            let z = read_varint(input)?;
            Some(UtcMillis(((z >> 1) as i64) ^ -((z & 1) as i64)))
        } else {
            None
        };
        let decision_seq = read_varint(input)?;
        let ordinal = read_varint(input)?;
        let key = Self {
            priority,
            deadline,
            decision_seq,
            rank,
            ordinal,
        };
        let consumed = &start[..start.len() - input.len()];
        let mut canonical = Vec::with_capacity(consumed.len());
        key.write_compact(&mut canonical);
        if canonical != consumed {
            return Err(MALFORMED);
        }
        Ok(key)
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/priority.rs"]
mod tests;
