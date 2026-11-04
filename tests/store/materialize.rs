//! `MaterializeInput`, `local_only`, the ApplyContext helper and
//! `require_author` (fact-kinds design sections 2 and 17).

use super::*;

use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::node::{NodeIdentity, tag_for};
use crate::protocol::time::{Clock, MonoInstant, UtcMillis};
use crate::store::apply::ApplyContext;
use crate::store::connection::StoreContext;
use crate::store::journal as jr;
use crate::store::node_directory::{self, NodeDirectoryRow};
use crate::sync::LogKey;
use crate::sync::facts::{FactError, FactKind, FactRegistry, with_input};
use crate::sync::handoff_block::SenderBlock;
use crate::sync::hlc::HlcStamp;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

#[derive(Serialize, Deserialize)]
struct Put {
    id: String,
}

/// Creates `e:<id>`, so the node's own log holds a settlement-shaped key.
struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.materialize.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        EntityKey::new(format!("e:{}", p.id)).ok()
    }
    fn needs(_: &Put) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        Ok(())
    }
}

struct Store {
    path: PathBuf,
    ctx: StoreContext,
    conn: Connection,
    node: NodeUuid,
}

impl Store {
    fn new() -> Self {
        let path = crate::test_support::scratch::temp_path(format!(
            "ht-materialize-{}.db",
            uuid::Uuid::new_v4()
        ));
        let identity = Arc::new(NodeIdentity::from_seed([90; 32]));
        let node = identity.uuid();
        let ctx = StoreContext::new(path.clone(), Arc::new(FixedClock))
            .with_node_seed(crate::test_support::node_seed_for(node));
        ctx.seed_incarnations(1);
        let mut conn = ctx.open_writer().unwrap();
        let registry = FactRegistry::builtins_plus(|b| b.with::<PutKind>());
        jr::open(&ctx, &mut conn, identity, "host", registry).unwrap();
        Self {
            path,
            ctx,
            conn,
            node,
        }
    }

    fn run<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<T, ApiError>,
    ) -> T {
        self.ctx
            .execute_decision(&mut self.conn, |_| Ok(()), |tx, d, ()| f(tx, d))
            .unwrap()
    }

    fn decision_seq(&self) -> u64 {
        let seq: i64 = self
            .conn
            .query_row("SELECT decision_seq FROM local_node", [], |r| r.get(0))
            .unwrap();
        seq as u64
    }

    /// Authors one `Put` into this node's own log (creates `e:k`).
    fn author_put(&mut self) {
        let ctx = &self.ctx;
        ctx.execute_decision(
            &mut self.conn,
            |_| Ok(()),
            |tx, d, ()| jr::author::<PutKind>(tx, ctx, d, &Put { id: "k".into() }).map(|_| ()),
        )
        .unwrap();
    }

    /// A remote seat homed at `home`.
    fn remote_seat(&self, id: &str, home: NodeUuid) {
        self.conn
            .execute(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) \
                 VALUES(?1,(SELECT id FROM local_node),'remote','native',1,0,?2,0,'agent',1)",
                params![id, home.to_string()],
            )
            .unwrap();
    }

    /// A local seat (home is this node).
    fn local_seat(&self, id: &str) {
        self.conn
            .execute_batch(&format!(
                "INSERT OR IGNORE INTO herdr_hosts(node_id,endpoint,created_at) \
                 VALUES ((SELECT id FROM local_node),'local',0);\
                 INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) \
                 VALUES('{id}',(SELECT id FROM local_node),'unresolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));"
            ))
            .unwrap();
    }

    fn set_directory(&self, node: NodeUuid, k: u32) {
        node_directory::upsert(
            &self.conn,
            &NodeDirectoryRow {
                node_uuid: node,
                tag: tag_for(node, k),
                tag_k: k,
                host_label: None,
                first_seen_at: 0,
                tag_hlc: None,
            },
        )
        .unwrap();
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = self.path.clone().into_os_string();
            name.push(suffix);
            let _ = std::fs::remove_file(name);
        }
    }
}

fn other(n: u8) -> NodeUuid {
    NodeIdentity::from_seed([n; 32]).uuid()
}

fn seat(id: &str) -> SeatId {
    SeatId::new(id)
}

/// A remote-origin input from `origin`, with the suppression query never true.
fn remote_input(origin: NodeUuid, decision: DecisionInstant) -> MaterializeInput<'static> {
    MaterializeInput {
        decision,
        decision_seq: 1,
        origin_node: origin,
        origin_ms: 1,
        local_origin: false,
        own_history: false,
        provenance: "replicated_from:x",
        origin_hlc: crate::sync::hlc::HlcStamp::from_parts(1, 0),
        origin_counter: 1,
        ingest_seq: Some(1),
        suppress: &|_, _| Ok(false),
        scope: Default::default(),
    }
}

