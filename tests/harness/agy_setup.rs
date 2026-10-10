//! agy owned plugin directory, enablement and allow entry (L3), in a temp HOME and state dir only.
//! Included from `src/harness/agy/setup.rs`; goldens are `tests/fixtures/agy/plugin/*` with the
//! placeholders `<EXE>`, `<STATE>`, `<HOST>`, `<VERSION>` and (SKILL.md only) `<SKILL_MD>`.
use super::*;
use crate::cli::setup::PermissionPolicy;
use crate::harness::{
    adapter::{
        HarnessAdapter, ResolvedSetupScope, SetupEnvironment, SetupFailure, SetupRequest,
        SetupStatus, StatusRequest, UnsetupRequest,
    },
    agy::{AgyAdapter, plugin},
};
use crate::protocol::{
    results::ErrorCode,
    time::{CallBudget, MonoInstant},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    exe: PathBuf,
    roots: AgyRoots,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ht-agy-setup-{}", uuid::Uuid::new_v4()));
        let home = root.join("home");
        let state = root.join("state");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let roots = AgyRoots::from_home(&home);
        Self {
            exe: root.join("bin/herdr-threads"),
            root,
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
            host_endpoint: Some(self.root.join("host.sock")),
            ..Default::default()
        }
    }
    fn scope(&self) -> ResolvedSetupScope {
        ResolvedSetupScope::ConfigRoot(self.roots.config_root.clone())
    }
    fn setup_request(&self) -> SetupRequest {
        self.setup_request_with(crate::cli::setup::PermissionPolicy::Grant)
    }
    fn setup_request_with(&self, policy: crate::cli::setup::PermissionPolicy) -> SetupRequest {
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
    fn setup_with(
        &self,
        policy: crate::cli::setup::PermissionPolicy,
    ) -> Result<crate::harness::adapter::SetupOutcome, SetupFailure> {
        AgyAdapter.setup(&self.setup_request_with(policy), &budget())
    }
    fn setup(&self) -> Result<crate::harness::adapter::SetupOutcome, SetupFailure> {
        self.setup_with(crate::cli::setup::PermissionPolicy::Grant)
    }
    fn status_request(&self) -> StatusRequest {
        StatusRequest {
            scope: self.scope(),
            environment: self.env(),
            native_binary: None,
        }
    }
    fn status(&self) -> crate::harness::adapter::LocalSetupStatus {
        match AgyAdapter.status(&self.status_request(), &budget()) {
            SetupStatus::Detailed(status) => *status,
            _ => panic!("agy status is detailed"),
        }
    }
    fn unsetup(&self) -> Result<crate::harness::adapter::RemovalOutcome, SetupFailure> {
        AgyAdapter.unsetup(
            &UnsetupRequest {
                scope: self.scope(),
                environment: self.env(),
            },
            &budget(),
        )
    }
    fn plugin_file(&self, rel: &str) -> PathBuf {
        self.roots.plugin_dir.join(rel)
    }
    fn write(&self, path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    /// The golden after placeholder substitution for this fixture's environment.
    fn golden(&self, name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/agy/plugin")
            .join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        text.replace("<SKILL_MD>", crate::cli::skill::SKILL_MD)
            .replace("<EXE>", self.exe.to_str().unwrap())
            .replace("<STATE>", self.state.to_str().unwrap())
            .replace("<HOST>", self.root.join("host.sock").to_str().unwrap())
            .replace("<VERSION>", env!("CARGO_PKG_VERSION"))
            .into_bytes()
    }
    fn settings_json(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.roots.settings).unwrap()).unwrap()
    }
    fn files_under(&self, dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
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
fn diagnostic_codes(diagnostics: &[crate::harness::adapter::SetupDiagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code.as_str()).collect()
}
fn is_conflict(result: Result<impl Sized, SetupFailure>) -> bool {
    matches!(result, Err(SetupFailure::Api(error)) if error.code == ErrorCode::Conflict)
}
const RULE: &str = "command(herdr-threads)";

#[test]
fn install_writes_the_three_files_byte_equal_to_the_goldens() {
    let fx = Fixture::new();
    let outcome = fx.setup().unwrap();
    for (rel, golden) in [
        ("plugin.json", "plugin.json"),
        ("hooks.json", "hooks.json"),
        ("skills/herdr-threads/SKILL.md", "SKILL.md"),
    ] {
        let written = std::fs::read(fx.plugin_file(rel)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&written),
            String::from_utf8_lossy(&fx.golden(golden)),
            "{rel}"
        );
    }
    assert_eq!(outcome.projection["action"], "installed");
    assert_eq!(outcome.projection["installed"], true);
    assert_eq!(
        outcome.actions,
        vec![crate::harness::adapter::SetupAction::InstalledOwned]
    );
    // The skill is the core skill plus the agy section, with no cap on the plugin copy.
    let skill =
        String::from_utf8(std::fs::read(fx.plugin_file("skills/herdr-threads/SKILL.md")).unwrap())
            .unwrap();
    assert!(skill.starts_with(crate::cli::skill::SKILL_MD));
    assert!(skill.contains("\n## Antigravity CLI (agy)\n"));
    // hooks.json is the D7 shape: flat handlers, seconds, 10/10/5.
    let hooks: Value =
        serde_json::from_slice(&std::fs::read(fx.plugin_file("hooks.json")).unwrap()).unwrap();
    assert_eq!(hooks["herdr-threads"]["SessionStart"][0]["timeout"], 10);
    assert_eq!(hooks["herdr-threads"]["PreInvocation"][0]["timeout"], 10);
    assert_eq!(hooks["herdr-threads"]["Stop"][0]["timeout"], 5);
}

