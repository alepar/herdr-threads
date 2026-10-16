use super::*;
use crate::protocol::{
    output::OutputSpec,
    results::{CommandResult, ErrorCode, Health},
};

fn health() -> CommandResult {
    let mut health = Health::unknown("instance".into(), "boot".into(), "v\n雪".into(), 1);
    health.limitations.push("\u{001b}[31m".into());
    CommandResult::Health(health)
}

#[test]
fn writer_emits_exact_selected_bytes_and_flushes() {
    let selected =
        crate::protocol::output::encode_selected(&health(), &OutputSpec::default()).unwrap();
    let mut sink = FlushSink::default();
    let written = write_selected(
        &health(),
        &OutputSpec::default(),
        selected.len() as u32,
        &mut sink,
    )
    .unwrap();
    assert_eq!(written, selected.len());
    assert_eq!(sink.bytes, selected);
    assert_eq!(sink.flushes, 1);
}

#[test]
fn actual_escaped_minimum_is_reported_without_partial_output() {
    let selected =
        crate::protocol::output::encode_selected(&health(), &OutputSpec::default()).unwrap();
    let mut sink = FlushSink::default();
    let error = write_selected(
        &health(),
        &OutputSpec::default(),
        (selected.len() - 1) as u32,
        &mut sink,
    )
    .unwrap_err();
    assert!(
        matches!(error, OutputError::Api(ref api) if api.code == ErrorCode::InvalidBudget && api.required_minimum_bytes == Some(selected.len() as u32))
    );
    assert!(sink.bytes.is_empty());
}

#[test]
fn write_and_flush_failures_propagate() {
    let mut sink = FlushSink {
        fail_write: true,
        ..Default::default()
    };
    assert!(matches!(
        write_selected(&health(), &OutputSpec::default(), 1000, &mut sink),
        Err(OutputError::Io(_))
    ));
    let mut sink = FlushSink {
        fail_flush: true,
        ..Default::default()
    };
    assert!(matches!(
        write_selected(&health(), &OutputSpec::default(), 1000, &mut sink),
        Err(OutputError::Io(_))
    ));
}

#[test]
fn oversized_unicode_body_is_rejected_before_writing() {
    use crate::protocol::{
        ids::{MessageId, ThreadId},
        results::{MessageContent, MessageDetails, MessageKind, MessageSummary},
        time::UtcMillis,
    };
    let body = "雪".repeat(22_000);
    let result = CommandResult::Message(MessageDetails {
        summary: MessageSummary {
            message: MessageId::new("m1"),
            thread: ThreadId::new("t1"),
            author: None,
            event_author: None,
            author_role: None,
            relays_user: false,
            user_intent: None,
            author_role_backfilled: false,
            kind: MessageKind::Ordinary,
            sequence: 7,
            created_at: UtcMillis(1),
            actor_label: None,
            preview_data: "".into(),
            preview_omitted: false,
            preview_detail_argv: None,
        },
        content: MessageContent::Ordinary {
            body_data: body,
            body_offset: 0,
            body_total_bytes: 66_000,
            body_complete: false,
            body_next_cursor: None,
            body_next_argv: Some(vec![
                "herdr-threads".into(),
                "body".into(),
                "m1".into(),
                "--offset".into(),
                "66000".into(),
            ]),
        },
    });
    let mut sink = FlushSink::default();
    let error = write_selected(&result, &OutputSpec::default(), 65_536, &mut sink).unwrap_err();
    assert!(
        matches!(error, OutputError::Api(ref api) if api.code == ErrorCode::InvalidBudget && api.required_minimum_bytes.unwrap() > 65_536)
    );
    assert!(sink.bytes.is_empty());
}

#[test]
fn history_continuation_is_emitted_and_parseable() {
    use crate::protocol::{
        pagination::{Consistency, Page, StopReason},
        results::MessageSummary,
    };
    let argv = vec![
        "herdr-threads".into(),
        "read".into(),
        "t1".into(),
        "--limit".into(),
        "7".into(),
    ];
    let result = CommandResult::History(Page::<MessageSummary> {
        items: vec![],
        next_cursor: Some("opaque".into()),
        next_argv: Some(argv.clone()),
        high_water_ordinal: 205,
        scope_revision: None,
        has_more: true,
        stop_reason: StopReason::Work,
        consistency: Consistency::BoundedLive,
    });
    let mut sink = FlushSink::default();
    write_selected(&result, &OutputSpec::default(), 4096, &mut sink).unwrap();
    let selected: serde_json::Value = serde_json::from_slice(&sink.bytes).unwrap();
    assert_eq!(
        selected["result"]["data"]["next_argv"],
        serde_json::json!(argv)
    );
    assert!(matches!(
        crate::cli::commands::parse_argv(argv).unwrap().action,
        crate::cli::commands::CliAction::Wire(crate::protocol::commands::Command::History(_))
    ));
}

