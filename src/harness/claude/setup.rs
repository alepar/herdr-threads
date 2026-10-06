//! Owned Claude local setup backend.
use crate::cli::{
    RunError,
    setup::{PromptSuggestionPolicy, SetupEnv, SetupRequest, SetupVerb},
};
use crate::harness::setup::legacy::*;
use crate::harness::{
    claude,
    context::Harness,
    prompt_suggestion::{self, SuggestionState},
    recipe,
    setup::{
        AllowRuleInspection, AllowRuleOwnership, NativeObservation, SettingsKind, SetupError,
        inspect_user_settings, read_settings_manifest, remove_user_settings, shared_command,
    },
};
use crate::protocol::results::ErrorCode;
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
pub const PROMPT_SUGGESTION_EXPLANATION: &str = "Claude Code shows a dim prompt suggestion in \
its input box after every turn (on by default). herdr-threads cannot tell that suggestion from \
text you typed, so it never sends a soft-deadline reminder (poke) into a Claude pane that shows \
one; only the hard-deadline warning reaches it. With prompt suggestions off \
(`promptSuggestionEnabled: false` in Claude's user settings), an idle Claude pane reads empty and \
pokes reach it. unsetup reverts the setting if setup set it.";

/// The non-interactive advice (nothing was changed).
pub(crate) fn prompt_suggestion_advice(settings: &Path) -> String {
    format!(
        "{PROMPT_SUGGESTION_EXPLANATION} Prompt suggestions are on in {} and were left \
         unchanged: re-run `herdr-threads setup claude --disable-prompt-suggestions` to turn them \
         off (or turn off Prompt suggestions in Claude's /config), or pass \
         --keep-prompt-suggestions to keep them without this note",
        settings.display()
    )
}

pub fn claude_config_dir_from(
    config_dir: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    match config_dir.filter(|value| !value.is_empty()) {
        Some(value) => Some(PathBuf::from(value)),
        None => home
            .filter(|value| !value.is_empty())
            .map(|home| PathBuf::from(home).join(".claude")),
    }
}

/// Ownership-manifest location for one owned file: private plugin state,
/// keyed by the file's path, never beside the file.
pub(crate) fn claude_manifest(env: &SetupEnv, settings: &Path) -> Result<PathBuf, RunError> {
    Ok(manifest_path(env.state_dir()?, "claude-user", settings))
}

/// The user-level Claude installation's file and manifest paths.
pub fn claude_paths(env: &SetupEnv) -> Result<(PathBuf, PathBuf), RunError> {
    let settings = env.claude_settings()?;
    let manifest = claude_manifest(env, &settings)?;
    Ok((settings, manifest))
}

pub(crate) fn claude_install(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    let settings = env.claude_settings()?;
    env.hook_argv(Harness::Claude)?;
    let (observed, _) = observe(request, env).map_err(refuse_executable)?;
    prepare_state(env)?;
    let manifest = claude_manifest(env, &settings)?;
    let mut file = OwnedFile::new(settings.clone(), manifest.clone(), b"{}");
    let mut warnings = Vec::new();
    let (installed, already, adopted) = install_settings(
        SettingsKind::ClaudeUser,
        request.verb,
        env,
        &mut file,
        &mut warnings,
    )?;
    let allow_rule = inspect_user_settings(
        SettingsKind::ClaudeUser,
        &settings,
        &manifest,
        NativeObservation::Unknown,
    )
    .ok()
    .and_then(|inspection| inspection.allow_rule);
    let command = shared_command(&installed.owned).unwrap_or_default();
    let prompt_suggestions =
        prompt_suggestion_step(request.prompt_suggestions, env, &settings, &mut warnings)?;
    Ok(json!({
        "action": if adopted { "adopted" } else if already { "already_installed" } else { "installed" },
        "adopted": installed.adopted.then(|| installed.installation_id.clone()),
        "harness": "claude",
        "scope": "user",
        "settings": settings.display().to_string(),
        "manifest": manifest.display().to_string(),
        "created_settings": file.created_file,
        "instance": instance_json(env),
        "hook_argv": env.hook_argv(Harness::Claude)?,
        "command": command,
        "events": installed.owned.iter().map(|entry| entry.event.clone()).collect::<Vec<_>>(),
        "allow_rule": allow_rule_json(allow_rule.as_ref()),
        "harness_version": observation_json(&Ok((observed, None))),
        "prompt_suggestions": prompt_suggestions,
        "observed": "unknown",
        "note": "installed is not observed: run `herdr-threads doctor` for native evidence. \
                 Claude sessions that run with another CLAUDE_CONFIG_DIR, or with \
                 --setting-sources excluding `user`, do not load these hooks",
        "warnings": warnings,
    }))
}

