//! Wire types of the operator and status CLI for cross-node sync (ht-5fr.8).
//!
//! Responsibility
//! - Owns: the closed [`SyncCommand`] set (`peer add|forget|list`,
//!   `sync status|verify|unapplied|resync|clone-cleared`), its request
//!   validation, and the [`SyncReply`] shapes the daemon answers with. The
//!   rows are plain data: the daemon fills them from `store::sync_status`,
//!   the coordinator snapshot and `store::incarnations`; the compact text
//!   encoder (`output_compact`) and `--json` render them.
//! - Hides: where each field comes from, and that node names on the wire are
//!   free text (a 32-hex uuid or a 3-character tag) resolved by the daemon.
//! - Must not: decide anything. Which commands mutate is
//!   [`SyncCommand::is_mutating`]; the CLI requires the `human` namespace for
//!   those and the daemon admits only the elected owner's peer UID, then
//!   records `operator:local-user:<uid>` provenance.
//! - Role: one new `Command::Sync` / `CommandResult::Sync` pair, so a new sync
//!   operator command is a variant here, not a new wire command.

use serde::{Deserialize, Serialize};

/// Longest node name or dial destination a request may carry, in characters.
pub const MAX_NAME_CHARS: usize = 1024;
/// Most `--log` selectors one resync names.
pub const MAX_RESYNC_LOGS: usize = 16;
/// Largest page of `sync unapplied list`.
pub const MAX_UNAPPLIED_PAGE: u16 = 64;
/// The page size when the CLI does not ask for one.
pub const DEFAULT_UNAPPLIED_PAGE: u16 = 32;
/// Most per-log rows `sync status` lists.
pub const MAX_STATUS_LOGS: usize = 64;

/// One operator or status request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SyncCommand {
    PeerAdd {
        ssh_dest: String,
        remote_command: Option<String>,
    },
    /// A dial destination or a node (uuid or tag).
    PeerForget {
        target: String,
    },
    PeerList,
    /// `log` narrows the per-log rows to one node (uuid or tag).
    Status {
        log: Option<String>,
    },
    Verify,
    /// Unapplied entries after `after` (an `ingest_seq`), at most `limit`.
    UnappliedList {
        after: Option<u64>,
        limit: u16,
    },
    UnappliedReplay,
    Resync {
        from: String,
        logs: Vec<String>,
    },
    CloneCleared,
    /// Resolve the address `target` on machine `machine` (an `@MACHINE`
    /// suffix: a node uuid or its prefix, a tag or a host name). A read: the
    /// home node's daemon resolves it against its own live view.
    ResolveAddress {
        #[serde(default)]
        machine: String,
        #[serde(default)]
        target: crate::sync::frame::AddrTarget,
    },
    /// Whether the remote seat `seat` can take a handoff now, asked of its
    /// home node (TRUST-POLICY N15). A read; answered as an
    /// [`AddressReply::Remote`].
    CheckRemoteSeat {
        #[serde(default)]
        seat: String,
    },
}

impl SyncCommand {
    /// Whether the command changes state: these are operator-gated.
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::PeerAdd { .. }
                | Self::PeerForget { .. }
                | Self::UnappliedReplay
                | Self::Resync { .. }
                | Self::CloneCleared
        )
    }

    /// Shape checks that need no store.
    pub fn validate(&self) -> Result<(), &'static str> {
        fn text(value: &str) -> Result<(), &'static str> {
            if value.is_empty() || value.chars().count() > MAX_NAME_CHARS {
                Err("sync name length out of range")
            } else if value.chars().any(char::is_control) {
                Err("sync name contains control characters")
            } else {
                Ok(())
            }
        }
        match self {
            Self::PeerAdd {
                ssh_dest,
                remote_command,
            } => {
                text(ssh_dest)?;
                remote_command.as_deref().map_or(Ok(()), text)
            }
            Self::PeerForget { target } => text(target),
            Self::Status { log } => log.as_deref().map_or(Ok(()), text),
            Self::UnappliedList { limit, .. } if *limit == 0 || *limit > MAX_UNAPPLIED_PAGE => {
                Err("unapplied page size out of range")
            }
            Self::Resync { from, logs } => {
                if logs.len() > MAX_RESYNC_LOGS {
                    return Err("too many --log selectors");
                }
                text(from)?;
                logs.iter().map(String::as_str).try_for_each(text)
            }
            Self::ResolveAddress { machine, .. } => text(machine),
            Self::CheckRemoteSeat { seat } => text(seat),
            Self::PeerList
            | Self::Verify
            | Self::UnappliedList { .. }
            | Self::UnappliedReplay
            | Self::CloneCleared => Ok(()),
        }
    }
}

