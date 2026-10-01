//! Selected CLI output contract. The exact encoder implementation is owned by
//! the CLI rendering checkpoint; query page fitting calls that same encoder.

use super::{
    results::{ApiError, CommandResult, ErrorCode, MessageSummary, SearchHit, ThreadSummary},
    wire::PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    Json,
}

/// Explicit global selectors to repeat in an exact continuation command.
/// These values select local context only and confer no identity or authority.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationContext {
    pub state_dir: Option<String>,
    /// Host endpoint pathname. It is a local filesystem path, not an opaque
    /// host ID: Herdr socket paths may be long or contain spaces.
    pub host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSpec {
    pub format: OutputFormat,
    pub context: ContinuationContext,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            format: OutputFormat::Json,
            context: ContinuationContext::default(),
        }
    }
}

impl OutputSpec {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .context
            .state_dir
            .as_ref()
            .is_some_and(|path| path.is_empty() || path.len() > 1024 || path.contains('\0'))
        {
            return Err("invalid state selector");
        }
        if self
            .context
            .host
            .as_ref()
            .is_some_and(|path| path.is_empty() || path.len() > 1024 || path.contains('\0'))
        {
            return Err("invalid host endpoint selector");
        }
        Ok(())
    }
}

/// Task 47 supplies `pub fn encode_selected(result: &CommandResult,
/// spec: &OutputSpec) -> Result<Vec<u8>, ApiError>` at this path. This callable
/// type fixes the signature without pretending an approximate encoder is usable
/// for query page fitting.
pub type SelectedEncoder = fn(&CommandResult, &OutputSpec) -> Result<Vec<u8>, ApiError>;

/// Encode the selected CLI representation, including its terminal newline.
pub fn encode_selected(result: &CommandResult, spec: &OutputSpec) -> Result<Vec<u8>, ApiError> {
    spec.validate().map_err(|detail| ApiError {
        code: ErrorCode::InvalidRequest,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    })?;
    let selected = selected_result(result, spec);
    #[derive(Serialize)]
    struct Envelope<'a> {
        version: u16,
        result: &'a CommandResult,
    }
    if spec.format == OutputFormat::Text {
        return encode_text(&selected, spec);
    }
    let mut bytes = serde_json::to_vec(&Envelope {
        version: PROTOCOL_VERSION,
        result: &selected,
    })
    .map_err(|error| ApiError {
        code: ErrorCode::InvalidRequest,
        detail: format!("selected output encoding failed: {error}"),
        restart_argv: None,
        required_minimum_bytes: None,
    })?;
    if matches!(result, CommandResult::CachedCheckInPage(_)) {
        let mut escaped = String::from_utf8(bytes).map_err(|_| ApiError {
            code: ErrorCode::InvalidRequest,
            detail: "invalid cache page encoding".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        })?;
        if escaped.chars().any(needs_terminal_escape) {
            escaped = escaped
                .chars()
                .map(|ch| {
                    if needs_terminal_escape(ch) {
                        format!("\\u{:04x}", ch as u32)
                    } else {
                        ch.to_string()
                    }
                })
                .collect();
        }
        bytes = escaped.into_bytes();
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Apply the selected snippet caps and detail commands without encoding.
pub fn selected_result(result: &CommandResult, spec: &OutputSpec) -> CommandResult {
    let mut selected = result.clone();
    match &mut selected {
        CommandResult::Directory(page) => {
            for summary in &mut page.items {
                cap_topic(summary, spec);
            }
        }
        CommandResult::History(page) => {
            for summary in &mut page.items {
                cap_preview(summary, spec);
            }
        }
        CommandResult::Search(search) => {
            for hit in &mut search.matches.items {
                match hit {
                    SearchHit::Topic(summary) => cap_topic(summary, spec),
                    SearchHit::Body(summary) => cap_preview(summary, spec),
                }
            }
        }
        CommandResult::DeliveryInspect(detail) => cap_preview(&mut detail.message, spec),
        CommandResult::Message(detail) => cap_preview(&mut detail.summary, spec),
        _ => {}
    }
    selected
}

fn cap_topic(summary: &mut ThreadSummary, spec: &OutputSpec) {
    let (snippet, clipped) = escaped_snippet(&summary.topic_data, spec.format);
    summary.topic_data = snippet;
    summary.topic_omitted |= clipped;
    if summary.topic_omitted {
        summary.topic_detail_argv = Some(detail_argv(
            spec,
            &["thread", "show", summary.thread.as_str()],
        ));
    }
}

fn cap_preview(summary: &mut MessageSummary, spec: &OutputSpec) {
    let (snippet, clipped) = escaped_snippet(&summary.preview_data, spec.format);
    summary.preview_data = snippet;
    summary.preview_omitted |= clipped;
    if summary.preview_omitted {
        summary.preview_detail_argv = Some(detail_argv(spec, &["body", summary.message.as_str()]));
    }
}

fn escaped_snippet(input: &str, format: OutputFormat) -> (String, bool) {
    const MAX_ESCAPED_BYTES: usize = 256;
    let mut out = String::new();
    let mut bytes = 0;
    for ch in input.chars() {
        let width = match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{0008}' | '\u{000c}' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            ch if format == OutputFormat::Text && needs_terminal_escape(ch) => 6,
            _ => ch.len_utf8(),
        };
        if bytes + width > MAX_ESCAPED_BYTES {
            return (out, true);
        }
        bytes += width;
        out.push(ch);
    }
    (out, false)
}

