//! The harness version manifest: schema 2 model, reader, embedded copy,
//! daemon cache and fetch policy (epic ht-xoc; spec
//! `docs/design/herdr-threads/2026-10-02-harness-version-evidence-design.md`).
//!
//! # Document (schema 2)
//!
//! `{schema_version: 2, generated_from, generated_at, latest_release,
//! contracts: {harness: contract_id}, rows: [...]}`. A row is keyed by
//! (`harness`, `version`, `contract_id`) and carries `status`
//! (`verified` | `known_broken`), `evidence` (`live` | `no_model` | `schema`;
//! B6's `none` is tolerated), `source` (`canary` | `manual`),
//! `supported_since`, `broken_event`, `broken_field`, `last_working`,
//! `issue_url`, plus B6's `recipe` and `known_broken` range list (read by
//! `scripts/canary/versions.py` and the `HT_TEST_RECIPES_JSON` override, not
//! by this reader). Unknown fields are ignored; an unknown `status`,
//! `evidence` or `source` string reads as absent for that row.
//!
//! The daemon uses a row's *status* only when the row's `contract_id` equals
//! its own (`status_row`); a row with another or a null contract id is no
//! status data. Release-pointer fields (`latest_release`, `supported_since`,
//! `last_working`, `issue_url`) are read for (harness, version) from any row
//! (`release_pointers`).
//!
//! # Invariants owned here
//!
//! - `schema_version` other than [`SUPPORTED_SCHEMA_VERSION`] means no
//!   manifest (never an error the operator sees).
//! - A document is at most [`MAX_MANIFEST_BYTES`] (256 KB). The writer
//!   (ht-xoc.6) keeps per harness the newest [`RETAIN_NEWEST_PER_HARNESS`]
//!   versions plus every `known_broken` row and every recipe-listed version,
//!   and fails above [`WRITER_FAIL_FRACTION_PERCENT`] percent of the cap.
//!   A `known_broken` row must carry `broken_event` and `broken_field`.
//! - Files produced by the writer and by the in-repo generator
//!   (`admission::versions_document`) both pass [`parse`].
//!
//! # Sources and fetching
//!
//! Consumers read [`ManifestService::current`]: the newer (by `generated_at`,
//! see [`prefer_newer`]) of a valid cached fetch and the copy embedded at
//! build ([`EMBEDDED`]; release builds replace the in-repo file with the
//! `harness-manifest` branch file first). The daemon calls
//! [`ManifestService::ensure_manifest`] from the recording path; it never
//! blocks its caller, it schedules one detached fetch at most (see
//! [`should_fetch`]). Nothing about the user is sent: a plain GET with an
//! optional `If-None-Match`.
use crate::daemon::settings::{HarnessManifestSetting, InstanceSettings};
use crate::protocol::time::Clock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub const SUPPORTED_SCHEMA_VERSION: u64 = 2;
pub const MANIFEST_BRANCH: &str = "harness-manifest";
/// At the root of [`MANIFEST_BRANCH`].
pub const MANIFEST_FILE: &str = "harness-versions.json";
/// Owner and repository as in `admission::ISSUES_URL`.
pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/alepar/herdr-threads/harness-manifest/harness-versions.json";
pub const MAX_MANIFEST_BYTES: usize = 262_144;
pub const RETAIN_NEWEST_PER_HARNESS: usize = 50;
pub const WRITER_FAIL_FRACTION_PERCENT: usize = 80;
/// At most one fetch per harness per this interval.
pub const FETCH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub const FETCH_TIMEOUT_SECS: u64 = 5;
/// Directory (inside the instance directory) holding the cache files.
pub const CACHE_DIR: &str = "harness-manifest";
const CACHE_FILE: &str = MANIFEST_FILE;
const META_FILE: &str = "meta.json";

