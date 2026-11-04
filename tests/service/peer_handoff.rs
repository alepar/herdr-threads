//! PeerHandoffRequester tests: a real `SqliteStore` (no journal), a fake sender recording the
//! bytes it is given, and a manual clock.

use super::*;

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use crate::protocol::authority::{CallerClaim, CallerRole, Harness};
use crate::protocol::ids::{ExecutionId, HostTargetId, NativeSessionId};
use crate::protocol::time::UtcMillis;
use crate::store::StoreSettings;
use crate::sync::frame::{LaunchPlan, LaunchTarget};
use crate::sync::handoff_block::SenderBlock;

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const TARGET: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OTHER: &str = "cccccccccccccccccccccccccccccccc";
const AUTHOR: &str = "sAAAauthor01";
const REMOTE: &str = "sBBBremote01";
const T0: i64 = 1_700_000_000_000;

struct TestClock {
    utc: AtomicI64,
    mono: AtomicU64,
    started: std::time::Instant,
    wall: std::sync::atomic::AtomicBool,
}
impl TestClock {
    fn advance(&self, ms: u64) {
        self.utc.fetch_add(ms as i64, Ordering::SeqCst);
        self.mono.fetch_add(ms, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        let mono = self.mono.load(Ordering::SeqCst);
        if self.wall.load(Ordering::SeqCst) {
            let elapsed = u64::try_from(self.started.elapsed().as_millis()).unwrap();
            return MonoInstant(mono + elapsed);
        }
        MonoInstant(mono)
    }
}

#[derive(Default)]
struct Fake {
    sent: Mutex<Vec<Vec<u8>>>,
    answers: Mutex<VecDeque<Option<LaunchAnswer>>>,
}
impl Fake {
    fn answer(&self, answer: Option<LaunchAnswer>) {
        self.answers.lock().unwrap().push_back(answer);
    }
    fn sent(&self) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap().clone()
    }
}
impl RequestSender for Fake {
    fn send(
        &self,
        _: NodeUuid,
        bytes: &[u8],
        _: Duration,
        _: &Cancellation,
    ) -> Option<LaunchAnswer> {
        self.sent.lock().unwrap().push(bytes.to_vec());
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Some(LaunchAnswer::Answered(LaunchState::Running {
                pane: None,
            })))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Rig {
    store: SqliteStore,
    writer: FairWriter,
    clock: Arc<TestClock>,
    identity: NodeIdentity,
    log: LogFn,
    lines: Arc<Mutex<Vec<String>>>,
    fake: Fake,
    dir: std::path::PathBuf,
}

impl Rig {
    fn new() -> Self {
        let dir = crate::test_support::scratch::temp_path(format!(
            "peer-handoff-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let clock = Arc::new(TestClock {
            utc: AtomicI64::new(T0),
            mono: AtomicU64::new(1_000),
            started: std::time::Instant::now(),
            wall: std::sync::atomic::AtomicBool::new(false),
        });
        let ctx = StoreContext::new(dir.join("store.db"), clock.clone()).with_test_host();
        let db = ctx.open_writer().unwrap();
        db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
        db.execute(
            &format!(
                "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
                 VALUES ('{AUTHOR}','{INSTANCE}','resolved','native','p',1,0,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))"
            ),
            [],
        )
        .unwrap();
        db.execute(
            &format!(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) \
                 VALUES ('{REMOTE}','{INSTANCE}','remote','native',1,0,'{TARGET}',0,'agent',1)"
            ),
            [],
        )
        .unwrap();
        db.execute(
            &format!(
                "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) \
                 VALUES ('{AUTHOR}',1,0,'p','b',1,'codex','n','x','cooperative_top_level',0,'term-p','inc')"
            ),
            [],
        )
        .unwrap();
        // The seats' creating facts, as replication would have applied them: a `thread.direct`
        // needs both seats' entities.
        db.execute(
            "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
             head_hash,applied_through,state,first_seen_at) \
             VALUES (?1,1,0,zeroblob(32),zeroblob(32),2,zeroblob(32),0,'active',0)",
            [TARGET],
        )
        .unwrap();
        let log_ix = db.last_insert_rowid();
        for (n, seat) in [AUTHOR, REMOTE].into_iter().enumerate() {
            let n = n as i64 + 1;
            db.execute(
                "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,needs,payload,signature,ingested_at,apply_state,applied_seq,raw) \
                 VALUES (?1,?2,zeroblob(32),zeroblob(32),1,'test.seat',1,x'0000',?3,'',x'7b7d',zeroblob(64),0,'applied',?2+1000000,zeroblob(100))",
                rusqlite::params![log_ix, n, seat],
            )
            .unwrap();
        }
        drop(db);
        let store = SqliteStore::new(ctx, INSTANCE.to_owned(), StoreSettings::default()).unwrap();
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = lines.clone();
        Self {
            store,
            writer: FairWriter::new(32),
            clock,
            identity: NodeIdentity::from_seed([7; 32]),
            log: Arc::new(move |l: &str| sink.lock().unwrap().push(l.to_owned())),
            lines,
            fake: Fake::default(),
            dir,
        }
    }

    fn requester(&self) -> Requester<'_> {
        Requester {
            store: &self.store,
            writer: &self.writer,
            clock: self.clock.as_ref(),
            identity: &self.identity,
            log: &self.log,
            sender: &self.fake,
            background: false,
        }
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        }
    }

    fn sql<R>(&self, f: impl FnOnce(&rusqlite::Connection) -> R) -> R {
        f(&self.store.context().open_writer().unwrap())
    }

    fn row(&self, n: u128) -> Option<Obligation> {
        self.sql(|c| ledger::get(c, &uuid::Uuid::from_u128(n).to_string()).unwrap())
    }

    fn poll(&self, n: u128, claim: Option<&CallerClaim>) -> Result<Polled, ApiError> {
        self.requester().poll(
            TARGET.parse().unwrap(),
            request(n, AUTHOR),
            claim,
            false,
            &self.budget(),
        )
    }
}

