//! Selected CLI output and exact byte budget enforcement.

use crate::protocol::{
    output::{OutputFormat, OutputSpec, encode_selected},
    results::{ApiError, CommandResult},
};
use crate::view::escape::{Context, escape_for_terminal};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
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
    static INBOX: RefCell<Option<InboxInvocation>> = const { RefCell::new(None) };
}

/// Local continuation hints, captured before an omitted read selector is resolved.
/// They grant no authority: every executed continuation is resolved normally.
#[derive(Clone)]
struct InboxInvocation {
    actor: super::actor_route::InvocationActor,
    seat: Option<crate::protocol::ids::SeatId>,
    own_text: bool,
    presentation: Presentation,
    spec: OutputSpec,
    page: crate::protocol::pagination::PageRequest,
}

pub(crate) struct InboxInvocationGuard(Option<InboxInvocation>);

impl InboxInvocationGuard {
    pub(crate) fn enter(
        parsed: &super::commands::ParsedCli,
        seat: Option<crate::protocol::ids::SeatId>,
    ) -> Self {
        use crate::protocol::commands::Command;
        let invocation = match &parsed.action {
            super::commands::CliAction::Wire(
                Command::Inbox(q) | Command::InboxBatch(q) | Command::InboxBatchV2(q),
            ) => Some(InboxInvocation {
                actor: parsed.actor,
                seat: q
                    .seat
                    .clone()
                    .or(seat)
                    .or_else(|| parsed.cooperative.as_ref().map(|s| s.seat.clone())),
                own_text: parsed.caller_read_default
                    && parsed.output.format == OutputFormat::Text
                    && parsed.presentation != Presentation::Machine,
                presentation: parsed.presentation,
                spec: parsed.output.clone(),
                page: q.page.clone(),
            }),
            _ => None,
        };
        Self(INBOX.with(|slot| slot.replace(invocation)))
    }
}

impl Drop for InboxInvocationGuard {
    fn drop(&mut self) {
        INBOX.with(|slot| slot.replace(self.0.take()));
    }
}

fn inbox_continuation(result: &CommandResult) -> CommandResult {
    let mut result = result.clone();
    INBOX.with(|slot| {
        let invocation = slot.borrow();
        let Some(context) = invocation.as_ref() else {
            return;
        };
        let next = match &mut result {
            CommandResult::Inbox(p) => &mut p.next_argv,
            CommandResult::InboxBatch(p) => &mut p.next_argv,
            CommandResult::InboxBatchV2(p) => &mut p.next_argv,
            _ => return,
        };
        let Some(argv) = next else { return };
        let mut state = context.spec.context.state_dir.clone();
        let mut host = context.spec.context.host.clone();
        let mut seat = context.seat.as_ref().map(|s| s.as_str().to_owned());
        let mut args = argv.iter().skip(1);
        let mut rest = Vec::new();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "human" | "--json" | "--human" | "--machine" => {}
                "--state-dir" => {
                    if let Some(value) = args.next() {
                        state.get_or_insert_with(|| value.clone());
                    }
                }
                "--host-endpoint" => {
                    if let Some(value) = args.next() {
                        host.get_or_insert_with(|| value.clone());
                    }
                }
                "--seat" => {
                    if let Some(value) = args.next() {
                        seat.get_or_insert_with(|| value.clone());
                    }
                }
                "--max-bytes" | "--limit" => {
                    args.next();
                }
                _ => rest.push(arg.clone()),
            }
        }
        let mut normalized = vec![argv[0].clone()];
        if context.actor == super::actor_route::InvocationActor::Human {
            normalized.push("human".into());
        }
        if let Some(state) = state {
            normalized.extend(["--state-dir".into(), state]);
        }
        if let Some(host) = host {
            normalized.extend(["--host-endpoint".into(), host]);
        }
        if context.spec.format == OutputFormat::Json {
            normalized.push("--json".into());
        }
        match context.presentation {
            Presentation::Human => normalized.push("--human".into()),
            Presentation::Machine => normalized.push("--machine".into()),
            Presentation::Auto => {}
        }
        normalized.extend(rest);
        if !context.own_text
            && let Some(seat) = seat
        {
            normalized.extend(["--seat".into(), seat]);
        }
        normalized.extend([
            "--limit".into(),
            context.page.limit.to_string(),
            "--max-bytes".into(),
            context.page.max_bytes.to_string(),
        ]);
        *argv = normalized;
    });
    result
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
    let result = inbox_continuation(result);
    let result = &result;
    let _namespace = crate::protocol::output::CommandNamespaceGuard::enter(
        crate::protocol::output::result_human_commands(result),
    );
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
/// encoder measured. Human inbox output also measures the final rendered bytes
/// against `max_bytes`. Other human renderers retain the machine-encoding budget
/// check only, so their final text may be longer or shorter than the budget.
/// The caller keeps any durable intent pending until this returns
/// successfully.
pub fn write_selected<W: Write>(
    result: &CommandResult,
    spec: &OutputSpec,
    max_bytes: u32,
    writer: &mut W,
) -> Result<usize, OutputError> {
    let decorated = inbox_continuation(result);
    let bytes = encode_selected(&decorated, spec)?;
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
    if matches!(
        result,
        CommandResult::Inbox(_) | CommandResult::InboxBatch(_) | CommandResult::InboxBatchV2(_)
    ) && bytes.len() > max_bytes as usize
    {
        return Err(OutputError::Api(
            ApiError::invalid_budget("selected inbox output exceeds byte budget")
                .with_required_minimum_bytes(bytes.len().try_into().unwrap_or(u32::MAX)),
        ));
    }
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
            results::{MessageContent, MessageKind, SearchHit},
        };
        if spec.format != OutputFormat::Text {
            return Ok(Self::default());
        }
        let selected = selected_result(result, spec);
        let ids: Vec<_> = match &selected {
            // System events may be canonically published by a manifest before
            // their physical message rows exist. Delivery mode applies only
            // to ordinary content, so do not query it for warning/info rows.
            CommandResult::History(page) => page
                .items
                .iter()
                .filter(|m| m.kind == MessageKind::Ordinary)
                .map(|m| m.message.clone())
                .collect(),
            CommandResult::Message(detail) => match &detail.content {
                MessageContent::Ordinary { .. } => vec![detail.summary.message.clone()],
                MessageContent::System { .. } => Vec::new(),
            },
            CommandResult::Search(search) => search
                .matches
                .items
                .iter()
                .filter_map(|hit| match hit {
                    SearchHit::Body(m) if m.kind == MessageKind::Ordinary => {
                        Some(m.message.clone())
                    }
                    SearchHit::Body(_) | SearchHit::Topic(_) => None,
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
