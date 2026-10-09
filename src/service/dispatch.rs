//! Domain dispatch keeps read work separate from writer and host work.

use crate::{
    identity::repair::OrdinaryIdentity,
    ports::{
        DurableWorkAdmission, HostPort, LocalService, ReadContext, RegisterAvailableRequest,
        SendPreparationProgress, ServiceAuthorityGate, ServiceConnectionAuthority, StorePort,
    },
    protocol::{
        authority::{
            COOPERATIVE_TOP_LEVEL_PROVENANCE, CallerClaim, CallerRole, Harness, OperatorActor,
            PeerIdentity,
        },
        commands::{AckModDelivered, Command, OperatorCommand, PermitMutation},
        output::OutputSpec,
        results::{ApiError, CommandResult, ErrorCode},
        service::{ServiceOperation, ServiceResult},
        time::{CallBudget, Clock},
        watch::{ModAckItem, ModAckOutcome, ModAckReason, ModAckReport},
    },
    service::{fair_writer::FairWriter, workers::BoundedLane},
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
    mod_channels: Arc<dyn crate::ports::ModChannels>,
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
            mod_channels: Arc::new(crate::ports::NoModChannels),
        }
    }
    /// The shared registry of live mod delivery channels (spec D2).
    pub fn with_mod_channels(mut self, channels: Arc<dyn crate::ports::ModChannels>) -> Self {
        self.mod_channels = channels;
        self
    }
    pub fn mod_channels(&self) -> &Arc<dyn crate::ports::ModChannels> {
        &self.mod_channels
    }
    /// True while the registry holds a live or grace entry for the seat.
    fn seat_has_mod_channel(&self, seat: &crate::protocol::ids::SeatId) -> bool {
        self.mod_channels.seat_live(seat)
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
        operator_override: bool,
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
        let operator = if operator_override {
            Some(OperatorActor::from_peer(peer, *owner_uid).ok_or_else(|| {
                error(
                    ErrorCode::Unauthorized,
                    "operator peer does not match elected owner",
                )
            })?)
        } else {
            None
        };
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
                    read,
                    operator,
                },
                permit,
                budget,
            ),
            mutation => self.store.mutate(mutation, permit, budget),
        }
    }
    /// Spec D6 pre-decision for `watch ack`: classify the whole batch against
    /// the seat's open binding and the live channel registry, then route the
    /// settlement through the cooperative mutation path. The store decides per
    /// id against the canonical view (A2); a stale client claim is never used
    /// for a resumed generation.
    fn ack_mod_delivered(
        &self,
        request: AckModDelivered,
        peer: PeerIdentity,
        budget: &CallBudget,
        read: ReadContext,
    ) -> Result<CommandResult, ApiError> {
        Command::AckModDelivered(request.clone())
            .validate()
            .map_err(|why| error(ErrorCode::InvalidRequest, why))?;
        let seat = request.claim.seat.clone();
        let report = |result: ModAckOutcome, reason: Option<ModAckReason>| {
            Ok(CommandResult::ModDeliveryAcked(ModAckReport {
                results: request
                    .messages
                    .iter()
                    .map(|id| ModAckItem {
                        id: id.clone(),
                        result,
                        reason,
                    })
                    .collect(),
            }))
        };
        let binding = self.store.mod_ack_binding(&self.instance, &seat, budget)?;
        let Some(binding) = binding.filter(|binding| {
            binding.provenance == COOPERATIVE_TOP_LEVEL_PROVENANCE && binding.harness == "claude"
        }) else {
            // The check-in that opens the binding may not have committed yet.
            return report(ModAckOutcome::Retryable, Some(ModAckReason::NoLiveChannel));
        };
        let same_session = request.claim.native_session == binding.native_session;
        let effective = if request.claim.binding_generation == binding.generation && same_session {
            request.clone()
        } else if same_session
            && binding
                .previous
                .as_ref()
                .is_some_and(|(generation, session)| {
                    *generation == request.claim.binding_generation
                        && *session == binding.native_session
                })
        {
            // Resume: the previous generation of the same native session. The
            // decision is against the canonical current binding, never the
            // stale client claim.
            AckModDelivered {
                claim: CallerClaim {
                    instance: self.instance.clone(),
                    seat: seat.clone(),
                    binding_generation: binding.generation,
                    role: CallerRole::TopLevel,
                    harness: Harness::Claude,
                    native_session: binding.native_session.clone(),
                    execution: binding.execution.clone(),
                    target: binding.target.clone(),
                },
                ..request.clone()
            }
        } else {
            return report(ModAckOutcome::StaleGeneration, None);
        };
        // A channel in reconnect or rebind grace counts as live.
        if !self.mod_channels.is_live(&seat, binding.generation) {
            return report(ModAckOutcome::Retryable, Some(ModAckReason::NoLiveChannel));
        }
        let result = self.cooperative_mutation(
            PermitMutation::AckModDelivered(effective),
            peer,
            budget,
            read,
            false,
        )?;
        if let CommandResult::ModDeliveryAcked(report) = &result
            && report
                .results
                .iter()
                .any(|item| item.result.counts_as_mod_ack())
        {
            self.mod_channels
                .record_ack(&seat, binding.generation, self.clock.utc_now());
        }
        Ok(result)
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
    /// Spec D2: decide a mod watch registration against A2 in one read
    /// transaction (the seat, its target's recovery hold, its open binding),
    /// then record the channel with the binding's canonical generation. The
    /// claim only names the seat and the native session it expects; nothing
    /// the claim says about the generation is trusted. A store read that
    /// fails is `busy` (retryable): the CLI retries with backoff.
    fn watch_register(
        &self,
        request: &crate::protocol::watch::WatchRequest,
        sink: Arc<dyn crate::ports::ModChannelSink>,
        budget: &CallBudget,
    ) -> Result<(crate::ports::ModChannelId, u64), crate::protocol::watch::WatchRefusalReason> {
        use crate::protocol::{authority::Harness, watch::WatchRefusalReason as Refusal};
        let claim = &request.claim;
        if claim.harness != Harness::Claude || claim.role != CallerRole::TopLevel {
            return Err(Refusal::NotClaude);
        }
        let view = self
            .store
            .mod_seat_view(&claim.seat, budget)
            .map_err(|_| Refusal::Busy)?;
        let Some(view) = view else {
            return Err(Refusal::Unresolved);
        };
        if view.retired || !view.continuity_resolved {
            return Err(Refusal::Unresolved);
        }
        if view.held {
            return Err(Refusal::Held);
        }
        let Some(binding) = view.binding.as_ref() else {
            return Err(Refusal::NoBinding);
        };
        if binding.harness != Harness::Claude.as_str() {
            return Err(Refusal::NotClaude);
        }
        if binding.provenance != crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE {
            return Err(Refusal::NoBinding);
        }
        if binding.native_session != claim.native_session.as_str() {
            return Err(Refusal::SessionMismatch);
        }
        if claim.instance != self.instance {
            return Err(Refusal::SessionMismatch);
        }
        let channel = self.mod_channels.register(
            crate::ports::ModChannelRegistration {
                seat: claim.seat.clone(),
                binding_generation: binding.generation,
                native_session: claim.native_session.clone(),
                harness: Harness::Claude,
                registered_at: self.clock.utc_now(),
            },
            sink,
        )?;
        Ok((channel, view.attention_version))
    }

    fn watch_unregister(
        &self,
        channel: crate::ports::ModChannelId,
        now: crate::protocol::time::UtcMillis,
    ) {
        self.mod_channels.unregister(channel, now);
    }

    fn service_control(
        &self,
        _command: Command,
        _peer: PeerIdentity,
        _instance: &str,
        _boot: &str,
        _gate: &crate::service::live_gate::LiveServiceGate,
        _budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(error(
            ErrorCode::Unsupported,
            "service recovery control is unavailable",
        ))
    }

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
        let admission = self
            .cooperative_runtime
            .as_ref()
            .map(|(_, writer)| &**writer);
        self.store
            .service_operation(operation, connection, gate, budget, admission)
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
            | Command::PickerDirectory(_)
            | Command::Seats(_)
            | Command::SeatInspect(_)
            | Command::Inbox(_)
            | Command::InboxBatch(_)
            | Command::InboxBatchV2(_)
            | Command::MessageDeliveryModes(_)
            | Command::Warnings(_)
            | Command::ActiveWarnings(_)
            | Command::Thread(_)
            | Command::ResolveThread(_)
            | Command::ThreadName(_)
            | Command::History(_)
            | Command::Participants(_)
            | Command::ParticipantLocations(_)
            | Command::Recipients(_)
            | Command::DeliveryInspect(_)
            | Command::PendingReceipts(_)
            | Command::HotThreads(_)
            | Command::Message(_)
            | Command::Diagnostics(_)
            | Command::RetirementJobs(_) => self.store.query(&command, &read, budget),
            Command::AttentionDigest(_) | Command::AttentionDigestDelivery(_) => {
                let seat = match &command {
                    Command::AttentionDigest(q) | Command::AttentionDigestDelivery(q) => {
                        q.seat.clone()
                    }
                    _ => unreachable!("matched above"),
                };
                let mut result = self.store.query(&command, &read, budget)?;
                // Spec D7: any live or grace entry for the seat, which also
                // covers the rebind grace of the SessionStart check-in that
                // rotated the generation.
                let live = self.seat_has_mod_channel(&seat);
                match &mut result {
                    CommandResult::AttentionDigest(digest)
                    | CommandResult::AttentionDigestDelivery { digest, .. } => {
                        digest.mod_channel_live = live;
                    }
                    _ => {}
                }
                Ok(result)
            }
            Command::OperationStatus(_) => Err(error(
                ErrorCode::Unauthorized,
                "verified operation scope required",
            )),
            Command::AckModDelivered(request) => {
                self.ack_mod_delivered(request, peer, budget, read)
            }
            command @ (Command::BeginHandoff(_)
            | Command::CompleteHandoff(_)
            | Command::CheckIn(_)
            | Command::CreateThread(_)
            | Command::Invite(_)
            | Command::Join(_)
            | Command::Accept(_)
            | Command::AcceptRequired(_)
            | Command::Reject(_)
            | Command::SendMessage(_)
            | Command::Ack(_)
            | Command::AckDisplayed(_)
            | Command::CompleteInboxDelivery(_)
            | Command::Leave(_)
            | Command::SetTopic(_)
            | Command::SetThreadName(_)
            | Command::Archive(_)
            | Command::Reopen(_)) => {
                let mutation = PermitMutation::try_from(command).map_err(|_| {
                    error(
                        ErrorCode::InvalidRequest,
                        "command is not an accountable mutation",
                    )
                })?;
                self.cooperative_mutation(mutation, peer, budget, read, false)
            }
            Command::OperatorCheckIn(check) => {
                if check.claim.harness != crate::protocol::authority::Harness::Human
                    || !matches!(
                        check.mode,
                        crate::protocol::commands::CheckInMode::Lifecycle { .. }
                    )
                {
                    return Err(error(
                        ErrorCode::InvalidRequest,
                        "operator check-in is a human lifecycle check-in",
                    ));
                }
                self.cooperative_mutation(PermitMutation::CheckIn(check), peer, budget, read, true)
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
            // Daemon lifecycle check order (TRUST-POLICY A4, C1): (1) the A4
            // agent-to-human refusal, (2) this C1 reattachment on a held or
            // unowned target, (3) the existing hold refusal / ordinary path.
            // The hook sends this seatless command only for a top-level
            // resume, so (1) (human lifecycle check-ins) never reaches it. It
            // is sent even when the pane still looks resolved (ht-p63): the
            // fresh target read here detects a Herdr incarnation the daemon
            // has not reconciled, and a current resolved owner is refused.
            Command::ContinuityCheckIn(continuity) => {
                let (owner_uid, _) = self.cooperative_runtime.as_ref().ok_or_else(|| {
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
                let identity = self.current_target.as_ref().ok_or_else(|| {
                    error(
                        ErrorCode::Unsupported,
                        "current-target preparation unavailable",
                    )
                })?;
                identity.continuity(continuity, budget)
            }
            // Spec §4 summary protocol. The store decides entitlement against the
            // canonical view (A2); here only the elected owner's peer is admitted.
            // A declared subagent (a summary worker) may issue all three.
            command
            @ (Command::Summary(_) | Command::SummaryJob(_) | Command::SummarySubmit(_)) => {
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
                let _turn = writer.enter_foreground(budget, self.clock.as_ref())?;
                match command {
                    Command::Summary(request) => self
                        .store
                        .summary(&request, budget)
                        .map(CommandResult::Summary),
                    Command::SummaryJob(request) => self
                        .store
                        .summary_job(&request, budget)
                        .map(CommandResult::SummaryJob),
                    Command::SummarySubmit(request) => self
                        .store
                        .summary_submit(&request, budget)
                        .map(CommandResult::SummarySubmitted),
                    _ => unreachable!("matched a summary command"),
                }
            }
            // TRUST-POLICY A3 `managed_launch`: the launcher reports a
            // correlated startup; the store decides against its effective
            // observation in one transaction (A2) and opens nothing but an
            // unregistered binding on a seat with none.
            Command::RecordManagedLaunch(launch) => {
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
                let _turn = writer.enter_foreground(budget, self.clock.as_ref())?;
                self.store.record_managed_launch(launch, budget)
            }
            Command::LocalIntents(_) => Err(error(
                ErrorCode::Unsupported,
                "local intents are client-owned",
            )),
            Command::Health
            | Command::Capabilities
            | Command::HookParseFailure(_)
            | Command::HarnessEvidence(_)
            | Command::HarnessEvidenceV2(_)
            | Command::HarnessHealthV2
            | Command::HarnessStates
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
    ApiError::new(code, detail)
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

    #[test]
    fn summary_commands_reach_the_store() {
        use crate::protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            ids::{ExecutionId, LeaseToken, NativeSessionId, SeatId, SummaryJobId, ThreadId},
            summary::{SummaryJobRequest, SummaryOutcome, SummaryRequest, SummarySubmitRequest},
        };
        let path = std::env::temp_dir().join(format!("summary-route-{}.db", uuid::Uuid::new_v4()));
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), clock.clone()),
                "i",
                StoreSettings::default(),
            )
            .unwrap(),
        );
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch(
                "INSERT OR IGNORE INTO host_instances(id,created_at) VALUES ('i',0);\
                 INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);\
                 INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);",
            )
            .unwrap();
        }
        let domain = DomainService::new("i".into(), store, clock.clone())
            .with_cooperative_owner(501, Arc::new(FairWriter::new(8)));
        let claim = CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            binding_generation: 0,
            role: CallerRole::Subagent,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("e"),
            target: HostTargetId::new("p"),
        };
        let budget = || CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        let summary = |claim: CallerClaim| {
            Command::Summary(SummaryRequest {
                thread: ThreadId::new("t"),
                claim,
            })
        };
        // A declared subagent of a seat that may read the thread gets an answer.
        match domain
            .handle(
                summary(claim.clone()),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap()
        {
            CommandResult::Summary(SummaryOutcome::Ready(ready)) => assert_eq!(ready.frontier, 0),
            other => panic!("expected a Ready summary, got {other:?}"),
        }
        // A job and a submission for a job nobody leased reach the store and are
        // refused there (not found), not by the route.
        let job = Command::SummaryJob(SummaryJobRequest {
            job_id: SummaryJobId::new("j"),
            lease_token: LeaseToken::new("l"),
            claim: claim.clone(),
        });
        let submit = Command::SummarySubmit(SummarySubmitRequest {
            job_id: SummaryJobId::new("j"),
            lease_token: LeaseToken::new("l"),
            submission: serde_json::json!({}),
            claim: claim.clone(),
        });
        for command in [job, submit] {
            let err = domain
                .handle(command, PeerIdentity::from_kernel(501), &budget())
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound);
        }
        // The peer must be the elected owner, for every summary command.
        let err = domain
            .handle(
                summary(claim.clone()),
                PeerIdentity::from_kernel(502),
                &budget(),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::Unauthorized);
        // A claim for another instance is unverified.
        let mut foreign = claim;
        foreign.instance = "other".into();
        let err = domain
            .handle(summary(foreign), PeerIdentity::from_kernel(501), &budget())
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::CallerUnverified);
        // Without an elected runtime the route is closed.
        let bare = DomainService::new(
            "i".into(),
            Arc::new(
                SqliteStore::new(
                    StoreContext::new(path.clone(), clock.clone()),
                    "i",
                    StoreSettings::default(),
                )
                .unwrap(),
            ),
            clock.clone(),
        );
        let err = bare
            .handle(
                Command::Summary(SummaryRequest {
                    thread: ThreadId::new("t"),
                    claim: CallerClaim {
                        instance: "i".into(),
                        seat: SeatId::new("s"),
                        binding_generation: 0,
                        role: CallerRole::TopLevel,
                        harness: Harness::Codex,
                        native_session: NativeSessionId::new("n"),
                        execution: ExecutionId::new("e"),
                        target: HostTargetId::new("p"),
                    },
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::CallerUnverified);
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

#[cfg(test)]
#[path = "../../tests/service/mod_ack_dispatch.rs"]
mod mod_ack_dispatch_tests;

#[cfg(test)]
#[path = "../../tests/service/mod_digest_flag.rs"]
mod mod_digest_flag_tests;
