//! agy doctor (agy-adapter D10): the `hooks.agy` projection from the L3 status, a bounded
//! read-only witness-health scan, the sandbox note, limitations, repairs and the Debug and
//! Summary text.
//!
//! Extension point (ht-poh.5): the downgrade line and limitation, and the line
//! `agy delivery: offered, never confirmed (N in generation G)`, are appended in both text forms:
//! in [`debug_text`] after the `hooks.agy.sandbox` line (as `hooks.agy.delivery: ...`) and in
//! [`summary_text`] after the plugin line (as `agy: delivery ...`). The limitation goes into
//! [`doctor_projection`]'s `limitations`. Nothing here reads delivery state.
use super::{
    paths::{self, AgyRoots},
    setup::{self, AllowEntryState},
    witness::{self, BRAIN_ENTRY_CAP},
};
use crate::{
    cli::doctor::scalar,
    harness::adapter::{
        DoctorProjection, DoctorTextForm, LocalSetupStatus, SetupStatus, StatusRequest,
    },
    protocol::time::CallBudget,
};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

/// The most recent `brain/<id>` directories the scan examines.
const SCAN_DIRS: usize = 64;
/// A conversation directory modified within this window is recent.
const RECENT: Duration = Duration::from_secs(24 * 60 * 60);
/// How many `brain/` entries pass between two deadline checks.
const DEADLINE_STRIDE: usize = 256;
/// `hooks.agy.sandbox`, verbatim.
pub const SANDBOX_NOTE: &str = "agy --sandbox sessions may not reach the herdr-threads daemon socket (untested); launch refuses --sandbox";

/// What the witness-health scan saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessHealth {
    pub presence_dir: bool,
    pub brain_entries: usize,
    /// False when `brain/` has more than [`BRAIN_ENTRY_CAP`] entries or the deadline ended the scan.
    pub bounded: bool,
    pub recent_top_level_without_lock: usize,
    /// Why `brain/` could not be listed, when it could not.
    pub error: Option<String>,
}
impl WitnessHealth {
    fn json(&self) -> Value {
        let mut value = json!({
            "presence_dir": self.presence_dir,
            "brain_entries": self.brain_entries,
            "bounded": self.bounded,
            "recent_top_level_without_lock": self.recent_top_level_without_lock,
        });
        if let Some(error) = &self.error {
            value["error"] = json!(error);
        }
        value
    }
}

fn is_dir(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}

/// Bounded, read-only scan: streams `data/brain` (at most [`BRAIN_ENTRY_CAP`] + 1 entries),
/// takes up to [`SCAN_DIRS`] conversation directories modified in the last 24 h, and counts those
/// that have no presence lock and are not listed under another scanned conversation. It uses
/// directory listings and `symlink_metadata` only: no file is ever opened.
pub fn witness_health(
    roots: &AgyRoots,
    now: SystemTime,
    expired: &dyn Fn() -> bool,
) -> WitnessHealth {
    let mut health = WitnessHealth {
        presence_dir: is_dir(&roots.data.join("presence")),
        brain_entries: 0,
        bounded: true,
        recent_top_level_without_lock: 0,
        error: None,
    };
    let brain = roots.data.join("brain");
    let entries = match std::fs::read_dir(&brain) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return health,
        Err(error) => {
            health.error = Some(format!(
                "{} cannot be listed ({:?})",
                brain.display(),
                error.kind()
            ));
            return health;
        }
    };
    let mut recent: Vec<String> = Vec::new();
    for entry in entries {
        if health.brain_entries >= BRAIN_ENTRY_CAP
            || (health.brain_entries.is_multiple_of(DEADLINE_STRIDE) && expired())
        {
            health.bounded = false;
            if health.brain_entries >= BRAIN_ENTRY_CAP {
                health.brain_entries += 1;
            }
            break;
        }
        let Ok(entry) = entry else {
            health.error = Some(format!("{} has an unreadable entry", brain.display()));
            break;
        };
        health.brain_entries += 1;
        if recent.len() >= SCAN_DIRS {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !paths::valid_conversation_id(&id) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        // A modification time in the future (clock skew) reads as recent.
        let fresh = meta.modified().map_or(true, |modified| {
            now.duration_since(modified)
                .map_or(true, |age| age <= RECENT)
        });
        if meta.file_type().is_dir() && fresh {
            recent.push(id);
        }
    }
    let lock_free = |id: &String| {
        let lock = roots.data.join("presence").join(format!("{id}.lock"));
        !std::fs::symlink_metadata(lock).is_ok_and(|meta| meta.file_type().is_file())
    };
    health.recent_top_level_without_lock = recent
        .iter()
        .filter(|id| lock_free(id))
        .filter(|id| {
            !recent.iter().any(|parent| {
                parent != *id
                    && witness::listing_entry(&roots.data, parent, id)
                        == crate::harness::adapter::Witness::Present
            })
        })
        .count();
    health
}

