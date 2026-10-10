//! Concrete selected-runtime adapter; every mutation decides under canonical A2.
use super::{attempts, *};
use crate::{
    ports::{BootstrapAttachmentGuard, BootstrapStorePort},
    protocol::{
        authority::{MutationPermit, OperatorActor},
        commands::{Command, PermitMutation},
        handoff::AutomaticBootstrapMutation,
        results::CommandResult,
        time::CallBudget,
    },
    store::{SqliteStore, connection::api_error, control, schema, seats},
};

/// These selected-runtime adapters also prepare unlinked ordinary sends.
/// Direct admission needs canonical linked/unlinked routing and exact frozen
/// marker/target equality for automatic handoff.
fn require_legacy_send_marker(
    send: &crate::protocol::commands::SendMessage,
) -> Result<(), ApiError> {
    if send.direct_recipient.is_some() {
        return Err(api_error(
            ErrorCode::Unsupported,
            "direct handoff sends require guarded automatic runtime",
        ));
    }
    Ok(())
}

fn required_guard(
    guard: Option<&BootstrapAttachmentGuard>,
) -> Result<&BootstrapAttachmentGuard, ApiError> {
    guard.ok_or_else(|| {
        api_error(
            ErrorCode::StaleHostObservation,
            "bootstrap deciding observation missing",
        )
    })
}
fn apply(
    tx: &rusqlite::Transaction<'_>,
    canonical: &crate::protocol::handoff::HandoffNamespace,
    command: &PermitMutation,
    guard: Option<&BootstrapAttachmentGuard>,
    now: crate::protocol::time::UtcMillis,
) -> Result<CommandResult, ApiError> {
    match command {
        PermitMutation::BeginBootstrap(v) => begin_pending(tx, canonical, &v.identity, now)
            .map(|v| CommandResult::Bootstrap(Box::new(v))),
        PermitMutation::ReserveBootstrapAttempt(v) => attempts::reserve_attempt(tx, canonical, v)
            .map(|v| CommandResult::BootstrapReserved(Box::new(v))),
        PermitMutation::RecordBootstrapCreated(v) => attempts::record_created(tx, canonical, v)
            .map(|v| CommandResult::Bootstrap(Box::new(v))),
        PermitMutation::RecordBootstrapNotSubmitted(v) => {
            attempts::record_not_submitted(tx, canonical, v)
                .map(|v| CommandResult::Bootstrap(Box::new(v)))
        }
        PermitMutation::CheckBootstrapSubmission(v) => attempts::check_submission(tx, canonical, v)
            .map(CommandResult::BootstrapSubmissionChecked),
        PermitMutation::AttachBootstrapHandoff(v) => {
            attach_pending(tx, canonical, v, required_guard(guard)?)
                .map(|v| CommandResult::Bootstrap(Box::new(v)))
        }
        _ => Err(api_error(
            ErrorCode::InvalidRequest,
            "not a selected bootstrap transition",
        )),
    }
}
fn validate_delivery_namespace(
    instance: &str,
    canonical: &HandoffNamespace,
    request: &crate::protocol::handoff::DeliveryMutation,
) -> Result<(), ApiError> {
    request
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let frozen = &request.plan.payload.namespace;
    if canonical.instance != instance
        || frozen.instance != canonical.instance
        || frozen.state_dir.as_os_str() != canonical.state_dir.as_os_str()
        || frozen.host_endpoint.as_os_str() != canonical.host_endpoint.as_os_str()
    {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "delivery selected namespace differs",
        ));
    }
    Ok(())
}

