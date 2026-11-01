//! JSON-RPC 2.0 line client for the herdr-core socket, one connection per call.
//!
//! Each call connects, captures the peer witness, writes one request line,
//! reads reply lines up to the response carrying its id, and closes. The only
//! methods are `hello`, `terminal.get`, `events.subscribe` (a one-shot
//! snapshot), `doc.get` and `terminal.run` (launch only, with a fixed
//! parameter set). There is deliberately no way to send
//! `terminal.list` (it prunes and fans out to machines), `doc.set`, or a
//! `caller` param (it mutates hc's parent bookkeeping): [`HcClient::call`] is
//! private and its params are built here.

use super::wire::{AppDoc, HcEvent, HcTerminal, HelloReply};
use crate::host::continuity::{CaptureError, LocalEndpointWitness, capture_peer_witness};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

/// Default budget for one call, connect to last reply byte.
pub const CALL_DEADLINE: Duration = Duration::from_secs(5);
/// Budget for `terminal.run`: herdr-core waits up to 10 s for the holder to
/// spawn the program before it answers `spawn_failed`.
pub const RUN_DEADLINE: Duration = Duration::from_secs(15);
/// A reply line larger than this is refused.
const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;
/// Notification lines read before the response before giving up.
const MAX_PRELUDE_LINES: usize = 4096;

/// What a launch asks of `terminal.run`. There is deliberately no `caller`,
/// hc `name`, template or first message here: the fixed parameters
/// (`no_notify`, `outlive_parent`, `no_parent`, `keep: false`) are set by
/// [`HcClient::terminal_run`] itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRequest {
    pub argv: Vec<String>,
    pub cwd: String,
    pub env: std::collections::BTreeMap<String, String>,
    pub labels: std::collections::BTreeMap<String, String>,
    /// An explicit parent terminal (`tN`); `None` starts the terminal parentless.
    pub parent: Option<String>,
}

#[derive(Debug)]
pub enum HcError {
    Io(std::io::Error),
    Timeout,
    /// The reply was not a decodable JSON-RPC response.
    Decode(String),
    /// The core answered with an error object.
    Rpc {
        code: i64,
        kind: Option<String>,
        message: String,
    },
    Witness(CaptureError),
    /// The peer is not the one the caller expected.
    WitnessChanged,
}

impl fmt::Display for HcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "hc socket: {e}"),
            Self::Timeout => write!(f, "hc call timed out"),
            Self::Decode(e) => write!(f, "hc reply: {e}"),
            Self::Rpc {
                code,
                kind,
                message,
            } => write!(
                f,
                "hc error {code} {}: {message}",
                kind.as_deref().unwrap_or("-")
            ),
            Self::Witness(e) => write!(f, "hc peer witness: {e:?}"),
            Self::WitnessChanged => write!(f, "hc peer changed"),
        }
    }
}

impl std::error::Error for HcError {}

impl From<std::io::Error> for HcError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// A decoded result and the witness of the peer that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct HcReply<T> {
    pub value: T,
    pub witness: LocalEndpointWitness,
}

/// The `events.subscribe` snapshot: `seq` is the sequence after which events
/// follow. `events` are notifications that preceded the response, known ones
/// only; `skipped` counts the unknown ones dropped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub seq: u64,
    pub terminals: Vec<HcTerminal>,
    pub events: Vec<HcEvent>,
    pub skipped: usize,
}

#[derive(Deserialize)]
struct SnapshotResult {
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    terminals: Vec<HcTerminal>,
}

#[derive(Debug, Clone)]
pub struct HcClient {
    socket: PathBuf,
    deadline: Duration,
    expected: Option<LocalEndpointWitness>,
}

