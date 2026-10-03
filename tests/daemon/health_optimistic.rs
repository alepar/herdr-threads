//! ht-p03.23: Health renders the optimistic admission and the known-broken
//! refusal truthfully, counts hook payloads not understood, and derives its
//! version text from the recipe tables. Each test names the mutation it kills.

use super::health_budget::{ready_inputs, worst_case_inputs};
use super::*;
use crate::app::{claude_status, claude_status_in, codex_status, supports_native_receipt};
use crate::harness::{
    admission::{self, Refusal, Row},
    claude,
    codex::{InstalledAdmission, VERSION_TIMEOUT, VersionError},
    recipe::Version,
    stub_binaries::write_stub_harness,
};
use serde_json::json;

fn verified_max() -> Version {
    claude::RECIPES
        .iter()
        .map(|recipe| recipe.max_version())
        .max()
        .unwrap()
}

fn observed(version: &str) -> Option<Result<String, VersionError>> {
    Some(Ok(version.to_owned()))
}

fn newer_version() -> String {
    let max = verified_max();
    format!("{}.{}.{}", max.major, max.minor, max.patch + 13)
}

/// A scratch directory holding a versions document that makes `claude`
/// 2.1.283 and 2.1.286 listed (a gap at 2.1.284 and 2.1.285) and, optionally,
/// a known-broken range.
fn override_table(
    tag: &str,
    broken: Option<(&str, &str)>,
) -> &'static [crate::harness::claude::ClaudeRecipe] {
    let dir =
        std::env::temp_dir().join(format!("ht-health-optimistic-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let document = dir.join("versions.json");
    let known_broken = broken
        .map(|(min, max)| vec![json!({"min": min, "max": max})])
        .unwrap_or_default();
    std::fs::write(
        &document,
        serde_json::to_vec(&json!({"schema_version": 1, "rows": [
            {"harness": "claude", "version": "2.1.283",
             "recipe": "claude-hooks-2.1.283", "evidence": "live", "known_broken": []},
            {"harness": "claude", "version": "2.1.286",
             "recipe": "claude-hooks-2.1.283", "evidence": "live",
             "known_broken": known_broken},
        ]}))
        .unwrap(),
    )
    .unwrap();
    let table = claude::admission_table_with(|key| {
        (key == "HT_TEST_RECIPES_JSON").then(|| document.clone().into_os_string())
    });
    let _ = std::fs::remove_dir_all(&dir);
    table
}

/// Kills: an optimistic admission that stays a limitation (it degraded Health
/// before), one that is dropped from Health, and one that flips the state.
#[test]
fn optimistic_is_an_informational_note_not_degraded() {
    let mut inputs = ready_inputs();
    let version = newer_version();
    inputs.claude = claude_status(observed(&version), CapabilityState::Unsupported);
    assert!(matches!(inputs.claude, HarnessStatus::Optimistic(_)));
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy, "{health:#?}");
    let expected = format!(
        "harness claude: claude {version}: optimistic \u{2014} newer than verified {}, assumed \
         compatible with recipe {}",
        verified_max(),
        claude::RECIPES[0].id
    );
    assert!(health.notes.contains(&expected), "{:#?}", health.notes);
    assert!(
        health
            .limitations
            .iter()
            .all(|line| !line.contains("optimistic")),
        "{:#?}",
        health.limitations
    );
    assert_eq!(health.harness.claude, HarnessState::Cooperative);
}

/// Kills: a Health line budget that the note and the parse-failure lines push
/// past 12 lines or past the wire cap, and notes folded away from the front.
#[test]
fn worst_case_line_budget_still_holds_with_note_and_parse_failures() {
    let mut inputs = worst_case_inputs();
    let long = "y".repeat(400);
    inputs.claude = HarnessStatus::Optimistic(long.clone());
    inputs.codex = HarnessStatus::Optimistic(long);
    inputs.hook_parse_failures = vec![("claude".into(), u64::MAX), ("codex".into(), u64::MAX)];
    let health = inputs.assemble();
    assert!(
        health.limitations.len() <= HEALTH_LINE_BUDGET,
        "{:#?}",
        health.limitations
    );
    assert!(
        health.notes.len() <= HEALTH_LINE_BUDGET,
        "{:#?}",
        health.notes
    );
    health.validate().expect("the wire cap still holds");
    for harness in ["claude", "codex"] {
        assert!(
            health.notes.contains(&format!(
                "{} hook payloads not understood ({harness})",
                u64::MAX
            )),
            "{:#?}",
            health.notes
        );
        assert!(
            health
                .notes
                .iter()
                .any(|line| line.starts_with(&format!("harness {harness}: yyy"))),
            "{:#?}",
            health.notes
        );
    }
}