/// Report form of the recorded allow rule; `null` when no manifest is recorded.
pub fn allow_rule_json(allow_rule: Option<&AllowRuleInspection>) -> Value {
    match allow_rule {
        None => Value::Null,
        Some(rule) => {
            let mut report = json!({
                "rule": rule.rule,
                "ownership": match rule.ownership {
                    AllowRuleOwnership::Owned => "owned",
                    AllowRuleOwnership::PreExisting => "pre_existing",
                    AllowRuleOwnership::NotRecorded => "not_recorded",
                    AllowRuleOwnership::Superseded => "superseded",
                },
                "present": rule.present,
            });
            if rule.ownership == AllowRuleOwnership::Superseded {
                report["note"] = json!(format!(
                    "recorded with the retired rule `{}`; re-run `herdr-threads setup claude` to \
                     replace it with `{}`",
                    claude::CALLER_CONTEXT_ALLOW_RULE,
                    rule.rule
                ));
            }
            report
        }
    }
}

/// Where setup records that it set Claude's prompt-suggestion key.
pub fn prompt_suggestion_manifest(env: &SetupEnv, settings: &Path) -> Result<PathBuf, RunError> {
    Ok(manifest_path(
        env.state_dir()?,
        "claude-prompt-suggestion",
        settings,
    ))
}

pub(crate) fn prompt_suggestion_error(error: SetupError, settings: &Path) -> RunError {
    let settings = settings.display();
    match error {
        SetupError::Invalid => invalid(format!(
            "{settings} is not a JSON object, or the prompt-suggestion record is a symlink or \
             damaged; prompt suggestions were not changed"
        )),
        SetupError::TooLarge => invalid(format!(
            "{settings} or the prompt-suggestion record exceeds the setup size bound; prompt \
             suggestions were not changed"
        )),
        SetupError::Conflict => api(
            ErrorCode::Conflict,
            format!(
                "{settings} changed while setup edited `{}`, or the prompt-suggestion record \
                 names another file; prompt suggestions were not changed. Re-run the command",
                claude::PROMPT_SUGGESTION_SETTING
            ),
        ),
        SetupError::Io => failed(format!(
            "could not read or replace {settings} or the prompt-suggestion record"
        )),
    }
}

/// The report form of the prompt-suggestion setting after `action`.
pub(crate) fn prompt_suggestion_json(
    state: SuggestionState,
    action: &str,
    recorded: bool,
) -> Value {
    json!({
        "setting": claude::PROMPT_SUGGESTION_SETTING,
        "state": state.as_str(),
        "action": action,
        "set_by_setup": recorded,
    })
}

/// `setup claude`, after the hooks: apply the policy. `advised` means the
/// setting is on and nothing was changed (an interactive run then asks).
pub(crate) fn prompt_suggestion_step(
    policy: PromptSuggestionPolicy,
    env: &SetupEnv,
    settings: &Path,
    warnings: &mut Vec<String>,
) -> Result<Value, RunError> {
    let manifest = prompt_suggestion_manifest(env, settings)?;
    let error = |e| prompt_suggestion_error(e, settings);
    let recorded = prompt_suggestion::recorded(&manifest)
        .map_err(error)?
        .is_some();
    let state = prompt_suggestion::read_state(settings).map_err(error)?;
    if state == SuggestionState::Disabled {
        return Ok(prompt_suggestion_json(state, "already_disabled", recorded));
    }
    Ok(match policy {
        PromptSuggestionPolicy::Disable => {
            prompt_suggestion::disable(settings, &manifest).map_err(error)?;
            prompt_suggestion_json(SuggestionState::Disabled, "disabled", true)
        }
        PromptSuggestionPolicy::Keep => prompt_suggestion_json(state, "kept", recorded),
        PromptSuggestionPolicy::Ask => {
            warnings.push(prompt_suggestion_advice(settings));
            prompt_suggestion_json(state, "advised", recorded)
        }
    })
}