fn request(n: u128, requester_seat: &str) -> LaunchRequest {
    LaunchRequest {
        id: uuid::Uuid::from_u128(n),
        requester_seat: requester_seat.into(),
        plan: LaunchPlan {
            target: LaunchTarget::Pane {
                space: None,
                tab: None,
                pane: "w1:p1".into(),
            },
            kind: "claude".into(),
            message: Some(Box::new(SenderBlock {
                body: "look at this".into(),
                ack_after_ms: Some(60_000),
                ..SenderBlock::default()
            })),
            ..LaunchPlan::default()
        },
    }
}

fn claim(seat: &str) -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: INSTANCE.into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}

fn decode(bytes: &[u8]) -> LaunchRequest {
    serde_json::from_slice(bytes).unwrap()
}

fn launched(seat: &str) -> Option<LaunchAnswer> {
    Some(LaunchAnswer::Answered(LaunchState::Launched {
        seat: seat.into(),
        pane: "w1:p2".into(),
    }))
}

fn ledger_rows(rig: &Rig) -> i64 {
    rig.sql(|c| {
        c.query_row("SELECT count(*) FROM handoff_obligations", [], |r| r.get(0))
            .unwrap()
    })
}

// Kills: a draft accepted on a claim naming another seat, a subagent claim, a seat with no open
// top-level binding or a retired seat; and a ledger row or frame left behind by a refusal.
#[test]
fn claim_mismatch_refused_no_row() {
    let rig = Rig::new();
    let wrong_seat = rig.poll(1, Some(&claim(REMOTE)));
    assert_eq!(wrong_seat.err().unwrap().code, ErrorCode::CallerUnverified);
    assert_eq!(
        rig.poll(1, None).err().unwrap().code,
        ErrorCode::CallerUnverified
    );
    let mut subagent = claim(AUTHOR);
    subagent.role = CallerRole::Subagent;
    assert_eq!(
        rig.poll(1, Some(&subagent)).err().unwrap().code,
        ErrorCode::CallerUnverified
    );
    rig.sql(|c| {
        c.execute("UPDATE occupant_bindings SET role='subagent'", [])
            .unwrap()
    });
    assert_eq!(
        rig.poll(1, Some(&claim(AUTHOR))).err().unwrap().code,
        ErrorCode::CallerUnverified
    );
    rig.sql(|c| {
        c.execute(
            "UPDATE occupant_bindings SET role='top_level', ended_at=1",
            [],
        )
        .unwrap()
    });
    assert_eq!(
        rig.poll(1, Some(&claim(AUTHOR))).err().unwrap().code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(ledger_rows(&rig), 0);
    assert!(rig.fake.sent().is_empty());
}

// Kills: a block not filled from the node and the binding, a row whose bytes differ from what was
// sent, a signature that does not verify under the node key.
#[test]
fn first_send_signs_and_records() {
    let rig = Rig::new();
    let polled = rig.poll(2, Some(&claim(AUTHOR))).unwrap();
    let row = rig.row(2).unwrap();
    assert_eq!(row.state, State::Sent);
    assert_eq!(polled.message, row.message);
    let sent = rig.fake.sent();
    assert_eq!(sent, vec![row.request_bytes.clone()]);
    let block = *decode(&sent[0]).plan.message.unwrap();
    block.verify(&rig.identity.public_key()).unwrap();
    assert_eq!(block.sender_node, Some(rig.identity.uuid()));
    assert_eq!(block.requested_at, T0);
    assert_eq!(block.author_role, Some(AuthorRole::Agent));
    let id = block.message.unwrap();
    assert_eq!(id.as_str().len(), 12);
    assert!(id.as_str().starts_with('m'));
    assert_eq!(row.message, id.as_str());
    assert_eq!(row.deadline_at, Some(T0 + 60_000));
}

// Kills: a re-sign on a later poll (a changed role or time would change the bytes).
#[test]
fn repeat_polls_send_row_bytes() {
    let rig = Rig::new();
    rig.poll(3, Some(&claim(AUTHOR))).unwrap();
    rig.sql(|c| {
        c.execute("UPDATE occupant_bindings SET harness='human'", [])
            .unwrap()
    });
    rig.clock.advance(5_000);
    let again = rig.poll(3, Some(&claim(AUTHOR))).unwrap();
    let sent = rig.fake.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], sent[1]);
    assert_eq!(again.message, rig.row(3).unwrap().message);
}

// Kills: a send that signs its own bytes although another poll's row won the insert.
#[test]
fn concurrent_first_insert_uses_winner() {
    let rig = Rig::new();
    let winner = b"{\"id\":\"winner\"}".to_vec();
    rig.sql(|c| {
        ledger::insert_or_get(
            c,
            &NewObligation {
                request: &uuid::Uuid::from_u128(4).to_string(),
                target_node: TARGET,
                author: AUTHOR,
                message: "mwinner",
                requested_at: T0,
                deadline_at: None,
                request_bytes: &winner,
            },
        )
        .unwrap();
    });
    let polled = rig.poll(4, Some(&claim(AUTHOR))).unwrap();
    assert_eq!(rig.fake.sent(), vec![winner]);
    assert_eq!(polled.message, "mwinner");
}

