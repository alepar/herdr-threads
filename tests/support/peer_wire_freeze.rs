//! Deterministic generator of the golden peer-wire fixtures
//! (`tests/fixtures/peer_wire/<version>/`).
//!
//! `scripts/freeze-peer-wire` overlays this file onto a checkout of a release
//! and runs it there as a `harness = false` test target, so the fixtures are
//! what that release's own codec emits for deterministic inputs (seeded keys,
//! fixed HLC, fixed hello nonce and boot); they are not a recorded live
//! session. It must therefore compile against every release that has a
//! fixture directory and uses only public or `test-support` APIs present at
//! the oldest one (v2.0.4): `NodeIdentity::from_seed`, `UnsignedEnvelope::seal`,
//! `WireEnvelope::from`, `PeerFrame::encode`, `FactRegistry::builtins`,
//! `HlcStamp::from_parts`, `Deps`, `EntityKey`. Never use an API added after
//! the oldest fixture release. It also compiles at HEAD as a module of the
//! `combined` test target, so it cannot rot.
//!
//! Inputs are fixed: node `a` (seed `[0xa1; 32]`) authors every entry, node
//! `b` (seed `[0xb2; 32]`) only appears as a peer; entry `i` carries HLC
//! `1_760_000_000_000 + i`; payloads are `serde_json::json!` literals that
//! must decode into the kind's typed payload at generation time.

use std::path::Path;

use herdr_threads::node::NodeIdentity;
use herdr_threads::sync::LogKey;
use herdr_threads::sync::envelope::{Deps, Envelope, UnsignedEnvelope, WireEnvelope};
use herdr_threads::sync::facts::{
    FactKind, FactRegistry, NodeGenesis, NodeRetag, invitation, membership, message, receipt, seat,
    thread,
};
use herdr_threads::sync::frame::{
    BASE_CAP, BranchWant, CloseReason, FORK_CAP, ForkRef, HaveLog, PeerFrame, PeerHello,
    PeerLimits, Role, SeenAge, TipRef,
};
use herdr_threads::sync::hlc::HlcStamp;
use serde_json::{Value, json};
use uuid::Uuid;

/// The label of the `message.sent` variant without `author_role`, the shape
/// older releases signed.
pub const NO_AUTHOR_ROLE: &str = "message.sent/no-author-role";

/// Every builtin `(kind, major)`, in chain order after the genesis. A rename
/// or a dropped kind fails to compile here; a new kind fails
/// `generator_kind_table_covers_builtins`.
pub const KINDS: &[(&str, u32)] = &[
    (
        <NodeGenesis as FactKind>::NAME,
        <NodeGenesis as FactKind>::MAJOR,
    ),
    (
        <NodeRetag as FactKind>::NAME,
        <NodeRetag as FactKind>::MAJOR,
    ),
    (
        <seat::SeatCreated as FactKind>::NAME,
        <seat::SeatCreated as FactKind>::MAJOR,
    ),
    (
        <seat::SeatRetired as FactKind>::NAME,
        <seat::SeatRetired as FactKind>::MAJOR,
    ),
    (
        <thread::ThreadCreated as FactKind>::NAME,
        <thread::ThreadCreated as FactKind>::MAJOR,
    ),
    (
        <thread::ThreadRegister as FactKind>::NAME,
        <thread::ThreadRegister as FactKind>::MAJOR,
    ),
    (
        <thread::ThreadDirect as FactKind>::NAME,
        <thread::ThreadDirect as FactKind>::MAJOR,
    ),
    (
        <invitation::InvitationCreated as FactKind>::NAME,
        <invitation::InvitationCreated as FactKind>::MAJOR,
    ),
    (
        <invitation::InvitationAnswered as FactKind>::NAME,
        <invitation::InvitationAnswered as FactKind>::MAJOR,
    ),
    (
        <membership::MembershipJoined as FactKind>::NAME,
        <membership::MembershipJoined as FactKind>::MAJOR,
    ),
    (
        <membership::MembershipLeft as FactKind>::NAME,
        <membership::MembershipLeft as FactKind>::MAJOR,
    ),
    (
        <message::MessageSent as FactKind>::NAME,
        <message::MessageSent as FactKind>::MAJOR,
    ),
    (
        <receipt::ReceiptSettled as FactKind>::NAME,
        <receipt::ReceiptSettled as FactKind>::MAJOR,
    ),
];