#[test]
fn setup_writes_only_under_the_temp_home_and_state() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    fx.unsetup().unwrap();
    fx.setup().unwrap();
    for path in fx.files_under(&fx.root) {
        assert!(
            path.starts_with(&fx.home) || path.starts_with(&fx.state),
            "{} escaped the temp roots",
            path.display()
        );
    }
    let written: Vec<String> = fx
        .files_under(&fx.root)
        .iter()
        .map(|p| p.strip_prefix(&fx.root).unwrap().display().to_string())
        .collect();
    assert!(
        written
            .iter()
            .any(|p| p.ends_with("config/plugins/herdr-threads/hooks.json")),
        "{written:?}"
    );
    assert!(
        written
            .iter()
            .any(|p| p.ends_with("antigravity-cli/settings.json")),
        "{written:?}"
    );
    assert!(fx.roots.config_root.starts_with(&fx.home));
}

#[test]
fn re_setup_is_unchanged() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let before: Vec<_> = fx
        .files_under(&fx.root)
        .into_iter()
        .map(|p| (std::fs::read(&p).unwrap(), p))
        .collect();
    let outcome = fx.setup().unwrap();
    assert_eq!(
        outcome.actions,
        vec![crate::harness::adapter::SetupAction::Unchanged]
    );
    assert_eq!(outcome.projection["action"], "already_installed");
    for file in outcome.projection["files"].as_array().unwrap() {
        assert_eq!(file["action"], "unchanged", "{file}");
    }
    let after: Vec<_> = fx
        .files_under(&fx.root)
        .into_iter()
        .map(|p| (std::fs::read(&p).unwrap(), p))
        .collect();
    assert_eq!(before, after, "an unchanged setup rewrote nothing");
}

#[test]
fn a_moved_executable_rerenders_the_hooks() {
    let mut fx = Fixture::new();
    fx.setup().unwrap();
    let old_hooks = std::fs::read(fx.plugin_file("hooks.json")).unwrap();
    fx.exe = fx.root.join("elsewhere/herdr-threads");
    // Before re-setup the plugin is installed but stale.
    let stale = fx.status();
    assert!(!stale.installed, "installed means installed and current");
    assert_eq!(stale.projection["plugin"]["installed"], true);
    assert_eq!(stale.projection["plugin"]["current"], false);
    assert!(diagnostic_codes(&stale.diagnostics).contains(&"agy_plugin_stale"));
    assert!(matches!(
        stale.repairs.as_slice(),
        [crate::harness::adapter::LocalRepair::RepairOwned]
    ));
    let outcome = fx.setup().unwrap();
    let actions: Vec<(String, String)> = outcome.projection["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().into(),
                f["action"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert!(
        actions.contains(&("hooks.json".into(), "updated".into())),
        "{actions:?}"
    );
    assert!(
        actions.contains(&("plugin.json".into(), "unchanged".into())),
        "{actions:?}"
    );
    let new_hooks = std::fs::read(fx.plugin_file("hooks.json")).unwrap();
    assert_ne!(old_hooks, new_hooks);
    assert_eq!(new_hooks, fx.golden("hooks.json"));
    let fresh = fx.status();
    assert!(fresh.installed);
    assert!(fresh.diagnostics.is_empty(), "{:?}", fresh.diagnostics);
    assert!(fresh.repairs.is_empty());
    // Unsetup still recognizes the re-rendered file as its own.
    assert!(fx.unsetup().unwrap().residue.is_empty());
    assert!(!fx.roots.plugin_dir.exists());
}

#[test]
fn a_user_edited_file_is_a_conflict_and_stays_untouched() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let hooks = fx.plugin_file("hooks.json");
    std::fs::write(&hooks, "{\"edited\":true}\n").unwrap();
    assert!(is_conflict(fx.setup()));
    assert_eq!(std::fs::read(&hooks).unwrap(), b"{\"edited\":true}\n");
    // Nothing else was rewritten around the edit: the other files still match the goldens.
    assert_eq!(
        std::fs::read(fx.plugin_file("plugin.json")).unwrap(),
        fx.golden("plugin.json")
    );
    let status = fx.status();
    assert!(!status.installed);
    assert!(diagnostic_codes(&status.diagnostics).contains(&"agy_plugin_stale"));
}

#[test]
fn a_foreign_plugin_directory_without_our_manifest_is_a_conflict() {
    let fx = Fixture::new();
    let foreign = fx.plugin_file("plugin.json");
    fx.write(
        &foreign,
        "{\"name\":\"herdr-threads\",\"by\":\"someone else\"}",
    );
    assert!(is_conflict(fx.setup()));
    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        "{\"name\":\"herdr-threads\",\"by\":\"someone else\"}"
    );
    assert!(!fx.plugin_file("hooks.json").exists());
    assert!(
        !fx.roots.settings.exists(),
        "a conflict adds no allow entry"
    );
    // Unsetup owns nothing there and leaves it alone.
    let outcome = fx.unsetup().unwrap();
    assert_eq!(outcome.projection["action"], "not_installed");
    assert!(foreign.exists());
}

