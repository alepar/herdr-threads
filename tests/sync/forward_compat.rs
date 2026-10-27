//! Forward compatibility of the peer wire, both ways (design section 7):
//!
//! - what the current code emits toward a peer that did not advertise
//!   `envelope.opaque.v1` decodes with the frozen 2.1.1 decoder
//!   (`tests/support/legacy_wire_v211.rs`) with equal fields, and never
//!   carries an opaque item;
//! - the current decoder tolerates what a newer peer may add (extra fields,
//!   unknown frame tags, close reasons and capabilities);
//! - the current encoders reproduce the golden 2.1.1 bytes.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use uuid::Uuid;

use herdr_threads::node::NodeIdentity;
use herdr_threads::sync::coordinator::gate_batch;
use herdr_threads::sync::envelope::{Envelope, StoredEntry, WireEnvelope};
use herdr_threads::sync::frame::{
    BASE_CAP, BranchWant, CloseReason, FORK_CAP, ForkRef, HaveLog, OPAQUE_CAP, PeerFrame,
    PeerHello, Role, SeenAge, TipRef, WireItem,
};
use herdr_threads::sync::link::{ENVELOPE_GENERATION, LocalHello};
use herdr_threads::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};

use super::{legacy_wire_v211 as legacy, peer_wire_freeze};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn signer() -> NodeIdentity {
    NodeIdentity::from_seed([0xa1; 32])
}

/// The hello a coordinator builds (`BASE_CAP`, `FORK_CAP`, `OPAQUE_CAP`),
/// framed the way the link frames it.
fn hello(role: Role) -> PeerHello {
    let identity = Arc::new(signer());
    let mut local = LocalHello::new(identity.clone(), Uuid::from_u128(0xb007));
    local.caps.push(FORK_CAP.to_owned());
    local.caps.push(OPAQUE_CAP.to_owned());
    assert_eq!(local.caps, [BASE_CAP, FORK_CAP, OPAQUE_CAP]);
    PeerHello {
        envelope: ENVELOPE_GENERATION,
        node: identity.uuid(),
        tag: identity.tag(),
        pubkey: hex(&identity.public_key()),
        boot: local.boot,
        caps: local.caps.clone(),
        limits: local.limits.clone(),
        nonce: hex(&[0x07; 32]),
        role,
    }
}

fn items(batch: &[StoredEntry]) -> Vec<WireItem> {
    batch.iter().map(WireItem::from).collect()
}

/// One log: `[known 1, known 2, opaque 3, known 4]` on top of the golden
/// chain's first two entries.
fn mixed_batch() -> Vec<StoredEntry> {
    let chain = peer_wire_freeze::envelopes();
    let (k1, k2) = (chain[0].1.clone(), chain[1].1.clone());
    let who = signer();
    let opaque = synthetic_entry(&who, &synthetic_unsigned(&who, 1, 3, k2.hash));
    let k4 = synthetic_unsigned(&who, 1, 4, opaque.hash()).seal(&who);
    vec![
        StoredEntry::from(k1),
        StoredEntry::from(k2),
        StoredEntry::from(opaque),
        StoredEntry::from(k4),
    ]
}

/// A fork branch `[known, opaque]`: the second entry must be withheld.
fn fork_branch() -> Vec<StoredEntry> {
    let chain = peer_wire_freeze::envelopes();
    let first: Envelope = chain[2].1.clone();
    let who = signer();
    let opaque = synthetic_entry(&who, &synthetic_unsigned(&who, 1, 4, first.hash));
    vec![StoredEntry::from(first), StoredEntry::from(opaque)]
}