// Kills: any answer moving the ledger the wrong way: Launched not recording the seat or opening
// the pair, a refusal deleting or leaving the row, a staged failure or a non-answer changing it.
#[test]
fn settle_table() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    // Launched: seat recorded, pair thread opened locally.
    rig.fake.answer(launched(REMOTE));
    let polled = rig.poll(5, Some(&c)).unwrap();
    assert!(matches!(
        polled.answer,
        RemoteLaunchAnswer::Answered {
            state: LaunchState::Launched { .. }
        }
    ));
    let row = rig.row(5).unwrap();
    assert_eq!(
        (row.state, row.seat.as_deref()),
        (State::Launched, Some(REMOTE))
    );
    let thread = direct_thread_id(&SeatId::new(AUTHOR), &SeatId::new(REMOTE)).unwrap();
    let exists: bool = rig.sql(|c| {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1)",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert!(
        exists,
        "the pair thread is opened: {:?}",
        rig.lines.lock().unwrap()
    );

    // Refused, plain Failed, NoCap: failed with the answer, shown to the polling CLI.
    for (n, answer) in [
        (
            6,
            Some(LaunchAnswer::Answered(LaunchState::Refused {
                detail: "no".into(),
            })),
        ),
        (
            7,
            Some(LaunchAnswer::Answered(LaunchState::Failed {
                detail: "boom".into(),
                pane: None,
                unconfirmed: false,
                staged: None,
            })),
        ),
        (
            8,
            Some(LaunchAnswer::NoCap {
                version: Some("2.9.0".into()),
                cap: crate::sync::frame::PEER_HANDOFF_CAP,
            }),
        ),
    ] {
        rig.fake.answer(answer);
        let polled = rig.poll(n, Some(&c)).unwrap();
        let row = rig.row(n).unwrap();
        assert_eq!(row.state, State::Failed, "{n}");
        assert!(row.alert_shown_at.is_some(), "{n}");
        let stored: RemoteLaunchAnswer =
            serde_json::from_str(row.answer.as_deref().unwrap()).unwrap();
        assert_eq!(stored, polled.answer, "{n}");
    }
    assert!(matches!(
        serde_json::from_str::<RemoteLaunchAnswer>(rig.row(8).unwrap().answer.as_deref().unwrap())
            .unwrap(),
        RemoteLaunchAnswer::NoCap { cap, .. } if cap == "peer.handoff.v1"
    ));

    // Failed after staging, and every non-answer: unchanged.
    let unchanged = [
        (
            9,
            Some(LaunchAnswer::Answered(LaunchState::Failed {
                detail: "late".into(),
                pane: None,
                unconfirmed: false,
                staged: Some(crate::sync::frame::LaunchStaged {
                    thread: "t".into(),
                    message: "m".into(),
                }),
            })),
        ),
        (
            10,
            Some(LaunchAnswer::Answered(LaunchState::Unavailable {
                reason: "busy".into(),
            })),
        ),
        (11, None),
        (12, Some(LaunchAnswer::NotLinked)),
        (13, Some(LaunchAnswer::LinkLost)),
    ];
    for (n, answer) in unchanged {
        rig.fake.answer(answer);
        rig.poll(n, Some(&c)).unwrap();
        assert_eq!(rig.row(n).unwrap().state, State::Sent, "{n}");
    }
    // Launched but the seat is not replicated here: recorded, reported as such.
    rig.fake.answer(launched("sZZZunknown1"));
    let polled = rig.poll(14, Some(&c)).unwrap();
    assert!(matches!(
        polled.answer,
        RemoteLaunchAnswer::NotReplicated { .. }
    ));
    assert_eq!(rig.row(14).unwrap().state, State::Launched);
}

// Kills: a terminal row that triggers a send, and wrong ledger answers.
#[test]
fn terminal_rows_answer_from_ledger() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    for n in [20, 21, 22] {
        rig.poll(n, Some(&c)).unwrap();
    }
    let sends = rig.fake.sent().len();
    let (m20, m21) = (rig.row(20).unwrap().message, rig.row(21).unwrap().message);
    rig.sql(|c| {
        assert!(ledger::link(c, &m20, REMOTE, T0 + 1).unwrap());
        c.execute(
            "UPDATE handoff_obligations SET seat='sBBBremote01' WHERE request=?1",
            [uuid::Uuid::from_u128(21).to_string()],
        )
        .unwrap();
        assert!(
            ledger::fail(
                c,
                &uuid::Uuid::from_u128(22).to_string(),
                "{\"k\":\"timeout\",\"secs\":3}",
                false,
                T0 + 1
            )
            .unwrap()
        );
    });
    rig.clock.advance(OBLIGATION_TTL_MS as u64 + 1);
    let thread = direct_thread_id(&SeatId::new(AUTHOR), &SeatId::new(REMOTE))
        .unwrap()
        .as_str()
        .to_owned();
    let linked = rig.poll(20, Some(&c)).unwrap();
    assert_eq!(
        linked.answer,
        RemoteLaunchAnswer::HandoffLedger {
            outcome: HandoffLedgerOutcome::Linked {
                seat: REMOTE.into(),
                thread,
                message: m20,
            }
        }
    );
    let expired = rig.poll(21, Some(&c)).unwrap();
    assert_eq!(
        expired.answer,
        RemoteLaunchAnswer::HandoffLedger {
            outcome: HandoffLedgerOutcome::Expired {
                seat: Some(REMOTE.into())
            }
        }
    );
    assert_eq!(expired.message, m21);
    let failed = rig.poll(22, Some(&c)).unwrap();
    assert_eq!(failed.answer, RemoteLaunchAnswer::Timeout { secs: 3 });
    assert_eq!(rig.fake.sent().len(), sends, "no frame for a terminal row");
}

