//! agy doctor (D10): the `hooks.agy` projection, limitations, repairs, the bounded witness-health
//! scan and the Debug and Summary text, in temp HOMEs and state dirs only.
//! Included from `src/harness/agy/doctor.rs`; layouts are `tests/fixtures/agy/layouts/*.txt`.
use super::*;
use crate::harness::{
    adapter::{
        HarnessAdapter, LocalRepair, ResolvedSetupScope, SetupEnvironment, SetupRequest,
        SetupStatus,
    },
    agy::{
        AgyAdapter,
        witness::tests::{Temp, build_layout},
    },
};
use crate::protocol::time::MonoInstant;
use std::path::PathBuf;

struct Fixture {
    _temp: Temp,
    home: PathBuf,
    state: PathBuf,
    exe: PathBuf,
    roots: AgyRoots,
}
impl Fixture {
    /// A temp HOME holding the layout `manifest` (or nothing for `None`).
    fn new(manifest: Option<&str>) -> Self {
        let temp = Temp::new();
        let home = temp.0.join("home");
        let state = temp.0.join("state");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        if let Some(manifest) = manifest {
            build_layout(&home, manifest);
        }
        let roots = AgyRoots::from_home(&home);
        Self {
            exe: temp.0.join("bin/herdr-threads"),
            _temp: temp,
            home,
            state,
            roots,
        }
    }
    fn env(&self) -> SetupEnvironment {
        SetupEnvironment {
            home: Some(self.home.clone().into()),
            executable: self.exe.clone(),
            state_dir: Some(self.state.clone()),
            host_endpoint: Some(self.state.join("host.sock")),
            ..Default::default()
        }
    }
    fn request(&self) -> StatusRequest {
        StatusRequest {
            scope: ResolvedSetupScope::ConfigRoot(self.roots.config_root.clone()),
            environment: self.env(),
            native_binary: None,
        }
    }
    fn setup(&self) {
        AgyAdapter
            .setup(
                &SetupRequest {
                    scope: ResolvedSetupScope::ConfigRoot(self.roots.config_root.clone()),
                    executable: self.exe.clone(),
                    environment: self.env(),
                    native_binary: None,
                    options: Default::default(),
                },
                &budget(),
            )
            .unwrap();
    }
    fn projection(&self) -> DoctorProjection {
        AgyAdapter
            .doctor_projection(&self.request(), &json!({}), &budget())
            .expect("agy projects a doctor block")
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Default::default(),
    }
}
fn health(fixture: &Fixture) -> WitnessHealth {
    witness_health(&fixture.roots, SystemTime::now(), &|| false)
}

#[test]
fn healthy_install_has_no_limitations_and_reports_witness_and_sandbox() {
    let fixture = Fixture::new(Some("top-level"));
    fixture.setup();
    let projection = fixture.projection();
    assert_eq!(projection.limitations, Vec::<String>::new());
    assert!(projection.safe_repairs.is_empty());
    assert!(projection.manual_repairs.is_empty());
    assert!(matches!(projection.status, Some(SetupStatus::Detailed(_))));
    assert_eq!(projection.hooks["plugin"]["installed"], json!(true));
    assert_eq!(projection.hooks["allow_entry"]["state"], json!("owned"));
    assert_eq!(
        projection.hooks["witness"],
        json!({"presence_dir": true, "brain_entries": 1, "bounded": true, "recent_top_level_without_lock": 0})
    );
    assert_eq!(projection.hooks["sandbox"], json!(SANDBOX_NOTE));
    assert_eq!(
        SANDBOX_NOTE,
        "agy --sandbox sessions may not reach the herdr-threads daemon socket (untested); launch refuses --sandbox"
    );
}

#[test]
fn missing_plugin_is_a_limitation_with_a_safe_repair() {
    let fixture = Fixture::new(Some("top-level"));
    let projection = fixture.projection();
    assert_eq!(
        projection.limitations[0],
        "agy plugin is not installed: run `herdr-threads setup agy`"
    );
    assert!(matches!(
        projection.safe_repairs[..],
        [LocalRepair::RepairOwned]
    ));
    assert!(projection.manual_repairs.is_empty());
}

#[test]
fn an_edited_plugin_file_reads_as_not_installed() {
    let fixture = Fixture::new(Some("top-level"));
    fixture.setup();
    std::fs::write(fixture.roots.plugin_dir.join("hooks.json"), "{}").unwrap();
    let projection = fixture.projection();
    assert!(
        projection.limitations[0].starts_with("agy plugin is not installed"),
        "{:?}",
        projection.limitations
    );
}

