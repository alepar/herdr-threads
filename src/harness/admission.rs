//! The harness admission ladder and the generated versions document.
//!
//! The recipe tables (`claude::RECIPES`, `codex::RECIPES`) stay authoritative.
//! [`classify`] maps any installed-version string to exactly one row of the
//! ladder, evaluated top to bottom, first match wins:
//!
//! 1. unparsable: refused;
//! 2. inside any recipe's `known_broken` range: refused (even when listed, or
//!    newer than every max);
//! 3. listed by a recipe: listed;
//! 4. not older than every recipe min, and the binary's schema fingerprint
//!    matches a recipe (Codex only): schema-matched;
//! 5. older than every recipe min: refused;
//! 6. otherwise optimistic: 6a newer than every recipe max, 6b inside the
//!    supported span (a gap between recipes, or inside a recipe's version set).
//!
//! An optimistic admission assumes a recipe (6a: the recipe with the greatest
//! max; 6b: the recipe whose `[min, max]` contains the version, else the one
//! with the greatest min at or below it), is live-unverified, and carries the
//! data the operator-facing label is rendered from.
//!
//! [`versions_document`] renders the tables as
//! `docs/compatibility/harness-versions.json`; a guard test fails on drift.
use super::recipe::{Evidence, Recipe, Version, VersionSet};
use serde_json::{Value, json};

/// Where a person reports an optimistic admission that went wrong.
pub const ISSUES_URL: &str = "https://github.com/alepar/herdr-threads/issues";

/// The committed versions document.
pub const VERSIONS_JSON_PATH: &str = "docs/compatibility/harness-versions.json";

/// Where an optimistic version sits relative to the verified versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Newer than every recipe's max (row 6a).
    NewerThanVerified,
    /// Inside the supported span but not listed (row 6b).
    WithinSpan,
}

impl Placement {
    /// Stable kebab-case word for evidence lines.
    pub fn label(self) -> &'static str {
        match self {
            Self::NewerThanVerified => "newer-than-verified",
            Self::WithinSpan => "within-span",
        }
    }
}

/// An optimistic admission: parsed under an assumed recipe, live-unverified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimisticAdmission {
    /// The greatest listed version over the table.
    pub verified_max: Version,
    /// Id of the recipe the version is parsed under.
    pub assumed_recipe: &'static str,
    /// The observed major version exceeds the assumed recipe's max major.
    pub major_version_change: bool,
    pub placement: Placement,
    pub issues_url: &'static str,
}

/// Why a version is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Row 1.
    Unparsable,
    /// Row 2: the range that contains the version, and the newest listed
    /// version below it that is not itself known broken.
    KnownBroken {
        range: VersionSet,
        newest_working: Option<Version>,
    },
    /// Row 5.
    OlderThanSupported(Version),
}

