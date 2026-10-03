use serde::{Deserialize, Serialize};

pub const DEFAULT_PAGE_LIMIT: u16 = 20;
pub const MAX_PAGE_LIMIT: u16 = 100;
pub const DEFAULT_PAGE_BYTES: u32 = 16_384;
pub const MAX_PAGE_BYTES: u32 = 65_536;
pub const MAX_CURSOR_BYTES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub cursor: Option<String>,
    pub limit: u16,
    pub max_bytes: u32,
}
impl Default for PageRequest {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: DEFAULT_PAGE_LIMIT,
            max_bytes: DEFAULT_PAGE_BYTES,
        }
    }
}
impl PageRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.limit == 0 || self.limit > MAX_PAGE_LIMIT {
            return Err("invalid page limit");
        }
        if self.max_bytes < 256 || self.max_bytes > MAX_PAGE_BYTES {
            return Err("invalid page byte bound");
        }
        if let Some(cursor) = &self.cursor {
            Cursor::decode(cursor)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Continuation {
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub next_argv: Option<Vec<String>>,
    pub high_water_ordinal: u64,
    pub scope_revision: Option<u64>,
    pub has_more: bool,
    pub stop_reason: StopReason,
    pub consistency: Consistency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Complete,
    Rows,
    Bytes,
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Consistency {
    BoundedLive,
}

impl<T> Page<T> {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.has_more {
            if self.next_cursor.as_deref().is_none_or(str::is_empty)
                || self.next_argv.as_ref().is_none_or(Vec::is_empty)
                || self.stop_reason == StopReason::Complete
            {
                return Err("incomplete page has no reachable continuation");
            }
        } else if self.stop_reason != StopReason::Complete {
            return Err("complete page has incomplete stop reason");
        }
        Ok(())
    }
}

/// An opaque continuation is an encoded, bounded typed query position.
///
/// ht-4is.8.18: cursors are stateless `c3:` tokens (see [`Cursor::encode`]),
/// not server-side handles. Every pagination rule is still enforced from the
/// token alone: scope, direction and order version travel in the compact
/// payload, the high-water ordinal and every typed position field travel as
/// varints, and the instance, scope key and filter digest are bound by a
/// truncated SHA-256 tag over all of it. A server-side cursor table would
/// turn every read into a write, need TTL expiry and GC, and break
/// continuations across a daemon restart; the stateless token keeps reads
/// read-only and continuations valid for as long as the position is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub instance: String,
    pub scope: CursorScope,
    pub scope_key: String,
    pub filter_digest: String,
    pub direction: CursorDirection,
    pub order_version: u16,
    pub last_examined_key: Option<String>,
    pub after_ordinal: u64,
    pub high_water_ordinal: u64,
    pub scope_revision: Option<u64>,
    pub filter_revision: Option<u64>,
    pub search: Option<SearchCursorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox: Option<InboxCursorState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<SeatAttentionCursorState>,
    /// Set only on a `c3:` cursor fresh from [`Cursor::decode`]; never on the
    /// wire. Construct cursors with `binding: None`.
    #[serde(skip)]
    pub binding: Option<CursorBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxCursorState {
    #[serde(rename = "r")]
    pub seat_revision: u64,
    #[serde(rename = "l")]
    pub lifecycle_revision: u64,
    #[serde(rename = "b")]
    pub baseline_decision_seq: u64,
    #[serde(rename = "m")]
    pub manifest_after_seq: u64,
    #[serde(rename = "w")]
    pub warning_after_seq: u64,
    #[serde(rename = "o")]
    pub warning_after_offset: i64,
}

/// Exact bounded wake-attention continuation. Short wire keys keep the whole
/// typed cursor under MAX_CURSOR_BYTES even with large SQLite ordinals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeatAttentionCursorState {
    #[serde(rename = "ia")]
    pub invitation_after_seq: i64,
    #[serde(rename = "io")]
    pub invitation_after_ordinal: i64,
    #[serde(rename = "id")]
    pub invitations_done: bool,
    #[serde(rename = "ip")]
    pub has_pending_invitation: bool,
    #[serde(rename = "if")]
    pub invitation_frontier: Option<(i64, i64)>,
    #[serde(rename = "r")]
    pub receipts: Option<ReceiptAttentionCursorState>,
    #[serde(rename = "d")]
    pub receipts_done: bool,
    #[serde(rename = "p")]
    pub has_pending_receipt: bool,
    #[serde(rename = "rf")]
    pub receipt_frontier_seq: Option<i64>,
    #[serde(rename = "pa")]
    pub physical_warning_after: i64,
    #[serde(rename = "ph")]
    pub physical_warning_high_water: i64,
    #[serde(rename = "ma")]
    pub manifest_warning_after: i64,
    #[serde(rename = "mh")]
    pub manifest_warning_high_water: i64,
    #[serde(rename = "n")]
    pub next_manifest_warning: bool,
    #[serde(rename = "w")]
    pub latest_warning_seq: Option<i64>,
    #[serde(rename = "wo")]
    pub latest_warning_offset: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptAttentionCursorState {
    #[serde(rename = "pa")]
    pub physical_after: i64,
    #[serde(rename = "ma")]
    pub manifest_after: i64,
    #[serde(rename = "ph")]
    pub physical_high_water: i64,
    #[serde(rename = "mh")]
    pub manifest_high_water: i64,
    #[serde(rename = "n")]
    pub next_manifest: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPhase {
    Topic,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchCursorState {
    pub phase: SearchPhase,
    pub topic_high_water: u64,
    /// In SearchCandidates order_version 2 this is the captured instance
    /// decision-sequence high water, not a physical message ordinal.
    pub body_high_water: u64,
    pub topic_revision: u64,
    pub last_decision_seq: Option<u64>,
    pub last_event_offset: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorScope {
    Directory,
    Seats,
    SeatInspect,
    DeliveryInspect,
    Inbox,
    InboxBatch,
    ActiveWarnings,
    Warnings,
    History,
    MessageBody,
    Diagnostics,
    Participants,
    ServiceMembership,
    Recipients,
    WarningRecipients,
    PendingReceipts,
    LocalIntents,
    RetirementJobs,
    WakeCandidates,
    WakeRecovery,
    WorkJobs,
    SearchCandidates,
}

impl Cursor {
    /// Encode as a short `c3:` token: a compact binary position plus a
    /// truncated SHA-256 tag binding it to its instance, scope key and filter
    /// digest (which are not carried). See [`CursorBinding`].
    pub fn encode(&self) -> Result<String, &'static str> {
        if self.binding.is_some()
            || self.instance.is_empty()
            || self.scope_key.is_empty()
            || self.filter_digest.is_empty()
        {
            return Err("cursor binding unavailable");
        }
        let mut bytes = self.compact_payload()?;
        bytes.extend_from_slice(&binding_tag(
            &self.instance,
            &self.scope_key,
            &self.filter_digest,
            &bytes,
        ));
        let mut encoded = String::from(COMPACT_PREFIX);
        encoded.push_str(&base64url_encode(&bytes));
        if encoded.len() > MAX_CURSOR_BYTES {
            return Err("cursor too large");
        }
        Ok(encoded)
    }

    /// The legacy `c2:` form (base64url JSON). Still accepted by [`decode`]
    /// for one release; new cursors are always [`encode`]d as `c3:`.
    ///
    /// [`decode`]: Self::decode
    /// [`encode`]: Self::encode
    pub fn encode_legacy(&self) -> Result<String, &'static str> {
        if self.binding.is_some() {
            return Err("cursor binding unavailable");
        }
        let json = serde_json::to_vec(self).map_err(|_| "cursor encoding failed")?;
        let mut encoded = String::from(LEGACY_PREFIX);
        encoded.push_str(&base64url_encode(&json));
        if encoded.len() > MAX_CURSOR_BYTES {
            return Err("cursor too large");
        }
        Ok(encoded)
    }

    /// Structurally decode either form. A `c3:` cursor comes back with empty
    /// instance, scope key and filter digest and its [`CursorBinding`] set:
    /// only [`validate_for`](Self::validate_for) (or
    /// [`decode_for`](Self::decode_for)) can check what it is bound to.
    pub fn decode(encoded: &str) -> Result<Self, &'static str> {
        if encoded.len() > MAX_CURSOR_BYTES {
            return Err("cursor too large");
        }
        let cursor = if let Some(payload) = encoded.strip_prefix(COMPACT_PREFIX) {
            if payload.is_empty() {
                return Err("malformed cursor");
            }
            Self::decode_compact(&base64url_decode(payload)?)?
        } else {
            let payload = encoded
                .strip_prefix(LEGACY_PREFIX)
                .ok_or("invalid cursor prefix")?;
            if payload.is_empty() {
                return Err("malformed cursor");
            }
            let bytes = base64url_decode(payload)?;
            let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| "malformed cursor")?;
            if cursor.instance.is_empty()
                || cursor.instance.len() > 128
                || cursor.scope_key.is_empty()
                || cursor.scope_key.len() > 128
                || cursor.filter_digest.is_empty()
                || cursor.filter_digest.len() > 128
            {
                return Err("invalid cursor position");
            }
            cursor
        };
        cursor.check_position()?;
        Ok(cursor)
    }

    /// [`decode`](Self::decode), [`validate_for`](Self::validate_for), and
    /// restore the binding strings, so the result equals the cursor that was
    /// encoded.
    pub fn decode_for(
        encoded: &str,
        instance: &str,
        scope: CursorScope,
        scope_key: &str,
        filter_digest: &str,
        direction: CursorDirection,
        order_version: u16,
    ) -> Result<Self, &'static str> {
        let mut cursor = Self::decode(encoded)?;
        cursor.validate_for(
            instance,
            scope,
            scope_key,
            filter_digest,
            direction,
            order_version,
        )?;
        cursor.instance = instance.to_owned();
        cursor.scope_key = scope_key.to_owned();
        cursor.filter_digest = filter_digest.to_owned();
        cursor.binding = None;
        Ok(cursor)
    }

    /// Whether this cursor names `scope_key`, when that is knowable without
    /// the instance and filter digest: a `c3:` cursor's binding is checked by
    /// [`validate_for`](Self::validate_for) at the store instead.
    pub fn may_name_scope_key(&self, scope_key: &str) -> bool {
        match &self.binding {
            Some(binding) => binding
                .key_check
                .is_none_or(|check| check == key_check(scope_key)),
            None => self.scope_key == scope_key,
        }
    }

    fn check_position(&self) -> Result<(), &'static str> {
        let cursor = self;
        if !(cursor.order_version == 1
            || (cursor.order_version == 2
                && cursor.scope == CursorScope::SearchCandidates
                && cursor.search.is_some()))
            || cursor
                .last_examined_key
                .as_ref()
                .is_some_and(|key| key.len() > 128)
            || cursor.after_ordinal > cursor.high_water_ordinal
            || cursor.inbox.as_ref().is_some_and(|inbox| {
                cursor.scope != CursorScope::Inbox
                    || cursor.search.is_some()
                    || inbox.manifest_after_seq < inbox.baseline_decision_seq
                    || inbox.warning_after_seq < inbox.baseline_decision_seq
                    || inbox.warning_after_offset < -1
            })
            || cursor.attention.as_ref().is_some_and(|attention| {
                !matches!(
                    cursor.scope,
                    CursorScope::WakeCandidates
                        | CursorScope::InboxBatch
                        | CursorScope::ActiveWarnings
                ) || cursor.search.is_some()
                    || attention.invitation_after_seq < 0
                    || attention.invitation_after_ordinal < 0
                    || (attention.invitation_after_seq == 0)
                        != (attention.invitation_after_ordinal == 0)
                    || attention
                        .invitation_frontier
                        .is_some_and(|(seq, ordinal)| seq <= 0 || ordinal <= 0)
                    || attention.receipt_frontier_seq.is_some_and(|seq| seq <= 0)
                    || attention.latest_warning_seq.is_some_and(|seq| seq <= 0)
                    || attention
                        .latest_warning_offset
                        .is_some_and(|offset| offset < 0)
                    || attention.latest_warning_seq.is_some()
                        != attention.latest_warning_offset.is_some()
                    || attention.physical_warning_after < 0
                    || attention.physical_warning_after > attention.physical_warning_high_water
                    || attention.manifest_warning_after < 0
                    || attention.manifest_warning_after > attention.manifest_warning_high_water
                    || attention.receipts.as_ref().is_some_and(|receipt| {
                        receipt.physical_after < 0
                            || receipt.physical_after > receipt.physical_high_water
                            || receipt.manifest_after < 0
                            || receipt.manifest_after > receipt.manifest_high_water
                    })
            })
        {
            return Err("invalid cursor position");
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        instance: &str,
        scope: CursorScope,
        scope_key: &str,
        filter_digest: &str,
        direction: CursorDirection,
        order_version: u16,
    ) -> Result<(), &'static str> {
        if self.scope != scope || self.direction != direction || self.order_version != order_version
        {
            return Err("invalid cursor scope");
        }
        match &self.binding {
            Some(binding) => {
                let payload = self.compact_payload()?;
                if binding.tag != binding_tag(instance, scope_key, filter_digest, &payload) {
                    return Err("invalid cursor scope");
                }
            }
            None => {
                if self.instance != instance
                    || self.scope_key != scope_key
                    || self.filter_digest != filter_digest
                {
                    return Err("invalid cursor scope");
                }
            }
        }
        Ok(())
    }

    /// Canonical compact position bytes (everything but the binding).
    fn compact_payload(&self) -> Result<Vec<u8>, &'static str> {
        let scope = SCOPE_CODES
            .iter()
            .position(|scope| *scope == self.scope)
            .ok_or("cursor encoding failed")? as u8;
        if self.after_ordinal > self.high_water_ordinal {
            return Err("invalid cursor position");
        }
        let mut flags = 0u8;
        if self.direction == CursorDirection::Descending {
            flags |= F_DESCENDING;
        }
        if self.last_examined_key.is_some() {
            flags |= F_KEY;
        }
        if self.scope_revision.is_some() {
            flags |= F_SCOPE_REVISION;
        }
        if self.filter_revision.is_some() {
            flags |= F_FILTER_REVISION;
        }
        if self.search.is_some() {
            flags |= F_SEARCH;
        }
        if self.inbox.is_some() {
            flags |= F_INBOX;
        }
        if self.attention.is_some() {
            flags |= F_ATTENTION;
        }
        let mut out = Compact(vec![scope, flags]);
        out.unsigned(u64::from(self.order_version));
        out.unsigned(self.after_ordinal);
        out.unsigned(self.high_water_ordinal - self.after_ordinal);
        if let Some(revision) = self.scope_revision {
            out.unsigned(revision);
        }
        if let Some(revision) = self.filter_revision {
            out.unsigned(revision);
        }
        if let Some(key) = &self.last_examined_key {
            if key.len() > 128 {
                return Err("cursor too large");
            }
            out.unsigned(key.len() as u64);
            out.0.extend_from_slice(key.as_bytes());
        }
        if let Some(search) = &self.search {
            let mut sflags = 0u8;
            if search.phase == SearchPhase::Body {
                sflags |= 1;
            }
            if search.last_decision_seq.is_some() {
                sflags |= 2;
            }
            if search.last_event_offset.is_some() {
                sflags |= 4;
            }
            out.0.push(sflags);
            out.unsigned(search.topic_high_water);
            out.unsigned(search.body_high_water);
            out.unsigned(search.topic_revision);
            if let Some(seq) = search.last_decision_seq {
                out.unsigned(seq);
            }
            if let Some(offset) = search.last_event_offset {
                out.unsigned(offset);
            }
        }
        if let Some(inbox) = &self.inbox {
            out.unsigned(inbox.seat_revision);
            out.unsigned(inbox.lifecycle_revision);
            out.unsigned(inbox.baseline_decision_seq);
            out.unsigned(inbox.manifest_after_seq);
            out.unsigned(inbox.warning_after_seq);
            out.signed(inbox.warning_after_offset);
        }
        if self.scope == CursorScope::MessageBody {
            let check = match &self.binding {
                Some(binding) => binding.key_check.ok_or("malformed cursor")?,
                None => key_check(&self.scope_key),
            };
            out.0.extend_from_slice(&check);
        }
        if let Some(a) = &self.attention {
            let bits = [
                a.invitations_done,
                a.has_pending_invitation,
                a.invitation_frontier.is_some(),
                a.receipts.is_some(),
                a.receipts_done,
                a.has_pending_receipt,
                a.receipt_frontier_seq.is_some(),
                a.next_manifest_warning,
                a.latest_warning_seq.is_some(),
                a.latest_warning_offset.is_some(),
                a.receipts.as_ref().is_some_and(|r| r.next_manifest),
            ];
            let packed = bits
                .iter()
                .enumerate()
                .fold(0u64, |acc, (bit, set)| acc | (u64::from(*set) << bit));
            out.unsigned(packed);
            out.signed(a.invitation_after_seq);
            out.signed(a.invitation_after_ordinal);
            if let Some((seq, ordinal)) = a.invitation_frontier {
                out.signed(seq);
                out.signed(ordinal);
            }
            if let Some(r) = &a.receipts {
                out.signed(r.physical_after);
                out.signed(r.manifest_after);
                out.signed(r.physical_high_water);
                out.signed(r.manifest_high_water);
            }
            if let Some(seq) = a.receipt_frontier_seq {
                out.signed(seq);
            }
            out.signed(a.physical_warning_after);
            out.signed(a.physical_warning_high_water);
            out.signed(a.manifest_warning_after);
            out.signed(a.manifest_warning_high_water);
            if let Some(seq) = a.latest_warning_seq {
                out.signed(seq);
            }
            if let Some(offset) = a.latest_warning_offset {
                out.signed(offset);
            }
        }
        Ok(out.0)
    }

    fn decode_compact(bytes: &[u8]) -> Result<Self, &'static str> {
        const MALFORMED: &str = "malformed cursor";
        if bytes.len() < 2 + CURSOR_TAG_BYTES {
            return Err(MALFORMED);
        }
        let (payload, tag) = bytes.split_at(bytes.len() - CURSOR_TAG_BYTES);
        let mut input = Reader(payload);
        let scope = *SCOPE_CODES
            .get(usize::from(input.byte()?))
            .ok_or(MALFORMED)?;
        let flags = input.byte()?;
        if flags & !F_ALL != 0 {
            return Err(MALFORMED);
        }
        let order_version = u16::try_from(input.unsigned()?).map_err(|_| MALFORMED)?;
        let after_ordinal = input.unsigned()?;
        let high_water_ordinal = after_ordinal
            .checked_add(input.unsigned()?)
            .ok_or(MALFORMED)?;
        let scope_revision = (flags & F_SCOPE_REVISION != 0)
            .then(|| input.unsigned())
            .transpose()?;
        let filter_revision = (flags & F_FILTER_REVISION != 0)
            .then(|| input.unsigned())
            .transpose()?;
        let last_examined_key = if flags & F_KEY != 0 {
            let len = usize::try_from(input.unsigned()?).map_err(|_| MALFORMED)?;
            if len > 128 {
                return Err(MALFORMED);
            }
            let raw = input.take(len)?;
            Some(String::from_utf8(raw.to_vec()).map_err(|_| MALFORMED)?)
        } else {
            None
        };
        let search = if flags & F_SEARCH != 0 {
            let sflags = input.byte()?;
            if sflags & !7 != 0 {
                return Err(MALFORMED);
            }
            Some(SearchCursorState {
                phase: if sflags & 1 != 0 {
                    SearchPhase::Body
                } else {
                    SearchPhase::Topic
                },
                topic_high_water: input.unsigned()?,
                body_high_water: input.unsigned()?,
                topic_revision: input.unsigned()?,
                last_decision_seq: (sflags & 2 != 0).then(|| input.unsigned()).transpose()?,
                last_event_offset: (sflags & 4 != 0).then(|| input.unsigned()).transpose()?,
            })
        } else {
            None
        };
        let inbox = if flags & F_INBOX != 0 {
            Some(InboxCursorState {
                seat_revision: input.unsigned()?,
                lifecycle_revision: input.unsigned()?,
                baseline_decision_seq: input.unsigned()?,
                manifest_after_seq: input.unsigned()?,
                warning_after_seq: input.unsigned()?,
                warning_after_offset: input.signed()?,
            })
        } else {
            None
        };
        let key_check = if scope == CursorScope::MessageBody {
            let mut check = [0u8; KEY_CHECK_BYTES];
            check.copy_from_slice(input.take(KEY_CHECK_BYTES)?);
            Some(check)
        } else {
            None
        };
        let attention = if flags & F_ATTENTION != 0 {
            let packed = input.unsigned()?;
            if packed >> 11 != 0 {
                return Err(MALFORMED);
            }
            let bit = |index: u32| packed & (1 << index) != 0;
            let invitation_after_seq = input.signed()?;
            let invitation_after_ordinal = input.signed()?;
            let invitation_frontier = if bit(2) {
                Some((input.signed()?, input.signed()?))
            } else {
                None
            };
            let receipts = if bit(3) {
                Some(ReceiptAttentionCursorState {
                    physical_after: input.signed()?,
                    manifest_after: input.signed()?,
                    physical_high_water: input.signed()?,
                    manifest_high_water: input.signed()?,
                    next_manifest: bit(10),
                })
            } else if bit(10) {
                return Err(MALFORMED);
            } else {
                None
            };
            let receipt_frontier_seq = bit(6).then(|| input.signed()).transpose()?;
            let physical_warning_after = input.signed()?;
            let physical_warning_high_water = input.signed()?;
            let manifest_warning_after = input.signed()?;
            let manifest_warning_high_water = input.signed()?;
            Some(SeatAttentionCursorState {
                invitation_after_seq,
                invitation_after_ordinal,
                invitations_done: bit(0),
                has_pending_invitation: bit(1),
                invitation_frontier,
                receipts,
                receipts_done: bit(4),
                has_pending_receipt: bit(5),
                receipt_frontier_seq,
                physical_warning_after,
                physical_warning_high_water,
                manifest_warning_after,
                manifest_warning_high_water,
                next_manifest_warning: bit(7),
                latest_warning_seq: bit(8).then(|| input.signed()).transpose()?,
                latest_warning_offset: bit(9).then(|| input.signed()).transpose()?,
            })
        } else {
            None
        };
        if !input.0.is_empty() {
            return Err(MALFORMED);
        }
        let mut binding = [0u8; CURSOR_TAG_BYTES];
        binding.copy_from_slice(tag);
        let cursor = Self {
            instance: String::new(),
            scope,
            scope_key: String::new(),
            filter_digest: String::new(),
            direction: if flags & F_DESCENDING != 0 {
                CursorDirection::Descending
            } else {
                CursorDirection::Ascending
            },
            order_version,
            last_examined_key,
            after_ordinal,
            high_water_ordinal,
            scope_revision,
            filter_revision,
            search,
            inbox,
            attention,
            binding: Some(CursorBinding {
                tag: binding,
                key_check,
            }),
        };
        // One position has exactly one encoding, so the tag `validate_for`
        // recomputes over the re-encoded payload is the tag that was sent.
        if cursor.compact_payload()? != payload {
            return Err(MALFORMED);
        }
        Ok(cursor)
    }
}

