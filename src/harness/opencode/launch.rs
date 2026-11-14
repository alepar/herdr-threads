//! opencode launch policy (opencode design D7): the interactive TUI's flag table from the
//! `opencode --help` capture with yargs grammar (`--name value | --name=value`), the startup
//! prompt through `--prompt`, and the content-free first prompt a launch without one gets.
//!
//! The grammar is closed: a flag outside the table, a positional (so every subcommand, present or
//! future, and the `[project]` directory) or `--` is refused, so a validated argv can only start
//! the TUI with the plugin loaded.
use crate::harness::{
    adapter::{
        LaunchPolicy, LaunchPreparation, LaunchRequest, LaunchScope, LocalSetupStatus,
        ResolvedSetupScope, StartupInputSpec, StartupInputTemplate,
    },
    launch::PaneShellProbe,
    registry::AdmittedHandle,
};
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::CallBudget,
};

/// The environment variable whose value is prepended to the caller's opencode arguments.
pub const OPTIONS_ENV: &str = "HERDR_THREADS_OPENCODE_OPTS";
/// The startup prompt as one argument. yargs takes a separate value only when it does not start
/// with `-`, so the prompt text rides the `=` form, which it takes whole.
const PROMPT_PREFIX: &str = "--prompt=";
/// The setup-owned plugin file under the config root (design D6).
const PLUGIN_FILE: &str = "plugins/herdr-threads.js";
/// opencode's extra config directory variable; an explicit profile directory is exported as it.
const CONFIG_DIR_ENV: &str = "OPENCODE_CONFIG_DIR";

pub(super) const LAUNCH_NOTES: &str = "opencode launch accepts -m/--model, --agent, -c/--continue, -s/--session, --fork (with -c or -s) and --prompt (--name value or --name=value). It refuses positionals and so every subcommand (run, serve, attach, acp, ...), --pure, --auto/--yolo/--dangerously-skip-permissions, --port/--hostname/--mdns, --mini, unknown flags and `--`. opencode creates its session at the first prompt, so a launch without --prompt starts with the prompt \"herdr-threads: wake up\". Extra arguments come from HERDR_THREADS_OPENCODE_OPTS in the same grammar.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Bool,
    Value,
}

/// Accepted flags by spelling, with their canonical key.
fn accepted(name: &str) -> Option<(&'static str, Shape)> {
    Some(match name {
        "-m" | "--model" => ("model", Shape::Value),
        "--agent" => ("agent", Shape::Value),
        "-c" | "--continue" => ("continue", Shape::Bool),
        "-s" | "--session" => ("session", Shape::Value),
        "--fork" => ("fork", Shape::Bool),
        "--prompt" => ("prompt", Shape::Value),
        _ => return None,
    })
}

/// Flags the launch always refuses, with the reason.
fn refused(name: &str) -> Option<&'static str> {
    Some(match name {
        "--pure" => "it disables the herdr-threads plugin",
        "--auto" | "--yolo" | "--dangerously-skip-permissions" => "it auto-approves permissions",
        "--port" | "--hostname" | "--mdns" => "it opens an unauthenticated server",
        "--mini" => "it starts the minimal interface, which herdr-threads does not support",
        _ => return None,
    })
}

fn error(code: ErrorCode, text: &str) -> ApiError {
    ApiError::new(code, text)
}
fn invalid(text: &str) -> ApiError {
    error(ErrorCode::InvalidRequest, text)
}
fn unsupported(text: &str) -> ApiError {
    error(ErrorCode::UnsupportedHarness, text)
}

/// What a validated argv selects that other launch steps care about.
#[derive(Debug, Default, PartialEq, Eq)]
struct Parsed {
    prompt: bool,
}

