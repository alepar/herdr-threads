//! The Claude mod watch protocol and the mod-side JSON-line contract
//! (spec D2-D4, D6, D7). Everything here is the seam contract of epic ht-j16:
//! it is inert until ht-j16.2 (registry and transport), ht-j16.3 (ack
//! decisions), ht-j16.5 (the `watch` child) and ht-j16.6 (the mod) land.
//!
//! # Connection
//!
//! A watch connection rides protocol version 6 behind capability
//! `mod.watch_v1` ([`super::capabilities::MOD_WATCH`]). Framing is the existing
//! four-byte big-endian length plus one JSON frame. The first client frame is
//! a [`WatchWireRequest`] (top-level key `watch`, disjoint from `WireRequest`
//! and `ServiceWireRequest`); the first daemon frame is a [`WatchReply`];
//! every later daemon frame is a [`WatchFrame`].
//!
//! # The `watch` stdout JSON-line schema
//!
//! The `watch` child prints one JSON object per line ([`WatchLine`]):
//!
//! | `kind` | extra keys | `id` |
//! |---|---|---|
//! | `message` | [`WatchMessage`] | the message id |
//! | `lazy` | [`WatchMessage`] (`ack_required: false`) | the message id |
//! | `attention` | `attention_version`, `text` | `attention:<version>` |
//! | `status` | `state`, `reason`, `exit` | `status:<n>` (per-process counter from 0) |
//!
//! Every line carries `schema` (1, [`WATCH_LINE_SCHEMA`]), `id`, `kind`, and
//! for message and lazy lines `truncated`. Consumers ignore unknown keys.
//!
//! Pages are bounded by [`WATCH_PAGE_MAX_ITEMS`] items and
//! [`WATCH_PAGE_MAX_BYTES`] bytes. A body longer than
//! [`WATCH_BODY_LIMIT_BYTES`] is cut at a char boundary and followed by
//! [`truncation_marker`], with `truncated: true`. A truncated item is never
//! acked by the mod; it settles only through `body`, `inbox` or `ack`.
//!
//! The mod frames all message text as untrusted data and never starts a
//! submit with `/`.

use super::{
    authority::CallerClaim,
    ids::{MessageId, SeatId, ThreadId},
    summary::{AuthorRole, UserIntent},
    time::UtcMillis,
    wire::{PROTOCOL_VERSION, valid_uuid, valid_wire_id},
};
use serde::{Deserialize, Deserializer, Serialize};

/// `schema` value of every [`WatchLine`].
pub const WATCH_LINE_SCHEMA: u32 = 1;
/// D4 per-message body limit.
pub const WATCH_BODY_LIMIT_BYTES: usize = 8 * 1024;
/// D4 page bound (items).
pub const WATCH_PAGE_MAX_ITEMS: usize = 32;
/// D4 page bound (bytes).
pub const WATCH_PAGE_MAX_BYTES: usize = 64 * 1024;
/// D2: watch connections have their own semaphore, outside `MAX_CONNECTIONS`.
pub const MAX_WATCH_CONNECTIONS: usize = 64;
/// D7: reconnect grace after a watch drop.
pub const MOD_RECONNECT_GRACE_MS: u64 = 30_000;
/// D7: seat-level rebind grace, started by `Close{binding_changed}`.
pub const MOD_REBIND_GRACE_MS: u64 = 30_000;
/// D7: a channel with no ack for this long may be stalled.
pub const MOD_STALL_AFTER_MS: u64 = 600_000;
/// D7: re-registration of a stalled binding generation is refused this long.
pub const MOD_STALL_COOLDOWN_MS: u64 = 600_000;
/// D6: cadence at which the mod retries `retryable` acks while registered.
pub const MOD_ACK_RETRY_MS: u64 = 30_000;
/// `off`: `watch` exits 3 before connecting (per-session override).
pub const MOD_DELIVERY_ENV: &str = "HERDR_THREADS_MOD_DELIVERY";
/// When set to a file path, the mod appends one [`ModLedgerEntry`] line per decision.
pub const MOD_LEDGER_ENV: &str = "HERDR_THREADS_MOD_LEDGER";
/// Exit code: the daemon stream ended (restart with backoff).
pub const WATCH_EXIT_STREAM_ENDED: i32 = 0;
/// Exit code: other error (restart with backoff).
pub const WATCH_EXIT_ERROR: i32 = 1;
/// Exit code: refused, retryable after backoff.
pub const WATCH_EXIT_RETRY: i32 = 2;
/// Exit code: permanent, stop until reload.
pub const WATCH_EXIT_STOP: i32 = 3;

/// The marker appended to a cut body.
pub fn truncation_marker(id: &MessageId) -> String {
    format!("…truncated; run herdr-threads body {}", id.as_str())
}

/// Daemon setting `mod_delivery` (`src/daemon/settings.rs` re-uses it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModDeliverySetting {
    #[default]
    On,
    Off,
}

/// Which delivery path a mod ack reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModDeliveryVia {
    /// Attached as context to a tool result.
    Context,
    /// Submitted as a new prompt while idle.
    Submit,
    /// Appended to the transcript (lazy rows).
    Append,
}

