// Existing delivery fixtures have an honestly known Agent invocation.
#[allow(clippy::too_many_arguments)]
fn retry_to_writer<C: LocalClient + ?Sized, W: std::io::Write>(
    journal: &Journal,
    reference: &IntentRef,
    namespace: &crate::protocol::handoff::HandoffNamespace,
    client: &C,
    clock: &dyn Clock,
    output: &OutputSpec,
    writer: &mut W,
) -> Result<serde_json::Value, super::super::RunError> {
    super::super::retry::run_delivery_retry_to_writer(
        journal,
        reference,
        super::super::actor_route::InvocationActor::Agent,
        namespace,
        client,
        clock,
        output,
        writer,
    )
}
use super::*;
use crate::protocol::{
    commands::Command,
    handoff::{HandoffChannel, HandoffState, topology_contract_tests as fixture},
    ids::{InvitationId, MessageId, SeatId, ThreadId},
    output::OutputSpec,
    results::{ApiError, CommandResult, ContinuityStatus, ErrorCode, SeatInspection},
    time::{CallBudget, MonoInstant, UtcMillis},
};
use std::sync::Mutex;
struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}
fn page(items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"items":items,"next_cursor":null,"next_argv":null,"high_water_ordinal":1,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"})
}
fn inspection() -> SeatInspection {
    serde_json::from_value(serde_json::json!({
        "summary":{"seat":"recipient","continuity":"resolved","target":"w1:p2","generation":1,"created_at":1,"retired_at":null},
        "mapping":{"state":"resolved","target":"w1:p2","detail_argv":null},
        "hold":null,"retirement":null,"open_binding":{"provenance":"cooperative_top_level","harness":"codex","target":"w1:p2"},"history":page(serde_json::json!([]))
    })).unwrap()
}
struct Client {
    inspection: SeatInspection,
    joined: bool,
    calls: Mutex<Vec<Command>>,
    lost: Mutex<Option<&'static str>>,
    saved: Mutex<std::collections::HashMap<String, CommandResult>>,
    completed: Mutex<bool>,
}
impl Client {
    fn new(joined: bool) -> Self {
        Self {
            inspection: inspection(),
            joined,
            calls: Mutex::new(vec![]),
            lost: Mutex::new(None),
            saved: Mutex::new(Default::default()),
            completed: Mutex::new(false),
        }
    }
}
impl LocalClient for Client {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        if let Command::HandoffDelivery(query) = &c {
            query.validate().unwrap();
            assert!(matches!(
                query.action,
                crate::protocol::handoff::DeliveryAction::Status(_)
            ));
            return if *self.completed.lock().unwrap() {
                Ok(CommandResult::Handoff(
                    crate::protocol::handoff::HandoffResult {
                        compound: query.identity().compound,
                        thread: Some(ThreadId::new("t1")),
                        state: HandoffState::Completed,
                    },
                ))
            } else {
                Err(ApiError::new(
                    ErrorCode::NotFound,
                    "no canonical completed delivery",
                ))
            };
        }
        if matches!(c, Command::Capabilities) {
            return Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: crate::protocol::capabilities::ADVERTISED
                        .iter()
                        .copied()
                        .chain([crate::protocol::capabilities::INVITATION_CONFIRMATION_V1])
                        .map(str::to_owned)
                        .collect(),
                },
            ));
        }
        self.calls.lock().unwrap().push(c.clone());
        let (key, phase, result) = match c {
            Command::Seats(q) => {
                assert_eq!(
                    q.target.as_ref().map(|target| target.as_str()),
                    Some("w1:p2")
                );
                return Ok(CommandResult::Seats(
                    serde_json::from_value(page(serde_json::json!([self.inspection.summary])))
                        .unwrap(),
                ));
            }
            Command::SeatInspect(q) => {
                assert_eq!(q.seat.as_str(), "recipient");
                return Ok(CommandResult::SeatInspect(self.inspection.clone()));
            }
            Command::ResolveThread(q) => {
                assert_eq!(q.selector, "chosen");
                return Ok(CommandResult::ThreadResolved(ThreadId::new("t1")));
            }
            Command::Directory(q) => {
                assert_eq!(q.membership.unwrap().as_str(), "sender");
                return Ok(CommandResult::Directory(serde_json::from_value(page(serde_json::json!([{"thread":"t1","name":null,"topic_data":"topic","topic_omitted":false,"topic_detail_argv":null,"archived":false,"orphaned":false,"message_count":0,"created_at":1,"ordinary_count":0,"system_count":0,"joined_count":1}]))).unwrap()));
            }
            Command::Participants(_) => {
                let rows = if self.joined {
                    serde_json::json!([{"seat":"recipient","episode":1,"joined":true,"retired":false,"physical_state":"joined","effective_state":"joined","joined_at":1,"left_at":null,"retirement_cutover":null,"cleanup_state":null,"accepted_invitation":null}])
                } else {
                    serde_json::json!([])
                };
                return Ok(CommandResult::Participants(
                    serde_json::from_value(page(rows)).unwrap(),
                ));
            }
            Command::BeginHandoff(q) => {
                assert_eq!(q.identity.compound.as_str(), "canonical-compound");
                return Ok(CommandResult::Handoff(
                    crate::protocol::handoff::HandoffResult {
                        compound: q.identity.compound,
                        thread: Some(ThreadId::new("t1")),
                        state: if *self.completed.lock().unwrap() {
                            HandoffState::Completed
                        } else {
                            HandoffState::Live
                        },
                    },
                ));
            }
            Command::CompleteHandoff(q) => {
                *self.completed.lock().unwrap() = true;
                if std::mem::take(&mut *self.lost.lock().unwrap()) == Some("complete") {
                    return Err(ApiError::new(
                        ErrorCode::UnknownOutcome,
                        "completion committed reply lost",
                    ));
                }
                assert_eq!(q.identity.compound.as_str(), "canonical-compound");
                return Ok(CommandResult::Handoff(
                    crate::protocol::handoff::HandoffResult {
                        compound: q.identity.compound,
                        thread: Some(ThreadId::new("t1")),
                        state: HandoffState::Completed,
                    },
                ));
            }
            Command::CreateThread(q) => (
                q.operation,
                "create",
                CommandResult::ThreadCreated(ThreadId::new("t1")),
            ),
            Command::Invite(q) => {
                assert_eq!(q.seat.as_str(), "recipient");
                (
                    q.operation,
                    "invite",
                    CommandResult::Invitation(InvitationId::new("i1")),
                )
            }
            Command::SendMessage(q) => {
                assert_eq!(q.body, "literal '$HOME' work");
                assert_eq!(q.invited_recipients, vec![SeatId::new("recipient")]);
                (
                    q.operation,
                    "send",
                    CommandResult::MessageSent(MessageId::new("m1")),
                )
            }
            other => panic!("forbidden allocation/rebind/registration/native route: {other:?}"),
        };
        let value = self
            .saved
            .lock()
            .unwrap()
            .entry(key.as_str().into())
            .or_insert(result)
            .clone();
        let mut lost = self.lost.lock().unwrap();
        if *lost == Some(phase) {
            *lost = None;
            return Err(ApiError::new(
                ErrorCode::UnknownOutcome,
                "committed reply lost",
            ));
        }
        Ok(value)
    }
}
fn request() -> Request {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--thread",
        "chosen",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong actual parser route")
    };
    request
}
fn plan() -> DeliveryPlan {
    let mut payload = fixture::payload().handoff;
    payload.keys.compound = crate::protocol::ids::OperationId::new("canonical-compound");
    payload.channel = HandoffChannel::Existing {
        thread: ThreadId::new("t1"),
    };
    payload.body = "literal '$HOME' work".into();
    DeliveryPlan {
        version: 1,
        payload,
        recipient: SeatId::new("recipient"),
    }
}
fn canonical_claim() -> CallerClaim {
    let mut claim = fixture::claim();
    claim.execution =
        crate::protocol::ids::ExecutionId::new("00000000-0000-4000-8000-000000000001");
    claim
}
fn record(journal: &Journal, plan: DeliveryPlan) -> IntentRef {
    let claim = canonical_claim();
    journal
        .record(
            super::super::journal::IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            },
            super::super::journal::SemanticMutation::freeze(
                super::super::journal::SemanticMutation::HandoffDelivery(Box::new(plan)),
                claim,
            )
            .unwrap(),
            1,
        )
        .unwrap()
}
#[test]
fn actual_existing_request_prepares_exact_recipient_without_mutation() {
    let client = Client::new(true);
    let claim = fixture::claim();
    let namespace = fixture::payload().handoff.namespace;
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let prepared = prepare(
        &request(),
        Preparation {
            claim: &claim,
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
    );
    assert!(
        prepared.is_ok(),
        "existing peer preparation missing: {prepared:?}"
    );
    let prepared = prepared.unwrap();
    assert_eq!(prepared.recipient.as_str(), "recipient");
    assert_eq!(prepared.payload.body, "literal '$HOME' work");
    assert_eq!(
        prepared.payload.channel,
        HandoffChannel::Existing {
            thread: ThreadId::new("t1")
        }
    );
    assert!(client.calls.lock().unwrap().iter().all(|c| matches!(
        c,
        Command::SeatInspect(_) | Command::ResolveThread(_) | Command::Directory(_)
    )));
}
#[test]
fn joined_delivery_skips_invite_and_stages_exact_body_without_launch() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let plan = plan();
    let reference = record(&journal, plan.clone());
    assert_ne!(reference.operation, plan.payload.keys.compound);
    let client = Client::new(true);
    let result = execute(&journal, &reference, &client, &TestClock);
    assert!(result.is_ok(), "delivery staging missing: {result:?}");
    let report = result.unwrap();
    assert_eq!(report["participation"], "joined");
    assert_eq!(
        report["message"],
        serde_json::json!({"kind":"message_sent","data":"m1"})
    );
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Invite(_)))
            .count(),
        0
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::SendMessage(_)))
            .count(),
        1
    );
}
#[test]
fn unsafe_recipient_preparation_refuses_before_any_write() {
    for state in [ContinuityStatus::Unresolved, ContinuityStatus::Retired] {
        let mut client = Client::new(true);
        client.inspection.summary.continuity = state;
        client.inspection.mapping.state = state;
        let claim = fixture::claim();
        let namespace = fixture::payload().handoff.namespace;
        let topology = crate::host::observation::HostTopology {
            spaces: vec![],
            tabs: vec![],
            panes: vec![],
        };
        assert!(
            prepare(
                &request(),
                Preparation {
                    claim: &claim,
                    namespace: &namespace,
                    topology: &topology,
                    client: &client,
                    clock: &TestClock
                }
            )
            .is_err()
        );
    }
}

struct TempRoot(std::path::PathBuf);
impl TempRoot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("sp-qhz8-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn invitation_reply_loss_replays_original_child_even_after_join() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let reference = record(&journal, plan());
    let mut client = Client::new(false);
    *client.lost.lock().unwrap() = Some("invite");
    assert!(execute(&journal, &reference, &client, &TestClock).is_err());
    client.joined = true;
    let report = execute(&journal, &reference, &client, &TestClock).unwrap();
    assert_eq!(
        report["participation"], "joined",
        "report must reflect current canonical participation, while retaining the old invitation result"
    );
    assert_eq!(
        report["invitation"],
        serde_json::json!({"kind":"invitation","data":"i1"})
    );
    assert_eq!(
        client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, Command::Invite(_)))
            .count(),
        2
    );
}

#[test]
fn new_unbound_delivery_reuses_exact_children_after_each_lost_reply() {
    for lost in ["create", "invite", "send"] {
        let tmp = TempRoot::new();
        let journal = Journal::open(tmp.0.join("intents")).unwrap();
        let mut plan = plan();
        plan.payload.channel = HandoffChannel::New {
            name: Some("review".into()),
            topic: "review".into(),
            goal: "review changes".into(),
        };
        let reference = record(&journal, plan.clone());
        let mut client = Client::new(false);
        client.inspection.open_binding = None;
        *client.lost.lock().unwrap() = Some(lost);
        assert!(execute(&journal, &reference, &client, &TestClock).is_err());
        let report = execute(&journal, &reference, &client, &TestClock).unwrap();
        assert_eq!(report["outcome"], "staged");
        assert_eq!(report["participation"], "staged_unbound");
        assert_eq!(report["thread"], "t1");
        assert_eq!(report["recipient"], "recipient");
        assert!(report.get("launch").is_none());
        let saved = client.saved.lock().unwrap();
        assert_eq!(saved.len(), 3);
        assert!(saved.contains_key(plan.payload.keys.create.as_str()));
        assert!(saved.contains_key(plan.payload.keys.invite.as_str()));
        assert!(saved.contains_key(plan.payload.keys.send.as_str()));
        for c in client.calls.lock().unwrap().iter() {
            match c {
                Command::BeginHandoff(q) => assert_eq!(q.operation, plan.payload.keys.begin),
                Command::CompleteHandoff(q) => assert_eq!(q.operation, plan.payload.keys.complete),
                Command::CreateThread(q) => assert_eq!(q.operation, plan.payload.keys.create),
                Command::Invite(q) => assert_eq!(q.operation, plan.payload.keys.invite),
                Command::SendMessage(q) => assert_eq!(q.operation, plan.payload.keys.send),
                _ => {}
            }
        }
    }
}

#[test]
fn held_foreign_and_launch_only_requests_refuse_before_mutation() {
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let claim = fixture::claim();
    let namespace = fixture::payload().handoff.namespace;
    let mut held = Client::new(false);
    held.inspection.hold = Some(
        serde_json::from_value(
            serde_json::json!({"target":"w1:p2","reason_data":"restore hold","detail_argv":[]}),
        )
        .unwrap(),
    );
    assert!(
        prepare(
            &request(),
            Preparation {
                claim: &claim,
                namespace: &namespace,
                topology: &topology,
                client: &held,
                clock: &TestClock
            }
        )
        .is_err()
    );
    let client = Client::new(false);
    let mut foreign = namespace.clone();
    foreign.instance = "foreign".into();
    assert!(
        prepare(
            &request(),
            Preparation {
                claim: &claim,
                namespace: &foreign,
                topology: &topology,
                client: &client,
                clock: &TestClock
            }
        )
        .is_err()
    );
    assert!(client.calls.lock().unwrap().is_empty());
    let mut launch = request();
    launch.kind = Some("codex".into());
    assert!(
        prepare(
            &launch,
            Preparation {
                claim: &claim,
                namespace: &namespace,
                topology: &topology,
                client: &client,
                clock: &TestClock
            }
        )
        .is_err()
    );
    assert!(client.calls.lock().unwrap().is_empty());
}

