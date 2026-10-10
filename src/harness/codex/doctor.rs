//! Codex's doctor surfaces: the `hooks.codex` projection, its debug and summary text, and its
//! manual-repair rules. `cli::doctor` keeps only the harness-neutral report assembly and
//! renders this module through the adapter seam.
use crate::{
    cli::doctor::{
        captured_legacy_status, event_registration, installed_declared, legacy_registration_line,
        legacy_summary_lines, permission_line, scalar,
    },
    harness::{adapter::DoctorTextForm, context::Harness},
    protocol::time::CallBudget,
};
use serde_json::{Value, json};

#[cfg(test)]
pub(crate) fn codex_installed_json(
    codex: &crate::harness::codex::InstalledAdmission,
) -> crate::cli::doctor::InstalledHarnessJson {
    use crate::{
        cli::doctor::{AdmissionState, InstalledHarnessJson},
        harness::codex::{Admission, InstalledRefusal},
    };
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

/// Manual repair rows Codex contributes: hook trust, and installing hooks explicitly. `report`
/// is the report so far (its write-safety context), `codex` the projected `hooks.codex` object.
pub(crate) fn manual_repairs(report: &Value, codex: &Value) -> Vec<Value> {
    let mut repairs = Vec::new();
    match codex["trust"]["status"].as_str() {
        Some("review_required") => repairs.push(json!({"action": "Codex hook trust", "outcome": "manual", "detail": "start codex interactively and review herdr-threads hooks in /hooks"})),
        Some("unknown") => repairs.push(json!({"action": "Codex hook trust", "outcome": "unknown", "detail": "trust could not be determined; inspect /hooks interactively"})),
        _ => (),
    }
    if codex["setup"]["installed"] == json!(false)
        && codex["installed"]["binary"].is_string()
        && matches!(
            codex["installed"]["admission"].as_str(),
            Some("contract_declared" | "listed" | "schema-matched, live-unverified" | "optimistic")
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
                "detail": "hooks are not installed; run `herdr-threads setup codex` explicitly"
            }));
        }
    }
    repairs
}

pub(super) fn legacy_codex_projection(
    request: &crate::harness::adapter::StatusRequest,
    daemon: &Value,
    _budget: &CallBudget,
) -> crate::harness::adapter::DoctorProjection {
    use crate::harness::adapter::*;
    let env = crate::cli::setup::SetupEnv::from_snapshot(&request.environment);
    let path = request.environment.path.clone();
    let mut limitations = Vec::new();
    let codex_inspection = crate::cli::setup::user_inspection(Harness::Codex, &env);
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
    let binary = crate::cli::hook::resolve_on_path("codex", path.as_deref());
    let installed = json!(installed_declared(
        "codex",
        super::RECIPES[0].id,
        path.as_deref()
    ));
    let last_hook: Option<crate::harness::codex_evidence::AdmissionRecord> = None;
    let sandbox_warning = super::setup::codex_unmeasured_allowance_warning(&env, None);
    let socket_policy_validation = binary.as_ref().map(|_| "not_run");
    if let Some(binary) = &binary {
        match &codex_inspection {
            Ok((file, inspection)) if !inspection.installed => limitations.push(format!(
                "codex is on PATH ({}) but its hooks are not installed in {}: run `herdr-threads setup codex` with this CODEX_HOME",
                binary.display(), file.display())),
            Ok(_) => (),
            Err(error) => limitations.push(format!("codex hooks cannot be inspected: {error}")),
        }
    }
    if let Some(warning) = &sandbox_warning {
        limitations.push(format!("codex sandbox: {warning}"));
    }
    let roots_warning = super::setup::codex_missing_roots_warning(&env, None);
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
        "permissions": super::setup::permission_status_json(&env),
        "trust": codex_trust,
        "recipes": crate::harness::recipe::describe(super::RECIPES),
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
    let manual_repairs = manual_repairs(daemon, &hooks);
    DoctorProjection {
        status: Some(captured_legacy_status(request, &hooks)),
        hooks,
        limitations,
        manual_repairs,
        safe_repairs: vec![],
        repair_options: Default::default(),
    }
}

/// Text for `report.hooks.codex`.
pub(super) fn doctor_text(codex: &Value, form: DoctorTextForm) -> String {
    match form {
        DoctorTextForm::Summary => summary_text(codex),
        DoctorTextForm::Debug => debug_text(codex),
    }
}

fn summary_text(codex: &Value) -> String {
    let mut out = legacy_summary_lines("codex", codex);
    if codex["setup"]["installed"] == json!(true) {
        match codex["trust"]["status"].as_str() {
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
    out
}

fn debug_text(codex: &Value) -> String {
    let mut out = String::new();
    out.push_str(&format!("hooks.codex: {}\n", scalar(&codex["detail"])));
    out.push_str(&format!(
        "hooks.codex.recipes: {}\n",
        scalar(&codex["recipes"])
    ));
    let setup = &codex["setup"];
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
    if let Some(line) = legacy_registration_line(
        "codex",
        setup,
        "; Codex then asks to review (trust) the rewritten hooks again",
    ) {
        out.push_str(&line);
    }
    out.push_str(&permission_line("codex", &codex["permissions"]));
    out.push_str(&format!(
        "hooks.codex.trust.status: {}\n",
        scalar(&codex["trust"]["status"])
    ));
    let installed = &codex["installed"];
    // A declared contract carries no evidence field; say what it means.
    let installed_line = match installed["admission"].as_str() {
        Some("contract_declared") => {
            "contract_declared; runtime metadata unavailable; rich optional capabilities unavailable".to_owned()
        }
        Some("not_found") => "no executable codex on PATH".to_owned(),
        _ if !installed["evidence"].is_null() => scalar(&installed["evidence"]),
        _ => scalar(&installed["admission"]),
    };
    out.push_str(&format!("hooks.codex.installed: {installed_line}\n"));
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
    let sandbox_warning = &codex["sandbox_warning"];
    if !sandbox_warning.is_null() {
        out.push_str(&format!(
            "hooks.codex.sandbox_warning: {}\n",
            scalar(sandbox_warning)
        ));
    }
    if !codex["socket_policy_validation"].is_null() {
        out.push_str(&format!(
            "hooks.codex.socket_policy_validation: {}\n",
            scalar(&codex["socket_policy_validation"])
        ));
    }
    let last_hook = &codex["last_hook"];
    if !last_hook.is_null() {
        out.push_str(&format!(
            "hooks.codex.last_hook: {}\n",
            scalar(&last_hook["evidence"])
        ));
    }
    out
}