fn check(s: &mut Store, origin: NodeUuid, kind: &str, rule: AuthorRule) -> Result<(), ApiError> {
    s.run(|tx, d| Ok(require_author(tx, &remote_input(origin, d), kind, &rule)))
}

#[test]
fn local_only_one_decision_seq_self_origin_never_suppresses() {
    let mut s = Store::new();
    s.author_put();
    let key = EntityKey::new("e:k").unwrap();
    let before = s.decision_seq();
    let node = s.node;
    let (seq, origin, local, prov, ms, suppressed, real) = s.run(|tx, d| {
        let m = local_only(tx, d)?;
        let cx = ApplyContext::new(
            LogKey {
                node,
                incarnation: 1,
            },
            1,
            1,
            HlcStamp::from_parts(1, 0),
            d,
            1,
            node,
        );
        Ok((
            m.decision_seq,
            m.origin_node,
            m.local_origin,
            m.provenance.to_owned(),
            m.origin_ms,
            (m.suppress)(tx, &key)?,
            cx.delivery_suppressed(tx, &key)?,
        ))
    });
    assert_eq!(seq, before + 1, "exactly one decision_seq allocated");
    assert_eq!(s.decision_seq(), before + 1);
    assert_eq!(origin, node);
    assert!(local);
    assert_eq!(prov, "local");
    assert_eq!(ms, 1_700_000_000_000);
    assert!(real, "the key is settled in this node's journal");
    assert!(!suppressed, "local_only never suppresses");
}

#[test]
fn apply_context_helper_copies_every_field() {
    let mut s = Store::new();
    s.author_put();
    let key = EntityKey::new("e:k").unwrap();
    let missing = EntityKey::new("e:none").unwrap();
    let local = s.node;
    let remote = other(91);
    s.run(|tx, d| {
        let cx = ApplyContext::new(
            LogKey {
                node: remote,
                incarnation: 4,
            },
            9,
            3,
            HlcStamp::from_parts(1234, 5),
            d,
            77,
            local,
        );
        let out = with_input(&cx, |m| {
            assert_eq!(m.decision, d);
            assert_eq!(m.decision_seq, 77);
            assert_eq!(m.origin_node, remote);
            assert_eq!(m.origin_ms, 1234);
            assert!(!m.local_origin);
            assert_eq!(m.provenance, format!("replicated_from:{remote}"));
            assert!((m.suppress)(tx, &key)?, "delegates to delivery_suppressed");
            assert!(!(m.suppress)(tx, &missing)?);
            Ok(7)
        });
        assert_eq!(out, Ok(7));

        let own = ApplyContext::new(
            LogKey {
                node: local,
                incarnation: 0,
            },
            1,
            1,
            HlcStamp::from_parts(5, 0),
            d,
            2,
            local,
        );
        with_input(&own, |m| {
            assert!(m.local_origin);
            assert_eq!(m.origin_node, local);
            assert_eq!(m.provenance, "local_journal");
            Ok(())
        })
        .unwrap();

        // Materializer errors classify through FactError::from_api.
        let rejected = with_input(&own, |_| -> Result<(), ApiError> {
            Err(ApiError::new(ErrorCode::InvalidRequest, "x"))
        });
        assert!(matches!(rejected, Err(FactError::Rejected(_))));
        let store = with_input(&own, |_| -> Result<(), ApiError> {
            Err(ApiError::new(ErrorCode::StoreBusy, "x"))
        });
        assert!(matches!(store, Err(FactError::Store(_))));
        Ok(())
    });
}

#[test]
fn require_author_seat_home() {
    let mut s = Store::new();
    let (b, c) = (other(92), other(93));
    s.remote_seat("sBseat", b);
    assert!(
        check(
            &mut s,
            b,
            "membership.joined",
            AuthorRule::SeatHome(seat("sBseat"))
        )
        .is_ok()
    );
    let err = check(
        &mut s,
        c,
        "membership.joined",
        AuthorRule::SeatHome(seat("sBseat")),
    )
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);
    // An unknown seat has no authorized author.
    assert!(
        check(
            &mut s,
            b,
            "membership.joined",
            AuthorRule::SeatHome(seat("sNone"))
        )
        .is_err()
    );
}

