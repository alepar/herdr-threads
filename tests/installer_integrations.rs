//! Installer integration decisions through the real executable, with isolated roots.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Output};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = herdr_threads::test_support::scratch::temp_path(format!(
            "ht-installer-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(root.join("bin")).unwrap();
        for (name, version) in [
            ("claude", "2.1.284 (Claude Code)"),
            ("codex", "codex-cli 0.158.0"),
        ] {
            let path = root.join("bin").join(name);
            fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_executable(
            std::path::Path::new(env!("CARGO_BIN_EXE_herdr-threads")),
            args,
        )
    }
    fn run_executable(&self, executable: &std::path::Path, args: &[&str]) -> Output {
        let mut command = herdr_threads::test_support::spawn::command(executable);
        herdr_threads::test_support::isolation::scrub_env(&mut command);
        command
            .current_dir(&self.0)
            .env("HOME", self.0.join("home"))
            .env("CODEX_HOME", self.0.join("codex"))
            .env("CLAUDE_CONFIG_DIR", self.0.join("claude"))
            .env("PATH", self.0.join("bin"))
            .args(herdr_threads::test_support::isolation::routed_argv(
                &[
                    std::ffi::OsString::from("--state-dir"),
                    self.0.join("state").into_os_string(),
                    std::ffi::OsString::from("--host-endpoint"),
                    self.0.join("herdr.sock").into_os_string(),
                ],
                args,
            ))
            .output()
            .unwrap()
    }
    fn install(&self) -> Output {
        self.run(&["internal", "installer-integrations", "--confirm-missing"])
    }
    fn skill(&self, harness: &str) -> PathBuf {
        self.0.join(harness).join("skills/herdr-threads/SKILL.md")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// Kills silently installing missing components when stdin/stdout are pipes.
#[test]
fn installer_noninteractive_missing_components_are_skipped_without_writes() {
    let f = Fixture::new();
    let out = f.run(&["internal", "installer-integrations"]);
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    for h in ["claude", "codex"] {
        assert!(text.contains(&format!("{h} hooks: skipped")), "{text}");
        assert!(text.contains(&format!("{h} skill: skipped")), "{text}");
        assert!(!f.0.join(h).exists());
    }
    assert!(!f.0.join("state").exists());
}

// Kills bundled consent/update decisions, duplicate hooks, and replacing unrelated skills/settings.
#[test]
fn installer_explicit_confirmation_installs_then_owned_rerun_needs_no_confirmation() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("claude/skills/other")).unwrap();
    fs::write(f.0.join("claude/skills/other/SKILL.md"), "keep me").unwrap();
    fs::write(f.0.join("claude/settings.json"), r#"{"model":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"keep-hook"}]}]}}"#).unwrap();
    success(&f.install());
    for h in ["claude", "codex"] {
        let skill = fs::read_to_string(f.skill(h)).unwrap();
        assert!(skill.contains("# herdr-threads"));
        assert!(skill.contains("Thread summaries"));
    }
    let settings = fs::read(f.0.join("claude/settings.json")).unwrap();
    let out = f.run(&["internal", "installer-integrations"]);
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    for h in ["claude", "codex"] {
        assert!(text.contains(&format!("{h} hooks: updated")), "{text}");
        assert!(text.contains(&format!("{h} skill: updated")), "{text}");
    }
    assert_eq!(
        fs::read(f.0.join("claude/settings.json")).unwrap(),
        settings
    );
    assert_eq!(
        fs::read_to_string(f.0.join("claude/skills/other/SKILL.md")).unwrap(),
        "keep me"
    );
    let value: serde_json::Value = serde_json::from_slice(&settings).unwrap();
    assert_eq!(value["model"], "keep");
    assert_eq!(
        value["hooks"]["Stop"][0]["hooks"][0]["command"],
        "keep-hook"
    );
}

// Kills overwriting a foreign lookalike or edited owned skill even with explicit consent.
#[test]
fn installer_preserves_foreign_and_edited_skills_and_continues_other_harness() {
    let f = Fixture::new();
    fs::create_dir_all(f.skill("claude").parent().unwrap()).unwrap();
    fs::write(
        f.skill("claude"),
        "---\nname: herdr-threads\n---\nforeign skill",
    )
    .unwrap();
    let out = f.install();
    assert!(!out.status.success());
    assert_eq!(
        fs::read_to_string(f.skill("claude")).unwrap(),
        "---\nname: herdr-threads\n---\nforeign skill"
    );
    assert!(f.skill("codex").exists());
    fs::write(f.skill("codex"), "edited skill").unwrap();
    let out = f.run(&["internal", "installer-integrations"]);
    assert!(!out.status.success());
    assert_eq!(
        fs::read_to_string(f.skill("codex")).unwrap(),
        "edited skill"
    );
}

