//! Installer-only consent policy over canonical setup and the embedded agent skill.
use super::{
    RunError, hook,
    installer_skill::{SkillFile, read_optional},
    setup::{self, PromptSuggestionPolicy, SetupEnv, SetupRequest, SetupVerb},
};
use crate::{
    harness::{
        codex_config,
        context::Harness,
        setup::{self as owned, NativeObservation, SettingsKind},
    },
    protocol::output::{OutputFormat, OutputSpec},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, IsTerminal, Write},
};

fn failure(detail: impl Into<String>) -> RunError {
    RunError::Io(io::Error::other(detail.into()))
}

fn hooks_installed(env: &SetupEnv, harness: Harness) -> Result<bool, RunError> {
    let (kind, path, manifest) = match harness {
        Harness::Claude => {
            let (path, manifest) = setup::claude_paths(env)?;
            (SettingsKind::ClaudeUser, path, manifest)
        }
        Harness::Codex => {
            let paths = setup::codex_paths(env)?;
            let allowance =
                codex_config::inspect(&paths.config, &paths.config_manifest).map_err(|error| {
                    failure(format!("invalid Codex allowance ownership: {error:?}"))
                })?;
            if allowance.recorded.as_ref().is_some_and(|manifest| {
                manifest.phase != owned::InstallPhase::Installed || !allowance.present
            }) {
                return Err(failure(
                    "recorded Codex allowance is partial or edited; preserved",
                ));
            }
            (SettingsKind::CodexUser, paths.hooks, paths.hooks_manifest)
        }
        Harness::Human => return Err(failure("human panes have no hooks")),
    };
    let bytes = read_optional(&path)?;
    let recorded = owned::read_settings_manifest(&manifest)
        .map_err(|error| failure(format!("invalid hook ownership manifest: {error:?}")))?;
    if let Some(recorded) = recorded {
        if recorded.phase != owned::InstallPhase::Installed {
            return Err(failure("hook installation is partial; preserved"));
        }
        // Canonical install verifies exact recorded groups/command before upgrading old declarations.
        // A completeness status alone would incorrectly classify valid older owned hooks as missing.
        return Ok(true);
    }
    let argv = env.hook_argv(harness)?;
    let inspection = owned::inspect_user_settings_for(
        kind,
        &path,
        &manifest,
        NativeObservation::Unknown,
        Some(&argv),
    )
    .map_err(|error| failure(format!("hook ownership inspection failed: {error:?}")))?;
    if inspection.installed {
        return Ok(true);
    }
    if let Some(bytes) = bytes {
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| failure("invalid hook settings; preserved"))?;
        if !value.is_object() {
            return Err(failure("hook settings are not an object; preserved"));
        }
        if let Some(hooks) = value.get("hooks") {
            let map = hooks
                .as_object()
                .ok_or_else(|| failure("invalid hooks map; preserved"))?;
            if map.values().any(|groups| !groups.is_array()) {
                return Err(failure("invalid hook groups; preserved"));
            }
        }
        // This conservative conflict check grants no ownership; canonical setup handles all writes.
        if value.get("hooks").is_some_and(foreign_hooks) {
            return Err(failure(
                "unowned or partial herdr-threads hooks; preserved (use setup-status to inspect)",
            ));
        }
    }
    Ok(false)
}

fn foreign_hooks(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "command" && value.as_str().is_some_and(|s| s.contains("herdr-threads")))
                || foreign_hooks(value)
        }),
        Value::Array(array) => array.iter().any(foreign_hooks),
        _ => false,
    }
}