const BASE_MS: u64 = 1_760_000_000_000;
const SEAT_A: &str = "sAbc12345678";
const SEAT_B: &str = "sXyz98765432";
const SEAT_C: &str = "sQ7ak3Fq9a2B";
const THREAD: &str = "tAbc12345678";
/// `direct_thread_id(SEAT_A, SEAT_B)`, spelled out so the generator needs no
/// protocol helper that an older release may lack.
const DIRECT_THREAD: &str = "d5hPrtBxfUW";
const MESSAGE: &str = "mAbc12345678";
const INVITATION: &str = "iAbc12345678";

fn author() -> NodeIdentity {
    NodeIdentity::from_seed([0xa1; 32])
}

fn peer() -> NodeIdentity {
    NodeIdentity::from_seed([0xb2; 32])
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").expect("write to String");
        out
    })
}

/// Seal one entry of kind `K` at `counter` on the author's incarnation-1 log.
/// The literal must decode into `K::Payload`; `creates`/`needs` come from the
/// typed payload so the signed keys agree with it.
fn seal<K: FactKind>(
    identity: &NodeIdentity,
    counter: u64,
    prev_hash: [u8; 32],
    payload: &Value,
) -> Envelope {
    let bytes = serde_json::to_vec(payload).expect("payload serializes");
    let typed: K::Payload = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("{} payload does not decode: {e}", K::NAME));
    let mut needs = K::needs(&typed);
    needs.sort();
    needs.dedup();
    let unsigned = UnsignedEnvelope {
        log: LogKey {
            node: identity.uuid(),
            incarnation: 1,
        },
        counter,
        prev_hash,
        hlc: HlcStamp::from_parts(BASE_MS + counter, 0),
        kind: K::NAME.to_owned(),
        kind_version: K::MAJOR,
        deps: Deps::empty(),
        creates: K::creates(&typed),
        needs,
        payload: bytes,
    };
    unsigned
        .validate_shape()
        .unwrap_or_else(|e| panic!("{} shape: {e}", K::NAME));
    unsigned.seal(identity)
}

fn genesis_payload(identity: &NodeIdentity) -> Value {
    json!({
        "pubkey": hex(&identity.public_key()),
        "tag": identity.tag(),
        "tag_k": 0,
        "incarnation": 1,
        "predecessor": null,
        "host_label": "freeze-a",
    })
}

fn message_payload(with_author_role: bool) -> Value {
    if with_author_role {
        json!({
            "message": MESSAGE,
            "thread": THREAD,
            "author": SEAT_A,
            "body": "freeze: ack request",
            "author_role": "agent",
            "priority": 0,
            "require_ack": true,
            "relays_user": true,
            "user_intent": "request",
            "sent_at": BASE_MS + 100,
            "duration_ms": 60_000,
            "recipients": [{
                "seat": SEAT_B,
                "receipt": "ack",
                "deadline_at": BASE_MS + 60_100,
            }],
        })
    } else {
        json!({
            "message": "mAbc12345679",
            "thread": THREAD,
            "author": SEAT_A,
            "body": "freeze: plain message",
            "priority": 1,
            "require_ack": false,
            "relays_user": false,
            "user_intent": null,
            "sent_at": BASE_MS + 101,
            "duration_ms": 0,
            "recipients": [{ "seat": SEAT_B, "receipt": "none", "deadline_at": null }],
        })
    }
}