/// The ladder row a version reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row<'a, P: 'static> {
    Refused(Refusal),
    Listed(&'a Recipe<P>),
    SchemaMatched(&'a Recipe<P>),
    Optimistic {
        recipe: &'a Recipe<P>,
        admission: OptimisticAdmission,
    },
}

fn known_broken_range<P>(table: &[Recipe<P>], version: Version) -> Option<VersionSet> {
    table
        .iter()
        .flat_map(|recipe| recipe.known_broken.iter())
        .find(|range| range.contains(version))
        .copied()
}

fn newest_working<P>(table: &[Recipe<P>], range: &VersionSet) -> Option<Version> {
    let VersionSet::Interval { min: Some(min), .. } = range else {
        return None;
    };
    table
        .iter()
        .flat_map(Recipe::listed_versions)
        .filter(|listed| listed < min && known_broken_range(table, *listed).is_none())
        .max()
}

/// Rows 1-6 in first-match order. `schema_match` is evaluated lazily, only
/// when rows 1-3 did not match and the version is not older than every min
/// (row 4); it returns the recipe whose captured fingerprint the binary
/// matches, or `None` (differs, unreadable, or a harness without
/// fingerprints, which is Claude).
pub fn classify<'a, P>(
    table: &'a [Recipe<P>],
    text: &str,
    schema_match: impl FnOnce() -> Option<&'a Recipe<P>>,
) -> Row<'a, P> {
    let Some(version) = Version::parse(text) else {
        return Row::Refused(Refusal::Unparsable);
    };
    if let Some(range) = known_broken_range(table, version) {
        let newest_working = newest_working(table, &range);
        return Row::Refused(Refusal::KnownBroken {
            range,
            newest_working,
        });
    }
    if let Some(recipe) = table
        .iter()
        .find(|recipe| recipe.versions.contains(version))
    {
        return Row::Listed(recipe);
    }
    let older_than_every_min = table.iter().all(|recipe| version < recipe.min_version());
    if !older_than_every_min && let Some(recipe) = schema_match() {
        return Row::SchemaMatched(recipe);
    }
    if older_than_every_min {
        return Row::Refused(Refusal::OlderThanSupported(version));
    }
    let newer_than_every_max = table.iter().all(|recipe| version > recipe.max_version());
    let (assumed, placement) = if newer_than_every_max {
        (
            table
                .iter()
                .max_by_key(|recipe| recipe.max_version())
                .expect("a version newer than every max implies a non-empty table"),
            Placement::NewerThanVerified,
        )
    } else {
        let spanning = table
            .iter()
            .find(|recipe| recipe.min_version() <= version && version <= recipe.max_version());
        let assumed = spanning.unwrap_or_else(|| {
            table
                .iter()
                .filter(|recipe| recipe.min_version() <= version)
                .max_by_key(|recipe| recipe.min_version())
                .expect("a version not older than every min has a recipe at or below it")
        });
        (assumed, Placement::WithinSpan)
    };
    let verified_max = table
        .iter()
        .map(Recipe::max_version)
        .max()
        .unwrap_or(assumed.max_version());
    Row::Optimistic {
        recipe: assumed,
        admission: OptimisticAdmission {
            verified_max,
            assumed_recipe: assumed.id,
            major_version_change: version.major > assumed.max_version().major,
            placement,
            issues_url: ISSUES_URL,
        },
    }
}

/// Operator-facing refusal for a version inside a known-broken range.
pub fn known_broken_message(
    harness: &str,
    version: &str,
    range: &VersionSet,
    newest_working: Option<Version>,
) -> String {
    let working = newest_working.map_or_else(
        || "no listed version is known to work".to_owned(),
        |version| format!("the newest working version is {version}"),
    );
    format!(
        "{harness} {version} is inside the known-broken range {range}; {working}. \
         Install a working version"
    )
}

/// `{"min": "X.Y.Z"|null, "max": "X.Y.Z"|null}` per known-broken range. An
/// `Exact` entry (never compiled in) encodes as one single-version range each.
fn ranges_json(known_broken: &[VersionSet]) -> Vec<Value> {
    let range = |min: Option<Version>, max: Option<Version>| json!({"min": min.map(|v| v.to_string()), "max": max.map(|v| v.to_string())});
    known_broken
        .iter()
        .flat_map(|set| match set {
            VersionSet::Interval { min, max } => vec![range(*min, *max)],
            VersionSet::Exact(versions) => versions
                .iter()
                .map(|version| range(Some(*version), Some(*version)))
                .collect(),
        })
        .collect()
}

/// One document row per (harness, listed version) of `table`, ordered by version.
pub fn version_rows<P>(harness: &str, table: &[Recipe<P>]) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut listed: Vec<(Version, &Recipe<P>)> = table
        .iter()
        .flat_map(|recipe| {
            recipe
                .listed_versions()
                .into_iter()
                .map(move |version| (version, recipe))
        })
        .collect();
    listed.sort_by_key(|(version, _)| *version);
    for (version, recipe) in listed {
        let evidence = recipe.evidence_level(version).unwrap_or(Evidence::None);
        rows.push(json!({
            "harness": harness,
            "version": version.to_string(),
            "status": "verified",
            "evidence": evidence,
            "contract_id": null,
            "source": "manual",
            "supported_since": null,
            "broken_event": null,
            "broken_field": null,
            "last_working": null,
            "issue_url": null,
            "recipe": recipe.id,
            "known_broken": ranges_json(recipe.known_broken),
        }));
    }
    rows
}

