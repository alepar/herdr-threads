use crate::{
    ports::{CooperativePermitRequest, DurableWorkAdmission, SendPreparationProgress, StorePort},
    protocol::{
        authority::{CallerClaim, ObligationRef},
        commands::{CreateThread, PermitMutation, SendMessage},
        ids::*,
        results::CommandResult,
        service::EventAuthor,
        summary::{AuthorRole, is_priority},
    },
    store::{SqliteStore, messages::send_payload, schema},
};
use rusqlite::Connection;

use super::super::cooperative_checkin_tests::{
    OwnedFixture, budget, check_in, claim, fixture, human_claim, lifecycle,
};

fn create_thread(store: &SqliteStore, context: &CallerClaim, key: &str) -> ThreadId {
    let create = CreateThread {
        name: None,
        claim: context.clone(),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new(key),
    };
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: create.claim.clone(),
                operation: create.operation.clone(),
                obligation: ObligationRef::CheckIn(context.seat.clone()),
                payload_hash: crate::store::control::cooperative_payload_hash(
                    "create_thread",
                    &create,
                )
                .unwrap(),
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::ThreadCreated(thread) = store
        .mutate(PermitMutation::CreateThread(create), permit, &budget())
        .unwrap()
    else {
        panic!("not a thread")
    };
    thread
}

fn send(
    store: &SqliteStore,
    context: &CallerClaim,
    thread: &ThreadId,
    key: &str,
    relays_user: bool,
) -> MessageId {
    send_with_intent(store, context, thread, key, relays_user, None)
}

fn send_with_intent(
    store: &SqliteStore,
    context: &CallerClaim,
    thread: &ThreadId,
    key: &str,
    relays_user: bool,
    user_intent: Option<crate::protocol::summary::UserIntent>,
) -> MessageId {
    let send = SendMessage {
        claim: context.clone(),
        thread: thread.clone(),
        body: "model mail".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new(key),
        relays_user,
        user_intent,
    };
    loop {
        match store
            .prepare_send_step(&send, DurableWorkAdmission { max_units: 16 }, &budget())
            .unwrap()
        {
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { .. } => {}
            SendPreparationProgress::Committed(_) => panic!("already committed"),
        }
    }
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: send.claim.clone(),
                operation: send.operation.clone(),
                obligation: ObligationRef::Control(send.thread.clone()),
                payload_hash: schema::canonical_digest(&send_payload(&send)).unwrap(),
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::MessageSent(id) = store
        .mutate(PermitMutation::SendMessage(send), permit, &budget())
        .unwrap()
    else {
        panic!("not sent")
    };
    id
}

