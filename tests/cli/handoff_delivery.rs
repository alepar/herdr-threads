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
}
impl Client {
    fn new(joined: bool) -> Self {
        Self {
            inspection: inspection(),
            joined,
            calls: Mutex::new(vec![]),
            lost: Mutex::new(None),
            saved: Mutex::new(Default::default()),
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
                        state: HandoffState::Live,
                    },
                ));
            }
            Command::CompleteHandoff(q) => {
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
        let root = std::env::temp_dir().join(format!("ht-qhz5-{}", uuid::Uuid::new_v4()));
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
        let context = crate::store::connection::StoreContext::new(
            root.join("canonical.db"),
            std::sync::Arc::new(TestClock),
        );
        let conn = context.open_writer().unwrap();
        conn.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
            [],
        )
        .unwrap();
        for (seat, target) in [("sender", "w1:p1"), ("recipient", "w1:p2")] {
            conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,1,1,0)",rusqlite::params![seat,target]).unwrap();
            conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)",[target]).unwrap();
            conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?2,'b',1,1,'codex','session','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||?2,'inc')",rusqlite::params![seat,target]).unwrap();
        }
        drop(conn);
        let store = crate::store::SqliteStore::new(
            crate::store::connection::StoreContext::new(
                root.join("canonical.db"),
                std::sync::Arc::new(TestClock),
            ),
            "i",
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
