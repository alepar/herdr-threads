//! Installer-only consent policy over canonical setup and the embedded agent skill.
use super::{
    RunError, hook,
    installer_skill::{SkillFile, SkillInspection},
    setup::{self, SetupEnv, SetupVerb},
};
use crate::protocol::output::{OutputFormat, OutputSpec};
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

fn failure(detail: impl Into<String>) -> RunError {
    RunError::Io(io::Error::other(detail.into()))
}

/// One more config root of a registered harness the installer reconciles after the effective
/// ones (`--config-root HARNESS=DIR`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraConfigRoot {
    pub harness: String,
    pub root: PathBuf,
}

/// Parse `--config-root HARNESS=DIR`: a registered agent harness with an installer policy and an
/// absolute, normalized directory.
pub(crate) fn parse_config_root(
    value: &str,
    registry: &crate::harness::registry::Registry,
) -> Result<ExtraConfigRoot, String> {
    let (harness, root) = value
        .split_once('=')
        .ok_or_else(|| format!("--config-root {value:?}: expected HARNESS=DIR"))?;
    let registration = registry
        .agent(harness)
        .and_then(|agent| registry.by_id(agent))
        .map_err(|error| format!("--config-root {harness:?}: {error}"))?;
    if registration.installer_policy().is_none() {
        return Err(format!(
            "--config-root {harness:?}: the installer does not reconcile this harness"
        ));
    }
    let root = PathBuf::from(root);
    if !root.is_absolute()
        || root.as_os_str().len() > 4096
        || root
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .any(u8::is_ascii_control)
        || !root.components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
    {
        return Err(format!(
            "--config-root {harness}={}: DIR must be an absolute path without `.` or `..`",
            root.display()
        ));
    }
    Ok(ExtraConfigRoot {
        harness: harness.to_owned(),
        root: root.components().collect(),
    })
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
    execute_for_registry_with(
        registry,
        env,
        &[],
        confirm_missing,
        false,
        interactive,
        confirm,
    )
}

/// The question for the permissions component of one harness.
pub(crate) fn permission_question(harness: &str) -> String {
    format!(
        "Let agents in {harness} run herdr-threads commands without prompting? Person and setup \
         commands still ask"
    )
}

/// The physical identity of a config root, for deduplication: canonical where it resolves.
fn root_identity(root: &Path) -> PathBuf {
    crate::harness::setup::canonical_path(root).unwrap_or_else(|_| root.to_path_buf())
}

/// The roots of one harness in reconciliation order: the effective root, then each extra root,
/// deduplicated by canonical path; the adapter's default root (its config-root variable unset)
/// goes first, so a file other roots share through a symlink gets its owner first.
fn ordered_roots(
    registration: &crate::harness::registry::Registration,
    env: &SetupEnv,
    extra: &[ExtraConfigRoot],
) -> Vec<Option<PathBuf>> {
    let name = registration.metadata().id;
    let mut roots = vec![env.config_roots.get(name).cloned()];
    roots.extend(
        extra
            .iter()
            .filter(|root| root.harness == name)
            .map(|root| Some(root.root.clone())),
    );
    let mut seen = std::collections::BTreeSet::new();
    roots.retain(|root| {
        root.as_deref()
            .is_none_or(|root| seen.insert(root_identity(root)))
    });
    let mut defaults = env.snapshot();
    defaults.declared.clear();
    if let Some(default) = registration
        .default_config_root(&defaults)
        .map(|root| root_identity(&root))
        && let Some(index) = roots.iter().position(|root| {
            root.as_deref()
                .is_some_and(|root| root_identity(root) == default)
        })
    {
        let root = roots.remove(index);
        roots.insert(0, root);
    }
    roots
}

fn execute_for_registry_with<F: FnMut(&str) -> io::Result<bool>>(
    registry: &crate::harness::registry::Registry,
    env: &SetupEnv,
    extra: &[ExtraConfigRoot],
    confirm_missing: bool,
    without_permissions: bool,
    interactive: bool,
    mut confirm: F,
) -> Value {
    use crate::harness::adapter::*;
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
        let roots = ordered_roots(registration, env, extra);
        let labelled = roots.len() > 1;
        for root in roots {
            let mut root_env = env.clone();
            if let Some(root) = &root {
                root_env.config_roots.insert(name.to_owned(), root.clone());
            }
            // With more than one root of a harness every entry and question names its root.
            let label = root.as_ref().filter(|_| labelled).map(|root| {
                match registration.text().config_root_hint {
                    "" => root.display().to_string(),
                    hint => format!("{hint}={}", root.display()),
                }
            });
            failed |= reconcile_root(
                registration,
                &root_env,
                root.as_deref(),
                label.as_deref(),
                confirm_missing,
                without_permissions,
                interactive,
                &mut confirm,
                &mut entries,
            );
        }
    }
    json!({"integrations":entries,"exit_status":if failed {1} else {0}})
}

