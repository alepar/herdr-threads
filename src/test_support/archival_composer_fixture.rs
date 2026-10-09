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
/// Static aliases exercise the existing host-kind contract; these are data only.
pub const ALIAS_64: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const ALIAS_65: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const ALIAS_CONTROL: &str = "synthetic_fourth\tdeclared";
static OVERLAY: Overlay = Overlay;
struct Overlay;
impl HarnessAdapter for Overlay {
    type Admission = ();
    fn metadata(&self) -> &'static AdapterMetadata {
        static METADATA: OnceLock<AdapterMetadata> = OnceLock::new();
        METADATA.get_or_init(|| {
            let base = ADAPTER.metadata();
            let host_kinds = Box::leak(
                base.host_kinds
                    .iter()
                    .copied()
                    .chain([ALIAS_64, ALIAS_65, ALIAS_CONTROL])
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            AdapterMetadata {
                id: base.id,
                display_label: base.display_label,
                context_spelling: base.context_spelling,
                context_aliases: base.context_aliases,
                executable: match &base.executable {
                    ExecutableLookup::Path(path) => ExecutableLookup::Path(path),
                    ExecutableLookup::Unsupported => ExecutableLookup::Unsupported,
                },
                host_kinds,
                setup_scopes: base.setup_scopes,
                budget: EventBudgetPolicy {
                    lifecycle_ms: base.budget.lifecycle_ms,
                    observer_ms: base.budget.observer_ms,
                },
                runtime_sources: base.runtime_sources,
            }
        })
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
