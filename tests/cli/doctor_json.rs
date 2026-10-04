//! Shape of the doctor `hooks.<harness>.installed` object and the closed set
//! of admission strings (ht-p03.47). The report-level shape tests that run
//! the built executable in a scratch environment live in `tests/setup_cli.rs`.
//! Each test names the mutation it kills.

use super::*;

struct DoctorClock(std::sync::atomic::AtomicU64);
impl Clock for DoctorClock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(0)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}
#[derive(Clone, Copy)]
enum DoctorMode {
    Rich,
    Absent,
    Wrong,
    Failed,
    Timeout,
    CapabilityFailure,
}
struct DoctorClient {
    mode: DoctorMode,
    clock: Arc<DoctorClock>,
    calls: std::sync::Mutex<Vec<(String, u64)>>,
}
impl LocalClient for DoctorClient {
    fn call(
        &self,
        command: Command,
        budget: &CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.calls
            .lock()
            .unwrap()
            .push((format!("{command:?}"), budget.deadline.0));
        self.clock
            .0
            .fetch_add(600, std::sync::atomic::Ordering::SeqCst);
        match command {
            Command::Capabilities => {
                if matches!(self.mode, DoctorMode::CapabilityFailure) {
                    return Err(crate::protocol::results::ApiError::new(
                        ErrorCode::HostUnavailable,
                        "fixture capabilities unavailable",
                    ));
                }
                let capabilities = if matches!(self.mode, DoctorMode::Absent) {
                    vec![crate::protocol::capabilities::HARNESS_STATES.into()]
                } else {
                    vec![crate::protocol::capabilities::HARNESS_HEALTH_V2.into()]
                };
                Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList { capabilities },
                ))
            }
            Command::HarnessHealthV2 => match self.mode {
                DoctorMode::Wrong => Ok(CommandResult::HarnessStates(HarnessStatesReport {
                    harnesses: vec![],
                })),
                DoctorMode::Failed => Err(crate::protocol::results::ApiError::new(
                    ErrorCode::HostUnavailable,
                    "fixture v2 unavailable",
                )),
                DoctorMode::Timeout => Err(crate::protocol::results::ApiError::new(
                    ErrorCode::DeadlineExceeded,
                    "fixture v2 deadline elapsed",
                )),
                _ => Ok(CommandResult::HarnessHealthV2(
                    crate::protocol::results::HarnessHealthV2Report {
                        harnesses: Default::default(),
                    },
                )),
            },
            Command::HarnessStates => Ok(CommandResult::HarnessStates(HarnessStatesReport {
                harnesses: vec![],
            })),
            _ => panic!("unexpected doctor request"),
        }
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.call(command, budget)
    }
}

/// Kills legacy-only negotiation and per-request fresh budgets.
#[test]
fn doctor_negotiates_rich_health_once_with_decreasing_budget() {
    let clock = Arc::new(DoctorClock(std::sync::atomic::AtomicU64::new(100)));
    let client = DoctorClient {
        mode: DoctorMode::Rich,
        clock: Arc::clone(&clock),
        calls: Default::default(),
    };
    let erased: Arc<dyn Clock> = clock;
    let budget = CallBudget {
        deadline: MonoInstant(erased.monotonic_now().0 + HEALTH_BUDGET_MS),
        cancellation: Default::default(),
    };
    let details = daemon_details(&client, &budget);
    assert!(details.rich.is_ok());
    assert_eq!(
        *client.calls.lock().unwrap(),
        vec![
            ("Capabilities".into(), 2100),
            ("HarnessHealthV2".into(), 2100)
        ]
    );
}