/// Kills: `supported` for a version that is not listed (the Wave 26 latent
/// bug: Codex native receipt alone made a schema-matched or optimistic
/// version supported), and `supported` without a native receipt.
#[test]
fn supported_requires_native_receipt_and_listed() {
    assert!(supports_native_receipt(CapabilityState::Supported, true));
    assert!(!supports_native_receipt(CapabilityState::Supported, false));
    assert!(!supports_native_receipt(CapabilityState::Unsupported, true));

    // Claude: a listed version with a native receipt is supported; an
    // unlisted one never is; a listed one without a receipt is cooperative.
    let listed = format!("{}", verified_max());
    assert!(matches!(
        claude_status(observed(&listed), CapabilityState::Supported),
        HarnessStatus::Supported(_)
    ));
    assert!(matches!(
        claude_status(observed(&newer_version()), CapabilityState::Supported),
        HarnessStatus::Optimistic(_)
    ));
    assert!(matches!(
        claude_status(observed(&listed), CapabilityState::Unsupported),
        HarnessStatus::Cooperative {
            live_unverified: false,
            ..
        }
    ));

    // Codex: an optimistic version (a stub no recipe lists, carrying no
    // schemas) is not supported even when the receipt capability is.
    let dir = std::env::temp_dir().join(format!("ht-health-codex-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    write_stub_harness(&dir, "codex", "0.199.0");
    let admission = InstalledAdmission::observe_on_path(Some(dir.as_os_str()), VERSION_TIMEOUT);
    let status = codex_status(&admission, CapabilityState::Supported);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(status, HarnessStatus::Optimistic(_)), "{status:?}");
}

/// Kills: a known-broken refusal that omits the range or the newest working
/// version in Health, with the range planted through the test-only recipe
/// override.
#[test]
fn known_broken_text_in_health() {
    let table = override_table("broken", Some(("2.1.286", "2.1.286")));
    let Row::Refused(Refusal::KnownBroken {
        range,
        newest_working,
    }) = admission::classify(table, "2.1.286", || None)
    else {
        panic!("2.1.286 must be refused as known broken");
    };
    let status = claude_status_in(
        table,
        Some(Err(VersionError::KnownBroken {
            version: "2.1.286".into(),
            range,
            newest_working,
        })),
        CapabilityState::Unsupported,
    );
    let mut inputs = ready_inputs();
    inputs.claude = status;
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert!(
        health.limitations.contains(&format!(
            "harness claude unsupported: claude 2.1.286: refused: known broken in {range}; \
             newest working: 2.1.283"
        )),
        "{:#?}",
        health.limitations
    );
}

/// Kills: the within-span placement rendering the newer-than-verified
/// wording (or none), and a missing assumed recipe id.
#[test]
fn within_span_label_renders() {
    let table = override_table("span", None);
    let status = claude_status_in(table, observed("2.1.285"), CapabilityState::Unsupported);
    let HarnessStatus::Optimistic(detail) = &status else {
        panic!("2.1.285 sits in the gap between listed versions: {status:?}");
    };
    assert_eq!(
        detail,
        "claude 2.1.285: optimistic \u{2014} unlisted within the supported span, assumed \
         compatible with recipe claude-hooks-2.1.283"
    );
    let mut inputs = ready_inputs();
    inputs.claude = status;
    let health = inputs.assemble();
    assert!(
        health
            .notes
            .iter()
            .any(|line| line.contains("unlisted within the supported span")),
        "{:#?}",
        health.notes
    );
}

/// Kills: the major-version flag lost on the way to Health.
#[test]
fn major_version_change_in_health() {
    let mut inputs = ready_inputs();
    inputs.claude = claude_status(observed("9.0.0"), CapabilityState::Unsupported);
    let health = inputs.assemble();
    assert!(
        health.notes.iter().any(|line| line
            .starts_with("harness claude: claude 9.0.0: optimistic")
            && line.ends_with("; major version change")),
        "{:#?}",
        health.notes
    );
}

/// Kills: a count line that is rendered for zero failures, mislabelled, or
/// that degrades Health.
#[test]
fn hook_parse_failure_count_line() {
    let mut inputs = ready_inputs();
    inputs.hook_parse_failures = vec![("claude".into(), 3), ("codex".into(), 0)];
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy);
    assert!(
        health
            .notes
            .contains(&"3 hook payloads not understood (claude)".to_owned()),
        "{:#?}",
        health.notes
    );
    assert!(
        health.notes.iter().all(|line| !line.contains("(codex)")),
        "{:#?}",
        health.notes
    );
}

