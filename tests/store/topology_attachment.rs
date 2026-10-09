use super::{created, fixture, identity, namespace, saved_recovery};
use herdr_threads::{
    protocol::{handoff::*, ids::*, time::UtcMillis},
    store::{handoff, topology_handoff},
};

fn child(id: &BootstrapIdentity) -> HandoffIdentity {
    HandoffIdentity {
        compound: id.payload.handoff_key.clone(),
        digest: "b".repeat(64),
        claim: id.claim.clone(),
        thread: id.payload.handoff.channel.thread().cloned(),
        recipient: SeatId::new("peer"),
        create_key: id.payload.handoff.keys.create.clone(),
        invite_key: id.payload.handoff.keys.invite.clone(),
        send_key: id.payload.handoff.keys.send.clone(),
    }
}
fn attached_fixture(tx: &rusqlite::Transaction<'_>, id: &BootstrapIdentity) -> BootstrapAttachment {
    topology_handoff::begin_pending(tx, &namespace(), id, UtcMillis(0)).unwrap();
    let a = BootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: created(),
        resolve_operation: id.payload.resolve_key.clone(),
        resolved_seat: SeatId::new("peer"),
        handoff: child(id),
    };
    tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1 AND attempt=1",[serde_json::to_vec(&a.created).unwrap()]).unwrap();
    tx.execute(
        "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
        [serde_json::to_vec(&a).unwrap()],
    )
    .unwrap();
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='attached' WHERE id=1",
        [],
    )
    .unwrap();
    a
}
fn linked(tx: &rusqlite::Transaction<'_>, id: &BootstrapIdentity) -> BootstrapAttachment {
    let a = attached_fixture(tx, id);
    handoff::begin_linked_pending(tx, &namespace(), &a.handoff, UtcMillis(1)).unwrap();
    a
}

#[test]
fn linked_live_child_refuses_unwrapped_completion_without_changing_either_fence() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let id = identity();
    let a = linked(&tx, &id);
    assert!(
        handoff::complete_pending(&tx, &a.handoff, UtcMillis(2)).is_err(),
        "linked child requires atomic report-backed wrapper"
    );
    assert_eq!(
        handoff::current(&tx, &a.handoff).unwrap().unwrap().state,
        HandoffState::Live
    );
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .state,
        BootstrapState::Attached
    );
}

