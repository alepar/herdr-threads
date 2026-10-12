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
use rusqlite::{Connection, params};
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

struct Fixture {
    service: DomainService,
    store: Arc<dyn StorePort>,
    /// The offered ledger the service and (in production) the wake lane share.
    state: Arc<DeliveryPullState>,
    clock: Arc<TestClock>,
    db: Connection,
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
    let directory = std::env::temp_dir().join(format!("delivery-pull-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let shared: Arc<dyn Clock> = clock.clone();
    let context = StoreContext::new(directory.join("store.db"), shared.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3)",
        [],
    )
    .unwrap();
    for (seat, target) in [("s", "p"), ("x", "px"), ("c", "pc")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,1,0,0)", [seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1)", []).unwrap();
    for seat in ["s", "x", "c"] {
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let state = Arc::new(DeliveryPullState::default());
    let service = DomainService::new("i".into(), Arc::clone(&store), shared)
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
    f.service = DomainService::new("i".into(), Arc::clone(&f.store), shared)
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
        instance: "i".into(),
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
        "SELECT generation FROM seats WHERE id=?1 AND instance_id='i' AND target_id=?2 AND state='resolved'",
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
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: claim.binding_generation,
        },
        claim,
        operation: OperationId::new(operation),
        lineage_parent: None,
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    }
}

fn binding_for_claim(f: &Fixture, claim: &CallerClaim) -> BindingId {
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

fn published_notice() -> (Fixture, CallerClaim, InvitationId, MessageId, String) {
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
