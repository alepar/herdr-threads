use herdr_threads::protocol::{
    commands::Command,
    ids::{ExecutionId, MessageId, SeatId},
    pagination::{
        CURSOR_FROM_OLDER_DAEMON, Cursor, CursorDirection, CursorScope, DeadlineCursor,
        DeadlineHighWaters, DeadlineWalks, DeliveryRowsCursor, DeliveryRowsPosition,
        InboxBatchV2BodyPosition, InboxBatchV2CursorState as State, InboxBatchV2Source as Source,
        MAX_CURSOR_BYTES, PageRequest, WarningsCursor,
    },
    priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
    time::UtcMillis,
};
#[test]
fn lazy_inbox_v2_codec_is_rejected_by_every_legacy_page_route() {
    use serde_json::json;
    let raw = base_v2().encode("i").unwrap();
    let args = [
        (
            "directory",
            json!({"membership":null,"membership_filter":"default","topic_contains":null}),
        ),
        ("picker_directory", json!({})),
        ("seats", json!({})),
        ("seat_inspect", json!({"seat":"s"})),
        ("inbox", json!({"seat":"s"})),
        ("inbox_batch", json!({"seat":"s"})),
        ("warnings", json!({"seat":"s"})),
        ("active_warnings", json!({"thread":"t"})),
        ("thread", json!({"thread":"t"})),
        ("history", json!({"thread":"t","initial":null})),
        ("participants", json!({"thread":"t"})),
        ("recipients", json!({"message":"m"})),
        ("delivery_inspect", json!({"message":"m"})),
        ("pending_receipts", json!({"seat":"s","thread":null})),
        ("local_intents", json!({})),
        (
            "search",
            json!({"literal":"body","thread":null,"max_candidates":10}),
        ),
        ("diagnostics", json!({"seat":"s","thread":null})),
        ("retirement_jobs", json!({})),
    ];
    for (kind, mut args) in args {
        args["page"] = json!({"cursor":raw,"limit":1,"max_bytes":4096});
        let command: Command = serde_json::from_value(json!({"kind":kind,"args":args})).unwrap();
        assert!(command.validate().is_err(), "{kind} must reject ib2");
    }
    let old = Cursor {
        instance: "i".into(),
        scope: CursorScope::InboxBatch,
        scope_key: "s".into(),
        filter_digest: "filter".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 0,
        high_water_ordinal: 3,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    };
    for raw in [old.encode().unwrap(), old.encode_legacy().unwrap()] {
        assert!(Cursor::decode(&raw).is_ok());
        assert!(State::decode(&raw).is_err());
        let command:Command=serde_json::from_value(json!({"kind":"inbox_batch_v2","args":{"seat":"s","page":PageRequest {cursor:Some(raw),..Default::default()}}})).unwrap();
        assert!(command.validate().is_err());
    }
}

#[test]
fn invitation_decision_context_is_additive_and_old_omitted_bytes_stay_exact() {
    use herdr_threads::protocol::results::InboxBatchV2Item;
    let old = r#"{"kind":"invitation","thread":"t","topic_data":"review","invitation":"inv","required_service":null}"#;
    let decoded: InboxBatchV2Item = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&decoded).unwrap(), old);
    let mut complete: serde_json::Value = serde_json::from_str(old).unwrap();
    complete["goal_data"] = "Decide whether this review belongs to your remit".into();
    complete["topic_revision"] = 7.into();
    let decoded: InboxBatchV2Item = serde_json::from_value(complete.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        complete,
        "canonical goal/revision survive the actual V2 codec"
    );
}

// ---- format 2 ----

fn walks(i: i64, u: Option<u64>, p: i64, m: i64, next: bool) -> DeadlineWalks {
    DeadlineWalks {
        invitation_after: i,
        unread_after: u,
        physical_after: p,
        manifest_after: m,
        next_manifest: next,
    }
}

