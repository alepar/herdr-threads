//! The delivery mod's owned install: files, the `env.CLAUDE_CODE_PLUGIN_DIRS`
//! edit, managed policy and revert (ht-j16.7). Every test works in a private
//! temp dir; nothing touches the real Claude config.
use super::*;
use serde_json::json;
use std::time::{Duration, SystemTime};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    root: Dir,
    settings: PathBuf,
    manifest: PathBuf,
    state: PathBuf,
}

impl Fixture {
    fn new(settings: &[u8]) -> Self {
        let root = std::env::temp_dir().join(format!("claude-mod-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let state = root.join("state");
        fs::create_dir_all(state.join("setup")).unwrap();
        let settings_path = root.join("settings.json");
        fs::write(&settings_path, settings).unwrap();
        Self {
            manifest: state.join("setup").join("claude-mod.json"),
            settings: settings_path,
            state,
            root: Dir(root),
        }
    }

    fn install(&self, shell: &[String], policy: &ManagedPolicySources) -> InstallOutcome {
        install(&InstallInput {
            settings: &self.settings,
            manifest: &self.manifest,
            state_dir: &self.state,
            shell_dirs: shell,
            policy,
            launch: None,
        })
        .unwrap()
    }

    fn install_launch(&self, launch: &[String]) -> InstallOutcome {
        install(&InstallInput {
            settings: &self.settings,
            manifest: &self.manifest,
            state_dir: &self.state,
            shell_dirs: &[],
            policy: &ManagedPolicySources::default(),
            launch: Some(launch),
        })
        .unwrap()
    }

    fn plain(&self) -> InstallOutcome {
        self.install(&[], &ManagedPolicySources::default())
    }

    fn revert(&self) -> RevertOutcome {
        revert(&self.settings, &self.manifest, &self.state).unwrap()
    }

    fn dir(&self) -> String {
        mod_dir(&self.state).to_str().unwrap().to_owned()
    }

    fn value(&self) -> Option<String> {
        settings_value(&self.settings)
    }
}

fn mtime(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn install_writes_four_files_and_appends_dir() {
    let f = Fixture::new(b"{\n  \"model\": \"x\"\n}\n");
    let outcome = f.plain();
    assert_eq!(outcome.action, ModAction::Installed);
    assert_eq!(outcome.files_written.len(), 4);
    let dir = mod_dir(&f.state);
    for (path, content) in MOD_FILES {
        assert_eq!(
            fs::read(dir.join(path)).unwrap(),
            content.as_bytes(),
            "{path}"
        );
        assert_eq!(
            fs::metadata(dir.join(path)).unwrap().permissions().mode() & 0o777,
            0o644,
            "{path}"
        );
    }
    // Never the README, tsconfig or tests.
    let mut names: Vec<_> = MOD_FILES.iter().map(|(p, _)| *p).collect();
    names.sort();
    assert_eq!(
        names,
        [
            ".claude-plugin/plugin.json",
            "hooks/hooks.json",
            "hooks/register.js",
            "types/index.d.ts"
        ]
    );
    assert!(!dir.join("README.md").exists());
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let settings: Value = serde_json::from_slice(&fs::read(&f.settings).unwrap()).unwrap();
    assert_eq!(settings["env"][PLUGIN_DIRS_ENV], json!(f.dir()));
    assert_eq!(settings["model"], "x");
    let manifest = recorded(&f.manifest).unwrap().unwrap();
    assert_eq!(manifest.appended, f.dir());
    assert_eq!(manifest.previous, None);
    assert_eq!(manifest.files_fingerprint, files_fingerprint(None));
    assert!(inspect(&f.settings, &f.manifest, &f.state, None).installed());
}

#[test]
fn preserves_existing_settings_value_colon_joined() {
    let f = Fixture::new(br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/a:/b","OTHER":"1"}}"#);
    f.plain();
    assert_eq!(f.value(), Some(format!("/a:/b:{}", f.dir())));
    let settings: Value = serde_json::from_slice(&fs::read(&f.settings).unwrap()).unwrap();
    assert_eq!(settings["env"]["OTHER"], "1");
    assert_eq!(
        recorded(&f.manifest).unwrap().unwrap().previous.as_deref(),
        Some("/a:/b")
    );
}

#[test]
fn rerun_is_idempotent_no_rewrite() {
    let f = Fixture::new(b"{}");
    f.plain();
    let dir = mod_dir(&f.state);
    let times: Vec<_> = MOD_FILES.iter().map(|(p, _)| mtime(&dir.join(p))).collect();
    let settings = fs::read(&f.settings).unwrap();
    let manifest = fs::read(&f.manifest).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let again = f.plain();
    assert_eq!(again.action, ModAction::AlreadyInstalled);
    assert!(again.files_written.is_empty());
    assert_eq!(
        MOD_FILES
            .iter()
            .map(|(p, _)| mtime(&dir.join(p)))
            .collect::<Vec<_>>(),
        times
    );
    assert_eq!(fs::read(&f.settings).unwrap(), settings);
    assert_eq!(fs::read(&f.manifest).unwrap(), manifest);
}

#[test]
fn upgrade_rewrites_changed_files_only() {
    let f = Fixture::new(b"{}");
    f.plain();
    let dir = mod_dir(&f.state);
    fs::write(dir.join("hooks/register.js"), b"// older version\n").unwrap();
    let untouched = mtime(&dir.join("hooks/hooks.json"));
    let settings = fs::read(&f.settings).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let upgraded = f.plain();
    assert_eq!(upgraded.action, ModAction::Upgraded);
    assert_eq!(upgraded.files_written, ["hooks/register.js"]);
    assert_eq!(
        fs::read(dir.join("hooks/register.js")).unwrap(),
        MOD_FILES[2].1.as_bytes()
    );
    assert_eq!(mtime(&dir.join("hooks/hooks.json")), untouched);
    // Same directory path: the settings value is unchanged.
    assert_eq!(fs::read(&f.settings).unwrap(), settings);
    assert!(files_current(&dir, None));
}

#[test]
fn revert_restores_bytes_exactly_when_untouched() {
    let original = b"{\n  \"model\": \"x\"\n}\n";
    let f = Fixture::new(original);
    f.plain();
    assert_ne!(fs::read(&f.settings).unwrap(), original);
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    assert_eq!(fs::read(&f.settings).unwrap(), original);
    assert!(!f.manifest.exists());
    assert!(!mod_dir(&f.state).exists());
    assert!(!f.state.join("claude-mod").exists());
    assert_eq!(f.revert(), RevertOutcome::NotRecorded);
}

#[test]
fn revert_keeps_user_added_dirs() {
    let f = Fixture::new(br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/a"}}"#);
    f.plain();
    // The person adds a directory and another key after setup.
    let edited = format!(
        r#"{{"env":{{"CLAUDE_CODE_PLUGIN_DIRS":"/a:{}:/c"}},"model":"y"}}"#,
        f.dir()
    );
    fs::write(&f.settings, edited).unwrap();
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    let settings: Value = serde_json::from_slice(&fs::read(&f.settings).unwrap()).unwrap();
    assert_eq!(
        settings,
        json!({"env": {"CLAUDE_CODE_PLUGIN_DIRS": "/a:/c"}, "model": "y"})
    );
    assert!(!mod_dir(&f.state).exists());
}

#[test]
fn revert_deletes_key_when_nothing_user_written_remains() {
    // Setup created `env` itself: the key and the empty `env` both go.
    let f = Fixture::new(br#"{"model":"x"}"#);
    f.plain();
    fs::write(
        &f.settings,
        format!(
            r#"{{"model":"y","env":{{"CLAUDE_CODE_PLUGIN_DIRS":"{}"}}}}"#,
            f.dir()
        ),
    )
    .unwrap();
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    let settings: Value = serde_json::from_slice(&fs::read(&f.settings).unwrap()).unwrap();
    assert_eq!(settings, json!({"model": "y"}));

    // `env` held other keys before setup: only our key goes.
    let f = Fixture::new(br#"{"env":{"A":"1"}}"#);
    f.plain();
    fs::write(
        &f.settings,
        format!(
            r#"{{"env":{{"A":"2","CLAUDE_CODE_PLUGIN_DIRS":"{}"}}}}"#,
            f.dir()
        ),
    )
    .unwrap();
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    let settings: Value = serde_json::from_slice(&fs::read(&f.settings).unwrap()).unwrap();
    assert_eq!(settings, json!({"env": {"A": "2"}}));
}

#[test]
fn revert_leaves_a_hand_removed_dir() {
    let f = Fixture::new(b"{}");
    f.plain();
    fs::write(
        &f.settings,
        br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/mine"}}"#,
    )
    .unwrap();
    assert_eq!(f.revert(), RevertOutcome::LeftChanged);
    assert_eq!(
        fs::read(&f.settings).unwrap(),
        br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/mine"}}"#
    );
    assert!(!f.manifest.exists());
}

fn managed(root: &Path, content: &str) -> ManagedPolicySources {
    let file = root.join("managed-settings.json");
    fs::write(&file, content).unwrap();
    ManagedPolicySources {
        files: vec![file],
        ..Default::default()
    }
}

#[test]
fn managed_disable_side_load_flags_skips_env_write() {
    let original = br#"{"model":"x"}"#;
    let f = Fixture::new(original);
    let policy = managed(&f.root.0, r#"{"disableSideloadFlags": true}"#);
    let outcome = f.install(&[], &policy);
    assert_eq!(outcome.action, ModAction::SkippedManagedPolicy);
    assert_eq!(
        outcome.policy,
        ManagedPolicy::DisableSideloadFlags {
            source: f.root.0.join("managed-settings.json")
        }
    );
    assert!(!outcome.recorded);
    assert_eq!(fs::read(&f.settings).unwrap(), original);
    assert!(!f.manifest.exists());
    assert!(!mod_dir(&f.state).exists());

    // A falsy flag, a missing file and an unrelated key do not block.
    for content in [r#"{"disableSideloadFlags": false}"#, r#"{"other": true}"#] {
        assert_eq!(
            managed(&f.root.0, content).check(),
            ManagedPolicy::None,
            "{content}"
        );
    }
    assert_eq!(
        ManagedPolicySources {
            files: vec![f.root.0.join("absent.json")],
            ..Default::default()
        }
        .check(),
        ManagedPolicy::None
    );

    // A policy that appears after install is reported with the record kept
    // so `--hooks-only` can remove the written path.
    let f = Fixture::new(original);
    f.plain();
    let after = f.install(
        &[],
        &managed(&f.root.0, r#"{"disableSideloadFlags": true}"#),
    );
    assert_eq!(after.action, ModAction::SkippedManagedPolicy);
    assert!(after.recorded);
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    assert_eq!(fs::read(&f.settings).unwrap(), original);
}

#[test]
fn shell_env_dirs_copied_and_recorded_then_removed_on_revert() {
    let original = br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/a"}}"#;
    let f = Fixture::new(original);
    // `/a` is already in the settings value; `/s1` is a duplicate.
    let shell = shell_dirs("/s1:/a::/s2:/s1");
    let outcome = f.install(&shell, &ManagedPolicySources::default());
    assert_eq!(outcome.copied_from_shell, ["/s1", "/s2"]);
    assert_eq!(f.value(), Some(format!("/a:/s1:/s2:{}", f.dir())));
    let manifest = recorded(&f.manifest).unwrap().unwrap();
    assert_eq!(manifest.copied_from_shell, ["/s1", "/s2"]);

    // The person edits the file after setup: the structural path removes the
    // appended and copied directories and keeps theirs.
    fs::write(
        &f.settings,
        format!(
            r#"{{"env":{{"CLAUDE_CODE_PLUGIN_DIRS":"/a:/s1:/s2:{}:/c"}}}}"#,
            f.dir()
        ),
    )
    .unwrap();
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    assert_eq!(f.value(), Some("/a:/c".to_owned()));

    // Untouched: exact bytes back, shell dirs included in the restoration.
    let f = Fixture::new(original);
    f.install(&shell, &ManagedPolicySources::default());
    assert_eq!(f.revert(), RevertOutcome::Reverted);
    assert_eq!(fs::read(&f.settings).unwrap(), original);
}

#[test]
fn revert_restores_the_previous_spelling_when_only_original_entries_remain() {
    // A trailing colon in the original is the person's spelling.
    let f = Fixture::new(br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":"/a:"}}"#);
    f.plain();
    fs::write(
        &f.settings,
        format!(
            r#"{{"env":{{"CLAUDE_CODE_PLUGIN_DIRS":"/a::{}"}},"x":1}}"#,
            f.dir()
        ),
    )
    .unwrap();
    f.revert();
    assert_eq!(f.value(), Some("/a:".to_owned()));
}

#[test]
fn version_gate_2_1_287() {
    assert_eq!(MOD_MIN_VERSION, (2, 1, 287));
    assert_eq!(version_supported("2.1.287"), Some(true));
    assert_eq!(version_supported("2.1.286"), Some(false));
    assert_eq!(version_supported("2.1.283"), Some(false));
    assert_eq!(version_supported("2.1.288"), Some(true));
    assert_eq!(version_supported("2.2.0"), Some(true));
    assert_eq!(version_supported("2.0.999"), Some(false));
    assert_eq!(version_supported("3.0.0"), Some(true));
    assert_eq!(version_supported("2.1.1000"), Some(true));
    assert_eq!(version_supported("2.1.287 (Claude Code)"), Some(true));
    assert_eq!(version_supported("garbage"), None);
    assert_eq!(version_supported("2.1"), None);
    assert_eq!(version_supported(""), None);
}

#[test]
fn invalid_value_types_are_refused_without_writes() {
    for original in [
        &br#"{"env":"x"}"#[..],
        br#"{"env":{"CLAUDE_CODE_PLUGIN_DIRS":["/a"]}}"#,
        b"[]",
    ] {
        let f = Fixture::new(original);
        let result = install(&InstallInput {
            settings: &f.settings,
            manifest: &f.manifest,
            state_dir: &f.state,
            shell_dirs: &[],
            policy: &ManagedPolicySources::default(),
            launch: None,
        });
        assert_eq!(result.unwrap_err(), SetupError::Invalid);
        assert_eq!(fs::read(&f.settings).unwrap(), original);
        assert!(!f.manifest.exists());
    }
}

/// A drop-in directory under the fixture root holding `files`.
fn dropins(root: &Path, files: &[(&str, &str)]) -> ManagedPolicySources {
    let dir = root.join("managed-settings.d");
    fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        fs::write(dir.join(name), content).unwrap();
    }
    ManagedPolicySources {
        dropin_dirs: vec![dir],
        ..Default::default()
    }
}

#[test]
fn dropin_json_with_disable_side_load_flags_skips_env_write() {
    let original = br#"{"model":"x"}"#;
    let f = Fixture::new(original);
    let policy = dropins(
        &f.root.0,
        &[
            ("10-a.json", "{}"),
            ("20-b.json", r#"{"disableSideloadFlags": true}"#),
        ],
    );
    let outcome = f.install(&[], &policy);
    assert_eq!(outcome.action, ModAction::SkippedManagedPolicy);
    assert_eq!(
        outcome.policy,
        ManagedPolicy::DisableSideloadFlags {
            source: f.root.0.join("managed-settings.d").join("20-b.json")
        }
    );
    assert_eq!(fs::read(&f.settings).unwrap(), original);
    assert!(!f.manifest.exists());
    assert!(!mod_dir(&f.state).exists());
}

#[test]
fn dropin_non_json_files_are_ignored() {
    let f = Fixture::new(b"{}");
    let policy = dropins(
        &f.root.0,
        &[
            ("README.txt", r#"{"disableSideloadFlags": true}"#),
            ("10-a.json", "{}"),
        ],
    );
    assert_eq!(policy.check(), ManagedPolicy::None);
}

#[test]
fn missing_dropin_dir_is_no_policy() {
    let f = Fixture::new(b"{}");
    let policy = ManagedPolicySources {
        dropin_dirs: vec![f.root.0.join("absent.d")],
        ..Default::default()
    };
    assert_eq!(policy.check(), ManagedPolicy::None);
}

#[test]
fn unparseable_dropin_file_is_unverifiable_and_skips() {
    let original = br#"{"model":"x"}"#;
    let f = Fixture::new(original);
    let policy = dropins(&f.root.0, &[("50.json", "{ not json")]);
    let outcome = f.install(&[], &policy);
    assert_eq!(outcome.action, ModAction::SkippedManagedPolicy);
    assert_eq!(
        outcome.policy,
        ManagedPolicy::Unverifiable {
            source: f.root.0.join("managed-settings.d").join("50.json")
        }
    );
    assert_eq!(fs::read(&f.settings).unwrap(), original);
    assert!(!mod_dir(&f.state).exists());
}

#[test]
fn unparseable_managed_file_is_unverifiable() {
    let f = Fixture::new(b"{}");
    for content in ["not json", "[]", ""] {
        assert!(
            matches!(
                managed(&f.root.0, content).check(),
                ManagedPolicy::Unverifiable { .. }
            ),
            "{content:?}"
        );
    }
    // A set flag is reported in preference to an unreadable source.
    let mut both = dropins(&f.root.0, &[("1.json", r#"{"disableSideloadFlags": 1}"#)]);
    both.files = managed(&f.root.0, "not json").files;
    assert!(matches!(
        both.check(),
        ManagedPolicy::DisableSideloadFlags { .. }
    ));
}

#[test]
fn unreadable_dropin_dir_is_unverifiable() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(b"{}");
    let policy = dropins(&f.root.0, &[("10.json", "{}")]);
    let dir = policy.dropin_dirs[0].clone();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&dir).is_ok(); // root ignores the mode
    let checked = policy.check();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    if !readable {
        assert_eq!(checked, ManagedPolicy::Unverifiable { source: dir });
    }
}

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn prefix(exe: &str) -> Vec<String> {
    argv(&[
        exe,
        "--state-dir",
        "/state",
        "--host-endpoint",
        "/host.sock",
    ])
}

#[test]
fn embedded_register_js_has_exactly_one_launch_marker() {
    let (_, content) = MOD_FILES
        .iter()
        .find(|(path, _)| *path == REGISTER_JS)
        .unwrap();
    let markers = content
        .lines()
        .filter(|line| line.starts_with(LAUNCH_MARKER))
        .count();
    assert_eq!(markers, 1);
    let launches = content
        .lines()
        .filter(|line| line.starts_with(LAUNCH_PREFIX))
        .count();
    assert_eq!(
        launches, 1,
        "no other line could be mistaken for the launch"
    );
}

#[test]
fn launch_prefix_strips_the_hook_tail() {
    let hooks = argv(&[
        "/abs/ht",
        "--state-dir",
        "/state",
        "--host-endpoint",
        "/host.sock",
        "hook",
        "claude",
    ]);
    assert_eq!(launch_prefix(&hooks).unwrap(), prefix("/abs/ht"));
    for bad in [
        argv(&["/abs/ht", "hook", "codex"]),
        argv(&["/abs/ht", "watch"]),
        argv(&["hook", "claude"]),
        Vec::new(),
    ] {
        assert_eq!(
            launch_prefix(&bad).unwrap_err(),
            SetupError::Invalid,
            "{bad:?}"
        );
    }
}

#[test]
fn install_renders_the_launch_into_register_js() {
    let f = Fixture::new(b"{}");
    let one = prefix("/abs/one");
    let other = prefix("/abs/two");
    assert_eq!(f.install_launch(&one).action, ModAction::Installed);
    let dir = mod_dir(&f.state);
    let embedded = MOD_FILES
        .iter()
        .find(|(path, _)| *path == REGISTER_JS)
        .unwrap()
        .1;
    let installed = fs::read_to_string(dir.join(REGISTER_JS)).unwrap();
    let rendered_line = installed
        .lines()
        .find(|line| line.starts_with(LAUNCH_PREFIX))
        .unwrap();
    assert!(rendered_line.ends_with(LAUNCH_SUFFIX));
    assert_eq!(installed_launch(&dir), Some(one.clone()));
    // Every other line is the embedded one.
    let without = |text: &str, starts: &str| -> Vec<String> {
        text.lines()
            .filter(|line| !line.starts_with(starts))
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        without(&installed, LAUNCH_PREFIX),
        without(embedded, LAUNCH_PREFIX)
    );
    assert_eq!(installed.ends_with('\n'), embedded.ends_with('\n'));
    for (path, content) in MOD_FILES {
        if path != REGISTER_JS {
            assert_eq!(fs::read(dir.join(path)).unwrap(), content.as_bytes());
        }
    }
    assert!(files_current(&dir, Some(&one)));
    assert!(!files_current(&dir, Some(&other)));
    assert!(!files_current(&dir, None));
    // A changed prefix rewrites only register.js.
    let outcome = f.install_launch(&other);
    assert_eq!(outcome.action, ModAction::Upgraded);
    assert_eq!(outcome.files_written, [REGISTER_JS]);
    assert_eq!(installed_launch(&dir), Some(other.clone()));
    assert!(files_current(&dir, Some(&other)));
    assert_eq!(f.install_launch(&other).action, ModAction::AlreadyInstalled);
    assert_ne!(
        files_fingerprint(Some(&one)),
        files_fingerprint(Some(&other))
    );
}

#[test]
fn inspect_reports_installed_launch_and_mismatch() {
    let f = Fixture::new(b"{}");
    let one = prefix("/abs/one");
    let other = prefix("/abs/two");
    let before = inspect(&f.settings, &f.manifest, &f.state, Some(&one));
    assert_eq!(before.installed_launch, None);
    assert!(!before.launch_current);
    f.install_launch(&one);
    let same = inspect(&f.settings, &f.manifest, &f.state, Some(&one));
    assert_eq!(same.installed_launch, Some(one.clone()));
    assert!(same.launch_current && same.installed());
    let differs = inspect(&f.settings, &f.manifest, &f.state, Some(&other));
    assert_eq!(differs.installed_launch, Some(one));
    assert!(!differs.launch_current);
    assert!(!differs.installed(), "stale launch is not an installed mod");
    // The unrendered null form reads as no launch.
    let g = Fixture::new(b"{}");
    g.plain();
    let bare = inspect(&g.settings, &g.manifest, &g.state, None);
    assert_eq!(bare.installed_launch, None);
    assert!(bare.launch_current);
}
