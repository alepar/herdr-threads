//! Seat attention digest: a read-only, seat-scoped, wire-versioned summary of
//! everything that asks the seat for attention (root adoption, wave-1 fix2 (a)).
//!
//! The token is built from the store's canonical logical publication keys (the
//! same `LogicalAttentionFrontier` the wake scheduler reserves against) plus
//! the seat's unavailability episode (carried, but never itself new
//! attention for the seat's own occupant). Publication keys are allocated from the
//! instance's monotone decision sequence, so every new invitation, addressed
//! receipt or actionable warning has a key strictly greater than every earlier
//! one: "something new arrived" is exactly "some component advanced beyond the
//! mark". Items only leaving (an ACK, an accepted invitation) never advance it.
use crate::protocol::ids::{SeatId, ThreadId};
use serde::{Deserialize, Serialize};

/// Digest wire version. A reader rejects any other version.
pub const DIGEST_VERSION: u32 = 1;
/// At most this many exact IDs per class; `has_more` reports the rest.
pub const MAX_DIGEST_IDS: usize = 4;
const TOKEN_PREFIX: &str = "v1";

/// One logical publication key: (decision sequence, event offset).
pub type PublicationKey = (u64, u64);

/// Opaque on the wire (a versioned string); typed for comparison.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct AttentionToken {
    pub invitation: Option<PublicationKey>,
    pub receipt: Option<PublicationKey>,
    pub warning: Option<PublicationKey>,
    pub unavailability_episode: u64,
}

impl AttentionToken {
    /// True when a publication component is strictly greater than the
    /// mark's: a new invitation, addressed receipt or actionable warning since
    /// the mark. The seat's *own* unavailability episode is carried (and
    /// joined) but is not attention for the seat's own model: the model that
    /// reads this token is the occupant, and its own outage is not mail
    /// (native-claude-demo-1 P3). Other recipients' unavailability still
    /// reaches the sender through the actionable `warning` component.
    pub fn advanced_beyond(&self, mark: &Self) -> bool {
        self.invitation > mark.invitation
            || self.receipt > mark.receipt
            || self.warning > mark.warning
    }

    /// Component-wise maximum. A mark only ever grows, so an item leaving and
    /// lowering the current maximum can never make an older key look new.
    pub fn join(&self, other: &Self) -> Self {
        Self {
            invitation: self.invitation.max(other.invitation),
            receipt: self.receipt.max(other.receipt),
            warning: self.warning.max(other.warning),
            unavailability_episode: self
                .unavailability_episode
                .max(other.unavailability_episode),
        }
    }

    pub fn encode(&self) -> String {
        let key = |key: Option<PublicationKey>| match key {
            Some((seq, offset)) => format!("{seq}-{offset}"),
            None => "_".to_owned(),
        };
        format!(
            "{TOKEN_PREFIX}.{}.{}.{}.{}",
            key(self.invitation),
            key(self.receipt),
            key(self.warning),
            self.unavailability_episode
        )
    }

    pub fn decode(raw: &str) -> Result<Self, &'static str> {
        if raw.len() > 256 {
            return Err("attention token too long");
        }
        let parts: Vec<&str> = raw.split('.').collect();
        let [prefix, invitation, receipt, warning, episode] = parts[..] else {
            return Err("malformed attention token");
        };
        if prefix != TOKEN_PREFIX {
            return Err("unsupported attention token version");
        }
        let number = |text: &str| -> Result<u64, &'static str> {
            if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err("malformed attention token number");
            }
            text.parse().map_err(|_| "malformed attention token number")
        };
        let key = |text: &str| -> Result<Option<PublicationKey>, &'static str> {
            if text == "_" {
                return Ok(None);
            }
            let (seq, offset) = text.split_once('-').ok_or("malformed attention key")?;
            Ok(Some((number(seq)?, number(offset)?)))
        };
        let token = Self {
            invitation: key(invitation)?,
            receipt: key(receipt)?,
            warning: key(warning)?,
            unavailability_episode: number(episode)?,
        };
        if token.encode() != raw {
            return Err("non-canonical attention token");
        }
        Ok(token)
    }
}

impl From<AttentionToken> for String {
    fn from(token: AttentionToken) -> Self {
        token.encode()
    }
}
impl TryFrom<String> for AttentionToken {
    type Error = &'static str;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Self::decode(&raw)
    }
}

/// One exact attention item: its identifier and the thread holding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionRef {
    pub id: String,
    pub thread: ThreadId,
    /// Invitations only: the pending required membership this invitation
    /// carries, so the hook can emit the exact `accept-required` argv instead
    /// of a plain `accept` that the service would refuse as stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<AttentionRequirement>,
}

/// The current pending requirement episode behind a listed invitation: its
/// service-generated ID and the revision `accept-required` must name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionRequirement {
    pub id: String,
    pub revision: u64,
}

/// One attention class: the pending count, at most `MAX_DIGEST_IDS` newest
/// items (invitations: those in listed receipt threads first, then the
/// newest; native-codex-matrix-2 P6), and whether more exist than are listed. The count comes from
/// bounded walks: it saturates at the store's attention count cap, and
/// `count_has_more` then says more may be pending than counted (digest fix5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionClass {
    pub count: u64,
    pub items: Vec<AttentionRef>,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub count_has_more: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttentionDigest {
    pub version: u32,
    pub seat: SeatId,
    pub token: AttentionToken,
    /// Pending invitations (a pending required membership rides its invitation).
    pub invitations: AttentionClass,
    /// Pending receipts addressed to the seat.
    pub receipts: AttentionClass,
    /// Actionable warnings the seat receives.
    pub warnings: AttentionClass,
    pub unavailability_open: bool,
    /// Spec D7: a mod delivery channel is live for this seat (grace included); the
    /// Claude hooks then omit the attention digest and ready commands. Set by the
    /// domain service, never by the store; absent on the wire when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub mod_channel_live: bool,
}

impl AttentionDigest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != DIGEST_VERSION {
            return Err("unsupported attention digest version");
        }
        for class in [&self.invitations, &self.receipts, &self.warnings] {
            if class.items.len() > MAX_DIGEST_IDS
                || class.items.len() as u64 > class.count
                || class.has_more != (class.count > class.items.len() as u64)
            {
                return Err("invalid attention class bound");
            }
        }
        Ok(())
    }

    /// Nothing asks for attention now.
    pub fn is_empty(&self) -> bool {
        self.invitations.count == 0 && self.receipts.count == 0 && self.warnings.count == 0
    }

    /// Compact fixed-shape summary (service-generated identifiers only).
    pub fn summary(&self) -> String {
        let class = |name: &str, class: &AttentionClass| {
            let items: Vec<String> = class
                .items
                .iter()
                .map(|item| format!("{}@{}", item.id, item.thread.as_str()))
                .collect();
            format!(
                "{name}={}{}{}{}",
                class.count,
                if class.count_has_more { "+" } else { "" },
                if items.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", items.join(", "))
                },
                if class.has_more { " +more" } else { "" }
            )
        };
        format!(
            "attention digest: {}; {}; {}",
            class("invitations", &self.invitations),
            class("receipts", &self.receipts),
            class("warnings", &self.warnings)
        )
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/attention.rs"]
mod tests;
