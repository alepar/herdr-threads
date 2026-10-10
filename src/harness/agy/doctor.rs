//! agy doctor projection stub. L6 replaces it.
use crate::harness::adapter::{DoctorProjection, StatusRequest};
use crate::protocol::time::CallBudget;

pub(super) fn doctor_projection(
    _: &StatusRequest,
    _: &serde_json::Value,
    _: &CallBudget,
) -> Option<DoctorProjection> {
    None
}