fn base_v2() -> State {
    State {
        seat: SeatId::new("s"),
        binding_generation: Some(1),
        execution: Some(ExecutionId::new("execution")),
        publication_decision_high_water: 50,
        last: None,
        rows: DeliveryRowsCursor {
            after: None,
            done: false,
        },
        deadline: DeadlineCursor {
            high: DeadlineHighWaters {
                invitation: 9,
                unread: Some(8),
                physical: 7,
                manifest: 6,
            },
            window_start: walks(1, Some(2), 3, 4, false),
            window_end: None,
            done: false,
        },
        warnings: WarningsCursor {
            physical_after: 2,
            manifest_after: -1,
            offered: 3,
            done: false,
        },
        body: None,
    }
}

fn key(
    rank: SourceRank,
    priority: u8,
    deadline: Option<i64>,
    seq: u64,
    ordinal: u64,
) -> DeliveryOrderKey {
    DeliveryOrderKey {
        priority: DeliveryPriority::new(priority).unwrap(),
        deadline: deadline.map(UtcMillis),
        decision_seq: seq,
        rank,
        ordinal,
    }
}

fn triple(k: &DeliveryOrderKey) -> DeliveryRowsPosition {
    DeliveryRowsPosition {
        priority: k.priority,
        decision_seq: k.decision_seq,
        ordinal: k.ordinal,
    }
}

/// Deterministic generator over every class/position/done-flag combination.
fn generated_states() -> Vec<State> {
    let mut out = Vec::new();
    let mut lcg = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |modulus: u64| {
        lcg = lcg
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (lcg >> 33) % modulus
    };
    let lasts = |n: &mut dyn FnMut(u64) -> u64| -> Vec<Option<DeliveryOrderKey>> {
        let mut v = vec![None];
        for rank in [
            SourceRank::Invitation,
            SourceRank::Warning,
            SourceRank::Receipt,
            SourceRank::DeliveryRow,
        ] {
            v.push(Some(key(rank, n(10) as u8, None, n(50), n(1000))));
            v.push(Some(key(
                rank,
                n(10) as u8,
                Some(n(1_000_000) as i64 - 500),
                n(50),
                u64::MAX,
            )));
        }
        v
    };
    for bound in [false, true] {
        for identity in [false, true] {
            for last in lasts(&mut next) {
                // rows.after: None, equal to last (elision candidate), different
                let mut afters = vec![
                    None,
                    Some(DeliveryRowsPosition {
                        priority: DeliveryPriority::DEFAULT,
                        decision_seq: 3,
                        ordinal: 9,
                    }),
                ];
                if let Some(k) = &last {
                    afters.push(Some(triple(k)));
                }
                for after in afters {
                    for (rows_done, deadline_done, warnings_done) in [
                        (false, false, false),
                        (true, false, false),
                        (false, true, false),
                        (false, false, true),
                        (true, true, true),
                    ] {
                        for (end, start_nm, end_nm) in [
                            (false, false, false),
                            (true, false, false),
                            (true, true, false),
                            (true, false, true),
                            (true, true, true),
                            (false, true, false),
                        ] {
                            let mut s = base_v2();
                            if !identity {
                                s.binding_generation = None;
                                s.execution = None;
                            }
                            if !bound {
                                s.deadline.high.unread = None;
                                s.deadline.window_start.unread_after = None;
                            }
                            s.last = last;
                            s.rows = DeliveryRowsCursor {
                                after,
                                done: rows_done,
                            };
                            s.deadline.done = deadline_done;
                            s.warnings.done = warnings_done;
                            s.deadline.window_start.next_manifest = start_nm;
                            s.deadline.window_start.invitation_after = next(4) as i64;
                            if end {
                                s.deadline.window_end = Some(walks(
                                    s.deadline.window_start.invitation_after + next(5) as i64,
                                    bound.then(|| 2 + next(7)),
                                    3 + next(5) as i64,
                                    4 + next(3) as i64,
                                    end_nm,
                                ));
                            }
                            out.push(s);
                        }
                    }
                }
            }
        }
    }
    // bodies per allowed source, with the class not done
    for source in [Source::Lazy, Source::Receipts, Source::Unread] {
        let mut s = base_v2();
        s.body = Some((
            source,
            InboxBatchV2BodyPosition {
                message: MessageId::new("m"),
                offset: 2,
                body_len: 10,
            },
        ));
        out.push(s.clone());
        s.last = Some(key(SourceRank::DeliveryRow, 1, None, 4, 5));
        s.rows.after = s.last.as_ref().map(triple);
        out.push(s);
    }
    // values near the i64 limit
    let mut s = base_v2();
    let m = i64::MAX;
    s.publication_decision_high_water = m as u64;
    s.deadline.high = DeadlineHighWaters {
        invitation: m,
        unread: Some(m as u64),
        physical: m,
        manifest: m,
    };
    s.deadline.window_start = walks(m - 2, Some(m as u64 - 2), m - 2, m - 2, true);
    s.deadline.window_end = Some(walks(m, Some(m as u64), m, m, true));
    s.warnings = WarningsCursor {
        physical_after: m,
        manifest_after: m,
        offered: m,
        done: false,
    };
    s.last = Some(key(
        SourceRank::Receipt,
        9,
        Some(i64::MIN),
        m as u64,
        u64::MAX,
    ));
    s.rows.after = Some(DeliveryRowsPosition {
        priority: DeliveryPriority::new(9).unwrap(),
        decision_seq: m as u64,
        ordinal: u64::MAX,
    });
    out.push(s);
    out
}