// Keep real canonical invitation transactions below the coordinator boundary.
// The fake covers the unrelated host/service routes and rejects every native,
// allocation, registration, acceptance and ACK route.
struct CanonicalInvites {
    fake: Client,
    store: crate::store::SqliteStore,
    context: crate::store::connection::StoreContext,
    lose_invite: Mutex<bool>,
}
impl CanonicalInvites {
    fn new(root: &std::path::Path) -> Self {
        Self::with_recipient(root, "recipient", "w1:p2")
    }
    fn with_recipient(root: &std::path::Path, recipient: &str, target: &str) -> Self {
        let context = crate::store::connection::StoreContext::new(
            root.join("canonical.db"),
            std::sync::Arc::new(TestClock),
        )
        .with_test_host();
        let conn = context.open_writer().unwrap();
        conn.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
        for (seat, target) in [("sender", "w1:p1"), (recipient, target)] {
            conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',?2,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",rusqlite::params![seat,target]).unwrap();
            conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)",[target]).unwrap();
            conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||?2,'inc')",rusqlite::params![seat,target]).unwrap();
        }
        drop(conn);
        let store = crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(
                root.join("canonical.db"),
                std::sync::Arc::new(TestClock),
            )
            .with_test_host(),
            "34750f98bd59fcfc946da45aaabe933b",
            Default::default(),
        )
        .unwrap();
        Self {
            fake: Client::new(false),
            store,
            context,
            lose_invite: Mutex::new(true),
        }
    }
    fn canonical(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        use crate::ports::StorePort;
        let mutation = crate::protocol::commands::PermitMutation::try_from(command).unwrap();
        if let crate::protocol::commands::PermitMutation::SendMessage(send) = &mutation {
            loop {
                match self.store.prepare_send_step(
                    send,
                    crate::ports::DurableWorkAdmission::new(16).unwrap(),
                    budget,
                )? {
                    crate::ports::SendPreparationProgress::Ready { .. } => break,
                    crate::ports::SendPreparationProgress::More { .. } => {}
                    crate::ports::SendPreparationProgress::Committed(result) => return Ok(result),
                }
            }
        }
        let request = crate::store::cooperative_permit_request(&mutation)?;
        let permit = self.store.issue_cooperative_permit(request, budget)?;
        self.store.mutate(mutation, permit, budget)
    }
}
impl LocalClient for CanonicalInvites {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
        if matches!(c, Command::CreateThread(_) | Command::Invite(_)) {
            self.fake.calls.lock().unwrap().push(c.clone());
            let invite = matches!(c, Command::Invite(_));
            let result = self.canonical(c, b)?;
            if invite && std::mem::take(&mut *self.lose_invite.lock().unwrap()) {
                return Err(ApiError::new(
                    ErrorCode::UnknownOutcome,
                    "canonical invitation committed; lost reply",
                ));
            }
            Ok(result)
        } else if matches!(c, Command::BeginHandoff(_) | Command::CompleteHandoff(_)) {
            let mut result = self.fake.call(c, b)?;
            if let CommandResult::Handoff(fence) = &mut result {
                let conn = self.context.open_writer().unwrap();
                let thread: Option<String> = conn
                    .query_row("SELECT id FROM threads LIMIT 1", [], |r| r.get(0))
                    .ok();
                if let Some(thread) = thread {
                    fence.thread = Some(ThreadId::new(thread));
                } else {
                    fence.thread = None;
                }
            }
            Ok(result)
        } else {
            self.fake.call(c, b)
        }
    }
}
#[test]
fn canonical_invitation_loss_and_old_replay_leave_new_episode_pending() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = CanonicalInvites::new(&tmp.0);
    let mut plan = plan();
    plan.payload.channel = HandoffChannel::New {
        name: None,
        topic: "Review".into(),
        goal: "Review changes".into(),
    };
    let reference = record(&journal, plan.clone());
    let lost = execute(&journal, &reference, &client, &TestClock);
    assert!(
        matches!(lost,Err(RunError::Api(ref error)) if error.code == ErrorCode::UnknownOutcome),
        "expected canonical invitation loss: {lost:?}"
    );
    let conn = client.context.open_writer().unwrap();
    let (first, thread, episode): (String, String, i64) = conn
        .query_row("SELECT id,thread_id,episode FROM invitations", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(episode, 1);
    drop(conn);
    let budget = super::super::cooperative_budget(&TestClock);
    // Independent recipient actions change the episode between sender retries.
    // They are setup actions, outside the delivery client's allowed routes.
    let mut recipient = canonical_claim();
    recipient.seat = SeatId::new("recipient");
    recipient.target = crate::protocol::ids::HostTargetId::new("w1:p2");
    client
        .canonical(
            Command::Accept(crate::protocol::commands::Accept {
                thread: ThreadId::new(thread.clone()),
                operation: crate::protocol::ids::OperationId::new("recipient-accepts"),
                claim: recipient.clone(),
            }),
            &budget,
        )
        .unwrap();
    client
        .canonical(
            Command::Leave(crate::protocol::commands::Leave {
                thread: ThreadId::new(thread.clone()),
                operation: crate::protocol::ids::OperationId::new("recipient-leaves"),
                claim: recipient,
            }),
            &budget,
        )
        .unwrap();
    let next = client
        .canonical(
            Command::Invite(crate::protocol::commands::Invite {
                confirmation: None,
                thread: ThreadId::new(thread.clone()),
                seat: SeatId::new("recipient"),
                deadline_millis: None,
                operation: crate::protocol::ids::OperationId::new("another-invitation"),
                claim: canonical_claim(),
            }),
            &budget,
        )
        .unwrap();
    let CommandResult::Invitation(second) = next else {
        panic!("expected new episode")
    };
    assert_ne!(second.as_str(), first);
    let report = execute(&journal, &reference, &client, &TestClock).unwrap();
    assert_eq!(report["invitation"]["data"], first);
    assert_eq!(report["thread"], thread);
    assert_eq!(report["recipient"], "recipient");
    let conn = client.context.open_writer().unwrap();
    let rows: Vec<(String, i64, String)> = conn
        .prepare("SELECT id,episode,state FROM invitations ORDER BY episode")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            (first, 1, "accepted".into()),
            (second.as_str().into(), 2, "pending".into())
        ]
    );
}

#[test]
fn pane_labels_select_canonical_seat_and_ambiguity_never_allocates() {
    use crate::host::observation::{HostTopology, TopologyPane, TopologySpace, TopologyTab};
    let mut topology = HostTopology {
        spaces: vec![TopologySpace {
            id: "w1".into(),
            label: Some("work".into()),
        }],
        tabs: vec![TopologyTab {
            id: "w1:t1".into(),
            space: "w1".into(),
            label: Some("review".into()),
        }],
        panes: vec![TopologyPane {
            target: crate::protocol::ids::HostTargetId::new("w1:p2"),
            space: "w1".into(),
            tab: "w1:t1".into(),
            label: Some("peer".into()),
            agent_names: vec![],
        }],
    };
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--space",
        "work",
        "--tab",
        "review",
        "--pane",
        "peer",
        "--thread",
        "chosen",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let claim = fixture::claim();
    let namespace = fixture::payload().handoff.namespace;
    let client = Client::new(true);
    let plan = prepare(
        &request,
        Preparation {
            claim: &claim,
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
    )
    .unwrap();
    assert_eq!(plan.recipient.as_str(), "recipient");
    let mut duplicate = topology.panes[0].clone();
    duplicate.target = crate::protocol::ids::HostTargetId::new("w1:p3");
    topology.panes.push(duplicate);
    let client = Client::new(true);
    assert!(
        prepare(
            &request,
            Preparation {
                claim: &claim,
                namespace: &namespace,
                topology: &topology,
                client: &client,
                clock: &TestClock
            }
        )
        .is_err()
    );
    assert!(client.calls.lock().unwrap().is_empty());
}

#[test]
fn canonical_pending_invitation_is_reused_in_existing_channel() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = CanonicalInvites::new(&tmp.0);
    *client.lose_invite.lock().unwrap() = false;
    let budget = super::super::cooperative_budget(&TestClock);
    let created = client
        .canonical(
            Command::CreateThread(crate::protocol::commands::CreateThread {
                name: None,
                topic: "Review".into(),
                goal: "Review".into(),
                operation: crate::protocol::ids::OperationId::new("prior-create"),
                claim: canonical_claim(),
            }),
            &budget,
        )
        .unwrap();
    let CommandResult::ThreadCreated(thread) = created else {
        panic!("not created")
    };
    let initial = client
        .canonical(
            Command::Invite(crate::protocol::commands::Invite {
                confirmation: None,
                thread: thread.clone(),
                seat: SeatId::new("recipient"),
                deadline_millis: None,
                operation: crate::protocol::ids::OperationId::new("prior-invite"),
                claim: canonical_claim(),
            }),
            &budget,
        )
        .unwrap();
    let CommandResult::Invitation(invitation) = initial else {
        panic!("not invited")
    };
    let mut plan = plan();
    plan.payload.channel = HandoffChannel::Existing {
        thread: thread.clone(),
    };
    let reference = publish(&journal, plan, canonical_claim(), &TestClock).unwrap();
    let original = journal.load(&reference).unwrap();
    let report = execute(&journal, &reference, &client, &TestClock).unwrap();
    assert_eq!(report["invitation"]["data"], invitation.as_str());
    assert_eq!(report["participation"], "invited_pending");
    let conn = client.context.open_writer().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM invitations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let sender_joined: bool = conn
        .query_row(
            "SELECT state='joined' FROM memberships WHERE thread_id=?1 AND seat_id='sender'",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(sender_joined);
    let calls = client.fake.calls.lock().unwrap();
    assert!(!calls.iter().any(|c| matches!(c, Command::CreateThread(_))));
    for c in calls.iter() {
        if let Command::BeginHandoff(q) = c {
            assert_eq!(q.identity.digest, original.header.semantic_digest);
            assert_ne!(q.identity.compound, reference.operation);
        }
    }
}

// A missing exact report field must refuse, rather than fabricate staged delivery.
macro_rules! corrupt_report_test {
    ($name:ident, $field:literal, $bad:expr) => {
        #[test]
        fn $name() {
            let tmp = TempRoot::new();
            let journal = Journal::open(tmp.0.join("intents")).unwrap();
            let reference = record(&journal, plan());
            let client = Client::new(false);
            execute(&journal, &reference, &client, &TestClock).unwrap();
            let mut progress: Progress = handoff::load_progress(&journal, &reference).unwrap();
            progress.report.as_mut().unwrap()[$field] = $bad;
            handoff::save_progress(&journal, &reference, &progress).unwrap();
            client.calls.lock().unwrap().clear();
            assert!(
                execute(&journal, &reference, &client, &TestClock).is_err(),
                "completed report corruption accepted: {}",
                $field
            );
            assert!(journal.load(&reference).is_ok());
            assert!(
                client
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|c| matches!(c, Command::BeginHandoff(_)))
            );
        }
    };
}
corrupt_report_test!(
    completed_corrupt_compound_refuses,
    "compound",
    serde_json::json!("wrong")
);
corrupt_report_test!(
    completed_corrupt_message_refuses,
    "message",
    serde_json::json!({"kind":"message_sent","data":"wrong"})
);
corrupt_report_test!(
    completed_corrupt_invitation_refuses,
    "invitation",
    serde_json::json!(null)
);
corrupt_report_test!(
    completed_corrupt_participation_refuses,
    "participation",
    serde_json::json!("working")
);
corrupt_report_test!(
    completed_corrupt_reference_refuses,
    "recovery_ref",
    serde_json::json!("local:999")
);
corrupt_report_test!(
    completed_unexpected_field_refuses,
    "accepted",
    serde_json::json!(true)
);

struct TerminalClient {
    original: crate::protocol::handoff::HandoffIdentity,
    calls: Mutex<usize>,
}
impl LocalClient for TerminalClient {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        *self.calls.lock().unwrap() += 1;
        let Command::HandoffDelivery(query) = c else {
            panic!("terminal retry attempted live effect/read: {c:?}")
        };
        query.validate().unwrap();
        let crate::protocol::handoff::DeliveryAction::Status(identity) = query.action else {
            panic!("terminal must use read-only exact status")
        };
        assert_eq!(identity, self.original);
        Ok(CommandResult::Handoff(
            crate::protocol::handoff::HandoffResult {
                compound: identity.compound,
                thread: Some(ThreadId::new("t1")),
                state: HandoffState::Completed,
            },
        ))
    }
}
fn terminal_fixture(journal: &Journal) -> (IntentRef, TerminalClient) {
    let reference = record(journal, plan());
    let client = Client::new(false);
    execute(journal, &reference, &client, &TestClock).unwrap();
    let identity = client
        .calls
        .lock()
        .unwrap()
        .iter()
        .find_map(|c| {
            if let Command::BeginHandoff(q) = c {
                Some(q.identity.clone())
            } else {
                None
            }
        })
        .unwrap();
    (
        reference,
        TerminalClient {
            original: identity,
            calls: Mutex::new(0),
        },
    )
}
#[derive(Default)]
struct FailedWriter {
    flush: bool,
    bytes: Vec<u8>,
}
impl std::io::Write for FailedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if !self.flush {
            return Err(std::io::Error::other("failed write"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("failed flush"))
    }
}
#[test]
fn retry_completed_strict_client_after_journal_restart_cleans_only_local_files() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    drop(journal);
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut writer = Vec::new();
    let report = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec {
            format: crate::protocol::output::OutputFormat::Json,
            ..OutputSpec::default()
        },
        &mut writer,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_slice(&writer).unwrap();
    assert_eq!(
        output["delivery"]["message"],
        serde_json::json!({"kind":"message_sent","data":"m1"})
    );
    assert_eq!(report["participation"], "invited_pending");
    assert!(journal.load(&reference).is_err());
    assert!(!handoff::progress_path(&journal, &reference).exists());
    assert_eq!(*client.calls.lock().unwrap(), 1);
}
#[test]
fn retry_failed_write_and_flush_retain_exact_report_and_original() {
    for flush in [false, true] {
        let tmp = TempRoot::new();
        let journal = Journal::open(tmp.0.join("intents")).unwrap();
        let (reference, client) = terminal_fixture(&journal);
        let mut writer = FailedWriter {
            flush,
            ..Default::default()
        };
        let failed = retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut writer,
        );
        assert!(
            matches!(failed, Err(RunError::Io(_))),
            "expected actual output failure: {failed:?}"
        );
        assert!(journal.load(&reference).is_ok());
        assert!(handoff::progress_path(&journal, &reference).exists());
        let mut writer = Vec::new();
        retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut writer,
        )
        .unwrap();
        assert!(String::from_utf8(writer).unwrap().contains("m1"));
    }
}