/// The conflicting counter-3 entry of the fork frames: same predecessor as
/// the chain's counter 3, a different seat.
fn conflicting(identity: &NodeIdentity, prev_hash: [u8; 32]) -> Envelope {
    seal::<seat::SeatCreated>(
        identity,
        3,
        prev_hash,
        &json!({ "seat": SEAT_C, "kind": "human", "display_name": null }),
    )
}

/// The author's chain: the genesis, one entry of every other builtin kind
/// and the `message.sent` variant without `author_role`, each labelled.
pub fn envelopes() -> Vec<(String, Envelope)> {
    let a = author();
    let mut chain: Vec<(String, Envelope)> = Vec::new();
    let mut push = |label: &str, build: &dyn Fn(u64, [u8; 32]) -> Envelope| {
        let counter = chain.len() as u64 + 1;
        let prev = chain.last().map_or([0u8; 32], |(_, e)| e.hash);
        let envelope = build(counter, prev);
        chain.push((label.to_owned(), envelope));
    };
    push(NodeGenesis::NAME, &|c, p| {
        seal::<NodeGenesis>(&a, c, p, &genesis_payload(&a))
    });
    push(NodeRetag::NAME, &|c, p| {
        seal::<NodeRetag>(&a, c, p, &json!({ "k": 1, "tag": a.tag() }))
    });
    push(seat::SeatCreated::NAME, &|c, p| {
        seal::<seat::SeatCreated>(
            &a,
            c,
            p,
            &json!({ "seat": SEAT_A, "kind": "agent", "display_name": null }),
        )
    });
    push(seat::SeatRetired::NAME, &|c, p| {
        seal::<seat::SeatRetired>(
            &a,
            c,
            p,
            &json!({ "seat": SEAT_A, "reason": "closed", "retired_at": BASE_MS + 4 }),
        )
    });
    push(thread::ThreadCreated::NAME, &|c, p| {
        seal::<thread::ThreadCreated>(
            &a,
            c,
            p,
            &json!({
                "thread": THREAD,
                "topic": "freeze topic",
                "goal": "",
                "name": null,
                "creator": SEAT_A,
            }),
        )
    });
    push(thread::ThreadRegister::NAME, &|c, p| {
        seal::<thread::ThreadRegister>(
            &a,
            c,
            p,
            &json!({
                "thread": THREAD,
                "field": "topic",
                "value": "frozen topic",
                "actor": SEAT_A,
            }),
        )
    });
    push(thread::ThreadDirect::NAME, &|c, p| {
        seal::<thread::ThreadDirect>(
            &a,
            c,
            p,
            &json!({
                "thread": DIRECT_THREAD,
                "seat_lo": SEAT_A,
                "seat_hi": SEAT_B,
                "creator": SEAT_A,
            }),
        )
    });
    push(invitation::InvitationCreated::NAME, &|c, p| {
        seal::<invitation::InvitationCreated>(
            &a,
            c,
            p,
            &json!({
                "invitation": INVITATION,
                "thread": THREAD,
                "inviter": SEAT_A,
                "inviter_kind": "seat",
                "invitee": SEAT_B,
                "invitee_n": 0,
                "created_at": BASE_MS + 8,
                "duration_ms": 60_000,
                "deadline_at": BASE_MS + 60_008,
                "actor_label": null,
            }),
        )
    });
    push(invitation::InvitationAnswered::NAME, &|c, p| {
        seal::<invitation::InvitationAnswered>(
            &a,
            c,
            p,
            &json!({
                "invitation": INVITATION,
                "invitee": SEAT_B,
                "outcome": "accepted",
                "reason": null,
                "answered_at": BASE_MS + 9,
                "n": 1,
            }),
        )
    });
    push(membership::MembershipJoined::NAME, &|c, p| {
        seal::<membership::MembershipJoined>(
            &a,
            c,
            p,
            &json!({ "thread": THREAD, "seat": SEAT_B, "n": 1, "at": BASE_MS + 10 }),
        )
    });
    push(membership::MembershipLeft::NAME, &|c, p| {
        seal::<membership::MembershipLeft>(
            &a,
            c,
            p,
            &json!({ "thread": THREAD, "seat": SEAT_B, "n": 2, "at": BASE_MS + 11 }),
        )
    });
    push(message::MessageSent::NAME, &|c, p| {
        seal::<message::MessageSent>(&a, c, p, &message_payload(true))
    });
    push(receipt::ReceiptSettled::NAME, &|c, p| {
        seal::<receipt::ReceiptSettled>(
            &a,
            c,
            p,
            &json!({
                "message": MESSAGE,
                "seat": SEAT_B,
                "how": "acked",
                "settled_at": BASE_MS + 13,
                "batch": { "id": "freeze-op", "last_in_thread": true },
            }),
        )
    });
    push(NO_AUTHOR_ROLE, &|c, p| {
        seal::<message::MessageSent>(&a, c, p, &message_payload(false))
    });
    chain
}

