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
/// A token carrying a lazy key: `v2.INV.REC.WARN.LAZY.EPISODE`. A token
/// without one keeps the v1 spelling, so existing marks and readers are
/// unchanged until lazy rows are requested and pending.
const TOKEN_PREFIX_LAZY: &str = "v2";
/// A per-binding token (ht-54r.9): `v3.INV.REC.WARN.LAZY.MAIL.EPISODE.BINDING`. It carries the
/// reading binding's new-mail key and the binding itself, so a token read for one binding is
/// never compared with another binding's mark. Tokens of a seat-level read keep the v1/v2 spelling.
const TOKEN_PREFIX_BINDING: &str = "v3";

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
    /// Newest pending published lazy row addressed to the seat: (publication
    /// decision sequence, recipient ordinal). Only in digests that asked for
    /// lazy rows. It advances the hook's change marker, never the wake
    /// frontier: lazy mail is presented at the next hook, never nudged.
    pub lazy: Option<PublicationKey>,
    /// Newest unread ordinary message past the reading binding's own cursor: (publication
    /// decision sequence, recipient ordinal). Only in a binding's digest. Presentation-only
    /// like `lazy`: it moves the hook's change marker, never the wake frontier.
    pub mail: Option<PublicationKey>,
    /// The binding this token was read for (an opaque stable hash of its binding key), absent in
    /// a seat-level read.
    pub binding: Option<u64>,
}

