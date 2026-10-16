//! `thread.created`, `thread.register` and `thread.direct` (fact-kinds design
//! section 5; ht-5fr.4.8): payloads, keys, idempotence, LWW and origin
//! authority. The two-node convergence cases are in `fact_convergence_thread`.
//!
//! The helpers at the top (authoring a kind on a [`Node`], reading threads)
//! are shared with that file.

use herdr_threads::node::tag_for;
use herdr_threads::protocol::ids::{
    SeatId, ThreadId, direct_thread_id, generate_public_id, prefix,
};
use herdr_threads::protocol::results::{ApiError, ErrorCode};
use herdr_threads::store::control;
use herdr_threads::store::journal;
use herdr_threads::store::materialize::{FactRoute, MaterializeInput, local_only};
use herdr_threads::store::thread_registers::{self, RegisterField as StoreField};
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::payload::{ReferencedSeats, RegisterField};
use herdr_threads::sync::facts::thread::{
    ThreadCreated, ThreadCreatedPayload, ThreadDirect, ThreadDirectPayload, ThreadRegister,
    ThreadRegisterPayload,
};
use herdr_threads::sync::facts::{FactKind, FactRegistry};
use herdr_threads::sync::hlc::HlcStamp;
use herdr_threads::test_support::fact_exchange;
use herdr_threads::test_support::write_guard;

use crate::facts_seat::{Node, registry};

// ---- shared fixtures --------------------------------------------------------

impl Node {
    /// A thread ID minted under this node's tag.
    pub fn mint_thread(&self) -> ThreadId {
        ThreadId::new(generate_public_id(prefix::THREAD, &tag_for(self.id, 0)))
    }

    /// A local seat, ready to be referenced (the emit publishes it).
    pub fn local_seat(&self) -> SeatId {
        let seat = self.mint_seat(0);
        self.add_local_seat(&seat);
        seat
    }

