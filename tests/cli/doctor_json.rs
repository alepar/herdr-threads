//! Shape of the doctor `hooks.<harness>.installed` object and the closed set
//! of admission strings (ht-p03.47). The report-level shape tests that run
//! the built executable in a scratch environment live in `tests/setup_cli.rs`.
//! Each test names the mutation it kills.

use super::*;

/// Kills: a renamed serde string, a reordered or dropped variant, and a
/// string that drifts from the Codex labels the evidence file stores.
#[test]
fn admission_strings_are_the_closed_set() {
    let strings: Vec<Value> = AdmissionState::ALL
        .iter()
        .map(|state| serde_json::to_value(state).unwrap())
        .collect();
    assert_eq!(
        strings,
        [
            "listed",
            "schema-matched, live-unverified",
            "optimistic",
            "refused",
            "not_found"
        ]
    );
    for state in AdmissionState::ALL {
        assert_eq!(json!(state), json!(state.as_str()), "{state:?}");
    }
}

/// Kills: a stub that reports admitted, or a non-null binary/version/recipe
/// for "no claude on PATH", or a missing key (absent instead of null).
#[test]
fn claude_stub_is_not_found_with_null_fields() {
    let value = serde_json::to_value(claude_installed_stub()).unwrap();
    assert_eq!(
        value,
        json!({"binary": null, "version": null, "admission": "not_found", "recipe": null})
    );
}

/// Kills: the typed Codex mapping disagreeing with `InstalledAdmission::state`
/// (the string the evidence file and `last_hook` use), for the states that can
/// be built without a binary.
#[test]
fn codex_not_found_maps_to_the_same_string_as_state() {
    let codex = crate::harness::codex::InstalledAdmission::observe_on_path(
        Some(std::ffi::OsStr::new("/nonexistent-ht-p03-47")),
        crate::harness::codex::VERSION_TIMEOUT,
    );
    let installed = codex_installed_json(&codex);
    assert_eq!(installed.admission.as_str(), codex.state());
    assert_eq!(installed.admission, AdmissionState::NotFound);
    assert_eq!(installed.binary, None);
}

// -- the claude PATH check (ht-p03.49) -------------------------------------

use crate::harness::stub_binaries::write_stub_harness;

struct PathCase {
    dir: PathBuf,
}

impl PathCase {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ht-doctor-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }

    fn installed(
        &self,
        lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
    ) -> InstalledHarnessJson {
        claude_installed_on(Some(self.dir.as_os_str()), lookup)
    }
}

