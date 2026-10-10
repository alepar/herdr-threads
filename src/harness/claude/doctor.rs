//! Claude's doctor surfaces: the `hooks.claude` projection, its PATH/observed/verdict text
//! helpers, its debug and summary text, and its repair rules. `cli::doctor` keeps only the
//! harness-neutral report assembly and renders this module through the adapter seam.
use crate::{
    cli::doctor::{
        AdmissionState, InstalledHarnessJson, captured_legacy_status, clean, event_registration,
        installed_declared, legacy_registration_line, legacy_summary_lines, repairs_allowed,
        scalar,
    },
    harness::{adapter::DoctorTextForm, context::Harness},
    protocol::{
        results::{HarnessState, HarnessStatesReport},
        time::CallBudget,
    },
};
use serde_json::{Value, json};

/// No `claude` on PATH: `not_found` with nulls.
pub fn claude_installed_stub() -> InstalledHarnessJson {
    InstalledHarnessJson {
        binary: None,
        version: None,
        admission: AdmissionState::NotFound,
        recipe: None,
    }
}

/// The `claude` a hook would resolve on `path`, run with `--version` (bounded
/// by the version deadline) and classified through the admission ladder.
/// `lookup` is the environment the test-only recipe override is read from.
/// A binary that cannot be run or reports no recognizable version is refused
/// with a null version; a version the ladder refuses keeps the observed
/// version and has no recipe.
#[cfg(test)]
pub(crate) fn claude_installed_on(
    path: Option<&std::ffi::OsStr>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> InstalledHarnessJson {
    claude_installed_with_warning(path, lookup).0
}

/// [`claude_installed_on`] plus the doctor warning its admission earns: the
/// optimistic label (with where to report problems) for an unlisted version
/// the ladder admits, or the known-broken refusal text naming the broken
/// range and the newest working version. `None` for listed, other refusals
/// and an absent `claude`.
#[cfg(test)]
pub(crate) fn claude_installed_with_warning(
    path: Option<&std::ffi::OsStr>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> (InstalledHarnessJson, Option<String>) {
    claude_installed_with_warning_budget(path, lookup, crate::harness::process::VERSION_TIMEOUT)
}
#[cfg(test)]
pub(crate) fn claude_installed_with_warning_budget(
    path: Option<&std::ffi::OsStr>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
    timeout: std::time::Duration,
) -> (InstalledHarnessJson, Option<String>) {
    use crate::harness::{admission, claude};
    let Some(binary) = crate::cli::hook::resolve_on_path("claude", path) else {
        return (claude_installed_stub(), None);
    };
    let version = if timeout.is_zero() {
        None
    } else {
        crate::harness::process::version_output(&binary, timeout)
            .ok()
            .and_then(|stdout| claude::version_from_output(&stdout))
    };
    let table = claude::admission_table_with(lookup);
    let mut warning = None;
    let (admission, recipe) = match version
        .as_deref()
        .map(|observed| claude::admit_in(table, observed))
    {
        Some(Ok(admitted)) => (
            match &admitted.admission {
                claude::ClaudeAdmission::Listed => AdmissionState::Listed,
                claude::ClaudeAdmission::Optimistic(optimistic) => {
                    warning = Some(format!(
                        "claude {}: {}",
                        version.as_deref().unwrap_or_default(),
                        crate::harness::optimistic_label(optimistic, true)
                    ));
                    AdmissionState::Optimistic
                }
            },
            Some(admitted.recipe.id.to_owned()),
        ),
        Some(Err(_)) | None => {
            if let Some(observed) = version.as_deref()
                && let admission::Row::Refused(admission::Refusal::KnownBroken {
                    range,
                    newest_working,
                }) = admission::classify(table, observed, || None)
            {
                warning = Some(crate::harness::known_broken_label(
                    "claude",
                    observed,
                    &range,
                    newest_working,
                ));
            }
            (AdmissionState::Refused, None)
        }
    };
    (
        InstalledHarnessJson {
            binary: Some(binary.display().to_string()),
            version,
            admission,
            recipe,
        },
        warning,
    )
}

/// The one text line for the Claude PATH check.
pub(crate) fn claude_path_line(installed: &Value) -> String {
    match installed["binary"].as_str() {
        None => "claude on PATH: not found".to_owned(),
        Some(binary) => {
            let version = installed["version"]
                .as_str()
                .map_or(String::new(), |version| format!(" {version}"));
            format!(
                "claude on PATH: {}{version} ({})",
                clean(binary),
                scalar(&installed["admission"])
            )
        }
    }
}

/// What `hooks.claude.observed` states: whether native Claude hook runs are
/// verified. Cooperative mode never records them, which is not a failure.
pub const CLAUDE_NOT_OBSERVABLE: &str =
    "not observable: Claude hook runs are not recorded (cooperative mode)";

/// What `hooks.claude.observed` says for the daemon's view of `claude`. Only a
/// cooperative harness carries the "(cooperative mode)" label; an unsupported
/// one (refused or not installed) prints the daemon's own reason: its Health
/// line, else `verdict` (the daemon's version verdict line for claude), else a
/// pointer to the verdict.
pub fn claude_observed_text(
    state: HarnessState,
    limitations: &[String],
    notes: &[String],
    verdict: Option<&str>,
) -> String {
    match state {
        HarnessState::Unknown => "unknown".into(),
        HarnessState::Supported => "observed".into(),
        HarnessState::Cooperative => CLAUDE_NOT_OBSERVABLE.into(),
        HarnessState::Unsupported => {
            let reason = limitations
                .iter()
                .chain(notes)
                .find_map(|line| {
                    line.strip_prefix("harness claude unsupported: ")
                        .or_else(|| line.strip_prefix("harness claude not installed: "))
                })
                .or(verdict)
                .unwrap_or(
                    "the daemon refused the installed claude version (see its version verdict below)",
                );
            format!("not observable: {reason}")
        }
    }
}

/// The daemon's version verdict line for claude: the `detected` line, else the
/// newest version row's `line`.
fn claude_verdict_line(states: Option<&HarnessStatesReport>) -> Option<String> {
    let claude = states?.harnesses.iter().find(|h| h.harness == "claude")?;
    claude
        .detected
        .as_ref()
        .map(|detected| detected.line.clone())
        .or_else(|| claude.versions.first().map(|row| row.line.clone()))
}

/// The one safe repair Claude offers: installing missing hooks it can inspect, behind the
/// report-wide write gate.
pub(crate) fn safe_repairs(
    report: &Value,
    claude: &Value,
) -> Vec<crate::harness::adapter::LocalRepair> {
    use crate::harness::adapter::LocalRepair;
    if repairs_allowed(report)
        && claude["setup"]["installed"] == json!(false)
        && claude["setup"]["error"].is_null()
        && claude["error"].is_null()
        && claude["installed"]["binary"].is_string()
        && matches!(
            claude["installed"]["admission"].as_str(),
            Some("contract_declared" | "listed" | "optimistic")
        )
    {
        vec![LocalRepair::InstallOwned]
    } else {
        vec![]
    }
}

/// Manual repair rows Claude contributes for its projected `hooks.claude` object.
pub(crate) fn manual_repairs(claude: &Value) -> Vec<Value> {
    if claude["error"].is_null() {
        return vec![];
    }
    vec![json!({
        "action": "setup claude",
        "outcome": "refused",
        "detail": "hook ownership could not be inspected; inspect doctor --debug before setup"
    })]
}

pub(super) fn legacy_claude_projection(
    request: &crate::harness::adapter::StatusRequest,
    daemon: &Value,
    _budget: &CallBudget,
) -> crate::harness::adapter::DoctorProjection {
    use crate::harness::adapter::*;
    let env = crate::cli::setup::SetupEnv::from_snapshot(&request.environment);
    let path = request.environment.path.clone();
    let mut limitations = Vec::new();
    let state = match daemon["daemon"]["harness_claude"].as_str() {
        Some("supported") => HarnessState::Supported,
        Some("cooperative") => HarnessState::Cooperative,
        Some("unsupported") => HarnessState::Unsupported,
        _ => HarnessState::Unknown,
    };
    let lines = |key: &str| {
        daemon["daemon"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let states =
        serde_json::from_value::<HarnessStatesReport>(daemon["harness_states"].clone()).ok();
    let observed_claude = claude_observed_text(
        state,
        &lines("limitations"),
        &lines("notes"),
        claude_verdict_line(states.as_ref()).as_deref(),
    );
    let mut claude = json!({
        "scope": "user",
        "recipes": crate::harness::recipe::describe(super::RECIPES),
        "compaction_recovery": "unavailable until current runtime is separately qualified; use resume/clear or herdr-threads summary",
        "observed": observed_claude,
    });
    // Problems found in this environment (the one the harnesses run in):
    // an installed harness whose hooks are missing. Each makes the result `degraded`.

    let claude_binary = crate::cli::hook::resolve_on_path("claude", path.as_deref());
    match crate::cli::setup::user_inspection(Harness::Claude, &env) {
        Ok((settings, inspection)) => {
            if let Some(binary) = &claude_binary
                && !inspection.installed
            {
                limitations.push(format!(
                    "claude is on PATH ({}) but its hooks are not installed in {}: run \
                     `herdr-threads setup claude`",
                    binary.display(),
                    settings.display()
                ));
            }
            claude["setup"] = json!({
                "settings": settings.display().to_string(),
                "installed": inspection.installed,
                "event_registration": event_registration(inspection.legacy_event_registration),
                "adopted": inspection.adopted.as_ref().map(|adoption| json!({
                    "owner": adoption.owner,
                    "recorded": adoption.recorded,
                })),
            });
            claude["allow_rule"] = crate::harness::setup::allow_rule_json(
                super::settings_kind(),
                inspection.allow_rule.as_ref(),
            );
            claude["prompt_suggestions"] = super::setup::prompt_suggestion_status(
                &env,
                &settings,
                request
                    .environment
                    .declared
                    .get(super::PROMPT_SUGGESTION_ENV)
                    .cloned(),
            );
        }
        Err(error) => {
            if claude_binary.is_some() {
                limitations.push(format!("claude hooks cannot be inspected: {error}"));
            }
            claude["setup"] = json!({"installed": false});
            claude["error"] = json!(error);
        }
    }
    let claude_installed = installed_declared("claude", super::RECIPES[0].id, path.as_deref());
    let claude_warning: Option<String> = None;
    claude["installed"] = json!(claude_installed);
    // With the daemon's answer the PATH version's verdict is the
    // detected-version line of the harness block; without it the admission
    // warning stands as before.
    claude["admission_warning"] = json!(if daemon["harness_states"].is_object() {
        None
    } else {
        claude_warning
    });

    let safe_repairs = safe_repairs(daemon, &claude);
    let manual_repairs = manual_repairs(&claude);
    DoctorProjection {
        status: Some(captured_legacy_status(request, &claude)),
        hooks: claude,
        limitations,
        manual_repairs,
        safe_repairs,
        repair_options: [("keep-prompt-suggestions".into(), true)]
            .into_iter()
            .collect(),
    }
}

/// Text for `report.hooks.claude`.
pub(super) fn doctor_text(claude: &Value, form: DoctorTextForm) -> String {
    match form {
        DoctorTextForm::Summary => legacy_summary_lines("claude", claude),
        DoctorTextForm::Debug => debug_text(claude),
    }
}

fn debug_text(claude: &Value) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "hooks.claude.settings: {}\n",
        scalar(&claude["setup"]["settings"])
    ));
    out.push_str(&format!(
        "hooks.claude.setup_installed: {}\n",
        if claude["setup"]["installed"] == json!(true) {
            "yes"
        } else {
            "no"
        }
    ));
    if let Some(line) = legacy_registration_line("claude", &claude["setup"], "") {
        out.push_str(&line);
    }
    let claude_installed = &claude["installed"];
    out.push_str(&claude_path_line(claude_installed));
    out.push('\n');
    for key in ["binary", "version", "admission", "recipe"] {
        out.push_str(&format!(
            "hooks.claude.installed.{key}: {}\n",
            scalar(&claude_installed[key])
        ));
    }
    if claude["admission_warning"].is_string() {
        out.push_str(&format!(
            "hooks.claude.warning: {}\n",
            scalar(&claude["admission_warning"])
        ));
    }
    let allow_rule = &claude["allow_rule"];
    if allow_rule.is_object() {
        out.push_str(&format!(
            "hooks.claude.allow_rule.rule: {}\n",
            scalar(&allow_rule["rule"])
        ));
        out.push_str(&format!(
            "hooks.claude.allow_rule.ownership: {}\n",
            scalar(&allow_rule["ownership"])
        ));
        out.push_str(&format!(
            "hooks.claude.allow_rule.present: {}\n",
            if allow_rule["present"] == json!(true) {
                "yes"
            } else {
                "no"
            }
        ));
        if allow_rule["note"].is_string() {
            out.push_str(&format!(
                "hooks.claude.allow_rule.note: {}\n",
                scalar(&allow_rule["note"])
            ));
        }
    }
    let suggestions = &claude["prompt_suggestions"];
    if suggestions.is_object() {
        out.push_str(&format!(
            "hooks.claude.prompt_suggestions: {} (`{}` in {}{})\n",
            scalar(&suggestions["state"]),
            scalar(&suggestions["setting"]),
            scalar(&claude["setup"]["settings"]),
            if suggestions["set_by_setup"] == json!(true) {
                ", set by setup"
            } else {
                ""
            }
        ));
        if suggestions["env_override"].is_string() {
            out.push_str(&format!(
                "hooks.claude.prompt_suggestions.env_override: {}\n",
                scalar(&suggestions["env_override"])
            ));
        }
        if suggestions["note"].is_string() {
            out.push_str(&format!(
                "hooks.claude.prompt_suggestions.note: {}\n",
                scalar(&suggestions["note"])
            ));
        }
    }
    out.push_str(&format!(
        "hooks.claude.observed: {}\n",
        scalar(&claude["observed"])
    ));
    out.push_str(&format!(
        "hooks.claude.recipes: {}\n",
        scalar(&claude["recipes"])
    ));
    out.push_str(&format!(
        "hooks.claude.compaction_recovery: {}\n",
        scalar(&claude["compaction_recovery"])
    ));
    if !claude["error"].is_null() {
        out.push_str(&format!(
            "hooks.claude.error: {}\n",
            scalar(&claude["error"])
        ));
    }
    out
}