/// Advertised failures stay unavailable with their cause; only absent capability
/// uses the labeled legacy projection, never a fabricated complete rich report.
#[test]
fn doctor_v2_absent_wrong_failed_and_timeout_remain_explicit() {
    for (mode, expected, calls) in [
        (
            DoctorMode::Absent,
            "does not advertise harness.health_v2",
            vec!["Capabilities", "HarnessStates"],
        ),
        (
            DoctorMode::Wrong,
            "returned another result",
            vec!["Capabilities", "HarnessHealthV2"],
        ),
        (
            DoctorMode::Failed,
            "fixture v2 unavailable",
            vec!["Capabilities", "HarnessHealthV2"],
        ),
        (
            DoctorMode::Timeout,
            "deadline elapsed",
            vec!["Capabilities", "HarnessHealthV2"],
        ),
        (
            DoctorMode::CapabilityFailure,
            "fixture capabilities unavailable",
            vec!["Capabilities"],
        ),
    ] {
        let clock = Arc::new(DoctorClock(std::sync::atomic::AtomicU64::new(100)));
        let client = DoctorClient {
            mode,
            clock,
            calls: Default::default(),
        };
        let budget = CallBudget {
            deadline: MonoInstant(2100),
            cancellation: Default::default(),
        };
        let details = daemon_details(&client, &budget);
        assert!(
            details
                .rich
                .as_ref()
                .err()
                .is_some_and(|why| why.contains(expected))
        );
        assert_eq!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            calls
        );
        assert!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|(_, deadline)| *deadline == 2100)
        );
        assert_eq!(details.states.is_ok(), matches!(mode, DoctorMode::Absent));
    }
}

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

use crate::harness::adapter::*;
struct ProfileDoctorAdapter {
    requests: std::sync::Mutex<Vec<ResolvedSetupScope>>,
    setup_deadlines: std::sync::Mutex<Vec<u64>>,
    safe: bool,
}
impl HarnessAdapter for ProfileDoctorAdapter {
    type Admission = ();
    fn metadata(&self) -> &'static AdapterMetadata {
        static META: AdapterMetadata = AdapterMetadata {
            id: "fourth",
            display_label: "Fourth",
            context_spelling: "Fourth",
            context_aliases: &[],
            executable: ExecutableLookup::Unsupported,
            host_kinds: &[],
            setup_scopes: &[SetupScopeKind::ConfigRoot, SetupScopeKind::Profile],
            runtime_sources: &[],
            budget: EventBudgetPolicy {
                lifecycle_ms: 5000,
                observer_ms: 1500,
            },
        };
        &META
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        &[]
    }
    fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
        panic!("doctor status must not use mutating installation observer")
    }
    fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
        panic!("no native admission")
    }
    fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
        panic!("no version inference")
    }
    fn classify(&self, _: &HookInput) -> ContractObservation {
        panic!("no callback")
    }
    fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
        panic!("no callback")
    }
    fn encode(
        &self,
        _: &(),
        _: &DecodedEvent,
        _: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        panic!("no callback")
    }
    fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
        panic!("no attribution")
    }
    fn resolve_setup_scope(
        &self,
        scope: &SetupScopeRequest,
        env: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        let root = env.config_roots["fourth"].clone();
        Ok(match scope {
            SetupScopeRequest::Default => ResolvedSetupScope::ConfigRoot(root),
            SetupScopeRequest::Profile(name) => ResolvedSetupScope::Profile {
                name: name.clone(),
                home: root.join(name),
            },
        })
    }
    fn status(&self, request: &StatusRequest, _: &CallBudget) -> SetupStatus {
        self.requests.lock().unwrap().push(request.scope.clone());
        let root = profile_root(&request.scope);
        SetupStatus::Detailed(Box::new(LocalSetupStatus {
            scope: request.scope.clone(),
            installed: root.join("owned").exists(),
            enabled: Some(false),
            admitted: Some(true),
            observed: None,
            configured_hook: None,
            fingerprint: None,
            diagnostics: vec![SetupDiagnostic::new(
                "manual_enable",
                DiagnosticSeverity::Info,
                "Enable manually; installation does not enable native callbacks",
            )],
            repairs: vec![LocalRepair::InstallOwned, LocalRepair::RemoveOwned],
            projection: json!({"installed": root.join("owned").exists(), "frozen_input": request.environment.declared["PROFILE_INPUT"].to_string_lossy()}),
        }))
    }
    fn doctor_projection(
        &self,
        request: &StatusRequest,
        _: &Value,
        _: &CallBudget,
    ) -> Option<DoctorProjection> {
        Some(DoctorProjection {
            status: None,
            hooks: json!({"owned": profile_root(&request.scope).join("owned").exists()}),
            limitations: vec![],
            manual_repairs: vec![
                json!({"action": "native enablement", "outcome": "manual", "detail": "Enable manually"}),
            ],
            safe_repairs: if self.safe && !profile_root(&request.scope).join("owned").exists() {
                vec![LocalRepair::InstallOwned]
            } else {
                vec![]
            },
            repair_options: Default::default(),
        })
    }
    fn setup(
        &self,
        request: &SetupRequest,
        budget: &CallBudget,
    ) -> Result<SetupOutcome, SetupFailure> {
        self.setup_deadlines.lock().unwrap().push(budget.deadline.0);
        let root = profile_root(&request.scope);
        std::fs::create_dir_all(root).map_err(SetupFailure::Io)?;
        std::fs::write(
            root.join("owned"),
            request.environment.declared["PROFILE_INPUT"].as_encoded_bytes(),
        )
        .map_err(SetupFailure::Io)?;
        Ok(SetupOutcome {
            actions: vec![SetupAction::InstalledOwned],
            diagnostic: String::new(),
            diagnostics: vec![],
            projection: json!({"action": "installed"}),
        })
    }
    fn unsetup(&self, _: &UnsetupRequest, _: &CallBudget) -> Result<RemovalOutcome, SetupFailure> {
        panic!("doctor must never remove")
    }
}
fn profile_root(scope: &ResolvedSetupScope) -> &std::path::Path {
    match scope {
        ResolvedSetupScope::ConfigRoot(root) => root,
        ResolvedSetupScope::Profile { home, .. } => home,
    }
}

