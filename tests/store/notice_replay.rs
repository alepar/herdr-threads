//! Canonical virtual-warning recovery without a process ancestor or daemon loop.
use crate::{
    ports::{LocalService, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{
            CheckIn, CheckInMode, Command, CreateThread, DeliveryMode, InvitationConfirmation,
            Invite, SendMessage,
        },
        ids::*,
        results::CommandResult,
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    service::{delivery_pull::DeliveryPullState, dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

pub(super) struct Fixture {
    service: DomainService,
    pub(super) store: Arc<dyn StorePort>,
    /// The offered ledger the service and (in production) the wake lane share.
    state: Arc<DeliveryPullState>,
    clock: Arc<TestClock>,
    pub(super) db: Connection,
    directory: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}

fn peer() -> PeerIdentity {
    PeerIdentity::from_kernel(501)
}

fn fourth() -> Harness {
    Harness::Agent(
        crate::harness::registry::builtins()
            .agent("synthetic_fourth")
            .unwrap(),
    )
}

fn fixture() -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let directory =
        crate::test_support::scratch::temp_path(format!("delivery-pull-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let shared: Arc<dyn Clock> = clock.clone();
    let context = StoreContext::new(directory.join("store.db"), shared.clone()).with_test_host();
    let db = context.open_writer().unwrap();
    db.execute_batch(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=3",
    )
    .unwrap();
    for (seat, target) in [("s", "p"), ("x", "px"), ("c", "pc")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',?2,1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", [seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence,journaled) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,1,0)", []).unwrap();
    for seat in ["s", "x", "c"] {
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let state = Arc::new(DeliveryPullState::default());
    let service = DomainService::new(
        "34750f98bd59fcfc946da45aaabe933b".into(),
        Arc::clone(&store),
        shared,
    )
    .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
    .with_delivery_pull(Arc::clone(&state));
    Fixture {
        service,
        store,
        state,
        clock,
        db,
        directory,
    }
}

fn restart(f: &mut Fixture) {
    let shared: Arc<dyn Clock> = f.clock.clone();
    f.state = Arc::new(DeliveryPullState::default());
    f.service = DomainService::new(
        "34750f98bd59fcfc946da45aaabe933b".into(),
        Arc::clone(&f.store),
        shared,
    )
    .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
    .with_delivery_pull(Arc::clone(&f.state));
}

fn claim(
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
    generation: u64,
) -> CallerClaim {
    CallerClaim {
        auxiliary_session: false,
        host: crate::test_support::test_host_locator(),
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        seat: SeatId::new(seat),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(target),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint {
            // One harness process per pane: a seat's sessions share it.
            pid: 1000 + seat.bytes().map(u32::from).sum::<u32>(),
            start: 1,
        }),
    }
}

fn fixture_claim(
    f: &Fixture,
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
) -> CallerClaim {
    let generation: i64 = f.db.query_row(
        "SELECT generation FROM seats WHERE id=?1 AND instance_id='34750f98bd59fcfc946da45aaabe933b' AND target_id=?2 AND state='resolved'",
        params![seat, target],
        |row| row.get(0),
    ).unwrap();
    claim(
        seat,
        target,
        harness,
        session,
        u64::try_from(generation).unwrap(),
    )
}

fn check_in(f: &Fixture, claim: CallerClaim, operation: &str) -> CallerClaim {
    let command = Command::CheckIn(CheckIn {
        readonly: false,
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: claim.binding_generation,
        },
        claim,
        operation: OperationId::new(operation),
        lineage_parent: None,
        continuity_key: None,
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    }
}

pub(super) fn binding_for_claim(f: &Fixture, claim: &CallerClaim) -> BindingId {
    BindingId::new(f.db.query_row::<String, _, _>(
        "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 AND native_session=?3 AND agent_id=?4 AND ended_at IS NULL",
        params![claim.seat.as_str(), claim.harness.as_str(), claim.native_session.as_str(), claim.agent_id.as_ref().map_or("", |agent| agent.as_str())],
        |row| row.get(0),
    ).unwrap())
}

fn presentation_test_pull(
    f: &Fixture,
    claim: &CallerClaim,
) -> crate::protocol::delivery::DeliveryPullReplyV2 {
    let result = f
        .service
        .handle(
            Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                defer_notice_settlement: false,
                claim: claim.clone(),
                consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: Vec::new(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryPulledV2(reply) = result else {
        panic!("wrong presentation result {result:?}")
    };
    reply
}

fn finish_presentation_attention_job(f: &Fixture, job: &str) {
    for _ in 0..16 {
        let progress = f
            .store
            .advance_work(
                job,
                crate::ports::DurableWorkAdmission::new(1).unwrap(),
                &budget(),
            )
            .unwrap();
        assert!(progress.processed_this_turn <= 1);
        assert_eq!(progress.last_error, None);
        if !progress.has_more {
            return;
        }
    }
    panic!("fixture attention job did not finish within its finite recipient bound");
}

pub(super) fn published_notice() -> (Fixture, CallerClaim, InvitationId, MessageId, String) {
    let f = fixture();
    let recipient = check_in(
        &f,
        fixture_claim(&f, "s", "p", fourth(), "virtual-warning-reader"),
        "virtual-warning-reader-ci",
    );
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "virtual-warning-inviter"),
        "virtual-warning-inviter-ci",
    );
    let CommandResult::ThreadCreated(invitation_thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "virtual warning replay invitation".into(),
                goal: "review".into(),
                operation: OperationId::new("virtual-warning-create"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("canonical thread")
    };
    let CommandResult::Invitation(invitation) = f
        .service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread: invitation_thread,
                seat: recipient.seat.clone(),
                deadline_millis: None,
                operation: OperationId::new("virtual-warning-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("canonical invitation")
    };
    // c is a joined member with no registered binding. The reader authors the
    // send, so its page cannot pick up an ordinary self-receipt instead.
    let CommandResult::MessageSent(message) = f
        .service
        .handle(
            Command::SendMessage(SendMessage {
                priority: None,
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "canonical unavailable member".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("virtual-warning-send"),
                claim: recipient.clone(),
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("canonical send")
    };
    let warning: String = f.db.query_row(
        "SELECT w.warning_id FROM send_manifests sm JOIN prepared_unavailable_warnings w ON w.preparation_id=sm.preparation_id WHERE sm.message_id=?1 AND w.affected_seat_id='c'",
        [message.as_str()], |row| row.get(0),
    ).unwrap();
    let send_job: String = f.db.query_row(
        "SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)",
        [message.as_str()], |row| row.get(0),
    ).unwrap();
    // Only bounded attention/recipient attribution. No physical warning writer,
    // daemon loop, process ancestor, clock wait or model is involved.
    finish_presentation_attention_job(&f, &send_job);
    finish_presentation_attention_job(&f, &format!("work:{warning}"));
    (f, recipient, invitation, message, warning)
}

// Kills: physical-only applicability retires a published virtual Notice on
// lost-output replay, even though its frozen event and recipient still exist.
#[test]
fn presentation_virtual_notice_replays_and_confirms_original_slots_without_materializing() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Confirmation,
        DeliveryPullReplyV2, PresentationConsumer, PresentationItemV2,
    };
    let (mut f, recipient, invitation, message, warning) = published_notice();
    let binding = binding_for_claim(&f, &recipient);
    let canonical = |f: &Fixture| -> String {
        f.db.query_row(
            "SELECT json_array(w.warning_id,w.preparation_id,w.warning_key,w.affected_seat_id,w.unavailability_episode,w.warning_offset,w.event_json,sm.message_id,sm.instance_id,sm.thread_id,sm.decision_seq,sm.base_sequence,sm.warning_count,sm.interval_high_water,d.ordinal,d.seat_id,d.thread_id,d.event_seq,d.event_offset,r.generation,r.ordinal) FROM digest_programmatic_warnings d JOIN warning_recipients r ON r.warning_id=d.warning_id AND r.seat_id=d.seat_id JOIN send_manifests sm ON sm.instance_id=?3 AND sm.decision_seq=d.event_seq JOIN prepared_unavailable_warnings w ON w.preparation_id=sm.preparation_id AND w.warning_offset=d.event_offset WHERE d.warning_id=?1 AND d.seat_id=?2 AND w.warning_id=d.warning_id",
            params![warning, recipient.seat.as_str(), recipient.instance], |row| row.get(0),
        ).unwrap()
    };
    let no_physical = |f: &Fixture| {
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM messages WHERE id=?1",
                [&warning],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0,
            "published virtual warning must remain unmaterialized"
        );
    };
    let binding_state = |f: &Fixture| -> String {
        f.db.query_row("SELECT json_array(binding_id,seat_id,generation,execution_id,native_session,liveness,liveness_pid,liveness_start,ended_at) FROM occupant_bindings WHERE binding_id=?1", [binding.as_str()], |row| row.get(0)).unwrap()
    };
    let unsettled = |f: &Fixture| -> String {
        f.db.query_row("SELECT json_array((SELECT state FROM invitations WHERE id=?1),(SELECT json_group_array(json_array(seat_id,state,ack_required)) FROM (SELECT seat_id,state,ack_required FROM receipt_state WHERE message_id=?2 ORDER BY seat_id)),(SELECT json_group_array(json_array(seat_id,state,ack_required)) FROM (SELECT seat_id,state,ack_required FROM receipts WHERE message_id=?2 ORDER BY seat_id)))", params![invitation.as_str(), message.as_str()], |row| row.get(0)).unwrap()
    };
    let outbox = |f: &Fixture| -> String {
        f.db.query_row("SELECT json_array(binding_id,seat_id,consumer,instance_id,generation,execution,outbox_token,outbox_descriptors,outbox_selection,outbox_render_key,outbox_codec_key,outbox_prefix,outbox_retired_mask,outbox_confirmed_mask) FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'", [binding.as_str()], |row| row.get(0)).unwrap()
    };
    let frontier = |f: &Fixture| -> i64 {
        f.db.query_row(
            "SELECT offered_ordinal FROM digest_notice_offer WHERE binding_id=?1",
            [binding.as_str()],
            |row| row.get(0),
        )
        .unwrap()
    };
    let expected_canonical = canonical(&f);
    let expected_binding = binding_state(&f);
    let expected_unsettled = unsettled(&f);
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM invitations WHERE id=?1",
            [invitation.as_str()],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    no_physical(&f);
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("first canonical page")
    };
    assert!(first.items.iter().any(|item| matches!(item, PresentationItemV2::Notice { warning: id, thread, .. } if id.as_str()==warning && thread.as_str()=="t")));
    assert!(
        first.items.iter().any(
            |item| matches!(item, PresentationItemV2::Invitation { id, .. } if id==&invitation)
        )
    );
    assert!(
        first
            .items
            .iter()
            .any(|item| matches!(item, PresentationItemV2::Attention { .. }))
    );
    assert!(
        !first
            .items
            .iter()
            .any(|item| matches!(item, PresentationItemV2::Message { .. }))
    );
    assert!(first.offered.is_empty() && first.truncated.is_empty());
    let token = first
        .presentation_token
        .clone()
        .expect("captured output token");
    let retained = outbox(&f);
    let offered_frontier = frontier(&f);
    let warning_ordinal: i64 =
        f.db.query_row(
            "SELECT ordinal FROM digest_programmatic_warnings WHERE seat_id=?1 AND warning_id=?2",
            params![recipient.seat.as_str(), warning],
            |row| row.get(0),
        )
        .unwrap();
    assert!(offered_frontier >= warning_ordinal);
    restart(&mut f);
    let DeliveryPullReplyV2::Page(replay) = presentation_test_pull(&f, &recipient) else {
        panic!("lost-output replay")
    };
    assert_eq!(
        replay, first,
        "full descriptor order, body, stamps, token, codec selection and bytes survive replay"
    );
    assert_eq!(
        outbox(&f),
        retained,
        "original masks and all frozen outbox fields stay exact"
    );
    assert_eq!(frontier(&f), offered_frontier);
    assert_eq!(canonical(&f), expected_canonical);
    assert_eq!(binding_state(&f), expected_binding);
    assert_eq!(unsettled(&f), expected_unsettled);
    no_physical(&f);
    let confirm = |token: String, emitted_controls| {
        let CommandResult::DeliveryPresentationConfirmedV2(reply) = f
            .service
            .handle(
                Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Hook,
                    token,
                    emitted_controls,
                    presentation_lease: None,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("canonical confirmation")
        };
        reply
    };
    let foreign = "ffffffffffffffffffffffffffffffff".to_owned();
    assert_ne!(foreign, token);
    let wrong = confirm(foreign, first.emitted_controls.clone());
    assert_eq!(wrong.status, Confirmation::NotCurrentPage);
    assert!(wrong.acknowledged_controls.is_empty());
    assert_eq!(outbox(&f), retained);
    // Confirm consumes captured Notice/Invitation slots even though the warning
    // has no physical row; omitted control stamps stay unconfirmed in place.
    let partial = confirm(token.clone(), vec![]);
    assert_eq!(partial.status, Confirmation::Confirmed);
    assert!(partial.acknowledged_controls.is_empty());
    let retained_json: serde_json::Value = serde_json::from_str(&retained).unwrap();
    let descriptors: Vec<serde_json::Value> =
        serde_json::from_str(retained_json[7].as_str().unwrap()).unwrap();
    let noncontrol_mask = descriptors
        .iter()
        .enumerate()
        .filter(|(_, descriptor)| {
            matches!(
                descriptor["item"]["kind"].as_str(),
                Some("notice" | "invitation")
            )
        })
        .fold(0i64, |mask, (slot, _)| mask | (1i64 << slot));
    assert_ne!(noncontrol_mask, 0);
    let partial_json: serde_json::Value = serde_json::from_str(&outbox(&f)).unwrap();
    for field in 0..12 {
        assert_eq!(
            partial_json[field], retained_json[field],
            "frozen field {field}"
        );
    }
    assert_eq!(partial_json[12], serde_json::json!(noncontrol_mask));
    assert_eq!(partial_json[13], serde_json::json!(noncontrol_mask));
    let completed = confirm(token.clone(), first.emitted_controls.clone());
    assert_eq!(completed.status, Confirmation::Confirmed);
    assert_eq!(completed.acknowledged_controls, first.emitted_controls);
    let duplicate = confirm(token.clone(), first.emitted_controls.clone());
    assert_eq!(duplicate.status, Confirmation::AlreadyConfirmed);
    assert_eq!(duplicate.acknowledged_controls, first.emitted_controls);
    let tokens: (Option<String>, Option<String>) = f.db.query_row("SELECT outbox_token,last_token FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'", [binding.as_str()], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!(tokens, (None, Some(token)));
    assert_eq!(f.db.query_row("SELECT complete_count FROM invitation_presentation_items WHERE binding_id=?1 AND consumer='hook' AND invitation_id=?2", params![binding.as_str(), invitation.as_str()], |row| row.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(canonical(&f), expected_canonical);
    assert_eq!(binding_state(&f), expected_binding);
    assert_eq!(unsettled(&f), expected_unsettled);
    assert_eq!(frontier(&f), offered_frontier);
    no_physical(&f);
}

// The measured SQL is the production literal, not a hand-maintained lookalike.
// Unpublished same-ID recipes must neither become authority nor lengthen replay.
#[test]
fn retained_virtual_notice_lookup_uses_indexed_constant_vm_work() {
    use rusqlite::StatementStatus;
    let (f, recipient, _, message, warning) = published_notice();
    let sql = super::NOTICE_REPLAY_APPLICABLE_SQL;
    let mut measured = Vec::new();
    let mut previous = 0;
    for count in [0, 64, 4096] {
        f.db.execute(
            "WITH RECURSIVE n(n) AS (SELECT ?1+1 WHERE ?1<?2 UNION ALL SELECT n+1 FROM n WHERE n<?2) INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT 'lookup-unpublished-'||n,'34750f98bd59fcfc946da45aaabe933b','actor','lookup-unpublished-'||n,zeroblob(32),'t',0,0,0,0,0,0,0,'building' FROM n",
            params![previous, count],
        ).unwrap();
        f.db.execute(
            "WITH RECURSIVE n(n) AS (SELECT ?1+1 WHERE ?1<?2 UNION ALL SELECT n+1 FROM n WHERE n<?2) INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) SELECT 'lookup-unpublished-'||n,w.warning_key,w.warning_id,w.affected_seat_id,w.unavailability_episode,w.warning_offset,w.event_json FROM n CROSS JOIN send_manifests sm ON sm.message_id=?3 CROSS JOIN prepared_unavailable_warnings w ON w.preparation_id=sm.preparation_id AND w.warning_id=?4",
            params![previous, count, message.as_str(), warning],
        ).unwrap();
        f.db.execute(
            "WITH RECURSIVE n(n) AS (SELECT ?1+1 WHERE ?1<?2 UNION ALL SELECT n+1 FROM n WHERE n<?2) INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) SELECT ?3,'lookup-unrelated-'||n,'t',1,1 FROM n",
            params![previous, count, recipient.seat.as_str()],
        ).unwrap();
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM prepared_unavailable_warnings WHERE warning_id=?1",
                [&warning],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            count + 1
        );
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM messages WHERE id=?1",
                [&warning],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert!(
            super::notice_replay_applicable(
                &f.db,
                &recipient,
                &MessageId::new(&warning),
                &ThreadId::new("t")
            )
            .unwrap()
        );
        for (label, id, thread, seat, instance, expected) in [
            (
                "published",
                warning.as_str(),
                "t",
                recipient.seat.as_str(),
                "34750f98bd59fcfc946da45aaabe933b",
                true,
            ),
            (
                "missing-warning",
                "lookup-missing",
                "t",
                recipient.seat.as_str(),
                "34750f98bd59fcfc946da45aaabe933b",
                false,
            ),
            (
                "wrong-thread",
                warning.as_str(),
                "lookup-other-thread",
                recipient.seat.as_str(),
                "34750f98bd59fcfc946da45aaabe933b",
                false,
            ),
            (
                "wrong-recipient",
                warning.as_str(),
                "t",
                "lookup-other-seat",
                "34750f98bd59fcfc946da45aaabe933b",
                false,
            ),
            (
                "wrong-instance",
                warning.as_str(),
                "t",
                recipient.seat.as_str(),
                "lookup-other-instance",
                false,
            ),
        ] {
            let plan =
                f.db.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                    .unwrap()
                    .query_map(params![id, thread, seat, instance], |row| {
                        row.get::<_, String>(3)
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
            for relation in ["messages", "d", "sm", "w", "r"] {
                assert!(
                    plan.iter()
                        .any(|step| step.contains(&format!("SEARCH {relation} "))),
                    "exact indexed search for {relation}: {plan:?}"
                );
            }
            assert!(
                plan.iter()
                    .any(|step| step.contains("send_manifests_instance_decision")),
                "published instance/decision uses its unique index: {plan:?}"
            );
            assert!(
                !plan
                    .iter()
                    .any(|step| step.starts_with("SCAN ") && !step.contains("CONSTANT ROW")),
                "no relation scan: {plan:?}"
            );
            let mut statement = f.db.prepare(sql).unwrap();
            let applicable: bool = statement
                .query_row(params![id, thread, seat, instance], |row| row.get(0))
                .unwrap();
            assert_eq!(applicable, expected, "{label}");
            let vm = statement.get_status(StatementStatus::VmStep);
            let scans = statement.get_status(StatementStatus::FullscanStep);
            let sorts = statement.get_status(StatementStatus::Sort);
            let version: String =
                f.db.query_row("SELECT sqlite_version()", [], |row| row.get(0))
                    .unwrap();
            eprintln!(
                "SQLite {version} retained Notice unpublished={count} case={label} VM={vm} fullscan={scans} sorts={sorts} plan={plan:?}"
            );
            assert_eq!(scans, 0, "{label}");
            assert_eq!(sorts, 0, "{label}");
            assert!(
                vm <= 256,
                "bounded exact lookup: rows={count} case={label} VM={vm}"
            );
            measured.push((count, label, vm));
        }
        previous = count;
    }
    for label in [
        "published",
        "missing-warning",
        "wrong-thread",
        "wrong-recipient",
        "wrong-instance",
    ] {
        let values = measured
            .iter()
            .filter(|(_, case, _)| *case == label)
            .map(|(_, _, vm)| *vm)
            .collect::<Vec<_>>();
        assert!(
            values.iter().max().unwrap() - values.iter().min().unwrap() <= 16,
            "backlog-independent VM work for {label}: {measured:?}"
        );
    }
    // Each distinct logical authority component must survive a keyed lookup;
    // the matching retained id alone does not authorize an unpublished recipe.
    for mutation in [
        "UPDATE digest_programmatic_warnings SET event_seq=event_seq+100000 WHERE warning_id=?1 AND seat_id=?2",
        "UPDATE digest_programmatic_warnings SET event_offset=event_offset+100000 WHERE warning_id=?1 AND seat_id=?2",
        "DELETE FROM warning_recipients WHERE warning_id=?1 AND seat_id=?2",
    ] {
        f.db.execute_batch("SAVEPOINT notice_lookup_control")
            .unwrap();
        f.db.execute(mutation, params![warning, recipient.seat.as_str()])
            .unwrap();
        assert!(
            !super::notice_replay_applicable(
                &f.db,
                &recipient,
                &MessageId::new(&warning),
                &ThreadId::new("t")
            )
            .unwrap(),
            "missing canonical authority: {mutation}"
        );
        f.db.execute_batch("ROLLBACK TO notice_lookup_control; RELEASE notice_lookup_control")
            .unwrap();
        assert!(
            super::notice_replay_applicable(
                &f.db,
                &recipient,
                &MessageId::new(&warning),
                &ThreadId::new("t")
            )
            .unwrap()
        );
    }
}

#[test]
fn pi_channel_owned_checkins_preserve_unoffered_notices_mail_and_read_cursors() {
    let (f, _, _, _, _) = published_notice();
    let pi = Harness::Agent(crate::harness::registry::builtins().agent("pi").unwrap());
    let mut pi_claim = fixture_claim(&f, "s", "p", pi, "pi-reader");
    pi_claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let mut recipient = check_in(&f, pi_claim, "pi-start");
    let sender = fixture_claim(&f, "x", "px", fourth(), "virtual-warning-inviter");
    f.service
        .handle(
            Command::SendMessage(SendMessage {
                priority: None,
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "pi unseen mail".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("pi-mail"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    f.service
        .handle(
            Command::SendMessage(SendMessage {
                priority: None,
                delivery_mode: DeliveryMode::Lazy,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "pi unseen lazy mail".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("pi-lazy-mail"),
                claim: fixture_claim(&f, "x", "px", fourth(), "virtual-warning-inviter"),
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let binding = binding_for_claim(&f, &recipient);
    assert!(
        !crate::store::attention::notice_offer_page_for(&f.db, "s", &binding, 32)
            .unwrap()
            .is_empty(),
        "lifecycle did not consume omitted notices"
    );
    let before:String=f.db.query_row("SELECT json_array((SELECT json_group_array(json_array(message_id,seat_id,state,ack_required)) FROM receipt_state WHERE seat_id='s'),(SELECT json_group_array(json_array(thread_id,binding_id,seq)) FROM binding_cursors WHERE binding_id IN (SELECT binding_id FROM occupant_bindings WHERE seat_id='s')),(SELECT json_group_array(json_array(message_id,state)) FROM lazy_recipients WHERE seat_id='s'),(SELECT json_group_array(json_array(binding_id,message_id)) FROM binding_row_displays WHERE binding_id IN (SELECT binding_id FROM occupant_bindings WHERE seat_id='s')))",[],|r|r.get(0)).unwrap();
    let warning_before: Option<i64> =
        f.db.query_row(
            "SELECT offered_through_seq FROM warning_offer WHERE binding_id=?1",
            [binding.as_str()],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    for (i, mode) in [
        CheckInMode::Current,
        CheckInMode::Lifecycle {
            expected_binding_generation: recipient.binding_generation,
        },
        CheckInMode::Current,
    ]
    .into_iter()
    .enumerate()
    {
        let CommandResult::CheckedIn(result) = f
            .service
            .handle(
                Command::CheckIn(CheckIn {
                    mode,
                    claim: recipient.clone(),
                    operation: OperationId::new(format!("pi-check-{i}")),
                    lineage_parent: None,
                    readonly: false,
                    continuity_key: None,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("check-in")
        };
        assert!(result.inbox.items.is_empty());
        assert!(result.notices.items.is_empty());
        assert!(result.warnings.items.is_empty());
        recipient = result.context;
    }
    let after:String=f.db.query_row("SELECT json_array((SELECT json_group_array(json_array(message_id,seat_id,state,ack_required)) FROM receipt_state WHERE seat_id='s'),(SELECT json_group_array(json_array(thread_id,binding_id,seq)) FROM binding_cursors WHERE binding_id IN (SELECT binding_id FROM occupant_bindings WHERE seat_id='s')),(SELECT json_group_array(json_array(message_id,state)) FROM lazy_recipients WHERE seat_id='s'),(SELECT json_group_array(json_array(binding_id,message_id)) FROM binding_row_displays WHERE binding_id IN (SELECT binding_id FROM occupant_bindings WHERE seat_id='s')))",[],|r|r.get(0)).unwrap();
    assert_eq!(before, after);
    let warning_after: Option<i64> =
        f.db.query_row(
            "SELECT offered_through_seq FROM warning_offer WHERE binding_id=?1",
            [binding.as_str()],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(warning_before, warning_after);
}

// A parked old nonwaking Notice is presentation backpressure, never the wake frontier.
// All warnings/descriptors here come from public send + canonical producer/work paths.
#[test]
fn late_notice_successor_preserves_newer_canonical_warning_wake_frontier() {
    use crate::protocol::{
        authority::ObligationRef,
        commands::Accept,
        delivery::{
            DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2, DeliveryPullReplyV2,
            PresentationConsumer, PresentationItemV2,
        },
    };
    let (mut f, owner, invitation, _, old_warning) = published_notice();
    let thread: String =
        f.db.query_row(
            "SELECT thread_id FROM invitations WHERE id=?1",
            [invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    f.service
        .handle(
            Command::Accept(Accept {
                thread: ThreadId::new(thread),
                operation: OperationId::new("liveness-accept"),
                claim: owner.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert!(!crate::store::attention::warning_wakes_seat(&f.db, "s", &old_warning).unwrap());
    let binding = binding_for_claim(&f, &owner);
    let pull = |f: &Fixture| super::legacy_confirm_tests::watch_input(f, &owner, &binding, true);
    let DeliveryPullReplyV2::Page(old) = f
        .store
        .presentation_pull(pull(&f), &budget())
        .unwrap()
        .reply
    else {
        panic!("old protected page")
    };
    assert!(old.items.iter().any(
        |v| matches!(v,PresentationItemV2::Notice {warning,..} if warning.as_str()==old_warning)
    ));
    let old_ordinal: i64 =
        f.db.query_row(
            "SELECT ordinal FROM digest_programmatic_warnings WHERE seat_id='s' AND warning_id=?1",
            [&old_warning],
            |r| r.get(0),
        )
        .unwrap();
    let sink = check_in(
        &f,
        fixture_claim(&f, "c", "pc", fourth(), "liveness-sink"),
        "liveness-sink-ci",
    );
    let CommandResult::MessageSent(message) = f
        .service
        .handle(
            Command::SendMessage(SendMessage {
                priority: None,
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "new origin receipt warning".into(),
                direct_recipient: None,
                invited_recipients: vec![sink.seat.clone()],
                deadline_millis: Some(1000),
                operation: OperationId::new("liveness-new-cause"),
                claim: owner.clone(),
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("real newer cause")
    };
    f.clock.0.fetch_add(2000, Ordering::SeqCst);
    let newer_warning = {
        let tx = f.db.transaction().unwrap();
        let overdue = crate::store::schema::record_overdue_if_pending(
            &tx,
            &ObligationRef::Receipt {
                message,
                seat: sink.seat,
            },
            &crate::ports::TimeBasis::Decision,
            f.clock.utc_now(),
        )
        .unwrap();
        assert!(overdue.inserted);
        let warning = overdue.warning.unwrap();
        tx.commit().unwrap();
        warning
    };
    finish_presentation_attention_job(&f, &format!("work:{}", newer_warning.as_str()));
    assert!(
        crate::store::attention::warning_wakes_seat(&f.db, "s", newer_warning.as_str()).unwrap()
    );
    let new_ordinal: i64 =
        f.db.query_row(
            "SELECT ordinal FROM digest_programmatic_warnings WHERE seat_id='s' AND warning_id=?1",
            [newer_warning.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(new_ordinal > old_ordinal);
    let candidate = |f: &Fixture| {
        f.store
            .wake_candidates(
                crate::test_support::test_host_id(),
                Default::default(),
                &budget(),
            )
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.seat == owner.seat)
            .expect("canonical wake candidate")
    };
    let before = candidate(&f);
    assert!(
        !before.has_pending_invitation
            && !before.has_pending_receipt
            && before.pending_lazy().is_none(),
        "only the warning may supply this wake"
    );
    assert!(before.has_actionable_work());
    let frontier = before.attention_witness.as_ref().unwrap().frontier();
    let newer_seq:u64=f.db.query_row("SELECT event_seq FROM digest_programmatic_warnings WHERE seat_id='s' AND warning_id=?1",[newer_warning.as_str()],|r|r.get::<_,i64>(0)).unwrap().try_into().unwrap();
    assert_eq!(frontier.actionable_warning.unwrap().decision_seq, newer_seq);
    let DeliveryPullReplyV2::Page(replay) = f
        .store
        .presentation_pull(pull(&f), &budget())
        .unwrap()
        .reply
    else {
        panic!("old replay")
    };
    assert_eq!(replay.presentation_token, old.presentation_token);
    assert!(replay.items.iter().any(
        |v| matches!(v,PresentationItemV2::Notice {warning,..} if warning.as_str()==old_warning)
    ));
    assert!(
        !replay.items.iter().any(
            |v| matches!(v,PresentationItemV2::Notice {warning,..} if warning==&newer_warning)
        )
    );
    let request = DeliveryPresentationConfirmV2 {
        claim: owner.clone(),
        consumer: PresentationConsumer::Watch,
        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
        token: old.presentation_token.clone().unwrap(),
        emitted_controls: old.emitted_controls,
    };
    assert_eq!(
        f.store
            .presentation_confirm(&request, &budget())
            .unwrap()
            .status,
        DeliveryPresentationConfirmationV2::Confirmed
    );
    assert_eq!(
        crate::store::attention::binding_notice_frontier(&f.db, binding.as_str()).unwrap(),
        old_ordinal
    );
    assert_eq!(
        candidate(&f).attention_witness.unwrap().frontier(),
        frontier,
        "old confirmation creates no newer publication"
    );
    let DeliveryPullReplyV2::Page(successor) = f
        .store
        .presentation_pull(pull(&f), &budget())
        .unwrap()
        .reply
    else {
        panic!("successor page")
    };
    assert_ne!(successor.presentation_token, old.presentation_token);
    assert!(
        successor.items.iter().any(
            |v| matches!(v,PresentationItemV2::Notice {warning,..} if warning==&newer_warning)
        )
    );
    assert_eq!(
        crate::store::attention::binding_notice_frontier(&f.db, binding.as_str()).unwrap(),
        old_ordinal,
        "buffering successor is not presentation"
    );
    assert_eq!(
        candidate(&f).attention_witness.unwrap().frontier(),
        frontier,
        "successor still owes the exact same warning wake"
    );
}

// Retained pre-warning_conditions data, not a warning produced by today's
// transition writers. Real Pi policy, store presentation and wake selection
// decide this fixture; there is no native runtime or fabricated receipt ACK.
mod legacy_pi_attention_offer {
    use super::*;
    use crate::protocol::delivery::{
        DeliveryPageV2, DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
        DeliveryPullReplyV2, PresentationConsumer, PresentationControlStampV2, PresentationItemV2,
    };

    fn retained_warning(f: &mut Fixture, label: &str, affected: &str) -> MessageId {
        let tx = f.db.transaction().unwrap();
        let invitation = InvitationId::new(format!("legacy-{label}"));
        tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES (?1,'t',?2,1,'pending',1,0,50,50,(SELECT id FROM local_node))", params![invitation.as_str(), affected]).unwrap();
        let (warning, inserted) = crate::store::schema::append_event_once(
            &tx,
            crate::store::schema::EventInput {
                thread: &ThreadId::new("t"),
                key: &format!("legacy-warning-{label}"),
                kind: "warn",
                payload_json: r#"{"event":"overdue"}"#,
                decision_at: f.clock.utc_now(),
                source_message: None,
                source_invitation: Some(&invitation),
            },
        )
        .unwrap();
        assert!(inserted);
        let seq: i64 = tx
            .query_row(
                "SELECT decision_seq FROM messages WHERE id=?1",
                [warning.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        let high: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM membership_intervals",
                [],
                |r| r.get(0),
            )
            .unwrap();
        tx.execute("INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,status,phase) VALUES (?1,?2,'t',?3,?4,'invitation',?5,'complete','complete')", params![warning.as_str(), seq, high, affected, invitation.as_str()]).unwrap();
        tx.execute(
            "INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES (?1,'s',?2)",
            params![warning.as_str(), seq],
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            crate::store::attention::informational_notice_pending(&f.db, "s", warning.as_str())
                .unwrap(),
            None,
            "historical scalar-offer class, not a modern Notice"
        );
        warning
    }

    fn fixture_warning() -> (Fixture, CallerClaim, MessageId) {
        let mut f = fixture();
        let pi = Harness::Agent(crate::harness::registry::builtins().agent("pi").unwrap());
        let owner = check_in(
            &f,
            fixture_claim(&f, "s", "p", pi, "legacy-pi-reader"),
            "legacy-pi-start",
        );
        let warning = retained_warning(&mut f, "first", "c");
        (f, owner, warning)
    }

    fn candidate(f: &Fixture) -> crate::ports::WakeCandidate {
        f.store
            .wake_candidates(
                crate::test_support::test_host_id(),
                Default::default(),
                &budget(),
            )
            .unwrap()
            .items
            .into_iter()
            .find(|v| v.seat.as_str() == "s")
            .expect("Pi wake candidate")
    }
    fn offer(f: &Fixture, owner: &CallerClaim) -> Option<i64> {
        let binding = binding_for_claim(f, owner);
        f.db.query_row(
            "SELECT offered_through_seq FROM warning_offer WHERE binding_id=?1",
            [binding.as_str()],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
    }
    fn page(f: &Fixture, owner: &CallerClaim) -> DeliveryPageV2 {
        page_mode(f, owner, true)
    }
    fn page_mode(f: &Fixture, owner: &CallerClaim, deferred: bool) -> DeliveryPageV2 {
        let binding = binding_for_claim(f, owner);
        for _ in 0..32 {
            let mut input =
                super::super::legacy_confirm_tests::watch_input(f, owner, &binding, deferred);
            input.new_message_slots = 0;
            match f.store.presentation_pull(input, &budget()).unwrap().reply {
                DeliveryPullReplyV2::Page(page)
                    if page
                        .items
                        .iter()
                        .any(|v| matches!(v, PresentationItemV2::Attention { .. })) =>
                {
                    return page;
                }
                DeliveryPullReplyV2::Page(_) | DeliveryPullReplyV2::Unchanged { .. } => {}
                other => panic!("legacy attention: {other:?}"),
            }
        }
        panic!("bounded legacy relevance did not produce Attention")
    }
    fn request(owner: &CallerClaim, page: &DeliveryPageV2) -> DeliveryPresentationConfirmV2 {
        DeliveryPresentationConfirmV2 {
            claim: owner.clone(),
            consumer: PresentationConsumer::Watch,
            presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
            token: page.presentation_token.clone().unwrap(),
            emitted_controls: page.emitted_controls.clone(),
        }
    }
    fn confirm(
        f: &Fixture,
        owner: &CallerClaim,
        page: &DeliveryPageV2,
    ) -> DeliveryPresentationConfirmationV2 {
        f.store
            .presentation_confirm(&request(owner, page), &budget())
            .unwrap()
            .status
    }

    #[test]
    fn exact_pi_attention_offer_stops_legacy_warning_wake_without_settling_obligation() {
        let (f, owner, warning) = fixture_warning();
        let before = candidate(&f);
        assert!(
            !before.has_pending_invitation
                && !before.has_pending_receipt
                && before.pending_lazy().is_none(),
            "only the retained legacy warning can supply this wake"
        );
        assert!(before.actionable_warning_seq.is_some() && before.has_actionable_work());
        let owner = check_in(&f, owner, "legacy-pi-observer");
        assert_eq!(
            offer(&f, &owner),
            None,
            "observer guidance is not a warning offer"
        );
        let first = page(&f, &owner);
        assert!(
            !first
                .items
                .iter()
                .any(|v| matches!(v, PresentationItemV2::Notice { .. })),
            "legacy warning is represented by generic Attention, not a Notice body"
        );
        assert_eq!(offer(&f, &owner), None, "allocation is not presentation");
        assert!(candidate(&f).has_actionable_work());
        let replay = page(&f, &owner);
        assert_eq!(first.presentation_token, replay.presentation_token);
        assert!(
            candidate(&f).has_actionable_work(),
            "lost output cannot quiet a wake"
        );
        assert_eq!(
            confirm(&f, &owner, &first),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert!(offer(&f, &owner).is_some());
        let after = f
            .store
            .wake_candidates(
                crate::test_support::test_host_id(),
                Default::default(),
                &budget(),
            )
            .unwrap();
        assert!(
            after.items.iter().all(|v| v.seat != owner.seat),
            "honest generic Attention offer suppresses repeated legacy wake"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT state FROM invitations WHERE id='legacy-first'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
        let effective = crate::store::effective::effective_warning_by_id(&f.db, warning.as_str())
            .unwrap()
            .unwrap();
        assert!(
            crate::store::effective::warning_condition_actionable(&f.db, &effective).unwrap(),
            "offer is neither condition closure nor receipt settlement"
        );
    }

    #[test]
    fn old_attention_offer_does_not_sweep_newer_legacy_warning_or_duplicate_confirmation() {
        let (mut f, owner, _) = fixture_warning();
        let first = page(&f, &owner);
        let newer = retained_warning(&mut f, "newer", "x");
        let seq: i64 =
            f.db.query_row(
                "SELECT decision_seq FROM messages WHERE id=?1",
                [newer.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            confirm(&f, &owner, &first),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert!(
            offer(&f, &owner).unwrap() < seq,
            "confirm must use allocation's frozen cutoff"
        );
        let current = candidate(&f);
        assert_eq!(current.actionable_warning_seq, Some(seq as u64));
        assert!(!current.warning_offered_for_current_occupant() && current.has_actionable_work());
        let captured = offer(&f, &owner);
        assert_eq!(
            confirm(&f, &owner, &first),
            DeliveryPresentationConfirmationV2::AlreadyConfirmed
        );
        assert_eq!(offer(&f, &owner), captured);
        assert!(candidate(&f).has_actionable_work());
    }

    #[test]
    fn captured_legacy_offer_does_not_cover_late_modern_notice_attribution() {
        use crate::protocol::authority::ObligationRef;
        let (mut f, owner, _) = fixture_warning();
        let author = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "modern-author"),
            "modern-author-start",
        );
        let CommandResult::MessageSent(message) = f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    priority: None,
                    delivery_mode: DeliveryMode::Ordinary,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: "real modern overdue cause".into(),
                    direct_recipient: None,
                    invited_recipients: vec![owner.seat.clone()],
                    deadline_millis: Some(1000),
                    operation: OperationId::new("modern-late-cause"),
                    claim: author,
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("modern cause")
        };
        f.clock.0.fetch_add(2000, Ordering::SeqCst);
        let modern = {
            let tx = f.db.transaction().unwrap();
            let result = crate::store::schema::record_overdue_if_pending(
                &tx,
                &ObligationRef::Receipt {
                    message: message.clone(),
                    seat: owner.seat.clone(),
                },
                &crate::ports::TimeBasis::Decision,
                f.clock.utc_now(),
            )
            .unwrap();
            assert!(result.inserted);
            tx.commit().unwrap();
            result.warning.unwrap()
        };
        assert_eq!(
            crate::store::attention::informational_notice_pending(&f.db, "s", modern.as_str())
                .unwrap(),
            Some(true)
        );
        assert_eq!(f.db.query_row("SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='s' AND warning_id=?1", [modern.as_str()], |r| r.get::<_, i64>(0)).unwrap(), 0, "current producer has published but not attributed this Notice");
        let first = page(&f, &owner);
        let seq: i64 =
            f.db.query_row(
                "SELECT decision_seq FROM messages WHERE id=?1",
                [modern.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            confirm(&f, &owner, &first),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert!(
            offer(&f, &owner).unwrap() >= seq,
            "modern publication predates the captured legacy cutoff"
        );
        finish_presentation_attention_job(&f, &format!("work:{}", modern.as_str()));
        assert_eq!(
            crate::store::attention::informational_notice_pending(&f.db, "s", modern.as_str())
                .unwrap(),
            Some(true)
        );
        let wake = candidate(&f);
        assert!(
            !wake.warning_offered_for_current_occupant(),
            "late modern Notice is independent of the legacy scalar offer"
        );
        assert!(wake.actionable_warning_seq.is_some_and(|v| v >= seq as u64));
        assert!(wake.has_actionable_work());
        assert_eq!(
            f.db.query_row(
                "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='s'",
                [message.as_str()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
    }

    #[test]
    fn manual_legacy_attention_cannot_be_replayed_by_automatic_watch() {
        let (f, owner, _) = fixture_warning();
        let first = page(&f, &owner);
        let binding = binding_for_claim(&f, &owner);
        let default = super::super::legacy_confirm_tests::watch_input(&f, &owner, &binding, false);
        assert!(matches!(
            f.store.presentation_pull(default, &budget()).unwrap().reply,
            DeliveryPullReplyV2::Refused {
                reason: crate::protocol::delivery::DeliveryRefusalReason::Busy
            }
        ));
        assert_eq!(offer(&f, &owner), None);
        assert_eq!(
            page(&f, &owner).presentation_token,
            first.presentation_token
        );
        assert_eq!(
            confirm(&f, &owner, &first),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        let default = super::super::legacy_confirm_tests::watch_input(&f, &owner, &binding, false);
        assert!(!matches!(
            f.store.presentation_pull(default, &budget()).unwrap().reply,
            DeliveryPullReplyV2::Refused { .. }
        ));
    }

    #[test]
    fn omitted_foreign_and_stale_controls_cannot_create_a_legacy_offer() {
        let (f, owner, _) = fixture_warning();
        let first = page(&f, &owner);
        let mut absent = request(&owner, &first);
        absent.emitted_controls.clear();
        f.store.presentation_confirm(&absent, &budget()).unwrap();
        assert_eq!(offer(&f, &owner), None);
        let mut foreign = request(&owner, &first);
        foreign.emitted_controls = vec![PresentationControlStampV2::Attention {
            epoch: "999".into(),
            version: 0,
        }];
        assert!(f.store.presentation_confirm(&foreign, &budget()).is_err());
        let mut token = request(&owner, &first);
        token.token = "f".repeat(32);
        assert_eq!(
            f.store
                .presentation_confirm(&token, &budget())
                .unwrap()
                .status,
            DeliveryPresentationConfirmationV2::NotCurrentPage
        );
        let mut stale = request(&owner, &first);
        stale.claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000099");
        assert_eq!(
            f.store
                .presentation_confirm(&stale, &budget())
                .unwrap()
                .status,
            DeliveryPresentationConfirmationV2::StaleBinding
        );
        assert_eq!(offer(&f, &owner), None);
        assert!(candidate(&f).has_actionable_work());
    }

    #[test]
    fn default_watch_and_other_harnesses_keep_historical_attention_offer_behavior() {
        let (f, owner, _) = fixture_warning();
        let default = page_mode(&f, &owner, false);
        assert_eq!(
            confirm(&f, &owner, &default),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert_eq!(
            offer(&f, &owner),
            None,
            "default watch has no new captured offer"
        );
        assert!(candidate(&f).has_actionable_work());

        let mut f = fixture();
        let owner = check_in(
            &f,
            fixture_claim(&f, "s", "p", fourth(), "default-reader"),
            "default-reader-start",
        );
        retained_warning(&mut f, "other-harness", "c");
        let before = offer(&f, &owner);
        let manual = page(&f, &owner);
        assert_eq!(
            confirm(&f, &owner, &manual),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert_eq!(
            offer(&f, &owner),
            before,
            "non-channel-owned policy is unchanged"
        );
        assert!(candidate(&f).has_actionable_work());
    }

    #[test]
    fn cleared_only_confirmation_cannot_offer_an_unobserved_legacy_attention() {
        let (f, owner, _) = fixture_warning();
        let _unobserved = page(&f, &owner);
        f.db.execute(
            "UPDATE invitations SET state='already_joined' WHERE id='legacy-first'",
            [],
        )
        .unwrap();
        let binding = binding_for_claim(&f, &owner);
        for _ in 0..32 {
            let input = super::super::legacy_confirm_tests::watch_input(&f, &owner, &binding, true);
            if let DeliveryPullReplyV2::Page(clear) =
                f.store.presentation_pull(input, &budget()).unwrap().reply
                && clear
                    .items
                    .iter()
                    .any(|v| matches!(v, PresentationItemV2::AttentionCleared { .. }))
            {
                assert!(
                    !clear
                        .items
                        .iter()
                        .any(|v| matches!(v, PresentationItemV2::Attention { .. }))
                );
                assert_eq!(
                    confirm(&f, &owner, &clear),
                    DeliveryPresentationConfirmationV2::Confirmed
                );
                assert_eq!(
                    offer(&f, &owner),
                    None,
                    "clearing an episode does not claim its Attention was observed"
                );
                return;
            }
        }
        panic!("bounded closed legacy episode did not produce Cleared");
    }

    #[test]
    fn notice_confirmation_without_attention_cannot_create_a_legacy_warning_offer() {
        let (f, _, _, _, _) = published_notice();
        let pi = Harness::Agent(crate::harness::registry::builtins().agent("pi").unwrap());
        let mut claim = fixture_claim(&f, "s", "p", pi, "pi-notice-only");
        claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
        let owner = check_in(&f, claim, "pi-notice-only-start");
        let binding = binding_for_claim(&f, &owner);
        for _ in 0..32 {
            let input = super::super::legacy_confirm_tests::watch_input(&f, &owner, &binding, true);
            if let DeliveryPullReplyV2::Page(notices) =
                f.store.presentation_pull(input, &budget()).unwrap().reply
                && notices
                    .items
                    .iter()
                    .any(|v| matches!(v, PresentationItemV2::Notice { .. }))
            {
                let mut bodies_only = request(&owner, &notices);
                bodies_only.emitted_controls.clear();
                assert_eq!(
                    f.store
                        .presentation_confirm(&bodies_only, &budget())
                        .unwrap()
                        .status,
                    DeliveryPresentationConfirmationV2::Confirmed
                );
                assert_eq!(offer(&f, &owner), None);
                assert!(
                    crate::store::attention::binding_notice_frontier(&f.db, binding.as_str())
                        .unwrap()
                        > 0
                );
                return;
            }
        }
        panic!("bounded canonical Notice page missing");
    }
}
