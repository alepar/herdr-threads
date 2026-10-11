use crate::harness::claude::setup::plan_claude;
use crate::harness::codex::setup::{
    codex_session_manifest, inspect_codex_session, plan_codex_for_contract, plan_codex_for_version,
    uninstall_codex, verify_codex_base,
};
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
    install_user_settings(
        crate::harness::claude::settings_kind(),
        config,
        manifest,
        argv,
        base,
    )
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
        crate::harness::claude::settings_kind(),
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
    inspect_user_settings(
        crate::harness::claude::settings_kind(),
        config,
        manifest,
        observed,
    )
}
fn remove_claude_user(
    config: &std::path::Path,
    manifest: &std::path::Path,
) -> Result<(), SetupError> {
    remove_user_settings(crate::harness::claude::settings_kind(), config, manifest)
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
    let plan = plan_codex_for_contract(
        &groups,
        &["/tmp/space 雪/owned".into()],
        &crate::harness::codex::CodexContract::registered(),
    )
    .unwrap();
    assert_eq!(plan.launch_argv[0], "-c");
    let flag = &plan.launch_argv[1];
    assert!(flag.starts_with("hooks.SessionStart=["));
    assert!(flag.contains("user-hook"));
    assert!(flag.contains("space 雪"));
    assert!(!plan.launch_argv.iter().any(|s| s.contains("trusted_hash")));
    assert_eq!(plan.events[0].groups.len(), 2);
    assert_eq!(
        plan_codex_for_contract(
            &plan.events,
            &["/tmp/space 雪/owned".into()],
            &crate::harness::codex::CodexContract::registered()
        )
        .unwrap()
        .events,
        plan.events
    );
}
/// Setup lines add only owned hook configuration and retain caller arguments.
#[test]
fn setup_codex_launch_argv_does_not_inject_daemon_flags() {
    let plan = plan_codex_for_version(&[], &["/tmp/owned".into()], &pinned()).unwrap();
    assert_eq!(plan.launch_argv, plan.session_config);
    assert_eq!(plan.launch_argv[0], "-c");
    assert!(!plan.launch_argv.iter().any(|arg| arg == "--no-daemon"));
    let with_flag = plan
        .launch_argv_for(vec!["--no-daemon".into(), "PROMPT".into()])
        .unwrap();
    assert_eq!(
        with_flag.iter().filter(|arg| *arg == "--no-daemon").count(),
        1
    );
    assert_eq!(with_flag.last().unwrap(), "PROMPT");
    assert_eq!(
        plan.launch_argv_for(vec!["exec".into(), "PROMPT".into()])
            .unwrap(),
        crate::harness::launch::compose_native_argv(
            crate::protocol::authority::Harness::Codex,
            vec!["exec".into(), "PROMPT".into()],
            plan.session_config.clone(),
        )
        .unwrap()
    );
    assert_eq!(
        plan.launch_argv_for(vec!["--daemon".into()]),
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
        (
            crate::harness::claude::settings_kind(),
            "settings.local.json",
        ),
        (crate::harness::claude::settings_kind(), "hooks.json"),
        (crate::harness::codex::settings_kind(), "settings.json"),
        (crate::harness::codex::settings_kind(), "config.toml"),
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
    let kind = crate::harness::codex::settings_kind();
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
            .ends_with("'hook-event' 'codex' 'SubagentStart'")
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
            crate::harness::claude::settings_kind(),
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
    let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    for entry in manifest.owned.iter().filter(|e| e.event != "PreToolUse") {
        let groups = settings["hooks"][&entry.event].as_array_mut().unwrap();
        groups.retain(|group| group != &entry.group);
        if groups.is_empty() {
            settings["hooks"]
                .as_object_mut()
                .unwrap()
                .remove(&entry.event);
        }
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
// upgrade intent. Driven through the real upgrade boundaries for each drift variant: every
// (boundary, resume-or-remove) pair runs once, and the drift variants alternate so each variant
// meets both boundaries and both outcomes (each case pays for durable installs).
#[test]
fn interrupted_drift_upgrade_resumes_or_removes_from_each_real_boundary() {
    let drifts = declaration_drifts();
    assert_eq!(
        drifts.len(),
        2,
        "the rotation below covers exactly two variants"
    );
    for (f, fault) in [
        InstallFault::InterruptAfterIntent,
        InstallFault::InterruptAfterPublication,
    ]
    .into_iter()
    .enumerate()
    {
        for (r, resume) in [false, true].into_iter().enumerate() {
            let (name, drift) = &drifts[(f + r) % 2];
            let (dir, config, manifest_path) = claude_scope();
            let original = br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"user-start"}]}]},"other":1}"#;
            fs::write(&config, original).unwrap();
            let argv = vec!["/tmp/owned".into()];
            let full = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
            drift_installation(&config, &manifest_path, drift);
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
            for event in ["PreToolUse", "SubagentStart", "SubagentStop"] {
                if after["hooks"][event] == json!([]) {
                    expected["hooks"][event] = json!([]);
                }
            }
            assert_eq!(after, expected, "{name} {fault:?} resume={resume}");
            assert!(!manifest_path.exists());
            fs::remove_dir_all(dir).unwrap();
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
                "PreToolUse":[expected[1].group.clone()],
                "SubagentStart":[expected[2].group.clone()],
                "SubagentStop":[expected[3].group.clone()]
            }}),
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
    assert_eq!(manifest.superseded.len(), manifest.owned.len());
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
        assert_eq!(
            owned_events,
            [
                "PreToolUse",
                "SessionStart",
                "SubagentStart",
                "SubagentStop"
            ],
            "{name}"
        );
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

// ---------------------------------------------------------------- historical allow rule
//
// Hook setup no longer writes a permission rule: the permission component owns permissions.
// Manifests recorded by earlier setups can still hold the broad rule (or the retired export
// rule); they stay readable, block a hook re-setup until the permission component retires the
// record, and unsetup still removes an owned copy.

const RULE: &str = crate::harness::claude::HERDR_THREADS_ALLOW_RULE;
const RETIRED: &str = crate::harness::claude::CALLER_CONTEXT_ALLOW_RULE;

fn count_of(config: &std::path::Path, rule: &str) -> usize {
    let v: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    v["permissions"]["allow"]
        .as_array()
        .map_or(0, |a| a.iter().filter(|r| r.as_str() == Some(rule)).count())
}

fn permission_record(rule: &str, pre_existing: bool) -> OwnedPermission {
    OwnedPermission {
        rule: rule.into(),
        fingerprint: format!(
            "sha256:{:x}",
            sha2::Sha256::digest(serde_json::to_vec(&json!({ "allow": rule })).unwrap())
        ),
        pre_existing,
    }
}

/// Rewrites a current installation into the shape an earlier setup recorded: `rule` recorded
/// and, unless the user held it, appended to `permissions.allow` as that setup published it.
fn record_historical(
    config: &std::path::Path,
    manifest_path: &std::path::Path,
    rule: &str,
    pre_existing: bool,
) {
    let mut manifest = read_manifest_file(manifest_path);
    manifest.permission = Some(permission_record(rule, pre_existing));
    if !pre_existing {
        let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
        let permissions = settings
            .as_object_mut()
            .unwrap()
            .entry("permissions")
            .or_insert_with(|| json!({}));
        permissions
            .as_object_mut()
            .unwrap()
            .entry("allow")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!(rule));
        let bytes = serde_json::to_vec(&settings).unwrap();
        fs::write(config, &bytes).unwrap();
        manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    }
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