/// Ask for explicit interactive consent and record only a yes; otherwise keep
/// the suggestion setting untouched. Noninteractive setup only advises.
pub fn settle_prompt_suggestions<R: io::BufRead + ?Sized, W: Write + ?Sized>(
    env: &SetupEnv,
    report: &mut Value,
    input: &mut R,
    out: &mut W,
) -> Result<(), RunError> {
    if report["prompt_suggestions"]["action"] != "advised" {
        return Ok(());
    }
    let settings = env.claude_settings()?;
    writeln!(out, "{PROMPT_SUGGESTION_EXPLANATION}")?;
    write!(out, "Disable prompt suggestions? [y/N] ")?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    let yes = matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    let advice = prompt_suggestion_advice(&settings);
    if let Some(warnings) = report["warnings"].as_array_mut() {
        warnings.retain(|warning| warning.as_str() != Some(advice.as_str()));
    }
    let suggestions = &mut report["prompt_suggestions"];
    if yes {
        let manifest = prompt_suggestion_manifest(env, &settings)?;
        prompt_suggestion::disable(&settings, &manifest)
            .map_err(|e| prompt_suggestion_error(e, &settings))?;
        suggestions["action"] = json!("disabled");
        suggestions["state"] = json!(SuggestionState::Disabled.as_str());
        suggestions["set_by_setup"] = json!(true);
    } else {
        suggestions["action"] = json!("kept");
    }
    Ok(())
}

pub(crate) fn claude_remove(env: &SetupEnv) -> Result<Value, RunError> {
    let (settings, manifest) = claude_paths(env)?;
    let kind = SettingsKind::ClaudeUser;
    // First, so the hook removal below can still restore the file byte for byte.
    let reverted =
        prompt_suggestion::revert(&settings, &prompt_suggestion_manifest(env, &settings)?)
            .map_err(|error| prompt_suggestion_error(error, &settings))?;
    let mut report = json!({
        "harness": "claude",
        "scope": "user",
        "settings": settings.display().to_string(),
        "manifest": manifest.display().to_string(),
        "prompt_suggestions": reverted.as_str(),
    });
    let mut recorded = read_settings_manifest(&manifest)
        .map_err(|e| settings_error(e, SetupVerb::Remove, kind, &settings, &manifest))?;
    if recorded.is_none() {
        recorded = adopt_for_removal(kind, env, &settings, &manifest)?;
    }
    let Some(recorded) = recorded else {
        report["action"] = json!("not_installed");
        report["note"] = json!(
            "no herdr-threads installation is recorded for these settings and this state \
             directory; nothing changed"
        );
        return Ok(report);
    };
    let allow_rule = match recorded.permission.as_ref() {
        Some(permission) if permission.pre_existing => "left_pre_existing",
        Some(_) => "removed",
        None => "not_recorded",
    };
    remove_user_settings(kind, &settings, &manifest)
        .map_err(|e| settings_error(e, SetupVerb::Remove, kind, &settings, &manifest))?;
    report["action"] = json!("removed");
    report["allow_rule"] = json!(allow_rule);
    if recorded.adopted {
        report["adopted"] = json!(recorded.installation_id);
        report["note"] = json!(
            "removed only the adopted hook groups (another setup's owner marker) from this \
             settings file; that setup's own settings file is unchanged"
        );
    }
    report["deleted_created_settings"] = json!(delete_created(&settings, &manifest, b"{}"));
    Ok(report)
}

fn claude_status(
    env: &SetupEnv,
    observation: Result<(Observed, Option<crate::harness::operational::CodexContract>), String>,
) -> Result<Value, RunError> {
    let settings = env.claude_settings()?;
    let mut report = json!({
        "action": "status",
        "harness": "claude",
        "scope": "user",
        "settings": settings.display().to_string(),
        "instance": instance_json(env),
        "recipes": recipe::describe(claude::RECIPES),
        "harness_version": observation_json(&observation),
    });
    settings_status(SettingsKind::ClaudeUser, env, &settings, &mut report)?;
    report["prompt_suggestions"] = prompt_suggestion_status(
        env,
        &settings,
        env.declared_environment
            .get(claude::PROMPT_SUGGESTION_ENV)
            .cloned(),
    );
    Ok(report)
}