#[test]
fn require_author_either_seat_home() {
    let mut s = Store::new();
    let (b, c, d) = (other(94), other(95), other(96));
    s.remote_seat("sLo", b);
    s.remote_seat("sHi", c);
    let rule = || AuthorRule::EitherSeatHome(seat("sLo"), seat("sHi"));
    assert!(check(&mut s, b, "thread.direct", rule()).is_ok());
    assert!(check(&mut s, c, "thread.direct", rule()).is_ok());
    assert!(check(&mut s, d, "thread.direct", rule()).is_err());
}

fn own_tag_seat(origin: NodeUuid, k: u32) -> SeatId {
    SeatId::new(format!("s{}abcd1234", tag_for(origin, k)))
}

#[test]
fn require_author_own_tag_current_and_pre_retag() {
    let mut s = Store::new();
    let b = other(97);
    s.set_directory(b, 2);
    for k in 0..=2 {
        assert!(
            check(
                &mut s,
                b,
                "seat.created",
                AuthorRule::OwnTag(own_tag_seat(b, k))
            )
            .is_ok(),
            "k={k}"
        );
    }
}

#[test]
fn require_author_own_tag_foreign_fails() {
    let mut s = Store::new();
    let (b, c) = (other(98), other(99));
    s.set_directory(b, 2);
    s.set_directory(c, 0);
    // C's tag, authored by B.
    assert!(
        check(
            &mut s,
            b,
            "seat.created",
            AuthorRule::OwnTag(own_tag_seat(c, 0))
        )
        .is_err()
    );
    // A retag counter above the origin's current one.
    assert!(
        check(
            &mut s,
            b,
            "seat.created",
            AuthorRule::OwnTag(own_tag_seat(b, 3))
        )
        .is_err()
    );
    // An origin with no directory row, and an ID that is not a public seat ID.
    let unknown = other(100);
    assert!(
        check(
            &mut s,
            unknown,
            "seat.created",
            AuthorRule::OwnTag(own_tag_seat(unknown, 0))
        )
        .is_err()
    );
    assert!(
        check(
            &mut s,
            b,
            "seat.created",
            AuthorRule::OwnTag(seat("s-legacy"))
        )
        .is_err()
    );
}

#[test]
fn require_author_any() {
    let mut s = Store::new();
    assert!(check(&mut s, other(101), "thread.register", AuthorRule::Any).is_ok());
}

#[test]
fn require_author_local_only_passes() {
    let mut s = Store::new();
    s.local_seat("sLocal");
    // A seat homed elsewhere would fail for a remote origin; a local_only
    // input (self origin, local_origin) passes every rule.
    s.remote_seat("sFar", other(102));
    let results = s.run(|tx, d| {
        let m = local_only(tx, d)?;
        Ok(vec![
            require_author(tx, &m, "k", &AuthorRule::SeatHome(seat("sFar"))),
            require_author(tx, &m, "k", &AuthorRule::OwnTag(seat("sFar"))),
            require_author(tx, &m, "k", &AuthorRule::SeatHome(seat("sLocal"))),
        ])
    });
    assert!(results.iter().all(Result::is_ok), "{results:?}");
}

#[test]
fn require_author_error_text() {
    let mut s = Store::new();
    let (b, c) = (other(103), other(104));
    s.remote_seat("sBseat", b);
    let err = check(
        &mut s,
        c,
        "receipt.settled",
        AuthorRule::SeatHome(seat("sBseat")),
    )
    .unwrap_err();
    let uuid8 = &c.to_string()[..8];
    assert_eq!(
        err.detail,
        format!("origin {uuid8} is not the authorized author of receipt.settled for sBseat")
    );
    assert!(matches!(FactError::from_api(err), FactError::Rejected(_)));
}

// ---- AuthorRule::HandoffRecipientHome (spec 4.2) -----------------------------

/// A block of sender `node` for author `author`, signed by `signer`.
fn handoff_block(node: NodeUuid, author: &str, signer: &NodeIdentity) -> SenderBlock {
    let mut block = SenderBlock {
        v: 1,
        request: uuid::Uuid::from_u128(7),
        sender_node: Some(node),
        author: Some(seat(author)),
        author_role: None,
        message: Some(crate::protocol::ids::MessageId::new("mHandoff0001")),
        body: "do the thing".into(),
        priority: 0,
        require_ack: true,
        requested_at: 1_700_000_000_000,
        ack_after_ms: Some(300_000),
        sig: String::new(),
    };
    block.sign(signer).unwrap();
    block
}

fn handoff_rule(block: SenderBlock, recipient: &str) -> AuthorRule {
    AuthorRule::HandoffRecipientHome {
        block: Box::new(block),
        recipient: seat(recipient),
    }
}