fn corpus() -> Vec<(String, PeerFrame)> {
    let who = signer();
    let node = who.uuid();
    let chain = peer_wire_freeze::envelopes();
    let tip = &chain[2].1;
    let head = &chain[chain.len() - 1].1;
    let mut frames: Vec<(String, PeerFrame)> = vec![
        ("hello/dialer".into(), PeerFrame::Hello(hello(Role::Dialer))),
        ("hello/dialed".into(), PeerFrame::Hello(hello(Role::Dialed))),
        (
            "auth".into(),
            PeerFrame::Auth {
                sig: hex(&who.sign(&[0x11; 32])),
            },
        ),
        ("accept".into(), PeerFrame::Accept),
    ];
    for reason in [
        CloseReason::EnvelopeUnsupported,
        CloseReason::NodeCollision,
        CloseReason::SelfLink,
        CloseReason::AuthFailed,
        CloseReason::Busy,
        CloseReason::Shutdown,
        CloseReason::ProtocolError,
    ] {
        for detail in [None, Some("why".to_owned())] {
            frames.push((
                format!("close/{reason:?}/{}", detail.is_some()),
                PeerFrame::Close { reason, detail },
            ));
        }
    }
    let log = |counter: u64, hash: &[u8; 32], frozen_at| HaveLog {
        node,
        inc: 1,
        counter,
        head: hex(hash),
        frozen_at,
    };
    frames.push((
        "have/full".into(),
        PeerFrame::Have {
            full: true,
            ask: true,
            logs: vec![log(head.counter, &head.hash, None)],
            forks: vec![],
        },
    ));
    frames.push((
        "have/delta+forks".into(),
        PeerFrame::Have {
            full: false,
            ask: false,
            logs: vec![log(tip.counter, &tip.hash, Some(3))],
            forks: vec![ForkRef {
                node,
                inc: 1,
                held: TipRef {
                    counter: tip.counter,
                    hash: hex(&tip.hash),
                },
                conflicting: TipRef {
                    counter: tip.counter,
                    hash: hex(&head.hash),
                },
                frozen_at: 3,
            }],
        },
    ));
    frames.push((
        "want".into(),
        PeerFrame::Want {
            node,
            inc: 1,
            from: 1,
        },
    ));
    frames.push(("unwant".into(), PeerFrame::Unwant { node, inc: 1 }));

    // Entries: the opaque gate cuts the batch for a peer without the cap.
    let cut = gate_batch(mixed_batch(), false);
    assert!(cut.withheld, "the gate cut the batch");
    assert_eq!(cut.items.len(), 2, "only the known prefix survives");
    frames.push((
        "entries/gated".into(),
        PeerFrame::Entries {
            node,
            inc: 1,
            entries: items(&cut.items),
        },
    ));
    // Fork evidence with an opaque entry present: withheld.
    let branch_cut = gate_batch(fork_branch(), false);
    assert!(branch_cut.withheld, "the opaque branch entry is withheld");
    assert_eq!(branch_cut.items.len(), 1);
    frames.push((
        "branch_entries/gated".into(),
        PeerFrame::BranchEntries {
            node,
            inc: 1,
            tip_hash: hex(&head.hash),
            entries: items(&branch_cut.items),
            more: true,
        },
    ));
    // A branch that starts with an opaque entry has an empty prefix: the
    // serve path answers `branch_none`, never an empty `branch_entries`.
    let mut opaque_first = fork_branch();
    opaque_first.reverse();
    let empty_cut = gate_batch(opaque_first, false);
    assert!(empty_cut.items.is_empty() && empty_cut.withheld);

    frames.push((
        "branch_want".into(),
        PeerFrame::BranchWant(BranchWant {
            node,
            inc: 1,
            tip: TipRef {
                counter: head.counter,
                hash: hex(&head.hash),
            },
            from: 3,
        }),
    ));
    frames.push((
        "branch_none".into(),
        PeerFrame::BranchNone {
            node,
            inc: 1,
            tip_hash: hex(&head.hash),
        },
    ));
    frames.push(("credit".into(), PeerFrame::Credit { bytes: 1 << 20 }));
    frames.push(("ping".into(), PeerFrame::Ping { n: 1 }));
    frames.push(("pong".into(), PeerFrame::Pong { n: 1 }));
    frames.push((
        "seen".into(),
        PeerFrame::Seen {
            ages: vec![SeenAge { node, age_ms: 1500 }],
        },
    ));
    frames
}