#[test]
fn a_file_that_is_a_symlink_is_a_conflict() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let target = fx.root.join("target.json");
    std::fs::write(&target, "x").unwrap();
    let hooks = fx.plugin_file("hooks.json");
    std::fs::remove_file(&hooks).unwrap();
    std::os::unix::fs::symlink(&target, &hooks).unwrap();
    assert!(is_conflict(fx.setup()));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "x");
}

#[test]
fn unsetup_removes_only_owned_files_and_created_directories() {
    let fx = Fixture::new();
    // agy's own files beside ours, which unsetup must never touch.
    fx.write(&fx.roots.plugins_config, "{\"plugins\":{}}");
    fx.write(
        &fx.roots.config.join("plugins/import_manifest.json"),
        "{\"imported\":1}",
    );
    fx.setup().unwrap();
    let outcome = fx.unsetup().unwrap();
    assert_eq!(outcome.projection["action"], "removed");
    assert!(outcome.residue.is_empty());
    assert_eq!(
        outcome.actions,
        vec![crate::harness::adapter::SetupAction::RemovedOwned]
    );
    assert!(
        !fx.roots.plugin_dir.exists(),
        "created directories are removed"
    );
    assert_eq!(
        std::fs::read_to_string(&fx.roots.plugins_config).unwrap(),
        "{\"plugins\":{}}"
    );
    assert_eq!(
        std::fs::read_to_string(fx.roots.config.join("plugins/import_manifest.json")).unwrap(),
        "{\"imported\":1}"
    );
    // The directories that already existed (agy's own) stay.
    assert!(fx.roots.config.join("plugins").is_dir());
    // A second unsetup finds nothing to do.
    assert_eq!(fx.unsetup().unwrap().projection["action"], "not_installed");
}

#[test]
fn unsetup_reports_residue_for_an_edited_file_and_keeps_it() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let skill = fx.plugin_file("skills/herdr-threads/SKILL.md");
    std::fs::write(&skill, "my own notes").unwrap();
    let outcome = fx.unsetup().unwrap();
    assert_eq!(outcome.residue, vec![skill.clone()]);
    assert_eq!(outcome.projection["incomplete"], true);
    assert_eq!(std::fs::read_to_string(&skill).unwrap(), "my own notes");
    assert!(!fx.plugin_file("plugin.json").exists());
    assert!(!fx.plugin_file("hooks.json").exists());
    assert!(
        diagnostic_codes(&outcome.diagnostics).contains(&"agy_plugin_residue"),
        "{:?}",
        outcome.diagnostics
    );
    // Once the user removes the residue, a later unsetup finishes the cleanup.
    std::fs::remove_file(&skill).unwrap();
    let again = fx.unsetup().unwrap();
    assert!(again.residue.is_empty());
    assert!(!fx.roots.plugin_dir.exists());
}

fn config_json(fx: &Fixture, text: &str) {
    fx.write(&fx.roots.plugins_config, text);
}

