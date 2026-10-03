use crate::harness::setup::*;
use serde_json::{Value, json};
use sha2::Digest;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

// The Claude installation the bulk of these tests exercise: user-level
// `settings.json` with the owned allow rule.
fn install_claude_user(
    config: &std::path::Path,
    manifest: &std::path::Path,
    argv: &[String],
    base: &[u8],
) -> Result<OwnershipManifest, SetupError> {
    install_user_settings(SettingsKind::ClaudeUser, config, manifest, argv, base)
}
fn install_claude_user_with_fault<F: FnOnce()>(
    config: &std::path::Path,
    manifest: &std::path::Path,
    argv: &[String],
    base: &[u8],
    fault: InstallFault,
    action: F,
) -> Result<OwnershipManifest, SetupError> {
    install_user_settings_with_fault(
        SettingsKind::ClaudeUser,
        config,
        manifest,
        argv,
        base,
        fault,
        action,
    )
}
fn inspect_claude_user(
    config: &std::path::Path,
    manifest: &std::path::Path,
    observed: NativeObservation,
) -> Result<HookInspection, SetupError> {
    inspect_user_settings(SettingsKind::ClaudeUser, config, manifest, observed)
}
fn remove_claude_user(
    config: &std::path::Path,
    manifest: &std::path::Path,
) -> Result<(), SetupError> {
    remove_user_settings(SettingsKind::ClaudeUser, config, manifest)
}
#[test]
fn claude_composes_preserves_permissions_and_is_idempotent() {
    let input=br#"{"permissions":{"deny":["Bash(rm *)"]},"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"user-hook"}]}],"Stop":[{"hooks":[{"type":"command","command":"stop"}]}]},"other":42}"#;
    let argv = vec!["/private/tmp/space dir/雪'quote".into(), "hook".into()];
    let plan = plan_claude(input, &argv).unwrap();
    let v: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
    assert_eq!(v["permissions"]["deny"], json!(["Bash(rm *)"]));
    assert_eq!(v["other"], 42);
    assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["command"], "stop");
    let again = plan_claude(&plan.proposed_bytes, &argv).unwrap();
    assert_eq!(again.proposed_bytes, plan.proposed_bytes);
    assert_eq!(again.owned, plan.owned);
    assert_eq!(
        plan_claude(input, &argv).unwrap().base_fingerprint,
        plan.base_fingerprint
    );
    let removed = uninstall_json(&plan.proposed_bytes, &plan.owned).unwrap();
    let v: Value = serde_json::from_slice(&removed).unwrap();
    assert_eq!(v["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(v["permissions"]["deny"], json!(["Bash(rm *)"]));
}
#[test]
fn uninstall_and_composition_refuse_changed_owned_entry() {
    let argv = vec!["/tmp/owned".into()];
    let plan = plan_claude(b"{}", &argv).unwrap();
    let mut v: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
    v["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = json!(99);
    let changed = serde_json::to_vec(&v).unwrap();
    assert_eq!(
        uninstall_json(&changed, &plan.owned),
        Err(SetupError::Conflict)
    );
    assert_eq!(plan_claude(&changed, &argv), Err(SetupError::Conflict));
    assert_eq!(
        plan_claude(br#"{"hooks":{"PreToolUse":5}}"#, &argv),
        Err(SetupError::Invalid)
    );
}
#[test]
fn codex_inline_flags_preserve_groups_and_do_not_invent_trust() {
    let groups = vec![EventGroups {
        event: "SessionStart".into(),
        groups: vec![json!({"hooks":[{"type":"command","command":"user-hook"}]})],
    }];
    let plan = plan_codex_for_version(&groups, &["/tmp/space 雪/owned".into()], &pinned()).unwrap();
    assert_eq!(plan.launch_argv[0], "--no-daemon");
    let flag = &plan.launch_argv[2];
    assert!(flag.starts_with("hooks.SessionStart=["));
    assert!(flag.contains("user-hook"));
    assert!(flag.contains("space 雪"));
    assert!(!plan.launch_argv.iter().any(|s| s.contains("trusted_hash")));
    assert_eq!(plan.events[0].groups.len(), 2);
    assert_eq!(
        plan_codex_for_version(&plan.events, &["/tmp/space 雪/owned".into()], &pinned())
            .unwrap()
            .events,
        plan.events
    );
}
/// P1 (ht-p03.15). Kills: a launch line that always prepends `--no-daemon` (Codex refuses the
/// flag twice, so the line fails where the user's `codex` function already adds it or the
/// caller passes it), one that drops it for a plain launch, and a second composition rule that
/// disagrees with `launch`'s.
#[test]
fn setup_codex_launch_argv_has_one_no_daemon() {
    let count = |argv: &[String]| argv.iter().filter(|a| *a == "--no-daemon").count();
    let plan = plan_codex_for_version(&[], &["/tmp/owned".into()], &pinned()).unwrap();
    assert_eq!(count(&plan.launch_argv), 1);
    assert_eq!(plan.launch_argv[0], "--no-daemon");
    assert_eq!(plan.launch_argv[1], "-c");
    assert_eq!(&plan.launch_argv[1..], plan.session_config.as_slice());

    // The caller's own flag is the single one (kept, not duplicated).
    let with_flag = plan
        .launch_argv_for(vec!["--no-daemon".into(), "PROMPT".into()], false)
        .unwrap();
    assert_eq!(count(&with_flag), 1, "{with_flag:?}");
    assert_eq!(with_flag.last().unwrap(), "PROMPT");
    // A shell function that adds it leaves none here, whether or not the caller had it.
    for caller in [
        vec!["PROMPT".to_owned()],
        vec!["--no-daemon".into(), "PROMPT".into()],
    ] {
        let wrapped = plan.launch_argv_for(caller, true).unwrap();
        assert_eq!(count(&wrapped), 0, "{wrapped:?}");
        assert_eq!(wrapped.last().unwrap(), "PROMPT");
        assert_eq!(wrapped[0], "-c");
    }
    // The same rule as `launch`.
    assert_eq!(
        plan.launch_argv_for(vec!["exec".into(), "PROMPT".into()], false)
            .unwrap(),
        crate::harness::launch::compose_native_argv_with(
            crate::protocol::authority::Harness::Codex,
            vec!["exec".into(), "PROMPT".into()],
            plan.session_config.clone(),
            false
        )
        .unwrap()
    );
    // A daemon-mode conflict is refused rather than composed.
    assert_eq!(
        plan.launch_argv_for(vec!["--daemon".into()], false),
        Err(SetupError::Invalid)
    );
}
#[test]
fn actual_shell_boundary_preserves_spaces_unicode_and_quotes() {
    let words = vec![
        "path with spaces/雪'foo".into(),
        "$(touch should-not-run)".into(),
    ];
    let command = shell_command(&words).unwrap();
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("set -- {command}; printf '%s\\n' \"$@\"")])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "path with spaces/雪'foo\n$(touch should-not-run)\n"
    );
    assert_eq!(
        shell_command(&["bad\0arg".into()]),
        Err(SetupError::Invalid)
    );
}
#[test]
fn concurrent_config_change_and_owned_command_edit_refuse_publication() {
    let argv = vec!["/tmp/owned".into()];
    let plan = plan_claude(b"{}", &argv).unwrap();
    assert_eq!(verify_base(&plan, b"{}"), Ok(()));
    assert_eq!(
        verify_base(&plan, b"{\"unrelated\":true}"),
        Err(SetupError::Conflict)
    );
    let mut v: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
    v["hooks"]["PreToolUse"][0]["hooks"][0]["command"] = json!("replacement");
    assert_eq!(
        uninstall_json(&serde_json::to_vec(&v).unwrap(), &plan.owned),
        Err(SetupError::Conflict)
    );
}
#[test]
fn codex_uninstall_keeps_unrelated_events_and_changed_group_conflicts() {
    let existing = vec![EventGroups {
        event: "SessionStart".into(),
        groups: vec![json!({"hooks":[{"type":"command","command":"user-hook"}]})],
    }];
    let plan = plan_codex_for_version(&existing, &["/tmp/owned".into()], &pinned()).unwrap();
    let removed = uninstall_codex(&plan.events, &plan.owned).unwrap();
    assert_eq!(removed[0].groups, existing[0].groups);
    assert_eq!(
        removed
            .iter()
            .find(|e| e.event == "SubagentStart")
            .unwrap()
            .groups
            .len(),
        0
    );
    assert_eq!(
        removed
            .iter()
            .find(|e| e.event == "PreToolUse")
            .unwrap()
            .groups
            .len(),
        0
    );
    let mut changed = plan.events.clone();
    changed[0].groups[1]["hooks"][0]["command"] = json!("edited");
    assert_eq!(
        uninstall_codex(&changed, &plan.owned),
        Err(SetupError::Conflict)
    );
}
#[test]
fn claude_launch_argv_preserves_path_without_shell_expansion() {
    let plan = plan_claude(b"{}", &["/tmp/owned".into()]).unwrap();
    assert_eq!(
        plan.launch_argv("/private/tmp/space 雪/settings.json")
            .unwrap(),
        vec!["--settings", "/private/tmp/space 雪/settings.json"]
    );
    assert_eq!(plan.launch_argv("bad\0path"), Err(SetupError::Invalid));
}
#[test]
fn codex_existing_groups_guard_refuses_concurrent_edit() {
    let groups = vec![EventGroups {
        event: "Stop".into(),
        groups: vec![json!({"hooks":[{"type":"command","command":"user"}]})],
    }];
    let plan = plan_codex_for_version(&groups, &["/tmp/owned".into()], &pinned()).unwrap();
    assert_eq!(verify_codex_base(&plan, &groups), Ok(()));
    let mut changed = groups;
    changed[0].groups.clear();
    assert_eq!(
        verify_codex_base(&plan, &changed),
        Err(SetupError::Conflict)
    );
}

#[test]
fn uninstall_preserves_unrelated_group_changes() {
    let plan = plan_claude(br#"{"hooks":{"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"user"}]}]}}"#, &["/tmp/owned".into()]).unwrap();
    let mut v: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
    v["hooks"]["PreToolUse"][0]["matcher"] = json!("Write");
    let removed = uninstall_json(&serde_json::to_vec(&v).unwrap(), &plan.owned).unwrap();
    let after: Value = serde_json::from_slice(&removed).unwrap();
    assert_eq!(after["hooks"]["PreToolUse"][0]["matcher"], "Write");
    assert_eq!(after["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
}

#[test]
fn project_install_inspect_and_remove_preserve_permissions_and_other_hooks() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let scope = dir.join("project with spaces 雪").join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest = dir.join("ownership manifest.json");
    let original = br#"{"permissions":{"deny":["Bash(rm *)"]},"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user"}]}]}}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/space path/herdr-threads".into(), "hook".into()];
    let installation = install_claude_user(&config, &manifest, &argv, original).unwrap();
    assert_eq!(installation.harness, "claude");
    let status = inspect_claude_user(&config, &manifest, NativeObservation::Unknown).unwrap();
    assert!(status.installed);
    assert_eq!(status.observed, NativeObservation::Unknown);
    assert!(status.configured_hook.is_some());
    let status = inspect_claude_user(&config, &manifest, NativeObservation::Observed).unwrap();
    assert_eq!(status.observed, NativeObservation::Observed);
    remove_claude_user(&config, &manifest).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["permissions"]["deny"], json!(["Bash(rm *)"]));
    assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["command"], "user");
    assert!(after["hooks"].get("PreToolUse").is_none());
    assert!(!manifest.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn project_setup_refuses_stale_base_and_changed_owned_entry() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let scope = dir.join(".claude");
    fs::create_dir(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user(&config, &manifest, &argv, b"{\"stale\":true}"),
        Err(SetupError::Conflict)
    );
    install_claude_user(&config, &manifest, &argv, b"{}").unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = json!(99);
    fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        remove_claude_user(&config, &manifest),
        Err(SetupError::Conflict)
    );
    assert!(manifest.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_session_descriptor_does_not_claim_native_observation() {
    let plan = plan_codex_for_version(&[], &["/tmp/space path/hook".into()], &pinned()).unwrap();
    let manifest = codex_session_manifest(&plan).unwrap();
    let status =
        inspect_codex_session(&plan.events, &manifest, NativeObservation::Unknown).unwrap();
    assert!(status.installed);
    assert_eq!(status.observed, NativeObservation::Unknown);
    assert_eq!(status.configured_hook.unwrap().scope, "session");
    let mut changed = plan.events.clone();
    changed[0].groups[0]["hooks"][0]["command"] = json!("other");
    assert!(
        !inspect_codex_session(&changed, &manifest, NativeObservation::Observed)
            .unwrap()
            .installed
    );
}

#[test]
fn removal_refuses_missing_owned_entry_even_when_event_is_empty() {
    let plan = plan_claude(b"{}", &["/tmp/owned".into()]).unwrap();
    assert_eq!(
        uninstall_json(br#"{"hooks":{"PreToolUse":[]}}"#, &plan.owned),
        Err(SetupError::Conflict)
    );
}

#[test]
fn project_install_refuses_preexisting_matching_hook_without_ownership() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let scope = dir.join(".claude");
    fs::create_dir(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest = dir.join("manifest.json");
    let argv = vec!["/tmp/owned".into()];
    let preexisting = plan_claude(b"{}", &argv).unwrap().proposed_bytes;
    fs::write(&config, &preexisting).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest, &argv, &preexisting),
        Err(SetupError::Conflict)
    );
    assert!(!manifest.exists());
    fs::remove_dir_all(dir).unwrap();
}

/// Kills: a user installation that edits any file other than the kind's own
/// (`settings.json` for Claude, `hooks.json` for Codex), e.g. a project's
/// `settings.local.json` or Codex's `config.toml`.
#[test]
fn user_install_rejects_any_other_settings_file() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let manifest = dir.join("manifest.json");
    for (kind, name) in [
        (SettingsKind::ClaudeUser, "settings.local.json"),
        (SettingsKind::ClaudeUser, "hooks.json"),
        (SettingsKind::CodexUser, "settings.json"),
        (SettingsKind::CodexUser, "config.toml"),
    ] {
        let config = dir.join(name);
        fs::write(&config, b"{}").unwrap();
        assert_eq!(
            install_user_settings(kind, &config, &manifest, &["/tmp/owned".into()], b"{}"),
            Err(SetupError::Invalid)
        );
        assert_eq!(fs::read(&config).unwrap(), b"{}");
        assert!(!manifest.exists());
    }
    fs::remove_dir_all(dir).unwrap();
}