impl Drop for PathCase {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn no_env(_: &str) -> Option<std::ffi::OsString> {
    None
}

/// Kills: a classification that ignores the ladder (listed/optimistic swapped
/// or collapsed), a known_broken version admitted, the override read from the
/// process environment instead of the injected lookup, a binary that is not
/// the resolved absolute path, a version that is not the observed X.Y.Z, and a
/// recipe set on a refusal or on not_found.
#[test]
fn doctor_classifies_claude_on_path() {
    let listed = PathCase::new("listed");
    let binary = write_stub_harness(&listed.dir, "claude", "2.1.286");
    let got = listed.installed(no_env);
    assert_eq!(got.admission, AdmissionState::Listed);
    assert_eq!(got.version.as_deref(), Some("2.1.286"));
    assert_eq!(got.recipe.as_deref(), Some("claude-hooks-2.1.283"));
    assert_eq!(got.binary, Some(binary.display().to_string()));

    let newer = PathCase::new("newer");
    write_stub_harness(&newer.dir, "claude", "2.1.299");
    let got = newer.installed(no_env);
    assert_eq!(got.admission, AdmissionState::Optimistic);
    assert_eq!(got.version.as_deref(), Some("2.1.299"));
    assert_eq!(got.recipe.as_deref(), Some("claude-hooks-2.1.287"));

    let broken = PathCase::new("broken");
    write_stub_harness(&broken.dir, "claude", "2.1.286");
    let document = broken.dir.join("versions.json");
    std::fs::write(
        &document,
        serde_json::to_vec(&json!({"schema_version": 1, "rows": [
            {"harness": "claude", "version": "2.1.285",
             "recipe": "claude-hooks-2.1.283", "evidence": "live", "known_broken": []},
            {"harness": "claude", "version": "2.1.286",
             "recipe": "claude-hooks-2.1.283", "evidence": "live",
             "known_broken": [{"min": "2.1.286", "max": "2.1.286"}]},
        ]}))
        .unwrap(),
    )
    .unwrap();
    let got = broken.installed(|key| {
        (key == "HT_TEST_RECIPES_JSON").then(|| document.clone().into_os_string())
    });
    assert_eq!(got.admission, AdmissionState::Refused);
    assert_eq!(got.version.as_deref(), Some("2.1.286"));
    assert_eq!(got.recipe, None);
    assert!(got.binary.is_some());
    // Without the injected override the same binary is listed: the process
    // environment was not consulted or mutated.
    assert_eq!(broken.installed(no_env).admission, AdmissionState::Listed);

    let empty = PathCase::new("empty");
    assert_eq!(empty.installed(no_env), claude_installed_stub());
    let value = serde_json::to_value(empty.installed(no_env)).unwrap();
    assert_eq!(
        value,
        json!({"binary": null, "version": null, "admission": "not_found", "recipe": null})
    );
}

/// Kills: an unrunnable or unrecognizable claude reported as not_found or
/// admitted, and a version invented for output the parser rejects.
#[test]
fn doctor_refuses_a_claude_with_no_readable_version() {
    let case = PathCase::new("garbled");
    let path = case.dir.join("claude");
    std::fs::write(&path, "#!/bin/sh\nprintf 'banana\\n'\n").unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let got = case.installed(no_env);
    assert_eq!(got.admission, AdmissionState::Refused);
    assert_eq!(got.version, None);
    assert_eq!(got.recipe, None);
    assert_eq!(got.binary, Some(path.display().to_string()));
}

/// Kills: no PATH line, two PATH lines, a line without the admission state,
/// and the not-found wording drifting.
#[test]
fn doctor_text_has_one_claude_path_line() {
    let report_for =
        |installed: &InstalledHarnessJson| json!({"hooks": {"claude": {"installed": installed}}});
    let case = PathCase::new("text");
    let binary = write_stub_harness(&case.dir, "claude", "2.1.299");
    let text = render_debug_text(&report_for(&case.installed(no_env)));
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("claude on PATH:"))
        .collect();
    assert_eq!(
        lines,
        [format!(
            "claude on PATH: {} 2.1.299 (optimistic)",
            binary.display()
        )]
    );

    let text = render_debug_text(&report_for(&claude_installed_stub()));
    let lines: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("claude on PATH:"))
        .collect();
    assert_eq!(lines, ["claude on PATH: not found"]);
}

#[test]
fn doctor_debug_text_shows_effective_wake_batch_setting() {
    let report = json!({
        "daemon": {"state": "healthy", "settings": {
            "invitation_default_ms": 300000,
            "receipt_default_ms": 300000,
            "minimum_wake_delay_ms": 30000,
            "wake_batch_delay_ms": 0
        }}
    });
    let text = render_debug_text(&report);
    assert!(
        text.contains("daemon.settings.wake_batch_delay_ms: 0\n"),
        "{text}"
    );
}

// -- version honesty (ht-p03.23) -------------------------------------------

fn warning_for(case: &PathCase) -> Option<String> {
    claude_installed_with_warning(Some(case.dir.as_os_str()), no_env).1
}