#[test]
fn enablement_reads_config_json() {
    let fx = Fixture::new();
    // Absent file: enabled.
    assert_eq!(fx.status().enabled, Some(true));
    // Absent key: enabled.
    config_json(&fx, "{\"plugins\":{\"other\":{\"enabled\":false}}}");
    assert_eq!(fx.status().enabled, Some(true));
    config_json(&fx, "{\"plugins\":{\"herdr-threads\":{\"enabled\":true}}}");
    assert_eq!(fx.status().enabled, Some(true));
    // Disabled: reported with the manual repair, never re-enabled by setup.
    let disabled_text = "{\"plugins\":{\"herdr-threads\":{\"enabled\":false}}}";
    config_json(&fx, disabled_text);
    fx.setup().unwrap();
    let status = fx.status();
    assert_eq!(status.enabled, Some(false));
    assert_eq!(status.projection["plugin"]["enabled"], false);
    let diagnostic = status
        .diagnostics
        .iter()
        .find(|d| d.code == "agy_plugin_disabled")
        .expect("a disabled plugin is a diagnostic");
    assert_eq!(
        diagnostic.severity,
        crate::harness::adapter::DiagnosticSeverity::Warning
    );
    let argv: Vec<String> = diagnostic
        .manual_argv()
        .unwrap()
        .iter()
        .map(|w| w.to_string_lossy().into_owned())
        .collect();
    assert_eq!(argv, ["agy", "plugin", "enable", "herdr-threads"]);
    assert_eq!(
        std::fs::read_to_string(&fx.roots.plugins_config).unwrap(),
        disabled_text
    );
    assert!(status.installed, "disabled is not uninstalled");
}

#[test]
fn an_unreadable_config_json_is_unknown_with_a_diagnostic() {
    let fx = Fixture::new();
    config_json(&fx, "{not json");
    let status = fx.status();
    assert_eq!(status.enabled, None);
    assert_eq!(status.projection["plugin"]["enabled"], Value::Null);
    assert!(diagnostic_codes(&status.diagnostics).contains(&"agy_plugin_config_unreadable"));
    // A symlink is refused, not followed.
    std::fs::remove_file(&fx.roots.plugins_config).unwrap();
    let target = fx.root.join("elsewhere.json");
    std::fs::write(
        &target,
        "{\"plugins\":{\"herdr-threads\":{\"enabled\":false}}}",
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, &fx.roots.plugins_config).unwrap();
    assert_eq!(fx.status().enabled, None);
    // Over 1 MiB is refused.
    std::fs::remove_file(&fx.roots.plugins_config).unwrap();
    config_json(&fx, &format!("{{\"pad\":\"{}\"}}", "x".repeat(1_048_577)));
    assert_eq!(fx.status().enabled, None);
}

#[test]
fn allow_entry_is_created_when_the_array_is_absent() {
    let fx = Fixture::new();
    assert_eq!(
        allow_entry_state(&fx.roots, &fx.env()),
        AllowEntryState::NotGranted
    );
    // A settings file with other content and no permissions at all.
    fx.write(&fx.roots.settings, "{\"theme\":\"dark\"}");
    let outcome = fx.setup().unwrap();
    assert_eq!(outcome.projection["allow_entry"]["state"], "owned");
    assert_eq!(
        fx.settings_json(),
        json!({"theme":"dark","permissions":{"allow":[RULE]}})
    );
    assert_eq!(
        allow_entry_state(&fx.roots, &fx.env()),
        AllowEntryState::Owned
    );
    // Removal deletes only the owned member and the containers setup created.
    fx.unsetup().unwrap();
    assert_eq!(fx.settings_json(), json!({"theme":"dark"}));
}

#[test]
fn allow_entry_creates_an_absent_settings_file_and_removes_it_again() {
    let fx = Fixture::new();
    assert!(!fx.roots.data.exists());
    {
        let _run = crate::harness::setup::BackupSession::start();
        fx.setup().unwrap();
    }
    assert_eq!(fx.settings_json(), json!({"permissions":{"allow":[RULE]}}));
    assert!(
        backups(&fx).is_empty(),
        "setup created the file: no pre-image"
    );
    fx.unsetup().unwrap();
    assert!(!fx.roots.settings.exists());
    // v0.6 backup model (human decision 2026-10-10, ht-poh.43): unsetup keeps setup's pre-images,
    // never pruned, so the directory setup created stays holding only them.
    let left = fx.files_under(&fx.roots.data);
    assert!(!left.is_empty(), "{left:?}");
    for path in &left {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            name.starts_with("settings.json.") && name.ends_with(".herdr-threads"),
            "only setup's `.herdr-threads` backups remain in antigravity-cli/: {left:?}"
        );
    }
    assert_eq!(left.len(), backups(&fx).len());
    assert_eq!(backups(&fx).len(), 1);
}

