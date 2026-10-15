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
        ownership::{read_descriptor, read_node_uuid_hint},
        paths::{NodePaths, check_owned_state_root, check_private_dir, is_unsafe_local_state},
    },
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
    #[serde(rename = "contract_declared")]
    ContractDeclared,
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
    pub const ALL: [AdmissionState; 6] = [
        AdmissionState::ContractDeclared,
        AdmissionState::Listed,
        AdmissionState::SchemaMatched,
        AdmissionState::Optimistic,
        AdmissionState::Refused,
        AdmissionState::NotFound,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AdmissionState::ContractDeclared => "contract_declared",
            AdmissionState::Listed => "listed",
            AdmissionState::SchemaMatched => crate::harness::recipe::SCHEMA_MATCHED_LABEL,
            AdmissionState::Optimistic => crate::harness::recipe::OPTIMISTIC_LABEL,
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

/// Executable availability and the declared contract; no runtime observation. `executable` is
/// the name looked up on `path`; `recipe` is the declared recipe id reported when it is found.
pub(crate) fn installed_declared(
    executable: &str,
    recipe: &str,
    path: Option<&std::ffi::OsStr>,
) -> InstalledHarnessJson {
    let binary = super::hook::resolve_on_path(executable, path);
    let recipe = binary.as_ref().map(|_| recipe.to_owned());
    InstalledHarnessJson {
        admission: if binary.is_some() {
            AdmissionState::ContractDeclared
        } else {
            AdmissionState::NotFound
        },
        binary: binary.map(|binary| binary.display().to_string()),
        version: None,
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

pub(crate) enum Daemon {
    NotRunning,
    Unreachable(String),
    Reachable(Box<Health>, Box<DaemonDetails>),
}
pub(crate) struct DaemonDetails {
    pub(crate) states: Result<HarnessStatesReport, String>,
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

pub(crate) fn probe_daemon(
    paths: &NodePaths,
    clock: &Arc<dyn Clock>,
    budget: &CallBudget,
) -> Result<Daemon, String> {
    let instance = match read_node_uuid_hint(paths) {
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

/// The node directory, when it exists and fails the 0700 ownership checks
/// `daemon ensure` applies. An absent directory is fine: ensure creates it 0700.
/// Only the typed unsafe-state marker counts, exactly as `daemon ensure`
/// classifies it, so the two commands cannot disagree.
fn unsafe_private_dir(paths: &NodePaths) -> Option<String> {
    match check_private_dir(&paths.node_dir) {
        Ok(()) => None,
        Err(error) if is_unsafe_local_state(&error) => Some(error.to_string()),
        // Absent, or other I/O failures (what ensure reports as unavailable, 3).
        Err(_) => None,
    }
}

/// Assemble the report and its exit status. Split from rendering for tests.
/// The effective harness-manifest fetch policy and the cache state, for
/// `report["harness_manifest"]`. Informational only: never a limitation.
/// `offline_env` is this process's `HERDR_THREADS_OFFLINE`; the daemon reads
/// its own environment (and `settings.json`) once at start.
pub(crate) fn harness_manifest_report(
    node_dir: &std::path::Path,
    offline_env: Option<&std::ffi::OsStr>,
) -> Value {
    use crate::harness::manifest::{
        SUPPORTED_SCHEMA_VERSION, cache_dir, format_rfc3339_utc, policy_from, policy_words,
        read_meta,
    };
    let meta = read_meta(&cache_dir(node_dir));
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
            match crate::daemon::settings::load(node_dir) {
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
                "harness {name} advisory history: {} {} \u{2014} {}\n",
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

pub(crate) fn captured_legacy_status(
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
        Some("contract_declared" | "listed" | "optimistic" | "schema-matched, live-unverified") => {
            Some(true)
        }
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
            "runtime_metadata_unavailable",
            DiagnosticSeverity::Info,
            "Runtime metadata unavailable; registered input contract does not qualify optional native capabilities",
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

pub fn report(state_dir: Option<PathBuf>, host_endpoint: Option<PathBuf>) -> (Value, i32) {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let budget = observation_budget(&clock);
    let registry = crate::harness::registry::builtins();
    let ((mut report, code), captured) =
        report_base(state_dir, host_endpoint, &clock, &budget, registry);
    let mut environment = doctor_environment(&report, registry, None);
    environment.clock = clock;
    let environment = local_ownership_environment(&environment, captured.as_ref(), false);
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
pub(crate) fn observation_budget(clock: &Arc<dyn Clock>) -> CallBudget {
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
        let resolved = match registration.resolve_setup_scope_for(
            &SetupScopeResolutionRequest {
                operation: SetupScopeOperation::Status,
                selector: scope,
                native_binary: None,
                environment,
            },
            budget,
        ) {
            Ok(resolution) => resolution.scope,
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
            let safe: Vec<&'static str> = projection
                .safe_repairs
                .iter()
                .filter_map(|repair| match repair {
                    LocalRepair::InstallOwned => Some("install_owned"),
                    LocalRepair::RepairOwned => Some("repair_owned"),
                    _ => None,
                })
                .collect();
            if !safe.is_empty() {
                local["safe_repairs"] = json!(safe);
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
/// Doctor's word for the daemon's Health state. A daemon that reports itself
/// starting ([`crate::protocol::results::Health::starting`]) is `starting`,
/// not `degraded`, until its first reconciliation or the startup bound.
pub(crate) fn daemon_state_label(health: &crate::protocol::results::Health) -> &'static str {
    match health.state {
        HealthState::Healthy => "healthy",
        HealthState::Degraded if health.starting() => "starting",
        HealthState::Degraded => "degraded",
        HealthState::Unavailable => "unavailable",
    }
}

/// Whether Health alone makes doctor's result `degraded`: a degraded daemon
/// that is merely starting does not.
pub(crate) fn health_degrades_result(health: &crate::protocol::results::Health) -> bool {
    health.state == HealthState::Degraded && !health.starting()
}

/// Doctor's one text line for a starting daemon, in place of an `issue:`
/// per not-yet-observed Health line (those print as `pending:`).
pub(crate) const DAEMON_STARTING_TEXT: &str = "note: daemon starting; first reconciliation pending";

fn writes_safe(report: &Value) -> bool {
    report["context"]["ok"] == true
        && report["state_dir"]["safe"] == true
        && report["context"]["source"]["state_dir_leftover"].is_null()
        && report["daemon"]["version_matches"] != false
        && matches!(
            report["daemon"]["state"].as_str(),
            Some("not_running" | "healthy" | "starting" | "degraded")
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

struct CapturedDoctorContext {
    context: crate::daemon::paths::RuntimeContext,
    source: Value,
    selected_state: PathBuf,
    selected_host: PathBuf,
}

fn local_ownership_environment(
    environment: &crate::harness::adapter::SetupEnvironment,
    captured: Option<&CapturedDoctorContext>,
    supplied: bool,
) -> crate::harness::adapter::SetupEnvironment {
    let mut local = environment.clone();
    if !supplied && let Some(captured) = captured {
        local.state_dir = Some(captured.selected_state.clone());
        local.host_endpoint = Some(captured.selected_host.clone());
    }
    local
}

fn report_base(
    state_dir: Option<PathBuf>,
    host_endpoint: Option<PathBuf>,
    clock: &Arc<dyn Clock>,
    budget: &CallBudget,
    registry: &crate::harness::registry::Registry,
) -> ((Value, i32), Option<CapturedDoctorContext>) {
    let inputs = super::instance::InstanceInputs::from_process(state_dir, host_endpoint);
    let (context, source, selected_state, selected_host) =
        match super::instance::resolve_context_with_selected_paths(&inputs) {
            Ok(resolved) => resolved,
            Err(error) => {
                let report = json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "protocol_version": PROTOCOL_VERSION,
                    "context": {"ok": false, "error": error.to_string()},
                    "result": "invalid_context",
                });
                return ((report, exit::EXIT_USAGE), None);
            }
        };
    let captured = CapturedDoctorContext {
        context,
        source,
        selected_state,
        selected_host,
    };
    let result = report_captured(&captured, clock, budget, registry);
    (result, Some(captured))
}

fn report_captured(
    captured: &CapturedDoctorContext,
    clock: &Arc<dyn Clock>,
    budget: &CallBudget,
    registry: &crate::harness::registry::Registry,
) -> (Value, i32) {
    let context = &captured.context;
    let source = &captured.source;
    let mut report = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": PROTOCOL_VERSION,
    });
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
    let resolved = NodePaths::resolve_read_only(&context.state_dir);
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
            report["node_dir"] = json!(paths.node_dir.display().to_string());
            report["daemon_log"] = json!(daemon_log_path(&paths).display().to_string());
            report["harness_manifest"] = harness_manifest_report(
                &paths.node_dir,
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
                        "state": daemon_state_label(&health),
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
                        } else if health_degrades_result(&health) {
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
            let local = harness
                .versions
                .iter()
                .filter(|row| {
                    row.in_health_window && row.source.starts_with("local evidence: violation")
                })
                .map(|row| {
                    format!(
                        "harness {} {} contract input failure: {}",
                        harness.harness, row.version, row.source
                    )
                });
            let scoped = harness
                .unattributed
                .iter()
                .filter(|row| {
                    row.reason.contains("contract input failure")
                        || row.reason.contains("contract input malformed")
                })
                .map(|row| row.reason.clone());
            for line in local.chain(scoped) {
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

/// The `setup.event_registration` value: `legacy` for shared or UUID-marked native commands
/// (still working, informational only), else `current`.
pub(crate) fn event_registration(legacy: bool) -> &'static str {
    if legacy { "legacy" } else { "current" }
}

/// The informational line for legacy native commands. Never a limitation:
/// the hooks work, so doctor does not turn `degraded` over it.
pub(crate) fn legacy_registration_line(
    harness: &str,
    setup: &Value,
    retrust_note: &str,
) -> Option<String> {
    (setup["event_registration"] == json!("legacy")).then(|| {
        format!(
            "{harness} hooks use legacy commands: re-run \
             `herdr-threads setup {harness}` to migrate to stable hook-event commands{retrust_note}\n"
        )
    })
}

/// The summary lines both legacy hook adapters print for their `report.hooks.<id>` object:
/// the admission and hook-installation verdict, then the runtime-metadata note.
pub(crate) fn legacy_summary_lines(id: &str, hooks: &Value) -> String {
    let admission = scalar(&hooks["installed"]["admission"]);
    let setup = if hooks["setup"]["installed"] == json!(true) {
        "hooks installed"
    } else {
        "hooks missing"
    };
    let mut out = format!("{id}: {admission}, {setup}\n");
    if hooks["installed"]["binary"].is_string() && hooks["installed"]["version"].is_null() {
        out.push_str(&format!(
            "{id}: runtime metadata unavailable; rich optional capabilities unavailable\n"
        ));
    }
    out
}

pub(crate) fn clean(value: &str) -> String {
    escape_for_terminal(value, Context::SingleLine).into_owned()
}

/// `hooks.<harness>.permissions: <state>`, with the note when there is one.
pub(crate) fn permission_line(harness: &str, permissions: &Value) -> String {
    let mut line = format!(
        "hooks.{harness}.permissions: {}\n",
        scalar(&permissions["state"])
    );
    if permissions["note"].is_string() {
        line.push_str(&format!(
            "hooks.{harness}.permissions.note: {}\n",
            scalar(&permissions["note"])
        ));
    }
    line
}

pub(crate) fn scalar(value: &Value) -> String {
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
            if let Some(detail) = daemon["admission"]["detail"].as_str() {
                out.push_str(&format!(
                    "  admission {}: {}\n",
                    scalar(&daemon["admission"]["state"]),
                    clean(detail)
                ));
            }
            for detail in daemon["limitations"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                out.push_str(&format!("  {}\n", clean(detail)));
            }
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

/// The registrations whose text the report renders, in `adapter_order` (the registry order
/// when the report names none).
fn text_adapters<'r>(
    report: &Value,
    registry: &'r crate::harness::registry::Registry,
) -> Vec<&'r crate::harness::registry::Registration> {
    match report["adapter_order"].as_array() {
        Some(order) => order
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|id| {
                registry
                    .registrations()
                    .iter()
                    .find(|registration| registration.metadata().id == id)
            })
            .collect(),
        None => registry.registrations().iter().collect(),
    }
}

/// The prior, detailed line-oriented text form of the report.
pub fn render_debug_text(report: &Value) -> String {
    render_debug_text_in(report, crate::harness::registry::builtins())
}

/// [`render_debug_text`] with each adapter's `hooks.<id>` text drawn from `registry`.
pub(crate) fn render_debug_text_in(
    report: &Value,
    registry: &crate::harness::registry::Registry,
) -> String {
    use crate::harness::adapter::DoctorTextForm;
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
        if !report["node_dir"].is_null() {
            out.push_str(&format!("node_dir: {}\n", scalar(&report["node_dir"])));
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
        ] {
            if !daemon[key].is_null() {
                out.push_str(&format!("daemon.{key}: {}\n", scalar(&daemon[key])));
            }
        }
        // Per-harness daemon states (`harness_<id>`), in sorted key order.
        let mut harness_keys: Vec<&String> = daemon
            .as_object()
            .into_iter()
            .flat_map(|object| object.keys())
            .filter(|key| key.starts_with("harness_"))
            .collect();
        harness_keys.sort();
        for key in harness_keys {
            if !daemon[key.as_str()].is_null() {
                out.push_str(&format!(
                    "daemon.{key}: {}\n",
                    scalar(&daemon[key.as_str()])
                ));
            }
        }
        for key in ["error", "hint"] {
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
        for registration in text_adapters(report, registry) {
            out.push_str(&registration.doctor_text(
                &report["hooks"][registration.metadata().id],
                DoctorTextForm::Debug,
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
    render_text_in(report, crate::harness::registry::builtins())
}

/// [`render_text`] with each adapter's summary text drawn from `registry`.
pub(crate) fn render_text_in(
    report: &Value,
    registry: &crate::harness::registry::Registry,
) -> String {
    use crate::harness::adapter::DoctorTextForm;
    let adapters = text_adapters(report, registry);
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
    for registration in &adapters {
        let hooks = &report["hooks"][registration.metadata().id];
        if hooks.is_null() {
            continue;
        }
        out.push_str(&registration.doctor_text(hooks, DoctorTextForm::Summary));
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
                    "{name} advisory history {}: {}\n",
                    scalar(&row["version"]),
                    scalar(&row["state"])
                ));
            }
        }
    }
    // While the daemon is starting, lines that only say something has not
    // been observed yet are `pending:`, not `issue:`; real issues stay.
    let starting = report["daemon"]["state"] == "starting";
    if starting {
        out.push_str(DAEMON_STARTING_TEXT);
        out.push('\n');
    }
    if let Some(items) = report["daemon"]["limitations"].as_array() {
        for item in items {
            let label = if starting
                && item
                    .as_str()
                    .is_some_and(crate::daemon::health::is_pending_observation_line)
            {
                "pending"
            } else {
                "issue"
            };
            out.push_str(&format!("{label}: {}\n", scalar(item)));
        }
    }
    if let Some(items) = report["limitations"].as_array() {
        for item in items {
            // A limitation an adapter's summary text already printed is not repeated.
            if item.as_str().is_some_and(|line| {
                adapters.iter().any(|registration| {
                    registration
                        .doctor_summary_claims()
                        .iter()
                        .any(|claim| line.starts_with(claim))
                })
            }) {
                continue;
            }
            out.push_str(&format!("issue: {}\n", scalar(item)));
        }
    }
    if !repair_plan(report).is_empty() || local_safe_repairs_offered(report) {
        out.push_str("fix: herdr-threads doctor fix\n");
    }
    out.push_str(&registry_text(report));
    out.push_str("details: herdr-threads doctor --debug\n");
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repair {
    EnsureDaemon,
}

/// Whether the report's context permits any automatic repair: a malformed context, an unsafe
/// state tree, leftover plugin state, a daemon version mismatch or an unanswerable daemon block
/// all forbid writes. Adapter projections gate their own safe repairs on this.
pub(crate) fn repairs_allowed(report: &Value) -> bool {
    report["context"]["ok"] == json!(true)
        && report["state_dir"]["safe"] == json!(true)
        && !report["context"]["source"]["state_dir_leftover"].is_string()
        && report["daemon"]["version_matches"] != json!(false)
        && matches!(
            report["daemon"]["state"].as_str(),
            Some("not_running" | "healthy" | "starting" | "degraded")
        )
}

/// The report-wide automatic repairs. Adapter repairs come from each projection's safe repairs
/// (see [`local_safe_repairs_offered`]), never from this plan.
fn repair_plan(report: &Value) -> Vec<Repair> {
    if !repairs_allowed(report) {
        return Vec::new();
    }
    let mut repairs = Vec::new();
    if report["daemon"]["state"] == "not_running" {
        repairs.push(Repair::EnsureDaemon);
    }
    repairs
}

/// Whether any collected local harness carries a safe repair `doctor fix` would apply.
fn local_safe_repairs_offered(report: &Value) -> bool {
    repairs_allowed(report)
        && report["local_harnesses"]
            .as_object()
            .into_iter()
            .flat_map(|locals| locals.values())
            .any(|local| {
                local["safe_repairs"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            })
}

/// `safe_repairs` is an internal hand-off from collection to the summary text; it is not part
/// of the `--json` document.
fn without_transient_fields(report: &Value) -> Value {
    let mut report = report.clone();
    if let Some(locals) = report["local_harnesses"].as_object_mut() {
        for local in locals.values_mut() {
            if let Some(local) = local.as_object_mut() {
                local.remove("safe_repairs");
            }
        }
    }
    report
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
    let _backups = crate::harness::setup::BackupSession::start();
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
    let ((mut report, mut code), captured) = report_base(
        parsed.output.context.state_dir.as_ref().map(PathBuf::from),
        parsed.output.context.host.as_ref().map(PathBuf::from),
        &clock,
        &budget,
        registry,
    );
    let mut environment = doctor_environment(&report, registry, supplied_environment);
    environment.clock = Arc::clone(&clock);
    let local_environment = local_ownership_environment(
        &environment,
        captured.as_ref(),
        supplied_environment.is_some(),
    );
    let owned = collect_local_with_budget(
        &mut report,
        registry,
        harness,
        scope,
        &local_environment,
        &budget,
    );
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
        repairs.extend(apply_owned_repairs(&report, owned, &local_environment));
        budget = observation_budget(&clock);
        if let Some(captured) = &captured {
            (report, code) = report_captured(captured, &clock, &budget, registry);
        }
        collect_local_with_budget(
            &mut report,
            registry,
            harness,
            scope,
            &local_environment,
            &budget,
        );
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
            let mut bytes =
                serde_json::to_vec(&json!({"doctor": without_transient_fields(&report)}))
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => {
            let mut text = if debug {
                render_debug_text_in(&report, registry)
            } else {
                render_text_in(&report, registry)
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

#[cfg(test)]
mod selected_ownership_tests {
    use super::*;
    #[test]
    fn selected_paths_preserve_local_ownership_without_overriding_supplied_environment() {
        let captured = CapturedDoctorContext {
            context: crate::daemon::paths::RuntimeContext {
                state_dir: PathBuf::from("/canonical/state"),
                host_endpoint: PathBuf::from("/canonical/host"),
                herdr_bin: None,
            },
            source: json!({}),
            selected_state: PathBuf::from("/selected/state"),
            selected_host: PathBuf::from("/selected/host"),
        };
        let environment = crate::harness::adapter::SetupEnvironment {
            state_dir: Some(PathBuf::from("/supplied/state")),
            host_endpoint: Some(PathBuf::from("/supplied/host")),
            executable: PathBuf::from("/supplied/executable"),
            ..Default::default()
        };
        let local = local_ownership_environment(&environment, Some(&captured), false);
        assert_eq!(local.state_dir, Some(captured.selected_state.clone()));
        assert_eq!(local.host_endpoint, Some(captured.selected_host.clone()));
        assert_eq!(
            captured.context.host_endpoint,
            PathBuf::from("/canonical/host")
        );
        let supplied = local_ownership_environment(&environment, Some(&captured), true);
        assert_eq!(supplied.state_dir, environment.state_dir);
        assert_eq!(supplied.host_endpoint, environment.host_endpoint);
        assert_eq!(supplied.executable, environment.executable);
    }
}