/// Kills: no doctor warning for an optimistic claude, a warning without the
/// issues URL or the assumed recipe, a warning for a listed claude, and one
/// that is not in the text report.
#[test]
fn doctor_optimistic_warning() {
    let case = PathCase::new("optimistic");
    write_stub_harness(&case.dir, "claude", "2.1.299");
    let warning = warning_for(&case).expect("an optimistic claude earns a warning");
    // Newer than every recipe: the recipe with the greatest max is assumed.
    let recipe = crate::harness::claude::RECIPES
        .iter()
        .max_by_key(|recipe| recipe.max_version())
        .unwrap()
        .id;
    assert!(
        warning.starts_with("claude 2.1.299: optimistic \u{2014} newer than verified "),
        "{warning}"
    );
    assert!(
        warning.contains(&format!("assumed compatible with recipe {recipe}")),
        "{warning}"
    );
    assert!(
        warning.ends_with(&format!(
            "(report issues: {})",
            crate::harness::admission::ISSUES_URL
        )),
        "{warning}"
    );

    let text = render_debug_text(&json!({"hooks": {"claude": {
        "installed": case.installed(no_env),
        "admission_warning": warning,
    }}}));
    assert!(
        text.lines()
            .any(|line| line.starts_with("hooks.claude.warning: claude 2.1.299: optimistic")),
        "{text}"
    );

    let listed = PathCase::new("listed-warning");
    write_stub_harness(&listed.dir, "claude", "2.1.286");
    assert_eq!(warning_for(&listed), None);
    assert_eq!(warning_for(&PathCase::new("absent-warning")), None);
}

/// Kills: a newer-major claude without the "major version change" words in
/// the doctor warning.
#[test]
fn doctor_major_version_change() {
    let case = PathCase::new("major");
    write_stub_harness(&case.dir, "claude", "3.0.0");
    let warning = warning_for(&case).unwrap();
    assert!(warning.contains("; major version change"), "{warning}");
    let minor = PathCase::new("minor");
    write_stub_harness(&minor.dir, "claude", "2.1.299");
    assert!(
        !warning_for(&minor)
            .unwrap()
            .contains("major version change")
    );
}

/// Kills: a known-broken refusal that names no range or no newest working
/// version in doctor, and one that is not planted through the injected recipe
/// override.
#[test]
fn doctor_known_broken_text() {
    let case = PathCase::new("known-broken");
    write_stub_harness(&case.dir, "claude", "2.1.286");
    let document = case.dir.join("versions.json");
    std::fs::write(
        &document,
        serde_json::to_vec(&json!({"schema_version": 1, "rows": [
            {"harness": "claude", "version": "2.1.285",
             "recipe": "claude-hooks-2.1.283", "evidence": "live", "known_broken": []},
            {"harness": "claude", "version": "2.1.286",
             "recipe": "claude-hooks-2.1.283", "evidence": "live",
             "known_broken": [{"min": "2.1.286", "max": "2.1.286"}]},
        ]}))
        .unwrap(),
    )
    .unwrap();
    let (installed, warning) = claude_installed_with_warning(Some(case.dir.as_os_str()), |key| {
        (key == "HT_TEST_RECIPES_JSON").then(|| document.clone().into_os_string())
    });
    assert_eq!(installed.admission, AdmissionState::Refused);
    assert_eq!(
        warning.as_deref(),
        Some(
            "claude 2.1.286: refused: known broken in [2.1.286, 2.1.286]; newest working: 2.1.285"
        )
    );
    assert_eq!(
        warning_for(&case),
        None,
        "without the override it is listed"
    );
}

// -- the harness manifest policy (ht-xoc.3) ---------------------------------

fn manifest_case(tag: &str) -> PathCase {
    PathCase::new(&format!("manifest-{tag}"))
}

