// Included inside native::tests so the socket server owns and joins every exchange.
mod bootstrap_recovery {
    use super::*;
    use crate::{
        ports::{BootstrapPaneObservation, DurableWorkAdmission, SnapshotHeader, StorePort},
        protocol::{commands::ResolveSeat, handoff::*, ids::ExecutionId},
        store::{SqliteStore, StoreSettings, connection::StoreContext, schema, topology_handoff},
    };
    use rusqlite::Connection;

    fn scoped_read(
        cli: &NativeCli,
        context: &HostCallContext,
    ) -> (BootstrapPaneObservation, LocalEndpointWitness) {
        use crate::ports::BootstrapObserver;
        let pane = cli
            .observe_bootstrap_target(&HostTargetId::new("w4:p1"), context)
            .unwrap();
        let witness = pane.witness().clone();
        (pane, witness)
    }

    fn snapshot_exchange() -> Exchange {
        Box::new(|stream, request| {
            assert_eq!(request["method"], "session.snapshot");
            answer(
                stream,
                &request,
                json!({"type":"session_snapshot","snapshot":{
                    "version":"0.9.1","protocol":22,"agents":[],"layouts":[],
                    "workspaces":[{"workspace_id":"w4","label":"Space"}],
                    "tabs":[{"tab_id":"w4:t1","label":"Tab"}],
                    "panes":[{"pane_id":"w4:p1","terminal_id":"term_1","workspace_id":"w4","tab_id":"w4:t1","focused":false,"agent_status":"idle","revision":1,"label":"Pane"}]
                }}),
            );
        })
    }

