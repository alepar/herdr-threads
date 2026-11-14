//! opencode setup, status, unsetup and doctor (design D6), in a temp HOME, config root and state
//! dir only. Included from `src/harness/opencode/setup.rs`. The embedded plugin is opaque: tests
//! compare installed bytes with what the renderer makes of the embedded source, and drive the
//! launch-line and upgrade paths with synthetic sources.
use super::*;
use crate::harness::{
    adapter::{
        DoctorTextForm, HarnessAdapter, ResolvedSetupScope, SetupEnvironment, SetupFailure,
        SetupRequest, SetupScopeRequest, SetupStatus, StatusRequest, UnsetupRequest,
    },
    opencode::{OpencodeAdapter, doctor},
};
use crate::protocol::{
    results::ErrorCode,
    time::{CallBudget, MonoInstant},
};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Sources with both placeholder lines, standing in for the plugin (the embedded file is opaque
/// here; a source without them is covered too).
const MARKED_V1: &str = "// v1\nconst LAUNCH = null // herdr-threads:launch (setup writes it)\nconst GRANT_PERMISSIONS = false // herdr-threads:permissions (setup writes it)\nexport default {}\n";
const MARKED_V2: &str = "// v2\nconst LAUNCH = null // herdr-threads:launch (setup writes it)\nconst GRANT_PERMISSIONS = false // herdr-threads:permissions (setup writes it)\nexport default { v: 2 }\n";

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    exe: PathBuf,
    config: PathBuf,
    declared: Vec<(&'static str, PathBuf)>,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_support::scratch::temp_path(format!(
            "ht-opencode-setup-{}",
            uuid::Uuid::new_v4()
        ));
        let home = root.join("home");
        let state = root.join("state");
        std::fs::create_dir_all(home.join("proj/.git")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        Self {
            config: home.join(".config/opencode"),
            exe: root.join("bin/herdr-threads"),
            root,
            home,
            state,
            declared: Vec::new(),
        }
    }
    fn env(&self) -> SetupEnvironment {
        SetupEnvironment {
            home: Some(self.home.clone().into()),
            cwd: self.home.join("proj"),
            executable: self.exe.clone(),
            state_dir: Some(self.state.clone()),
            host_endpoint: Some(self.root.join("host.sock")),
            declared: self
                .declared
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone().into_os_string()))
                .collect(),
            ..Default::default()
        }
    }
    fn scope(&self) -> ResolvedSetupScope {
        ResolvedSetupScope::ConfigRoot(self.config.clone())
    }
    fn request(&self, policy: PermissionPolicy) -> SetupRequest {
        SetupRequest {
            scope: self.scope(),
            executable: self.exe.clone(),
            environment: self.env(),
            native_binary: None,
            options: policy
                .option()
                .map(|name| (name.to_owned(), true))
                .into_iter()
                .collect(),
        }
    }
    fn setup(&self) -> Result<crate::harness::adapter::SetupOutcome, SetupFailure> {
        OpencodeAdapter.setup(&self.request(PermissionPolicy::Ask), &budget())
    }
    fn setup_src(
        &self,
        policy: PermissionPolicy,
        source: &str,
    ) -> Result<crate::harness::adapter::SetupOutcome, SetupFailure> {
        setup_with(&self.request(policy), source)
    }
    fn status_request(&self) -> StatusRequest {
        StatusRequest {
            scope: self.scope(),
            environment: self.env(),
            native_binary: None,
        }
    }
    fn status_src(&self, source: &str) -> crate::harness::adapter::LocalSetupStatus {
        match status_with(&self.status_request(), source) {
            SetupStatus::Detailed(status) => *status,
            _ => panic!("opencode status is detailed"),
        }
    }
    fn unsetup(&self) -> Result<crate::harness::adapter::RemovalOutcome, SetupFailure> {
        OpencodeAdapter.unsetup(
            &UnsetupRequest {
                scope: self.scope(),
                environment: self.env(),
            },
            &budget(),
        )
    }
    fn plugin(&self) -> PathBuf {
        self.config.join(PLUGIN_PATH)
    }
    fn skill(&self) -> PathBuf {
        self.config.join(SKILL_PATH)
    }
    fn write(&self, path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn launch(&self) -> Launch {
        launch(&self.env()).unwrap()
    }
    fn manifest(&self) -> PathBuf {
        manifest_file(&self.config, &self.env()).unwrap()
    }
    fn files_under(&self, dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(self.files_under(&path));
            } else {
                out.push(path);
            }
        }
        out.sort();
        out
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Default::default(),
    }
}
fn is_conflict(result: Result<impl Sized, SetupFailure>) -> bool {
    matches!(result, Err(SetupFailure::Api(error)) if error.code == ErrorCode::Conflict)
}
fn states(status: &crate::harness::adapter::LocalSetupStatus) -> (String, String) {
    (
        status.projection["plugin"]["state"]
            .as_str()
            .unwrap()
            .to_owned(),
        status.projection["skill"]["state"]
            .as_str()
            .unwrap()
            .to_owned(),
    )
}
fn pair(plugin: &str, skill: &str) -> (String, String) {
    (plugin.to_owned(), skill.to_owned())
}

