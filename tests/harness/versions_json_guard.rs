//! `docs/compatibility/harness-versions.json` is generated from the recipe
//! tables; this guard fails on drift. `HT_BLESS=1` regenerates the file.
use crate::harness::admission::{
    self, VERSIONS_JSON_PATH, diff_versions_documents, override_table, render_versions_document,
    version_rows, versions_document,
};
use crate::harness::claude::{self, ClaudeProfile, InputSchema};
use crate::harness::codex;
use crate::harness::recipe::{Evidence, NativeSupport, Recipe, Version, VersionSet};
use serde_json::{Value, json};

fn committed_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(VERSIONS_JSON_PATH)
}

#[test]
fn versions_json_matches_recipes() {
    let generated = versions_document();
    if std::env::var_os("HT_BLESS").is_some_and(|value| value == "1") {
        std::fs::write(committed_path(), render_versions_document(&generated)).unwrap();
        return;
    }
    let text = std::fs::read_to_string(committed_path())
        .expect("docs/compatibility/harness-versions.json exists (HT_BLESS=1 regenerates)");
    let committed: Value = serde_json::from_str(&text).expect("committed file is JSON");
    admission::diff_versions_documents(&committed, &generated).unwrap_or_else(|drift| {
        panic!("harness-versions.json drifted from the recipes: {drift} (HT_BLESS=1 regenerates)")
    });
    assert_eq!(
        text,
        render_versions_document(&generated),
        "the committed file is not the canonical rendering"
    );
}

#[test]
fn generated_document_lists_every_version_in_order_with_evidence() {
    let document = versions_document();
    let rows: Vec<(String, String, String, String)> = document["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let text = |key: &str| row[key].as_str().unwrap().to_owned();
            (
                text("harness"),
                text("version"),
                text("recipe"),
                text("evidence"),
            )
        })
        .collect();
    let row = |a: &str, b: &str, c: &str, d: &str| (a.into(), b.into(), c.into(), d.into());
    assert_eq!(
        rows,
        vec![
            row("claude", "2.1.283", "claude-hooks-2.1.283", "live"),
            row("claude", "2.1.284", "claude-hooks-2.1.283", "no_model"),
            row("claude", "2.1.285", "claude-hooks-2.1.283", "live"),
            row("claude", "2.1.286", "claude-hooks-2.1.283", "live"),
            row("claude", "2.1.287", "claude-hooks-2.1.283", "live"),
            row("codex", "0.157.1", "codex-hooks-v1", "no_model"),
            row("codex", "0.158.0", "codex-hooks-v1", "live"),
            row("codex", "0.159.3", "codex-hooks-v1", "live"),
        ]
    );
    assert!(
        document["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["known_broken"] == json!([]))
    );
    assert_eq!(document["schema_version"], 1);
}

/// Kills: a guard that only compares row counts: a changed evidence level,
/// a dropped row, an added known_broken range and an extra row each fail,
/// naming the row.
#[test]
fn planted_drift_is_detected() {
    let generated = versions_document();
    assert_eq!(diff_versions_documents(&generated, &generated), Ok(()));

    let mut evidence = generated.clone();
    evidence["rows"][1]["evidence"] = json!("live");
    let error = diff_versions_documents(&evidence, &generated).unwrap_err();
    assert!(error.contains("claude 2.1.284"), "{error}");

    let mut dropped = generated.clone();
    dropped["rows"].as_array_mut().unwrap().remove(5);
    let error = diff_versions_documents(&dropped, &generated).unwrap_err();
    assert!(error.contains("codex 0.157.1"), "{error}");

    let mut broken = generated.clone();
    broken["rows"][6]["known_broken"] = json!([{"min": "0.160.0", "max": null}]);
    let error = diff_versions_documents(&broken, &generated).unwrap_err();
    assert!(error.contains("codex 0.158.0"), "{error}");

    let mut extra = generated.clone();
    extra["rows"].as_array_mut().unwrap().push(json!({
        "harness": "codex", "version": "0.159.0", "recipe": "codex-hooks-v1",
        "evidence": "none", "known_broken": []}));
    let error = diff_versions_documents(&extra, &generated).unwrap_err();
    assert!(error.contains("codex 0.159.0"), "{error}");

    let mut reordered = generated.clone();
    reordered["rows"].as_array_mut().unwrap().swap(0, 1);
    assert!(diff_versions_documents(&reordered, &generated).is_err());
}

const fn v(major: u32, minor: u32, patch: u32) -> Version {
    Version::new(major, minor, patch)
}

