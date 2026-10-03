use herdr_threads::protocol::{
    commands::Command,
    pagination::{Cursor, CursorScope, PageRequest, *},
    results::{ErrorCode, Health},
    time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    wire::{PROTOCOL_VERSION, WireRequest, WireResponse},
};

/// The built binary or a helper process with every inherited HERDR_/CLAUDE/CODEX
/// variable removed (ht-p03.24); a test sets the variables it needs after this
/// call. The scrub is part of the `test-support` build; without the feature the
/// suite still compiles and the process is spawned as before.
fn scrubbed_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
    let mut command = std::process::Command::new(program);
    #[cfg(feature = "test-support")]
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
}

const WIRE_FIXTURE_INSTANCE: &str = "00000000-0000-4000-8000-000000000001";

fn wire_fixture(command: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r1","expected_instance":WIRE_FIXTURE_INSTANCE,"command":command})
}

fn wire_fixture_cursor(scope: CursorScope, scope_key: &str) -> Cursor {
    Cursor {
        instance: WIRE_FIXTURE_INSTANCE.into(),
        scope,
        scope_key: scope_key.into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 1,
        high_water_ordinal: 5,
        scope_revision: Some(1),
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
}

#[test]
fn warning_recipients_do_not_claim_receipt_or_ack_state() {
    use herdr_threads::protocol::results::{CommandResult, WarningRecipient};
    let recipient = WarningRecipient {
        seat: herdr_threads::protocol::ids::SeatId::new("seat-1"),
    };
    let encoded = serde_json::to_value(CommandResult::WarningRecipients(Page {
        items: vec![recipient],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 1,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }))
    .unwrap();
    assert_eq!(encoded["kind"], "warning_recipients");
    assert!(encoded["data"]["items"][0].get("status").is_none());
    assert!(encoded["data"]["items"][0].get("ack_provenance").is_none());
}

#[test]
fn wake_attention_cursor_roundtrips_all_source_high_waters_under_wire_cap() {
    let max = i64::MAX - 1;
    let cursor = Cursor {
        instance: "instance".into(),
        scope: CursorScope::WakeCandidates,
        scope_key: "seat".into(),
        filter_digest: "wake".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 1,
        high_water_ordinal: 2,
        scope_revision: Some(7),
        filter_revision: None,
        search: None,
        inbox: None,
        attention: Some(SeatAttentionCursorState {
            invitation_after_seq: max,
            invitation_after_ordinal: max,
            invitations_done: true,
            has_pending_invitation: true,
            invitation_frontier: Some((max, max)),
            receipts: Some(ReceiptAttentionCursorState {
                physical_after: max,
                manifest_after: max,
                physical_high_water: max,
                manifest_high_water: max,
                next_manifest: true,
            }),
            receipts_done: false,
            has_pending_receipt: false,
            receipt_frontier_seq: Some(max),
            physical_warning_after: max,
            physical_warning_high_water: max,
            manifest_warning_after: max,
            manifest_warning_high_water: max,
            next_manifest_warning: false,
            latest_warning_seq: Some(max),
            latest_warning_offset: Some(max),
        }),
        binding: None,
    };
    let encoded = cursor.encode().unwrap();
    assert!(encoded.len() <= MAX_CURSOR_BYTES);
    assert_eq!(
        Cursor::decode_for(
            &encoded,
            "instance",
            CursorScope::WakeCandidates,
            "seat",
            "wake",
            CursorDirection::Ascending,
            1
        )
        .unwrap(),
        cursor
    );
    // The legacy form still round-trips for one release.
    assert_eq!(
        Cursor::decode(&cursor.encode_legacy().unwrap()).unwrap(),
        cursor
    );
}

#[test]
fn health_settings_are_typed_and_unknown_until_resolved() {
    use herdr_threads::protocol::results::{Health, HealthSettings};
    let mut health = Health::unknown(
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
        "v".into(),
        1,
    );
    assert!(health.settings.is_none());
    health.settings = Some(HealthSettings {
        invitation_default_ms: 300_000,
        receipt_default_ms: 300_000,
        minimum_wake_delay_ms: 30_000,
    });
    assert!(health.validate().is_ok());
    let json = serde_json::to_value(&health).unwrap();
    assert_eq!(json["settings"]["minimum_wake_delay_ms"], 30_000);
    health.settings.as_mut().unwrap().minimum_wake_delay_ms = 0;
    assert!(health.validate().is_err());
}

#[test]
fn search_logical_order_v2_cursor_is_typed_and_scope_limited() {
    let cursor = Cursor {
        instance: "i".into(),
        scope: CursorScope::SearchCandidates,
        scope_key: "literal".into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 2,
        last_examined_key: None,
        after_ordinal: 0,
        high_water_ordinal: 10,
        scope_revision: None,
        filter_revision: None,
        inbox: None,
        attention: None,
        search: Some(SearchCursorState {
            phase: SearchPhase::Body,
            topic_high_water: 5,
            body_high_water: 12,
            topic_revision: 1,
            last_decision_seq: Some(7),
            last_event_offset: Some(2),
        }),
        binding: None,
    };
    let raw = cursor.encode().unwrap();
    assert!(raw.len() <= MAX_CURSOR_BYTES);
    assert_eq!(
        Cursor::decode_for(
            &raw,
            "i",
            CursorScope::SearchCandidates,
            "literal",
            "digest",
            CursorDirection::Ascending,
            2
        )
        .unwrap(),
        cursor
    );
    let mut wrong = cursor;
    wrong.scope = CursorScope::Inbox;
    assert!(Cursor::decode(&wrong.encode().unwrap()).is_err());
}

#[test]
fn joined_invite_outcome_does_not_claim_an_invitation_episode() {
    use herdr_threads::protocol::results::{AlreadyJoined, CommandResult};
    let result = CommandResult::AlreadyJoined(AlreadyJoined {
        thread: herdr_threads::protocol::ids::ThreadId::new("thread-1"),
        seat: herdr_threads::protocol::ids::SeatId::new("seat-1"),
    });
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire["kind"], "already_joined");
    assert!(wire["data"].get("invitation").is_none());
}

#[test]
fn response_uncertainty_has_a_stable_typed_wire_code() {
    let code: ErrorCode = serde_json::from_str("\"unknown_outcome\"").unwrap();
    assert_eq!(serde_json::to_string(&code).unwrap(), "\"unknown_outcome\"");
}

#[test]
fn wire_instance_mismatch_and_uuid_boot_are_typed() {
    let code: ErrorCode = serde_json::from_str("\"instance_mismatch\"").unwrap();
    assert_eq!(code, ErrorCode::InstanceMismatch);
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "r".into(),
        expected_instance: "00000000-0000-4000-8000-000000000001".into(),
        expected_boot: None,
        output: None,
        command: Command::Health,
    };
    let malformed = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: "r".into(),
        instance: request.expected_instance.clone(),
        daemon_boot: "boot-a".into(),
        result: Err(
            herdr_threads::protocol::results::ApiError::instance_mismatch("wrong instance"),
        ),
    };
    assert!(!malformed.correlates_to(&request, None));
    assert!(
        serde_json::from_value::<WireResponse>(serde_json::to_value(malformed).unwrap()).is_err()
    );
}

