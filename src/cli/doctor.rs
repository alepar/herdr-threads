//! `doctor`: bounded local diagnostics and explicit owned repairs. A plain
//! check remains read-only. It reports the resolved context,
//! state directory safety, daemon reachability/health, and the owned Claude
//! user-level Claude and Codex hook installations, keeping "installed" separate
//! from "observed" native capability.

use super::{RunError, commands::ParsedCli, exit};
use crate::{
    app::SystemClock,
    client::local::LocalSocketClient,
    daemon::{
        logs::daemon_log_path,
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, check_owned_state_root, check_private_dir, is_unsafe_local_state},
    },
    harness::context::Harness,
    ports::LocalClient,
    protocol::{
        commands::Command,
        output::OutputFormat,
        results::{
            CommandResult, ErrorCode, HarnessState, HarnessStatesReport, Health, HealthState,
        },
        time::{CallBudget, Clock, MonoInstant},
        wire::PROTOCOL_VERSION,
    },
    view::escape::{Context, escape_for_terminal},
};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};

const HEALTH_BUDGET_MS: u64 = 2_000;

/// The closed set of doctor admission strings (ht-p03.47).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AdmissionState {
    #[serde(rename = "listed")]
    Listed,
    /// Emitted for Codex only.
    #[serde(rename = "schema-matched, live-unverified")]
    SchemaMatched,
    #[serde(rename = "optimistic")]
    Optimistic,
    #[serde(rename = "refused")]
    Refused,
    /// No such harness on PATH.
    #[serde(rename = "not_found")]
    NotFound,
}

impl AdmissionState {
    pub const ALL: [AdmissionState; 5] = [
        AdmissionState::Listed,
        AdmissionState::SchemaMatched,
        AdmissionState::Optimistic,
        AdmissionState::Refused,
        AdmissionState::NotFound,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AdmissionState::Listed => "listed",
            AdmissionState::SchemaMatched => crate::harness::codex::SCHEMA_MATCHED_LABEL,
            AdmissionState::Optimistic => crate::harness::codex::OPTIMISTIC_LABEL,
            AdmissionState::Refused => "refused",
            AdmissionState::NotFound => "not_found",
        }
    }
}