fn hello(identity: &NodeIdentity, role: Role, boot: u128, nonce_byte: &str) -> PeerFrame {
    PeerFrame::Hello(PeerHello {
        envelope: 1,
        node: identity.uuid(),
        tag: identity.tag(),
        pubkey: hex(&identity.public_key()),
        boot: Uuid::from_u128(boot),
        caps: vec![BASE_CAP.to_owned(), FORK_CAP.to_owned()],
        limits: PeerLimits {
            max_frame: 1 << 20,
            batch_entries: 256,
            batch_bytes: 256 * 1024,
            credit: 1 << 20,
        },
        nonce: nonce_byte.repeat(32),
        role,
    })
}

/// The session's frames in order, each encoded by the release's own codec,
/// tagged with its direction (`a>b` or `b>a`).
pub fn session(envelopes: &[(String, Envelope)]) -> Vec<(&'static str, Vec<u8>)> {
    let (a, b) = (author(), peer());
    let node_a = a.uuid();
    let chain: Vec<&Envelope> = envelopes.iter().map(|(_, e)| e).collect();
    let tip3 = chain[2];
    let rival = conflicting(&a, tip3.prev_hash);
    let head = chain.last().expect("non-empty chain");
    // `Into` resolves to `WireEnvelope` in a release whose entries are bare
    // wire envelopes and to `WireItem` where they are items.
    let wire = |list: &[&Envelope]| -> Vec<_> { list.iter().copied().map(Into::into).collect() };
    let frames: Vec<(&'static str, PeerFrame)> = vec![
        ("a>b", hello(&a, Role::Dialer, 0xa1, "aa")),
        ("b>a", hello(&b, Role::Dialed, 0xb2, "bb")),
        (
            "a>b",
            PeerFrame::Auth {
                sig: hex(&a.sign(&[0x11; 32])),
            },
        ),
        (
            "b>a",
            PeerFrame::Auth {
                sig: hex(&b.sign(&[0x22; 32])),
            },
        ),
        ("b>a", PeerFrame::Accept),
        (
            "a>b",
            PeerFrame::Have {
                full: true,
                ask: false,
                logs: vec![HaveLog {
                    node: node_a,
                    inc: 1,
                    counter: head.counter,
                    head: hex(&head.hash),
                    frozen_at: None,
                }],
                forks: vec![],
            },
        ),
        (
            "b>a",
            PeerFrame::Have {
                full: true,
                ask: true,
                logs: vec![],
                forks: vec![],
            },
        ),
        (
            "b>a",
            PeerFrame::Want {
                node: node_a,
                inc: 1,
                from: 1,
            },
        ),
        (
            "a>b",
            PeerFrame::Entries {
                node: node_a,
                inc: 1,
                entries: wire(&chain),
            },
        ),
        ("b>a", PeerFrame::Credit { bytes: 1 << 20 }),
        ("a>b", PeerFrame::Ping { n: 1 }),
        ("b>a", PeerFrame::Pong { n: 1 }),
        (
            "a>b",
            PeerFrame::Seen {
                ages: vec![SeenAge {
                    node: node_a,
                    age_ms: 1500,
                }],
            },
        ),
        (
            "b>a",
            PeerFrame::Unwant {
                node: node_a,
                inc: 1,
            },
        ),
        (
            "a>b",
            PeerFrame::Have {
                full: false,
                ask: false,
                logs: vec![HaveLog {
                    node: node_a,
                    inc: 1,
                    counter: tip3.counter,
                    head: hex(&tip3.hash),
                    frozen_at: Some(3),
                }],
                forks: vec![ForkRef {
                    node: node_a,
                    inc: 1,
                    held: TipRef {
                        counter: tip3.counter,
                        hash: hex(&tip3.hash),
                    },
                    conflicting: TipRef {
                        counter: rival.counter,
                        hash: hex(&rival.hash),
                    },
                    frozen_at: 3,
                }],
            },
        ),
        (
            "b>a",
            PeerFrame::BranchWant(BranchWant {
                node: node_a,
                inc: 1,
                tip: TipRef {
                    counter: rival.counter,
                    hash: hex(&rival.hash),
                },
                from: 3,
            }),
        ),
        (
            "a>b",
            PeerFrame::BranchEntries {
                node: node_a,
                inc: 1,
                tip_hash: hex(&rival.hash),
                entries: wire(&[&rival]),
                more: false,
            },
        ),
        (
            "a>b",
            PeerFrame::BranchNone {
                node: node_a,
                inc: 1,
                tip_hash: hex(&rival.hash),
            },
        ),
        (
            "a>b",
            PeerFrame::Close {
                reason: CloseReason::Shutdown,
                detail: Some("freeze".to_owned()),
            },
        ),
    ];
    frames
        .into_iter()
        .map(|(dir, frame)| (dir, frame.encode().expect("frame encodes")))
        .collect()
}