#[test]
fn a_pre_existing_allow_entry_is_recorded_and_left_by_unsetup() {
    let fx = Fixture::new();
    let original = "{\"permissions\":{\"allow\":[\"command(herdr-threads)\",\"command(ls)\"]}}";
    fx.write(&fx.roots.settings, original);
    let outcome = fx.setup().unwrap();
    assert_eq!(outcome.projection["allow_entry"]["state"], "pre_existing");
    assert_eq!(
        fx.status().projection["allow_entry"]["state"],
        "pre_existing"
    );
    assert_eq!(
        fx.settings_json()["permissions"]["allow"],
        json!([RULE, "command(ls)"]),
        "the user's identical member is not duplicated"
    );
    fx.unsetup().unwrap();
    assert_eq!(
        std::fs::read_to_string(&fx.roots.settings).unwrap(),
        original
    );
}

#[test]
fn an_allow_entry_agy_rewrote_away_is_missing_and_setup_re_adds_it() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    assert_eq!(fx.status().projection["allow_entry"]["state"], "owned");
    // agy rewrites its settings file without the member.
    fx.write(&fx.roots.settings, "{\"model\":\"x\"}");
    let status = fx.status();
    assert_eq!(status.projection["allow_entry"]["state"], "missing");
    assert!(diagnostic_codes(&status.diagnostics).contains(&"agy_allow_entry_missing"));
    assert!(matches!(
        status.repairs.as_slice(),
        [crate::harness::adapter::LocalRepair::RepairOwned]
    ));
    assert!(status.installed, "the plugin itself is still installed");
    let outcome = fx.setup().unwrap();
    assert_eq!(outcome.projection["allow_entry"]["state"], "owned");
    assert_eq!(outcome.projection["action"], "installed");
    assert_eq!(
        fx.settings_json(),
        json!({"model":"x","permissions":{"allow":[RULE]}})
    );
    let healthy = fx.status();
    assert!(healthy.diagnostics.is_empty(), "{:?}", healthy.diagnostics);
    assert!(healthy.repairs.is_empty());
}

#[test]
fn unsetup_removes_only_the_owned_member() {
    let fx = Fixture::new();
    fx.write(
        &fx.roots.settings,
        "{\"permissions\":{\"allow\":[\"command(ls)\"]}}",
    );
    fx.setup().unwrap();
    assert_eq!(
        fx.settings_json()["permissions"]["allow"],
        json!(["command(ls)", RULE])
    );
    let outcome = fx.unsetup().unwrap();
    assert_eq!(outcome.projection["allow_entry_removed"], true);
    assert_eq!(
        fx.settings_json()["permissions"]["allow"],
        json!(["command(ls)"])
    );
    assert_eq!(
        allow_entry_state(&fx.roots, &fx.env()),
        AllowEntryState::NotGranted
    );
}

#[test]
fn a_damaged_settings_file_refuses_setup_without_changing_it() {
    let fx = Fixture::new();
    fx.write(&fx.roots.settings, "[1,2,3]");
    assert!(fx.setup().is_err());
    assert_eq!(
        std::fs::read_to_string(&fx.roots.settings).unwrap(),
        "[1,2,3]"
    );
}

#[test]
fn setup_status_json_has_the_d7_shape() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    let request = fx.status_request();
    let mut projection = fx.status().projection;
    AgyAdapter.setup_status_extras(&request, &mut projection);
    assert_eq!(
        projection["plugin"],
        json!({
            "path": fx.roots.plugin_dir.display().to_string(),
            "installed": true,
            "current": true,
            "enabled": true,
            "version": env!("CARGO_PKG_VERSION"),
        })
    );
    assert_eq!(
        projection["allow_entry"],
        json!({
            "file": fx.roots.settings.display().to_string(),
            "rule": RULE,
            "state": "owned",
        })
    );
    assert_eq!(projection["installed"], true);
    // The bare commands' summary line reads the hooks file and the executable observation.
    assert_eq!(
        projection["hooks_file"],
        fx.plugin_file("hooks.json").display().to_string()
    );
    assert_eq!(projection["harness_version"]["admission"], "unavailable");
    let mut with_binary = fx.status_request();
    with_binary.native_binary = Some(PathBuf::from("/opt/agy/bin/agy"));
    let SetupStatus::Detailed(found) = AgyAdapter.status(&with_binary, &budget()) else {
        panic!("detailed");
    };
    assert_eq!(
        found.projection["harness_version"]["admission"],
        "contract_declared"
    );
    assert_eq!(
        found.projection["harness_version"]["binary"],
        "/opt/agy/bin/agy"
    );
    // The extras name the ownership records, which exist.
    for key in ["plugin", "allow_entry"] {
        let manifest = projection["manifests"][key].as_str().unwrap();
        assert!(
            Path::new(manifest).starts_with(fx.state.join("setup")),
            "{manifest}"
        );
        assert!(Path::new(manifest).exists(), "{manifest}");
    }
    let plugin_manifest = projection["manifests"]["plugin"].as_str().unwrap();
    assert!(
        Path::new(plugin_manifest)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("agy-plugin-")
    );
    assert_eq!(
        Path::new(plugin_manifest)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .len(),
        "agy-plugin-".len() + 32
    );
}