/// Kills accepting a profile then inspecting the default, dropping frozen input,
/// authorizing repair from status operations alone, unsafe-state writes or enabling
/// callbacks just because aggregate evidence/owned files are present.
#[test]
fn doctor_actual_registered_profile_status_and_safe_owned_repair() {
    use crate::harness::registry::{Registration, Registry};
    let case = PathCase::new("registered-profile");
    let adapter = Box::leak(Box::new(ProfileDoctorAdapter {
        requests: Default::default(),
        setup_deadlines: Default::default(),
        safe: true,
    }));
    let registry = Registry::new(Box::leak(
        vec![
            Registration::new(adapter),
            Registration::new(&crate::harness::claude::ClaudeAdapter),
        ]
        .into_boxed_slice(),
    ))
    .unwrap();
    let state = case.dir.join("state");
    let host = case.dir.join("herdr.sock");
    let parsed = super::super::commands::parse_argv_in_registry(
        [
            std::ffi::OsString::from("herdr-threads"),
            "doctor".into(),
            "--json".into(),
            "--harness".into(),
            "fourth".into(),
            "--profile".into(),
            "work".into(),
            "--state-dir".into(),
            state.clone().into_os_string(),
            "--host-endpoint".into(),
            host.clone().into_os_string(),
        ],
        &registry,
    )
    .unwrap();
    let environment = SetupEnvironment {
        config_roots: [("fourth".into(), case.dir.join("profiles"))]
            .into_iter()
            .collect(),
        declared: [("PROFILE_INPUT".into(), "frozen-value".into())]
            .into_iter()
            .collect(),
        path: Some("/no-native-doctor-path".into()),
        state_dir: Some(state),
        host_endpoint: Some(host),
        executable: PathBuf::from("/isolated/herdr-threads"),
        ..Default::default()
    };
    let mut out = Vec::new();
    assert!(matches!(
        run_registered(&parsed, &registry, Some(&environment), &mut out),
        Err(RunError::Exit(exit::EXIT_UNAVAILABLE))
    ));
    let mut doc: Value = serde_json::from_slice::<Value>(&out).unwrap()["doctor"].clone();
    assert_eq!(doc["adapter_order"], json!(["fourth"]));
    assert_eq!(doc["local_harnesses"]["fourth"]["scope"]["profile"], "work");
    assert_eq!(doc["local_harnesses"]["fourth"]["enabled"], false);
    assert_eq!(doc["local_harnesses"]["fourth"]["observed"], Value::Null);
    assert!(doc["hooks"]["claude"].is_null());
    let scope = SetupScopeRequest::Profile("work".into());
    doc["daemon"]["state"] = json!("healthy");
    // All-scopes evidence does not alter selected-profile local axes.
    doc["harness_health_v2"] = json!({"harnesses": {"fourth": {"runtime_evidence": [{"scope": {"kind": "runtime_evidence_all_scopes"}, "state": "working"}]}}});
    let owned = collect_local(&mut doc, &registry, Some("fourth"), &scope, &environment);
    assert_eq!(doc["local_harnesses"]["fourth"]["observed"], Value::Null);
    assert_eq!(doc["local_harnesses"]["fourth"]["enabled"], false);
    let repairs = apply_owned_repairs(&doc, owned, &environment);
    assert_eq!(repairs[0]["action"], "setup fourth");
    assert_eq!(repairs[0]["outcome"], "attempted");
    assert_eq!(
        std::fs::read(case.dir.join("profiles/work/owned")).unwrap(),
        b"frozen-value"
    );
    assert!(!case.dir.join("profiles/owned").exists());
    let owned = collect_local(&mut doc, &registry, Some("fourth"), &scope, &environment);
    assert!(
        owned.is_empty(),
        "installed owned files are not automatically rewritten"
    );
    assert_eq!(doc["local_harnesses"]["fourth"]["enabled"], false);
    assert!(doc["local_harnesses"]["fourth"]["manual_repairs"][0]["detail"] == "Enable manually");
    std::fs::remove_file(case.dir.join("profiles/work/owned")).unwrap();
    let owned = collect_local(&mut doc, &registry, Some("fourth"), &scope, &environment);
    doc["state_dir"]["safe"] = json!(false);
    assert!(apply_owned_repairs(&doc, owned, &environment).is_empty());
    assert!(!case.dir.join("profiles/work/owned").exists());
    assert!(
        adapter
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|scope| matches!(scope, ResolvedSetupScope::Profile {name, ..} if name == "work"))
    );
    assert_eq!(adapter.setup_deadlines.lock().unwrap().len(), 1);
    let manual = Box::leak(Box::new(ProfileDoctorAdapter {
        requests: Default::default(),
        setup_deadlines: Default::default(),
        safe: false,
    }));
    let registry = Registry::new(Box::leak(
        vec![Registration::new(manual)].into_boxed_slice(),
    ))
    .unwrap();
    assert!(
        collect_local(&mut doc, &registry, Some("fourth"), &scope, &environment).is_empty(),
        "setup status operations do not authorize doctor repair"
    );
}