/// agy has left a trace on this machine: an executable on PATH, or its data or plugin directory.
/// Without one, a missing plugin is no degradation.
fn agy_present(roots: &AgyRoots, request: &StatusRequest) -> bool {
    crate::cli::hook::resolve_on_path("agy", request.environment.path.as_deref()).is_some()
        || is_dir(&roots.data)
        || is_dir(&roots.plugin_dir)
}

fn limitations_of(
    status: &LocalSetupStatus,
    allow: AllowEntryState,
    health: &WitnessHealth,
) -> Vec<String> {
    let plugin = &status.projection["plugin"];
    let mut out = Vec::new();
    if plugin["installed"] != json!(true) {
        out.push("agy plugin is not installed: run `herdr-threads setup agy`".to_owned());
    } else if plugin["current"] != json!(true) {
        out.push(
            "agy plugin is stale: it differs from this herdr-threads build; run `herdr-threads setup agy`"
                .to_owned(),
        );
    }
    if plugin["enabled"] == json!(false) {
        out.push("agy plugin is disabled: run `agy plugin enable herdr-threads`".to_owned());
    }
    if allow == AllowEntryState::Missing {
        out.push(format!(
            "agy allow entry {} is missing from antigravity-cli/settings.json: run `herdr-threads setup agy`",
            setup::ALLOW_RULE
        ));
    }
    if health.recent_top_level_without_lock > 0 {
        out.push(format!(
            "SessionStart without witness: {} recent top-level conversations lack a presence lock",
            health.recent_top_level_without_lock
        ));
    }
    if !health.bounded {
        out.push(format!(
            "subagent check bounded: brain/ has {} entries",
            health.brain_entries
        ));
    }
    if let Some(error) = &health.error {
        out.push(format!("agy witness scan incomplete: {error}"));
    }
    out
}

pub(super) fn doctor_projection(
    request: &StatusRequest,
    _: &Value,
    budget: &CallBudget,
) -> Option<DoctorProjection> {
    let SetupStatus::Detailed(status) = setup::status(request) else {
        return None;
    };
    let roots = match &request.scope {
        crate::harness::adapter::ResolvedSetupScope::ConfigRoot(root) => {
            AgyRoots::from_config_root(root.clone())
        }
        _ => return None,
    };
    let clock = &request.environment.clock;
    let expired = || budget.cancellation.is_cancelled() || clock.monotonic_now() >= budget.deadline;
    let health = witness_health(&roots, SystemTime::now(), &expired);
    let allow = match status.projection["allow_entry"]["state"].as_str() {
        Some("owned") => AllowEntryState::Owned,
        Some("pre_existing") => AllowEntryState::PreExisting,
        Some("not_granted") => AllowEntryState::NotGranted,
        _ => AllowEntryState::Missing,
    };
    let present = agy_present(&roots, request);
    let mut hooks = status.projection.clone();
    hooks["witness"] = health.json();
    hooks["sandbox"] = json!(SANDBOX_NOTE);
    let limitations = if present {
        limitations_of(&status, allow, &health)
    } else {
        Vec::new()
    };
    let mut manual_repairs = Vec::new();
    if present && status.enabled == Some(false) {
        manual_repairs.push(json!({
            "action": "agy plugin enable herdr-threads",
            "outcome": "manual",
            "detail": "the herdr-threads plugin is disabled in agy; run `agy plugin enable herdr-threads` (herdr-threads never re-enables it)",
        }));
    }
    let safe_repairs = if present {
        status.repairs.clone()
    } else {
        Vec::new()
    };
    Some(DoctorProjection {
        status: Some(SetupStatus::Detailed(status)),
        hooks,
        limitations,
        manual_repairs,
        safe_repairs,
        repair_options: Default::default(),
    })
}