/// Kills: a parse failure that is logged every time (no rate limit), one that
/// is not counted past the first, and counts that mix harnesses.
#[test]
fn parse_failure_is_rate_limited_in_daemon_log_and_counted() {
    use crate::daemon::logs::{HookParseFailures, RateLimitedLaneLog};
    use std::sync::{Arc, Mutex};
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink_lines = Arc::clone(&lines);
    let log = Arc::new(RateLimitedLaneLog::new(
        Arc::new(crate::app::SystemClock::new()),
        Arc::new(move |line: &str| sink_lines.lock().unwrap().push(line.to_owned())),
    ));
    let failures = HookParseFailures::new(log);
    for _ in 0..3 {
        failures.record("claude", "Invalid");
    }
    failures.record("codex", "Invalid");
    assert_eq!(
        failures.snapshot(),
        vec![("claude".to_owned(), 3), ("codex".to_owned(), 1)]
    );
    let lines = lines.lock().unwrap();
    assert_eq!(
        *lines,
        vec![
            "hook claude: parse_failure: Invalid".to_owned(),
            "hook codex: parse_failure: Invalid".to_owned()
        ],
        "one line per harness inside the window"
    );
}

/// Kills: the cooperative receipt line naming versions of its own instead of
/// the recipe tables', and a line past the Health line bound.
#[test]
fn receipt_line_is_derived_from_the_recipe_tables() {
    let line = cooperative_receipt_line();
    for recipe in claude::RECIPES.iter() {
        assert!(line.contains(&recipe.versions.to_string()), "{line}");
    }
    for recipe in crate::harness::codex::RECIPES.iter() {
        assert!(line.contains(&recipe.versions.to_string()), "{line}");
    }
    assert!(line.len() <= 256, "{} bytes: {line}", line.len());
}

/// True when `text` holds a `digits.digits.digits` run (a harness version).
fn has_version(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit()
            && (index == 0 || !bytes[index - 1].is_ascii_alphanumeric())
        {
            let mut cursor = index;
            let mut groups = 0;
            loop {
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                if cursor == start {
                    break;
                }
                groups += 1;
                if groups == 3 {
                    return true;
                }
                if cursor < bytes.len() && bytes[cursor] == b'.' {
                    cursor += 1;
                } else {
                    break;
                }
            }
        }
        index += 1;
    }
    false
}

fn code_lines(source: &str) -> String {
    let end = source.find("#[cfg(test)]").unwrap_or(source.len());
    source[..end]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Kills: a harness version string typed into Health, setup or the docs'
/// recipe table instead of derived from the recipe tables.
#[test]
fn no_hardcoded_harness_versions() {
    assert!(
        has_version("claude 2.1.286") && !has_version("protocol 1.2") && !has_version("sha256")
    );
    for (name, source) in [
        (
            "src/daemon/health.rs",
            include_str!("../../src/daemon/health.rs"),
        ),
        ("src/cli/setup.rs", include_str!("../../src/cli/setup.rs")),
    ] {
        for line in code_lines(source).lines() {
            assert!(!has_version(line), "{name} hard-codes a version: {line}");
        }
    }
    let docs = include_str!("../../docs/compatibility/harnesses.md");
    let rows: Vec<&str> = docs
        .lines()
        .filter(|line| line.starts_with("| `claude-hooks-") || line.starts_with("| `codex-hooks-"))
        .collect();
    assert_eq!(
        rows.len(),
        2,
        "the recipe registry table has one row per recipe"
    );
    for row in rows {
        let versions = row.split('|').nth(2).unwrap();
        assert!(
            !has_version(versions),
            "docs version list hard-coded: {versions}"
        );
    }
}
