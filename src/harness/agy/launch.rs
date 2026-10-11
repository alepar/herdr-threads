//! agy launch policy (D8): the `agy --help` 1.3.2 flag table with Go flag grammar
//! (`--name value | --name=value`), the plugin gate, and the handoff startup-input template.
//!
//! The grammar is deliberately closed: a flag outside the table, a print-mode flag, a positional,
//! a subcommand or `--` is refused, so a validated argv can only start the interactive agent.
use super::{
    paths::AgyRoots,
    plugin,
    setup::{ALLOW_RULE, AllowEntryState},
};
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

/// The environment variable whose value is prepended to the caller's agy arguments.
pub const OPTIONS_ENV: &str = "HERDR_THREADS_AGY_OPTS";
const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const MODES: &[&str] = &["accept-edits", "plan"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Bool,
    Value,
    Enum(&'static [&'static str]),
    /// A value that may start with `-` (it is text, and Go takes the next argument whole).
    Text,
}
struct Flag {
    key: &'static str,
    shape: Shape,
    repeatable: bool,
}
const fn flag(key: &'static str, shape: Shape) -> Flag {
    Flag {
        key,
        shape,
        repeatable: false,
    }
}

/// Accepted flags by spelling. `-c` is `--continue` (the most recent conversation), never
/// `--conversation`.
fn accepted(name: &str) -> Option<Flag> {
    Some(match name {
        "--add-dir" => Flag {
            repeatable: true,
            ..flag("add-dir", Shape::Value)
        },
        "--agent" => flag("agent", Shape::Value),
        "--conversation" => flag("conversation", Shape::Value),
        "--effort" => flag("effort", Shape::Enum(EFFORTS)),
        "--mode" => flag("mode", Shape::Enum(MODES)),
        "--model" => flag("model", Shape::Value),
        "--project" => flag("project", Shape::Value),
        "--log-file" => flag("log-file", Shape::Value),
        "-i" | "--prompt-interactive" => flag("prompt-interactive", Shape::Text),
        "-c" | "--continue" => flag("continue", Shape::Bool),
        "--new-project" => flag("new-project", Shape::Bool),
        _ => return None,
    })
}

/// Flags the launch always refuses: they leave the interactive session or widen its authority.
const REFUSED: &[&str] = &[
    "-p",
    "--print",
    "--prompt",
    "--dangerously-skip-permissions",
    "--sandbox",
    "--remote-control",
    "--input-format",
    "--output-format",
    "--json-schema",
    "--print-timeout",
    "--disable-slash-commands",
];

fn error(code: ErrorCode, text: &str) -> ApiError {
    ApiError::new(code, text)
}
fn invalid(text: &str) -> ApiError {
    error(ErrorCode::InvalidRequest, text)
}

/// What a validated argv selects that other launch steps care about.
#[derive(Debug, Default, PartialEq, Eq)]
struct Parsed {
    prompt_interactive: bool,
}

fn parse(argv: &[String]) -> Result<Parsed, ApiError> {
    crate::ports::validate_native_argv(argv).map_err(invalid)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut parsed = Parsed::default();
    let mut i = 0;
    while i < argv.len() {
        let token = argv[i].as_str();
        i += 1;
        if token.is_empty() {
            return Err(invalid("agy launch argument is empty"));
        }
        if token == "--" {
            return Err(error(
                ErrorCode::UnsupportedHarness,
                "agy launch refuses `--`",
            ));
        }
        if !token.starts_with('-') || token == "-" {
            return Err(error(
                ErrorCode::UnsupportedHarness,
                "agy launch refuses positional arguments and subcommands",
            ));
        }
        let (name, inline) = if token.starts_with("--") {
            match token.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (token, None),
            }
        } else {
            // Short flags are exactly one letter: `-c`, `-i`, `-p`. `-model` is a single-dash long form.
            let bytes = token.as_bytes();
            if !bytes[1].is_ascii_alphabetic() || (bytes.len() > 2 && bytes[2] != b'=') {
                return Err(error(
                    ErrorCode::UnsupportedHarness,
                    "agy launch refuses single-dash long forms",
                ));
            }
            (&token[..2], token.get(3..).filter(|_| bytes.len() > 2))
        };
        if REFUSED.contains(&name) {
            return Err(error(
                ErrorCode::UnsupportedHarness,
                &format!(
                    "agy launch refuses {name}: it leaves the interactive session or widens its permissions"
                ),
            ));
        }
        let Some(flag) = accepted(name) else {
            return Err(error(
                ErrorCode::UnsupportedHarness,
                &format!("agy launch refuses the uncaptured option {name}"),
            ));
        };
        if !seen.insert(flag.key) && !flag.repeatable {
            return Err(invalid(&format!("duplicate agy option {name}")));
        }
        if flag.shape == Shape::Bool {
            if inline.is_some() {
                return Err(invalid(&format!("agy option {name} takes no value")));
            }
            continue;
        }
        let value = match inline {
            Some(value) => value,
            None => {
                let value = argv.get(i).map(String::as_str);
                i += 1;
                value.ok_or_else(|| invalid(&format!("agy option {name} requires a value")))?
            }
        };
        if value.is_empty()
            || (inline.is_none() && value.starts_with('-') && flag.shape != Shape::Text)
        {
            return Err(invalid(&format!(
                "agy option {name} requires one unambiguous value"
            )));
        }
        if let Shape::Enum(allowed) = flag.shape
            && !allowed.contains(&value)
        {
            return Err(invalid(&format!(
                "agy option {name} accepts only {}",
                allowed.join("|")
            )));
        }
        if flag.key == "prompt-interactive" {
            parsed.prompt_interactive = true;
        }
    }
    if seen.contains("continue") && seen.contains("conversation") {
        return Err(error(
            ErrorCode::Conflict,
            "agy -c/--continue (most recent conversation) conflicts with --conversation",
        ));
    }
    Ok(parsed)
}