fn needs_terminal_escape(ch: char) -> bool {
    matches!(ch, '\u{007f}'..='\u{009f}' | '\u{2028}' | '\u{2029}')
}

/// JSON syntax is useful for readable typed text, but its default string
/// serializer leaves C1 controls and Unicode separators as literal UTF-8.
/// Escape them here, inside the selected-byte encoder used by page fitting.
fn text_json(value: &serde_json::Value) -> String {
    let serialized = value.to_string();
    let mut text = String::with_capacity(serialized.len());
    for ch in serialized.chars() {
        if needs_terminal_escape(ch) {
            text.push_str(&format!("\\u{:04x}", ch as u32));
        } else {
            text.push(ch);
        }
    }
    text
}

fn detail_argv(spec: &OutputSpec, command: &[&str]) -> Vec<String> {
    let mut argv = vec!["herdr-threads".to_string()];
    if let Some(state_dir) = &spec.context.state_dir {
        argv.push("--state-dir".into());
        argv.push(state_dir.clone());
    }
    if let Some(host) = &spec.context.host {
        argv.push("--host-endpoint".into());
        argv.push(host.as_str().into());
    }
    if spec.format == OutputFormat::Json {
        argv.push("--json".into());
    }
    argv.extend(command.iter().map(|part| (*part).into()));
    argv
}

fn encode_text(result: &CommandResult, spec: &OutputSpec) -> Result<Vec<u8>, ApiError> {
    if let CommandResult::CachedCheckInPage(page) = result {
        let mut text = String::from("cached_check_in_page\n");
        text.push_str("reference: ");
        text.push_str(&text_json(&serde_json::json!(page.reference)));
        text.push('\n');
        text.push_str("output_sha256: ");
        text.push_str(&page.output_sha256);
        text.push('\n');
        text.push_str(&format!(
            "cached: true\nhistorical: {}\nstart: {}\nend: {}\ntotal: {}\n",
            page.historical, page.start, page.end, page.total
        ));
        text.push_str("untrusted_chunk_data: ");
        text.push_str(&text_json(&serde_json::json!(page.chunk_data)));
        text.push('\n');
        text.push_str("next_argv: ");
        text.push_str(&text_json(&serde_json::json!(page.next_argv)));
        text.push('\n');
        return Ok(text.into_bytes());
    }
    if let Some(text) = compact::render(result, spec) {
        return Ok(text.into_bytes());
    }
    let value = serde_json::to_value(result).map_err(|error| ApiError {
        code: ErrorCode::InvalidRequest,
        detail: format!("selected output encoding failed: {error}"),
        restart_argv: None,
        required_minimum_bytes: None,
    })?;
    let kind = value["kind"].as_str().expect("serialized result kind");
    let data = &value["data"];
    let mut text = String::new();
    text.push_str(kind);
    text.push('\n');
    // ht-4is.8.18: no page metadata blob and no command or cursor repeated
    // inside the JSON: each `*_argv` is printed once, as its own command line
    // (a page's continuation as the single trailing `next:` line, only when
    // there is more). `--json` keeps the full structured form.
    if let Some(items) = data.get("items").and_then(serde_json::Value::as_array) {
        for item in items {
            text.push_str("item: ");
            text.push_str(&text_json(&without_commands(item)));
            text.push('\n');
        }
    } else if let Some(fields) = data.as_object() {
        for (name, value) in fields {
            if is_command_field(name, fields) {
                continue;
            }
            text.push_str(name);
            text.push_str(": ");
            text.push_str(&text_json(&without_commands(value)));
            text.push('\n');
        }
    } else {
        text.push_str("value: ");
        text.push_str(&text_json(data));
        text.push('\n');
    }
    append_commands("", data, &mut text);
    Ok(text.into_bytes())
}