#[test]
fn retry_copied_uuid_wrong_root_or_socket_refuses_before_client_or_output() {
    for terminal in [false, true] {
        for state in [false, true] {
            let tmp = TempRoot::new();
            let journal = Journal::open(tmp.0.join("intents")).unwrap();
            let (reference, client) = terminal_fixture(&journal);
            if terminal {
                retry_to_writer(
                    &journal,
                    &reference,
                    &plan().payload.namespace,
                    &client,
                    &TestClock,
                    &OutputSpec::default(),
                    &mut Vec::new(),
                )
                .unwrap();
            }
            *client.calls.lock().unwrap() = 0;
            let mut namespace = plan().payload.namespace;
            if state {
                namespace.state_dir = "/copied-instance".into()
            } else {
                namespace.host_endpoint = "/another-host.sock".into()
            }
            let mut writer = Vec::new();
            assert!(
                retry_to_writer(
                    &journal,
                    &reference,
                    &namespace,
                    &client,
                    &TestClock,
                    &OutputSpec::default(),
                    &mut writer
                )
                .is_err()
            );
            assert!(writer.is_empty());
            assert_eq!(*client.calls.lock().unwrap(), 0);
        }
    }
}
#[test]
fn retry_completion_reply_loss_replays_only_the_completed_fence() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let reference = record(&journal, plan());
    let client = Client::new(false);
    *client.lost.lock().unwrap() = Some("complete");
    assert!(execute(&journal, &reference, &client, &TestClock).is_err());
    client.calls.lock().unwrap().clear();
    let report = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(report["message"]["data"], "m1");
    assert!(
        client
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| matches!(c, Command::BeginHandoff(_)))
    );
}
struct CleanupFailureWriter {
    root: std::path::PathBuf,
    path: Option<std::path::PathBuf>,
    sync: bool,
}
impl std::io::Write for CleanupFailureWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        if let Some(path) = &self.path {
            std::fs::remove_file(path)?;
            std::fs::create_dir(path)?;
        } else if self.sync {
            // Search/write permissions permit unlink, but opening the directory
            // for fsync fails. No global failpoint or process state is used.
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o300))?;
        }
        Ok(())
    }
}
macro_rules! cleanup_failure_test {
    ($name:ident, $intent:expr, $sync:expr) => {
        #[test]
        fn $name() {
            use std::os::unix::fs::PermissionsExt;
            let tmp = TempRoot::new();
            let journal = Journal::open(tmp.0.join("intents")).unwrap();
            let (reference, client) = terminal_fixture(&journal);
            let original = journal.snapshot_delivery_origin(&reference).unwrap();
            let path = if $sync {
                None
            } else if $intent {
                Some(
                    std::fs::read_dir(journal.root())
                        .unwrap()
                        .map(|e| e.unwrap().path())
                        .find(|p| p.extension().is_some_and(|e| e == "intent"))
                        .unwrap(),
                )
            } else {
                Some(handoff::progress_path(&journal, &reference))
            };
            let mut writer = CleanupFailureWriter {
                root: journal.root().into(),
                path: path.clone(),
                sync: $sync,
            };
            let failed = retry_to_writer(
                &journal,
                &reference,
                &plan().payload.namespace,
                &client,
                &TestClock,
                &OutputSpec::default(),
                &mut writer,
            );
            std::fs::set_permissions(journal.root(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            assert!(
                matches!(failed, Err(RunError::Io(_))),
                "expected actual cleanup error: {failed:?}"
            );
            if let Some(path) = path {
                std::fs::remove_dir(path).unwrap();
            }
            drop(journal);
            let journal = Journal::open(tmp.0.join("intents")).unwrap();
            let found = resolve_recovery_ref(&journal, &reference.recovery_ref()).unwrap();
            assert_eq!(found, reference);
            let pending = load_original(&journal, &found).unwrap();
            assert_eq!(
                pending.header.semantic_digest,
                Journal::decode_delivery_origin(&reference, &original)
                    .unwrap()
                    .header
                    .semantic_digest
            );
            let report = retry_to_writer(
                &journal,
                &found,
                &plan().payload.namespace,
                &client,
                &TestClock,
                &OutputSpec::default(),
                &mut Vec::new(),
            )
            .unwrap();
            assert_eq!(report["message"]["data"], "m1");
            assert!(journal.load(&reference).is_err());
            assert!(!handoff::progress_path(&journal, &reference).exists());
            assert!(terminal_path(&journal, &reference).is_file());
        }
    };
}
cleanup_failure_test!(retry_progress_removal_failure_is_replayable, false, false);
cleanup_failure_test!(retry_intent_removal_failure_is_replayable, true, false);
cleanup_failure_test!(
    retry_directory_sync_failure_after_unlink_is_replayable,
    false,
    true
);
#[test]
fn retry_terminal_corruption_refuses_without_output_or_client() {
    for field in ["original", "report", "completed", "version"] {
        let tmp = TempRoot::new();
        let journal = Journal::open(tmp.0.join("intents")).unwrap();
        let (reference, client) = terminal_fixture(&journal);
        retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let path = terminal_path(&journal, &reference);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match field {
            "original" => {
                value["original"] = serde_json::json!(
                    value["original"]
                        .as_str()
                        .unwrap()
                        .replace("literal '$HOME' work", "tampered work")
                )
            }
            "report" => {
                value["progress"]["report"]["recipient"] = serde_json::json!("another-seat")
            }
            "completed" => value["completed"]["state"] = serde_json::json!("live"),
            _ => value["version"] = serde_json::json!(2),
        }
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        *client.calls.lock().unwrap() = 0;
        let mut writer = Vec::new();
        assert!(
            retry_to_writer(
                &journal,
                &reference,
                &plan().payload.namespace,
                &client,
                &TestClock,
                &OutputSpec::default(),
                &mut writer
            )
            .is_err()
        );
        assert!(writer.is_empty());
        assert_eq!(*client.calls.lock().unwrap(), 0);
    }
}

struct CanonicalDelivery {
    base: CanonicalInvites,
    terminal_only: bool,
}
impl LocalClient for CanonicalDelivery {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
        if let Command::HandoffDelivery(query) = &c {
            assert!(matches!(
                query.action,
                crate::protocol::handoff::DeliveryAction::Status(_)
            ));
            use crate::ports::BootstrapStorePort;
            self.base.fake.calls.lock().unwrap().push(c.clone());
            return self
                .base
                .store
                .delivery_query(&query.plan.payload.namespace, query, b);
        }
        assert!(!self.terminal_only, "historical retry attempted {c:?}");
        if matches!(
            c,
            Command::BeginHandoff(_)
                | Command::CompleteHandoff(_)
                | Command::CreateThread(_)
                | Command::Invite(_)
                | Command::SendMessage(_)
        ) {
            self.base.fake.calls.lock().unwrap().push(c.clone());
            self.base.canonical(c, b)
        } else {
            self.base.fake.call(c, b)
        }
    }
}
#[test]
fn retry_canonical_completed_after_both_bindings_change_archive_and_store_restart() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut client = CanonicalDelivery {
        base: CanonicalInvites::new(&tmp.0),
        terminal_only: false,
    };
    let mut plan = plan();
    plan.payload.channel = HandoffChannel::New {
        name: None,
        topic: "review".into(),
        goal: "review changes".into(),
    };
    let reference = record(&journal, plan.clone());
    let original_report = execute(&journal, &reference, &client, &TestClock).unwrap();
    let conn = client.base.context.open_writer().unwrap();
    conn.execute("UPDATE threads SET archived=1", []).unwrap();
    conn.execute("UPDATE occupant_bindings SET generation=2", [])
        .unwrap();
    let before: (i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM invitations),(SELECT COUNT(*) FROM messages)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    drop(conn);
    client.base.store = crate::store::SqliteStore::new(
        crate::store::connection::StoreContext::new(
            tmp.0.join("canonical.db"),
            std::sync::Arc::new(TestClock),
        )
        .with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        Default::default(),
    )
    .unwrap();
    client.terminal_only = true;
    client.base.fake.calls.lock().unwrap().clear();
    let report = retry_to_writer(
        &journal,
        &reference,
        &plan.payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(report, original_report);
    let conn = client.base.context.open_writer().unwrap();
    let after: (i64, i64) = conn
        .query_row(
            "SELECT (SELECT COUNT(*) FROM invitations),(SELECT COUNT(*) FROM messages)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    assert!(!client.base.fake.calls.lock().unwrap().is_empty());
}

fn terminal_unknown_field_refuses(header: bool) {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    let path = terminal_path(&journal, &reference);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    if header {
        let original = value["original"].as_str().unwrap();
        let (line, body) = original.split_once('\n').unwrap();
        let mut h: serde_json::Value = serde_json::from_str(line).unwrap();
        h["authority_override"] = serde_json::json!("human");
        value["original"] = serde_json::json!(format!("{}\n{}", h, body));
    } else {
        value["progress"]["staged"]["accepted"] = serde_json::json!(true);
    }
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        load_original(&journal, &reference).is_err(),
        "unknown original/staged field accepted, header={header}"
    );
}
#[test]
fn retry_terminal_unknown_nested_fields_refuse() {
    terminal_unknown_field_refuses(false);
}
#[test]
fn retry_terminal_unknown_header_fields_refuse() {
    terminal_unknown_field_refuses(true);
}

#[test]
fn retry_reference_refuses_conflicting_terminal_even_with_original_intent() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    let failed = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut FailedWriter::default(),
    );
    assert!(matches!(failed, Err(RunError::Io(_))));
    let second = IntentRef {
        ordinal: reference.ordinal,
        operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
    };
    let mut terminal = read_terminal(&journal, &reference).unwrap().unwrap();
    let (header, body) = terminal.original.split_once('\n').unwrap();
    let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
    header["reference"] = serde_json::to_value(&second).unwrap();
    terminal.original = format!("{}\n{}", header, body);
    save_terminal(&journal, &second, &terminal).unwrap();
    assert!(
        resolve_recovery_ref(&journal, &reference.recovery_ref()).is_err(),
        "original intent hid conflicting retained reference"
    );
}

#[test]
fn retry_terminal_changed_valid_participation_is_corrupt_history() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    let path = terminal_path(&journal, &reference);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["progress"]["report"]["participation"] = serde_json::json!("joined");
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    *client.calls.lock().unwrap() = 0;
    let mut writer = Vec::new();
    assert!(
        retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut writer
        )
        .is_err(),
        "valid vocabulary silently changed historical report"
    );
    assert!(writer.is_empty());
    assert_eq!(*client.calls.lock().unwrap(), 0);
}
corrupt_report_test!(
    completed_changed_valid_participation_refuses,
    "participation",
    serde_json::json!("staged_unbound")
);

struct PublicationSyncFailureClient<'a> {
    terminal: &'a TerminalClient,
    root: std::path::PathBuf,
}
impl LocalClient for PublicationSyncFailureClient<'_> {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
        use std::os::unix::fs::PermissionsExt;
        let result = self.terminal.call(c, b)?;
        // Permit publication/unlink and direct file reads, but refuse opening
        // the private directory for its durability sync. No global state.
        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o300)).unwrap();
        Ok(result)
    }
}
fn failed_publication_retry_boundary(output_boundary: bool) {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, terminal) = terminal_fixture(&journal);
    let original = journal.snapshot_delivery_origin(&reference).unwrap();
    let client = PublicationSyncFailureClient {
        terminal: &terminal,
        root: journal.root().into(),
    };
    let mut first_output = Vec::new();
    let first = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut first_output,
    );
    std::fs::set_permissions(journal.root(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        matches!(first, Err(RunError::Io(ref e)) if e.kind() == std::io::ErrorKind::PermissionDenied),
        "publication must reach actual directory sync failure: {first:?}"
    );
    assert!(terminal_path(&journal, &reference).is_file());
    // No success presentation before durability; only the non-success
    // pending report that canonical completion was observed.
    let first_text = String::from_utf8(first_output).unwrap();
    assert!(
        !first_text.contains("\"outcome\":\"staged\"") && !first_text.contains("outcome: staged"),
        "{first_text}"
    );
    assert!(
        first_text.contains("completed_terminal_unconfirmed"),
        "{first_text}"
    );
    assert_eq!(
        journal.snapshot_delivery_origin(&reference).unwrap(),
        original
    );
    assert!(handoff::progress_path(&journal, &reference).is_file());
    std::fs::set_permissions(journal.root(), std::fs::Permissions::from_mode(0o300)).unwrap();
    // The origin helper must remain read-only even when directory sync fails.
    let origin = load_original(&journal, &reference);
    let mut output = Vec::new();
    let retry = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    );
    std::fs::set_permissions(journal.root(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        origin.is_ok(),
        "read-only origin selection attempted durability work: {origin:?}"
    );
    assert!(
        matches!(retry, Err(RunError::Io(ref e)) if e.kind() == std::io::ErrorKind::PermissionDenied),
        "retry must propagate actual barrier error: {retry:?}"
    );
    if output_boundary {
        assert!(
            output.is_empty(),
            "retry presented before terminal durability was confirmed"
        );
    } else {
        assert!(
            handoff::progress_path(&journal, &reference).is_file(),
            "retry removed progress before terminal durability was confirmed"
        );
        assert_eq!(
            journal.snapshot_delivery_origin(&reference).unwrap(),
            original
        );
    }
    let report = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &terminal,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(report["message"]["data"], "m1");
    assert!(journal.load(&reference).is_err());
    assert!(!handoff::progress_path(&journal, &reference).exists());
}
#[test]
fn fix_retry_publication_sync_failure_refuses_output_until_durable() {
    failed_publication_retry_boundary(true);
}
#[test]
fn fix_retry_publication_sync_failure_retains_files_until_durable() {
    failed_publication_retry_boundary(false);
}
fn corrupted_standalone_progress_refuses(with_terminal: bool) {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    if with_terminal {
        let failed = retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut FailedWriter::default(),
        );
        assert!(matches!(failed, Err(RunError::Io(_))));
        assert!(terminal_path(&journal, &reference).is_file());
    }
    let path = handoff::progress_path(&journal, &reference);
    let mut raw: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw["staged"]["accepted"] = serde_json::json!(true);
    let corrupt = serde_json::to_vec(&raw).unwrap();
    std::fs::write(&path, &corrupt).unwrap();
    let original = journal.snapshot_delivery_origin(&reference).unwrap();
    let mut output = Vec::new();
    let result = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    );
    assert!(
        result.is_err(),
        "raw staged.accepted silently discarded, with_terminal={with_terminal}: {result:?}"
    );
    assert!(output.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    assert_eq!(
        journal.snapshot_delivery_origin(&reference).unwrap(),
        original
    );
    if !with_terminal {
        assert!(!terminal_path(&journal, &reference).exists());
    }
}
#[test]
fn fix_retry_preterminal_unknown_staged_field_refuses() {
    corrupted_standalone_progress_refuses(false);
}
#[test]
fn fix_retry_surviving_progress_unknown_staged_field_refuses() {
    corrupted_standalone_progress_refuses(true);
}

fn oversized_progress_prefix_refuses(with_terminal: bool) {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    if with_terminal {
        let failed = retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut FailedWriter::default(),
        );
        assert!(matches!(failed, Err(RunError::Io(_))));
        assert!(terminal_path(&journal, &reference).is_file());
    }
    let path = handoff::progress_path(&journal, &reference);
    // The valid genuine JSON plus whitespace fits exactly in the old read
    // limit. Its malformed extra frame must not disappear past that limit.
    let mut corrupt = std::fs::read(&path).unwrap();
    corrupt.resize(4 * 1024 * 1024, b' ');
    corrupt.extend_from_slice(br#"{"staged":{"accepted":true}}"#);
    std::fs::write(&path, &corrupt).unwrap();
    let original = journal.snapshot_delivery_origin(&reference).unwrap();
    let mut output = Vec::new();
    let result = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    );
    assert!(
        result.is_err(),
        "oversized progress suffix silently ignored, with_terminal={with_terminal}: {result:?}"
    );
    assert!(output.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    assert_eq!(
        journal.snapshot_delivery_origin(&reference).unwrap(),
        original
    );
    if !with_terminal {
        assert!(!terminal_path(&journal, &reference).exists());
    }
}
#[test]
fn fix_reader_preterminal_oversized_prefix_refuses() {
    oversized_progress_prefix_refuses(false);
}
#[test]
fn fix_reader_surviving_progress_oversized_prefix_refuses() {
    oversized_progress_prefix_refuses(true);
}

