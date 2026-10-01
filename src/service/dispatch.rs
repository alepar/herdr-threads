//! Domain dispatch keeps read work separate from writer and host work.

use crate::{
    identity::repair::OrdinaryIdentity,
    ports::{
        DurableWorkAdmission, HostPort, LocalService, ReadContext, RegisterAvailableRequest,
        SendPreparationProgress, ServiceAuthorityGate, ServiceConnectionAuthority, StorePort,
    },
    protocol::{
        authority::{CallerRole, OperatorActor, PeerIdentity},
        commands::{Command, OperatorCommand, PermitMutation},
        output::OutputSpec,
        results::{ApiError, CommandResult, ErrorCode},
        service::{ServiceOperation, ServiceResult},
        time::{CallBudget, Clock},
    },
    service::workers::{BoundedLane, FairWriter},
};
use std::sync::Arc;

pub struct DomainService {
    instance: String,
    store: Arc<dyn StorePort>,
    clock: Arc<dyn Clock>,
    search: BoundedLane,
    current_target: Option<Arc<OrdinaryIdentity>>,
    operator_owner_uid: Option<u32>,
    cooperative_runtime: Option<(u32, Arc<FairWriter>)>,
}

impl DomainService {
    pub fn new(instance: String, store: Arc<dyn StorePort>, clock: Arc<dyn Clock>) -> Self {
        Self {
            instance,
            store,
            clock,
            search: BoundedLane::new(1, 4),
            current_target: None,
            operator_owner_uid: None,
            cooperative_runtime: None,
        }
    }
    /// Supplied by the elected runtime, never by a request payload.
    pub fn with_operator_owner(mut self, owner_uid: u32) -> Self {
        self.operator_owner_uid = Some(owner_uid);
        self
    }
    /// Elected local runtime authority and its shared foreground writer.
    pub fn with_cooperative_owner(mut self, owner_uid: u32, writer: Arc<FairWriter>) -> Self {
        self.cooperative_runtime = Some((owner_uid, writer));
        self
    }

    fn cooperative_mutation(
        &self,
        mutation: PermitMutation,
        peer: PeerIdentity,
        budget: &CallBudget,
        read: ReadContext,
    ) -> Result<CommandResult, ApiError> {
        let (owner_uid, writer) = self.cooperative_runtime.as_ref().ok_or_else(|| {
            error(
                ErrorCode::CallerUnverified,
                "cooperative elected runtime unavailable",
            )
        })?;
        if peer.effective_uid() != *owner_uid {
            return Err(error(
                ErrorCode::Unauthorized,
                "caller peer does not match elected owner",
            ));
        }
        let request = crate::store::cooperative_permit_request(&mutation)?;
        if request.claim.instance != self.instance {
            return Err(error(
                ErrorCode::CallerUnverified,
                "caller instance does not match service",
            ));
        }
        if request.claim.role != CallerRole::TopLevel {
            return Err(error(
                ErrorCode::CallerUnverified,
                "declared subagent cannot mutate",
            ));
        }
        if let PermitMutation::SendMessage(command) = &mutation {
            loop {
                let progress = {
                    let _turn = writer.enter_foreground(budget, self.clock.as_ref())?;
                    self.store.prepare_send_step(
                        command,
                        DurableWorkAdmission::new(16)
                            .map_err(|detail| error(ErrorCode::InvalidRequest, detail))?,
                        budget,
                    )?
                };
                match progress {
                    SendPreparationProgress::Committed(result) => return Ok(result),
                    SendPreparationProgress::Ready { .. } => break,
                    SendPreparationProgress::More { .. } => {
                        // The writer turn is released between quanta.
                        failpoint!("send.between_preparation_steps", self.instance);
                    }
                }
            }
        }
        // Admission precedes the issuer's bounded write. Retain this one turn
        // through the decision; the store rechecks context, budget and permit age.
        let _turn = writer.enter_foreground(budget, self.clock.as_ref())?;
        let permit = self.store.issue_cooperative_permit(request, budget)?;
        match mutation {
            PermitMutation::CheckIn(command) => self.store.register_available(
                RegisterAvailableRequest {
                    command,
                    registration: None,
                    read,
                },
                permit,
                budget,
            ),
            mutation => self.store.mutate(mutation, permit, budget),
        }
    }
    pub fn with_host(
        instance: String,
        store: Arc<dyn StorePort>,
        clock: Arc<dyn Clock>,
        host: Arc<dyn HostPort>,
        writer: Arc<FairWriter>,
    ) -> Self {
        let current_target = OrdinaryIdentity::new(
            instance.clone(),
            Arc::clone(&store),
            host,
            Arc::clone(&clock),
            writer,
        );
        let mut service = Self::new(instance, store, clock);
        service.current_target = Some(Arc::new(current_target));
        service
    }
    pub fn with_identity(
        instance: String,
        store: Arc<dyn StorePort>,
        clock: Arc<dyn Clock>,
        identity: Arc<OrdinaryIdentity>,
    ) -> Self {
        let mut service = Self::new(instance, store, clock);
        service.current_target = Some(identity);
        service
    }
}

impl LocalService for DomainService {
    fn audit_service_disconnect(
        &self,
        boot: &str,
        generation: u64,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.store
            .audit_service_disconnect(boot, generation, peer, budget)
    }