/// Kills bypassing the adapter's declared setup options at automatic repair.
#[test]
fn doctor_owned_repair_refuses_undeclared_options_before_writes() {
    use crate::harness::registry::{Registration, Registry};
    let case = PathCase::new("repair-options");
    let adapter = Box::leak(Box::new(ProfileDoctorAdapter {
        requests: Default::default(),
        setup_deadlines: Default::default(),
        safe: true,
    }));
    let registry = Registry::new(Box::leak(
        vec![Registration::new(adapter)].into_boxed_slice(),
    ))
    .unwrap();
    let environment = SetupEnvironment {
        executable: PathBuf::from("/isolated/herdr-threads"),
        declared: [("PROFILE_INPUT".into(), "frozen-value".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let report = json!({"context": {"ok": true}, "state_dir": {"safe": true}, "daemon": {"state": "healthy"}});
    let owned = vec![OwnedRepair {
        registration: &registry.registrations()[0],
        scope: ResolvedSetupScope::ConfigRoot(case.dir.clone()),
        options: [("undeclared".into(), true)].into_iter().collect(),
    }];
    let outcomes = apply_owned_repairs(&report, owned, &environment);
    assert_eq!(outcomes[0]["outcome"], "failed");
    assert!(!case.dir.join("owned").exists());
    assert!(adapter.setup_deadlines.lock().unwrap().is_empty());
}

/// Kills a second native version capture whose result can contradict the local row.
#[test]
fn doctor_concrete_projection_uses_one_native_capture() {
    let case = PathCase::new("single-capture");
    let count = case.dir.join("count");
    let binary = case.dir.join("claude");
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf 'x\\n' >> '{}'\nprintf '2.1.286 (Claude Code)\\n'\n",
            count.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let state = case.dir.join("state");
    let host = case.dir.join("herdr.sock");
    let registry = crate::harness::registry::builtins();
    let parsed = super::super::commands::parse_argv_in_registry(
        [
            std::ffi::OsString::from("herdr-threads"),
            "doctor".into(),
            "--json".into(),
            "--harness".into(),
            "claude".into(),
            "--state-dir".into(),
            state.clone().into_os_string(),
            "--host-endpoint".into(),
            host.clone().into_os_string(),
        ],
        registry,
    )
    .unwrap();
    let environment = SetupEnvironment {
        config_roots: [("claude".into(), case.dir.join("config"))]
            .into_iter()
            .collect(),
        path: Some(case.dir.clone().into_os_string()),
        state_dir: Some(state),
        host_endpoint: Some(host),
        executable: PathBuf::from("/isolated/herdr-threads"),
        ..Default::default()
    };
    let mut out = Vec::new();
    let _ = run_registered(&parsed, registry, Some(&environment), &mut out);
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "x\n");
    let doc: Value = serde_json::from_slice::<Value>(&out).unwrap()["doctor"].clone();
    assert_eq!(doc["hooks"]["claude"]["installed"]["admission"], "listed");
    assert_eq!(doc["local_harnesses"]["claude"]["admitted"], true);
}

/// Kills a legacy native observation retaining its five-second deadline instead
/// of the decreasing doctor observation budget. The version helper owns/reaps sleep.
#[test]
fn doctor_local_native_capture_is_clamped_to_shared_doctor_deadline() {
    let case = PathCase::new("bounded-capture");
    let binary = case.dir.join("claude");
    std::fs::write(
        &binary,
        "#!/bin/sh\n/bin/sleep 3\nprintf '2.1.286 (Claude Code)\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let state = case.dir.join("state");
    let host = case.dir.join("herdr.sock");
    let registry = crate::harness::registry::builtins();
    let parsed = super::super::commands::parse_argv_in_registry(
        [
            std::ffi::OsString::from("herdr-threads"),
            "doctor".into(),
            "--json".into(),
            "--harness".into(),
            "claude".into(),
            "--state-dir".into(),
            state.clone().into_os_string(),
            "--host-endpoint".into(),
            host.clone().into_os_string(),
        ],
        registry,
    )
    .unwrap();
    let environment = SetupEnvironment {
        config_roots: [("claude".into(), case.dir.join("config"))]
            .into_iter()
            .collect(),
        path: Some(case.dir.clone().into_os_string()),
        state_dir: Some(state),
        host_endpoint: Some(host),
        executable: PathBuf::from("/isolated/herdr-threads"),
        ..Default::default()
    };
    let mut out = Vec::new();
    let _ = run_registered(&parsed, registry, Some(&environment), &mut out);
    let doc: Value = serde_json::from_slice::<Value>(&out).unwrap()["doctor"].clone();
    assert_eq!(doc["hooks"]["claude"]["installed"]["version"], Value::Null);
}
