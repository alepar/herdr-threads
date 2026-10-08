use super::*;
use crate::{
    ports::*,
    protocol::{
        handoff::topology_contract_tests::{created, identity},
        handoff::*,
        ids::*,
        results::{ApiError, ErrorCode},
        time::{CallBudget, MonoInstant, UtcMillis},
    },
    store::{schema, topology_handoff},
};
use rusqlite::{Connection, params};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn canonical_fixture() -> (Connection, BootstrapIdentity, BootstrapAttachmentGuard) {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0)).unwrap();
    let mut id = identity();
    id.payload.handoff.channel = HandoffChannel::New {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
    };
    id.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
    id.digest = id.semantic_digest().unwrap();
    let boot = created().host_incarnation;
    db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,observation_sequence,observation_admission_sequence,observation_decided_sequence,lifecycle_revision,recovery_boot,recovery_epoch) VALUES('i',0,?1,1,1,1,1,1,?1,1)",[boot.as_str()]).unwrap();
    db.execute("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,created_at) VALUES('g','i',?1,1,1,'structural-incarnation',0,0,'published',0,0,0,0)",[boot.as_str()]).unwrap();
    db.execute_batch("UPDATE host_instances SET active_snapshot_id='g',recovery_baseline_generation_id='g'; INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('sender','i','resolved','native','w1:p1',1,0,0);").unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('sender',1,'w1:p1',?1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0)",[boot.as_str()]).unwrap();
    for (pane, terminal, generation) in [("w1:p1", "caller-terminal", 0), ("w1:p2", "terminal", 1)]
    {
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i',?1,?2,1,?3,2,'fresh',0,?4,'structural-incarnation','native_current_target',1)",params![pane,boot.as_str(),generation,terminal]).unwrap();
    }
    let observation = HostObservation {
        focused: false,
        target: HostTargetId::new("w1:p2"),
        host_boot: boot.clone(),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(1),
        observed_at_mono: MonoInstant(1),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Unknown,
        terminal: Some(TerminalId::new("terminal")),
        occupancy: StructuralOccupancy::Unknown,
        incarnation: IncarnationEvidence::Verified {
            identity: "structural-incarnation".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("fresh-structural-call"),
        connection_epoch: 1,
        observation_sequence: 2,
        started_at_mono: MonoInstant(1),
        completed_at_mono: MonoInstant(1),
    };
    let admission = HostObservationAdmission {
        instance: "i".into(),
        sequence: 1,
        expected_active: Some(SnapshotGenerationId::store_issued("g".into())),
        expected_boot: Some(boot),
        expected_epoch: 1,
        lifecycle_revision: 0,
        invalidation_revision: 0,
    };
    let guard = BootstrapAttachmentGuard::try_new(
        &crate::protocol::commands::ResolveSeat {
            target: observation.target.clone(),
            operation: id.payload.resolve_key.clone(),
        },
        BootstrapPaneObservation::try_new(
            observation,
            HostTargetId::new("w1"),
            HostTargetId::new("w1:t2"),
        )
        .unwrap(),
        &admission,
    )
    .unwrap();
    (db, id, guard)
}
struct Canonical {
    db: Mutex<Connection>,
    namespace: HandoffNamespace,
    guard: BootstrapAttachmentGuard,
    calls: AtomicUsize,
}
impl LocalClient for Canonical {
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let Command::RecoverBootstrap(request) = command else {
            panic!("recovery attempted downstream effect: {command:?}")
        };
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        let result = topology_handoff::attempts::recover(
            &tx,
            &self.namespace,
            &request,
            unsafe { libc::geteuid() },
            UtcMillis(3),
            Some(&self.guard),
        )?;
        tx.commit().unwrap();
        Ok(CommandResult::BootstrapRecovered(Box::new(result)))
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
}
struct Env {
    root: std::path::PathBuf,
    journal: Journal,
    reference: IntentRef,
    identity: BootstrapIdentity,
    canonical: Canonical,
}
impl Env {
    fn new() -> Self {
        Self::new_with_hint(false)
    }
    fn new_with_hint(hint: bool) -> Self {
        let root = std::env::temp_dir().join(format!("ht-human-topology-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let runtime = crate::daemon::paths::RuntimeContext::explicit(
            root.join("state dir"),
            root.join("host sock"),
            None,
        )
        .unwrap();
        let paths = crate::daemon::paths::InstancePaths::resolve(&runtime).unwrap();
        let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
        let (mut db, mut identity, guard) = canonical_fixture();
        identity.payload.handoff.namespace.state_dir = runtime.state_dir;
        identity.payload.handoff.namespace.host_endpoint = runtime.host_endpoint;
        identity.payload.cwd = root.clone();
        identity.digest = identity.semantic_digest().unwrap();
        let semantic = SemanticMutation::freeze(
            SemanticMutation::HandoffBootstrap(Box::new(super::super::journal::BootstrapPlan {
                version: 1,
                payload: identity.payload.clone(),
            })),
            identity.claim.clone(),
        )
        .unwrap();
        let reference = journal.record(identity.scope.clone(), semantic, 1).unwrap();
        let namespace = identity.payload.handoff.namespace.clone();
        let tx = db.transaction().unwrap();
        if hint {
            let keys = &identity.payload.handoff.keys;
            let child = HandoffIdentity {
                compound: identity.payload.handoff_key.clone(),
                digest: "b".repeat(64),
                claim: identity.claim.clone(),
                recipient: SeatId::new("peer"),
                thread: None,
                create_key: keys.create.clone(),
                invite_key: keys.invite.clone(),
                send_key: keys.send.clone(),
            };
            crate::store::handoff::import_hint(&tx, &child, None, "test-local-hint", UtcMillis(0))
                .unwrap();
        }
        topology_handoff::begin_pending(&tx, &namespace, &identity, UtcMillis(1)).unwrap();
        topology_handoff::attempts::reserve_attempt(
            &tx,
            &namespace,
            &ReserveBootstrapAttempt {
                identity: identity.clone(),
                operation: BootstrapAttempt::first()
                    .operation(&identity.compound, "reserve")
                    .unwrap(),
                expected_attempt: BootstrapAttempt::first(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        Self {
            root,
            journal,
            reference,
            identity,
            canonical: Canonical {
                db: Mutex::new(db),
                namespace,
                guard,
                calls: AtomicUsize::new(0),
            },
        }
    }
    fn request(&self, disposition: Assertion, attempt: u32) -> Request {
        Request {
            reference: LocalRecoveryRef::parse(self.reference.recovery_ref()).unwrap(),
            attempt: BootstrapAttempt::new(attempt).unwrap(),
            disposition,
        }
    }
    fn plan(
        &self,
        disposition: Assertion,
        attempt: u32,
        canonical: BootstrapRecoveryDisposition,
    ) -> RecoveryPlan {
        prepare(
            &self.journal,
            &self.request(disposition, attempt),
            canonical,
            unsafe { libc::geteuid() },
            &self.canonical.namespace,
        )
        .unwrap()
    }
    fn publish(&self, plan: &RecoveryPlan) -> IntentRef {
        publish(&self.journal, plan, &self.canonical.namespace, 2).unwrap()
    }
    fn retry<W: Write>(
        &self,
        reference: &IntentRef,
        writer: &mut W,
    ) -> Result<CommandResult, RunError> {
        super::super::retry::run_topology_recovery_retry_to_writer(
            &self.journal,
            reference,
            super::super::actor_route::InvocationActor::Human,
            &self.canonical.namespace,
            &self.canonical,
            &crate::app::SystemClock::new(),
            &OutputSpec::default(),
            writer,
        )
    }
    fn status(&self) -> BootstrapResult {
        let mut db = self.canonical.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        topology_handoff::current(&tx, &self.canonical.namespace, &self.identity)
            .unwrap()
            .unwrap()
    }
    fn noncreation(&self) -> RecoveryPlan {
        self.plan(
            Assertion::NotCreated,
            1,
            BootstrapRecoveryDisposition::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            },
        )
    }
    fn evidence(&self) -> CreatedTab {
        let mut evidence = created();
        evidence.witness.endpoint = self.canonical.namespace.host_endpoint.clone();
        evidence
    }
    fn created(&self) -> RecoveryPlan {
        self.plan(
            Assertion::CreatedPane(created().root_pane),
            1,
            BootstrapRecoveryDisposition::CreatedPane {
                evidence: self.evidence(),
                structural_reference: self.canonical.guard.ordinary().call_id().clone(),
            },
        )
    }
    fn cancel(&self) -> RecoveryPlan {
        self.plan(
            Assertion::Cancelled {
                reason: "inspected abandonment".into(),
            },
            1,
            BootstrapRecoveryDisposition::Cancelled {
                reason: "inspected abandonment".into(),
                quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
                child_guard: BootstrapCancellationGuard {
                    attached_child: None,
                },
            },
        )
    }
    fn snapshot(&self) -> Vec<(std::ffi::OsString, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(self.journal.root())
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    }
    fn reserve_second(&self) -> ReserveBootstrapResult {
        let mut db = self.canonical.db.lock().unwrap();
        let tx = db.transaction().unwrap();
        let attempt = BootstrapAttempt::new(2).unwrap();
        let result = topology_handoff::attempts::reserve_attempt(
            &tx,
            &self.canonical.namespace,
            &ReserveBootstrapAttempt {
                identity: self.identity.clone(),
                operation: attempt
                    .operation(&self.identity.compound, "reserve")
                    .unwrap(),
                expected_attempt: attempt,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        result
    }
}
impl Drop for Env {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
struct LostOutput;
impl Write for LostOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("lost presentation"))
    }
}

#[test]
fn separate_operator_scope_keeps_original_claim_bytes_and_ref_immutable() {
    let env = Env::new();
    let original = env.journal.load(&env.reference).unwrap();
    let plan = env.noncreation();
    let reference = env.publish(&plan);
    let pending = env.journal.load(&reference).unwrap();
    assert_eq!(
        pending.header.kind,
        crate::protocol::results::IntentKind::OperatorRecoverBootstrap
    );
    assert!(pending.semantic.frozen_claim().is_none());
    assert_eq!(
        super::super::journal::classify_original_actor(&pending.header.scope, &pending.semantic)
            .unwrap(),
        OriginalActor::HumanOrOperator
    );
    assert_eq!(
        env.journal.load(&env.reference).unwrap().semantic,
        original.semantic
    );
    assert_eq!(plan.request.identity.claim, env.identity.claim);
    assert_eq!(plan.original_ref, env.reference);
    let mut other = plan.clone();
    other.operator_uid = other.operator_uid.wrapping_add(1);
    assert!(publish(&env.journal, &other, &env.canonical.namespace, 2).is_err());
    let foreign_uid_ref = env
        .journal
        .record(
            IntentScope::Operator {
                instance: env.canonical.namespace.instance.clone(),
                local_user_uid: other.operator_uid,
            },
            SemanticMutation::OperatorRecoverBootstrap(Box::new(other.clone())),
            3,
        )
        .unwrap();
    let error = env.retry(&foreign_uid_ref, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(error,RunError::Api(ref e) if e.detail.contains("different local account")),
        "{error:?}"
    );
    assert!(env.journal.load(&foreign_uid_ref).is_ok());
    assert_eq!(env.canonical.calls.load(Ordering::Relaxed), 0);
    assert_ne!(
        env.journal
            .load(&foreign_uid_ref)
            .unwrap()
            .header
            .semantic_digest,
        pending.header.semantic_digest
    );

    assert_ne!(
        prepare(
            &env.journal,
            &env.request(Assertion::NotCreated, 2),
            plan.request.disposition.clone(),
            plan.operator_uid,
            &env.canonical.namespace
        )
        .unwrap()
        .request
        .operation,
        plan.request.operation
    );
    let mut foreign = env.canonical.namespace.clone();
    foreign.host_endpoint = "/foreign.sock".into();
    assert!(
        prepare(
            &env.journal,
            &env.request(Assertion::NotCreated, 1),
            plan.request.disposition.clone(),
            plan.operator_uid,
            &foreign
        )
        .is_err()
    );
    let mut human = plan.clone();
    human.request.identity.claim.harness = crate::protocol::authority::Harness::Human;
    human.request.identity.digest = human.request.identity.semantic_digest().unwrap();
    human.request.operation = human.request.decision_operation().unwrap();
    assert!(human.validate().is_err());
}

#[test]
fn root_operator_retry_refuses_before_completed_presentation_and_cleanup() {
    let env = Env::new();
    let reference = env.publish(&env.noncreation());
    std::fs::write(
        env.journal
            .root()
            .join(format!("handoff-{}.progress", reference.operation.as_str())),
        br#"{"completed":true,"launch":{"outcome":"started"}}"#,
    )
    .unwrap();
    let before = env.snapshot();
    let ns = &env.canonical.namespace;
    let args = vec![
        "ht".to_owned(),
        "--state-dir".into(),
        ns.state_dir.display().to_string(),
        "--host-endpoint".into(),
        ns.host_endpoint.display().to_string(),
        "retry".into(),
        reference.recovery_ref(),
    ];
    let mut output = Vec::new();
    let error = super::super::run_in_pane(args.clone(), None, &mut output).unwrap_err();
    assert!(
        matches!(error,RunError::Io(ref e) if e.kind()==io::ErrorKind::PermissionDenied),
        "{error:?}"
    );
    assert!(output.is_empty());
    assert_eq!(before, env.snapshot());
    assert_eq!(env.canonical.calls.load(Ordering::Relaxed), 0);
    let mut args = args;
    args.insert(1, "human".into());
    let error = super::super::run_in_pane(args, None, &mut output).unwrap_err();
    assert!(
        matches!(error,RunError::Api(ref e) if e.code==ErrorCode::Unsupported),
        "{error:?}"
    );
    assert_eq!(before, env.snapshot());
}

#[test]
fn normal_operation_lock_refuses_known_inflight_without_call_or_cleanup() {
    let env = Env::new();
    let plan = env.noncreation();
    let reference = env.publish(&plan);
    let _lock = super::super::handoff::lock(&env.journal, &env.reference).unwrap();
    let before = env.snapshot();
    assert!(publish(&env.journal, &plan, &env.canonical.namespace, 3).is_err());
    assert!(env.retry(&reference, &mut Vec::new()).is_err());
    assert_eq!(env.canonical.calls.load(Ordering::Relaxed), 0);
    assert_eq!(before, env.snapshot());
}

#[test]
fn lost_noncreation_output_replays_old_decision_after_attempt_two_without_new_permission() {
    let env = Env::new();
    let reference = env.publish(&env.noncreation());
    assert!(env.retry(&reference, &mut LostOutput).is_err());
    assert!(env.journal.load(&reference).is_ok());
    assert_eq!(env.status().attempt.get(), 2);
    assert_eq!(env.status().attempt_state, BootstrapAttemptState::Prepared);
    assert!(matches!(
        env.reserve_second(),
        ReserveBootstrapResult::Authorized { .. }
    ));
    let CommandResult::BootstrapRecovered(result) = env.retry(&reference, &mut Vec::new()).unwrap()
    else {
        panic!("not recovery")
    };
    assert_eq!(result.attempt.get(), 1);
    assert_eq!(result.state, BootstrapState::Prepared);
    assert_eq!(env.status().attempt.get(), 2);
    assert_eq!(
        env.status().attempt_state,
        BootstrapAttemptState::PossibleCreation
    );
    assert!(matches!(
        env.reserve_second(),
        ReserveBootstrapResult::Replay { .. }
    ));
    assert!(env.journal.load(&env.reference).is_ok());
    assert!(env.journal.load(&reference).is_err());
    let stale = env.publish(&env.cancel());
    assert!(env.retry(&stale, &mut Vec::new()).is_err());
    let late = env.publish(&env.created());
    assert!(env.retry(&late, &mut Vec::new()).is_err());
    assert!(env.status().creation.is_none());
    let mut db = env.canonical.db.lock().unwrap();
    let tx = db.transaction().unwrap();
    let late = RecordBootstrapCreated {
        identity: env.identity.clone(),
        operation: BootstrapAttempt::first()
            .operation(&env.identity.compound, "record")
            .unwrap(),
        expected_attempt: BootstrapAttempt::first(),
        evidence: env.evidence(),
    };
    assert!(
        topology_handoff::attempts::record_created(&tx, &env.canonical.namespace, &late).is_err()
    );
}

#[test]
fn created_pane_uses_actual_canonical_structural_and_ordinary_guards_without_allocation() {
    for sql in [
        None,
        Some("UPDATE host_instances SET observation_admission_sequence=2"),
        Some("UPDATE observed_targets SET terminal_id='changed' WHERE target_id='w1:p2'"),
        Some(
            "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) SELECT 'i','w1:p2',host_boot,1,'held' FROM host_instances",
        ),
    ] {
        let env = Env::new();
        let reference = env.publish(&env.created());
        if let Some(sql) = sql {
            env.canonical.db.lock().unwrap().execute_batch(sql).unwrap();
        }
        let result = env.retry(&reference, &mut Vec::new());
        if sql.is_some() {
            assert!(result.is_err(), "{sql:?}");
            assert_eq!(env.status().state, BootstrapState::PossibleCreation);
        } else {
            assert!(result.is_ok(), "{result:?}");
            assert_eq!(env.status().creation, Some(env.evidence()));
            assert_eq!(env.status().state, BootstrapState::Created);
        }
        let db = env.canonical.db.lock().unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn cancellation_is_absorbing_and_live_legacy_hint_refuses_release() {
    let env = Env::new();
    let reference = env.publish(&env.cancel());
    env.retry(&reference, &mut Vec::new()).unwrap();
    assert_eq!(env.status().state, BootstrapState::Cancelled);
    let reference = env.publish(&env.noncreation());
    assert!(env.retry(&reference, &mut Vec::new()).is_err());
    let env = Env::new_with_hint(true);
    let reference = env.publish(&env.cancel());
    let error = env.retry(&reference, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(error,RunError::Api(ref e) if e.detail.contains("live legacy child")),
        "{error:?}"
    );
    assert_eq!(env.status().state, BootstrapState::PossibleCreation);
}

#[test]
fn ready_argv_pins_routes_ref_attempt_and_quotes_reason_with_real_parser() {
    let env = Env::new();
    let prefix = vec![
        "ht".into(),
        "--state-dir".into(),
        env.canonical.namespace.state_dir.display().to_string(),
        "--host-endpoint".into(),
        env.canonical.namespace.host_endpoint.display().to_string(),
        "--json".into(),
    ];
    for assertion in [
        Assertion::NotCreated,
        Assertion::CreatedPane(HostTargetId::new("w1:p2")),
        Assertion::Cancelled {
            reason: "inspected 'quoted' $HOME reason".into(),
        },
    ] {
        let request = env.request(assertion, 7);
        let argv = recovery_argv(&prefix, &request);
        assert_eq!(argv[1], "human");
        let rendered = argv
            .iter()
            .map(|s| shlex::try_quote(s).unwrap().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let reparsed =
            super::super::commands::parse_argv(shlex::split(&rendered).unwrap()).unwrap();
        assert_eq!(
            reparsed.action,
            super::super::commands::CliAction::TopologyRecover(request)
        );
        assert_eq!(
            reparsed.output.context.state_dir.as_deref(),
            env.canonical.namespace.state_dir.to_str()
        );
    }
}

#[test]
fn conflicting_concurrent_assertions_never_authorize_both_dispositions() {
    let env = Env::new();
    let noncreation = env.publish(&env.noncreation());
    let cancel = env.publish(&env.cancel());
    let barrier = std::sync::Barrier::new(2);
    let (left, right) = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            barrier.wait();
            env.retry(&noncreation, &mut Vec::new())
        });
        let right = scope.spawn(|| {
            barrier.wait();
            env.retry(&cancel, &mut Vec::new())
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    assert_ne!(
        left.is_ok(),
        right.is_ok(),
        "one exact decision must win: {left:?} / {right:?}"
    );
    assert_eq!(
        env.canonical
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM bootstrap_recovery_decisions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn wrong_original_kind_harness_or_exact_scope_never_becomes_recovery() {
    let env = Env::new();
    let request = env.request(Assertion::NotCreated, 1);
    let disposition = env.noncreation().request.disposition;
    for (scope, semantic) in [
        (
            IntentScope::ServiceAllocation {
                instance: "i".into(),
                target: HostTargetId::new("w1:p2"),
            },
            SemanticMutation::ResolveSeat {
                target: HostTargetId::new("w1:p2"),
            },
        ),
        (
            env.identity.scope.clone(),
            SemanticMutation::freeze(
                SemanticMutation::HandoffDelivery(Box::new(super::super::journal::DeliveryPlan {
                    version: 1,
                    payload: env.identity.payload.handoff.clone(),
                    recipient: SeatId::new("peer"),
                })),
                env.identity.claim.clone(),
            )
            .unwrap(),
        ),
    ] {
        let reference = env.journal.record(scope, semantic, 3).unwrap();
        let mut request = request.clone();
        request.reference = LocalRecoveryRef::parse(reference.recovery_ref()).unwrap();
        assert!(
            prepare(
                &env.journal,
                &request,
                disposition.clone(),
                unsafe { libc::geteuid() },
                &env.canonical.namespace
            )
            .is_err()
        );
    }
    let plan = env.noncreation();
    let semantic = SemanticMutation::OperatorRecoverBootstrap(Box::new(plan));
    assert!(
        super::super::journal::classify_original_actor(&env.identity.scope, &semantic).is_err()
    );
    assert!(
        super::super::journal::classify_original_actor(
            &IntentScope::Operator {
                instance: "foreign".into(),
                local_user_uid: unsafe { libc::geteuid() }
            },
            &semantic
        )
        .is_err()
    );
    assert!(SemanticMutation::freeze(semantic, env.identity.claim.clone()).is_err());
}

#[derive(Clone, Copy, Debug)]
enum OriginalDamage {
    Missing,
    Ambiguous,
    Malformed,
    ConflictingTerminal,
    Changed,
}
#[derive(Clone, Copy, Debug)]
enum RecoveryEntry {
    Publish,
    RetryWrapper,
    Consumer,
}

fn invalid_retained_original_is_read_only(damage: OriginalDamage, entry: RecoveryEntry) {
    let env = Env::new();
    let plan = env.noncreation();
    plan.validate().unwrap();
    // Publish the valid operator payload directly for retry setup, without the
    // operation lock whose creation this regression exercises.
    let operator_ref = env
        .journal
        .record(
            IntentScope::Operator {
                instance: env.canonical.namespace.instance.clone(),
                local_user_uid: plan.operator_uid,
            },
            SemanticMutation::OperatorRecoverBootstrap(Box::new(plan.clone())),
            2,
        )
        .unwrap();
    let original_path = env.journal.root().join(format!(
        "{:020}-{}.intent",
        env.reference.ordinal,
        env.reference.operation.as_str()
    ));
    let original_lock = env
        .journal
        .root()
        .join(format!("handoff-{}.lock", env.reference.operation.as_str()));
    assert!(!original_lock.exists());
    match damage {
        OriginalDamage::Missing => std::fs::remove_file(&original_path).unwrap(),
        OriginalDamage::Ambiguous => {
            let duplicate = env.journal.root().join(format!(
                "{:020}-{}.intent",
                env.reference.ordinal,
                uuid::Uuid::new_v4()
            ));
            std::fs::copy(&original_path, duplicate).unwrap();
        }
        OriginalDamage::Malformed => std::fs::write(
            &original_path,
            b"malformed original header\nmalformed semantic",
        )
        .unwrap(),
        OriginalDamage::ConflictingTerminal => {
            let conflict = env.journal.root().join(format!(
                "delivery-{:020}-{}.terminal",
                env.reference.ordinal,
                env.reference.operation.as_str()
            ));
            std::fs::write(conflict, b"{}").unwrap();
        }
        OriginalDamage::Changed => {
            use sha2::{Digest, Sha256};
            let pending = env.journal.load(&env.reference).unwrap();
            let SemanticMutation::Frozen { claim, mutation } = pending.semantic else {
                panic!("fixture original must be frozen")
            };
            let SemanticMutation::HandoffBootstrap(mut bootstrap) = *mutation else {
                panic!("fixture original must be bootstrap")
            };
            bootstrap.payload.handoff.body =
                "changed original after operator plan was frozen".into();
            let semantic =
                SemanticMutation::freeze(SemanticMutation::HandoffBootstrap(bootstrap), claim)
                    .unwrap();
            let mut header = pending.header;
            header.semantic_digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&semantic).unwrap())
            );
            std::fs::write(
                &original_path,
                format!(
                    "{}\n{}",
                    serde_json::to_string(&header).unwrap(),
                    serde_json::to_string(&semantic).unwrap()
                ),
            )
            .unwrap();
            let changed = env
                .journal
                .load(&env.reference)
                .expect("changed original remains valid, unique and self-consistent");
            assert_ne!(changed.header.semantic_digest, plan.request.identity.digest);
        }
    }
    let before = env.snapshot();
    let canonical_before = env.status();
    let mut output = Vec::new();
    let refused = match entry {
        RecoveryEntry::Publish => {
            publish(&env.journal, &plan, &env.canonical.namespace, 3).is_err()
        }
        RecoveryEntry::RetryWrapper => env.retry(&operator_ref, &mut output).is_err(),
        RecoveryEntry::Consumer => retry_to_writer(
            &env.journal,
            &operator_ref,
            &env.canonical.namespace,
            &env.canonical,
            &crate::app::SystemClock::new(),
            &OutputSpec::default(),
            &mut output,
        )
        .is_err(),
    };
    assert!(
        refused,
        "{entry:?}/{damage:?} must refuse invalid retained original"
    );
    assert!(output.is_empty());
    assert_eq!(env.canonical.calls.load(Ordering::Relaxed), 0);
    assert_eq!(env.status(), canonical_before);
    assert!(
        !original_lock.exists(),
        "{entry:?}/{damage:?} refusal created original operation lock"
    );
    assert_eq!(
        env.snapshot(),
        before,
        "{entry:?}/{damage:?} refusal mutated local intent/state"
    );
    assert!(
        env.journal.load(&operator_ref).is_ok(),
        "refusal cleaned the operator decision"
    );
}

macro_rules! retained_original_refusal_tests {
    ($($name:ident:$damage:ident,$entry:ident;)*) => {$ (
        #[test]
        fn $name() {invalid_retained_original_is_read_only(OriginalDamage::$damage,RecoveryEntry::$entry);}
    )*};
}
retained_original_refusal_tests! {
    invalid_retained_original_missing_publication:Missing,Publish;
    invalid_retained_original_ambiguous_publication:Ambiguous,Publish;
    invalid_retained_original_malformed_publication:Malformed,Publish;
    invalid_retained_original_conflicting_publication:ConflictingTerminal,Publish;
    invalid_retained_original_changed_publication:Changed,Publish;
    invalid_retained_original_missing_retry_wrapper:Missing,RetryWrapper;
    invalid_retained_original_ambiguous_retry_wrapper:Ambiguous,RetryWrapper;
    invalid_retained_original_malformed_retry_wrapper:Malformed,RetryWrapper;
    invalid_retained_original_conflicting_retry_wrapper:ConflictingTerminal,RetryWrapper;
    invalid_retained_original_changed_retry_wrapper:Changed,RetryWrapper;
    invalid_retained_original_missing_consumer:Missing,Consumer;
    invalid_retained_original_ambiguous_consumer:Ambiguous,Consumer;
    invalid_retained_original_malformed_consumer:Malformed,Consumer;
    invalid_retained_original_conflicting_consumer:ConflictingTerminal,Consumer;
    invalid_retained_original_changed_consumer:Changed,Consumer;
}