/// Kills: a Codex hooks.json installation that drops a declared event or its
/// Bash matcher, records an allow rule (Codex has none), loses a user's
/// existing groups (Herdr's own SessionStart hook), or is not removed back
/// to the original bytes.
#[test]
fn codex_user_hooks_install_inspect_and_remove_keep_existing_groups() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let config = dir.join("hooks.json");
    let manifest = dir.join("manifest.json");
    let original = br#"{"hooks":{"SessionStart":[{"hooks":[{"command":"bash '/h/herdr-agent-state.sh' session","timeout":10,"type":"command"}]}]}}"#;
    fs::write(&config, original).unwrap();
    let argv = vec![
        "/opt/herdr-threads".to_owned(),
        "hook".to_owned(),
        "codex".to_owned(),
    ];
    let kind = SettingsKind::CodexUser;
    let installed = install_user_settings(kind, &config, &manifest, &argv, original).unwrap();
    assert!(installed.permission.is_none());
    assert_eq!(installed.scope, "user");
    let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    let start = value["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(start.len(), 2);
    assert_eq!(
        start[0]["hooks"][0]["command"],
        "bash '/h/herdr-agent-state.sh' session"
    );
    assert_eq!(value["hooks"]["PreToolUse"][0]["matcher"], "^Bash$");
    assert!(
        value["hooks"]["SubagentStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("# herdr-threads-owner:")
    );
    assert!(value.get("permissions").is_none());
    let status =
        inspect_user_settings(kind, &config, &manifest, NativeObservation::Unknown).unwrap();
    assert!(status.installed);
    assert!(status.allow_rule.is_none());
    assert_eq!(status.configured_hook.unwrap().scope, "user");
    // Idempotent.
    assert_eq!(
        install_user_settings(kind, &config, &manifest, &argv, original).unwrap(),
        installed
    );
    // A Claude manifest never inspects a Codex file and vice versa.
    assert_eq!(
        inspect_user_settings(
            SettingsKind::ClaudeUser,
            &config,
            &manifest,
            NativeObservation::Unknown
        ),
        Err(SetupError::Conflict)
    );
    remove_user_settings(kind, &config, &manifest).unwrap();
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(!manifest.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ownership_manifest_can_reopen_large_valid_settings_backup() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest = dir.join("manifest.json");
    let original = serde_json::to_vec(&json!({"note":"x".repeat(400_000)})).unwrap();
    fs::write(&config, &original).unwrap();
    install_claude_user(&config, &manifest, &["/tmp/owned".into()], &original).unwrap();
    assert!(
        inspect_claude_user(&config, &manifest, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    remove_claude_user(&config, &manifest).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inspection_fingerprint_tracks_current_claude_bytes_and_codex_groups() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let manifest =
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], b"{}").unwrap();
    let mut current: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    current["permissions"]["deny"] = json!(["Bash(rm *)"]);
    fs::write(&config, serde_json::to_vec(&current).unwrap()).unwrap();
    let now = fs::read(&config).unwrap();
    let status = inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
    assert!(status.installed);
    assert_eq!(
        status.configured_hook.unwrap().fingerprint,
        format!("sha256:{:x}", sha2::Sha256::digest(&now))
    );
    assert_ne!(
        manifest.installed_fingerprint,
        format!("sha256:{:x}", sha2::Sha256::digest(&now))
    );
    let plan = plan_codex_for_version(&[], &["/tmp/owned".into()], &pinned()).unwrap();
    let codex_manifest = codex_session_manifest(&plan).unwrap();
    let mut events = plan.events.clone();
    events.push(EventGroups {
        event: "Stop".into(),
        groups: vec![json!({"hooks":[{"type":"command","command":"user"}]})],
    });
    let status =
        inspect_codex_session(&events, &codex_manifest, NativeObservation::Observed).unwrap();
    assert!(status.installed);
    assert_eq!(status.observed, NativeObservation::Observed);
    assert_ne!(
        status.configured_hook.unwrap().fingerprint,
        codex_manifest.installed_fingerprint
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inspection_rejects_empty_or_damaged_ownership() {
    let plan = plan_codex_for_version(&[], &["/tmp/owned".into()], &pinned()).unwrap();
    let mut manifest = codex_session_manifest(&plan).unwrap();
    manifest.owned.clear();
    assert!(
        !inspect_codex_session(&plan.events, &manifest, NativeObservation::Observed)
            .unwrap()
            .installed
    );
    // Kills: inspection ignoring the declared Bash matcher. A group that lost
    // its matcher (and is present in the events) must not count as installed.
    manifest = codex_session_manifest(&plan).unwrap();
    let tool = manifest
        .owned
        .iter()
        .position(|e| e.event == "PreToolUse")
        .unwrap();
    let mut events = plan.events.clone();
    let row = events.iter_mut().find(|e| e.event == "PreToolUse").unwrap();
    row.groups[0].as_object_mut().unwrap().remove("matcher");
    manifest.owned[tool].group = row.groups[0].clone();
    manifest.owned[tool].fingerprint = format!(
        "sha256:{:x}",
        sha2::Sha256::digest(serde_json::to_vec(&row.groups[0]).unwrap())
    );
    assert!(
        !inspect_codex_session(&events, &manifest, NativeObservation::Observed)
            .unwrap()
            .installed
    );
    manifest = codex_session_manifest(&plan).unwrap();
    manifest.owned[0].fingerprint = "damaged".into();
    assert!(
        !inspect_codex_session(&plan.events, &manifest, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let mut claude =
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], b"{}").unwrap();
    claude.owned.clear();
    fs::write(&manifest_path, serde_json::to_vec(&claude).unwrap()).unwrap();
    let status = inspect_claude_user(&config, &manifest_path, NativeObservation::Observed).unwrap();
    assert!(!status.installed);
    assert!(status.configured_hook.is_none());
    assert_eq!(status.observed, NativeObservation::Observed);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepared_manifest_recovers_detected_unrelated_edit_without_overwrite() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let changed = br#"{"permissions":{"deny":["Bash(rm *)"]}}"#;
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            b"{}",
            InstallFault::AfterManifest,
            || fs::write(&config, changed).unwrap()
        ),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), changed);
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&manifest_path).unwrap()).unwrap()["phase"],
        "prepared"
    );
    let recovered = install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    assert_eq!(recovered.original_bytes, b"{}");
    let installed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(installed["permissions"]["deny"], json!(["Bash(rm *)"]));
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(fs::read(&config).unwrap(), changed);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepared_manifest_can_be_removed_after_replacement_failure() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            b"{}",
            InstallFault::BeforeReplacement,
            || fs::remove_file(&config).unwrap()
        ),
        Err(SetupError::Io)
    );
    assert!(manifest_path.exists());
    fs::write(&config, br#"{"other":42}"#).unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(fs::read(&config).unwrap(), br#"{"other":42}"#);
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn partial_manifest_write_is_invisible_and_retryable() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            b"{}",
            InstallFault::PartialManifest,
            || {}
        ),
        Err(SetupError::Io)
    );
    assert_eq!(fs::read(&config).unwrap(), b"{}");
    assert!(!manifest_path.exists());
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepared_removal_refuses_edited_owned_command() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            b"{}",
            InstallFault::AfterManifest,
            || fs::write(&config, br#"{"other":1}"#).unwrap()
        ),
        Err(SetupError::Conflict)
    );
    let manifest: OwnershipManifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let pre_tool = manifest
        .owned
        .iter()
        .find(|entry| entry.event == "PreToolUse")
        .unwrap();
    let mut value = json!({"other":1,"hooks":{"PreToolUse":[pre_tool.group.clone()]}});
    value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = json!(99);
    let changed = serde_json::to_vec(&value).unwrap();
    fs::write(&config, &changed).unwrap();
    assert_eq!(
        remove_claude_user(&config, &manifest_path),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), changed);
    assert!(manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn installed_manifest_retry_is_idempotent_after_publication() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    let first = install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    let config_bytes = fs::read(&config).unwrap();
    let manifest_bytes = fs::read(&manifest_path).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Ok(first)
    );
    assert_eq!(fs::read(&config).unwrap(), config_bytes);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn public_retry_after_settings_publication_keeps_later_unrelated_edit() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    let mut manifest = install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    manifest.phase = InstallPhase::Prepared;
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut current: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    current["permissions"] = json!({"deny":["Bash(rm *)"]});
    fs::write(&config, serde_json::to_vec(&current).unwrap()).unwrap();
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["permissions"]["deny"], json!(["Bash(rm *)"]));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepared_retry_refuses_independently_added_identical_hook() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            b"{}",
            InstallFault::AfterManifest,
            || fs::write(&config, br#"{"other":1}"#).unwrap()
        ),
        Err(SetupError::Conflict)
    );
    let independent = plan_claude(br#"{"other":1}"#, &argv)
        .unwrap()
        .proposed_bytes;
    fs::write(&config, &independent).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(fs::read(&config).unwrap(), independent);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn owned_command_marker_keeps_shell_argv_unchanged() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let argv = vec![
        "/bin/echo".into(),
        "a b'雪".into(),
        "$(touch should-not-run)".into(),
    ];
    let manifest = install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    let command = manifest.owned[0].group["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert!(command.contains("# herdr-threads-owner:"));
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", command])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "a b'雪 $(touch should-not-run) --event SessionStart\n"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn damaged_installation_marker_is_not_owned() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    fs::write(&config, b"{}").unwrap();
    let mut manifest =
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], b"{}").unwrap();
    let installed = fs::read(&config).unwrap();
    manifest.installation_id = uuid::Uuid::new_v4().to_string();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let status = inspect_claude_user(&config, &manifest_path, NativeObservation::Observed).unwrap();
    assert!(!status.installed);
    assert!(status.configured_hook.is_none());
    assert_eq!(status.observed, NativeObservation::Observed);
    assert_eq!(
        remove_claude_user(&config, &manifest_path),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), installed);
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn ownership_manifest_is_private_when_first_published_and_after_replacement() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    let original = br#"{"token":"private-value"}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user_with_fault(
        &config,
        &manifest_path,
        &argv,
        original,
        InstallFault::AfterManifest,
        || {
            assert_eq!(
                fs::metadata(&manifest_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        },
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&manifest_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn damaged_installation_baseline_cannot_erase_unrelated_settings_or_publish_invalid_json() {
    for damaged in [b"{}".as_slice(), b"not json".as_slice()] {
        let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
        let scope = dir.join(".claude");
        fs::create_dir_all(&scope).unwrap();
        let config = scope.join("settings.json");
        let manifest_path = dir.join("manifest.json");
        let original = br#"{"unrelated":"keep"}"#;
        fs::write(&config, original).unwrap();
        let mut manifest =
            install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], original).unwrap();
        manifest.installation_base_bytes = damaged.to_vec();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        remove_claude_user(&config, &manifest_path).unwrap();
        let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(after["unrelated"], "keep");
        assert!(!manifest_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

fn pinned() -> crate::harness::codex::InstalledVersion {
    crate::harness::codex::InstalledVersion::pinned_for_test()
}

fn claude_scope() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest_path = dir.join("manifest.json");
    (dir, config, manifest_path)
}

fn with_command(group: &Value, command: &str) -> Value {
    let mut group = group.clone();
    group["hooks"][0]["command"] = json!(command);
    group
}

/// Rewrites a current installation into the pre-fix shape: only the Bash hook is owned.
fn downgrade_to_bash_only(config: &std::path::Path, manifest_path: &std::path::Path) {
    let mut manifest: OwnershipManifest =
        serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    let session = manifest
        .owned
        .iter()
        .find(|entry| entry.event == "SessionStart")
        .unwrap()
        .clone();
    let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    settings["hooks"]["SessionStart"]
        .as_array_mut()
        .unwrap()
        .retain(|group| group != &session.group);
    if settings["hooks"]["SessionStart"]
        .as_array()
        .unwrap()
        .is_empty()
    {
        settings["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("SessionStart");
    }
    let bytes = serde_json::to_vec(&settings).unwrap();
    fs::write(config, &bytes).unwrap();
    manifest.owned.retain(|entry| entry.event == "PreToolUse");
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn project_setup_installs_inspects_and_removes_every_declared_claude_hook() {
    let (dir, config, manifest_path) = claude_scope();
    let original = br#"{"permissions":{"deny":["Bash(rm *)"]},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"user-start"}]}],"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"user-read"}]}]}}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/space path/herdr-threads".into(), "hook".into()];
    let manifest = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    let declared = crate::harness::claude::declared_hooks_for_argv(&argv).unwrap();
    let declared = declared.as_object().unwrap();
    let marked = format!(
        "{} # herdr-threads-owner:{}",
        shell_command(&argv).unwrap(),
        manifest.installation_id
    );
    let owned_events: Vec<_> = manifest.owned.iter().map(|e| e.event.as_str()).collect();
    let mut declared_events: Vec<_> = declared.keys().map(String::as_str).collect();
    declared_events.sort();
    let mut sorted_owned = owned_events.clone();
    sorted_owned.sort();
    assert_eq!(sorted_owned, declared_events);
    let installed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    for (event, groups) in declared {
        let expected = with_command(&groups[0], &event_command(&marked, event));
        assert!(
            installed["hooks"][event]
                .as_array()
                .unwrap()
                .contains(&expected),
            "{event} not installed"
        );
        assert_eq!(
            manifest
                .owned
                .iter()
                .find(|entry| &entry.event == event)
                .unwrap()
                .group,
            expected
        );
    }
    assert_eq!(
        installed["hooks"]["SessionStart"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        installed["hooks"]["PreToolUse"].as_array().unwrap().len(),
        2
    );
    assert!(
        inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    let before_retry = fs::read(&config).unwrap();
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    assert_eq!(fs::read(&config).unwrap(), before_retry);
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(fs::read(&config).unwrap(), original);
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn inspection_requires_session_start_and_removal_refuses_its_edit() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    let installed = fs::read(&config).unwrap();
    let mut value: Value = serde_json::from_slice(&installed).unwrap();
    value["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(99);
    let edited = serde_json::to_vec(&value).unwrap();
    fs::write(&config, &edited).unwrap();
    assert!(
        !inspect_claude_user(&config, &manifest_path, NativeObservation::Observed)
            .unwrap()
            .installed
    );
    assert_eq!(
        remove_claude_user(&config, &manifest_path),
        Err(SetupError::Conflict)
    );
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), edited);
    assert!(manifest_path.exists());
    value["hooks"]["SessionStart"] = json!([]);
    fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        !inspect_claude_user(&config, &manifest_path, NativeObservation::Observed)
            .unwrap()
            .installed
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bash_only_installation_is_not_current_and_converges_on_resetup() {
    let (dir, config, manifest_path) = claude_scope();
    let original = br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user"}]}]}}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    assert!(
        !inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    let mut legacy: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    legacy["permissions"]["deny"] = json!(["Bash(rm *)"]);
    fs::write(&config, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let upgraded = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    assert_eq!(upgraded.owned, first.owned);
    assert_eq!(upgraded.installation_id, first.installation_id);
    assert_eq!(upgraded.phase, InstallPhase::Installed);
    let status = inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
    assert!(status.installed);
    let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(now["permissions"]["deny"], json!(["Bash(rm *)"]));
    assert_eq!(now["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    assert_eq!(now["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    let settled = fs::read(&config).unwrap();
    let settled_manifest = fs::read(&manifest_path).unwrap();
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    assert_eq!(fs::read(&config).unwrap(), settled);
    assert_eq!(fs::read(&manifest_path).unwrap(), settled_manifest);
    remove_claude_user(&config, &manifest_path).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["permissions"]["deny"], json!(["Bash(rm *)"]));
    assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["command"], "user");
    assert!(
        !String::from_utf8(fs::read(&config).unwrap())
            .unwrap()
            .contains("/tmp/owned")
    );
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bash_only_installation_removes_cleanly_without_resetup() {
    let (dir, config, manifest_path) = claude_scope();
    let original = br#"{"other":1}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    let mut legacy: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    legacy["hooks"]["Stop"] = json!([{"hooks":[{"type":"command","command":"user"}]}]);
    fs::write(&config, serde_json::to_vec(&legacy).unwrap()).unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["other"], 1);
    assert_eq!(after["hooks"]["Stop"][0]["hooks"][0]["command"], "user");
    assert!(
        !String::from_utf8(fs::read(&config).unwrap())
            .unwrap()
            .contains("/tmp/owned")
    );
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bash_only_upgrade_refuses_edited_owned_bash_hook() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"] = json!(99);
    let edited = serde_json::to_vec(&value).unwrap();
    fs::write(&config, &edited).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), edited);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
    fs::remove_dir_all(dir).unwrap();
}

fn read_manifest_file(manifest_path: &std::path::Path) -> OwnershipManifest {
    serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap()
}

fn group_fingerprint(group: &Value) -> String {
    format!(
        "sha256:{:x}",
        sha2::Sha256::digest(serde_json::to_vec(group).unwrap())
    )
}

/// Models an installation made under an earlier declaration: every owned group is rewritten in
/// place by `drift`, with consistent fingerprints, marker and installed fingerprint.
fn drift_installation(
    config: &std::path::Path,
    manifest_path: &std::path::Path,
    drift: &dyn Fn(&str, &mut Value),
) {
    let mut manifest = read_manifest_file(manifest_path);
    let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    for entry in &mut manifest.owned {
        let mut group = entry.group.clone();
        drift(&entry.event, &mut group);
        for slot in settings["hooks"][&entry.event].as_array_mut().unwrap() {
            if slot == &entry.group {
                *slot = group.clone();
            }
        }
        entry.fingerprint = group_fingerprint(&group);
        entry.group = group;
    }
    let bytes = serde_json::to_vec(&settings).unwrap();
    fs::write(config, &bytes).unwrap();
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

type Drift = Box<dyn Fn(&str, &mut Value)>;

fn declaration_drifts() -> Vec<(&'static str, Drift)> {
    vec![
        (
            "timeout",
            Box::new(|_: &str, group: &mut Value| group["hooks"][0]["timeout"] = json!(5)),
        ),
        (
            "matcher",
            Box::new(|event: &str, group: &mut Value| {
                if event == "SessionStart" {
                    group["matcher"] = json!("startup|resume");
                }
            }),
        ),
    ]
}

// Kills: gating removal on the current declaration (adding `|| !is_current_declaration(&manifest.owned)`
// to the `recorded_ownership_valid` gate in `remove_claude_project`).
#[test]
fn removal_after_declaration_drift_removes_exactly_the_recorded_groups() {
    for (name, drift) in declaration_drifts() {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"permissions":{"allow":["Read"]},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"user-start"}]}],"PreToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"user-read"}]}]}}"#;
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        drift_installation(&config, &manifest_path, &drift);
        assert!(
            !inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed,
            "{name}: a drifted installation is not the current declaration"
        );
        assert_eq!(
            remove_claude_user(&config, &manifest_path),
            Ok(()),
            "{name}: removal must follow the manifest's recorded entries"
        );
        assert_eq!(fs::read(&config).unwrap(), original, "{name}");
        assert!(!manifest_path.exists(), "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: dropping `uninstall_json` exactness from drifted removal (an edited recorded group removed).
#[test]
fn removal_after_declaration_drift_still_refuses_an_edited_recorded_group() {
    for (name, drift) in declaration_drifts() {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, b"{}").unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
        drift_installation(&config, &manifest_path, &drift);
        let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        value["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(77);
        let edited = serde_json::to_vec(&value).unwrap();
        fs::write(&config, &edited).unwrap();
        let manifest_before = fs::read(&manifest_path).unwrap();
        assert_eq!(
            remove_claude_user(&config, &manifest_path),
            Err(SetupError::Conflict),
            "{name}"
        );
        assert_eq!(
            install_claude_user(&config, &manifest_path, &argv, b"{}"),
            Err(SetupError::Conflict),
            "{name}"
        );
        assert_eq!(fs::read(&config).unwrap(), edited, "{name}");
        assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before, "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: re-setup refusing (or ignoring) a manifest recorded under an earlier declaration shape.
#[test]
fn resetup_after_declaration_drift_upgrades_to_the_current_declaration() {
    for (name, drift) in declaration_drifts() {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user"}]}]}}"#;
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        drift_installation(&config, &manifest_path, &drift);
        let upgraded = install_claude_user(&config, &manifest_path, &argv, original)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(upgraded.owned, first.owned, "{name}");
        assert_eq!(upgraded.phase, InstallPhase::Installed, "{name}");
        assert!(
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed,
            "{name}"
        );
        let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(now["hooks"]["SessionStart"], json!([first.owned[0].group]));
        assert_eq!(now["hooks"]["PreToolUse"], json!([first.owned[1].group]));
        let settled = fs::read(&config).unwrap();
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(fs::read(&config).unwrap(), settled, "{name}");
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), original, "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: dropping each recorded-entry clause (unique events, one hook per group, fingerprint) from
// removal validation. The groups are absent, so prepared removal would otherwise succeed.
#[test]
fn removal_refuses_malformed_recorded_entries() {
    type Damage = Box<dyn Fn(&mut Vec<OwnedEntry>)>;
    let cases: Vec<(&str, Damage)> = vec![
        (
            "duplicate event",
            Box::new(|owned: &mut Vec<OwnedEntry>| {
                let mut copy = owned[1].clone();
                copy.group["hooks"][0]["timeout"] = json!(3);
                copy.fingerprint = group_fingerprint(&copy.group);
                owned.push(copy);
            }),
        ),
        (
            "two hooks in one group",
            Box::new(|owned: &mut Vec<OwnedEntry>| {
                let hook = owned[0].group["hooks"][0].clone();
                owned[0].group["hooks"].as_array_mut().unwrap().push(hook);
                owned[0].fingerprint = group_fingerprint(&owned[0].group);
            }),
        ),
        (
            "fingerprint",
            Box::new(|owned: &mut Vec<OwnedEntry>| {
                owned[0].fingerprint = format!("sha256:{}", "0".repeat(64));
            }),
        ),
    ];
    for (name, damage) in cases {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, b"{}").unwrap();
        let argv = vec!["/tmp/owned".into()];
        assert_eq!(
            install_claude_user_with_fault(
                &config,
                &manifest_path,
                &argv,
                b"{}",
                InstallFault::AfterManifest,
                || fs::write(&config, br#"{"other":1}"#).unwrap()
            ),
            Err(SetupError::Conflict)
        );
        let mut manifest = read_manifest_file(&manifest_path);
        damage(&mut manifest.owned);
        let damaged = serde_json::to_vec(&manifest).unwrap();
        fs::write(&manifest_path, &damaged).unwrap();
        assert_eq!(
            remove_claude_user(&config, &manifest_path),
            Err(SetupError::Conflict),
            "{name}"
        );
        assert_eq!(fs::read(&config).unwrap(), br#"{"other":1}"#, "{name}");
        assert_eq!(fs::read(&manifest_path).unwrap(), damaged, "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: replacing a legacy install's base with the structurally stripped current bytes on upgrade.
#[test]
fn bash_only_upgrade_keeps_legacy_base_bytes_when_unchanged_since_install() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(fs::read(&config).unwrap(), b"{}");
    fs::remove_dir_all(dir).unwrap();
}

// Kills: publishing the upgrade without first recording the Prepared full-declaration intent, and a
// resume that loses a later unrelated edit or the byte-exact base. Driven through the real upgrade.
#[test]
fn interrupted_upgrade_resumes_from_each_real_publication_boundary() {
    for fault in [
        InstallFault::InterruptAfterIntent,
        InstallFault::InterruptAfterPublication,
    ] {
        for later_edit in [false, true] {
            let (dir, config, manifest_path) = claude_scope();
            let original = br#"{"other":1}"#;
            fs::write(&config, original).unwrap();
            let argv = vec!["/tmp/owned".into()];
            let full = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            downgrade_to_bash_only(&config, &manifest_path);
            let legacy = fs::read(&config).unwrap();
            let interrupted = install_claude_user_with_fault(
                &config,
                &manifest_path,
                &argv,
                original,
                fault,
                || {
                    if later_edit {
                        let mut v: Value =
                            serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                        v["later"] = json!("keep");
                        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
                    }
                },
            );
            assert_eq!(interrupted, Err(SetupError::Io), "{fault:?}");
            let intent = read_manifest_file(&manifest_path);
            assert_eq!(intent.phase, InstallPhase::Prepared, "{fault:?}");
            assert_eq!(intent.owned, full.owned, "{fault:?}");
            let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
            let published = fault == InstallFault::InterruptAfterPublication;
            assert_eq!(
                now["hooks"]["SessionStart"].is_array(),
                published,
                "{fault:?}"
            );
            if !published && !later_edit {
                assert_eq!(fs::read(&config).unwrap(), legacy);
            }
            let resumed = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            assert_eq!(resumed.phase, InstallPhase::Installed);
            assert_eq!(resumed.owned, full.owned);
            assert!(
                inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                    .unwrap()
                    .installed
            );
            let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
            assert_eq!(now["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
            assert_eq!(now["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
            remove_claude_user(&config, &manifest_path).unwrap();
            let after = fs::read(&config).unwrap();
            if later_edit {
                let after: Value = serde_json::from_slice(&after).unwrap();
                assert_eq!(after["other"], 1);
                assert_eq!(after["later"], "keep");
                assert!(!after.to_string().contains("/tmp/owned"));
            } else {
                assert_eq!(after, original, "{fault:?}");
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

// Kills: prepared removal that ignores recorded groups missing from settings, or that removes by
// event name rather than exact recorded group. Driven from the real intent boundary.
#[test]
fn prepared_removal_after_interrupted_upgrade_removes_only_present_exact_groups() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, br#"{"other":1}"#).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, br#"{"other":1}"#).unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            br#"{"other":1}"#,
            InstallFault::InterruptAfterIntent,
            || {}
        ),
        Err(SetupError::Io)
    );
    assert_eq!(
        read_manifest_file(&manifest_path).phase,
        InstallPhase::Prepared
    );
    remove_claude_user(&config, &manifest_path).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["other"], 1);
    assert!(!after.to_string().contains("/tmp/owned"));
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

// Kills: recording upgrade intent before proving the upgrade target composes without conflict.
#[test]
fn upgrade_refuses_unmarked_session_start_added_after_bash_only_install_without_intent() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    let unmarked = crate::harness::claude::declared_hooks_for_argv(&argv).unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    value["hooks"]["SessionStart"] = unmarked["SessionStart"].clone();
    let independent = serde_json::to_vec(&value).unwrap();
    fs::write(&config, &independent).unwrap();
    let manifest_before = fs::read(&manifest_path).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), independent);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
    remove_claude_user(&config, &manifest_path).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["hooks"]["SessionStart"], unmarked["SessionStart"]);
    assert_eq!(after["hooks"]["PreToolUse"], json!([]));
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepared_retry_refuses_independent_unmarked_session_start_hook() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            b"{}",
            InstallFault::AfterManifest,
            || fs::write(&config, br#"{"other":1}"#).unwrap()
        ),
        Err(SetupError::Conflict)
    );
    let unmarked = crate::harness::claude::declared_hooks_for_argv(&argv).unwrap();
    let independent = json!({"other":1,"hooks":{"SessionStart":unmarked["SessionStart"].clone()}});
    let independent = serde_json::to_vec(&independent).unwrap();
    fs::write(&config, &independent).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), independent);
    fs::remove_dir_all(dir).unwrap();
}

// Kills: prepared removal or resumption that ignores the superseded groups recorded by a drifted
// upgrade intent. Driven through the real upgrade boundaries for each drift variant.
#[test]
fn interrupted_drift_upgrade_resumes_or_removes_from_each_real_boundary() {
    for (name, drift) in declaration_drifts() {
        for fault in [
            InstallFault::InterruptAfterIntent,
            InstallFault::InterruptAfterPublication,
        ] {
            for resume in [false, true] {
                let (dir, config, manifest_path) = claude_scope();
                let original = br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"user-start"}]}]},"other":1}"#;
                fs::write(&config, original).unwrap();
                let argv = vec!["/tmp/owned".into()];
                let full = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
                drift_installation(&config, &manifest_path, &drift);
                assert_eq!(
                    install_claude_user_with_fault(
                        &config,
                        &manifest_path,
                        &argv,
                        original,
                        fault,
                        || {}
                    ),
                    Err(SetupError::Io),
                    "{name} {fault:?}"
                );
                let intent = read_manifest_file(&manifest_path);
                assert_eq!(intent.phase, InstallPhase::Prepared, "{name} {fault:?}");
                assert!(!intent.superseded.is_empty(), "{name} {fault:?}");
                if resume {
                    let resumed =
                        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
                    assert_eq!(resumed.owned, full.owned, "{name} {fault:?}");
                    assert!(resumed.superseded.is_empty());
                    let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                    assert_eq!(
                        now["hooks"]["PreToolUse"],
                        json!([full.owned[1].group]),
                        "{name} {fault:?}"
                    );
                    assert!(
                        inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                            .unwrap()
                            .installed
                    );
                }
                remove_claude_user(&config, &manifest_path)
                    .unwrap_or_else(|e| panic!("{name} {fault:?} resume={resume}: {e:?}"));
                let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                let original: Value = serde_json::from_slice(original).unwrap();
                let mut expected = original.clone();
                if after["hooks"]["PreToolUse"] == json!([]) {
                    expected["hooks"]["PreToolUse"] = json!([]);
                }
                assert_eq!(after, expected, "{name} {fault:?} resume={resume}");
                assert!(!manifest_path.exists());
                fs::remove_dir_all(dir).unwrap();
            }
        }
    }
}

/// Marked hook group for an event the current declaration no longer owns (an earlier declaration
/// that also installed a `Stop` hook), with the installation's own marked command.
fn dropped_stop_entry(manifest: &OwnershipManifest) -> OwnedEntry {
    let first = &manifest.owned[0];
    let base = first.group["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .replace(&format!(" '--event' '{}'", first.event), "");
    let command = event_command(&base, "Stop");
    let group = json!({"hooks":[{"type":"command","command":command,"timeout":10}]});
    OwnedEntry {
        event: "Stop".into(),
        fingerprint: group_fingerprint(&group),
        group,
    }
}

/// Models an installation made under an earlier declaration that also owned a `Stop` group: the
/// group is appended to settings and recorded, with a consistent installed fingerprint.
fn add_dropped_stop(config: &std::path::Path, manifest_path: &std::path::Path) -> OwnedEntry {
    let mut manifest = read_manifest_file(manifest_path);
    let entry = dropped_stop_entry(&manifest);
    let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    let hooks = settings["hooks"].as_object_mut().unwrap();
    hooks
        .entry("Stop")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap()
        .push(entry.group.clone());
    let bytes = serde_json::to_vec(&settings).unwrap();
    fs::write(config, &bytes).unwrap();
    manifest.owned.push(entry.clone());
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    entry
}

fn owned_command_present(config: &std::path::Path) -> bool {
    String::from_utf8(fs::read(config).unwrap())
        .unwrap()
        .contains("/tmp/owned")
}

const USER_STOP: &[u8] =
    br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user"}]}]},"other":1}"#;

// Kills: (a) an upgrade that leaves the dropped event's exact recorded group installed; (b) N1,
// publishing the strip's emptied `"Stop":[]` when the file is unchanged since the recorded base
// (restoring `compose_json(base, owned)` is replaced by strip-then-compose), which makes the later
// Installed removal fall back to a structural strip instead of the byte-exact original.
#[test]
fn dropped_event_upgrade_removes_exact_group_and_restores_byte_exact() {
    for original in [&br#"{"other":1}"#[..], USER_STOP] {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        let current_shape = fs::read(&config).unwrap();
        add_dropped_stop(&config, &manifest_path);
        assert!(
            !inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed
        );
        let upgraded = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(upgraded.owned, first.owned);
        assert_eq!(upgraded.phase, InstallPhase::Installed);
        assert!(upgraded.superseded.is_empty());
        assert_eq!(
            String::from_utf8(fs::read(&config).unwrap()).unwrap(),
            String::from_utf8(current_shape.clone()).unwrap(),
            "the upgrade publishes exactly base + current declaration"
        );
        assert!(
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed
        );
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(
            String::from_utf8(fs::read(&config).unwrap()).unwrap(),
            String::from_utf8(original.to_vec()).unwrap()
        );
        assert!(!manifest_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: publishing the strip's emptied `"Stop":[]` when the file changed after the recorded base
// (the not-kept path): an event array created only for the dropped hook must not survive, while a
// user's own `Stop` group, and a later unrelated edit, do.
#[test]
fn dropped_event_upgrade_after_later_edit_keeps_user_state_and_no_empty_dropped_event() {
    for original in [&br#"{"other":1}"#[..], USER_STOP] {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        add_dropped_stop(&config, &manifest_path);
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v["later"] = json!("keep");
        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        let user: Value = serde_json::from_slice(original).unwrap();
        assert_eq!(now["hooks"].get("Stop"), user["hooks"].get("Stop"));
        assert_eq!(now["later"], "keep");
        remove_claude_user(&config, &manifest_path).unwrap();
        let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(after["other"], 1);
        assert_eq!(after["later"], "keep");
        assert_eq!(after["hooks"].get("Stop"), user["hooks"].get("Stop"));
        assert!(!owned_command_present(&config));
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills MY (disabling the residual guard in `plan_resume` for a superseded event the target no
// longer owns) and MU (skipping the `leftover` check before the no-publish early return): an
// interrupted upgrade whose dropped `Stop` group is edited after the intent must conflict and stay
// Prepared rather than finalize `Installed` over a stray owned-command hook. The unedited control
// resumes and removes the dropped group.
#[test]
fn interrupted_dropped_event_upgrade_refuses_edited_leftover_and_resumes_exact() {
    for edit in [false, true] {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"other":1}"#;
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        add_dropped_stop(&config, &manifest_path);
        assert_eq!(
            install_claude_user_with_fault(
                &config,
                &manifest_path,
                &argv,
                original,
                InstallFault::InterruptAfterIntent,
                || {
                    if edit {
                        let mut v: Value =
                            serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                        v["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(77);
                        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
                    }
                }
            ),
            Err(SetupError::Io)
        );
        let intent = read_manifest_file(&manifest_path);
        assert_eq!(intent.phase, InstallPhase::Prepared);
        assert_eq!(intent.owned, first.owned);
        assert_eq!(
            intent
                .superseded
                .iter()
                .map(|e| e.event.as_str())
                .collect::<Vec<_>>(),
            ["Stop"]
        );
        if edit {
            let settings_before = fs::read(&config).unwrap();
            let manifest_before = fs::read(&manifest_path).unwrap();
            assert_eq!(
                install_claude_user(&config, &manifest_path, &argv, original),
                Err(SetupError::Conflict)
            );
            assert_eq!(fs::read(&config).unwrap(), settings_before);
            assert_eq!(fs::read(&manifest_path).unwrap(), manifest_before);
            assert_eq!(
                read_manifest_file(&manifest_path).phase,
                InstallPhase::Prepared
            );
            assert_eq!(
                remove_claude_user(&config, &manifest_path),
                Err(SetupError::Conflict)
            );
            assert_eq!(fs::read(&config).unwrap(), settings_before);
        } else {
            let resumed = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            assert_eq!(resumed.phase, InstallPhase::Installed);
            let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
            assert!(now["hooks"].get("Stop").is_none());
            remove_claude_user(&config, &manifest_path).unwrap();
            assert_eq!(fs::read(&config).unwrap(), original);
        }
        fs::remove_dir_all(dir).unwrap();
    }
}

/// Interrupts a fresh install after its Prepared manifest is durable, leaving `{"other":1,...}`.
fn prepared_fresh_install(
    config: &std::path::Path,
    manifest_path: &std::path::Path,
    argv: &[String],
    original: &[u8],
) {
    fs::write(config, original).unwrap();
    assert_eq!(
        install_claude_user_with_fault(
            config,
            manifest_path,
            argv,
            original,
            InstallFault::AfterManifest,
            || fs::write(config, br#"{"other":1,"later":"keep"}"#).unwrap()
        ),
        Err(SetupError::Conflict)
    );
    assert_eq!(
        read_manifest_file(manifest_path).phase,
        InstallPhase::Prepared
    );
}

// Kills MS (dropping `manifest.superseded.push(entry)` in the Prepared branch that re-targets a
// manifest recorded under an earlier declaration): the partially published old-shape groups are
// then neither superseded nor owned, so re-setup conflicts (drifted SessionStart) or finalizes
// over a stray owned `Stop` hook. Also kills leaving an emptied dropped-event array behind.
#[test]
fn prepared_install_under_earlier_declaration_converges_on_resetup() {
    type Old = Box<dyn Fn(&mut OwnershipManifest)>;
    let mut cases: Vec<(&str, Old)> = declaration_drifts()
        .into_iter()
        .map(|(name, drift)| {
            let old: Old = Box::new(move |manifest: &mut OwnershipManifest| {
                for entry in &mut manifest.owned {
                    drift(&entry.event, &mut entry.group);
                    entry.fingerprint = group_fingerprint(&entry.group);
                }
            });
            (name, old)
        })
        .collect();
    cases.push((
        "dropped Stop",
        Box::new(|manifest: &mut OwnershipManifest| {
            let stop = dropped_stop_entry(manifest);
            manifest.owned.push(stop);
        }),
    ));
    for (name, old) in cases {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"other":1}"#;
        let argv = vec!["/tmp/owned".into()];
        prepared_fresh_install(&config, &manifest_path, &argv, original);
        let mut manifest = read_manifest_file(&manifest_path);
        let expected = manifest.owned.clone();
        old(&mut manifest);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        // The earlier declaration's last group was published before the interruption.
        let published = manifest.owned.last().unwrap().clone();
        let mut settings: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        settings["hooks"] = json!({ published.event.clone(): [published.group.clone()] });
        fs::write(&config, serde_json::to_vec(&settings).unwrap()).unwrap();
        let resumed = install_claude_user(&config, &manifest_path, &argv, original)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(resumed.owned, expected, "{name}");
        assert_eq!(resumed.phase, InstallPhase::Installed, "{name}");
        assert!(resumed.superseded.is_empty(), "{name}");
        let now: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            now,
            json!({"other":1,"later":"keep","hooks":{
                "SessionStart":[expected[0].group.clone()],
                "PreToolUse":[expected[1].group.clone()]
            },"permissions":{"allow":[crate::harness::claude::HERDR_THREADS_ALLOW_RULE]}}),
            "{name}"
        );
        assert!(
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed,
            "{name}"
        );
        remove_claude_user(&config, &manifest_path).unwrap();
        assert!(!owned_command_present(&config), "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills MV (dropping the conflict when a Prepared manifest recorded under an earlier declaration
// would supersede an event that `superseded` already holds): the merged intent would record the
// event twice, which recorded-entry validation rejects, so a later interruption would strand an
// unremovable manifest. Both files stay unchanged and removal still works.
#[test]
fn prepared_resetup_refuses_second_supersession_of_the_same_event() {
    let (name, drift) = declaration_drifts().into_iter().next().unwrap();
    let (dir, config, manifest_path) = claude_scope();
    let original = br#"{"other":1}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    drift_installation(&config, &manifest_path, &drift);
    assert_eq!(
        install_claude_user_with_fault(
            &config,
            &manifest_path,
            &argv,
            original,
            InstallFault::InterruptAfterIntent,
            || {}
        ),
        Err(SetupError::Io),
        "{name}"
    );
    // The intent was recorded by a binary whose declaration has since changed again.
    let mut manifest = read_manifest_file(&manifest_path);
    assert_eq!(manifest.superseded.len(), 2);
    for entry in &mut manifest.owned {
        entry.group["hooks"][0]["timeout"] = json!(9);
        entry.fingerprint = group_fingerprint(&entry.group);
    }
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &manifest_bytes).unwrap();
    let settings = fs::read(&config).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, original),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), settings);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    remove_claude_user(&config, &manifest_path).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(after["other"], 1);
    assert!(!owned_command_present(&config));
    fs::remove_dir_all(dir).unwrap();
}

// Kills MZ (dropping the requirement that `superseded` share the owned marked command in
// `recorded_ownership_valid`): a damaged manifest naming a user's own group as superseded would
// let removal or resumption delete that user group.
#[test]
fn superseded_entries_with_a_foreign_command_are_not_owned() {
    let (dir, config, manifest_path) = claude_scope();
    let argv = vec!["/tmp/owned".into()];
    prepared_fresh_install(&config, &manifest_path, &argv, b"{}");
    let user = json!({"hooks":[{"type":"command","command":"user-start"}]});
    let settings = serde_json::to_vec(&json!({"hooks":{"SessionStart":[user.clone()]}})).unwrap();
    fs::write(&config, &settings).unwrap();
    let mut manifest = read_manifest_file(&manifest_path);
    manifest.superseded = vec![OwnedEntry {
        event: "SessionStart".into(),
        fingerprint: group_fingerprint(&user),
        group: user,
    }];
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &manifest_bytes).unwrap();
    assert_eq!(
        remove_claude_user(&config, &manifest_path),
        Err(SetupError::Conflict)
    );
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), settings);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    fs::remove_dir_all(dir).unwrap();
}

// Kills MW (accepting `superseded` outside the Prepared phase in `recorded_ownership_valid`): an
// Installed manifest carrying superseded groups is damaged, so removal, re-setup and inspection
// must not act on it.
#[test]
fn installed_manifest_with_superseded_entries_is_not_owned() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    let mut manifest = read_manifest_file(&manifest_path);
    let mut stale = manifest.owned[0].clone();
    stale.group["hooks"][0]["timeout"] = json!(3);
    stale.fingerprint = group_fingerprint(&stale.group);
    manifest.superseded = vec![stale];
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    fs::write(&manifest_path, &manifest_bytes).unwrap();
    let settings = fs::read(&config).unwrap();
    assert!(
        !inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    assert_eq!(
        remove_claude_user(&config, &manifest_path),
        Err(SetupError::Conflict)
    );
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), settings);
    assert_eq!(fs::read(&manifest_path).unwrap(), manifest_bytes);
    fs::remove_dir_all(dir).unwrap();
}

/// Changed-file upgrade from `original` with a dropped `Stop` group and a later unrelated edit,
/// optionally modelling a legacy manifest (empty recorded base). Returns the published and the
/// removed settings.
fn dropped_stop_changed_file_upgrade(original: &[u8], legacy: bool) -> (Value, Value) {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    add_dropped_stop(&config, &manifest_path);
    if legacy {
        let mut manifest = read_manifest_file(&manifest_path);
        manifest.installation_base_bytes = Vec::new();
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }
    let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    v["later"] = json!("keep");
    fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
    let upgraded = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    assert_eq!(upgraded.phase, InstallPhase::Installed);
    assert!(upgraded.superseded.is_empty());
    let published: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    let removed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert!(!owned_command_present(&config));
    assert!(!manifest_path.exists());
    fs::remove_dir_all(dir).unwrap();
    (published, removed)
}

// Kills BH (`&& !base_held` -> `&& (true || !base_held)` in the emptied-event prune): an empty
// `"Stop":[]` the user's recorded base already held is user state, so emptying it by stripping the
// dropped `Stop` group must not delete the key from the published file or the restored file.
#[test]
fn changed_file_upgrade_keeps_an_empty_event_array_the_recorded_base_held() {
    let (published, removed) =
        dropped_stop_changed_file_upgrade(br#"{"hooks":{"Stop":[]},"other":1}"#, false);
    assert_eq!(published["hooks"].get("Stop"), Some(&json!([])));
    assert_eq!(published["later"], "keep");
    assert_eq!(removed["hooks"].get("Stop"), Some(&json!([])));
    assert_eq!(removed["other"], 1);
    assert_eq!(removed["later"], "keep");
}

// Kills LG (`.is_none_or(..)` -> `.is_some_and(..)` for the recorded base in the emptied-event
// prune): a legacy manifest with no recorded base cannot prove which event arrays the user held,
// so nothing is pruned and the emptied `"Stop":[]` survives both publication and removal.
#[test]
fn legacy_empty_base_changed_file_upgrade_prunes_no_event_array() {
    let (published, removed) = dropped_stop_changed_file_upgrade(br#"{"other":1}"#, true);
    assert_eq!(published["hooks"].get("Stop"), Some(&json!([])));
    assert_eq!(removed["hooks"].get("Stop"), Some(&json!([])));
    assert_eq!(removed["hooks"].get("PreToolUse"), Some(&json!([])));
    assert_eq!(removed["other"], 1);
    assert_eq!(removed["later"], "keep");
}

// Kills OW (pruning only superseded events, i.e. dropping `manifest.owned` from the events the
// emptied-event prune considers) and the former DR scope clause (`if dropped`, which restricted the
// prune to events the target no longer owns): after a changed-file drift upgrade the restoration
// base must not keep an emptied array for any owned event the recorded base lacked, drifted
// (superseded) or not, so removal leaves no `"Event":[]` residue.
#[test]
fn changed_file_drift_upgrade_removal_leaves_no_emptied_owned_event_array() {
    for (name, drift) in declaration_drifts() {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"other":1}"#;
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        drift_installation(&config, &manifest_path, &drift);
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v["later"] = json!("keep");
        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
        let upgraded = install_claude_user(&config, &manifest_path, &argv, original)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(upgraded.owned, first.owned, "{name}");
        let published: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        for entry in &first.owned {
            assert_eq!(
                published["hooks"][&entry.event],
                json!([entry.group]),
                "{name}: {}",
                entry.event
            );
        }
        remove_claude_user(&config, &manifest_path).unwrap();
        let removed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            removed,
            json!({"hooks":{},"later":"keep","other":1}),
            "{name}: every owned event array the base lacked is pruned"
        );
        assert!(!manifest_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills OBH2 (owned events pruned without the base-held check: `if !base_held` ->
// `if (!base_held || (base_hooks.is_some() && manifest.owned.iter().any(|o| o.event == entry.event)))`
// in the emptied-event prune): a non-legacy recorded base that held an empty array for every owned
// event is user state, so after a changed-file drift upgrade removal must restore those `[]`
// arrays rather than prune them as if they existed only for our groups.
#[test]
fn changed_file_drift_upgrade_keeps_empty_owned_event_arrays_the_recorded_base_held() {
    for (name, drift) in declaration_drifts() {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"hooks":{"SessionStart":[],"PreToolUse":[]},"other":1}"#;
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        let mut owned_events: Vec<&str> = first.owned.iter().map(|o| o.event.as_str()).collect();
        owned_events.sort_unstable();
        assert_eq!(owned_events, ["PreToolUse", "SessionStart"], "{name}");
        drift_installation(&config, &manifest_path, &drift);
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v["later"] = json!("keep");
        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
        let upgraded = install_claude_user(&config, &manifest_path, &argv, original)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(upgraded.phase, InstallPhase::Installed, "{name}");
        assert_eq!(upgraded.owned, first.owned, "{name}");
        let published: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        for entry in &first.owned {
            assert_eq!(
                published["hooks"][&entry.event],
                json!([entry.group]),
                "{name}: {}",
                entry.event
            );
        }
        remove_claude_user(&config, &manifest_path).unwrap();
        let removed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            removed,
            json!({"hooks":{"SessionStart":[],"PreToolUse":[]},"later":"keep","other":1}),
            "{name}: owned event arrays the recorded base held survive removal"
        );
        assert!(!owned_command_present(&config), "{name}");
        assert!(!manifest_path.exists(), "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// ---------------------------------------------------------------- owned allow rule

const RULE: &str = crate::harness::claude::HERDR_THREADS_ALLOW_RULE;

fn allow_of(config: &std::path::Path) -> Value {
    let v: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    v["permissions"]["allow"].clone()
}

fn rule_count(config: &std::path::Path) -> usize {
    allow_of(config)
        .as_array()
        .map_or(0, |a| a.iter().filter(|r| r.as_str() == Some(RULE)).count())
}

fn allow_rule(config: &std::path::Path, manifest_path: &std::path::Path) -> AllowRuleInspection {
    inspect_claude_user(config, manifest_path, NativeObservation::Unknown)
        .unwrap()
        .allow_rule
        .unwrap()
}

/// Rewrites a current installation into the shape recorded before the rule was declared:
/// no recorded permission and (unless the user holds it) no rule in settings.
fn strip_recorded_rule(config: &std::path::Path, manifest_path: &std::path::Path) {
    let mut manifest = read_manifest_file(manifest_path);
    let owned_rule = manifest.permission.take().is_some_and(|p| !p.pre_existing);
    if owned_rule {
        let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
        let allow = settings["permissions"]["allow"].as_array_mut().unwrap();
        allow.retain(|r| r.as_str() != Some(RULE));
        if allow.is_empty() {
            settings["permissions"]
                .as_object_mut()
                .unwrap()
                .remove("allow");
        }
        if settings["permissions"].as_object().unwrap().is_empty() {
            settings.as_object_mut().unwrap().remove("permissions");
        }
        let bytes = serde_json::to_vec(&settings).unwrap();
        fs::write(config, &bytes).unwrap();
        manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    }
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

// Kills: not composing the rule on install, removing a pre-existing rule, duplicating it, or
// leaving an owned rule (or an emptied permissions/allow residue) behind on unsetup.
#[test]
fn allow_rule_install_status_remove_round_trip_with_and_without_preexisting_rule() {
    let bases: [(&str, &[u8], bool); 5] = [
        ("empty", b"{}", false),
        ("no permissions", br#"{"other":1}"#, false),
        (
            "other rules",
            br#"{"permissions":{"allow":["Read","Bash(echo *)"],"deny":["Bash(rm *)"]}}"#,
            false,
        ),
        (
            "preexisting",
            br#"{"permissions":{"allow":["Read","Bash(herdr-threads *)"]}}"#,
            true,
        ),
        (
            "preexisting only",
            br#"{"permissions":{"allow":["Bash(herdr-threads *)"]},"x":[]}"#,
            true,
        ),
    ];
    for (name, original, pre_existing) in bases {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let manifest = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        let recorded = manifest.permission.clone().unwrap();
        assert_eq!(recorded.rule, RULE, "{name}");
        assert_eq!(recorded.pre_existing, pre_existing, "{name}");
        assert_eq!(rule_count(&config), 1, "{name}: never duplicated");
        let status =
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
        assert!(status.installed, "{name}");
        let rule = status.allow_rule.unwrap();
        assert!(rule.present, "{name}");
        assert_eq!(
            rule.ownership,
            if pre_existing {
                AllowRuleOwnership::PreExisting
            } else {
                AllowRuleOwnership::Owned
            },
            "{name}"
        );
        // Idempotent re-run.
        let settled = fs::read(&config).unwrap();
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(fs::read(&config).unwrap(), settled, "{name}");
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), original, "{name}: byte-exact");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: structural removal (after an unrelated edit) that keeps the owned rule, removes a
// pre-existing rule, removes a user duplicate, or drops an allow array the base held.
#[test]
fn allow_rule_structural_removal_after_unrelated_edit() {
    let cases: [(&str, &[u8]); 4] = [
        ("absent", br#"{"other":1}"#),
        ("empty allow", br#"{"permissions":{"allow":[]}}"#),
        ("empty permissions", br#"{"permissions":{}}"#),
        (
            "preexisting",
            br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#,
        ),
    ];
    for (name, original) in cases {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v["later"] = json!("keep");
        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
        remove_claude_user(&config, &manifest_path).unwrap();
        let mut expected: Value = serde_json::from_slice(original).unwrap();
        expected["later"] = json!("keep");
        assert!(!owned_command_present(&config), "{name}");
        // Structural hook removal may leave emptied event arrays (existing behaviour); the
        // assertion here is about everything outside `hooks`.
        let mut after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        after.as_object_mut().unwrap().remove("hooks");
        assert_eq!(after, expected, "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
    // A user duplicate added after install survives: only the owned occurrence is removed.
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], b"{}").unwrap();
    let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    v["permissions"]["allow"]
        .as_array_mut()
        .unwrap()
        .push(json!(RULE));
    fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(rule_count(&config), 1);
    fs::remove_dir_all(dir).unwrap();
}

// Kills: silently re-adding (or ignoring) a recorded rule the user removed, and removal refusing
// when the owned rule is already gone.
#[test]
fn removed_allow_rule_is_reported_and_resetup_refuses_but_unsetup_succeeds() {
    for original in [
        &br#"{"other":1}"#[..],
        &br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#[..],
    ] {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v["permissions"]["allow"]
            .as_array_mut()
            .unwrap()
            .retain(|r| r.as_str() != Some(RULE));
        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
        let edited = fs::read(&config).unwrap();
        let status =
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
        assert!(!status.installed);
        assert!(!status.allow_rule.unwrap().present);
        assert_eq!(
            install_claude_user(&config, &manifest_path, &argv, original),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), edited);
        remove_claude_user(&config, &manifest_path).unwrap();
        assert!(!owned_command_present(&config));
        assert_eq!(rule_count(&config), 0);
        assert!(!manifest_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: accepting a manifest-recorded rule other than the declared one (a forged manifest could
// otherwise direct unsetup to delete an arbitrary user rule) or a rule with a stale fingerprint.
#[test]
fn recorded_permission_must_be_the_declared_rule_with_its_fingerprint() {
    for forge in [
        |p: &mut OwnedPermission| {
            // A self-consistent record of a different (user) rule.
            p.rule = "Read".into();
            p.fingerprint = format!(
                "sha256:{:x}",
                sha2::Sha256::digest(serde_json::to_vec(&json!({"allow":"Read"})).unwrap())
            );
        },
        |p: &mut OwnedPermission| p.fingerprint = "sha256:00".into(),
    ] {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"permissions":{"allow":["Read"]}}"#;
        fs::write(&config, original).unwrap();
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], original).unwrap();
        let mut manifest = read_manifest_file(&manifest_path);
        forge(manifest.permission.as_mut().unwrap());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let before = fs::read(&config).unwrap();
        assert!(
            !inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed
        );
        assert_eq!(
            remove_claude_user(&config, &manifest_path),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: an installation recorded before the rule was declared counting as installed, re-setup
// not adding the rule, claiming a user's rule as owned, or losing byte-exact restoration.
#[test]
fn installation_without_recorded_rule_upgrades_on_resetup() {
    for (name, original) in [
        ("absent", &br#"{"other":1}"#[..]),
        (
            "user holds it",
            &br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#[..],
        ),
    ] {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let first = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        strip_recorded_rule(&config, &manifest_path);
        let status =
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
        assert!(!status.installed, "{name}");
        assert_eq!(
            status.allow_rule.unwrap().ownership,
            AllowRuleOwnership::NotRecorded,
            "{name}"
        );
        let upgraded = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(upgraded.permission, first.permission, "{name}");
        assert_eq!(upgraded.phase, InstallPhase::Installed, "{name}");
        assert_eq!(rule_count(&config), 1, "{name}");
        assert!(allow_rule(&config, &manifest_path).present, "{name}");
        assert!(
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed,
            "{name}"
        );
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), original, "{name}: byte-exact");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: a crash between recording the rule intent and publishing it (or between publishing and
// finalizing) that strands the rule, loses it, or breaks byte-exact removal.
#[test]
fn interrupted_rule_upgrade_resumes_or_removes_from_each_real_boundary() {
    for fault in [
        InstallFault::InterruptAfterIntent,
        InstallFault::InterruptAfterPublication,
    ] {
        for resume in [true, false] {
            let (dir, config, manifest_path) = claude_scope();
            let original = br#"{"permissions":{"deny":["Bash(rm *)"]}}"#;
            fs::write(&config, original).unwrap();
            let argv = vec!["/tmp/owned".into()];
            install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            strip_recorded_rule(&config, &manifest_path);
            let interrupted = install_claude_user_with_fault(
                &config,
                &manifest_path,
                &argv,
                original,
                fault,
                || {},
            );
            assert_eq!(interrupted, Err(SetupError::Io), "{fault:?}");
            let intent = read_manifest_file(&manifest_path);
            assert_eq!(intent.phase, InstallPhase::Prepared, "{fault:?}");
            assert!(intent.permission.is_some_and(|p| !p.pre_existing));
            assert_eq!(
                rule_count(&config),
                usize::from(fault == InstallFault::InterruptAfterPublication),
                "{fault:?}"
            );
            if resume {
                let resumed =
                    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
                assert_eq!(resumed.phase, InstallPhase::Installed);
                assert_eq!(rule_count(&config), 1);
                assert!(
                    inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                        .unwrap()
                        .installed
                );
            }
            remove_claude_user(&config, &manifest_path).unwrap();
            assert!(!owned_command_present(&config));
            assert_eq!(rule_count(&config), 0, "{fault:?} resume={resume}");
            if resume {
                assert_eq!(fs::read(&config).unwrap(), original, "{fault:?}");
            } else {
                // Prepared removal is structural: only emptied hook arrays may remain.
                let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                assert_eq!(after["permissions"], json!({"deny":["Bash(rm *)"]}));
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

// Kills: a fresh install interrupted at a real boundary that, on retry, publishes without the
// rule or duplicates a pre-existing one, and prepared removal that leaves a published owned rule
// or removes a pre-existing one.
#[test]
fn interrupted_fresh_install_publishes_or_removes_the_rule() {
    for pre_existing in [false, true] {
        let original: &[u8] = if pre_existing {
            br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#
        } else {
            br#"{"other":1}"#
        };
        let argv: Vec<String> = vec!["/tmp/owned".into()];
        // Boundary 1: manifest prepared, settings changed concurrently before publication.
        for resume in [true, false] {
            let (dir, config, manifest_path) = claude_scope();
            fs::write(&config, original).unwrap();
            let mut edited: Value = serde_json::from_slice(original).unwrap();
            edited["later"] = json!("keep");
            let edited = serde_json::to_vec(&edited).unwrap();
            assert_eq!(
                install_claude_user_with_fault(
                    &config,
                    &manifest_path,
                    &argv,
                    original,
                    InstallFault::AfterManifest,
                    || fs::write(&config, &edited).unwrap()
                ),
                Err(SetupError::Conflict)
            );
            let prepared = read_manifest_file(&manifest_path);
            assert_eq!(prepared.phase, InstallPhase::Prepared);
            assert_eq!(
                prepared.permission.as_ref().unwrap().pre_existing,
                pre_existing
            );
            assert_eq!(fs::read(&config).unwrap(), edited);
            if resume {
                install_claude_user(&config, &manifest_path, &argv, original).unwrap();
                assert_eq!(rule_count(&config), 1, "pre_existing={pre_existing}");
                assert!(
                    inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                        .unwrap()
                        .installed
                );
            }
            remove_claude_user(&config, &manifest_path).unwrap();
            assert_eq!(
                fs::read(&config).unwrap(),
                edited,
                "pre_existing={pre_existing} resume={resume}"
            );
            fs::remove_dir_all(dir).unwrap();
        }
        // Boundary 2: settings published, manifest not yet finalized.
        for resume in [true, false] {
            let (dir, config, manifest_path) = claude_scope();
            fs::write(&config, original).unwrap();
            install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            let mut manifest = read_manifest_file(&manifest_path);
            manifest.phase = InstallPhase::Prepared;
            fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let published = fs::read(&config).unwrap();
            if resume {
                install_claude_user(&config, &manifest_path, &argv, original).unwrap();
                assert_eq!(fs::read(&config).unwrap(), published);
                assert_eq!(
                    read_manifest_file(&manifest_path).phase,
                    InstallPhase::Installed
                );
            }
            remove_claude_user(&config, &manifest_path).unwrap();
            assert_eq!(rule_count(&config), usize::from(pre_existing));
            assert!(!owned_command_present(&config));
            if resume {
                assert_eq!(fs::read(&config).unwrap(), original);
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

// The retired rule (still recognized in old manifests) matches exactly the export prefix the
// unused `updatedInput` encoder emits; the installed rule is the herdr-threads command rule.
#[test]
fn retired_rule_is_the_adapter_rewrite_prefix_and_installed_rule_is_the_cli() {
    use crate::harness::claude;
    assert_eq!(RULE, "Bash(herdr-threads *)");
    assert_eq!(
        claude::CALLER_CONTEXT_ALLOW_RULE,
        format!("Bash({}*)", claude::CALLER_CONTEXT_EXPORT_PREFIX)
    );
    let input = br#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_use_id":"t","session_id":"s","tool_input":{"command":"echo hi"}}"#;
    let out = claude::encode_tool_response(input, "2.1.283", "ctx_Ab-19", "", 4096).unwrap();
    let v: Value = serde_json::from_slice(&out).unwrap();
    let command = v["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap();
    let first = command.split(";\n").next().unwrap();
    assert_eq!(first, "export HERDR_THREADS_CALLER_CONTEXT='ctx_Ab-19'");
    assert!(first.starts_with(claude::CALLER_CONTEXT_EXPORT_PREFIX));
}

// Kills: composing the owned rule again when it is already present (a hook-declaration upgrade
// over a file that holds the published rule would duplicate it).
#[test]
fn hook_upgrade_over_published_rule_does_not_duplicate_it() {
    let (dir, config, manifest_path) = claude_scope();
    let original = br#"{"other":1}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    downgrade_to_bash_only(&config, &manifest_path);
    let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    v["later"] = json!("keep");
    fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    assert_eq!(rule_count(&config), 1);
    assert!(
        inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(rule_count(&config), 0);
    assert!(!owned_command_present(&config));
    fs::remove_dir_all(dir).unwrap();
}

// ---------------------------------------------------------------- retired export rule upgrade

const RETIRED: &str = crate::harness::claude::CALLER_CONTEXT_ALLOW_RULE;

fn count_of(config: &std::path::Path, rule: &str) -> usize {
    allow_of(config)
        .as_array()
        .map_or(0, |a| a.iter().filter(|r| r.as_str() == Some(rule)).count())
}

fn retired_record(pre_existing: bool) -> OwnedPermission {
    OwnedPermission {
        rule: RETIRED.into(),
        fingerprint: format!(
            "sha256:{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&json!({ "allow": RETIRED })).unwrap())
        ),
        pre_existing,
    }
}

/// Rewrites a current installation into exactly what the previous setup wrote: the manifest
/// records the retired export rule (`retired_pre_existing`: the user's own) and the settings hold
/// the retired rule where setup appended it, instead of an owned declared rule.
fn record_as_retired(
    config: &std::path::Path,
    manifest_path: &std::path::Path,
    retired_pre_existing: bool,
) {
    let mut manifest = read_manifest_file(manifest_path);
    let declared_owned = !manifest.permission.as_ref().unwrap().pre_existing;
    let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    let allow = settings["permissions"]["allow"].as_array_mut().unwrap();
    if declared_owned {
        allow.retain(|r| r.as_str() != Some(RULE));
    }
    if !retired_pre_existing {
        allow.push(json!(RETIRED));
    }
    if allow.is_empty() {
        settings["permissions"]
            .as_object_mut()
            .unwrap()
            .remove("allow");
    }
    if settings["permissions"].as_object().unwrap().is_empty() {
        settings.as_object_mut().unwrap().remove("permissions");
    }
    let bytes = serde_json::to_vec(&settings).unwrap();
    fs::write(config, &bytes).unwrap();
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    manifest.permission = Some(retired_record(retired_pre_existing));
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

const RETIRED_BASES: [(&str, &[u8], bool, bool); 5] = [
    // (name, original, retired rule is the user's own, declared rule is the user's own)
    ("empty", b"{}", false, false),
    ("other rules", br#"{"permissions":{"allow":["Read"],"deny":["Bash(rm *)"]},"x":1}"#, false, false),
    (
        "user held retired",
        br#"{"permissions":{"allow":["Bash(export HERDR_THREADS_CALLER_CONTEXT=*)","Read"]}}"#,
        true,
        false,
    ),
    (
        "user held declared",
        br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#,
        false,
        true,
    ),
    (
        "user held both",
        br#"{"permissions":{"allow":["Bash(herdr-threads *)","Bash(export HERDR_THREADS_CALLER_CONTEXT=*)"]}}"#,
        true,
        true,
    ),
];

// Kills: reporting an export-rule installation as installed, re-setup that keeps the owned
// export rule, removes a user's export rule, claims a user's declared rule as owned, duplicates
// either rule, or loses byte-exact restoration on the later unsetup.
#[test]
fn resetup_upgrades_retired_export_rule_installation() {
    for (name, original, retired_user, declared_user) in RETIRED_BASES {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        record_as_retired(&config, &manifest_path, retired_user);
        let status =
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
        assert!(!status.installed, "{name}");
        let rule = status.allow_rule.unwrap();
        assert_eq!(rule.ownership, AllowRuleOwnership::Superseded, "{name}");
        assert_eq!(rule.rule, RULE, "{name}");
        assert_eq!(rule.present, declared_user, "{name}");

        let upgraded = install_claude_user(&config, &manifest_path, &argv, original)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let permission = upgraded.permission.clone().unwrap();
        assert_eq!(permission.rule, RULE, "{name}");
        assert_eq!(permission.pre_existing, declared_user, "{name}");
        assert_eq!(upgraded.superseded_permission, None, "{name}");
        assert_eq!(upgraded.phase, InstallPhase::Installed, "{name}");
        assert_eq!(count_of(&config, RULE), 1, "{name}: declared rule once");
        assert_eq!(
            count_of(&config, RETIRED),
            usize::from(retired_user),
            "{name}: owned retired rule removed, user's kept"
        );
        let status =
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
        assert!(status.installed, "{name}");
        assert_eq!(
            status.allow_rule.unwrap().ownership,
            if declared_user {
                AllowRuleOwnership::PreExisting
            } else {
                AllowRuleOwnership::Owned
            },
            "{name}"
        );
        // Idempotent afterwards.
        let settled = fs::read(&config).unwrap();
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(fs::read(&config).unwrap(), settled, "{name}");
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), original, "{name}: byte-exact");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: unsetup of a never-upgraded export-rule installation that keeps the owned export rule,
// removes a user's own, or loses byte-exact restoration.
#[test]
fn unsetup_of_retired_export_rule_installation_removes_only_the_owned_rule() {
    for (name, original, retired_user, _) in RETIRED_BASES {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], original).unwrap();
        record_as_retired(&config, &manifest_path, retired_user);
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), original, "{name}: byte-exact");
        assert!(!manifest_path.exists(), "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: re-setup refusing (or re-adding the export rule) when the user already deleted the
// owned retired rule by hand: the upgrade removes it anyway.
#[test]
fn upgrade_tolerates_hand_removed_retired_rule() {
    let (dir, config, manifest_path) = claude_scope();
    let original = br#"{"other":1}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    record_as_retired(&config, &manifest_path, false);
    let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("permissions");
    fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
    assert_eq!(count_of(&config, RULE), 1);
    assert_eq!(count_of(&config, RETIRED), 0);
    assert!(
        inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
            .unwrap()
            .installed
    );
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(count_of(&config, RULE), 0);
    assert!(!owned_command_present(&config));
    fs::remove_dir_all(dir).unwrap();
}

// Kills: an upgrade interrupted after recording intent (or after publication) that forgets the
// owned export rule, strands it on resume or on prepared removal, or strands the new rule.
#[test]
fn interrupted_export_rule_upgrade_resumes_or_removes_both_rules() {
    for fault in [
        InstallFault::InterruptAfterIntent,
        InstallFault::InterruptAfterPublication,
    ] {
        for resume in [true, false] {
            let (dir, config, manifest_path) = claude_scope();
            let original = br#"{"permissions":{"deny":["Bash(rm *)"]}}"#;
            fs::write(&config, original).unwrap();
            let argv = vec!["/tmp/owned".into()];
            install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            record_as_retired(&config, &manifest_path, false);
            assert_eq!(
                install_claude_user_with_fault(
                    &config,
                    &manifest_path,
                    &argv,
                    original,
                    fault,
                    || {},
                ),
                Err(SetupError::Io),
                "{fault:?}"
            );
            let intent = read_manifest_file(&manifest_path);
            assert_eq!(intent.phase, InstallPhase::Prepared, "{fault:?}");
            assert_eq!(intent.superseded_permission, Some(retired_record(false)));
            assert_eq!(intent.permission.as_ref().unwrap().rule, RULE);
            let published = fault == InstallFault::InterruptAfterPublication;
            assert_eq!(count_of(&config, RULE), usize::from(published), "{fault:?}");
            assert_eq!(
                count_of(&config, RETIRED),
                usize::from(!published),
                "{fault:?}"
            );
            if resume {
                let resumed =
                    install_claude_user(&config, &manifest_path, &argv, original).unwrap();
                assert_eq!(resumed.phase, InstallPhase::Installed);
                assert_eq!(resumed.superseded_permission, None);
                assert_eq!(count_of(&config, RULE), 1);
                assert_eq!(count_of(&config, RETIRED), 0);
            }
            remove_claude_user(&config, &manifest_path).unwrap();
            assert!(!owned_command_present(&config));
            assert_eq!(count_of(&config, RULE), 0, "{fault:?} resume={resume}");
            assert_eq!(count_of(&config, RETIRED), 0, "{fault:?} resume={resume}");
            if resume {
                assert_eq!(fs::read(&config).unwrap(), original, "{fault:?}");
            } else {
                // Prepared removal is structural: only emptied hook arrays may remain.
                let mut after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                after.as_object_mut().unwrap().remove("hooks");
                assert_eq!(
                    after,
                    json!({"permissions":{"deny":["Bash(rm *)"]}}),
                    "{fault:?}"
                );
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

// Kills: a manifest that forges a superseded rule (a user's pre-existing export rule, the
// declared rule, or one outside a prepared upgrade) directing removal of a rule setup never
// added.
#[test]
fn forged_superseded_permission_is_refused() {
    let forgeries: [fn(&mut OwnershipManifest); 3] = [
        |m| {
            m.phase = InstallPhase::Prepared;
            m.superseded_permission = Some(retired_record(true));
        },
        |m| {
            m.phase = InstallPhase::Prepared;
            m.superseded_permission = m.permission.clone();
        },
        |m| m.superseded_permission = Some(retired_record(false)),
    ];
    for forge in forgeries {
        let (dir, config, manifest_path) = claude_scope();
        let original =
            br#"{"permissions":{"allow":["Bash(export HERDR_THREADS_CALLER_CONTEXT=*)"]}}"#;
        fs::write(&config, original).unwrap();
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], original).unwrap();
        let mut manifest = read_manifest_file(&manifest_path);
        forge(&mut manifest);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let before = fs::read(&config).unwrap();
        assert_eq!(
            remove_claude_user(&config, &manifest_path),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: the structural upgrade path (settings edited since install, so the recorded base cannot
// be reused) removing a user's own export rule, keeping the owned one, or duplicating either
// rule; and unsetup afterwards dropping the unrelated edit.
#[test]
fn upgrade_after_unrelated_edit_replaces_only_the_owned_export_rule() {
    for (name, original, retired_user, declared_user) in RETIRED_BASES {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        record_as_retired(&config, &manifest_path, retired_user);
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v["later"] = json!("keep");
        fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
        install_claude_user(&config, &manifest_path, &argv, original)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(count_of(&config, RULE), 1, "{name}");
        assert_eq!(
            count_of(&config, RETIRED),
            usize::from(retired_user),
            "{name}"
        );
        assert!(
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed,
            "{name}"
        );
        remove_claude_user(&config, &manifest_path).unwrap();
        assert!(!owned_command_present(&config), "{name}");
        assert_eq!(
            count_of(&config, RULE),
            usize::from(declared_user),
            "{name}"
        );
        assert_eq!(
            count_of(&config, RETIRED),
            usize::from(retired_user),
            "{name}"
        );
        let mut after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        after.as_object_mut().unwrap().remove("hooks");
        let mut expected: Value = serde_json::from_slice(original).unwrap();
        expected["later"] = json!("keep");
        assert_eq!(after, expected, "{name}");
        fs::remove_dir_all(dir).unwrap();
    }
}

// ---- per-event registration (`--event`) ----

fn codex_scope() -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".codex");
    fs::create_dir_all(&scope).unwrap();
    (
        dir.clone(),
        scope.join("hooks.json"),
        dir.join("manifest.json"),
    )
}

fn command_of(entry: &OwnedEntry) -> String {
    entry.group["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Every owned group registers its own event: `'--event' '<event>'` sits after the harness word
/// and before the owner marker.
fn assert_event_per_group(entries: &[OwnedEntry], base: &str) {
    assert!(!entries.is_empty());
    for entry in entries {
        let command = command_of(entry);
        assert_eq!(
            command,
            event_command(base, &entry.event),
            "{} group",
            entry.event
        );
        assert!(
            command.contains(&format!(
                " '--event' '{}' # herdr-threads-owner:",
                entry.event
            )),
            "{command}"
        );
    }
}

#[test]
fn claude_setup_registers_event_per_group() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/x/herdr-threads".into(), "hook".into(), "claude".into()];
    let manifest = install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    let base = format!(
        "{} # herdr-threads-owner:{}",
        shell_command(&argv).unwrap(),
        manifest.installation_id
    );
    assert_eq!(manifest.owned.len(), 2);
    assert_event_per_group(&manifest.owned, &base);
    assert_eq!(
        command_of(&manifest.owned[0]),
        format!(
            "'/x/herdr-threads' 'hook' 'claude' '--event' 'SessionStart' # herdr-threads-owner:{}",
            manifest.installation_id
        )
    );
    // The file carries exactly the recorded groups.
    let installed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    for entry in &manifest.owned {
        assert_eq!(installed["hooks"][&entry.event], json!([entry.group]));
    }
    remove_claude_user(&config, &manifest_path).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_setup_registers_event_per_group() {
    let (dir, config, manifest_path) = codex_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/x/herdr-threads".into(), "hook".into(), "codex".into()];
    let manifest = install_user_settings(
        SettingsKind::CodexUser,
        &config,
        &manifest_path,
        &argv,
        b"{}",
    )
    .unwrap();
    let base = format!(
        "{} # herdr-threads-owner:{}",
        shell_command(&argv).unwrap(),
        manifest.installation_id
    );
    let events: Vec<&str> = manifest.owned.iter().map(|e| e.event.as_str()).collect();
    assert_eq!(events, ["SessionStart", "SubagentStart", "PreToolUse"]);
    assert_event_per_group(&manifest.owned, &base);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_session_plan_registers_event_per_group() {
    let argv: Vec<String> = vec!["/x/herdr-threads".into(), "hook".into(), "codex".into()];
    let plan = plan_codex_for_version(&[], &argv, &pinned()).unwrap();
    let base = shell_command(&argv).unwrap();
    assert_eq!(plan.owned.len(), 3);
    for entry in &plan.owned {
        assert_eq!(command_of(entry), event_command(&base, &entry.event));
        assert!(command_of(entry).ends_with(&format!(" '--event' '{}'", entry.event)));
    }
    // The session `-c` overrides carry the evented commands.
    let flags = plan.session_config.join("\n");
    for event in ["SessionStart", "SubagentStart", "PreToolUse"] {
        assert!(
            flags.contains(&format!("'hook' 'codex' '--event' '{event}'")),
            "{event}: {flags}"
        );
    }
}

#[test]
fn event_command_and_base_command_round_trip() {
    let marker = " # herdr-threads-owner:0b5e8f3c-1111-4222-8333-444455556666";
    let quoted = "'/x/it'\\''s here/herdr-threads' 'hook' 'claude'";
    for base in [
        "'/x/herdr-threads' 'hook' 'claude'".to_owned(),
        quoted.to_owned(),
        format!("'/x/herdr-threads' 'hook' 'codex'{marker}"),
        format!("{quoted}{marker}"),
    ] {
        for event in ["SessionStart", "PreToolUse", "SubagentStart"] {
            let evented = event_command(&base, event);
            assert_ne!(evented, base);
            assert_eq!(base_command(&evented, event), base.as_str(), "{evented}");
            // The pair precedes the marker; the marker stays last.
            if base.contains(marker) {
                assert!(evented.ends_with(marker), "{evented}");
                assert!(evented.contains(&format!("'--event' '{event}'{marker}")));
            } else {
                assert!(evented.ends_with(&format!("'--event' '{event}'")));
            }
            // A legacy command, or a pair for another event, is returned unchanged.
            assert_eq!(base_command(&base, event), base.as_str());
            assert_eq!(base_command(&evented, "Stop"), evented.as_str());
        }
    }
}

/// Rewrites a current installation into the form made before per-event registration: every
/// `--event` pair removed from the settings and the manifest's recorded groups, with consistent
/// fingerprints.
fn downgrade_to_legacy_registration(config: &std::path::Path, manifest_path: &std::path::Path) {
    let mut manifest = read_manifest_file(manifest_path);
    let mut text = String::from_utf8(fs::read(config).unwrap()).unwrap();
    for entry in &mut manifest.owned {
        let pair = format!(" '--event' '{}'", entry.event);
        assert!(
            text.contains(&pair),
            "{} not evented in the file",
            entry.event
        );
        text = text.replace(&pair, "");
        let command = command_of(entry).replace(&pair, "");
        entry.group["hooks"][0]["command"] = json!(command);
        entry.fingerprint = group_fingerprint(&entry.group);
    }
    assert!(!text.contains("--event"));
    fs::write(config, text.as_bytes()).unwrap();
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(text.as_bytes()));
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

#[test]
fn legacy_install_is_installed_but_flagged() {
    for (kind, harness_word) in [
        (SettingsKind::ClaudeUser, "claude"),
        (SettingsKind::CodexUser, "codex"),
    ] {
        let (dir, config, manifest_path) = if kind == SettingsKind::ClaudeUser {
            claude_scope()
        } else {
            codex_scope()
        };
        fs::write(&config, b"{}").unwrap();
        let argv = vec![
            "/x/herdr-threads".into(),
            "hook".into(),
            harness_word.into(),
        ];
        install_user_settings(kind, &config, &manifest_path, &argv, b"{}").unwrap();
        let current =
            inspect_user_settings(kind, &config, &manifest_path, NativeObservation::Unknown)
                .unwrap();
        assert!(
            current.installed && !current.legacy_event_registration,
            "{harness_word}"
        );

        downgrade_to_legacy_registration(&config, &manifest_path);
        let legacy =
            inspect_user_settings(kind, &config, &manifest_path, NativeObservation::Unknown)
                .unwrap();
        assert!(
            legacy.installed,
            "{harness_word}: a legacy install still launches"
        );
        assert!(legacy.legacy_event_registration, "{harness_word}");
        assert!(legacy.configured_hook.is_some());
        // Removal of a legacy install trusts its recorded entries.
        remove_user_settings(kind, &config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), b"{}");
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_mixed_event_registration_is_neither_current_nor_legacy() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/x/herdr-threads".into(), "hook".into(), "claude".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    // Strip the pair from the SessionStart group only.
    let mut manifest = read_manifest_file(&manifest_path);
    let mut text = String::from_utf8(fs::read(&config).unwrap()).unwrap();
    let entry = &mut manifest.owned[0];
    assert_eq!(entry.event, "SessionStart");
    let pair = " '--event' 'SessionStart'";
    text = text.replace(pair, "");
    entry.group["hooks"][0]["command"] = json!(command_of(entry).replace(pair, ""));
    entry.fingerprint = group_fingerprint(&entry.group);
    fs::write(&config, text.as_bytes()).unwrap();
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(text.as_bytes()));
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let status = inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
    assert!(!status.installed && !status.legacy_event_registration);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn setup_upgrades_legacy_registration() {
    for (kind, harness_word, user_hooks) in [
        (
            SettingsKind::ClaudeUser,
            "claude",
            &br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user"}]}]},"other":1}"#[..],
        ),
        (SettingsKind::CodexUser, "codex", &br#"{"other":1}"#[..]),
    ] {
        let (dir, config, manifest_path) = if kind == SettingsKind::ClaudeUser {
            claude_scope()
        } else {
            codex_scope()
        };
        fs::write(&config, user_hooks).unwrap();
        let argv = vec![
            "/x/herdr-threads".into(),
            "hook".into(),
            harness_word.into(),
        ];
        install_user_settings(kind, &config, &manifest_path, &argv, user_hooks).unwrap();
        downgrade_to_legacy_registration(&config, &manifest_path);
        let legacy_owned = read_manifest_file(&manifest_path).owned;

        let upgraded =
            install_user_settings(kind, &config, &manifest_path, &argv, user_hooks).unwrap();
        assert_eq!(upgraded.phase, InstallPhase::Installed);
        assert!(upgraded.superseded.is_empty());
        let base = format!(
            "{} # herdr-threads-owner:{}",
            shell_command(&argv).unwrap(),
            upgraded.installation_id
        );
        assert_event_per_group(&upgraded.owned, &base);
        assert_ne!(upgraded.owned, legacy_owned);
        // No legacy group is left behind: every owned group of each event is the evented one.
        let value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        let text = String::from_utf8(fs::read(&config).unwrap()).unwrap();
        for legacy in &legacy_owned {
            let legacy_command = command_of(legacy);
            let groups = value["hooks"][&legacy.event].as_array().unwrap();
            assert!(
                groups
                    .iter()
                    .all(|g| g["hooks"][0]["command"] != json!(legacy_command)),
                "{} still has its legacy group",
                legacy.event
            );
        }
        assert_eq!(
            text.matches("herdr-threads-owner:").count(),
            upgraded.owned.len()
        );
        let status =
            inspect_user_settings(kind, &config, &manifest_path, NativeObservation::Unknown)
                .unwrap();
        assert!(status.installed && !status.legacy_event_registration);
        // The user's own hooks survive, and uninstall removes everything added.
        remove_user_settings(kind, &config, &manifest_path).unwrap();
        let after: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        let before: Value = serde_json::from_slice(user_hooks).unwrap();
        assert_eq!(after["other"], before["other"]);
        assert_eq!(after["hooks"].get("Stop"), before["hooks"].get("Stop"));
        assert!(
            !fs::read_to_string(&config)
                .unwrap()
                .contains("herdr-threads")
        );
        assert!(!manifest_path.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn adoption_finds_evented_marked_copy() {
    let (dir, config, manifest_path) = codex_scope();
    let (other_dir, other_config, other_manifest) = codex_scope();
    fs::write(&config, b"{}").unwrap();
    let argv: Vec<String> = vec!["/x/herdr-threads".into(), "hook".into(), "codex".into()];
    let installed = install_user_settings(
        SettingsKind::CodexUser,
        &config,
        &manifest_path,
        &argv,
        b"{}",
    )
    .unwrap();
    // A second CODEX_HOME holding a byte copy of the hooks file (trust travels with it).
    fs::copy(&config, &other_config).unwrap();
    let adoptable = adoptable_user_settings(SettingsKind::CodexUser, &other_config, &argv)
        .unwrap()
        .expect("an exact evented marked copy is adoptable");
    assert_eq!(adoptable.installation_id, installed.installation_id);
    assert_event_per_group(
        &adoptable.owned,
        &format!(
            "{} # herdr-threads-owner:{}",
            shell_command(&argv).unwrap(),
            installed.installation_id
        ),
    );
    let before = fs::read(&other_config).unwrap();
    let adopted = adopt_user_settings(
        SettingsKind::CodexUser,
        &other_config,
        &other_manifest,
        &argv,
    )
    .unwrap()
    .unwrap();
    assert!(adopted.adopted);
    assert_eq!(
        fs::read(&other_config).unwrap(),
        before,
        "adoption never writes the hook file"
    );
    // A copy whose commands lost their `--event` (an unmarked-by-event copy) is not adopted.
    let text = fs::read_to_string(&config)
        .unwrap()
        .replace(" '--event' 'SessionStart'", "");
    fs::write(&other_config, text).unwrap();
    assert!(
        adoptable_user_settings(SettingsKind::CodexUser, &other_config, &argv)
            .unwrap()
            .is_none()
    );
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(other_dir).unwrap();
}