#[test]
fn deciding_create_attaches_exact_new_thread_to_both_fences() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let mut id = identity();
    id.payload.handoff.channel = HandoffChannel::New {
        name: None,
        topic: "new".into(),
        goal: "goal".into(),
    };
    id.digest = id.semantic_digest().unwrap();
    let a = linked(&tx, &id);
    topology_handoff::attach_created(
        &tx,
        &namespace(),
        "i",
        "seat:s",
        "unrelated",
        &ThreadId::new("t"),
    )
    .unwrap();
    assert_eq!(
        tx.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    assert!(
        topology_handoff::attach_created(
            &tx,
            &namespace(),
            "foreign",
            "seat:s",
            "create",
            &ThreadId::new("t")
        )
        .is_err()
    );
    assert_eq!(
        tx.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    topology_handoff::attach_created(
        &tx,
        &namespace(),
        "i",
        "seat:s",
        "create",
        &ThreadId::new("t"),
    )
    .unwrap();
    assert_eq!(
        handoff::current(&tx, &a.handoff).unwrap().unwrap().thread,
        Some(ThreadId::new("t"))
    );
    assert_eq!(
        tx.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        Some("t".into()),
        "CREATE must protect bootstrap in the same deciding transaction"
    );
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn existing_thread_bootstrap_vetoes_archival_before_topology_effects() {
    use herdr_threads::store::archival::{self, Runtime};
    let mut db = fixture();
    {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0)).unwrap();
        tx.commit().unwrap();
    }
    db.execute_batch(
        "UPDATE memberships SET state='left'; UPDATE occupant_bindings SET ended_at=1;",
    )
    .unwrap();
    for at in (0..=3_600_000).step_by(60_000) {
        let rt = Runtime {
            boot: "boot".into(),
            mono: at,
            utc: UtcMillis(at),
            after_ms: 3_600_000,
            host_generation: 0,
            coherent: true,
            valid_until_mono: None,
            legacy_source: Some("covered".into()),
        };
        for _ in 0..20 {
            if !archival::advance(&db, "i", &rt).unwrap().has_more {
                break;
            }
        }
    }
    assert!(
        !db.query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
            .get::<_, bool>(0))
            .unwrap(),
        "bootstrap protects selected existing thread before native topology exists"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM bootstrap_attachments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn completion(id: &BootstrapIdentity, a: &BootstrapAttachment) -> CompleteLinkedBootstrap {
    use sha2::{Digest, Sha256};
    // Literal production launcher prompt for this pinned namespace, retained as
    // data independently of the validator's builder.
    let prompt = r#"Expected handoff command routing (JSON data): null Prefer a startup hook command group only when its instance UUID, canonical state directory and canonical host endpoint exactly match every expected routing field above. Missing (null), different or ambiguous routing cannot supersede this handoff's target. Open your durable inbox using that matching group. Otherwise use the exact fallback: `herdr-threads --state-dir /state --host-endpoint /host.sock inbox`. The task for thread t is stored in inbox; follow its printed next: commands for complete bodies. Do not reread it with read/body. When waiting for replies, finish your turn and let hooks notify you of new mail; do not poll or run follow. Launch does not accept invitations or ACK messages. Accept invitations separately; default text inbox ACKs fully displayed messages."#;
    let mut argv = id.payload.launch.argv.clone();
    argv.push(prompt.into());
    let report = serde_json::json!({"outcome":"started","pane":"w:p2","seat":"peer","harness":"codex","argv":argv});
    CompleteLinkedBootstrap {
        identity: id.clone(),
        attachment: a.clone(),
        operation: id.payload.linked_complete_key.clone(),
        legacy_completion: HandoffMutation {
            identity: a.handoff.clone(),
            operation: id.payload.handoff.keys.complete.clone(),
        },
        retained: LinkedBootstrapReport {
            thread: ThreadId::new("t"),
            recipient: SeatId::new("peer"),
            pane: a.created.root_pane.clone(),
            kind: "launch".into(),
            launch: id.payload.launch.clone(),
            report_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&report).unwrap())),
            report,
            terminal: a.created.terminal.clone(),
            host_incarnation: a.created.host_incarnation.clone(),
        },
    }
}

#[test]
fn linked_completion_stores_real_report_and_both_terminals_with_exact_historical_replay() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let id = identity();
    let a = linked(&tx, &id);
    let command = completion(&id, &a);
    let done = topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(2))
        .unwrap();
    assert_eq!(done.legacy_result.state, HandoffState::Completed);
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .state,
        BootstrapState::Completed
    );
    tx.execute_batch("UPDATE threads SET archived=1; UPDATE memberships SET state='left'; UPDATE seats SET generation=2;").unwrap();
    assert_eq!(
        topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(3))
            .unwrap(),
        done
    );
    assert_eq!(
        handoff::import_hint(
            &tx,
            &a.handoff,
            Some(&ThreadId::new("t")),
            "delayed",
            UtcMillis(4)
        )
        .unwrap()
        .state,
        HandoffState::Completed
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM bootstrap_reports", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    tx.commit().unwrap();
}

#[test]
fn failed_second_terminal_update_rolls_back_child_report_and_operations_even_if_caller_commits() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let id = identity();
    let a = linked(&tx, &id);
    let command = completion(&id, &a);
    tx.execute_batch("CREATE TEMP TRIGGER reject_parent_terminal BEFORE UPDATE ON bootstrap_handoffs WHEN NEW.state='completed' BEGIN SELECT RAISE(ABORT,'test rollback'); END;").unwrap();
    assert!(
        topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(2))
            .is_err()
    );
    assert_eq!(
        handoff::current(&tx, &a.handoff).unwrap().unwrap().state,
        HandoffState::Live
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM bootstrap_reports", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    tx.execute_batch("DROP TRIGGER reject_parent_terminal")
        .unwrap();
    assert!(
        topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(3))
            .is_ok(),
        "failed wrapper leaves both fences eligible for exact retry"
    );
    tx.commit().unwrap();
}

