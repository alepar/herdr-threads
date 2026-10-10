//! `setup` / `status` / `unsetup` stubs. Real behavior lands with L3. Until then each refuses with
//! `UnsupportedHarness`, which the bare multi-harness commands report as "refused" rather than as
//! a failure, so an agy-less or agy-ful machine keeps today's exit statuses.
use crate::{
    harness::adapter::{
        RemovalOutcome, SetupFailure, SetupOutcome, SetupRequest, SetupStatus, StatusRequest,
        UnsetupRequest,
    },
    protocol::results::{ApiError, ErrorCode},
};

fn refusal(operation: &str) -> SetupFailure {
    SetupFailure::Api(ApiError::new(
        ErrorCode::UnsupportedHarness,
        format!("agy: {operation} is not yet supported"),
    ))
}
pub(super) fn setup(_: &SetupRequest) -> Result<SetupOutcome, SetupFailure> {
    Err(refusal("setup"))
}
pub(super) fn status(_: &StatusRequest) -> SetupStatus {
    SetupStatus::Failed(refusal("setup-status"))
}
pub(super) fn unsetup(_: &UnsetupRequest) -> Result<RemovalOutcome, SetupFailure> {
    Err(refusal("unsetup"))
}