/// The `hooks.<harness>.installed` object: the harness binary on PATH and how
/// it was admitted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstalledHarnessJson {
    pub binary: Option<String>,
    pub version: Option<String>,
    pub admission: AdmissionState,
    pub recipe: Option<String>,
}

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
fn claude_installed_on(
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
fn claude_installed_with_warning(
    path: Option<&std::ffi::OsStr>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> (InstalledHarnessJson, Option<String>) {
    claude_installed_with_warning_budget(path, lookup, crate::harness::codex::VERSION_TIMEOUT)
}
fn claude_installed_with_warning_budget(
    path: Option<&std::ffi::OsStr>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
    timeout: std::time::Duration,
) -> (InstalledHarnessJson, Option<String>) {
    use crate::harness::{admission, claude};
    let Some(binary) = super::hook::resolve_on_path("claude", path) else {
        return (claude_installed_stub(), None);
    };
    let version = if timeout.is_zero() {
        None
    } else {
        crate::harness::codex::version_output(&binary, timeout)
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
fn claude_path_line(installed: &Value) -> String {
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

fn codex_installed_json(codex: &crate::harness::codex::InstalledAdmission) -> InstalledHarnessJson {
    use crate::harness::codex::{Admission, InstalledRefusal};
    let (admission, version, recipe) = match &codex.result {
        Ok(version) => (
            match version.admission() {
                Admission::Listed => AdmissionState::Listed,
                Admission::SchemaMatched { .. } => AdmissionState::SchemaMatched,
                Admission::Optimistic { .. } => AdmissionState::Optimistic,
            },
            Some(version.as_str().to_owned()),
            Some(version.recipe().id.to_owned()),
        ),
        Err(InstalledRefusal::NotFound) => (AdmissionState::NotFound, None, None),
        Err(InstalledRefusal::Refused(_)) => (AdmissionState::Refused, None, None),
    };
    InstalledHarnessJson {
        binary: codex.binary.as_ref().map(|path| path.display().to_string()),
        version,
        admission,
        recipe,
    }
}

fn harness_state(state: HarnessState) -> &'static str {
    match state {
        HarnessState::Unknown => "unknown",
        HarnessState::Unsupported => "unsupported",
        HarnessState::Cooperative => "cooperative",
        HarnessState::Supported => "supported",
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

enum Daemon {
    NotRunning,
    Unreachable(String),
    Reachable(Box<Health>, Box<DaemonDetails>),
}
struct DaemonDetails {
    states: Result<HarnessStatesReport, String>,
    rich: Result<crate::protocol::results::HarnessHealthV2Report, String>,
}

/// Negotiate once, with the exact Health call's deadline and cancellation.
/// Legacy projections may remain useful after a rich failure, but are never
/// represented as a complete rich answer.
fn daemon_details(client: &dyn LocalClient, budget: &CallBudget) -> DaemonDetails {
    use crate::protocol::capabilities::{Capabilities, HARNESS_HEALTH_V2, HARNESS_STATES};
    let capabilities = match client.call(Command::Capabilities, budget) {
        Ok(CommandResult::Capabilities(list)) => Ok(Capabilities::from_list(list.capabilities)),
        Ok(_) => Err(
            "capabilities returned another result; rich runtime evidence unavailable".to_owned(),
        ),
        Err(error) => Err(format!(
            "capabilities failed: {} ({}); rich runtime evidence unavailable",
            error.detail,
            exit::code_name(&error.code)
        )),
    };
    let rich = match &capabilities {
        Err(error) => Err(error.clone()),
        Ok(capabilities) if !capabilities.supports(HARNESS_HEALTH_V2) => Err("unsupported: the daemon does not advertise harness.health_v2 (an older daemon); legacy Health/harness.states do not include rich runtime evidence".into()),
        Ok(_) => match client.call(Command::HarnessHealthV2, budget) {
            Ok(CommandResult::HarnessHealthV2(report)) => report.validate().map(|()| report).map_err(|error| format!("advertised harness.health_v2 invalid result: {error}")),
            Ok(_) => Err("advertised harness.health_v2 returned another result".into()),
            Err(error) => Err(format!("advertised harness.health_v2 failed: {} ({})", error.detail, exit::code_name(&error.code))),
        },
    };
    let states = if capabilities
        .as_ref()
        .is_ok_and(|capabilities| capabilities.supports(HARNESS_STATES))
    {
        match client.call(Command::HarnessStates, budget) {
            Ok(CommandResult::HarnessStates(report)) => Ok(report),
            Ok(_) => Err("the daemon answered harness.states with another result".into()),
            Err(error) => Err(format!(
                "harness.states failed: {} ({})",
                error.detail,
                exit::code_name(&error.code)
            )),
        }
    } else {
        Err("the daemon does not advertise harness.states (an older daemon)".into())
    };
    DaemonDetails { states, rich }
}

fn probe_daemon(
    paths: &InstancePaths,
    clock: &Arc<dyn Clock>,
    budget: &CallBudget,
) -> Result<Daemon, String> {
    let instance = match read_existing_namespace(paths) {
        Ok(Some(instance)) => instance,
        Ok(None) => return Ok(Daemon::NotRunning),
        Err(error) => return Err(format!("namespace: {error}")),
    };
    let descriptor = match read_descriptor(paths, instance) {
        Ok(descriptor) => descriptor,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Daemon::NotRunning),
        Err(error) => return Err(format!("endpoint descriptor: {error}")),
    };
    if descriptor.protocol_version != PROTOCOL_VERSION {
        return Ok(Daemon::Unreachable(
            crate::daemon::lifecycle::skew_error(
                ErrorCode::UnknownWireVersion,
                &descriptor.software_version,
                descriptor.protocol_version,
            )
            .detail,
        ));
    }
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(clock),
        instance,
        Some(descriptor.boot_id),
    );
    match client.call(Command::Health, budget) {
        Ok(CommandResult::Health(health)) => {
            let details = daemon_details(&client, budget);
            Ok(Daemon::Reachable(Box::new(health), Box::new(details)))
        }
        Ok(_) => Ok(Daemon::Unreachable(
            "daemon answered health with another result".into(),
        )),
        Err(error) => Ok(Daemon::Unreachable(format!(
            "endpoint published but health failed: {} ({})",
            error.detail,
            exit::code_name(&error.code)
        ))),
    }
}

/// First existing plugin-created private directory (`instances/`, then the
/// instance directory) that fails the 0700 ownership checks `daemon ensure`
/// applies. Absent directories are fine: ensure creates them 0700.
/// Only the typed unsafe-state marker counts, exactly as `daemon ensure`
/// classifies it, so the two commands cannot disagree.
fn unsafe_private_dir(paths: &InstancePaths) -> Option<String> {
    let instances = paths.instance_dir.parent()?;
    for dir in [instances, paths.instance_dir.as_path()] {
        match check_private_dir(dir) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) if is_unsafe_local_state(&error) => return Some(error.to_string()),
            // Other I/O failures are what ensure reports as unavailable (3).
            Err(_) => return None,
        }
    }
    None
}

/// Assemble the report and its exit status. Split from rendering for tests.
/// The effective harness-manifest fetch policy and the cache state, for
/// `report["harness_manifest"]`. Informational only: never a limitation.
/// `offline_env` is this process's `HERDR_THREADS_OFFLINE`; the daemon reads
/// its own environment (and `settings.json`) once at start.
pub(crate) fn harness_manifest_report(
    instance_dir: &std::path::Path,
    offline_env: Option<&std::ffi::OsStr>,
) -> Value {
    use crate::harness::manifest::{
        SUPPORTED_SCHEMA_VERSION, cache_dir, format_rfc3339_utc, policy_from, policy_words,
        read_meta,
    };
    let meta = read_meta(&cache_dir(instance_dir));
    let cache_fetched_at = meta.fetched_at_ms.map(format_rfc3339_utc);
    let (policy, source, settings_error, policy_from_who, policy_recorded_at) =
        if let Some(recorded) = &meta.daemon_policy {
            (
                json!(recorded.policy),
                json!(recorded.source),
                Value::Null,
                "daemon",
                json!(format_rfc3339_utc(recorded.recorded_at_ms)),
            )
        } else {
            match crate::daemon::settings::load(instance_dir) {
                Ok(settings) => {
                    let (policy, source) = policy_words(policy_from(&settings, offline_env));
                    (
                        json!(policy),
                        json!(source),
                        Value::Null,
                        "this environment",
                        Value::Null,
                    )
                }
                Err(error) => (
                    Value::Null,
                    Value::Null,
                    json!(error.to_string()),
                    "this environment",
                    Value::Null,
                ),
            }
        };
    json!({
        "policy": policy,
        "source": source,
        "settings_error": settings_error,
        "cache_fetched_at": cache_fetched_at,
        "cache_etag": meta.etag,
        "embedded_schema_version": SUPPORTED_SCHEMA_VERSION,
        "policy_from": policy_from_who,
        "policy_recorded_at": policy_recorded_at,
    })
}

/// The text lines for [`harness_manifest_report`].
fn harness_manifest_text(manifest: &Value) -> String {
    let mut out = String::new();
    if let Some(error) = manifest["settings_error"].as_str() {
        out.push_str(&format!("harness manifest: settings error: {error}\n"));
    } else {
        let recorded = manifest["policy_recorded_at"]
            .as_str()
            .filter(|_| manifest["policy_from"] == "daemon");
        match (manifest["policy"].as_str(), manifest["source"].as_str(), recorded) {
            (Some("off"), Some("settings"), None) => {
                out.push_str("harness manifest: off (settings.json)\n")
            }
            (Some("off"), Some("settings"), Some(at)) => out.push_str(&format!(
                "harness manifest: off (settings.json; recorded at daemon start {at})\n"
            )),
            (Some("off"), _, None) => out.push_str(
                "harness manifest: off (HERDR_THREADS_OFFLINE=1 in this environment; the daemon reads its own environment at start)\n",
            ),
            (Some("off"), _, Some(at)) => out.push_str(&format!(
                "harness manifest: off (HERDR_THREADS_OFFLINE=1 in the daemon's environment; recorded at daemon start {at})\n"
            )),
            (_, _, Some(at)) => out.push_str(&format!(
                "harness manifest: auto (recorded at daemon start {at})\n"
            )),
            _ => out.push_str("harness manifest: auto\n"),
        }
    }
    match manifest["cache_fetched_at"].as_str() {
        Some(at) => out.push_str(&format!(
            "manifest cache: fetched {at} (etag {})\n",
            manifest["cache_etag"].as_str().unwrap_or("none")
        )),
        None => out.push_str("manifest cache: never fetched (using the embedded copy)\n"),
    }
    out
}

/// The per-harness verdict blocks (`harness.states`), then the reason doctor
/// has none when the daemon could not answer. Everything is escaped.
fn harness_states_text(report: &Value) -> String {
    let mut out = String::new();
    let Some(harnesses) = report["harness_states"]["harnesses"].as_array() else {
        if let Some(why) = report["harness_states_unavailable"].as_str() {
            out.push_str(&format!("harness states unavailable: {}\n", clean(why)));
        }
        return out;
    };
    for harness in harnesses {
        let name = scalar(&harness["harness"]);
        let versions = harness["versions"].as_array().cloned().unwrap_or_default();
        let detected = &harness["detected"];
        if let Some(newest) = versions.first() {
            out.push_str(&format!(
                "harness {name}: {} {} \u{2014} {}\n",
                scalar(&newest["state"]),
                scalar(&newest["version"]),
                scalar(&newest["source"])
            ));
        } else if detected.is_object() {
            out.push_str(&format!(
                "harness {name}: {} {} (on PATH, no session yet) \u{2014} {}\n",
                scalar(&detected["state"]),
                scalar(&detected["version"]),
                scalar(&detected["line"])
            ));
        }
        if detected.is_object() && !versions.is_empty() {
            out.push_str(&format!(
                "  {name} {} (on PATH, no session yet): {} \u{2014} {}\n",
                scalar(&detected["version"]),
                scalar(&detected["state"]),
                scalar(&detected["line"])
            ));
        }
        for (index, row) in versions.iter().enumerate() {
            // Other rows still in the Health window.
            if index > 0 && row["in_health_window"] == json!(true) {
                out.push_str(&format!(
                    "  {name} {}: {} \u{2014} {}\n",
                    scalar(&row["version"]),
                    scalar(&row["state"]),
                    scalar(&row["source"])
                ));
            }
            if row["state"] == json!("broken")
                && (index == 0 || row["in_health_window"] == json!(true))
            {
                out.push_str(&format!("  {}\n", scalar(&row["line"])));
            }
            if index == 0 || row["in_health_window"] == json!(true) {
                for note in row["notes"].as_array().into_iter().flatten() {
                    out.push_str(&format!("  note: {}\n", scalar(note)));
                }
                if let Some(url) = row["issue_url"].as_str() {
                    out.push_str(&format!("  issue: {}\n", clean(url)));
                }
            }
        }
        let newest_seen = versions
            .first()
            .and_then(|row| row["last_seen_at"].as_u64())
            .unwrap_or(0);
        if let (Some(reason), Some(at)) = (
            harness["unattributed"]["reason"].as_str(),
            harness["unattributed"]["at"].as_u64(),
        ) && at > newest_seen
        {
            out.push_str(&format!(
                "version evidence unavailable: {} ({})\n",
                clean(reason),
                crate::harness::manifest::format_rfc3339_utc(i64::try_from(at).unwrap_or(i64::MAX))
            ));
        }
        if let Some(count) = harness["hook_parse_failures"].as_u64().filter(|n| *n > 0) {
            out.push_str(&format!("hook payloads not understood: {count}\n"));
        }
    }
    out
}

fn local_timeout(
    request: &crate::harness::adapter::StatusRequest,
    budget: &CallBudget,
) -> std::time::Duration {
    if budget.cancellation.is_cancelled() {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_millis(
        budget
            .deadline
            .0
            .saturating_sub(request.environment.clock.monotonic_now().0),
    )
    .min(crate::harness::codex::VERSION_TIMEOUT)
}

fn captured_legacy_status(
    request: &crate::harness::adapter::StatusRequest,
    hooks: &Value,
) -> crate::harness::adapter::SetupStatus {
    use crate::harness::adapter::*;
    if let Some(error) = hooks["error"]
        .as_str()
        .or_else(|| hooks["setup"]["error"].as_str())
    {
        return SetupStatus::Failed(SetupFailure::Invalid(error.chars().take(1024).collect()));
    }
    let admitted = match hooks["installed"]["admission"].as_str() {
        Some("listed" | "optimistic" | "schema-matched, live-unverified") => Some(true),
        Some("refused") if hooks["installed"]["version"].is_string() => Some(false),
        _ => None,
    };
    let mut diagnostics = vec![SetupDiagnostic::new(
        "native_enablement_unknown",
        DiagnosticSeverity::Info,
        "Native enablement and runtime observation are separate from local installation",
    )];
    if hooks["installed"]["binary"].is_string() && hooks["installed"]["version"].is_null() {
        diagnostics.push(SetupDiagnostic::new(
            "native_observation_unavailable",
            DiagnosticSeverity::Warning,
            "Native version/admission observation unavailable within doctor budget",
        ));
    }
    for error in [
        &hooks["error"],
        &hooks["setup"]["error"],
        &hooks["installed"]["error"],
    ] {
        if let Some(error) = error.as_str() {
            diagnostics.push(SetupDiagnostic::new(
                "local_inspection_unavailable",
                DiagnosticSeverity::Warning,
                error,
            ));
        }
    }
    SetupStatus::Detailed(Box::new(LocalSetupStatus {
        scope: request.scope.clone(),
        installed: hooks["setup"]["installed"].as_bool().unwrap_or(false),
        enabled: None,
        admitted,
        observed: None,
        configured_hook: None,
        fingerprint: None,
        diagnostics,
        repairs: vec![],
        projection: hooks.clone(),
    }))
}

pub(crate) fn legacy_claude_projection(
    request: &crate::harness::adapter::StatusRequest,
    daemon: &Value,
    budget: &CallBudget,
) -> crate::harness::adapter::DoctorProjection {
    use crate::harness::adapter::*;
    let env = super::setup::SetupEnv::from_snapshot(&request.environment);
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
        "recipes": crate::harness::recipe::describe(crate::harness::claude::RECIPES),
        "compaction_recovery": crate::harness::claude::compaction_recovery(),
        "observed": observed_claude,
    });
    // Problems found in this environment (the one the harnesses run in):
    // an installed harness whose hooks are missing, a refused codex, or a
    // Codex sandbox warning. Each makes the result `degraded`.

    let claude_binary = super::hook::resolve_on_path("claude", path.as_deref());
    match super::setup::user_inspection(Harness::Claude, &env) {
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
            claude["allow_rule"] = super::setup::allow_rule_json(inspection.allow_rule.as_ref());
            claude["prompt_suggestions"] = super::setup::prompt_suggestion_status(
                &env,
                &settings,
                request
                    .environment
                    .declared
                    .get(crate::harness::claude::PROMPT_SUGGESTION_ENV)
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
    let (claude_installed, claude_warning) = claude_installed_with_warning_budget(
        path.as_deref(),
        |key| request.environment.declared.get(key).cloned(),
        local_timeout(request, budget),
    );
    claude["installed"] = json!(claude_installed);
    // With the daemon's answer the PATH version's verdict is the
    // detected-version line of the harness block; without it the admission
    // warning stands as before.
    claude["admission_warning"] = json!(if daemon["harness_states"].is_object() {
        None
    } else {
        claude_warning
    });

    let mut projected = daemon.clone();
    projected["hooks"] = json!({"claude": claude});
    let safe_repairs = if repair_plan(&projected).contains(&Repair::SetupClaude) {
        vec![LocalRepair::InstallOwned]
    } else {
        vec![]
    };
    let mut manual_repairs = Vec::new();
    add_manual_repairs(&projected, &mut manual_repairs);
    manual_repairs.retain(|row| {
        row["outcome"] != "none" && row["action"] == "setup claude"
            || row["action"] == "claude version"
    });
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

pub(crate) fn legacy_codex_projection(
    request: &crate::harness::adapter::StatusRequest,
    daemon: &Value,
    budget: &CallBudget,
) -> crate::harness::adapter::DoctorProjection {
    use crate::harness::adapter::*;
    let env = super::setup::SetupEnv::from_snapshot(&request.environment);
    let path = request.environment.path.clone();
    let mut limitations = Vec::new();
    let codex_inspection = super::setup::user_inspection(Harness::Codex, &env);
    let codex_setup = match &codex_inspection {
        Ok((file, inspection)) => json!({
            "hooks_file": file.display().to_string(),
            "installed": inspection.installed,
            "event_registration": event_registration(inspection.legacy_event_registration),
            "adopted": inspection.adopted.as_ref().map(|adoption| json!({
                "owner": adoption.owner,
                "recorded": adoption.recorded,
            })),
        }),
        Err(error) => json!({"installed": false, "error": error}),
    };
    let codex_trust = if codex_setup["installed"] == json!(true) {
        super::setup::codex_trust_report(&env)
    } else {
        json!({"status": "not_installed"})
    };
    if codex_trust["status"] == "review_required" {
        limitations.push("Codex hook review required; start codex interactively and review herdr-threads hooks in /hooks".into());
    }
    // The `codex` a hook would resolve on this PATH, observed read-only and
    // bounded by the version-observation deadline. A listed version is
    // admitted exactly; an unlisted one only when its embedded hook schemas
    // hash-match a recipe (schema-matched, live-unverified).
    // The hook's private fingerprint cache is consulted read-only (a warm
    // cache answers without rescanning; doctor never writes it), and the
    // admission evidence the latest hook stored is shown beside it.
    let private = request
        .environment
        .state_dir
        .as_deref()
        .and_then(|state| crate::harness::codex_evidence::existing(state).ok());
    let cache = private
        .as_deref()
        .map(crate::harness::codex_evidence::cache_path);
    let codex = match crate::harness::codex::resolve_on_path(path.as_deref()) {
        Some(binary)
            if local_timeout(request, budget).is_zero() || budget.cancellation.is_cancelled() =>
        {
            crate::harness::codex::InstalledAdmission {
                binary: Some(binary),
                result: Err(crate::harness::codex::InstalledRefusal::Refused(
                    crate::harness::codex::VersionError::Unavailable,
                )),
            }
        }
        Some(binary) => crate::harness::codex::InstalledAdmission::observe_binary(
            binary,
            local_timeout(request, budget),
            cache.as_deref().map_or(
                crate::harness::codex_schema::FingerprintCache::Memory,
                crate::harness::codex_schema::FingerprintCache::ReadOnly,
            ),
        ),
        None => crate::harness::codex::InstalledAdmission::observe_on_path(
            None,
            local_timeout(request, budget),
        ),
    };
    let mut installed = json!(codex_installed_json(&codex));
    installed["evidence"] = json!(codex.line());
    if let Some(source) = codex
        .result
        .as_ref()
        .ok()
        .and_then(|version| version.fingerprinted())
    {
        installed["fingerprint_source"] = json!(source.display().to_string());
    }
    let last_hook = private.as_deref().and_then(|private| {
        crate::harness::codex_evidence::read(&crate::harness::codex_evidence::admission_path(
            private,
        ))
    });
    match &codex.result {
        Ok(_) => (),
        Err(refusal) => installed["error"] = json!(refusal.to_string()),
    }
    let sandbox_warning = super::setup::codex_unmeasured_allowance_warning(
        &env,
        codex.result.as_ref().ok().map(|version| version.as_str()),
    );
    let socket_policy_validation = codex.result.as_ref().ok().map(|_| "not_run");
    if let Some(binary) = &codex.binary {
        match &codex.result {
            Err(_) => limitations.push(format!(
                "codex on PATH ({}) is not admitted: {}",
                binary.display(),
                codex.line()
            )),
            Ok(_) => match &codex_inspection {
                Ok((file, inspection)) if !inspection.installed => limitations.push(format!(
                    "codex is on PATH ({}) but its hooks are not installed in {}: run \
                     `herdr-threads setup codex` with this CODEX_HOME",
                    binary.display(),
                    file.display()
                )),
                Ok(_) => (),
                Err(error) => limitations.push(format!("codex hooks cannot be inspected: {error}")),
            },
        }
    }
    if let Some(warning) = &sandbox_warning {
        limitations.push(format!("codex sandbox: {warning}"));
    }
    let roots_warning = super::setup::codex_missing_roots_warning(
        &env,
        codex.result.as_ref().ok().map(|version| version.as_str()),
    );
    if let Some(warning) = &roots_warning {
        limitations.push(format!("codex sandbox: {warning}"));
    }
    let proxy_warnings = super::setup::codex_foreign_proxy_warnings(&env);
    for warning in &proxy_warnings {
        limitations.push(format!("codex sandbox: {warning}"));
    }

    let hooks = json!({
        "scope": "user",
        "detail": "Codex hooks are user-level ($CODEX_HOME/hooks.json, set up by `herdr-threads setup codex`); Codex runs them only once trusted",
        "setup": codex_setup,
        "trust": codex_trust,
        "recipes": crate::harness::recipe::describe(crate::harness::codex::RECIPES),
        "installed": installed,
        "sandbox_warning": sandbox_warning,
        "socket_policy_validation": socket_policy_validation,
        "command_execution": "approved_outside_sandbox",
        "sandbox_roots_warning": roots_warning,
        "sandbox_proxy_warnings": proxy_warnings,
        "last_hook": last_hook.map(|record| json!({
            "binary": record.binary,
            "admission": record.admission,
            "evidence": record.evidence,
            "recorded_unix_ms": record.recorded_unix_ms,
        })),
    });
    let mut projected = daemon.clone();
    projected["hooks"] = json!({"codex": hooks});
    let mut manual_repairs = Vec::new();
    add_manual_repairs(&projected, &mut manual_repairs);
    manual_repairs.retain(|row| {
        row["action"]
            .as_str()
            .is_some_and(|action| action.contains("Codex") || action.contains("codex"))
    });
    DoctorProjection {
        status: Some(captured_legacy_status(request, &hooks)),
        hooks,
        limitations,
        manual_repairs,
        safe_repairs: vec![],
        repair_options: Default::default(),
    }
}

pub fn report(state_dir: Option<PathBuf>, host_endpoint: Option<PathBuf>) -> (Value, i32) {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let budget = observation_budget(&clock);
    let registry = crate::harness::registry::builtins();
    let (mut report, code) = report_base(state_dir, host_endpoint, &clock, &budget, registry);
    let mut environment = doctor_environment(&report, registry, None);
    environment.clock = clock;
    collect_local_with_budget(
        &mut report,
        registry,
        None,
        &Default::default(),
        &environment,
        &budget,
    );
    (report, code)
}

fn doctor_environment(
    report: &Value,
    registry: &crate::harness::registry::Registry,
    supplied: Option<&crate::harness::adapter::SetupEnvironment>,
) -> crate::harness::adapter::SetupEnvironment {
    if let Some(environment) = supplied {
        return environment.clone();
    }
    let context = crate::daemon::paths::RuntimeContext {
        herdr_bin: None,
        state_dir: report["context"]["state_dir"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_default(),
        host_endpoint: report["context"]["host_endpoint"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_default(),
    };
    let mut environment = super::setup::SetupEnv::for_context(&context).snapshot();
    environment.instance_source = report["context"]["source"].clone();
    environment.declared = registry
        .registrations()
        .iter()
        .flat_map(|adapter| adapter.setup_environment_inputs())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
        .collect();
    #[cfg(feature = "test-support")]
    if let Some(value) = std::env::var_os("HT_TEST_RECIPES_JSON") {
        environment
            .declared
            .insert("HT_TEST_RECIPES_JSON".into(), value);
    }
    environment
}
fn observation_budget(clock: &Arc<dyn Clock>) -> CallBudget {
    CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(HEALTH_BUDGET_MS)),
        cancellation: Default::default(),
    }
}
#[cfg(test)]
fn local_budget(environment: &crate::harness::adapter::SetupEnvironment) -> CallBudget {
    observation_budget(&environment.clock)
}
fn scope_json(scope: &crate::harness::adapter::ResolvedSetupScope) -> Value {
    use crate::harness::adapter::ResolvedSetupScope;
    match scope {
        ResolvedSetupScope::ConfigRoot(home) => {
            json!({"kind": "local_config_root", "profile": null, "home": home})
        }
        ResolvedSetupScope::Profile { name, home } => {
            json!({"kind": "local_profile", "profile": name, "home": home})
        }
    }
}
struct OwnedRepair {
    registration: &'static crate::harness::registry::Registration,
    scope: crate::harness::adapter::ResolvedSetupScope,
    options: crate::harness::adapter::SetupOptions,
}
#[cfg(test)]
fn collect_local(
    report: &mut Value,
    registry: &crate::harness::registry::Registry,
    selected: Option<&str>,
    scope: &crate::harness::adapter::SetupScopeRequest,
    environment: &crate::harness::adapter::SetupEnvironment,
) -> Vec<OwnedRepair> {
    collect_local_with_budget(
        report,
        registry,
        selected,
        scope,
        environment,
        &local_budget(environment),
    )
}
fn collect_local_with_budget(
    report: &mut Value,
    registry: &crate::harness::registry::Registry,
    selected: Option<&str>,
    scope: &crate::harness::adapter::SetupScopeRequest,
    environment: &crate::harness::adapter::SetupEnvironment,
    budget: &CallBudget,
) -> Vec<OwnedRepair> {
    use crate::harness::adapter::*;
    if report["context"]["ok"] != true {
        return vec![];
    }
    report["hooks"] = json!({});
    report["local_harnesses"] = json!({});
    let entries: Vec<_> = registry
        .registrations()
        .iter()
        .filter(|registration| selected.is_none_or(|id| id == registration.metadata().id))
        .collect();
    report["adapter_order"] = json!(
        entries
            .iter()
            .map(|registration| registration.metadata().id)
            .collect::<Vec<_>>()
    );
    let mut repairs = Vec::new();
    let mut limitations = report["limitations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for registration in entries {
        let id = registration.metadata().id;
        let resolved = match registration.resolve_setup_scope(scope, environment) {
            Ok(scope) => scope,
            Err(error) => {
                report["local_harnesses"][id] = json!({"scope": {"kind": "local_unresolved", "profile": match scope {SetupScopeRequest::Profile(name) => Some(name), _ => None}}, "installed": null, "enabled": null, "admitted": null, "observed": null, "diagnostics": [{"text": error.to_string()}]});
                continue;
            }
        };
        let request = StatusRequest {
            scope: resolved.clone(),
            environment: environment.clone(),
            native_binary: None,
        };
        let mut projection = registration.doctor_projection(&request, report, budget);
        let status = projection
            .as_mut()
            .and_then(|projection| projection.status.take())
            .unwrap_or_else(|| {
                if budget.cancellation.is_cancelled()
                    || environment.clock.monotonic_now() >= budget.deadline
                {
                    SetupStatus::Unavailable {
                        diagnostic: "doctor observation deadline elapsed or request cancelled"
                            .into(),
                    }
                } else {
                    registration.status(&request, budget)
                }
            });
        let mut local = json!({"scope": scope_json(&resolved), "installed": null, "enabled": null, "admitted": null, "observed": null, "diagnostics": [], "manual_repairs": []});
        match status {
            SetupStatus::Detailed(status) if status.scope == resolved => {
                local["installed"] = json!(status.installed);
                local["enabled"] = json!(status.enabled);
                local["admitted"] = json!(status.admitted);
                local["observed"] = json!(status.observed);
                local["diagnostics"] = json!(status.diagnostics.iter().take(16).map(|row| json!({"code": row.code, "text": row.text, "manual_argv": row.manual_argv().map(|argv| argv.iter().map(|arg| arg.to_string_lossy()).collect::<Vec<_>>())})).collect::<Vec<_>>());
                report["hooks"][id] = status.projection;
            }
            SetupStatus::Available {
                installed,
                enabled,
                diagnostic,
            } => {
                local["installed"] = json!(installed);
                local["enabled"] = json!(enabled);
                local["diagnostics"] =
                    json!([{"text": diagnostic.chars().take(1024).collect::<String>()}]);
            }
            SetupStatus::Detailed(_) => {
                // A rejected capture cannot project hooks or authorize selected-scope repair.
                projection = None;
                local["diagnostics"] =
                    json!([{"text": "adapter returned status for a different local scope"}]);
            }
            SetupStatus::Failed(error) => {
                local["diagnostics"] =
                    json!([{"text": error.to_string().chars().take(1024).collect::<String>()}])
            }
            SetupStatus::Unsupported(error) => {
                local["diagnostics"] = json!([{"text": error.to_string()}])
            }
            SetupStatus::Unavailable { diagnostic } => {
                local["diagnostics"] =
                    json!([{"text": diagnostic.chars().take(1024).collect::<String>()}])
            }
        }
        if let Some(projection) = projection {
            report["hooks"][id] = projection.hooks;
            local["manual_repairs"] = json!(
                projection
                    .manual_repairs
                    .into_iter()
                    .take(16)
                    .collect::<Vec<_>>()
            );
            limitations.extend(
                projection
                    .limitations
                    .into_iter()
                    .take(16)
                    .map(|line| json!(line.chars().take(1024).collect::<String>())),
            );
            // RemoveOwned is never a doctor's automatic repair. Even declared installation
            // repairs remain gated by context safety and setup ownership at execution.
            if projection.safe_repairs.iter().any(|repair| {
                matches!(repair, LocalRepair::InstallOwned | LocalRepair::RepairOwned)
            }) {
                repairs.push(OwnedRepair {
                    registration,
                    scope: resolved,
                    options: projection.repair_options,
                });
            }
        } else if report["hooks"][id].is_null() {
            report["hooks"][id] = json!({"local": local});
        }
        report["local_harnesses"][id] = local;
    }
    report["limitations"] = json!(limitations);
    if !limitations.is_empty() && report["result"] == "ok" {
        report["result"] = json!("degraded");
    }
    repairs
}
fn writes_safe(report: &Value) -> bool {
    report["context"]["ok"] == true
        && report["state_dir"]["safe"] == true
        && report["context"]["source"]["state_dir_leftover"].is_null()
        && report["daemon"]["version_matches"] != false
        && matches!(
            report["daemon"]["state"].as_str(),
            Some("not_running" | "healthy" | "degraded")
        )
}
fn repair_outcome(name: &str, outcome: Result<String, String>) -> Value {
    match outcome {
        Ok(detail) => json!({"action": name, "outcome": "attempted", "detail": detail}),
        Err(detail) => json!({"action": name, "outcome": "failed", "detail": detail}),
    }
}
fn add_global_manual_repairs(report: &Value, repairs: &mut Vec<Value>) {
    if report["context"]["ok"] == false {
        repairs.push(json!({"action": "context", "outcome": "refused", "detail": "invalid context; specify a valid state directory and host endpoint"}));
    }
    if report["state_dir"]["safe"] == false {
        repairs.push(json!({"action": "state directory", "outcome": "refused", "detail": "unsafe state directory; inspect and repair manually"}));
    }
    if report["context"]["source"]["state_dir_leftover"].is_string() {
        repairs.push(json!({"action": "plugin state", "outcome": "refused", "detail": "leftover state after uninstall; inspect before setup or daemon ensure"}));
    }
    if report["daemon"]["version_matches"] == false {
        repairs.push(json!({"action": "daemon version", "outcome": "refused", "detail": "version mismatch; inspect and repair manually"}));
    } else if report["daemon"]["state"] == "unreachable" {
        repairs.push(json!({"action": "daemon", "outcome": "manual", "detail": "endpoint is unreachable; inspect daemon state with doctor --debug"}));
    }
}

fn report_base(
    state_dir: Option<PathBuf>,
    host_endpoint: Option<PathBuf>,
    clock: &Arc<dyn Clock>,
    budget: &CallBudget,
    registry: &crate::harness::registry::Registry,
) -> (Value, i32) {
    let mut report = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": PROTOCOL_VERSION,
    });
    let inputs = super::instance::InstanceInputs::from_process(state_dir, host_endpoint);
    let (context, source) = match super::instance::resolve_context(&inputs) {
        Ok(resolved) => resolved,
        Err(error) => {
            report["context"] = json!({"ok": false, "error": error.to_string()});
            report["result"] = json!("invalid_context");
            return (report, exit::EXIT_USAGE);
        }
    };
    let host_present = std::fs::symlink_metadata(&context.host_endpoint).is_ok();
    report["context"] = json!({
        "ok": true,
        "state_dir": context.state_dir.display().to_string(),
        "host_endpoint": context.host_endpoint.display().to_string(),
        "host_endpoint_present": host_present,
        "source": source,
    });
    let mut state_error = match std::fs::symlink_metadata(&context.state_dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        _ => check_owned_state_root(&context.state_dir)
            .err()
            .map(|error| error.to_string()),
    };
    // Resolving may check the plugin-created runtime socket directory; an
    // unsafe one is the same invalid local context `daemon ensure` reports
    // (status 2), not an unavailable daemon.
    let resolved = InstancePaths::resolve_read_only(&context);
    if state_error.is_none() {
        state_error = match &resolved {
            Err(error) if is_unsafe_local_state(error) => Some(error.to_string()),
            Err(_) => None,
            Ok(paths) => unsafe_private_dir(paths),
        };
    }
    report["state_dir"] = json!({
        "exists": context.state_dir.exists(),
        "safe": state_error.is_none(),
        "error": state_error,
    });
    let mut code = exit::EXIT_OK;
    let mut result = "ok";
    if state_error.is_some() {
        code = exit::EXIT_USAGE;
        result = "unsafe_state_dir";
    }
    // The daemon's version verdicts, or why doctor has none.
    let mut rich = Err("the daemon is not reachable".to_owned());
    let mut states: Result<HarnessStatesReport, String> =
        Err("the daemon's instance could not be resolved".into());
    let mut daemon_limitations: Vec<String> = Vec::new();
    match resolved {
        Err(error) => {
            report["daemon"] = json!({"state": "unavailable", "error": error.to_string()});
            if state_error.is_none() {
                code = exit::EXIT_UNAVAILABLE;
                result = "unavailable";
            }
        }
        Ok(paths) => {
            report["instance_dir"] = json!(paths.instance_dir.display().to_string());
            report["daemon_log"] = json!(daemon_log_path(&paths).display().to_string());
            report["harness_manifest"] = harness_manifest_report(
                &paths.instance_dir,
                std::env::var_os("HERDR_THREADS_OFFLINE").as_deref(),
            );
            match probe_daemon(&paths, clock, budget) {
                Ok(Daemon::NotRunning) => {
                    states = Err("the daemon is not running".into());
                    report["daemon"] = if state_error.is_some() {
                        // Ensure refuses an unsafe state tree; do not suggest it.
                        json!({"state": "not_running"})
                    } else {
                        json!({
                            "state": "not_running",
                            "hint": crate::daemon::remedy::remedy(
                                None,
                                &crate::daemon::remedy::RemedyContext::Exit3,
                            ),
                        })
                    };
                    if code == exit::EXIT_OK {
                        code = exit::EXIT_UNAVAILABLE;
                        result = "unavailable";
                    }
                }
                Ok(Daemon::Unreachable(detail)) | Err(detail) => {
                    states = Err("the daemon is unreachable".into());
                    report["daemon"] = json!({"state": "unreachable", "error": detail});
                    if code == exit::EXIT_OK {
                        code = exit::EXIT_UNAVAILABLE;
                        result = "unavailable";
                    }
                }
                Ok(Daemon::Reachable(health, answered)) => {
                    states = answered.states;
                    rich = answered.rich;
                    daemon_limitations = health.limitations.clone();
                    let version_matches = health.software_version == env!("CARGO_PKG_VERSION");
                    report["daemon"] = json!({
                        "state": match health.state {
                            HealthState::Healthy => "healthy",
                            HealthState::Degraded => "degraded",
                            HealthState::Unavailable => "unavailable",
                        },
                        "software_version": health.software_version,
                        "version_matches": version_matches,
                        "boot_id": health.boot_id,
                        "host_reachability": health.host.reachability,
                        "harness_claude": harness_state(health.harness.claude),
                        "harness_codex": harness_state(health.harness.codex),
                        "settings": health.settings,
                        "limitations": health.limitations,
                        "notes": health.notes,
                    });
                    if code == exit::EXIT_OK {
                        if !version_matches {
                            code = exit::EXIT_UNAVAILABLE;
                            result = "daemon_version_mismatch";
                        } else if health.state == HealthState::Unavailable {
                            code = exit::EXIT_UNAVAILABLE;
                            result = "unavailable";
                        } else if health.state == HealthState::Degraded {
                            result = "degraded";
                        }
                    }
                }
            }
        }
    }
    let mut limitations = Vec::new();
    if let Some(leftover) = source["state_dir_leftover"].as_str() {
        limitations.push(format!("state directory: {leftover}"));
    }
    // A broken verdict is a limitation like it is in Health (a line Health
    // already shows is not repeated); working and new verdicts are not.
    if let Ok(states) = &states {
        for harness in &states.harnesses {
            let broken = |state: &str| state == "broken";
            let lines = harness
                .detected
                .iter()
                .filter(|detected| broken(&detected.state))
                .map(|detected| detected.line.clone())
                .chain(
                    harness
                        .versions
                        .iter()
                        .filter(|row| row.in_health_window && broken(&row.state))
                        .map(|row| row.line.clone()),
                );
            for line in lines {
                if !daemon_limitations.contains(&line) {
                    limitations.push(line);
                }
            }
        }
    }
    if !limitations.is_empty() && result == "ok" {
        result = "degraded";
    }
    report["limitations"] = json!(limitations);
    match &states {
        Ok(states) => {
            report["harness_states"] = serde_json::to_value(states).unwrap_or(Value::Null);
        }
        Err(why) => {
            report["harness_states"] = Value::Null;
            report["harness_states_unavailable"] = json!(why);
        }
    }
    match rich {
        Ok(rich) => {
            if rich.harnesses.len() == registry.registrations().len()
                && rich.harnesses.keys().all(|id| registry.agent(id).is_ok())
            {
                report["harness_health_v2"] = json!(rich);
            } else {
                report["harness_health_v2"] = Value::Null;
                report["harness_health_v2_unavailable"] = json!(
                    "advertised harness.health_v2 does not match the current registry harnesses"
                );
            }
        }
        Err(reason) => {
            report["harness_health_v2"] = Value::Null;
            report["harness_health_v2_unavailable"] = json!(reason);
        }
    }
    report["skill"] = json!({
        "summary_procedure": if crate::cli::skill::has_summary_procedure(crate::cli::skill::SKILL_MD) {
            "present"
        } else {
            "missing"
        },
    });
    report["result"] = json!(result);
    (report, code)
}

/// The `setup.event_registration` value: `legacy` for hooks installed before per-event
/// registration (still working, informational only), else `current`.
fn event_registration(legacy: bool) -> &'static str {
    if legacy { "legacy" } else { "current" }
}

/// The informational line for hooks that predate per-event registration. Never a limitation:
/// the hooks work, so doctor does not turn `degraded` over it.
fn legacy_registration_line(harness: &str, setup: &Value) -> Option<String> {
    (setup["event_registration"] == json!("legacy")).then(|| {
        let retrust = if harness == "codex" {
            "; Codex then asks to review (trust) the rewritten hooks again"
        } else {
            ""
        };
        format!(
            "{harness} hooks predate per-event registration (no --event): re-run \
             `herdr-threads setup {harness}`{retrust}\n"
        )
    })
}

fn clean(value: &str) -> String {
    escape_for_terminal(value, Context::SingleLine).into_owned()
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => clean(text),
        Value::Null => "none".into(),
        other => clean(&other.to_string()),
    }
}

