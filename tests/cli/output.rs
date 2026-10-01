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