    struct ReadControl<'a> {
        cli: &'a NativeCli,
        cancel: bool,
    }
    impl crate::ports::BootstrapObserver for ReadControl<'_> {
        fn observe_bootstrap_target(
            &self,
            target: &HostTargetId,
            context: &HostCallContext,
        ) -> Result<BootstrapPaneObservation, ApiError> {
            let result = crate::ports::BootstrapObserver::observe_bootstrap_target(
                self.cli, target, context,
            )?;
            if self.cancel {
                context.budget.cancellation.cancel();
            }
            Ok(result)
        }
    }
    struct DatabaseFiles(PathBuf);
    impl Drop for DatabaseFiles {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }

    #[test]
    fn phase13_actual_native_distinct_reads_recover_without_relabeling() {
        native_recovery_case("valid");
    }

    #[test]
    fn phase13_actual_native_recovery_rejects_witness_and_canonical_changes() {
        for change in [
            "endpoint",
            "device",
            "inode",
            "birth",
            "birth_seconds",
            "metadata",
            "change_seconds",
            "live_metadata",
            "schema",
            "platform",
            "start_microseconds",
            "pid",
            "uid",
            "start",
            "boot",
            "tab",
            "terminal",
            "admission",
            "lifecycle",
            "owner",
            "current_terminal",
            "current_incarnation",
            "hold",
            "invalidation",
            "epoch",
            "namespace",
            "cancel",
        ] {
            native_recovery_case(change);
        }
    }

    fn native_recovery_case(change: &str) {
        let (socket, cli, worker) =
            serve_sequence(vec![pane_exchange(), snapshot_exchange(), pane_exchange()]);
        let context = HostCallContext {
            budget: CallBudget {
                deadline: MonoInstant(10000),
                cancellation: Default::default(),
            },
            expected_boot: None,
            expected_epoch: Some(3),
        };
        let (historical, witness) = scoped_read(&cli, &context);
        let historic_reference = historical.observation().call_id.clone();
        let observation = historical.observation();
        let evidence = crate::ports::CreatedTab {
            correlation: HostCallId::new("operator-inspected-creation"),
            workspace: historical.workspace().clone(),
            tab: historical.tab().clone(),
            root_pane: observation.target.clone(),
            terminal: observation.terminal.clone().unwrap(),
            host_incarnation: observation.host_boot.clone(),
            witness,
        };
        let path = std::env::temp_dir().join(format!("ht-bootstrap-{}.db", uuid::Uuid::new_v4()));
        let _files = DatabaseFiles(path.clone());
        let mut db = Connection::open(&path).unwrap();
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at) VALUES('i',0)",
            [],
        )
        .unwrap();
        let store = SqliteStore::new(
            StoreContext::new(path.clone(), cli.clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap();
        let admission = store.begin_host_observation("i", &context.budget).unwrap();
        let snapshot = cli.enumerate_targets(&context).unwrap();
        let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
        let stage = store.begin_snapshot_stage(header, &context.budget).unwrap();
        store
            .stage_snapshot_targets(
                &stage.id,
                0,
                &snapshot.targets,
                DurableWorkAdmission::new(16).unwrap(),
                &context.budget,
            )
            .unwrap();
        store
            .seal_snapshot_stage(&stage.id, &context.budget)
            .unwrap();
        store
            .publish_snapshot_stage(&stage.id, &context.budget)
            .unwrap();
        let mut id = crate::protocol::handoff::topology_contract_tests::identity();
        id.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000001");
        id.claim.target = HostTargetId::new("w4:p1");
        id.payload.workspace = HostTargetId::new("w4");
        id.payload.handoff.namespace.host_endpoint = socket.clone();
        id.payload.handoff.channel = HandoffChannel::New {
            name: None,
            topic: "topic".into(),
            goal: "goal".into(),
        };
        id.digest = id.semantic_digest().unwrap();
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,structural_incarnation_kind,structural_connection_epoch,structural_observation_sequence,created_at) VALUES('sender','i','resolved','native','w4:p1',1,1,'term_1',?1,?1,3,'native_current_target',3,1,0)", [evidence.host_incarnation.as_str()]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('sender',1,'w4:p1',?1,3,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,0,'term_1',?1)", [evidence.host_incarnation.as_str()]).unwrap();
        {
            let tx = db.transaction().unwrap();
            topology_handoff::begin_pending(&tx, &id.payload.handoff.namespace, &id, UtcMillis(0))
                .unwrap();
            topology_handoff::attempts::reserve_attempt(
                &tx,
                &id.payload.handoff.namespace,
                &ReserveBootstrapAttempt {
                    identity: id.clone(),
                    expected_attempt: BootstrapAttempt::first(),
                    operation: BootstrapAttempt::first()
                        .operation(&id.compound, "reserve")
                        .unwrap(),
                },
            )
            .unwrap();
            tx.commit().unwrap();
        }
        let cli = Arc::new(cli);
        let store = Arc::new(store);
        let identity = crate::identity::repair::OrdinaryIdentity::new(
            "i".into(),
            store.clone(),
            cli.clone(),
            cli.clock.clone(),
            Arc::new(crate::service::fair_writer::FairWriter::new(8)),
        );
        let mut evidence = evidence;
        match change {
            "endpoint" => evidence.witness.endpoint = "/different.sock".into(),
            "device" => evidence.witness.socket.device += 1,
            "inode" => evidence.witness.socket.inode += 1,
            "birth" => evidence.witness.socket.birth_nanoseconds += 1,
            "birth_seconds" => evidence.witness.socket.birth_seconds += 1,
            "change_seconds" => evidence.witness.socket.change_seconds += 1,
            "start_microseconds" => {
                evidence.witness.start_microseconds =
                    (evidence.witness.start_microseconds + 1) % 1_000_000
            }
            "schema" => evidence.witness.schema = 2,
            "platform" => evidence.witness.platform = "unsupported".into(),
            "metadata" => evidence.witness.socket.change_nanoseconds += 1,
            "pid" => evidence.witness.peer_pid += 1,
            "uid" => evidence.witness.peer_uid += 1,
            "start" => evidence.witness.start_seconds += 1,
            "boot" => evidence.host_incarnation = HostBootId::new("different-boot"),
            "tab" => evidence.tab = HostTargetId::new("w4:other-tab"),
            "terminal" => evidence.terminal = TerminalId::new("different-terminal"),
            _ => {}
        }
        if matches!(change, "pid" | "uid" | "start" | "start_microseconds") {
            evidence.host_incarnation = ServerIncarnation::from_witness(&evidence.witness)
                .unwrap()
                .boot;
        }
        let canonical_before: Vec<u8> = db
            .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                r.get(0)
            })
            .unwrap();
        let mut request = RecoverBootstrap {
            identity: id.clone(),
            expected_attempt: BootstrapAttempt::first(),
            operation: id.compound.clone(),
            disposition: BootstrapRecoveryDisposition::CreatedPane {
                evidence,
                structural_reference: historic_reference,
            },
        };
        request.operation = request.decision_operation().unwrap();
        let frozen = serde_json::to_vec(&request).unwrap();
        if change == "live_metadata" {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&socket).unwrap().permissions().mode();
            fs::set_permissions(&socket, fs::Permissions::from_mode(mode ^ 0o020)).unwrap();
        }
        let mut fresh_reference = None;
        let result = identity.with_bootstrap_observation(
            &ReadControl { cli: cli.as_ref(), cancel: change == "cancel" },
            &ResolveSeat {
                target: HostTargetId::new("w4:p1"),
                operation: id.payload.resolve_key.clone(),
            },
            &context.budget,
            |guard| {
                fresh_reference = Some(guard.ordinary().call_id().clone());
                if change == "live_metadata" {
                    let BootstrapRecoveryDisposition::CreatedPane { evidence, .. } = &request.disposition else { unreachable!() };
                    assert_eq!(guard.witness().peer_pid, evidence.witness.peer_pid);
                    assert_ne!(guard.witness().socket, evidence.witness.socket, "actual owned socket metadata changed between reads");
                }
                if let BootstrapRecoveryDisposition::CreatedPane {
                    structural_reference,
                    ..
                } = &request.disposition
                {
                    assert_ne!(
                        structural_reference,
                        guard.ordinary().call_id(),
                        "actual native calls generate distinct IDs"
                    );
                }
                let sql = match change {
                    "admission" => "UPDATE host_instances SET observation_admission_sequence=observation_admission_sequence+1",
                    "lifecycle" => "UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1",
                    "owner" => "UPDATE seats SET structural_terminal_id='changed' WHERE id='sender'",
                    "current_terminal" => "UPDATE observed_targets SET terminal_id='changed'",
                    "current_incarnation" => "UPDATE observed_targets SET incarnation='changed'",
                    "hold" => "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) SELECT 'i','w4:p1',host_boot,3,'held' FROM host_instances",
                    "invalidation" => "UPDATE host_instances SET invalidation_revision=invalidation_revision+1",
                    "epoch" => "UPDATE host_instances SET host_epoch=host_epoch+1",
                    _ => "SELECT 1",
                };
                db.execute_batch(sql).unwrap();
                let tx = db.transaction().unwrap();
                let mut ns = id.payload.handoff.namespace.clone();
                if change == "namespace" { ns.state_dir = "/other-state".into(); }
                let result = topology_handoff::attempts::recover(
                    &tx,
                    &ns,
                    &request,
                    501,
                    UtcMillis(1),
                    Some(&guard),
                );
                tx.commit().unwrap();
                result
            },
        );
        // Always join the socket server before asserting the intended RED/GREEN result.
        worker.join().unwrap();
        fs::remove_file(&socket).unwrap();
        assert_eq!(serde_json::to_vec(&request).unwrap(), frozen);
        if change == "valid" {
            assert!(
                result.is_ok(),
                "independent actual native deciding observation refused: {result:?}"
            );
            assert!(fresh_reference.unwrap().as_str().starts_with("cli-"));
            let tx = db.transaction().unwrap();
            let saved: Vec<u8> = tx
                .query_row(
                    "SELECT result_json FROM bootstrap_recovery_decisions",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            tx.execute_batch("UPDATE host_instances SET lifecycle_revision=lifecycle_revision+7; DELETE FROM observed_targets").unwrap();
            assert_eq!(
                topology_handoff::attempts::recover(
                    &tx,
                    &id.payload.handoff.namespace,
                    &request,
                    501,
                    UtcMillis(2),
                    None
                )
                .unwrap(),
                result.unwrap()
            );
            assert_eq!(
                tx.query_row::<Vec<u8>, _, _>(
                    "SELECT result_json FROM bootstrap_recovery_decisions",
                    [],
                    |r| r.get(0)
                )
                .unwrap(),
                saved
            );
            drop(tx);
        } else {
            assert!(result.is_err(), "{change} accepted: {result:?}");
            assert_eq!(
                db.query_row::<i64, _, _>(
                    "SELECT count(*) FROM bootstrap_recovery_decisions",
                    [],
                    |r| r.get(0)
                )
                .unwrap(),
                0,
                "{change}"
            );
            assert_eq!(
                db.query_row::<Vec<u8>, _, _>(
                    "SELECT identity_json FROM bootstrap_handoffs",
                    [],
                    |r| r.get(0)
                )
                .unwrap(),
                canonical_before,
                "{change}"
            );
        }
        assert_eq!(
            db.query_row::<i64, _, _>("SELECT count(*) FROM allocation_decisions", [], |r| r
                .get(0))
                .unwrap(),
            0
        );
        drop(identity);
        drop(store);
        drop(db);
    }
}
