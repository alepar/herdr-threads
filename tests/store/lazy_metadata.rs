use herdr_threads::{
    ports::{ReadContext, StorePort},
    protocol::{
        commands::{Command, DeliveryMode, MessageDeliveryModesQuery},
        ids::MessageId,
        output::OutputSpec,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use rusqlite::{Connection, params};
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn setup(iso: &TestIsolation) -> (SqliteStore, Connection) {
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0),('foreign',0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0),('ft','foreign','foreign','foreign',0,0);
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('s','i','unresolved','native',1,0);").unwrap();
    for n in 1..=100 {
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES(?1,'i','t',?2,'ordinary',?2,?3,0,?4)",params![format!("msg-{n}"),n,if n%2==0 {"ordinary message"} else {"[lazy] alleged passive message"},if n%2==0 {"lazy"} else {"ordinary"}]).unwrap();
    }
    db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at,delivery_mode) VALUES('fm','foreign','ft',1,'ordinary',1,'foreign',0,'lazy');
        INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) VALUES('block','i','t','v1',0,0,1,2,'hash','cached narrative','s','model','prompt',0);
        INSERT INTO summary_jobs(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,created_at,lease_seat_id,lease_token,lease_until) VALUES('summary-job','i','t','v1',0,1,3,4,0,'s','lease',9000);
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-2','i','scope','op',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy');
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-2','msg-2','t','s');
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-2','msg-2','i','t',2,0,2,0,1,0);").unwrap();
    (
        SqliteStore::new(context, "i", StoreSettings::default()).unwrap(),
        db,
    )
}
fn read(
    store: &SqliteStore,
    ids: Vec<MessageId>,
    budget: &CallBudget,
) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
    store.query(
        &Command::MessageDeliveryModes(MessageDeliveryModesQuery { messages: ids }),
        &ReadContext {
            instance: "i".into(),
            output: OutputSpec::default(),
            operation_scope: None,
        },
        budget,
    )
}
#[test]
fn lazy_metadata_exact_ids_max100_instance_scope() {
    let iso = TestIsolation::new("lazy-metadata-bounds");
    let (store, _db) = setup(&iso);
    assert_eq!(
        read(&store, vec![], &budget()).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        read(&store, vec![MessageId::new("msg-1"); 101], &budget())
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let ids = (1..=100)
        .map(|n| MessageId::new(format!("msg-{n}")))
        .collect::<Vec<_>>();
    let CommandResult::MessageDeliveryModes(modes) = read(&store, ids.clone(), &budget()).unwrap()
    else {
        panic!("wrong result")
    };
    assert_eq!(
        modes.iter().map(|m| m.message.clone()).collect::<Vec<_>>(),
        ids
    );
    for id in ["fm", "missing", "m", "msg-1%"] {
        assert_eq!(
            read(
                &store,
                vec![MessageId::new("msg-1"), MessageId::new(id)],
                &budget()
            )
            .unwrap_err()
            .code,
            ErrorCode::NotFound,
            "{id}"
        );
    }
    for id in ["", "local:recovery", "bad id"] {
        let wire = serde_json::json!({"kind":"message_delivery_modes","args":{"messages":[id]}});
        assert!(serde_json::from_value::<Command>(wire).is_err(), "{id}");
    }
}
#[test]
fn lazy_metadata_canonical_modes_ignore_body_claims() {
    let iso = TestIsolation::new("lazy-metadata-canonical");
    let (store, _db) = setup(&iso);
    for _ in 0..2 {
        let CommandResult::MessageDeliveryModes(modes) = read(
            &store,
            vec![
                MessageId::new("msg-2"),
                MessageId::new("msg-1"),
                MessageId::new("msg-2"),
            ],
            &budget(),
        )
        .unwrap() else {
            panic!("wrong result")
        };
        assert_eq!(
            modes
                .iter()
                .map(|m| (m.message.as_str(), m.delivery_mode))
                .collect::<Vec<_>>(),
            [
                ("msg-2", DeliveryMode::Lazy),
                ("msg-1", DeliveryMode::Ordinary),
                ("msg-2", DeliveryMode::Lazy)
            ]
        );
    }
}
// Capture every durable table, including populated delivery, cache and lease
// rows: accidental updates, deletions and new jobs all change this snapshot.
fn snapshot(db: &Connection) -> Vec<(String, Vec<String>)> {
    let names = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut stmt = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let width = stmt.column_count();
            let mut rows = stmt
                .query_map([], |r| {
                    Ok(format!(
                        "{:?}",
                        (0..width)
                            .map(|n| r.get::<_, rusqlite::types::Value>(n))
                            .collect::<Result<Vec<_>, _>>()?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}
#[test]
fn lazy_metadata_read_changes_no_progress_or_summary_work() {
    let iso = TestIsolation::new("lazy-metadata-readonly");
    let (store, db) = setup(&iso);
    let before = snapshot(&db);
    for _ in 0..3 {
        read(
            &store,
            vec![MessageId::new("msg-2"), MessageId::new("msg-1")],
            &budget(),
        )
        .unwrap();
    }
    read(
        &store,
        vec![MessageId::new("msg-2"), MessageId::new("fm")],
        &budget(),
    )
    .unwrap_err();
    assert_eq!(snapshot(&db), before);
}
#[test]
fn lazy_metadata_obeys_read_budget_and_cancellation() {
    let iso = TestIsolation::new("lazy-metadata-budget");
    let (store, _db) = setup(&iso);
    let mut expired = budget();
    expired.deadline = MonoInstant(99);
    assert_eq!(
        read(&store, vec![MessageId::new("msg-1")], &expired)
            .unwrap_err()
            .code,
        ErrorCode::ReadBudgetExhausted
    );
    let cancelled = budget();
    cancelled.cancellation.cancel();
    assert_eq!(
        read(&store, vec![MessageId::new("msg-1")], &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
}
