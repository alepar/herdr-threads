//! `doctor`: bounded, read-only local diagnostics. It never starts the daemon
//! and never writes harness configuration. It reports the resolved context,
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
        results::{CommandResult, ErrorCode, HarnessState, Health, HealthState},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
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
fn claude_installed_with_warning(
    path: Option<&std::ffi::OsStr>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> (InstalledHarnessJson, Option<String>) {
    use crate::harness::{admission, claude};
    let Some(binary) = super::hook::resolve_on_path("claude", path) else {
        return (claude_installed_stub(), None);
    };
    let version =
        crate::harness::codex::version_output(&binary, crate::harness::codex::VERSION_TIMEOUT)
            .ok()
            .and_then(|stdout| claude::version_from_output(&stdout));
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
/// one (refused or not installed) prints the daemon's own reason.
pub fn claude_observed_text(
    state: HarnessState,
    limitations: &[String],
    notes: &[String],
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
                .unwrap_or("the daemon did not admit claude");
            format!("not observable: {reason}")
        }
    }
}

enum Daemon {
    NotRunning,
    Unreachable(String),
    Reachable(Box<Health>),
}

fn probe_daemon(paths: &InstancePaths) -> Result<Daemon, String> {
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
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let budget = CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(HEALTH_BUDGET_MS)),
        cancellation: Cancellation::default(),
    };
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        instance,
        Some(descriptor.boot_id),
    );
    match client.call(Command::Health, &budget) {
        Ok(CommandResult::Health(health)) => Ok(Daemon::Reachable(Box::new(health))),
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
        ManifestPolicy, OffReason, SUPPORTED_SCHEMA_VERSION, cache_dir, format_rfc3339_utc,
        policy_from, read_meta,
    };
    let meta = read_meta(&cache_dir(instance_dir));
    let cache_fetched_at = meta.fetched_at_ms.map(format_rfc3339_utc);
    let (policy, source, settings_error) = match crate::daemon::settings::load(instance_dir) {
        Ok(settings) => {
            let (policy, source) = match policy_from(&settings, offline_env) {
                ManifestPolicy::Auto => ("auto", "default"),
                ManifestPolicy::Off(OffReason::Settings) => ("off", "settings"),
                ManifestPolicy::Off(OffReason::OfflineEnv) => ("off", "offline_env"),
            };
            (json!(policy), json!(source), Value::Null)
        }
        Err(error) => (Value::Null, Value::Null, json!(error.to_string())),
    };
    json!({
        "policy": policy,
        "source": source,
        "settings_error": settings_error,
        "cache_fetched_at": cache_fetched_at,
        "cache_etag": meta.etag,
        "embedded_schema_version": SUPPORTED_SCHEMA_VERSION,
    })
}

