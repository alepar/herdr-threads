//! Serializer/SQLite characterizations and binding-scoped presentation controls.
//! The migration probes exercise individual statements, not the complete
//! populated-store production migration runner.

use std::collections::HashMap;

use herdr_threads::protocol::ids::MessageId;
use rusqlite::{Connection, StatementStatus};

#[test]
fn ordinary_partial_worst_escaped_witness_preserves_exact_bytes() {
    // Every permitted ASCII-graphic ID byte needs at most two JSON bytes;
    // every raw UTF-8 byte needs at most six. Quotes/backslashes and NUL attain
    // those maxima. A one-entry JSON object adds seven structural bytes.
    let id = "\"\\".repeat(64);
    let parsed = MessageId::parse(id.clone()).expect("128-byte graphic ID");
    assert_eq!(parsed.as_str(), id);
    let body = "\0".repeat(8191);
    let partial = HashMap::from([(id.clone(), body.clone())]);
    let json = serde_json::to_string(&partial).expect("serialize witness");
    assert_eq!(json.len(), 49_409);
    assert!(json.len() <= 49_415);
    let restored: HashMap<String, String> = serde_json::from_str(&json).expect("round trip");
    assert_eq!(restored.len(), 1);
    assert_eq!(restored.get(&id), Some(&body));
    assert_eq!(restored[&id].as_bytes(), vec![0; 8191]);

    // The serialized cap alone is insufficient to enforce raw length <8192:
    // the first forbidden raw length still fits exactly. D's canonical guard
    // tests must independently cover that semantic rejection.
    let at_raw_limit = HashMap::from([(id, "\0".repeat(8192))]);
    assert_eq!(
        serde_json::to_string(&at_raw_limit)
            .expect("serialize raw-limit witness")
            .len(),
        49_415
    );
}

fn populated(rows: i64) -> rusqlite::Result<Connection> {
    let db = Connection::open_in_memory()?;
    db.execute_batch("CREATE TABLE invitations(id INTEGER PRIMARY KEY, topic TEXT NOT NULL)")?;
    db.execute(
        "WITH RECURSIVE n(id) AS (SELECT 1 WHERE ?1>0 UNION ALL SELECT id+1 FROM n WHERE id<?1)
         INSERT INTO invitations(id,topic) SELECT id,'retained topic' FROM n",
        [rows],
    )?;
    Ok(db)
}

fn alter_steps(db: &Connection, sql: &str) -> rusqlite::Result<i32> {
    let mut statement = db.prepare(sql)?;
    statement.execute([])?;
    Ok(statement.get_status(StatementStatus::VmStep))
}

fn quick_check_steps(db: &Connection) -> rusqlite::Result<i32> {
    let mut statement = db.prepare("PRAGMA quick_check(invitations)")?;
    let results = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(results, ["ok"]);
    Ok(statement.get_status(StatementStatus::VmStep))
}

#[test]
fn linked_sqlite_default_column_alter_has_row_independent_vm_work() -> rusqlite::Result<()> {
    let mut default_steps = Vec::new();
    let mut checked_outer_steps = Vec::new();
    let mut direct_check_steps = Vec::new();
    for count in [0, 64, 4096] {
        let db = populated(count)?;
        let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
        let steps = alter_steps(
            &db,
            "ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0",
        )?;
        let (retained, defaults): (i64, i64) = db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(epoch=0 AND topic='retained topic'),0) FROM invitations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!((retained, defaults), (count, count));
        assert!(steps > 0, "VM counter must be active");
        default_steps.push(steps);

        let checked = populated(count)?;
        let outer_steps = alter_steps(
            &checked,
            "ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0 CHECK(epoch>=0)",
        )?;
        // ALTER's CHECK validation runs a nested pragma_quick_check VDBE.
        // Its outer statement counter is NOT a measurement of total nested
        // work. Direct quick_check below characterizes scan work separately.
        let scan_steps = quick_check_steps(&checked)?;
        checked_outer_steps.push(outer_steps);
        direct_check_steps.push(scan_steps);
        eprintln!(
            "sqlite_version={version} rows={count} default_alter_vm_steps={steps} checked_alter_outer_vm_steps={outer_steps} direct_quick_check_vm_steps={scan_steps} (outer CHECK counter excludes nested-work guarantee)"
        );
    }
    assert_eq!(default_steps[0], default_steps[1]);
    assert_eq!(default_steps[1], default_steps[2]);
    assert!(checked_outer_steps.iter().all(|steps| *steps > 0));
    assert!(direct_check_steps[1] > direct_check_steps[0]);
    assert!(direct_check_steps[2] > direct_check_steps[1]);
    Ok(())
}