/// Write `session.jsonl`, `envelopes.jsonl` and `kinds.txt` into `out`.
/// Lines are built by concatenation so each frame and wire envelope keeps the
/// exact text the codec emitted.
pub fn write_fixture(out: &Path) {
    assert!(
        FactRegistry::builtins().len() >= KINDS.len(),
        "the registry lost a kind the generator knows"
    );
    std::fs::create_dir_all(out).expect("create the output directory");
    let envelopes = envelopes();
    let mut session_text = String::new();
    for (dir, frame) in session(&envelopes) {
        session_text.push_str(&format!(
            "{{\"dir\":\"{dir}\",\"frame\":{}}}\n",
            std::str::from_utf8(&frame).expect("frames are UTF-8 JSON")
        ));
    }
    let mut envelope_text = String::new();
    for (label, envelope) in &envelopes {
        let wire = serde_json::to_string(&WireEnvelope::from(envelope)).expect("wire serializes");
        envelope_text.push_str(&format!(
            "{{\"label\":\"{label}\",\"canonical\":\"{}\",\"wire\":{wire}}}\n",
            hex(&envelope.canonical_bytes())
        ));
    }
    let kinds: String = KINDS
        .iter()
        .map(|(name, major)| format!("- {name} v{major}\n"))
        .collect();
    std::fs::write(out.join("session.jsonl"), session_text).expect("write session.jsonl");
    std::fs::write(out.join("envelopes.jsonl"), envelope_text).expect("write envelopes.jsonl");
    std::fs::write(out.join("kinds.txt"), kinds).expect("write kinds.txt");
}

/// Entry point when the script builds this file as a `harness = false` test
/// target in a release checkout; dead when it is a module of `combined`.
#[allow(dead_code)]
fn main() {
    let out = std::env::var("PEER_WIRE_FREEZE_OUT").expect("PEER_WIRE_FREEZE_OUT is required");
    write_fixture(Path::new(&out));
}