fn plugin_roots(scope: &ResolvedSetupScope) -> Result<AgyRoots, ApiError> {
    match scope {
        ResolvedSetupScope::ConfigRoot(root) => Ok(AgyRoots::from_config_root(root.clone())),
        ResolvedSetupScope::Profile { .. } => Err(error(
            ErrorCode::InvalidRequest,
            "agy launch requires a config root",
        )),
    }
}

pub(super) struct AgyLaunch;
impl LaunchPolicy for AgyLaunch {
    fn native_options_env(&self) -> Option<&'static str> {
        Some(OPTIONS_ENV)
    }
    fn prepare_startup_input(
        &self,
        caller: &[String],
        _: &StartupInputSpec,
    ) -> Result<Option<StartupInputTemplate>, ApiError> {
        if parse(caller)?.prompt_interactive {
            return Err(error(
                ErrorCode::Conflict,
                "agy startup prompt is occupied by -i/--prompt-interactive",
            ));
        }
        let mut template = StartupInputTemplate::positional(caller.len());
        template.before.push("--prompt-interactive".into());
        Ok(Some(template))
    }
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        _: &dyn PaneShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchScope, ApiError> {
        parse(&request.argv)?;
        let root = request
            .environment
            .config_roots
            .get("agy")
            .cloned()
            .or_else(|| {
                let home = request.environment.home.as_deref()?;
                Some(AgyRoots::from_home(std::path::Path::new(home)).config_root)
            })
            .ok_or_else(|| {
                error(
                    ErrorCode::MissingHook,
                    "agy config root unavailable (HOME unset)",
                )
            })?;
        Ok(LaunchScope {
            setup: ResolvedSetupScope::ConfigRoot(root),
            working_directory: request.environment.cwd.clone(),
            config_source: "home_default",
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
            return Err(invalid("agy rejects owned launch arguments"));
        }
        parse(&caller)?;
        Ok(caller)
    }
    fn prepare_launch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        admitted: &AdmittedHandle,
        status: &LocalSetupStatus,
        _: &dyn PaneShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchPreparation, ApiError> {
        if admitted.metadata().id != "agy" || status.scope != scope.setup {
            return Err(invalid("launch admission or scope mismatch"));
        }
        if !status.installed {
            return Err(error(
                ErrorCode::MissingHook,
                "the agy plugin is missing or stale; run herdr-threads setup agy",
            ));
        }
        match status.enabled {
            Some(true) => {}
            Some(false) => {
                return Err(error(
                    ErrorCode::Conflict,
                    "the herdr-threads plugin is disabled in agy; run `agy plugin enable herdr-threads`",
                ));
            }
            None => {
                return Err(error(
                    ErrorCode::Conflict,
                    "agy plugin enablement is unknown (config.json unreadable); fix it or run `agy plugin enable herdr-threads`",
                ));
            }
        }
        let hook = status.configured_hook.clone().ok_or_else(|| {
            error(
                ErrorCode::MissingHook,
                "the agy plugin hooks are not configured; run herdr-threads setup agy",
            )
        })?;
        let roots = plugin_roots(&scope.setup)?;
        if plugin::status(&roots, &request.environment).hook.as_ref() != Some(&hook) {
            return Err(error(
                ErrorCode::Conflict,
                "the agy plugin changed before preparation",
            ));
        }
        let allow = status.projection["allow_entry"]["state"]
            .as_str()
            .unwrap_or(AllowEntryState::Missing.as_str());
        let mut warnings = Vec::new();
        if allow == AllowEntryState::Missing.as_str() {
            warnings.push(format!(
                "the allow entry {ALLOW_RULE} is missing from {}; agy will ask before each herdr-threads command; run herdr-threads setup agy",
                roots.settings.display()
            ));
        }
        Ok(LaunchPreparation {
            argv: self.compose_argv(request.argv.clone(), Vec::new())?,
            hook,
            working_directory: scope.working_directory.clone(),
            environment_overrides: Default::default(),
            report: serde_json::json!({"agy": {
                "plugin": status.projection["plugin"],
                "allow_entry": status.projection["allow_entry"],
                "warnings": warnings,
            }}),
        })
    }
    fn configuration_fingerprint(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
    ) -> Result<String, ApiError> {
        let roots = plugin_roots(&scope.setup)?;
        let status = plugin::status(&roots, &request.environment);
        let hook = status.hook.ok_or_else(|| {
            error(
                ErrorCode::MissingHook,
                "the agy plugin is missing or stale; run herdr-threads setup agy",
            )
        })?;
        let enabled = match status.enabled {
            Some(true) => "enabled",
            Some(false) => "disabled",
            None => "unknown",
        };
        Ok(crate::harness::setup::fingerprint(
            format!("agy-launch-v1\0{}\0{enabled}", hook.fingerprint).as_bytes(),
        ))
    }
    fn expected_host_kinds(&self) -> &'static [&'static str] {
        &["agy"]
    }
}

#[cfg(test)]
#[path = "../../../tests/harness/agy_launch.rs"]
mod tests;