impl BootstrapStorePort for SqliteStore {
    fn automatic_available(&self) -> bool {
        true
    }
    fn bootstrap_begin_permit(
        &self,
        canonical: &HandoffNamespace,
        command: &crate::protocol::handoff::BeginBootstrap,
        budget: &CallBudget,
    ) -> Result<MutationPermit, ApiError> {
        seats::issue_bootstrap_begin_permit(
            &self.context,
            &*self.writer(budget)?,
            &self.instance,
            canonical,
            command,
            budget,
        )
    }
    fn bootstrap_prepare_send_step(
        &self,
        canonical: &HandoffNamespace,
        command: &crate::protocol::commands::SendMessage,
        admission: crate::ports::DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<crate::ports::SendPreparationProgress, ApiError> {
        crate::store::messages::prepare_bootstrap_send_step(
            &self.context,
            &mut *self.writer(budget)?,
            command,
            self.settings.message_limits,
            budget,
            admission,
            canonical,
        )
    }
    fn automatic_bootstrap_check_start(
        &self,
        canonical: &HandoffNamespace,
        request: &AutomaticBootstrapMutation,
        guard: BootstrapAttachmentGuard,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        self.context.execute_budgeted_decision(
            &mut *self.writer(budget)?,
            budget,
            |_| Ok(()),
            |tx, _, ()| super::automatic::check_start(tx, canonical, request, &guard),
        )
    }
    fn automatic_bootstrap_query(
        &self,
        canonical: &HandoffNamespace,
        request: &AutomaticBootstrapMutation,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic bootstrap instance differs",
            ));
        }
        self.context.execute_budgeted_decision(
            &mut *self.writer(budget)?,
            budget,
            |_| Ok(()),
            |tx, _, ()| super::automatic::query(tx, canonical, request),
        )
    }
    fn automatic_bootstrap_recovery_replay(
        &self,
        canonical: &HandoffNamespace,
        request: &AutomaticBootstrapMutation,
        actor: &OperatorActor,
        budget: &CallBudget,
    ) -> Result<Option<CommandResult>, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        self.context.execute_budgeted_decision(
            &mut *self.writer(budget)?,
            budget,
            |_| Ok(()),
            |tx, _, ()| super::automatic::recovery_replay(tx, canonical, request, actor),
        )
    }
    fn automatic_bootstrap_recover(
        &self,
        canonical: &HandoffNamespace,
        request: &AutomaticBootstrapMutation,
        actor: &OperatorActor,
        guard: Option<BootstrapAttachmentGuard>,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        self.context.execute_budgeted_decision(
            &mut *self.writer(budget)?,
            budget,
            |_| Ok(()),
            |tx, _, ()| super::automatic::recover(tx, canonical, request, actor, guard.as_ref()),
        )
    }
    fn automatic_bootstrap_complete(
        &self,
        canonical: &HandoffNamespace,
        request: &AutomaticBootstrapMutation,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        self.context.execute_budgeted_decision(
            &mut *self.writer(budget)?,
            budget,
            |_| Ok(()),
            |tx, at, ()| super::automatic::complete(tx, canonical, request, at.utc),
        )
    }
    fn automatic_bootstrap_mutate(
        &self,
        canonical: &HandoffNamespace,
        request: &AutomaticBootstrapMutation,
        permit: MutationPermit,
        guard: Option<BootstrapAttachmentGuard>,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic bootstrap instance differs",
            ));
        }
        if matches!(
            request.action,
            crate::protocol::handoff::AutomaticBootstrapAction::Complete { .. }
        ) {
            return self.automatic_bootstrap_complete(canonical, request, budget);
        }
        super::automatic::mutate(
            &self.context,
            &mut *self.writer(budget)?,
            canonical,
            request,
            permit,
            guard.as_ref(),
            budget,
        )
    }
    fn automatic_delivery_prepare_send_step(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::AutomaticDeliveryMutation,
        admission: crate::ports::DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<crate::ports::SendPreparationProgress, ApiError> {
        crate::store::handoff::automatic::validate_namespace(canonical, request)?;
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        crate::store::messages::prepare_automatic_send_step(
            &self.context,
            &mut *self.writer(budget)?,
            request,
            self.settings.message_limits,
            budget,
            admission,
        )
    }
    fn automatic_delivery_query(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::AutomaticDeliveryMutation,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        crate::store::handoff::automatic::validate_namespace(canonical, request)?;
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        self.context.execute_budgeted_decision(
            &mut *self.writer(budget)?,
            budget,
            |_| Ok(()),
            |tx, _, ()| crate::store::handoff::automatic::query(tx, request),
        )
    }
    fn automatic_delivery_mutate(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::AutomaticDeliveryMutation,
        permit: MutationPermit,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        crate::store::handoff::automatic::validate_namespace(canonical, request)?;
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "automatic instance differs",
            ));
        }
        if let crate::protocol::handoff::AutomaticDeliveryAction::Send(send) = &request.action {
            return crate::ports::StorePort::mutate(
                self,
                PermitMutation::SendMessage(send.clone()),
                permit.with_automatic_delivery(request),
                budget,
            );
        }
        crate::store::handoff::automatic::mutate(
            &self.context,
            &mut *self.writer(budget)?,
            budget,
            request,
            permit,
            self.settings.invitation_default_ms,
        )
    }
    fn delivery_query(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::DeliveryMutation,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        validate_delivery_namespace(&self.instance, canonical, request)?;
        let mut db = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut db,
            budget,
            |_| Ok(()),
            |tx, _, ()| {
                use crate::protocol::handoff::DeliveryAction;
                match &request.action {
                    DeliveryAction::Status(identity) => {
                        crate::store::handoff::current(tx, identity)?
                            .map(CommandResult::Handoff)
                            .ok_or_else(|| api_error(ErrorCode::NotFound, "delivery missing"))
                    }
                    DeliveryAction::Prepare(identity) => {
                        crate::store::handoff::validate_live(
                            tx,
                            identity,
                            identity.thread.as_ref(),
                        )?;
                        seats::eligible_delivery_recipient(
                            tx,
                            &self.instance,
                            &identity.recipient,
                        )?;
                        Ok(CommandResult::SeatResolved(identity.recipient.clone()))
                    }
                    _ => Err(ApiError::invalid_request("not a delivery query")),
                }
            },
        )
    }
    fn delivery_mutate(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::DeliveryMutation,
        permit: MutationPermit,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        validate_delivery_namespace(&self.instance, canonical, request)?;
        let permit = permit.with_delivery(request);
        match &request.action {
            crate::protocol::handoff::DeliveryAction::Begin(command) => {
                crate::store::handoff::mutate_in_namespace(
                    &self.context,
                    &mut *self.writer(budget)?,
                    budget,
                    command,
                    permit,
                    false,
                    canonical,
                )
            }
            crate::protocol::handoff::DeliveryAction::Complete(command) => {
                crate::store::handoff::mutate(
                    &self.context,
                    &mut *self.writer(budget)?,
                    budget,
                    command,
                    permit,
                    true,
                )
            }
            _ => crate::ports::StorePort::mutate(
                self,
                request.inner().map_err(ApiError::invalid_request)?,
                permit,
                budget,
            ),
        }
    }
    fn delivery_prepare_send_step(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::DeliveryMutation,
        admission: crate::ports::DurableWorkAdmission,
        budget: &CallBudget,
    ) -> Result<crate::ports::SendPreparationProgress, ApiError> {
        if let crate::protocol::handoff::DeliveryAction::Send(send) = &request.action {
            require_legacy_send_marker(send)?;
        }
        validate_delivery_namespace(&self.instance, canonical, request)?;
        let crate::protocol::handoff::DeliveryAction::Send(send) = &request.action else {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "delivery send preparation requires exact send phase",
            ));
        };
        crate::store::messages::prepare_delivery_send_step(
            &self.context,
            &mut *self.writer(budget)?,
            send,
            self.settings.message_limits,
            budget,
            admission,
            request,
        )
    }

    fn bootstrap_mutate(
        &self,
        canonical: &HandoffNamespace,
        command: PermitMutation,
        mut permit: MutationPermit,
        guard: Option<BootstrapAttachmentGuard>,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance || permit.claim().instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "selected bootstrap instance differs",
            ));
        }
        let mut db = self.writer(budget)?;
        match &command {
            PermitMutation::BeginHandoff(v) => {
                return crate::store::handoff::mutate_in_namespace(
                    &self.context,
                    &mut db,
                    budget,
                    v,
                    permit,
                    false,
                    canonical,
                );
            }
            PermitMutation::CreateThread(v) => {
                return control::create_thread_in_namespace(
                    &self.context,
                    &mut db,
                    budget,
                    v,
                    permit,
                    canonical,
                );
            }
            PermitMutation::ResolveBootstrapSeat(v) => {
                return seats::resolve_bootstrap_seat_accountable(
                    &self.context,
                    &mut db,
                    canonical,
                    v,
                    required_guard(guard.as_ref())?,
                    budget,
                    permit,
                )
                .map(CommandResult::SeatResolved);
            }
            _ => {}
        }
        let input = crate::store::cooperative_permit_request(&command)?;
        let (claim, issuance, _) = permit.cooperative_metadata();
        if let PermitMutation::BeginBootstrap(v) = &command {
            // The frozen begin key belongs to the legacy child operation too.
            // Parent replay is retained by its canonical row and child-key registry.
            return self.context.execute_budgeted_decision_with_constraints(
                &mut db,
                budget,
                Some(&issuance),
                |tx| {
                    seats::cooperative_instance(tx, &self.instance, &claim)?;
                    current(tx, canonical, &v.identity)
                },
                |tx, at, status| {
                    if status.is_none_or(|v| {
                        v.state != crate::protocol::handoff::BootstrapState::Completed
                    }) {
                        control::decide_accountable(
                            tx,
                            at,
                            &mut permit,
                            &claim,
                            &claim.seat,
                            &input.operation,
                            &input.obligation,
                            &input.payload_hash,
                        )?;
                    }
                    begin_pending(tx, canonical, &v.identity, at.utc)
                        .map(|v| CommandResult::Bootstrap(Box::new(v)))
                },
            );
        }
        if let PermitMutation::CompleteLinkedBootstrap(v) = &command {
            return self.context.execute_budgeted_decision_with_constraints(
                &mut db,
                budget,
                Some(&issuance),
                |tx| {
                    seats::cooperative_instance(tx, &self.instance, &claim)?;
                    current(tx, canonical, &v.identity)?
                        .ok_or_else(|| api_error(ErrorCode::NotFound, "bootstrap missing"))
                },
                |tx, at, status| {
                    if status.state != crate::protocol::handoff::BootstrapState::Completed {
                        control::decide_accountable(
                            tx,
                            at,
                            &mut permit,
                            &claim,
                            &claim.seat,
                            &input.operation,
                            &input.obligation,
                            &input.payload_hash,
                        )?;
                    }
                    complete_linked_pending(tx, canonical, v, at.utc)
                        .map(|v| CommandResult::LinkedBootstrapCompleted(Box::new(v)))
                },
            );
        }
        // The result retained for reserve is historical. Its presentation runs
        // the canonical helper again, which returns Replay, never authorization.
        let fresh = std::cell::Cell::new(false);
        schema::execute_budgeted_idempotent_transaction_with_constraints(
            &self.context,
            &mut db,
            budget,
            Some(&issuance),
            &format!("seat:{}", claim.seat.as_str()),
            input.operation.as_str(),
            input.payload_hash,
            |tx| seats::cooperative_instance(tx, &self.instance, &claim),
            |_| Ok(()),
            |tx, at| {
                control::decide_accountable(
                    tx,
                    at,
                    &mut permit,
                    &claim,
                    &claim.seat,
                    &input.operation,
                    &input.obligation,
                    &input.payload_hash,
                )?;
                let result = apply(tx, canonical, &command, guard.as_ref(), at.utc)?;
                fresh.set(true);
                Ok(result)
            },
            |tx, historical| {
                if fresh.get() {
                    return Ok(historical);
                }
                apply(
                    tx,
                    canonical,
                    &command,
                    guard.as_ref(),
                    self.context.clock().utc_now(),
                )
            },
        )
    }
    fn bootstrap_query(
        &self,
        canonical: &HandoffNamespace,
        command: &Command,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "selected bootstrap instance differs",
            ));
        }
        let mut db = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut db,
            budget,
            |_| Ok(()),
            |tx, _, ()| match command {
                Command::BootstrapStatus(v) => current(tx, canonical, &v.identity)?
                    .map(|v| CommandResult::Bootstrap(Box::new(v)))
                    .ok_or_else(|| api_error(ErrorCode::NotFound, "bootstrap missing")),
                _ => Err(api_error(
                    ErrorCode::InvalidRequest,
                    "not a bootstrap query",
                )),
            },
        )
    }
    fn bootstrap_recovery_replay(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::RecoverBootstrap,
        actor: &OperatorActor,
        budget: &CallBudget,
    ) -> Result<Option<crate::protocol::handoff::BootstrapRecoveryResult>, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "selected bootstrap instance differs",
            ));
        }
        let mut db = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut db,
            budget,
            |_| Ok(()),
            |tx, _, ()| {
                let saved = attempts::recovery_replay(tx, canonical, request)?;
                if saved.as_ref().is_some_and(|saved| {
                    saved.operator_uid != actor.effective_uid()
                        || saved.operator_provenance != actor.audit_label()
                }) {
                    return Err(api_error(
                        ErrorCode::Unauthorized,
                        "saved recovery belongs to another operator account",
                    ));
                }
                Ok(saved)
            },
        )
    }
    fn bootstrap_recover(
        &self,
        canonical: &HandoffNamespace,
        request: &crate::protocol::handoff::RecoverBootstrap,
        actor: &OperatorActor,
        guard: Option<BootstrapAttachmentGuard>,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if canonical.instance != self.instance {
            return Err(api_error(
                ErrorCode::InstanceMismatch,
                "selected bootstrap instance differs",
            ));
        }
        let mut db = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut db,
            budget,
            |_| Ok(()),
            |tx, at, ()| {
                attempts::recover(
                    tx,
                    canonical,
                    request,
                    actor.effective_uid(),
                    at.utc,
                    guard.as_ref(),
                )
                .map(|v| CommandResult::BootstrapRecovered(Box::new(v)))
            },
        )
    }
}