#[test]
fn disabled_plugin_is_a_limitation_with_a_manual_repair_only() {
    let fixture = Fixture::new(Some("top-level"));
    fixture.setup();
    std::fs::write(
        &fixture.roots.plugins_config,
        r#"{"plugins":{"herdr-threads":{"enabled":false}}}"#,
    )
    .unwrap();
    let projection = fixture.projection();
    assert_eq!(
        projection.limitations,
        ["agy plugin is disabled: run `agy plugin enable herdr-threads`"]
    );
    assert_eq!(
        projection.manual_repairs[0]["action"],
        json!("agy plugin enable herdr-threads")
    );
    assert!(projection.safe_repairs.is_empty());
}

#[test]
fn missing_allow_entry_is_a_limitation_with_a_safe_repair() {
    let fixture = Fixture::new(Some("top-level"));
    fixture.setup();
    std::fs::write(&fixture.roots.settings, "{}").unwrap();
    let projection = fixture.projection();
    assert_eq!(projection.hooks["allow_entry"]["state"], json!("missing"));
    assert_eq!(
        projection.limitations,
        [
            "agy allow entry command(herdr-threads) is missing from antigravity-cli/settings.json: run `herdr-threads setup agy`"
        ]
    );
    assert!(matches!(
        projection.safe_repairs[..],
        [LocalRepair::RepairOwned]
    ));
}

#[test]
fn recent_conversations_without_a_lock_are_counted_and_limited() {
    let fixture = Fixture::new(Some("recent-without-lock"));
    fixture.setup();
    let projection = fixture.projection();
    assert_eq!(
        projection.hooks["witness"]["recent_top_level_without_lock"],
        json!(2)
    );
    assert_eq!(projection.hooks["witness"]["presence_dir"], json!(false));
    assert_eq!(
        projection.limitations,
        ["SessionStart without witness: 2 recent top-level conversations lack a presence lock"]
    );
}

