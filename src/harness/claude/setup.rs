//! Owned Claude local setup backend.
use crate::cli::{
    RunError,
    setup::{PromptSuggestionPolicy, SetupEnv, SetupRequest, SetupVerb},
};
use crate::harness::setup::legacy::*;
use crate::harness::{
    claude,
    claude_mod::{self, ManagedPolicy, ManagedPolicySources},
    context::Harness,
    prompt_suggestion::{self, SuggestionState},
    recipe,
    setup::{
        AllowRuleInspection, AllowRuleOwnership, NativeObservation, SettingsKind, SetupError,
        inspect_user_settings, inspect_user_settings_for, read_settings_manifest,
        remove_user_settings, shared_command, shell_command,
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

/// The installed hooks are recorded, complete and run this executable's
/// command: a re-run changes nothing.
fn hooks_current(env: &SetupEnv, settings: &Path, manifest: &Path) -> bool {
    let Ok(argv) = env.hook_argv(Harness::Claude) else {
        return false;
    };
    let Ok(expected) = shell_command(&argv) else {
        return false;
    };
    inspect_user_settings_for(
        SettingsKind::ClaudeUser,
        settings,
        manifest,
        NativeObservation::Unknown,
        Some(&argv),
    )
    .is_ok_and(|inspection| inspection.installed)
        && recorded_command(manifest)
            .is_some_and(|recorded| recorded.starts_with(&format!("{expected} # ")))
}

pub(crate) fn claude_install(request: &SetupRequest, env: &SetupEnv) -> Result<Value, RunError> {
    let settings = env.claude_settings()?;
    env.hook_argv(Harness::Claude)?;
    let (observed, _) = observe(request, env).map_err(refuse_executable)?;
    prepare_state(env)?;
    let manifest = claude_manifest(env, &settings)?;
    let mut file = OwnedFile::new(settings.clone(), manifest.clone(), b"{}");
    let mut warnings = Vec::new();
    // The hook installer's byte-exact restore needs the file as it left it.
    // When it is about to rewrite the hooks, take the mod's settings entry
    // out first (the files stay) and put it back afterwards.
    let mod_lifted = if hooks_current(env, &settings, &manifest) {
        false
    } else {
        let lifted = claude_mod::lift(&settings, &mod_manifest(env, &settings)?)
            .map_err(|error| mod_error(error, &settings))?;
        lifted.is_some()
    };
    // Everything after the lift runs in one closure: any error in it puts
    // the mod's settings entry back (one restore path).
    let mut mod_done = false;
    let result = (|| -> Result<Value, RunError> {
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
        let delivery_mod = mod_install_step(request, env, &settings, &mut warnings)?;
        mod_done = true;
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
            "mod": delivery_mod,
            "observed": "unknown",
            "note": "installed is not observed: run `herdr-threads doctor` for native evidence. \
                     Claude sessions that run with another CLAUDE_CONFIG_DIR, or with \
                     --setting-sources excluding `user`, do not load these hooks",
            "warnings": warnings,
        }))
    })();
    if result.is_err() && mod_lifted && !mod_done {
        let _ = mod_install_step(request, env, &settings, &mut Vec::new());
    }
    result
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
    // The mod edit first, then the prompt suggestion, then the hooks: each
    // later step can still restore the file byte for byte (install order is
    // the reverse).
    let mod_reverted =
        claude_mod::revert(&settings, &mod_manifest(env, &settings)?, env.state_dir()?)
            .map_err(|error| mod_error(error, &settings))?;
    let reverted =
        prompt_suggestion::revert(&settings, &prompt_suggestion_manifest(env, &settings)?)
            .map_err(|error| prompt_suggestion_error(error, &settings))?;
    let mut report = json!({
        "harness": "claude",
        "scope": "user",
        "settings": settings.display().to_string(),
        "manifest": manifest.display().to_string(),
        "prompt_suggestions": reverted.as_str(),
        "mod": mod_reverted.as_str(),
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
    report["mod"] = mod_status(env, &settings);
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

// ------------------------------------------------------------ delivery mod

/// The Claude setup option behind `setup claude --hooks-only`: install or keep
/// the hooks, and remove the delivery mod's settings entry and files.
pub const HOOKS_ONLY_OPTION: &str = "hooks-only";

/// Where setup records that it wrote the delivery mod's settings entry.
pub fn mod_manifest(env: &SetupEnv, settings: &Path) -> Result<PathBuf, RunError> {
    Ok(manifest_path(env.state_dir()?, "claude-mod", settings))
}

fn mod_error(error: SetupError, settings: &Path) -> RunError {
    let settings = settings.display();
    match error {
        SetupError::Invalid => invalid(format!(
            "{settings} is not a JSON object, its `env.{}` is not a string, or the delivery mod \
             record or directory is a symlink or damaged; the delivery mod was not changed",
            claude::PLUGIN_DIRS_ENV
        )),
        SetupError::TooLarge => invalid(format!(
            "{settings} or the delivery mod record exceeds the setup size bound; the delivery \
             mod was not changed"
        )),
        SetupError::Conflict => api(
            ErrorCode::Conflict,
            format!(
                "{settings} changed while setup edited `env.{}`, or the delivery mod record \
                 names another file; the delivery mod was not changed. Re-run the command",
                claude::PLUGIN_DIRS_ENV
            ),
        ),
        SetupError::Io => failed(format!(
            "could not read or replace {settings}, the delivery mod record or the mod files"
        )),
    }
}

/// The shell's own `CLAUDE_CODE_PLUGIN_DIRS` value, as setup captured it.
fn shell_plugin_value(env: &SetupEnv) -> Option<String> {
    env.declared_environment
        .get(claude::PLUGIN_DIRS_ENV)
        .map(|value| value.to_string_lossy().into_owned())
}

/// The directories of the shell's own `CLAUDE_CODE_PLUGIN_DIRS`.
fn shell_plugin_dirs(env: &SetupEnv) -> Vec<String> {
    shell_plugin_value(env)
        .map(|value| claude_mod::shell_dirs(&value))
        .unwrap_or_default()
}

/// Ask whether to carry the shell's plugin directories into the settings
/// value (a settings `env` value replaces the shell value). Yes by default:
/// declining would silently drop them from Claude sessions.
pub fn ask_include_shell_dirs<R: io::BufRead, W: Write>(
    dirs: &[String],
    input: &mut R,
    out: &mut W,
) -> io::Result<bool> {
    write!(
        out,
        "{} is set in this shell ({}). A settings `env` value replaces the shell value in \
         Claude sessions: include those directories in the value setup writes? [Y/n] ",
        claude::PLUGIN_DIRS_ENV,
        dirs.join(":")
    )?;
    out.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(!matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "n" | "no"
    ))
}

fn managed_policy_json(policy: &ManagedPolicy) -> Value {
    match policy {
        ManagedPolicy::None => json!({"state": "none"}),
        ManagedPolicy::DisableSideloadFlags { source } => json!({
            "state": "disable_side_load_flags",
            "source": source.display().to_string(),
        }),
        ManagedPolicy::Unverifiable { source } => json!({
            "state": "unverifiable",
            "source": source.display().to_string(),
        }),
    }
}

/// The install warning / setup-status advice for a policy that blocks the
/// write, `None` when it does not.
fn managed_policy_advice(policy: &ManagedPolicy, written: bool) -> Option<String> {
    match policy {
        ManagedPolicy::None => None,
        ManagedPolicy::DisableSideloadFlags { source } => Some(managed_policy_warning(
            &source.display().to_string(),
            written,
        )),
        ManagedPolicy::Unverifiable { source } => {
            let mut text = format!(
                "managed settings ({}) could not be read or parsed; setup cannot rule out {}, \
                 so the delivery mod was not installed (hooks only; native wake is the fallback)",
                source.display(),
                claude::DISABLE_SIDELOAD_FLAGS_KEY
            );
            if written {
                text.push_str(
                    "; to remove the path setup wrote earlier, run \
                     `herdr-threads setup claude --hooks-only`",
                );
            }
            Some(text)
        }
    }
}

fn managed_policy_warning(source: &str, written: bool) -> String {
    let tail = if written {
        "Claude Code will refuse to start with the path setup wrote earlier: run \
         `herdr-threads setup claude --hooks-only` to remove it"
    } else {
        "the delivery mod was not installed (hooks only; native wake is the fallback)"
    };
    format!(
        "managed settings ({source}) set {}: Claude Code refuses {}; {tail}",
        claude::DISABLE_SIDELOAD_FLAGS_KEY,
        claude::PLUGIN_DIRS_ENV
    )
}

/// `setup claude`, after the hooks and the prompt suggestion: write the
/// delivery mod and its settings entry (or, with `--hooks-only`, remove
/// them).
fn mod_install_step(
    request: &SetupRequest,
    env: &SetupEnv,
    settings: &Path,
    warnings: &mut Vec<String>,
) -> Result<Value, RunError> {
    let state = env.state_dir()?;
    let manifest = mod_manifest(env, settings)?;
    let error = |e| mod_error(e, settings);
    let dir = claude_mod::mod_dir(state).display().to_string();
    if request.hooks_only {
        let outcome = claude_mod::revert(settings, &manifest, state).map_err(error)?;
        return Ok(json!({
            "action": "removed_hooks_only",
            "settings_entry": outcome.as_str(),
            "dir": dir,
            "note": "hooks only: the delivery mod is not installed; native wake is the fallback",
        }));
    }
    let shell = shell_plugin_dirs(env);
    let accepted = if shell.is_empty() || !crate::cli::setup::interactive() {
        // Non-interactive: include them, so they are not silently lost.
        shell.clone()
    } else if ask_include_shell_dirs(&shell, &mut io::stdin().lock(), &mut io::stderr())? {
        shell.clone()
    } else {
        Vec::new()
    };
    let sources = ManagedPolicySources::platform(env.claude_config_dir.as_deref());
    // The mod launches `watch` exactly the way the hooks run.
    let launch = claude_mod::launch_prefix(&env.hook_argv(Harness::Claude)?).map_err(error)?;
    let outcome = claude_mod::install(&claude_mod::InstallInput {
        settings,
        manifest: &manifest,
        state_dir: state,
        shell_dirs: &accepted,
        policy: &sources,
        launch: Some(&launch),
    })
    .map_err(error)?;
    if let Some(advice) = managed_policy_advice(&outcome.policy, outcome.recorded) {
        warnings.push(advice);
    }
    let shell_report = if shell.is_empty() {
        Value::Null
    } else {
        let value = shell_plugin_value(env).unwrap_or_default();
        let settings_value = claude_mod::settings_value(settings).unwrap_or_default();
        let kept = claude_mod::shell_dirs(&settings_value);
        let lost: Vec<&String> = shell.iter().filter(|dir| !kept.contains(dir)).collect();
        if outcome.action != claude_mod::ModAction::SkippedManagedPolicy {
            warnings.push(format!(
                "{} is set in this shell ({value}); a settings `env` value replaces it in \
                 Claude sessions. {}",
                claude::PLUGIN_DIRS_ENV,
                if lost.is_empty() {
                    "Its directories are in the value setup wrote"
                } else {
                    "Its directories were not added to the settings value"
                }
            ));
        }
        json!({
            "value": value,
            "copied": outcome.copied_from_shell,
            "all_in_settings_value": lost.is_empty(),
        })
    };
    Ok(json!({
        "action": outcome.action.as_str(),
        "dir": dir,
        "files_written": outcome.files_written,
        "settings_value": claude_mod::settings_value(settings),
        "managed_policy": managed_policy_json(&outcome.policy),
        "shell_env": shell_report,
        "note": "an install does not prove any session loaded the mod",
    }))
}

/// The `mod` object of the Claude setup status (spec D8): install state,
/// managed policy, shell value and session override. Local files only: the
/// adapter status that launch and doctor read never runs the wrapper or
/// contacts the daemon. `setup-status` adds the version gate and the daemon's
/// channel status ([`complete_setup_status`]).
fn mod_status(env: &SetupEnv, settings: &Path) -> Value {
    const NOTE: &str = "an install does not prove any session loaded the mod";
    let Ok(state) = env.state_dir() else {
        return json!({"installed": false, "error": "state directory unknown", "note": NOTE});
    };
    let Ok(manifest) = mod_manifest(env, settings) else {
        return json!({"installed": false, "error": "state directory unknown", "note": NOTE});
    };
    let expected_launch = env
        .hook_argv(Harness::Claude)
        .ok()
        .and_then(|argv| claude_mod::launch_prefix(&argv).ok());
    let inspection = claude_mod::inspect(settings, &manifest, state, expected_launch.as_deref());
    let policy = ManagedPolicySources::platform(env.claude_config_dir.as_deref()).check();
    let mut report = json!({
        "installed": inspection.installed(),
        "dir": claude_mod::mod_dir(state).display().to_string(),
        "files_current": inspection.files_current,
        "files_present": inspection.files_present,
        "launch_current": expected_launch.as_ref().map(|_| inspection.launch_current),
        "launch": inspection.installed_launch,
        "recorded": inspection.recorded,
        "settings_value_contains_mod_dir": inspection.settings_value_contains_mod_dir,
        "managed_policy": managed_policy_json(&policy),
        "shell_env": shell_plugin_value(env),
        "claude_version": null,
        "claude_version_supported": null,
        "daemon": null,
        "note": NOTE,
    });
    if inspection.files_present && expected_launch.is_some() && !inspection.launch_current {
        report["launch_note"] = json!(
            "the mod launches a different herdr-threads invocation than the hooks; \
             run herdr-threads setup claude"
        );
    }
    if let Some(setting) = env
        .declared_environment
        .get(crate::protocol::watch::MOD_DELIVERY_ENV)
        && setting.to_string_lossy().trim().eq_ignore_ascii_case("off")
    {
        // This process's own environment, not the daemon-side switch.
        report["session_override"] = json!("off");
    }
    if let Some(advice) = managed_policy_advice(
        &policy,
        inspection.settings_value_contains_mod_dir || inspection.recorded,
    ) {
        report["advice"] = json!(advice);
    }
    report
}

/// `setup-status claude` only: add the mod's `claude --version` gate and the
/// running daemon's channel status to the adapter status projection.
pub(crate) fn complete_setup_status(
    projection: &mut Value,
    scope: &crate::harness::adapter::ResolvedSetupScope,
    native_binary: Option<&Path>,
    environment: &crate::harness::adapter::SetupEnvironment,
) {
    if !projection["mod"].is_object() {
        return;
    }
    let Ok(env) = scoped_legacy_environment(Harness::Claude, scope, environment) else {
        return;
    };
    let Ok(legacy) = legacy_request(
        Harness::Claude,
        SetupVerb::Status,
        native_binary,
        PromptSuggestionPolicy::Ask,
    ) else {
        return;
    };
    let (version, supported, reason) = claude_version_gate(&legacy, &env);
    let report = &mut projection["mod"];
    report["claude_version"] = json!(version);
    report["claude_version_supported"] = json!(supported);
    report["daemon"] = mod_daemon_status(&env);
    if let Some(reason) = reason {
        report["claude_version_reason"] = json!(reason);
    }
    if supported == Some(false) {
        report["claude_version_note"] = json!("mod unsupported, native wake fallback");
    }
}

/// Bound on the `claude --version` run behind `setup-status`.
const CLAUDE_VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// The installed Claude's `major.minor.patch`, whether it meets the mod's
/// minimum and, when that is unknown, why. Runs the resolved binary's
/// `--version` under a bound; only `setup-status` does (never `setup claude`,
/// launch or doctor).
fn claude_version_gate(
    request: &SetupRequest,
    env: &SetupEnv,
) -> (Option<String>, Option<bool>, Option<&'static str>) {
    let Ok((observed, _)) = observe(request, env) else {
        return (None, None, Some("no claude executable found"));
    };
    let Ok(stdout) = crate::harness::codex::version_output_cancellable(
        &observed.binary,
        CLAUDE_VERSION_TIMEOUT,
        &crate::protocol::time::Cancellation::default(),
    ) else {
        return (None, None, Some("claude --version failed or timed out"));
    };
    let text = String::from_utf8_lossy(&stdout);
    let first = text.lines().next().unwrap_or_default().trim();
    let version = first
        .split_whitespace()
        .next()
        .filter(|token| claude_mod::version_supported(token).is_some());
    match version {
        Some(version) => (
            Some(version.to_owned()),
            claude_mod::version_supported(version),
            None,
        ),
        None => (None, None, Some("unrecognised claude --version output")),
    }
}

const CHANNEL_STATUS_UNAVAILABLE: &str = "channel status unavailable";

/// The daemon's mod channel status, read only from a daemon that is already
/// running (never started here).
fn mod_daemon_status(env: &SetupEnv) -> Value {
    use crate::cli::doctor::{Daemon, observation_budget, probe_daemon};
    let unavailable = |why: &str| json!({"status": CHANNEL_STATUS_UNAVAILABLE, "reason": why});
    let (Ok(state), Ok(host)) = (env.state_dir(), env.host_endpoint()) else {
        return unavailable("the Herdr instance could not be resolved");
    };
    let Ok(context) =
        crate::daemon::paths::RuntimeContext::explicit(state.to_path_buf(), host, None)
    else {
        return unavailable("the Herdr instance could not be resolved");
    };
    let Ok(paths) = crate::daemon::paths::InstancePaths::resolve_read_only(&context) else {
        return unavailable("the daemon's paths could not be resolved");
    };
    let clock: std::sync::Arc<dyn crate::protocol::time::Clock> =
        std::sync::Arc::new(crate::app::SystemClock::new());
    let budget = observation_budget(&clock);
    match probe_daemon(&paths, &clock, &budget) {
        Ok(Daemon::Reachable(_, details)) => match &details.states {
            Ok(report) => match &report.mod_channels {
                Some(status) => json!({
                    "status": "reachable",
                    "mod_delivery": status.mod_delivery,
                    "live_channels": status.live_channels,
                    "channels": status.channels.iter().map(|channel| json!({
                        "seat": channel.seat,
                        "harness": channel.harness,
                        "state": channel.state,
                    })).collect::<Vec<_>>(),
                }),
                None => unavailable("the daemon keeps no channel registry"),
            },
            Err(why) => unavailable(why),
        },
        Ok(Daemon::NotRunning) => unavailable("no daemon is running"),
        Ok(Daemon::Unreachable(why)) => unavailable(&why),
        Err(why) => unavailable(&why),
    }
}

/// One `mod:` summary line for the per-harness text report.
pub(crate) fn mod_line(value: &Value) -> Option<String> {
    use crate::cli::setup::scalar;
    if let Some(outcome) = value.as_str() {
        return Some(format!("mod: {outcome}"));
    }
    let object = value.as_object()?;
    if let Some(action) = object.get("action").and_then(Value::as_str) {
        let mut line = format!("mod: {action}");
        if let Some(source) = value["managed_policy"]["source"].as_str() {
            line.push_str(&format!(" (managed policy {})", scalar(&json!(source))));
        }
        return Some(line);
    }
    let installed = if value["installed"] == true {
        "installed"
    } else {
        "not installed"
    };
    let mut line = format!("mod: {installed}");
    if let Some(source) = value["managed_policy"]["source"].as_str() {
        line.push_str(&format!(
            "; managed policy {} blocks it",
            scalar(&json!(source))
        ));
    }
    match value["claude_version_supported"].as_bool() {
        Some(true) => line.push_str("; claude >= 2.1.287"),
        Some(false) => line.push_str("; mod unsupported, native wake fallback"),
        None => line.push_str("; claude version not observed"),
    }
    line.push_str(&format!(
        "; daemon: {}",
        scalar(&json!(
            value["daemon"]["status"]
                .as_str()
                .map(|status| if status == "reachable" {
                    format!(
                        "mod_delivery {}, {} live channel(s)",
                        scalar(&value["daemon"]["mod_delivery"]),
                        scalar(&value["daemon"]["live_channels"])
                    )
                } else {
                    status.to_owned()
                })
                .unwrap_or_default()
        ))
    ));
    if value["session_override"] == "off" {
        line.push_str("; session override HERDR_THREADS_MOD_DELIVERY=off");
    }
    line.push_str("; an install does not prove any session loaded the mod");
    Some(line)
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    /// The shell-directories question defaults to yes (declining would drop
    /// them from Claude sessions) and only an explicit no leaves them out.
    #[test]
    fn shell_dirs_question_defaults_to_include() {
        let dirs = vec!["/s1".to_owned(), "/s2".to_owned()];
        for (answer, include) in [
            ("\n", true),
            ("y\n", true),
            ("yes\n", true),
            ("n\n", false),
            ("No\n", false),
            ("", true),
        ] {
            let mut out = Vec::new();
            let got =
                ask_include_shell_dirs(&dirs, &mut io::Cursor::new(answer), &mut out).unwrap();
            assert_eq!(got, include, "{answer:?}");
            let asked = String::from_utf8(out).unwrap();
            assert!(
                asked.contains("/s1:/s2") && asked.contains("[Y/n]"),
                "{asked}"
            );
        }
    }
}