// Kills: a stale request file that signs or records anything.
#[test]
fn resume_only_without_row_answers_unknown() {
    let rig = Rig::new();
    let polled = rig
        .requester()
        .poll(
            TARGET.parse().unwrap(),
            request(30, AUTHOR),
            Some(&claim(AUTHOR)),
            true,
            &rig.budget(),
        )
        .unwrap();
    assert_eq!(
        polled.answer,
        RemoteLaunchAnswer::HandoffLedger {
            outcome: HandoffLedgerOutcome::Unknown
        }
    );
    assert_eq!(ledger_rows(&rig), 0);
    assert!(rig.fake.sent().is_empty());
}

// Kills: a window longer than the ledger lifetime accepted (its row would expire before the
// deadline), and the boundary itself refused.
#[test]
fn deadline_over_ttl_refused() {
    let rig = Rig::new();
    let mut over = request(31, AUTHOR);
    over.plan.message.as_mut().unwrap().ack_after_ms = Some(OBLIGATION_TTL_MS + 1);
    let err = rig
        .requester()
        .poll(
            TARGET.parse().unwrap(),
            over,
            Some(&claim(AUTHOR)),
            false,
            &rig.budget(),
        )
        .err()
        .unwrap();
    assert_eq!(err.detail, DEADLINE_TOO_LONG);
    assert_eq!(ledger_rows(&rig), 0);
    assert!(rig.fake.sent().is_empty());
    let mut edge = request(32, AUTHOR);
    edge.plan.message.as_mut().unwrap().ack_after_ms = Some(OBLIGATION_TTL_MS);
    rig.requester()
        .poll(
            TARGET.parse().unwrap(),
            edge,
            Some(&claim(AUTHOR)),
            false,
            &rig.budget(),
        )
        .unwrap();
    assert_eq!(ledger_rows(&rig), 1);
}

// Kills: re-sending a terminal row, running expiry after the pass, an answer settled as shown, a
// failure not logged, an expiry not logged.
#[test]
fn resend_pass() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    for n in 40..=44 {
        rig.poll(n, Some(&c)).unwrap();
    }
    let m = |n: u128| rig.row(n).unwrap().message;
    rig.sql(|db| {
        assert!(
            ledger::launched(
                db,
                &uuid::Uuid::from_u128(41).to_string(),
                "sZZZunknown1",
                T0
            )
            .unwrap()
        );
        assert!(ledger::link(db, &m(42), REMOTE, T0).unwrap());
        assert!(ledger::fail(db, &uuid::Uuid::from_u128(43).to_string(), "{}", true, T0).unwrap());
    });
    let before = rig.fake.sent().len();
    rig.fake
        .answer(Some(LaunchAnswer::Answered(LaunchState::Refused {
            detail: "target said no".into(),
        })));
    let linked: HashSet<NodeUuid> = [TARGET.parse().unwrap()].into();
    let mut state = ResendState::default();
    let cancel = Cancellation::default();
    rig.requester()
        .resend_pass(&mut state, &linked, &cancel)
        .unwrap();
    let sent = rig.fake.sent();
    // Rows 40, 41 and 44 are open; 42 linked and 43 failed are not.
    assert_eq!(sent.len() - before, 3);
    let mut sent_ids: Vec<_> = sent[before..].iter().map(|b| decode(b).id).collect();
    sent_ids.sort();
    assert_eq!(
        sent_ids,
        [40u128, 41, 44].map(uuid::Uuid::from_u128).to_vec()
    );
    // The first re-send (row 40, oldest) got the refusal: failed, not shown to a CLI.
    let failed = rig.row(40).unwrap();
    assert_eq!(failed.state, State::Failed);
    assert_eq!(failed.alert_shown_at, None);
    assert!(
        rig.lines.lock().unwrap().iter().any(|l| l
            .contains(&format!("handoff {} to ", &failed.request[..8]))
            && l.ends_with("failed: target said no")),
        "{:?}",
        rig.lines.lock().unwrap()
    );

    // Expiry runs before the pass: an expired row is not re-sent and logs once.
    rig.clock.advance(OBLIGATION_TTL_MS as u64 + 1);
    let before = rig.fake.sent().len();
    rig.requester()
        .resend_pass(&mut state, &linked, &cancel)
        .unwrap();
    assert_eq!(rig.fake.sent().len(), before);
    assert_eq!(rig.row(44).unwrap().state, State::Expired);
    let lines = rig.lines.lock().unwrap().clone();
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("expired unconfirmed"))
            .count(),
        1,
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("expired unlinked: seat sZZZunknown1")),
        "{lines:?}"
    );
}