fn row(conn: &Connection, id: &MessageId) -> (Option<String>, i64, i64) {
    conn.query_row(
        "SELECT author_role,relays_user,author_role_backfilled FROM messages WHERE id=?1",
        [id.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

fn agent_thread() -> (
    OwnedFixture<SqliteStore>,
    OwnedFixture<Connection>,
    CallerClaim,
    ThreadId,
) {
    let (store, conn, _) = fixture();
    let context = check_in(&store, lifecycle(claim(), "agent"))
        .unwrap()
        .context;
    let thread = create_thread(&store, &context, "create");
    (store, conn, context, thread)
}

#[test]
fn cooperative_agent_send_records_agent() {
    let (store, conn, context, thread) = agent_thread();
    let id = send(&store, &context, &thread, "send", false);
    assert_eq!(row(&conn, &id), (Some("agent".to_owned()), 0, 0));
}

#[test]
fn operator_human_send_records_human() {
    let (store, conn, _) = fixture();
    let context = check_in(&store, lifecycle(human_claim(&claim(), 0), "human"))
        .unwrap()
        .context;
    let thread = create_thread(&store, &context, "create");
    let id = send(&store, &context, &thread, "send", false);
    assert_eq!(row(&conn, &id), (Some("human".to_owned()), 0, 0));
}

#[test]
fn relays_user_flag_is_recorded_and_priority() {
    let (store, conn, context, thread) = agent_thread();
    let relayed = send(&store, &context, &thread, "relayed", true);
    let (role, relays, _) = row(&conn, &relayed);
    assert_eq!((role.as_deref(), relays), (Some("agent"), 1));
    assert!(is_priority(
        role.as_deref().and_then(AuthorRole::from_column),
        relays == 1
    ));
    // The same body without the flag is a different operation and records 0.
    let plain = send(&store, &context, &thread, "plain", false);
    let (role, relays, _) = row(&conn, &plain);
    assert_eq!((role.as_deref(), relays), (Some("agent"), 0));
    assert!(!is_priority(
        role.as_deref().and_then(AuthorRole::from_column),
        relays == 1
    ));
    let mut request = SendMessage {
        claim: context,
        thread,
        body: "model mail".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("digest"),
        relays_user: false,
        user_intent: None,
    };
    let without = schema::canonical_digest(&send_payload(&request)).unwrap();
    request.relays_user = true;
    let with = schema::canonical_digest(&send_payload(&request)).unwrap();
    assert_ne!(without, with);
    assert_eq!(send_payload(&request)["relays_user"], true);
    request.relays_user = false;
    assert!(send_payload(&request).get("relays_user").is_none());
}

#[test]
fn system_events_follow_the_author() {
    let (_store, mut conn, context, thread) = agent_thread();
    let tx = conn.transaction().unwrap();
    fn event<'a>(thread: &'a ThreadId, key: &'a str) -> schema::EventInput<'a> {
        schema::EventInput {
            thread,
            key,
            kind: "info",
            payload_json: "{}",
            decision_at: crate::protocol::time::UtcMillis(500),
            source_message: None,
            source_invitation: None,
        }
    }
    let (native, _) = schema::append_attributed_event_once(
        &tx,
        event(&thread, "native-event"),
        EventAuthor::Native(context.seat.clone()),
    )
    .unwrap();
    let (built_in, _) = schema::append_event_once(&tx, event(&thread, "built-in-event")).unwrap();
    tx.commit().unwrap();
    assert_eq!(row(&conn, &native), (Some("agent".to_owned()), 0, 0));
    assert_eq!(row(&conn, &built_in), (None, 0, 0));
}

// Catches physical INSERT or public message/history projections dropping intent.
#[test]
fn user_intent_storage_roundtrip() {
    use crate::protocol::{
        commands::{BodyReadRequest, Command, HistoryQuery, MessageQuery},
        output::OutputSpec,
        summary::UserIntent,
    };
    for human in [false, true] {
        let (store, conn, _) = fixture();
        let c = if human {
            human_claim(&claim(), 0)
        } else {
            claim()
        };
        let context = check_in(&store, lifecycle(c, "intent-author"))
            .unwrap()
            .context;
        let thread = create_thread(&store, &context, "create");
        for (n, intent) in [
            None,
            Some(UserIntent::Query),
            Some(UserIntent::Request),
            Some(UserIntent::Rule),
        ]
        .into_iter()
        .enumerate()
        {
            let id = send_with_intent(
                &store,
                &context,
                &thread,
                &format!("send-{n}"),
                !human,
                intent,
            );
            assert_eq!(
                conn.query_row(
                    "SELECT user_intent FROM messages WHERE id=?1",
                    [id.as_str()],
                    |r| r.get::<_, Option<String>>(0)
                )
                .unwrap()
                .as_deref(),
                intent.map(UserIntent::as_str)
            );
            let CommandResult::Message(details) = store
                .query(
                    &Command::Message(MessageQuery {
                        message: id.clone(),
                        body: BodyReadRequest {
                            cursor: None,
                            offset: None,
                            max_bytes: 16384,
                        },
                    }),
                    &crate::ports::ReadContext {
                        instance: "i".into(),
                        output: OutputSpec::default(),
                        operation_scope: None,
                    },
                    &budget(),
                )
                .unwrap()
            else {
                panic!("not a message")
            };
            assert_eq!(details.summary.user_intent, intent);
            let mut json = serde_json::to_value(&details.summary).unwrap();
            json.as_object_mut().unwrap().remove("user_intent");
            assert_eq!(
                serde_json::from_value::<crate::protocol::results::MessageSummary>(json)
                    .unwrap()
                    .user_intent,
                None
            );
        }
        let CommandResult::History(history) = store
            .query(
                &Command::History(HistoryQuery {
                    thread: thread.clone(),
                    page: Default::default(),
                    initial: None,
                    full_bodies: false,
                }),
                &crate::ports::ReadContext {
                    instance: "i".into(),
                    output: OutputSpec::default(),
                    operation_scope: None,
                },
                &budget(),
            )
            .unwrap()
        else {
            panic!("not history")
        };
        assert_eq!(
            history
                .items
                .iter()
                .filter(|m| m.kind == crate::protocol::results::MessageKind::Ordinary)
                .map(|m| m.user_intent)
                .collect::<Vec<_>>(),
            [
                Some(UserIntent::Rule),
                Some(UserIntent::Request),
                Some(UserIntent::Query),
                None
            ]
        );
        conn.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,author_role,user_intent) VALUES ('bad-intent','i',?1,100,'ordinary',?2,'bad',0,100,'human','instruction')",rusqlite::params![thread.as_str(),context.seat.as_str()]).unwrap();
        assert_eq!(
            store
                .query(
                    &Command::Message(MessageQuery {
                        message: MessageId::new("bad-intent"),
                        body: BodyReadRequest {
                            cursor: None,
                            offset: None,
                            max_bytes: 16384
                        }
                    }),
                    &crate::ports::ReadContext {
                        instance: "i".into(),
                        output: OutputSpec::default(),
                        operation_scope: None
                    },
                    &budget()
                )
                .unwrap_err()
                .code,
            crate::protocol::results::ErrorCode::StoreCorrupt
        );
    }
}

// Catches the daemon using caller hints rather than the live binding for intent.
#[test]
fn user_intent_send_canonical_eligibility() {
    use crate::protocol::summary::UserIntent;
    for (human, relay) in [(true, false), (true, true), (false, true), (false, false)] {
        let (store, conn, _) = fixture();
        let initial = if human {
            human_claim(&claim(), 0)
        } else {
            claim()
        };
        let context = check_in(&store, lifecycle(initial, "eligible-intent"))
            .unwrap()
            .context;
        let thread = create_thread(&store, &context, "create");
        for (n, intent) in [
            None,
            Some(UserIntent::Query),
            Some(UserIntent::Request),
            Some(UserIntent::Rule),
        ]
        .into_iter()
        .enumerate()
        {
            // Unrelayed agents retain unclassified sending; refusal cases are tested at publication.
            if !human && !relay && intent.is_some() {
                continue;
            }
            let id = send_with_intent(
                &store,
                &context,
                &thread,
                &format!("intent-{n}"),
                relay,
                intent,
            );
            assert_eq!(
                row(&conn, &id),
                (
                    Some(if human { "human" } else { "agent" }.into()),
                    i64::from(relay),
                    0
                )
            );
            let recorded: Option<String> = conn
                .query_row(
                    "SELECT user_intent FROM messages WHERE id=?1",
                    [id.as_str()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(recorded.as_deref(), intent.map(UserIntent::as_str));
        }
    }
}