impl HcClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            deadline: CALL_DEADLINE,
            expected: None,
        }
    }

    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }

    /// Every call fails with [`HcError::WitnessChanged`] unless its peer's
    /// witness equals `witness`.
    pub fn expecting(mut self, witness: LocalEndpointWitness) -> Self {
        self.expected = Some(witness);
        self
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub async fn hello(&self) -> Result<HcReply<HelloReply>, HcError> {
        self.typed("hello", json!({})).await
    }

    pub async fn terminal_get(&self, terminal: &str) -> Result<HcReply<HcTerminal>, HcError> {
        self.typed("terminal.get", json!({"terminal": terminal}))
            .await
    }

    /// Subscribes with an empty filter, reads the snapshot and closes.
    pub async fn events_subscribe_snapshot(&self) -> Result<HcReply<Snapshot>, HcError> {
        let (result, prelude, witness) = self.call("events.subscribe", json!({})).await?;
        let body =
            SnapshotResult::deserialize(&result).map_err(|e| HcError::Decode(e.to_string()))?;
        let mut snapshot = Snapshot {
            seq: body.seq,
            terminals: body.terminals,
            ..Snapshot::default()
        };
        for message in &prelude {
            match HcEvent::from_notification(message) {
                Some(event) if !event.is_unknown() => snapshot.events.push(event),
                _ => snapshot.skipped += 1,
            }
        }
        Ok(HcReply {
            value: snapshot,
            witness,
        })
    }

    pub async fn doc_get(&self, name: &str) -> Result<HcReply<AppDoc>, HcError> {
        self.typed("doc.get", json!({"name": name})).await
    }

    /// Starts a terminal. The parameters are fixed here: `no_notify`,
    /// `outlive_parent` and `no_parent` true, `keep` false (any finish is
    /// final), no `caller`, no `name`, no `first_message`. The caller gives
    /// the client a deadline of at least [`RUN_DEADLINE`] (a spawn may take
    /// that long).
    pub async fn terminal_run(&self, request: &RunRequest) -> Result<HcReply<HcTerminal>, HcError> {
        let mut params = json!({
            "argv": request.argv,
            "cwd": request.cwd,
            "env": request.env,
            "labels": request.labels,
            "keep": false,
            "no_notify": true,
            "outlive_parent": true,
            "no_parent": true,
        });
        if let Some(parent) = &request.parent {
            params["parent"] = json!(parent);
        }
        self.typed("terminal.run", params).await
    }

    async fn typed<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<HcReply<T>, HcError> {
        let (result, _, witness) = self.call(method, params).await?;
        let value = T::deserialize(&result).map_err(|e| HcError::Decode(e.to_string()))?;
        Ok(HcReply { value, witness })
    }

    /// One exchange: the result, the notification lines before it, the witness.
    async fn call(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(Value, Vec<Value>, LocalEndpointWitness), HcError> {
        tokio::time::timeout(self.deadline, self.exchange(method, params))
            .await
            .map_err(|_| HcError::Timeout)?
    }

    async fn exchange(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(Value, Vec<Value>, LocalEndpointWitness), HcError> {
        let mut stream = UnixStream::connect(&self.socket).await?;
        let witness = capture_peer_witness(&stream, &self.socket).map_err(HcError::Witness)?;
        if self.expected.as_ref().is_some_and(|w| w != &witness) {
            return Err(HcError::WitnessChanged);
        }
        let mut line = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
        }))
        .map_err(|e| HcError::Decode(e.to_string()))?;
        line.push(b'\n');
        stream.write_all(&line).await?;
        let mut reader = BufReader::new(stream);
        let mut prelude = Vec::new();
        loop {
            let mut text = String::new();
            let read = (&mut reader)
                .take(MAX_LINE_BYTES)
                .read_line(&mut text)
                .await?;
            if read == 0 || !text.ends_with('\n') {
                return Err(HcError::Decode(
                    "connection closed before a response".into(),
                ));
            }
            let message: Value =
                serde_json::from_str(&text).map_err(|e| HcError::Decode(e.to_string()))?;
            if message.get("id").is_none_or(Value::is_null) {
                if prelude.len() >= MAX_PRELUDE_LINES {
                    return Err(HcError::Decode("too many notifications".into()));
                }
                prelude.push(message);
                continue;
            }
            if let Some(error) = message.get("error").filter(|e| !e.is_null()) {
                return Err(HcError::Rpc {
                    code: error["code"].as_i64().unwrap_or_default(),
                    kind: error["data"]["kind"].as_str().map(str::to_owned),
                    message: error["message"].as_str().unwrap_or_default().to_owned(),
                });
            }
            let result = message
                .get("result")
                .cloned()
                .ok_or_else(|| HcError::Decode("response without result".into()))?;
            return Ok((result, prelude, witness));
        }
    }
}
