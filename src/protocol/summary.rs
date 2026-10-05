//! Thread summary wire contract (design spec §1-§6, §13). Owned by ht-1ip.1:
//! the priority predicate, summary settings, ledger/block/job types, and the
//! `Summary`/`SummaryJob`/`SummarySubmit` request and outcome types. Later
//! tasks implement the behaviour behind them.
use crate::protocol::{
    authority::CallerClaim,
    ids::{LeaseToken, MessageId, SeatId, SummaryBlockId, SummaryJobId, ThreadId},
    results::MessageKind,
    time::UtcMillis,
};
use serde::{Deserialize, Serialize};

/// The herdr-threads skill section the recovery hook text cites (ht-1ip.9) and
/// the skill titles its worker procedure with (ht-1ip.11).
pub const SUMMARY_PROCEDURE_REF: &str = "Thread summaries";
/// Submission wire schema accepted by `SummarySubmit` (spec §6).
pub const SUBMISSION_SCHEMA: u32 = 2;
/// Rollup fan-in; fixed in this version (spec §3).
pub const FAN_IN: u32 = 8;
/// `prompt_version` and `model` bound (spec §5): non-empty, at most this many bytes.
pub const MAX_PROVENANCE_LABEL_BYTES: usize = 64;
/// A prefill instruction carries verbatim `text` up to this size, else `text_ref` (spec §5).
pub const PREFILL_TEXT_MAX_BYTES: usize = 2048;
/// Per-block and per-fold identifier caps (spec §5).
pub const BLOCK_IDENTIFIER_CAP: usize = 64;
pub const FOLD_IDENTIFIER_CAP: usize = 128;
/// Leasing and p99 bounds (spec §4, §8).
pub const RESERVATION_LAPSE_MS: u64 = 60_000;
pub const LEASE_MIN_MS: u64 = 60_000;
pub const LEASE_MAX_MS: u64 = 600_000;
pub const P99_SAMPLE_WINDOW: usize = 200;
pub const P99_MIN_SAMPLES: usize = 20;
pub const P99_CLAMP_MIN_MS: u64 = 30_000;
pub const P99_CLAMP_MAX_MS: u64 = 600_000;

/// `messages.author_role` (spec §1). Distinct from `messages.author_kind`
/// (0002: native|programmatic|built_in), which this never replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorRole {
    Human,
    Agent,
    Service,
}
impl AuthorRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Service => "service",
        }
    }
    pub fn from_column(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "agent" => Some(Self::Agent),
            "service" => Some(Self::Service),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserIntent {
    Query,
    Request,
    Rule,
}
impl UserIntent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Request => "request",
            Self::Rule => "rule",
        }
    }
    pub fn from_column(value: &str) -> Option<Self> {
        match value {
            "query" => Some(Self::Query),
            "request" => Some(Self::Request),
            "rule" => Some(Self::Rule),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleChange {
    Withdrawn,
    Replaced,
}
impl RuleChange {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Withdrawn => "withdrawn",
            Self::Replaced => "replaced",
        }
    }
    pub fn from_column(value: &str) -> Option<Self> {
        match value {
            "withdrawn" => Some(Self::Withdrawn),
            "replaced" => Some(Self::Replaced),
            _ => None,
        }
    }
}

/// The single definition of a priority message (spec §1): a human author, or an
/// agent's cooperative `--relays-user` claim. A NULL role reads as agent.
pub fn is_priority(author_role: Option<AuthorRole>, relays_user: bool) -> bool {
    author_role == Some(AuthorRole::Human) || relays_user
}

/// Spec §6: `budget_bytes` bounds the whole encoded submission.
pub fn submission_budget_bytes(level: u32, narrative_bytes: u32) -> u32 {
    narrative_bytes.saturating_add(if level == 0 { 8 * 1024 } else { 1024 })
}

/// `prompt_version` / `model`: stored verbatim, only bounded (spec §5).
pub fn valid_provenance_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_PROVENANCE_LABEL_BYTES
}

