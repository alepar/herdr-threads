//! agy launch policy stub. L5 replaces it; until then every launch step refuses.
use crate::harness::{
    adapter::{LaunchPolicy, LaunchPreparation, LaunchRequest, LaunchScope, LocalSetupStatus},
    launch::PaneShellProbe,
    registry::AdmittedHandle,
};
use crate::protocol::{results::ApiError, time::CallBudget};

fn refused<T>() -> Result<T, ApiError> {
    Err(ApiError::unsupported("agy launch is not yet supported"))
}

pub(super) struct AgyLaunch;
impl LaunchPolicy for AgyLaunch {
    fn resolve_scope(
        &self,
        _: &LaunchRequest,
        _: &dyn PaneShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchScope, ApiError> {
        refused()
    }
    fn validate_native_argv(&self, _: &[String]) -> Result<(), ApiError> {
        refused()
    }
    fn compose_argv(&self, _: Vec<String>, _: Vec<String>) -> Result<Vec<String>, ApiError> {
        refused()
    }
    fn prepare_launch(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
        _: &AdmittedHandle,
        _: &LocalSetupStatus,
        _: &dyn PaneShellProbe,
        _: &CallBudget,
    ) -> Result<LaunchPreparation, ApiError> {
        refused()
    }
    fn configuration_fingerprint(
        &self,
        _: &LaunchRequest,
        _: &LaunchScope,
    ) -> Result<String, ApiError> {
        refused()
    }
    fn expected_host_kinds(&self) -> &'static [&'static str] {
        &["agy"]
    }
}
