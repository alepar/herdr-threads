use super::*;
use crate::cli::retry::run_retry;
use crate::protocol::{
    authority::{CallerClaim, Harness},
    commands::Command,
    ids::{ExecutionId, HostTargetId, MessageId, NativeSessionId, SeatId, ThreadId},
    output::{OutputFormat, OutputSpec},
    results::CommandResult,
};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn temp() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("herdr-intent-test-{}", uuid::Uuid::new_v4()))
}
fn claim() -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("legacy-fixture"),
        binding_generation: 0,
        role: crate::protocol::authority::CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("session-secret"),
        execution: ExecutionId::new("execution-secret"),
        target: HostTargetId::new("target-secret"),
    }
}
fn send() -> SemanticMutation {
    SemanticMutation::SendMessage {
        thread: ThreadId::new("thread-1"),
        body: "body secret\n".into(),
        invited_recipients: vec![],
        deadline_millis: None,
    }
}
fn scope() -> IntentScope {
    IntentScope::Native {
        instance: "instance-1".into(),
        seat: SeatId::new("seat-1"),
    }
}

#[test]
fn pending_header_has_typed_local_token_and_never_reads_body() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 123).unwrap();
    let raw = std::fs::read_to_string(journal.path(&reference)).unwrap();
    std::fs::write(
        journal.path(&reference),
        raw.replace("body secret", "body edited"),
    )
    .unwrap();
    let page = journal.page(&Default::default()).unwrap();
    let item = &page.items[0];
    assert_eq!(item.operation, reference.operation);
    assert_eq!(item.recovery_ref.as_str(), "local:1");
    assert_eq!(item.created_at.0, 123);
    assert_eq!(item.kind, crate::protocol::results::IntentKind::SendMessage);
    assert_eq!(item.status, crate::protocol::results::IntentStatus::Pending);
    assert_eq!(item.thread, Some(ThreadId::new("thread-1")));
    assert!(
        journal
            .resolve_recovery_ref(item.recovery_ref.as_str())
            .is_err()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn resolve_seat_uses_only_service_allocation_scope_and_stable_key() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let target = HostTargetId::new("target-1");
    let service = IntentScope::ServiceAllocation {
        instance: "instance-1".into(),
        target: target.clone(),
    };
    let semantic = SemanticMutation::ResolveSeat {
        target: target.clone(),
    };
    assert!(journal.record(scope(), semantic.clone(), 1).is_err());
    let reference = journal.record(service.clone(), semantic, 1).unwrap();
    let result = run_retry(
        &journal,
        &reference,
        &service,
        || panic!("resolution cannot request native proof"),
        |command| {
            let Command::ResolveSeat(resolve) = command else {
                panic!("wrong command")
            };
            assert_eq!(resolve.target, target);
            assert_eq!(resolve.operation, reference.operation);
            Ok(CommandResult::SeatResolved(SeatId::new("seat-1")))
        },
        |_| Err(io::Error::new(io::ErrorKind::BrokenPipe, "lost output")),
    );
    assert!(result.is_err());
    assert_eq!(
        journal.load(&reference).unwrap().operation,
        reference.operation
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn page_budget_uses_selected_text_bytes_and_reports_exact_minimum() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    let prefix = vec!["pending-ops".into()];
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..Default::default()
    };
    let request = crate::protocol::pagination::PageRequest {
        cursor: None,
        limit: 1,
        max_bytes: 256,
    };
    let result = journal.page_with_output(&request, &prefix, &spec);
    match result {
        Ok(page) => {
            let encoded =
                crate::protocol::output::encode_selected(&CommandResult::LocalIntents(page), &spec)
                    .unwrap();
            assert!(encoded.len() <= request.max_bytes as usize);
        }
        Err(error) => {
            assert_eq!(
                error.code,
                crate::protocol::results::ErrorCode::InvalidBudget
            );
            assert!(error.required_minimum_bytes.unwrap() > request.max_bytes);
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn encoded_writer_flush_failure_keeps_exact_pending_operation() {
    struct FlushFails(Vec<u8>);
    impl io::Write for FlushFails {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"))
        }
    }
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 123).unwrap();
    let mut writer = FlushFails(Vec::new());
    let result = crate::cli::retry::run_retry_to_writer(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |_| Ok(CommandResult::MessageSent(MessageId::new("message-1"))),
        &OutputSpec::default(),
        &mut writer,
    );
    assert!(result.is_err());
    assert!(!writer.0.is_empty());
    assert_eq!(
        journal.load(&reference).unwrap().operation,
        reference.operation
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_unknown_outcome_keeps_key_until_explicit_retry() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 123).unwrap();
    let calls = AtomicUsize::new(0);
    let mut output = Vec::new();
    let error = crate::cli::retry::run_retry_api_to_writer(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |command| {
            calls.fetch_add(1, Ordering::SeqCst);
            let Command::SendMessage(send) = command else {
                panic!("wrong command")
            };
            assert_eq!(send.operation, reference.operation);
            Err(crate::protocol::results::ApiError {
                code: crate::protocol::results::ErrorCode::UnknownOutcome,
                detail: "response lost".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            })
        },
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap_err();
    assert!(
        matches!(error, crate::cli::retry::RetryFailure::Submit(ref api) if api.code == crate::protocol::results::ErrorCode::UnknownOutcome)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(output.is_empty());
    assert_eq!(
        journal.load(&reference).unwrap().operation,
        reference.operation
    );
    let result = crate::cli::retry::run_retry_api_to_writer(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |command| {
            let Command::SendMessage(send) = command else {
                panic!("wrong command")
            };
            assert_eq!(send.operation, reference.operation);
            Ok(CommandResult::MessageSent(MessageId::new("message-1")))
        },
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap();
    assert_eq!(
        result,
        CommandResult::MessageSent(MessageId::new("message-1"))
    );
    assert!(journal.load(&reference).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_pre_submission_error_preserves_its_code_and_pending_key() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 123).unwrap();
    let mut output = Vec::new();
    let error = crate::cli::retry::run_retry_api_to_writer(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |_| {
            Err(crate::protocol::results::ApiError {
                code: crate::protocol::results::ErrorCode::HostUnavailable,
                detail: "connect refused".into(),
                restart_argv: None,
                required_minimum_bytes: None,
            })
        },
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap_err();
    assert!(
        matches!(error, crate::cli::retry::RetryFailure::Submit(ref api) if api.code == crate::protocol::results::ErrorCode::HostUnavailable)
    );
    assert!(output.is_empty());
    assert_eq!(
        journal.load(&reference).unwrap().operation,
        reference.operation
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_new_path_refuses_submit_when_publication_fails() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    std::fs::write(dir.join("next-ordinal"), b"corrupt\n").unwrap();
    let mut output = Vec::new();
    let result = crate::cli::retry::run_new_api_to_writer(
        &journal,
        scope(),
        send(),
        1,
        || Ok(claim()),
        |_| -> Result<CommandResult, crate::protocol::results::ApiError> {
            panic!("must not submit")
        },
        &OutputSpec::default(),
        &mut output,
    );
    assert!(matches!(
        result,
        Err(crate::cli::retry::RetryFailure::Local(_))
    ));
    assert!(output.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn duplicate_state_selector_cannot_create_ambiguous_local_cursor_scope() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    let prefix = vec![
        "herdr-threads".into(),
        "--state-dir".into(),
        "one".into(),
        "--state-dir".into(),
        "two".into(),
        "pending-ops".into(),
    ];
    let spec = OutputSpec {
        context: crate::protocol::output::ContinuationContext {
            state_dir: Some("one".into()),
            host: None,
        },
        ..Default::default()
    };
    let error = journal
        .page_with_output(&Default::default(), &prefix, &spec)
        .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn json_pending_page_requires_json_in_exact_continuation_context() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    let prefix = vec!["pending-ops".into()];
    let error = journal
        .page_with_output(&Default::default(), &prefix, &OutputSpec::default())
        .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn retry_lookup_rejects_noncanonical_local_token_alias() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    assert!(journal.resolve_recovery_ref("local:01").is_err());
    assert!(journal.resolve_recovery_ref("local:+1").is_err());
    assert!(journal.resolve_recovery_ref("local:1").is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lone_header_budget_minimum_matches_exact_selected_bytes() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    let prefix = vec!["pending-ops".into()];
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..Default::default()
    };
    let full = journal
        .page_with_output(
            &crate::protocol::pagination::PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: 4096,
            },
            &prefix,
            &spec,
        )
        .unwrap();
    let expected =
        crate::protocol::output::encode_selected(&CommandResult::LocalIntents(full), &spec)
            .unwrap()
            .len() as u32;
    let result = journal.page_with_output(
        &crate::protocol::pagination::PageRequest {
            cursor: None,
            limit: 1,
            max_bytes: 256,
        },
        &prefix,
        &spec,
    );
    if expected > 256 {
        assert_eq!(result.unwrap_err().required_minimum_bytes, Some(expected));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn committed_result_and_broken_output_keep_recoverable_intent() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 123).unwrap();
    let submits = Arc::new(AtomicUsize::new(0));
    let count = submits.clone();
    let failure = run_retry(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        move |cmd| {
            count.fetch_add(1, Ordering::SeqCst);
            assert!(matches!(cmd, Command::SendMessage(_)));
            Ok(CommandResult::MessageSent(MessageId::new("message-1")))
        },
        |_| Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken")),
    );
    assert!(failure.is_err());
    assert_eq!(
        journal.load(&reference).unwrap().operation.as_str(),
        reference.operation.as_str()
    );
    let result = run_retry(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |_| Ok(CommandResult::MessageSent(MessageId::new("message-1"))),
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        result,
        CommandResult::MessageSent(MessageId::new("message-1"))
    );
    assert!(journal.load(&reference).is_err());
    assert_eq!(submits.load(Ordering::SeqCst), 1);
    let repeated = journal.record(scope(), send(), 124).unwrap();
    assert_ne!(reference.operation, repeated.operation);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn page_waits_for_publication_lock_before_capturing_high_water() {
    let dir = temp();
    let journal = Arc::new(Journal::open(&dir).unwrap());
    let guard = journal.lock().unwrap();
    journal.reserve(1).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = journal.clone();
    let handle = std::thread::spawn(move || tx.send(reader.page(&Default::default())).unwrap());
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    drop(guard);
    let page = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(page.high_water_ordinal, 1);
    handle.join().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pagination_survives_removal_and_append_without_skipping_original_entries() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let refs: Vec<_> = (0..3)
        .map(|n| journal.record(scope(), send(), n).unwrap())
        .collect();
    let first = journal
        .page(&crate::protocol::pagination::PageRequest {
            cursor: None,
            limit: 1,
            max_bytes: 4096,
        })
        .unwrap();
    assert_eq!(
        journal
            .resolve_recovery_ref(first.items[0].recovery_ref.as_str())
            .unwrap(),
        refs[0]
    );
    journal.complete(&refs[0]).unwrap();
    journal.record(scope(), send(), 4).unwrap();
    let second = journal
        .page(&crate::protocol::pagination::PageRequest {
            cursor: first.next_cursor,
            limit: 1,
            max_bytes: 4096,
        })
        .unwrap();
    assert_eq!(
        journal
            .resolve_recovery_ref(second.items[0].recovery_ref.as_str())
            .unwrap(),
        refs[1]
    );
    let third = journal
        .page(&crate::protocol::pagination::PageRequest {
            cursor: second.next_cursor,
            limit: 1,
            max_bytes: 4096,
        })
        .unwrap();
    assert_eq!(
        journal
            .resolve_recovery_ref(third.items[0].recovery_ref.as_str())
            .unwrap(),
        refs[2]
    );
    assert!(third.next_cursor.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn mismatched_operator_user_cannot_submit_or_remove_intent() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let admin = IntentScope::Operator {
        instance: "instance-1".into(),
        local_user_uid: u32::MAX,
    };
    let reference = journal
        .record(
            admin.clone(),
            SemanticMutation::OperatorFreshSeat {
                target: HostTargetId::new("target-1"),
            },
            44,
        )
        .unwrap();
    let submit_count = AtomicUsize::new(0);
    let result = run_retry(
        &journal,
        &reference,
        &admin,
        || panic!("operator must not use native proof"),
        |_| {
            submit_count.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::OperatorFreshSeat(SeatId::new("seat-2")))
        },
        |_| Ok(()),
    );
    assert!(result.is_err());
    assert_eq!(submit_count.load(Ordering::SeqCst), 0);
    assert!(journal.load(&reference).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_native_proof_retains_intent_and_never_submits() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 45).unwrap();
    let result = run_retry(
        &journal,
        &reference,
        &scope(),
        || {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "no current proof",
            ))
        },
        |_| panic!("must not submit"),
        |_| Ok(()),
    );
    assert!(result.is_err());
    assert!(journal.load(&reference).is_ok());
    let raw = std::fs::read_to_string(journal.path(&reference)).unwrap();
    assert!(!raw.contains("session-secret"));
    assert!(!raw.contains("execution-secret"));
    assert!(raw.contains("body secret"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rolled_back_allocator_fails_instead_of_reusing_ordinal() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let first = journal.record(scope(), send(), 1).unwrap();
    std::fs::write(dir.join("next-ordinal"), b"0\n").unwrap();
    assert!(journal.record(scope(), send(), 2).is_err());
    assert!(journal.load(&first).is_ok());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_allocator_after_completed_intent_fails_instead_of_reusing_key() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 1).unwrap();
    journal.complete(&reference).unwrap();
    std::fs::remove_file(dir.join("next-ordinal")).unwrap();
    assert!(journal.record(scope(), send(), 2).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reserved_crash_gap_is_never_reused() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    {
        let _guard = journal.lock().unwrap();
        journal.reserve(1).unwrap();
    }
    let reference = journal.record(scope(), send(), 3).unwrap();
    assert_eq!(reference.ordinal, 2);
    let page = journal.page(&Default::default()).unwrap();
    assert_eq!(page.high_water_ordinal, 2);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].recovery_ref.as_str(), "local:2");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_publishers_receive_distinct_ordered_keys() {
    let dir = temp();
    let journal = Arc::new(Journal::open(&dir).unwrap());
    let handles: Vec<_> = (0..12)
        .map(|n| {
            let journal = journal.clone();
            std::thread::spawn(move || journal.record(scope(), send(), n).unwrap())
        })
        .collect();
    let mut ordinals: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().unwrap().ordinal)
        .collect();
    ordinals.sort_unstable();
    assert_eq!(ordinals, (1..=12).collect::<Vec<_>>());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn changed_semantic_body_cannot_be_retried_and_pending_page_does_not_read_it() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), send(), 5).unwrap();
    let path = journal.path(&reference);
    let raw = std::fs::read_to_string(&path)
        .unwrap()
        .replace("body secret", "body edited");
    std::fs::write(&path, raw).unwrap();
    let page = journal.page(&Default::default()).unwrap();
    let display = serde_json::to_string(&page).unwrap();
    assert!(!display.contains("body edited"));
    assert!(!display.contains("semantic_digest"));
    assert!(journal.load(&reference).is_err());
    assert!(
        run_retry(
            &journal,
            &reference,
            &scope(),
            || Ok(claim()),
            |_| panic!("must not submit changed payload"),
            |_| Ok(())
        )
        .is_err()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn journal_corruption_prevents_new_mutation_submit() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    std::fs::write(dir.join("next-ordinal"), b"bad\n").unwrap();
    let submits = AtomicUsize::new(0);
    let result = crate::cli::retry::run_new(
        &journal,
        scope(),
        send(),
        6,
        || Ok(claim()),
        |_| {
            submits.fetch_add(1, Ordering::SeqCst);
            Ok(CommandResult::MessageSent(MessageId::new("m1")))
        },
        |_| Ok(()),
    );
    assert!(result.is_err());
    assert_eq!(submits.load(Ordering::SeqCst), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pending_pages_traverse_205_entries_with_bounded_headers() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    for n in 1..=205 {
        journal.record(scope(), send(), n).unwrap();
    }
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let request = crate::protocol::pagination::PageRequest {
            cursor,
            limit: 20,
            max_bytes: 4096,
        };
        let page = journal.page(&request).unwrap();
        assert!(page.items.len() <= 20);
        assert!(serde_json::to_vec(&page).unwrap().len() <= 4096);
        seen.extend(page.items.iter().map(|i| {
            i.recovery_ref
                .as_str()
                .strip_prefix("local:")
                .unwrap()
                .parse::<u64>()
                .unwrap()
        }));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, (1..=205).collect::<Vec<_>>());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn continuation_preserves_validated_cli_context_prefix() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    journal.record(scope(), send(), 2).unwrap();
    let prefix = vec![
        "herdr-threads".into(),
        "--instance".into(),
        "instance-1".into(),
        "pending-ops".into(),
    ];
    let page = journal
        .page_with_argv(
            &crate::protocol::pagination::PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: 4096,
            },
            &prefix,
        )
        .unwrap();
    assert!(page.next_argv.unwrap().starts_with(&prefix));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_semantic_batch_does_not_reserve_ordinal_or_publish_intent() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    assert!(
        journal
            .record(scope(), SemanticMutation::Ack { messages: vec![] }, 1)
            .is_err()
    );
    assert!(
        journal
            .record(
                scope(),
                SemanticMutation::SendMessage {
                    thread: ThreadId::new("thread-1"),
                    body: "hello".into(),
                    invited_recipients: (0..101)
                        .map(|n| SeatId::new(format!("seat-{n}")))
                        .collect(),
                    deadline_millis: None,
                },
                2
            )
            .is_err()
    );
    let first_valid = journal.record(scope(), send(), 3).unwrap();
    assert_eq!(first_valid.ordinal, 1);
    let page = journal.page(&Default::default()).unwrap();
    assert_eq!(page.items.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn local_cursor_rejects_changed_explicit_context_and_round_trips_same_context() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    journal.record(scope(), send(), 1).unwrap();
    journal.record(scope(), send(), 2).unwrap();
    let one = vec![
        "herdr-threads".into(),
        "--instance".into(),
        "instance-1".into(),
        "pending-ops".into(),
    ];
    let other = vec![
        "herdr-threads".into(),
        "--instance".into(),
        "instance-2".into(),
        "pending-ops".into(),
    ];
    let first = journal
        .page_with_argv(
            &crate::protocol::pagination::PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: 4096,
            },
            &one,
        )
        .unwrap();
    let cursor = first.next_cursor.unwrap();
    let continued = crate::protocol::pagination::PageRequest {
        cursor: Some(cursor),
        limit: 1,
        max_bytes: 4096,
    };
    let mismatch = journal
        .page_with_output(
            &continued,
            &other,
            &OutputSpec {
                format: OutputFormat::Text,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(
        mismatch.code,
        crate::protocol::results::ErrorCode::InvalidCursor
    );
    let second = journal.page_with_argv(&continued, &one).unwrap();
    assert_eq!(second.items[0].recovery_ref.as_str(), "local:2");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_and_old_pending_cursors_are_invalid_cursor_even_on_empty_journal() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let prefix = vec!["pending-ops".into()];
    let output = OutputSpec {
        format: OutputFormat::Text,
        ..Default::default()
    };
    for raw in ["c1:old", "c2:not-base64!", "c2:"] {
        let request = crate::protocol::pagination::PageRequest {
            cursor: Some(raw.into()),
            limit: 1,
            max_bytes: 4096,
        };
        let error = journal
            .page_with_output(&request, &prefix, &output)
            .unwrap_err();
        assert_eq!(
            error.code,
            crate::protocol::results::ErrorCode::InvalidCursor,
            "{raw}"
        );
    }
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_pending_page_bounds_stay_invalid_request_without_publishing_intent() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let prefix = vec!["pending-ops".into()];
    let output = OutputSpec {
        format: OutputFormat::Text,
        ..Default::default()
    };
    for (limit, max_bytes) in [(0, 4096), (101, 4096), (1, 255), (1, 65_537)] {
        let request = crate::protocol::pagination::PageRequest {
            cursor: None,
            limit,
            max_bytes,
        };
        let error = journal
            .page_with_output(&request, &prefix, &output)
            .unwrap_err();
        assert_eq!(
            error.code,
            crate::protocol::results::ErrorCode::InvalidRequest
        );
    }
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
    let reference = journal.record(scope(), send(), 1).unwrap();
    assert_eq!(reference.ordinal, 1);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Kills: (a) keeping every new intent after a correlated daemon rejection
/// (the composition-probe B7/P2-9 stuck `resolve_seat` intents), and
/// (b) discarding an intent whose outcome is unknown or never reached the
/// daemon. Only a definitive first-submission rejection is discarded.
#[test]
fn new_resolution_discards_only_definitively_rejected_first_submission() {
    use crate::protocol::results::{ApiError, ErrorCode};
    fn api(code: ErrorCode) -> ApiError {
        ApiError {
            code,
            detail: "fixture".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }
    }
    let resolve = || SemanticMutation::ResolveSeat {
        target: HostTargetId::new("w4:p1"),
    };
    let allocation = || IntentScope::ServiceAllocation {
        instance: "instance-1".into(),
        target: HostTargetId::new("w4:p1"),
    };
    let no_claim = || Err(io::Error::other("ordinary resolution has no native claim"));
    for (answer, kept) in [
        (Ok(Err(api(ErrorCode::StaleHostObservation))), false),
        (Ok(Err(api(ErrorCode::TargetUnresolved))), false),
        (Ok(Err(api(ErrorCode::NotFound))), false),
        (Ok(Err(api(ErrorCode::UnknownOutcome))), true),
        (Err(api(ErrorCode::UnknownOutcome)), true),
        (Err(api(ErrorCode::HostUnavailable)), true),
    ] {
        let dir = temp();
        let journal = Journal::open(&dir).unwrap();
        let expected = match &answer {
            Ok(Err(error)) | Err(error) => error.code.clone(),
            Ok(Ok(_)) => unreachable!(),
        };
        let mut output = Vec::new();
        let failure = crate::cli::retry::run_new_api_to_writer_discarding_rejection(
            &journal,
            allocation(),
            resolve(),
            1,
            no_claim,
            |command| {
                assert!(matches!(command, Command::ResolveSeat(_)));
                answer
            },
            &OutputSpec::default(),
            &mut output,
        )
        .unwrap_err();
        assert!(
            matches!(failure, crate::cli::retry::RetryFailure::Submit(ref error) if error.code == expected)
        );
        assert!(output.is_empty());
        let pending = journal.page(&Default::default()).unwrap().items.len();
        assert_eq!(pending, usize::from(kept), "{expected:?} kept={kept}");
        std::fs::remove_dir_all(dir).unwrap();
    }
    // A committed answer is printed and completed, as before.
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let mut output = Vec::new();
    crate::cli::retry::run_new_api_to_writer_discarding_rejection(
        &journal,
        allocation(),
        resolve(),
        1,
        no_claim,
        |_| Ok(Ok(CommandResult::SeatResolved(SeatId::new("seat-1")))),
        &OutputSpec::default(),
        &mut output,
    )
    .unwrap();
    assert!(!output.is_empty());
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}