/// Installation settings for summaries, catch-up and pokes (spec §13). Loaded
/// from the instance `settings.json` under `"summary"`; every key optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SummarySettings {
    pub chunk_bytes: u32,
    pub display_bytes: u32,
    pub narrative_bytes: u32,
    pub bundle_bytes: u32,
    pub fold_display_bytes: u32,
    pub max_new_leases: u32,
    pub tracker_prefixes: Vec<String>,
    pub fan_in: u32,
    pub hot_window_ms: u64,
    pub exit_grace_ms: u64,
    pub p99_cold_ms: u64,
    pub soft_fraction: f64,
}
impl Default for SummarySettings {
    fn default() -> Self {
        Self {
            chunk_bytes: 24 * 1024,
            display_bytes: 40 * 1024,
            narrative_bytes: 3 * 1024,
            bundle_bytes: 48 * 1024,
            fold_display_bytes: 24 * 1024,
            max_new_leases: 8,
            tracker_prefixes: vec!["ht-".to_string()],
            fan_in: FAN_IN,
            hot_window_ms: 24 * 60 * 60 * 1000,
            exit_grace_ms: 60_000,
            p99_cold_ms: 90_000,
            soft_fraction: 0.6,
        }
    }
}
impl SummarySettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        let positive = [
            (self.chunk_bytes, "chunk_bytes must be positive"),
            (self.display_bytes, "display_bytes must be positive"),
            (self.narrative_bytes, "narrative_bytes must be positive"),
            (self.bundle_bytes, "bundle_bytes must be positive"),
            (
                self.fold_display_bytes,
                "fold_display_bytes must be positive",
            ),
            (self.max_new_leases, "max_new_leases must be positive"),
        ];
        for (value, message) in positive {
            if value == 0 {
                return Err(message);
            }
        }
        if self.fan_in != FAN_IN {
            return Err("fan_in is fixed at 8 in this version");
        }
        if self.hot_window_ms == 0 {
            return Err("hot_window_ms must be positive");
        }
        if self.exit_grace_ms == 0 {
            return Err("exit_grace_ms must be positive");
        }
        if self.p99_cold_ms == 0 {
            return Err("p99_cold_ms must be positive");
        }
        if !(self.soft_fraction.is_finite() && self.soft_fraction > 0.0 && self.soft_fraction < 1.0)
        {
            return Err("soft_fraction must be strictly between 0 and 1");
        }
        if self.tracker_prefixes.is_empty() || self.tracker_prefixes.len() > 8 {
            return Err("tracker_prefixes needs 1 to 8 entries");
        }
        for prefix in &self.tracker_prefixes {
            if prefix.is_empty()
                || prefix.len() > 16
                || !prefix
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err("tracker_prefixes entries are 1 to 16 ASCII alphanumeric, '-' or '_'");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeqRange {
    pub first_seq: u64,
    pub last_seq: u64,
}

// ---- ledger records (spec §5) ----
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenItemKind {
    Ask,
    Commitment,
    Question,
    Blocker,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierKind {
    Path,
    BeadId,
    Sha,
    Url,
    Error,
}
/// Status an item has in the fold. Decisions start `Active`, instructions and open items `Open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Active,
    Open,
    Done,
    Resolved,
    Superseded,
}
/// Status a transition may set: instruction done|superseded, open item
/// resolved|superseded, decision superseded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewStatus {
    Done,
    Resolved,
    Superseded,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemBody {
    UserInstruction {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author_seat: Option<SeatId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        author_role: Option<AuthorRole>,
        relays_user: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_intent: Option<UserIntent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text_ref: Option<u64>,
        /// The source message, so a spilled `text_ref` can be fetched with `body`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<MessageId>,
    },
    Decision {
        by_seat: SeatId,
        text: String,
    },
    OpenItem {
        kind: OpenItemKind,
        from_seat: SeatId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_seat: Option<SeatId>,
        text: String,
    },
}
/// One introduced item. `id` is tied to the immutable thread: `i.<seq>` for a
/// prefill instruction, `<chunking_version>.<chunk index>.<n>` for a model item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerItem {
    pub id: String,
    pub seq: u64,
    pub body: ItemBody,
}
/// Identifiers are keyed by value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identifier {
    pub value: String,
    pub kind: IdentifierKind,
    pub seqs: Vec<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_change: Option<RuleChange>,
    pub target_id: String,
    pub new_status: NewStatus,
    pub cite_seq: u64,
}
/// What a level-0 block stores besides its narrative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level0Records {
    pub items: Vec<LedgerItem>,
    pub identifiers: Vec<Identifier>,
    pub transitions: Vec<Transition>,
}