/// First client frame of a watch connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WatchWireRequest {
    pub version: u16,
    pub request_id: String,
    pub expected_instance: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_boot: Option<String>,
    pub watch: WatchRequest,
}

impl<'de> Deserialize<'de> for WatchWireRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            version: u16,
            request_id: String,
            expected_instance: String,
            #[serde(default)]
            expected_boot: Option<String>,
            watch: WatchRequest,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.version != PROTOCOL_VERSION {
            return Err(serde::de::Error::custom("unknown wire version"));
        }
        if !valid_wire_id(&raw.request_id) || !valid_uuid(&raw.expected_instance) {
            return Err(serde::de::Error::custom("invalid request or instance id"));
        }
        if raw
            .expected_boot
            .as_deref()
            .is_some_and(|boot| !valid_uuid(boot))
        {
            return Err(serde::de::Error::custom("invalid expected boot"));
        }
        Ok(Self {
            version: raw.version,
            request_id: raw.request_id,
            expected_instance: raw.expected_instance,
            expected_boot: raw.expected_boot,
            watch: raw.watch,
        })
    }
}

impl WatchWireRequest {
    pub fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// The claim the CLI builds for `HERDR_PANE_ID`, harness `claude`, native
/// session from `--session`. The daemon decides registration in one
/// transaction against A2 (spec D2) and records `cooperative_mod_channel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchRequest {
    pub claim: CallerClaim,
}

/// First daemon frame of a watch connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchReply {
    pub version: u16,
    pub request_id: String,
    pub instance: String,
    pub daemon_boot: String,
    pub outcome: WatchOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum WatchOutcome {
    Accepted(WatchAccepted),
    Refused(WatchRefusal),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchAccepted {
    /// The seat's current attention version at registration.
    pub attention_version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchRefusal {
    pub reason: WatchRefusalReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Later daemon frames. Notify call sites (spec D2): a new ordinary or lazy
/// row for the seat, an attention change, and a registration's initial sweep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum WatchFrame {
    Attention { version: u64 },
    Close { reason: WatchCloseReason },
}

/// Why a registration is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchRefusalReason {
    NoBinding,
    SessionMismatch,
    Held,
    Unresolved,
    /// Within `MOD_STALL_COOLDOWN_MS` of a stall for that generation.
    Cooldown,
    /// Over `MAX_WATCH_CONNECTIONS`.
    Busy,
    Stopping,
    NotClaude,
    Disabled,
}

impl WatchRefusalReason {
    /// Exit code of `watch` for this refusal: retryable ones 2, permanent 3.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::NotClaude | Self::Disabled => WATCH_EXIT_STOP,
            _ => WATCH_EXIT_RETRY,
        }
    }
}

/// Why the daemon ends a channel (spec D7 removal/grace per reason).
/// `binding_changed` starts the seat-level rebind grace; `retired`,
/// `unresolved`, `stalled`, `disabled` and `stopping` remove the channel at
/// once and kick the wake lane; `stalled` also starts the cooldown;
/// `replaced` means a newer channel took over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchCloseReason {
    Replaced,
    BindingChanged,
    Retired,
    Unresolved,
    Stalled,
    Disabled,
    Stopping,
}

impl WatchCloseReason {
    /// Exit code of `watch` after this Close: `disabled` and `replaced` stop
    /// (so two watchers of one seat cannot ping-pong); the rest restart.
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Disabled | Self::Replaced => WATCH_EXIT_STOP,
            _ => WATCH_EXIT_STREAM_ENDED,
        }
    }
}

/// The `reason` of a `status` line: a refusal, a close, or a CLI-local reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchStatusReason {
    NoBinding,
    SessionMismatch,
    Held,
    Unresolved,
    Cooldown,
    Busy,
    Stopping,
    NotClaude,
    Disabled,
    Replaced,
    BindingChanged,
    Retired,
    Stalled,
    NoPane,
    EnvDisabled,
    Unsupported,
    DaemonUnavailable,
    Error,
    StreamEnded,
}

impl WatchStatusReason {
    /// Shared names (`unresolved`, `stopping`, `disabled`) follow the refusal
    /// mapping; a Close's exit comes from [`WatchCloseReason::exit_code`].
    pub fn exit_code(self) -> i32 {
        match self {
            Self::NoBinding
            | Self::SessionMismatch
            | Self::Held
            | Self::Unresolved
            | Self::Cooldown
            | Self::Busy
            | Self::Stopping => WATCH_EXIT_RETRY,
            Self::NotClaude
            | Self::Disabled
            | Self::Replaced
            | Self::NoPane
            | Self::EnvDisabled
            | Self::Unsupported => WATCH_EXIT_STOP,
            Self::DaemonUnavailable | Self::Error => WATCH_EXIT_ERROR,
            Self::BindingChanged | Self::Retired | Self::Stalled | Self::StreamEnded => {
                WATCH_EXIT_STREAM_ENDED
            }
        }
    }
}