/// The versions document generated from the compiled recipe tables, in the
/// manifest's schema 2 (see `harness::manifest`). The canary-only fields are
/// present and null: the canary writer (not this generator) fills them.
/// Rows carry a null `contract_id`, so a daemon never takes their status
/// from the manifest (the compiled recipe tables already cover them).
pub fn versions_document() -> Value {
    let mut rows = version_rows("claude", super::claude::RECIPES);
    rows.extend(version_rows("codex", super::codex::RECIPES));
    json!({
        "schema_version": 2,
        "generated_from": "src/harness/claude.rs RECIPES, src/harness/codex.rs RECIPES (cargo test versions_json_guard; HT_BLESS=1 regenerates)",
        "generated_at": null,
        "latest_release": null,
        "contracts": {},
        "rows": rows,
    })
}

/// Document key order, so the committed file reads harness-first.
const KEY_ORDER: [&str; 21] = [
    "schema_version",
    "generated_from",
    "generated_at",
    "latest_release",
    "contracts",
    "rows",
    "harness",
    "version",
    "status",
    "evidence",
    "contract_id",
    "source",
    "supported_since",
    "broken_event",
    "broken_field",
    "last_working",
    "issue_url",
    "recipe",
    "known_broken",
    "min",
    "max",
];

fn write_pretty(value: &Value, depth: usize, out: &mut String) {
    let pad = |depth: usize| "  ".repeat(depth);
    match value {
        Value::Object(map) if !map.is_empty() => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by_key(|key| {
                (
                    KEY_ORDER
                        .iter()
                        .position(|known| known == key)
                        .unwrap_or(KEY_ORDER.len()),
                    (*key).clone(),
                )
            });
            out.push_str("{\n");
            for (index, key) in keys.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                out.push_str(&Value::String((*key).clone()).to_string());
                out.push_str(": ");
                write_pretty(&map[*key], depth + 1, out);
                out.push_str(if index + 1 < keys.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                write_pretty(item, depth + 1, out);
                out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// Pretty-printed with a trailing newline and a stable key order: the
/// committed file's exact bytes.
pub fn render_versions_document(document: &Value) -> String {
    let mut text = String::new();
    write_pretty(document, 0, &mut text);
    text.push('\n');
    text
}

type KeyedRow<'a> = ((String, String), &'a Value);

fn keyed_rows(document: &Value) -> Result<Vec<KeyedRow<'_>>, String> {
    let rows = document
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("versions document has no `rows` array")?;
    rows.iter()
        .map(|row| {
            let field = |key: &str| {
                row.get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or(format!("a row has no string `{key}`: {row}"))
            };
            Ok(((field("harness")?, field("version")?), row))
        })
        .collect()
}

/// Compare a committed versions document with a generated one; the error
/// names the first row (or top-level field) that differs.
pub fn diff_versions_documents(committed: &Value, generated: &Value) -> Result<(), String> {
    let committed_rows = keyed_rows(committed)?;
    let generated_rows = keyed_rows(generated)?;
    for ((harness, version), row) in &generated_rows {
        match committed_rows
            .iter()
            .find(|(key, _)| key == &(harness.clone(), version.clone()))
        {
            None => return Err(format!("row {harness} {version} is missing from the file")),
            Some((_, found)) if found != row => {
                return Err(format!(
                    "row {harness} {version} differs: file has {found}, recipes give {row}"
                ));
            }
            Some(_) => (),
        }
    }
    for ((harness, version), _) in &committed_rows {
        if !generated_rows
            .iter()
            .any(|(key, _)| key == &(harness.clone(), version.clone()))
        {
            return Err(format!(
                "row {harness} {version} is in the file but in no recipe"
            ));
        }
    }
    for key in ["schema_version", "generated_from"] {
        if committed.get(key) != generated.get(key) {
            return Err(format!("`{key}` differs"));
        }
    }
    let order = |rows: &[KeyedRow<'_>]| rows.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
    if order(&committed_rows) != order(&generated_rows) {
        return Err("rows are in a different order".into());
    }
    Ok(())
}

/// Parse a `{"min":..,"max":..}` range.
#[cfg(any(test, feature = "test-support"))]
fn parse_range(value: &Value) -> Result<VersionSet, String> {
    let bound = |key: &str| -> Result<Option<Version>, String> {
        match value.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Version::parse(text)
                .map(Some)
                .ok_or(format!("known_broken `{key}` is not X.Y.Z: {text:?}")),
            Some(other) => Err(format!("known_broken `{key}` is not a string: {other}")),
        }
    };
    Ok(VersionSet::Interval {
        min: bound("min")?,
        max: bound("max")?,
    })
}

/// Test-only override (builds with `cfg(test)` or the `test-support` feature;
/// release builds contain none of this, not even the variable name):
/// `HT_TEST_RECIPES_JSON=<path>` names a versions document whose `harness`
/// rows replace the compiled `compiled` table for admission, so known-broken
/// refusals can be exercised before any real break exists. Rows group by
/// `recipe` id into a leaked, per-path cached table of `Exact` version sets;
/// the profile and evidence paths are the compiled recipe's with that id, else
/// the first compiled recipe's. `None` when the variable is unset or the file has no rows for
/// `harness`. A malformed document panics: this is test scaffolding. The
/// profile must be `Send + Sync` so the leaked table can be cached.
#[cfg(any(test, feature = "test-support"))]
pub fn override_table<P: Copy + Send + Sync + 'static>(
    harness: &str,
    compiled: &'static [Recipe<P>],
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<&'static [Recipe<P>]> {
    use std::any::{Any, TypeId};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Key = (TypeId, String, std::ffi::OsString);
    type Cache = Mutex<HashMap<Key, &'static (dyn Any + Send + Sync)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();

    let path = lookup("HT_TEST_RECIPES_JSON")?;
    let key: Key = (TypeId::of::<P>(), harness.to_owned(), path.clone());
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(&key) {
        return cached
            .downcast_ref::<Option<&'static [Recipe<P>]>>()
            .copied()
            .flatten();
    }
    let table = build_override(harness, compiled, &path);
    cache.insert(key, Box::leak(Box::new(table)));
    table
}