/// Claude's prompt-suggestion setting as `setup-status` and `doctor` report
/// it: the user settings value, whether setup set it, and the per-session
/// environment override when one is set. Best effort: an unreadable file is
/// reported, never an error.
pub fn prompt_suggestion_status(
    env: &SetupEnv,
    settings: &Path,
    env_override: Option<OsString>,
) -> Value {
    let state = prompt_suggestion::read_state(settings);
    let recorded = prompt_suggestion_manifest(env, settings)
        .ok()
        .and_then(|manifest| prompt_suggestion::recorded(&manifest).ok().flatten())
        .is_some();
    let mut report = json!({
        "setting": claude::PROMPT_SUGGESTION_SETTING,
        "state": match &state {
            Ok(state) => state.as_str(),
            Err(_) => "unreadable",
        },
        "set_by_setup": recorded,
    });
    if let Some(value) = env_override {
        report["env_override"] = json!(format!(
            "{}={}",
            claude::PROMPT_SUGGESTION_ENV,
            value.to_string_lossy()
        ));
    }
    if state != Ok(SuggestionState::Disabled) {
        report["note"] = json!(
            "Claude soft-deadline pokes skip a pane that shows a prompt suggestion; run \
             `herdr-threads setup claude --disable-prompt-suggestions` to turn them off"
        );
    }
    report
}

pub(crate) fn setup(
    request: &crate::harness::adapter::SetupRequest,
) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
    let policy = if request.options.get("disable-prompt-suggestions") == Some(&true) {
        PromptSuggestionPolicy::Disable
    } else if request.options.get("keep-prompt-suggestions") == Some(&true) {
        PromptSuggestionPolicy::Keep
    } else {
        PromptSuggestionPolicy::Ask
    };
    legacy_adapter_setup(Harness::Claude, request, claude_install, policy)
}
pub(crate) fn status(
    request: &crate::harness::adapter::StatusRequest,
    budget: &crate::protocol::time::CallBudget,
) -> crate::harness::adapter::SetupStatus {
    legacy_adapter_status(Harness::Claude, request, |legacy, env| {
        let timeout = Duration::from_millis(
            budget
                .deadline
                .0
                .saturating_sub(request.environment.clock.monotonic_now().0),
        )
        .min(Duration::from_secs(5));
        claude_status(
            env,
            observe_bounded(legacy, env, None, timeout, &budget.cancellation),
        )
    })
}
pub(crate) fn unsetup(
    request: &crate::harness::adapter::UnsetupRequest,
) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure> {
    legacy_adapter_unsetup(Harness::Claude, request, claude_remove)
}

pub(crate) fn observe(
    request: &SetupRequest,
    env: &SetupEnv,
) -> Result<(Observed, Option<crate::harness::operational::CodexContract>), String> {
    observe_with_cache(request, env, None)
}
pub(crate) fn observe_with_cache(
    request: &SetupRequest,
    env: &SetupEnv,
    _codex_cache: Option<&Path>,
) -> Result<(Observed, Option<crate::harness::operational::CodexContract>), String> {
    observe_bounded(
        request,
        env,
        _codex_cache,
        Duration::from_secs(5),
        &crate::protocol::time::Cancellation::default(),
    )
}

fn observe_bounded(
    request: &SetupRequest,
    env: &SetupEnv,
    _codex_cache: Option<&Path>,
    timeout: Duration,
    cancellation: &crate::protocol::time::Cancellation,
) -> Result<(Observed, Option<crate::harness::operational::CodexContract>), String> {
    if timeout.is_zero() || cancellation.is_cancelled() {
        return Err("native status observation budget exhausted or cancelled".into());
    }
    let binary = harness_binary(request, env)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no executable `claude` on PATH; pass --harness-binary".to_owned())?;
    use std::os::unix::fs::PermissionsExt;
    if !std::fs::metadata(&binary)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    {
        return Err(format!("{} is not an executable file", binary.display()));
    }
    Ok((
        Observed {
            binary,
            version: None,
            recipe: "claude-hooks-2.1.283",
        },
        None,
    ))
}