// ---------------------------------------------------------------- render

#[test]
fn render_replaces_only_the_marker_lines_and_skips_an_absent_one() {
    let launch = Launch {
        argv: vec!["/x/ht".into(), "--state-dir".into(), "/s".into()],
    };
    let launch_line = "const LAUNCH = {\"argv\":[\"/x/ht\",\"--state-dir\",\"/s\"]} // herdr-threads:launch (written by setup opencode)";
    assert_eq!(
        render_plugin(MARKED_V1, &launch, true),
        format!("// v1\n{launch_line}\n{GRANT_GRANTED}\nexport default {{}}\n")
    );
    assert_eq!(
        render_plugin(MARKED_V1, &launch, false),
        format!(
            "// v1\n{launch_line}\nconst GRANT_PERMISSIONS = false // herdr-threads:permissions (setup writes it)\nexport default {{}}\n"
        )
    );
    assert!(installed_grant(
        render_plugin(MARKED_V1, &launch, true).as_bytes()
    ));
    assert!(!installed_grant(
        render_plugin(MARKED_V1, &launch, false).as_bytes()
    ));
    assert_eq!(
        installed_launch(render_plugin(MARKED_V1, &launch, false).as_bytes()),
        Some(launch.clone())
    );
    let plain = "// no marker\nexport default {}\n";
    assert_eq!(render_plugin(plain, &launch, true), plain);
    assert_eq!(installed_launch(plain.as_bytes()), None);
}

/// The embedded plugin carries each placeholder at most once (exactly once once the plugin
/// lands; the earlier placeholder file carries none).
#[test]
fn the_embedded_source_has_at_most_one_of_each_marker() {
    for marker in [LAUNCH_MARKER, GRANT_MARKER] {
        assert!(
            PLUGIN_SOURCE
                .lines()
                .filter(|line| line.starts_with(marker))
                .count()
                <= 1
        );
    }
}

#[test]
fn the_launch_argv_is_the_validated_hook_prefix() {
    let fx = Fixture::new();
    let launch = fx.launch();
    assert_eq!(
        launch.argv,
        vec![
            fx.exe.to_str().unwrap().to_owned(),
            "--state-dir".into(),
            fx.state.to_str().unwrap().to_owned(),
            "--host-endpoint".into(),
            fx.root.join("host.sock").to_str().unwrap().to_owned(),
        ]
    );
    assert_eq!(
        heartbeat_path(&fx.state),
        fx.state.join("opencode/plugin-heartbeat.json")
    );
}

