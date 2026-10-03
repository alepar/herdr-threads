//! Hidden `herdr-threads internal ...` helpers for scripts/install.sh.
//!
//! `internal json-field <path>` reads one JSON document on stdin and prints
//! the value at a dotted path (object keys; array indexes as numbers), so the
//! installer never splits JSON with shell tools. Exit 1: the path is absent;
//! exit 2: stdin is not JSON. Local only; never contacts the daemon.
use super::RunError;
use std::io::Write;

/// Print the value at `path` of `input` (strings raw, other values as JSON).
pub fn json_field<W: Write>(input: &str, path: &str, writer: &mut W) -> Result<(), RunError> {
    let document: serde_json::Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("herdr-threads: internal json-field: invalid JSON: {error}");
            return Err(RunError::Exit(2));
        }
    };
    let mut value = &document;
    for segment in path.split('.').filter(|segment| !segment.is_empty()) {
        let next = match value {
            serde_json::Value::Object(map) => map.get(segment),
            serde_json::Value::Array(items) => {
                segment.parse::<usize>().ok().and_then(|i| items.get(i))
            }
            _ => None,
        };
        match next {
            Some(found) => value = found,
            None => return Err(RunError::Exit(1)),
        }
    }
    match value {
        serde_json::Value::String(text) => writeln!(writer, "{text}")?,
        other => writeln!(writer, "{other}")?,
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/cli/internal_json_field.rs"]
mod tests;