fn parse(argv: &[String]) -> Result<Parsed, ApiError> {
    crate::ports::validate_native_argv(argv).map_err(invalid)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut i = 0;
    while i < argv.len() {
        let token = argv[i].as_str();
        i += 1;
        if token.is_empty() {
            return Err(invalid("opencode launch argument is empty"));
        }
        if token == "--" {
            return Err(unsupported("opencode launch refuses `--`"));
        }
        if !token.starts_with('-') || token == "-" {
            return Err(unsupported(
                "opencode launch refuses positional arguments: subcommands (run, serve, attach, acp, ...) and a project directory",
            ));
        }
        let (name, inline) = if token.starts_with("--") {
            match token.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (token, None),
            }
        } else if token.len() == 2 {
            (token, None)
        } else {
            // `-mvalue`, `-m=value` and grouped `-cs` are yargs forms outside the table.
            return Err(unsupported(&format!(
                "opencode launch refuses the short form {token}; pass each short flag alone"
            )));
        };
        if let Some(reason) = refused(name) {
            return Err(unsupported(&format!(
                "opencode launch refuses {name}: {reason}"
            )));
        }
        let Some((key, shape)) = accepted(name) else {
            return Err(unsupported(&format!(
                "opencode launch refuses the uncaptured option {name}"
            )));
        };
        if !seen.insert(key) {
            return Err(invalid(&format!("duplicate opencode option {name}")));
        }
        if shape == Shape::Bool {
            if inline.is_some() {
                return Err(invalid(&format!("opencode option {name} takes no value")));
            }
            continue;
        }
        let value = match inline {
            Some(value) => value,
            None => {
                let value = argv.get(i).map(String::as_str);
                i += 1;
                value.ok_or_else(|| invalid(&format!("opencode option {name} requires a value")))?
            }
        };
        // yargs reads a separate value starting with `-` as the next flag.
        if value.is_empty() || (inline.is_none() && value.starts_with('-')) {
            return Err(invalid(&format!(
                "opencode option {name} requires one unambiguous value"
            )));
        }
    }
    if seen.contains("continue") && seen.contains("session") {
        return Err(error(
            ErrorCode::Conflict,
            "opencode -c/--continue (last session) conflicts with -s/--session",
        ));
    }
    if seen.contains("fork") && !seen.contains("continue") && !seen.contains("session") {
        return Err(invalid(
            "opencode --fork requires -c/--continue or -s/--session",
        ));
    }
    Ok(Parsed {
        prompt: seen.contains("prompt"),
    })
}

fn config_root(scope: &ResolvedSetupScope) -> Result<&std::path::Path, ApiError> {
    match scope {
        ResolvedSetupScope::ConfigRoot(root) => Ok(root),
        ResolvedSetupScope::Profile { .. } => {
            Err(invalid("opencode launch requires a config root"))
        }
    }
}

