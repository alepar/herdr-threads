//! Closed interactive grammar, captured from native pi's CLI parser. No version probe.
use crate::harness::{adapter::*, launch::PaneShellProbe, registry::AdmittedHandle};
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::CallBudget,
};
use std::path::{Path, PathBuf};
pub(super) const LAUNCH_NOTES: &str = "pi launch accepts --model, --provider (with --model), --thinking, -c/--continue, -r/--resume, --session, --fork, --session-dir, -e/--extension and --skill. Values are separate arguments. One startup input may follow `--`. Print, JSON, RPC, extension-disabling, secret-bearing and unknown options are refused. Extra arguments come from HERDR_THREADS_PI_OPTS.";
fn invalid(text: &str) -> ApiError {
    ApiError::new(ErrorCode::InvalidRequest, text)
}
fn refused() -> ApiError {
    ApiError::new(
        ErrorCode::UnsupportedHarness,
        "pi launch refuses unsupported or ambiguous arguments",
    )
}
#[derive(Default)]
struct Parsed {
    prompt: bool,
}
fn parse(argv: &[String]) -> Result<Parsed, ApiError> {
    // Never echo argument text, including native validator errors: an unknown form may be a secret.
    crate::ports::validate_native_argv(argv).map_err(|_| invalid("invalid pi launch arguments"))?;
    let mut seen = std::collections::BTreeSet::new();
    let mut index = 0;
    let mut prompt = false;
    while let Some(token) = argv.get(index) {
        index += 1;
        if token == "--" {
            if argv.len() != index + 1 || argv[index].is_empty() || argv[index].starts_with('@') {
                return Err(refused());
            }
            prompt = true;
            break;
        }
        let (key, takes_value, repeat) = match token.as_str() {
            "--model" => ("model", true, false),
            "--provider" => ("provider", true, false),
            "--thinking" => ("thinking", true, false),
            "-c" | "--continue" => ("continue", false, false),
            "-r" | "--resume" => ("resume", false, false),
            "--session" => ("session", true, false),
            "--fork" => ("fork", true, false),
            "--session-dir" => ("session-dir", true, false),
            "-e" | "--extension" => ("extension", true, true),
            "--skill" => ("skill", true, true),
            _ => return Err(refused()),
        };
        if !seen.insert(key) && !repeat {
            return Err(invalid("duplicate pi launch option"));
        }
        if takes_value {
            let value = argv
                .get(index)
                .filter(|v| !v.is_empty() && !v.starts_with('-'))
                .ok_or_else(|| invalid("pi launch option requires one value"))?;
            index += 1;
            if key == "thinking"
                && !["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                    .contains(&value.as_str())
            {
                return Err(invalid("invalid pi thinking level"));
            }
        }
    }
    if seen.contains("provider") && !seen.contains("model") {
        return Err(invalid("pi --provider requires --model"));
    }
    if ["continue", "resume", "session", "fork"]
        .into_iter()
        .filter(|key| seen.contains(key))
        .count()
        > 1
    {
        return Err(invalid("pi session selection options conflict"));
    }
    Ok(Parsed { prompt })
}
fn root(scope: &ResolvedSetupScope) -> Result<&Path, ApiError> {
    super::setup::root_of(scope).map_err(|_| invalid("pi requires an agent directory"))
}
pub(super) struct PiLaunch;
impl LaunchPolicy for PiLaunch {
    fn native_options_env(&self) -> Option<&'static str> {
        Some("HERDR_THREADS_PI_OPTS")
    }
    fn apply_model(&self, caller: Vec<String>, model: &str) -> Result<Vec<String>, ApiError> {
        Ok(["--model".into(), model.into()]
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
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "pi startup input is occupied",
            ));
        }
        let mut template = StartupInputTemplate::positional(caller.len());
        template.before.push("--".into());
        Ok(Some(template))
    }
    fn explicit_directory_env(
        &self,
        directory: &Path,
    ) -> Result<std::collections::BTreeMap<String, String>, ApiError> {
        let directory = directory
            .to_str()
            .ok_or_else(|| invalid("invalid explicit profile directory"))?;
        Ok([("PI_CODING_AGENT_DIR".into(), directory.into())].into())
    }
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        probe: &dyn PaneShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchScope, ApiError> {
        parse(&request.argv)?;
        let env = &request.environment;
        if budget.is_exhausted(env.clock.as_ref()) {
            return Err(ApiError::new(
                ErrorCode::DeadlineExceeded,
                "launch budget exhausted",
            ));
        }
        let pane = || {
            probe
                .pane_shell_env_bounded("PI_CODING_AGENT_DIR", env.clock.as_ref(), budget)
                .filter(|s| !s.is_empty())
        };
        let (root, source) = if let Some(root) = &env.explicit_profile_directory {
            (root.clone(), "explicit_profile_directory")
        } else if let Some(value) = pane() {
            let mut pane_env = env.clone();
            pane_env
                .declared
                .insert("PI_CODING_AGENT_DIR".into(), value.into());
            (
                super::setup::default_config_root(&pane_env)
                    .ok_or_else(|| invalid("pi agent directory unavailable"))?,
                "pane_shell",
            )
        } else if let Some(root) = env.config_roots.get("pi") {
            (root.clone(), "launcher")
        } else {
            (
                super::setup::default_config_root(env)
                    .ok_or_else(|| invalid("pi agent directory unavailable (HOME unset)"))?,
                "home_default",
            )
        };
        Ok(LaunchScope {
            setup: ResolvedSetupScope::ConfigRoot(root),
            working_directory: env.cwd.clone(),
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
            return Err(invalid("pi rejects owned launch arguments"));
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
        if admitted.metadata().id != "pi" || status.scope != scope.setup {
            return Err(invalid("pi launch admission or scope mismatch"));
        }
        root(&scope.setup)?;
        let argv = self.compose_argv(request.argv.clone(), vec![])?;
        let hook = crate::harness::launch::optional_launch_hook(status);
        Ok(LaunchPreparation {
            argv,
            hook,
            working_directory: scope.working_directory.clone(),
            environment_overrides: Default::default(),
            report: serde_json::json!({"pi":{"extension_available":status.installed}}),
        })
    }
    fn configuration_fingerprint(
        &self,
        _: &LaunchRequest,
        scope: &LaunchScope,
    ) -> Result<String, ApiError> {
        let path: PathBuf = root(&scope.setup)?.join(super::setup::EXTENSION_PATH);
        let bytes = std::fs::read(path).map_err(|_| {
            ApiError::new(
                ErrorCode::MissingHook,
                "pi extension missing; run herdr-threads setup pi",
            )
        })?;
        Ok(crate::harness::setup::fingerprint(&bytes))
    }
    fn expected_host_kinds(&self) -> &'static [&'static str] {
        super::METADATA.host_kinds
    }
}