/// A `*_argv` command (printed as its own line) or a cursor its sibling
/// command already carries, or page bookkeeping the command line replaces.
fn is_command_field(name: &str, fields: &serde_json::Map<String, serde_json::Value>) -> bool {
    if name.ends_with("_argv") {
        return true;
    }
    if let Some(stem) = name.strip_suffix("_cursor")
        && fields.contains_key(&format!("{stem}_argv"))
    {
        return true;
    }
    fields.contains_key("items")
        && fields.contains_key("has_more")
        && matches!(
            name,
            "high_water_ordinal" | "scope_revision" | "stop_reason" | "consistency"
        )
}

fn without_commands(value: &serde_json::Value) -> serde_json::Value {
    match value {
        // Peer/system event JSON is data, shown verbatim (its `*_argv`-named
        // fields are never commands; `append_commands` skips it too).
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .filter(|(name, _)| !is_command_field(name, fields))
                .map(|(name, value)| {
                    let value = if name == "event_json" {
                        value.clone()
                    } else {
                        without_commands(value)
                    };
                    (name.clone(), value)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(without_commands).collect())
        }
        other => other.clone(),
    }
}

fn append_commands(prefix: &str, value: &serde_json::Value, text: &mut String) {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, item) in fields {
                if name.ends_with("_argv") {
                    if let Some(argv) = item.as_array() {
                        let label = if name == "next_argv" {
                            "next"
                        } else {
                            &name[..name.len() - 5]
                        };
                        if !prefix.is_empty() {
                            text.push_str(prefix);
                            text.push('.');
                        }
                        text.push_str(label);
                        text.push_str(": ");
                        let args: Vec<String> = argv
                            .iter()
                            .map(|arg| arg.as_str().expect("serialized argv string").to_owned())
                            .collect();
                        text.push_str(&format_command_argv(&args));
                        text.push('\n');
                    }
                } else if name != "event_json" {
                    let child = if name == "items" {
                        "item".to_string()
                    } else if prefix.is_empty() {
                        name.to_string()
                    } else {
                        format!("{prefix}.{name}")
                    };
                    append_commands(&child, item, text);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                append_commands(prefix, item, text);
            }
        }
        _ => {}
    }
}

fn shell_quote(arg: &str, text: &mut String) {
    if arg
        .chars()
        .any(|ch| ch.is_control() || needs_terminal_escape(ch))
    {
        text.push_str("$'");
        for ch in arg.chars() {
            match ch {
                '\n' => text.push_str("\\n"),
                '\r' => text.push_str("\\r"),
                '\t' => text.push_str("\\t"),
                '\'' => text.push_str("\\'"),
                '\\' => text.push_str("\\\\"),
                ch if ch.is_control() => {
                    let mut buffer = [0; 4];
                    for byte in ch.encode_utf8(&mut buffer).bytes() {
                        text.push_str(&format!("\\x{byte:02x}"));
                    }
                }
                ch if needs_terminal_escape(ch) => {
                    let mut buffer = [0; 4];
                    for byte in ch.encode_utf8(&mut buffer).bytes() {
                        text.push_str(&format!("\\x{byte:02x}"));
                    }
                }
                ch => text.push(ch),
            }
        }
        text.push('\'');
        return;
    }
    if !arg.is_empty()
        && arg
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&byte))
    {
        text.push_str(arg);
        return;
    }
    text.push('\'');
    for ch in arg.chars() {
        if ch == '\'' {
            text.push_str("'\\''");
        } else {
            text.push(ch);
        }
    }
    text.push('\'');
}

/// Render an exact argv for a user-facing action with the selected encoder's
/// established terminal-safe shell quoting. The returned string has no newline.
pub fn format_command_argv(argv: &[String]) -> String {
    let mut text = String::new();
    for (index, arg) in argv.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        shell_quote(arg, &mut text);
    }
    text
}

#[path = "output_compact.rs"]
mod compact;

#[cfg(test)]
#[path = "../../tests/protocol/output.rs"]
mod tests;