#[test]
fn the_skill_is_the_shared_skill_plus_the_opencode_section() {
    let skill = skill_md();
    assert!(skill.starts_with(crate::cli::skill::SKILL_MD));
    assert!(skill.ends_with(&format!("## opencode\n{OPENCODE_SKILL_GUIDANCE}")));
    assert!(OPENCODE_SKILL_GUIDANCE.contains("HERDR_THREADS_OPENCODE_SESSION"));
}

// ---------------------------------------------------------------- install

#[test]
fn fresh_install_writes_exactly_the_two_owned_files() {
    let fx = Fixture::new();
    fx.write(&fx.config.join("opencode.json"), "{\"permission\":{}}");
    fx.write(&fx.config.join("plugins/other.js"), "other");
    let outcome = fx.setup().unwrap();
    assert_eq!(outcome.projection["action"], "installed");
    assert_eq!(
        std::fs::read(fx.plugin()).unwrap(),
        render_plugin(PLUGIN_SOURCE, &fx.launch(), false).into_bytes()
    );
    assert_eq!(std::fs::read_to_string(fx.skill()).unwrap(), skill_md());
    assert_eq!(
        std::fs::read_to_string(fx.config.join("opencode.json")).unwrap(),
        "{\"permission\":{}}"
    );
    assert_eq!(
        fx.files_under(&fx.home),
        vec![
            fx.config.join("opencode.json"),
            fx.config.join("plugins/herdr-threads.js"),
            fx.config.join("plugins/other.js"),
            fx.config.join("skills/herdr-threads/SKILL.md"),
        ]
    );
    let status = fx.status_src(PLUGIN_SOURCE);
    assert!(status.installed);
    assert_eq!(states(&status), pair("installed", "installed"));
}

#[test]
fn a_re_run_is_unchanged() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let before = std::fs::metadata(fx.plugin()).unwrap().modified().unwrap();
    let again = fx.setup().unwrap();
    assert_eq!(again.projection["action"], "already_installed");
    assert!(
        again.projection["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["action"] == "unchanged")
    );
    assert_eq!(
        std::fs::metadata(fx.plugin()).unwrap().modified().unwrap(),
        before
    );
}

#[test]
fn an_owned_older_file_is_upgraded() {
    let fx = Fixture::new();
    fx.setup_src(PermissionPolicy::Ask, MARKED_V1).unwrap();
    assert_eq!(
        states(&fx.status_src(MARKED_V2)),
        pair("stale", "installed")
    );
    let outcome = fx.setup_src(PermissionPolicy::Ask, MARKED_V2).unwrap();
    assert_eq!(outcome.projection["action"], "installed");
    assert_eq!(outcome.projection["files"][0]["action"], "updated");
    assert_eq!(
        std::fs::read_to_string(fx.plugin()).unwrap(),
        render_plugin(MARKED_V2, &fx.launch(), false)
    );
    assert_eq!(
        states(&fx.status_src(MARKED_V2)),
        pair("installed", "installed")
    );
}

#[test]
fn a_moved_executable_rerenders_the_launch_line() {
    let mut fx = Fixture::new();
    fx.setup_src(PermissionPolicy::Ask, MARKED_V1).unwrap();
    fx.exe = fx.root.join("other/herdr-threads");
    assert_eq!(states(&fx.status_src(MARKED_V1)).0, "stale");
    fx.setup_src(PermissionPolicy::Ask, MARKED_V1).unwrap();
    assert_eq!(
        installed_launch(&std::fs::read(fx.plugin()).unwrap()).unwrap(),
        fx.launch()
    );
}

