//! Test-only synchronization and fault fixtures; absent from ordinary builds.
pub mod attention_oracle;
pub mod counting_client;
pub mod failpoints;
pub mod history;
#[cfg(feature = "test-support")]
pub mod isolated_herdr;
pub mod isolation;
pub mod owner_watch;
#[cfg(feature = "test-support")]
pub mod search_barrier;
#[cfg(feature = "test-support")]
pub mod server_completion;
pub mod spawn;

/// Explicit authenticated-peer fixture for direct handler controls.
/// Production peers are constructed only from the socket kernel credential.
pub fn peer_identity(uid: u32) -> crate::protocol::authority::PeerIdentity {
    crate::protocol::authority::PeerIdentity::from_kernel(uid)
}

/// The `Unsupported` rejection a fixture returns for a port route it does not serve.
pub fn unserved(detail: &str) -> crate::protocol::results::ApiError {
    crate::protocol::results::ApiError::unsupported(detail)
}

/// Explicit `LocalService` bodies for the routes a fixture does not serve,
/// named one per route: `unserved_local_service_routes!(service_control, ...)`.
/// `handle_with_output` serves the default output through `handle`.
#[macro_export]
macro_rules! unserved_local_service_routes {
    () => {};
    (service_control $(, $rest:ident)* $(,)?) => {
        fn service_control(
            &self,
            _: $crate::protocol::commands::Command,
            _: $crate::protocol::authority::PeerIdentity,
            _: &str,
            _: &str,
            _: &$crate::service::live_gate::LiveServiceGate,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::protocol::results::CommandResult, $crate::protocol::results::ApiError> {
            Err($crate::test_support::unserved("service recovery control is unavailable"))
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
    (audit_service_disconnect $(, $rest:ident)* $(,)?) => {
        fn audit_service_disconnect(
            &self,
            _: &str,
            _: u64,
            _: $crate::protocol::authority::PeerIdentity,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<(), $crate::protocol::results::ApiError> {
            Err($crate::test_support::unserved("service recovery audit is unavailable"))
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
    (service_operation $(, $rest:ident)* $(,)?) => {
        fn service_operation(
            &self,
            _: $crate::protocol::service::ServiceOperation,
            _: &$crate::ports::ServiceConnectionAuthority,
            _: &dyn $crate::ports::ServiceAuthorityGate,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::protocol::service::ServiceResult, $crate::protocol::results::ApiError> {
            Err($crate::test_support::unserved("service operation route is unavailable"))
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
    (handle_with_output $(, $rest:ident)* $(,)?) => {
        fn handle_with_output(
            &self,
            command: $crate::protocol::commands::Command,
            peer: $crate::protocol::authority::PeerIdentity,
            budget: &$crate::protocol::time::CallBudget,
            output: &$crate::protocol::output::OutputSpec,
        ) -> Result<$crate::protocol::results::CommandResult, $crate::protocol::results::ApiError> {
            if *output != $crate::protocol::output::OutputSpec::default() {
                return Err($crate::test_support::unserved(
                    "selected output is unavailable in this service",
                ));
            }
            self.handle(command, peer, budget)
        }
        $crate::unserved_local_service_routes!($($rest),*);
    };
}

/// Explicit `LocalClient::call_with_output` for a fixture that serves only the
/// default output through `call`.
#[macro_export]
macro_rules! default_output_local_client {
    () => {
        fn call_with_output(
            &self,
            command: $crate::protocol::commands::Command,
            output: &$crate::protocol::output::OutputSpec,
            budget: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::protocol::results::CommandResult, $crate::protocol::results::ApiError> {
            if *output != $crate::protocol::output::OutputSpec::default() {
                return Err($crate::protocol::results::ApiError::invalid_request(
                    "selected output unsupported by this client",
                ));
            }
            self.call(command, budget)
        }
    };
}

/// Explicit `DeadlinePort` durable-work methods for a fixture with no work jobs.
#[macro_export]
macro_rules! no_durable_work {
    () => {
        fn pending_work(
            &self,
            _: $crate::protocol::pagination::PageRequest,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<
            $crate::protocol::pagination::Page<$crate::ports::WorkCandidate>,
            $crate::protocol::results::ApiError,
        > {
            Ok($crate::protocol::pagination::Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: $crate::protocol::pagination::StopReason::Complete,
                consistency: $crate::protocol::pagination::Consistency::BoundedLive,
            })
        }
        fn advance_work(
            &self,
            _: &str,
            _: $crate::ports::DurableWorkAdmission,
            _: &$crate::protocol::time::CallBudget,
        ) -> Result<$crate::ports::WorkProgress, $crate::protocol::results::ApiError> {
            unreachable!("advance_work requires a discovered work job")
        }
    };
}

/// Construct sealed bootstrap attachment evidence from a qualified test current
/// observation and explicit same-response scope. Absent in ordinary builds.
pub fn bootstrap_attachment_guard(
    request: &crate::protocol::commands::ResolveSeat,
    observation: crate::ports::HostObservation,
    workspace: crate::protocol::ids::HostTargetId,
    tab: crate::protocol::ids::HostTargetId,
    admission: &crate::ports::HostObservationAdmission,
    witness: crate::host::continuity::LocalEndpointWitness,
) -> Result<crate::ports::BootstrapAttachmentGuard, &'static str> {
    let pane =
        crate::ports::BootstrapPaneObservation::try_new(observation, workspace, tab, witness)?;
    crate::ports::BootstrapAttachmentGuard::try_new(request, pane, admission)
}