/// Kills: a policy line that ignores settings.json or the environment, a
/// missing key (absent instead of null), a cache line for a never-fetched
/// cache, and an error that is swallowed into a default policy.
#[test]
fn doctor_reports_the_manifest_policy_and_cache() {
    use std::ffi::OsStr;
    let case = manifest_case("policy");
    let report = harness_manifest_report(&case.dir, None);
    assert_eq!(
        report,
        json!({"policy": "auto", "source": "default", "settings_error": null,
               "cache_fetched_at": null, "cache_etag": null, "embedded_schema_version": 2,
               "policy_from": "this environment", "policy_recorded_at": null})
    );
    let text = harness_manifest_text(&report);
    assert_eq!(
        text,
        "harness manifest: auto\nmanifest cache: never fetched (using the embedded copy)\n"
    );

    let env = harness_manifest_report(&case.dir, Some(OsStr::new("1")));
    assert_eq!(
        (env["policy"].as_str(), env["source"].as_str()),
        (Some("off"), Some("offline_env"))
    );
    assert!(harness_manifest_text(&env).starts_with(
        "harness manifest: off (HERDR_THREADS_OFFLINE=1 in this environment; \
             the daemon reads its own environment at start)\n"
    ));
    let not_one = harness_manifest_report(&case.dir, Some(OsStr::new("0")));
    assert_eq!(not_one["policy"], "auto");

    std::fs::write(
        case.dir.join("settings.json"),
        br#"{"harness_manifest":"off"}"#,
    )
    .unwrap();
    let off = harness_manifest_report(&case.dir, None);
    assert_eq!(
        (off["policy"].as_str(), off["source"].as_str()),
        (Some("off"), Some("settings"))
    );
    assert!(harness_manifest_text(&off).starts_with("harness manifest: off (settings.json)\n"));
    // Settings win the label when the environment also says offline.
    let both = harness_manifest_report(&case.dir, Some(OsStr::new("1")));
    assert_eq!(both["source"], "settings");

    let cache = case.dir.join("harness-manifest");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("meta.json"),
        br#"{"etag":"W/\"abc\"","fetched_at_ms":1790000123000,"attempts":{}}"#,
    )
    .unwrap();
    let cached = harness_manifest_report(&case.dir, None);
    assert_eq!(cached["cache_fetched_at"], "2026-09-21T14:15:23Z");
    assert!(
        harness_manifest_text(&cached)
            .ends_with("manifest cache: fetched 2026-09-21T14:15:23Z (etag W/\"abc\")\n")
    );
    // Attempts without a successful fetch still read as never fetched.
    std::fs::write(cache.join("meta.json"), br#"{"attempts":{"claude":5}}"#).unwrap();
    assert_eq!(
        harness_manifest_report(&case.dir, None)["cache_fetched_at"],
        Value::Null
    );

    std::fs::write(
        case.dir.join("settings.json"),
        br#"{"harness_manifest":"maybe"}"#,
    )
    .unwrap();
    let bad = harness_manifest_report(&case.dir, None);
    assert_eq!(
        (bad["policy"].clone(), bad["source"].clone()),
        (Value::Null, Value::Null)
    );
    let error = bad["settings_error"].as_str().unwrap();
    assert!(error.contains("settings.json"), "{error}");
    assert!(harness_manifest_text(&bad).starts_with("harness manifest: settings error: "));
}

/// Kills: doctor computing the policy from its own environment when the
/// daemon recorded its effective policy at start.
#[test]
fn doctor_reports_the_daemon_recorded_policy() {
    let case = manifest_case("daemon-policy");
    let cache = case.dir.join("harness-manifest");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join("meta.json"),
        br#"{"attempts":{},"daemon_policy":{"policy":"off","source":"offline_env","recorded_at_ms":1790000123000}}"#,
    )
    .unwrap();
    let report = harness_manifest_report(&case.dir, None);
    assert_eq!(report["policy"], "off");
    assert_eq!(report["source"], "offline_env");
    assert_eq!(report["policy_from"], "daemon");
    assert_eq!(report["policy_recorded_at"], "2026-09-21T14:15:23Z");
    assert!(harness_manifest_text(&report).starts_with(
        "harness manifest: off (HERDR_THREADS_OFFLINE=1 in the daemon's environment; \
         recorded at daemon start 2026-09-21T14:15:23Z)\n"
    ));
}