fn registry_text(report: &Value) -> String {
    let mut out = String::new();
    for id in report["adapter_order"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let local = &report["local_harnesses"][id];
        if !local.is_null() {
            let axis = |name: &str| {
                local[name]
                    .as_bool()
                    .map_or("unknown".into(), |value| value.to_string())
            };
            out.push_str(&format!(
                "{} {} {}: installed: {}, enabled: {}, admitted: {}, observed: {}\n",
                clean(id),
                scalar(&local["scope"]["kind"]),
                local["scope"]["profile"]
                    .as_str()
                    .map(clean)
                    .unwrap_or_default(),
                axis("installed"),
                axis("enabled"),
                axis("admitted"),
                axis("observed")
            ));
            for row in local["diagnostics"].as_array().into_iter().flatten() {
                out.push_str(&format!("  {}\n", scalar(&row["text"])));
            }
        }
        let daemon = &report["harness_health_v2"]["harnesses"][id];
        if !daemon.is_null() {
            out.push_str(&format!(
                "{} {}: installation {}; enablement {}; callback {}\n",
                clean(id),
                scalar(&daemon["scope"]["kind"]),
                scalar(&daemon["installation"]["state"]),
                scalar(&daemon["enablement"]["state"]),
                scalar(&daemon["callback_observation"]["state"])
            ));
            for row in daemon["runtime_evidence"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "  {} {}: {}\n",
                    scalar(&row["scope"]["kind"]),
                    scalar(&row["state"]),
                    scalar(&row["line"])
                ));
            }
        }
    }
    if let Some(reason) = report["harness_health_v2_unavailable"].as_str() {
        out.push_str(&format!(
            "rich harness evidence unavailable: {}\n",
            clean(reason)
        ));
    }
    out
}