// Removing the wrapper's original-actor gate presents/cleans a completed Human
// handoff even after its intent was removed, before any selected current actor.
#[test]
fn actor_prerequisite_retained_human_delivery_refuses_before_canonical_or_cleanup() {
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut claim = canonical_claim();
    claim.harness = crate::protocol::authority::Harness::Human;
    let reference = journal
        .record(
            super::super::journal::IntentScope::Cooperative {
                instance: claim.instance.clone(),
                seat: claim.seat.clone(),
            },
            super::super::journal::SemanticMutation::freeze(
                super::super::journal::SemanticMutation::HandoffDelivery(Box::new(plan())),
                claim,
            )
            .unwrap(),
            1,
        )
        .unwrap();
    // Historical delivery v1 admitted this internal Human original before the
    // guarded Agent-only envelope existed. Produce its old staged/report/terminal
    // encoding using the unchanged legacy child/fence API, never a new envelope.
    let live_client = Client::new(false);
    let pending = journal.load(&reference).unwrap();
    let human = pending.semantic.frozen_claim().unwrap().clone();
    let p = plan();
    let keys = &p.payload.keys;
    let original = crate::protocol::handoff::HandoffIdentity {
        compound: keys.compound.clone(),
        digest: pending.header.semantic_digest,
        claim: human.clone(),
        thread: p.payload.channel.thread().cloned(),
        recipient: p.recipient.clone(),
        create_key: keys.create.clone(),
        invite_key: keys.invite.clone(),
        send_key: keys.send.clone(),
    };
    handoff::keyed_fence(
        &live_client,
        &TestClock,
        &original,
        keys.begin.clone(),
        false,
    )
    .unwrap();
    let mut staged = handoff::StagedWork::default();
    handoff::stage_work(
        handoff::Staging {
            policy: None,
            channel: &p.payload.channel,
            body: &p.payload.body,
            recipient: &p.recipient,
            create_key: &keys.create,
            invite_key: &keys.invite,
            send_key: &keys.send,
            skip_joined: true,
        },
        &mut staged,
        &mut "create",
        &|semantic, key| {
            Ok(live_client.call(
                semantic.to_command(key.clone(), Some(human.clone()))?,
                &super::super::cooperative_budget(&TestClock),
            )?)
        },
        &mut |_| Ok(()),
        &live_client,
        &TestClock,
    )
    .unwrap();
    let report = serde_json::json!({"compound":keys.compound,"thread":staged.thread,"recipient":p.recipient,"participation":"invited_pending","outcome":"staged","invitation":staged.invitation,"message":staged.message,"recovery_ref":reference.recovery_ref()});
    let progress = Progress {
        staged,
        report_digest: Some(retained_digest(&report).unwrap()),
        report: Some(report),
    };
    let completed = handoff::keyed_fence(
        &live_client,
        &TestClock,
        &original,
        keys.complete.clone(),
        true,
    )
    .unwrap();
    save_terminal(
        &journal,
        &reference,
        &Terminal {
            version: 1,
            original: String::from_utf8(journal.snapshot_delivery_origin(&reference).unwrap())
                .unwrap(),
            completed,
            progress_digest: retained_digest(&progress).unwrap(),
            progress,
        },
    )
    .unwrap();
    journal.complete(&reference).unwrap();
    let client = TerminalClient {
        original,
        calls: Mutex::new(0),
    };
    let path = terminal_path(&journal, &reference);
    let before = std::fs::read(&path).unwrap();
    *client.calls.lock().unwrap() = 0;
    let mut output = Vec::new();
    let failure = retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap_err();
    assert!(
        format!("{failure:?}").contains("person/operator retry requires immediate human namespace"),
        "{failure:?}"
    );
    assert_eq!(*client.calls.lock().unwrap(), 0);
    assert!(output.is_empty());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn actor_prerequisite_retained_agent_evidence_must_be_unique_valid_and_present() {
    for damage in ["missing", "malformed", "ambiguous", "contradictory_intent"] {
        let tmp = TempRoot::new();
        let journal = Journal::open(tmp.0.join("intents")).unwrap();
        let (reference, client) = terminal_fixture(&journal);
        retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(journal.load(&reference).is_err());
        let terminal = terminal_path(&journal, &reference);
        let bytes = std::fs::read(&terminal).unwrap();
        match damage {
            "missing" => std::fs::remove_file(&terminal).unwrap(),
            "malformed" => std::fs::write(&terminal, b"{malformed").unwrap(),
            "ambiguous" => {
                let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let original = value["original"].as_str().unwrap();
                let (header, body) = original.split_once('\n').unwrap();
                let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
                let operation =
                    crate::protocol::ids::OperationId::new(uuid::Uuid::new_v4().to_string());
                header["reference"]["operation"] = serde_json::json!(operation.as_str());
                value["original"] = serde_json::json!(format!("{}\n{}", header, body));
                let other = IntentRef {
                    ordinal: reference.ordinal,
                    operation,
                };
                std::fs::write(
                    terminal_path(&journal, &other),
                    serde_json::to_vec(&value).unwrap(),
                )
                .unwrap();
                // Both records independently pass the real bounded origin loader.
                load_original(&journal, &reference).unwrap();
                load_original(&journal, &other).unwrap();
            }
            "contradictory_intent" => {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let original = value["original"].as_str().unwrap();
                let path = journal.root().join(format!(
                    "{:020}-{}.intent",
                    reference.ordinal,
                    reference.operation.as_str()
                ));
                // Equivalent JSON with changed bytes still contradicts the retained original.
                std::fs::write(path, format!("{original}\n")).unwrap();
            }
            _ => unreachable!(),
        }
        let snapshot = || {
            let mut rows: Vec<_> = std::fs::read_dir(journal.root())
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    (
                        path.file_name().unwrap().to_owned(),
                        std::fs::read(path).unwrap(),
                    )
                })
                .collect();
            rows.sort();
            rows
        };
        let before = snapshot();
        *client.calls.lock().unwrap() = 0;
        let mut output = Vec::new();
        let error = retry_to_writer(
            &journal,
            &reference,
            &plan().payload.namespace,
            &client,
            &TestClock,
            &OutputSpec::default(),
            &mut output,
        )
        .unwrap_err();
        if damage == "ambiguous" {
            assert!(
                format!("{error:?}").contains("ambiguous delivery reference"),
                "{error:?}"
            );
        }
        assert_eq!(*client.calls.lock().unwrap(), 0, "{damage}");
        assert!(output.is_empty(), "{damage}");
        assert_eq!(snapshot(), before, "{damage}");
    }
}

#[test]
fn actor_prerequisite_public_retained_agent_delivery_requires_elected_daemon_without_output_or_cleanup()
 {
    let tmp = TempRoot::new();
    let runtime = crate::daemon::paths::RuntimeContext::explicit(
        tmp.0.join("state"),
        tmp.0.join("host.sock"),
        None,
    )
    .unwrap();
    let paths = crate::daemon::paths::NodePaths::resolve(&runtime.state_dir).unwrap();
    let journal = Journal::open(paths.node_dir.join("intents")).unwrap();
    let (reference, client) = terminal_fixture(&journal);
    retry_to_writer(
        &journal,
        &reference,
        &plan().payload.namespace,
        &client,
        &TestClock,
        &OutputSpec::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(journal.load(&reference).is_err());
    let snapshot = || {
        let mut rows: Vec<_> = std::fs::read_dir(journal.root())
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                )
            })
            .collect();
        rows.sort();
        rows
    };
    let before = snapshot();
    let argv = vec![
        "ht".to_owned(),
        "--state-dir".into(),
        runtime.state_dir.display().to_string(),
        "--host-endpoint".into(),
        runtime.host_endpoint.clone().unwrap().display().to_string(),
        "retry".into(),
        reference.recovery_ref(),
    ];
    let mut output = Vec::new();
    let error = super::super::run_in_pane(argv, None, &mut output).unwrap_err();
    assert!(
        matches!(
            error,
            super::super::RunError::Api(ApiError {
                code: ErrorCode::HostUnavailable,
                ..
            })
        ),
        "public retained Agent delivery requires an elected daemon before output or cleanup: {error:?}"
    );
    assert!(output.is_empty());
    assert_eq!(snapshot(), before);
    assert!(!paths.descriptor_path.exists());
    assert!(!paths.database_path.exists());
    assert!(!paths.node_dir.join("contexts").exists());
}

// Real guarded delivery envelopes reach the canonical deciding store APIs. A
// selected phase either fails before its call or loses only the reply after
// the actual canonical commit; nothing fabricates a completed fence.
struct GuardedCanonical {
    base: CanonicalInvites,
    lose_reply: Mutex<Option<&'static str>>,
    fail_before: Mutex<Option<(&'static str, ErrorCode)>>,
    actions: Mutex<Vec<&'static str>>,
    // After the actual canonical completion commits, make the journal root
    // unwritable so only the local terminal save fails.
    readonly_after_complete: Mutex<Option<std::path::PathBuf>>,
}
impl GuardedCanonical {
    fn new(root: &std::path::Path) -> Self {
        Self {
            base: CanonicalInvites::new(root),
            lose_reply: Mutex::new(None),
            fail_before: Mutex::new(None),
            actions: Mutex::new(vec![]),
            readonly_after_complete: Mutex::new(None),
        }
    }
    fn counts(&self) -> (i64, i64, i64) {
        let conn = self.base.context.open_writer().unwrap();
        conn.query_row(
            "SELECT (SELECT count(*) FROM threads),(SELECT count(*) FROM invitations),(SELECT count(*) FROM messages WHERE kind='ordinary')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    }
    fn fence(&self) -> String {
        let conn = self.base.context.open_writer().unwrap();
        conn.query_row("SELECT state FROM channel_handoff_fences", [], |r| r.get(0))
            .unwrap()
    }
}
impl LocalClient for GuardedCanonical {
    fn call_with_output(
        &self,
        c: Command,
        _: &OutputSpec,
        b: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(c, b)
    }
    fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
        use crate::ports::{BootstrapStorePort, StorePort};
        use crate::protocol::handoff::DeliveryAction;
        let Command::HandoffDelivery(request) = c else {
            assert!(
                matches!(
                    c,
                    Command::SeatInspect(_) | Command::Participants(_) | Command::Capabilities
                ),
                "unguarded delivery route {c:?}"
            );
            if matches!(c, Command::Participants(_)) {
                let mut fail = self.fail_before.lock().unwrap();
                if fail.as_ref().is_some_and(|(p, _)| *p == "participants") {
                    let (_, code) = fail.take().unwrap();
                    return Err(ApiError::new(code, "injected participants read failure"));
                }
            }
            if self.base.fake.joined
                && matches!(c, Command::Participants(_) | Command::SeatInspect(_))
            {
                return crate::store::queries::query(
                    &self.base.context,
                    "34750f98bd59fcfc946da45aaabe933b",
                    &c,
                    b,
                );
            }
            return self.base.fake.call(c, b);
        };
        if request.plan.payload.body.len() == 65_536 {
            let wire = crate::protocol::wire::WireRequest {
                version: crate::protocol::wire::PROTOCOL_VERSION,
                request_id: "maximum-body".into(),
                expected_instance: "00000000000040008000000000000001".into(),
                expected_boot: None,
                output: None,
                command: Command::HandoffDelivery(request.clone()),
            };
            let bytes = serde_json::to_vec(&wire).unwrap();
            assert!(bytes.len() + 4 <= crate::protocol::wire::MAX_WIRE_FRAME_BYTES);
            crate::protocol::wire::WireRequest::decode(&bytes).unwrap();
        }
        let canonical = request.plan.payload.namespace.clone();
        let phase = match &request.action {
            DeliveryAction::Status(_) | DeliveryAction::Prepare(_) => {
                return self.base.store.delivery_query(&canonical, &request, b);
            }
            DeliveryAction::Begin(_) => "begin",
            DeliveryAction::Create(_) => "create",
            DeliveryAction::Invite(_) => "invite",
            DeliveryAction::Send(_) => "send",
            DeliveryAction::Complete(_) => "complete",
        };
        self.actions.lock().unwrap().push(phase);
        {
            let mut fail = self.fail_before.lock().unwrap();
            if fail.as_ref().is_some_and(|(p, _)| *p == phase) {
                let (_, code) = fail.take().unwrap();
                return Err(ApiError::new(code, "injected failure before decision"));
            }
        }
        if let DeliveryAction::Send(_) = &request.action {
            loop {
                match self.base.store.delivery_prepare_send_step(
                    &canonical,
                    &request,
                    crate::ports::DurableWorkAdmission::new(16).unwrap(),
                    b,
                )? {
                    crate::ports::SendPreparationProgress::Committed(result) => return Ok(result),
                    crate::ports::SendPreparationProgress::Ready { .. } => break,
                    crate::ports::SendPreparationProgress::More { .. } => {}
                }
            }
        }
        let permit = self.base.store.issue_cooperative_permit(
            crate::store::cooperative_permit_request(
                &request.inner().map_err(ApiError::invalid_request)?,
            )?,
            b,
        )?;
        let result = self
            .base
            .store
            .delivery_mutate(&canonical, &request, permit, b)?;
        if phase == "complete"
            && let Some(root) = self.readonly_after_complete.lock().unwrap().take()
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o500)).unwrap();
        }
        let mut lose = self.lose_reply.lock().unwrap();
        if *lose == Some(phase) {
            *lose = None;
            return Err(ApiError::new(
                ErrorCode::UnknownOutcome,
                "canonical decision committed; reply lost",
            ));
        }
        Ok(result)
    }
}
fn guarded_fixture(tmp: &TempRoot) -> (Journal, IntentRef, DeliveryPlan, GuardedCanonical) {
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut plan = plan();
    plan.payload.channel = HandoffChannel::New {
        name: None,
        topic: "review".into(),
        goal: "review changes".into(),
    };
    let reference = record(&journal, plan.clone());
    (journal, reference, plan, GuardedCanonical::new(&tmp.0))
}
fn guarded_retry<W: std::io::Write>(
    journal: &Journal,
    reference: &IntentRef,
    plan: &DeliveryPlan,
    client: &GuardedCanonical,
    json: bool,
    writer: &mut W,
) -> Result<serde_json::Value, RunError> {
    let original = load_original(journal, reference).unwrap();
    let guarded = FrozenDeliveryClient::new(client, &original).unwrap();
    retry_to_writer(
        journal,
        reference,
        &plan.payload.namespace,
        &guarded,
        &TestClock,
        &OutputSpec {
            format: if json {
                crate::protocol::output::OutputFormat::Json
            } else {
                crate::protocol::output::OutputFormat::Text
            },
            ..OutputSpec::default()
        },
        writer,
    )
}
fn pending_frame(bytes: &[u8]) -> serde_json::Value {
    let frame: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| {
        panic!(
            "missing useful delivery pending report: {e}; stdout={}",
            String::from_utf8_lossy(bytes)
        )
    });
    let report = frame["delivery_pending"].clone();
    assert!(report.is_object(), "{frame}");
    report
}
fn assert_pending_routing(report: &serde_json::Value, reference: &IntentRef, plan: &DeliveryPlan) {
    assert_eq!(report["failed"], true);
    assert_eq!(report["recovery_ref"], reference.recovery_ref());
    assert_eq!(report["compound"], plan.payload.keys.compound.as_str());
    assert_eq!(report["recipient"], "recipient");
    assert_eq!(
        report["namespace"],
        serde_json::to_value(&plan.payload.namespace).unwrap()
    );
    assert_eq!(report["status_is_last_observed"], true);
    assert!(report.get("participation").is_none());
    assert_ne!(report["outcome"], "staged");
    let retry: Vec<String> = serde_json::from_value(report["retry_argv"].clone()).unwrap();
    assert_eq!(retry[0], "env");
    assert_eq!(
        retry[1],
        format!("HERDR_PANE_ID={}", canonical_claim().target.as_str())
    );
    assert_eq!(
        retry[retry.len() - 2..],
        ["retry".to_owned(), reference.recovery_ref()]
    );
    assert!(retry.contains(&"/state".to_owned()) && retry.contains(&"/host.sock".to_owned()));
    assert!(report["manual_launch_after_confirming_no_start_argv"].is_null());
    assert!(!report.to_string().contains("literal '$HOME' work"));
}

#[test]
fn pending_delivery_writer_partial_commits_preserve_observations() {
    for lost in ["create", "invite", "send"] {
        for json in [true, false] {
            let tmp = TempRoot::new();
            let (journal, reference, plan, client) = guarded_fixture(&tmp);
            *client.lose_reply.lock().unwrap() = Some(lost);
            let mut bytes = vec![];
            let error =
                guarded_retry(&journal, &reference, &plan, &client, json, &mut bytes).unwrap_err();
            assert!(
                matches!(&error, RunError::Api(e) if e.code == ErrorCode::UnknownOutcome),
                "original phase error retained: {error:?}"
            );
            let counts = client.counts();
            assert_eq!(
                counts,
                (1, i64::from(lost != "create"), i64::from(lost == "send")),
                "each selected phase actually committed"
            );
            if json {
                let report = pending_frame(&bytes);
                assert_pending_routing(&report, &reference, &plan);
                assert_eq!(report["phase"], lost);
                assert_eq!(report["outcome"], "pending");
                assert_eq!(report["completion"], "not_attempted");
                let thread: Option<String> = client
                    .base
                    .context
                    .open_writer()
                    .unwrap()
                    .query_row("SELECT id FROM threads", [], |r| r.get(0))
                    .ok();
                match lost {
                    "create" => {
                        assert!(report["thread"].is_null(), "unreturned thread is unknown");
                        assert_eq!(report["uncertain"], "thread");
                    }
                    "invite" => {
                        assert_eq!(report["thread"], thread.clone().unwrap());
                        assert!(report["invitation"].is_null());
                        assert_eq!(report["invitation_attempted"], true);
                        assert_eq!(report["uncertain"], "invitation");
                    }
                    _ => {
                        assert_eq!(report["thread"], thread.clone().unwrap());
                        assert_eq!(report["invitation"]["kind"], "invitation");
                        assert!(report["message"].is_null());
                        assert_eq!(report["uncertain"], "message");
                    }
                }
                if lost != "send" {
                    assert!(report["message"].is_null());
                }
            } else {
                let text = String::from_utf8(bytes.clone()).unwrap();
                assert!(text.contains(&format!("phase: {lost}")), "{text}");
                assert!(text.contains("outcome: pending"), "{text}");
                assert!(text.contains("retry_argv: env HERDR_PANE_ID="), "{text}");
                assert!(!text.contains("staged_unbound") && !text.contains("outcome: staged"));
            }
            assert!(bytes.len() <= 1024 * 1024);
            assert_eq!(client.fence(), "live");
            assert!(journal.load(&reference).is_ok());
            assert!(!terminal_path(&journal, &reference).exists());
            let report =
                guarded_retry(&journal, &reference, &plan, &client, json, &mut vec![]).unwrap();
            assert_eq!(report["outcome"], "staged");
            assert_eq!(client.counts(), (1, 1, 1), "replay reused exact children");
            assert_eq!(client.fence(), "completed");
        }
    }
}

