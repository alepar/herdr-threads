//! Test-only exact synthetic_fourth identity with an optional composer policy.
//! The builtin remains without a composer. No native/runtime authority is supplied.
use super::synthetic_fourth::ADAPTER;
use crate::harness::{
    adapter::*,
    composer::ComposerRead,
    registry::{Registration, Registry},
};
use crate::protocol::{results::ApiError, time::CallBudget};
use crate::store::archival::{self, ObservationTicket, Progress, Runtime};
use rusqlite::{Connection, Transaction, TransactionBehavior};
use std::sync::OnceLock;
static OVERLAY: Overlay = Overlay;
struct Overlay;
impl HarnessAdapter for Overlay {
    type Admission = ();
    fn metadata(&self) -> &'static AdapterMetadata {
        ADAPTER.metadata()
    }
    fn contracts(&self) -> &'static [ContractDescriptor] {
        ADAPTER.contracts()
    }
    fn observe_install(&self, e: &InstallEnvironment, b: &CallBudget) -> InstallObservation {
        ADAPTER.observe_install(e, b)
    }
    fn admit(&self, r: &AdmissionRequest, b: &CallBudget) -> AdmissionDecision<()> {
        ADAPTER.admit(r, b)
    }
    fn version_ladder(&self, r: &RuntimeIdentity) -> Ladder {
        ADAPTER.version_ladder(r)
    }
    fn classify(&self, i: &HookInput) -> ContractObservation {
        ADAPTER.classify(i)
    }
    fn decode(&self, a: &(), i: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
        ADAPTER.decode(a, i)
    }
    fn encode(
        &self,
        a: &(),
        e: &DecodedEvent,
        o: &NeutralOffer,
    ) -> Result<EncodedOutput, EncodeFailure> {
        ADAPTER.encode(a, e, o)
    }
    fn attribute_runtime(&self, i: &HookInput, b: &CallBudget) -> RuntimeAttribution {
        ADAPTER.attribute_runtime(i, b)
    }
    fn setup(&self, r: &SetupRequest, b: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
        ADAPTER.setup(r, b)
    }
    fn status(&self, r: &StatusRequest, b: &CallBudget) -> SetupStatus {
        ADAPTER.status(r, b)
    }
    fn unsetup(&self, r: &UnsetupRequest, b: &CallBudget) -> Result<RemovalOutcome, SetupFailure> {
        ADAPTER.unsetup(r, b)
    }
    fn hook_admission_policy(&self) -> HookAdmissionPolicy {
        ADAPTER.hook_admission_policy()
    }
    fn setup_environment_inputs(&self) -> &'static [&'static str] {
        ADAPTER.setup_environment_inputs()
    }
    fn resolve_setup_scope(
        &self,
        r: &SetupScopeRequest,
        e: &SetupEnvironment,
    ) -> Result<ResolvedSetupScope, SetupFailure> {
        ADAPTER.resolve_setup_scope(r, e)
    }
    fn launch_policy(&self) -> Option<&dyn LaunchPolicy> {
        ADAPTER.launch_policy()
    }
    fn composer_policy(&self) -> Option<&dyn ComposerPolicy> {
        Some(self)
    }
}
impl ComposerPolicy for Overlay {
    fn capabilities(&self, _: Option<&str>) -> crate::harness::recipe::PokeCapabilities {
        crate::harness::recipe::PokeCapabilities::NONE
    }
    fn read(&self, d: &str, _: Option<u16>) -> ComposerRead {
        match d {
            "SYNTHETIC COMPOSER EMPTY" => ComposerRead::Empty,
            "SYNTHETIC COMPOSER TEXT" => ComposerRead::Text("fixture draft".into()),
            "SYNTHETIC COMPOSER UNSAFE" => ComposerRead::Unsafe {
                text: "fixture unsafe".into(),
                reason: "captured synthetic unsafe",
            },
            _ => ComposerRead::Unreadable,
        }
    }
    fn clear_key(&self) -> &'static str {
        "ctrl+u"
    }
    fn restore_text(&self, s: &str) -> String {
        s.into()
    }
}
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Registry::new(Box::leak(
            vec![Registration::new(&OVERLAY)].into_boxed_slice(),
        ))
        .expect("valid exact fourth overlay")
    })
}
pub fn advance(
    db: &Connection,
    instance: &str,
    rt: &Runtime,
    r: &Registry,
) -> Result<Progress, ApiError> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .expect("fixture transaction");
    let out = archival::advance_in_with_registry(&tx, instance, rt, r)?;
    tx.commit().expect("fixture commit");
    Ok(out)
}
pub fn ticket(
    db: &Connection,
    instance: &str,
    seat: &str,
    rt: &Runtime,
    r: &Registry,
) -> Result<Option<ObservationTicket>, ApiError> {
    archival::observation_ticket_with_registry(db, instance, seat, rt, r)
}
pub fn next(
    db: &Connection,
    instance: &str,
    rt: &Runtime,
    r: &Registry,
) -> Result<Option<ObservationTicket>, ApiError> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .expect("fixture transaction");
    let out = archival::next_observation_in_with_registry(&tx, instance, rt, r)?;
    tx.commit().expect("fixture commit");
    Ok(out)
}
pub fn sample(
    db: &Connection,
    t: &ObservationTicket,
    rt: &Runtime,
    s: &crate::ports::ComposerObservation,
    r: &Registry,
) -> Result<bool, ApiError> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .expect("fixture transaction");
    let out = archival::record_sample_in_with_registry(&tx, t, rt, s, r)?;
    tx.commit().expect("fixture commit");
    Ok(out)
}
