//! Codex's doctor surfaces. The seam only forwards; `ht-poh.2.4` moves the Codex projection and
//! text bodies out of `cli::doctor` into this module.
use crate::harness::adapter::DoctorTextForm;
use serde_json::Value;

/// Text for `report.hooks.codex`; empty renders nothing until the doctor bodies move here.
pub(super) fn doctor_text(_hooks: &Value, _form: DoctorTextForm) -> String {
    String::new()
}