impl From<WatchRefusalReason> for WatchStatusReason {
    fn from(reason: WatchRefusalReason) -> Self {
        match reason {
            WatchRefusalReason::NoBinding => Self::NoBinding,
            WatchRefusalReason::SessionMismatch => Self::SessionMismatch,
            WatchRefusalReason::Held => Self::Held,
            WatchRefusalReason::Unresolved => Self::Unresolved,
            WatchRefusalReason::Cooldown => Self::Cooldown,
            WatchRefusalReason::Busy => Self::Busy,
            WatchRefusalReason::Stopping => Self::Stopping,
            WatchRefusalReason::NotClaude => Self::NotClaude,
            WatchRefusalReason::Disabled => Self::Disabled,
        }
    }
}

impl From<WatchCloseReason> for WatchStatusReason {
    fn from(reason: WatchCloseReason) -> Self {
        match reason {
            WatchCloseReason::Replaced => Self::Replaced,
            WatchCloseReason::BindingChanged => Self::BindingChanged,
            WatchCloseReason::Retired => Self::Retired,
            WatchCloseReason::Unresolved => Self::Unresolved,
            WatchCloseReason::Stalled => Self::Stalled,
            WatchCloseReason::Disabled => Self::Disabled,
            WatchCloseReason::Stopping => Self::Stopping,
        }
    }
}

/// One stdout line of `watch`. No `deny_unknown_fields` (incompatible with
/// `flatten`); consumers ignore unknown keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchLine {
    pub schema: u32,
    pub id: String,
    #[serde(flatten)]
    pub item: WatchItem,
}

impl WatchLine {
    pub fn new(id: impl Into<String>, item: WatchItem) -> Self {
        Self {
            schema: WATCH_LINE_SCHEMA,
            id: id.into(),
            item,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WatchItem {
    Message(WatchMessage),
    Lazy(WatchMessage),
    Attention(WatchAttention),
    Status(WatchStatus),
}

/// A message or lazy row. `body` is the full body, or its first
/// `WATCH_BODY_LIMIT_BYTES` bytes (char boundary) followed by
/// [`truncation_marker`]; `body_len` is the stored length; `ack_required` is
/// false for lazy rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchMessage {
    pub thread: ThreadId,
    pub thread_name: Option<String>,
    pub sender: Option<SeatId>,
    pub sender_name: Option<String>,
    pub author_role: Option<AuthorRole>,
    pub relays_user: bool,
    pub user_intent: Option<UserIntent>,
    pub body: String,
    pub body_len: u64,
    pub truncated: bool,
    pub ack_required: bool,
}

/// `text` is the existing fixed attention marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchAttention {
    pub attention_version: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchStatus {
    pub state: WatchStatusState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<WatchStatusReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchStatusState {
    Connected,
    Refused,
    Closing,
}

/// Per-id result of a mod ack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModAckOutcome {
    Settled,
    AlreadySettled,
    RefusedTerminal,
    /// The item was delivered under an older binding generation (its context
    /// no longer holds it); it is re-streamed to the new session.
    StaleGeneration,
    /// Kept and retried on the next Attention, after the next registration
    /// and every `MOD_ACK_RETRY_MS`.
    Retryable,
}

impl ModAckOutcome {
    /// Only `settled` and `already_settled` count toward the stall clock.
    pub fn counts_as_mod_ack(self) -> bool {
        matches!(self, Self::Settled | Self::AlreadySettled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModAckReason {
    Unknown,
    NotAddressed,
    Truncated,
    NoLiveChannel,
    Busy,
    Unreachable,
}

/// One id's result; also the exact `watch ack` stdout line, one per id. Exit
/// 0 iff every id has a line. `truncated` is decided from the stored body
/// length, never a client hint. Resume re-ack rule: an ack for an item
/// delivered under the immediately previous generation of the same native
/// session is accepted; any other older generation is `stale_generation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModAckItem {
    pub id: MessageId,
    pub result: ModAckOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ModAckReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModAckReport {
    pub results: Vec<ModAckItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModChannelState {
    Live,
    ReconnectGrace,
    RebindGrace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModChannelEntry {
    pub seat: SeatId,
    pub harness: String,
    pub binding_generation: u64,
    pub connected_since: UtcMillis,
    pub state: ModChannelState,
}

/// Live mod channels and the daemon setting, for setup-status. At most
/// `MAX_WATCH_CONNECTIONS` entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModChannelStatus {
    pub mod_delivery: ModDeliverySetting,
    pub live_channels: u32,
    pub channels: Vec<ModChannelEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModLedgerKind {
    Received,
    Delivered,
    Acked,
    Held,
    Submit,
    Refused,
    Restart,
}

/// Written by the mod only when `HERDR_THREADS_MOD_LEDGER` names a file, one
/// JSON line per decision; tests (ht-j16.6, .9, .10) assert against it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModLedgerEntry {
    pub at: u64,
    pub kind: ModLedgerKind,
    pub ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<ModDeliveryVia>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
#[path = "../../tests/protocol/watch.rs"]
mod tests;