// Kills: terminal rows kept past `PRUNE_AFTER_MS` in a long-lived daemon (prune only at start),
// and a prune on every pass instead of once per `PRUNE_EVERY_MS`.
#[test]
fn resend_pass_prunes_old_terminal_rows() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    let id = |n: u128| uuid::Uuid::from_u128(n).to_string();
    rig.poll(60, Some(&c)).unwrap();
    rig.sql(|db| assert!(ledger::fail(db, &id(60), "{}", true, T0).unwrap()));
    let none: HashSet<NodeUuid> = HashSet::new();
    let cancel = Cancellation::default();
    let mut state = ResendState::default();
    // Inside the retention: kept.
    rig.requester()
        .resend_pass(&mut state, &none, &cancel)
        .unwrap();
    assert!(rig.row(60).is_some());
    rig.clock.advance(ledger::PRUNE_AFTER_MS as u64 + 1);
    rig.requester()
        .resend_pass(&mut state, &none, &cancel)
        .unwrap();
    assert!(
        rig.row(60).is_none(),
        "an old failed row survived a prune pass"
    );
    // A second old row is kept inside the period and deleted by the next one.
    rig.poll(61, Some(&c)).unwrap();
    rig.sql(|db| assert!(ledger::fail(db, &id(61), "{}", true, T0).unwrap()));
    rig.requester()
        .resend_pass(&mut state, &none, &cancel)
        .unwrap();
    assert!(rig.row(61).is_some(), "pruned inside PRUNE_EVERY_MS");
    rig.clock.advance(PRUNE_EVERY_MS as u64);
    rig.requester()
        .resend_pass(&mut state, &none, &cancel)
        .unwrap();
    assert!(rig.row(61).is_none());
}

// Kills: a pass that re-sends every tick, one that waits out the period after a link-up, a
// target that is not linked being sent to.
#[test]
fn resend_cadence() {
    let rig = Rig::new();
    rig.poll(50, Some(&claim(AUTHOR))).unwrap();
    let node: NodeUuid = TARGET.parse().unwrap();
    let other: NodeUuid = OTHER.parse().unwrap();
    let cancel = Cancellation::default();
    let mut state = ResendState::default();
    let count = |rig: &Rig| rig.fake.sent().len();
    let base = count(&rig);
    let pass = |state: &mut ResendState, linked: &[NodeUuid]| {
        let set: HashSet<NodeUuid> = linked.iter().copied().collect();
        rig.requester().resend_pass(state, &set, &cancel).unwrap();
    };
    // Not linked: nothing.
    pass(&mut state, &[other]);
    assert_eq!(count(&rig), base);
    // Newly linked: at once.
    pass(&mut state, &[other, node]);
    assert_eq!(count(&rig), base + 1);
    // Still linked, inside the period: nothing.
    rig.clock.advance(2_000);
    pass(&mut state, &[node]);
    rig.clock.advance(27_000);
    pass(&mut state, &[node]);
    assert_eq!(count(&rig), base + 1);
    // The period elapsed.
    rig.clock.advance(1_000);
    pass(&mut state, &[node]);
    assert_eq!(count(&rig), base + 2);
    // Link flap: immediately on the new link-up.
    pass(&mut state, &[]);
    pass(&mut state, &[node]);
    assert_eq!(count(&rig), base + 3);
    // Every send is the row's bytes.
    let sent = rig.fake.sent();
    assert!(sent.iter().all(|b| *b == sent[0]));
}

// Kills: a handoff answer without the ledger's message id (the CLI could not print it).
#[test]
fn every_answer_carries_the_message() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    let first = rig.poll(60, Some(&c)).unwrap();
    let repeat = rig.poll(60, Some(&c)).unwrap();
    let message = rig.row(60).unwrap().message;
    assert_eq!(
        (first.message, repeat.message),
        (message.clone(), message.clone())
    );
    rig.sql(|db| ledger::link(db, &message, REMOTE, T0).unwrap());
    assert_eq!(rig.poll(60, Some(&c)).unwrap().message, message);
}

/// A sender that mimics the coordinator: a second send of an id already in flight is answered
/// with the in-flight refusal at once. While `closed`, a send of a fresh id blocks (after
/// signalling `entered`) until [`Gated::open`].
struct Gated {
    in_flight: Mutex<HashSet<uuid::Uuid>>,
    sends: Mutex<Vec<uuid::Uuid>>,
    entered: Mutex<std::sync::mpsc::Sender<()>>,
    open: (Mutex<bool>, std::sync::Condvar),
}
const IN_FLIGHT_REFUSAL: &str = "too many launch requests in flight on the asking node";
impl Gated {
    fn new() -> (Self, std::sync::mpsc::Receiver<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let gated = Self {
            in_flight: Mutex::default(),
            sends: Mutex::default(),
            entered: Mutex::new(tx),
            open: (Mutex::new(false), std::sync::Condvar::new()),
        };
        (gated, rx)
    }
    fn open(&self) {
        *self.open.0.lock().unwrap() = true;
        self.open.1.notify_all();
    }
    fn sends(&self, id: u128) -> usize {
        let id = uuid::Uuid::from_u128(id);
        self.sends
            .lock()
            .unwrap()
            .iter()
            .filter(|s| **s == id)
            .count()
    }
}
impl RequestSender for Gated {
    fn send(
        &self,
        _: NodeUuid,
        bytes: &[u8],
        _: Duration,
        _: &Cancellation,
    ) -> Option<LaunchAnswer> {
        let id = decode(bytes).id;
        self.sends.lock().unwrap().push(id);
        if !self.in_flight.lock().unwrap().insert(id) {
            return Some(LaunchAnswer::Answered(LaunchState::Unavailable {
                reason: IN_FLIGHT_REFUSAL.into(),
            }));
        }
        let _ = self.entered.lock().unwrap().send(());
        let mut open = self.open.0.lock().unwrap();
        while !*open {
            open = self.open.1.wait(open).unwrap();
        }
        drop(open);
        self.in_flight.lock().unwrap().remove(&id);
        Some(LaunchAnswer::Answered(LaunchState::Running { pane: None }))
    }
}