#[test]
fn cursor_v2_round_trips_every_combination() {
    let states = generated_states();
    assert!(
        states.len() > 1000,
        "generator covers the combinations: {}",
        states.len()
    );
    for s in &states {
        let raw = s
            .encode("instance")
            .unwrap_or_else(|e| panic!("{e}: {s:?}"));
        assert!(raw.starts_with("ib2:") && raw.len() <= MAX_CURSOR_BYTES);
        assert_eq!(&State::decode_for(&raw, "instance", &s.seat).unwrap(), s);
        assert_eq!(&State::decode(&raw).unwrap(), s);
    }
    // worst case: 128-byte ids, huge values, window end, body
    let mut w = states.last().unwrap().clone();
    w.seat = SeatId::new("s".repeat(128));
    w.execution = Some(ExecutionId::new("e".repeat(128)));
    w.binding_generation = Some(i64::MAX as u64);
    w.body = Some((
        Source::Unread,
        InboxBatchV2BodyPosition {
            message: MessageId::new("m".repeat(128)),
            offset: 1,
            body_len: u64::MAX,
        },
    ));
    let raw = w.encode("instance").unwrap();
    assert!(
        raw.len() <= MAX_CURSOR_BYTES,
        "worst case {} bytes",
        raw.len()
    );
    assert_eq!(State::decode_for(&raw, "instance", &w.seat).unwrap(), w);
}

