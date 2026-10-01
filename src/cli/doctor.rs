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
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, check_owned_state_root, check_private_dir, is_unsafe_local_state},
    },
    harness::{context::Harness, setup::NativeObservation},
    ports::LocalClient,
    protocol::{
        commands::Command,
        output::OutputFormat,
        results::{CommandResult, HarnessState, Health, HealthState},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
        wire::PROTOCOL_VERSION,
    },
};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};

const HEALTH_BUDGET_MS: u64 = 2_000;

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

fn observation(state: NativeObservation) -> &'static str {
    match state {
        NativeObservation::Unknown => "unknown",
        NativeObservation::Observed => "observed",
        NativeObservation::Unsupported => CLAUDE_NOT_OBSERVABLE,
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
        return Ok(Daemon::Unreachable(format!(
            "daemon protocol {} differs from executable protocol {PROTOCOL_VERSION}; run `daemon stop` with the older executable, then `daemon ensure`",
            descriptor.protocol_version
        )));
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
    let mut observed_claude = NativeObservation::Unknown;
    match resolved {
        Err(error) => {
            report["daemon"] = json!({"state": "unavailable", "error": error.to_string()});
            code = code.max(exit::EXIT_UNAVAILABLE);
            result = "unavailable";
        }
        Ok(paths) => {
            report["instance_dir"] = json!(paths.instance_dir.display().to_string());
            match probe_daemon(&paths) {
                Ok(Daemon::NotRunning) => {
                    report["daemon"] = if state_error.is_some() {
                        // Ensure refuses an unsafe state tree; do not suggest it.
                        json!({"state": "not_running"})
                    } else {
                        json!({
                            "state": "not_running",
                            "hint": "run `herdr-threads daemon ensure`",
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
                    observed_claude = match health.harness.claude {
                        HarnessState::Supported => NativeObservation::Observed,
                        HarnessState::Cooperative | HarnessState::Unsupported => {
                            NativeObservation::Unsupported
                        }
                        HarnessState::Unknown => NativeObservation::Unknown,
                    };
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
        "observed": observation(observed_claude),
    });
    // Problems found in this environment (the one the harnesses run in):
    // an installed harness whose hooks are missing, a refused codex, or a
    // Codex sandbox warning. Each makes the result `degraded`.
    let mut limitations: Vec<String> = Vec::new();
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
            claude["settings"] = json!(settings.display().to_string());
            claude["installed"] = json!(inspection.installed);
            if let Some(adoption) = &inspection.adopted {
                claude["adopted"] = json!({"owner": adoption.owner, "recorded": adoption.recorded});
            }
            claude["allow_rule"] = super::setup::allow_rule_json(inspection.allow_rule.as_ref());
        }
        Err(error) => {
            if claude_binary.is_some() {
                limitations.push(format!("claude hooks cannot be inspected: {error}"));
            }
            claude["installed"] = json!(false);
            claude["error"] = json!(error);
        }
    }
    let codex_inspection = super::setup::user_inspection(Harness::Codex, &env);
    let codex_setup = match &codex_inspection {
        Ok((file, inspection)) => json!({
            "hooks_file": file.display().to_string(),
            "installed": inspection.installed,
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
    let mut installed = json!({
        "binary": codex.binary.as_ref().map(|path| path.display().to_string()),
        "admission": codex.state(),
        "evidence": codex.line(),
    });
    let last_hook = private.as_deref().and_then(|private| {
        crate::harness::codex_evidence::read(&crate::harness::codex_evidence::admission_path(
            private,
        ))
    });
    match &codex.result {
        Ok(version) => {
            installed["version"] = json!(version.as_str());
            installed["recipe"] = json!(version.recipe().id);
        }
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

fn clean(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
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
            scalar(&claude["settings"])
        ));
        out.push_str(&format!(
            "hooks.claude.installed: {}\n",
            if claude["installed"] == json!(true) {
                "yes"
            } else {
                "no"
            }
        ));
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
        let installed = &report["hooks"]["codex"]["installed"];
        out.push_str(&format!(
            "hooks.codex.installed: {}\n",
            scalar(&installed["evidence"])
        ));
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