#[cfg(any(test, feature = "test-support"))]
fn build_override<P: Copy + 'static>(
    harness: &str,
    compiled: &'static [Recipe<P>],
    path: &std::ffi::OsStr,
) -> Option<&'static [Recipe<P>]> {
    fn leak<T>(value: T) -> &'static T {
        Box::leak(Box::new(value))
    }
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("HT_TEST_RECIPES_JSON {path:?} unreadable: {error}"));
    let document: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("HT_TEST_RECIPES_JSON {path:?} is not JSON: {error}"));
    let rows = document
        .get("rows")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("HT_TEST_RECIPES_JSON {path:?} has no rows array"));
    struct Group {
        id: String,
        versions: Vec<Version>,
        levels: Vec<(Version, Evidence)>,
        broken: Vec<VersionSet>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for row in rows {
        if row.get("harness").and_then(Value::as_str) != Some(harness) {
            continue;
        }
        let text = |key: &str| {
            row.get(key)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("override row has no string `{key}`: {row}"))
        };
        let version = Version::parse(text("version"))
            .unwrap_or_else(|| panic!("override row version is not X.Y.Z: {row}"));
        let level: Evidence = serde_json::from_value(row["evidence"].clone())
            .unwrap_or_else(|error| panic!("override row evidence invalid ({error}): {row}"));
        let id = text("recipe");
        let index = groups
            .iter()
            .position(|group| group.id == id)
            .unwrap_or_else(|| {
                groups.push(Group {
                    id: id.to_owned(),
                    versions: Vec::new(),
                    levels: Vec::new(),
                    broken: Vec::new(),
                });
                groups.len() - 1
            });
        let group = &mut groups[index];
        group.versions.push(version);
        group.levels.push((version, level));
        for range in row
            .get("known_broken")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            let range = parse_range(range).unwrap_or_else(|error| panic!("{error}"));
            if !group.broken.contains(&range) {
                group.broken.push(range);
            }
        }
    }
    if groups.is_empty() || compiled.is_empty() {
        return None;
    }
    let recipes: Vec<Recipe<P>> = groups
        .into_iter()
        .map(|mut group| {
            group.versions.sort();
            group.versions.dedup();
            group.levels.sort_by_key(|(version, _)| *version);
            group.levels.dedup_by_key(|(version, _)| *version);
            let basis = compiled
                .iter()
                .find(|recipe| recipe.id == group.id)
                .unwrap_or(&compiled[0]);
            Recipe {
                id: leak(group.id).as_str(),
                versions: VersionSet::Exact(leak(group.versions).as_slice()),
                evidence: basis.evidence,
                scope: "HT_TEST_RECIPES_JSON override",
                evidence_levels: leak(group.levels).as_slice(),
                known_broken: leak(group.broken).as_slice(),
                profile: basis.profile,
            }
        })
        .collect();
    Some(leak(recipes).as_slice())
}
