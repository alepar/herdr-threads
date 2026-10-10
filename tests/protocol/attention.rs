use super::*;

fn token(
    invitation: Option<PublicationKey>,
    receipt: Option<PublicationKey>,
    warning: Option<PublicationKey>,
    episode: u64,
) -> AttentionToken {
    AttentionToken {
        lazy: None,
        invitation,
        receipt,
        warning,
        unavailability_episode: episode,
    }
}

// Kills: an equality comparison (an ACK lowering the pending maximum would
// re-emit), dropping any one component from `advanced_beyond`, or a mark that
// is replaced instead of joined (the lowered maximum would later hide nothing
// but make an old key look new again).
#[test]
fn only_a_new_publication_advances_and_the_mark_only_grows() {
    let mark = token(Some((10, 1)), Some((11, 0)), None, 0);
    // Receipt 11 ACKed: the current maximum falls back to an older receipt.
    let after_ack = token(Some((10, 1)), Some((9, 0)), None, 0);
    assert!(!after_ack.advanced_beyond(&mark));
    assert!(!mark.advanced_beyond(&mark));
    assert!(!AttentionToken::default().advanced_beyond(&mark));
    for newer in [
        token(Some((12, 1)), Some((9, 0)), None, 0),
        token(Some((10, 1)), Some((12, 0)), None, 0),
        token(Some((10, 1)), Some((9, 0)), Some((12, 3)), 0),
        // Same decision, later batch offset.
        token(Some((10, 2)), None, None, 0),
    ] {
        assert!(newer.advanced_beyond(&mark), "{newer:?}");
    }
    // The seat's own unavailability episode alone is not attention for its
    // own model (native-claude-demo-1 P3). Kills: counting
    // `unavailability_episode` in `advanced_beyond` (every host snapshot that
    // opened an episode re-emitted the full offer on the next tool call).
    assert!(!token(Some((10, 1)), Some((11, 0)), None, 1).advanced_beyond(&mark));
    assert!(!token(None, None, None, u64::MAX).advanced_beyond(&mark));
    let joined = mark.join(&after_ack);
    assert_eq!(joined, mark);
    let grown = mark.join(&token(None, None, Some((12, 3)), 2));
    assert_eq!(grown, token(Some((10, 1)), Some((11, 0)), Some((12, 3)), 2));
}

// Kills: a decoder that accepts another version, trailing garbage, or a
// non-canonical spelling (two spellings of one token would compare unequal in
// stored marks).
#[test]
fn token_is_versioned_and_canonical_on_the_wire() {
    let value = token(Some((10, 1)), None, Some((u64::MAX, 0)), 7);
    let raw = value.encode();
    assert_eq!(raw, format!("v1.10-1._.{}-0.7", u64::MAX));
    assert_eq!(AttentionToken::decode(&raw), Ok(value));
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(json, format!("\"{raw}\""));
    assert_eq!(
        serde_json::from_str::<AttentionToken>(&json).unwrap(),
        value
    );
    for bad in [
        "v2.10-1._._.7",
        "v1.10-1._._",
        "v1.10-1._._.7.8",
        "v1.010-1._._.7",
        "v1.10-1._._.-7",
        "v1.10+1._._.7",
        "v1.10-1.x._.7",
        "",
    ] {
        assert!(AttentionToken::decode(bad).is_err(), "{bad}");
    }
}

// Kills: a digest whose listed items exceed the ID bound, whose `has_more`
// disagrees with its count, or whose version is not checked.
#[test]
fn digest_bounds_are_explicit() {
    let item = |n: usize| AttentionRef {
        id: format!("id-{n}"),
        thread: ThreadId::new("t"),
        requirement: None,
    };
    let mut digest = AttentionDigest {
        lazy: None,
        version: DIGEST_VERSION,
        seat: SeatId::new("s"),
        token: AttentionToken::default(),
        invitations: AttentionClass {
            count: 6,
            items: (0..MAX_DIGEST_IDS).map(item).collect(),
            has_more: true,
            count_has_more: false,
        },
        receipts: AttentionClass::default(),
        warnings: AttentionClass::default(),
        unavailability_open: false,
        mod_channel_live: false,
    };
    assert_eq!(digest.validate(), Ok(()));
    assert!(!digest.is_empty());
    assert!(
        digest.summary().contains("invitations=6 [id-0@t"),
        "{}",
        digest.summary()
    );
    assert!(digest.summary().contains("+more"));
    digest.invitations.has_more = false;
    assert!(digest.validate().is_err());
    digest.invitations.has_more = true;
    digest.invitations.items.push(item(9));
    assert!(digest.validate().is_err());
    digest.invitations.items.pop();
    digest.version = 2;
    assert!(digest.validate().is_err());
}