/// One dial target, as `peer list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRow {
    pub target_id: i64,
    pub ssh_dest: String,
    pub source: String,
    pub enabled: bool,
    pub state: String,
    pub stopped_reason: Option<String>,
    pub remote_command: Option<String>,
    /// The learned node's tag, else its uuid prefix.
    pub node: Option<String>,
    /// `reachable`, `unreachable` or `unknown` (no node learned yet).
    pub reach: String,
    pub attempts: u32,
    pub last_error: Option<String>,
}

/// One known peer node, as `peer list` shows it (ht-xg88 node status).
/// Every field is display data: nothing routes, dials or attributes by it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRow {
    /// The node's tag, else its uuid prefix.
    #[serde(default)]
    pub node: String,
    /// Its host label: the machine name an `@MACHINE` address matches
    /// (captured at the node's genesis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// `linked`, `stalled`, `reachable` or `unreachable`.
    #[serde(default)]
    pub state: String,
    /// With `stalled`: which of `app`, `tx`, `rx`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stalled: Vec<String>,
    /// The last frame of any kind (unix ms); `None` when never heard from.
    #[serde(default)]
    pub last_contact_at: Option<i64>,
    /// The last frame its coordinator sent (unix ms).
    #[serde(default)]
    pub app_seen_at: Option<i64>,
    /// Since when it has an active link (unix ms).
    #[serde(default)]
    pub linked_since: Option<i64>,
    /// Its self-reported daemon version, last known.
    #[serde(default)]
    pub version: Option<String>,
    /// `[acked, head]`: it holds `acked` of our active log, whose head is
    /// `head` (last known).
    #[serde(default)]
    pub tx: Option<(u64, u64)>,
    /// `[have, head]`: we hold `have` of its active log, which it advertised
    /// at `head` (last known).
    #[serde(default)]
    pub rx: Option<(u64, u64)>,
    /// Open clone evidence names this node.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clone_suspected: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One log, as `sync status` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRow {
    /// The owning node's tag, else its uuid prefix.
    pub node: String,
    pub incarnation: u32,
    pub is_self: bool,
    pub counter: u64,
    /// The first 8 hex characters of the head hash.
    pub head: String,
    pub applied_through: u64,
    pub parked_unknown: u64,
    pub parked_dep: u64,
    /// Entries of a signed layout this node does not understand: stored and
    /// relayed byte-exactly, never applied. Omitted from JSON when zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub parked_layout: u64,
    /// Of the `parked_layout` entries, those whose layout this build has a
    /// parser for but whose promotion failed. Omitted from JSON when zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cannot_promote: u64,
    pub failed: u64,
    pub frozen_at: Option<u64>,
    /// Entries a peer advertises beyond `counter`; `None` when no active link
    /// advertises the log.
    pub lag: Option<u64>,
    pub skew_ahead_ms: u64,
    /// Why the log applies nothing further: `opaque_layout` (its next entry
    /// has a signed layout this node does not understand) or a
    /// `deps <node8>/<inc>@<c>` wait. `None` when nothing blocks it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_on: Option<String>,
}

/// One link, as `sync status` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRow {
    pub node: String,
    /// `active` or `standby`.
    pub mode: String,
    pub role: String,
    pub rx_logs: u32,
    pub unsolicited: u64,
    /// Logs parked on this link: the peer lacks `envelope.opaque.v1` and the
    /// next entry it wants of the log is opaque (not a count of entries).
    #[serde(default, alias = "withheld")]
    pub withheld_logs: u64,
    pub dest: Option<String>,
    pub last_error: Option<String>,
    /// Since when the link is up (unix ms).
    #[serde(default)]
    pub since: Option<i64>,
    /// The last inbound frame of any kind (unix ms).
    #[serde(default)]
    pub last_rx: Option<i64>,
    /// The last frame the peer's coordinator sent (unix ms).
    #[serde(default)]
    pub app_rx: Option<i64>,
    /// Logs the peer has armed a subscription for.
    #[serde(default)]
    pub subscriptions: u32,
    /// How far the peer is behind on our active log (`None` off an active
    /// link or before its first full `have`).
    #[serde(default)]
    pub peer_lag: Option<u64>,
    /// Which of `app`, `tx`, `rx` stalled on this link.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stalled: Vec<String>,
}

/// One open clone-evidence row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRow {
    pub ev_ix: i64,
    pub subject: String,
    pub kind: String,
    pub log: Option<String>,
    pub detail: String,
    pub detected_at: i64,
    pub last_seen_at: i64,
}

/// One log under repair (frozen, forked or re-issuing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairRow {
    pub log: String,
    pub frozen_at: Option<u64>,
    pub origin: Option<String>,
    pub evidence: u32,
    pub branches: u32,
    pub reissued: u32,
    pub pending: u32,
    /// Entries of the log that hold an unknown signed layout and cannot be
    /// re-issued (never silently absent from `pending`).
    #[serde(default)]
    pub cannot_reissue: u32,
}