/// The in-repo manifest, embedded at build.
pub const EMBEDDED: &str = include_str!("../../docs/compatibility/harness-versions.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    TooLarge,
    NotJson,
    UnsupportedSchema(Option<u64>),
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "manifest larger than {MAX_MANIFEST_BYTES} bytes"),
            Self::NotJson => write!(f, "manifest is not JSON"),
            Self::UnsupportedSchema(Some(version)) => {
                write!(f, "unsupported manifest schema_version {version}")
            }
            Self::UnsupportedSchema(None) => write!(f, "manifest has no integer schema_version"),
            Self::Invalid(detail) => write!(f, "invalid manifest: {detail}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    Verified,
    KnownBroken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowEvidence {
    Live,
    NoModel,
    Schema,
    /// B6's `none`: listed without evidence.
    Unproven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSource {
    Canary,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestRow {
    pub harness: String,
    pub version: String,
    pub status: Option<RowStatus>,
    pub evidence: Option<RowEvidence>,
    pub contract_id: Option<String>,
    pub source: Option<RowSource>,
    pub supported_since: Option<String>,
    pub broken_event: Option<String>,
    pub broken_field: Option<String>,
    pub last_working: Option<String>,
    pub issue_url: Option<String>,
}

/// Runtime rows never participate in legacy semver lookup or release ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStage {
    SourceCaptured,
    NoModel,
    Live,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Verified,
    KnownBroken,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSource {
    Canary,
    Manual,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRow {
    pub harness: String,
    pub identity: super::runtime::RuntimeIdentity,
    pub domain: String,
    pub origin: super::evidence::EvidenceOrigin,
    pub contract_id: String,
    pub status: RuntimeStatus,
    pub evidence_stage: RuntimeStage,
    pub source: RuntimeSource,
    pub required_milestones: Vec<String>,
    pub successful_milestones: Vec<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub broken_event: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub broken_field: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub supported_since: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub issue_url: Option<String>,
    /// UTC milliseconds, matching rich local evidence wire/store timestamps.
    pub last_seen_at: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeContract {
    pub domain: String,
    pub origin: super::evidence::EvidenceOrigin,
    pub id: String,
    pub events: Vec<RuntimeEvent>,
    pub required_milestones: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEvent {
    pub event: String,
    #[serde(deserialize_with = "required_nullable")]
    pub milestone: Option<String>,
    pub always_send: bool,
}
fn required_nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}
fn names_unique(names: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    names.iter().all(|s| seen.insert(s))
}
fn hash16(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn harness_id(s: &str) -> bool {
    s.len() <= 64
        && s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
}
fn field_name(s: &str) -> bool {
    s.len() <= 64
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._".contains(&b))
}
type RuntimeKey = (String, String, String, String, String);
fn runtime_key(
    harness: &str,
    key: &str,
    domain: &str,
    origin: super::evidence::EvidenceOrigin,
    contract: &str,
) -> RuntimeKey {
    (
        harness.into(),
        key.into(),
        domain.into(),
        serde_json::to_string(&origin).expect("origin enum serializes"),
        contract.into(),
    )
}
#[derive(Deserialize)]
struct RuntimeCollections {
    #[serde(default, deserialize_with = "unique_contract_map")]
    runtime_contracts: BTreeMap<String, Vec<RuntimeContract>>,
    #[serde(default)]
    runtime_rows: Vec<RuntimeRow>,
    #[serde(skip)]
    runtime_index: BTreeMap<RuntimeKey, usize>,
}
fn unique_contract_map<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Vec<RuntimeContract>>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, Vec<RuntimeContract>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique runtime harness descriptor map")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, Vec<RuntimeContract>>()? {
                if out.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate runtime harness key"));
                }
            }
            Ok(out)
        }
    }
    d.deserialize_map(Visitor)
}
fn runtime_collections(bytes: &[u8]) -> Result<RuntimeCollections, ManifestError> {
    let invalid = |reason: &str| ManifestError::Invalid(format!("runtime collections: {reason}"));
    // Deserialize rich fields directly: Value normalizes duplicate JSON keys. The
    // frozen legacy reader still consumes its original permissive Value projection.
    let mut collections: RuntimeCollections =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid strict descriptors or rows"))?;
    let contracts = &collections.runtime_contracts;
    if contracts.len() > 64 {
        return Err(invalid("too many harnesses"));
    }
    for (harness, domains) in contracts {
        // Manifest history can contain multiple contract versions for one domain;
        // discovery describes one current binary. Reuse its per-descriptor validator
        // but enforce the manifest's domain/origin/id uniqueness independently.
        let mut seen = std::collections::HashSet::new();
        for d in domains {
            if !seen.insert((
                &d.domain,
                serde_json::to_string(&d.origin).expect("origin serializes"),
                &d.id,
            )) {
                return Err(invalid("duplicate runtime descriptor"));
            }
            if d.required_milestones.is_empty() {
                return Err(invalid("empty required milestones"));
            }
            let entry = super::discovery::AdapterEntry {
                id: harness.clone(),
                display_name: harness.clone(),
                host_kinds: vec![],
                setup_scopes: vec![],
                legacy_contract_id: None,
                canary_strategy: None,
                contracts: vec![super::discovery::DomainContract {
                    domain: d.domain.clone(),
                    origin: d.origin,
                    id: d.id.clone(),
                    required_milestones: d.required_milestones.clone(),
                    events: d
                        .events
                        .iter()
                        .map(|e| super::discovery::DomainEvent {
                            event: e.event.clone(),
                            milestone: e.milestone.clone(),
                            always_send: e.always_send,
                        })
                        .collect(),
                }],
            };
            super::discovery::Discovery {
                schema_version: 1,
                adapters: vec![entry],
            }
            .validate()
            .map_err(|_| invalid("invalid discovery descriptor"))?;
        }
        if !harness_id(harness) {
            return Err(invalid("invalid harness id"));
        }
    }
    let rows = &collections.runtime_rows;
    // The total document cap bounds protected/manual history too; the reader does not
    // prune the writer's retained known-broken or manual rows.
    let mut index = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        if !harness_id(&row.harness)
            || row.last_seen_at > i64::MAX as u64
            || !hash16(&row.contract_id)
            || !names_unique(&row.required_milestones)
            || !names_unique(&row.successful_milestones)
            || row.successful_milestones.len() > 8
            || row
                .supported_since
                .as_ref()
                .is_some_and(|s| !super::runtime::printable(s, 128))
            || row
                .issue_url
                .as_ref()
                .is_some_and(|s| !super::runtime::printable(s, 512))
        {
            return Err(invalid("unbounded or duplicate row metadata"));
        }
        let Some(descriptor) = contracts.get(&row.harness).and_then(|ds| {
            ds.iter().find(|d| {
                d.domain == row.domain && d.origin == row.origin && d.id == row.contract_id
            })
        }) else {
            return Err(invalid("row has no exact declared descriptor"));
        };
        let mut required = row.required_milestones.clone();
        required.sort();
        let mut declared = descriptor.required_milestones.clone();
        declared.sort();
        if required != declared
            || row.successful_milestones.iter().any(|m| {
                !descriptor
                    .events
                    .iter()
                    .any(|e| e.milestone.as_ref() == Some(m))
            })
        {
            return Err(invalid("milestones do not match exact domain"));
        }
        match row.status {
            RuntimeStatus::Verified
                if row.evidence_stage == RuntimeStage::SourceCaptured
                    || row.broken_event.is_some()
                    || row.broken_field.is_some()
                    || !required
                        .iter()
                        .all(|m| row.successful_milestones.contains(m)) =>
            {
                return Err(invalid("verified row lacks qualified complete evidence"));
            }
            RuntimeStatus::KnownBroken
                if row
                    .broken_event
                    .as_ref()
                    .is_none_or(|e| !descriptor.events.iter().any(|d| &d.event == e))
                    || row.broken_field.as_ref().is_none_or(|s| !field_name(s)) =>
            {
                return Err(invalid("known-broken row lacks declared event/field"));
            }
            _ => (),
        }
        let key = runtime_key(
            &row.harness,
            &row.identity.key,
            &row.domain,
            row.origin,
            &row.contract_id,
        );
        if index.insert(key, i).is_some() {
            return Err(invalid("duplicate exact runtime row"));
        }
    }
    collections.runtime_index = index;
    Ok(collections)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Manifest {
    pub generated_at: Option<String>,
    pub latest_release: Option<String>,
    pub contracts: BTreeMap<String, String>,
    pub rows: Vec<ManifestRow>,
    runtime_contracts: BTreeMap<String, Vec<RuntimeContract>>,
    runtime_rows: Vec<RuntimeRow>,
    runtime_index: BTreeMap<RuntimeKey, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReleasePointers {
    pub latest_release: Option<String>,
    pub supported_since: Option<String>,
    pub last_working: Option<String>,
    pub issue_url: Option<String>,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Parse a manifest document (schema 2 only, at most 256 KB).
pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge);
    }
    let document: Value = serde_json::from_slice(bytes).map_err(|_| ManifestError::NotJson)?;
    let schema = document.get("schema_version").and_then(Value::as_u64);
    if schema != Some(SUPPORTED_SCHEMA_VERSION) {
        return Err(ManifestError::UnsupportedSchema(schema));
    }
    let rows = document
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| ManifestError::Invalid("`rows` is not an array".into()))?;
    let mut parsed = Vec::with_capacity(rows.len());
    for row in rows {
        let (Some(harness), Some(version)) = (text(row, "harness"), text(row, "version")) else {
            return Err(ManifestError::Invalid(format!(
                "a row has no string `harness` and `version`: {row}"
            )));
        };
        parsed.push(ManifestRow {
            status: match row.get("status").and_then(Value::as_str) {
                Some("verified") => Some(RowStatus::Verified),
                Some("known_broken") => Some(RowStatus::KnownBroken),
                _ => None,
            },
            evidence: match row.get("evidence").and_then(Value::as_str) {
                Some("live") => Some(RowEvidence::Live),
                Some("no_model") => Some(RowEvidence::NoModel),
                Some("schema") => Some(RowEvidence::Schema),
                Some("none") => Some(RowEvidence::Unproven),
                _ => None,
            },
            contract_id: text(row, "contract_id"),
            source: match row.get("source").and_then(Value::as_str) {
                Some("canary") => Some(RowSource::Canary),
                Some("manual") => Some(RowSource::Manual),
                _ => None,
            },
            supported_since: text(row, "supported_since"),
            broken_event: text(row, "broken_event"),
            broken_field: text(row, "broken_field"),
            last_working: text(row, "last_working"),
            issue_url: text(row, "issue_url"),
            harness,
            version,
        });
    }
    let contracts = document
        .get("contracts")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(harness, id)| Some((harness.clone(), id.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let RuntimeCollections {
        runtime_contracts,
        runtime_rows,
        runtime_index,
    } = runtime_collections(bytes)?;
    Ok(Manifest {
        runtime_contracts,
        runtime_rows,
        runtime_index,
        generated_at: text(&document, "generated_at"),
        latest_release: text(&document, "latest_release"),
        contracts,
        rows: parsed,
    })
}

impl Manifest {
    /// Validated historical metadata; use runtime_row with the current descriptor
    /// before deriving a usable verdict. Unknown/stale rows remain diagnostic only.
    pub fn runtime_rows(&self) -> &[RuntimeRow] {
        &self.runtime_rows
    }
    /// Exact current descriptor lookup; historical unrelated declarations are unavailable.
    pub fn runtime_row(
        &self,
        harness: &str,
        identity: &super::runtime::RuntimeIdentity,
        descriptor: &super::adapter::ContractDescriptor,
    ) -> Option<&RuntimeRow> {
        if identity.validate().is_err()
            || descriptor.validate().is_err()
            || descriptor.required_milestones.is_empty()
            || descriptor.contract.harness != harness
        {
            return None;
        }
        let id = descriptor.contract_id_v2().ok()?;
        let declared = self.runtime_contracts.get(harness)?.iter().find(|d| {
            d.domain == descriptor.domain_id && d.origin == descriptor.origin && d.id == id
        })?;
        // Discovery omits full native field declarations: do not pretend to rehash
        // its projection. Match the current binary's canonical contract id AND all
        // projected metadata before using a historical row.
        if declared
            .required_milestones
            .iter()
            .map(String::as_str)
            .ne(descriptor.required_milestones.iter().copied())
            || declared.events.len() != descriptor.events.len()
            || declared.events.iter().zip(descriptor.events).any(|(a, b)| {
                a.event != b.native_event
                    || a.milestone.as_deref() != b.milestone
                    || a.always_send != b.always_send
            })
        {
            return None;
        }
        let index = self.runtime_index.get(&runtime_key(
            harness,
            &identity.key,
            descriptor.domain_id,
            descriptor.origin,
            &id,
        ))?;
        let row = self.runtime_rows.get(*index)?;
        if &row.identity != identity {
            return None;
        }
        // For a current known-broken contract, the named field must actually be
        // part of that event or the contract discriminator, not merely an identifier.
        if row.status == RuntimeStatus::KnownBroken {
            let event = descriptor
                .contract
                .events
                .iter()
                .find(|e| Some(e.event) == row.broken_event.as_deref())?;
            let field = row.broken_field.as_deref()?;
            if field != descriptor.contract.discriminator
                && event.fields.iter().all(|f| f.path != field)
            {
                return None;
            }
        }
        Some(row)
    }

    /// The row whose status the daemon may use: `contract_id == own` and a
    /// known status. A row of another (or no) contract is no status data.
    pub fn status_row(
        &self,
        harness: &str,
        version: &str,
        own_contract_id: &str,
    ) -> Option<&ManifestRow> {
        self.rows.iter().find(|row| {
            row.harness == harness
                && row.version == version
                && row.contract_id.as_deref() == Some(own_contract_id)
                && row.status.is_some()
        })
    }

    /// Release pointers for (harness, version): each field from the
    /// own-contract row when non-null there, else from the first other row in
    /// file order where it is non-null. `latest_release` is the document's.
    pub fn release_pointers(
        &self,
        harness: &str,
        version: &str,
        own_contract_id: &str,
    ) -> ReleasePointers {
        let mut matching: Vec<&ManifestRow> = self
            .rows
            .iter()
            .filter(|row| row.harness == harness && row.version == version)
            .collect();
        // Stable: the own-contract rows first, the others in file order.
        matching.sort_by_key(|row| row.contract_id.as_deref() != Some(own_contract_id));
        let pick = |field: fn(&ManifestRow) -> &Option<String>| {
            matching.iter().find_map(|row| field(row).clone())
        };
        ReleasePointers {
            latest_release: self.latest_release.clone(),
            supported_since: pick(|row| &row.supported_since),
            last_working: pick(|row| &row.last_working),
            issue_url: pick(|row| &row.issue_url),
        }
    }

    /// A `verified` row for (harness, version) under a known contract id that
    /// differs from `own_contract_id`: the version works with another
    /// contract (another herdr-threads release). When several qualify, the one
    /// with the greatest `supported_since` (a row without one ranks last).
    pub fn other_contract_verified(
        &self,
        harness: &str,
        version: &str,
        own_contract_id: &str,
    ) -> Option<&ManifestRow> {
        let since = |row: &ManifestRow| {
            row.supported_since
                .as_deref()
                .and_then(|text| super::recipe::Version::parse(text.trim_start_matches('v')))
        };
        self.rows
            .iter()
            .filter(|row| {
                row.harness == harness
                    && row.version == version
                    && row.status == Some(RowStatus::Verified)
                    && row
                        .contract_id
                        .as_deref()
                        .is_some_and(|id| id != own_contract_id)
            })
            .max_by_key(|row| since(row))
    }

    /// Any row for (harness, version), of any contract.
    pub fn has_row(&self, harness: &str, version: &str) -> bool {
        self.rows
            .iter()
            .any(|row| row.harness == harness && row.version == version)
    }
}

/// The embedded copy, parsed once. A parse failure is a bug caught by a test;
/// at runtime it yields an empty manifest.
pub fn embedded() -> &'static Manifest {
    static PARSED: OnceLock<Manifest> = OnceLock::new();
    PARSED.get_or_init(|| parse(EMBEDDED.as_bytes()).unwrap_or_default())
}