#[test]
fn linked_sqlite_checked_column_validates_existing_rows_and_rolls_back() -> rusqlite::Result<()> {
    let db = populated(64)?;
    // Default zero is valid for the new column; only the last retained row
    // violates the cross-column CHECK. Success would silently skip old rows.
    let error = db
        .execute_batch(
            "ALTER TABLE invitations ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0 CHECK(epoch>=0 AND id<64)",
        )
        .expect_err("the existing last row must fail validation");
    assert!(
        error.to_string().contains("CHECK constraint failed"),
        "expected existing-row CHECK refusal, got {error}"
    );
    let version: String = db.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    eprintln!("sqlite_version={version} checked_alter_existing_row_refusal={error}");
    let epoch_columns: i64 = db.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('invitations') WHERE name='epoch'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(epoch_columns, 0, "failed ALTER must not leave the column");
    let retained: i64 = db.query_row(
        "SELECT COUNT(*) FROM invitations WHERE topic='retained topic'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(retained, 64);
    Ok(())
}

// These store-boundary tests deliberately use two connected sessions with the
// same seat generation. A seat-keyed presentation slot, latest-binding notice
// frontier, or receipt-only recovery eligibility breaks their assertions.
mod binding_presentation {
    use herdr_threads::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::{PresentationInput, StorePort},
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            delivery::{
                DeliveryPageV2, DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationItemV2,
                PresentationRenderRequest,
            },
            ids::{BindingId, ExecutionId, HostTargetId, NativeSessionId, SeatId},
            time::{CallBudget, Clock, MonoInstant, UtcMillis},
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    use rusqlite::Connection;
    use std::{path::PathBuf, sync::Arc};
    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1000)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(1)
        }
    }
    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(100_000),
            cancellation: Default::default(),
        }
    }
    struct Fixture {
        store: SqliteStore,
        db: Connection,
        directory: PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
    impl Fixture {
        fn new() -> Self {
            Self::with_clock(Arc::new(FixedClock))
        }
        fn with_clock(clock: Arc<dyn Clock>) -> Self {
            use std::os::unix::fs::DirBuilderExt;
            let directory =
                std::env::temp_dir().join(format!("bound-presentation-{}", uuid::Uuid::new_v4()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .unwrap();
            let context = StoreContext::new(directory.join("store.db"), clock).with_test_host();
            let db = context.open_writer().unwrap();
            db.execute_batch("UPDATE herdr_hosts SET host_boot='boot',host_epoch=1; UPDATE local_node SET decision_seq=100;
                INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','p',1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
                INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','boot',1,0,0,'fresh','term','inc','coherent_enumeration',1);
                INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,1);
                INSERT INTO memberships(thread_id,seat_id,state,voluntary_state) VALUES('t','s','joined','joined');
                INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s',1,1);").unwrap();
            for name in ["a", "b"] {
                db.execute("INSERT INTO occupant_bindings(binding_id,seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES(?1,'s',1,0,'p','term','inc','boot',1,'claude',?1,?1,'cooperative_env_session',0,0)", [name]).unwrap();
                db.execute(
                    "INSERT INTO binding_cursors(binding_id,thread_id,seq) VALUES(?1,'t',0)",
                    [name],
                )
                .unwrap();
            }
            let store = SqliteStore::new(
                context,
                "34750f98bd59fcfc946da45aaabe933b",
                StoreSettings::default(),
            )
            .unwrap();
            Self {
                store,
                db,
                directory,
            }
        }
        fn claim(&self, name: &str) -> CallerClaim {
            CallerClaim {
                host: herdr_threads::test_support::test_host_locator(),
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                seat: SeatId::new("s"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Claude,
                native_session: NativeSessionId::new(name),
                execution: ExecutionId::new(name),
                target: HostTargetId::new("p"),
                agent_id: None,
                liveness: None,
            }
        }
        fn input(&self, name: &str) -> PresentationInput {
            self.input_for_claim(name, self.claim(name))
        }
        fn input_for_claim(&self, name: &str, claim: CallerClaim) -> PresentationInput {
            let frozen_render = self
                .store
                .presentation_render_selection(&claim, PresentationConsumer::Watch, &budget())
                .unwrap()
                .map(|identity| FrozenPresentationRender {
                    identity,
                    codec: None,
                });
            PresentationInput {
                binding_id: BindingId::new(name),
                owner_replay: None,
                ordinary_due_snapshot: None,
                frozen_render,
                render_profile: PresentationRenderProfile::WatchJsonLinesV1,
                pull: DeliveryPullV2 {
                    claim,
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
            }
        }
        fn pull(&self, name: &str) -> DeliveryPageV2 {
            match self
                .store
                .presentation_pull(self.input(name), &budget())
                .unwrap()
                .reply
            {
                DeliveryPullReplyV2::Page(page) => page,
                other => panic!("expected bound page: {other:?}"),
            }
        }
        fn notice(&self) {
            self.db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES('notice','34750f98bd59fcfc946da45aaabe933b','t',50,'warn','{}',0,50,1);
                INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES('s','notice','t',50,1);").unwrap();
        }
        fn confirm(&self, name: &str, page: &DeliveryPageV2) -> DeliveryPresentationConfirmationV2 {
            self.store
                .presentation_confirm(
                    &DeliveryPresentationConfirmV2 {
                        claim: self.claim(name),
                        consumer: PresentationConsumer::Watch,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        token: page
                            .presentation_token
                            .clone()
                            .expect("notice outbox token"),
                        emitted_controls: page.emitted_controls.clone(),
                    },
                    &budget(),
                )
                .unwrap()
                .status
        }
        fn ordinary(&self) {
            self.ordinary_body("unread by b");
        }
        fn ordinary_body(&self, body: &str) {
            self.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('m','34750f98bd59fcfc946da45aaabe933b','t',1,'ordinary',?1,0,3)", [body]).unwrap();
            self.db.execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('prep','34750f98bd59fcfc946da45aaabe933b','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
                INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('prep','t','s',1,300000,1);
                INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('34750f98bd59fcfc946da45aaabe933b','prep','m','t',3,0,1,0,1,0);
                INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_binding_id,ack_actor_seat_id,ack_generation,ack_observation) VALUES('m','s','acked',7,'a','s',1,json_object('harness','claude','session','a','execution','a','host_boot','boot','binding_generation',1,'target_generation',0,'provenance','cooperative_env_session','observed_at',0,'decided_at',7));
                UPDATE binding_cursors SET seq=1 WHERE binding_id='a';").unwrap();
        }
    }
    #[test]
    fn same_generation_binding_outboxes_and_notice_frontiers_are_independent() {
        let f = Fixture::new();
        f.notice();
        let a = f.pull("a");
        assert!(
            a.items
                .iter()
                .any(|v| matches!(v, PresentationItemV2::Notice { .. }))
        );
        let b = f.pull("b");
        assert!(
            b.items
                .iter()
                .any(|v| matches!(v, PresentationItemV2::Notice { .. })),
            "A's frontier must not hide B's notice"
        );
        assert_ne!(a.presentation_token, b.presentation_token);
        assert_eq!(
            f.confirm("b", &a),
            DeliveryPresentationConfirmationV2::NotCurrentPage
        );
        assert_eq!(
            f.pull("a").presentation_token,
            a.presentation_token,
            "B cannot reset A's recovery outbox"
        );
        assert_eq!(
            f.confirm("a", &a),
            DeliveryPresentationConfirmationV2::Confirmed
        );
        assert_eq!(
            f.pull("b").presentation_token,
            b.presentation_token,
            "A's confirmation cannot retire B's descriptors"
        );
        let cursors: i64 =
            f.db.query_row("SELECT sum(seq) FROM binding_cursors", [], |r| r.get(0))
                .unwrap();
        assert_eq!(
            cursors, 0,
            "physical presentation confirmation moves no message read cursor"
        );
    }
    #[test]
    fn binding_input_cannot_select_a_sibling_or_canonical_subagent() {
        let f = Fixture::new();
        f.notice();
        let mut wrong = f.input("a");
        wrong.binding_id = BindingId::new("b");
        assert!(matches!(
            f.store.presentation_pull(wrong, &budget()).unwrap().reply,
            DeliveryPullReplyV2::Refused { .. }
        ));
        f.db.execute(
            "UPDATE occupant_bindings SET role='subagent',parent_binding='a' WHERE binding_id='b'",
            [],
        )
        .unwrap();
        assert!(matches!(
            f.store
                .presentation_pull(f.input("b"), &budget())
                .unwrap()
                .reply,
            DeliveryPullReplyV2::Refused { .. }
        ));
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM invitation_presentation_consumers",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn sibling_settled_receipt_remains_in_bound_normal_and_exact_recovery() {
        use herdr_threads::{
            delivery::ledger::{Offer, OfferedLedger},
            ports::{WatchReplayScope, WatchReplaySnapshot},
            protocol::ids::MessageId,
        };
        let f = Fixture::new();
        f.ordinary();
        let before: (String, i64, String) =
            f.db.query_row(
                "SELECT state,acked_at,ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let page = f.pull("b");
        assert!(page.items.iter().any(|v| matches!(v, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && !content.ack_required)), "B's bound normal stream must retain A-settled unread mail");
        let mut ledger = OfferedLedger::new();
        let entry = ledger
            .binding_mut(&SeatId::new("s"), &BindingId::new("b"), 1)
            .unwrap();
        let stamp = Offer {
            first_at: UtcMillis(10),
            last_at: UtcMillis(20),
            count: 4,
            truncated: false,
        };
        entry.offers.insert(MessageId::new("m"), stamp);
        let snapshot = entry.due_snapshot(PresentationConsumer::Watch);
        let mut input = f.input("b");
        input.known_messages = vec![MessageId::new("m")];
        input.blocked_messages = input.known_messages.clone();
        input.new_message_slots = 255;
        input.owner_replay = Some(WatchReplaySnapshot {
            scope: WatchReplayScope {
                seat: SeatId::new("s"),
                binding_id: BindingId::new("b"),
                generation: 1,
                native_session: "b".into(),
                execution: ExecutionId::new("b"),
                lease: "0123456789abcdef0123456789abcdef".into(),
                incarnation: snapshot.incarnation,
            },
            remaining: snapshot.stamps.clone(),
            active: true,
        });
        input.ordinary_due_snapshot = Some(snapshot);
        let recovered = f.store.presentation_pull(input, &budget()).unwrap();
        assert!(
            recovered.reclaimed_messages.is_empty(),
            "another binding's ACK is not B's physical completion"
        );
        assert_eq!(
            recovered.owner_replay.emitted.get(&MessageId::new("m")),
            Some(&stamp)
        );
        assert_eq!(
            f.db.query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id='b'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            f.db.query_row(
                "SELECT state,acked_at,ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?
                ))
            )
            .unwrap(),
            before
        );
    }
    #[test]
    fn carried_message_rechecks_own_unread_and_sibling_ack_without_touching_receipt() {
        let f = Fixture::new();
        f.ordinary();
        f.db.execute("UPDATE receipt_state SET state='pending',acked_at=NULL,ack_binding_id=NULL,ack_actor_seat_id=NULL,ack_generation=NULL,ack_observation=NULL WHERE message_id='m'", []).unwrap();
        let first = f.pull("b");
        let message = first
            .items
            .iter()
            .find_map(|item| match item {
                PresentationItemV2::Message { id, lazy, content } => {
                    Some(herdr_threads::protocol::delivery::PresentationMessage {
                        id: id.clone(),
                        lazy: *lazy,
                        content: content.clone(),
                    })
                }
                _ => None,
            })
            .expect("ordinary output to retain as a carry witness");
        assert!(message.content.ack_required);
        let carry =
            serde_json::json!({"prefix":[],"items":[{"content":message,"marker_lazy":false}]})
                .to_string();
        f.db.execute("UPDATE receipt_state SET state='acked',acked_at=7,ack_binding_id='a',ack_actor_seat_id='s',ack_generation=1,ack_observation=json_object('harness','claude','session','a','execution','a','host_boot','boot','binding_generation',1,'target_generation',0,'provenance','cooperative_env_session','observed_at',0,'decided_at',7) WHERE message_id='m'", []).unwrap();
        let seed_carry = || {
            f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1,ordinary_done=1,rotation=0 WHERE binding_id='b' AND consumer='watch'", [&carry]).unwrap()
        };
        seed_carry();
        let output = f.store.presentation_pull(f.input("b"), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(page) = output.reply else {
            panic!("B unread carry must remain eligible")
        };
        assert!(page.items.iter().any(|item| matches!(item, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && !content.ack_required)));
        assert!(output.work.ordinary_carry_key_probes > 0);
        assert!(output.work.ordinary_carry_key_probes <= 579);
        assert!(output.work.ordinary_carry_hydrated <= 32);
        f.db.execute("UPDATE binding_cursors SET seq=1 WHERE binding_id='b'", [])
            .unwrap();
        seed_carry();
        let output = f.store.presentation_pull(f.input("b"), &budget()).unwrap();
        if let DeliveryPullReplyV2::Page(page) = output.reply {
            assert!(
                !page
                    .items
                    .iter()
                    .any(|item| matches!(item, PresentationItemV2::Message { .. })),
                "B's own completed cursor retires carry"
            );
        }
        assert_eq!(
            f.db.query_row(
                "SELECT ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "a"
        );
    }
    #[test]
    fn carry_quantum_accounts_all_thirty_two_keys_and_refuses_a_thirty_third() {
        let f = Fixture::new();
        f.ordinary();
        let first = f.pull("b");
        let retained = first
            .items
            .iter()
            .find_map(|item| match item {
                PresentationItemV2::Message { id, lazy, content } => {
                    Some(herdr_threads::protocol::delivery::PresentationMessage {
                        id: id.clone(),
                        lazy: *lazy,
                        content: content.clone(),
                    })
                }
                _ => None,
            })
            .unwrap();
        let mut items = vec![serde_json::json!({"content":retained,"marker_lazy":false})];
        for n in 2..=32 {
            let id = format!("m{n}");
            let prep = format!("prep{n}");
            f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary','unread by b',0,?3)", rusqlite::params![id,n,n+3]).unwrap();
            f.db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT ?1,instance_id,operation_scope,?1,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status FROM send_preparations WHERE id='prep'", [&prep]).unwrap();
            f.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES(?1,'t','s',1,300000,1)", [&prep]).unwrap();
            f.db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('34750f98bd59fcfc946da45aaabe933b',?1,?2,'t',?3,0,?4,0,1,0)", rusqlite::params![prep,id,n+3,n]).unwrap();
            f.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_binding_id,ack_actor_seat_id,ack_generation,ack_observation) VALUES(?1,'s','acked',7,'a','s',1,json_object('harness','claude','session','a','execution','a','host_boot','boot','binding_generation',1,'target_generation',0,'provenance','cooperative_env_session','observed_at',0,'decided_at',7))", [&id]).unwrap();
            let mut item = items[0].clone();
            item["content"]["id"] = serde_json::json!(id);
            items.push(item);
        }
        f.db.execute("UPDATE binding_cursors SET seq=32 WHERE binding_id='a'", [])
            .unwrap();
        let carry = serde_json::json!({"prefix":[],"items":items}).to_string();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1,ordinary_done=1,rotation=0 WHERE binding_id='b' AND consumer='watch'", [&carry]).unwrap();
        let output = f.store.presentation_pull(f.input("b"), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(page) = output.reply else {
            panic!("32 canonical unread carry rows")
        };
        assert_eq!(
            page.items
                .iter()
                .filter(|item| matches!(item, PresentationItemV2::Message { .. }))
                .count(),
            32
        );
        assert_eq!(
            output.work.ordinary, 0,
            "carry must not add a fresh ordinary walk"
        );
        assert_eq!(
            output.work.ordinary_due_key_probes, 0,
            "due/replay accounting is separate"
        );
        assert_eq!(output.work.ordinary_carry_hydrated, 32);
        assert!(output.work.ordinary_carry_seeks <= 32);
        assert!(
            output.work.ordinary_carry_key_probes > 32
                && output.work.ordinary_carry_key_probes <= 579
        );
        items.push(items[0].clone());
        let invalid = serde_json::json!({"prefix":[],"items":items}).to_string();
        assert!(
            invalid.len() <= 155731,
            "isolate the carry item-count bound"
        );
        let snapshot = || {
            [
                ("invitation_presentation_consumers", "binding_id,consumer"),
                ("presentation_warning_proofs", "binding_id,consumer"),
                ("presentation_warning_fence", "singleton"),
                ("binding_cursors", "binding_id,thread_id"),
                ("receipt_state", "message_id,seat_id"),
            ]
            .map(|(table, order)| {
                let mut statement =
                    f.db.prepare(&format!("SELECT * FROM {table} ORDER BY {order}"))
                        .unwrap();
                let width = statement.column_count();
                statement
                    .query_map([], |row| {
                        (0..width)
                            .map(|n| row.get::<_, rusqlite::types::Value>(n))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap()
            })
        };
        let before = snapshot();
        let error = f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1 WHERE binding_id='b' AND consumer='watch'", [&invalid]).unwrap_err();
        assert!(
            matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_CHECK),
            "canonical SQL must reject the 33rd carry item"
        );
        assert_eq!(snapshot(), before, "rejected SQL changes no state");

        // Corrupt only this isolated fixture to exercise the independent reader
        // guard. Restore canonical CHECKs before any production call.
        f.db.pragma_update(None, "ignore_check_constraints", "ON")
            .unwrap();
        let seeded = f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_pending=?1 WHERE binding_id='b' AND consumer='watch'", [&invalid]);
        f.db.pragma_update(None, "ignore_check_constraints", "OFF")
            .unwrap();
        seeded.unwrap();
        let corrupt_before = snapshot();
        let error = f
            .store
            .presentation_pull(f.input("b"), &budget())
            .unwrap_err();
        assert_eq!(
            error.code,
            herdr_threads::protocol::results::ErrorCode::StoreCorrupt,
            "33rd carry item must refuse before unbounded work"
        );
        assert_eq!(f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get::<_,String>(0)).unwrap(), invalid, "refused pull rolls back its canonical state changes");
        assert_eq!(
            snapshot(),
            corrupt_before,
            "refused pull rolls back consumer, proof, fence, cursor and receipt state"
        );
    }
    #[test]
    fn unread_partial_presentation_resumes_after_same_binding_full_body_read() {
        use herdr_threads::{
            ports::ReadContext,
            protocol::{
                commands::{AdvanceCursors, BodyReadRequest, Command, MessageQuery},
                ids::{MessageId, OperationId},
                pagination::{InboxBatchV2CursorState, InboxBatchV2Source},
                results::{CommandResult, MessageContent},
            },
        };
        let f = Fixture::new();
        let body = "z".repeat(65_536);
        f.ordinary_body(&body);
        // The public post-display mutation checks canonical A2 mapping and a
        // canonical execution UUID, unlike the smaller presentation-only fixture.
        let execution = "11111111-1111-4111-8111-111111111111";
        f.db.execute(
            "UPDATE occupant_bindings SET execution_id=?1 WHERE binding_id='b'",
            [execution],
        )
        .unwrap();
        let mut claim = f.claim("b");
        claim.execution = ExecutionId::new(execution);
        let input = || f.input_for_claim("b", claim.clone());
        // A real second published recipient follows the long unread message.
        f.db.execute_batch("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('later','34750f98bd59fcfc946da45aaabe933b','t',2,'ordinary','later mail',0,4);
            INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) SELECT 'later-prep',instance_id,operation_scope,'later-op',digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status FROM send_preparations WHERE id='prep';
            INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('later-prep','t','s',1,300000,1);
            INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('34750f98bd59fcfc946da45aaabe933b','later-prep','later','t',4,0,2,0,1,0);
            INSERT INTO receipt_state(message_id,seat_id,state,acked_at,ack_binding_id,ack_actor_seat_id,ack_generation,ack_observation) VALUES('later','s','acked',7,'a','s',1,json_object('harness','claude','session','a','execution','a','host_boot','boot','binding_generation',1,'target_generation',0,'provenance','cooperative_env_session','observed_at',0,'decided_at',7));").unwrap();
        let first = f.store.presentation_pull(input(), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(first) = first.reply else {
            panic!("initial unread presentation must produce its truncated body")
        };
        assert!(first.items.iter().any(|item| matches!(item, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && content.truncated && content.body_len==65_536)));
        assert!(!first.items.iter().any(
            |item| matches!(item, PresentationItemV2::Message { id, .. } if id.as_str()=="later")
        ));
        let (cursor, skip, done): (String, Option<String>, i64) = f.db.query_row("SELECT ordinary_cursor,ordinary_skip_body,ordinary_done FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        let persisted = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &SeatId::new("s"),
        )
        .unwrap();
        assert_eq!(persisted.source, InboxBatchV2Source::Unread);
        assert_eq!(persisted.body.as_ref().unwrap().message.as_str(), "m");
        assert_eq!(skip.as_deref(), Some("m"));
        assert_eq!(done, 0, "the real source persisted an unfinished body walk");

        // Reproduce `body m` followed by its actual post-display cursor command.
        // No test SQL changes any cursor after the fixture was established.
        let read = ReadContext {
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            output: Default::default(),
            operation_scope: None,
        };
        let mut body_cursor = None;
        let mut full_body = String::new();
        let mut complete = false;
        for _ in 0..4 {
            let result = f
                .store
                .query(
                    &Command::Message(MessageQuery {
                        message: MessageId::new("m"),
                        body: BodyReadRequest {
                            cursor: body_cursor,
                            offset: None,
                            max_bytes: 65_536,
                        },
                    }),
                    &read,
                    &budget(),
                )
                .unwrap();
            let CommandResult::Message(message) = result else {
                panic!("body read result")
            };
            let MessageContent::Ordinary {
                body_data,
                body_complete,
                body_next_cursor,
                ..
            } = message.content
            else {
                panic!("ordinary full body")
            };
            full_body.push_str(&body_data);
            if body_complete {
                complete = true;
                break;
            }
            body_cursor = Some(body_next_cursor.expect("unfinished body has continuation"));
        }
        assert!(complete);
        assert_eq!(full_body, body);
        let advanced = f
            .store
            .advance_cursors(
                &AdvanceCursors {
                    in_turn: false,
                    invitations: vec![],
                    claim: claim.clone(),
                    operation: OperationId::new("body-read-b"),
                    displayed: vec![MessageId::new("m")],
                },
                &budget(),
            )
            .unwrap();
        assert!(
            matches!(advanced, CommandResult::CursorsAdvanced(ids) if ids==vec![MessageId::new("m")])
        );
        let positions = || {
            f.db.query_row("SELECT (SELECT seq FROM binding_cursors WHERE binding_id='a' AND thread_id='t'),(SELECT seq FROM binding_cursors WHERE binding_id='b' AND thread_id='t')", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap()
        };
        assert_eq!(
            positions(),
            (1, 1),
            "only B catches up to A's existing cursor"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT ack_binding_id FROM receipt_state WHERE message_id='m'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "a"
        );
        assert_eq!(f.db.query_row("SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get::<_,String>(0)).unwrap(), cursor, "full-body read must not secretly rewrite presentation progress");

        let resumed = f.store.presentation_pull(input(), &budget()).expect(
            "a legitimate same-binding full read must not wedge private presentation continuation",
        );
        assert_eq!(resumed.work.ordinary_continuation_key_probes, 15);
        assert!(resumed.work.ordinary <= 100);
        assert_eq!(resumed.work.ordinary_due_key_probes, 0);
        assert_eq!(resumed.work.ordinary_carry_key_probes, 0);
        let DeliveryPullReplyV2::Page(resumed) = resumed.reply else {
            panic!("presentation must reach later unread mail")
        };
        assert!(resumed.items.iter().any(
            |item| matches!(item, PresentationItemV2::Message { id, .. } if id.as_str()=="later")
        ));
        assert!(!resumed.items.iter().any(
            |item| matches!(item, PresentationItemV2::Message { id, .. } if id.as_str()=="m")
        ));
        assert_eq!(
            positions(),
            (1, 1),
            "presentation does not advance either binding cursor"
        );
    }
    fn continued_body_fixture() -> (Fixture, CallerClaim, String) {
        continued_body_fixture_with_clock(Arc::new(FixedClock), false)
    }
    fn continued_body_fixture_with_clock(
        clock: Arc<dyn Clock>,
        sibling_unread: bool,
    ) -> (Fixture, CallerClaim, String) {
        use herdr_threads::protocol::pagination::{InboxBatchV2CursorState, InboxBatchV2Source};
        let f = Fixture::with_clock(clock);
        f.ordinary_body(&"z".repeat(65_536));
        if sibling_unread {
            // Initial fixture state, before any presentation/read: A also has
            // unread content, so its later public advance is a real transition.
            f.db.execute("UPDATE binding_cursors SET seq=0 WHERE binding_id='a'", [])
                .unwrap();
        }
        let mut claim = f.claim("b");
        claim.execution = ExecutionId::new("11111111-1111-4111-8111-111111111111");
        f.db.execute(
            "UPDATE occupant_bindings SET execution_id=?1 WHERE binding_id='b'",
            [claim.execution.as_str()],
        )
        .unwrap();
        let initial = f
            .store
            .presentation_pull(f.input_for_claim("b", claim.clone()), &budget())
            .unwrap();
        assert!(initial.work.ordinary > 0);
        assert_eq!(initial.work.ordinary_continuation_key_probes, 0);
        let cursor: String = f.db.query_row("SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get(0)).unwrap();
        let state = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &claim.seat,
        )
        .unwrap();
        assert_eq!(state.source, InboxBatchV2Source::Unread);
        assert_eq!(state.body.as_ref().unwrap().message.as_str(), "m");
        (f, claim, cursor)
    }
    fn read_complete_body_and_advance(f: &Fixture, claim: &CallerClaim, operation: &str) {
        use herdr_threads::{
            ports::ReadContext,
            protocol::{
                commands::{AdvanceCursors, BodyReadRequest, Command, MessageQuery},
                ids::{MessageId, OperationId},
                results::{CommandResult, MessageContent},
            },
        };
        let read = ReadContext {
            instance: "34750f98bd59fcfc946da45aaabe933b".into(),
            output: Default::default(),
            operation_scope: None,
        };
        let mut cursor = None;
        let mut text = String::new();
        let mut completed = false;
        for _ in 0..4 {
            let result = f
                .store
                .query(
                    &Command::Message(MessageQuery {
                        message: MessageId::new("m"),
                        body: BodyReadRequest {
                            cursor,
                            offset: None,
                            max_bytes: 65_536,
                        },
                    }),
                    &read,
                    &budget(),
                )
                .unwrap();
            let CommandResult::Message(message) = result else {
                panic!("body response")
            };
            let MessageContent::Ordinary {
                body_data,
                body_complete,
                body_next_cursor,
                ..
            } = message.content
            else {
                panic!("ordinary body")
            };
            text.push_str(&body_data);
            if body_complete {
                completed = true;
                break;
            }
            cursor = Some(body_next_cursor.unwrap());
        }
        assert!(completed);
        assert_eq!(text, "z".repeat(65_536));
        f.store
            .advance_cursors(
                &AdvanceCursors {
                    in_turn: false,
                    invitations: vec![],
                    claim: claim.clone(),
                    operation: OperationId::new(operation),
                    displayed: vec![MessageId::new("m")],
                },
                &budget(),
            )
            .unwrap();
    }
    #[test]
    fn continued_body_retirement_is_private_and_charged_once() {
        use herdr_threads::{
            ports::ReadContext,
            protocol::{
                commands::{Command, InboxQuery},
                pagination::{InboxBatchV2CursorState, PageRequest},
                results::ErrorCode,
            },
        };
        let (f, claim, cursor) = continued_body_fixture();
        read_complete_body_and_advance(&f, &claim, "private-retire");
        let public = f
            .store
            .query(
                &Command::InboxBatchV2(InboxQuery {
                    thread: None,
                    seat: Some(claim.seat.clone()),
                    claim: Some(claim.clone()),
                    page: PageRequest {
                        cursor: Some(cursor.clone()),
                        limit: 32,
                        max_bytes: 65_536,
                    },
                }),
                &ReadContext {
                    instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                    output: Default::default(),
                    operation_scope: None,
                },
                &budget(),
            )
            .unwrap_err();
        assert_eq!(
            public.code,
            ErrorCode::InvalidCursor,
            "public cursor eligibility remains strict"
        );
        // Isolate the one continued candidate: the other captured sources have
        // no pending items, and this control marks their existing walks complete.
        let mut state = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &claim.seat,
        )
        .unwrap();
        state.attention.receipts_done = true;
        state.attention.next_manifest_warning = true;
        let isolated = state.encode("34750f98bd59fcfc946da45aaabe933b").unwrap();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_cursor=?1 WHERE binding_id='b' AND consumer='watch'", [&isolated]).unwrap();
        let result = f
            .store
            .presentation_pull(f.input_for_claim("b", claim), &budget())
            .unwrap();
        assert_eq!(result.work.ordinary_continuation_key_probes, 15);
        assert_eq!(
            result.work.ordinary, 1,
            "retirement consumes exactly one existing source visit"
        );
        assert_eq!(result.work.ordinary_due_key_probes, 0);
        assert_eq!(result.work.ordinary_carry_key_probes, 0);
        let (cursor, partial, skip, done): (Option<String>, String, Option<String>, i64) = f.db.query_row("SELECT ordinary_cursor,ordinary_partial,ordinary_skip_body,ordinary_done FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(
            (cursor, partial.as_str(), skip, done),
            (None, "{}", None, 1)
        );
    }
    #[test]
    fn continued_pending_body_survives_own_display_cursor() {
        use herdr_threads::protocol::pagination::InboxBatchV2CursorState;
        let (f, claim, cursor) = continued_body_fixture();
        // A valid sub-8KiB private accumulator isolates a still-Pending
        // continued body after the captured receipt walk already completed.
        let mut state = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &claim.seat,
        )
        .unwrap();
        state.body.as_mut().unwrap().offset = 4096;
        state.attention.receipts_done = true;
        state.attention.next_manifest_warning = true;
        let cursor = state.encode("34750f98bd59fcfc946da45aaabe933b").unwrap();
        let partial = serde_json::json!({"m":"z".repeat(4096)}).to_string();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_cursor=?1,ordinary_partial=?2,ordinary_skip_body=NULL WHERE binding_id='b' AND consumer='watch'", rusqlite::params![cursor, partial]).unwrap();
        f.db.execute("UPDATE receipt_state SET state='pending',acked_at=NULL,ack_binding_id=NULL,ack_actor_seat_id=NULL,ack_generation=NULL,ack_observation=NULL WHERE message_id='m'", []).unwrap();
        read_complete_body_and_advance(&f, &claim, "pending-retained");
        let result = f
            .store
            .presentation_pull(f.input_for_claim("b", claim), &budget())
            .unwrap();
        assert_eq!(result.work.ordinary_continuation_key_probes, 15);
        let DeliveryPullReplyV2::Page(page) = result.reply else {
            panic!("pending continuation remains model-visible")
        };
        assert!(page.items.iter().any(|item| matches!(item, PresentationItemV2::Message { id, content, .. } if id.as_str()=="m" && content.ack_required)));
        assert_eq!(
            f.db.query_row(
                "SELECT state FROM receipt_state WHERE message_id='m'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "pending"
        );
    }
    #[test]
    fn continued_unread_body_survives_sibling_full_read() {
        use herdr_threads::protocol::{ids::MessageId, pagination::InboxBatchV2CursorState};
        let (f, claim, cursor) = continued_body_fixture_with_clock(Arc::new(FixedClock), true);
        let mut state = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &claim.seat,
        )
        .unwrap();
        state.body.as_mut().unwrap().offset = 4096;
        state.attention.receipts_done = true;
        state.attention.next_manifest_warning = true;
        let partial = serde_json::json!({"m":"z".repeat(4096)}).to_string();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_cursor=?1,ordinary_partial=?2,ordinary_skip_body=NULL WHERE binding_id='b' AND consumer='watch'", rusqlite::params![state.encode("34750f98bd59fcfc946da45aaabe933b").unwrap(),partial]).unwrap();
        assert_eq!(
            f.db.query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id='a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        let mut sibling = f.claim("a");
        sibling.execution = ExecutionId::new("22222222-2222-4222-8222-222222222222");
        f.db.execute(
            "UPDATE occupant_bindings SET execution_id=?1 WHERE binding_id='a'",
            [sibling.execution.as_str()],
        )
        .unwrap();
        read_complete_body_and_advance(&f, &sibling, "sibling-read");
        assert_eq!(
            f.db.query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id='a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        let result = f
            .store
            .presentation_pull(f.input_for_claim("b", claim), &budget())
            .unwrap();
        assert_eq!(result.work.ordinary_continuation_key_probes, 15);
        assert!(result.work.ordinary > 0);
        let DeliveryPullReplyV2::Page(page) = result.reply else {
            panic!("B's unread continuation must be emitted despite A's read")
        };
        let content = page
            .items
            .iter()
            .find_map(|item| match item {
                PresentationItemV2::Message { id, content, .. } if id.as_str() == "m" => {
                    Some(content)
                }
                _ => None,
            })
            .expect("exact continued B message");
        assert!(
            !content.ack_required,
            "A already settled the shared receipt"
        );
        assert_eq!(content.body_len, 65_536);
        assert!(content.truncated);
        assert_eq!(
            content.body,
            format!(
                "{}{}",
                "z".repeat(8192),
                herdr_threads::protocol::watch::truncation_marker_for(
                    &[],
                    &MessageId::new("m"),
                    false
                )
            )
        );
        assert_eq!(
            f.db.query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id='b'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn continued_body_refuses_corrupt_accumulator_and_missing_authority_without_progress() {
        use herdr_threads::protocol::results::ErrorCode;
        for missing_authority in [false, true] {
            let (f, claim, cursor) = continued_body_fixture();
            read_complete_body_and_advance(&f, &claim, "guard-read");
            if missing_authority {
                f.db.execute(
                    "DELETE FROM prepared_recipients WHERE preparation_id='prep' AND seat_id='s'",
                    [],
                )
                .unwrap();
            } else {
                f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_skip_body=NULL,ordinary_partial=?1 WHERE binding_id='b' AND consumer='watch'", [r#"{"m":"short"}"#]).unwrap();
            }
            let before: (String, String, Option<String>) = f.db.query_row("SELECT ordinary_cursor,ordinary_partial,ordinary_skip_body FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            let error = f
                .store
                .presentation_pull(f.input_for_claim("b", claim), &budget())
                .unwrap_err();
            assert_eq!(
                error.code,
                if missing_authority {
                    ErrorCode::InvalidCursor
                } else {
                    ErrorCode::StoreCorrupt
                }
            );
            let after: (String, String, Option<String>) = f.db.query_row("SELECT ordinary_cursor,ordinary_partial,ordinary_skip_body FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!(after, before);
            assert_eq!(after.0, cursor);
        }
    }
    #[test]
    fn continued_body_retirement_rolls_back_when_final_state_write_fails() {
        let (f, claim, cursor) = continued_body_fixture();
        read_complete_body_and_advance(&f, &claim, "rollback-read");
        f.db.execute_batch("CREATE TRIGGER refuse_continuation_save BEFORE UPDATE ON invitation_presentation_consumers WHEN NEW.binding_id='b' AND NEW.ordinary_cursor IS NOT OLD.ordinary_cursor BEGIN SELECT RAISE(ABORT,'test late presentation save'); END;").unwrap();
        assert!(
            f.store
                .presentation_pull(f.input_for_claim("b", claim.clone()), &budget())
                .is_err()
        );
        assert_eq!(f.db.query_row("SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get::<_,String>(0)).unwrap(), cursor);
        assert_eq!(f.db.query_row("SELECT ordinary_skip_body FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'", [], |r| r.get::<_,String>(0)).unwrap(), "m");
        f.db.execute_batch("DROP TRIGGER refuse_continuation_save;")
            .unwrap();
        let retried = f
            .store
            .presentation_pull(f.input_for_claim("b", claim), &budget())
            .unwrap();
        assert_eq!(retried.work.ordinary_continuation_key_probes, 15);
        assert!(retried.work.ordinary <= 100);
    }
    #[test]
    fn continued_body_cancellation_before_final_save_rolls_back_and_retries() {
        use herdr_threads::protocol::{
            pagination::InboxBatchV2CursorState, results::ErrorCode, time::Cancellation,
        };
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        struct CancelClock {
            token: Cancellation,
            armed: AtomicBool,
            calls: AtomicUsize,
            fired_at: AtomicUsize,
        }
        impl Clock for CancelClock {
            fn utc_now(&self) -> UtcMillis {
                UtcMillis(1000)
            }
            fn monotonic_now(&self) -> MonoInstant {
                let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                // Existing checks: two writer admission reads, then two done
                // source rotations and the empty receipt-source completion.
                // is_exhausted reads cancellation BEFORE this clock. Cancel
                // here and return an unexpired time so that check returns false.
                if self.armed.load(Ordering::SeqCst) && call == 2 + 2 + 1 {
                    self.fired_at.store(call, Ordering::SeqCst);
                    self.token.cancel();
                }
                MonoInstant(1)
            }
        }
        let token = Cancellation::default();
        let clock = Arc::new(CancelClock {
            token: token.clone(),
            armed: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            fired_at: AtomicUsize::new(0),
        });
        let (f, claim, cursor) = continued_body_fixture_with_clock(clock.clone(), false);
        read_complete_body_and_advance(&f, &claim, "cancel-before-save");
        let mut state = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &claim.seat,
        )
        .unwrap();
        // Keep one empty source-completion step after retirement. This makes
        // cancellation observable on OLD code before its final save, without
        // depending on a clock call introduced by the proposed new guard.
        state.attention.receipts_done = false;
        let receipts = state.attention.receipts.as_mut().unwrap();
        receipts.physical_after = receipts.physical_high_water;
        receipts.manifest_after = receipts.manifest_high_water;
        state.attention.next_manifest_warning = true;
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_cursor=?1 WHERE binding_id='b' AND consumer='watch'", [state.encode("34750f98bd59fcfc946da45aaabe933b").unwrap()]).unwrap();
        let snapshot = || {
            let mut statement = f.db.prepare("SELECT * FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'").unwrap();
            let columns = statement.column_count();
            statement
                .query_row([], |row| {
                    (0..columns)
                        .map(|n| row.get::<_, rusqlite::types::Value>(n))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
        };
        let before = snapshot();
        let input = f.input_for_claim("b", claim.clone());
        let mut cancelled_budget = budget();
        cancelled_budget.cancellation = token.clone();
        clock.calls.store(0, Ordering::SeqCst);
        clock.armed.store(true, Ordering::SeqCst);
        let result = f.store.presentation_pull(input, &cancelled_budget);
        clock.armed.store(false, Ordering::SeqCst);
        assert_eq!(
            clock.fired_at.load(Ordering::SeqCst),
            5,
            "cancellation must fire in an existing pre-save check"
        );
        assert!(
            token.is_cancelled(),
            "the request really was cancelled on old code too"
        );
        let error = result.expect_err(
            "an actually cancelled terminal continuation must not commit its private progress",
        );
        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!(
            snapshot(),
            before,
            "all private fields roll back byte-for-byte"
        );
        let retry = f
            .store
            .presentation_pull(f.input_for_claim("b", claim), &budget())
            .unwrap();
        assert_eq!(retry.work.ordinary_continuation_key_probes, 15);
        assert_eq!(retry.work.ordinary, 1);
        assert_ne!(
            snapshot(),
            before,
            "a fresh-budget retry commits the retirement"
        );
    }
    #[test]
    fn continued_body_cancellation_during_decision_sample_rolls_back_tentative_save() {
        use herdr_threads::protocol::{
            pagination::InboxBatchV2CursorState, results::ErrorCode, time::Cancellation,
        };
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        struct DecisionClock {
            token: Cancellation,
            armed: AtomicBool,
            decision_sample: AtomicBool,
            utc_samples: AtomicUsize,
            fired: AtomicBool,
        }
        impl Clock for DecisionClock {
            fn utc_now(&self) -> UtcMillis {
                // presentation_pull's source work does not sample UTC. Both
                // decision wrappers sample UTC only AFTER pull returned from
                // its tentative final state write, before decision acceptance.
                if self.armed.load(Ordering::SeqCst) {
                    self.utc_samples.fetch_add(1, Ordering::SeqCst);
                    self.decision_sample.store(true, Ordering::SeqCst);
                }
                UtcMillis(1000)
            }
            fn monotonic_now(&self) -> MonoInstant {
                if self.armed.load(Ordering::SeqCst)
                    && self.decision_sample.swap(false, Ordering::SeqCst)
                {
                    self.token.cancel();
                    self.fired.store(true, Ordering::SeqCst);
                }
                MonoInstant(1)
            }
        }
        let token = Cancellation::default();
        let clock = Arc::new(DecisionClock {
            token: token.clone(),
            armed: AtomicBool::new(false),
            decision_sample: AtomicBool::new(false),
            utc_samples: AtomicUsize::new(0),
            fired: AtomicBool::new(false),
        });
        let (f, claim, cursor) = continued_body_fixture_with_clock(clock.clone(), false);
        read_complete_body_and_advance(&f, &claim, "decision-cancel-read");
        let mut state = InboxBatchV2CursorState::decode_for(
            &cursor,
            "34750f98bd59fcfc946da45aaabe933b",
            &claim.seat,
        )
        .unwrap();
        state.attention.receipts_done = true;
        state.attention.next_manifest_warning = true;
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_cursor=?1 WHERE binding_id='b' AND consumer='watch'", [state.encode("34750f98bd59fcfc946da45aaabe933b").unwrap()]).unwrap();
        let snapshot = || {
            let mut statement = f.db.prepare("SELECT * FROM invitation_presentation_consumers WHERE binding_id='b' AND consumer='watch'").unwrap();
            let columns = statement.column_count();
            statement
                .query_row([], |row| {
                    (0..columns)
                        .map(|n| row.get::<_, rusqlite::types::Value>(n))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
        };
        let before = snapshot();
        let input = f.input_for_claim("b", claim.clone());
        let mut cancelled_budget = budget();
        cancelled_budget.cancellation = token.clone();
        clock.armed.store(true, Ordering::SeqCst);
        let result = f.store.presentation_pull(input, &cancelled_budget);
        clock.armed.store(false, Ordering::SeqCst);
        assert_eq!(
            clock.utc_samples.load(Ordering::SeqCst),
            1,
            "the final decision sample was reached after tentative save"
        );
        assert!(clock.fired.load(Ordering::SeqCst));
        assert!(
            token.is_cancelled(),
            "cancellation occurred at the actual decision boundary"
        );
        assert_eq!(
            result
                .expect_err("unaccepted presentation decision must roll back")
                .code,
            ErrorCode::Cancelled
        );
        assert_eq!(
            snapshot(),
            before,
            "tentative saved retirement is not an accepted commit"
        );
        let retry = f
            .store
            .presentation_pull(f.input_for_claim("b", claim), &budget())
            .unwrap();
        assert_eq!(retry.work.ordinary_continuation_key_probes, 15);
        assert_eq!(retry.work.ordinary, 1);
        assert_ne!(snapshot(), before);
    }
}
