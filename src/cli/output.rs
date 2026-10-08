//! Selected CLI output and exact byte budget enforcement.

use crate::protocol::{
    output::{OutputFormat, OutputSpec, encode_selected},
    results::{ApiError, CommandResult},
};
use crate::view::escape::{Context, escape_for_terminal};
use std::{
    borrow::Cow,
    cell::Cell,
    ffi::OsString,
    io::{self, Write},
};

/// How a text-format result is presented. `--json` always wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presentation {
    /// Human form when stdout is a terminal and no agent-harness marker is
    /// present ([`HARNESS_MARKERS`]), machine form otherwise.
    #[default]
    Auto,
    /// `--human`: the human form even when stdout is not a terminal.
    Human,
    /// `--machine`: the established `key: value` machine text form.
    Machine,
}

thread_local! {
    static STDOUT_IS_TERMINAL: Cell<bool> = const { Cell::new(false) };
    static HARNESS_MARKED: Cell<bool> = const { Cell::new(false) };
    static HUMAN: Cell<bool> = const { Cell::new(false) };
}

/// Environment variables an agent harness sets for the subprocesses of its
/// shell tool, so a harness that runs commands in a PTY (Codex unified exec
/// with `tty: true`) is still recognized as an agent, not a person.
/// Verified: `CLAUDECODE` (Claude Code, every captured hook env under
/// `docs/evidence/claude-28*-hook-capture/payloads/*.env-names.txt`);
/// `CODEX_THREAD_ID` (in the shell-subprocess environment allow-list of the
/// installed Codex 0.159.3 binary) and `CODEX_SANDBOX` /
/// `CODEX_SANDBOX_NETWORK_DISABLED` (set by Codex's exec policy for sandboxed
/// commands; both names are in the same binary). `CODEX_MANAGED_BY_NPM` is
/// not listed: only an npm-launched Codex sets it, and no capture shows it in
/// tool subprocesses.
pub const HARNESS_MARKERS: [&str; 4] = [
    "CLAUDECODE",
    "CODEX_THREAD_ID",
    "CODEX_SANDBOX",
    "CODEX_SANDBOX_NETWORK_DISABLED",
];

/// Whether `env` carries a non-empty agent-harness marker.
pub fn harness_marked(env: impl Fn(&str) -> Option<OsString>) -> bool {
    HARNESS_MARKERS
        .iter()
        .any(|name| env(name).is_some_and(|value| !value.is_empty()))
}

/// Record whether this process runs under an agent harness. Only the process
/// entrypoint sets it; library and test callers default to unmarked.
pub fn set_harness_marked(marked: bool) {
    HARNESS_MARKED.with(|cell| cell.set(marked));
}

/// Record whether this thread's CLI writer is an interactive terminal. Only
/// the process entrypoint sets it; library and test callers default to the
/// machine form.
pub fn set_stdout_is_terminal(terminal: bool) {
    STDOUT_IS_TERMINAL.with(|cell| cell.set(terminal));
}

/// Whether this thread's CLI writer was recorded as an interactive terminal.
pub fn stdout_is_terminal() -> bool {
    STDOUT_IS_TERMINAL.with(Cell::get)
}

/// Selects the human renderer for this thread's command run until dropped.
pub struct PresentationGuard(bool);

impl PresentationGuard {
    pub fn enter(presentation: Presentation, spec: &OutputSpec) -> Self {
        let human = spec.format == OutputFormat::Text
            && match presentation {
                Presentation::Human => true,
                Presentation::Machine => false,
                // A harness marker beats the terminal check: a PTY harness is an agent.
                Presentation::Auto => {
                    STDOUT_IS_TERMINAL.with(Cell::get) && !HARNESS_MARKED.with(Cell::get)
                }
            };
        Self(HUMAN.with(|cell| cell.replace(human)))
    }
}

impl Drop for PresentationGuard {
    fn drop(&mut self) {
        HUMAN.with(|cell| cell.set(self.0));
    }
}

/// Whether this run selected the human presentation.
pub fn human_active() -> bool {
    HUMAN.with(Cell::get)
}

/// The bytes a CLI command emits for `result`: the selected machine encoding,
/// or the human form when this run selected it and one exists for the kind.
pub fn emitted_bytes(result: &CommandResult, spec: &OutputSpec) -> Result<Vec<u8>, ApiError> {
    if HUMAN.with(Cell::get)
        && spec.format == OutputFormat::Text
        && let Some(text) = super::human::render(result, spec)
    {
        // Every human renderer escapes peer text; this catches one that does
        // not (debug builds and tests).
        debug_assert!(
            matches!(
                escape_for_terminal(&text, Context::MultiLine),
                Cow::Borrowed(_)
            ),
            "human output holds text that escape_for_terminal would escape"
        );
        return Ok(text.into_bytes());
    }
    encode_selected(result, spec)
}

#[derive(Debug)]
pub enum OutputError {
    Api(ApiError),
    Io(io::Error),
}

impl From<ApiError> for OutputError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