// Kills treating an installed manifest with a removed group as an owned install to repair silently.
#[test]
fn installer_partial_hooks_are_preserved_and_skill_updates_independently() {
    let f = Fixture::new();
    success(&f.install());
    let hooks = f.0.join("codex/hooks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&hooks).unwrap()).unwrap();
    value["hooks"]["SessionStart"] = serde_json::json!([]);
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&hooks, &bytes).unwrap();
    let out = f.run(&["internal", "installer-integrations"]);
    assert!(!out.status.success());
    assert_eq!(fs::read(hooks).unwrap(), bytes);
    assert!(String::from_utf8_lossy(&out.stdout).contains("codex skill: updated"));
}

// Kills equating a named foreign command or invalid settings to missing owned hooks.
#[test]
fn installer_foreign_and_malformed_hooks_are_preserved_without_adoption() {
    for bytes in [br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"herdr-threads hook claude"}]}]}}"#.as_slice(), b"invalid json"] {
        let f = Fixture::new();
        fs::create_dir_all(f.0.join("claude")).unwrap();
        fs::write(f.0.join("claude/settings.json"), bytes).unwrap();
        let out = f.install();
        assert!(!out.status.success());
        assert_eq!(fs::read(f.0.join("claude/settings.json")).unwrap(), bytes);
        assert!(f.skill("claude").exists());
        assert!(f.skill("codex").exists());
    }
}

// Kills treating a leftover or foreign skill directory as a missing skill to populate.
#[test]
fn installer_foreign_skill_directory_without_entrypoint_is_preserved() {
    let f = Fixture::new();
    let path = f.skill("claude");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path.parent().unwrap().join("README"), "foreign").unwrap();
    assert!(!f.install().status.success());
    assert!(!path.exists());
    assert_eq!(
        fs::read_to_string(path.parent().unwrap().join("README")).unwrap(),
        "foreign"
    );
    assert!(f.skill("codex").exists());
}

// Kills no-op updates that leave a previous owned skill revision in place.
#[test]
fn installer_updates_a_previous_owned_skill_revision_without_confirmation() {
    use sha2::{Digest, Sha256};
    let f = Fixture::new();
    success(&f.install());
    let old = b"previous owned revision";
    let skill = f.skill("codex");
    fs::write(&skill, old).unwrap();
    let manifest = fs::read_dir(f.0.join("state/setup"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("codex-skill-")
        })
        .unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    record["fingerprint"] = serde_json::json!(format!("sha256:{:x}", Sha256::digest(old)));
    fs::write(manifest, serde_json::to_vec(&record).unwrap()).unwrap();
    let out = f.run(&["internal", "installer-integrations"]);
    success(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("codex skill: updated"));
    assert!(
        fs::read_to_string(skill)
            .unwrap()
            .contains("Thread summaries")
    );
}

// Kills silently resuming a prepared allowance just because all its keys are present.
#[test]
fn installer_prepared_codex_allowance_is_preserved_with_intact_hooks() {
    let f = Fixture::new();
    success(&f.install());
    // Seed the legacy allowance through its canonical library; current setup may
    // intentionally omit new allowances for this harness metadata.
    let config_path = f.0.join("codex/config.toml");
    fs::write(&config_path, b"").unwrap();
    let path =
        herdr_threads::cli::setup::manifest_path(&f.0.join("state"), "codex-config", &config_path);
    herdr_threads::harness::codex_config::install(
        &config_path,
        &path,
        "/tmp/ht-installer-fixture.sock",
        &[],
    )
    .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["phase"] = serde_json::json!("prepared");
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&path, &bytes).unwrap();
    let hooks = fs::read(f.0.join("codex/hooks.json")).unwrap();
    let config = fs::read(f.0.join("codex/config.toml")).unwrap();
    assert!(
        !f.run(&["internal", "installer-integrations"])
            .status
            .success()
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(fs::read(f.0.join("codex/hooks.json")).unwrap(), hooks);
    assert_eq!(fs::read(f.0.join("codex/config.toml")).unwrap(), config);
}