pub(super) struct OpencodeLaunch;
impl LaunchPolicy for OpencodeLaunch {
    fn native_options_env(&self) -> Option<&'static str> {
        Some(OPTIONS_ENV)
    }
    fn apply_model(&self, caller: Vec<String>, model: &str) -> Result<Vec<String>, ApiError> {
        Ok(["-m".into(), model.into()]
            .into_iter()
            .chain(caller)
            .collect())
    }
    fn prepare_startup_input(
        &self,
        caller: &[String],
        _: &StartupInputSpec,
    ) -> Result<Option<StartupInputTemplate>, ApiError> {
        if parse(caller)?.prompt {
            return Err(error(
                ErrorCode::Conflict,
                "opencode startup prompt is occupied by --prompt",
            ));
        }
        let mut template = StartupInputTemplate::positional(caller.len());
        template.prefix = PROMPT_PREFIX.into();
        Ok(Some(template))
    }
    /// The launched opencode loads `OPENCODE_CONFIG_DIR` as a config directory, so an explicit
    /// profile directory is exported as it.
    fn explicit_directory_env(
        &self,
        directory: &std::path::Path,
    ) -> Result<std::collections::BTreeMap<String, String>, ApiError> {
        let directory = directory
            .to_str()
            .ok_or_else(|| invalid("invalid explicit profile directory"))?;
        Ok([(CONFIG_DIR_ENV.into(), directory.into())].into())
    }
    /// Setup's precedence (design D6) read where the launched process gets its environment:
    /// an explicit profile directory, else the pane shell's `OPENCODE_CONFIG_DIR`, else its
    /// `$XDG_CONFIG_HOME/opencode`, else the launcher's root (setup's own resolution of the
    /// launcher environment), else `~/.config/opencode`. Relative or empty values are ignored.
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        probe: &dyn PaneShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchScope, ApiError> {
        parse(&request.argv)?;
        let environment = &request.environment;
        if budget.is_exhausted(environment.clock.as_ref()) {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "launch budget exhausted",
            ));
        }
        let pane = |variable: &str| {
            probe
                .pane_shell_env_bounded(variable, environment.clock.as_ref(), budget)
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        let (root, source) = if let Some(root) = &environment.explicit_profile_directory {
            (root.clone(), "explicit_profile_directory")
        } else if let Some(root) =
            pane(CONFIG_DIR_ENV).or_else(|| pane("XDG_CONFIG_HOME").map(|dir| dir.join("opencode")))
        {
            (root, "pane_shell")
        } else if let Some(root) = environment.config_roots.get(super::METADATA.id) {
            (root.clone(), "launcher")
        } else {
            let home = environment.home.as_deref().ok_or_else(|| {
                error(
                    ErrorCode::MissingHook,
                    "opencode config root unavailable (HOME unset)",
                )
            })?;
            (
                std::path::Path::new(home).join(".config/opencode"),
                "home_default",
            )
        };
        Ok(LaunchScope {
            setup: ResolvedSetupScope::ConfigRoot(root),
            working_directory: environment.cwd.clone(),
            config_source: source,
        })
    }
    fn validate_native_argv(&self, argv: &[String]) -> Result<(), ApiError> {
        parse(argv).map(|_| ())
    }
    fn compose_argv(
        &self,
        caller: Vec<String>,
        owned: Vec<String>,
    ) -> Result<Vec<String>, ApiError> {
        if !owned.is_empty() {
            return Err(invalid("opencode rejects owned launch arguments"));
        }
        parse(&caller)?;
        Ok(caller)
    }
    /// The validated caller argv; with no `--prompt` it gains the content-free first prompt,
    /// because opencode creates its session (and the plugin enrolls it) only at a prompt.
    fn prepare_launch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        admitted: &AdmittedHandle,
        status: &LocalSetupStatus,
        _: &dyn PaneShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchPreparation, ApiError> {
        if admitted.metadata().id != super::METADATA.id || status.scope != scope.setup {
            return Err(invalid("launch admission or scope mismatch"));
        }
        config_root(&scope.setup)?;
        let mut argv = self.compose_argv(request.argv.clone(), Vec::new())?;
        let first_prompt = !parse(&argv)?.prompt;
        if first_prompt {
            argv.push(format!(
                "{PROMPT_PREFIX}{}",
                crate::notification::policy::WAKE_UP
            ));
        }
        // Readiness is advisory: a missing plugin keeps the native launch and says so.
        let hook = crate::harness::launch::optional_launch_hook(status);
        let mut warnings = Vec::new();
        if hook.is_none() {
            warnings.push("opencode plugin unavailable; run herdr-threads setup opencode");
        }
        Ok(LaunchPreparation {
            argv,
            hook,
            working_directory: scope.working_directory.clone(),
            environment_overrides: Default::default(),
            report: serde_json::json!({"opencode": {
                "first_prompt": first_prompt,
                "warnings": warnings,
            }}),
        })
    }
    /// The installed plugin's bytes: a plugin changed between preparation and submission is
    /// a conflict.
    fn configuration_fingerprint(
        &self,
        _: &LaunchRequest,
        scope: &LaunchScope,
    ) -> Result<String, ApiError> {
        let plugin = config_root(&scope.setup)?.join(PLUGIN_FILE);
        let bytes = std::fs::read(&plugin).map_err(|_| {
            error(
                ErrorCode::MissingHook,
                "the opencode plugin is missing; run herdr-threads setup opencode",
            )
        })?;
        Ok(crate::harness::setup::fingerprint(
            format!(
                "opencode-launch-v1\0{}",
                crate::harness::setup::fingerprint(&bytes)
            )
            .as_bytes(),
        ))
    }
    fn expected_host_kinds(&self) -> &'static [&'static str] {
        super::METADATA.host_kinds
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/opencode_launch.rs"]
mod tests;