    fn service_operation(
        &self,
        operation: ServiceOperation,
        connection: &ServiceConnectionAuthority,
        gate: &dyn ServiceAuthorityGate,
        budget: &CallBudget,
    ) -> Result<ServiceResult, ApiError> {
        if let Some((_, writer)) = &self.cooperative_runtime {
            self.store
                .service_operation_admitted(operation, connection, gate, budget, writer)
        } else {
            self.store
                .service_operation(operation, connection, gate, budget)
        }
    }
    fn handle(
        &self,
        command: Command,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.handle_with_output(command, peer, budget, &OutputSpec::default())
    }

    fn handle_with_output(
        &self,
        command: Command,
        peer: PeerIdentity,
        budget: &CallBudget,
        output: &OutputSpec,
    ) -> Result<CommandResult, ApiError> {
        let read = ReadContext {
            instance: self.instance.clone(),
            output: output.clone(),
            operation_scope: None,
        };
        if matches!(&command, Command::Search(_)) {
            let _search = self.search.enter(budget, self.clock.as_ref())?;
            return self.store.query(&command, &read, budget);
        }
        match command {
            Command::Directory(_)
            | Command::Seats(_)
            | Command::SeatInspect(_)
            | Command::Inbox(_)
            | Command::Warnings(_)
            | Command::Thread(_)
            | Command::History(_)
            | Command::Participants(_)
            | Command::Recipients(_)
            | Command::DeliveryInspect(_)
            | Command::PendingReceipts(_)
            | Command::AttentionDigest(_)
            | Command::Message(_)
            | Command::Diagnostics(_)
            | Command::RetirementJobs(_) => self.store.query(&command, &read, budget),
            Command::OperationStatus(_) => Err(error(
                ErrorCode::Unauthorized,
                "verified operation scope required",
            )),
            command @ (Command::CheckIn(_)
            | Command::CreateThread(_)
            | Command::Invite(_)
            | Command::Accept(_)
            | Command::AcceptRequired(_)
            | Command::SendMessage(_)
            | Command::Ack(_)
            | Command::Leave(_)
            | Command::SetTopic(_)
            | Command::Archive(_)
            | Command::Reopen(_)) => {
                let mutation = PermitMutation::try_from(command).map_err(|_| {
                    error(
                        ErrorCode::InvalidRequest,
                        "command is not an accountable mutation",
                    )
                })?;
                self.cooperative_mutation(mutation, peer, budget, read)
            }
            Command::ResolveSeat(request) => {
                let preparation = self.current_target.as_ref().ok_or_else(|| {
                    error(
                        ErrorCode::Unsupported,
                        "current-target preparation unavailable",
                    )
                })?;
                preparation
                    .resolve(request, budget)
                    .map(CommandResult::SeatResolved)
            }
            command @ (Command::OperatorRebind(_)
            | Command::OperatorFreshSeat(_)
            | Command::OperatorOrphanInvite(_)
            | Command::OperatorRetire(_)
            | Command::OperatorReplace(_)) => {
                let actor = self
                    .operator_owner_uid
                    .and_then(|uid| OperatorActor::from_peer(peer, uid))
                    .ok_or_else(|| {
                        error(
                            ErrorCode::Unauthorized,
                            "operator peer does not match elected owner",
                        )
                    })?;
                let preparation = self.current_target.as_ref().ok_or_else(|| {
                    error(
                        ErrorCode::Unsupported,
                        "current-target preparation unavailable",
                    )
                })?;
                let operator = OperatorCommand::try_from(command).map_err(|_| {
                    error(
                        ErrorCode::InvalidRequest,
                        "command is not an operator action",
                    )
                })?;
                preparation.operator(operator, actor, budget)
            }
            Command::LocalIntents(_) => Err(error(
                ErrorCode::Unsupported,
                "local intents are client-owned",
            )),
            Command::Health
            | Command::Stop(_)
            | Command::ServiceInspect
            | Command::ServiceDisconnect(_)
            | Command::Search(_) => Err(error(
                ErrorCode::Unsupported,
                "control route unavailable in domain service",
            )),
        }
    }
}

fn error(code: ErrorCode, detail: &str) -> ApiError {
    ApiError {
        code,
        detail: detail.into(),
        restart_argv: None,
        required_minimum_bytes: None,
    }
}

#[cfg(test)]
mod operator_tests {
    use super::*;
    use crate::{
        app::SystemClock,
        protocol::{
            commands::OperatorFreshSeat,
            ids::{HostTargetId, OperationId},
            time::{Cancellation, MonoInstant},
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    #[test]
    fn mismatched_kernel_peer_is_rejected_before_target_preparation() {
        let path = std::env::temp_dir().join(format!("operator-peer-{}.db", uuid::Uuid::new_v4()));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), clock.clone()),
                "i",
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let domain = DomainService::new("i".into(), store, clock.clone()).with_operator_owner(501);
        let result = domain.handle(
            Command::OperatorFreshSeat(OperatorFreshSeat {
                target: HostTargetId::new("p"),
                operation: OperationId::new("op"),
            }),
            PeerIdentity::from_kernel(502),
            &CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0 + 1000),
                cancellation: Cancellation::default(),
            },
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::Unauthorized);
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(domain);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
#[path = "../../tests/service/cooperative.rs"]
mod cooperative_tests;
