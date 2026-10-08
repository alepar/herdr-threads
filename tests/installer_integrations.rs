//! Installer integration decisions through the real executable, with isolated roots.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Output};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ht-installer-{}", uuid::Uuid::new_v4()));
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
            .args(["--state-dir"])
            .arg(self.0.join("state"))
            .args(["--host-endpoint"])
            .arg(self.0.join("herdr.sock"))
            .args(args)
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