const LEGACY_PREFIX: &str = "c2:";
const COMPACT_PREFIX: &str = "c3:";
/// Bytes of the truncated SHA-256 binding tag carried by a `c3:` cursor.
pub const CURSOR_TAG_BYTES: usize = 6;

/// The truncated SHA-256 tag of a decoded `c3:` cursor. It binds the compact
/// position to the instance, scope key and filter digest the cursor was
/// issued for, which the token itself does not carry. A cursor replayed
/// against another instance, thread, filter or (since scope, direction and
/// order version are in the hashed payload) another query shape, and a
/// forged or corrupted position, all fail [`Cursor::validate_for`] exactly as
/// a mismatched `c2:` cursor does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorBinding {
    pub tag: [u8; CURSOR_TAG_BYTES],
    /// A message-body cursor also carries a short digest of its message ID,
    /// so the wire request can refuse another message's body cursor before
    /// it reaches the store (as it could with the `c2:` scope key).
    pub key_check: Option<[u8; KEY_CHECK_BYTES]>,
}

/// Bytes of the scope-key digest a `c3:` message-body cursor carries.
pub const KEY_CHECK_BYTES: usize = 3;

fn key_check(scope_key: &str) -> [u8; KEY_CHECK_BYTES] {
    use sha2::{Digest, Sha256};
    let digest = Sha256::new()
        .chain_update(b"herdr-threads cursor key\0")
        .chain_update(scope_key.as_bytes())
        .finalize();
    let mut check = [0u8; KEY_CHECK_BYTES];
    check.copy_from_slice(&digest[..KEY_CHECK_BYTES]);
    check
}