#[test]
fn pending_delivery_writer_completion_reply_loss_is_uncertain() {
    for (committed, json) in [(true, true), (true, false), (false, true)] {
        let tmp = TempRoot::new();
        let (journal, reference, plan, client) = guarded_fixture(&tmp);
        if committed {
            *client.lose_reply.lock().unwrap() = Some("complete");
        } else {
            *client.fail_before.lock().unwrap() = Some(("complete", ErrorCode::HostUnavailable));
        }
        let mut bytes = vec![];
        assert!(guarded_retry(&journal, &reference, &plan, &client, json, &mut bytes).is_err());
        if json {
            let report = pending_frame(&bytes);
            assert_pending_routing(&report, &reference, &plan);
            assert_eq!(report["phase"], "complete");
            assert_eq!(report["outcome"], "completion_uncertain");
            assert_eq!(report["completion"], "uncertain");
            assert_eq!(report["uncertain"], "completion");
            assert_eq!(report["last_observed_state"], "live");
            assert!(report["thread"].is_string());
            assert_eq!(report["invitation"]["kind"], "invitation");
            assert_eq!(report["message"]["kind"], "message_sent");
        } else {
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.contains("phase: complete"), "{text}");
            assert!(text.contains("outcome: completion_uncertain"), "{text}");
        }
        assert_eq!(client.fence(), if committed { "completed" } else { "live" });
        assert_eq!(client.counts(), (1, 1, 1));
        assert!(!terminal_path(&journal, &reference).exists());
        client.actions.lock().unwrap().clear();
        let report =
            guarded_retry(&journal, &reference, &plan, &client, json, &mut vec![]).unwrap();
        assert_eq!(report["outcome"], "staged");
        assert_eq!(client.counts(), (1, 1, 1));
        assert_eq!(client.fence(), "completed");
        assert!(
            client
                .actions
                .lock()
                .unwrap()
                .iter()
                .all(|a| matches!(*a, "begin" | "complete")),
            "replay restaged work: {:?}",
            client.actions.lock().unwrap()
        );
    }
}

#[test]
fn pending_delivery_writer_output_failure_stays_replayable() {
    for flush in [false, true] {
        let tmp = TempRoot::new();
        let (journal, reference, plan, client) = guarded_fixture(&tmp);
        *client.lose_reply.lock().unwrap() = Some("send");
        let progress_before = || std::fs::read(handoff::progress_path(&journal, &reference)).ok();
        let mut writer = FailedWriter {
            flush,
            ..Default::default()
        };
        let error =
            guarded_retry(&journal, &reference, &plan, &client, true, &mut writer).unwrap_err();
        assert!(matches!(error, RunError::Io(_)), "{error:?}");
        if flush {
            assert!(!writer.bytes.is_empty(), "flush loss reached the writer");
        }
        let saved = progress_before();
        assert!(saved.is_some());
        assert!(journal.load(&reference).is_ok());
        assert_eq!(client.fence(), "live");
        assert!(!terminal_path(&journal, &reference).exists());
        let report =
            guarded_retry(&journal, &reference, &plan, &client, true, &mut vec![]).unwrap();
        assert_eq!(report["outcome"], "staged");
        assert_eq!(client.counts(), (1, 1, 1));
    }
}

#[test]
fn pending_delivery_writer_authority_and_corrupt_history_stay_silent() {
    for code in [
        ErrorCode::Unauthorized,
        ErrorCode::CallerUnverified,
        ErrorCode::InstanceMismatch,
        ErrorCode::OperationPayloadMismatch,
    ] {
        let tmp = TempRoot::new();
        let (journal, reference, plan, client) = guarded_fixture(&tmp);
        *client.fail_before.lock().unwrap() = Some(("invite", code.clone()));
        let mut bytes = vec![];
        assert!(guarded_retry(&journal, &reference, &plan, &client, true, &mut bytes).is_err());
        assert!(bytes.is_empty(), "authority refusal {code:?} reported");
    }
    // Genuine Begin refusals before canonical admission stay silent.
    for code in [ErrorCode::Unauthorized, ErrorCode::InvalidRequest] {
        let tmp = TempRoot::new();
        let (journal, reference, plan, client) = guarded_fixture(&tmp);
        *client.fail_before.lock().unwrap() = Some(("begin", code.clone()));
        let mut bytes = vec![];
        assert!(guarded_retry(&journal, &reference, &plan, &client, true, &mut bytes).is_err());
        assert!(bytes.is_empty(), "Begin refusal {code:?} reported");
        assert_eq!(client.counts(), (0, 0, 0));
    }
    // Corrupt retained progress never becomes a trusted pending report.
    let tmp = TempRoot::new();
    let (journal, reference, plan, client) = guarded_fixture(&tmp);
    *client.lose_reply.lock().unwrap() = Some("invite");
    assert!(guarded_retry(&journal, &reference, &plan, &client, true, &mut vec![]).is_err());
    let path = handoff::progress_path(&journal, &reference);
    let mut progress: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    progress["unexpected"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&progress).unwrap()).unwrap();
    let mut bytes = vec![];
    assert!(guarded_retry(&journal, &reference, &plan, &client, true, &mut bytes).is_err());
    assert!(bytes.is_empty(), "{}", String::from_utf8_lossy(&bytes));
    // Wrong current namespace refuses before any client call or output.
    let tmp = TempRoot::new();
    let (journal, reference, plan, client) = guarded_fixture(&tmp);
    let original = load_original(&journal, &reference).unwrap();
    let guarded = FrozenDeliveryClient::new(&client, &original).unwrap();
    let mut namespace = plan.payload.namespace.clone();
    namespace.state_dir = "/copied-instance".into();
    let mut bytes = vec![];
    assert!(
        retry_to_writer(
            &journal,
            &reference,
            &namespace,
            &guarded,
            &TestClock,
            &OutputSpec::default(),
            &mut bytes
        )
        .is_err()
    );
    assert!(bytes.is_empty());
    assert!(client.actions.lock().unwrap().is_empty());
}

// After publication, a Begin transport failure or committed reply loss is
// indistinguishable to the client: report with unknown canonical status.
#[test]
fn pending_delivery_writer_begin_failure_reports_unknown_status() {
    for (lose, json) in [(true, true), (false, true), (true, false)] {
        let tmp = TempRoot::new();
        let (journal, reference, plan, client) = guarded_fixture(&tmp);
        if lose {
            *client.lose_reply.lock().unwrap() = Some("begin");
        } else {
            *client.fail_before.lock().unwrap() = Some(("begin", ErrorCode::HostUnavailable));
        }
        let mut bytes = vec![];
        assert!(guarded_retry(&journal, &reference, &plan, &client, json, &mut bytes).is_err());
        if json {
            let report = pending_frame(&bytes);
            assert_eq!(report["phase"], "begin");
            assert_eq!(report["outcome"], "pending");
            assert!(report["last_observed_state"].is_null());
            assert_eq!(report["status_unknown"], true);
            assert_eq!(report["status_is_last_observed"], false);
            assert_eq!(report["uncertain"], "begin");
            for absent in ["thread", "invitation", "message"] {
                assert!(report[absent].is_null());
            }
            assert_eq!(report["recovery_ref"], reference.recovery_ref());
            let retry: Vec<String> = serde_json::from_value(report["retry_argv"].clone()).unwrap();
            assert_eq!(
                retry[retry.len() - 2..],
                ["retry".to_owned(), reference.recovery_ref()]
            );
        } else {
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.contains("phase: begin"), "{text}");
            assert!(text.contains("status_unknown: true"), "{text}");
        }
        assert_eq!(client.counts(), (0, 0, 0));
        let report =
            guarded_retry(&journal, &reference, &plan, &client, json, &mut vec![]).unwrap();
        assert_eq!(report["outcome"], "staged");
        assert_eq!(client.counts(), (1, 1, 1));
    }
}

#[test]
fn pending_delivery_writer_progress_io_failure_reports_after_begin() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempRoot::new();
    let (journal, reference, plan, client) = guarded_fixture(&tmp);
    *client.lose_reply.lock().unwrap() = Some("invite");
    assert!(guarded_retry(&journal, &reference, &plan, &client, true, &mut vec![]).is_err());
    let path = handoff::progress_path(&journal, &reference);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let mut bytes = vec![];
    let failed = guarded_retry(&journal, &reference, &plan, &client, true, &mut bytes);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(failed.is_err());
    let report = pending_frame(&bytes);
    assert_eq!(report["phase"], "progress");
    assert_eq!(report["last_observed_state"], "live");
    assert_eq!(report["status_unknown"], false);
    assert!(
        report["thread"].is_null(),
        "unreadable retained progress is unknown"
    );
    assert_eq!(client.counts(), (1, 1, 0));
    let report = guarded_retry(&journal, &reference, &plan, &client, true, &mut vec![]).unwrap();
    assert_eq!(report["outcome"], "staged");
    assert_eq!(client.counts(), (1, 1, 1));
}