// Wave 21. Kills: selecting by isatty alone (a PTY harness gets the human
// form), a marker that also beats `--human`, a marker list that matches an
// empty value, and a library caller that never recorded a terminal going human.
#[test]
fn harness_markers_select_agent_output_even_on_a_tty() {
    let spec = OutputSpec {
        format: crate::protocol::output::OutputFormat::Text,
        ..OutputSpec::default()
    };
    let human = |presentation| {
        let guard = PresentationGuard::enter(presentation, &spec);
        let active = human_active();
        drop(guard);
        active
    };
    set_stdout_is_terminal(true);
    set_harness_marked(false);
    assert!(human(Presentation::Auto), "a person on a terminal");
    set_harness_marked(true);
    assert!(!human(Presentation::Auto), "marked PTY harness is an agent");
    assert!(human(Presentation::Human), "--human still wins");
    assert!(!human(Presentation::Machine));
    set_stdout_is_terminal(false);
    assert!(!human(Presentation::Auto));
    set_harness_marked(false);
    set_stdout_is_terminal(false);

    let env = |set: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| std::ffi::OsString::from(value))
        }
    };
    assert!(!harness_marked(env(&[])));
    assert!(!harness_marked(env(&[("CLAUDECODE", "")])));
    assert!(!harness_marked(env(&[("TERM", "xterm"), ("HOME", "/h")])));
    for marker in [
        "CLAUDECODE",
        "CODEX_THREAD_ID",
        "CODEX_SANDBOX",
        "CODEX_SANDBOX_NETWORK_DISABLED",
    ] {
        let set: &'static [(&'static str, &'static str)] =
            Box::leak(vec![(marker, "1")].into_boxed_slice());
        assert!(harness_marked(env(set)), "{marker}");
    }
    assert!(!harness_marked(env(&[("CODEX_HOME", "/x")])));
}