// ---- fold (computed by the daemon, rendered by one renderer; ht-1ip.4) ----
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldDisplay {
    Full,
    OneLine,
    TextRef,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoldEntry {
    pub item: LedgerItem,
    pub status: ItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at_seq: Option<u64>,
    pub display: FoldDisplay,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fold {
    pub entries: Vec<FoldEntry>,
    pub identifiers: Vec<Identifier>,
    pub rendered_bytes: u32,
}

// ---- blocks (spec §5) ----
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockProvenance {
    DerivedSummary,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockHeader {
    pub thread: ThreadId,
    pub chunking_version: String,
    pub level: u32,
    pub index: u64,
    pub range: SeqRange,
    pub children: Vec<SummaryBlockId>,
    pub source_hash: String,
    pub fallback: bool,
    pub provenance: BlockProvenance,
    pub author_seat: SeatId,
    pub prompt_version: String,
    pub model: String,
    pub created_at: UtcMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub block_id: SummaryBlockId,
    pub header: BlockHeader,
    pub narrative: String,
}

// ---- bundles (spec §3, §4) ----
/// One message as rendered into a job bundle or a Ready tail (header line plus
/// body for ordinary messages, the compact event line for info/warn). Peer data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<UserIntent>,
    pub sequence: u64,
    pub message: MessageId,
    pub kind: MessageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<SeatId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_role: Option<AuthorRole>,
    pub relays_user: bool,
    pub created_at: UtcMillis,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildNarrative {
    pub block_id: SummaryBlockId,
    pub level: u32,
    pub index: u64,
    pub range: SeqRange,
    pub narrative: String,
    pub fallback: bool,
}
/// Raw text behind an open pinned user instruction in a rollup bundle; spilled
/// to `text_ref` first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedText {
    pub item_id: String,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_ref: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobBundle {
    pub job_id: SummaryJobId,
    pub thread: ThreadId,
    pub chunking_version: String,
    pub level: u32,
    pub index: u64,
    pub range: SeqRange,
    pub submission_schema: u32,
    pub budget_bytes: u32,
    pub narrative_bytes: u32,
    pub messages: Vec<BundleMessage>,
    pub children: Vec<ChildNarrative>,
    pub fold: Fold,
    pub pinned: Vec<PinnedText>,
    /// Encoded size; `oversized` when still above `bundle_bytes` after spilling (spec §3).
    pub size_bytes: u32,
    pub oversized: bool,
}

// ---- tickets and outcomes (spec §4) ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobTicket {
    pub job_id: SummaryJobId,
    pub lease_token: LeaseToken,
    pub lease_until: UtcMillis,
    pub level: u32,
    pub index: u64,
    pub range: SeqRange,
    pub budget_bytes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRef {
    pub job_id: SummaryJobId,
    pub level: u32,
    pub index: u64,
    pub range: SeqRange,
    pub lease_until: UtcMillis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverSizes {
    pub narrative_bytes: u32,
    pub display_bytes: u32,
    pub fold_bytes: u32,
    pub fold_display_bytes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryReady {
    pub frontier: u64,
    pub cover: Vec<Block>,
    pub fold: Fold,
    pub tail: Vec<BundleMessage>,
    pub tail_complete: bool,
    pub over_budget: bool,
    pub sizes: CoverSizes,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryWork {
    pub frontier: u64,
    pub jobs: Vec<JobTicket>,
    pub leased_elsewhere: Vec<JobRef>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum SummaryOutcome {
    Ready(SummaryReady),
    Work(SummaryWork),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum SummaryJobOutcome {
    Bundle(JobBundle),
    ReservationLapsed { leased_elsewhere: JobRef },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum SubmitOutcome {
    Stored {
        block_id: SummaryBlockId,
        fallback: bool,
    },
    Rejected {
        reasons: Vec<String>,
    },
}

// ---- requests ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryRequest {
    pub thread: ThreadId,
    pub claim: CallerClaim,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryJobRequest {
    pub job_id: SummaryJobId,
    pub lease_token: LeaseToken,
    pub claim: CallerClaim,
}
/// `submission` stays raw JSON so an unparsable or unknown-schema submission is
/// a validator `Rejected` (spec §6 rule 1) that counts toward the fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummarySubmitRequest {
    pub job_id: SummaryJobId,
    pub lease_token: LeaseToken,
    pub submission: serde_json::Value,
    pub claim: CallerClaim,
}

// ---- submission (spec §6), parsed by the daemon, never trusted as instructions ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewDecision {
    #[serde(rename = "ref")]
    pub reference: String,
    pub seq: u64,
    pub by_seat: SeatId,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewOpenItem {
    #[serde(rename = "ref")]
    pub reference: String,
    pub seq: u64,
    pub kind: OpenItemKind,
    pub from_seat: SeatId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_seat: Option<SeatId>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}
/// `target` is a bundle-fold id or a `ref` from the same submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedTransition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_change: Option<RuleChange>,
    pub target: String,
    pub new_status: NewStatus,
    pub cite_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub submission_schema: u32,
    pub narrative: String,
    #[serde(default)]
    pub new_decisions: Vec<NewDecision>,
    #[serde(default)]
    pub new_open_items: Vec<NewOpenItem>,
    #[serde(default)]
    pub transitions: Vec<ProposedTransition>,
    pub prompt_version: String,
    pub model: String,
}
impl Submission {
    /// Rule 1's parse step only; level, budget and content rules are ht-1ip.4's validator.
    pub fn parse(value: &serde_json::Value) -> Result<Self, String> {
        let object = value.as_object().ok_or("submission is not a JSON object")?;
        match object
            .get("submission_schema")
            .and_then(serde_json::Value::as_u64)
        {
            Some(n) if n == u64::from(SUBMISSION_SCHEMA) => {}
            Some(n) => return Err(format!("unknown submission_schema {n}")),
            None => return Err("missing submission_schema".into()),
        }
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid submission: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_intent_contract_bundle_roundtrip() {
        let old = json!({"sequence":1,"message":"m1","kind":"ordinary","author":"sa","author_role":"human","relays_user":false,"created_at":1,"text":"input"});
        let mut message: BundleMessage = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(message.user_intent, None);
        assert_eq!(serde_json::to_value(&message).unwrap(), old);
        for intent in [UserIntent::Query, UserIntent::Request, UserIntent::Rule] {
            message.user_intent = Some(intent);
            let json = serde_json::to_value(&message).unwrap();
            assert_eq!(json["user_intent"], intent.as_str());
            assert_eq!(
                serde_json::from_value::<BundleMessage>(json)
                    .unwrap()
                    .user_intent,
                Some(intent)
            );
        }
    }
    #[test]
    fn user_intent_contract_old_item_and_transition_omit_new_fields() {
        use serde_json::json;
        let old_item = json!({"type":"user_instruction","relays_user":false,"text":"old"});
        let item: ItemBody = serde_json::from_value(old_item.clone()).unwrap();
        assert!(matches!(
            &item,
            ItemBody::UserInstruction {
                user_intent: None,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(&item).unwrap(), old_item);
        let old_transition = json!({"target_id":"i.1","new_status":"superseded","cite_seq":2});
        let transition: Transition = serde_json::from_value(old_transition.clone()).unwrap();
        assert_eq!(transition.rule_change, None);
        assert_eq!(serde_json::to_value(transition).unwrap(), old_transition);
    }
    #[test]
    fn user_intent_contract_enum_spellings_and_some_roundtrip() {
        use serde_json::json;
        for (intent, spelling) in [
            (UserIntent::Query, "query"),
            (UserIntent::Request, "request"),
            (UserIntent::Rule, "rule"),
        ] {
            assert_eq!(intent.as_str(), spelling);
            assert_eq!(UserIntent::from_column(spelling), Some(intent));
            assert_eq!(serde_json::to_value(intent).unwrap(), json!(spelling));
            let raw = json!({"type":"user_instruction","relays_user":true,"user_intent":spelling});
            let item: ItemBody = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(serde_json::to_value(item).unwrap(), raw);
        }
        for (change, spelling) in [
            (RuleChange::Withdrawn, "withdrawn"),
            (RuleChange::Replaced, "replaced"),
        ] {
            assert_eq!(change.as_str(), spelling);
            assert_eq!(RuleChange::from_column(spelling), Some(change));
            let raw = json!({"target":"i.1","new_status":"superseded","cite_seq":2,"rule_change":spelling});
            let proposed: ProposedTransition = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(proposed.rule_change, Some(change));
            assert_eq!(serde_json::to_value(proposed).unwrap(), raw);
        }
        assert!(serde_json::from_value::<UserIntent>(json!("instruction")).is_err());
        assert!(serde_json::from_value::<RuleChange>(json!("completed")).is_err());
    }

    #[test]
    fn priority_is_human_author_or_relayed_user_ask() {
        assert!(is_priority(Some(AuthorRole::Human), false));
        assert!(is_priority(Some(AuthorRole::Human), true));
        assert!(is_priority(Some(AuthorRole::Agent), true));
        assert!(is_priority(None, true));
        assert!(!is_priority(Some(AuthorRole::Agent), false));
        assert!(!is_priority(Some(AuthorRole::Service), false));
        // A NULL (backfilled, no covering binding) role reads as agent.
        assert!(!is_priority(None, false));
    }

    #[test]
    fn author_role_round_trips_its_column_values() {
        for role in [AuthorRole::Human, AuthorRole::Agent, AuthorRole::Service] {
            assert_eq!(AuthorRole::from_column(role.as_str()), Some(role));
        }
        assert_eq!(AuthorRole::from_column("native"), None);
    }

    #[test]
    fn summary_settings_defaults_match_the_spec() {
        let s = SummarySettings::default();
        assert_eq!(s.chunk_bytes, 24 * 1024);
        assert_eq!(s.display_bytes, 40 * 1024);
        assert_eq!(s.narrative_bytes, 3 * 1024);
        assert_eq!(s.bundle_bytes, 48 * 1024);
        assert_eq!(s.fold_display_bytes, 24 * 1024);
        assert_eq!(s.max_new_leases, 8);
        assert_eq!(s.tracker_prefixes, vec!["ht-".to_string()]);
        assert_eq!(s.fan_in, FAN_IN);
        assert_eq!(s.hot_window_ms, 24 * 60 * 60 * 1000);
        assert_eq!(s.exit_grace_ms, 60_000);
        assert_eq!(s.p99_cold_ms, 90_000);
        assert_eq!(s.soft_fraction, 0.6);
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn summary_settings_reject_non_positive_and_out_of_range_values() {
        let bad: Vec<fn(&mut SummarySettings)> = vec![
            |s| s.chunk_bytes = 0,
            |s| s.display_bytes = 0,
            |s| s.narrative_bytes = 0,
            |s| s.bundle_bytes = 0,
            |s| s.fold_display_bytes = 0,
            |s| s.max_new_leases = 0,
            |s| s.fan_in = 10,
            |s| s.hot_window_ms = 0,
            |s| s.exit_grace_ms = 0,
            |s| s.p99_cold_ms = 0,
            |s| s.soft_fraction = 0.0,
            |s| s.soft_fraction = 1.0,
            |s| s.soft_fraction = f64::NAN,
            |s| s.tracker_prefixes = vec![],
            |s| s.tracker_prefixes = vec![String::new()],
            |s| s.tracker_prefixes = vec!["has space-".into()],
        ];
        for mutate in bad {
            let mut s = SummarySettings::default();
            mutate(&mut s);
            assert!(s.validate().is_err(), "{s:?}");
        }
    }

    #[test]
    fn submission_budget_and_labels_follow_spec_section_6() {
        assert_eq!(submission_budget_bytes(0, 3072), 3072 + 8192);
        assert_eq!(submission_budget_bytes(1, 3072), 3072 + 1024);
        assert!(valid_provenance_label("summary-worker-v1"));
        assert!(valid_provenance_label(&"m".repeat(64)));
        assert!(!valid_provenance_label(""));
        assert!(!valid_provenance_label(&"m".repeat(65)));
    }

    fn fold() -> Fold {
        Fold {
            entries: vec![FoldEntry {
                item: LedgerItem {
                    id: "i.3".into(),
                    seq: 3,
                    body: ItemBody::UserInstruction {
                        author_seat: Some(SeatId::new("sh")),
                        author_role: Some(AuthorRole::Human),
                        relays_user: false,
                        user_intent: None,
                        text: Some("do the thing".into()),
                        text_ref: None,
                        message_id: None,
                    },
                },
                status: ItemStatus::Open,
                closed_at_seq: None,
                display: FoldDisplay::Full,
            }],
            identifiers: vec![Identifier {
                value: "src/lib.rs".into(),
                kind: IdentifierKind::Path,
                seqs: vec![3],
            }],
            rendered_bytes: 64,
        }
    }

    fn block() -> Block {
        Block {
            block_id: SummaryBlockId::new("b1"),
            header: BlockHeader {
                thread: ThreadId::new("t1"),
                chunking_version: "c1".into(),
                level: 0,
                index: 0,
                range: SeqRange {
                    first_seq: 1,
                    last_seq: 9,
                },
                children: vec![],
                source_hash: "abc".into(),
                fallback: false,
                provenance: BlockProvenance::DerivedSummary,
                author_seat: SeatId::new("sa"),
                prompt_version: "p1".into(),
                model: "m1".into(),
                created_at: UtcMillis(100),
            },
            narrative: "they talked".into(),
        }
    }

    fn job_ref() -> JobRef {
        JobRef {
            job_id: SummaryJobId::new("j2"),
            level: 0,
            index: 1,
            range: SeqRange {
                first_seq: 10,
                last_seq: 19,
            },
            lease_until: UtcMillis(500),
        }
    }

    #[test]
    fn ready_and_work_round_trip_through_json() {
        let ready = SummaryOutcome::Ready(SummaryReady {
            frontier: 12,
            cover: vec![block()],
            fold: fold(),
            tail: vec![BundleMessage {
                sequence: 12,
                message: MessageId::new("m12"),
                kind: MessageKind::Ordinary,
                author: Some(SeatId::new("sa")),
                author_role: Some(AuthorRole::Agent),
                relays_user: true,
                user_intent: None,
                created_at: UtcMillis(90),
                text: "hi".into(),
            }],
            tail_complete: true,
            over_budget: false,
            sizes: CoverSizes {
                narrative_bytes: 11,
                display_bytes: 100,
                fold_bytes: 64,
                fold_display_bytes: 64,
            },
        });
        let value = serde_json::to_value(&ready).unwrap();
        assert_eq!(value["status"], "ready");
        assert!(value["data"].is_object());
        assert_eq!(
            value["data"]["cover"][0]["header"]["provenance"],
            "derived_summary"
        );
        assert_eq!(
            serde_json::from_value::<SummaryOutcome>(value).unwrap(),
            ready
        );

        let work = SummaryOutcome::Work(SummaryWork {
            frontier: 12,
            jobs: vec![JobTicket {
                job_id: SummaryJobId::new("j1"),
                lease_token: LeaseToken::new("l1"),
                lease_until: UtcMillis(400),
                level: 0,
                index: 0,
                range: SeqRange {
                    first_seq: 1,
                    last_seq: 9,
                },
                budget_bytes: 11264,
            }],
            leased_elsewhere: vec![job_ref()],
        });
        let value = serde_json::to_value(&work).unwrap();
        assert_eq!(value["status"], "work");
        assert_eq!(value["data"]["leased_elsewhere"][0]["job_id"], "j2");
        assert_eq!(
            serde_json::from_value::<SummaryOutcome>(value).unwrap(),
            work
        );
    }

    #[test]
    fn job_outcomes_round_trip() {
        let bundle = SummaryJobOutcome::Bundle(JobBundle {
            job_id: SummaryJobId::new("j1"),
            thread: ThreadId::new("t1"),
            chunking_version: "c1".into(),
            level: 1,
            index: 0,
            range: SeqRange {
                first_seq: 1,
                last_seq: 80,
            },
            submission_schema: SUBMISSION_SCHEMA,
            budget_bytes: 4096,
            narrative_bytes: 3072,
            messages: vec![],
            children: vec![ChildNarrative {
                block_id: SummaryBlockId::new("b1"),
                level: 0,
                index: 0,
                range: SeqRange {
                    first_seq: 1,
                    last_seq: 9,
                },
                narrative: "n".into(),
                fallback: true,
            }],
            fold: fold(),
            pinned: vec![PinnedText {
                item_id: "i.3".into(),
                seq: 3,
                text: None,
                text_ref: Some(3),
            }],
            size_bytes: 500,
            oversized: false,
        });
        let value = serde_json::to_value(&bundle).unwrap();
        assert_eq!(value["status"], "bundle");
        assert_eq!(
            serde_json::from_value::<SummaryJobOutcome>(value).unwrap(),
            bundle
        );

        let lapsed = SummaryJobOutcome::ReservationLapsed {
            leased_elsewhere: job_ref(),
        };
        let value = serde_json::to_value(&lapsed).unwrap();
        assert_eq!(value["status"], "reservation_lapsed");
        assert_eq!(
            serde_json::from_value::<SummaryJobOutcome>(value).unwrap(),
            lapsed
        );

        let stored = SubmitOutcome::Stored {
            block_id: SummaryBlockId::new("b9"),
            fallback: true,
        };
        let value = serde_json::to_value(&stored).unwrap();
        assert_eq!(value["status"], "stored");
        assert_eq!(
            serde_json::from_value::<SubmitOutcome>(value).unwrap(),
            stored
        );
        let rejected = SubmitOutcome::Rejected {
            reasons: vec!["too big".into()],
        };
        let value = serde_json::to_value(&rejected).unwrap();
        assert_eq!(value["status"], "rejected");
        assert_eq!(
            serde_json::from_value::<SubmitOutcome>(value).unwrap(),
            rejected
        );
    }

    #[test]
    fn user_intent_generation2_parses_schema2_and_refuses_schema1() {
        let full = json!({
            "submission_schema": SUBMISSION_SCHEMA,
            "narrative": "n",
            "new_decisions": [{"ref": "d1", "seq": 4, "by_seat": "sa", "text": "use sqlite"}],
            "new_open_items": [{"ref": "o1", "seq": 5, "kind": "ask", "from_seat": "sa", "text": "who?"}],
            "transitions": [{"target": "i.3", "new_status": "done", "cite_seq": 6}],
            "prompt_version": "p1",
            "model": "m1",
        });
        let parsed = Submission::parse(&full).unwrap();
        assert_eq!(parsed.new_decisions[0].reference, "d1");
        assert_eq!(parsed.new_open_items[0].seq, 5);
        assert_eq!(parsed.new_open_items[0].kind, OpenItemKind::Ask);
        assert_eq!(parsed.transitions[0].target, "i.3");
        assert_eq!(parsed.transitions[0].new_status, NewStatus::Done);

        let rollup = json!({"submission_schema": SUBMISSION_SCHEMA, "narrative": "n", "prompt_version": "p", "model": "m"});
        let parsed = Submission::parse(&rollup).unwrap();
        assert!(parsed.new_decisions.is_empty());
        assert!(parsed.new_open_items.is_empty());
        assert!(parsed.transitions.is_empty());

        let mut v1 = rollup.clone();
        v1["submission_schema"] = json!(1);
        assert!(
            Submission::parse(&v1)
                .unwrap_err()
                .contains("unknown submission_schema 1")
        );
        let mut unknown = rollup.clone();
        unknown["submission_schema"] = json!(SUBMISSION_SCHEMA + 1);
        assert!(Submission::parse(&unknown).is_err());
        let missing = json!({"narrative": "n", "prompt_version": "p", "model": "m"});
        assert!(Submission::parse(&missing).is_err());
        assert!(Submission::parse(&json!("text")).is_err());
        let mut extra = rollup.clone();
        extra["surprise"] = json!(1);
        assert!(Submission::parse(&extra).is_err());
    }
}