#[test]
fn pending_delivery_writer_terminal_save_failure_after_completion_reports() {
    use std::os::unix::fs::PermissionsExt;
    for json in [true, false] {
        let tmp = TempRoot::new();
        let (journal, reference, plan, client) = guarded_fixture(&tmp);
        *client.readonly_after_complete.lock().unwrap() = Some(journal.root().to_path_buf());
        let mut bytes = vec![];
        let failed = guarded_retry(&journal, &reference, &plan, &client, json, &mut bytes);
        std::fs::set_permissions(journal.root(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(failed.is_err());
        assert_eq!(client.fence(), "completed");
        if json {
            let report = pending_frame(&bytes);
            assert_eq!(report["phase"], "terminal");
            assert_eq!(report["outcome"], "completed_terminal_unconfirmed");
            assert_eq!(report["completion"], "observed");
            assert_eq!(report["last_observed_state"], "completed");
            assert!(report["uncertain"].is_null());
            assert!(report["thread"].is_string());
            assert_eq!(report["message"]["kind"], "message_sent");
        } else {
            let text = String::from_utf8(bytes).unwrap();
            assert!(text.contains("phase: terminal"), "{text}");
            assert!(
                text.contains("outcome: completed_terminal_unconfirmed"),
                "{text}"
            );
        }
        assert!(!terminal_path(&journal, &reference).exists());
        client.actions.lock().unwrap().clear();
        let report =
            guarded_retry(&journal, &reference, &plan, &client, json, &mut vec![]).unwrap();
        assert_eq!(report["outcome"], "staged");
        assert_eq!(client.counts(), (1, 1, 1));
        assert!(
            client.actions.lock().unwrap().iter().all(|a| *a == "begin"),
            "{:?}",
            client.actions.lock().unwrap()
        );
    }
}

// A retained invitation_attempted=false never labels the invitation uncertain.
#[test]
fn pending_delivery_writer_invite_refusal_before_attempt_is_not_uncertain() {
    let tmp = TempRoot::new();
    let (journal, reference, plan, client) = guarded_fixture(&tmp);
    // The joined-recipient read fails before the keyed invitation is attempted.
    *client.fail_before.lock().unwrap() = Some(("participants", ErrorCode::HostUnavailable));
    let mut bytes = vec![];
    assert!(guarded_retry(&journal, &reference, &plan, &client, true, &mut bytes).is_err());
    let report = pending_frame(&bytes);
    assert_eq!(report["phase"], "invite");
    assert_eq!(report["invitation_attempted"], false);
    assert!(report["invitation"].is_null());
    assert!(report["uncertain"].is_null(), "{report}");
    assert_eq!(client.counts(), (1, 0, 0));
}

// The delivery pending inspection continuation must be the public read-only
// pending-ops command under the exact original pinned routing, accepted by
// the same public parser the binary uses, in JSON argv and in text.
fn assert_delivery_inspect_is_public_pending_ops(plan: &DeliveryPlan, json: bool, bytes: &[u8]) {
    let ns = &plan.payload.namespace;
    let state = ns.state_dir.to_string_lossy().into_owned();
    let host = ns.host_endpoint.to_string_lossy().into_owned();
    let expected: Vec<String> = vec![
        crate::cli::hook::CLI_ARGV0.into(),
        "--state-dir".into(),
        state.clone(),
        "--host-endpoint".into(),
        host.clone(),
        "pending-ops".into(),
    ];
    let argv: Vec<String> = if json {
        serde_json::from_value(pending_frame(bytes)["inspect_argv"].clone()).unwrap()
    } else {
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let line = text
            .lines()
            .find_map(|l| l.strip_prefix("inspect_argv: "))
            .unwrap_or_else(|| panic!("no inspect_argv line: {text}"));
        shlex::split(line).unwrap_or_else(|| panic!("unquotable: {line}"))
    };
    let parsed = crate::cli::commands::parse_argv(&argv)
        .unwrap_or_else(|e| panic!("public parser refused inspect_argv {argv:?}: {e:?}"));
    assert!(
        matches!(
            parsed.action,
            crate::cli::commands::CliAction::PendingOps(_)
        ),
        "inspect_argv is not the read-only pending-ops action: {argv:?}"
    );
    assert_eq!(
        parsed.actor,
        crate::cli::actor_route::InvocationActor::Agent
    );
    assert_eq!(
        parsed.output.context.state_dir.as_deref(),
        Some(state.as_str())
    );
    assert_eq!(parsed.output.context.host.as_deref(), Some(host.as_str()));
    assert_eq!(argv, expected);
}

#[test]
fn pending_delivery_writer_inspect_argv_parses_as_public_pending_ops() {
    for json in [true, false] {
        for case in ["begin", "create", "invite", "send", "complete", "terminal"] {
            let tmp = TempRoot::new();
            let (journal, reference, plan, client) = guarded_fixture(&tmp);
            if case == "terminal" {
                *client.readonly_after_complete.lock().unwrap() =
                    Some(journal.root().to_path_buf());
            } else {
                *client.lose_reply.lock().unwrap() = Some(case);
            }
            let mut bytes = vec![];
            let failed = guarded_retry(&journal, &reference, &plan, &client, json, &mut bytes);
            if case == "terminal" {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(journal.root(), std::fs::Permissions::from_mode(0o700))
                    .unwrap();
            }
            assert!(failed.is_err(), "{case}");
            assert!(
                String::from_utf8_lossy(&bytes).contains(&format!("phase: {case}"))
                    || (json && pending_frame(&bytes)["phase"] == case),
                "{case}: {}",
                String::from_utf8_lossy(&bytes)
            );
            assert_delivery_inspect_is_public_pending_ops(&plan, json, &bytes);
        }
    }
}

#[test]
fn maximum_escaped_delivery_origin_and_terminal_survive_cleanup_retry() {
    let temp = TempRoot::new();
    let journal = Journal::open(temp.0.join("intents")).unwrap();
    let mut plan = plan();
    plan.payload.body = "\u{1}".repeat(65_536);
    let reference = record(&journal, plan.clone());
    let origin = journal.snapshot_delivery_origin(&reference).unwrap();
    assert!(origin.len() >= 393_216);
    assert_eq!(
        delivery_plan(&Journal::decode_delivery_origin(&reference, &origin).unwrap())
            .unwrap()
            .payload
            .body,
        "\u{1}".repeat(65_536)
    );
    let progress = Progress::default();
    // The terminal serializer nests the exact original JSON as a string;
    // every six-byte body escape gains one more backslash.
    let terminal = Terminal {
        version: 1,
        original: String::from_utf8(origin).unwrap(),
        completed: HandoffResult {
            compound: plan.payload.keys.compound.clone(),
            thread: Some(ThreadId::new("t1")),
            state: HandoffState::Completed,
        },
        progress_digest: retained_digest(&progress).unwrap(),
        progress,
    };
    let bytes = serde_json::to_vec(&terminal).unwrap();
    assert!(bytes.len() >= 458_752);
    eprintln!(
        "maximum escaped delivery: origin={} terminal={}",
        terminal.original.len(),
        bytes.len()
    );
    save_terminal(&journal, &reference, &terminal).unwrap();
    let saved: Terminal =
        serde_json::from_slice(&std::fs::read(terminal_path(&journal, &reference)).unwrap())
            .unwrap();
    let pending = Journal::decode_delivery_origin(&reference, saved.original.as_bytes()).unwrap();
    assert_eq!(
        delivery_plan(&pending).unwrap().payload.body,
        "\u{1}".repeat(65_536)
    );
}

#[test]
fn maximum_instruction_file_freezes_through_existing_delivery_and_retry() {
    for body in ["x".repeat(65_536), "\u{1}".repeat(65_536)] {
        let temp = TempRoot::new();
        let path = temp.0.join("instructions");
        std::fs::write(&path, &body).unwrap();
        let parsed = crate::cli::commands::parse_argv([
            "ht",
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--new-thread",
            "--topic",
            "review",
            "--goal",
            "review changes",
            "--deadline",
            "7",
            "--invite-deadline",
            "11",
            "--file",
            path.to_str().unwrap(),
        ])
        .unwrap();
        let crate::cli::commands::CliAction::TopologyHandoff(request) = parsed.action else {
            panic!("wrong action")
        };
        let mut plan = plan();
        plan.payload.body = request.body;
        plan.payload.policy = request.policy;
        plan.payload.channel = HandoffChannel::New {
            name: None,
            topic: "review".into(),
            goal: "review changes".into(),
        };
        let journal = Journal::open(temp.0.join("intents")).unwrap();
        let reference = publish(&journal, plan.clone(), canonical_claim(), &TestClock).unwrap();
        let original = journal.snapshot_delivery_origin(&reference).unwrap();
        std::fs::write(&path, "changed").unwrap();
        std::fs::remove_file(&path).unwrap();
        let client = GuardedCanonical::new(&temp.0);
        *client.lose_reply.lock().unwrap() = Some("send");
        assert!(
            guarded_retry(&journal, &reference, &plan, &client, true, &mut Vec::new()).is_err()
        );
        guarded_retry(&journal, &reference, &plan, &client, true, &mut Vec::new()).unwrap();
        let conn = client.base.context.open_writer().unwrap();
        let saved: String = conn
            .query_row("SELECT body FROM messages WHERE kind='ordinary'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(saved, body);
        let (confirmation, duration, invite_deadline, receipt) =
            handoff_policy_canonical_values(&client);
        assert_eq!((confirmation.as_str(), duration), ("require_ack", 11000));
        assert_eq!(invite_deadline, receipt.decision_at + 11000);
        assert_eq!(receipt.frozen_duration_ms, 7000);
        assert_eq!(receipt.deadline_at, Some(receipt.decision_at + 7000));
        let frozen = (confirmation, duration, invite_deadline, receipt.deadline_at);
        let terminal = read_terminal(&journal, &reference).unwrap().unwrap();
        assert_eq!(terminal.original.as_bytes(), original);
        guarded_retry(&journal, &reference, &plan, &client, true, &mut Vec::new()).unwrap();
        assert_eq!(client.counts(), (1, 1, 1));
        let replay = handoff_policy_canonical_values(&client);
        assert_eq!((replay.0, replay.1, replay.2, replay.3.deadline_at), frozen);
    }
}

fn handoff_policy_advance_work(
    context: &crate::store::connection::StoreContext,
    db: &mut rusqlite::Connection,
) {
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: crate::protocol::time::Cancellation::default(),
    };
    for _ in 0..100 {
        let jobs: Vec<String> = db
            .prepare("SELECT id FROM work_jobs WHERE status='pending' ORDER BY ordinal")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        if jobs.is_empty() {
            return;
        }
        for job in jobs {
            crate::store::materialization::advance_work(
                db,
                &job,
                crate::ports::DurableWorkAdmission::new(16).unwrap(),
                &budget,
                context.clock(),
            )
            .unwrap();
        }
    }
    panic!("bounded canonical work failed to drain");
}

// These checks use the canonical deciding delivery handler, not fake child results.
fn handoff_policy_canonical_values(
    client: &GuardedCanonical,
) -> (String, i64, i64, crate::store::effective::EffectiveReceipt) {
    let db = client.base.context.open_writer().unwrap();
    let (mode, duration, deadline): (String, i64, i64) = db
        .query_row(
            "SELECT confirmation,frozen_duration_ms,deadline_at FROM invitations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let message: String = db
        .query_row("SELECT id FROM messages WHERE kind='ordinary'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let receipt = crate::store::effective::effective_receipt(&db, &message, "recipient")
        .unwrap()
        .unwrap();
    (mode, duration, deadline, receipt)
}

#[test]
fn handoff_policy_default_choice_resolves_at_child_creation_then_replay_preserves_deadlines() {
    use crate::protocol::{commands::InvitationConfirmation, handoff::HandoffPolicy};
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut p = plan();
    p.payload.channel = HandoffChannel::New {
        name: None,
        topic: "review".into(),
        goal: "review changes".into(),
    };
    p.payload.policy = Some(HandoffPolicy {
        deadline_millis: None,
        invitation_confirmation: InvitationConfirmation::RequireAck,
        invite_deadline_millis: None,
    });
    let reference = record(&journal, p.clone());
    let original = journal.snapshot_delivery_origin(&reference).unwrap();
    let mut client = GuardedCanonical::new(&tmp.0);
    *client.fail_before.lock().unwrap() = Some(("invite", ErrorCode::HostUnavailable));
    assert!(guarded_retry(&journal, &reference, &p, &client, true, &mut vec![]).is_err());
    assert_eq!(client.counts(), (1, 0, 0));
    let settings = |message, invitation| crate::store::StoreSettings {
        invitation_default_ms: Some(invitation),
        message_limits: crate::store::messages::MessageLimits {
            receipt_duration_ms: message,
            ..Default::default()
        },
        ..Default::default()
    };
    client.base.store = crate::store::SqliteStore::new(
        crate::store::connection::StoreContext::new(
            tmp.0.join("canonical.db"),
            std::sync::Arc::new(TestClock),
        )
        .with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        settings(13000, 17000),
    )
    .unwrap();
    *client.lose_reply.lock().unwrap() = Some("send");
    assert!(guarded_retry(&journal, &reference, &p, &client, true, &mut vec![]).is_err());
    let before = handoff_policy_canonical_values(&client);
    assert_eq!(
        (&before.0, before.1, before.3.frozen_duration_ms),
        (&"require_ack".to_owned(), 17000, 13000)
    );
    client.base.store = crate::store::SqliteStore::new(
        crate::store::connection::StoreContext::new(
            tmp.0.join("canonical.db"),
            std::sync::Arc::new(TestClock),
        )
        .with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        settings(23000, 29000),
    )
    .unwrap();
    guarded_retry(&journal, &reference, &p, &client, true, &mut vec![]).unwrap();
    assert_eq!(handoff_policy_canonical_values(&client), before);
    assert_eq!(
        read_terminal(&journal, &reference)
            .unwrap()
            .unwrap()
            .original
            .as_bytes(),
        original
    );
    assert_eq!(client.counts(), (1, 1, 1));
}

#[test]
fn handoff_policy_lazy_invitation_still_warns_sender_for_unregistered_ack_required_recipient() {
    use crate::protocol::{commands::InvitationConfirmation, handoff::HandoffPolicy};
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut p = plan();
    p.payload.channel = HandoffChannel::New {
        name: None,
        topic: "review".into(),
        goal: "review changes".into(),
    };
    p.payload.policy = Some(HandoffPolicy {
        deadline_millis: Some(7000),
        invitation_confirmation: InvitationConfirmation::Lazy,
        invite_deadline_millis: None,
    });
    let reference = record(&journal, p.clone());
    let client = GuardedCanonical::new(&tmp.0);
    guarded_retry(&journal, &reference, &p, &client, true, &mut vec![]).unwrap();
    let (mode, _, _, receipt) = handoff_policy_canonical_values(&client);
    assert_eq!(mode, "lazy");
    assert_eq!(
        receipt.state,
        crate::store::effective::EffectiveReceiptState::Pending
    );
    assert_eq!(receipt.frozen_duration_ms, 7000);
    assert_eq!(receipt.available_at, None);
    assert_eq!(receipt.deadline_at, Some(receipt.decision_at + 7000));
    struct Due(i64);
    impl Clock for Due {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    let context = crate::store::connection::StoreContext::new(
        tmp.0.join("canonical.db"),
        std::sync::Arc::new(Due(receipt.deadline_at.unwrap() + 1)),
    )
    .with_test_host();
    let mut db = context.open_writer().unwrap();
    let registered: Option<i64> = db
        .query_row(
            "SELECT registered_at FROM occupant_bindings WHERE seat_id='recipient'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(registered, None);
    handoff_policy_advance_work(&context, &mut db);
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    for _ in 0..100 {
        crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
        if crate::store::effective::effective_receipt(&db, &receipt.message_id, "recipient")
            .unwrap()
            .unwrap()
            .warning_message_id
            .is_some()
        {
            break;
        }
    }
    let warned = crate::store::effective::effective_receipt(&db, &receipt.message_id, "recipient")
        .unwrap()
        .unwrap();
    let warning = warned
        .warning_message_id
        .expect("unregistered recipient is discovered overdue");
    let origin: (String, String) = db
        .query_row(
            "SELECT seat_id,source_message_id FROM receipt_warning_origins WHERE warning_id=?1",
            [&warning],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(origin, ("sender".into(), receipt.message_id));
    assert_eq!(
        db.query_row("SELECT state FROM invitations", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
}

#[test]
fn handoff_policy_missing_readiness_guarded_launch_stages_unregistered_body_and_warns_sender() {
    struct CanonicalLaunch(CanonicalInvites);
    impl LocalClient for CanonicalLaunch {
        fn call_with_output(
            &self,
            c: Command,
            _: &OutputSpec,
            b: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(c, b)
        }
        fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
            match c {
                Command::BeginHandoff(_)
                | Command::CompleteHandoff(_)
                | Command::CreateThread(_)
                | Command::Invite(_)
                | Command::SendMessage(_) => self.0.canonical(c, b),
                _ => self.0.fake.call(c, b),
            }
        }
    }
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut client = CanonicalLaunch(CanonicalInvites::with_recipient(
        &tmp.0,
        "seat_launch",
        "w9:p1",
    ));
    let (result, submitted, _, records) =
        super::super::launch::tests::task10_hermes_native_fixture(false, |launcher, launch| {
            let env = launcher.parts.env.snapshot();
            let request = handoff::HandoffRequest {
                automatic: false,
                policy: Some(crate::protocol::handoff::HandoffPolicy {
                    deadline_millis: Some(7000),
                    invitation_confirmation:
                        crate::protocol::commands::InvitationConfirmation::Lazy,
                    invite_deadline_millis: None,
                }),
                thread: None,
                thread_name: None,
                topic: Some("review".into()),
                goal: Some("review changes".into()),
                body: "durable body unavailable to native argv".into(),
                launch,
            };
            let output = OutputSpec {
                context: crate::protocol::output::ContinuationContext {
                    state_dir: env.state_dir.map(|p| p.display().to_string()),
                    host: env.host_endpoint.map(|p| p.display().to_string()),
                },
                ..Default::default()
            };
            handoff::start(
                &journal,
                request,
                canonical_claim(),
                "peer",
                &client,
                launcher,
                &TestClock,
                &output,
                &mut vec![],
            )
        });
    assert!(
        result.is_ok(),
        "actual safe Hermes launch with absent owned integration: {result:?}"
    );
    assert_eq!(submitted.len(), 1);
    assert!(submitted[0].configured_hook.is_none());
    assert!(
        submitted[0]
            .argv
            .iter()
            .all(|a| !a.contains("durable body unavailable"))
    );
    assert_eq!(records.len(), 1);
    let mut db = client.0.context.open_writer().unwrap();
    let message: String = db
        .query_row("SELECT id FROM messages WHERE kind='ordinary'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let receipt = crate::store::effective::effective_receipt(&db, &message, "seat_launch")
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.state,
        crate::store::effective::EffectiveReceiptState::Pending
    );
    assert_eq!(receipt.frozen_duration_ms, 7000);
    assert_eq!(receipt.available_at, None);
    assert_eq!(receipt.deadline_at, Some(receipt.decision_at + 7000));
    let registered: Option<i64> = db
        .query_row(
            "SELECT registered_at FROM occupant_bindings WHERE seat_id='seat_launch'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(registered, None);
    struct Due(i64);
    impl Clock for Due {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    let context = crate::store::connection::StoreContext::new(
        tmp.0.join("canonical.db"),
        std::sync::Arc::new(Due(receipt.deadline_at.unwrap() + 1)),
    )
    .with_test_host();
    handoff_policy_advance_work(&context, &mut db);
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    for _ in 0..100 {
        crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
        if crate::store::effective::effective_receipt(&db, &message, "seat_launch")
            .unwrap()
            .unwrap()
            .warning_message_id
            .is_some()
        {
            break;
        }
    }
    let warning = crate::store::effective::effective_receipt(&db, &message, "seat_launch")
        .unwrap()
        .unwrap()
        .warning_message_id
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT seat_id FROM receipt_warning_origins WHERE warning_id=?1",
            [&warning],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "sender"
    );
    assert_eq!(
        db.query_row(
            "SELECT confirmation||':'||state FROM invitations",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "lazy:pending"
    );
    // Fixture already has a cooperative binding but no registration; an actual
    // accountable ACK is independently authorized and clears the warning normally.
    let mut claim = canonical_claim();
    claim.seat = SeatId::new("seat_launch");
    claim.target = crate::protocol::ids::HostTargetId::new("w9:p1");
    let ack = crate::protocol::commands::Ack {
        messages: vec![MessageId::new(&message)],
        operation: crate::protocol::ids::OperationId::new("late-accountable-ack"),
        claim,
    };
    client.0.store = crate::store::SqliteStore::new(
        crate::store::connection::StoreContext::new(
            tmp.0.join("canonical.db"),
            std::sync::Arc::new(Due(receipt.deadline_at.unwrap() + 2)),
        )
        .with_test_host(),
        "34750f98bd59fcfc946da45aaabe933b",
        Default::default(),
    )
    .unwrap();
    client
        .0
        .canonical(
            Command::Ack(ack),
            &CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: crate::protocol::time::Cancellation::default(),
            },
        )
        .unwrap();
    assert_eq!(
        crate::store::effective::effective_receipt(&db, &message, "seat_launch")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::Acknowledged
    );
    assert_eq!(db.query_row("SELECT count(*) FROM warning_conditions WHERE open_warning_id=?1 AND clear_warning_id IS NOT NULL", [&warning], |r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn handoff_policy_already_joined_unavailable_recipient_warns_without_new_invitation() {
    use crate::protocol::{
        commands::{Accept, CreateThread, InvitationConfirmation, Invite},
        handoff::HandoffPolicy,
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut client = GuardedCanonical::new(&tmp.0);
    let budget = super::super::cooperative_budget(&TestClock);
    let CommandResult::ThreadCreated(thread) = client
        .base
        .canonical(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "joined".into(),
                goal: "joined".into(),
                operation: OperationId::new("prior-create"),
                claim: canonical_claim(),
            }),
            &budget,
        )
        .unwrap()
    else {
        panic!()
    };
    client
        .base
        .canonical(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::Lazy),
                thread: thread.clone(),
                seat: SeatId::new("recipient"),
                deadline_millis: None,
                operation: OperationId::new("prior-invite"),
                claim: canonical_claim(),
            }),
            &budget,
        )
        .unwrap();
    let mut recipient = canonical_claim();
    recipient.seat = SeatId::new("recipient");
    recipient.target = crate::protocol::ids::HostTargetId::new("w1:p2");
    client
        .base
        .canonical(
            Command::Accept(Accept {
                thread: thread.clone(),
                operation: OperationId::new("prior-accept"),
                claim: recipient,
            }),
            &budget,
        )
        .unwrap();
    client
        .base
        .context
        .open_writer()
        .unwrap()
        .execute(
            "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='recipient'",
            [],
        )
        .unwrap();
    client.base.fake.joined = true; // Reads below are canonical, not this synthetic participant flag.
    let mut p = plan();
    p.payload.channel = HandoffChannel::Existing { thread };
    p.payload.policy = Some(HandoffPolicy {
        deadline_millis: Some(7000),
        invitation_confirmation: InvitationConfirmation::Lazy,
        invite_deadline_millis: None,
    });
    let reference = record(&journal, p.clone());
    guarded_retry(&journal, &reference, &p, &client, true, &mut vec![]).unwrap();
    assert_eq!(
        client.counts(),
        (1, 1, 1),
        "retain only the prior accepted invitation"
    );
    assert!(!client.actions.lock().unwrap().contains(&"invite"));
    let (_, _, _, receipt) = handoff_policy_canonical_values(&client);
    assert_eq!(receipt.available_at, None);
    assert_eq!(receipt.frozen_duration_ms, 7000);
    assert_eq!(receipt.deadline_at, Some(receipt.decision_at + 7000));
    struct Due(i64);
    impl Clock for Due {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    let context = crate::store::connection::StoreContext::new(
        tmp.0.join("canonical.db"),
        std::sync::Arc::new(Due(receipt.deadline_at.unwrap() + 1)),
    )
    .with_test_host();
    let mut db = context.open_writer().unwrap();
    handoff_policy_advance_work(&context, &mut db);
    let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
    for _ in 0..100 {
        crate::store::receipts::scan_due(&context, &mut db, 100, &mut cursor).unwrap();
    }
    let warning = crate::store::effective::effective_receipt(&db, &receipt.message_id, "recipient")
        .unwrap()
        .unwrap()
        .warning_message_id
        .unwrap();
    let origin: (String, String) = db
        .query_row(
            "SELECT seat_id,source_message_id FROM receipt_warning_origins WHERE warning_id=?1",
            [&warning],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(origin, ("sender".into(), receipt.message_id));
}

#[test]
fn handoff_policy_completed_canonical_status_bypasses_missing_capabilities_but_pending_refuses() {
    struct MissingCapabilities<'a>(&'a GuardedCanonical, std::sync::atomic::AtomicUsize);
    impl LocalClient for MissingCapabilities<'_> {
        fn call_with_output(
            &self,
            c: Command,
            _: &OutputSpec,
            b: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(c, b)
        }
        fn call(&self, c: Command, b: &CallBudget) -> Result<CommandResult, ApiError> {
            if matches!(c, Command::Capabilities) {
                self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Ok(CommandResult::Capabilities(
                    crate::protocol::results::CapabilityList {
                        capabilities: vec![],
                    },
                ));
            }
            assert!(
                matches!(&c, Command::HandoffDelivery(q) if matches!(q.action, crate::protocol::handoff::DeliveryAction::Status(_))),
                "only actual canonical read is permitted: {c:?}"
            );
            self.0.call(c, b)
        }
    }
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut p = plan();
    p.payload.channel = HandoffChannel::New {
        name: None,
        topic: "review".into(),
        goal: "review".into(),
    };
    p.payload.policy = Some(crate::protocol::handoff::HandoffPolicy {
        deadline_millis: Some(7000),
        invitation_confirmation: crate::protocol::commands::InvitationConfirmation::Lazy,
        invite_deadline_millis: None,
    });
    let reference = record(&journal, p.clone());
    let base = GuardedCanonical::new(&tmp.0);
    let missing = MissingCapabilities(&base, std::sync::atomic::AtomicUsize::new(0));
    let attempt = || {
        let pending = load_original(&journal, &reference).unwrap();
        let frozen = FrozenDeliveryClient::new(&missing, &pending).unwrap();
        retry_to_writer(
            &journal,
            &reference,
            &p.payload.namespace,
            &frozen,
            &TestClock,
            &OutputSpec::default(),
            &mut vec![],
        )
    };
    assert!(
        attempt().is_err(),
        "pending operation needs current semantic capability"
    );
    assert_eq!(missing.1.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(base.actions.lock().unwrap().is_empty());
    assert_eq!(base.counts(), (0, 0, 0));
    *base.lose_reply.lock().unwrap() = Some("complete");
    assert!(guarded_retry(&journal, &reference, &p, &base, true, &mut vec![]).is_err());
    assert_eq!(base.fence(), "completed");
    base.actions.lock().unwrap().clear();
    let before = base.counts();
    assert_eq!(attempt().unwrap()["outcome"], "staged");
    assert_eq!(
        missing.1.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "completed presentation performs no capabilities read"
    );
    assert!(
        base.actions.lock().unwrap().is_empty(),
        "no Begin or child operation after completion"
    );
    assert_eq!(base.counts(), before);
}

// Catches conflating acceptance with ACK, or clearing both origin warnings when
// only one of a timed handoff's independent obligations has been settled.
#[test]
fn timed_handoff_acceptance_and_ack_clear_only_their_own_overdue_condition() {
    use crate::protocol::{
        commands::{Accept, Ack, InvitationConfirmation},
        handoff::HandoffPolicy,
    };
    struct At(i64);
    impl Clock for At {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(self.0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    for accept_first in [true, false] {
        let tmp = TempRoot::new();
        let journal = Journal::open(tmp.0.join("intents")).unwrap();
        let mut p = plan();
        p.payload.channel = HandoffChannel::New {
            name: None,
            topic: "timed review".into(),
            goal: "independent obligations".into(),
        };
        let mut client = GuardedCanonical::new(&tmp.0);
        p.payload.policy = Some(HandoffPolicy {
            deadline_millis: Some(7000),
            invitation_confirmation: InvitationConfirmation::RequireAck,
            invite_deadline_millis: Some(11000),
        });
        let reference = record(&journal, p.clone());
        guarded_retry(&journal, &reference, &p, &client, true, &mut vec![]).unwrap();
        let (mode, duration, invitation_deadline, receipt) =
            handoff_policy_canonical_values(&client);
        assert_eq!(
            (mode.as_str(), duration, invitation_deadline),
            ("require_ack", 11000, 11001)
        );
        assert_eq!(receipt.deadline_at, Some(7001));
        assert_eq!(receipt.available_at, None);
        let context_at = |time| {
            crate::store::connection::StoreContext::new(
                tmp.0.join("canonical.db"),
                std::sync::Arc::new(At(time)),
            )
            .with_test_host()
        };
        let early = context_at(7001);
        let mut db = early.open_writer().unwrap();
        handoff_policy_advance_work(&early, &mut db);
        let mut cursor = crate::store::receipts::ReceiptDueCursor::default();
        for _ in 0..16 {
            crate::store::receipts::scan_due(&early, &mut db, 100, &mut cursor).unwrap();
        }
        crate::store::invitation_due::scan_invitation_due_batch(&early, &mut db, None, 100)
            .unwrap();
        let message_warning =
            crate::store::effective::effective_receipt(&db, &receipt.message_id, "recipient")
                .unwrap()
                .unwrap()
                .warning_message_id
                .unwrap();
        let invitation_warning = |db: &rusqlite::Connection| {
            db.query_row("SELECT warning_message_id FROM invitations", [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .unwrap()
        };
        assert_eq!(
            invitation_warning(&db),
            None,
            "message expiry must not expire the invitation early"
        );
        let late = context_at(11001);
        crate::store::invitation_due::scan_invitation_due_batch(&late, &mut db, None, 100).unwrap();
        let invitation_warning =
            invitation_warning(&db).expect("invitation expires independently without registration");
        assert_ne!(message_warning, invitation_warning);
        assert_eq!(db.query_row("SELECT count(*) FROM occupant_bindings WHERE seat_id='recipient' AND registered_at IS NOT NULL", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
        let cleared = |warning: &str| {
            db.query_row("SELECT clear_warning_id IS NOT NULL FROM warning_conditions WHERE open_warning_id=?1", [warning], |r|r.get::<_,bool>(0)).unwrap()
        };
        assert!(!cleared(&message_warning));
        assert!(!cleared(&invitation_warning));
        assert_eq!(
            db.query_row(
                "SELECT seat_id FROM receipt_warning_origins WHERE warning_id=?1",
                [&message_warning],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "sender"
        );
        let sender_attention = || {
            crate::store::attention::wake_seat_attention(&db, "sender")
                .unwrap()
                .attention
                .latest_warning_seq
        };
        assert!(sender_attention().is_some());
        client.base.store = crate::store::SqliteStore::new(
            context_at(11002),
            "34750f98bd59fcfc946da45aaabe933b",
            Default::default(),
        )
        .unwrap();
        let mut recipient = canonical_claim();
        recipient.seat = SeatId::new("recipient");
        recipient.target = crate::protocol::ids::HostTargetId::new("w1:p2");
        let thread = ThreadId::new(
            db.query_row("SELECT id FROM threads", [], |r| r.get::<_, String>(0))
                .unwrap(),
        );
        let accept = Command::Accept(Accept {
            thread,
            operation: OperationId::new("independent-accept"),
            claim: recipient.clone(),
        });
        let ack = Command::Ack(Ack {
            messages: vec![MessageId::new(&receipt.message_id)],
            operation: OperationId::new("independent-ack"),
            claim: recipient,
        });
        let (first, second) = if accept_first {
            (accept, ack)
        } else {
            (ack, accept)
        };
        let budget = CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        };
        client.base.canonical(first, &budget).unwrap();
        assert_eq!(cleared(&invitation_warning), accept_first);
        assert_eq!(cleared(&message_warning), !accept_first);
        assert!(
            sender_attention().is_some(),
            "the other open condition must retain sender attention"
        );
        let partial =
            crate::store::effective::effective_receipt(&db, &receipt.message_id, "recipient")
                .unwrap()
                .unwrap();
        assert_eq!(
            partial.state,
            if accept_first {
                crate::store::effective::EffectiveReceiptState::Pending
            } else {
                crate::store::effective::EffectiveReceiptState::Acknowledged
            }
        );
        client.base.canonical(second, &budget).unwrap();
        assert!(cleared(&invitation_warning));
        assert!(cleared(&message_warning));
        assert_eq!(sender_attention(), None, "final clears are passive");
        assert_eq!(
            db.query_row("SELECT count(*) FROM warning_conditions", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("SELECT state FROM invitations", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "accepted"
        );
        let final_receipt =
            crate::store::effective::effective_receipt(&db, &receipt.message_id, "recipient")
                .unwrap()
                .unwrap();
        assert_eq!(
            final_receipt.state,
            crate::store::effective::EffectiveReceiptState::Acknowledged
        );
        assert_eq!(final_receipt.deadline_at, Some(7001));
        assert_eq!(final_receipt.frozen_duration_ms, 7000);
        assert_eq!(client.counts(), (1, 1, 1));
    }
}

#[test]
fn automatic_delivery_preparation_stays_unsupported_before_any_read() {
    let client = Client::new(false);
    let mut request = request();
    request.automatic = true;
    request.thread = None;
    let error = prepare(
        &request,
        Preparation {
            claim: &fixture::claim(),
            namespace: &fixture::payload().handoff.namespace,
            topology: &crate::host::observation::HostTopology {
                spaces: vec![],
                tabs: vec![],
                panes: vec![],
            },
            client: &client,
            clock: &TestClock,
        },
    )
    .unwrap_err();
    assert!(matches!(error,RunError::Api(ref error) if error.code==ErrorCode::Unsupported));
    assert!(client.calls.lock().unwrap().is_empty());
}

#[test]
fn automatic_existing_seat_full_request_ensures_and_sends() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    assert!(request.automatic);
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(true);
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let mut bytes = vec![];
    let result = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut bytes,
    );
    assert!(
        result.is_ok(),
        "existing-seat automatic route failed: {result:?}"
    );
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.iter().filter(|c| matches!(c, Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Ensure(_)))).count(), 1);
    assert_eq!(calls.iter().filter(|c| matches!(c, Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Send(_)))).count(), 1);
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert!(*client.sent.lock().unwrap());
    assert!(!bytes.is_empty());
}

#[test]
fn automatic_existing_pane_full_request_resolves_one_seat() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--pane",
        "w1:p2",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    assert!(request.automatic);
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(true);
    let topology = crate::host::observation::HostTopology {
        spaces: vec![crate::host::observation::TopologySpace {
            id: "w1".into(),
            label: None,
        }],
        tabs: vec![crate::host::observation::TopologyTab {
            id: "w1:t1".into(),
            space: "w1".into(),
            label: None,
        }],
        panes: vec![
            crate::host::observation::TopologyPane {
                target: crate::protocol::ids::HostTargetId::new("w1:p1"),
                space: "w1".into(),
                tab: "w1:t1".into(),
                label: None,
                agent_names: vec![],
            },
            crate::host::observation::TopologyPane {
                target: crate::protocol::ids::HostTargetId::new("w1:p2"),
                space: "w1".into(),
                tab: "w1:t1".into(),
                label: None,
                agent_names: vec![],
            },
        ],
    };
    let namespace = fixture::payload().handoff.namespace;
    let mut bytes = vec![];
    let result = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut bytes,
    );
    assert!(
        result.is_ok(),
        "existing-pane automatic route failed: {result:?}"
    );
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Seats(_)))
            .count(),
        1
    );
    assert_eq!(calls.iter().filter(|c| matches!(c, Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Ensure(_)))).count(), 1);
    assert_eq!(calls.iter().filter(|c| matches!(c, Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Send(_)))).count(), 1);
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert!(*client.sent.lock().unwrap());
    assert!(!bytes.is_empty());
}

#[test]
fn automatic_existing_unsupported_refuses_before_publication_and_effects() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(false);
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let result = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut vec![],
    );
    assert!(
        matches!(result, Err(RunError::Api(ref error)) if error.code == ErrorCode::Unsupported),
        "{result:?}"
    );
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert!(journal.resolve_recovery_ref("local:1").is_err());
}

#[test]
fn automatic_existing_pane_unsupported_refuses_before_resolution_and_publication() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--pane",
        "w1:p2",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(false);
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let result = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut vec![],
    );
    assert!(
        matches!(result, Err(RunError::Api(ref e)) if e.code == ErrorCode::Unsupported),
        "{result:?}"
    );
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert!(journal.resolve_recovery_ref("local:1").is_err());
}

#[test]
fn automatic_existing_lost_ensure_then_reject_keeps_original_and_refuses_send() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(true);
    *client.lost.lock().unwrap() = Some("ensure");
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let mut first_output = vec![];
    let first = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut first_output,
    );
    assert!(
        matches!(first, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{first:?}"
    );
    assert!(
        !first_output.is_empty(),
        "lost Ensure needs explicit retry guidance"
    );
    let reference = resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    let original = load_automatic_original(&journal, &reference).unwrap();
    let frozen = automatic_original(&original).unwrap();
    let keys = frozen.delivery().payload.keys.clone();
    *client.rejected.lock().unwrap() = true;
    let mut retry_output = vec![];
    let retry = retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut retry_output,
    );
    assert!(
        matches!(retry, Err(RunError::Api(ref e)) if e.code == ErrorCode::MembershipRequired),
        "{retry:?}"
    );
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert!(!*client.sent.lock().unwrap());
    assert!(
        !retry_output.is_empty(),
        "rejected recipient needs retained recovery guidance"
    );
    let after =
        automatic_original(&load_automatic_original(&journal, &reference).unwrap()).unwrap();
    assert_eq!(after.delivery().payload.keys, keys);
    assert_eq!(after.delivery().payload.body, "literal '$HOME' work");
    *client.rejected.lock().unwrap() = false;
    retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut vec![],
    )
    .unwrap();
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert!(*client.sent.lock().unwrap());
}