#[test]
fn emitted_frames_decode_with_2_1_1_shadow() {
    let frames = corpus();
    let tags: std::collections::BTreeSet<String> = frames
        .iter()
        .map(|(_, f)| {
            let v: Value = serde_json::from_slice(&f.encode().unwrap()).unwrap();
            v["t"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        tags.len(),
        15,
        "every frame kind is in the corpus: {tags:?}"
    );
    for (name, frame) in &frames {
        let bytes = frame.encode().unwrap();
        let shadow = legacy::decode(&bytes)
            .unwrap_or_else(|e| panic!("{name}: the 2.1.1 decoder refuses it: {e}"));
        assert!(
            !matches!(shadow, legacy::PeerFrame::Unknown),
            "{name}: decodes to Unknown in 2.1.1"
        );
        // Equal known fields: what the 2.1.1 types keep, re-encoded, is the
        // whole frame (a field the current code adds would be dropped and
        // show here), in the same bytes.
        let back = shadow.encode().unwrap();
        let (ours, theirs): (Value, Value) = (
            serde_json::from_slice(&bytes).unwrap(),
            serde_json::from_slice(&back).unwrap(),
        );
        assert_eq!(ours, theirs, "{name}: fields differ after the 2.1.1 decode");
        assert_eq!(back, bytes, "{name}: bytes differ after the 2.1.1 decode");
    }
}

// Kills: a node_info frame (ht-xg88, sent only to a peer advertising
// node.info.v1) that a 2.1.1 peer would refuse instead of ignoring, were one
// ever sent to it.
#[test]
fn node_info_is_unknown_to_2_1_1() {
    let frame = PeerFrame::NodeInfo(herdr_threads::sync::frame::NodeInfo {
        version: "2.4.0".into(),
    });
    let shadow = legacy::decode(&frame.encode().unwrap()).expect("2.1.1 decodes it");
    assert!(matches!(shadow, legacy::PeerFrame::Unknown));
}

#[test]
fn emitted_hello_envelope_is_one() {
    let mut seen = 0;
    for (name, frame) in corpus() {
        if let PeerFrame::Hello(hello) = &frame {
            assert_eq!(hello.envelope, 1, "{name}");
            let v: Value = serde_json::from_slice(&frame.encode().unwrap()).unwrap();
            assert_eq!(v["envelope"], json!(1), "{name}: the encoded field");
            seen += 1;
        }
    }
    assert_eq!(seen, 2, "both hellos were checked");
}

#[test]
fn opaque_items_never_reach_a_non_cap_peer() {
    for (name, frame) in corpus() {
        let text = String::from_utf8(frame.encode().unwrap()).unwrap();
        assert!(!text.contains("\"opaque\""), "{name}: {text}");
    }
    // The check bites: the same batch for a peer with the capability does
    // carry the opaque item.
    let open = gate_batch(mixed_batch(), true);
    assert_eq!(open.items.len(), 4);
    let frame = PeerFrame::Entries {
        node: signer().uuid(),
        inc: 1,
        entries: items(&open.items),
    };
    let text = String::from_utf8(frame.encode().unwrap()).unwrap();
    assert!(text.contains("\"opaque\""), "control frame: {text}");
}

#[derive(Deserialize)]
struct SessionLine {
    dir: String,
    frame: Box<RawValue>,
}

#[derive(Deserialize)]
struct EnvelopeLine {
    wire: Box<RawValue>,
}

fn fixture_lines(version: &str, file: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/peer_wire")
        .join(version)
        .join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `value` with one extra field in each of its objects in turn (one result
/// per object, at every depth, including inside arrays).
fn with_one_extra_field(value: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    match value {
        Value::Object(map) => {
            let mut top = map.clone();
            top.insert("zz_future".into(), json!({"n": 1}));
            out.push(Value::Object(top));
            for (key, child) in map {
                for mutated in with_one_extra_field(child) {
                    let mut copy = map.clone();
                    copy.insert(key.clone(), mutated);
                    out.push(Value::Object(copy));
                }
            }
        }
        Value::Array(list) => {
            for (i, child) in list.iter().enumerate() {
                for mutated in with_one_extra_field(child) {
                    let mut copy = list.clone();
                    copy[i] = mutated;
                    out.push(Value::Array(copy));
                }
            }
        }
        _ => {}
    }
    out
}

#[test]
fn mutated_fixtures_decode_with_current_decoder() {
    let mut mutated = 0;
    let mut nested = std::collections::BTreeMap::new();
    for version in ["2.1.1", "2.0.4"] {
        for line in fixture_lines(version, "session.jsonl") {
            let parsed: SessionLine = serde_json::from_str(&line).unwrap();
            let frame: Value = serde_json::from_str(parsed.frame.get()).unwrap();
            let tag = frame["t"].as_str().unwrap().to_owned();
            let variants = with_one_extra_field(&frame);
            let deepest = nested.entry(tag.clone()).or_insert(0);
            *deepest = (*deepest).max(variants.len());
            for variant in variants {
                let bytes = serde_json::to_vec(&variant).unwrap();
                let decoded = PeerFrame::decode(&bytes)
                    .unwrap_or_else(|e| panic!("{version} {tag}: {e}: {variant}"));
                assert!(
                    !matches!(decoded, PeerFrame::Unknown),
                    "{version} {tag}: decodes to Unknown: {variant}"
                );
                mutated += 1;
            }
        }
    }
    assert!(mutated > 100, "mutations ran: {mutated}");
    // Nested objects are mutated too: a hello (limits), a have (a log) and
    // the entry frames (each item) have more than their top-level object.
    for tag in ["hello", "have", "entries", "branch_entries"] {
        assert!(nested[tag] >= 2, "{tag}: {} variants at most", nested[tag]);
    }

    // A frame tag from the future.
    let future = PeerFrame::decode(br#"{"t":"future_frame","x":[1,2]}"#).unwrap();
    assert!(matches!(future, PeerFrame::Unknown));
    // A close reason from the future.
    match PeerFrame::decode(br#"{"t":"close","reason":"future_reason"}"#).unwrap() {
        PeerFrame::Close { reason, .. } => assert_eq!(reason, CloseReason::Unknown),
        other => panic!("not a close: {other:?}"),
    }
    // A capability from the future stays in the hello's list.
    let hello_line = fixture_lines("2.1.1", "session.jsonl")
        .into_iter()
        .map(|l| serde_json::from_str::<SessionLine>(&l).unwrap())
        .find_map(|l| {
            let v: Value = serde_json::from_str(l.frame.get()).unwrap();
            (v["t"] == "hello").then_some(v)
        })
        .expect("a hello in the session");
    let mut with_cap = hello_line.clone();
    with_cap["caps"]
        .as_array_mut()
        .unwrap()
        .push(json!("future.v9"));
    match PeerFrame::decode(&serde_json::to_vec(&with_cap).unwrap()).unwrap() {
        PeerFrame::Hello(h) => {
            assert!(h.caps.iter().any(|c| c == "future.v9"), "{:?}", h.caps);
            assert!(h.caps.iter().any(|c| c == BASE_CAP));
        }
        other => panic!("not a hello: {other:?}"),
    }
}

#[test]
fn current_encoders_reproduce_2_1_1_fixtures() {
    let envelopes = peer_wire_freeze::envelopes();
    let session = peer_wire_freeze::session(&envelopes);
    let golden = fixture_lines("2.1.1", "session.jsonl");
    assert_eq!(session.len(), golden.len(), "frame count");
    for (i, ((dir, bytes), line)) in session.iter().zip(&golden).enumerate() {
        let parsed: SessionLine = serde_json::from_str(line).unwrap();
        assert_eq!(*dir, parsed.dir, "frame {i}: direction");
        assert_eq!(
            std::str::from_utf8(bytes).unwrap(),
            parsed.frame.get(),
            "frame {i} ({dir}): bytes differ from 2.1.1 session.jsonl"
        );
    }
    let golden = fixture_lines("2.1.1", "envelopes.jsonl");
    assert_eq!(envelopes.len(), golden.len(), "envelope count");
    for ((label, envelope), line) in envelopes.iter().zip(&golden) {
        let parsed: EnvelopeLine = serde_json::from_str(line).unwrap();
        let wire = serde_json::to_string(&WireEnvelope::from(envelope)).unwrap();
        assert_eq!(wire, parsed.wire.get(), "{label}: WireEnvelope bytes");
        let item = serde_json::to_string(&WireItem::from(envelope)).unwrap();
        assert_eq!(item, parsed.wire.get(), "{label}: WireItem::Known bytes");
    }
}