// Kills: hook setup composing (or recording) a permission rule, inspection requiring one, or
// removal touching a rule the user holds.
#[test]
fn hook_setup_records_and_writes_no_permission_rule() {
    let bases: [&[u8]; 3] = [
        b"{}",
        br#"{"permissions":{"allow":["Read"],"deny":["Bash(rm *)"]}}"#,
        br#"{"permissions":{"allow":["Bash(herdr-threads *)"]},"x":[]}"#,
    ];
    for original in bases {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        let argv = vec!["/tmp/owned".into()];
        let manifest = install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(manifest.permission, None);
        let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("hooks");
        let mut expected: Value = serde_json::from_slice(original).unwrap();
        expected.as_object_mut().unwrap().remove("hooks");
        assert_eq!(v, expected, "only hooks change");
        let status =
            inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
        assert!(status.installed);
        assert_eq!(status.allow_rule, None);
        assert_eq!(
            recorded_hook_permission(
                crate::harness::claude::settings_kind(),
                &config,
                &manifest_path
            ),
            Ok(None)
        );
        let settled = fs::read(&config).unwrap();
        install_claude_user(&config, &manifest_path, &argv, original).unwrap();
        assert_eq!(fs::read(&config).unwrap(), settled);
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), original, "byte-exact");
        fs::remove_dir_all(dir).unwrap();
    }
}