#[test]
fn automatic_existing_lost_send_replays_published_message_after_reject() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(true);
    *client.lost.lock().unwrap() = Some("send");
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let first = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut vec![],
    );
    assert!(
        matches!(first, Err(RunError::Api(ref e)) if e.code == ErrorCode::UnknownOutcome),
        "{first:?}"
    );
    *client.rejected.lock().unwrap() = true;
    let reference = resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    let report = retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut vec![],
    )
    .unwrap();
    assert_eq!(report["message"]["data"], "m-pair");
    assert_eq!(*client.invitations.lock().unwrap(), 1);
    assert!(*client.sent.lock().unwrap());
}

#[test]
fn automatic_existing_completed_retry_uses_terminal_without_ensure_or_send() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(true);
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let mut first_output = vec![];
    run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut first_output,
    )
    .unwrap();
    let before = client.calls.lock().unwrap().len();
    let reference = resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    let mut second_output = vec![];
    let report = retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &namespace,
        &client,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut second_output,
    )
    .unwrap();
    assert_eq!(report["message"]["data"], "m-pair");
    assert_eq!(second_output, first_output);
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.len(), before + 1);
    assert!(
        matches!(&calls[before], Command::AutomaticHandoffDelivery(v) if matches!(v.action, crate::protocol::handoff::AutomaticDeliveryAction::Status))
    );
    assert_eq!(*client.invitations.lock().unwrap(), 1);
}

