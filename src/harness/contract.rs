//! Native hook payload contracts as data, with a stable identifier and a
//! violation classifier.
//!
//! Each harness module ([`super::claude::CONTRACT`], [`super::codex::CONTRACT`])
//! declares the event kinds it consumes and, per event, the fields its parsers
//! require or read, with their JSON types. The declaration is checked against
//! the parsers by the drift tests in `tests/harness/contract.rs`: a parser that
//! starts requiring an undeclared field fails them.
//!
//! [`contract_id`] is the first 16 hex characters of SHA-256 over the contract's
//! canonical JSON ([`canonical_json`]: sorted keys, events sorted by name,
//! fields sorted by path, no whitespace). It depends only on the declaration,
//! never on the build or toolchain.
//!
//! [`classify`] decides whether a hook payload satisfies a contract. Value
//! constraints are not contract terms: string length and control-character
//! limits, enumerated `source` values (for example Codex `fork`), the
//! `tool_name == "Bash"` filter and the `agent_id`/`agent_type` pairing rule
//! are never checked here. A payload that satisfies the contract but fails
//! them is the caller's concern (the parse-failure path), never a violation.
use super::recipe::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Largest hook stdin the native parsers accept; larger is [`Malformed::TooLarge`].
pub const MAX_PAYLOAD: usize = 65_536;

/// The JSON type a declared field must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonType {
    String,
    /// The key must be present (when required); the value a string or null.
    StringOrNull,
    Object,
    Bool,
    Number,
    Array,
}

impl JsonType {
    fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::StringOrNull => "string_or_null",
            Self::Object => "object",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::Array => "array",
        }
    }

    fn admits(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::StringOrNull => value.is_string() || value.is_null(),
            Self::Object => value.is_object(),
            Self::Bool => value.is_boolean(),
            Self::Number => value.is_number(),
            Self::Array => value.is_array(),
        }
    }
}

/// One declared payload field. `path` is dotted: `tool_input.command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSpec {
    pub path: &'static str,
    pub ty: JsonType,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    Lifecycle,
    Tool,
    Other,
}

impl EventClass {
    fn name(self) -> &'static str {
        match self {
            Self::Lifecycle => "lifecycle",
            Self::Tool => "tool",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventContract {
    pub event: &'static str,
    pub class: EventClass,
    pub fields: &'static [FieldSpec],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessContract {
    pub harness: &'static str,
    /// The payload key naming the event kind.
    pub discriminator: &'static str,
    pub events: &'static [EventContract],
}

/// Shorthand for the declarations in the harness modules.
pub const fn field(path: &'static str, ty: JsonType, required: bool) -> FieldSpec {
    FieldSpec { path, ty, required }
}

/// The declared contract for `harness` (`claude` or `codex`).
pub fn contract_for(harness: &str) -> Option<&'static HarnessContract> {
    match harness {
        "claude" => Some(&super::claude::CONTRACT),
        "codex" => Some(&super::codex::CONTRACT),
        _ => None,
    }
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("a str always serializes")
}

/// Canonical JSON of a contract: keys in byte order, `events` sorted by event
/// name, each event's `fields` sorted by path, no whitespace.
pub fn canonical_json(c: &HarnessContract) -> String {
    let mut events: Vec<&EventContract> = c.events.iter().collect();
    events.sort_by_key(|e| e.event);
    let events: Vec<String> = events
        .into_iter()
        .map(|e| {
            let mut fields: Vec<&FieldSpec> = e.fields.iter().collect();
            fields.sort_by_key(|f| f.path);
            let fields: Vec<String> = fields
                .into_iter()
                .map(|f| {
                    format!(
                        "{{\"path\":{},\"required\":{},\"type\":{}}}",
                        json_string(f.path),
                        f.required,
                        json_string(f.ty.name())
                    )
                })
                .collect();
            format!(
                "{{\"class\":{},\"event\":{},\"fields\":[{}]}}",
                json_string(e.class.name()),
                json_string(e.event),
                fields.join(",")
            )
        })
        .collect();
    format!(
        "{{\"discriminator\":{},\"events\":[{}],\"harness\":{}}}",
        json_string(c.discriminator),
        events.join(","),
        json_string(c.harness)
    )
}

/// First 16 lowercase hex characters of SHA-256 over [`canonical_json`].
pub fn contract_id(c: &HarnessContract) -> String {
    let digest = Sha256::digest(canonical_json(c).as_bytes());
    let mut id = String::with_capacity(16);
    for byte in &digest[..8] {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

/// Why a payload could not be judged against the contract at all. Malformed
/// never counts as a violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    TooLarge,
    /// Unparseable or truncated JSON.
    NotJson,
    NotObject,
    /// No declared event could be selected.
    UnknownEvent,
}

/// The classifier's verdict. `Violation` means a well-formed object for a
/// known event kind that breaks the contract; `Malformed` is never counted as
/// a violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    Ok {
        event: &'static str,
    },
    Violation {
        event: &'static str,
        field: &'static str,
    },
    Malformed(Malformed),
}

fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |v, key| v.as_object()?.get(key))
}