    /// Authors `thread.created` for a fresh thread created by `creator`.
    pub fn create_thread(
        &self,
        creator: &SeatId,
        topic: &str,
        name: Option<&str>,
    ) -> Result<ThreadId, ApiError> {
        let thread = self.mint_thread();
        let payload = ThreadCreatedPayload {
            thread: thread.clone(),
            topic: topic.into(),
            goal: "goal".into(),
            name: name.map(str::to_owned),
            creator: creator.clone(),
        };
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<ThreadCreated>(tx, self.ctx(), d, &payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })?;
        Ok(thread)
    }

    /// Authors `thread.register` above the register's current stamp, the way
    /// the commands do.
    pub fn write_register(
        &self,
        thread: &ThreadId,
        field: RegisterField,
        value: Option<&str>,
    ) -> Result<(), ApiError> {
        self.write_register_as(None, thread, field, value)
    }

    /// `write_register` naming the acting seat in the payload.
    pub fn write_register_as(
        &self,
        actor: Option<&SeatId>,
        thread: &ThreadId,
        field: RegisterField,
        value: Option<&str>,
    ) -> Result<(), ApiError> {
        let payload = ThreadRegisterPayload {
            thread: thread.clone(),
            field,
            value: value.map(str::to_owned),
            actor: actor.cloned(),
        };
        self.run(|tx, d| {
            let floor = thread_registers::current(tx, thread, store_field(field))
                .expect("read the register")
                .map(|(hlc, _)| hlc);
            FactRoute::Journal
                .emit::<ThreadRegister>(tx, self.ctx(), d, &payload, floor, |_, _, _| Ok(()))
                .map(|_| ())
        })
    }

    /// Authors `thread.direct` for the pair as `(lo, hi)` in order.
    pub fn author_direct(&self, a: &SeatId, b: &SeatId) -> Result<ThreadId, ApiError> {
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let thread = direct_thread_id(lo, hi).expect("two distinct seats");
        let payload = ThreadDirectPayload {
            thread: thread.clone(),
            seat_lo: lo.clone(),
            seat_hi: hi.clone(),
            creator: None,
        };
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<ThreadDirect>(tx, self.ctx(), d, &payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })?;
        Ok(thread)
    }

    /// `(topic, name, archived, created_at, journaled)` of a thread row.
    #[allow(clippy::type_complexity)]
    pub fn thread_row(
        &self,
        thread: &ThreadId,
    ) -> Option<(String, Option<String>, bool, i64, bool)> {
        self.query(
            &format!(
                "SELECT topic,name,archived,created_at,journaled FROM threads WHERE id='{}'",
                thread.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .into_iter()
        .next()
    }

    /// The parsed `event_json` of every `register:<thread>:<field>:` event,
    /// oldest first.
    pub fn register_event_json(&self, thread: &ThreadId, field: &str) -> Vec<serde_json::Value> {
        self.query(
            &format!(
                "SELECT event_json FROM messages WHERE thread_id='{}' AND event_key LIKE 'register:{}:{field}:%' ORDER BY sequence",
                thread.as_str(),
                thread.as_str()
            ),
            |r| r.get::<_, String>(0),
        )
        .into_iter()
        .map(|json| serde_json::from_str(&json).expect("event json"))
        .collect()
    }

    /// The event keys of a thread's timeline, oldest first.
    pub fn event_keys(&self, thread: &ThreadId) -> Vec<String> {
        self.query(
            &format!(
                "SELECT event_key FROM messages WHERE thread_id='{}' AND event_key IS NOT NULL ORDER BY sequence",
                thread.as_str()
            ),
            |r| r.get(0),
        )
    }

    /// `(created_hlc, created_node)` of a pair thread.
    pub fn direct_creator(&self, thread: &ThreadId) -> Option<(i64, String)> {
        self.query(
            &format!(
                "SELECT created_hlc,created_node FROM direct_threads WHERE thread_id='{}'",
                thread.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .into_iter()
        .next()
    }

    /// `(hlc, node)` of the `n`th entry of `kind` in this node's journal.
    pub fn stamp_of(&self, kind: &str, n: usize) -> (i64, String) {
        self.query(
            &format!(
                "SELECT j.hlc,l.node_uuid FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE j.kind='{kind}' ORDER BY j.ingest_seq"
            ),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .remove(n)
    }

    /// The row of one register: `(value, hlc, node)`.
    pub fn register(
        &self,
        thread: &ThreadId,
        field: &str,
    ) -> Option<(Option<String>, i64, String)> {
        self.query(
            &format!(
                "SELECT value,hlc,node_uuid FROM thread_registers WHERE thread_id='{}' AND field='{field}'",
                thread.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .into_iter()
        .next()
    }

    /// Every table the thread facts write, as one comparable string. Rows
    /// are projected for a bystander node, so no node's own rows drop out and
    /// two nodes' strings compare.
    pub fn thread_projection(&self) -> String {
        let specs: Vec<_> = herdr_threads::test_support::journal_oracle::replicated_tables()
            .into_iter()
            .filter(|t| matches!(t.name, "threads" | "direct_threads" | "thread_registers"))
            .collect();
        format!(
            "{:?}",
            herdr_threads::test_support::journal_oracle::replicated_projection(
                &self.store,
                herdr_threads::node::NodeUuid::from_bytes([0xee; 16]),
                &specs
            )
        )
    }
}

pub fn store_field(field: RegisterField) -> StoreField {
    match field {
        RegisterField::Topic => StoreField::Topic,
        RegisterField::Name => StoreField::Name,
        RegisterField::Archived => StoreField::Archived,
    }
}

/// Brings the nodes' seats and threads to every store.
pub fn sync_all(nodes: &[&Node]) {
    for a in nodes {
        for b in nodes {
            if !std::ptr::eq(*a, *b) {
                fact_exchange::sync(&a.store, &b.store);
            }
        }
    }
}

// ---- payloads, keys, registration ------------------------------------------

#[test]
fn payload_round_trips_creates_needs() {
    let thread = ThreadId::new("tAbc12345678");
    let creator = SeatId::new("sAbc12345678");
    let created = ThreadCreatedPayload {
        thread: thread.clone(),
        topic: "t".into(),
        goal: "g".into(),
        name: Some("n".into()),
        creator: creator.clone(),
    };
    let json = serde_json::to_string(&created).unwrap();
    assert_eq!(
        json,
        r#"{"thread":"tAbc12345678","topic":"t","goal":"g","name":"n","creator":"sAbc12345678"}"#
    );
    assert_eq!(
        serde_json::from_str::<ThreadCreatedPayload>(&json).unwrap(),
        created
    );
    // An additive field is ignored; goal and name default.
    let newer: ThreadCreatedPayload = serde_json::from_str(
        r#"{"thread":"tAbc12345678","topic":"t","creator":"sAbc12345678","future":1}"#,
    )
    .unwrap();
    assert_eq!((newer.goal.as_str(), newer.name), ("", None));
    assert_eq!(ThreadCreated::NAME, "thread.created");
    assert_eq!(
        ThreadCreated::creates(&created).map(|k| k.as_str().to_owned()),
        Some("tAbc12345678".into())
    );
    assert_eq!(
        ThreadCreated::needs(&created),
        vec![EntityKey::new("sAbc12345678").unwrap()]
    );
    assert_eq!(created.referenced_seats(), vec![creator.clone()]);

    let register = ThreadRegisterPayload {
        thread: thread.clone(),
        field: RegisterField::Archived,
        value: Some("1".into()),
        actor: None,
    };
    let json = serde_json::to_string(&register).unwrap();
    assert_eq!(
        json,
        r#"{"thread":"tAbc12345678","field":"archived","value":"1"}"#
    );
    assert_eq!(
        serde_json::from_str::<ThreadRegisterPayload>(&json).unwrap(),
        register
    );
    // An actor round-trips; an unknown extra field still decodes (additive rule).
    let acted = ThreadRegisterPayload {
        actor: Some(creator.clone()),
        ..register.clone()
    };
    let acted_json = serde_json::to_string(&acted).unwrap();
    assert!(
        acted_json.contains(r#""actor":"sAbc12345678""#),
        "{acted_json}"
    );
    assert_eq!(
        serde_json::from_str::<ThreadRegisterPayload>(&acted_json).unwrap(),
        acted
    );
    let extra: ThreadRegisterPayload = serde_json::from_str(
        r#"{"thread":"tAbc12345678","field":"archived","value":"1","future":7}"#,
    )
    .unwrap();
    assert_eq!(extra, register);
    let cleared: ThreadRegisterPayload =
        serde_json::from_str(r#"{"thread":"tAbc12345678","field":"name","value":null}"#).unwrap();
    assert_eq!(cleared.value, None);
    assert_eq!(ThreadRegister::NAME, "thread.register");
    assert_eq!(ThreadRegister::creates(&register), None);
    assert_eq!(
        ThreadRegister::needs(&register),
        vec![EntityKey::new("tAbc12345678").unwrap()]
    );
    assert!(register.referenced_seats().is_empty());

    let lo = SeatId::new("sAAAA12345678");
    let hi = SeatId::new("sBBBB12345678");
    let direct = ThreadDirectPayload {
        thread: direct_thread_id(&lo, &hi).unwrap(),
        seat_lo: lo.clone(),
        seat_hi: hi.clone(),
        creator: None,
    };
    let json = serde_json::to_string(&direct).unwrap();
    assert_eq!(
        serde_json::from_str::<ThreadDirectPayload>(&json).unwrap(),
        direct
    );
    assert_eq!(ThreadDirect::NAME, "thread.direct");
    assert_eq!(
        ThreadDirect::creates(&direct).map(|k| k.as_str().to_owned()),
        Some(direct.thread.as_str().to_owned())
    );
    assert_eq!(
        ThreadDirect::needs(&direct),
        vec![
            EntityKey::new(lo.as_str()).unwrap(),
            EntityKey::new(hi.as_str()).unwrap()
        ]
    );
    assert_eq!(direct.referenced_seats(), vec![lo, hi]);

    let builtins = FactRegistry::builtins();
    for kind in ["thread.created", "thread.register", "thread.direct"] {
        assert!(builtins.lookup(kind, 1).is_some(), "{kind} registered");
    }
}

// ---- thread.direct validation ----------------------------------------------

#[test]
fn wrong_direct_id_rejected() {
    let node = Node::with(51, registry());
    let (a, b) = (node.local_seat(), node.local_seat());
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let right = direct_thread_id(&lo, &hi).unwrap();
    let bad_cases = [
        (
            "a random id",
            ThreadDirectPayload {
                thread: node.mint_thread(),
                seat_lo: lo.clone(),
                seat_hi: hi.clone(),
                creator: None,
            },
        ),
        (
            "seats out of order",
            ThreadDirectPayload {
                thread: right.clone(),
                seat_lo: hi.clone(),
                seat_hi: lo.clone(),
                creator: None,
            },
        ),
        (
            "one seat twice",
            ThreadDirectPayload {
                thread: right.clone(),
                seat_lo: lo.clone(),
                seat_hi: lo.clone(),
                creator: None,
            },
        ),
    ];
    for (what, payload) in bad_cases {
        let err = node
            .run(|tx, d| {
                FactRoute::Journal
                    .emit::<ThreadDirect>(tx, node.ctx(), d, &payload, None, |_, _, _| Ok(()))
                    .map(|_| ())
            })
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidRequest, "{what}");
        assert!(
            err.detail.contains("does not match its seat pair"),
            "{what}: {}",
            err.detail
        );
    }
    assert_eq!(node.count_kind("thread.direct"), 0, "nothing was journaled");
    assert!(node.thread_row(&right).is_none());
}

// ---- idempotence -----------------------------------------------------------

#[test]
fn each_kind_applied_twice_no_change_no_event() {
    let node = Node::with(52, registry());
    let creator = node.local_seat();
    let thread = node.create_thread(&creator, "topic", Some("name")).unwrap();
    let (a, b) = (node.local_seat(), node.local_seat());
    let pair = node.author_direct(&a, &b).unwrap();

    // thread.created: the same payload again.
    let before = (
        node.thread_projection(),
        node.event_keys(&thread),
        node.event_keys(&pair),
    );
    let again = ThreadCreatedPayload {
        thread: thread.clone(),
        topic: "topic".into(),
        goal: "goal".into(),
        name: Some("name".into()),
        creator: creator.clone(),
    };
    node.run(|tx, d| journal::author::<ThreadCreated>(tx, node.ctx(), d, &again).map(|_| ()))
        .unwrap();
    // thread.direct: the same pair again.
    let (lo, hi) = if a < b { (&a, &b) } else { (&b, &a) };
    let again_direct = ThreadDirectPayload {
        thread: pair.clone(),
        seat_lo: lo.clone(),
        seat_hi: hi.clone(),
        creator: None,
    };
    node.run(|tx, d| journal::author::<ThreadDirect>(tx, node.ctx(), d, &again_direct).map(|_| ()))
        .unwrap();
    // thread.register: the current value again (a later stamp, the same value).
    node.write_register(&thread, RegisterField::Topic, Some("topic"))
        .unwrap();
    node.write_register(&thread, RegisterField::Name, Some("name"))
        .unwrap();
    node.write_register(&thread, RegisterField::Archived, Some("0"))
        .unwrap();
    let after_events = (node.event_keys(&thread), node.event_keys(&pair));
    assert_eq!(
        (after_events.0, after_events.1),
        (before.1.clone(), before.2.clone()),
        "no second event"
    );
    // Only the register stamps moved (the winners are newer copies of the
    // same values); every replicated value is the same.
    let rows = |p: &str| -> String {
        p.split("thread_registers")
            .next()
            .unwrap_or_default()
            .into()
    };
    assert_eq!(rows(&node.thread_projection()), rows(&before.0));
    assert_eq!(node.count_kind("thread.created"), 2);
    assert_eq!(node.count_kind("thread.direct"), 2);
    assert!(
        node.journal()
            .iter()
            .all(|(_, state, _)| state == "applied"),
        "every copy applied"
    );
    assert_eq!(
        node.thread_row(&thread),
        Some((
            "topic".into(),
            Some("name".into()),
            false,
            node_created_at(&node, &thread),
            true
        ))
    );
}

fn node_created_at(node: &Node, thread: &ThreadId) -> i64 {
    node.thread_row(thread).expect("a thread row").3
}

/// A fixture input for a direct materializer call: an entry at `hlc` from
/// `node` carried by the journal row `ingest_seq`.
fn input_at(node: &Node, hlc: HlcStamp, ingest_seq: i64) -> MaterializeInput<'static> {
    MaterializeInput {
        decision: herdr_threads::store::connection::DecisionInstant {
            utc: herdr_threads::protocol::time::UtcMillis(1),
            monotonic: herdr_threads::protocol::time::MonoInstant(1),
        },
        decision_seq: 1,
        origin_node: node.id,
        origin_ms: 1,
        local_origin: true,
        own_history: true,
        provenance: "local",
        origin_hlc: hlc,
        origin_counter: 99,
        ingest_seq: Some(ingest_seq),
        suppress: &|_, _| Ok(false),
        scope: Default::default(),
    }
}

#[test]
fn reissued_register_loses_and_appends_nothing() {
    let node = Node::with(53, registry());
    let creator = node.local_seat();
    let thread = node.create_thread(&creator, "first", None).unwrap();
    node.write_register(&thread, RegisterField::Topic, Some("second"))
        .unwrap();
    let (value, hlc, _) = node.register(&thread, "topic").unwrap();
    assert_eq!(value.as_deref(), Some("second"));
    let events = node.event_keys(&thread);
    let projection = node.thread_projection();
    // A second incarnation re-issues the register (here: the copy a fresh log
    // would carry). Its entry is stamped at the original's `(hlc, node)`, so it
    // loses (equal pair) and so does anything older.
    node.run(|tx, d| journal::begin_incarnation(tx, node.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    let carrier: i64 = node.query("SELECT max(ingest_seq) FROM journal", |r| r.get(0))[0];
    for stamp in [
        HlcStamp::from_u64(hlc as u64),
        HlcStamp::from_u64(hlc as u64 - 1),
    ] {
        for value in ["third", "second"] {
            let _scope = write_guard::enter_replicated_write_scope();
            node.run(|tx, _| {
                control::materialize_register(
                    tx,
                    &input_at(&node, stamp, carrier),
                    &ThreadRegisterPayload {
                        thread: thread.clone(),
                        field: RegisterField::Topic,
                        value: Some(value.into()),
                        actor: None,
                    },
                )
            })
            .unwrap();
        }
    }
    assert_eq!(node.event_keys(&thread), events, "no event appended");
    assert_eq!(node.thread_projection(), projection, "no value changed");
    assert_eq!(
        node.thread_row(&thread).map(|r| r.0).as_deref(),
        Some("second")
    );
}

// ---- origin authority ------------------------------------------------------

/// A, B and C with B's seat published everywhere.
fn three_nodes(seed: u8) -> (Node, Node, Node, SeatId) {
    let reg = registry();
    let (a, b, c) = (
        Node::with(seed, reg),
        Node::with(seed + 1, reg),
        Node::with(seed + 2, reg),
    );
    let seat = b.local_seat();
    // B publishes its seat by referencing it in a thread of its own.
    b.create_thread(&seat, "b's", None).unwrap();
    sync_all(&[&a, &b, &c]);
    (a, b, c, seat)
}

#[test]
fn foreign_creator_thread_created_failed() {
    let (a, _b, c, b_seat) = three_nodes(54);
    assert!(c.seat(&b_seat).is_some(), "C holds B's seat");
    // C creates a thread whose creator is B's seat. C's own apply passes
    // (local origin); every other node rejects it.
    let thread = c.create_thread(&b_seat, "forged", None).unwrap();
    fact_exchange::sync(&a.store, &c.store);
    let failed: Vec<_> = a
        .journal()
        .into_iter()
        .filter(|(k, _, _)| k == "thread.created")
        .filter(|(_, state, _)| state == "failed")
        .collect();
    assert_eq!(failed.len(), 1);
    let error = failed[0].2.as_deref().unwrap();
    assert!(
        error.contains("is not the authorized author of thread.created for")
            && error.contains(b_seat.as_str()),
        "{error}"
    );
    assert!(a.thread_row(&thread).is_none(), "no thread row");
    assert!(a.register(&thread, "topic").is_none(), "no register row");
    assert!(a.event_keys(&thread).is_empty(), "no event");
}

#[test]
fn thread_direct_by_non_home_failed() {
    let (a, b, c, b_seat) = three_nodes(57);
    let a_seat = a.local_seat();
    a.create_thread(&a_seat, "a's", None).unwrap();
    sync_all(&[&a, &b, &c]);
    assert!(c.seat(&a_seat).is_some() && c.seat(&b_seat).is_some());
    // C is home to neither seat of the pair.
    let pair = c.author_direct(&a_seat, &b_seat).unwrap();
    fact_exchange::sync(&a.store, &c.store);
    fact_exchange::sync(&b.store, &c.store);
    for node in [&a, &b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == "thread.direct" && state == "failed")
            .collect();
        assert_eq!(failed.len(), 1);
        assert!(
            failed[0]
                .2
                .as_deref()
                .unwrap()
                .contains("is not the authorized author of thread.direct for"),
            "{:?}",
            failed[0].2
        );
        assert!(node.thread_row(&pair).is_none(), "no thread row");
        assert!(node.direct_creator(&pair).is_none(), "no direct row");
    }
}

// ---- the node-local route --------------------------------------------------

#[test]
fn local_only_creation_journals_nothing() {
    let node = Node::with(58, registry());
    let before = node.journal_kinds();
    let thread = node.mint_thread();
    node.run(|tx, d| {
        let input = local_only(tx, d)?;
        control::materialize_thread_created(
            tx,
            &input,
            &control::NewThread {
                thread: &thread,
                topic: "managed",
                goal: "goal",
                name: None,
                creator: None,
                managed_owner: None,
                journaled: false,
            },
        )
    })
    .unwrap();
    assert_eq!(node.journal_kinds(), before, "no journal entry");
    let row = node.thread_row(&thread).unwrap();
    assert_eq!((row.0.as_str(), row.4), ("managed", false), "journaled = 0");
    assert!(node.register(&thread, "topic").is_none(), "no register");
    assert!(node.event_keys(&thread).is_empty(), "no creation event");
}