#[test]
fn local_status_projects_installed_enabled_and_the_configured_hook() {
    let fx = Fixture::new();
    let absent = fx.status();
    assert!(!absent.installed);
    assert!(absent.configured_hook.is_none());
    assert_eq!(absent.projection["plugin"]["installed"], false);
    assert_eq!(absent.projection["plugin"]["version"], Value::Null);
    fx.setup().unwrap();
    let status = fx.status();
    assert!(status.installed);
    assert_eq!(status.enabled, Some(true));
    let hook = status
        .configured_hook
        .expect("an exact installation has a hook");
    assert_eq!(hook.scope, "user");
    assert_eq!(
        hook.path,
        fx.plugin_file("hooks.json").display().to_string()
    );
    assert_eq!(
        hook.fingerprint,
        crate::harness::setup::fingerprint(&fx.golden("hooks.json"))
    );
    assert_eq!(
        status.fingerprint.as_deref(),
        Some(hook.fingerprint.as_str())
    );
    // plugin::status is the API later layers consume.
    let direct = plugin::status(&fx.roots, &fx.env());
    assert!(direct.installed && direct.current);
    assert_eq!(direct.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(direct.path, fx.roots.plugin_dir);
}

#[test]
fn a_missing_owned_file_is_a_stale_installation_that_setup_repairs() {
    let fx = Fixture::new();
    fx.setup().unwrap();
    std::fs::remove_file(fx.plugin_file("plugin.json")).unwrap();
    let status = fx.status();
    assert!(!status.installed);
    assert!(status.configured_hook.is_none());
    assert!(diagnostic_codes(&status.diagnostics).contains(&"agy_plugin_stale"));
    let outcome = fx.setup().unwrap();
    assert_eq!(outcome.projection["action"], "installed");
    assert!(fx.status().installed);
    assert_eq!(
        std::fs::read(fx.plugin_file("plugin.json")).unwrap(),
        fx.golden("plugin.json")
    );
}

#[test]
fn report_lines_name_the_plugin_and_the_allow_entry() {
    let fx = Fixture::new();
    let outcome = fx.setup().unwrap();
    assert_eq!(
        AgyAdapter.setup_report_lines(&outcome.projection),
        vec![
            format!("plugin {}", fx.roots.plugin_dir.display()),
            "allow entry command(herdr-threads): owned".to_owned(),
            "permissions: granted".to_owned(),
        ]
    );
    let fx = Fixture::new();
    fx.write(
        &fx.roots.settings,
        &format!("{{\"permissions\":{{\"allow\":[\"{RULE}\"]}}}}"),
    );
    let outcome = fx.setup().unwrap();
    assert_eq!(
        AgyAdapter.setup_report_lines(&outcome.projection)[1],
        "allow entry command(herdr-threads): pre-existing"
    );
    assert!(AgyAdapter.setup_report_lines(&json!({})).is_empty());
}

#[test]
fn the_settings_kind_is_allow_member_only() {
    let spec = AgyAdapter
        .settings_kind()
        .expect("agy registers a settings kind");
    assert_eq!(spec.harness, "agy");
    assert_eq!(spec.file_name, "settings.json");
    assert!((spec.hook_groups)("cmd").is_empty());
    let allow = spec.owned_allow.expect("an owned allow member");
    assert_eq!(allow.rule, RULE);
    assert!(allow.retired.is_empty());
    assert!(allow.native_rewrites);
}

#[test]
fn setup_needs_a_state_dir_and_refuses_profiles() {
    let fx = Fixture::new();
    let mut request = fx.setup_request();
    request.environment.state_dir = None;
    assert!(matches!(
        AgyAdapter.setup(&request, &budget()),
        Err(SetupFailure::Invalid(_))
    ));
    let mut request = fx.setup_request();
    request.scope = ResolvedSetupScope::Profile {
        name: "p".into(),
        home: fx.home.clone(),
    };
    assert!(matches!(
        AgyAdapter.setup(&request, &budget()),
        Err(SetupFailure::Invalid(_))
    ));
    assert!(!fx.roots.config_root.exists(), "a refusal creates nothing");
}

// ---- consent and backups (ht-poh.18) ----

fn backups(fx: &Fixture) -> Vec<Vec<u8>> {
    crate::harness::setup::user_config_backups(&fx.roots.settings)
}
fn permissions_of(fx: &Fixture) -> Value {
    fx.status().projection["permissions"].clone()
}

#[test]
fn setup_options_declare_with_and_without_permissions() {
    let options = AgyAdapter.setup_options();
    let names: Vec<_> = options.iter().map(|o| (o.name, o.conflicts)).collect();
    assert_eq!(
        names,
        [
            ("with-permissions", &["without-permissions"][..]),
            ("without-permissions", &["with-permissions"][..]),
        ]
    );
}

#[test]
fn ask_without_a_grant_advises_and_writes_nothing() {
    let _run = crate::harness::setup::BackupSession::start();
    let fx = Fixture::new();
    let outcome = fx.setup_with(PermissionPolicy::Ask).unwrap();
    assert!(fx.plugin_file(plugin::HOOKS_JSON).exists());
    assert!(!fx.roots.settings.exists());
    assert_eq!(outcome.projection["allow_entry"]["state"], "not_granted");
    let permissions = &outcome.projection["permissions"];
    assert_eq!(permissions["state"], "not_installed");
    assert_eq!(permissions["rule"], RULE);
    assert_eq!(permissions["action"], "advised");
    assert_eq!(permissions["note"], AGY_PERMISSION_ADVICE);
    let status = fx.status();
    assert!(!diagnostic_codes(&status.diagnostics).contains(&"agy_allow_entry_missing"));
    assert!(status.repairs.is_empty());
}

#[test]
fn ask_keeps_a_recorded_grant_and_re_adds_it_after_an_agy_rewrite() {
    let fx = Fixture::new();
    {
        let _run = crate::harness::setup::BackupSession::start();
        fx.setup().unwrap();
    }
    let rewritten = "{\"model\":\"x\",\"permissions\":{\"allow\":[]}}";
    fx.write(&fx.roots.settings, rewritten);
    let _run = crate::harness::setup::BackupSession::start();
    let outcome = fx.setup_with(PermissionPolicy::Ask).unwrap();
    assert_eq!(outcome.projection["permissions"]["action"], "kept");
    assert_eq!(outcome.projection["allow_entry"]["state"], "owned");
    assert_eq!(
        fx.settings_json(),
        json!({"model":"x","permissions":{"allow":[RULE]}})
    );
    assert_eq!(backups(&fx), [rewritten.as_bytes().to_vec()]);
}

#[test]
fn ask_records_a_users_own_member_without_changing_the_file() {
    let _run = crate::harness::setup::BackupSession::start();
    let fx = Fixture::new();
    let own = "{\"permissions\":{\"allow\":[\"command(herdr-threads)\",\"command(ls)\"]}}";
    fx.write(&fx.roots.settings, own);
    let outcome = fx.setup_with(PermissionPolicy::Ask).unwrap();
    assert_eq!(std::fs::read_to_string(&fx.roots.settings).unwrap(), own);
    assert_eq!(outcome.projection["allow_entry"]["state"], "pre_existing");
    assert_eq!(outcome.projection["permissions"]["pre_existing"], true);
    assert_eq!(outcome.projection["permissions"]["action"], "kept");
    assert!(backups(&fx).is_empty());
    fx.unsetup().unwrap();
    assert!(
        fx.settings_json()["permissions"]["allow"]
            .as_array()
            .unwrap()
            .contains(&json!(RULE))
    );
}

#[test]
fn grant_backs_up_an_existing_file_once_per_run() {
    let _run = crate::harness::setup::BackupSession::start();
    let fx = Fixture::new();
    fx.write(&fx.roots.settings, "{\"theme\":\"dark\"}");
    fx.setup_with(PermissionPolicy::Grant).unwrap();
    assert_eq!(
        fx.settings_json(),
        json!({"theme":"dark","permissions":{"allow":[RULE]}})
    );
    eprintln!(
        "DBG {:?}",
        fx.files_under(fx.roots.settings.parent().unwrap())
    );
    assert_eq!(backups(&fx), [b"{\"theme\":\"dark\"}".to_vec()]);
    let granted = std::fs::read(&fx.roots.settings).unwrap();
    fx.setup_with(PermissionPolicy::Grant).unwrap();
    assert_eq!(std::fs::read(&fx.roots.settings).unwrap(), granted);
    assert_eq!(backups(&fx).len(), 1);
}

#[test]
fn grant_creating_the_file_keeps_no_backup() {
    let _run = crate::harness::setup::BackupSession::start();
    let fx = Fixture::new();
    assert!(!fx.roots.settings.exists());
    fx.setup_with(PermissionPolicy::Grant).unwrap();
    assert!(fx.roots.settings.exists());
    assert!(backups(&fx).is_empty());
}

#[test]
fn decline_removes_only_the_owned_member_and_keeps_the_plugin() {
    let fx = Fixture::new();
    fx.write(
        &fx.roots.settings,
        "{\"permissions\":{\"allow\":[\"command(ls)\"]}}",
    );
    {
        let _run = crate::harness::setup::BackupSession::start();
        fx.setup().unwrap();
    }
    let granted = std::fs::read(&fx.roots.settings).unwrap();
    let earlier = backups(&fx).len();
    let _run = crate::harness::setup::BackupSession::start();
    let outcome = fx.setup_with(PermissionPolicy::Decline).unwrap();
    assert_eq!(outcome.projection["permissions"]["action"], "removed");
    assert_eq!(outcome.projection["allow_entry"]["state"], "not_granted");
    assert_eq!(
        fx.settings_json()["permissions"]["allow"],
        json!(["command(ls)"])
    );
    assert!(fx.plugin_file(plugin::HOOKS_JSON).exists());
    let kept = backups(&fx);
    assert_eq!(kept.len(), earlier + 1);
    assert!(kept.contains(&granted), "the pre-decline bytes are kept");
}

#[test]
fn decline_keeps_a_pre_existing_member() {
    let _run = crate::harness::setup::BackupSession::start();
    let fx = Fixture::new();
    fx.write(
        &fx.roots.settings,
        "{\"permissions\":{\"allow\":[\"command(herdr-threads)\"]}}",
    );
    fx.setup().unwrap();
    let outcome = fx.setup_with(PermissionPolicy::Decline).unwrap();
    assert_eq!(outcome.projection["permissions"]["action"], "removed");
    assert_eq!(outcome.projection["allow_entry"]["state"], "pre_existing");
    assert_eq!(fx.settings_json()["permissions"]["allow"], json!([RULE]));
}

#[test]
fn unsetup_backs_up_settings_before_removing_the_owned_member() {
    let fx = Fixture::new();
    fx.write(&fx.roots.settings, "{\"theme\":\"dark\"}");
    {
        let _run = crate::harness::setup::BackupSession::start();
        fx.setup().unwrap();
    }
    let granted = std::fs::read(&fx.roots.settings).unwrap();
    let earlier = backups(&fx).len();
    let _run = crate::harness::setup::BackupSession::start();
    fx.unsetup().unwrap();
    let kept = backups(&fx);
    assert_eq!(kept.len(), earlier + 1);
    assert!(kept.contains(&granted), "the pre-unsetup bytes are kept");
}

#[test]
fn settle_consent_yes_grants_and_no_declines() {
    let _run = crate::harness::setup::BackupSession::start();
    let ask = |input: &[u8]| {
        let fx = Fixture::new();
        let outcome = fx.setup_with(PermissionPolicy::Ask).unwrap();
        let mut projection = outcome.projection;
        let mut out = Vec::new();
        let mut reader = input;
        AgyAdapter
            .settle_setup_consent(&fx.env(), &mut projection, &mut reader, &mut out)
            .unwrap();
        let present = fx.roots.settings.exists();
        let state = fx.status().projection["allow_entry"]["state"].clone();
        (projection, String::from_utf8(out).unwrap(), present, state)
    };
    let (report, out, present, state) = ask(b"\n");
    assert_eq!(report["permissions"]["action"], "granted");
    assert_eq!(report["allow_entry"]["state"], "owned");
    assert_eq!(out, AGY_PERMISSION_QUESTION);
    assert!(present);
    assert_eq!(state, "owned");
    for input in [&b"n\n"[..], &b""[..]] {
        let (report, _, present, state) = ask(input);
        assert_eq!(report["permissions"]["action"], "declined");
        assert!(report["permissions"].get("note").is_none());
        assert!(!present);
        assert_eq!(state, "not_granted");
    }
    // A report that was not advised is never asked about.
    let fx = Fixture::new();
    let mut projection = fx.setup_with(PermissionPolicy::Grant).unwrap().projection;
    let mut out = Vec::new();
    AgyAdapter
        .settle_setup_consent(&fx.env(), &mut projection, &mut &b"n\n"[..], &mut out)
        .unwrap();
    assert!(out.is_empty());
    assert_eq!(projection["permissions"]["action"], "granted");
    assert_eq!(fx.status().projection["allow_entry"]["state"], "owned");
    let _ = permissions_of(&fx);
}