#[test]
fn an_unowned_file_at_an_owned_path_is_a_conflict_and_nothing_changes() {
    for rel in [PLUGIN_PATH, SKILL_PATH] {
        let fx = Fixture::new();
        let path = fx.config.join(rel);
        fx.write(&path, "mine");
        assert_eq!(
            states(&fx.status_src(PLUGIN_SOURCE)),
            if rel == PLUGIN_PATH {
                pair("unowned", "not_installed")
            } else {
                pair("not_installed", "unowned")
            }
        );
        assert!(is_conflict(fx.setup()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
        assert_eq!(fx.files_under(&fx.home), vec![path]);
        assert!(!fx.manifest().exists());
    }
}

#[test]
fn an_edited_owned_file_is_a_conflict_and_stays() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    fx.write(&fx.skill(), "edited");
    assert_eq!(
        states(&fx.status_src(PLUGIN_SOURCE)),
        pair("installed", "modified")
    );
    assert!(is_conflict(fx.setup()));
    assert_eq!(std::fs::read_to_string(fx.skill()).unwrap(), "edited");
}

#[test]
fn a_symlink_at_an_owned_path_is_a_conflict() {
    let fx = Fixture::new();
    fx.write(&fx.root.join("target.js"), "x");
    std::fs::create_dir_all(fx.config.join("plugins")).unwrap();
    std::os::unix::fs::symlink(fx.root.join("target.js"), fx.plugin()).unwrap();
    assert!(is_conflict(fx.setup()));
    assert_eq!(
        std::fs::read_to_string(fx.root.join("target.js")).unwrap(),
        "x"
    );
}

// ---------------------------------------------------------------- unsetup

#[test]
fn unsetup_removes_only_owned_files_and_created_directories() {
    let fx = Fixture::new();
    fx.write(&fx.config.join("opencode.json"), "{}");
    fx.write(&fx.config.join("plugins/other.js"), "other");
    fx.setup().unwrap();
    let removed = fx.unsetup().unwrap();
    assert_eq!(removed.projection["action"], "removed");
    assert!(removed.residue.is_empty());
    assert_eq!(
        fx.files_under(&fx.home),
        vec![
            fx.config.join("opencode.json"),
            fx.config.join("plugins/other.js")
        ]
    );
    // `plugins/` predates setup; `skills/` was created by it.
    assert!(fx.config.join("plugins").is_dir());
    assert!(!fx.config.join("skills").exists());
    assert!(!fx.manifest().exists());
    assert_eq!(fx.unsetup().unwrap().projection["action"], "not_installed");
}

#[test]
fn unsetup_keeps_an_edited_file_as_residue() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    fx.write(&fx.plugin(), "edited");
    let removed = fx.unsetup().unwrap();
    assert_eq!(removed.residue, vec![fx.plugin()]);
    assert_eq!(std::fs::read_to_string(fx.plugin()).unwrap(), "edited");
    assert!(!fx.skill().exists());
    assert!(fx.manifest().exists());
}

#[test]
fn unsetup_never_removes_an_unowned_file() {
    let fx = Fixture::new();
    fx.write(&fx.plugin(), "mine");
    assert_eq!(fx.unsetup().unwrap().projection["action"], "not_installed");
    assert_eq!(std::fs::read_to_string(fx.plugin()).unwrap(), "mine");
}

// ---------------------------------------------------------------- config root

