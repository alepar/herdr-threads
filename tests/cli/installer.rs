use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture {
    root: PathBuf,
    env: SetupEnv,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("ht-installer-prompts-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("bin")).unwrap();
        for (name, version) in [
            ("claude", "2.1.284 (Claude Code)"),
            ("codex", "codex-cli 0.158.0"),
        ] {
            let file = root.join("bin").join(name);
            fs::write(&file, format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n")).unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = SetupEnv {
            executable: root.join("herdr-threads"),
            state_dir: Some(root.join("state")),
            cwd: root.clone(),
            path: Some(root.join("bin").into_os_string()),
            codex_home: Some(root.join("codex")),
            claude_config_dir: Some(root.join("claude")),
            host_endpoint: Some(root.join("herdr.sock")),
            instance_source: Value::Null,
        };
        Self { root, env }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn confirmation_requires_yes_and_keeps_eof_or_decline() {
    for (answer, expected) in [
        ("y\n", true),
        ("YES\n", true),
        ("\n", false),
        ("n\n", false),
        ("", false),
        ("maybe\n", false),
    ] {
        let mut output = Vec::new();
        assert_eq!(
            confirm("Install?", &mut io::Cursor::new(answer), &mut output).unwrap(),
            expected
        );
        assert!(String::from_utf8(output).unwrap().ends_with("[y/N] "));
    }
}

// Kills a single bundled confirmation, ignoring decline, or prompting for owned skills.
#[test]
fn separate_missing_decisions_and_owned_updates_have_independent_side_effects() {
    let f = Fixture::new();
    let mut prompts = Vec::new();
    let report = execute(&f.env, false, true, |question| {
        prompts.push(question.to_owned());
        Ok(question.contains("skill"))
    });
    assert_eq!(report["exit_status"], 0, "{report}");
    assert_eq!(
        prompts,
        [
            "Install herdr-threads hooks for claude?",
            "Install herdr-threads skill for claude?",
            "Install herdr-threads hooks for codex?",
            "Install herdr-threads skill for codex?"
        ]
    );
    assert!(!f.root.join("claude/settings.json").exists());
    assert!(!f.root.join("codex/hooks.json").exists());
    for h in ["claude", "codex"] {
        assert!(
            f.root
                .join(h)
                .join("skills/herdr-threads/SKILL.md")
                .exists()
        );
    }
    prompts.clear();
    let report = execute(&f.env, false, true, |question| {
        prompts.push(question.to_owned());
        Ok(false)
    });
    assert_eq!(report["exit_status"], 0, "{report}");
    assert_eq!(
        prompts,
        [
            "Install herdr-threads hooks for claude?",
            "Install herdr-threads hooks for codex?"
        ]
    );
    for entry in report["integrations"].as_array().unwrap() {
        assert_eq!(
            entry["outcome"],
            if entry["component"] == "skill" {
                "updated"
            } else {
                "declined"
            }
        );
    }
}

// Kills adopting a skill directory created by another writer while the user considers consent.
#[test]
fn skill_directory_created_during_prompt_is_preserved() {
    let f = Fixture::new();
    let report = execute(&f.env, false, true, |question| {
        if question == "Install herdr-threads skill for claude?" {
            fs::create_dir_all(f.root.join("claude/skills/herdr-threads")).unwrap();
            fs::write(f.root.join("claude/skills/herdr-threads/README"), "foreign").unwrap();
            Ok(true)
        } else {
            Ok(false)
        }
    });
    assert_eq!(report["exit_status"], 1);
    assert!(!f.root.join("claude/skills/herdr-threads/SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(f.root.join("claude/skills/herdr-threads/README")).unwrap(),
        "foreign"
    );
}

// Kills bypassing the foreign-hook precheck when configuration appears during consent.
#[test]
fn foreign_hooks_created_during_prompt_are_preserved() {
    let f = Fixture::new();
    let bytes = br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"/other/herdr-threads hook claude"}]}]}}"#;
    let report = execute(&f.env, false, true, |question| {
        if question == "Install herdr-threads hooks for claude?" {
            fs::create_dir_all(f.root.join("claude")).unwrap();
            fs::write(f.root.join("claude/settings.json"), bytes).unwrap();
            Ok(true)
        } else {
            Ok(false)
        }
    });
    assert_eq!(report["exit_status"], 1);
    assert_eq!(
        fs::read(f.root.join("claude/settings.json")).unwrap(),
        bytes
    );
}