/// Author `sA` homed on this node, recipient `sR` homed on the returned node. No
/// `peer_requests`, ledger or directory row exists: the rule reads the fact and the
/// replicated identity only.
fn handoff_store() -> (Store, NodeUuid) {
    let mut s = Store::new();
    let target = other(120);
    s.local_seat("sA");
    s.remote_seat("sR", target);
    assert_eq!(
        s.run(|tx, _| Ok(tx
            .query_row("SELECT count(*) FROM handoff_obligations", [], |r| r
                .get::<_, i64>(0))
            .unwrap())),
        0
    );
    (s, target)
}

#[test]
fn handoff_rule_reads_only_fact_and_identity() {
    let (mut s, target) = handoff_store();
    let me = NodeIdentity::from_seed([90; 32]);
    let node = s.node;
    let good = || handoff_rule(handoff_block(node, "sA", &me), "sR");
    assert!(check(&mut s, target, "message.sent", good()).is_ok());

    // Origin is not the recipient's home (the sender node itself, a third node).
    for origin in [node, other(121)] {
        let err = check(&mut s, origin, "message.sent", good());
        assert_eq!(err.unwrap_err().code, ErrorCode::InvalidRequest, "{origin}");
    }
    // The author is homed elsewhere than the block's sender node.
    s.remote_seat("sAway", other(122));
    let away = handoff_rule(handoff_block(node, "sAway", &me), "sR");
    assert!(check(&mut s, target, "message.sent", away).is_err());
    // Signed by another node's key.
    let forged = handoff_rule(
        handoff_block(node, "sA", &NodeIdentity::from_seed([91; 32])),
        "sR",
    );
    assert!(check(&mut s, target, "message.sent", forged).is_err());
    // A tampered signature, a block with no sender node.
    let mut tampered = handoff_block(node, "sA", &me);
    tampered.body.push('!');
    assert!(check(&mut s, target, "message.sent", handoff_rule(tampered, "sR")).is_err());
    let mut anonymous = handoff_block(node, "sA", &me);
    anonymous.sender_node = None;
    assert!(
        check(
            &mut s,
            target,
            "message.sent",
            handoff_rule(anonymous, "sR")
        )
        .is_err()
    );
    // An unknown recipient has no home.
    assert!(
        check(
            &mut s,
            target,
            "message.sent",
            handoff_rule(handoff_block(node, "sA", &me), "sNone")
        )
        .is_err()
    );
}

#[test]
fn handoff_rule_error_text_names_author_and_recipient() {
    let (mut s, _target) = handoff_store();
    let me = NodeIdentity::from_seed([90; 32]);
    let node = s.node;
    let c = other(123);
    let err = check(
        &mut s,
        c,
        "message.sent",
        handoff_rule(handoff_block(node, "sA", &me), "sR"),
    )
    .unwrap_err();
    assert_eq!(
        err.detail,
        format!(
            "origin {} is not the authorized author of message.sent for sA/sR",
            &c.to_string()[..8]
        )
    );
}

// Kills: an unordered or single-row key read (two readers could disagree), a rule that checks
// only the newest key, and one that accepts a key no log of the sender node holds.
#[test]
fn node_keys_newest_incarnation_first_and_any_verifies() {
    let (mut s, target) = handoff_store();
    let me = NodeIdentity::from_seed([90; 32]);
    let later = NodeIdentity::from_seed([91; 32]);
    let node = s.node;
    // The next incarnation of this node, under another key (the schema allows it).
    let current: i64 = s
        .conn
        .query_row(
            "SELECT incarnation FROM sync_logs WHERE node_uuid=?1",
            [node.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(current < i64::from(u32::MAX));
    s.conn
        .execute(
            "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
             head_hash,applied_through,state,first_seen_at) \
             VALUES (?1,?3,1,?2,zeroblob(32),0,zeroblob(32),0,'active',0)",
            rusqlite::params![node.to_string(), later.public_key().to_vec(), current + 1],
        )
        .unwrap();
    let keys = s.run(|tx, _| node_keys(tx, node).map_err(store_error));
    assert_eq!(keys, vec![later.public_key(), me.public_key()]);
    // A node this store holds no log of has no key.
    assert!(
        s.run(|tx, _| node_keys(tx, other(124)).map_err(store_error))
            .is_empty()
    );
    for signer in [&me, &later] {
        let rule = handoff_rule(handoff_block(node, "sA", signer), "sR");
        assert!(check(&mut s, target, "message.sent", rule).is_ok());
    }
    let stranger = NodeIdentity::from_seed([92; 32]);
    let rule = handoff_rule(handoff_block(node, "sA", &stranger), "sR");
    assert!(check(&mut s, target, "message.sent", rule).is_err());
}