impl Rig {
    fn requester_with<'a>(&'a self, sender: &'a dyn RequestSender) -> Requester<'a> {
        Requester {
            sender,
            ..self.requester()
        }
    }
}

/// Opens the gate when dropped, so a failing assertion cannot leave a blocked send holding the
/// thread scope.
struct OpenOnDrop<'a>(&'a Gated);
impl Drop for OpenOnDrop<'_> {
    fn drop(&mut self) {
        self.0.open();
    }
}

fn resend_once(rig: &Rig, sender: &Gated, state: &mut ResendState) {
    let linked: HashSet<NodeUuid> = [TARGET.parse().unwrap()].into();
    rig.requester_with(sender)
        .resend_pass(state, &linked, &Cancellation::default())
        .unwrap();
}

fn poll_with(rig: &Rig, sender: &Gated, n: u128) -> Result<Polled, ApiError> {
    rig.requester_with(sender).poll(
        TARGET.parse().unwrap(),
        request(n, AUTHOR),
        Some(&claim(AUTHOR)),
        false,
        &rig.budget(),
    )
}

// Kills: a re-send and a CLI poll sending the same request id at once, so the coordinator answers
// the second with the in-flight `Unavailable` and the CLI prints it; a re-send that waits for the
// CLI instead of skipping its row, or that records `last_send` for a skipped row.
#[test]
fn cli_poll_never_sees_the_resend_in_flight() {
    // The re-send is in flight; a CLI poll for the same id arrives.
    let rig = Rig::new();
    rig.poll(90, Some(&claim(AUTHOR))).unwrap();
    let (gated, entered) = Gated::new();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let mut state = ResendState::default();
    std::thread::scope(|scope| {
        let _open = OpenOnDrop(&gated);
        scope.spawn(|| resend_once(&rig, &gated, &mut state));
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        scope.spawn(|| done_tx.send(poll_with(&rig, &gated, 90)).unwrap());
        // Negative wait: the poll has not sent while the re-send holds the id.
        assert!(done_rx.recv_timeout(Duration::from_millis(200)).is_err());
        gated.open();
        let polled = done_rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                polled.answer,
                RemoteLaunchAnswer::Answered {
                    state: LaunchState::Running { .. }
                }
            ),
            "{:?}",
            polled.answer
        );
    });
    assert_eq!(gated.sends(90), 2);

    // The CLI poll is in flight; the re-send skips the row and leaves it due.
    let rig = Rig::new();
    rig.poll(91, Some(&claim(AUTHOR))).unwrap();
    let (gated, entered) = Gated::new();
    let mut state = ResendState::default();
    std::thread::scope(|scope| {
        let _open = OpenOnDrop(&gated);
        let poller = scope.spawn(|| poll_with(&rig, &gated, 91));
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        resend_once(&rig, &gated, &mut state);
        assert_eq!(gated.sends(91), 1);
        assert!(state.last_send.is_empty(), "{:?}", state.last_send);
        gated.open();
        poller.join().unwrap().unwrap();
    });
    resend_once(&rig, &gated, &mut state);
    assert_eq!(gated.sends(91), 2);
}

/// A budget on the wall-following clock, so waits that block in real time count against it.
fn wall_budget(rig: &Rig, ms: u64) -> CallBudget {
    rig.clock.wall.store(true, Ordering::SeqCst);
    CallBudget {
        deadline: MonoInstant(rig.clock.monotonic_now().0 + ms),
        cancellation: Cancellation::default(),
    }
}

/// A linked target that does not answer: the first send (the re-send) blocks until released, every
/// later send records `(when, wait)` on the rig's injected clock and times out at once, as
/// `SyncSender` does after its wait.
struct Slow {
    clock: Arc<TestClock>,
    entered: Mutex<std::sync::mpsc::Sender<()>>,
    first: Mutex<bool>,
    open: (Mutex<bool>, std::sync::Condvar),
    later: Mutex<Vec<(MonoInstant, Duration)>>,
}
impl Slow {
    fn new(clock: Arc<TestClock>) -> (Self, std::sync::mpsc::Receiver<()>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let slow = Self {
            clock,
            entered: Mutex::new(tx),
            first: Mutex::new(true),
            open: (Mutex::new(false), std::sync::Condvar::new()),
            later: Mutex::default(),
        };
        (slow, rx)
    }
    fn release(&self) {
        *self.open.0.lock().unwrap() = true;
        self.open.1.notify_all();
    }
}
impl RequestSender for Slow {
    fn send(
        &self,
        _: NodeUuid,
        _: &[u8],
        wait: Duration,
        _: &Cancellation,
    ) -> Option<LaunchAnswer> {
        let first = std::mem::replace(&mut *self.first.lock().unwrap(), false);
        if first {
            let _ = self.entered.lock().unwrap().send(());
            let mut open = self.open.0.lock().unwrap();
            while !*open {
                open = self.open.1.wait(open).unwrap();
            }
            return Some(LaunchAnswer::Answered(LaunchState::Running { pane: None }));
        }
        self.later
            .lock()
            .unwrap()
            .push((self.clock.monotonic_now(), wait));
        None
    }
}
struct ReleaseOnDrop<'a>(&'a Slow);
impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