fn satisfies(value: &Value, spec: &FieldSpec) -> bool {
    match lookup(value, spec.path) {
        None => !spec.required,
        // A required string that is JSON null is missing; a required
        // string-or-null key only has to be present.
        Some(Value::Null) => match spec.ty {
            JsonType::StringOrNull => true,
            _ => !spec.required,
        },
        Some(v) => spec.ty.admits(v),
    }
}

/// Classify `stdin` against `contract`. `registered_event` is the event the
/// hook was registered for (named on the hook command line); when present and
/// declared it selects the event and the payload's own discriminator is only
/// checked against the declared key, never trusted to pick it. Without a
/// registration (legacy) the payload's discriminator selects the event.
pub fn classify(
    contract: &HarnessContract,
    registered_event: Option<&str>,
    stdin: &[u8],
) -> Classification {
    if stdin.len() > MAX_PAYLOAD {
        return Classification::Malformed(Malformed::TooLarge);
    }
    let Ok(value) = serde_json::from_slice::<Value>(stdin) else {
        return Classification::Malformed(Malformed::NotJson);
    };
    if !value.is_object() {
        return Classification::Malformed(Malformed::NotObject);
    }
    let discriminator = value.get(contract.discriminator).and_then(Value::as_str);
    let event = match registered_event {
        Some(name) => contract.events.iter().find(|e| e.event == name),
        None => discriminator.and_then(|d| contract.events.iter().find(|e| e.event == d)),
    };
    let Some(event) = event else {
        return Classification::Malformed(Malformed::UnknownEvent);
    };
    if registered_event.is_some() && discriminator != Some(event.event) {
        return Classification::Violation {
            event: event.event,
            field: contract.discriminator,
        };
    }
    let mut fields: Vec<&FieldSpec> = event.fields.iter().collect();
    fields.sort_by_key(|f| f.path);
    match fields.into_iter().find(|f| !satisfies(&value, f)) {
        Some(f) => Classification::Violation {
            event: event.event,
            field: f.path,
        },
        None => Classification::Ok { event: event.event },
    }
}

/// The canonical harness version string, bare `MAJOR.MINOR.PATCH`, from the
/// raw `--version` output: surrounding whitespace is trimmed, then `claude`
/// loses an optional ` (Claude Code)` suffix and `codex` an optional
/// `codex-cli ` prefix. Anything else, including pre-release suffixes
/// (stable-only, so evidence is never conflated between a pre-release and its
/// release) and unknown harnesses, is `None`.
pub fn normalize_version(harness: &str, raw: &str) -> Option<String> {
    let trimmed = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    let core = match harness {
        "claude" => trimmed.strip_suffix(" (Claude Code)").unwrap_or(trimmed),
        "codex" => trimmed.strip_prefix("codex-cli ").unwrap_or(trimmed),
        _ => return None,
    };
    Version::parse(core).map(|v| v.to_string())
}

/// Version strings the canary and operators can compare against, per harness.
pub fn normalize_examples(harness: &str) -> &'static [(&'static str, &'static str)] {
    match harness {
        "claude" => &[("2.1.286 (Claude Code)", "2.1.286")],
        "codex" => &[("codex-cli 0.158.0", "0.158.0")],
        _ => &[],
    }
}

/// `contract-id --json` document: `{claude, codex, normalize}` (or one
/// harness), keys sorted.
pub fn contract_ids_json(harness: Option<&str>) -> Value {
    let names: Vec<&str> = match harness {
        Some(h) => vec![h],
        None => vec!["claude", "codex"],
    };
    let mut doc = serde_json::Map::new();
    let mut normalize = serde_json::Map::new();
    for name in names {
        let Some(contract) = contract_for(name) else {
            continue;
        };
        doc.insert(name.to_owned(), Value::String(contract_id(contract)));
        let examples: serde_json::Map<String, Value> = normalize_examples(name)
            .iter()
            .map(|(raw, v)| ((*raw).to_owned(), Value::String((*v).to_owned())))
            .collect();
        normalize.insert(name.to_owned(), Value::Object(examples));
    }
    doc.insert("normalize".to_owned(), Value::Object(normalize));
    Value::Object(doc)
}

/// `contract-id` output: the JSON document (one line), or one `<harness> <id>`
/// line per harness.
pub fn render_contract_ids(harness: Option<&str>, json: bool) -> String {
    if json {
        return format!("{}\n", contract_ids_json(harness));
    }
    let names: &[&str] = match harness {
        Some("claude") => &["claude"],
        Some("codex") => &["codex"],
        _ => &["claude", "codex"],
    };
    names
        .iter()
        .filter_map(|name| contract_for(name).map(|c| format!("{name} {}\n", contract_id(c))))
        .collect()
}

/// `harness-version normalize` output, or the stderr text for an unrecognized
/// raw string.
pub fn render_normalize(harness: &str, raw: &str, json: bool) -> Result<String, String> {
    let version = normalize_version(harness, raw)
        .ok_or_else(|| format!("unrecognized {harness} version: {raw}"))?;
    Ok(if json {
        format!(
            "{}\n",
            serde_json::json!({"harness": harness, "raw": raw, "version": version})
        )
    } else {
        format!("{version}\n")
    })
}