/// Wire codes of `CursorScope` in a `c3:` cursor. Append only: a code is
/// never reused or reordered.
const SCOPE_CODES: [CursorScope; 22] = [
    CursorScope::Directory,
    CursorScope::Seats,
    CursorScope::SeatInspect,
    CursorScope::DeliveryInspect,
    CursorScope::Inbox,
    CursorScope::Warnings,
    CursorScope::History,
    CursorScope::MessageBody,
    CursorScope::Diagnostics,
    CursorScope::Participants,
    CursorScope::ServiceMembership,
    CursorScope::Recipients,
    CursorScope::WarningRecipients,
    CursorScope::PendingReceipts,
    CursorScope::LocalIntents,
    CursorScope::RetirementJobs,
    CursorScope::WakeCandidates,
    CursorScope::WakeRecovery,
    CursorScope::WorkJobs,
    CursorScope::SearchCandidates,
    CursorScope::InboxBatch,
    CursorScope::ActiveWarnings,
];
const F_DESCENDING: u8 = 0x01;
const F_KEY: u8 = 0x02;
const F_SCOPE_REVISION: u8 = 0x04;
const F_FILTER_REVISION: u8 = 0x08;
const F_SEARCH: u8 = 0x10;
const F_INBOX: u8 = 0x20;
const F_ATTENTION: u8 = 0x40;
const F_ALL: u8 = 0x7f;