// The poll never gets the slot (the re-send holds it): it settles as a timeout without sending.
#[test]
fn cli_poll_after_a_slow_resend_stays_inside_the_bound() {
    let bound = REPLICA_WAIT_MS + SETTLE_MARGIN_MS + 1_000;
    let n = 92_u128;
    let rig = Rig::new();
    rig.poll(n, Some(&claim(AUTHOR))).unwrap();
    let (slow, entered) = Slow::new(rig.clock.clone());
    let mut state = ResendState::default();
    let linked: HashSet<NodeUuid> = [TARGET.parse().unwrap()].into();
    std::thread::scope(|scope| {
        let _release = ReleaseOnDrop(&slow);
        scope.spawn(|| {
            rig.requester_with(&slow)
                .resend_pass(&mut state, &linked, &Cancellation::default())
                .unwrap();
        });
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let budget = wall_budget(&rig, bound);
        let start = std::time::Instant::now();
        let polled = rig
            .requester_with(&slow)
            .poll(
                TARGET.parse().unwrap(),
                request(n, AUTHOR),
                Some(&claim(AUTHOR)),
                false,
                &budget,
            )
            .unwrap();
        let elapsed = start.elapsed();
        assert!(
            matches!(polled.answer, RemoteLaunchAnswer::Timeout { .. }),
            "{n}: {:?}",
            polled.answer
        );
        assert!(elapsed < Duration::from_millis(bound), "{n}: {elapsed:?}");
        let later = slow.later.lock().unwrap().clone();
        assert!(
            later.is_empty(),
            "the poll sent without the slot: {later:?}"
        );
        slow.release();
    });
}

// Kills: a poll that reuses the pre-wait window for its send, so slot wait + send + replica wait
// overrun the transport bound and the CLI gets a transport error instead of "still pending".
// The clock is injected: the slot wait "takes" 4 s once the poll is known to be waiting, so no
// real-time margin enters the assertions.
#[test]
fn cli_poll_after_a_slow_resend_recomputes_its_window() {
    let n = 93_u128;
    let rig = Rig::new();
    rig.poll(n, Some(&claim(AUTHOR))).unwrap();
    let (slow, entered) = Slow::new(rig.clock.clone());
    let mut state = ResendState::default();
    let linked: HashSet<NodeUuid> = [TARGET.parse().unwrap()].into();
    let room = SEND_WAIT_MS;
    let bound = REPLICA_WAIT_MS + SETTLE_MARGIN_MS + room;
    let deadline = MonoInstant(rig.clock.monotonic_now().0 + bound);
    let budget = CallBudget {
        deadline,
        cancellation: Cancellation::default(),
    };
    let id = uuid::Uuid::from_u128(n).to_string();
    std::thread::scope(|scope| {
        let _release = ReleaseOnDrop(&slow);
        scope.spawn(|| {
            rig.requester_with(&slow)
                .resend_pass(&mut state, &linked, &Cancellation::default())
                .unwrap();
        });
        entered.recv_timeout(Duration::from_secs(10)).unwrap();
        let poll = scope.spawn(|| {
            rig.requester_with(&slow)
                .poll(
                    TARGET.parse().unwrap(),
                    request(n, AUTHOR),
                    Some(&claim(AUTHOR)),
                    false,
                    &budget,
                )
                .unwrap()
        });
        let gate = std::time::Instant::now();
        while !super::waiting_for(&id) {
            assert!(
                gate.elapsed() < Duration::from_secs(10),
                "the poll never entered the slot wait"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        rig.clock.advance(4_000);
        slow.release();
        let polled = poll.join().unwrap();
        assert!(
            matches!(polled.answer, RemoteLaunchAnswer::Timeout { .. }),
            "{:?}",
            polled.answer
        );
    });
    let later = slow.later.lock().unwrap().clone();
    let (at, wait) = *later.last().expect("the poll sent after the slot freed");
    assert!(
        at.0 + wait.as_millis() as u64 <= deadline.0 - REPLICA_WAIT_MS - SETTLE_MARGIN_MS,
        "send window {wait:?} at {at:?} overruns the bound"
    );
    assert_eq!(wait, Duration::from_millis(room - 4_000));
}

/// Inserts the target seat and its pair thread with the author, as replication and `open_pair`
/// would.
fn hold_pair(rig: &Rig, seat: &str) {
    rig.sql(|c| {
        c.pragma_update(None, "foreign_keys", "OFF").unwrap();
        c.execute(
            &format!(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) \
                 VALUES ('{seat}','{INSTANCE}','remote','native',1,0,'{TARGET}',0,'agent',1)"
            ),
            [],
        )
        .unwrap();
        let thread = crate::protocol::ids::direct_thread_id(&SeatId::new(AUTHOR), &SeatId::new(seat))
            .unwrap();
        c.execute(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) \
             VALUES (?1,?2,'topic','goal',0,0,0)",
            rusqlite::params![thread.as_str(), INSTANCE],
        )
        .unwrap();
    });
}