#[test]
fn new_thread_completion_preserves_original_child_none_through_create_and_replay() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let mut id = identity();
    id.payload.handoff.channel = HandoffChannel::New {
        name: None,
        topic: "new".into(),
        goal: "goal".into(),
    };
    id.digest = id.semantic_digest().unwrap();
    let a = linked(&tx, &id);
    topology_handoff::attach_created(
        &tx,
        &namespace(),
        "i",
        "seat:s",
        "create",
        &ThreadId::new("t"),
    )
    .unwrap();
    let command = completion(&id, &a);
    assert!(command.legacy_completion.identity.thread.is_none());
    let done = topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(2))
        .unwrap();
    assert_eq!(done.legacy_result.thread, Some(ThreadId::new("t")));
    assert!(done.attachment.handoff.thread.is_none());
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .completed,
        Some(Box::new(done.clone()))
    );
    assert_eq!(
        topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(3))
            .unwrap(),
        done
    );
}

#[test]
fn linked_completion_report_storage_failures_rollback_all_canonical_rows() {
    for timing in ["BEFORE", "AFTER"] {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let id = identity();
        let a = linked(&tx, &id);
        let command = completion(&id, &a);
        tx.execute_batch(&format!("CREATE TEMP TRIGGER fail_report {timing} INSERT ON bootstrap_reports BEGIN SELECT RAISE(ABORT,'report refusal'); END;")).unwrap();
        assert!(
            topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(2))
                .is_err()
        );
        assert_eq!(
            handoff::current(&tx, &a.handoff).unwrap().unwrap().state,
            HandoffState::Live
        );
        assert_eq!(
            topology_handoff::current(&tx, &namespace(), &id)
                .unwrap()
                .unwrap()
                .state,
            BootstrapState::Attached
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        tx.commit().unwrap();
    }
}

#[test]
fn linked_completion_refuses_failed_unknown_changed_arguments_and_legacy_operation_conflicts() {
    use sha2::{Digest, Sha256};
    for change in [
        "failed",
        "unknown",
        "digest",
        "options",
        "owned",
        "prompt",
        "routing",
        "wrapper-key",
        "legacy-key",
        "operation-digest",
        "operation-result",
    ] {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let id = identity();
        let a = linked(&tx, &id);
        let mut command = completion(&id, &a);
        match change {
            "failed" => command.retained.report["outcome"] = "failed".into(),
            "unknown" => command.retained.report["outcome"] = "outcome_unknown".into(),
            "digest" => command.retained.report_digest = "0".repeat(64),
            "options" => command.retained.report["argv"][1] = "changed".into(),
            "owned" => command.retained.report["argv"]
                .as_array_mut()
                .unwrap()
                .insert(0, "--settings=unexpected".into()),
            "prompt" => command.retained.report["argv"][2] = "invented prompt".into(),
            "routing" => {
                let prompt = command.retained.report["argv"][2]
                    .as_str()
                    .unwrap()
                    .replace("/state", "/foreign");
                command.retained.report["argv"][2] = prompt.into();
            }
            "wrapper-key" => command.operation = OperationId::new("wrong-wrapper"),
            "legacy-key" => command.legacy_completion.operation = OperationId::new("wrong-legacy"),
            "operation-digest" | "operation-result" => {
                let digest = if change == "operation-digest" {
                    [0; 32]
                } else {
                    herdr_threads::store::control::cooperative_payload_hash(
                        "complete_handoff",
                        &command.legacy_completion,
                    )
                    .unwrap()
                };
                tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES('seat:s','complete',?1,?2,0)",rusqlite::params![digest.as_slice(),serde_json::to_string(&herdr_threads::protocol::results::CommandResult::SeatResolved(SeatId::new("wrong"))).unwrap()]).unwrap();
            }
            _ => unreachable!(),
        }
        if change != "digest" {
            command.retained.report_digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&command.retained.report).unwrap())
            );
        }
        assert!(
            topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(2))
                .is_err(),
            "{change}"
        );
        assert_eq!(
            handoff::current(&tx, &a.handoff).unwrap().unwrap().state,
            HandoffState::Live,
            "{change}"
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM bootstrap_reports", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{change}"
        );
        tx.commit().unwrap();
    }
}