// Kills: a hook re-setup that re-adds, keeps managing or silently drops a historical rule
// instead of leaving it to the permission component, and a retirement that loses the hooks.
#[test]
fn historical_rule_is_reported_and_blocks_hook_resetup_until_retired() {
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec!["/tmp/owned".into()];
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    record_historical(&config, &manifest_path, RULE, false);
    let status = inspect_claude_user(&config, &manifest_path, NativeObservation::Unknown).unwrap();
    assert!(status.installed);
    let rule = status.allow_rule.unwrap();
    assert_eq!(
        (rule.rule, rule.ownership, rule.present),
        (RULE, AllowRuleOwnership::Owned, true)
    );
    let held = fs::read(&config).unwrap();
    assert_eq!(
        install_claude_user(&config, &manifest_path, &argv, b"{}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), held);
    let recorded = recorded_hook_permission(
        crate::harness::claude::settings_kind(),
        &config,
        &manifest_path,
    )
    .unwrap()
    .unwrap();
    assert_eq!(recorded.owned, vec![RULE.to_owned()]);
    assert!(recorded.created_permissions && recorded.created_allow);
    retire_hook_permission(&manifest_path, &recorded).unwrap();
    assert_eq!(read_manifest_file(&manifest_path).permission, None);
    // Retiring again is done, not a conflict; the hooks stay installed.
    retire_hook_permission(&manifest_path, &recorded).unwrap();
    install_claude_user(&config, &manifest_path, &argv, b"{}").unwrap();
    assert_eq!(fs::read(&config).unwrap(), held);
    assert_eq!(
        count_of(&config, RULE),
        1,
        "the rule is the component's to change"
    );
    fs::remove_dir_all(dir).unwrap();
}

// Kills: unsetup of a never-migrated installation that keeps an owned historical rule, removes
// a user's own, removes a user duplicate, or loses byte-exact restoration.
#[test]
fn unsetup_removes_only_an_owned_historical_rule() {
    let cases: [(&[u8], &str, bool); 4] = [
        (br#"{"other":1}"#, RULE, false),
        (br#"{"permissions":{"allow":["Read"]}}"#, RETIRED, false),
        (
            br#"{"permissions":{"allow":["Bash(herdr-threads *)"]}}"#,
            RULE,
            true,
        ),
        (br#"{"permissions":{"allow":[]}}"#, RULE, false),
    ];
    for (original, rule, pre_existing) in cases {
        let (dir, config, manifest_path) = claude_scope();
        fs::write(&config, original).unwrap();
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], original).unwrap();
        record_historical(&config, &manifest_path, rule, pre_existing);
        remove_claude_user(&config, &manifest_path).unwrap();
        assert_eq!(
            fs::read(&config).unwrap(),
            original,
            "{rule} {pre_existing}"
        );
        fs::remove_dir_all(dir).unwrap();
    }
    // A user duplicate added later survives: only the owned occurrence goes.
    let (dir, config, manifest_path) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], b"{}").unwrap();
    record_historical(&config, &manifest_path, RULE, false);
    let mut v: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    v["permissions"]["allow"]
        .as_array_mut()
        .unwrap()
        .push(json!(RULE));
    fs::write(&config, serde_json::to_vec(&v).unwrap()).unwrap();
    remove_claude_user(&config, &manifest_path).unwrap();
    assert_eq!(count_of(&config, RULE), 1);
    assert!(!owned_command_present(&config));
    fs::remove_dir_all(dir).unwrap();
}

// Kills: accepting a recorded rule other than the declared or retired one (a forged manifest
// could otherwise direct removal of an arbitrary user rule) or one with a stale fingerprint.
#[test]
fn forged_historical_record_is_refused() {
    for forge in [
        |m: &mut OwnershipManifest| m.permission = Some(permission_record("Read", false)),
        |m: &mut OwnershipManifest| m.permission.as_mut().unwrap().fingerprint = "sha256:00".into(),
        |m: &mut OwnershipManifest| m.superseded_permission = Some(permission_record(RULE, true)),
    ] {
        let (dir, config, manifest_path) = claude_scope();
        let original = br#"{"permissions":{"allow":["Read"]}}"#;
        fs::write(&config, original).unwrap();
        install_claude_user(&config, &manifest_path, &["/tmp/owned".into()], original).unwrap();
        record_historical(&config, &manifest_path, RULE, false);
        let mut manifest = read_manifest_file(&manifest_path);
        forge(&mut manifest);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let before = fs::read(&config).unwrap();
        assert_eq!(
            recorded_hook_permission(
                crate::harness::claude::settings_kind(),
                &config,
                &manifest_path
            ),
            Err(SetupError::Conflict)
        );
        assert_eq!(
            remove_claude_user(&config, &manifest_path),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), before);
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
    let events: Vec<&str> = manifest.owned.iter().map(|e| e.event.as_str()).collect();
    assert_eq!(
        events,
        [
            "SessionStart",
            "PreToolUse",
            "SubagentStart",
            "SubagentStop"
        ]
    );
    for entry in &manifest.owned {
        assert_eq!(
            command_of(entry),
            format!("'/x/herdr-threads' 'hook-event' 'claude' '{}'", entry.event)
        );
    }
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
        crate::harness::codex::settings_kind(),
        &config,
        &manifest_path,
        &argv,
        b"{}",
    )
    .unwrap();
    let events: Vec<&str> = manifest.owned.iter().map(|e| e.event.as_str()).collect();
    assert_eq!(
        events,
        [
            "SessionStart",
            "SubagentStart",
            "PreToolUse",
            "SubagentStop",
            "SessionEnd"
        ]
    );
    for entry in &manifest.owned {
        assert_eq!(
            command_of(entry),
            format!("'/x/herdr-threads' 'hook-event' 'codex' '{}'", entry.event)
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_session_plan_registers_event_per_group() {
    let argv: Vec<String> = vec!["/x/herdr-threads".into(), "hook".into(), "codex".into()];
    let plan = plan_codex_for_version(&[], &argv, &pinned()).unwrap();
    assert_eq!(plan.owned.len(), 5);
    for entry in &plan.owned {
        assert_eq!(
            command_of(entry),
            format!("'/x/herdr-threads' 'hook-event' 'codex' '{}'", entry.event)
        );
    }
    // The session `-c` overrides carry the evented commands.
    let flags = plan.session_config.join("\n");
    for event in [
        "SessionStart",
        "SubagentStart",
        "PreToolUse",
        "SubagentStop",
        "SessionEnd",
    ] {
        assert!(
            flags.contains(&format!("'hook-event' 'codex' '{event}'")),
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
    seed_uuid_registration(config, manifest_path, false);
}

#[test]
fn legacy_install_is_installed_but_flagged() {
    for (kind, harness_word) in [
        (crate::harness::claude::settings_kind(), "claude"),
        (crate::harness::codex::settings_kind(), "codex"),
    ] {
        let (dir, config, manifest_path) = if kind == crate::harness::claude::settings_kind() {
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
    let pair = " 'SessionStart'";
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
            crate::harness::claude::settings_kind(),
            "claude",
            &br#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"user"}]}]},"other":1}"#[..],
        ),
        (
            crate::harness::codex::settings_kind(),
            "codex",
            &br#"{"other":1}"#[..],
        ),
    ] {
        let (dir, config, manifest_path) = if kind == crate::harness::claude::settings_kind() {
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
        for entry in &upgraded.owned {
            assert_eq!(
                command_of(entry),
                format!(
                    "'/x/herdr-threads' 'hook-event' '{harness_word}' '{}'",
                    entry.event
                )
            );
        }
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
        assert_eq!(text.matches("herdr-threads-owner:").count(), 0);
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
    install_user_settings(
        crate::harness::codex::settings_kind(),
        &config,
        &manifest_path,
        &argv,
        b"{}",
    )
    .unwrap();
    let installed = seed_uuid_registration(&config, &manifest_path, true);
    // A second CODEX_HOME holding a byte copy of the hooks file (trust travels with it).
    fs::copy(&config, &other_config).unwrap();
    let adoptable =
        adoptable_user_settings(crate::harness::codex::settings_kind(), &other_config, &argv)
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
        crate::harness::codex::settings_kind(),
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
        adoptable_user_settings(crate::harness::codex::settings_kind(), &other_config, &argv)
            .unwrap()
            .is_none()
    );
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(other_dir).unwrap();
}

#[test]
fn operation_scope_defaults_preserve_legacy_resolution_and_refuse_foreign_generations() {
    use crate::harness::{adapter::*, registry};
    use crate::protocol::time::{CallBudget, MonoInstant};
    let registry = registry::builtins();
    let registration = registry.by_id(registry.agent("claude").unwrap()).unwrap();
    let root = std::env::temp_dir().join(format!("scope-default-{}", uuid::Uuid::new_v4()));
    let environment = SetupEnvironment {
        config_roots: [("claude".into(), root.join("claude"))].into(),
        ..Default::default()
    };
    let selector = SetupScopeRequest::Default;
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let request = SetupScopeResolutionRequest {
        operation: SetupScopeOperation::Remove,
        selector: &selector,
        native_binary: None,
        environment: &environment,
    };
    let mut resolution = registration
        .resolve_setup_scope_for(&request, &budget)
        .unwrap();
    assert_eq!(
        resolution.scope,
        registration
            .resolve_setup_scope(&selector, &environment)
            .unwrap()
    );
    assert!(resolution.removal_generation.is_none());
    resolution.removal_generation = Some(uuid::Uuid::new_v4().to_string());
    assert!(
        registration
            .unsetup_resolved(
                &UnsetupRequest {
                    scope: resolution.scope.clone(),
                    environment: environment.clone()
                },
                &resolution,
                &budget
            )
            .is_err()
    );
    resolution.removal_generation = None;
    assert!(
        registration
            .unsetup_resolved(
                &UnsetupRequest {
                    scope: ResolvedSetupScope::ConfigRoot(root.join("foreign")),
                    environment: environment.clone()
                },
                &resolution,
                &budget
            )
            .is_err()
    );
    let expired = CallBudget {
        deadline: MonoInstant(0),
        cancellation: Default::default(),
    };
    assert!(
        registration
            .resolve_setup_scope_for(&request, &expired)
            .is_err()
    );
    let cancelled = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    cancelled.cancellation.cancel();
    assert!(
        registration
            .resolve_setup_scope_for(&request, &cancelled)
            .is_err()
    );
    assert!(!root.exists(), "refusal created a native or state root");
}

// ------------------------------------------------ extension proof (core-sweep D4, D5, D6)

/// A test-only adapter registered in a test `Registry`: it declares a config root, foreground
/// advice and an allow-member-only settings kind, and nothing in core names it. The setup
/// engine, the foreground report and the inspection plumbing reach it through the registry alone.
mod extension_proof {
    use crate::cli::setup::{SetupEnv, default_config_roots, foreground};
    use crate::harness::{
        adapter::*,
        registry::{Registration, Registry},
        setup::{
            InstallPhase, NativeObservation, OwnedAllowSpec, SettingsKind, SettingsKindSpec,
            SetupError, inspect_user_settings, install_user_settings, legacy, plan_settings,
            read_settings_manifest, remove_user_settings,
        },
    };
    use crate::protocol::time::CallBudget;
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};

    const ID: &str = "exttest";
    const MEMBER: &str = "Bash(exttest *)";

    fn no_groups(_: &str) -> Vec<(&'static str, Value)> {
        Vec::new()
    }
    fn session_group(command: &str) -> Vec<(&'static str, Value)> {
        vec![(
            "SessionStart",
            json!({"hooks": [{"type": "command", "command": command}]}),
        )]
    }
    static MEMBER_KIND: SettingsKindSpec = SettingsKindSpec {
        harness: ID,
        file_name: "ext-settings.json",
        hook_groups: no_groups,
        owned_allow: Some(OwnedAllowSpec {
            rule: MEMBER,
            retired: &[],
            native_rewrites: true,
        }),
        adoption_note: "",
        legacy_upgrade_warning: "",
        permission_rules: crate::harness::setup::no_permission_rules,
    };
    static HOOK_KIND: SettingsKindSpec = SettingsKindSpec {
        harness: ID,
        file_name: "ext-hooks.json",
        hook_groups: session_group,
        owned_allow: None,
        adoption_note: "",
        legacy_upgrade_warning: "",
        permission_rules: crate::harness::setup::no_permission_rules,
    };

    fn inspect_ext(bytes: &[u8], report: &mut Value) -> Result<(), String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "ext config is not UTF-8".to_owned())?;
        if text.contains("bad") {
            return Err("ext config is invalid".into());
        }
        report["ext_flag"] = json!(text.contains("on"));
        if text.contains("on") {
            report["status"] = json!("configured");
        }
        Ok(())
    }
    static FOREGROUND: ForegroundAdvice = ForegroundAdvice {
        settings_file: "ext.toml",
        advice: "turn the ext flag on",
        fixed_fields: &[("fixed_field", "always")],
        inspect: inspect_ext,
    };
    static METADATA: AdapterMetadata = AdapterMetadata {
        id: ID,
        display_label: "Exttest",
        context_spelling: "Exttest",
        context_aliases: &[],
        executable: ExecutableLookup::Unsupported,
        host_kinds: &[],
        setup_scopes: &[SetupScopeKind::ConfigRoot],
        budget: EventBudgetPolicy {
            lifecycle_ms: 1000,
            observer_ms: 1000,
        },
        runtime_sources: &[],
        agent_env_markers: &[],
    };

    struct Ext {
        kind: &'static SettingsKindSpec,
    }
    impl HarnessAdapter for Ext {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            &METADATA
        }
        fn setup_environment_inputs(&self) -> &'static [&'static str] {
            &["HT_TEST_ROOT"]
        }
        fn default_config_root(&self, environment: &SetupEnvironment) -> Option<PathBuf> {
            match environment.declared.get("HT_TEST_ROOT") {
                Some(root) if !root.is_empty() => Some(PathBuf::from(root)),
                _ => environment
                    .home
                    .as_ref()
                    .map(|home| PathBuf::from(home).join(".testharness")),
            }
        }
        fn settings_kind(&self) -> Option<&'static SettingsKindSpec> {
            Some(self.kind)
        }
        fn foreground_advice(&self) -> Option<&'static ForegroundAdvice> {
            Some(&FOREGROUND)
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            &[]
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            unreachable!()
        }
        fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            unreachable!()
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            unreachable!()
        }
        fn classify(&self, _: &HookInput) -> ContractObservation {
            unreachable!()
        }
        fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            unreachable!()
        }
        fn encode(
            &self,
            _: &(),
            _: &DecodedEvent,
            _: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            unreachable!()
        }
        fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            unreachable!()
        }
        fn setup(&self, _: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            unreachable!()
        }
        fn status(&self, _: &StatusRequest, _: &CallBudget) -> SetupStatus {
            unreachable!()
        }
        fn unsetup(
            &self,
            _: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            unreachable!()
        }
    }

    fn registry(kind: &'static SettingsKindSpec) -> &'static Registry {
        let adapter: &'static Ext = Box::leak(Box::new(Ext { kind }));
        Box::leak(Box::new(
            Registry::new(Box::leak(
                vec![Registration::new(adapter)].into_boxed_slice(),
            ))
            .expect("a well-formed extension registers"),
        ))
    }

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ht-ext-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn environment(home: Option<&Path>, root: Option<&Path>) -> SetupEnvironment {
        SetupEnvironment {
            home: home.map(|home| home.as_os_str().to_owned()),
            declared: root
                .map(|root| ("HT_TEST_ROOT".to_owned(), root.as_os_str().to_owned()))
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    fn setup_env(roots: std::collections::BTreeMap<String, PathBuf>, state: &Path) -> SetupEnv {
        SetupEnv {
            home: None,
            declared_environment: Default::default(),
            executable: PathBuf::from("/x/herdr-threads"),
            state_dir: Some(state.to_path_buf()),
            cwd: PathBuf::from("/"),
            path: None,
            config_roots: roots,
            host_endpoint: Some(PathBuf::from("/x/host.sock")),
            instance_source: Value::Null,
        }
    }

    /// Kills: a core that resolves only the roots it names, or lets `HOME` win over the
    /// adapter's declared variable, or keeps an empty value.
    #[test]
    fn a_registered_adapter_contributes_its_config_root() {
        let registry = registry(&MEMBER_KIND);
        let home = Path::new("/h");
        let declared =
            default_config_roots(registry, &environment(Some(home), Some(Path::new("/r"))));
        assert_eq!(declared.get(ID), Some(&PathBuf::from("/r")));
        let fallback = default_config_roots(registry, &environment(Some(home), None));
        assert_eq!(fallback.get(ID), Some(&PathBuf::from("/h/.testharness")));
        let empty = default_config_roots(registry, &environment(Some(home), Some(Path::new(""))));
        assert_eq!(empty.get(ID), Some(&PathBuf::from("/h/.testharness")));
        assert!(default_config_roots(registry, &environment(None, None)).is_empty());
        assert_eq!(declared.len(), 1, "only registered adapters have roots");
    }

    /// Kills: a foreground report that skips the adapter's fixed fields or `user_config`, a
    /// missing file that is an error, an adapter parse error that leaves `status` untouched, or
    /// a hard-coded adapter table (this adapter is in no core table).
    #[test]
    fn foreground_report_assembles_from_the_adapters_advice() {
        let registry = registry(&MEMBER_KIND);
        let dir = scratch("foreground");
        let env = setup_env([(ID.to_owned(), dir.clone())].into(), &dir);
        let missing = foreground::inspect_in(registry, ID, &env);
        assert_eq!(missing["status"], "required");
        assert_eq!(missing["scope"], "user");
        assert_eq!(missing["execution"], "unknown");
        assert_eq!(
            missing["settings_file"],
            dir.join("ext.toml").display().to_string()
        );
        assert_eq!(missing["advice"], "turn the ext flag on");
        assert_eq!(missing["modified_by_setup"], false);
        assert_eq!(missing["fixed_field"], "always");
        assert_eq!(missing["user_config"], "missing");
        assert!(missing.get("ext_flag").is_none() && missing.get("inspection_error").is_none());

        std::fs::write(dir.join("ext.toml"), "flag = on").unwrap();
        let configured = foreground::inspect_in(registry, ID, &env);
        assert_eq!(configured["status"], "configured");
        assert_eq!(configured["ext_flag"], true);
        assert_eq!(configured["user_config"], "read");
        assert_eq!(configured["fixed_field"], "always");

        std::fs::write(dir.join("ext.toml"), "flag = off").unwrap();
        let off = foreground::inspect_in(registry, ID, &env);
        assert_eq!(
            (off["status"].as_str(), off["ext_flag"].as_bool()),
            (Some("required"), Some(false))
        );

        std::fs::write(dir.join("ext.toml"), "bad").unwrap();
        let bad = foreground::inspect_in(registry, ID, &env);
        assert_eq!(bad["status"], "unknown");
        assert_eq!(bad["inspection_error"], "ext config is invalid");
        assert!(bad.get("user_config").is_none(), "{bad}");

        let rootless = setup_env(Default::default(), &dir);
        let unknown = foreground::inspect_in(registry, ID, &rootless);
        assert_eq!(unknown["status"], "unknown");
        assert_eq!(
            unknown["inspection_error"],
            "user config directory is unknown"
        );
        assert_eq!(unknown["settings_file"], Value::Null);

        assert_eq!(
            foreground::inspect_in(registry, "unregistered", &env),
            Value::Null
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    struct Fixture {
        dir: PathBuf,
        kind: SettingsKind,
        config: PathBuf,
        manifest: PathBuf,
        /// The baseline setup read when it first ran: re-running setup passes it again.
        base: Vec<u8>,
    }
    impl Fixture {
        /// The settings file and manifest exactly where the generic plumbing puts them: under
        /// the registered adapter's resolved config root and the state directory.
        fn new(spec: &'static SettingsKindSpec, base: Option<&str>) -> Self {
            let registry = registry(spec);
            let dir = scratch("engine");
            let roots = default_config_roots(registry, &environment(None, Some(&dir.join("root"))));
            let env = setup_env(roots, &dir.join("state"));
            let kind = legacy::registered_kind_in(registry, ID).expect("registered kind");
            let (config, manifest) = legacy::settings_paths(&env, kind).unwrap();
            assert_eq!(config, dir.join("root").join(spec.file_name));
            assert_eq!(
                manifest.parent().unwrap(),
                dir.join("state").join("setup"),
                "manifest under the state directory"
            );
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
            if let Some(base) = base {
                std::fs::write(&config, base).unwrap();
            }
            Self {
                dir,
                kind,
                config,
                manifest,
                base: base.unwrap_or_default().as_bytes().to_vec(),
            }
        }
        fn argv() -> Vec<String> {
            vec!["/x/herdr-threads".to_owned()]
        }
        fn install(&self) -> Result<crate::harness::setup::OwnershipManifest, SetupError> {
            install_user_settings(
                self.kind,
                &self.config,
                &self.manifest,
                &Self::argv(),
                &self.base,
            )
        }
        fn installed(&self) -> bool {
            inspect_user_settings(
                self.kind,
                &self.config,
                &self.manifest,
                NativeObservation::Unknown,
            )
            .unwrap()
            .installed
        }
        fn value(&self) -> Value {
            serde_json::from_slice(&std::fs::read(&self.config).unwrap()).unwrap()
        }
        fn rewrite(&self, value: Value) {
            std::fs::write(&self.config, serde_json::to_vec(&value).unwrap()).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Kills: a member the user already held that is recorded as owned (and removed), an absent
    /// `permissions.allow` that is not created or not pruned again, a native rewrite that keeps
    /// the member but reads as not installed, and a rewrite that drops the member but reads as
    /// installed or is not re-added by the next install.
    #[test]
    fn install_status_and_unsetup_run_the_generic_engine_for_a_registered_kind() {
        // Pre-existing: recorded, untouched, never removed.
        let base = format!(r#"{{"permissions":{{"allow":["{MEMBER}"]}},"keep":1}}"#);
        let held = Fixture::new(&MEMBER_KIND, Some(&base));
        let manifest = held.install().unwrap();
        assert!(manifest.owned.is_empty() && manifest.phase == InstallPhase::Installed);
        assert!(
            manifest
                .permission
                .as_ref()
                .is_some_and(|p| p.pre_existing && p.rule == MEMBER)
        );
        assert_eq!(std::fs::read(&held.config).unwrap(), base.as_bytes());
        assert!(held.installed());
        remove_user_settings(held.kind, &held.config, &held.manifest).unwrap();
        assert_eq!(std::fs::read(&held.config).unwrap(), base.as_bytes());
        assert!(read_settings_manifest(&held.manifest).unwrap().is_none());

        // Array absent: created by install, pruned by unsetup.
        let absent = Fixture::new(&MEMBER_KIND, Some(r#"{"keep":1}"#));
        let manifest = absent.install().unwrap();
        assert!(
            manifest
                .permission
                .as_ref()
                .is_some_and(|p| !p.pre_existing)
        );
        assert_eq!(absent.value()["permissions"]["allow"], json!([MEMBER]));
        assert!(absent.installed());
        remove_user_settings(absent.kind, &absent.config, &absent.manifest).unwrap();
        assert_eq!(absent.value(), json!({"keep": 1}));

        // Native rewrite that keeps the member (keys reordered, a key added): still installed.
        // A rewrite that drops it reads as not installed; the next install re-adds the member.
        let rewritten = Fixture::new(&MEMBER_KIND, Some(r#"{"keep":1}"#));
        rewritten.install().unwrap();
        rewritten
            .rewrite(json!({"added": true, "permissions": {"allow": [MEMBER, "Read"]}, "keep": 1}));
        assert!(
            rewritten.installed(),
            "a rewrite keeping the member stays installed"
        );
        rewritten.rewrite(json!({"added": true, "keep": 1}));
        assert!(
            !rewritten.installed(),
            "a rewrite dropping the member is not installed"
        );
        rewritten.install().unwrap();
        assert!(rewritten.installed());
        assert!(
            rewritten.value()["permissions"]["allow"]
                .as_array()
                .unwrap()
                .iter()
                .any(|rule| rule == MEMBER)
        );
        remove_user_settings(rewritten.kind, &rewritten.config, &rewritten.manifest).unwrap();
        assert!(
            !rewritten.value()["permissions"]["allow"]
                .as_array()
                .is_some_and(|allow| allow.iter().any(|rule| rule == MEMBER))
        );
    }

    /// Kills: `plan_settings` that only knows two adapters' groups, an edited owned group that
    /// is accepted instead of refused, or a registered hook kind whose install leaves nothing
    /// to remove.
    #[test]
    fn a_registered_hook_kind_plans_installs_and_uninstalls_through_the_generic_paths() {
        let argv = Fixture::argv();
        let plan = plan_settings(SettingsKind::new(&HOOK_KIND), b"{}", &argv).unwrap();
        assert_eq!(plan.owned.len(), 1);
        assert_eq!(plan.owned[0].event, "SessionStart");
        let again =
            plan_settings(SettingsKind::new(&HOOK_KIND), &plan.proposed_bytes, &argv).unwrap();
        assert_eq!(
            again.proposed_bytes, plan.proposed_bytes,
            "composition is idempotent"
        );
        let mut edited: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
        edited["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(1);
        assert_eq!(
            plan_settings(
                SettingsKind::new(&HOOK_KIND),
                &serde_json::to_vec(&edited).unwrap(),
                &argv
            ),
            Err(SetupError::Conflict)
        );

        let fixture = Fixture::new(&HOOK_KIND, Some(r#"{"keep":1}"#));
        let manifest = fixture.install().unwrap();
        assert_eq!(manifest.owned.len(), 1);
        assert!(manifest.permission.is_none());
        assert!(fixture.installed());
        assert_eq!(fixture.value()["keep"], 1);
        remove_user_settings(fixture.kind, &fixture.config, &fixture.manifest).unwrap();
        assert_eq!(fixture.value().get("hooks"), None);
        assert!(!fixture.installed());
    }
}

fn backups(dir: &std::path::Path) -> Vec<(String, Vec<u8>, u32)> {
    let mut found: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".herdr-threads"))
        .map(|path| {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read(&path).unwrap(), mode)
        })
        .collect();
    found.sort();
    found
}

/// Kills a replacement that loses the prior bytes, exposes them, or changes the file's mode.
#[test]
fn user_config_backup_keeps_exact_prior_bytes_privately() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let config = dir.join("settings.json");
    fs::write(&config, b"{\"a\":1}").unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
    write_user_config(&config, b"{\"a\":1}", b"{\"a\":2}").unwrap();
    assert_eq!(fs::read(&config).unwrap(), b"{\"a\":2}");
    assert_eq!(
        fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let found = backups(&dir);
    assert_eq!(found.len(), 1, "{found:?}");
    let (name, bytes, mode) = &found[0];
    assert_eq!(bytes, b"{\"a\":1}");
    assert_eq!(*mode, 0o600);
    // settings.json.<YYYYMMDDTHHMMSSZ>-<uuid>.herdr-threads
    let stamp = name
        .strip_prefix("settings.json.")
        .and_then(|rest| rest.strip_suffix(".herdr-threads"))
        .unwrap();
    let (time, id) = stamp.split_at(16);
    assert!(time.ends_with('Z') && time.as_bytes()[8] == b'T', "{name}");
    assert!(uuid::Uuid::parse_str(&id[1..]).is_ok(), "{name}");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    fs::remove_dir_all(dir).unwrap();
}

/// Kills a backup or write for an unchanged file, and a write over bytes the caller did not see.
#[test]
fn user_config_unchanged_or_stale_writes_nothing() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let config = dir.join("hooks.json");
    fs::write(&config, b"{}").unwrap();
    write_user_config(&config, b"{}", b"{}").unwrap();
    assert!(backups(&dir).is_empty());
    assert_eq!(
        write_user_config(&config, b"{\"seen\":true}", b"{\"x\":1}"),
        Err(SetupError::Conflict)
    );
    assert_eq!(fs::read(&config).unwrap(), b"{}");
    assert!(backups(&dir).is_empty());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    fs::remove_dir_all(dir).unwrap();
}

/// Kills a real hook writer that publishes or removes without keeping each prior version.
#[test]
fn hook_install_and_removal_back_up_each_prior_version() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    let scope = dir.join(".claude");
    fs::create_dir_all(&scope).unwrap();
    let config = scope.join("settings.json");
    let manifest = dir.join("manifest.json");
    let original = br#"{"permissions":{"deny":["Bash(rm *)"]}}"#;
    fs::write(&config, original).unwrap();
    let argv = vec!["/tmp/herdr-threads".into(), "hook".into()];
    install_claude_user(&config, &manifest, &argv, original).unwrap();
    let installed = fs::read(&config).unwrap();
    assert_ne!(installed, original);
    assert_eq!(
        backups(&scope)
            .into_iter()
            .map(|(_, bytes, _)| bytes)
            .collect::<Vec<_>>(),
        [original.to_vec()]
    );
    remove_claude_user(&config, &manifest).unwrap();
    assert!(backups(&scope).iter().all(|(_, _, mode)| *mode == 0o600));
    let mut saved: Vec<_> = backups(&scope)
        .into_iter()
        .map(|(_, bytes, _)| bytes)
        .collect();
    saved.sort();
    let mut expected = vec![original.to_vec(), installed];
    expected.sort();
    assert_eq!(saved, expected);
    fs::remove_dir_all(dir).unwrap();
}

/// Kills skipping the backup of an existing file that holds no settings: every pre-image is kept.
#[test]
fn user_config_without_settings_is_still_backed_up() {
    let dir = std::env::temp_dir().join(format!("herdr-setup-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let config = dir.join("settings.json");
    for empty in [&b""[..], b"{}", b" {\n}\n"] {
        fs::write(&config, empty).unwrap();
        write_user_config(&config, empty, b"{\"a\":1}").unwrap();
        assert_eq!(fs::read(&config).unwrap(), b"{\"a\":1}");
    }
    let mut expected = vec![b"".to_vec(), b"{}".to_vec(), b" {\n}\n".to_vec()];
    expected.sort();
    assert_eq!(user_config_backups(&config), expected);
    fs::remove_dir_all(dir).unwrap();
}

/// Stable reinstall must reuse exact owned groups at their current positions. A UUID belongs
/// in the private manifest, never in native command/trust-key identity.
#[test]
fn stable_owned_reinstall_preserves_identity_positions_and_foreign_groups() {
    for (kind, harness) in [
        (crate::harness::claude::settings_kind(), "claude"),
        (crate::harness::codex::settings_kind(), "codex"),
    ] {
        let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join(kind.file_name());
        let manifest_path = dir.join("ownership.json");
        let base = br#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"before"}]}]},"unrelated":17}"#;
        fs::write(&config, base).unwrap();
        let argv = vec![
            "/x/herdr-threads".into(),
            "--state-dir".into(),
            "/route state".into(),
            "--host-endpoint".into(),
            "/route.sock".into(),
            "hook".into(),
            harness.into(),
        ];
        let first = install_user_settings(kind, &config, &manifest_path, &argv, base).unwrap();
        assert!(!first.installation_id.is_empty());
        for entry in &first.owned {
            assert_eq!(
                command_of(entry),
                format!(
                    "'/x/herdr-threads' '--state-dir' '/route state' '--host-endpoint' '/route.sock' 'hook-event' '{harness}' '{}'",
                    entry.event
                )
            );
        }
        let mut changed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        changed["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks":[{"type":"command","command":"after"}]}));
        let current = serde_json::to_vec(&changed).unwrap();
        fs::write(&config, &current).unwrap();
        let again = install_user_settings(kind, &config, &manifest_path, &argv, base).unwrap();
        assert_eq!(again.installation_id, first.installation_id);
        assert_eq!(again.owned, first.owned);
        assert_eq!(fs::read(&config).unwrap(), current);
        assert!(
            inspect_user_settings(kind, &config, &manifest_path, NativeObservation::Unknown)
                .unwrap()
                .installed
        );
        remove_user_settings(kind, &config, &manifest_path).unwrap();
        let removed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(removed["unrelated"], 17);
        assert_eq!(
            removed["hooks"]["SessionStart"].as_array().unwrap().len(),
            2
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}

/// Exact identical commands without this file's manifest remain foreign. Setup and status
/// cannot steal them; a recorded owned group edited by its user cannot be removed.
#[test]
fn stable_foreign_identical_and_owned_drift_are_preserved() {
    for (kind, harness) in [
        (crate::harness::claude::settings_kind(), "claude"),
        (crate::harness::codex::settings_kind(), "codex"),
    ] {
        let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join(kind.file_name());
        let manifest_path = dir.join("ownership.json");
        let argv = vec!["/x/herdr-threads".into(), "hook".into(), harness.into()];
        fs::write(&config, b"{}").unwrap();
        let first = install_user_settings(kind, &config, &manifest_path, &argv, b"{}").unwrap();
        let foreign = fs::read(&config).unwrap();
        fs::remove_file(&manifest_path).unwrap();
        assert_eq!(
            install_user_settings(kind, &config, &manifest_path, &argv, &foreign),
            Err(SetupError::Conflict)
        );
        assert!(!manifest_path.exists());
        assert_eq!(fs::read(&config).unwrap(), foreign);
        assert!(
            !inspect_user_settings_for(
                kind,
                &config,
                &manifest_path,
                NativeObservation::Unknown,
                Some(&argv)
            )
            .unwrap()
            .installed
        );
        fs::write(&manifest_path, serde_json::to_vec(&first).unwrap()).unwrap();
        for index in [0, 1] {
            let mut duplicate: Value = serde_json::from_slice(&foreign).unwrap();
            let group = duplicate["hooks"]["SessionStart"][0].clone();
            duplicate["hooks"]["SessionStart"]
                .as_array_mut()
                .unwrap()
                .insert(index, group);
            let bytes = serde_json::to_vec(&duplicate).unwrap();
            fs::write(&config, &bytes).unwrap();
            assert_eq!(
                install_user_settings(kind, &config, &manifest_path, &argv, b"{}"),
                Err(SetupError::Conflict)
            );
            assert_eq!(
                remove_user_settings(kind, &config, &manifest_path),
                Err(SetupError::Conflict)
            );
            assert!(
                !inspect_user_settings(kind, &config, &manifest_path, NativeObservation::Unknown)
                    .unwrap()
                    .installed
            );
            assert_eq!(fs::read(&config).unwrap(), bytes);
        }
        let mut drift: Value = serde_json::from_slice(&foreign).unwrap();
        drift["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(99);
        let drift = serde_json::to_vec(&drift).unwrap();
        fs::write(&config, &drift).unwrap();
        assert_eq!(
            install_user_settings(kind, &config, &manifest_path, &argv, b"{}"),
            Err(SetupError::Conflict)
        );
        assert_eq!(
            remove_user_settings(kind, &config, &manifest_path),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), drift);
        fs::remove_dir_all(&dir).unwrap();
    }
}

/// Seed a genuinely recorded historical installation, rather than depending on today's
/// producer to emit yesterday's bytes. Manifest and every group fingerprint agree.
fn seed_uuid_registration(
    config: &std::path::Path,
    manifest_path: &std::path::Path,
    evented: bool,
) -> OwnershipManifest {
    let mut manifest = read_manifest_file(manifest_path);
    let harness = manifest.harness.clone();
    let mut settings: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    for entry in &mut manifest.owned {
        let old_group = entry.group.clone();
        let event = if evented {
            format!(" '--event' '{}'", entry.event)
        } else {
            String::new()
        };
        entry.group["hooks"][0]["command"] = json!(format!(
            "'/x/herdr-threads' 'hook' '{harness}'{event} # herdr-threads-owner:{}",
            manifest.installation_id
        ));
        entry.fingerprint = group_fingerprint(&entry.group);
        let groups = settings["hooks"][&entry.event].as_array_mut().unwrap();
        let index = groups.iter().position(|g| g == &old_group).unwrap();
        groups[index] = entry.group.clone();
    }
    manifest.version = 2;
    let bytes = serde_json::to_vec(&settings).unwrap();
    manifest.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    fs::write(config, bytes).unwrap();
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    manifest
}

/// Only explicit setup migrates old owned groups. Durable intent/publication interruptions
/// recover with the same identity and preserve unrelated groups in their original order.
#[test]
fn stable_uuid_and_shared_migration_recovers_and_preserves_order() {
    for kind in [
        crate::harness::claude::settings_kind(),
        crate::harness::codex::settings_kind(),
    ] {
        for evented in [false, true] {
            for fault in [
                InstallFault::None,
                InstallFault::InterruptAfterIntent,
                InstallFault::InterruptAfterPublication,
            ] {
                let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
                fs::create_dir(&dir).unwrap();
                let config = dir.join(kind.file_name());
                let manifest_path = dir.join("ownership.json");
                let argv = vec![
                    "/x/herdr-threads".into(),
                    "hook".into(),
                    kind.harness().into(),
                ];
                fs::write(&config, b"{}").unwrap();
                install_user_settings(kind, &config, &manifest_path, &argv, b"{}").unwrap();
                let legacy = seed_uuid_registration(&config, &manifest_path, evented);
                let mut settings: Value =
                    serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                let groups = settings["hooks"]["SessionStart"].as_array_mut().unwrap();
                groups.insert(0, json!({"hooks":[{"type":"command","command":"before"}]}));
                groups.push(json!({"hooks":[{"type":"command","command":"after"}]}));
                settings["unrelated"] = json!({"keep":true});
                fs::write(&config, serde_json::to_vec(&settings).unwrap()).unwrap();
                let before = fs::read(&config).unwrap();
                let status = inspect_user_settings(
                    kind,
                    &config,
                    &manifest_path,
                    NativeObservation::Unknown,
                )
                .unwrap();
                assert!(status.installed);
                assert_eq!(fs::read(&config).unwrap(), before, "status cannot migrate");
                let result = install_user_settings_with_fault(
                    kind,
                    &config,
                    &manifest_path,
                    &argv,
                    b"{}",
                    fault,
                    || {},
                );
                if fault != InstallFault::None {
                    assert_eq!(result, Err(SetupError::Io));
                }
                let migrated =
                    install_user_settings(kind, &config, &manifest_path, &argv, b"{}").unwrap();
                assert_eq!(migrated.installation_id, legacy.installation_id);
                assert!(migrated.superseded.is_empty());
                for entry in &migrated.owned {
                    assert_eq!(
                        command_of(entry),
                        format!(
                            "'/x/herdr-threads' 'hook-event' '{}' '{}'",
                            kind.harness(),
                            entry.event
                        )
                    );
                }
                let current: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                assert_eq!(
                    current["hooks"]["SessionStart"][0],
                    settings["hooks"]["SessionStart"][0]
                );
                assert_eq!(
                    current["hooks"]["SessionStart"][2],
                    settings["hooks"]["SessionStart"][2]
                );
                remove_user_settings(kind, &config, &manifest_path).unwrap();
                let removed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
                assert_eq!(removed["unrelated"], json!({"keep":true}));
                assert_eq!(
                    removed["hooks"]["SessionStart"].as_array().unwrap().len(),
                    2
                );
                fs::remove_dir_all(&dir).unwrap();
            }
        }
    }
}

#[test]
fn stable_codex_layer_command_is_identified_without_adoption() {
    let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&dir).unwrap();
    let config = dir.join("config.toml");
    fs::write(&config, "[[hooks.SessionStart]]\nhooks = [{ type = \"command\", command = \"'/x/herdr-threads' '--state-dir' '/route' 'hook-event' 'codex' 'SessionStart'\" }]\n").unwrap();
    let matching = crate::harness::codex::setup::observe_codex_layer(
        &config,
        "'/x/herdr-threads' '--state-dir' '/route' 'hook' 'codex'",
    )
    .unwrap();
    assert!(
        matching.has_owned_command,
        "an identical registered event in another layer would execute twice"
    );
    let other = crate::harness::codex::setup::observe_codex_layer(
        &config,
        "'/x/herdr-threads' '--state-dir' '/other' 'hook' 'codex'",
    )
    .unwrap();
    assert!(!other.has_owned_command);
    assert!(other.has_other_herdr_threads_hook);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stable_same_event_foreign_route_conflicts_without_config_changes() {
    for kind in [
        crate::harness::claude::settings_kind(),
        crate::harness::codex::settings_kind(),
    ] {
        let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join(kind.file_name());
        let manifest = dir.join("ownership.json");
        let base = serde_json::to_vec(&json!({"unrelated": 19, "hooks":{"SessionStart":[{"hooks":[{"type":"command","command":format!("'/x/herdr-threads' '--state-dir' '/foreign' 'hook-event' '{}' 'SessionStart'", kind.harness())}]}]}})).unwrap();
        fs::write(&config, &base).unwrap();
        let argv = vec![
            "/x/herdr-threads".into(),
            "--state-dir".into(),
            "/route".into(),
            "hook".into(),
            kind.harness().into(),
        ];
        assert_eq!(
            install_user_settings(kind, &config, &manifest, &argv, &base),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), base);
        assert!(!manifest.exists());
        fs::remove_dir_all(dir).unwrap();
    }
    for kind in [
        crate::harness::claude::settings_kind(),
        crate::harness::codex::settings_kind(),
    ] {
        let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join(kind.file_name());
        let manifest = dir.join("ownership.json");
        let argv = vec![
            "/x/herdr-threads".into(),
            "--state-dir".into(),
            "/route".into(),
            "hook".into(),
            kind.harness().into(),
        ];
        fs::write(&config, b"{}").unwrap();
        install_user_settings(kind, &config, &manifest, &argv, b"{}").unwrap();
        let recorded = fs::read(&manifest).unwrap();
        let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        value["hooks"]["SessionStart"].as_array_mut().unwrap().push(json!({"hooks":[{"type":"command","command":format!("'/x/herdr-threads' '--state-dir' '/foreign' 'hook-event' '{}' 'SessionStart'", kind.harness())}]}));
        let current = serde_json::to_vec(&value).unwrap();
        fs::write(&config, &current).unwrap();
        assert_eq!(
            install_user_settings(kind, &config, &manifest, &argv, b"{}"),
            Err(SetupError::Conflict)
        );
        assert_eq!(fs::read(&config).unwrap(), current);
        assert_eq!(fs::read(&manifest).unwrap(), recorded);
        let status_manifest =
            legacy::manifest_path(&dir, &format!("{}-user", kind.harness()), &config);
        fs::create_dir_all(status_manifest.parent().unwrap()).unwrap();
        fs::write(&status_manifest, &recorded).unwrap();
        let env = crate::cli::setup::SetupEnv {
            home: Some(dir.as_os_str().to_owned()),
            declared_environment: Default::default(),
            executable: "/x/herdr-threads".into(),
            state_dir: Some(dir.clone()),
            cwd: dir.clone(),
            path: None,
            config_roots: [
                ("codex".into(), dir.clone()),
                ("claude".into(), dir.clone()),
            ]
            .into(),
            host_endpoint: Some(dir.join("host.sock")),
            instance_source: Value::Null,
        };
        let mut status = json!({});
        legacy::settings_status(kind, &env, &config, &mut status).unwrap();
        assert_eq!(
            status["installed"], true,
            "the exact owned registration still exists"
        );
        assert_eq!(status["registration_conflict"], true, "{status}");
        assert_eq!(
            fs::read(&config).unwrap(),
            current,
            "status must only inspect"
        );
        assert_eq!(fs::read(&manifest).unwrap(), recorded);
        // Unsetup can still remove this installation's exact groups without touching the foreign route.
        remove_user_settings(kind, &config, &manifest).unwrap();
        let remaining: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            remaining["hooks"]["SessionStart"].as_array().unwrap().len(),
            1
        );
        assert!(
            remaining["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("'/foreign'")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn stable_quoted_owner_marker_text_is_not_an_ownership_suffix() {
    let (dir, config, manifest) = claude_scope();
    fs::write(&config, b"{}").unwrap();
    let argv = vec![
        "/x/ # herdr-threads-owner:literal/threads".into(),
        "--state-dir".into(),
        "/state/ # herdr-threads-owner:literal".into(),
        "hook".into(),
        "claude".into(),
    ];
    let first = install_user_settings(
        crate::harness::claude::settings_kind(),
        &config,
        &manifest,
        &argv,
        b"{}",
    )
    .unwrap();
    let bytes = fs::read(&config).unwrap();
    let again = install_user_settings(
        crate::harness::claude::settings_kind(),
        &config,
        &manifest,
        &argv,
        b"{}",
    )
    .unwrap();
    assert_eq!(again.owned, first.owned);
    assert_eq!(again.installation_id, first.installation_id);
    assert_eq!(fs::read(&config).unwrap(), bytes);
    remove_user_settings(crate::harness::claude::settings_kind(), &config, &manifest).unwrap();
    assert_eq!(fs::read(&config).unwrap(), b"{}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stable_v1_shared_owned_migration_after_intent_recovers() {
    for kind in [
        crate::harness::claude::settings_kind(),
        crate::harness::codex::settings_kind(),
    ] {
        let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join(kind.file_name());
        let manifest_path = dir.join("ownership.json");
        let argv = vec![
            "/x/herdr-threads".into(),
            "hook".into(),
            kind.harness().into(),
        ];
        fs::write(&config, b"{}").unwrap();
        install_user_settings(kind, &config, &manifest_path, &argv, b"{}").unwrap();
        let mut legacy = seed_uuid_registration(&config, &manifest_path, false);
        let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        for entry in &mut legacy.owned {
            let command = format!("'/x/herdr-threads' 'hook' '{}'", kind.harness());
            entry.group["hooks"][0]["command"] = json!(command);
            entry.fingerprint = group_fingerprint(&entry.group);
            value["hooks"][&entry.event] = json!([entry.group]);
        }
        legacy.version = 1;
        legacy.installation_id.clear();
        let bytes = serde_json::to_vec(&value).unwrap();
        legacy.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
        fs::write(&config, bytes).unwrap();
        fs::write(&manifest_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(
            install_user_settings_with_fault(
                kind,
                &config,
                &manifest_path,
                &argv,
                b"{}",
                InstallFault::InterruptAfterIntent,
                || {}
            ),
            Err(SetupError::Io)
        );
        let migrated = install_user_settings(kind, &config, &manifest_path, &argv, b"{}").unwrap();
        assert!(!migrated.installation_id.is_empty());
        assert_eq!(migrated.version, 3);
        remove_user_settings(kind, &config, &manifest_path).unwrap();
        assert_eq!(fs::read(&config).unwrap(), b"{}");
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn stable_migration_result_is_not_reported_unchanged_and_warns_for_retrust() {
    use crate::cli::setup::{SetupEnv, SetupVerb};
    use crate::harness::setup::legacy::{OwnedFile, install_settings};
    for recorded in [true, false] {
        let dir = std::env::temp_dir().join(format!("ht-task9-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join("hooks.json");
        let manifest_path = dir.join("ownership.json");
        let env = SetupEnv {
            home: Some(dir.as_os_str().to_owned()),
            declared_environment: Default::default(),
            executable: "/x/herdr-threads".into(),
            state_dir: Some(dir.clone()),
            cwd: dir.clone(),
            path: None,
            config_roots: [("codex".into(), dir.clone())].into(),
            host_endpoint: Some(dir.join("host.sock")),
            instance_source: Value::Null,
        };
        let argv = env
            .hook_argv(crate::harness::context::Harness::Codex)
            .unwrap();
        fs::write(&config, b"{}").unwrap();
        let mut legacy = install_user_settings(
            crate::harness::codex::settings_kind(),
            &config,
            &manifest_path,
            &argv,
            b"{}",
        )
        .unwrap();
        let mut value: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        for entry in &mut legacy.owned {
            let command = format!(
                "{} '--event' '{}' # herdr-threads-owner:{}",
                shell_command(&argv).unwrap(),
                entry.event,
                legacy.installation_id
            );
            entry.group["hooks"][0]["command"] = json!(command);
            entry.fingerprint = group_fingerprint(&entry.group);
            value["hooks"][&entry.event] = json!([entry.group]);
        }
        legacy.version = 2;
        let before = serde_json::to_vec(&value).unwrap();
        legacy.installed_fingerprint = format!("sha256:{:x}", sha2::Sha256::digest(&before));
        fs::write(&config, &before).unwrap();
        if recorded {
            fs::write(&manifest_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        } else {
            fs::remove_file(&manifest_path).unwrap();
        }
        let mut file = OwnedFile::new(config.clone(), manifest_path.clone(), b"{}");
        let mut warnings = Vec::new();
        let (migrated, already, adopted_unchanged) = install_settings(
            crate::harness::codex::settings_kind(),
            SetupVerb::Install,
            &env,
            &mut file,
            &mut warnings,
        )
        .unwrap();
        assert_ne!(fs::read(&config).unwrap(), before);
        assert_eq!(migrated.installation_id, legacy.installation_id);
        assert!(
            !already && !adopted_unchanged,
            "changed registrations cannot claim unchanged results: {already}/{adopted_unchanged}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("rewritten hook commands need")),
            "{warnings:?}"
        );
        remove_user_settings(
            crate::harness::codex::settings_kind(),
            &config,
            &manifest_path,
        )
        .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn stable_legacy_marker_boundary_ignores_quoted_path_text() {
    let argv = vec![
        "/x/ # herdr-threads-owner:quoted".into(),
        "hook".into(),
        "claude".into(),
    ];
    let base = shell_command(&argv).unwrap();
    let suffix = " # herdr-threads-owner:0b5e8f3c-1111-4222-8333-444455556666";
    assert_eq!(
        event_command(&base, "SessionStart"),
        format!("{base} '--event' 'SessionStart'")
    );
    let marked = format!("{base}{suffix}");
    let evented = format!("{base} '--event' 'SessionStart'{suffix}");
    assert_eq!(event_command(&marked, "SessionStart"), evented);
    assert_eq!(base_command(&evented, "SessionStart"), marked);
}
