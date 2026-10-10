//! Installer-only consent policy over canonical setup and the embedded agent skill.
use super::{
    RunError, hook,
    installer_skill::SkillFile,
    setup::{self, SetupEnv, SetupVerb},
};
use crate::protocol::output::{OutputFormat, OutputSpec};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, IsTerminal, Write},
};

fn failure(detail: impl Into<String>) -> RunError {
    RunError::Io(io::Error::other(detail.into()))
}

/// Consent is asked separately per missing component; owned writes are validated by their backend.
#[cfg(test)]
fn execute_for_registry<F: FnMut(&str) -> io::Result<bool>>(
    registry: &crate::harness::registry::Registry,
    env: &SetupEnv,
    confirm_missing: bool,
    interactive: bool,
    confirm: F,
) -> Value {
    execute_for_registry_with(registry, env, confirm_missing, false, interactive, confirm)
}

/// The question for the permissions component of one harness.
pub(crate) fn permission_question(harness: &str) -> String {
    format!(
        "Let agents in {harness} run herdr-threads commands without prompting? Person and setup \
         commands still ask"
    )
}

fn execute_for_registry_with<F: FnMut(&str) -> io::Result<bool>>(
    registry: &crate::harness::registry::Registry,
    env: &SetupEnv,
    confirm_missing: bool,
    without_permissions: bool,
    interactive: bool,
    mut confirm: F,
) -> Value {
    use crate::harness::adapter::*;
    let snapshot = env.snapshot();
    let budget = crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(
            snapshot.clock.monotonic_now().0.saturating_add(30_000),
        ),
        cancellation: Default::default(),
    };
    let mut entries = Vec::new();
    let mut failed = false;
    for registration in registry.registrations() {
        let name = registration.metadata().id;
        let ExecutableLookup::Path(executable) = registration.metadata().executable else {
            continue;
        };
        if hook::resolve_on_path(executable, env.path.as_deref()).is_none() {
            continue;
        }
        if registration.installer_policy().is_none() {
            entries.push(json!({"harness":name,"component":"installer","outcome":"unavailable","detail":"registered adapter declares no installer policy"}));
            continue;
        }
        let request = registration
            .resolve_setup_scope_for(
                &SetupScopeResolutionRequest {
                    operation: SetupScopeOperation::Status,
                    selector: &SetupScopeRequest::Default,
                    native_binary: None,
                    environment: &snapshot,
                },
                &budget,
            )
            .map(|resolution| StatusRequest {
                scope: resolution.scope,
                environment: snapshot.clone(),
                native_binary: None,
            });
        for component in ["hooks", "skill"] {
            let mut skill_file = None;
            let state = request
                .as_ref()
                .map_err(|e| failure(e.to_string()))
                .and_then(|request| {
                    if component == "hooks" {
                        registration
                            .inspect_installer_hooks(request, &budget)
                            .map(|state| Some(state == InstallerHookState::Owned))
                            .map_err(|e| failure(e.to_string()))
                    } else {
                        let Some(destination) = registration
                            .installer_skill_destination(&request.scope)
                            .map_err(|e| failure(e.to_string()))?
                        else {
                            return Ok(None);
                        };
                        SkillFile::inspect(env, name, &destination).map(|file| {
                            let installed = file.installed();
                            skill_file = Some(file);
                            Some(installed)
                        })
                    }
                });
            let mut entry = json!({"harness":name,"component":component});
            let result = state.and_then(|installed| {
                let Some(installed) = installed else {
                    entry["outcome"] = json!("unavailable");
                    entry["detail"] = json!("registered adapter declares no skill destination");
                    return Ok(());
                };
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
                    let request = request.as_ref().map_err(|e| failure(e.to_string()))?;
                    if registration
                        .inspect_installer_hooks(request, &budget)
                        .map_err(|e| failure(e.to_string()))?
                        != if installed {
                            InstallerHookState::Owned
                        } else {
                            InstallerHookState::Missing
                        }
                    {
                        return Err(failure(
                            "hook ownership changed during reconciliation; preserved",
                        ));
                    }
                    let mut options = SetupOptions::new();
                    if registration
                        .setup_options()
                        .iter()
                        .any(|o| o.name == "keep-prompt-suggestions")
                    {
                        options.insert("keep-prompt-suggestions".into(), true);
                    }
                    let report = setup::execute_registered_with_expected_scope(
                        registration,
                        SetupVerb::Install,
                        &SetupScopeRequest::Default,
                        None,
                        options,
                        &snapshot,
                        Some(&request.scope),
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
        // Permissions follow the hooks: granted only where the hooks are in place.
        let granted = request
            .as_ref()
            .map_err(|e| failure(e.to_string()))
            .and_then(|request| {
                registration
                    .installer_permissions_granted(request)
                    .map_err(|e| failure(e.to_string()))
            });
        let hooks_ready = entries.iter().any(|e| {
            e["harness"] == name
                && e["component"] == "hooks"
                && matches!(e["outcome"].as_str(), Some("installed" | "updated"))
        });
        // An adapter without a permission component (its integration needs no grant) has no
        // permissions entry.
        let granted = match granted {
            Ok(None) => continue,
            Ok(Some(granted)) => Ok(granted),
            Err(error) => Err(error),
        };
        let mut entry = json!({"harness":name,"component":"permissions"});
        let result = granted.and_then(|granted| {
            if granted {
                entry["outcome"] = json!("kept");
                return Ok(());
            }
            if without_permissions {
                entry["outcome"] = json!("skipped");
                entry["detail"] = json!("--without-permissions");
                return Ok(());
            }
            if !hooks_ready {
                entry["outcome"] = json!("skipped");
                entry["detail"] = json!("hooks not installed");
                return Ok(());
            }
            if !confirm_missing {
                if !interactive {
                    entry["outcome"] = json!("skipped");
                    entry["detail"] = json!(
                        "missing; no terminal; rerun install.sh --setup to grant, or run \
                         `herdr-threads setup <harness> --with-permissions`"
                    );
                    return Ok(());
                }
                if !confirm(&permission_question(name))? {
                    entry["outcome"] = json!("declined");
                    return Ok(());
                }
            }
            let mut options = SetupOptions::new();
            for option in registration.setup_options() {
                if option.name == "keep-prompt-suggestions"
                    || option.name == crate::cli::setup::WITH_PERMISSIONS
                {
                    options.insert(option.name.into(), true);
                }
            }
            let report = setup::execute_registered(
                registration,
                SetupVerb::Install,
                &SetupScopeRequest::Default,
                None,
                options,
                &snapshot,
            )?;
            if report["permissions"]["state"] != "installed" {
                return Err(failure(format!(
                    "permission rules not installed: {}",
                    report["permissions"]["error"]
                        .as_str()
                        .or(report["permissions"]["note"].as_str())
                        .unwrap_or("unknown state")
                )));
            }
            entry["outcome"] = json!("installed");
            Ok(())
        });
        if let Err(error) = result {
            failed = true;
            entry["outcome"] = json!("failed");
            entry["detail"] = json!(error.to_string());
        }
        entries.push(entry);
    }
    json!({"integrations":entries,"exit_status":if failed {1} else {0}})
}
#[cfg(test)]
fn execute<F: FnMut(&str) -> io::Result<bool>>(
    env: &SetupEnv,
    confirm_missing: bool,
    interactive: bool,
    confirm: F,
) -> Value {
    execute_with(env, confirm_missing, false, interactive, confirm)
}

fn execute_with<F: FnMut(&str) -> io::Result<bool>>(
    env: &SetupEnv,
    confirm_missing: bool,
    without_permissions: bool,
    interactive: bool,
    confirm: F,
) -> Value {
    execute_for_registry_with(
        crate::harness::registry::builtins(),
        env,
        confirm_missing,
        without_permissions,
        interactive,
        confirm,
    )
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
    without_permissions: bool,
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
    let report = execute_with(
        &env,
        confirm_missing,
        without_permissions,
        tty.is_some(),
        |question| {
            let tty = tty.as_mut().expect("interactive terminal");
            let mut input = io::BufReader::new(tty.try_clone()?);
            confirm(question, &mut input, tty)
        },
    );
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