#[test]
fn config_root_precedence() {
    let mut fx = Fixture::new();
    let resolve = |fx: &Fixture, explicit: Option<&Path>| {
        let mut env = fx.env();
        if let Some(root) = explicit {
            env.config_roots
                .insert("opencode".into(), root.to_path_buf());
        }
        match OpencodeAdapter.resolve_setup_scope(&SetupScopeRequest::Default, &env) {
            Ok(ResolvedSetupScope::ConfigRoot(root)) => root,
            other => panic!("{other:?}"),
        }
    };
    // ~/.config/opencode
    assert_eq!(resolve(&fx, None), fx.home.join(".config/opencode"));
    // $XDG_CONFIG_HOME/opencode (a relative or empty value is ignored)
    fx.declared = vec![("XDG_CONFIG_HOME", PathBuf::from("relative"))];
    assert_eq!(resolve(&fx, None), fx.home.join(".config/opencode"));
    fx.declared = vec![("XDG_CONFIG_HOME", fx.root.join("xdg"))];
    assert_eq!(resolve(&fx, None), fx.root.join("xdg/opencode"));
    // OPENCODE_CONFIG_DIR
    fx.declared
        .push(("OPENCODE_CONFIG_DIR", fx.root.join("occ")));
    assert_eq!(resolve(&fx, None), fx.root.join("occ"));
    // --config-root
    assert_eq!(
        resolve(&fx, Some(&fx.root.join("explicit"))),
        fx.root.join("explicit")
    );
    // No HOME and nothing declared: unavailable.
    let env = SetupEnvironment::default();
    assert!(
        OpencodeAdapter
            .resolve_setup_scope(&SetupScopeRequest::Default, &env)
            .is_err()
    );
}

#[test]
fn setup_installs_under_an_explicit_root_only() {
    let mut fx = Fixture::new();
    fx.config = fx.root.join("explicit");
    fx.setup().unwrap();
    assert!(fx.root.join("explicit").join(PLUGIN_PATH).is_file());
    assert!(fx.files_under(&fx.home).is_empty());
}

// ---------------------------------------------------------------- consent

fn granted(fx: &Fixture) -> bool {
    installed_grant(&std::fs::read(fx.plugin()).unwrap())
}

#[test]
fn consent_on_and_off_render_and_record_the_variant() {
    let fx = Fixture::new();
    let grant = fx.setup_src(PermissionPolicy::Grant, MARKED_V1).unwrap();
    assert_eq!(grant.projection["permissions"]["state"], "granted");
    assert_eq!(grant.projection["permissions"]["action"], "granted");
    assert!(granted(&fx));
    // The recorded variant is current, not modified.
    let status = fx.status_src(MARKED_V1);
    assert!(status.installed);
    assert_eq!(status.projection["permissions"]["state"], "granted");
    // Ask keeps the recorded grant.
    let kept = fx.setup_src(PermissionPolicy::Ask, MARKED_V1).unwrap();
    assert_eq!(kept.projection["permissions"]["action"], "kept");
    assert_eq!(kept.projection["action"], "already_installed");
    assert!(granted(&fx));
    // Decline re-renders without the grant.
    let declined = fx.setup_src(PermissionPolicy::Decline, MARKED_V1).unwrap();
    assert_eq!(declined.projection["permissions"]["action"], "removed");
    assert_eq!(declined.projection["permissions"]["state"], "not_granted");
    assert!(!granted(&fx));
    assert!(fx.status_src(MARKED_V1).installed);
    assert_eq!(recorded_permissions(&fx.config, &fx.env()), Some(false));
}

#[test]
fn consent_with_a_marker_free_source_is_recorded_without_changing_bytes() {
    let fx = Fixture::new();
    let plain = "export default {}\n";
    fx.setup_src(PermissionPolicy::Decline, plain).unwrap();
    let outcome = fx.setup_src(PermissionPolicy::Grant, plain).unwrap();
    assert_eq!(outcome.projection["action"], "installed");
    assert_eq!(std::fs::read_to_string(fx.plugin()).unwrap(), plain);
    assert_eq!(recorded_permissions(&fx.config, &fx.env()), Some(true));
}

#[test]
fn ask_without_a_record_advises_and_the_terminal_answer_settles_it() {
    for (answer, grant) in [("\n", true), ("n\n", false)] {
        let fx = Fixture::new();
        let outcome = fx.setup_src(PermissionPolicy::Ask, MARKED_V1).unwrap();
        let mut report = outcome.projection;
        assert_eq!(report["permissions"]["action"], "advised");
        assert_eq!(report["permissions"]["note"], OPENCODE_PERMISSION_ADVICE);
        assert!(!granted(&fx));
        let mut out = Vec::new();
        settle_permissions_with(
            &fx.scope(),
            &fx.env(),
            &mut report,
            &mut std::io::Cursor::new(answer),
            &mut out,
            MARKED_V1,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            OPENCODE_PERMISSION_QUESTION
        );
        assert_eq!(granted(&fx), grant);
        assert_eq!(
            report["permissions"]["action"],
            if grant { "granted" } else { "declined" }
        );
        assert!(report["permissions"]["note"].is_null());
    }
}

