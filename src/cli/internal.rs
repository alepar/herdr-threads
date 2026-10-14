//! Hidden `herdr-threads internal ...` helpers for scripts/install.sh.
//!
//! `internal json-field <path>` reads one JSON document on stdin and prints
//! the value at a dotted path (object keys; array indexes as numbers), so the
//! installer never splits JSON with shell tools. Exit 1: the path is absent;
//! exit 2: stdin is not JSON. With `--keys` it prints the keys of the object
//! at the path instead, one per line (keys holding a newline or another control
//! character are omitted); exit 3 when the value, or any value on the path to
//! it, is not an object, and exit 1 only when an object lacks the next key, so a
//! caller can tell "absent" (1) from "unusable" (2: not UTF-8 or not JSON; 3).
//! Local only; never contacts the daemon.
use super::RunError;
use std::io::Write;

/// Print the value at `path` of `input` (strings raw, other values as JSON).
pub fn json_field<W: Write>(input: &str, path: &str, writer: &mut W) -> Result<(), RunError> {
    json_field_with(input, path, false, writer)
}

/// [`json_field`], or with `keys` the object's keys one per line, exactly as written (a key
/// holding a control character is omitted, so every printed line is one whole key).
pub fn json_field_with<W: Write>(
    input: &str,
    path: &str,
    keys: bool,
    writer: &mut W,
) -> Result<(), RunError> {
    json_field_bytes(input.as_bytes(), path, keys, writer)
}

/// [`json_field_with`] over raw stdin bytes: input that is not UTF-8 or not JSON exits 2.
pub fn json_field_bytes<W: Write>(
    input: &[u8],
    path: &str,
    keys: bool,
    writer: &mut W,
) -> Result<(), RunError> {
    let document: serde_json::Value = match serde_json::from_slice(input) {
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
            // `--keys` walks objects only: any other value on the path is unusable (exit 3),
            // never "absent". The value mode keeps array indexes and exit 1.
            _ if keys => return Err(RunError::Exit(3)),
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
    if keys {
        // Exit 3: present but not an object (exit 1 stays "absent").
        let serde_json::Value::Object(map) = value else {
            return Err(RunError::Exit(3));
        };
        for key in map.keys().filter(|key| !key.chars().any(char::is_control)) {
            writeln!(writer, "{key}")?;
        }
        writer.flush()?;
        return Ok(());
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