/// The newer of a valid cache and the embedded copy. The embedded copy wins
/// when there is no cache, or when both carry `generated_at` and the embedded
/// one is strictly newer (RFC 3339 UTC strings order lexicographically), so a
/// cache from before a herdr-threads upgrade never shadows a newer embedded
/// copy. A cache without `generated_at` wins.
pub fn prefer_newer(cache: Option<Manifest>, embedded: &Manifest) -> Manifest {
    match cache {
        None => embedded.clone(),
        Some(cache) => match (
            cache.generated_at.as_deref(),
            embedded.generated_at.as_deref(),
        ) {
            (Some(cached), Some(built_in)) if built_in > cached => embedded.clone(),
            _ => cache,
        },
    }
}

// -- policy -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffReason {
    Settings,
    OfflineEnv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestPolicy {
    Auto,
    Off(OffReason),
}

/// `HERDR_THREADS_OFFLINE=1` or `harness_manifest: off` turns fetching off;
/// the settings label wins when both are set. Computed once at daemon start.
pub fn policy_from(settings: &InstanceSettings, offline_env: Option<&OsStr>) -> ManifestPolicy {
    if settings.harness_manifest == HarnessManifestSetting::Off {
        ManifestPolicy::Off(OffReason::Settings)
    } else if offline_env.is_some_and(|value| value == "1") {
        ManifestPolicy::Off(OffReason::OfflineEnv)
    } else {
        ManifestPolicy::Auto
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchReason {
    /// The first time a version is seen. The caller (ht-xoc.4) passes this
    /// only for a version with no local evidence row; the manifest half of
    /// "no row anywhere" is checked here.
    UnseenVersion {
        version: String,
    },
    /// An exact v2 domain observation; never projected into legacy release rows.
    UnseenRuntime {
        identity: super::runtime::RuntimeIdentity,
        domain: String,
        origin: super::evidence::EvidenceOrigin,
        contract_id: String,
    },
    FreshViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Fetch,
    Skip(&'static str),
}

/// Persisted next to the cache: the ETag and fetch time of the last good
/// body and the last attempt time per harness (success or failure).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CacheMeta {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub fetched_at_ms: Option<i64>,
    #[serde(default)]
    pub attempts: BTreeMap<String, i64>,
    /// The policy the daemon resolved at its last start, for `doctor`.
    #[serde(default)]
    pub daemon_policy: Option<RecordedPolicy>,
}

/// The effective fetch policy the daemon recorded at start (cache meta), so
/// `doctor` reports the daemon's policy, not its own environment's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedPolicy {
    /// `auto` or `off`.
    pub policy: String,
    /// `default`, `settings` or `offline_env`.
    pub source: String,
    pub recorded_at_ms: i64,
}

/// The (`policy`, `source`) words for a policy, shared by the daemon's record
/// and `doctor`.
pub fn policy_words(policy: ManifestPolicy) -> (&'static str, &'static str) {
    match policy {
        ManifestPolicy::Auto => ("auto", "default"),
        ManifestPolicy::Off(OffReason::Settings) => ("off", "settings"),
        ManifestPolicy::Off(OffReason::OfflineEnv) => ("off", "offline_env"),
    }
}

/// Pure fetch decision. `has_row` is whether the cached-or-embedded manifest
/// has any row for the version of an `UnseenVersion`.
pub fn should_fetch(
    policy: ManifestPolicy,
    reason: &FetchReason,
    harness: &str,
    now_ms: i64,
    meta: &CacheMeta,
    has_row: bool,
) -> Decision {
    match policy {
        ManifestPolicy::Off(_) => return Decision::Skip("fetching is off"),
        ManifestPolicy::Auto => (),
    }
    if let FetchReason::UnseenRuntime {
        identity,
        domain,
        contract_id,
        ..
    } = reason
        && (identity.validate().is_err()
            || !super::evidence::valid_name(domain)
            || contract_id.len() != 16
            || !contract_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    {
        return Decision::Skip("invalid exact runtime observation");
    }
    if matches!(reason, FetchReason::UnseenVersion { .. }) && has_row {
        return Decision::Skip("the manifest already has a row for this version");
    }
    let interval = i64::try_from(FETCH_INTERVAL.as_millis()).unwrap_or(i64::MAX);
    match meta.attempts.get(harness) {
        Some(last) if now_ms.saturating_sub(*last) < interval => {
            Decision::Skip("already attempted within the last 24 hours")
        }
        _ => Decision::Fetch,
    }
}

// -- fetching ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// `curl` is not available: same as offline.
    Offline,
    Failed(String),
    TooLarge,
}

pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str, etag: Option<&str>) -> Result<FetchOutcome, FetchError>;
}