/// The text lines for [`harness_manifest_report`].
fn harness_manifest_text(manifest: &Value) -> String {
    let mut out = String::new();
    if let Some(error) = manifest["settings_error"].as_str() {
        out.push_str(&format!("harness manifest: settings error: {error}\n"));
    } else {
        match (manifest["policy"].as_str(), manifest["source"].as_str()) {
            (Some("off"), Some("settings")) => {
                out.push_str("harness manifest: off (settings.json)\n")
            }
            (Some("off"), _) => out.push_str(
                "harness manifest: off (HERDR_THREADS_OFFLINE=1 in this environment; the daemon reads its own environment at start)\n",
            ),
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

pub fn report(state_dir: Option<PathBuf>, host_endpoint: Option<PathBuf>) -> (Value, i32) {
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
    let resolved = InstancePaths::resolve(&context);
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
    let mut observed_claude = String::from("unknown");
    match resolved {
        Err(error) => {
            report["daemon"] = json!({"state": "unavailable", "error": error.to_string()});
            code = code.max(exit::EXIT_UNAVAILABLE);
            result = "unavailable";
        }
        Ok(paths) => {
            report["instance_dir"] = json!(paths.instance_dir.display().to_string());
            report["daemon_log"] = json!(daemon_log_path(&paths).display().to_string());
            report["harness_manifest"] = harness_manifest_report(
                &paths.instance_dir,
                std::env::var_os("HERDR_THREADS_OFFLINE").as_deref(),
            );
            match probe_daemon(&paths) {
                Ok(Daemon::NotRunning) => {
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
                    report["daemon"] = json!({"state": "unreachable", "error": detail});
                    if code == exit::EXIT_OK {
                        code = exit::EXIT_UNAVAILABLE;
                        result = "unavailable";
                    }
                }
                Ok(Daemon::Reachable(health)) => {
                    let version_matches = health.software_version == env!("CARGO_PKG_VERSION");
                    observed_claude = claude_observed_text(
                        health.harness.claude,
                        &health.limitations,
                        &health.notes,
                    );
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
    let env = super::setup::SetupEnv::for_context(&context);
    let mut claude = json!({
        "scope": "user",
        "recipes": crate::harness::recipe::describe(crate::harness::claude::RECIPES),
        "observed": observed_claude,
    });
    // Problems found in this environment (the one the harnesses run in):
    // an installed harness whose hooks are missing, a refused codex, or a
    // Codex sandbox warning. Each makes the result `degraded`.
    let mut limitations: Vec<String> = Vec::new();
    if let Some(leftover) = source["state_dir_leftover"].as_str() {
        limitations.push(format!("state directory: {leftover}"));
    }
    let path = std::env::var_os("PATH");
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
        }
        Err(error) => {
            if claude_binary.is_some() {
                limitations.push(format!("claude hooks cannot be inspected: {error}"));
            }
            claude["setup"] = json!({"installed": false});
            claude["error"] = json!(error);
        }
    }
    let (claude_installed, claude_warning) =
        claude_installed_with_warning(path.as_deref(), |key| std::env::var_os(key));
    claude["installed"] = json!(claude_installed);
    claude["admission_warning"] = json!(claude_warning);
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
    // The `codex` a hook would resolve on this PATH, observed read-only and
    // bounded by the version-observation deadline. A listed version is
    // admitted exactly; an unlisted one only when its embedded hook schemas
    // hash-match a recipe (schema-matched, live-unverified).
    // The hook's private fingerprint cache is consulted read-only (a warm
    // cache answers without rescanning; doctor never writes it), and the
    // admission evidence the latest hook stored is shown beside it.
    let private = crate::harness::codex_evidence::existing(&context.state_dir).ok();
    let cache = private
        .as_deref()
        .map(crate::harness::codex_evidence::cache_path);
    let codex = match crate::harness::codex::resolve_on_path(path.as_deref()) {
        Some(binary) => crate::harness::codex::InstalledAdmission::observe_binary(
            binary,
            crate::harness::codex::VERSION_TIMEOUT,
            cache.as_deref().map_or(
                crate::harness::codex_schema::FingerprintCache::Memory,
                crate::harness::codex_schema::FingerprintCache::ReadOnly,
            ),
        ),
        None => crate::harness::codex::InstalledAdmission::observe_on_path(
            None,
            crate::harness::codex::VERSION_TIMEOUT,
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
    if !limitations.is_empty() && result == "ok" {
        result = "degraded";
    }
    report["limitations"] = json!(limitations);
    report["hooks"] = json!({
        "claude": claude,
        "codex": {
            "scope": "user",
            "detail": "Codex hooks are user-level ($CODEX_HOME/hooks.json, set up by `herdr-threads setup codex`); Codex runs them only once trusted",
            "setup": codex_setup,
            "recipes": crate::harness::recipe::describe(crate::harness::codex::RECIPES),
            "installed": installed,
            "sandbox_warning": sandbox_warning,
            "sandbox_roots_warning": roots_warning,
            "sandbox_proxy_warnings": proxy_warnings,
            "last_hook": last_hook.map(|record| json!({
                "binary": record.binary,
                "admission": record.admission,
                "evidence": record.evidence,
                "recorded_unix_ms": record.recorded_unix_ms,
            })),
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
        format!(
            "{harness} hooks predate per-event registration (no --event): re-run \
             `herdr-threads setup {harness}`\n"
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

/// Compact line-oriented text form of the report.
pub fn render_text(report: &Value) -> String {
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
        out.push_str(&format!(
            "hooks.claude.observed: {}\n",
            scalar(&claude["observed"])
        ));
        out.push_str(&format!(
            "hooks.claude.recipes: {}\n",
            scalar(&claude["recipes"])
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
    out.push_str(&format!("result: {}\n", scalar(&report["result"])));
    out
}

pub(crate) fn run<W: Write>(parsed: &ParsedCli, writer: &mut W) -> Result<(), RunError> {
    let (report, code) = report(
        parsed.output.context.state_dir.as_ref().map(PathBuf::from),
        parsed.output.context.host.as_ref().map(PathBuf::from),
    );
    let bytes = match parsed.output.format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&json!({"doctor": report}))
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => render_text(&report).into_bytes(),
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
