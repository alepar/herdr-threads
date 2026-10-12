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
use super::adapter::VersionSyntax;
use super::recipe::Version;
use super::registry::Registry;
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

/// The declared contract for `harness`, from the builtin registry.
pub fn contract_for(harness: &str) -> Option<&'static HarnessContract> {
    contract_in(super::registry::builtins(), harness)
}

/// The native-payload contract `harness` declares in `registry`.
pub fn contract_in(registry: &Registry, harness: &str) -> Option<&'static HarnessContract> {
    let registration = registry.by_id(registry.agent(harness).ok()?).ok()?;
    registration
        .contracts()
        .iter()
        .find(|d| d.origin == super::evidence::EvidenceOrigin::NativePayload)
        .map(|d| d.contract)
}

/// The version syntax `harness` declares in `registry`.
fn syntax_in(registry: &Registry, harness: &str) -> Option<&'static VersionSyntax> {
    registry
        .by_id(registry.agent(harness).ok()?)
        .ok()?
        .version_syntax()
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
/// raw `--version` output: surrounding whitespace is trimmed, then the
/// harness's declared [`VersionSyntax`] suffix, then its prefix, are stripped
/// when present. Anything else, including pre-release suffixes (stable-only,
/// so evidence is never conflated between a pre-release and its release),
/// harnesses without a declared syntax and unknown harnesses, is `None`.
pub fn normalize_version(harness: &str, raw: &str) -> Option<String> {
    normalize_version_in(super::registry::builtins(), harness, raw)
}

/// [`normalize_version`] against an explicit registry.
pub fn normalize_version_in(registry: &Registry, harness: &str, raw: &str) -> Option<String> {
    let syntax = syntax_in(registry, harness)?;
    let trimmed = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    let core = trimmed.strip_suffix(syntax.suffix).unwrap_or(trimmed);
    let core = core.strip_prefix(syntax.prefix).unwrap_or(core);
    Version::parse(core).map(|v| v.to_string())
}

/// Version strings the canary and operators can compare against, per harness.
pub fn normalize_examples(harness: &str) -> &'static [(&'static str, &'static str)] {
    normalize_examples_in(super::registry::builtins(), harness)
}

fn normalize_examples_in(
    registry: &Registry,
    harness: &str,
) -> &'static [(&'static str, &'static str)] {
    syntax_in(registry, harness).map_or(&[], |syntax| syntax.examples)
}

/// The ids `contract-id` lists without a selector, in registry order: adapters
/// with a native-payload contract and a version syntax.
fn listed_ids(registry: &Registry) -> Vec<&'static str> {
    registry
        .registrations()
        .iter()
        .map(|r| r.metadata().id)
        .filter(|id| contract_in(registry, id).is_some() && syntax_in(registry, id).is_some())
        .collect()
}

/// `contract-id --json` document: one key per listed harness plus
/// `normalize` (or one harness), keys sorted.
pub fn contract_ids_json(harness: Option<&str>) -> Value {
    contract_ids_json_in(super::registry::builtins(), harness)
}

/// [`contract_ids_json`] against an explicit registry.
pub fn contract_ids_json_in(registry: &Registry, harness: Option<&str>) -> Value {
    let names: Vec<&str> = match harness {
        Some(h) => vec![h],
        None => listed_ids(registry),
    };
    let mut doc = serde_json::Map::new();
    let mut normalize = serde_json::Map::new();
    for name in names {
        let Some(contract) = contract_in(registry, name) else {
            continue;
        };
        doc.insert(name.to_owned(), Value::String(contract_id(contract)));
        let examples: serde_json::Map<String, Value> = normalize_examples_in(registry, name)
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
    render_contract_ids_in(super::registry::builtins(), harness, json)
}

/// [`render_contract_ids`] against an explicit registry.
pub fn render_contract_ids_in(registry: &Registry, harness: Option<&str>, json: bool) -> String {
    if json {
        return format!("{}\n", contract_ids_json_in(registry, harness));
    }
    let listed = listed_ids(registry);
    let names: Vec<&str> = match harness {
        Some(h) if listed.contains(&h) => vec![h],
        _ => listed,
    };
    names
        .iter()
        .filter_map(|name| {
            contract_in(registry, name).map(|c| format!("{name} {}\n", contract_id(c)))
        })
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
