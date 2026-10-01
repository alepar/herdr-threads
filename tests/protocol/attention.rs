use super::*;

fn token(
    invitation: Option<PublicationKey>,
    receipt: Option<PublicationKey>,
    warning: Option<PublicationKey>,
    episode: u64,
) -> AttentionToken {
    AttentionToken {
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