#[derive(Default)]
struct FlushSink {
    bytes: Vec<u8>,
    flushes: usize,
    fail_write: bool,
    fail_flush: bool,
}
impl std::io::Write for FlushSink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.fail_write {
            return Err(std::io::Error::other("closed"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        if self.fail_flush {
            return Err(std::io::Error::other("flush failed"));
        }
        Ok(())
    }
}

#[test]
fn invitation_policy_compact_keeps_lazy_discoverable_and_timed_deadline_honest() {
    use crate::protocol::{
        commands::InvitationConfirmation,
        ids::{InvitationId, ThreadId},
        pagination::{Consistency, Page, StopReason},
        results::InboxBatchItem,
        time::UtcMillis,
    };
    for (mode, deadline, expected) in [
        (
            Some(InvitationConfirmation::Lazy),
            None,
            "confirmation: lazy",
        ),
        (
            Some(InvitationConfirmation::RequireAck),
            Some(UtcMillis(7000)),
            "confirmation: acceptance required",
        ),
        (None, None, "confirmation: acceptance required"),
    ] {
        let result = CommandResult::InboxBatch(Page {
            items: vec![InboxBatchItem::Invitation {
                thread: ThreadId::new("thread-1"),
                topic_data: "topic".into(),
                goal_data: Some("goal".into()),
                invitation: InvitationId::new("invitation-1"),
                confirmation: mode,
                deadline_at: deadline,
                required_service: None,
            }],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        });
        let spec = OutputSpec {
            format: crate::protocol::output::OutputFormat::Text,
            ..OutputSpec::default()
        };
        let bytes = crate::protocol::output::encode_selected(&result, &spec).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(expected), "{text}");
        assert!(text.contains("invitation-1 thread-1"), "{text}");
        assert!(text.contains("accept thread-1"), "{text}");
        assert_eq!(text.contains("deadline"), deadline.is_some(), "{text}");
        let bytes =
            crate::protocol::output::encode_selected(&result, &OutputSpec::default()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let item = &json["result"]["data"]["items"][0];
        match mode {
            Some(InvitationConfirmation::Lazy) => assert_eq!(item["confirmation"], "lazy"),
            Some(InvitationConfirmation::RequireAck) => {
                assert_eq!(item["confirmation"], "require_ack")
            }
            None => assert!(item.get("confirmation").is_none()),
        }
        assert_eq!(
            item.get("deadline_at").and_then(serde_json::Value::as_i64),
            deadline.map(|v| v.0)
        );
    }
}

#[test]
fn invitation_policy_lazy_hook_display_remains_recoverable_without_acceptance() {
    use crate::protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::InvitationConfirmation,
        ids::*,
        pagination::{Consistency, Page, StopReason},
        results::InboxBatchV2Item,
    };
    let offer = crate::cli::hook_inbox::InboxOffer {
        receipt_completion: true,
        in_turn_offer: false,
        claim: CallerClaim {
            instance: "instance".into(),
            seat: SeatId::new("seat"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("session"),
            execution: ExecutionId::new("execution"),
            target: HostTargetId::new("w:p"),
            agent_id: None,
            liveness: None,
        },
        page: Page {
            items: vec![InboxBatchV2Item::Invitation {
                thread: ThreadId::new("thread-1"),
                topic_data: "topic".into(),
                goal_data: None,
                topic_revision: None,
                invitation: InvitationId::new("invitation-1"),
                confirmation: Some(InvitationConfirmation::Lazy),
                deadline_at: None,
                required_service: None,
            }],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: false,
            stop_reason: StopReason::Complete,
            consistency: Consistency::BoundedLive,
        },
        cursors: false,
    };
    let (context, completions) =
        crate::cli::hook_inbox::append(String::new(), &offer, &["herdr-threads".into()], 4096);
    assert!(context.contains("invitation-1 thread-1"), "{context}");
    assert!(context.contains("confirmation: lazy"), "{context}");
    assert!(context.contains("accept thread-1"), "{context}");
    assert!(
        completions.is_empty(),
        "invitation presentation must not settle membership or messages"
    );
}

#[test]
fn read_modes_annotation_len_is_the_prepended_bytes() {
    use crate::protocol::ids::MessageId;
    let modes = ReadModes {
        marked: vec![
            (MessageId::new("msg-2"), "[p0]"),
            (MessageId::new("msg-3"), "[lazy]"),
        ],
        enforce_final_bound: false,
    };
    assert_eq!(modes.annotate(b"x".to_vec()).len(), 1 + modes.annotation_len());
    assert_eq!(modes.annotation_len(), 11 + 13);
    assert_eq!(ReadModes::default().annotation_len(), 0);
}

// Kills (ht-yha.66 M2): a `[p0] <id>` row for a recorded-ordinary message (pre-upgrade, its p0
// implied by its mode; or with explicit ACK recipients, whose ACK line already marks it), a
// marker for a default-priority delivery-row send, or an annotation length that counts the
// dropped rows.
#[test]
fn read_modes_mark_only_delivery_row_sends_with_a_non_default_priority() {
    use crate::protocol::{
        commands::DeliveryMode, ids::MessageId, priority::DeliveryPriority,
        results::MessageDeliveryMode,
    };
    let mode = |id: &str, delivery_mode, priority| MessageDeliveryMode {
        message: MessageId::new(id),
        delivery_mode,
        priority: Some(DeliveryPriority::new(priority).unwrap()),
    };
    let modes = ReadModes {
        marked: ReadModes::rows(vec![
            mode("msg-old", DeliveryMode::Ordinary, 0),
            mode("msg-ack", DeliveryMode::Ordinary, 0),
            mode("msg-p0", DeliveryMode::Lazy, 0),
            mode("msg-p3", DeliveryMode::Lazy, 3),
            mode("msg-p1", DeliveryMode::Lazy, 1),
        ])
        .collect(),
        enforce_final_bound: false,
    };
    assert_eq!(
        modes.marked,
        vec![
            (MessageId::new("msg-p0"), "[p0]"),
            (MessageId::new("msg-p3"), "[p3]"),
        ]
    );
    assert_eq!(
        modes.annotate(b"x".to_vec()),
        b"[p0] msg-p0\n[p3] msg-p3\nx".to_vec()
    );
    assert_eq!(modes.annotation_len(), 12 + 12);
}