// Kills: a flag that is always serialized (breaking older decoders), one that
// does not round-trip, or an old payload without the key failing to decode.
#[test]
fn mod_channel_live_absent_when_false_and_round_trips_when_true() {
    let mut digest = AttentionDigest {
        lazy: None,
        version: DIGEST_VERSION,
        seat: SeatId::new("s"),
        token: AttentionToken::default(),
        invitations: AttentionClass::default(),
        receipts: AttentionClass::default(),
        warnings: AttentionClass::default(),
        unavailability_open: false,
        mod_channel_live: false,
    };
    let quiet = serde_json::to_value(&digest).unwrap();
    assert!(quiet.get("mod_channel_live").is_none(), "{quiet}");
    let old: AttentionDigest = serde_json::from_value(quiet).unwrap();
    assert!(!old.mod_channel_live);
    digest.mod_channel_live = true;
    let live = serde_json::to_value(&digest).unwrap();
    assert_eq!(live["mod_channel_live"], serde_json::json!(true));
    let back: AttentionDigest = serde_json::from_value(live).unwrap();
    assert_eq!(back, digest);
}

// Lazy hook delivery: a lazy key is a fourth publication component, spelled
// v2 only when present (v1 marks and readers stay valid), canonical, and it
// advances and joins like the others. Kills: lazy left out of
// `advanced_beyond` or `join`, a v2 spelling for a token without a lazy key
// (two spellings of one token), and a decoder that drops the lazy key.
#[test]
fn lazy_key_is_a_v2_component_that_advances_and_joins() {
    let plain = token(Some((10, 1)), None, None, 3);
    assert!(plain.encode().starts_with("v1."));
    let lazy = AttentionToken {
        lazy: Some((12, 40)),
        ..plain
    };
    let raw = lazy.encode();
    assert_eq!(raw, "v2.10-1._._.12-40.3");
    assert_eq!(AttentionToken::decode(&raw), Ok(lazy));
    assert_eq!(
        serde_json::from_str::<AttentionToken>(&format!("\"{raw}\"")).unwrap(),
        lazy
    );
    assert!(lazy.advanced_beyond(&plain));
    assert!(!plain.advanced_beyond(&lazy));
    assert!(
        AttentionToken {
            lazy: Some((13, 2)),
            ..plain
        }
        .advanced_beyond(&lazy)
    );
    // Completing the newest lazy row lowers the current key: not new.
    assert!(
        !AttentionToken {
            lazy: Some((11, 9)),
            ..plain
        }
        .advanced_beyond(&lazy)
    );
    assert_eq!(plain.join(&lazy), lazy);
    for bad in [
        "v2.10-1._._._.3",
        "v2.10-1._._.12-40",
        "v3.10-1._._.12-40.3",
    ] {
        assert!(AttentionToken::decode(bad).is_err(), "{bad}");
    }
}

// Kills: a lazy key without a lazy class, an unbounded lazy class, a lazy
// class ignored by `is_empty`, or the summary dropping it.
#[test]
fn digest_lazy_class_is_bounded_and_counted() {
    let mut digest = AttentionDigest {
        version: DIGEST_VERSION,
        seat: SeatId::new("s"),
        token: AttentionToken::default(),
        invitations: AttentionClass::default(),
        receipts: AttentionClass::default(),
        warnings: AttentionClass::default(),
        unavailability_open: false,
        lazy: None,
        mod_channel_live: false,
    };
    assert!(digest.validate().is_ok() && digest.is_empty());
    assert!(!digest.summary().contains("lazy"));
    digest.token.lazy = Some((5, 1));
    assert!(digest.validate().is_err(), "key without rows");
    digest.lazy = Some(AttentionClass {
        count: 1,
        items: vec![AttentionRef {
            id: "m1".into(),
            thread: ThreadId::new("t"),
            requirement: None,
        }],
        has_more: false,
        count_has_more: false,
    });
    assert!(digest.validate().is_ok());
    assert!(!digest.is_empty());
    assert!(digest.summary().ends_with("; lazy=1 [m1@t]"));
    digest.lazy.as_mut().unwrap().count = 0;
    assert!(digest.validate().is_err(), "listed beyond its count");
    let json = serde_json::to_value(AttentionDigest {
        lazy: None,
        token: AttentionToken::default(),
        ..digest.clone()
    })
    .unwrap();
    assert!(
        json.get("lazy").is_none(),
        "absent on the wire when not asked"
    );
}
