//! Bounded UTF-8 message input from one explicit source.

use crate::protocol::results::ApiError;
use std::io::Read;

/// The store's body limit is the single source of truth, so the CLI refuses an
/// oversize body before it is journaled as a durable intent.
pub use crate::store::messages::MAX_BODY_BYTES;

fn invalid(detail: impl Into<String>) -> ApiError {
    ApiError::invalid_request(detail)
}

pub fn read_body(
    inline: Option<String>,
    file: Option<String>,
    stdin: bool,
) -> Result<String, ApiError> {
    // Touch process stdin (its process-wide lock) only when it is the source:
    // an inline or file body must never wait on whoever holds stdin.
    if stdin {
        read_body_from(inline, file, stdin, &mut std::io::stdin().lock())
    } else {
        read_body_from(inline, file, stdin, &mut std::io::empty())
    }
}

/// Injecting the stdin reader keeps byte and UTF-8 validation testable without
/// replacing process stdin or invoking a shell.
pub fn read_body_from(
    inline: Option<String>,
    file: Option<String>,
    stdin: bool,
    input: &mut dyn Read,
) -> Result<String, ApiError> {
    let sources = usize::from(inline.is_some()) + usize::from(file.is_some()) + usize::from(stdin);
    if sources != 1 {
        return Err(invalid("select exactly one of --body, --file or --stdin"));
    }
    if let Some(body) = inline {
        return validate_body(body);
    }
    let mut file = file
        .map(|path| {
            std::fs::File::open(path)
                .map_err(|error| invalid(format!("cannot open body file: {error}")))
        })
        .transpose()?;
    let reader: &mut dyn Read = if let Some(file) = file.as_mut() {
        file
    } else {
        input
    };
    let mut bytes = Vec::new();
    reader
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| invalid(format!("cannot read body: {error}")))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(invalid("body exceeds byte limit"));
    }
    validate_body(String::from_utf8(bytes).map_err(|_| invalid("body must be valid UTF-8"))?)
}

fn validate_body(body: String) -> Result<String, ApiError> {
    if body.is_empty() {
        return Err(invalid("body cannot be empty"));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(invalid("body exceeds byte limit"));
    }
    Ok(body)
}