/// Everything `sync status` reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReply {
    pub logs: Vec<LogRow>,
    /// Logs beyond [`MAX_STATUS_LOGS`] that were left out.
    pub logs_omitted: u64,
    pub links: Vec<LinkRow>,
    /// Logs the fork sink holds (no `want` for them).
    pub paused_logs: Vec<String>,
    pub repairs: Vec<RepairRow>,
    pub evidence: Vec<EvidenceRow>,
    /// Own logs a clone suspicion names without a rotation naming them.
    pub own_unknown_newer: Vec<String>,
    pub lineage_branches: Vec<String>,
    pub restore_successor: Option<String>,
    /// The two remedy lines, present while evidence about this node is open.
    pub remedies: Vec<String>,
    pub store_bytes: u64,
    /// Every want is paused until this unix-ms time (no disk headroom).
    pub headroom_until: Option<i64>,
    /// The coordinator did not answer; link rows are missing.
    pub coordinator_unavailable: bool,
    /// The discovery worker's state token; `None` when the daemon runs no
    /// discovery worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<String>,
}

/// One broken invariant, for `sync verify`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViolationRow {
    pub invariant: String,
    pub log: Option<String>,
    pub counter: Option<u64>,
    pub detail: String,
}

/// One parked or failed journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnappliedRow {
    pub ingest_seq: u64,
    pub log: String,
    pub counter: u64,
    pub kind: String,
    pub state: String,
    /// `parked_dep`: the entity it waits for; `failed`: the error.
    pub detail: Option<String>,
}

/// A bounded page of unapplied entries and the per-state totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnappliedPage {
    pub rows: Vec<UnappliedRow>,
    pub parked_unknown: u64,
    pub parked_dep: u64,
    pub failed: u64,
    /// The `after` for the next page, when there is more.
    pub next_after: Option<u64>,
}

/// What one bounded replay did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReply {
    pub attempted: u64,
    pub applied: u64,
    pub has_more: bool,
}

/// The daemon's answer to a [`SyncCommand`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SyncReply {
    PeerAdded {
        target_id: i64,
        ssh_dest: String,
    },
    PeerForgotten {
        disabled: u32,
    },
    Peers {
        rows: Vec<PeerRow>,
        /// The discovery worker's state token; `None` without a worker.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        discovery: Option<String>,
        /// Every known peer node's status (ht-xg88).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        nodes: Vec<NodeRow>,
    },
    Status(Box<StatusReply>),
    Verified {
        violations: Vec<ViolationRow>,
    },
    Unapplied(UnappliedPage),
    Replayed(ReplayReply),
    /// The request is recorded; the daemon stops and the next start rebuilds.
    ResyncRequested {
        from: String,
    },
    CloneCleared {
        cleared: u32,
    },
    /// The answer to [`SyncCommand::ResolveAddress`].
    Address(AddressReply),
}

/// One machine (node) as an address names it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineRow {
    /// 32 lowercase hex.
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub tag: String,
    /// Its `host_label` (the hostname it signed at genesis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// How the daemon resolved an `@MACHINE` address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum AddressReply {
    /// The machine is this node: the caller resolves the location locally.
    SelfNode,
    /// The machine is this node, but the location is the exact ID of a seat
    /// homed on another node (`home`, when known): refused (spec D2).
    SeatElsewhere {
        #[serde(default)]
        seat: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home: Option<MachineRow>,
    },
    /// No node matches; `known` lists up to eight known nodes.
    UnknownMachine {
        #[serde(default)]
        known: Vec<MachineRow>,
    },
    /// Several nodes match.
    AmbiguousMachine {
        #[serde(default)]
        matches: Vec<MachineRow>,
    },
    /// The home node answered (or could not).
    Remote {
        #[serde(default)]
        node: MachineRow,
        #[serde(default)]
        answer: RemoteAddress,
    },
    #[serde(other)]
    Unknown,
}

/// What became of the request to the home node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum RemoteAddress {
    /// The home node's own outcome. A `seat` outcome names a seat this node
    /// already holds (the daemon waited for it to replicate).
    Answered {
        #[serde(default)]
        outcome: crate::sync::frame::AddrOutcome,
    },
    /// No live direct link to the node; its last contact (unix ms).
    Unreachable {
        #[serde(default)]
        last_contact_at: Option<i64>,
    },
    /// The link does not carry `addr.resolve.v1`.
    NoCap {
        #[serde(default)]
        version: Option<String>,
        /// The missing capability (empty from an older daemon: `addr.resolve.v1`).
        #[serde(default)]
        cap: String,
    },
    /// No answer within the wait.
    Timeout {
        #[serde(default)]
        secs: u64,
    },
    /// The node named a seat this node does not hold yet.
    NotReplicated {
        #[serde(default)]
        seat: String,
    },
    #[default]
    #[serde(other)]
    Unknown,
}
