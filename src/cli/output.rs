//! Selected CLI output and exact byte budget enforcement.

use crate::protocol::{
    output::{OutputFormat, OutputSpec, encode_selected},
    results::{ApiError, CommandResult, ErrorCode},
};
use std::{
    cell::Cell,
    io::{self, Write},
};

/// How a text-format result is presented. `--json` always wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presentation {
    /// Human form when stdout is a terminal, machine form otherwise.
    #[default]
    Auto,
    /// `--human`: the human form even when stdout is not a terminal.
    Human,
    /// `--machine`: the established `key: value` machine text form.
    Machine,
}

thread_local! {
    static STDOUT_IS_TERMINAL: Cell<bool> = const { Cell::new(false) };
    static HUMAN: Cell<bool> = const { Cell::new(false) };
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
                Presentation::Auto => STDOUT_IS_TERMINAL.with(Cell::get),
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

/// Emit precisely the bytes measured by the shared encoder (or, for a person at
/// a terminal, its human rendering; the budget is still enforced on the
/// machine bytes). The caller keeps any durable intent pending until this
/// returns successfully.
pub fn write_selected<W: Write>(
    result: &CommandResult,
    spec: &OutputSpec,
    max_bytes: u32,
    writer: &mut W,
) -> Result<usize, OutputError> {
    let bytes = encode_selected(result, spec)?;
    if bytes.len() > max_bytes as usize {
        return Err(OutputError::Api(ApiError {
            code: ErrorCode::InvalidBudget,
            detail: "selected output exceeds byte budget".into(),
            restart_argv: None,
            required_minimum_bytes: Some(bytes.len().try_into().unwrap_or(u32::MAX)),
        }));
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

#[cfg(test)]
mod tests {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/cli/output.rs"));
}