#[test]
fn foreign_selected_namespace_and_bare_begin_create_refuse_even_only_copied_instance_match() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let mut id = identity();
    id.payload.handoff.channel = HandoffChannel::New {
        name: None,
        topic: "new".into(),
        goal: "goal".into(),
    };
    id.digest = id.semantic_digest().unwrap();
    let a = linked(&tx, &id);
    let mut foreign = namespace();
    foreign.state_dir = "/foreign".into();
    assert!(handoff::begin_pending(&tx, &a.handoff, UtcMillis(2)).is_err());
    assert!(handoff::begin_linked_pending(&tx, &foreign, &a.handoff, UtcMillis(2)).is_err());
    assert!(handoff::attach_created(&tx, "i", "seat:s", "create", &ThreadId::new("t")).is_err());
    assert!(
        topology_handoff::attach_created(
            &tx,
            &foreign,
            "i",
            "seat:s",
            "create",
            &ThreadId::new("t")
        )
        .is_err()
    );
    assert_eq!(
        handoff::current(&tx, &a.handoff).unwrap().unwrap().thread,
        None
    );
    assert_eq!(
        tx.query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
}

#[test]
fn completed_wrapper_reopens_historically_and_corruption_never_revives_protection() {
    let (_directory, path, mut db) = super::file_fixture();
    let id = identity();
    let command;
    let done = {
        let tx = db.transaction().unwrap();
        let a = linked(&tx, &id);
        command = completion(&id, &a);
        let done =
            topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(2))
                .unwrap();
        tx.commit().unwrap();
        done
    };
    drop(db);
    let mut db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("UPDATE threads SET archived=1; UPDATE memberships SET state='left'; UPDATE seats SET generation=2; UPDATE occupant_bindings SET ended_at=3;").unwrap();
    {
        let tx = db.transaction().unwrap();
        assert_eq!(
            topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(4))
                .unwrap(),
            done
        );
        tx.commit().unwrap();
    }
    db.execute_batch("DROP TRIGGER bootstrap_report_immutable;")
        .unwrap();
    let mut corrupt = done.clone();
    corrupt.retained.report_digest = "0".repeat(64);
    db.execute(
        "UPDATE bootstrap_reports SET completed_json=?1",
        [serde_json::to_vec(&corrupt).unwrap()],
    )
    .unwrap();
    drop(db);
    let mut db = rusqlite::Connection::open(&path).unwrap();
    let tx = db.transaction().unwrap();
    assert!(
        topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(5))
            .is_err()
    );
    assert!(topology_handoff::current(&tx, &namespace(), &id).is_err());
    assert_eq!(
        tx.query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "completed"
    );
    assert_eq!(
        handoff::current(&tx, &command.attachment.handoff)
            .unwrap()
            .unwrap()
            .state,
        HandoffState::Completed
    );
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM bootstrap_handoffs WHERE state NOT IN ('completed','cancelled')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    tx.commit().unwrap();
}

/// Schema-state control only: the real cancellation writer/order race is the
/// parent's cross-leaf composition gate, never reimplemented by this fixture.
#[test]
fn cancelled_attached_fixture_retains_evidence_and_refuses_later_linked_begin_or_import() {
    let mut db = fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    let a = attached_fixture(&tx, &id);
    let retained = saved_recovery(
        &tx,
        &id,
        BootstrapRecoveryDisposition::Cancelled {
            reason: "inspected lost pane before child begin".into(),
            quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
            child_guard: BootstrapCancellationGuard {
                attached_child: Some(a.handoff.clone()),
            },
        },
        BootstrapState::Cancelled,
        Some(a.created.clone()),
    );
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='cancelled',terminal_at=2 WHERE id=1",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let status = topology_handoff::current(&db, &namespace(), &id)
        .unwrap()
        .unwrap();
    assert_eq!(status.state, BootstrapState::Cancelled);
    assert_eq!(status.attachment, Some(a.clone()));
    assert_eq!(status.creation, Some(a.created.clone()));
    assert_eq!(status.recovery, Some(Box::new(retained)));
    let tx = db.transaction().unwrap();
    assert!(handoff::begin_linked_pending(&tx, &namespace(), &a.handoff, UtcMillis(3)).is_err());
    assert!(handoff::begin_pending(&tx, &a.handoff, UtcMillis(3)).is_err());
    assert!(
        handoff::import_hint(
            &tx,
            &a.handoff,
            a.handoff.thread.as_ref(),
            "delayed",
            UtcMillis(3)
        )
        .is_err()
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM bootstrap_attachments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .state,
        BootstrapState::Cancelled
    );
    tx.commit().unwrap();
}