fn binding_tag(
    instance: &str,
    scope_key: &str,
    filter_digest: &str,
    payload: &[u8],
) -> [u8; CURSOR_TAG_BYTES] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"herdr-threads cursor c3\0");
    for part in [instance, scope_key, filter_digest] {
        hasher.update((part.len() as u32).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.update(payload);
    let digest = hasher.finalize();
    let mut tag = [0u8; CURSOR_TAG_BYTES];
    tag.copy_from_slice(&digest[..CURSOR_TAG_BYTES]);
    tag
}

/// LEB128 unsigned and zigzag signed varints.
struct Compact(Vec<u8>);
impl Compact {
    fn unsigned(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.0.push(byte);
                return;
            }
            self.0.push(byte | 0x80);
        }
    }
    fn signed(&mut self, value: i64) {
        self.unsigned(((value << 1) ^ (value >> 63)) as u64);
    }
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, &'static str> {
        let (first, rest) = self.0.split_first().ok_or("malformed cursor")?;
        self.0 = rest;
        Ok(*first)
    }
    fn take(&mut self, len: usize) -> Result<&[u8], &'static str> {
        if self.0.len() < len {
            return Err("malformed cursor");
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }
    fn unsigned(&mut self) -> Result<u64, &'static str> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            let bits = u64::from(byte & 0x7f);
            if shift == 63 && bits > 1 {
                return Err("malformed cursor");
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err("malformed cursor")
    }
    fn signed(&mut self) -> Result<i64, &'static str> {
        let raw = self.unsigned()?;
        Ok(((raw >> 1) as i64) ^ -((raw & 1) as i64))
    }
}

const BASE64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(BASE64URL[((n >> 18) & 63) as usize] as char);
        result.push(BASE64URL[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(BASE64URL[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(BASE64URL[(n & 63) as usize] as char);
        }
    }
    result
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, &'static str> {
    if input.len() % 4 == 1 {
        return Err("malformed cursor");
    }
    let mut result = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            let value = BASE64URL
                .iter()
                .position(|candidate| candidate == byte)
                .ok_or("malformed cursor")? as u32;
            n |= value << (18 - index * 6);
        }
        // Canonical only: the unused low bits of a final partial group must
        // be zero, so one token has exactly one spelling.
        if chunk.len() == 2 && n & 0xf000 != 0 || chunk.len() == 3 && n & 0x00c0 != 0 {
            return Err("malformed cursor");
        }
        result.push((n >> 16) as u8);
        if chunk.len() > 2 {
            result.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            result.push(n as u8);
        }
    }
    Ok(result)
}