// Kills basename-dependent routing, a different guide, or an alias bypassing native context refusal.
#[test]
fn installer_ht_executable_preserves_skill_follow_and_native_authorization() {
    let f = Fixture::new();
    let alias = f.0.join("bin/ht");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_herdr-threads"), &alias).unwrap();
    for args in [
        vec!["skill"],
        vec!["follow", "--help"],
        vec!["follow", "thread-1", "--machine"],
        vec!["ack", "msg-1"],
    ] {
        let canonical = f.run(&args);
        let shortened = f.run_executable(&alias, &args);
        assert_eq!(shortened.status.code(), canonical.status.code(), "{args:?}");
        if args.contains(&"--help") {
            let help = String::from_utf8(shortened.stdout.clone()).unwrap();
            // Clap accurately names the invocation in usage; all options/routing remain equal.
            assert_eq!(
                help.replace("Usage: ht follow", "Usage: herdr-threads follow")
                    .as_bytes(),
                canonical.stdout,
                "{args:?}"
            );
        } else {
            assert_eq!(shortened.stdout, canonical.stdout, "{args:?}");
        }
        assert_eq!(shortened.stderr, canonical.stderr, "{args:?}");
        if args[0] == "ack" {
            assert!(
                String::from_utf8_lossy(&shortened.stderr)
                    .contains("after that seat's lifecycle check-in")
            );
        }
        if args[0] == "skill" || args.contains(&"--help") {
            success(&shortened);
        } else {
            assert!(
                !shortened.status.success(),
                "native claim must still be required"
            );
        }
    }
    assert!(!f.0.join("state").exists());
}

// Kills treating missing/partial/invalid ownership as consent to overwrite or repair a skill.
#[test]
fn installer_refuses_incomplete_skill_manifest_states_independently() {
    for case in ["missing-file", "missing-manifest", "prepared", "malformed"] {
        let f = Fixture::new();
        success(&f.install());
        let path = f.skill("claude");
        let original = fs::read(&path).unwrap();
        let manifest = fs::read_dir(f.0.join("state/setup"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("claude-skill-")
            })
            .unwrap();
        match case {
            "missing-file" => fs::remove_file(&path).unwrap(),
            "missing-manifest" => fs::remove_file(&manifest).unwrap(),
            "prepared" => {
                let mut record: serde_json::Value =
                    serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
                record["complete"] = serde_json::json!(false);
                fs::write(&manifest, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            "malformed" => fs::write(&manifest, b"{broken").unwrap(),
            _ => unreachable!(),
        }
        let before_manifest = fs::read(&manifest).ok();
        let out = f.install();
        assert!(!out.status.success(), "{case}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("claude skill: failed"), "{case}: {text}");
        assert!(text.contains("codex skill: updated"), "{case}: {text}");
        assert_eq!(fs::read(&manifest).ok(), before_manifest, "{case}");
        assert_eq!(
            fs::read(&path).ok(),
            if case == "missing-file" {
                None
            } else {
                Some(original)
            },
            "{case}"
        );
    }
}

impl Fixture {
    /// Runs with `CODEX_HOME` at `codex_home` instead of the fixture's default root.
    fn run_codex_home(&self, codex_home: &std::path::Path, args: &[&str]) -> Output {
        let mut command = herdr_threads::test_support::spawn::command(std::path::Path::new(env!(
            "CARGO_BIN_EXE_herdr-threads"
        )));
        herdr_threads::test_support::isolation::scrub_env(&mut command);
        command
            .current_dir(&self.0)
            .env("HOME", self.0.join("home"))
            .env("CODEX_HOME", codex_home)
            .env("CLAUDE_CONFIG_DIR", self.0.join("claude"))
            .env("PATH", self.0.join("bin"))
            .args(herdr_threads::test_support::isolation::routed_argv(
                &[
                    std::ffi::OsString::from("--state-dir"),
                    self.0.join("state").into_os_string(),
                    std::ffi::OsString::from("--host-endpoint"),
                    self.0.join("herdr.sock").into_os_string(),
                ],
                args,
            ))
            .output()
            .unwrap()
    }
    /// The legacy hook command (`... 'hook' 'codex'`) this fixture's setup reports.
    fn legacy_base(&self) -> String {
        let status: serde_json::Value =
            serde_json::from_slice(&self.run(&["setup-status", "codex", "--json"]).stdout).unwrap();
        let command = status["setup"]["command"].as_str().unwrap();
        command
            .split(" # herdr-threads-owner:")
            .next()
            .unwrap()
            .to_owned()
    }
    /// A second CODEX_HOME holding a copy of another setup's hook file as an older release wrote
    /// it: only `events` registered, each with the legacy owner-marked per-event command, no
    /// manifest of its own. Group shapes are the current ones with the command swapped.
    fn older_copy(&self, events: &[&str]) -> PathBuf {
        let base = self.legacy_base();
        let id = uuid::Uuid::new_v4();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(self.0.join("codex/hooks.json")).unwrap()).unwrap();
        let hooks = value["hooks"].as_object_mut().unwrap();
        hooks.retain(|event, _| events.contains(&event.as_str()));
        for (event, groups) in hooks.iter_mut() {
            groups[0]["hooks"][0]["command"] = serde_json::json!(format!(
                "{base} '--event' '{event}' # herdr-threads-owner:{id}"
            ));
        }
        let copy = self.0.join("codex-copy");
        fs::create_dir_all(&copy).unwrap();
        fs::write(copy.join("hooks.json"), serde_json::to_vec(&value).unwrap()).unwrap();
        copy
    }
}

/// Owner markers per event in a hook file.
fn owners(hooks: &std::path::Path) -> std::collections::BTreeMap<String, Vec<String>> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(hooks).unwrap()).unwrap();
    value["hooks"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(event, groups)| {
            let markers = groups
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["hooks"].as_array().unwrap())
                .filter_map(|hook| {
                    hook["command"]
                        .as_str()?
                        .split("herdr-threads-owner:")
                        .nth(1)
                })
                .map(str::to_owned)
                .collect();
            (event.clone(), markers)
        })
        .collect()
}