// Kills: a background re-send answering Launched{seat} for a seat this store does not hold
// withdrawing the sender's alert or the peer list line while the target is offline, and the
// in-thread warning not replacing the alert once the pair thread arrives.
#[test]
fn expired_launched_rerun_clears_its_alert() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    rig.poll(61, Some(&c)).unwrap();
    let request = uuid::Uuid::from_u128(61).to_string();
    let deadline = rig.row(61).unwrap().deadline_at.expect("an ACK deadline");
    // The target launched a seat whose pair thread is not held: the overdue alert stays pending.
    rig.sql(|c| assert!(ledger::launched(c, &request, "sZZZunknown1", T0 + 1).unwrap()));
    let mut db = rig.store.context().open_writer().unwrap();
    assert_eq!(ledger::due_pass(&mut db, deadline + 1, 16).unwrap(), 0);
    assert_eq!(ledger::alert_count(&db, AUTHOR).unwrap(), 1);
    drop(db);
    rig.clock.advance(OBLIGATION_TTL_MS as u64 + 1);
    let polled = rig.poll(61, Some(&c)).unwrap();
    assert_eq!(
        polled.answer,
        RemoteLaunchAnswer::HandoffLedger {
            outcome: HandoffLedgerOutcome::Expired {
                seat: Some("sZZZunknown1".into())
            }
        }
    );
    rig.sql(|db| assert_eq!(ledger::alert_count(db, AUTHOR).unwrap(), 0));
    assert!(rig.row(61).unwrap().alert_shown_at.is_some());
}

#[test]
fn background_launched_unreplicated_keeps_the_alert() {
    let rig = Rig::new();
    rig.poll(60, Some(&claim(AUTHOR))).unwrap();
    let request = uuid::Uuid::from_u128(60).to_string();
    let deadline = rig.row(60).unwrap().deadline_at.expect("an ACK deadline");
    // The background re-send: the target launched, but nothing replicated (`NotReplicated`).
    rig.fake.answer(launched("sZZZunknown1"));
    let linked: HashSet<NodeUuid> = [TARGET.parse().unwrap()].into();
    let mut state = ResendState::default();
    rig.requester()
        .resend_pass(&mut state, &linked, &Cancellation::default())
        .unwrap();
    let row = rig.row(60).unwrap();
    assert_eq!(
        (row.state, row.seat.as_deref()),
        (State::Launched, Some("sZZZunknown1"))
    );
    // The target is offline from here on; the deadline passes.
    let mut db = rig.store.context().open_writer().unwrap();
    assert_eq!(ledger::due_pass(&mut db, deadline + 1, 16).unwrap(), 0);
    assert_eq!(ledger::alert_count(&db, AUTHOR).unwrap(), 1);
    let rows = ledger::ledger_rows(&db).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].state.as_str(), rows[0].overdue),
        ("launched", true)
    );
    // Replication and the pair arrive: one in-thread warning replaces the alert.
    hold_pair(&rig, "sZZZunknown1");
    assert_eq!(ledger::due_pass(&mut db, deadline + 2, 16).unwrap(), 1);
    let warnings: i64 = db
        .query_row(
            "SELECT count(*) FROM messages WHERE event_key=?1",
            [format!("overdue:handoff:{request}")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 1);
    assert_eq!(ledger::alert_count(&db, AUTHOR).unwrap(), 0);
    assert!(ledger::ledger_rows(&db).unwrap().is_empty());
}

/// A sender that, when it sends request `poison`, installs a trigger failing every update of
/// that row on a side connection, as a store fault would (the store refuses a schema it did not
/// create at open, so the trigger lands after the writer is open).
struct Poisoning<'a> {
    inner: &'a Fake,
    db: std::path::PathBuf,
    poison: uuid::Uuid,
}
impl RequestSender for Poisoning<'_> {
    fn send(
        &self,
        node: NodeUuid,
        bytes: &[u8],
        wait: Duration,
        cancel: &Cancellation,
    ) -> Option<LaunchAnswer> {
        if decode(bytes).id == self.poison {
            let side = rusqlite::Connection::open(&self.db).unwrap();
            side.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS fail_one BEFORE UPDATE ON handoff_obligations \
                 WHEN OLD.request='{}' BEGIN SELECT RAISE(ABORT,'injected'); END",
                self.poison
            ))
            .unwrap();
        }
        self.inner.send(node, bytes, wait, cancel)
    }
}

// Kills: one row whose settlement errors stopping the re-send of the rows after it, and the
// failure going unlogged.
#[test]
fn resend_pass_skips_a_failing_row() {
    let rig = Rig::new();
    let c = claim(AUTHOR);
    for n in 70..=72 {
        rig.poll(n, Some(&c)).unwrap();
    }
    for _ in 0..3 {
        rig.fake.answer(launched("sZZZunknown1"));
    }
    let sender = Poisoning {
        inner: &rig.fake,
        db: rig.dir.join("store.db"),
        poison: uuid::Uuid::from_u128(71),
    };
    let linked: HashSet<NodeUuid> = [TARGET.parse().unwrap()].into();
    let mut state = ResendState::default();
    rig.requester_with(&sender)
        .resend_pass(&mut state, &linked, &Cancellation::default())
        .unwrap();
    rusqlite::Connection::open(rig.dir.join("store.db"))
        .unwrap()
        .execute_batch("DROP TRIGGER fail_one")
        .unwrap();
    assert_eq!(rig.row(70).unwrap().state, State::Launched);
    assert_eq!(rig.row(71).unwrap().state, State::Sent);
    assert_eq!(rig.row(72).unwrap().state, State::Launched);
    let lines = rig.lines.lock().unwrap().clone();
    let second = uuid::Uuid::from_u128(71).to_string();
    let named: Vec<_> = lines.iter().filter(|l| l.contains(&second[..8])).collect();
    assert_eq!(named.len(), 1, "{lines:?}");
    assert!(named[0].contains("re-send failed"), "{named:?}");
}