/// Consent is asked separately per missing component; owned writes are validated by their backend.
fn execute<F: FnMut(&str) -> io::Result<bool>>(
    env: &SetupEnv,
    confirm_missing: bool,
    interactive: bool,
    mut confirm: F,
) -> Value {
    let mut entries = Vec::new();
    let mut failed = false;
    for harness in setup::HARNESSES {
        let name = match harness {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::Human => continue,
        };
        if hook::resolve_on_path(name, env.path.as_deref()).is_none() {
            continue;
        }
        for component in ["hooks", "skill"] {
            let mut skill_file = None;
            let state = if component == "hooks" {
                hooks_installed(env, harness)
            } else {
                SkillFile::inspect(env, harness).map(|file| {
                    let installed = file.installed();
                    skill_file = Some(file);
                    installed
                })
            };
            let mut entry = json!({"harness":name,"component":component});
            let result = state.and_then(|installed| {
                if !installed && !confirm_missing {
                    if !interactive {
                        entry["outcome"] = json!("skipped");
                        entry["detail"] = json!(
                            "missing; no terminal; rerun install.sh --setup to confirm installation"
                        );
                        return Ok(());
                    }
                    if !confirm(&format!("Install herdr-threads {component} for {name}?"))? {
                        entry["outcome"] = json!("declined");
                        return Ok(());
                    }
                }
                if component == "hooks" {
                    // Consent can wait indefinitely; foreign or partial state introduced while
                    // asking must still refuse before canonical setup reads a new baseline.
                    if hooks_installed(env, harness)? != installed {
                        return Err(failure(
                            "hook ownership changed during reconciliation; preserved",
                        ));
                    }
                    let report = setup::execute(
                        &SetupRequest {
                            verb: SetupVerb::Install,
                            harness,
                            harness_binary: None,
                            prompt_suggestions: PromptSuggestionPolicy::Keep,
                        },
                        env,
                    )?;
                    entry["warnings"] = report["warnings"].clone();
                    entry["trust"] = report["trust"]["note"].clone();
                } else {
                    skill_file.take().expect("inspected skill").install(env)?;
                }
                entry["outcome"] = json!(if installed { "updated" } else { "installed" });
                Ok(())
            });
            if let Err(error) = result {
                failed = true;
                entry["outcome"] = json!("failed");
                entry["detail"] = json!(error.to_string());
            }
            entries.push(entry);
        }
    }
    json!({"integrations": entries, "exit_status": if failed {1} else {0}})
}

fn confirm<R: BufRead, W: Write>(
    question: &str,
    input: &mut R,
    output: &mut W,
) -> io::Result<bool> {
    write!(output, "{question} [y/N] ")?;
    output.flush()?;
    let mut answer = String::new();
    std::io::Read::take(&mut *input, 128).read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn printable(text: &str) -> String {
    crate::view::escape::escape_for_terminal(
        &text.chars().take(600).collect::<String>(),
        crate::view::escape::Context::SingleLine,
    )
    .into_owned()
}

pub fn run<W: Write>(
    confirm_missing: bool,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let env = SetupEnv::from_process(output)?;
    // The controlling terminal works with curl | bash; redirected stdout never prompts.
    let terminal = io::stdout().is_terminal() && output.format != OutputFormat::Json;
    let mut tty = terminal
        .then(|| {
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
        })
        .and_then(Result::ok);
    let report = execute(&env, confirm_missing, tty.is_some(), |question| {
        let tty = tty.as_mut().expect("interactive terminal");
        let mut input = io::BufReader::new(tty.try_clone()?);
        confirm(question, &mut input, tty)
    });
    if output.format == OutputFormat::Json {
        serde_json::to_writer(&mut *writer, &report).map_err(io::Error::other)?;
        writeln!(writer)?;
    } else {
        let color = terminal
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb");
        let entries = report["integrations"]
            .as_array()
            .expect("integration entries");
        if entries.is_empty() {
            writeln!(
                writer,
                "integrations: skipped (no supported harness found on PATH)"
            )?;
        }
        for entry in entries {
            let verdict = format!(
                "{} {}: {}",
                entry["harness"].as_str().unwrap(),
                entry["component"].as_str().unwrap(),
                entry["outcome"].as_str().unwrap()
            );
            if color {
                let code = if entry["outcome"] == "failed" {
                    31
                } else if entry["outcome"] == "skipped" || entry["outcome"] == "declined" {
                    33
                } else {
                    32
                };
                write!(writer, "\x1b[1;{code}m{verdict}\x1b[0m")?;
            } else {
                write!(writer, "{verdict}")?;
            }
            if let Some(detail) = entry["detail"].as_str() {
                write!(writer, " ({})", printable(detail))?;
            }
            writeln!(writer)?;
            if let Some(warnings) = entry["warnings"].as_array() {
                for warning in warnings.iter().take(2).filter_map(Value::as_str) {
                    writeln!(writer, "  warning: {}", printable(warning))?;
                }
            }
            if let Some(trust) = entry["trust"].as_str() {
                writeln!(writer, "  {}", printable(trust))?;
            }
        }
    }
    writer.flush()?;
    if report["exit_status"] == 0 {
        Ok(())
    } else {
        Err(RunError::Exit(1))
    }
}

#[cfg(test)]
#[path = "../../tests/cli/installer.rs"]
mod tests;