#[test]
fn the_scan_skips_listed_locked_and_old_conversations() {
    // A conversation another scanned one lists is a child: only the parent counts.
    assert_eq!(
        health(&Fixture::new(Some("recent-listed"))).recent_top_level_without_lock,
        1
    );
    // A presence lock makes it a known top-level conversation.
    assert_eq!(
        health(&Fixture::new(Some("top-level"))).recent_top_level_without_lock,
        0
    );
    // A directory untouched for two days is not recent.
    let fixture = Fixture::new(Some("recent-without-lock"));
    let old = SystemTime::now() - Duration::from_secs(48 * 60 * 60);
    let brain = fixture.roots.data.join("brain");
    for entry in std::fs::read_dir(&brain).unwrap() {
        std::fs::File::open(entry.unwrap().path())
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    assert_eq!(health(&fixture).recent_top_level_without_lock, 0);
    assert_eq!(health(&fixture).brain_entries, 2);
}

#[test]
fn an_over_cap_brain_is_unbounded_and_limited() {
    let fixture = Fixture::new(Some("over-cap-brain"));
    fixture.setup();
    let projection = fixture.projection();
    assert_eq!(projection.hooks["witness"]["bounded"], json!(false));
    assert_eq!(
        projection.hooks["witness"]["brain_entries"],
        json!(BRAIN_ENTRY_CAP + 1)
    );
    assert_eq!(
        projection.limitations,
        ["subagent check bounded: brain/ has 65537 entries"]
    );
}

#[test]
fn the_scan_reads_at_most_64_directories_and_never_file_contents() {
    let fixture = Fixture::new(None);
    let brain = fixture.roots.data.join("brain");
    for i in 0..100 {
        let transcript = brain.join(format!("c{i}/.system_generated/logs/transcript_full.jsonl"));
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        std::fs::write(&transcript, "secret").unwrap();
        // Unreadable content: a scan that opened a file would fail on it.
        std::fs::set_permissions(
            &transcript,
            std::os::unix::fs::PermissionsExt::from_mode(0o000),
        )
        .unwrap();
    }
    let seen = health(&fixture);
    assert_eq!(seen.brain_entries, 100);
    assert_eq!(seen.recent_top_level_without_lock, 64);
    assert!(seen.bounded);
    assert_eq!(seen.error, None);
}

#[test]
fn an_expired_deadline_stops_the_scan_unbounded() {
    let fixture = Fixture::new(Some("recent-without-lock"));
    let seen = witness_health(&fixture.roots, SystemTime::now(), &|| true);
    assert!(!seen.bounded);
    assert_eq!(seen.brain_entries, 0);
}

#[test]
fn without_any_trace_of_agy_nothing_degrades() {
    let fixture = Fixture::new(None);
    let projection = fixture.projection();
    assert!(projection.limitations.is_empty());
    assert!(projection.safe_repairs.is_empty());
    assert!(projection.manual_repairs.is_empty());
    assert_eq!(projection.hooks["witness"]["brain_entries"], json!(0));
}

fn golden_hooks() -> Value {
    json!({
        "plugin": {"path": "/h/.gemini/config/plugins/herdr-threads", "installed": true, "current": true, "enabled": true, "version": "0.9.0"},
        "hooks_file": "/h/.gemini/config/plugins/herdr-threads/hooks.json",
        "allow_entry": {"file": "/h/.gemini/antigravity-cli/settings.json", "rule": "command(herdr-threads)", "state": "pre_existing"},
        "harness_version": {"admission": "contract_declared", "binary": "/bin/agy", "version": null},
        "witness": {"presence_dir": true, "brain_entries": 3, "bounded": false, "recent_top_level_without_lock": 2},
        "sandbox": SANDBOX_NOTE,
    })
}

#[test]
fn text_without_a_projection_is_empty() {
    for form in [DoctorTextForm::Debug, DoctorTextForm::Summary] {
        assert_eq!(doctor_text(&Value::Null, form), "");
    }
}

#[test]
fn debug_text_golden() {
    assert_eq!(
        doctor_text(&golden_hooks(), DoctorTextForm::Debug),
        "hooks.agy.plugin: /h/.gemini/config/plugins/herdr-threads installed=yes current=yes enabled=yes version=0.9.0\n\
         hooks.agy.hooks_file: /h/.gemini/config/plugins/herdr-threads/hooks.json\n\
         hooks.agy.allow_entry: command(herdr-threads) pre-existing in /h/.gemini/antigravity-cli/settings.json\n\
         hooks.agy.installed: contract_declared (binary /bin/agy)\n\
         hooks.agy.witness: presence_dir=yes brain_entries=3 bounded=no recent_top_level_without_lock=2\n\
         hooks.agy.sandbox: agy --sandbox sessions may not reach the herdr-threads daemon socket (untested); launch refuses --sandbox\n"
    );
}

#[test]
fn summary_text_golden() {
    let mut hooks = golden_hooks();
    assert_eq!(
        doctor_text(&hooks, DoctorTextForm::Summary),
        "agy: plugin installed, allow entry pre-existing\n"
    );
    hooks["plugin"]["enabled"] = json!(false);
    assert_eq!(
        doctor_text(&hooks, DoctorTextForm::Summary),
        "agy: plugin disabled, allow entry pre-existing\n"
    );
    hooks["plugin"]["current"] = json!(false);
    assert_eq!(
        doctor_text(&hooks, DoctorTextForm::Summary),
        "agy: plugin stale, allow entry pre-existing\n"
    );
    hooks["plugin"]["installed"] = json!(false);
    hooks["allow_entry"]["state"] = json!("missing");
    assert_eq!(
        doctor_text(&hooks, DoctorTextForm::Summary),
        "agy: plugin missing, allow entry missing\n"
    );
}

/// The report the full doctor render reads, from one agy projection.
fn report_of(projection: &DoctorProjection) -> Value {
    json!({
        "result": "degraded",
        "context": {"ok": true},
        "state_dir": {"safe": true},
        "daemon": {"state": "healthy", "version_matches": true},
        "adapter_order": ["agy"],
        "hooks": {"agy": projection.hooks},
        "limitations": projection.limitations,
        "local_harnesses": {"agy": {
            "safe_repairs": if projection.safe_repairs.is_empty() { json!([]) } else { json!(["repair_owned"]) },
        }},
    })
}

#[test]
fn a_safe_repair_makes_the_full_summary_print_the_fix_line() {
    let fixture = Fixture::new(Some("top-level"));
    let broken = fixture.projection();
    let summary = crate::cli::doctor::render_text(&report_of(&broken));
    assert!(
        summary.contains("agy: plugin missing, allow entry missing\n"),
        "{summary}"
    );
    assert!(
        summary.contains("issue: agy plugin is not installed"),
        "{summary}"
    );
    assert!(
        summary.contains("fix: herdr-threads doctor fix\n"),
        "{summary}"
    );

    fixture.setup();
    let healthy = fixture.projection();
    let summary = crate::cli::doctor::render_text(&report_of(&healthy));
    assert!(
        summary.contains("agy: plugin installed, allow entry owned\n"),
        "{summary}"
    );
    assert!(!summary.contains("fix:"), "{summary}");
}

#[test]
fn a_disabled_plugin_gets_no_fix_line() {
    let fixture = Fixture::new(Some("top-level"));
    fixture.setup();
    std::fs::write(
        &fixture.roots.plugins_config,
        r#"{"plugins":{"herdr-threads":{"enabled":false}}}"#,
    )
    .unwrap();
    let summary = crate::cli::doctor::render_text(&report_of(&fixture.projection()));
    assert!(
        summary.contains("issue: agy plugin is disabled"),
        "{summary}"
    );
    assert!(!summary.contains("fix:"), "{summary}");
}