#[test]
fn cursor_v2_elides_the_row_position_after_a_delivery_row() {
    let last = key(SourceRank::DeliveryRow, 3, None, 11, 22);
    let mut elided = base_v2();
    elided.last = Some(last);
    elided.rows.after = Some(triple(&last));
    let mut receipt_last = elided.clone();
    receipt_last.last = Some(key(SourceRank::Receipt, 3, None, 11, 22));
    let a = elided.encode("i").unwrap();
    let b = receipt_last.encode("i").unwrap();
    assert!(a.len() < b.len(), "{} !< {}", a.len(), b.len());
    assert_eq!(State::decode(&a).unwrap().rows.after, Some(triple(&last)));
    assert_eq!(State::decode(&b).unwrap().rows.after, Some(triple(&last)));
    // a different position on a delivery-row last is not elided and survives
    let mut other = elided.clone();
    other.rows.after = Some(DeliveryRowsPosition {
        ordinal: 23,
        ..triple(&last)
    });
    assert_eq!(State::decode(&other.encode("i").unwrap()).unwrap(), other);
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64[((n >> 18) & 63) as usize] as char);
        out.push(B64[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64[((n >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(B64[(n & 63) as usize] as char);
        }
    }
    out
}
fn b64_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in input.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= (B64.iter().position(|x| x == c).unwrap() as u32) << (18 - i * 6);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    out
}
/// Re-tags an edited payload (same tag recipe as the codec) so only the structural checks can
/// reject it.
fn retag(payload: &[u8], instance: &str, seat: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"herdr-threads cursor c3\0");
    for part in [instance, seat, "inbox_batch_v2"] {
        hasher.update((part.len() as u32).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.update(payload);
    let mut bytes = payload.to_vec();
    bytes.extend_from_slice(
        &hasher.finalize()[..herdr_threads::protocol::pagination::CURSOR_TAG_BYTES],
    );
    format!("ib2:{}", b64_encode(&bytes))
}

fn payload_of(raw: &str) -> Vec<u8> {
    let bytes = b64_decode(&raw[4..]);
    bytes[..bytes.len() - herdr_threads::protocol::pagination::CURSOR_TAG_BYTES].to_vec()
}

#[test]
fn cursor_v2_rejects_noncanonical_and_out_of_bounds() {
    let mut elided = base_v2();
    let last = key(SourceRank::DeliveryRow, 1, None, 4, 5);
    elided.last = Some(last);
    elided.rows.after = Some(triple(&last));
    let mut windowed = base_v2();
    windowed.deadline.window_end = Some(walks(2, Some(3), 4, 5, true));
    let good_raw = windowed.encode("i").unwrap();
    assert!(State::decode_for(&good_raw, "i", &windowed.seat).is_ok());
    let seat = windowed.seat.clone();
    let check = |payload: Vec<u8>, why: &str| {
        let raw = retag(&payload, "i", "s");
        assert!(State::decode_for(&raw, "i", &seat).is_err(), "{why}");
    };
    let good = payload_of(&good_raw);
    // sanity: an untouched re-tag decodes
    assert!(State::decode_for(&retag(&good, "i", "s"), "i", &seat).is_ok());
    // flipped flag bits: each single-bit flip of the flag bytes either is rejected or changes nothing legal
    for bit in 0..16 {
        let mut p = good.clone();
        p[1 + bit / 8] ^= 1 << (bit % 8);
        let raw = retag(&p, "i", "s");
        if let Ok(s) = State::decode_for(&raw, "i", &seat) {
            assert_ne!(
                s, windowed,
                "flipping flag bit {bit} must not decode to the same state"
            );
        }
    }
    check(
        {
            let mut p = good.clone();
            p[2] |= 0xf0;
            p
        },
        "reserved flag bits",
    );
    check(
        {
            let mut p = good.clone();
            p.push(0);
            p
        },
        "trailing byte",
    );
    // overlong varint: dh=50 encoded as 0xb2 0x00 instead of 0x32
    {
        let mut p = good.clone();
        let dh_at = p.iter().position(|b| *b == 50).unwrap();
        p.splice(dh_at..=dh_at, [0xb2, 0x00]);
        check(p, "overlong varint");
    }
    // out of bounds, rejected at encode
    let mut s = base_v2();
    s.deadline.window_start.invitation_after = 10;
    assert!(s.encode("i").is_err(), "after > high");
    let mut s = windowed.clone();
    s.deadline.window_end = Some(walks(0, Some(3), 4, 5, false));
    assert!(s.encode("i").is_err(), "end < start");
    let mut s = windowed.clone();
    s.deadline.window_end = Some(walks(2, Some(3), 4, 99, false));
    assert!(s.encode("i").is_err(), "end > high");
    let mut s = base_v2();
    s.body = Some((
        Source::Warnings,
        InboxBatchV2BodyPosition {
            message: MessageId::new("m"),
            offset: 1,
            body_len: 2,
        },
    ));
    assert!(s.encode("i").is_err(), "body on warnings");
    let mut s = base_v2();
    s.body = Some((
        Source::Lazy,
        InboxBatchV2BodyPosition {
            message: MessageId::new("m"),
            offset: 3,
            body_len: 2,
        },
    ));
    assert!(s.encode("i").is_err(), "offset > len");
    let mut s = base_v2();
    s.rows.done = true;
    s.body = Some((
        Source::Lazy,
        InboxBatchV2BodyPosition {
            message: MessageId::new("m"),
            offset: 1,
            body_len: 2,
        },
    ));
    assert!(s.encode("i").is_err(), "body on done class");
    let mut s = base_v2();
    s.deadline.high.unread = None;
    assert!(s.encode("i").is_err(), "unread pairing");
    let mut s = base_v2();
    s.rows.after = Some(DeliveryRowsPosition {
        priority: DeliveryPriority::DEFAULT,
        decision_seq: 51,
        ordinal: 0,
    });
    assert!(s.encode("i").is_err(), "rows decision_seq > dh");
    let mut s = base_v2();
    s.last = Some(key(SourceRank::Receipt, 1, None, 51, 0));
    assert!(s.encode("i").is_err(), "last decision_seq > dh");
    let mut s = base_v2();
    s.binding_generation = None;
    assert!(s.encode("i").is_err(), "identity pairing");
    // an elided flag with a non-delivery-row last: patch a valid elided token's last rank bits
    let raw = elided.encode("i").unwrap();
    let mut p = payload_of(&raw);
    let header_at = p.iter().position(|b| *b == (1 | (3 << 4))).unwrap(); // priority 1, rank DeliveryRow
    p[header_at] = 1 | (2 << 4); // Receipt
    check(p, "elided flag with a receipt last");
}

#[test]
fn cursor_v2_rejects_a_format_1_token_with_the_older_daemon_detail() {
    // A format-1 payload starts with version byte 1; the codec rejects it before reading further.
    let v1 = retag(&[1, 0, 1, 1, b's', 0, 0, 0, 0], "instance", "s");
    assert_eq!(State::decode(&v1), Err(CURSOR_FROM_OLDER_DAEMON));
    assert_eq!(
        State::decode_for(&v1, "instance", &SeatId::new("s")),
        Err(CURSOR_FROM_OLDER_DAEMON)
    );
    assert_eq!(
        CURSOR_FROM_OLDER_DAEMON,
        "cursor from an older daemon; run inbox again"
    );
    let unknown = retag(&[3, 0, 0], "instance", "s");
    assert_eq!(State::decode(&unknown), Err("unknown v2 cursor version"));
}

/// The pre-upgrade (base 3a1203e0) `decode_parts` rules, mirrored: `ib2:` prefix, base64url,
/// more than a tag of bytes, then the first payload byte must be 1.
fn old_v1_decode_rules(encoded: &str) -> Result<u8, &'static str> {
    let raw = encoded.strip_prefix("ib2:").ok_or("not a v2 cursor")?;
    let bytes = b64_decode(raw);
    if bytes.len() <= herdr_threads::protocol::pagination::CURSOR_TAG_BYTES {
        return Err("truncated v2 cursor");
    }
    match bytes[0] {
        1 => Ok(1),
        _ => Err("unknown v2 cursor version"),
    }
}

#[test]
fn old_cli_rejects_a_new_daemon_format_2_cursor_cleanly() {
    // Accepted limit (spec Compatibility, old client / new daemon): an old CLI cannot follow a
    // format-2 `next:` token; it fails with its own error and a plain `inbox` rerun recovers.
    let token = base_v2().encode("instance").unwrap();
    assert_eq!(payload_of(&token)[0], 2);
    assert_eq!(
        old_v1_decode_rules(&token),
        Err("unknown v2 cursor version")
    );
    // The new client is symmetric: a format-1 token is a clean stale-cursor error, not a page.
    let v1 = retag(&[1, 0, 1, 1, b's', 0, 0, 0, 0], "instance", "s");
    assert_eq!(old_v1_decode_rules(&v1), Ok(1));
    assert_eq!(State::decode(&v1), Err(CURSOR_FROM_OLDER_DAEMON));
    assert_eq!(State::validate_decodable(&v1), Ok(()));
}

#[test]
fn cursor_v2_binding_mismatch_is_unchanged() {
    let s = base_v2();
    let raw = s.encode("instance").unwrap();
    assert_eq!(
        State::decode_for(&raw, "other", &s.seat),
        Err("v2 cursor binding mismatch")
    );
    assert_eq!(
        State::decode_for(&raw, "instance", &SeatId::new("foreign")),
        Err("v2 cursor binding mismatch")
    );
    assert!(State::decode_for(&raw, "instance", &s.seat).is_ok());
}