/// The exact form canary report.py's known_broken snippet emits.
static SNIPPET: &[Recipe<ClaudeProfile>] = &[Recipe {
    id: "claude-hooks-2.1.283",
    versions: VersionSet::Exact(&[v(2, 1, 286)]),
    evidence: &[],
    scope: "planted",
    evidence_levels: &[(v(2, 1, 286), Evidence::Live)],
    known_broken: &[VersionSet::Interval {
        min: Some(Version::new(2, 1, 290)),
        max: None,
    }],
    profile: ClaudeProfile {
        input_schema: InputSchema::Hooks2_1_283,
        model_receipt: NativeSupport::Unsupported,
    },
}];

#[test]
fn known_broken_snippet_round_trips() {
    let rows = version_rows("claude", SNIPPET);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["known_broken"],
        json!([{"min": "2.1.290", "max": null}])
    );
    let path = std::env::temp_dir().join(format!(
        "herdr-threads-snippet-roundtrip-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"schema_version": 1, "rows": rows})).unwrap(),
    )
    .unwrap();
    let key = path.clone().into_os_string();
    let table = override_table("claude", claude::RECIPES, |name| {
        (name == "HT_TEST_RECIPES_JSON").then(|| key.clone())
    })
    .expect("override");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        table[0].known_broken,
        &[VersionSet::Interval {
            min: Some(Version::new(2, 1, 290)),
            max: None
        }]
    );
    // And the closed and both-open encodings.
    static BOTH: [VersionSet; 2] = [
        VersionSet::Interval {
            min: None,
            max: Some(v(1, 0, 0)),
        },
        VersionSet::Interval {
            min: Some(v(1, 0, 0)),
            max: Some(v(1, 0, 2)),
        },
    ];
    let encoded = admission_ranges(&BOTH);
    assert_eq!(
        encoded,
        json!([{"min": null, "max": "1.0.0"}, {"min": "1.0.0", "max": "1.0.2"}])
    );
}

const ONE: &[Version] = &[v(1, 0, 0)];
const ONE_LEVEL: &[(Version, Evidence)] = &[(v(1, 0, 0), Evidence::None)];

fn admission_ranges(ranges: &'static [VersionSet]) -> Value {
    let table = [Recipe {
        id: "r",
        versions: VersionSet::Exact(ONE),
        evidence: &[],
        scope: "planted",
        evidence_levels: ONE_LEVEL,
        known_broken: ranges,
        profile: SNIPPET[0].profile,
    }];
    version_rows("claude", &table)[0]["known_broken"].clone()
}

// -- registry invariants --------------------------------------------------

fn overlaps<P>(a: &Recipe<P>, b: &Recipe<P>) -> bool {
    a.min_version() <= b.max_version() && b.min_version() <= a.max_version()
}

fn check_registry<P>(table: &[Recipe<P>]) {
    for (index, recipe) in table.iter().enumerate() {
        let listed = recipe.listed_versions();
        assert!(!listed.is_empty(), "{}", recipe.id);
        // evidence_levels cover exactly the listed versions, once each.
        let mut levels: Vec<Version> = recipe.evidence_levels.iter().map(|(v, _)| *v).collect();
        levels.sort();
        let mut expected = listed.clone();
        expected.sort();
        assert_eq!(
            levels, expected,
            "{} evidence_levels vs listed versions",
            recipe.id
        );
        let mut deduped = levels.clone();
        deduped.dedup();
        assert_eq!(
            deduped, levels,
            "{} has a duplicate evidence entry",
            recipe.id
        );
        if let VersionSet::Interval { min, max } = recipe.versions {
            let (min, max) = (min.expect("closed interval"), max.expect("closed interval"));
            assert_eq!(
                (min.major, min.minor),
                (max.major, max.minor),
                "{}",
                recipe.id
            );
            let patches: Vec<Version> = (min.patch..=max.patch)
                .map(|patch| Version::new(min.major, min.minor, patch))
                .collect();
            assert_eq!(
                expected, patches,
                "{} must enumerate every patch",
                recipe.id
            );
        }
        for range in recipe.known_broken {
            assert!(
                matches!(range, VersionSet::Interval { .. }),
                "{}",
                recipe.id
            );
        }
        for other in &table[index + 1..] {
            assert!(
                !overlaps(recipe, other),
                "{} overlaps {}",
                recipe.id,
                other.id
            );
        }
    }
}

#[test]
fn compiled_registries_satisfy_the_evidence_and_known_broken_invariants() {
    check_registry(claude::RECIPES);
    check_registry(codex::RECIPES);
    // Recipe version sets are bounded; known_broken is where open bounds live.
    for recipe in claude::RECIPES {
        if let VersionSet::Interval { min, max } = recipe.versions {
            assert!(min.is_some() && max.is_some(), "{}", recipe.id);
        }
    }
    assert!(claude::RECIPES.iter().all(|r| r.known_broken.is_empty()));
    assert!(codex::RECIPES.iter().all(|r| r.known_broken.is_empty()));
}