#[test]
fn automatic_existing_retry_requires_exact_namespace_path_bytes_before_client() {
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--",
        "literal '$HOME' work",
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let client = AutomaticPeerClient::new(true);
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut vec![],
    )
    .unwrap();
    let reference = resolve_automatic_recovery_ref(&journal, "local:1").unwrap();
    let mut alias = namespace.clone();
    alias.state_dir = "/state//".into();
    let before = client.calls.lock().unwrap().len();
    let mut bytes = vec![];
    let result = retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &alias,
        &client,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut bytes,
    );
    assert!(result.is_err());
    assert!(bytes.is_empty());
    assert_eq!(client.calls.lock().unwrap().len(), before);
}

struct AutomaticPeerClient {
    body: String,
    supported: bool,
    calls: Mutex<Vec<Command>>,
    thread: Mutex<Option<ThreadId>>,
    sent: Mutex<bool>,
    completed: Mutex<bool>,
    lost: Mutex<Option<&'static str>>,
    rejected: Mutex<bool>,
    invitations: Mutex<usize>,
}
impl AutomaticPeerClient {
    fn new(supported: bool) -> Self {
        Self {
            body: "literal '$HOME' work".into(),
            supported,
            calls: Mutex::new(vec![]),
            thread: Mutex::new(None),
            sent: Mutex::new(false),
            completed: Mutex::new(false),
            lost: Mutex::new(None),
            rejected: Mutex::new(false),
            invitations: Mutex::new(0),
        }
    }
}
impl LocalClient for AutomaticPeerClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::Capabilities => Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList {
                    capabilities: if self.supported {
                        vec![
                            crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1.into(),
                            crate::protocol::capabilities::ENSURE_CREATION_POLICY_V1.into(),
                            crate::protocol::capabilities::SEND_TIME_RECEIPTS_V1.into(),
                            crate::protocol::capabilities::INVITATION_CONFIRMATION_V1.into(),
                        ]
                    } else {
                        vec![]
                    },
                },
            )),
            Command::SeatInspect(q) => {
                assert_eq!(q.seat.as_str(), "recipient");
                Ok(CommandResult::SeatInspect(inspection()))
            }
            Command::Seats(q) => {
                assert_eq!(q.target.unwrap().as_str(), "w1:p2");
                Ok(CommandResult::Seats(
                    serde_json::from_value(page(serde_json::json!([inspection().summary])))
                        .unwrap(),
                ))
            }
            Command::AutomaticHandoffDelivery(request) => {
                use crate::protocol::handoff::{AutomaticDeliveryAction, HandoffResult};
                assert_eq!(request.plan().recipient.as_str(), "recipient");
                assert_eq!(request.plan().payload.body, self.body);
                let keys = &request.plan().payload.keys;
                let status = |state, thread| {
                    CommandResult::Handoff(HandoffResult {
                        compound: keys.compound.clone(),
                        state,
                        thread,
                    })
                };
                match &request.action {
                    AutomaticDeliveryAction::Prepare => {
                        Ok(CommandResult::SeatResolved(SeatId::new("recipient")))
                    }
                    AutomaticDeliveryAction::Status | AutomaticDeliveryAction::Begin { .. } => {
                        Ok(status(
                            if *self.completed.lock().unwrap() {
                                HandoffState::Completed
                            } else {
                                HandoffState::Live
                            },
                            self.thread.lock().unwrap().clone(),
                        ))
                    }
                    AutomaticDeliveryAction::Ensure(q) => {
                        assert_eq!(q.operation, keys.ensure);
                        let mut thread = self.thread.lock().unwrap();
                        if thread.is_none() {
                            *self.invitations.lock().unwrap() += 1;
                            *thread = Some(ThreadId::new("t-pair"));
                        }
                        drop(thread);
                        let mut lost = self.lost.lock().unwrap();
                        if *lost == Some("ensure") {
                            *lost = None;
                            return Err(ApiError::unknown_outcome("Ensure committed; reply lost"));
                        }
                        if let Some(policy) = &q.creation_policy {
                            use crate::protocol::results::{
                                DirectThreadEnsureOutcome, DirectThreadEnsured,
                                DirectThreadInvitation,
                            };
                            Ok(CommandResult::DirectThreadEnsured(DirectThreadEnsured {thread:ThreadId::new("t-pair"),recipient:SeatId::new("recipient"),outcome:DirectThreadEnsureOutcome::Created,
                                invitation:Some(DirectThreadInvitation {id:crate::protocol::ids::InvitationId::new("iv-pair"),episode:1,confirmation:policy.confirmation,
                                    effective_deadline_at:if policy.confirmation==crate::protocol::commands::InvitationConfirmation::RequireAck {Some(UtcMillis(11001))} else {None},inviter:Some(q.claim.seat.clone())})}))
                        } else {
                            Ok(CommandResult::ThreadCreated(ThreadId::new("t-pair")))
                        }
                    }
                    AutomaticDeliveryAction::Send(q) => {
                        assert_eq!(q.operation, keys.send);
                        assert_eq!(q.thread.as_str(), "t-pair");
                        assert_eq!(q.direct_recipient.as_ref().unwrap().as_str(), "recipient");
                        if !*self.sent.lock().unwrap() && *self.rejected.lock().unwrap() {
                            return Err(ApiError::new(
                                ErrorCode::MembershipRequired,
                                "recipient rejected; explicitly reinvite",
                            ));
                        }
                        *self.sent.lock().unwrap() = true;
                        let mut lost = self.lost.lock().unwrap();
                        if *lost == Some("send") {
                            *lost = None;
                            return Err(ApiError::unknown_outcome("Send committed; reply lost"));
                        }
                        Ok(CommandResult::MessageSent(MessageId::new("m-pair")))
                    }
                    AutomaticDeliveryAction::Complete { operation } => {
                        assert_eq!(operation, &keys.complete);
                        assert!(*self.sent.lock().unwrap());
                        *self.completed.lock().unwrap() = true;
                        let mut lost = self.lost.lock().unwrap();
                        if *lost == Some("complete") {
                            *lost = None;
                            return Err(ApiError::unknown_outcome(
                                "Complete committed; reply lost",
                            ));
                        }
                        Ok(status(
                            HandoffState::Completed,
                            Some(ThreadId::new("t-pair")),
                        ))
                    }
                }
            }
            other => panic!("automatic existing producer used forbidden command: {other:?}"),
        }
    }
}

#[test]
fn automatic_existing_max_escaped_body_and_independent_frozen_deadlines() {
    let body = "\0".repeat(65536);
    let parsed = super::super::commands::parse_argv([
        "ht",
        "handoff",
        "--existing",
        "--seat",
        "recipient",
        "--deadline",
        "7",
        "--invite-deadline",
        "11",
        "--",
        &body,
    ])
    .unwrap();
    let super::super::commands::CliAction::TopologyHandoff(request) = parsed.action else {
        panic!("wrong route")
    };
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let mut client = AutomaticPeerClient::new(true);
    client.body = body.clone();
    let topology = crate::host::observation::HostTopology {
        spaces: vec![],
        tabs: vec![],
        panes: vec![],
    };
    let namespace = fixture::payload().handoff.namespace;
    let result = run_automatic_existing(
        &request,
        Preparation {
            claim: &canonical_claim(),
            namespace: &namespace,
            topology: &topology,
            client: &client,
            clock: &TestClock,
        },
        &journal,
        super::super::actor_route::InvocationActor::Agent,
        &OutputSpec::default(),
        &mut vec![],
    );
    assert!(result.is_ok(), "maximum exact body refused: {result:?}");
    let calls = client.calls.lock().unwrap();
    let send = calls
        .iter()
        .find_map(|c| match c {
            Command::AutomaticHandoffDelivery(q) => match &q.action {
                crate::protocol::handoff::AutomaticDeliveryAction::Send(v) => Some(v),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(send.body, body);
    assert_eq!(send.deadline_millis, Some(7000));
}

// Kills: skipping the receipt negotiation for a retained policy=None original.
// Discovery is overridden; every execution call uses the existing producer fixture.
struct ReceiptCapabilityPeer<'a> {
    inner: &'a AutomaticPeerClient,
    receipts: bool,
    calls: Mutex<Vec<Command>>,
}
impl LocalClient for ReceiptCapabilityPeer<'_> {
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        if matches!(command, Command::Capabilities) {
            let mut capabilities = vec![crate::protocol::capabilities::AUTOMATIC_HANDOFF_V1.into()];
            if self.receipts {
                capabilities.push(crate::protocol::capabilities::SEND_TIME_RECEIPTS_V1.into());
            }
            return Ok(CommandResult::Capabilities(
                crate::protocol::results::CapabilityList { capabilities },
            ));
        }
        self.inner.call(command, budget)
    }
}

#[test]
fn historical_none_automatic_existing_retry_requires_receipts_before_effects_and_preserves_origin()
{
    use crate::protocol::handoff::{
        AutomaticDeliveryAction, AutomaticDeliveryPlan, AutomaticHandoffPayload, AutomaticOriginal,
    };
    use sha2::{Digest, Sha256};
    let tmp = TempRoot::new();
    let journal = Journal::open(tmp.0.join("intents")).unwrap();
    let inner = AutomaticPeerClient::new(true);
    let namespace = fixture::payload().handoff.namespace;
    let payload = AutomaticHandoffPayload::new(
        namespace.clone(),
        crate::protocol::ids::OperationId::new("retained-receipt-existing"),
        inner.body.clone(),
    )
    .unwrap();
    let plan = AutomaticDeliveryPlan::new(payload, SeatId::new("recipient")).unwrap();
    assert!(plan.payload.policy.is_none());
    let keys = plan.payload.keys.clone();
    let claim = canonical_claim();
    let reference = publish_automatic(
        &journal,
        AutomaticOriginal::Delivery(Box::new(plan.clone())),
        claim.clone(),
        &TestClock,
    )
    .unwrap();
    let original = load_automatic_original(&journal, &reference).unwrap();
    let bytes = journal.snapshot_automatic_origin(&reference).unwrap();
    let semantic = serde_json::to_vec(&original.semantic).unwrap();
    assert!(
        !String::from_utf8(semantic.clone())
            .unwrap()
            .contains("\"policy\"")
    );
    assert_eq!(
        original.header.semantic_digest,
        format!("{:x}", Sha256::digest(&semantic))
    );
    let peer = ReceiptCapabilityPeer {
        inner: &inner,
        receipts: false,
        calls: Mutex::new(vec![]),
    };
    let mut parsed =
        super::super::commands::parse_argv(["ht", "retry", &reference.recovery_ref()]).unwrap();
    parsed.output = OutputSpec::default();
    let mut output = Vec::new();
    assert!(
        !super::super::topology_runtime::try_completed_with_namespace(
            &parsed,
            &journal,
            &namespace,
            &peer,
            &TestClock,
            &mut output
        )
        .unwrap()
    );
    assert!(output.is_empty());
    let result = retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &namespace,
        &peer,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut output,
    );
    assert!(
        matches!(result, Err(RunError::Api(ref error)) if error.code == ErrorCode::Unsupported),
        "pending None original must negotiate receipts: {result:?}"
    );
    assert_eq!(
        journal.snapshot_automatic_origin(&reference).unwrap(),
        bytes
    );
    assert_eq!(
        load_automatic_original(&journal, &reference)
            .unwrap()
            .semantic,
        original.semantic
    );
    assert_eq!(*inner.invitations.lock().unwrap(), 0);
    assert!(!*inner.sent.lock().unwrap());
    assert!(!*inner.completed.lock().unwrap());
    assert!(inner.thread.lock().unwrap().is_none());
    assert!(
        peer.calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Command::Capabilities))
    );
    assert!(peer.calls.lock().unwrap().iter().all(|c| matches!(c,
        Command::Capabilities | Command::SeatInspect(_) | Command::Seats(_)) || matches!(c,
        Command::AutomaticHandoffDelivery(q) if matches!(q.action, AutomaticDeliveryAction::Status))), "refusal permits only discovery/read/status");
    let capable = ReceiptCapabilityPeer {
        inner: &inner,
        receipts: true,
        calls: Mutex::new(vec![]),
    };
    let mut first = Vec::new();
    retry_automatic_to_writer(
        &journal,
        &reference,
        super::super::actor_route::InvocationActor::Agent,
        &namespace,
        &capable,
        None,
        &TestClock,
        &OutputSpec::default(),
        &mut first,
    )
    .unwrap();
    assert_eq!(*inner.invitations.lock().unwrap(), 1);
    assert!(*inner.completed.lock().unwrap());
    for c in capable.calls.lock().unwrap().iter() {
        if let Command::AutomaticHandoffDelivery(q) = c {
            assert_eq!(q.plan(), &plan);
            assert_eq!(q.plan().payload.keys, keys);
            if let AutomaticDeliveryAction::Ensure(ensure) = &q.action {
                assert!(ensure.creation_policy.is_none());
                assert_eq!(ensure.operation, keys.ensure);
                assert_eq!(ensure.claim, claim);
            }
            if let AutomaticDeliveryAction::Send(send) = &q.action {
                assert_eq!(send.operation, keys.send);
                assert_eq!(send.body, plan.payload.body);
                assert_eq!(send.claim, claim);
            }
        }
    }
    let terminal = load_automatic_terminal(&journal, &reference)
        .unwrap()
        .unwrap();
    assert_eq!(terminal.original.as_bytes(), bytes);
    let retained =
        super::super::retry::load_original_for_actor(&journal, &reference.recovery_ref()).unwrap();
    assert_eq!(retained.semantic, original.semantic);
    assert_eq!(
        retained.header.semantic_digest,
        original.header.semantic_digest
    );
    // Actual parsed public completed classifier stays exempt on an old peer.
    let mut parsed =
        super::super::commands::parse_argv(["ht", "retry", &reference.recovery_ref()]).unwrap();
    parsed.output = OutputSpec::default();
    peer.calls.lock().unwrap().clear();
    let mut replay = Vec::new();
    assert!(
        super::super::topology_runtime::try_completed_with_namespace(
            &parsed,
            &journal,
            &namespace,
            &peer,
            &TestClock,
            &mut replay
        )
        .unwrap()
    );
    assert_eq!(replay, first);
    assert!(peer.calls.lock().unwrap().iter().all(|c| matches!(c, Command::Capabilities) || matches!(c,
        Command::AutomaticHandoffDelivery(q) if matches!(q.action, AutomaticDeliveryAction::Status))));
    assert_eq!(*inner.invitations.lock().unwrap(), 1);
}