// ---------------------------------------------------------------- status

#[test]
fn status_states() {
    let fx = Fixture::new();
    let status = fx.status_src(PLUGIN_SOURCE);
    assert!(!status.installed);
    assert_eq!(states(&status), pair("not_installed", "not_installed"));
    assert_eq!(status.projection["permissions"]["state"], "not_granted");
    assert!(status.repairs.is_empty());
    fx.setup().unwrap();
    std::fs::remove_file(fx.skill()).unwrap();
    let status = fx.status_src(PLUGIN_SOURCE);
    assert_eq!(states(&status), pair("installed", "missing"));
    assert!(!status.installed);
    assert!(matches!(
        status.repairs.as_slice(),
        [LocalRepair::RepairOwned]
    ));
    fx.setup().unwrap();
    fx.write(&fx.plugin(), "edited");
    let status = fx.status_src(PLUGIN_SOURCE);
    assert_eq!(states(&status), pair("modified", "installed"));
    assert!(status.repairs.is_empty());
    assert!(
        status
            .diagnostics
            .iter()
            .any(|d| d.code == "opencode_plugin_modified")
    );
}

#[test]
fn the_report_lines_name_both_files_and_the_consent() {
    let fx = Fixture::new();
    let outcome = fx.setup_src(PermissionPolicy::Grant, MARKED_V1).unwrap();
    assert_eq!(
        setup_report_lines(&outcome.projection),
        vec![
            format!("plugin {}: installed", fx.plugin().display()),
            format!("skill {}: installed", fx.skill().display()),
            "permissions: granted (granted)".to_owned(),
        ]
    );
}

// ---------------------------------------------------------------- doctor

fn doctor_hooks(fx: &Fixture) -> (Value, Vec<String>) {
    let projection =
        doctor::doctor_projection(&fx.status_request(), &Value::Null, &budget()).unwrap();
    (projection.hooks, projection.limitations)
}
fn heartbeat(fx: &Fixture, body: Value) {
    let path = heartbeat_path(&fx.state);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&body).unwrap()).unwrap();
}
fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
        + 60_000
}

#[test]
fn doctor_without_opencode_or_its_root_reports_no_limitation() {
    let fx = Fixture::new();
    let (hooks, limitations) = doctor_hooks(&fx);
    assert!(limitations.is_empty(), "{limitations:?}");
    assert_eq!(hooks["heartbeat"]["state"], "absent");
    assert_eq!(
        doctor::doctor_text(&hooks, DoctorTextForm::Summary),
        "opencode: plugin not-installed, skill not-installed, permissions not-granted, heartbeat absent\n"
    );
}

#[test]
fn doctor_reports_an_installed_plugin_that_never_loaded() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let (hooks, limitations) = doctor_hooks(&fx);
    assert_eq!(hooks["heartbeat"]["state"], "absent");
    assert!(
        limitations
            .iter()
            .any(|line| line.starts_with("opencode plugin installed, never loaded")),
        "{limitations:?}"
    );
}