/// Reconcile the hooks, skill and permissions of one harness in one config root; true when a
/// component failed.
#[allow(clippy::too_many_arguments)]
fn reconcile_root<F: FnMut(&str) -> io::Result<bool>>(
    registration: &crate::harness::registry::Registration,
    env: &SetupEnv,
    root: Option<&Path>,
    label: Option<&str>,
    confirm_missing: bool,
    without_permissions: bool,
    interactive: bool,
    confirm: &mut F,
    entries: &mut Vec<Value>,
) -> bool {
    use crate::harness::adapter::*;
    let name = registration.metadata().id;
    let target = label.map_or_else(|| name.to_owned(), |label| format!("{name} ({label})"));
    let new_entry = |component: &str| {
        let mut entry = json!({"harness":name,"component":component});
        if let Some(root) = root {
            entry["config_root"] = json!(root.display().to_string());
        }
        if let Some(label) = label {
            entry["root_label"] = json!(label);
        }
        entry
    };
    let snapshot = env.snapshot();
    let budget = crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(
            snapshot.clock.monotonic_now().0.saturating_add(30_000),
        ),
        cancellation: Default::default(),
    };
    let mut failed = false;
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
    let mut hooks_ready = false;
    for component in ["hooks", "skill"] {
        let mut skill_file = None;
        let mut entry = new_entry(component);
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
                    match SkillFile::inspect(env, name, &destination)? {
                        SkillInspection::File(file) => {
                            let installed = file.installed();
                            skill_file = Some(file);
                            Ok(Some(installed))
                        }
                        // Shared through a symlink: reported, never written, never asked.
                        SkillInspection::Shared(owner) => {
                            entry["outcome"] = json!("shared");
                            entry["detail"] = json!(format!(
                                "shared via symlink; managed at {}",
                                owner.display()
                            ));
                            Ok(None)
                        }
                        SkillInspection::SharedMissing(detail) => {
                            entry["outcome"] = json!("skipped");
                            entry["detail"] = json!(detail);
                            Ok(None)
                        }
                    }
                }
            });
        let result = state.and_then(|installed| {
            let Some(installed) = installed else {
                if entry.get("outcome").is_none() {
                    entry["outcome"] = json!("unavailable");
                    entry["detail"] = json!("registered adapter declares no skill destination");
                }
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
                if !confirm(&format!("Install herdr-threads {component} for {target}?"))? {
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
                for option in registration.setup_options() {
                    // --without-permissions also undoes the takeover of an earlier grant.
                    // The installer never writes a pane environment: it only reports
                    // and advises (`setup <harness> --with-pane-env` writes it).
                    if option.name == "keep-prompt-suggestions"
                        || option.name == crate::cli::setup::ADVISE_PANE_ENV
                        || (without_permissions
                            && option.name == crate::cli::setup::WITHOUT_PERMISSIONS)
                    {
                        options.insert(option.name.into(), true);
                    }
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
                if let Some(note) = report["pane_env"]["note"].as_str() {
                    entry["pane_env_advice"] = json!(note);
                }
                hooks_ready = true;
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
    let state = request
        .as_ref()
        .map_err(|e| failure(e.to_string()))
        .and_then(|request| {
            registration
                .installer_permission_state(request)
                .map_err(|e| failure(e.to_string()))
        });
    // An adapter without a permission component (its integration needs no grant) has no
    // permissions entry.
    let state = match state {
        Ok(None) => return failed,
        // Asked for no permissions: nothing to inspect or fail on.
        _ if without_permissions => {
            let mut entry = new_entry("permissions");
            entry["outcome"] = json!("skipped");
            entry["detail"] = json!("--without-permissions");
            entries.push(entry);
            return failed;
        }
        Ok(Some(state)) => Ok(state),
        Err(error) => Err(error),
    };
    let mut entry = new_entry("permissions");
    let result = state.and_then(|state| {
        match state {
            InstallerPermissionState::Granted => {
                entry["outcome"] = json!("kept");
                return Ok(());
            }
            // Shared through a symlink: reported, never written, never asked.
            InstallerPermissionState::Shared(detail) => {
                entry["outcome"] = json!("shared");
                entry["detail"] = json!(detail);
                return Ok(());
            }
            InstallerPermissionState::SharedMissing(detail) => {
                entry["outcome"] = json!("skipped");
                entry["detail"] = json!(detail);
                return Ok(());
            }
            InstallerPermissionState::Missing => {}
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
            if !confirm(&permission_question(&target))? {
                entry["outcome"] = json!("declined");
                return Ok(());
            }
        }
        let mut options = SetupOptions::new();
        for option in registration.setup_options() {
            if option.name == "keep-prompt-suggestions"
                || option.name == crate::cli::setup::ADVISE_PANE_ENV
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
    failed
}
#[cfg(test)]
fn execute<F: FnMut(&str) -> io::Result<bool>>(
    env: &SetupEnv,
    confirm_missing: bool,
    interactive: bool,
    confirm: F,
) -> Value {
    execute_with(env, &[], confirm_missing, false, interactive, confirm)
}

fn execute_with<F: FnMut(&str) -> io::Result<bool>>(
    env: &SetupEnv,
    extra: &[ExtraConfigRoot],
    confirm_missing: bool,
    without_permissions: bool,
    interactive: bool,
    confirm: F,
) -> Value {
    execute_for_registry_with(
        crate::harness::registry::builtins(),
        env,
        extra,
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
    extra: &[ExtraConfigRoot],
    confirm_missing: bool,
    without_permissions: bool,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<(), RunError> {
    let _backups = crate::harness::setup::BackupSession::start();
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
        extra,
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
                "{} {}{}: {}",
                entry["harness"].as_str().unwrap(),
                entry["component"].as_str().unwrap(),
                entry["root_label"]
                    .as_str()
                    .map(|label| format!(" ({})", printable(label)))
                    .unwrap_or_default(),
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
            if let Some(advice) = entry["pane_env_advice"].as_str() {
                writeln!(writer, "  pane environment: {}", printable(advice))?;
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