/// Write the selected encoding of `result` and return the number of bytes
/// written. For a machine consumer those are exactly the bytes the shared
/// encoder measured. For a person at a terminal they are the human rendering
/// instead, which is not measured: `max_bytes` is enforced on the machine
/// encoding only, so the human text may be longer or shorter than the budget.
/// The caller keeps any durable intent pending until this returns
/// successfully.
pub fn write_selected<W: Write>(
    result: &CommandResult,
    spec: &OutputSpec,
    max_bytes: u32,
    writer: &mut W,
) -> Result<usize, OutputError> {
    let bytes = encode_selected(result, spec)?;
    if bytes.len() > max_bytes as usize {
        return Err(OutputError::Api(
            ApiError::invalid_budget("selected output exceeds byte budget")
                .with_required_minimum_bytes(bytes.len().try_into().unwrap_or(u32::MAX)),
        ));
    }
    let bytes = if HUMAN.with(Cell::get) {
        emitted_bytes(result, spec)?
    } else {
        bytes
    };
    writer.write_all(&bytes).map_err(OutputError::Io)?;
    writer.flush().map_err(OutputError::Io)?;
    Ok(bytes.len())
}

/// Canonical CLI-only annotation of selected read records. This never changes
/// the v1 result, saved summary content, or delivery progress.
#[derive(Default)]
pub(crate) struct ReadModes {
    lazy: Vec<crate::protocol::ids::MessageId>,
}

impl ReadModes {
    pub(crate) fn lookup<C: crate::ports::LocalClient + ?Sized>(
        result: &CommandResult,
        spec: &OutputSpec,
        client: &C,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<Self, ApiError> {
        use crate::protocol::{
            capabilities::MESSAGE_DELIVERY_MODES,
            commands::{Command, DeliveryMode, MessageDeliveryModesQuery},
            output::selected_result,
            results::SearchHit,
        };
        if spec.format != OutputFormat::Text {
            return Ok(Self::default());
        }
        let selected = selected_result(result, spec);
        let ids: Vec<_> = match &selected {
            CommandResult::History(page) => page.items.iter().map(|m| m.message.clone()).collect(),
            CommandResult::Message(detail) => vec![detail.summary.message.clone()],
            CommandResult::Search(search) => search
                .matches
                .items
                .iter()
                .filter_map(|hit| match hit {
                    SearchHit::Body(m) => Some(m.message.clone()),
                    SearchHit::Topic(_) => None,
                })
                .collect(),
            _ => return Ok(Self::default()),
        };
        if ids.is_empty() || !client.supports_capability(MESSAGE_DELIVERY_MODES, budget) {
            return Ok(Self::default());
        }
        let mut lazy = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .collect();
        for batch in ids.chunks(100) {
            let result = match client.call(
                Command::MessageDeliveryModes(MessageDeliveryModesQuery {
                    messages: batch.to_vec(),
                }),
                budget,
            ) {
                // An optional read extension may be unavailable even when a
                // cached capability list advertises it. Keep the legacy read
                // rendering; never infer a mode or perform a mutation.
                Err(error) if error.code == crate::protocol::results::ErrorCode::Unsupported => {
                    return Ok(Self::default());
                }
                result => result?,
            };
            let CommandResult::MessageDeliveryModes(modes) = result else {
                return Err(ApiError::store_corrupt("daemon returned no message modes"));
            };
            if modes.len() != batch.len()
                || modes
                    .iter()
                    .zip(batch)
                    .any(|(mode, id)| &mode.message != id)
            {
                return Err(ApiError::store_corrupt(
                    "daemon returned mismatched message modes",
                ));
            }
            lazy.extend(
                modes
                    .into_iter()
                    .filter(|mode| mode.delivery_mode == DeliveryMode::Lazy)
                    .map(|mode| mode.message),
            );
        }
        Ok(Self { lazy })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lazy.is_empty()
    }

    /// A service-generated row preceding the unchanged selected encoding.
    /// Peer text cannot fabricate this column-zero row (body lines are indented,
    /// and history/search peer fields are escaped by the established encoder).
    pub(crate) fn annotate(&self, bytes: Vec<u8>) -> Vec<u8> {
        if self.is_empty() {
            return bytes;
        }
        let mut out = Vec::new();
        for id in &self.lazy {
            out.extend_from_slice(format!("[lazy] {}\n", id.as_str()).as_bytes());
        }
        out.extend_from_slice(&bytes);
        out
    }

    pub(crate) fn write<W: Write + ?Sized>(
        &self,
        bytes: Vec<u8>,
        max_bytes: u32,
        writer: &mut W,
    ) -> Result<(), OutputError> {
        let bytes = self.annotate(bytes);
        if !self.is_empty() && bytes.len() > max_bytes as usize {
            return Err(OutputError::Api(
                ApiError::invalid_budget("annotated read exceeds byte budget")
                    .with_required_minimum_bytes(bytes.len().try_into().unwrap_or(u32::MAX)),
            ));
        }
        writer.write_all(&bytes).map_err(OutputError::Io)?;
        writer.flush().map_err(OutputError::Io)
    }
}

#[cfg(test)]
mod tests {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/cli/output.rs"));
}