/// Per event, the spelling of each herdr-threads registration: `native` (`hook-event`),
/// `legacy` (owner-marked) or `other`. Events with no registration are omitted.
fn spellings(hooks: &std::path::Path) -> std::collections::BTreeMap<String, Vec<&'static str>> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(hooks).unwrap()).unwrap();
    value["hooks"]
        .as_object()
        .unwrap()
        .iter()
        .filter_map(|(event, groups)| {
            let found: Vec<&'static str> = groups
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|group| group["hooks"].as_array().unwrap())
                .filter_map(|hook| hook["command"].as_str())
                .filter(|command| command.contains("herdr-threads"))
                .map(|command| {
                    if command.contains("herdr-threads-owner:") {
                        "legacy"
                    } else if command.contains("'hook-event'") {
                        "native"
                    } else {
                        "other"
                    }
                })
                .collect();
            (!found.is_empty()).then(|| (event.clone(), found))
        })
        .collect()
}

/// The owner marker the copy's SessionStart group carries.
fn owner_of(copy: &std::path::Path) -> String {
    owners(&copy.join("hooks.json"))["SessionStart"][0].clone()
}

const OLDER_CODEX_EVENTS: [&str; 3] = ["SessionStart", "SubagentStart", "PreToolUse"];
/// The events v0.9.0 declared, before later releases added more.
const V090_CODEX_EVENTS: [&str; 5] = [
    "SessionStart",
    "SubagentStart",
    "PreToolUse",
    "SubagentStop",
    "SessionEnd",
];

