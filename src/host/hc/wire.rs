//! Tolerant serde types for herdr-core replies and notifications.
//!
//! Every field a reply may omit or null carries `#[serde(default)]`, and no
//! type denies unknown fields: a newer core adds fields and event types
//! without breaking this client. Unknown event types decode into
//! [`HcEventKind::Other`] and unknown notification methods into
//! [`HcEvent::Other`], so a caller can skip them.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// A core's identity: `id` is permanent until its state is wiped.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct CoreInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// The `hello` result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct HelloReply {
    #[serde(default)]
    pub core: CoreInfo,
    #[serde(default)]
    pub methods: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub generation: u32,
    #[serde(default)]
    pub version: String,
}

/// One 7501 status record; `null` on a terminal that never reported.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct HcStatus {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub progress: Option<u8>,
}

/// What runs in a live terminal now.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct HcForeground {
    #[serde(default)]
    pub pid: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: String,
}

/// A terminal record, live or finished.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct HcTerminal {
    #[serde(default)]
    pub address: String,
    /// `"live"` or `"finished"`; any other string is kept as sent.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub status: Option<HcStatus>,
    #[serde(default)]
    pub turn: u64,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub foreground: Option<HcForeground>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub keep: bool,
    /// Why the program ended or is pending; an unknown string is kept.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub signal: Option<i32>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub finished_ms: Option<u64>,
    #[serde(default)]
    pub status_ms: Option<u64>,
}

/// The `doc.get` result: an opaque document and its version (0: never written).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct AppDoc {
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub version: u64,
}

/// A fact-change event type of `events.subscribe`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HcEventKind {
    Created,
    Lost,
    Restarted,
    Keep,
    Finished,
    Removed,
    Status,
    Turn,
    Title,
    Labels,
    Size,
    Doc,
    /// A type this client does not know; skipped by callers.
    Other(String),
}

impl HcEventKind {
    fn parse(kind: &str) -> Self {
        match kind {
            "created" => Self::Created,
            "lost" => Self::Lost,
            "restarted" => Self::Restarted,
            "keep" => Self::Keep,
            "finished" => Self::Finished,
            "removed" => Self::Removed,
            "status" => Self::Status,
            "turn" => Self::Turn,
            "title" => Self::Title,
            "labels" => Self::Labels,
            "size" => Self::Size,
            "doc" => Self::Doc,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// A notification line from an `events.subscribe` connection.
#[derive(Debug, Clone, PartialEq)]
pub enum HcEvent {
    /// A fact change, `seq` ordered.
    Event {
        seq: u64,
        kind: HcEventKind,
        terminal: Option<String>,
        data: Value,
    },
    Heartbeat {
        seq: u64,
    },
    /// The subscriber fell behind: a fresh snapshot replaces its state.
    Missed {
        seq: u64,
        terminals: Vec<HcTerminal>,
    },
    /// A notification method this client does not know.
    Other {
        method: String,
    },
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
struct EventParams {
    #[serde(default)]
    seq: u64,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    terminal: Option<String>,
    #[serde(default)]
    data: Value,
}

#[derive(Deserialize)]
struct SeqParams {
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    terminals: Vec<HcTerminal>,
}

impl HcEvent {
    /// Decodes one notification object (a JSON-RPC message without `id`).
    /// Anything unrecognised becomes [`HcEvent::Other`]; a known method with
    /// unreadable params is `None`.
    pub fn from_notification(message: &Value) -> Option<Self> {
        let envelope = Envelope::deserialize(message).ok()?;
        match envelope.method.as_str() {
            "event" => {
                let p = EventParams::deserialize(&envelope.params).ok()?;
                Some(Self::Event {
                    seq: p.seq,
                    kind: HcEventKind::parse(&p.kind),
                    terminal: p.terminal,
                    data: p.data,
                })
            }
            "heartbeat" => {
                let p = SeqParams::deserialize(&envelope.params).ok()?;
                Some(Self::Heartbeat { seq: p.seq })
            }
            "missed" => {
                let p = SeqParams::deserialize(&envelope.params).ok()?;
                Some(Self::Missed {
                    seq: p.seq,
                    terminals: p.terminals,
                })
            }
            other => Some(Self::Other {
                method: other.to_owned(),
            }),
        }
    }

    /// Whether a caller skips this event (an unknown method or event type).
    pub fn is_unknown(&self) -> bool {
        matches!(
            self,
            Self::Other { .. }
                | Self::Event {
                    kind: HcEventKind::Other(_),
                    ..
                }
        )
    }
}
