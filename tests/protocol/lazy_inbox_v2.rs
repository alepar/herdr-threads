use herdr_threads::protocol::{
    commands::Command,
    ids::{ExecutionId, MessageId, SeatId},
    pagination::{
        Cursor, CursorDirection, CursorScope, InboxBatchV2BodyPosition,
        InboxBatchV2CursorState as State, InboxBatchV2Source as Source, MAX_CURSOR_BYTES,
        PageRequest, ReceiptAttentionCursorState, SeatAttentionCursorState,
    },
};
fn state() -> State {
    State {
        seat: SeatId::new("s"),
        binding_generation: Some(1),
        execution: Some(ExecutionId::new("execution")),
        source: Source::Lazy,
        lazy_after_ordinal: 0,
        lazy_high_water_ordinal: 2,
        publication_decision_high_water: 4,
        body: Some(InboxBatchV2BodyPosition {
            message: MessageId::new("m"),
            offset: 2,
            body_len: 10,
        }),
        attention: SeatAttentionCursorState {
            invitation_after_seq: 0,
            invitation_after_ordinal: 0,
            invitations_done: false,
            has_pending_invitation: false,
            invitation_frontier: Some((0, 2)),
            receipts: Some(ReceiptAttentionCursorState {
                physical_after: 0,
                manifest_after: 0,
                physical_high_water: 2,
                manifest_high_water: 2,
                next_manifest: false,
            }),
            receipts_done: false,
            has_pending_receipt: false,
            receipt_frontier_seq: Some(4),
            physical_warning_after: 0,
            physical_warning_high_water: 4,
            manifest_warning_after: 0,
            manifest_warning_high_water: 4,
            next_manifest_warning: false,
            latest_warning_seq: None,
            latest_warning_offset: Some(0),
        },
    }
}
#[test]
fn lazy_inbox_v2_codec_round_trip_binding_and_bounds() {
    for source in [
        Source::Lazy,
        Source::Receipts,
        Source::Invitations,
        Source::Warnings,
    ] {
        let mut value = state();
        value.source = source;
        if matches!(source, Source::Invitations | Source::Warnings) {
            value.body = None;
        }
        let raw = value.encode("instance").unwrap();
        assert!(raw.starts_with("ib2:"));
        assert!(raw.len() <= MAX_CURSOR_BYTES);
        assert_eq!(
            State::decode_for(&raw, "instance", &value.seat).unwrap(),
            value
        );
        assert!(State::decode_for(&raw, "other", &value.seat).is_err());
        assert!(State::decode_for(&raw, "instance", &SeatId::new("foreign")).is_err());
        assert!(Cursor::decode(&raw).is_err());
        let mut corrupt = raw.into_bytes();
        corrupt[5] = if corrupt[5] == b'A' { b'B' } else { b'A' };
        assert!(
            State::decode_for(
                &String::from_utf8(corrupt).unwrap(),
                "instance",
                &value.seat
            )
            .is_err()
        );
    }
    let mut extreme = state();
    extreme.seat = SeatId::new("s".repeat(128));
    extreme.execution = Some(ExecutionId::new("e".repeat(128)));
    extreme.binding_generation = Some(i64::MAX as u64);
    extreme.lazy_high_water_ordinal = i64::MAX as u64;
    extreme.publication_decision_high_water = i64::MAX as u64;
    extreme.attention.receipt_frontier_seq = Some(i64::MAX);
    extreme.attention.physical_warning_high_water = i64::MAX;
    extreme.attention.manifest_warning_high_water = i64::MAX;
    extreme.attention.invitation_frontier = Some((0, i64::MAX));
    let r = extreme.attention.receipts.as_mut().unwrap();
    r.physical_high_water = i64::MAX;
    r.manifest_high_water = i64::MAX;
    extreme.body = Some(InboxBatchV2BodyPosition {
        message: MessageId::new("m".repeat(128)),
        offset: 1,
        body_len: u64::MAX,
    });
    let raw = extreme.encode("instance").unwrap();
    assert!(raw.len() <= MAX_CURSOR_BYTES);
    assert_eq!(
        State::decode_for(&raw, "instance", &extreme.seat).unwrap(),
        extreme
    );
    for bad in 0..6 {
        let mut value = state();
        match bad {
            0 => value.lazy_after_ordinal = 3,
            1 => value.body.as_mut().unwrap().offset = 11,
            2 => value.binding_generation = None,
            3 => value.attention.receipts = None,
            4 => value.source = Source::Warnings,
            _ => {
                value.source = Source::Receipts;
                value.attention.receipts_done = true;
            }
        }
        assert!(value.encode("instance").is_err());
    }
    assert!(State::decode(&"ib2:A".repeat(300)).is_err());
}
#[test]
fn lazy_inbox_v2_codec_is_rejected_by_every_legacy_page_route() {
    use serde_json::json;
    let raw = state().encode("i").unwrap();
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