/// Kills: doctor rejecting the daemon's timing keys (one schema for both
/// readers) or accepting an out-of-range timing value.
#[test]
fn doctor_accepts_timing_keys_and_mixed_settings() {
    let case = manifest_case("mixed");
    let file = case.dir.join("settings.json");
    std::fs::write(
        &file,
        br#"{"invitation_default_ms":120000,"receipt_default_ms":240000,"minimum_wake_delay_ms":45000}"#,
    )
    .unwrap();
    let timing = harness_manifest_report(&case.dir, None);
    assert_eq!(timing["settings_error"], Value::Null);
    assert_eq!(timing["policy"], "auto");
    assert_eq!(timing["source"], "default");

    std::fs::write(
        &file,
        br#"{"invitation_default_ms":120000,"receipt_default_ms":240000,"minimum_wake_delay_ms":45000,"harness_manifest":"off"}"#,
    )
    .unwrap();
    let mixed = harness_manifest_report(&case.dir, None);
    assert_eq!(mixed["settings_error"], Value::Null);
    assert_eq!(
        (mixed["policy"].as_str(), mixed["source"].as_str()),
        (Some("off"), Some("settings"))
    );
    assert!(harness_manifest_text(&mixed).starts_with("harness manifest: off (settings.json)\n"));

    std::fs::write(&file, br#"{"minimum_wake_delay_ms":1}"#).unwrap();
    let bad = harness_manifest_report(&case.dir, None);
    let error = bad["settings_error"].as_str().unwrap();
    assert!(error.contains("settings.json"), "{error}");
}

/// Kills: a report that computes the manifest object but never attaches it
/// (or never prints it) in the full doctor output.
#[test]
fn doctor_report_and_text_carry_the_manifest_policy() {
    let case = manifest_case("report");
    let state = case.dir.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let endpoint = case.dir.join("herdr.sock");
    let context =
        crate::daemon::paths::RuntimeContext::explicit(state.clone(), endpoint.clone(), None)
            .unwrap();
    let instance = crate::daemon::paths::instance_dir(&context);
    std::fs::create_dir_all(&instance).unwrap();
    std::fs::write(
        instance.join("settings.json"),
        br#"{"harness_manifest":"off"}"#,
    )
    .unwrap();
    let (report, _) = report(Some(state), Some(endpoint));
    assert_eq!(report["harness_manifest"]["policy"], "off");
    assert_eq!(report["harness_manifest"]["source"], "settings");
    assert_eq!(report["harness_manifest"]["embedded_schema_version"], 2);
    assert!(
        report["limitations"]
            .as_array()
            .is_none_or(|limitations| limitations
                .iter()
                .all(|l| !l.to_string().contains("manifest"))),
        "informational only"
    );
    let text = render_debug_text(&report);
    assert!(
        text.contains("harness manifest: off (settings.json)\n"),
        "{text}"
    );
    assert!(
        text.contains("manifest cache: never fetched (using the embedded copy)\n"),
        "{text}"
    );
}

/// Kills: a `harness_states` key that is missing when the daemon answers or
/// not null with a reason when it cannot (ht-xoc.5).
#[test]
fn doctor_json_carries_harness_states_or_the_reason() {
    use crate::protocol::results::{HarnessStateReport, HarnessStatesReport};
    let state = HarnessStatesReport {
        harnesses: vec![HarnessStateReport {
            harness: "claude".into(),
            contract_id: None,
            detected: None,
            versions: Vec::new(),
            unattributed: None,
            hook_parse_failures: 0,
        }],
    };
    let value = serde_json::to_value(&state).unwrap();
    assert_eq!(value["harnesses"][0]["harness"], "claude");
    assert!(value["harnesses"][0]["contract_id"].is_null());
    // No daemon in a scratch state dir: the report says why.
    let case = manifest_case("states-none");
    let state = case.dir.join("state");
    std::fs::create_dir_all(&state).unwrap();
    let (report, _) = report(Some(state), Some(case.dir.join("herdr.sock")));
    assert!(report["harness_states"].is_null(), "{report}");
    assert_eq!(
        report["harness_states_unavailable"], "the daemon is not running",
        "{report}"
    );
    let text = render_debug_text(&report);
    assert!(
        text.contains("harness states unavailable: the daemon is not running\n"),
        "{text}"
    );
}