/// Text for `report.hooks.agy`.
pub(super) fn doctor_text(hooks: &Value, form: DoctorTextForm) -> String {
    // No projection (a skipped or failed observation) renders nothing.
    if !hooks["plugin"].is_object() {
        return String::new();
    }
    match form {
        DoctorTextForm::Summary => summary_text(hooks),
        DoctorTextForm::Debug => debug_text(hooks),
    }
}

fn yes(value: &Value) -> &'static str {
    match value.as_bool() {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn summary_text(hooks: &Value) -> String {
    let plugin = &hooks["plugin"];
    let state = if plugin["installed"] != json!(true) {
        "plugin missing"
    } else if plugin["current"] != json!(true) {
        "plugin stale"
    } else if plugin["enabled"] == json!(false) {
        "plugin disabled"
    } else {
        "plugin installed"
    };
    format!(
        "agy: {state}, allow entry {}\n",
        scalar(&hooks["allow_entry"]["state"]).replace('_', "-")
    )
}

fn debug_text(hooks: &Value) -> String {
    let plugin = &hooks["plugin"];
    let mut out = String::new();
    out.push_str(&format!(
        "hooks.agy.plugin: {} installed={} current={} enabled={} version={}\n",
        scalar(&plugin["path"]),
        yes(&plugin["installed"]),
        yes(&plugin["current"]),
        yes(&plugin["enabled"]),
        scalar(&plugin["version"]),
    ));
    out.push_str(&format!(
        "hooks.agy.hooks_file: {}\n",
        scalar(&hooks["hooks_file"])
    ));
    let allow = &hooks["allow_entry"];
    out.push_str(&format!(
        "hooks.agy.allow_entry: {} {} in {}\n",
        scalar(&allow["rule"]),
        scalar(&allow["state"]).replace('_', "-"),
        scalar(&allow["file"]),
    ));
    let version = &hooks["harness_version"];
    out.push_str(&format!(
        "hooks.agy.installed: {} (binary {})\n",
        scalar(&version["admission"]),
        scalar(&version["binary"]),
    ));
    let witness = &hooks["witness"];
    if witness.is_object() {
        out.push_str(&format!(
            "hooks.agy.witness: presence_dir={} brain_entries={} bounded={} recent_top_level_without_lock={}\n",
            yes(&witness["presence_dir"]),
            scalar(&witness["brain_entries"]),
            yes(&witness["bounded"]),
            scalar(&witness["recent_top_level_without_lock"]),
        ));
        if witness["error"].is_string() {
            out.push_str(&format!(
                "hooks.agy.witness.error: {}\n",
                scalar(&witness["error"])
            ));
        }
    }
    if hooks["sandbox"].is_string() {
        out.push_str(&format!(
            "hooks.agy.sandbox: {}\n",
            scalar(&hooks["sandbox"])
        ));
    }
    out
}

#[cfg(test)]
#[path = "../../../tests/harness/agy_doctor.rs"]
mod tests;
