//! Pinned37daf85 source grammar and separately observed prelaunch API/profile.
//! No synthetic success is native recognition, callback admission or receipt.
use super::{assets, runtime};
use crate::harness::adapter::*;
use crate::protocol::{
    results::{ApiError, ErrorCode},
    time::CallBudget,
};
use std::{any::Any, collections::BTreeMap};
pub struct HermesLaunch;
pub static POLICY: HermesLaunch = HermesLaunch;
fn error(code: ErrorCode, text: &str) -> ApiError {
    ApiError::new(code, text)
}
#[derive(Debug)]
struct Args {
    profile: String,
    values: Vec<(String, String)>,
}
fn parse(argv: &[String]) -> Result<Args, ApiError> {
    if argv.len() > 32
        || argv
            .iter()
            .any(|s| s.is_empty() || s.len() > 4096 || s.chars().any(char::is_control))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "Hermes launch argv exceeds bounds",
        ));
    }
    let mut profile = None;
    let mut cli = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut values = vec![];
    let mut i = 0;
    while i < argv.len() {
        let token = &argv[i];
        if token == "--cli" {
            if cli {
                return Err(error(ErrorCode::InvalidRequest, "duplicate classic mode"));
            }
            cli = true;
            i += 1;
            continue;
        }
        if let Some(value) = token.strip_prefix("--profile=") {
            if profile.replace(value.to_owned()).is_some() {
                return Err(error(ErrorCode::InvalidRequest, "duplicate profile"));
            }
            i += 1;
            continue;
        }
        let key = match token.as_str() {
            "-p" | "--profile" => "profile",
            "-m" | "--model" => "--model",
            "--provider" => "--provider",
            "-q" | "--query" => "--query",
            _ => {
                return Err(error(
                    ErrorCode::UnsupportedHarness,
                    "uncaptured Hermes option or caller subcommand",
                ));
            }
        };
        let value = argv
            .get(i + 1)
            .filter(|s| !s.starts_with('-'))
            .ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "Hermes option requires one unambiguous value",
                )
            })?;
        if key == "profile" {
            if profile.replace(value.clone()).is_some() {
                return Err(error(ErrorCode::InvalidRequest, "duplicate profile"));
            }
        } else {
            if !seen.insert(key) {
                return Err(error(ErrorCode::InvalidRequest, "duplicate Hermes option"));
            }
            values.push((key.into(), value.clone()));
        }
        i += 2;
    }
    let profile = profile
        .unwrap_or_else(|| "default".into())
        .trim()
        .to_lowercase();
    if profile.is_empty()
        || profile.len() > 64
        || !profile.as_bytes()[0].is_ascii_alphanumeric()
        || !profile
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid selected Hermes profile",
        ));
    }
    Ok(Args { profile, values })
}
struct Captured {
    observation: runtime::PrelaunchObservation,
    fingerprint: String,
}
fn observe(
    request: &LaunchRequest,
    scope: &LaunchScope,
    budget: &CallBudget,
) -> Result<runtime::PrelaunchObservation, ApiError> {
    let ResolvedSetupScope::Profile { name, home } = &scope.setup else {
        return Err(error(
            ErrorCode::InvalidRequest,
            "Hermes launch requires selected native profile",
        ));
    };
    if parse(&request.argv)?.profile != *name {
        return Err(error(ErrorCode::Conflict, "selected profile changed"));
    }
    let native = request
        .native_binary
        .clone()
        .or_else(|| super::launcher(&request.environment))
        .ok_or_else(|| {
            error(
                ErrorCode::UnsupportedHarness,
                "Hermes executable unavailable",
            )
        })?;
    let helper = super::inspection_helper(&request.environment)
        .map_err(|_| error(ErrorCode::Conflict, "owned inspection helper unavailable"))?;
    let observed = runtime::observe_prelaunch(&native, &helper, name, &request.environment, budget)
        .map_err(|_| {
            error(
                ErrorCode::UnsupportedHarness,
                "source-bound prelaunch API/profile observation unavailable",
            )
        })?;
    if observed.profile.home != *home || observed.profile.configured_enabled() != Some(true) {
        return Err(error(
            ErrorCode::MissingHook,
            "Hermes selected profile must explicitly enable owned integration",
        ));
    }
    Ok(observed)
}
fn owned(
    request: &LaunchRequest,
    observation: &runtime::PrelaunchObservation,
) -> Result<assets::AssetStatus, ApiError> {
    let state = request
        .environment
        .state_dir
        .as_ref()
        .ok_or_else(|| error(ErrorCode::MissingHook, "instance state unavailable"))?;
    let status = assets::status(&observation.profile, state).map_err(|_| {
        error(
            ErrorCode::MissingHook,
            "locked owned Hermes asset generation unavailable",
        )
    })?;
    let settings = status.launch_settings.as_ref().ok_or_else(|| {
        error(
            ErrorCode::MissingHook,
            "complete owned Hermes settings unavailable",
        )
    })?;
    if !status.installed
        || status.configured_enabled != Some(true)
        || status.launch_hook.is_none()
        || settings.rust_executable != request.environment.executable
        || &settings.state_root != state
        || Some(&settings.host_endpoint) != request.environment.host_endpoint.as_ref()
    {
        return Err(error(
            ErrorCode::Conflict,
            "Hermes owned generation or instance settings mismatch",
        ));
    }
    Ok(status)
}
impl LaunchPolicy for HermesLaunch {
    fn requires_process_hint(&self) -> bool {
        true
    }
    fn uses_prelaunch_observation(&self) -> bool {
        true
    }
    fn prepare_startup_input(
        &self,
        caller: &[String],
        _: &StartupInputSpec,
    ) -> Result<Option<StartupInputTemplate>, ApiError> {
        let args = parse(caller)?;
        if args.values.iter().any(|(key, _)| key == "--query") {
            return Err(error(ErrorCode::Conflict, "Hermes query input is occupied"));
        }
        if caller.len().checked_add(2).is_none_or(|n| n > 32) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "Hermes launch argv exceeds bounds",
            ));
        }
        let mut template = StartupInputTemplate::positional(caller.len());
        template.before.push("--query".into());
        template.max_arg_bytes = 4096;
        Ok(Some(template))
    }
    fn validate_native_argv(&self, argv: &[String]) -> Result<(), ApiError> {
        parse(argv).map(|_| ())
    }
    fn resolve_scope(
        &self,
        request: &LaunchRequest,
        _: &dyn crate::harness::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchScope, ApiError> {
        let profile = parse(&request.argv)?.profile;
        let observed = super::inspect_profile(
            &profile,
            request.native_binary.as_deref(),
            &request.environment,
            budget,
        )
        .map_err(|_| {
            error(
                ErrorCode::UnsupportedHarness,
                "selected native Hermes profile unavailable",
            )
        })?;
        Ok(LaunchScope {
            setup: ResolvedSetupScope::Profile {
                name: profile,
                home: observed.home,
            },
            working_directory: request.environment.cwd.clone(),
            config_source: "official_selected_profile_observation",
        })
    }
    fn observe_prelaunch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        budget: &CallBudget,
    ) -> Result<Box<dyn Any + Send + Sync>, ApiError> {
        let observation = observe(request, scope, budget)?;
        let fingerprint = owned(request, &observation)?
            .launch_hook
            .unwrap()
            .fingerprint;
        Ok(Box::new(Captured {
            observation,
            fingerprint,
        }))
    }
    fn compose_argv(
        &self,
        caller: Vec<String>,
        owned: Vec<String>,
    ) -> Result<Vec<String>, ApiError> {
        if !owned.is_empty() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "Hermes rejects unrecognized owned argv",
            ));
        }
        let args = parse(&caller)?;
        let mut result = vec![
            "--profile".into(),
            args.profile,
            "--cli".into(),
            "chat".into(),
        ];
        for (flag, value) in args.values {
            result.extend([flag, value]);
        }
        Ok(result)
    }
    fn prepare_launch(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
        _: &crate::harness::registry::AdmittedHandle,
        _: &LocalSetupStatus,
        _: &dyn crate::harness::launch::CodexShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchPreparation, ApiError> {
        Err(error(
            ErrorCode::UnsupportedHarness,
            "Hermes callbacks cannot qualify prelaunch",
        ))
    }
    fn prepare_prelaunch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        state: &(dyn Any + Send + Sync),
        status: &LocalSetupStatus,
        _: &dyn crate::harness::launch::CodexShellProbe,
        budget: &CallBudget,
    ) -> Result<LaunchPreparation, ApiError> {
        if budget.is_exhausted(request.environment.clock.as_ref()) {
            return Err(error(
                ErrorCode::DeadlineExceeded,
                "Hermes prelaunch budget exhausted",
            ));
        }
        let captured = state.downcast_ref::<Captured>().ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "wrong launch-only observation type",
            )
        })?;
        let current = owned(request, &captured.observation)?;
        let hook = current.launch_hook.unwrap();
        if status.enabled != Some(true)
            || status.configured_hook.as_ref() != Some(&hook)
            || status.fingerprint.as_deref() != Some(captured.fingerprint.as_str())
        {
            return Err(error(
                ErrorCode::Conflict,
                "Hermes configuration changed during preparation",
            ));
        }
        Ok(LaunchPreparation {
            argv: self.compose_argv(request.argv.clone(), vec![])?,
            hook,
            working_directory: scope.working_directory.clone(),
            environment_overrides: BTreeMap::new(),
            report: serde_json::json!({"hermes":{"profile":captured.observation.profile.profile,"home":captured.observation.profile.home,
            "identity":captured.observation.profile.identity,"identity_provenance":"startup_captured_prelaunch_observation",
            "environment_scope":"declared_child_input_plus_native_bootstrap_profile_effects","api":"presence_only","callback_qualified":false,"native_acceptance":"unmet"}}),
        })
    }
    fn recheck_prelaunch(
        &self,
        request: &LaunchRequest,
        scope: &LaunchScope,
        state: &(dyn Any + Send + Sync),
        budget: &CallBudget,
    ) -> Result<String, ApiError> {
        let captured = state.downcast_ref::<Captured>().ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "wrong launch-only observation type",
            )
        })?;
        let current = observe(request, scope, budget)?;
        if current.observation_fingerprint != captured.observation.observation_fingerprint {
            return Err(error(
                ErrorCode::Conflict,
                "Hermes API/profile/config observation changed before submission",
            ));
        }
        Ok(owned(request, &current)?.launch_hook.unwrap().fingerprint)
    }
    fn configuration_fingerprint(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
    ) -> Result<String, ApiError> {
        Err(error(
            ErrorCode::UnsupportedHarness,
            "Hermes requires budgeted launch-only reinspection",
        ))
    }
    fn expected_host_kinds(&self) -> &'static [&'static str] {
        &["hermes"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_grammar_fixtures_preserve_owned_chat_profile_and_refuse_ambiguous_forms() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/hermes/launch-argv.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["evidence_stage"],
            "source_grammar_and_synthetic_policy"
        );
        for case in fixture["cases"].as_array().unwrap() {
            let input: Vec<String> = serde_json::from_value(case["input"].clone()).unwrap();
            let result = POLICY.compose_argv(input, vec![]);
            if case["refused"] == true {
                assert!(result.is_err(), "accepted {case}");
            } else {
                let expected: Vec<String> =
                    serde_json::from_value(case["expected"].clone()).unwrap();
                assert_eq!(result.unwrap(), expected);
            }
        }
    }
}
