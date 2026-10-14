//! Q4 controls: real text body output and private Lazy continuation before acceptance.
//! Mounted under tests/service/delivery_pull.rs; independent of the Q3 control module.
use super::*;
use crate::{
    cli::{journal::Journal, run_cooperative},
    delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
    harness::context::{
        ContextJournal, Harness as ContextHarness, OccupantContext, Role, SessionReference,
    },
    ports::{LocalClient, PresentationInput, PresentationOutput},
    protocol::{
        commands::{
            DeliveryMode, EnsureCreationPolicy, EnsureDirectThread, InvitationConfirmation,
            SendMessage,
        },
        delivery::{
            DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationItemV2,
            PresentationRenderRequest,
        },
        output::OutputSpec,
        results::{ApiError, MessageContent, MessageDetails},
    },
};
use rusqlite::{OptionalExtension, types::Value};
use std::{
    io::{self, Write},
    sync::{Mutex, atomic::AtomicBool},
    time::Duration,
};

struct BodyFixture {
    service: DomainService,
    store: Arc<dyn StorePort>,
    clock: Arc<TestClock>,
    db: Connection,
    directory: std::path::PathBuf,
    instance: uuid::Uuid,
    sender: CallerClaim,
    a: CallerClaim,
    b: CallerClaim,
    thread: ThreadId,
}
impl Drop for BodyFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl BodyFixture {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let instance = uuid::Uuid::new_v4();
        let temp_base = std::env::temp_dir().canonicalize().unwrap();
        let directory = temp_base.join(format!("q4-body-{instance}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
        let shared: Arc<dyn Clock> = clock.clone();
        let context = StoreContext::new(directory.join("store.db"), shared.clone());
        let db = context.open_writer().unwrap();
        db.pragma_update(
            None,
            "application_id",
            crate::store::control::LEGACY_PAIR_MARKER,
        )
        .unwrap();
        db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES(?1,0,'b',1,3)", [instance.to_string()]).unwrap();
        for (seat, target) in [("sender", "px"), ("reader", "pc")] {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES(?1,?2,'resolved','native',?3,1,0,0)", params![seat,instance.to_string(),target]).unwrap();
            db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES(?1,?2,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc','coherent_enumeration',1)", params![instance.to_string(),target]).unwrap();
        }
        let store: Arc<dyn StorePort> = Arc::new(
            SqliteStore::new(context, instance.to_string(), StoreSettings::default()).unwrap(),
        );
        let service = DomainService::new(instance.to_string(), store.clone(), shared)
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
        let enroll = |seat: &str, target: &str, session: &str, pid: u32| {
            let claim = CallerClaim {
                instance: instance.to_string(),
                seat: SeatId::new(seat),
                binding_generation: 1,
                harness: Harness::Claude,
                role: CallerRole::TopLevel,
                native_session: NativeSessionId::new(session),
                agent_id: None,
                execution: ExecutionId::new(
                    crate::harness::context::session_execution(
                        instance,
                        ContextHarness::Claude,
                        session,
                        None,
                    )
                    .to_string(),
                ),
                target: HostTargetId::new(target),
                liveness: Some(crate::protocol::authority::LivenessFingerprint { pid, start: 1 }),
            };
            match service
                .handle(
                    Command::CheckIn(CheckIn {
                        mode: CheckInMode::Lifecycle {
                            expected_binding_generation: claim.binding_generation,
                        },
                        claim,
                        operation: OperationId::new(format!("ci-{session}")),
                        lineage_parent: None,
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap()
            {
                CommandResult::CheckedIn(result) => result.context,
                other => panic!("canonical enrollment: {other:?}"),
            }
        };
        let sender = enroll("sender", "px", "sender", 1001);
        let a = enroll("reader", "pc", "reader-a", 2001);
        let b = enroll("reader", "pc", "reader-b", 2002);
        let thread = match service
            .handle(
                Command::EnsureDirectThread(EnsureDirectThread {
                    creation_policy: Some(EnsureCreationPolicy {
                        confirmation: InvitationConfirmation::Lazy,
                        deadline_millis: None,
                    }),
                    recipient: a.seat.clone(),
                    operation: OperationId::new("ensure-pair"),
                    claim: sender.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        {
            CommandResult::DirectThreadEnsured(result) => result.thread,
            other => panic!("canonical direct Ensure: {other:?}"),
        };
        let this = Self {
            service,
            store,
            clock,
            db,
            directory,
            instance,
            sender,
            a,
            b,
            thread,
        };
        this.assert_pending();
        assert_eq!((this.cursor(&this.a), this.cursor(&this.b)), (None, None));
        this
    }
    fn send(&self, operation: &str, body: &str) -> MessageId {
        match self
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    priority: None,
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: self.thread.clone(),
                    body: body.into(),
                    direct_recipient: Some(self.a.seat.clone()),
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(operation),
                    claim: self.sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        {
            CommandResult::MessageSent(id) => id,
            other => panic!("canonical direct Lazy send: {other:?}"),
        }
    }
    fn binding(&self, claim: &CallerClaim) -> BindingId {
        BindingId::new(self.db.query_row("SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 AND native_session=?3 AND agent_id='' AND role='top_level' AND ended_at IS NULL AND liveness='connected'", params![claim.seat.as_str(),claim.harness.as_str(),claim.native_session.as_str()], |r| r.get::<_,String>(0)).unwrap())
    }
    fn cursor(&self, claim: &CallerClaim) -> Option<i64> {
        self.db
            .query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
                params![self.binding(claim).as_str(), self.thread.as_str()],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
    }
    fn sequence(&self, message: &MessageId) -> i64 {
        self.db
            .query_row(
                "SELECT sequence FROM messages WHERE id=?1",
                [message.as_str()],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn consumer(&self, claim: &CallerClaim) -> Vec<(String, Value)> {
        let mut stmt = self.db.prepare("SELECT * FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'").unwrap();
        let names = stmt
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        stmt.query_row([self.binding(claim).as_str()], |row| {
            names
                .iter()
                .enumerate()
                .map(|(n, name)| Ok((name.clone(), row.get(n)?)))
                .collect()
        })
        .unwrap()
    }
    fn private_pull(&self, claim: &CallerClaim) -> PresentationOutput {
        let frozen_render = self
            .store
            .presentation_render_selection(claim, PresentationConsumer::Watch, &budget())
            .unwrap()
            .map(|identity| FrozenPresentationRender {
                identity,
                codec: None,
            });
        self.store
            .presentation_pull(
                PresentationInput {
                    binding_id: self.binding(claim),
                    owner_replay: None,
                    ordinary_due_snapshot: None,
                    frozen_render,
                    render_profile: PresentationRenderProfile::WatchJsonLinesV1,
                    pull: DeliveryPullV2 {
                        claim: claim.clone(),
                        consumer: PresentationConsumer::Watch,
                        render: PresentationRenderRequest::WatchJsonLinesV1,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        marker_prefix: vec![],
                    },
                    attention_version: 0,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: true,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap()
    }
    fn assert_pending(&self) {
        let state: (String,String,i64,i64) = self.db.query_row("SELECT m.state,i.state,(SELECT count(*) FROM membership_intervals mi WHERE mi.thread_id=m.thread_id AND mi.seat_id=m.seat_id),(SELECT COALESCE(MAX(attention_version),0) FROM wake_work WHERE seat_id=m.seat_id) FROM memberships m JOIN invitations i ON i.thread_id=m.thread_id AND i.seat_id=m.seat_id AND i.episode=m.episode WHERE m.thread_id=?1 AND m.seat_id=?2", params![self.thread.as_str(),self.a.seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(state, ("invited".into(), "pending".into(), 0, 0));
        let receipts: i64 = self.db.query_row("SELECT (SELECT count(*) FROM receipts WHERE seat_id=?1)+(SELECT count(*) FROM receipt_state WHERE seat_id=?1)", [self.a.seat.as_str()], |r| r.get(0)).unwrap();
        assert_eq!(receipts, 0);
    }
    fn cli<W: Write>(
        &self,
        claim: &CallerClaim,
        client: &BodyClient<'_>,
        argv: &[String],
        writer: &mut W,
    ) -> Result<(), crate::cli::RunError> {
        use std::os::unix::fs::PermissionsExt;
        let session_root = self.directory.join(claim.native_session.as_str());
        std::fs::create_dir_all(&session_root).unwrap();
        std::fs::set_permissions(&session_root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = OccupantContext {
            format_version: 1,
            instance: self.instance,
            seat: claim.seat.as_str().into(),
            target: claim.target.as_str().into(),
            harness: ContextHarness::Claude,
            binding_generation: claim.binding_generation,
            execution: uuid::Uuid::parse_str(claim.execution.as_str()).unwrap(),
            session: SessionReference::Native(claim.native_session.as_str().into()),
            role: Role::TopLevel,
        };
        let contexts = ContextJournal::open(
            &session_root,
            self.instance,
            claim.seat.as_str(),
            Duration::from_millis(20),
        )
        .unwrap()
        .with_agent_caller(context, claim.liveness.unwrap())
        .unwrap();
        let journal = Journal::open(self.directory.join("intents")).unwrap();
        run_cooperative(
            crate::cli::commands::parse_argv(argv.iter().map(String::as_str)).unwrap(),
            &journal,
            &contexts,
            None,
            Role::TopLevel,
            client,
            self.clock.as_ref(),
            writer,
        )
    }
}

struct BodyClient<'a> {
    service: &'a DomainService,
    flushed: &'a AtomicBool,
    calls: Mutex<Vec<Command>>,
    last: Mutex<Option<MessageDetails>>,
}
impl LocalClient for BodyClient<'_> {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        [
            crate::protocol::capabilities::BINDING_CURSORS,
            crate::protocol::capabilities::MESSAGE_DELIVERY_MODES,
        ]
        .contains(&name)
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.call_with_output(command, &OutputSpec::default(), budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        if matches!(command, Command::AdvanceCursors(_)) {
            assert!(
                self.flushed.load(Ordering::SeqCst),
                "canonical display mutation must follow successful stdout flush"
            );
        }
        self.calls.lock().unwrap().push(command.clone());
        let result = self
            .service
            .handle_with_output(command, peer(), budget, output)?;
        if let CommandResult::Message(message) = &result {
            *self.last.lock().unwrap() = Some(message.clone());
        }
        Ok(result)
    }
}
enum Failure {
    None,
    Write,
    Flush,
}
struct BodyWriter<'a> {
    flushed: &'a AtomicBool,
    failure: Failure,
    bytes: Vec<u8>,
}
impl Write for BodyWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if matches!(self.failure, Failure::Write) {
            if !self.bytes.is_empty() {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            }
            self.bytes.extend_from_slice(&bytes[..bytes.len().min(3)]);
            return Ok(bytes.len().min(3));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if matches!(self.failure, Failure::Flush) {
            return Err(io::Error::other("body fixture flush"));
        }
        self.flushed.store(true, Ordering::SeqCst);
        Ok(())
    }
}
fn long_utf8_body() -> String {
    format!("{}tail", "π💡".repeat(10_922))
}
fn body_argv(message: &MessageId, extra: &[&str]) -> Vec<String> {
    let mut argv = vec![
        "herdr-threads".into(),
        "body".into(),
        message.as_str().into(),
    ];
    argv.extend(extra.iter().map(|s| (*s).to_owned()));
    argv
}

#[test]
fn preaccept_utf8_body_final_flushed_text_advances_only_its_binding_and_invalidates_its_consumer() {
    let f = BodyFixture::new();
    let body = long_utf8_body();
    assert_eq!(body.len(), 65_536);
    let message = f.send("long-body", &body);
    f.private_pull(&f.a);
    f.private_pull(&f.b);
    let before_a = f.consumer(&f.a);
    let before_b = f.consumer(&f.b);
    let flushed = AtomicBool::new(false);
    let client = BodyClient {
        service: &f.service,
        flushed: &flushed,
        calls: Mutex::new(vec![]),
        last: Mutex::new(None),
    };
    let mut argv = body_argv(&message, &["--max-bytes", "2048"]);
    let mut exact_body = String::new();
    let mut complete = false;
    for step in 0..128 {
        let invocation = crate::cli::commands::parse_argv(argv.iter().map(String::as_str)).unwrap();
        let crate::cli::commands::CliAction::Wire(Command::Message(request)) = &invocation.action
        else {
            panic!("printed continuation must use the public Body route")
        };
        assert_eq!(request.message, message);
        assert_eq!(
            request.body.max_bytes, 2048,
            "printed continuation changed original bound at step {step}: {argv:?}"
        );
        flushed.store(false, Ordering::SeqCst);
        let mut writer = BodyWriter {
            flushed: &flushed,
            failure: Failure::None,
            bytes: vec![],
        };
        f.cli(&f.a, &client, &argv, &mut writer)
            .unwrap_or_else(|error| {
                let last_offset = client.last.lock().unwrap().as_ref().and_then(|details| {
                    if let MessageContent::Ordinary { body_offset, .. } = details.content {
                        Some(body_offset)
                    } else {
                        None
                    }
                });
                panic!(
                    "body output step={step} argv={argv:?} last_offset={last_offset:?}: {error:?}"
                );
            });
        assert!(flushed.load(Ordering::SeqCst));
        let details = client.last.lock().unwrap().take().unwrap();
        let MessageContent::Ordinary {
            body_data,
            body_offset,
            body_complete,
            body_next_argv,
            body_next_cursor,
            body_total_bytes,
        } = details.content
        else {
            panic!("ordinary body");
        };
        assert_eq!(body_total_bytes, 65_536);
        assert_eq!(body_offset as usize, exact_body.len());
        assert!(body.is_char_boundary(body_offset as usize));
        assert!(
            writer.bytes.len() <= 2048,
            "actual final Body stdout exceeds original byte bound at step {step}"
        );
        let output = std::str::from_utf8(&writer.bytes).unwrap();
        assert!(output.contains(&body_data));
        let printed_body = output
            .lines()
            .filter_map(|line| line.strip_prefix("  "))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            printed_body, body_data,
            "actual printed UTF8 body differs from canonical selected bytes"
        );
        assert!(
            !printed_body.is_empty(),
            "continued Body read made no byte progress"
        );
        assert_eq!(
            body.get(body_offset as usize..body_offset as usize + printed_body.len()),
            Some(printed_body.as_str())
        );
        exact_body.push_str(&printed_body);
        assert!(body.is_char_boundary(exact_body.len()));
        let printed_next: Vec<_> = output
            .lines()
            .filter_map(|line| line.strip_prefix("more: "))
            .collect();
        if body_complete {
            assert!(
                printed_next.is_empty(),
                "complete body printed a continuation"
            );
            assert!(body_next_cursor.is_none());
            assert!(body_next_argv.is_none());
            complete = true;
            break;
        }
        assert_eq!(f.cursor(&f.a), None, "partial output is not whole display");
        assert_eq!(f.consumer(&f.a), before_a);
        assert_eq!(f.consumer(&f.b), before_b);
        assert_eq!(
            printed_next.len(),
            1,
            "incomplete body must print one actual continuation"
        );
        let emitted = shlex::split(printed_next[0])
            .expect("printed Body continuation must retain shell argument boundaries");
        let next = crate::cli::commands::parse_argv(emitted.iter().map(String::as_str)).unwrap();
        let crate::cli::commands::CliAction::Wire(Command::Message(next_request)) = &next.action
        else {
            panic!("printed continuation changed Body action")
        };
        assert_eq!(next_request.message, message);
        assert_eq!(
            next_request.body.max_bytes, 2048,
            "backend bound escaped into actual stdout continuation at step {step}"
        );
        assert_eq!(
            next_request.body.cursor, body_next_cursor,
            "printed opaque Body cursor differs from raw backend continuation"
        );
        assert!(next_request.body.cursor.is_some());
        assert!(next_request.body.offset.is_none());
        assert_eq!(next.actor, invocation.actor);
        assert_eq!(next.output, invocation.output);
        let mut expected = body_next_argv.expect("raw backend continuation metadata");
        assert_eq!(
            expected
                .iter()
                .filter(|arg| arg.as_str() == "--max-bytes")
                .count(),
            1
        );
        let bound = expected
            .iter()
            .position(|arg| arg == "--max-bytes")
            .unwrap()
            + 1;
        expected[bound] = "2048".into();
        assert_eq!(
            emitted, expected,
            "printed non-budget argv tokens/order changed"
        );
        // Follow the model-visible command. BodyClient.last intentionally keeps
        // the pre-normalization backend result as independent byte/cursor evidence.
        argv = emitted;
    }
    assert!(complete);
    assert_eq!(exact_body, body);
    assert_eq!(f.cursor(&f.a), Some(f.sequence(&message)));
    assert_eq!(f.cursor(&f.b), None);
    let mut expected_a = before_a;
    for (name, value) in &mut expected_a {
        if ["ordinary_passive_generation", "mod_version"].contains(&name.as_str()) {
            let Value::Integer(n) = value else {
                panic!("canonical private counter");
            };
            *n += 1;
        }
    }
    assert_eq!(
        f.consumer(&f.a),
        expected_a,
        "display invalidates passively, preserving capture/carry/frontiers"
    );
    assert_eq!(f.consumer(&f.b), before_b);
    let calls = client.calls.lock().unwrap();
    let advances = calls
        .iter()
        .filter_map(|c| {
            if let Command::AdvanceCursors(a) = c {
                Some(a)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(advances.len(), 1);
    assert_eq!(advances[0].claim, f.a);
    assert_eq!(advances[0].displayed, vec![message]);
    assert!(calls.iter().all(|c| matches!(
        c,
        Command::Message(_) | Command::MessageDeliveryModes(_) | Command::AdvanceCursors(_)
    )));
    f.assert_pending();
}

#[test]
fn preaccept_body_json_machine_partial_and_failed_stdout_do_not_advance_or_invalidate() {
    let f = BodyFixture::new();
    let message = f.send("negative-body", &"π💡".repeat(1000));
    f.private_pull(&f.a);
    f.private_pull(&f.b);
    let (before_a, before_b) = (f.consumer(&f.a), f.consumer(&f.b));
    let flushed = AtomicBool::new(false);
    let client = BodyClient {
        service: &f.service,
        flushed: &flushed,
        calls: Mutex::new(vec![]),
        last: Mutex::new(None),
    };
    for (extra, failure) in [
        (vec!["--json"], Failure::None),
        (vec!["--machine"], Failure::None),
        (vec!["--max-bytes", "1024"], Failure::None),
        (vec!["--offset", "6"], Failure::None),
        (vec![], Failure::Write),
        (vec![], Failure::Flush),
    ] {
        flushed.store(false, Ordering::SeqCst);
        let fails = !matches!(failure, Failure::None);
        let mut writer = BodyWriter {
            flushed: &flushed,
            failure,
            bytes: vec![],
        };
        let result = f.cli(&f.a, &client, &body_argv(&message, &extra), &mut writer);
        assert_eq!(result.is_err(), fails);
        assert_eq!((f.cursor(&f.a), f.cursor(&f.b)), (None, None));
        assert_eq!(
            (f.consumer(&f.a), f.consumer(&f.b)),
            (before_a.clone(), before_b.clone())
        );
        f.assert_pending();
    }
    assert!(
        client
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| matches!(c, Command::Message(_) | Command::MessageDeliveryModes(_))),
        "read-only or failed outputs dispatch no mutations"
    );
}

#[test]
fn private_captured_lazy_tail_resumes_preaccept_after_sibling_full_text_display() {
    use crate::protocol::{
        delivery::DeliveryPresentationConfirmV2,
        pagination::{InboxBatchV2CursorState, InboxBatchV2Source},
    };
    let f = BodyFixture::new();
    let message = f.send("captured-long", &long_utf8_body());
    let later = f.send("following", "following targeted Lazy body");
    let initial = f.private_pull(&f.b);
    let DeliveryPullReplyV2::Page(first) = initial.reply else {
        panic!("initial private body page");
    };
    assert!(first.items.iter().any(|item| matches!(item,PresentationItemV2::Message { id,lazy:true,content } if id==&message && content.truncated)));
    let cursor: String = f.db.query_row("SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",[f.binding(&f.b).as_str()],|r|r.get(0)).unwrap();
    let captured =
        InboxBatchV2CursorState::decode_for(&cursor, &f.instance.to_string(), &f.b.seat).unwrap();
    let (source, tail) = captured
        .body
        .expect("real producer must retain the long Lazy continuation");
    assert_eq!(source, InboxBatchV2Source::Lazy);
    assert_eq!(tail.message, message);
    assert!(tail.offset > 0);
    assert!(long_utf8_body().is_char_boundary(tail.offset as usize));
    if let Some(token) = first.presentation_token {
        f.store
            .presentation_confirm(
                &DeliveryPresentationConfirmV2 {
                    claim: f.b.clone(),
                    consumer: PresentationConsumer::Watch,
                    presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                    token,
                    emitted_controls: first.emitted_controls,
                },
                &budget(),
            )
            .unwrap();
    }
    let before_b = f.consumer(&f.b);
    let flushed = AtomicBool::new(false);
    let client = BodyClient {
        service: &f.service,
        flushed: &flushed,
        calls: Mutex::new(vec![]),
        last: Mutex::new(None),
    };
    let mut argv = body_argv(&message, &["--max-bytes", "65536"]);
    for _ in 0..4 {
        flushed.store(false, Ordering::SeqCst);
        let mut writer = BodyWriter {
            flushed: &flushed,
            failure: Failure::None,
            bytes: vec![],
        };
        f.cli(&f.a, &client, &argv, &mut writer).unwrap();
        let details = client.last.lock().unwrap().take().unwrap();
        let MessageContent::Ordinary {
            body_complete,
            body_next_argv,
            ..
        } = details.content
        else {
            panic!("ordinary body");
        };
        if body_complete {
            break;
        }
        argv = body_next_argv.unwrap();
    }
    assert_eq!(f.cursor(&f.a), Some(f.sequence(&message)));
    assert_eq!(f.cursor(&f.b), None);
    assert_eq!(
        f.consumer(&f.b),
        before_b,
        "sibling display cannot reset captured Lazy continuation"
    );
    let mut following = false;
    for _ in 0..4 {
        let next = f.private_pull(&f.b);
        assert!(next.work.ordinary <= 100);
        assert_eq!(next.work.ordinary_due_key_probes, 0);
        let DeliveryPullReplyV2::Page(page) = next.reply else {
            continue;
        };
        assert!(
            !page
                .items
                .iter()
                .any(|item| matches!(item,PresentationItemV2::Message { id,.. } if id==&message)),
            "a captured truncated body's tail is not a second physical full-body offer"
        );
        following |= page.items.iter().any(|item| matches!(item,PresentationItemV2::Message { id,lazy:true,content } if id==&later && content.body=="following targeted Lazy body"));
        if following {
            break;
        }
    }
    assert!(
        following,
        "private Source::Lazy continuation must consume its captured tail and reach following unread work before accept"
    );
    assert_eq!(
        f.cursor(&f.b),
        None,
        "physical presentation still supplies no display cursor authority"
    );
    f.assert_pending();
}

#[test]
fn private_old_episode_lazy_tail_retires_and_reaches_fresh_preaccept_body() {
    use crate::protocol::{
        commands::{Invite, Reject},
        delivery::{DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2},
        pagination::{InboxBatchV2CursorState, InboxBatchV2Source},
    };
    let f = BodyFixture::new();
    let old = f.send("old-episode-long", &long_utf8_body());
    let initial = f.private_pull(&f.b);
    let DeliveryPullReplyV2::Page(first) = initial.reply else {
        panic!("initial private prefix");
    };
    assert!(first.items.iter().any(|item| matches!(item,PresentationItemV2::Message { id,lazy:true,content } if id==&old && content.truncated && !content.ack_required)));
    let cursor: String = f.db.query_row("SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'", [f.binding(&f.b).as_str()], |r| r.get(0)).unwrap();
    let captured =
        InboxBatchV2CursorState::decode_for(&cursor, &f.instance.to_string(), &f.b.seat).unwrap();
    let (source, tail) = captured.body.expect("real canonical long-body capture");
    assert_eq!(source, InboxBatchV2Source::Lazy);
    assert_eq!(tail.message, old);
    assert!(tail.offset > 0 && long_utf8_body().is_char_boundary(tail.offset as usize));
    let confirm_output = |page: &crate::protocol::delivery::DeliveryPageV2| {
        let mut bytes = Vec::new();
        for line in crate::delivery::presentation::presentation_watch_lines(&page.items) {
            serde_json::to_writer(&mut bytes, &line).unwrap();
            bytes.push(b'\n');
        }
        bytes.flush().unwrap();
        assert_eq!(bytes.len(), page.output_bytes as usize);
        assert!(bytes.len() <= 65_536);
        if let Some(token) = &page.presentation_token {
            let confirmed = f
                .store
                .presentation_confirm(
                    &DeliveryPresentationConfirmV2 {
                        claim: f.b.clone(),
                        consumer: PresentationConsumer::Watch,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        token: token.clone(),
                        emitted_controls: page.emitted_controls.clone(),
                    },
                    &budget(),
                )
                .unwrap();
            assert_eq!(
                confirmed.status,
                DeliveryPresentationConfirmationV2::Confirmed
            );
        }
    };
    confirm_output(&first);
    assert_eq!(
        (f.cursor(&f.a), f.cursor(&f.b)),
        (None, None),
        "physical prefix is not semantic completion"
    );
    let old_rows = || {
        f.db.query_row("SELECT m.sequence,m.delivery_mode,m.actor_seat_id,length(CAST(m.body AS BLOB)),r.state,r.legacy_pending,r.preparation_id FROM messages m JOIN lazy_recipients r ON r.message_id=m.id JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=m.id WHERE m.id=?1 AND r.seat_id=?2", params![old.as_str(),f.b.seat.as_str()], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?,r.get::<_,String>(6)?))).unwrap()
    };
    let before_old = old_rows();
    assert_eq!(before_old.1, "lazy");
    assert_eq!(before_old.2, f.sender.seat.as_str());
    assert_eq!(before_old.3, 65_536);
    assert_eq!(before_old.4, "pending");
    assert_eq!(before_old.5, 0);
    let old_invitation: InvitationId =
        f.db.query_row(
            "SELECT id FROM invitations WHERE thread_id=?1 AND seat_id=?2 AND episode=1",
            params![f.thread.as_str(), f.b.seat.as_str()],
            |r| r.get::<_, String>(0),
        )
        .map(InvitationId::new)
        .unwrap();
    let rejected = f
        .service
        .handle(
            Command::Reject(Reject {
                claim: f.b.clone(),
                thread: f.thread.clone(),
                invitation: old_invitation.clone(),
                reason: "close old episode without displaying the captured remainder".into(),
                operation: OperationId::new("reject-captured-episode"),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert!(matches!(rejected, CommandResult::Rejected(_)));
    let reinvited = f
        .service
        .handle(
            Command::Invite(Invite {
                claim: f.sender.clone(),
                thread: f.thread.clone(),
                seat: f.b.seat.clone(),
                confirmation: Some(InvitationConfirmation::Lazy),
                deadline_millis: None,
                operation: OperationId::new("reinvite-captured-episode"),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::Invitation(current_invitation) = reinvited else {
        panic!("canonical fresh invitation");
    };
    assert_ne!(current_invitation, old_invitation);
    let fresh_body = "new episode starts at byte zero: β🚦";
    let fresh = f.send("fresh-episode-body", fresh_body);
    let scope: (i64,String,String,i64,i64,i64) = f.db.query_row("SELECT i.episode,m.state,COALESCE(m.voluntary_state,m.state),i.created_decision_seq,(SELECT decision_seq FROM messages WHERE id=?3),(SELECT decision_seq FROM messages WHERE id=?4) FROM invitations i JOIN memberships m ON m.thread_id=i.thread_id AND m.seat_id=i.seat_id AND m.episode=i.episode WHERE i.id=?1 AND i.seat_id=?2",params![current_invitation.as_str(),f.b.seat.as_str(),old.as_str(),fresh.as_str()],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        (scope.0, scope.1.as_str(), scope.2.as_str()),
        (2, "invited", "invited")
    );
    assert!(
        scope.4 < scope.3 && scope.3 < scope.5,
        "real current-invitation decision separates old and fresh bodies"
    );
    assert_eq!(
        f.cursor(&f.b),
        None,
        "no display cursor enables the new episode"
    );
    let mut following = false;
    for _ in 0..8 {
        let result = f.private_pull(&f.b);
        assert!(result.work.ordinary <= 100);
        assert_eq!(result.work.ordinary_due_key_probes, 0);
        assert_eq!(result.work.ordinary_carry_key_probes, 0);
        if let DeliveryPullReplyV2::Page(page) = result.reply {
            assert!(
                !page
                    .items
                    .iter()
                    .any(|item| matches!(item,PresentationItemV2::Message { id,.. } if id==&old)),
                "a prior-episode captured tail cannot reappear or supply the fresh body's offset"
            );
            following |= page.items.iter().any(|item| matches!(item,PresentationItemV2::Message { id,lazy:true,content } if id==&fresh && content.body==fresh_body && content.body_len==fresh_body.len() as u64 && !content.truncated && !content.ack_required));
            confirm_output(&page);
        }
        if following {
            break;
        }
    }
    assert!(
        following,
        "private ineligible old-episode continuation must retire and reach the actual fresh body without accepting"
    );
    assert_eq!((f.cursor(&f.a), f.cursor(&f.b)), (None, None));
    assert_eq!(
        old_rows(),
        before_old,
        "retirement keeps canonical historical addressing and origin"
    );
    let retained_old_body: String =
        f.db.query_row(
            "SELECT body FROM messages WHERE id=?1",
            [old.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        retained_old_body == long_utf8_body(),
        "retirement does not rewrite historical body bytes"
    );
    let (skip,partial): (Option<String>,String) = f.db.query_row("SELECT ordinary_skip_body,ordinary_partial FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",[f.binding(&f.b).as_str()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_ne!(skip.as_deref(), Some(old.as_str()));
    let partial: std::collections::HashMap<String, String> =
        serde_json::from_str(&partial).unwrap();
    assert!(!partial.contains_key(old.as_str()));
    f.assert_pending();
}

#[path = "q4_retirement_refusal_controls.rs"]
mod q4_retirement_refusal_controls;