#[test]
fn begin_first_live_child_remains_live_and_import_never_suppresses_it() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let id = identity();
    let a = linked(&tx, &id);
    let original = handoff::current(&tx, &a.handoff).unwrap().unwrap();
    assert_eq!(original.state, HandoffState::Live);
    assert_eq!(
        handoff::import_hint(
            &tx,
            &a.handoff,
            a.handoff.thread.as_ref(),
            "delayed",
            UtcMillis(3)
        )
        .unwrap(),
        original
    );
    assert_eq!(
        handoff::current(&tx, &a.handoff).unwrap().unwrap(),
        original
    );
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
            .unwrap()
            .unwrap()
            .state,
        BootstrapState::Attached
    );
    tx.commit().unwrap();
}

/// Upgrade simulation: the genuine literal V1 report stays byte-identical
/// while today's prompt renderer and/or composer change. Pending deciding
/// completion and exact historical replay must still accept it; corrupted
/// reports still refuse with both fences unchanged.
#[test]
fn genuine_v1_report_completes_and_replays_after_current_composition_drift() {
    use herdr_threads::harness::launch::{compose_native_argv, current_drift};
    use sha2::{Digest, Sha256};
    for (label, suffix, rejected) in [
        ("prose", Some(" Re-read the task body."), None),
        ("admission", None, Some("fixed")),
        ("both", Some(" Re-read the task body."), Some("fixed")),
    ] {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let id = identity();
        let a = linked(&tx, &id);
        let command = completion(&id, &a);
        let genuine = serde_json::to_vec(&command.retained).unwrap();
        let _drift = current_drift::arm(suffix, rejected);
        if rejected.is_some() {
            assert!(
                compose_native_argv(
                    id.payload.launch.harness,
                    id.payload.launch.argv.clone(),
                    vec![]
                )
                .is_err(),
                "{label}: current composer observes the drift"
            );
        }
        for change in ["options", "owned", "prompt", "routing", "body", "recipient"] {
            let mut bad = command.clone();
            match change {
                "options" => bad.retained.report["argv"][1] = "changed".into(),
                "owned" => bad.retained.report["argv"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, "-chooks.Stop=[x]".into()),
                "prompt" => bad.retained.report["argv"][2] = "invented prompt".into(),
                "routing" => {
                    let p = bad.retained.report["argv"][2].as_str().unwrap().replace(
                        "JSON data): null",
                        r#"JSON data): {"instance":"00000000-0000-4000-8000-000000000001","state_dir":"/state","host_endpoint":"/host.sock"}"#,
                    );
                    bad.retained.report["argv"][2] = p.into();
                }
                "body" => {
                    let p = format!("{} work", bad.retained.report["argv"][2].as_str().unwrap());
                    bad.retained.report["argv"][2] = p.into();
                }
                "recipient" => bad.retained.recipient = SeatId::new("other"),
                _ => unreachable!(),
            }
            bad.retained.report_digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&bad.retained.report).unwrap())
            );
            assert!(
                topology_handoff::complete_linked_pending(&tx, &namespace(), &bad, UtcMillis(2))
                    .is_err(),
                "{label}/{change}"
            );
            assert_eq!(
                handoff::current(&tx, &a.handoff).unwrap().unwrap().state,
                HandoffState::Live,
                "{label}/{change}"
            );
        }
        let done =
            topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(3))
                .unwrap_or_else(|e| panic!("{label}: genuine V1 report refused: {e:?}"));
        assert_eq!(serde_json::to_vec(&done.retained).unwrap(), genuine);
        assert_eq!(
            topology_handoff::complete_linked_pending(&tx, &namespace(), &command, UtcMillis(4))
                .unwrap(),
            done,
            "{label}"
        );
        assert_eq!(
            topology_handoff::current(&tx, &namespace(), &id)
                .unwrap()
                .unwrap()
                .state,
            BootstrapState::Completed
        );
        tx.commit().unwrap();
    }
}