// Kills: the installer refusing, and setup writing a second owned set beside, a hook file
// copied from another setup by a release that declared fewer events (each copied event then
// fired twice); unsetup unable to remove such a copy. The target is the adapter's current
// declaration, whatever it holds.
#[test]
fn installer_and_setup_upgrade_an_older_copied_hook_file_in_place() {
    for older in [&OLDER_CODEX_EVENTS[..], &V090_CODEX_EVENTS[..]] {
        let f = Fixture::new();
        success(&f.install());
        let declared = spellings(&f.0.join("codex/hooks.json"));
        assert!(declared.len() > older.len(), "{declared:?}");
        let copy = f.older_copy(older);
        let owner = owner_of(&copy);
        let out = f.run_codex_home(&copy, &["internal", "installer-integrations"]);
        success(&out);
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("codex hooks: updated"),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        // Every declared event exactly once, in the current spelling; nothing left beside it.
        let after = spellings(&copy.join("hooks.json"));
        assert_eq!(after, declared);
        let status: serde_json::Value = serde_json::from_slice(
            &f.run_codex_home(&copy, &["setup-status", "codex", "--json"])
                .stdout,
        )
        .unwrap();
        assert_eq!(status["setup"]["installed"], true, "{status}");
        assert_eq!(status["setup"]["adopted"]["owner"], owner, "{status}");
        assert_eq!(status["setup"]["duplicate_hook"], false, "{status}");
        // Rerunning setup is idempotent, and unsetup removes the adopted installation.
        let out = f.run_codex_home(&copy, &["setup", "codex"]);
        success(&out);
        assert_eq!(spellings(&copy.join("hooks.json")), after);
        success(&f.run_codex_home(&copy, &["unsetup", "codex"]));
        assert!(spellings(&copy.join("hooks.json")).is_empty());

        // Setup itself on a fresh older copy: an installation that rewrote the file (Codex
        // review again), never "adopted ... the file was not changed".
        let h = Fixture::new();
        success(&h.install());
        let copy = h.older_copy(older);
        let owner = owner_of(&copy);
        let out = h.run_codex_home(&copy, &["setup", "codex", "--json"]);
        success(&out);
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let report = &report["setup"];
        assert_eq!(report["action"], "installed", "{report}");
        assert_eq!(report["adopted"], owner, "{report}");
        let warnings = report["warnings"].to_string();
        assert!(warnings.contains("review again"), "{warnings}");
        assert!(!warnings.contains("not changed"), "{warnings}");
        assert_eq!(report["trust"]["status"], "review_required", "{report}");
        assert_eq!(spellings(&copy.join("hooks.json")), declared);
    }

    // A copy whose SubagentStart group runs the hook with another event's argument, or with an
    // extra argument, is an edit, not a missing event: refused with the hand-edit remedy, the
    // file left alone, and the installer refuses it too.
    let swap = |from: &'static str, to: &'static str| move |c: &str| c.replace(from, to);
    type Edit = Box<dyn Fn(&str) -> String>;
    let edits: Vec<(&str, Edit)> = vec![
        (
            "wrong event",
            Box::new(swap(
                "'--event' 'SubagentStart'",
                "'--event' 'SessionStart'",
            )),
        ),
        (
            "extra argument",
            Box::new(swap("'SubagentStart' #", "'SubagentStart' '--verbose' #")),
        ),
        (
            "tab",
            Box::new(swap("'SubagentStart' #", "'SubagentStart'\t#")),
        ),
        ("leading space", Box::new(|c: &str| format!(" {c}"))),
        (
            "environment prefix",
            Box::new(|c: &str| format!("FOO=1 {c}")),
        ),
        (
            "unmarked extra argument",
            Box::new(|c: &str| format!("{} '--verbose'", c.split_once(" # ").unwrap().0)),
        ),
    ];
    // Both an older subset and a full current copy: a full copy with one edited group is read
    // as missing that event unless the edit is recognized.
    for (edit, apply, full) in edits
        .iter()
        .flat_map(|(edit, apply)| [(edit, apply, false), (edit, apply, true)])
    {
        let w = Fixture::new();
        success(&w.install());
        let declared: Vec<String> = spellings(&w.0.join("codex/hooks.json"))
            .into_keys()
            .collect();
        let events: Vec<&str> = if full {
            declared.iter().map(String::as_str).collect()
        } else {
            OLDER_CODEX_EVENTS.to_vec()
        };
        let copy = w.older_copy(&events);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(copy.join("hooks.json")).unwrap()).unwrap();
        let slot = &mut value["hooks"]["SubagentStart"][0]["hooks"][0]["command"];
        let original = slot.as_str().unwrap().to_owned();
        let command = apply(&original);
        assert_ne!(command, original, "{edit}");
        *slot = serde_json::json!(command);
        let wrong = serde_json::to_vec(&value).unwrap();
        fs::write(copy.join("hooks.json"), &wrong).unwrap();
        let out = w.run_codex_home(&copy, &["setup", "codex"]);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .contains("remove those groups from the file by hand"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = w.run_codex_home(&copy, &["internal", "installer-integrations"]);
        assert!(!out.status.success());
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("codex hooks: failed"),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(
            fs::read(copy.join("hooks.json")).unwrap(),
            wrong,
            "{edit} full={full}"
        );
    }

    // A copy registered without `--event` is refused with the hand-edit remedy (unsetup does not
    // own it), and the file is left alone.
    let n = Fixture::new();
    success(&n.install());
    let copy = n.older_copy(&OLDER_CODEX_EVENTS);
    let unevented = String::from_utf8(fs::read(copy.join("hooks.json")).unwrap()).unwrap();
    let unevented = OLDER_CODEX_EVENTS.iter().fold(unevented, |text, event| {
        text.replace(&format!(" '--event' '{event}'"), "")
    });
    fs::write(copy.join("hooks.json"), &unevented).unwrap();
    let out = n.run_codex_home(&copy, &["setup", "codex"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("remove those groups from the file by hand"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read(copy.join("hooks.json")).unwrap(),
        unevented.as_bytes()
    );

    // A copy whose group was edited is not adopted: setup refuses rather than add a second set.
    let g = Fixture::new();
    success(&g.install());
    let copy = g.older_copy(&OLDER_CODEX_EVENTS);
    let edited = String::from_utf8(fs::read(copy.join("hooks.json")).unwrap())
        .unwrap()
        .replacen("\"timeout\":10", "\"timeout\":11", 1);
    fs::write(copy.join("hooks.json"), &edited).unwrap();
    let out = g.run_codex_home(&copy, &["setup", "codex"]);
    assert!(
        !out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(
        fs::read(copy.join("hooks.json")).unwrap(),
        edited.as_bytes()
    );
}

// Kills: a legacy sandbox allowance whose keys were all removed by hand blocking the hook
// refresh, or still being reported as installed; an edited allowance no longer refusing.
#[test]
fn installer_refreshes_hooks_beside_a_hand_removed_codex_allowance() {
    let f = Fixture::new();
    success(&f.install());
    let config_path = f.0.join("codex/config.toml");
    let manifest =
        herdr_threads::cli::setup::manifest_path(&f.0.join("state"), "codex-config", &config_path);
    let user = b"model = \"x\"\n";
    fs::write(&config_path, user).unwrap();
    herdr_threads::harness::codex_config::install(
        &config_path,
        &manifest,
        "/tmp/ht-installer-fixture.sock",
        &[],
    )
    .unwrap();
    let installed = fs::read(&config_path).unwrap();
    // Edited (one key changed): still refused, nothing changed.
    let edited = String::from_utf8(installed.clone())
        .unwrap()
        .replace("network_access = true", "network_access = false");
    assert_ne!(edited.as_bytes(), installed.as_slice());
    fs::write(&config_path, &edited).unwrap();
    let out = f.run(&["internal", "installer-integrations"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .contains("recorded Codex allowance is partial or edited"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(fs::read(&config_path).unwrap(), edited.as_bytes());
    // Every key removed by hand: the hooks refresh, the file and record are left alone.
    fs::write(&config_path, user).unwrap();
    let record = fs::read(&manifest).unwrap();
    let out = f.run(&["internal", "installer-integrations"]);
    success(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains("codex hooks: updated"));
    success(&f.run(&["setup", "codex"]));
    assert_eq!(fs::read(&config_path).unwrap(), user);
    assert_eq!(fs::read(&manifest).unwrap(), record);
    let status: serde_json::Value =
        serde_json::from_slice(&f.run(&["setup-status", "codex", "--json"]).stdout).unwrap();
    let sandbox = &status["setup"]["sandbox"];
    assert_eq!(sandbox["recorded"], true, "{sandbox}");
    assert_eq!(sandbox["present"], false, "{sandbox}");
    assert_eq!(sandbox["removed_by_hand"], true, "{sandbox}");
    assert!(sandbox.get("warning").is_none(), "{sandbox}");
    // Owned unsetup still retires the record.
    success(&f.run(&["unsetup", "codex"]));
    assert!(!manifest.exists());
    assert_eq!(fs::read(&config_path).unwrap(), user);
}

// Kills: a hook file an earlier release's setup already doubled (a legacy owner-marked group left
// beside the recorded installation) reported as a healthy installation with no warning.
#[test]
fn setup_and_status_flag_a_hook_file_with_doubled_owned_groups() {
    let f = Fixture::new();
    success(&f.install());
    let base = f.legacy_base();
    let hooks = f.0.join("codex/hooks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&hooks).unwrap()).unwrap();
    let mut stray = value["hooks"]["SessionStart"][0].clone();
    stray["hooks"][0]["command"] = serde_json::json!(format!(
        "{base} '--event' 'SessionStart' # herdr-threads-owner:{}",
        uuid::Uuid::new_v4()
    ));
    value["hooks"]["SessionStart"]
        .as_array_mut()
        .unwrap()
        .insert(0, stray);
    fs::write(&hooks, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(spellings(&hooks)["SessionStart"], ["legacy", "native"]);
    let status: serde_json::Value =
        serde_json::from_slice(&f.run(&["setup-status", "codex", "--json"]).stdout).unwrap();
    let status = &status["setup"];
    assert_eq!(status["duplicate_hook"], true, "{status}");
    assert!(
        status["warnings"]
            .to_string()
            .contains("more than once for SessionStart"),
        "{status}"
    );
    let out = f.run(&["setup", "codex", "--json"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Setup refuses the recorded file holding another registration beside its own groups.
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("beside the recorded installation"), "{text}");
    let out = f.run(&["internal", "installer-integrations"]);
    assert!(!out.status.success());
    // Never resolved by adding yet another registration.
    assert_eq!(spellings(&hooks)["SessionStart"], ["legacy", "native"]);

    // An edited copy (TAB before its owner comment) beside a recorded installation is refused
    // before the recorded groups are touched.
    let g = Fixture::new();
    success(&g.install());
    let hooks = g.0.join("codex/hooks.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&hooks).unwrap()).unwrap();
    let mut stray = value["hooks"]["PreToolUse"][0].clone();
    stray["hooks"][0]["command"] = serde_json::json!(format!(
        "{} '--event' 'PreToolUse'\t# herdr-threads-owner:{}",
        g.legacy_base(),
        uuid::Uuid::new_v4()
    ));
    value["hooks"]["PreToolUse"]
        .as_array_mut()
        .unwrap()
        .push(stray);
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&hooks, &bytes).unwrap();
    let out = g.run(&["setup", "codex"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("beside the recorded installation"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(&hooks).unwrap(), bytes);
}

// Kills the installer writing a shell startup file (even with --confirm-missing), or dropping
// the pane environment advice from its output.
#[test]
fn installer_never_writes_the_pane_environment_and_only_advises() {
    let f = Fixture::new();
    let home = f.0.join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join(".bashrc"), "alias ll='ls -l'\n").unwrap();
    let run = |args: &[&str]| {
        let mut command =
            herdr_threads::test_support::spawn::command(env!("CARGO_BIN_EXE_herdr-threads"));
        herdr_threads::test_support::isolation::scrub_env(&mut command);
        command
            .current_dir(&f.0)
            .env("HOME", &home)
            .env("SHELL", "/bin/zsh")
            .env("XDG_CONFIG_HOME", f.0.join("xdg"))
            .env("CODEX_HOME", f.0.join("codex"))
            .env("CLAUDE_CONFIG_DIR", f.0.join("claude"))
            .env("PATH", f.0.join("bin"))
            .args(herdr_threads::test_support::isolation::routed_argv(
                &[
                    std::ffi::OsString::from("--state-dir"),
                    f.0.join("state").into_os_string(),
                    std::ffi::OsString::from("--host-endpoint"),
                    f.0.join("herdr.sock").into_os_string(),
                ],
                args,
            ))
            .output()
            .unwrap()
    };
    let out = run(&["internal", "installer-integrations", "--confirm-missing"]);
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("codex hooks: installed"), "{text}");
    // The hand-add lines come first, so the 600-character cap never cuts them.
    for line in [
        "  pane environment: Add to your shell startup file: zsh/bash `if [ \"${HERDR_ENV-}\" = 1 ]; then export CODEX_EXEC_SERVER_URL=; fi`;",
        "fish `if test \"$HERDR_ENV\" = 1; set -gx CODEX_EXEC_SERVER_URL \"\"; end`.",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    assert!(text.contains("--with-pane-env"), "{text}");
    assert_eq!(
        fs::read_to_string(home.join(".bashrc")).unwrap(),
        "alias ll='ls -l'\n"
    );
    assert!(!home.join(".zshenv").exists());
    let names: Vec<String> = fs::read_dir(&home)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, [".bashrc"], "no pre-image or new startup file");
    let json = run(&["--json", "internal", "installer-integrations"]);
    success(&json);
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let hooks = report["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["harness"] == "codex" && entry["component"] == "hooks")
        .unwrap();
    assert!(
        hooks["pane_env_advice"]
            .as_str()
            .is_some_and(|advice| advice.contains("CODEX_EXEC_SERVER_URL=")),
        "{report}"
    );
    assert_eq!(
        fs::read_to_string(home.join(".bashrc")).unwrap(),
        "alias ll='ls -l'\n"
    );
}

// Kills a CLI that drops --config-root, writes through an aisw-style symlinked profile, or
// prints an unlabelled verdict for a second root of a harness.
#[test]
fn installer_extra_config_roots_label_verdicts_and_report_shared_files() {
    let f = Fixture::new();
    success(&f.install());
    let profile = f.0.join("profiles/codex/p1");
    fs::create_dir_all(&profile).unwrap();
    std::os::unix::fs::symlink(f.0.join("codex/skills"), profile.join("skills")).unwrap();
    std::os::unix::fs::symlink(f.0.join("codex/rules"), profile.join("rules")).unwrap();
    let root_arg = format!("codex={}", profile.display());
    let out = f.run(&[
        "internal",
        "installer-integrations",
        "--confirm-missing",
        "--config-root",
        &root_arg,
    ]);
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    let label = format!("CODEX_HOME={}", profile.display());
    assert!(
        text.contains(&format!("codex hooks ({label}): installed")),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "codex skill ({label}): shared (shared via symlink; managed at "
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("codex permissions ({label}): shared")),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "codex skill (CODEX_HOME={}): updated",
            f.0.join("codex").display()
        )),
        "{text}"
    );
    assert!(text.contains("claude skill: updated"), "{text}");
    assert!(
        fs::symlink_metadata(profile.join("skills"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // An invalid root fails only its own entry: every other root is still reconciled.
    for bad in ["codex=relative", "nope=/x", "codex", "codex=/a\nb"] {
        let out = f.run(&[
            "internal",
            "installer-integrations",
            "--config-root",
            bad,
            "--config-root",
            &root_arg,
        ]);
        assert!(!out.status.success(), "{bad}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(" config-root: failed (--config-root"),
            "{bad}: {text}"
        );
        assert!(text.contains("claude skill: updated"), "{bad}: {text}");
        assert!(
            text.contains(&format!("codex hooks ({label}): updated")),
            "{bad}: {text}"
        );
        assert!(!text.contains("a\nb"), "{bad}: {text}");
    }
}

// Kills a skill check that treats a profile's own record (made before its skills/ became a
// symlink) as a reason to require a regular directory, prefers that stale record over the
// real root's, or writes through a link to an unmanaged directory (ht-lmjf).
#[test]
fn installer_reports_a_recorded_profile_skill_shared_once_its_skills_is_a_symlink() {
    let f = Fixture::new();
    success(&f.install());
    let shared = f.0.join("profiles/codex/shared");
    let foreign = f.0.join("profiles/codex/foreign");
    let plain = f.0.join("profiles/codex/plain");
    let sibling = f.0.join("profiles/codex/sibling");
    let roots: Vec<String> = [&shared, &foreign, &plain, &sibling]
        .iter()
        .map(|profile| {
            fs::create_dir_all(profile).unwrap();
            format!("codex={}", profile.display())
        })
        .collect();
    let run = |roots: &[&String]| {
        let mut args = vec!["internal", "installer-integrations", "--confirm-missing"];
        for root in roots {
            args.extend(["--config-root", root.as_str()]);
        }
        f.run(&args)
    };
    // Each profile gets its own record while skills/ is still a regular directory.
    success(&run(&roots.iter().collect::<Vec<_>>()));
    let stale: Vec<String> = [&shared, &sibling]
        .iter()
        .map(|profile| {
            let skill = profile.join("skills/herdr-threads/SKILL.md");
            assert!(skill.is_file());
            skill.display().to_string()
        })
        .collect();
    // An external tool then links skills/ to the default root's and to an unmanaged directory.
    for profile in [&shared, &sibling] {
        fs::remove_dir_all(profile.join("skills")).unwrap();
        std::os::unix::fs::symlink(f.0.join("codex/skills"), profile.join("skills")).unwrap();
    }
    let elsewhere = f.0.join("elsewhere/skills");
    fs::create_dir_all(elsewhere.join("herdr-threads")).unwrap();
    fs::write(elsewhere.join("herdr-threads/SKILL.md"), "foreign\n").unwrap();
    fs::remove_dir_all(foreign.join("skills")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, foreign.join("skills")).unwrap();
    // Both linked profiles' own records are stale (an older revision's fingerprint): the real
    // root's record decides, whatever order the records are listed in.
    for entry in fs::read_dir(f.0.join("state/setup")).unwrap() {
        let path = entry.unwrap().path();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap_or_default();
        if manifest["path"]
            .as_str()
            .is_some_and(|p| stale.iter().any(|s| s == p))
        {
            manifest["fingerprint"] = "stale".into();
            fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        }
    }
    let out = run(&[&roots[0], &roots[2], &roots[3]]);
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    for profile in [&shared, &sibling] {
        assert!(
            text.contains(&format!(
                "codex skill (CODEX_HOME={}): shared (shared via symlink; managed at {})",
                profile.display(),
                f.skill("codex").display()
            )),
            "{text}"
        );
    }
    assert!(
        text.contains(&format!(
            "codex skill (CODEX_HOME={}): updated",
            plain.display()
        )),
        "{text}"
    );
    assert!(fs::symlink_metadata(plain.join("skills")).unwrap().is_dir());
    // A link to a directory no managed root owns is refused and nothing is written through it.
    let out = run(&[&roots[1]]);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&format!(
            "codex skill (CODEX_HOME={}): failed ({} is shared through a symlink to {}, which \
             herdr-threads does not own unedited",
            foreign.display(),
            foreign.join("skills/herdr-threads/SKILL.md").display(),
            fs::canonicalize(elsewhere.join("herdr-threads"))
                .unwrap()
                .join("SKILL.md")
                .display()
        )),
        "{text}"
    );
    assert_eq!(
        fs::read_to_string(elsewhere.join("herdr-threads/SKILL.md")).unwrap(),
        "foreign\n"
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 1);
}

// Kills a shared verdict that names whichever record validated (a sibling profile's link path)
// instead of the real file, when the default root is itself a dotfile manager's symlink (ht-lmjf).
#[test]
fn installer_names_the_real_skill_when_the_default_root_is_a_symlink() {
    let f = Fixture::new();
    let real = f.0.join("dotfiles/codex");
    fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, f.0.join("codex")).unwrap();
    success(&f.install());
    // Profiles sort before the default root, and their records stay valid (same revision).
    let profiles = [f.0.join("aisw/a"), f.0.join("aisw/b")];
    let roots: Vec<String> = profiles
        .iter()
        .map(|profile| {
            fs::create_dir_all(profile).unwrap();
            format!("codex={}", profile.display())
        })
        .collect();
    let run = || {
        let mut args = vec!["internal", "installer-integrations", "--confirm-missing"];
        for root in &roots {
            args.extend(["--config-root", root.as_str()]);
        }
        f.run(&args)
    };
    success(&run());
    for profile in &profiles {
        fs::remove_dir_all(profile.join("skills")).unwrap();
        std::os::unix::fs::symlink(f.0.join("codex/skills"), profile.join("skills")).unwrap();
    }
    let out = run();
    success(&out);
    let text = String::from_utf8_lossy(&out.stdout);
    let canonical = fs::canonicalize(&real)
        .unwrap()
        .join("skills/herdr-threads/SKILL.md");
    for profile in &profiles {
        assert!(
            text.contains(&format!(
                "codex skill (CODEX_HOME={}): shared (shared via symlink; managed at {})",
                profile.display(),
                canonical.display()
            )),
            "{text}"
        );
    }
}