#[test]
fn doctor_reads_the_heartbeat() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let all = serde_json::json!({"append": true, "wake": true, "status": true, "session_get": true, "toast": true});
    heartbeat(
        &fx,
        serde_json::json!({"schema": 1, "plugin": "herdr-threads", "pid": 42, "role": "active",
            "directory": "/p", "launch": true, "permissions": "not_consented", "capabilities": all,
            "loaded_at": now_ms(), "at": now_ms()}),
    );
    let (hooks, limitations) = doctor_hooks(&fx);
    assert_eq!(hooks["heartbeat"]["state"], "loaded");
    assert_eq!(hooks["heartbeat"]["pid"], 42);
    assert!(limitations.is_empty(), "{limitations:?}");

    // Loaded before the installed file was written: not loaded since setup.
    heartbeat(&fx, serde_json::json!({"role": "active", "loaded_at": 0}));
    let (hooks, limitations) = doctor_hooks(&fx);
    assert_eq!(hooks["heartbeat"]["state"], "stale");
    assert!(
        limitations
            .iter()
            .any(|l| l.contains("not loaded since setup"))
    );

    // An inert second copy, a lost capability, a degraded one and a user deny.
    heartbeat(
        &fx,
        serde_json::json!({"role": "inert", "loaded_at": now_ms(),
            "capabilities": {"append": true, "wake": false}, "degraded": ["append_row"],
            "permission": {"granted": ["herdr-threads *"], "denied": ["ht *"]}, "future": 1}),
    );
    let (hooks, limitations) = doctor_hooks(&fx);
    assert_eq!(hooks["heartbeat"]["state"], "inert");
    for expected in [
        "opencode loaded a second herdr-threads plugin copy, which stays inert; remove the extra copy",
        "opencode plugin degraded: no wake (no idle wake (wake=no_wake))",
        "opencode plugin degraded: no append_row (appends are acked without the stored message id)",
        "opencode permission rule denies \"ht *\": the plugin adds no allow for it",
    ] {
        assert!(
            limitations.iter().any(|l| l == expected),
            "{expected}: {limitations:?}"
        );
    }

    // Unreadable.
    std::fs::write(heartbeat_path(&fx.state), b"not json").unwrap();
    assert_eq!(doctor_hooks(&fx).0["heartbeat"]["state"], "unreadable");
}

#[test]
fn doctor_flags_a_second_plugin_copy_and_notes_same_named_skills() {
    let mut fx = Fixture::new();
    fx.declared = vec![("OPENCODE_CONFIG_DIR", fx.root.join("occ"))];
    fx.config = fx.root.join("occ");
    fx.setup().unwrap();
    let xdg_copy = fx.home.join(".config/opencode/plugins/herdr-threads.js");
    let project_copy = fx.home.join("proj/.opencode/plugin/herdr-threads.ts");
    fx.write(&xdg_copy, "old");
    fx.write(&project_copy, "old");
    let agents_skill = fx.home.join(".agents/skills/herdr-threads/SKILL.md");
    let claude_skill = fx.home.join(".claude/skills/herdr-threads/SKILL.md");
    fx.write(&agents_skill, "x");
    fx.write(&claude_skill, "x");
    let (hooks, limitations) = doctor_hooks(&fx);
    assert_eq!(
        hooks["copies"],
        serde_json::json!([xdg_copy.to_str().unwrap(), project_copy.to_str().unwrap()])
    );
    assert!(
        limitations
            .iter()
            .any(|l| l.starts_with("second herdr-threads plugin copy installed")),
        "{limitations:?}"
    );
    assert_eq!(
        hooks["skill_copies"],
        serde_json::json!([
            claude_skill.to_str().unwrap(),
            agents_skill.to_str().unwrap()
        ])
    );
    // A same-named skill is a note, never a limitation.
    assert!(!limitations.iter().any(|l| l.contains("SKILL.md")));
    let summary = doctor::doctor_text(&hooks, DoctorTextForm::Summary);
    assert!(
        summary.contains(&format!(
            "opencode: same-named skill {} (opencode keeps one copy at random)",
            agents_skill.display()
        )),
        "{summary}"
    );
    let debug = doctor::doctor_text(&hooks, DoctorTextForm::Debug);
    assert!(debug.contains("hooks.opencode.copies: "), "{debug}");
}