/// The URL the daemon fetches. Test builds may point it elsewhere (for
/// example a `file://` URL) with `HT_TEST_MANIFEST_URL`.
pub fn manifest_url() -> String {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(url) = std::env::var("HT_TEST_MANIFEST_URL")
        .ok()
        .filter(|url| !url.is_empty())
    {
        return url;
    }
    MANIFEST_URL.to_owned()
}

/// Fetches with `curl -fsS --max-time 5` (no other request headers). A
/// missing `curl` is [`FetchError::Offline`].
pub struct CurlFetcher {
    work_dir: PathBuf,
    path_override: Option<OsString>,
}

impl CurlFetcher {
    /// `work_dir` holds the temporary body and header files (removed after
    /// each fetch); it must exist.
    pub fn new(work_dir: PathBuf) -> Self {
        Self {
            work_dir,
            path_override: None,
        }
    }

    /// Resolve `curl` in `path` instead of the process `PATH`.
    pub fn with_path(work_dir: PathBuf, path: OsString) -> Self {
        Self {
            work_dir,
            path_override: Some(path),
        }
    }

    fn resolve_curl(&self) -> Option<PathBuf> {
        let path = self
            .path_override
            .clone()
            .or_else(|| std::env::var_os("PATH"))?;
        std::env::split_paths(&path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("curl"))
            .find(|candidate| {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(candidate)
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
    }
}

fn etag_from_headers(headers: &str) -> Option<String> {
    headers.lines().rev().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("etag")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

impl Fetcher for CurlFetcher {
    fn fetch(&self, url: &str, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        let curl = self.resolve_curl().ok_or(FetchError::Offline)?;
        let stem = format!("fetch-{}-{}", std::process::id(), uuid::Uuid::new_v4());
        let body_path = self.work_dir.join(format!("{stem}.body"));
        let header_path = self.work_dir.join(format!("{stem}.headers"));
        let result = (|| {
            let (Some(body_arg), Some(header_arg)) = (body_path.to_str(), header_path.to_str())
            else {
                return Err(FetchError::Failed("cache path is not UTF-8".into()));
            };
            let timeout = FETCH_TIMEOUT_SECS.to_string();
            let limit = MAX_MANIFEST_BYTES.to_string();
            let if_none_match = etag.map(|etag| format!("If-None-Match: {etag}"));
            let mut args: Vec<&str> = vec![
                "-fsS",
                "--max-time",
                &timeout,
                "--max-filesize",
                &limit,
                "-o",
                body_arg,
                "-D",
                header_arg,
                "-w",
                "%{http_code}",
            ];
            if let Some(header) = &if_none_match {
                args.extend(["-H", header]);
            }
            args.push(url);
            let output = super::codex::bounded_output(
                &curl,
                &args,
                16,
                Duration::from_secs(FETCH_TIMEOUT_SECS + 2),
            )
            .map_err(|_| FetchError::Failed("curl failed or timed out".into()))?;
            let code = String::from_utf8_lossy(&output).trim().to_owned();
            match code.as_str() {
                "304" => Ok(FetchOutcome::NotModified),
                // `file://` URLs (tests) report 000.
                "200" | "000" => {
                    let mut bytes = Vec::new();
                    std::fs::File::open(&body_path)
                        .and_then(|file| {
                            file.take(MAX_MANIFEST_BYTES as u64 + 1)
                                .read_to_end(&mut bytes)
                        })
                        .map_err(|error| FetchError::Failed(format!("body unreadable: {error}")))?;
                    if bytes.len() > MAX_MANIFEST_BYTES {
                        return Err(FetchError::TooLarge);
                    }
                    let headers = std::fs::read_to_string(&header_path).unwrap_or_default();
                    Ok(FetchOutcome::Body {
                        bytes,
                        etag: etag_from_headers(&headers),
                    })
                }
                other => Err(FetchError::Failed(format!(
                    "unexpected HTTP status {other}"
                ))),
            }
        })();
        let _ = std::fs::remove_file(&body_path);
        let _ = std::fs::remove_file(&header_path);
        result
    }
}

// -- cache and service ------------------------------------------------------

type LogSink = Arc<dyn Fn(&str) + Send + Sync>;

struct Inner {
    dir: PathBuf,
    policy: ManifestPolicy,
    fetcher: Arc<dyn Fetcher>,
    clock: Arc<dyn Clock>,
    log: LogSink,
    cache: Mutex<Option<Arc<Manifest>>>,
    meta: Mutex<CacheMeta>,
    in_flight: AtomicBool,
}

/// The daemon's manifest source: cache, embedded fallback and the fetch
/// policy. Cheap to share (`Arc<ManifestService>`).
pub struct ManifestService {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// The cache directory for an instance directory.
pub fn cache_dir(instance_dir: &Path) -> PathBuf {
    instance_dir.join(CACHE_DIR)
}

/// Read `meta.json` from a cache directory (defaults when absent or bad).
pub fn read_meta(cache_dir: &Path) -> CacheMeta {
    std::fs::read(cache_dir.join(META_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

impl Inner {
    fn persist_meta(&self, meta: &CacheMeta) {
        match serde_json::to_vec(meta) {
            Ok(bytes) => {
                if let Err(error) = write_private(&self.dir.join(META_FILE), &bytes) {
                    (self.log)(&format!("harness manifest: cannot write meta: {error}"));
                }
            }
            Err(error) => (self.log)(&format!("harness manifest: cannot encode meta: {error}")),
        }
    }

    fn load_cache(&self) -> Option<Manifest> {
        parse(&read_capped(&self.dir.join(CACHE_FILE))?).ok()
    }

    fn current(&self) -> Arc<Manifest> {
        let mut cache = lock(&self.cache);
        if let Some(manifest) = cache.as_ref() {
            return Arc::clone(manifest);
        }
        let manifest = Arc::new(prefer_newer(self.load_cache(), embedded()));
        *cache = Some(Arc::clone(&manifest));
        manifest
    }

    fn run_fetch(&self, harness: &str) {
        // Only a valid cached body can be revalidated with its ETag.
        let etag = lock(&self.meta)
            .etag
            .clone()
            .filter(|_| self.load_cache().is_some());
        let url = manifest_url();
        let outcome = self.fetcher.fetch(&url, etag.as_deref());
        let now = self.clock.utc_now().0;
        match outcome {
            Ok(FetchOutcome::NotModified) => {
                let mut meta = lock(&self.meta);
                meta.fetched_at_ms = Some(now);
                self.persist_meta(&meta);
            }
            Ok(FetchOutcome::Body { bytes, etag }) => match parse(&bytes) {
                Ok(manifest) => {
                    if let Err(error) = write_private(&self.dir.join(CACHE_FILE), &bytes) {
                        (self.log)(&format!("harness manifest: cannot write cache: {error}"));
                        return;
                    }
                    let mut meta = lock(&self.meta);
                    meta.etag = etag;
                    meta.fetched_at_ms = Some(now);
                    self.persist_meta(&meta);
                    *lock(&self.cache) = Some(Arc::new(prefer_newer(Some(manifest), embedded())));
                    (self.log)(&format!("harness manifest: fetched for {harness}"));
                }
                Err(error) => (self.log)(&format!(
                    "harness manifest: fetched document ignored ({error}); keeping the current copy"
                )),
            },
            Err(FetchError::Offline) => (self.log)(
                "harness manifest: fetch skipped (curl not found); using the current copy",
            ),
            Err(FetchError::TooLarge) => (self.log)(&format!(
                "harness manifest: fetched document ignored (over {MAX_MANIFEST_BYTES} bytes)"
            )),
            Err(FetchError::Failed(detail)) => {
                (self.log)(&format!("harness manifest: fetch failed: {detail}"))
            }
        }
    }
}

impl crate::daemon::harness_evidence::RichManifestSource for ManifestService {
    fn contains(
        &self,
        harness: &str,
        identity: &super::runtime::RuntimeIdentity,
        domain: &str,
        origin: super::evidence::EvidenceOrigin,
        contract_id: &str,
    ) -> bool {
        let registry = super::registry::builtins();
        let Some(registration) = registry
            .agent(harness)
            .ok()
            .and_then(|id| registry.by_id(id).ok())
        else {
            return false;
        };
        let Some(descriptor) = registration.contracts().iter().find(|d| {
            d.domain_id == domain
                && d.origin == origin
                && d.contract_id_v2().ok().as_deref() == Some(contract_id)
        }) else {
            return false;
        };
        self.current()
            .runtime_row(harness, identity, descriptor)
            .is_some()
    }
}

/// Clears the in-flight flag however the fetch thread ends.
struct InFlight(Arc<Inner>);
impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.store(false, Ordering::Release);
    }
}

impl ManifestService {
    /// `cache_dir` is created (0700) if missing; failure to create it only
    /// disables caching (logged), never the service.
    pub fn new(
        cache_dir: PathBuf,
        policy: ManifestPolicy,
        fetcher: Arc<dyn Fetcher>,
        clock: Arc<dyn Clock>,
        log: LogSink,
    ) -> Self {
        {
            use std::os::unix::fs::DirBuilderExt;
            if let Err(error) = std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&cache_dir)
            {
                log(&format!(
                    "harness manifest: cannot create {}: {error}",
                    cache_dir.display()
                ));
            }
        }
        let mut meta = read_meta(&cache_dir);
        let (policy_word, source_word) = policy_words(policy);
        meta.daemon_policy = Some(RecordedPolicy {
            policy: policy_word.to_owned(),
            source: source_word.to_owned(),
            recorded_at_ms: clock.utc_now().0,
        });
        let this = Self {
            inner: Arc::new(Inner {
                dir: cache_dir,
                policy,
                fetcher,
                clock,
                log,
                cache: Mutex::new(None),
                meta: Mutex::new(meta.clone()),
                in_flight: AtomicBool::new(false),
            }),
        };
        // Logs on failure; never fails the service.
        this.inner.persist_meta(&meta);
        this
    }

    pub fn policy(&self) -> ManifestPolicy {
        self.inner.policy
    }

    /// The newer of a valid cached fetch and the embedded copy. Never touches
    /// the network.
    pub fn current(&self) -> Arc<Manifest> {
        self.inner.current()
    }

    /// Schedule a fetch when [`should_fetch`] says so, and return at once.
    /// The fetch runs on a detached thread (single flight: a call while one
    /// is running records its attempt, because the running fetch serves every
    /// harness, and returns); errors are logged and otherwise silent; an
    /// unsupported, oversized or invalid body never replaces a valid cache.
    pub fn ensure_manifest(&self, harness: &str, reason: FetchReason) {
        let inner = &self.inner;
        if let FetchReason::UnseenRuntime {
            identity,
            domain,
            origin,
            contract_id,
        } = &reason
            && crate::daemon::harness_evidence::RichManifestSource::contains(
                self,
                harness,
                identity,
                domain,
                *origin,
                contract_id,
            )
        {
            return;
        }
        let has_row = match &reason {
            FetchReason::UnseenVersion { version } => inner.current().has_row(harness, version),
            FetchReason::FreshViolation | FetchReason::UnseenRuntime { .. } => false,
        };
        let now = inner.clock.utc_now().0;
        {
            let meta = lock(&inner.meta);
            if should_fetch(inner.policy, &reason, harness, now, &meta, has_row) != Decision::Fetch
            {
                return;
            }
        }
        if inner
            .in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            lock(&inner.meta).attempts.insert(harness.to_owned(), now);
            return;
        }
        // Record the attempt in memory now (a failure still counts); the
        // thread persists it before any network I/O.
        lock(&inner.meta).attempts.insert(harness.to_owned(), now);
        let thread_inner = Arc::clone(inner);
        let harness_name = harness.to_owned();
        let spawned = std::thread::Builder::new()
            .name("harness-manifest-fetch".into())
            .spawn(move || {
                let guard = InFlight(Arc::clone(&thread_inner));
                {
                    let meta = lock(&thread_inner.meta).clone();
                    thread_inner.persist_meta(&meta);
                }
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    thread_inner.run_fetch(&harness_name)
                }));
                // Attempts recorded by calls during the flight become durable.
                let meta = lock(&thread_inner.meta).clone();
                thread_inner.persist_meta(&meta);
                drop(guard);
            });
        if let Err(error) = spawned {
            inner.in_flight.store(false, Ordering::Release);
            (inner.log)(&format!("harness manifest: cannot start fetch: {error}"));
        }
    }

    /// Wait until no fetch is running (tests join the detached thread).
    #[cfg(any(test, feature = "test-support"))]
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while self.inner.in_flight.load(Ordering::Acquire) {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a UTC millisecond timestamp.
pub fn format_rfc3339_utc(millis: i64) -> String {
    let seconds = millis.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