#[test]
fn wire_request_expected_boot_is_optional_validated_and_typed() {
    let base = |boot: &str| {
        format!(
            r#"{{"version":{PROTOCOL_VERSION},"request_id":"r","expected_instance":"00000000-0000-4000-8000-000000000001"{boot},"command":{{"kind":"health"}}}}"#
        )
    };
    let absent: WireRequest = serde_json::from_str(&base("")).unwrap();
    assert_eq!(absent.expected_boot, None);
    assert!(
        !serde_json::to_string(&absent)
            .unwrap()
            .contains("expected_boot")
    );
    let uuid = "00000000-0000-4000-8000-000000000002";
    let present: WireRequest =
        serde_json::from_str(&base(&format!(r#","expected_boot":"{uuid}""#))).unwrap();
    assert_eq!(present.expected_boot.as_deref(), Some(uuid));
    assert_eq!(
        serde_json::to_value(&present).unwrap()["expected_boot"],
        uuid
    );
    assert!(serde_json::from_str::<WireRequest>(&base(r#","expected_boot":"boot-a""#)).is_err());
    let code: ErrorCode = serde_json::from_str("\"daemon_boot_changed\"").unwrap();
    assert_eq!(code, ErrorCode::DaemonBootChanged);
}

#[test]
fn cli_directory_accept_and_body_offset_have_explicit_wire_contracts() {
    use herdr_threads::protocol::commands::{
        Accept, BodyReadRequest, DirectoryMembership, DirectoryQuery,
    };
    let directory: DirectoryQuery = serde_json::from_value(serde_json::json!({
        "membership":"seat-1", "membership_filter":"invited", "topic_contains":null,
        "page":{"cursor":null,"limit":20,"max_bytes":16384}
    }))
    .unwrap();
    assert_eq!(directory.membership_filter, DirectoryMembership::Invited);
    assert_eq!(directory.membership.unwrap().as_str(), "seat-1");
    let accept: Accept = serde_json::from_value(serde_json::json!({
        "thread":"thread-1", "operation":"op-1",
        "claim":{"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n","execution":"e","target":"p"}
    }))
    .unwrap();
    assert_eq!(accept.thread.as_str(), "thread-1");
    let offset: BodyReadRequest = serde_json::from_value(serde_json::json!({
        "offset":5,"cursor":null,"max_bytes":1024
    }))
    .unwrap();
    assert!(
        offset
            .validate(&herdr_threads::protocol::ids::MessageId::new("m"))
            .is_ok()
    );
    let conflict: BodyReadRequest = serde_json::from_value(serde_json::json!({
        "offset":5,"cursor":"invalid","max_bytes":1024
    }))
    .unwrap();
    assert!(
        conflict
            .validate(&herdr_threads::protocol::ids::MessageId::new("m"))
            .is_err()
    );
}

#[test]
fn ordinary_seat_resolution_has_a_private_operation_key() {
    use herdr_threads::protocol::commands::ResolveSeat;
    let request: ResolveSeat = serde_json::from_value(serde_json::json!({
        "target":"pane","operation":"op"
    }))
    .unwrap();
    assert_eq!(request.operation.as_str(), "op");
    assert!(serde_json::from_value::<ResolveSeat>(serde_json::json!({"target":"pane"})).is_err());
}

#[test]
fn send_preparation_progress_reports_visited_count_and_preparation_id() {
    use herdr_threads::ports::SendPreparationProgress;
    let progress = SendPreparationProgress::More {
        visited: 16,
        preparation_id: "prep-1".into(),
    };
    assert_eq!(progress.visited(), 16);
    assert_eq!(progress.preparation_id(), Some("prep-1"));
}

#[test]
fn registration_request_binds_operation_and_selected_offer_context() {
    use herdr_threads::ports::{ReadContext, RegisterAvailableRequest};
    use herdr_threads::protocol::{
        authority::{CallerClaim, Harness},
        commands::CheckIn,
        ids::*,
        output::OutputSpec,
    };
    let request = RegisterAvailableRequest {
        command: CheckIn {
            mode: herdr_threads::protocol::commands::CheckInMode::Current,
            claim: CallerClaim {
                instance: "i".into(),
                seat: SeatId::new("legacy-fixture"),
                binding_generation: 0,
                role: herdr_threads::protocol::authority::CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("n"),
                execution: ExecutionId::new("e"),
                target: HostTargetId::new("p"),
            },
            operation: OperationId::new("op"),
        },
        read: ReadContext {
            instance: "i".into(),
            output: OutputSpec::default(),
            operation_scope: None,
        },
        operator: None,
    };
    assert_eq!(request.command.operation.as_str(), "op");
    assert!(request.read.validate().is_ok());
}

#[test]
fn due_receipt_cursor_retains_both_physical_and_sparse_positions() {
    use herdr_threads::ports::{
        DuePhase, DuePhaseCursor, DueScanRequest, DueScanState, ExtensionLapseKey,
        ReceiptSparseCursor,
    };
    use herdr_threads::protocol::ids::{MessageId, SeatId, ThreadId};
    let cursor = DuePhaseCursor {
        high_water_ordinal: 70,
        after_deadline: Some(UtcMillis(100)),
        after_ordinal: 7,
        receipt_sparse: Some(ReceiptSparseCursor {
            high_water_rowid: 80,
            after_deadline: Some(UtcMillis(101)),
            after_message: Some(MessageId::new("m")),
            after_seat: Some(SeatId::new("s")),
            next_sparse: true,
        }),
        extension_through: Some(UtcMillis(102)),
        extension_after: Some(ExtensionLapseKey {
            until: UtcMillis(103),
            seat: SeatId::new("s"),
            thread: ThreadId::new("t"),
        }),
    };
    let request = DueScanRequest {
        state: DueScanState {
            invitations: None,
            receipts: Some(cursor.clone()),
            next_phase: DuePhase::Receipts,
        },
        max_candidates: 100,
        run_invitations: false,
        run_receipts: true,
    };
    assert!(request.validate().is_ok());
    assert_eq!(request.state.receipts, Some(cursor));
    let mut invalid = request;
    invalid
        .state
        .receipts
        .as_mut()
        .unwrap()
        .receipt_sparse
        .as_mut()
        .unwrap()
        .after_seat = None;
    assert!(invalid.validate().is_err());
}

#[test]
fn native_launch_capability_is_an_adapter_owned_boundary() {
    use herdr_threads::ports::NativeLaunchCapability;
    assert_ne!(
        NativeLaunchCapability::Unsupported,
        NativeLaunchCapability::HostGuardedStart
    );
}

#[test]
fn local_intent_header_has_bounded_recovery_reference_and_typed_state() {
    use herdr_threads::protocol::results::{IntentKind, IntentStatus, LocalIntent};
    let header: LocalIntent = serde_json::from_value(serde_json::json!({
        "operation":"op-1","recovery_ref":"local:R_123","created_at":42,
        "kind":"send_message","thread":null,"status":"unknown_outcome"
    }))
    .unwrap();
    assert_eq!(header.recovery_ref.as_str(), "local:R_123");
    assert!(herdr_threads::protocol::ids::MessageId::parse("local:R_123").is_err());
    assert!(herdr_threads::protocol::ids::LocalRecoveryRef::parse("m-123").is_err());
    assert_eq!(header.kind, IntentKind::SendMessage);
    assert_eq!(header.status, IntentStatus::UnknownOutcome);
    let too_long = serde_json::json!({
        "operation":"op-1","recovery_ref":"x".repeat(65),"created_at":42,
        "kind":"send_message","thread":null,"status":"pending"
    });
    assert!(serde_json::from_value::<LocalIntent>(too_long).is_err());
}

#[test]
fn omitted_topic_and_preview_keep_exact_detail_routes() {
    use herdr_threads::protocol::results::{MessageSummary, ThreadSummary};
    let topic: ThreadSummary = serde_json::from_value(serde_json::json!({
        "thread":"t", "topic_data":"short", "topic_omitted":true,
        "topic_detail_argv":["thread","show","t"], "archived":false,
        "orphaned":false,"message_count":1,"created_at":1,"ordinary_count":1,
        "system_count":0,"joined_count":1
    }))
    .unwrap();
    assert_eq!(topic.topic_detail_argv.unwrap()[2], "t");
    let preview: MessageSummary = serde_json::from_value(serde_json::json!({
        "message":"m","thread":"t","author":"s","kind":"ordinary",
        "sequence":1,"created_at":1,"actor_label":null,"preview_data":"short",
        "preview_omitted":true,"preview_detail_argv":["message","show","m"]
    }))
    .unwrap();
    assert_eq!(preview.preview_detail_argv.unwrap()[2], "m");
}

#[test]
fn command_bounds_goal_operator_deadline_and_search_work() {
    let claim = serde_json::json!({"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n","execution":"e","target":"p"});
    let create = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r","expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":"create_thread","args":{"topic":"topic","goal":"x".repeat(1025),"operation":"o","claim":claim}}});
    assert!(WireRequest::decode(create.to_string().as_bytes()).is_err());
    let orphan = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r","expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":"operator_orphan_invite","args":{"thread":"t","seat":"s","deadline_millis":1000,"operation":"o"}}});
    assert!(WireRequest::decode(orphan.to_string().as_bytes()).is_ok());
    let search = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r","expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":"search","args":{"literal":"word","thread":null,"page":{"cursor":null,"limit":20,"max_bytes":16384},"max_candidates":101}}});
    assert!(WireRequest::decode(search.to_string().as_bytes()).is_err());
}

#[test]
fn compound_and_range_read_commands_have_strict_wire_shapes() {
    let page = serde_json::json!({"cursor":null,"limit":20,"max_bytes":16384});
    for (kind, args) in [
        ("seats", serde_json::json!({"page":page})),
        ("seat_inspect", serde_json::json!({"seat":"s","page":page})),
        (
            "delivery_inspect",
            serde_json::json!({"message":"m","page":page}),
        ),
        ("thread", serde_json::json!({"thread":"t","page":page})),
        (
            "history",
            serde_json::json!({"thread":"t","page":page,"initial":{"kind":"after","sequence":3}}),
        ),
    ] {
        let request = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r","expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":kind,"args":args}});
        assert!(
            WireRequest::decode(request.to_string().as_bytes()).is_ok(),
            "rejected {kind}"
        );
    }
    use sha2::{Digest, Sha256};
    let filter = Sha256::digest(serde_json::to_vec("t").unwrap());
    let cursor = Cursor {
        filter_digest: filter[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        ..wire_fixture_cursor(CursorScope::History, "t")
    }
    .encode()
    .unwrap();
    let conflicting = wire_fixture(
        serde_json::json!({"kind":"history","args":{"thread":"t","page":{"cursor":cursor,"limit":20,"max_bytes":16384},"initial":{"kind":"after","sequence":3}}}),
    );
    let mut selector_only = conflicting.clone();
    selector_only["command"]["args"]["page"]["cursor"] = serde_json::Value::Null;
    assert!(WireRequest::decode(selector_only.to_string().as_bytes()).is_ok());
    let mut cursor_only = conflicting.clone();
    cursor_only["command"]["args"]["initial"] = serde_json::Value::Null;
    assert!(WireRequest::decode(cursor_only.to_string().as_bytes()).is_ok());
    assert!(
        WireRequest::decode(conflicting.to_string().as_bytes())
            .unwrap_err()
            .to_string()
            .contains("history selector conflicts with cursor")
    );
}

#[test]
fn incomplete_page_requires_a_reachable_continuation() {
    let page = serde_json::json!({"items":[],"next_cursor":"c2:YWJj","next_argv":["thread","show","t","--cursor","c2:YWJj"],"high_water_ordinal":3,"scope_revision":null,"has_more":true,"stop_reason":"work","consistency":"bounded_live"});
    let parsed: Page<serde_json::Value> = serde_json::from_value(page).unwrap();
    assert!(parsed.validate().is_ok());
    let no_cursor = serde_json::json!({"items":[],"next_cursor":null,"next_argv":null,"high_water_ordinal":3,"scope_revision":null,"has_more":true,"stop_reason":"work","consistency":"bounded_live"});
    let parsed: Page<serde_json::Value> = serde_json::from_value(no_cursor).unwrap();
    assert!(parsed.validate().is_err());
}

#[test]
fn system_message_has_no_fabricated_author_or_body_cursor() {
    let value = serde_json::json!({"kind":"message","data":{
        "summary":{"message":"w","thread":"t","author":null,"kind":"warn","sequence":2,"created_at":1000,"actor_label":null,"preview_data":"warning","preview_omitted":false,"preview_detail_argv":null},
        "content":{"kind":"system","event":{"kind":"warn","event_json":{"reason":"late"},"source_message":null,"source_invitation":null,"decision_at":1000,"classified_at":1000,"materialized_at":null},"current_condition":null}
    }});
    let result: herdr_threads::protocol::results::CommandResult =
        serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), value);
    let mut bad = value;
    bad["data"]["body_data"] = serde_json::json!("");
    assert!(
        serde_json::from_value::<herdr_threads::protocol::results::CommandResult>(bad).is_err()
    );
}

#[test]
fn thread_detail_and_search_hit_preserve_compound_metadata() {
    let summary = serde_json::json!({"thread":"t","topic_data":"topic","topic_omitted":false,"topic_detail_argv":null,"archived":false,"orphaned":false,"message_count":3,"created_at":100,"ordinary_count":2,"system_count":1,"joined_count":1});
    let page = serde_json::json!({"items":[],"next_cursor":null,"next_argv":null,"high_water_ordinal":0,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"});
    let detail = serde_json::json!({"kind":"thread","data":{"summary":summary,"goal_data":"goal","created_at":100,"participant_count":1,"participants":page,"pending_receipt_count":2,"pending_receipts_argv":["pending-receipts","--thread","t"]}});
    let result: herdr_threads::protocol::results::CommandResult =
        serde_json::from_value(detail.clone()).unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), detail);
    let hit = serde_json::json!({"kind":"topic","data":summary});
    let search = serde_json::json!({"kind":"search","data":{"matches":{"items":[hit],"next_cursor":null,"next_argv":null,"high_water_ordinal":1,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"},"examined_candidates":1,"examined_utf8_bytes":5}});
    let result: herdr_threads::protocol::results::CommandResult =
        serde_json::from_value(search.clone()).unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), search);
}

#[test]
fn seat_and_delivery_inspection_are_compound_pages() {
    let page = serde_json::json!({"items":[],"next_cursor":null,"next_argv":null,"high_water_ordinal":0,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"});
    let seat = serde_json::json!({"kind":"seat_inspect","data":{
        "summary":{"seat":"s","continuity":"resolved","target":"p","generation":2,"created_at":1,"retired_at":null},
        "mapping":{"state":"resolved","target":"p","detail_argv":["seat","inspect","s"]},
        "hold":null,"open_binding":null,"retirement":null,"history":page
    }});
    let parsed: herdr_threads::protocol::results::CommandResult =
        serde_json::from_value(seat.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), seat);
    let message = serde_json::json!({"message":"m","thread":"t","author":"s","kind":"ordinary","sequence":1,"created_at":1,"actor_label":null,"preview_data":"hello","preview_omitted":false,"preview_detail_argv":null});
    let delivery = serde_json::json!({"kind":"delivery_inspect","data":{
        "message":message,"delivery":{"committed":1,"attempted":null,"submitted":null,"read":null,"acknowledged":0},"recipients":page
    }});
    let parsed: herdr_threads::protocol::results::CommandResult =
        serde_json::from_value(delivery.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), delivery);
}

#[test]
fn pending_and_retired_rows_preserve_physical_and_provenance_facts() {
    let participant = serde_json::json!({"seat":"s","episode":2,"joined":false,"retired":true,"physical_state":"joined","effective_state":"retired","joined_at":10,"left_at":null,"retirement_cutover":20,"cleanup_state":"pending","accepted_invitation":{"invitation":"v","actor":"s","generation":2,"native_observation":"proof","accepted_at":10}});
    let parsed: herdr_threads::protocol::results::Participant =
        serde_json::from_value(participant.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), participant);
    let receipt = serde_json::json!({"message":"m","thread":"t","seat":"s","sequence":4,"sender":"author","decision_at":5,"available_at":null,"deadline":null,"overdue":false});
    let parsed: herdr_threads::protocol::results::PendingReceipt =
        serde_json::from_value(receipt.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), receipt);
}

#[test]
fn pending_receipt_from_a_service_author_has_no_seat_sender() {
    use herdr_threads::protocol::{
        ids::{MessageId, SeatId, ThreadId},
        results::PendingReceipt,
        time::UtcMillis,
    };
    let programmatic = serde_json::json!({"message":"m","thread":"t","seat":"s","sequence":4,"sender":null,"sender_author":{"kind":"programmatic","id":"graph"},"decision_at":5,"available_at":null,"deadline":null,"overdue":false});
    let parsed: PendingReceipt = serde_json::from_value(programmatic.clone()).unwrap();
    assert_eq!(parsed.sender, None);
    assert!(parsed.sender_author.is_some());
    assert_eq!(serde_json::to_value(parsed).unwrap(), programmatic);

    let native = PendingReceipt {
        message: MessageId::new("m"),
        thread: ThreadId::new("t"),
        seat: SeatId::new("s"),
        sequence: 4,
        sender: Some(SeatId::new("author")),
        sender_author: None,
        decision_at: UtcMillis(5),
        available_at: None,
        deadline: None,
        overdue: false,
        effective_deadline: None,
        deferred_until: None,
    };
    assert_eq!(
        serde_json::to_value(native).unwrap(),
        serde_json::json!({"message":"m","thread":"t","seat":"s","sequence":4,"sender":"author","decision_at":5,"available_at":null,"deadline":null,"overdue":false})
    );
}

#[test]
fn read_errors_report_typed_cursor_budget_and_required_minimum() {
    for spelling in [
        "invalid_cursor",
        "invalid_budget",
        "read_budget_exhausted",
        "sequence_exhausted",
    ] {
        let code: ErrorCode = serde_json::from_value(serde_json::json!(spelling)).unwrap();
        assert_eq!(serde_json::to_value(code).unwrap(), spelling);
    }
    let value = serde_json::json!({"code":"invalid_budget","detail":"too small","restart_argv":["thread","show","t"],"required_minimum_bytes":900});
    let error: herdr_threads::protocol::results::ApiError =
        serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(error).unwrap(), value);
}

#[test]
fn selected_output_context_is_bounded_and_json_is_the_direct_default() {
    use herdr_threads::protocol::output::{ContinuationContext, OutputFormat, OutputSpec};
    let direct = OutputSpec::default();
    assert_eq!(direct.format, OutputFormat::Json);
    assert!(direct.validate().is_ok());
    let invalid = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some("x".repeat(1025)),
            host: None,
        },
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn wire_request_carries_expected_instance_without_granting_authority() {
    let valid = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r","expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":"health"}});
    let request = WireRequest::decode(valid.to_string().as_bytes()).unwrap();
    assert_eq!(
        request.expected_instance,
        "00000000-0000-4000-8000-000000000001"
    );
    let missing = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r","command":{"kind":"health"}});
    assert!(WireRequest::decode(missing.to_string().as_bytes()).is_err());
    let too_long = serde_json::json!({"version":PROTOCOL_VERSION,"request_id":"r".repeat(129),"expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":"health"}});
    assert!(WireRequest::decode(too_long.to_string().as_bytes()).is_err());
}

#[test]
fn wire_response_is_bounded_and_correlates_actual_identity() {
    use herdr_threads::protocol::wire::{MAX_WIRE_FRAME_BYTES, encode_wire_response};
    let request = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "r".into(),
        expected_instance: "00000000-0000-4000-8000-000000000001".into(),
        expected_boot: None,
        output: None,
        command: Command::Health,
    };
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: "r".into(),
        instance: "00000000-0000-4000-8000-000000000001".into(),
        daemon_boot: "00000000-0000-4000-8000-000000000002".into(),
        result: Ok(herdr_threads::protocol::results::CommandResult::Health(
            Health::unknown(
                "00000000-0000-4000-8000-000000000001".into(),
                "00000000-0000-4000-8000-000000000002".into(),
                "1".into(),
                1,
            ),
        )),
    };
    assert!(response.correlates_to(&request, Some("00000000-0000-4000-8000-000000000002")));
    assert!(!response.correlates_to(&request, Some("old")));
    let mut mismatched_health = response.clone();
    if let Ok(herdr_threads::protocol::results::CommandResult::Health(health)) =
        &mut mismatched_health.result
    {
        health.instance_id = "00000000-0000-4000-8000-000000000099".into();
    }
    assert!(encode_wire_response(&mismatched_health).is_err());
    let frame = encode_wire_response(&response).unwrap();
    assert_eq!(
        frame.len() - 4,
        u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize
    );
    assert!(frame.len() <= MAX_WIRE_FRAME_BYTES);
    let mut oversized = response;
    oversized.result = Err(herdr_threads::protocol::results::ApiError::invalid_request(
        "x".repeat(MAX_WIRE_FRAME_BYTES),
    ));
    assert!(encode_wire_response(&oversized).is_err());
}

#[test]
fn fresh_structure_does_not_imply_verified_execution() {
    use herdr_threads::ports::{
        ExecutionEvidence, HostObservation, HostUiState, IncarnationEvidence,
        ObservationProvenance, StructuralOccupancy,
    };
    use herdr_threads::protocol::ids::{HostBootId, HostCallId, HostTargetId, TerminalId};
    let mut observed = HostObservation {
        focused: false,
        target: HostTargetId::new("p"),
        host_boot: HostBootId::new("boot"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(10),
        observed_at_mono: MonoInstant(10),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new("terminal")),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("call"),
        connection_epoch: 1,
        observation_sequence: 2,
        started_at_mono: MonoInstant(9),
        completed_at_mono: MonoInstant(10),
    };
    assert!(observed.is_fresh_structure());
    assert!(!observed.has_verified_execution());
    observed.incarnation = IncarnationEvidence::Verified {
        identity: String::new(),
        evidence_kind: herdr_threads::ports::EvidenceKind::NativeCurrentTarget,
    };
    observed.execution = ExecutionEvidence::Verified {
        execution: herdr_threads::protocol::ids::ExecutionId::new("exec"),
        evidence_kind: herdr_threads::ports::EvidenceKind::NativeInvocation,
    };
    assert!(!observed.has_verified_execution());
    observed.incarnation = IncarnationEvidence::Verified {
        identity: "host-incarnation".into(),
        evidence_kind: herdr_threads::ports::EvidenceKind::NativeCurrentTarget,
    };
    assert!(observed.has_verified_execution());
}

#[test]
fn complete_host_enumeration_needs_coherent_incarnation_proof() {
    use herdr_threads::ports::{
        EnumerationEvidence, EvidenceKind, HostSnapshot, IncarnationEvidence,
    };
    use herdr_threads::protocol::ids::HostBootId;
    let mut snapshot = HostSnapshot {
        boot: HostBootId::new("boot"),
        epoch: 2,
        observation_sequence: 0,
        complete: true,
        enumeration: EnumerationEvidence::CompleteUnverified,
        incarnation: IncarnationEvidence::Unknown,
        targets: vec![],
    };
    assert!(!snapshot.authorizes_absence_closure());
    snapshot.enumeration = EnumerationEvidence::CoherentVerified;
    assert!(!snapshot.authorizes_absence_closure());
    snapshot.incarnation = IncarnationEvidence::Verified {
        identity: "native-incarnation".into(),
        evidence_kind: EvidenceKind::CoherentEnumeration,
    };
    assert!(!snapshot.authorizes_absence_closure());
    snapshot.observation_sequence = 7;
    assert!(snapshot.authorizes_absence_closure());
}

#[test]
fn managed_launch_request_preserves_bounded_native_argument_tokens() {
    use herdr_threads::ports::{ConfiguredHook, NativeLaunchRequest};
    use herdr_threads::protocol::{
        authority::Harness,
        ids::{HostTargetId, SeatId, TerminalId},
    };
    let mut request = NativeLaunchRequest {
        seat: SeatId::new("s"),
        target: HostTargetId::new("p"),
        harness: Harness::Codex,
        argv: vec![],
        configured_hook: ConfiguredHook {
            scope: "project".into(),
            path: "/tmp/hook".into(),
            fingerprint: "hash".into(),
        },
        expected_terminal: TerminalId::new("terminal"),
        expected_generation: 2,
        expected_incarnation: "native-incarnation".into(),
        name_hint: None,
    };
    assert!(request.validate().is_ok());
    request.argv = vec!["--no-daemon".into()];
    assert!(request.validate().is_ok());
    // A realistic multi-paragraph initial prompt fits (it used to be capped at 1 KiB).
    request.argv = vec!["--model".into(), "haiku".into(), "x".repeat(4096)];
    assert!(request.validate().is_ok());
    request.argv = vec!["x".repeat(NativeLaunchRequest::MAX_ARG_BYTES + 1)];
    assert!(request.validate().unwrap_err().contains("over 16 KiB"));
    request.argv = vec!["x".repeat(NativeLaunchRequest::MAX_ARG_BYTES); 3];
    assert!(
        request
            .validate()
            .unwrap_err()
            .contains("total over 32 KiB")
    );
    request.argv = vec![String::new()];
    assert!(request.validate().is_err());
    // Herdr types the command into the pane's shell: a line break would submit it early.
    request.argv = vec!["line one\nline two".into()];
    assert!(request.validate().unwrap_err().contains("line break"));
}

#[test]
fn due_scan_keeps_independent_phase_errors_and_explicit_continuation() {
    use herdr_threads::ports::{DuePhase, DuePhaseProgress, DueScanProgress, DueScanState};
    let state = DueScanState::default();
    let progress = DueScanProgress {
        state: DueScanState {
            next_phase: DuePhase::Receipts,
            ..state
        },
        examined_candidates: 0,
        warnings_added: 0,
        invitations: DuePhaseProgress::Failed(
            herdr_threads::protocol::results::ErrorCode::StoreBusy,
            herdr_threads::protocol::results::BoundedError::parse("locked").unwrap(),
        ),
        receipts: DuePhaseProgress::More,
        has_more: true,
    };
    assert_eq!(progress.state.next_phase, DuePhase::Receipts);
    assert!(progress.has_more);
    assert_eq!(progress.warnings_added, 0);
}

#[test]
fn due_scan_can_defer_one_phase_without_losing_its_cursor() {
    use herdr_threads::ports::{DuePhaseProgress, DueScanRequest, DueScanState};
    let request = DueScanRequest {
        state: DueScanState::default(),
        max_candidates: 100,
        run_invitations: false,
        run_receipts: true,
    };
    assert!(request.validate().is_ok());
    assert_eq!(DuePhaseProgress::Skipped, DuePhaseProgress::Skipped);
    let invalid = DueScanRequest {
        run_receipts: false,
        ..request
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn wake_candidate_exposes_pending_obligation_with_zero_projection_bits() {
    use herdr_threads::ports::{WakeCandidate, WarningOfferFrontier};
    use herdr_threads::protocol::ids::{ExecutionId, SeatId};
    let candidate = WakeCandidate {
        seat: SeatId::new("s"),
        attention_witness: None,
        effectively_retired: false,
        continuity_resolved: true,
        binding_generation: Some(2),
        binding_execution: Some(ExecutionId::new("exec")),
        target: None,
        reason_bits: 0,
        has_pending_invitation: false,
        has_pending_receipt: true,
        actionable_warning_generation: None,
        actionable_warning_seq: Some(9),
        warning_offer: Some(WarningOfferFrontier {
            generation: 2,
            execution: ExecutionId::new("exec"),
            offered_through_seq: 8,
        }),
        attention_version: 0,
        checkpoint_version: 0,
        retry_step: 0,
        reservation_id: None,
        reservation_boot: None,
        last_reservation_id: None,
        last_reservation_boot: None,
        last_reserved_frontier: Default::default(),
        minimum_delay_ms: 0,
        effective_delay_ms: 0,
        last_outcome: None,
        last_reserved_at_utc: None,
    };
    assert!(candidate.has_actionable_work());
    assert!(!candidate.warning_offered_for_current_occupant());
}

#[test]
fn request_wire_rejects_authority_and_unknown_versions() {
    for payload in [
        r#"{"version":1,"request_id":"r1","command":{"kind":"health"},"verified_actor":{"seat":"forged"}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"health"},"operator_actor":{"uid":0}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"health"},"mutation_permit":{"seat":"forged"}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"health"},"trusted_decision_at":1}"#,
        r#"{"version":99,"request_id":"r1","command":{"kind":"health"}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"operator_fresh_seat","args":{"target":"p1","operation":"o1","operator_actor":{"uid":0}}}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"check_in","args":{"mode":{"kind":"current"},"operation":"o1","claim":{"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n1","execution":"e1","target":"p1","verified":true}}}}"#,
    ] {
        // The fixtures spell version 1; stamp the current protocol so only the
        // field under test (or the explicit version 99) is invalid.
        let payload = payload.replacen(
            "\"version\":1,",
            &format!("\"version\":{PROTOCOL_VERSION},"),
            1,
        );
        let mut request: serde_json::Value = serde_json::from_str(&payload).unwrap();
        request["expected_instance"] = serde_json::json!(WIRE_FIXTURE_INSTANCE);
        let mut control = request.clone();
        control["version"] = serde_json::json!(PROTOCOL_VERSION);
        for field in [
            "verified_actor",
            "operator_actor",
            "mutation_permit",
            "trusted_decision_at",
        ] {
            control.as_object_mut().unwrap().remove(field);
        }
        if let Some(args) = control["command"]["args"].as_object_mut() {
            args.remove("operator_actor");
            if let Some(claim) = args
                .get_mut("claim")
                .and_then(serde_json::Value::as_object_mut)
            {
                claim.remove("verified");
            }
        }
        assert!(
            WireRequest::decode(control.to_string().as_bytes()).is_ok(),
            "invalid control for {payload}"
        );
        assert!(
            WireRequest::decode(request.to_string().as_bytes()).is_err(),
            "accepted {payload}"
        );
    }
    let good = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "r1".into(),
        expected_instance: "00000000-0000-4000-8000-000000000001".into(),
        expected_boot: None,
        output: None,
        command: Command::Health,
    };
    let bytes = serde_json::to_vec(&good).unwrap();
    assert_eq!(WireRequest::decode(&bytes).unwrap(), good);
}

#[test]
fn clock_budget_and_page_bounds_are_independent() {
    struct FakeClock {
        utc: UtcMillis,
        mono: MonoInstant,
    }
    impl Clock for FakeClock {
        fn utc_now(&self) -> UtcMillis {
            self.utc
        }
        fn monotonic_now(&self) -> MonoInstant {
            self.mono
        }
    }
    let clock = FakeClock {
        utc: UtcMillis(9_000),
        mono: MonoInstant(10),
    };
    let budget = CallBudget {
        deadline: MonoInstant(20),
        cancellation: Cancellation::default(),
    };
    assert!(!budget.is_exhausted(&clock));
    assert_eq!(clock.utc_now(), UtcMillis(9_000));
    let later = FakeClock {
        utc: UtcMillis(-1_000),
        mono: MonoInstant(20),
    };
    assert!(budget.is_exhausted(&later));
    budget.cancellation.cancel();
    assert!(budget.is_exhausted(&clock));
    assert_eq!(DEFAULT_PAGE_LIMIT, 20);
    assert_eq!(MAX_PAGE_LIMIT, 100);
    assert_eq!(DEFAULT_PAGE_BYTES, 16_384);
    assert_eq!(MAX_PAGE_BYTES, 65_536);
    assert!(
        PageRequest {
            cursor: None,
            limit: 101,
            max_bytes: 65_536
        }
        .validate()
        .is_err()
    );
    assert!(
        PageRequest {
            cursor: None,
            limit: 0,
            max_bytes: 65_536
        }
        .validate()
        .is_err()
    );
    assert!(
        PageRequest {
            cursor: None,
            limit: 20,
            max_bytes: 65_537
        }
        .validate()
        .is_err()
    );
    assert!(
        PageRequest {
            cursor: Some("x".repeat(1_025)),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}

#[test]
fn wire_rejects_malformed_cursor_in_page_command() {
    let valid_cursor = wire_fixture_cursor(CursorScope::Directory, "all")
        .encode()
        .unwrap();
    let request = wire_fixture(
        serde_json::json!({"kind":"directory","args":{"membership":null,"membership_filter":"all","topic_contains":null,"page":{"cursor":valid_cursor,"limit":20,"max_bytes":16384}}}),
    );
    assert!(WireRequest::decode(request.to_string().as_bytes()).is_ok());
    let too_long = "x".repeat(1025);
    for cursor in ["not-a-cursor", "c1:0", "c1:zz", too_long.as_str()] {
        let mut bad = request.clone();
        bad["command"]["args"]["page"]["cursor"] = serde_json::json!(cursor);
        let error = WireRequest::decode(bad.to_string().as_bytes()).unwrap_err();
        assert!(
            error.to_string().contains("cursor"),
            "wrong rejection for {cursor}: {error}"
        );
    }
    let cursor = Cursor {
        instance: "instance".into(),
        scope: CursorScope::Directory,
        scope_key: "thread:t1".into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 5,
        high_water_ordinal: 9,
        scope_revision: Some(2),
        filter_revision: Some(1),
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    };
    let encoded = cursor.encode().unwrap();
    assert!(
        encoded.starts_with("c3:") && encoded.len() <= 24,
        "{encoded}"
    );
    assert!(Cursor::decode("c1:7b7d").is_err());
    let decoded = Cursor::decode(&encoded).unwrap();
    assert!(decoded.binding.is_some() && decoded.instance.is_empty());
    assert_eq!(
        Cursor::decode_for(
            &encoded,
            "instance",
            CursorScope::Directory,
            "thread:t1",
            "digest",
            CursorDirection::Ascending,
            1
        )
        .unwrap(),
        cursor
    );
    // ht-4is.8.18: the compact cursor is exactly as strict as the legacy one.
    let legacy = cursor.encode_legacy().unwrap();
    assert!(legacy.starts_with("c2:"));
    for raw in [&encoded, &legacy] {
        let parsed = Cursor::decode(raw).unwrap();
        let check = |instance: &str,
                     scope: CursorScope,
                     key: &str,
                     filter: &str,
                     direction: CursorDirection,
                     version: u16| {
            parsed
                .validate_for(instance, scope, key, filter, direction, version)
                .is_ok()
        };
        let asc = CursorDirection::Ascending;
        assert!(check(
            "instance",
            CursorScope::Directory,
            "thread:t1",
            "digest",
            asc,
            1
        ));
        assert!(!check(
            "other",
            CursorScope::Directory,
            "thread:t1",
            "digest",
            asc,
            1
        ));
        assert!(!check(
            "instance",
            CursorScope::Seats,
            "thread:t1",
            "digest",
            asc,
            1
        ));
        assert!(!check(
            "instance",
            CursorScope::Directory,
            "thread:t2",
            "digest",
            asc,
            1
        ));
        assert!(!check(
            "instance",
            CursorScope::Directory,
            "thread:t1",
            "other",
            asc,
            1
        ));
        assert!(!check(
            "instance",
            CursorScope::Directory,
            "thread:t1",
            "digest",
            CursorDirection::Descending,
            1
        ));
        assert!(!check(
            "instance",
            CursorScope::Directory,
            "thread:t1",
            "digest",
            asc,
            2
        ));
    }
    // Every single-character change of the token is refused: malformed, or a
    // position the tag does not bind.
    let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    for index in 3..encoded.len() {
        for replacement in alphabet.chars() {
            let mut forged = encoded.clone();
            if forged[index..].starts_with(replacement) {
                continue;
            }
            forged.replace_range(index..index + 1, &replacement.to_string());
            let accepted = Cursor::decode(&forged).is_ok_and(|c| {
                c.validate_for(
                    "instance",
                    CursorScope::Directory,
                    "thread:t1",
                    "digest",
                    CursorDirection::Ascending,
                    1,
                )
                .is_ok()
            });
            assert!(!accepted, "{forged}");
        }
    }
    assert!(
        cursor
            .validate_for(
                "instance",
                CursorScope::Directory,
                "thread:t1",
                "digest",
                CursorDirection::Ascending,
                1
            )
            .is_ok()
    );
    assert!(
        cursor
            .validate_for(
                "other",
                CursorScope::Directory,
                "thread:t1",
                "digest",
                CursorDirection::Ascending,
                1
            )
            .is_err()
    );
}

#[test]
fn health_distinguishes_software_protocol_boot_and_mismatch() {
    let mut health = Health::unknown(
        "00000000-0000-4000-8000-000000000001".into(),
        "00000000-0000-4000-8000-000000000002".into(),
        "0.1.0".into(),
        1,
    );
    health.limitations.push("host unavailable".into());
    let json = serde_json::to_value(&health).unwrap();
    assert_eq!(json["software_version"], "0.1.0");
    assert_eq!(json["protocol_version"], 1);
    assert_eq!(json["boot_id"], "00000000-0000-4000-8000-000000000002");
    assert!(health.validate().is_ok());
    health.state = herdr_threads::protocol::results::HealthState::Healthy;
    assert!(health.validate().is_err());
    let response =
        WireResponse::version_mismatch("r1".into(), "i".into(), "boot-a".into(), "daemon 0.2.0");
    assert_eq!(
        response.result.unwrap_err().code,
        ErrorCode::DaemonVersionMismatch
    );
}

#[test]
fn direct_request_deserialization_enforces_version_and_page_validation() {
    let good = wire_fixture(serde_json::json!({"kind":"health"}));
    assert!(serde_json::from_value::<WireRequest>(good.clone()).is_ok());
    assert!(WireRequest::decode(good.to_string().as_bytes()).is_ok());
    let mut bad_version = good;
    bad_version["version"] = serde_json::json!(9);
    assert!(
        serde_json::from_value::<WireRequest>(bad_version.clone())
            .unwrap_err()
            .to_string()
            .contains("unknown wire version")
    );
    assert!(WireRequest::decode(bad_version.to_string().as_bytes()).is_err());
    let cursor = wire_fixture_cursor(CursorScope::Directory, "all")
        .encode()
        .unwrap();
    let mut directory = wire_fixture(
        serde_json::json!({"kind":"directory","args":{"membership":null,"membership_filter":"all","topic_contains":null,"page":{"cursor":cursor,"limit":20,"max_bytes":16384}}}),
    );
    assert!(serde_json::from_value::<WireRequest>(directory.clone()).is_ok());
    assert!(WireRequest::decode(directory.to_string().as_bytes()).is_ok());
    directory["command"]["args"]["page"]["cursor"] = serde_json::json!("broken");
    assert!(
        serde_json::from_value::<WireRequest>(directory.clone())
            .unwrap_err()
            .to_string()
            .contains("invalid cursor prefix")
    );
    assert!(WireRequest::decode(directory.to_string().as_bytes()).is_err());
}

#[test]
fn production_entry_rejects_missing_command_without_creating_state() {
    let temp = std::env::temp_dir().join(format!("herdr-threads-contract-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let output = scrubbed_command(env!("CARGO_BIN_EXE_herdr-threads"))
        .current_dir(&temp)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    // Bare invocation prints plain usage, not an error line with a code.
    assert!(!stderr.contains("(invalid_request)"), "{stderr}");
    assert!(!stderr.contains("ApiError"), "{stderr}");
    assert!(stderr.contains("Usage:"), "{stderr}");
    assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0);
    std::fs::remove_dir(temp).unwrap();
}

#[test]
fn empty_opaque_id_is_rejected_at_wire_boundary() {
    let mut request = wire_fixture(
        serde_json::json!({"kind":"operator_fresh_seat","args":{"target":"p1","operation":"o1"}}),
    );
    assert!(WireRequest::decode(request.to_string().as_bytes()).is_ok());
    request["command"]["args"]["target"] = serde_json::json!("");
    assert!(WireRequest::decode(request.to_string().as_bytes()).is_err());
}

#[test]
fn exact_internal_time_and_retirement_limits_are_exposed() {
    use herdr_threads::{
        ports::{RETIREMENT_MAX_UNITS, RETIREMENT_WORK_MILLIS},
        protocol::authority::MAX_PERMIT_MILLIS,
    };
    assert_eq!(MAX_PERMIT_MILLIS, 250);
    assert_eq!(RETIREMENT_MAX_UNITS, 16);
    assert_eq!(RETIREMENT_WORK_MILLIS, 5);
}

#[test]
fn all_command_variants_round_trip_without_actor_fields() {
    use serde_json::{Value, json};
    let p = json!({"cursor":null,"limit":20,"max_bytes":16384});
    let c = json!({"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n1","execution":"e1","target":"p1"});
    let cases: Vec<(&str, Option<Value>)> = vec![
        ("health", None),
        (
            "stop",
            Some(json!({"expected_boot":"00000000-0000-4000-8000-000000000002"})),
        ),
        (
            "directory",
            Some(
                json!({"membership":null,"membership_filter":"default","topic_contains":null,"page":p}),
            ),
        ),
        ("inbox", Some(json!({"seat":null,"page":p}))),
        ("warnings", Some(json!({"seat":"s1","page":p}))),
        ("thread", Some(json!({"thread":"t1","page":p}))),
        (
            "history",
            Some(json!({"thread":"t1","page":p,"initial":null})),
        ),
        ("participants", Some(json!({"thread":"t1","page":p}))),
        ("recipients", Some(json!({"message":"m1","page":p}))),
        (
            "pending_receipts",
            Some(json!({"seat":null,"thread":null,"page":p})),
        ),
        ("local_intents", Some(json!({"page":p}))),
        (
            "search",
            Some(json!({"literal":"term","thread":null,"page":p,"max_candidates":10})),
        ),
        (
            "message",
            Some(json!({"message":"m1","body":{"cursor":null,"offset":null,"max_bytes":16384}})),
        ),
        (
            "diagnostics",
            Some(json!({"seat":null,"thread":null,"page":p})),
        ),
        ("operation_status", Some(json!({"operation":"o1"}))),
        ("retirement_jobs", Some(json!({"page":p}))),
        (
            "resolve_seat",
            Some(json!({"target":"p1","operation":"o1"})),
        ),
        (
            "check_in",
            Some(json!({"mode":{"kind":"current"},"claim":c,"operation":"o1"})),
        ),
        (
            "create_thread",
            Some(json!({"topic":"topic","goal":"goal","operation":"o1","claim":c})),
        ),
        (
            "invite",
            Some(
                json!({"thread":"t1","seat":"s1","deadline_millis":null,"operation":"o1","claim":c}),
            ),
        ),
        (
            "accept",
            Some(json!({"thread":"t1","operation":"o1","claim":c})),
        ),
        (
            "send_message",
            Some(
                json!({"thread":"t1","body":"body","invited_recipients":["s1"],"deadline_millis":null,"operation":"o1","claim":c}),
            ),
        ),
        ("summary", Some(json!({"thread":"t1","claim":c}))),
        (
            "summary_job",
            Some(json!({"job_id":"j1","lease_token":"l1","claim":c})),
        ),
        (
            "summary_submit",
            Some(
                json!({"job_id":"j1","lease_token":"l1","submission":{"submission_schema":1,"narrative":"n","prompt_version":"p","model":"m"},"claim":c}),
            ),
        ),
        (
            "ack",
            Some(json!({"messages":["m1"],"operation":"o1","claim":c})),
        ),
        (
            "leave",
            Some(json!({"thread":"t1","operation":"o1","claim":c})),
        ),
        (
            "set_topic",
            Some(json!({"thread":"t1","topic":"new","operation":"o1","claim":c})),
        ),
        (
            "archive",
            Some(json!({"thread":"t1","operation":"o1","claim":c})),
        ),
        (
            "reopen",
            Some(json!({"thread":"t1","operation":"o1","claim":c})),
        ),
        (
            "operator_rebind",
            Some(json!({"seat":"s1","target":"p1","operation":"o1"})),
        ),
        (
            "operator_fresh_seat",
            Some(json!({"target":"p1","operation":"o1"})),
        ),
        (
            "operator_orphan_invite",
            Some(json!({"thread":"t1","seat":"s1","deadline_millis":null,"operation":"o1"})),
        ),
    ];
    for (kind, args) in cases {
        let mut command = json!({"kind":kind});
        if let Some(args) = args {
            command["args"] = args;
        }
        let raw = json!({"version":PROTOCOL_VERSION,"request_id":"r1","expected_instance":"00000000-0000-4000-8000-000000000001","command":command});
        let parsed: WireRequest =
            serde_json::from_value(raw.clone()).unwrap_or_else(|e| panic!("{kind}: {e}"));
        let out = serde_json::to_value(&parsed).unwrap();
        assert_eq!(out, raw, "{kind}");
    }
}

#[test]
fn send_message_relays_user_is_optional_and_round_trips() {
    use serde_json::json;
    let c = json!({"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n1","execution":"e1","target":"p1"});
    let mut send = json!({"thread":"t1","body":"body","invited_recipients":["s1"],"deadline_millis":null,"operation":"o1","claim":c});
    let Command::SendMessage(plain) =
        serde_json::from_value::<Command>(json!({"kind":"send_message","args":send.clone()}))
            .unwrap()
    else {
        panic!("not a send")
    };
    assert!(!plain.relays_user);
    send["relays_user"] = json!(true);
    let raw = json!({"kind":"send_message","args":send});
    let Command::SendMessage(relayed) = serde_json::from_value::<Command>(raw.clone()).unwrap()
    else {
        panic!("not a send")
    };
    assert!(relayed.relays_user);
    assert_eq!(
        serde_json::to_value(Command::SendMessage(relayed)).unwrap(),
        raw
    );
}

#[test]
fn summary_commands_accept_subagent_claims() {
    use serde_json::json;
    let claim = |role: &str| json!({"instance":"i","seat":"s1","binding_generation":0,"role":role,"harness":"codex","native_session":"n1","execution":"e1","target":"p1"});
    let submission =
        json!({"submission_schema":1,"narrative":"n","prompt_version":"p","model":"m"});
    let commands = [
        json!({"kind":"summary","args":{"thread":"t1","claim":claim("subagent")}}),
        json!({"kind":"summary_job","args":{"job_id":"j1","lease_token":"l1","claim":claim("subagent")}}),
        json!({"kind":"summary_submit","args":{"job_id":"j1","lease_token":"l1","submission":submission,"claim":claim("subagent")}}),
    ];
    for raw in commands {
        let command: Command = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(command.validate(), Ok(()), "{raw}");
    }
    let not_object: Command = serde_json::from_value(
        json!({"kind":"summary_submit","args":{"job_id":"j1","lease_token":"l1","submission":"text","claim":claim("top_level")}}),
    )
    .unwrap();
    assert_eq!(
        not_object.validate(),
        Err("summary submission must be a JSON object")
    );
}

#[test]
fn stop_requires_canonical_daemon_boot_and_no_caller_claim() {
    use herdr_threads::protocol::commands::StopRequest;
    assert!(
        Command::Stop(StopRequest {
            expected_boot: "b1".into()
        })
        .validate()
        .is_err()
    );
    assert!(
        Command::Stop(StopRequest {
            expected_boot: "00000000-0000-4000-8000-000000000002".into()
        })
        .validate()
        .is_ok()
    );
}

#[test]
fn action_formatter_uses_selected_output_shell_quoting() {
    use herdr_threads::protocol::output::format_command_argv;
    let argv = vec![
        "herdr-threads".to_string(),
        "".to_string(),
        "a'b".to_string(),
        "line\nnext".to_string(),
    ];
    assert_eq!(
        format_command_argv(&argv),
        "herdr-threads '' 'a'\\''b' $'line\\nnext'"
    );
}

#[test]
fn action_formatter_round_trips_unicode_separators_through_bash_and_zsh() {
    use herdr_threads::protocol::output::format_command_argv;
    let argv = vec![
        "/tmp/space ' $;\u{0085}\u{2028}\u{2029}".to_owned(),
        "line\nnext".to_owned(),
    ];
    let script = format!(
        "set -- {}; printf '%s\\0' \"$@\"",
        format_command_argv(&argv)
    );
    for shell in ["bash", "zsh"] {
        let output = std::process::Command::new(shell)
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        assert!(output.status.success(), "{shell}");
        let actual: Vec<&[u8]> = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .collect();
        assert_eq!(
            actual,
            argv.iter().map(|arg| arg.as_bytes()).collect::<Vec<_>>(),
            "{shell}"
        );
    }
}

#[test]
fn wire_rejects_unbounded_search_and_recipient_batches() {
    use serde_json::json;
    let p = json!({"cursor":null,"limit":20,"max_bytes":16384});
    let mut search = wire_fixture(
        json!({"kind":"search","args":{"literal":"needle","thread":null,"page":p,"max_candidates":1}}),
    );
    for bound in [1, 100] {
        search["command"]["args"]["max_candidates"] = json!(bound);
        assert!(WireRequest::decode(search.to_string().as_bytes()).is_ok());
    }
    for bound in [0, 101, 65535] {
        search["command"]["args"]["max_candidates"] = json!(bound);
        assert!(
            WireRequest::decode(search.to_string().as_bytes())
                .unwrap_err()
                .to_string()
                .contains("invalid search candidate bound")
        );
    }
    let recipients: Vec<String> = (0..100).map(|n| format!("s{n}")).collect();
    let mut send = wire_fixture(
        json!({"kind":"send_message","args":{"thread":"t1","body":"body","invited_recipients":recipients,"deadline_millis":null,"operation":"o1","claim":{"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n1","execution":"e1","target":"p1"}}}),
    );
    assert!(WireRequest::decode(send.to_string().as_bytes()).is_ok());
    send["command"]["args"]["invited_recipients"]
        .as_array_mut()
        .unwrap()
        .push(json!("s100"));
    assert!(
        WireRequest::decode(send.to_string().as_bytes())
            .unwrap_err()
            .to_string()
            .contains("too many explicit recipients")
    );
}

#[test]
fn all_result_variants_round_trip_with_compact_payloads() {
    use herdr_threads::protocol::results::CommandResult;
    use serde_json::{Value, json};
    let page = json!({"items":[],"next_cursor":null,"next_argv":null,"high_water_ordinal":0,"scope_revision":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live"});
    let summary = json!({"thread":"t1","topic_data":"topic","topic_omitted":false,"topic_detail_argv":null,"archived":false,"orphaned":false,"message_count":0,"created_at":1,"ordinary_count":0,"system_count":0,"joined_count":1});
    let message = json!({"message":"m1","thread":"t1","author":"s1","kind":"ordinary","sequence":1,"created_at":1,"actor_label":null,"preview_data":"preview","preview_omitted":false,"preview_detail_argv":null});
    let cases: Vec<(&str, Value)> = vec![
        (
            "health",
            serde_json::to_value(Health::unknown(
                "00000000-0000-4000-8000-000000000001".into(),
                "00000000-0000-4000-8000-000000000002".into(),
                "0.1.0".into(),
                1,
            ))
            .unwrap(),
        ),
        (
            "stop_accepted",
            json!({"boot_id":"00000000-0000-4000-8000-000000000002"}),
        ),
        ("directory", page.clone()),
        ("inbox", page.clone()),
        ("warnings", page.clone()),
        (
            "thread",
            json!({"summary":summary,"goal_data":"goal","created_at":1,"participant_count":1,"participants":page,"pending_receipt_count":0,"pending_receipts_argv":["pending-receipts","--thread","t1"]}),
        ),
        ("history", page.clone()),
        ("participants", page.clone()),
        ("recipients", page.clone()),
        ("pending_receipts", page.clone()),
        ("local_intents", page.clone()),
        (
            "search",
            json!({"matches":page,"examined_candidates":10,"examined_utf8_bytes":100}),
        ),
        (
            "message",
            json!({"summary":message,"content":{"kind":"ordinary","body_data":"body","body_offset":0,"body_total_bytes":4,"body_complete":true,"body_next_cursor":null,"body_next_argv":null}}),
        ),
        ("diagnostics", page.clone()),
        (
            "operation_status",
            json!({"operation":"o1","committed":true,"result_id":"m1"}),
        ),
        ("retirement_jobs", page.clone()),
        ("seat_resolved", json!("s1")),
        (
            "checked_in",
            json!({"context_disposition":"current","context":{"instance":"i","seat":"s1","binding_generation":1,"role":"top_level","harness":"codex","target":"p1","native_session":"n1","execution":"e1"},"seat":"s1","offered_through":null,"warning_count":0,"warning_count_has_more":false,"warnings":page,"inbox":page}),
        ),
        ("thread_created", json!("t1")),
        ("invitation", json!("i1")),
        ("accepted", json!("i1")),
        ("message_sent", json!("m1")),
        (
            "acknowledged",
            json!({"acknowledged":["m1"],"already_acknowledged":[]}),
        ),
        ("left", json!("t1")),
        ("topic_changed", json!("t1")),
        ("archived", json!("t1")),
        ("reopened", json!("t1")),
        ("operator_rebound", json!("s1")),
        ("operator_fresh_seat", json!("s1")),
        ("operator_invited", json!("i1")),
    ];
    for (kind, data) in cases {
        let raw = json!({"kind":kind,"data":data});
        let parsed: CommandResult =
            serde_json::from_value(raw.clone()).unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert_eq!(serde_json::to_value(parsed).unwrap(), raw, "{kind}");
    }
}

#[test]
fn operator_boundary_only_accepts_three_administrative_commands() {
    use herdr_threads::protocol::{
        commands::{Ack, OperatorCommand, OperatorFreshSeat, OperatorOrphanInvite, OperatorRebind},
        ids::*,
    };
    let native = Command::Ack(Ack {
        messages: vec![MessageId::new("m1")],
        operation: OperationId::new("o1"),
        claim: herdr_threads::protocol::authority::CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("legacy-fixture"),
            binding_generation: 0,
            role: herdr_threads::protocol::authority::CallerRole::TopLevel,
            harness: herdr_threads::protocol::authority::Harness::Codex,
            native_session: NativeSessionId::new("n1"),
            execution: ExecutionId::new("e1"),
            target: HostTargetId::new("p1"),
        },
    });
    assert!(OperatorCommand::try_from(native).is_err());
    for command in [
        Command::OperatorFreshSeat(OperatorFreshSeat {
            target: HostTargetId::new("p1"),
            operation: OperationId::new("o1"),
        }),
        Command::OperatorRebind(OperatorRebind {
            seat: SeatId::new("s1"),
            target: HostTargetId::new("p1"),
            operation: OperationId::new("o1"),
        }),
        Command::OperatorOrphanInvite(OperatorOrphanInvite {
            thread: ThreadId::new("t1"),
            seat: SeatId::new("s1"),
            deadline_millis: None,
            operation: OperationId::new("o1"),
        }),
    ] {
        assert!(OperatorCommand::try_from(command).is_ok());
    }
}

#[test]
fn continuation_cannot_advance_past_its_captured_high_water() {
    let cursor = Cursor {
        instance: "instance".into(),
        scope: CursorScope::History,
        scope_key: "thread:t1".into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 10,
        high_water_ordinal: 9,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    };
    // A compact cursor cannot even express a position past its high water;
    // the legacy form is refused on decode.
    assert!(cursor.encode().is_err());
    assert!(Cursor::decode(&cursor.encode_legacy().unwrap()).is_err());
}

#[test]
fn retirement_status_limits_last_error_size() {
    use herdr_threads::protocol::results::RetirementStatus;
    let raw = serde_json::json!({
        "job":"j1","seat":"s1","effective_retired":true,"retired_at":1,
        "cleanup_state":"running","warning_history_complete":false,"phase":"warnings",
        "processed_units":1,"remaining_estimate":null,"last_error":"x".repeat(513)
    });
    assert!(serde_json::from_value::<RetirementStatus>(raw).is_err());
}

#[test]
fn command_envelope_rejects_extra_actor_and_time_fields() {
    for payload in [
        r#"{"version":1,"request_id":"r1","command":{"kind":"health","verified_actor":{"seat":"s1"}}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"ack","args":{"messages":["m1"],"operation":"o1","claim":{"instance":"i","seat":"s1","binding_generation":0,"role":"top_level","harness":"codex","native_session":"n1","execution":"e1","target":"p1"}},"operator_actor":{"uid":0}}}"#,
        r#"{"version":1,"request_id":"r1","command":{"kind":"health","trusted_decision_at":123}}"#,
    ] {
        // The fixtures spell version 1; stamp the current protocol so only the
        // field under test (or the explicit version 99) is invalid.
        let payload = payload.replacen(
            "\"version\":1,",
            &format!("\"version\":{PROTOCOL_VERSION},"),
            1,
        );
        let mut request: serde_json::Value = serde_json::from_str(&payload).unwrap();
        request["expected_instance"] = serde_json::json!(WIRE_FIXTURE_INSTANCE);
        let mut control = request.clone();
        for field in ["verified_actor", "operator_actor", "trusted_decision_at"] {
            control["command"].as_object_mut().unwrap().remove(field);
        }
        assert!(
            WireRequest::decode(control.to_string().as_bytes()).is_ok(),
            "invalid control for {payload}"
        );
        assert!(
            WireRequest::decode(request.to_string().as_bytes())
                .unwrap_err()
                .to_string()
                .contains("unknown command envelope field")
        );
    }
}

#[test]
fn message_body_read_requires_explicit_position_and_encoded_budget() {
    let mut request = wire_fixture(
        serde_json::json!({"kind":"message","args":{"message":"m1","body":{"cursor":null,"offset":0,"max_bytes":16384}}}),
    );
    assert!(WireRequest::decode(request.to_string().as_bytes()).is_ok());
    request["command"]["args"]
        .as_object_mut()
        .unwrap()
        .remove("body");
    assert!(
        WireRequest::decode(request.to_string().as_bytes())
            .unwrap_err()
            .to_string()
            .contains("missing field `body`")
    );
}

#[test]
fn body_continuation_is_message_scoped_and_diagnostics_has_distinct_scope() {
    use serde_json::json;
    let cursor = Cursor {
        instance: "instance".into(),
        scope: CursorScope::MessageBody,
        scope_key: "m1".into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 512,
        high_water_ordinal: 1024,
        scope_revision: None,
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    }
    .encode()
    .unwrap();
    let request = |message: &str, cursor: &str, max_bytes: u32| {
        json!({
            "version":PROTOCOL_VERSION,"request_id":"r1","expected_instance":"00000000-0000-4000-8000-000000000001","command":{"kind":"message","args":{
                "message":message,"body":{"cursor":cursor,"offset":null,"max_bytes":max_bytes}
            }}
        })
    };
    assert!(WireRequest::decode(request("m1", &cursor, 16384).to_string().as_bytes()).is_ok());
    assert!(WireRequest::decode(request("m2", &cursor, 16384).to_string().as_bytes()).is_err());
    assert!(WireRequest::decode(request("m1", &cursor, 65537).to_string().as_bytes()).is_err());
    let history_cursor = Cursor {
        scope: CursorScope::History,
        ..Cursor::decode_for(
            &cursor,
            "instance",
            CursorScope::MessageBody,
            "m1",
            "digest",
            CursorDirection::Ascending,
            1,
        )
        .unwrap()
    }
    .encode()
    .unwrap();
    assert!(
        WireRequest::decode(request("m1", &history_cursor, 16384).to_string().as_bytes()).is_err()
    );
    let diagnostic = Cursor {
        instance: "instance".into(),
        scope: CursorScope::Diagnostics,
        scope_key: "seat:s1".into(),
        filter_digest: "digest".into(),
        direction: CursorDirection::Ascending,
        order_version: 1,
        last_examined_key: None,
        after_ordinal: 1,
        high_water_ordinal: 5,
        scope_revision: Some(1),
        filter_revision: None,
        search: None,
        inbox: None,
        attention: None,
        binding: None,
    };
    let parsed = Cursor::decode(&diagnostic.encode().unwrap()).unwrap();
    assert_eq!(parsed.scope, CursorScope::Diagnostics);
}

#[test]
fn ordinary_seat_resolve_cannot_enter_native_permit_mutation_route() {
    use herdr_threads::protocol::{
        commands::{PermitMutation, ResolveSeat},
        ids::HostTargetId,
    };
    let ordinary = Command::ResolveSeat(ResolveSeat {
        target: HostTargetId::new("p1"),
        operation: herdr_threads::protocol::ids::OperationId::new("o1"),
    });
    assert!(PermitMutation::try_from(ordinary).is_err());
}

#[test]
fn diagnostics_request_rejects_another_query_scope() {
    let cursor = wire_fixture_cursor(CursorScope::Diagnostics, "all");
    let mut request = wire_fixture(
        serde_json::json!({"kind":"diagnostics","args":{"seat":null,"thread":null,"page":{"cursor":cursor.encode().unwrap(),"limit":20,"max_bytes":16384}}}),
    );
    assert!(WireRequest::decode(request.to_string().as_bytes()).is_ok());
    let wrong_scope = Cursor {
        scope: CursorScope::Directory,
        ..cursor
    };
    let encoded = wrong_scope.encode().unwrap();
    assert!(Cursor::decode(&encoded).is_ok());
    request["command"]["args"]["page"]["cursor"] = serde_json::json!(encoded);
    assert!(
        WireRequest::decode(request.to_string().as_bytes())
            .unwrap_err()
            .to_string()
            .contains("diagnostics cursor scope mismatch")
    );
}

#[test]
fn cooperative_check_in_wire_preserves_context_and_rejects_declared_children() {
    use herdr_threads::protocol::commands::Command;
    let wire = serde_json::json!({"kind":"check_in", "args": {
        "claim": {"instance":"i", "seat":"s", "binding_generation":0,
            "role":"top_level", "harness":"codex", "target":"p",
            "native_session":"plugin_context:n", "execution":"e"},
        "mode":{"kind":"lifecycle", "expected_binding_generation":0},
        "operation":"op"
    }});
    let command: Command =
        serde_json::from_value(wire.clone()).expect("cooperative CheckIn context is supported");
    assert!(command.validate().is_ok());
    assert_eq!(serde_json::to_value(command).unwrap(), wire);
    let mut child = wire;
    child["args"]["claim"]["role"] = serde_json::json!("subagent");
    let command: Command = serde_json::from_value(child).unwrap();
    assert!(command.validate().is_err());
}

// Wave-2 (a) wire contract: inbox items and check-in results carry explicit
// `has_more` flags beside their capped pending-warning counts, and results
// recorded before the flags existed (cached check-ins) still decode, as "not
// more". Kills: dropping either flag from the wire, or requiring it on decode.
#[test]
fn pending_warning_count_flags_are_on_the_wire_and_default_to_false() {
    use herdr_threads::protocol::results::{InboxItem, MAX_PENDING_WARNING_COUNT};
    assert_eq!(MAX_PENDING_WARNING_COUNT, 1_000);
    let item: InboxItem = serde_json::from_value(serde_json::json!({
        "thread": "t", "invitations": 0, "pending_receipts": 0, "warnings": 1000,
        "warnings_has_more": true
    }))
    .unwrap();
    assert!(item.warnings_has_more);
    assert_eq!(
        serde_json::to_value(&item).unwrap()["warnings_has_more"],
        serde_json::json!(true)
    );
    let legacy: InboxItem = serde_json::from_value(serde_json::json!({
        "thread": "t", "invitations": 0, "pending_receipts": 0, "warnings": 2
    }))
    .unwrap();
    assert!(!legacy.warnings_has_more);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap()["warnings_has_more"],
        serde_json::json!(false)
    );
    let page = serde_json::json!({"items":[],"next_cursor":null,"next_argv":null,"has_more":false,"stop_reason":"complete","consistency":"bounded_live","high_water_ordinal":0,"scope_revision":null});
    let checked_in: herdr_threads::protocol::results::CheckInResult =
        serde_json::from_value(serde_json::json!({"context_disposition":"current","context":{"instance":"i","seat":"s1","binding_generation":1,"role":"top_level","harness":"codex","target":"p1","native_session":"n1","execution":"e1"},"seat":"s1","offered_through":null,"warning_count":3,"warnings":page,"inbox":page}))
            .unwrap();
    assert_eq!(
        (checked_in.warning_count, checked_in.warning_count_has_more),
        (3, false)
    );
}

#[test]
fn message_summary_author_fields_default_and_round_trip() {
    use herdr_threads::protocol::{results::MessageSummary, summary::AuthorRole};
    let mut raw = serde_json::json!({
        "message":"m","thread":"t","author":"s","kind":"ordinary",
        "sequence":1,"created_at":1,"actor_label":null,"preview_data":"short",
        "preview_omitted":false,"preview_detail_argv":null
    });
    let old: MessageSummary = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(old.author_role, None);
    assert!(!old.relays_user && !old.author_role_backfilled);
    assert_eq!(serde_json::to_value(&old).unwrap(), raw);
    raw["author_role"] = serde_json::json!("human");
    raw["relays_user"] = serde_json::json!(true);
    raw["author_role_backfilled"] = serde_json::json!(true);
    let new: MessageSummary = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(new.author_role, Some(AuthorRole::Human));
    assert!(new.relays_user && new.author_role_backfilled);
    assert_eq!(serde_json::to_value(&new).unwrap(), raw);
}

// Kills: a hint-less Accepted that no longer serializes as the bare
// invitation id (stored operation results and journals would stop parsing), a
// hinted one that loses its thread, or a legacy stored string that stops
// deserializing.
#[test]
fn accepted_without_hint_is_the_bare_invitation_id_and_with_hint_round_trips() {
    use herdr_threads::protocol::{
        ids::{InvitationId, ThreadId},
        results::{AcceptedInvitation, CommandResult},
    };
    let bare = CommandResult::Accepted(InvitationId::new("inv-1").into());
    let encoded = serde_json::to_value(&bare).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({"kind":"accepted","data":"inv-1"})
    );
    // A stored result from before the hint existed.
    let legacy: CommandResult =
        serde_json::from_str(r#"{"kind":"accepted","data":"inv-1"}"#).unwrap();
    assert_eq!(legacy, bare);
    let hinted = CommandResult::Accepted(AcceptedInvitation {
        invitation: InvitationId::new("inv-1"),
        summary_available: Some(ThreadId::new("t-9")),
    });
    let encoded = serde_json::to_value(&hinted).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({"kind":"accepted","data":{"invitation":"inv-1","summary_available":"t-9"}})
    );
    assert_eq!(
        serde_json::from_value::<CommandResult>(encoded).unwrap(),
        hinted
    );
    // The object form without a hint is accepted too; unknown fields are not.
    let object: CommandResult =
        serde_json::from_str(r#"{"kind":"accepted","data":{"invitation":"inv-1"}}"#).unwrap();
    assert_eq!(object, bare);
    assert!(
        serde_json::from_str::<CommandResult>(
            r#"{"kind":"accepted","data":{"invitation":"inv-1","extra":1}}"#
        )
        .is_err()
    );
}

// Kills: wire types that drift (the hot query and its result must round trip),
// a limit outside 1..=8 reaching the store, or an unbounded overflow.
#[test]
fn hot_threads_wire_types_round_trip_and_bound_the_limit() {
    use herdr_threads::protocol::{
        commands::{Command, HotThreadsQuery},
        ids::{SeatId, ThreadId},
        results::{CommandResult, HotReason, HotThread, HotThreads},
    };
    let query = Command::HotThreads(HotThreadsQuery {
        seat: SeatId::new("s"),
        limit: 8,
    });
    let encoded = serde_json::to_value(&query).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({"kind":"hot_threads","args":{"seat":"s","limit":8}})
    );
    assert_eq!(serde_json::from_value::<Command>(encoded).unwrap(), query);
    assert!(query.validate().is_ok());
    for limit in [0, 9] {
        let bad = Command::HotThreads(HotThreadsQuery {
            seat: SeatId::new("s"),
            limit,
        });
        assert!(bad.validate().is_err(), "limit {limit}");
    }
    let result = CommandResult::HotThreads(HotThreads {
        hot: vec![HotThread {
            thread: ThreadId::new("t1"),
            topic_data: "topic".into(),
            reason: HotReason::PendingReceipt,
            effective_deadline: Some(UtcMillis(7)),
            last_activity: UtcMillis(5),
        }],
        overflow: vec![ThreadId::new("t2")],
    });
    let encoded = serde_json::to_value(&result).unwrap();
    assert_eq!(encoded["data"]["hot"][0]["reason"], "pending_receipt");
    assert_eq!(
        serde_json::from_value::<CommandResult>(encoded).unwrap(),
        result
    );
}

#[test]
fn deadline_extension_fields_are_additive_on_pending_receipts_and_recipients() {
    use herdr_threads::protocol::results::{PendingReceipt, Recipient};
    // Old JSON without the new fields parses and serializes unchanged.
    let old = serde_json::json!({"message":"m","thread":"t","seat":"s","sequence":4,"sender":"author","decision_at":5,"available_at":0,"deadline":60_000,"overdue":true});
    let parsed: PendingReceipt = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(parsed.effective_deadline, None);
    assert_eq!(parsed.deferred_until, None);
    assert_eq!(serde_json::to_value(parsed).unwrap(), old);
    // New fields round-trip.
    let extended = serde_json::json!({"message":"m","thread":"t","seat":"s","sequence":4,"sender":"author","decision_at":5,"available_at":0,"deadline":60_000,"overdue":false,"effective_deadline":120_000,"deferred_until":120_000});
    let parsed: PendingReceipt = serde_json::from_value(extended.clone()).unwrap();
    assert_eq!(parsed.deferred_until.map(|at| at.0), Some(120_000));
    assert_eq!(serde_json::to_value(parsed).unwrap(), extended);
    let old = serde_json::json!({"seat":"s","status":"pending","physical_status":"pending","effective_status":"pending","retirement_cutover":null,"cleanup_state":null,"ack_provenance":null});
    let parsed: Recipient = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(parsed.deadline, None);
    assert_eq!(serde_json::to_value(parsed).unwrap(), old);
    let extended = serde_json::json!({"seat":"s","status":"pending","physical_status":"pending","effective_status":"pending","retirement_cutover":null,"cleanup_state":null,"ack_provenance":null,"deadline":60_000,"effective_deadline":120_000,"deferred_until":120_000});
    let parsed: Recipient = serde_json::from_value(extended.clone()).unwrap();
    assert_eq!(parsed.effective_deadline.map(|at| at.0), Some(120_000));
    assert_eq!(serde_json::to_value(parsed).unwrap(), extended);
}