/// The prior, detailed line-oriented text form of the report.
pub fn render_debug_text(report: &Value) -> String {
    let mut out = String::from("herdr-threads doctor\n");
    out.push_str(&format!(
        "version: {} (protocol {})\n",
        scalar(&report["version"]),
        scalar(&report["protocol_version"])
    ));
    let context = &report["context"];
    if context["ok"] == json!(false) {
        out.push_str(&format!(
            "context: invalid: {}\n",
            scalar(&context["error"])
        ));
    } else {
        out.push_str(&format!("state_dir: {}\n", scalar(&context["state_dir"])));
        out.push_str(&format!(
            "state_dir.source: {}\n",
            scalar(&context["source"]["state_dir"])
        ));
        let state = &report["state_dir"];
        if state["exists"] == json!(false) {
            out.push_str("state_dir.status: absent (created 0700 by `daemon ensure`)\n");
        } else if state["safe"] == json!(true) {
            out.push_str("state_dir.status: ok\n");
        } else {
            out.push_str(&format!(
                "state_dir.status: unsafe: {}\n",
                scalar(&state["error"])
            ));
        }
        out.push_str(&format!(
            "host_endpoint: {} ({})\n",
            scalar(&context["host_endpoint"]),
            if context["host_endpoint_present"] == json!(true) {
                "present"
            } else {
                "missing"
            }
        ));
        out.push_str(&format!(
            "host_endpoint.source: {}\n",
            scalar(&context["source"]["host_endpoint"])
        ));
        if !report["instance_dir"].is_null() {
            out.push_str(&format!(
                "instance_dir: {}\n",
                scalar(&report["instance_dir"])
            ));
        }
        if !report["daemon_log"].is_null() {
            out.push_str(&format!("daemon_log: {}\n", scalar(&report["daemon_log"])));
        }
        out.push_str(&harness_states_text(report));
        if report["harness_manifest"].is_object() {
            out.push_str(&harness_manifest_text(&report["harness_manifest"]));
        }
        let daemon = &report["daemon"];
        out.push_str(&format!("daemon: {}\n", scalar(&daemon["state"])));
        for key in [
            "software_version",
            "version_matches",
            "boot_id",
            "host_reachability",
            "harness_claude",
            "harness_codex",
            "error",
            "hint",
        ] {
            if !daemon[key].is_null() {
                out.push_str(&format!("daemon.{key}: {}\n", scalar(&daemon[key])));
            }
        }
        for key in [
            "invitation_default_ms",
            "receipt_default_ms",
            "minimum_wake_delay_ms",
            "wake_batch_delay_ms",
        ] {
            if !daemon["settings"][key].is_null() {
                out.push_str(&format!(
                    "daemon.settings.{key}: {}\n",
                    scalar(&daemon["settings"][key])
                ));
            }
        }
        if let Some(limitations) = daemon["limitations"].as_array() {
            for limitation in limitations {
                out.push_str(&format!("daemon.limitation: {}\n", scalar(limitation)));
            }
        }
        if let Some(notes) = daemon["notes"].as_array() {
            for note in notes {
                out.push_str(&format!("daemon.note: {}\n", scalar(note)));
            }
        }
        out.push_str(&format!(
            "skill.summary_procedure: {}\n",
            scalar(&report["skill"]["summary_procedure"])
        ));
        let claude = &report["hooks"]["claude"];
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
        if let Some(line) = legacy_registration_line("claude", &claude["setup"]) {
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
        out.push_str(&format!(
            "hooks.codex: {}\n",
            scalar(&report["hooks"]["codex"]["detail"])
        ));
        out.push_str(&format!(
            "hooks.codex.recipes: {}\n",
            scalar(&report["hooks"]["codex"]["recipes"])
        ));
        let setup = &report["hooks"]["codex"]["setup"];
        out.push_str(&format!(
            "hooks.codex.hooks_file: {}\n",
            scalar(&setup["hooks_file"])
        ));
        out.push_str(&format!(
            "hooks.codex.setup_installed: {}\n",
            if setup["installed"] == json!(true) {
                "yes"
            } else {
                "no"
            }
        ));
        if let Some(line) = legacy_registration_line("codex", setup) {
            out.push_str(&line);
        }
        out.push_str(&format!(
            "hooks.codex.trust.status: {}\n",
            scalar(&report["hooks"]["codex"]["trust"]["status"])
        ));
        let installed = &report["hooks"]["codex"]["installed"];
        out.push_str(&format!(
            "hooks.codex.installed: {}\n",
            scalar(&installed["evidence"])
        ));
        if !installed["fingerprint_source"].is_null() {
            out.push_str(&format!(
                "codex fingerprint source: {}\n",
                scalar(&installed["fingerprint_source"])
            ));
        }
        if !installed["error"].is_null() {
            out.push_str(&format!(
                "hooks.codex.error: {}\n",
                scalar(&installed["error"])
            ));
        }
        let sandbox_warning = &report["hooks"]["codex"]["sandbox_warning"];
        if !sandbox_warning.is_null() {
            out.push_str(&format!(
                "hooks.codex.sandbox_warning: {}\n",
                scalar(sandbox_warning)
            ));
        }
        if !report["hooks"]["codex"]["socket_policy_validation"].is_null() {
            out.push_str(&format!(
                "hooks.codex.socket_policy_validation: {}\n",
                scalar(&report["hooks"]["codex"]["socket_policy_validation"])
            ));
        }
        let last_hook = &report["hooks"]["codex"]["last_hook"];
        if !last_hook.is_null() {
            out.push_str(&format!(
                "hooks.codex.last_hook: {}\n",
                scalar(&last_hook["evidence"])
            ));
        }
    }
    if let Some(limitations) = report["limitations"].as_array() {
        for limitation in limitations {
            out.push_str(&format!("limitation: {}\n", scalar(limitation)));
        }
    }
    out.push_str(&registry_text(report));
    out.push_str(&format!("result: {}\n", scalar(&report["result"])));
    out
}

/// Verdicts and actions first; detailed inventory is available with --debug.
pub fn render_text(report: &Value) -> String {
    let mut out = format!("doctor: {}\n", scalar(&report["result"]));
    let context = &report["context"];
    if context["ok"] == json!(false) {
        out.push_str(&format!("issue: {}\n", scalar(&context["error"])));
        return out;
    }
    let state = &report["state_dir"];
    if state["safe"] == json!(false) {
        out.push_str(&format!(
            "issue: unsafe state directory: {}\n",
            scalar(&state["error"])
        ));
    }
    let daemon = &report["daemon"];
    out.push_str(&format!("daemon: {}\n", scalar(&daemon["state"])));
    if let Some(error) = daemon["error"].as_str() {
        out.push_str(&format!("issue: {}\n", clean(error)));
    }
    if daemon["version_matches"] == json!(false) {
        out.push_str("issue: daemon version differs from this CLI; inspect with --debug\n");
    }
    for name in ["claude", "codex"] {
        let harness = &report["hooks"][name];
        if harness.is_null() {
            continue;
        }
        let admission = scalar(&harness["installed"]["admission"]);
        let setup = if harness["setup"]["installed"] == json!(true) {
            "hooks installed"
        } else {
            "hooks missing"
        };
        out.push_str(&format!("{name}: {admission}, {setup}\n"));
    }
    if report["hooks"]["codex"]["setup"]["installed"] == json!(true) {
        match report["hooks"]["codex"]["trust"]["status"].as_str() {
            Some("review_required") => out.push_str(
                "issue: Codex hook review required; start codex interactively and review herdr-threads hooks in /hooks\n",
            ),
            Some("recorded_unverified") => out.push_str(
                "Codex hook trust recorded (current hashes unverified)\n",
            ),
            _ => out.push_str(
                "Codex hook trust unknown; inspect /hooks in an interactive Codex session\n",
            ),
        }
    }
    if let Some(harnesses) = report["harness_states"]["harnesses"].as_array() {
        for harness in harnesses {
            let name = scalar(&harness["harness"]);
            let newest = harness["versions"].as_array().and_then(|rows| rows.first());
            let detected = &harness["detected"];
            let row = if detected.is_object()
                && newest.is_some_and(|row| row["version"] != detected["version"])
            {
                Some(detected)
            } else {
                newest.or_else(|| detected.is_object().then_some(detected))
            };
            if let Some(row) = row {
                out.push_str(&format!(
                    "{name} version {}: {}\n",
                    scalar(&row["version"]),
                    scalar(&row["state"])
                ));
            }
        }
    }
    if let Some(items) = report["daemon"]["limitations"].as_array() {
        for item in items {
            out.push_str(&format!("issue: {}\n", scalar(item)));
        }
    }
    if let Some(items) = report["limitations"].as_array() {
        for item in items {
            if item
                .as_str()
                .is_some_and(|line| line.starts_with("Codex hook review required"))
            {
                continue;
            }
            out.push_str(&format!("issue: {}\n", scalar(item)));
        }
    }
    if !repair_plan(report).is_empty() {
        out.push_str("fix: herdr-threads doctor fix\n");
    }
    out.push_str(&registry_text(report));
    out.push_str("details: herdr-threads doctor --debug\n");
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repair {
    EnsureDaemon,
    SetupClaude,
}

/// Only evidence that the existing owner-aware operations can act on belongs
/// in this plan. A malformed context or unsafe state tree blocks all writes.
fn repair_plan(report: &Value) -> Vec<Repair> {
    if report["context"]["ok"] != json!(true) || report["state_dir"]["safe"] != json!(true) {
        return Vec::new();
    }
    if report["context"]["source"]["state_dir_leftover"].is_string() {
        return Vec::new();
    }
    if report["daemon"]["version_matches"] == json!(false)
        || !matches!(
            report["daemon"]["state"].as_str(),
            Some("not_running" | "healthy" | "degraded")
        )
    {
        return Vec::new();
    }
    let mut repairs = Vec::new();
    if report["daemon"]["state"] == "not_running" {
        repairs.push(Repair::EnsureDaemon);
    }
    let claude = &report["hooks"]["claude"];
    if claude["setup"]["installed"] == json!(false)
        && claude["setup"]["error"].is_null()
        && claude["error"].is_null()
        && claude["installed"]["binary"].is_string()
        && matches!(
            claude["installed"]["admission"].as_str(),
            Some("listed" | "optimistic")
        )
    {
        repairs.push(Repair::SetupClaude);
    }
    repairs
}

fn add_manual_repairs(report: &Value, repairs: &mut Vec<Value>) {
    if report["context"]["ok"] == json!(false) {
        repairs.push(json!({"action": "context", "outcome": "refused", "detail": "invalid context; specify a valid state directory and host endpoint"}));
    }
    if report["state_dir"]["safe"] == json!(false) {
        repairs.push(json!({"action": "state directory", "outcome": "refused", "detail": "unsafe state directory; inspect and repair manually"}));
    }
    if report["context"]["source"]["state_dir_leftover"].is_string() {
        repairs.push(json!({"action": "plugin state", "outcome": "refused", "detail": "leftover state after uninstall; inspect before setup or daemon ensure"}));
    }
    if report["daemon"]["version_matches"] == json!(false) {
        repairs.push(json!({"action": "daemon version", "outcome": "refused", "detail": "version mismatch; inspect and repair manually"}));
    } else if report["daemon"]["state"] == "unreachable" {
        repairs.push(json!({"action": "daemon", "outcome": "manual", "detail": "endpoint is unreachable; inspect daemon state with doctor --debug"}));
    }
    match report["hooks"]["codex"]["trust"]["status"].as_str() {
        Some("review_required") => repairs.push(json!({"action": "Codex hook trust", "outcome": "manual", "detail": "start codex interactively and review herdr-threads hooks in /hooks"})),
        Some("unknown") => repairs.push(json!({"action": "Codex hook trust", "outcome": "unknown", "detail": "trust could not be determined; inspect /hooks interactively"})),
        _ => (),
    }
    let codex = &report["hooks"]["codex"];
    if codex["setup"]["installed"] == json!(false)
        && codex["installed"]["binary"].is_string()
        && matches!(
            codex["installed"]["admission"].as_str(),
            Some("listed" | "schema-matched, live-unverified" | "optimistic")
        )
    {
        if !codex["setup"]["error"].is_null() {
            repairs.push(json!({"action": "setup codex", "outcome": "refused", "detail": "hook ownership could not be inspected; inspect doctor --debug before setup"}));
        } else if report["context"]["ok"] == json!(true)
            && report["state_dir"]["safe"] == json!(true)
            && report["context"]["source"]["state_dir_leftover"].is_null()
        {
            repairs.push(json!({
                "action": "setup codex",
                "outcome": "manual",
                "detail": "global socket policy is unvalidated for the effective Codex configuration; run `herdr-threads setup codex` explicitly after reviewing its sandbox changes"
            }));
        }
    }
    if codex["setup"]["installed"] == json!(true)
        && codex["installed"]["version"]
            .as_str()
            .is_some_and(|version| {
                !super::setup::CODEX_SANDBOX_MEASURED_VERSIONS.contains(&version)
            })
    {
        repairs.push(json!({"action": "Codex socket policy", "outcome": "manual", "detail": "legacy in-sandbox socket policy unvalidated; managed launch uses approved outside-sandbox CLI execution instead; global auto repair withheld"}));
    }
    if !report["hooks"]["claude"]["error"].is_null() {
        repairs.push(json!({"action": "setup claude", "outcome": "refused", "detail": "hook ownership could not be inspected; inspect doctor --debug before setup"}));
    }
    for name in ["claude", "codex"] {
        if report["hooks"][name]["installed"]["admission"] == "refused" {
            repairs.push(json!({"action": format!("{name} version"), "outcome": "manual", "detail": "installed harness version was refused; inspect doctor --debug and install an admitted version"}));
        }
    }
    if repairs.is_empty() {
        repairs.push(
            json!({"action": "doctor fix", "outcome": "none", "detail": "no safe repair needed"}),
        );
    }
}

fn apply_owned_repairs(
    report: &Value,
    owned: Vec<OwnedRepair>,
    environment: &crate::harness::adapter::SetupEnvironment,
) -> Vec<Value> {
    let mut repairs = Vec::new();
    let budget = CallBudget {
        deadline: MonoInstant(environment.clock.monotonic_now().0.saturating_add(30_000)),
        cancellation: Default::default(),
    };
    if writes_safe(report) {
        for repair in owned {
            let outcome = (|| -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
                let requested_scope = match &repair.scope {
                    crate::harness::adapter::ResolvedSetupScope::Profile {name, ..} => crate::harness::adapter::SetupScopeRequest::Profile(name.clone()),
                    _ => Default::default(),
                };
                crate::harness::setup::validate_local_request(Some(repair.registration), true, &requested_scope, &repair.options)?;
                if !environment.executable.is_absolute() { return Err(crate::harness::adapter::SetupFailure::Invalid("owned executable must be an absolute path".into())); }
                repair.registration.setup(
                &crate::harness::adapter::SetupRequest {
                    scope: repair.scope,
                    executable: environment.executable.clone(),
                    environment: environment.clone(),
                    native_binary: None,
                    options: repair.options,
                },
                &budget,
            )
            })();
            repairs.push(repair_outcome(
                &format!("setup {}", repair.registration.metadata().id),
                outcome
                    .map(|outcome| {
                        outcome.projection["action"]
                            .as_str()
                            .unwrap_or("attempted")
                            .to_owned()
                    })
                    .map_err(|error| error.to_string()),
            ));
        }
    }
    repairs
}

pub(crate) fn run<W: Write>(parsed: &ParsedCli, writer: &mut W) -> Result<(), RunError> {
    run_registered(parsed, crate::harness::registry::builtins(), None, writer)
}

pub(crate) fn run_registered<W: Write>(
    parsed: &ParsedCli,
    registry: &crate::harness::registry::Registry,
    supplied_environment: Option<&crate::harness::adapter::SetupEnvironment>,
    writer: &mut W,
) -> Result<(), RunError> {
    let (debug, fix, harness, scope) = match &parsed.action {
        super::commands::CliAction::Doctor {
            debug,
            fix,
            harness,
            scope,
        } => (*debug, *fix, harness.as_deref(), scope),
        _ => unreachable!("doctor run is only called for doctor"),
    };
    let clock: Arc<dyn Clock> = supplied_environment
        .map(|environment| Arc::clone(&environment.clock))
        .unwrap_or_else(|| Arc::new(SystemClock::new()));
    let mut budget = observation_budget(&clock);
    let (mut report, mut code) = report_base(
        parsed.output.context.state_dir.as_ref().map(PathBuf::from),
        parsed.output.context.host.as_ref().map(PathBuf::from),
        &clock,
        &budget,
        registry,
    );
    let mut environment = doctor_environment(&report, registry, supplied_environment);
    environment.clock = Arc::clone(&clock);
    let owned =
        collect_local_with_budget(&mut report, registry, harness, scope, &environment, &budget);
    if fix {
        let mut repairs = Vec::new();
        if writes_safe(&report) && report["daemon"]["state"] == "not_running" {
            let outcome = (|| -> Result<(), RunError> {
                let context = crate::daemon::paths::RuntimeContext {
                    herdr_bin: super::hook::resolve_on_path("herdr", environment.path.as_deref()),
                    state_dir: environment
                        .state_dir
                        .clone()
                        .ok_or_else(|| io::Error::other("state directory unavailable"))?,
                    host_endpoint: environment
                        .host_endpoint
                        .clone()
                        .ok_or_else(|| io::Error::other("host endpoint unavailable"))?,
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(crate::daemon::lifecycle::ensure_running(
                    &context,
                    &environment.executable,
                    Arc::clone(&environment.clock),
                ))?;
                Ok(())
            })();
            repairs.push(repair_outcome(
                "daemon ensure",
                outcome
                    .map(|()| "attempted".into())
                    .map_err(|error| error.to_string()),
            ));
        }
        repairs.extend(apply_owned_repairs(&report, owned, &environment));
        budget = observation_budget(&clock);
        (report, code) = report_base(
            parsed.output.context.state_dir.as_ref().map(PathBuf::from),
            parsed.output.context.host.as_ref().map(PathBuf::from),
            &clock,
            &budget,
            registry,
        );
        collect_local_with_budget(&mut report, registry, harness, scope, &environment, &budget);
        add_global_manual_repairs(&report, &mut repairs);
        for id in report["adapter_order"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            repairs.extend(
                report["local_harnesses"][id]["manual_repairs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
        }
        if repairs.is_empty() {
            repairs.push(json!({"action": "doctor fix", "outcome": "none", "detail": "no safe repair needed"}));
        }
        if repairs.iter().any(|entry| entry["outcome"] == "failed") && code == exit::EXIT_OK {
            code = exit::EXIT_FAILED;
        }
        report["repairs"] = json!(repairs);
    }
    let bytes = match parsed.output.format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&json!({"doctor": report}))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => {
            let mut text = if debug {
                render_debug_text(&report)
            } else {
                render_text(&report)
            };
            if let Some(repairs) = report["repairs"].as_array() {
                for repair in repairs {
                    text.push_str(&format!(
                        "repair: {}: {}: {}\n",
                        scalar(&repair["action"]),
                        scalar(&repair["outcome"]),
                        scalar(&repair["detail"])
                    ));
                }
            }
            text.into_bytes()
        }
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    if code == exit::EXIT_OK {
        Ok(())
    } else {
        Err(RunError::Exit(code))
    }
}

#[cfg(test)]
#[path = "../../tests/cli/doctor_json.rs"]
mod doctor_json;

#[cfg(test)]
#[path = "../../tests/cli/doctor_labels.rs"]
mod doctor_labels;