/// The stable opaque form of a binding key carried by [`AttentionToken::binding`] (FNV-1a over
/// seat, harness, native session and agent id; an empty agent id is a top-level session). It is
/// derived from the key, not the binding row, so a digest read before the session's binding
/// exists carries the token its later digests will.
pub fn binding_token_hash(seat: &str, harness: &str, native_session: &str, agent_id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [seat, harness, native_session, agent_id] {
        for byte in (part.len() as u64)
            .to_be_bytes()
            .into_iter()
            .chain(part.bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
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
        // A mark read for another binding covers nothing of this one: compare with an empty mark.
        let empty = Self::default();
        let mark = if self.binding == mark.binding {
            mark
        } else {
            &empty
        };
        self.invitation > mark.invitation
            || self.receipt > mark.receipt
            || self.warning > mark.warning
            || self.lazy > mark.lazy
            || self.mail > mark.mail
    }

    /// Component-wise maximum. A mark only ever grows, so an item leaving and
    /// lowering the current maximum can never make an older key look new.
    pub fn join(&self, other: &Self) -> Self {
        // Another binding's frontier is never merged in: the newer read replaces it.
        if self.binding != other.binding {
            return *other;
        }
        Self {
            invitation: self.invitation.max(other.invitation),
            receipt: self.receipt.max(other.receipt),
            warning: self.warning.max(other.warning),
            unavailability_episode: self
                .unavailability_episode
                .max(other.unavailability_episode),
            lazy: self.lazy.max(other.lazy),
            mail: self.mail.max(other.mail),
            binding: self.binding,
        }
    }

    pub fn encode(&self) -> String {
        let key = |key: Option<PublicationKey>| match key {
            Some((seq, offset)) => format!("{seq}-{offset}"),
            None => "_".to_owned(),
        };
        if self.mail.is_some() || self.binding.is_some() {
            return format!(
                "{TOKEN_PREFIX_BINDING}.{}.{}.{}.{}.{}.{}.{}",
                key(self.invitation),
                key(self.receipt),
                key(self.warning),
                key(self.lazy),
                key(self.mail),
                self.unavailability_episode,
                self.binding
                    .map_or_else(|| "_".to_owned(), |b| format!("{b:016x}"))
            );
        }
        match self.lazy {
            None => format!(
                "{TOKEN_PREFIX}.{}.{}.{}.{}",
                key(self.invitation),
                key(self.receipt),
                key(self.warning),
                self.unavailability_episode
            ),
            lazy => format!(
                "{TOKEN_PREFIX_LAZY}.{}.{}.{}.{}.{}",
                key(self.invitation),
                key(self.receipt),
                key(self.warning),
                key(lazy),
                self.unavailability_episode
            ),
        }
    }

    pub fn decode(raw: &str) -> Result<Self, &'static str> {
        if raw.len() > 320 {
            return Err("attention token too long");
        }
        let parts: Vec<&str> = raw.split('.').collect();
        let (invitation, receipt, warning, lazy, mail, episode, binding) = match parts[..] {
            [TOKEN_PREFIX, invitation, receipt, warning, episode] => {
                (invitation, receipt, warning, None, None, episode, None)
            }
            [
                TOKEN_PREFIX_LAZY,
                invitation,
                receipt,
                warning,
                lazy,
                episode,
            ] => (
                invitation,
                receipt,
                warning,
                Some(lazy),
                None,
                episode,
                None,
            ),
            [
                TOKEN_PREFIX_BINDING,
                invitation,
                receipt,
                warning,
                lazy,
                mail,
                episode,
                binding,
            ] => (
                invitation,
                receipt,
                warning,
                Some(lazy),
                Some(mail),
                episode,
                Some(binding),
            ),
            [prefix, ..]
                if prefix != TOKEN_PREFIX
                    && prefix != TOKEN_PREFIX_LAZY
                    && prefix != TOKEN_PREFIX_BINDING =>
            {
                return Err("unsupported attention token version");
            }
            _ => return Err("malformed attention token"),
        };
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
        let binding = match binding {
            None | Some("_") => None,
            Some(hex) if hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Some(u64::from_str_radix(hex, 16).map_err(|_| "malformed attention binding")?)
            }
            Some(_) => return Err("malformed attention binding"),
        };
        let token = Self {
            invitation: key(invitation)?,
            receipt: key(receipt)?,
            warning: key(warning)?,
            unavailability_episode: number(episode)?,
            lazy: lazy.map(key).transpose()?.flatten(),
            mail: mail.map(key).transpose()?.flatten(),
            binding,
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
    /// Pending lazy rows addressed to the seat, only when the query asked for
    /// them (`AttentionDigestQuery.lazy`). Presentation-only attention: it
    /// changes the hook's marker and summary, never wakes or nudges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lazy: Option<AttentionClass>,
    /// Unread ordinary mail past the reading binding's own cursor (ht-54r.9), only in a
    /// binding's digest. Each binding of a seat gets its own notice for one new message, and a
    /// display by one never clears the other's. Presentation-only, like `lazy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<AttentionClass>,
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
        if self.token.lazy.is_some() && self.lazy.as_ref().is_none_or(|lazy| lazy.count == 0) {
            return Err("lazy attention key without pending lazy rows");
        }
        if self.token.mail.is_some() && self.mail.as_ref().is_none_or(|mail| mail.count == 0) {
            return Err("mail attention key without unread mail");
        }
        for class in [&self.invitations, &self.receipts, &self.warnings]
            .into_iter()
            .chain(&self.lazy)
            .chain(&self.mail)
        {
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
        self.invitations.count == 0
            && self.receipts.count == 0
            && self.warnings.count == 0
            && self.lazy.as_ref().is_none_or(|lazy| lazy.count == 0)
            && self.mail.as_ref().is_none_or(|mail| mail.count == 0)
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
        let lazy = self
            .lazy
            .as_ref()
            .filter(|lazy| lazy.count > 0)
            .map_or_else(String::new, |lazy| format!("; {}", class("lazy", lazy)));
        let mail = self
            .mail
            .as_ref()
            .filter(|mail| mail.count > 0)
            .map_or_else(String::new, |mail| format!("; {}", class("mail", mail)));
        format!(
            "attention digest: {}; {}; {}{lazy}{mail}",
            class("invitations", &self.invitations),
            class("receipts", &self.receipts),
            class("warnings", &self.warnings)
        )
    }
}

#[cfg(test)]
#[path = "../../tests/protocol/attention.rs"]
mod tests;
