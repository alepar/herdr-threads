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
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
        thread: ThreadId::new("thread-1"),
        body: "body secret\n".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        relays_user: false,
        user_intent: None,
    }
}
fn scope() -> IntentScope {
    IntentScope::Native {
        instance: "instance-1".into(),
        seat: SeatId::new("seat-1"),
    }
}

#[test]
fn displayed_body_progress_requires_contiguous_flushed_chunks() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let mut display_claim = claim();
    display_claim.seat = SeatId::new("seat-original");
    let message = MessageId::new("message-original");
    assert!(
        !journal
            .record_displayed_chunk(&display_claim, &message, 5, 10, 15)
            .unwrap(),
        "a supplied final-page cursor cannot establish earlier output"
    );
    assert!(
        !journal
            .record_displayed_chunk(&display_claim, &message, 0, 5, 15)
            .unwrap()
    );
    assert!(
        !journal
            .record_displayed_chunk(&display_claim, &message, 5, 10, 15)
            .unwrap()
    );
    let mut successor = display_claim.clone();
    successor.binding_generation += 1;
    successor.execution = ExecutionId::new("successor-execution");
    assert!(
        !journal
            .record_displayed_chunk(&successor, &message, 10, 15, 15)
            .unwrap(),
        "another occupant cannot inherit earlier displayed chunks"
    );
    assert!(
        journal
            .record_displayed_chunk(&display_claim, &message, 10, 15, 15)
            .unwrap()
    );
    assert!(
        journal
            .record_displayed_chunk(&display_claim, &message, 10, 15, 15)
            .unwrap(),
        "replaying a flushed final page is idempotent"
    );
    journal
        .clear_displayed_chunk(&display_claim, &message)
        .unwrap();
    assert!(
        !journal
            .record_displayed_chunk(&display_claim, &message, 10, 15, 15)
            .unwrap(),
        "settlement clears the local chain"
    );
    assert!(
        journal
            .record_displayed_chunk(&display_claim, &message, 0, 15, 15)
            .unwrap(),
        "a complete one-page body needs no earlier chain"
    );
    std::fs::remove_dir_all(dir).unwrap();
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
            Err(crate::protocol::results::ApiError::unknown_outcome(
                "response lost",
            ))
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
            Err(crate::protocol::results::ApiError::host_unavailable(
                "connect refused",
            ))
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
    // A barrier releases every publisher together, so the ordinals contend
    // for the journal lock regardless of thread start-up timing; the
    // assertion is on the committed keys only, never on arrival order.
    let start = Arc::new(std::sync::Barrier::new(12));
    let handles: Vec<_> = (0..12)
        .map(|n| {
            let journal = journal.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                // The allocator's own 1 s lock wait is a product bound that
                // a loaded disk (twelve fsync-ing writers) can exceed; a
                // timed-out publisher consumed no ordinal and retries, as a
                // CLI caller does. Any other error fails the test at once.
                let give_up = std::time::Instant::now() + std::time::Duration::from_secs(60);
                loop {
                    match journal.record(scope(), send(), n) {
                        Ok(reference) => return reference,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::TimedOut
                                && std::time::Instant::now() < give_up => {}
                        Err(error) => panic!("publisher {n}: {error}"),
                    }
                }
            })
        })
        .collect();
    let references: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let mut ordinals: Vec<_> = references.iter().map(|r| r.ordinal).collect();
    ordinals.sort_unstable();
    assert_eq!(ordinals, (1..=12).collect::<Vec<_>>());
    let committed = journal.page(&Default::default()).unwrap();
    assert_eq!(committed.items.len(), 12);
    assert_eq!(committed.high_water_ordinal, 12);
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
                    delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                    thread: ThreadId::new("thread-1"),
                    body: "hello".into(),
                    invited_recipients: (0..101)
                        .map(|n| SeatId::new(format!("seat-{n}")))
                        .collect(),
                    deadline_millis: None,
                    relays_user: false,
                    user_intent: None,
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
        ApiError::new(code, "fixture")
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

#[test]
fn allocator_lock_leaf_symlink_is_refused() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let elsewhere = temp();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let victim = elsewhere.join("victim");
    std::fs::write(&victim, b"").unwrap();
    let _ = std::fs::remove_file(dir.join("allocator.lock"));
    std::os::unix::fs::symlink(&victim, dir.join("allocator.lock")).unwrap();
    assert!(
        journal.record(scope(), send(), 1).is_err(),
        "a symlinked allocator.lock must not be followed"
    );
    assert!(journal.lock().is_err());
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(elsewhere).unwrap();
}

fn continuity_semantic(target: &str, event_id: &str) -> SemanticMutation {
    SemanticMutation::ContinuityCheckIn {
        target: HostTargetId::new(target),
        harness: Harness::Claude,
        native_session: NativeSessionId::new("sess-1"),
        source: "resume".into(),
        event_id: event_id.into(),
        execution: ExecutionId::new("exec-1"),
    }
}
fn continuity_scope(instance: &str, target: &str) -> IntentScope {
    IntentScope::Continuity {
        instance: instance.into(),
        target: HostTargetId::new(target),
    }
}

#[test]
fn continuity_intent_round_trips_and_is_found_by_instance_and_target() {
    use crate::protocol::{commands::ContinuityCheckIn, ids::OperationId};
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal
        .record(
            continuity_scope("i1", "w1:p1"),
            continuity_semantic("w1:p1", "evt-1"),
            7,
        )
        .unwrap();
    // Another pane and another instance never see it.
    assert!(
        journal
            .pending_continuity("i1", &HostTargetId::new("w1:p2"))
            .unwrap()
            .is_none()
    );
    assert!(
        journal
            .pending_continuity("i2", &HostTargetId::new("w1:p1"))
            .unwrap()
            .is_none()
    );
    let found = journal
        .pending_continuity("i1", &HostTargetId::new("w1:p1"))
        .unwrap()
        .expect("pending continuity intent");
    assert_eq!(found.header.reference, reference);
    assert_eq!(found.operation, reference.operation);
    assert_eq!(
        found.header.kind,
        crate::protocol::results::IntentKind::ContinuityCheckIn
    );
    // It becomes the same wire command under the journal's operation key.
    let command = found
        .semantic
        .to_command(found.operation.clone(), None)
        .unwrap();
    assert_eq!(
        command,
        Command::ContinuityCheckIn(ContinuityCheckIn {
            target: HostTargetId::new("w1:p1"),
            harness: Harness::Claude,
            native_session: NativeSessionId::new("sess-1"),
            source: "resume".into(),
            operation: OperationId::new(reference.operation.as_str()),
            execution: ExecutionId::new("exec-1"),
        })
    );
    // Completion removes it.
    journal.complete(&reference).unwrap();
    assert!(
        journal
            .pending_continuity("i1", &HostTargetId::new("w1:p1"))
            .unwrap()
            .is_none()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn continuity_intent_refuses_foreign_scope_and_non_resume_source() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    // The scope names one pane; a request for another is a scope mismatch.
    assert!(
        journal
            .record(
                continuity_scope("i1", "w1:p1"),
                continuity_semantic("w1:p2", "evt"),
                1
            )
            .is_err()
    );
    // A native seat scope cannot carry a continuity request.
    assert!(
        journal
            .record(scope(), continuity_semantic("w1:p1", "evt"), 1)
            .is_err()
    );
    // Only a resume can be journaled.
    let SemanticMutation::ContinuityCheckIn {
        target,
        harness,
        native_session,
        event_id,
        execution,
        ..
    } = continuity_semantic("w1:p1", "evt")
    else {
        unreachable!()
    };
    assert!(
        journal
            .record(
                continuity_scope("i1", "w1:p1"),
                SemanticMutation::ContinuityCheckIn {
                    target,
                    harness,
                    native_session,
                    source: "startup".into(),
                    event_id,
                    execution,
                },
                1
            )
            .is_err()
    );
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pending_continuity_skips_vanished_unparsable_fifo_and_symlink_entries() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal
        .record(
            continuity_scope("i1", "w1:p1"),
            continuity_semantic("w1:p1", "evt-1"),
            7,
        )
        .unwrap();
    // A FIFO with no writer would block a plain open; a symlink to an endless
    // device would never finish a read; garbage does not parse.
    let fifo = dir.join(format!("{:020}-fifo.intent", 1));
    let c_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    std::os::unix::fs::symlink("/dev/zero", dir.join(format!("{:020}-zero.intent", 1))).unwrap();
    std::os::unix::fs::symlink(
        dir.join("missing-target"),
        dir.join(format!("{:020}-dangling.intent", 1)),
    )
    .unwrap();
    std::fs::write(dir.join(format!("{:020}-garbage.intent", 1)), b"not json\n").unwrap();
    std::fs::write(dir.join(format!("{:020}-empty.intent", 1)), b"").unwrap();
    let found = journal
        .pending_continuity("i1", &HostTargetId::new("w1:p1"))
        .unwrap()
        .expect("the valid intent is still found");
    assert_eq!(found.header.reference, reference);
    // With only bad entries present the scan finds nothing and does not fail.
    journal.complete(&reference).unwrap();
    assert!(
        journal
            .pending_continuity("i1", &HostTargetId::new("w1:p1"))
            .unwrap()
            .is_none()
    );
    let _ = std::fs::remove_file(fifo);
    std::fs::remove_dir_all(dir).unwrap();
}

// Kills: a hook stuck on a FIFO in the shared intents directory, or one
// unparsable entry failing every scan.
#[test]
// The junk entries take the names of recorded intents because the allocator
// (`counter`, used by `record_check_in`) rejects any other `.intent` name and
// one ahead of its counter; the scans do not care.
fn scans_skip_fifo_symlink_and_garbage_entries() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    let dir = temp();
    let journal = Arc::new(Journal::open(&dir).unwrap());
    let refs: Vec<_> = (1..=4)
        .map(|n| journal.record(scope(), send(), n).unwrap())
        .collect();
    let real = refs[3].clone();
    let (fifo_path, garbage_path, link_path) = (
        journal.path(&refs[0]),
        journal.path(&refs[1]),
        journal.path(&refs[2]),
    );
    for path in [&fifo_path, &garbage_path, &link_path] {
        std::fs::remove_file(path).unwrap();
    }
    let fifo = std::ffi::CString::new(fifo_path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `fifo` is a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    std::fs::write(&garbage_path, "not json\n").unwrap();
    symlink(journal.path(&real), &link_path).unwrap();

    let cooperative = IntentScope::Cooperative {
        instance: "i".into(),
        seat: SeatId::new("legacy-fixture"),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = Arc::clone(&journal);
    std::thread::spawn(move || {
        let found = worker
            .find_check_in(&cooperative, "evt")
            .map(|found| found.is_none());
        let completed = worker.complete_operation(&OperationId::new("unknown-operation"));
        let recorded = worker
            .record_check_in(cooperative, "evt", 2, || {
                Ok((claim(), CheckInMode::Current))
            })
            .map(|_| ());
        let _ = tx.send((found, completed, recorded));
    });
    let (found, completed, recorded) = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a scan blocked on the FIFO");
    assert!(found.unwrap(), "no check-in is pending");
    completed.unwrap();
    recorded.unwrap();
    // The real intent is untouched by the skipped entries.
    assert!(journal.path(&real).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn send_relays_user_is_journaled_only_when_set_and_old_intents_still_load() {
    let relayed = match send() {
        SemanticMutation::SendMessage {
            thread,
            body,
            invited_recipients,
            deadline_millis,
            user_intent,
            ..
        } => SemanticMutation::SendMessage {
            delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
            thread,
            body,
            invited_recipients,
            deadline_millis,
            relays_user: true,
            user_intent,
        },
        _ => unreachable!(),
    };
    let relayed_json = serde_json::to_value(&relayed).unwrap();
    assert_eq!(relayed_json["relays_user"], true);
    let plain_json = serde_json::to_value(send()).unwrap();
    assert!(plain_json.get("relays_user").is_none());
    // An intent journaled before the flag existed has no key and reads as false.
    let old: SemanticMutation = serde_json::from_value(plain_json).unwrap();
    assert_eq!(old, send());
    let back: SemanticMutation = serde_json::from_value(relayed_json).unwrap();
    let command = back
        .to_command(
            crate::protocol::ids::OperationId::new("op-relay"),
            Some(claim()),
        )
        .unwrap();
    let Command::SendMessage(sent) = command else {
        panic!("wrong command")
    };
    assert!(sent.relays_user);
}

#[test]
fn thread_names_legacy_create_semantics_omit_name_and_replay() {
    let old = serde_json::json!({"kind":"create_thread","topic":"topic","goal":"goal"});
    let semantic: SemanticMutation = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(&semantic).unwrap(), old);
    assert_eq!(
        serde_json::to_vec(&semantic).unwrap(),
        serde_json::to_vec(&SemanticMutation::CreateThread {
            name: None,
            topic: "topic".into(),
            goal: "goal".into()
        })
        .unwrap()
    );
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal.record(scope(), semantic, 1).unwrap();
    run_retry(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |command| {
            let Command::CreateThread(create) = command else {
                panic!("create")
            };
            assert_eq!(create.name, None);
            assert!(serde_json::to_value(&create).unwrap().get("name").is_none());
            Ok(CommandResult::ThreadCreated(ThreadId::new("tLEGACY01")))
        },
        |_| Ok(()),
    )
    .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thread_names_journal_retry_uses_frozen_id_after_rename() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let mut parsed =
        crate::cli::commands::parse_argv(["ht", "thread", "rename", "team café", "new name"])
            .unwrap();
    crate::cli::threads::resolve_cli_threads(&mut parsed, |name| {
        assert_eq!(name, "team café");
        Ok::<_, ApiError>(ThreadId::new("tORIGINAL"))
    })
    .unwrap();
    let crate::cli::commands::CliAction::Mutation(crate::cli::commands::MutationSpec::Name {
        thread,
        name,
    }) = parsed.action
    else {
        panic!("name")
    };
    let semantic = SemanticMutation::SetThreadName { thread, name };
    let reference = journal.record(scope(), semantic, 1).unwrap();
    let loaded = journal.load(&reference).unwrap();
    assert_eq!(loaded.header.thread, Some(ThreadId::new("tORIGINAL")));
    // The daemon committed, but failed output preserves the keyed intent.
    let result = run_retry(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |command| {
            let Command::SetThreadName(change) = command else {
                panic!("name")
            };
            assert_eq!(change.thread, ThreadId::new("tORIGINAL"));
            assert_eq!(change.name.as_deref(), Some("new name"));
            Ok(CommandResult::ThreadNameChanged(ThreadId::new("tORIGINAL")))
        },
        |_| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "output interrupted",
            ))
        },
    );
    assert!(result.is_err());
    // A later name owner is irrelevant: replay sends the same canonical ID
    // and operation key without performing any selector lookup.
    run_retry(
        &journal,
        &reference,
        &scope(),
        || Ok(claim()),
        |command| {
            let Command::SetThreadName(change) = command else {
                panic!("name")
            };
            assert_eq!(change.thread, ThreadId::new("tORIGINAL"));
            assert_eq!(change.operation, reference.operation);
            Ok(CommandResult::ThreadNameChanged(change.thread))
        },
        |bytes| {
            assert_eq!(
                bytes,
                &CommandResult::ThreadNameChanged(ThreadId::new("tORIGINAL"))
            );
            Ok(())
        },
    )
    .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invitation_rejection_journal_freezes_exact_id_reason_and_binding() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reason = "Outside role\nexplicit peer data: \u{1b}[31m";
    let semantic = SemanticMutation::freeze(
        SemanticMutation::Reject {
            thread: ThreadId::new("tResolved"),
            invitation: InvitationId::new("vExact"),
            reason: reason.into(),
        },
        claim(),
    )
    .unwrap();
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: "i".into(),
                seat: claim().seat,
            },
            semantic.clone(),
            100,
        )
        .unwrap();
    let reopened = Journal::open(&dir).unwrap().load(&reference).unwrap();
    assert_eq!(reopened.semantic, semantic);
    let Command::Reject(command) = reopened
        .semantic
        .to_command(reopened.operation.clone(), None)
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(command.invitation.as_str(), "vExact");
    assert_eq!(command.reason, reason);
    assert_eq!(command.claim, claim());
    assert_eq!(command.thread.as_str(), "tResolved");
    let mut child = command.clone();
    child.claim.role = crate::protocol::authority::CallerRole::Subagent;
    assert!(Command::Reject(child).validate().is_err());
    let mut oversized = command;
    oversized.reason = "é".repeat(2049);
    assert!(Command::Reject(oversized).validate().is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn user_intent_journal_roundtrip_and_legacy_omission() {
    use crate::protocol::summary::UserIntent;
    let old = serde_json::to_value(send()).unwrap();
    assert!(old.get("user_intent").is_none());
    let legacy: SemanticMutation = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(serde_json::to_value(legacy).unwrap(), old);
    for intent in [UserIntent::Query, UserIntent::Request, UserIntent::Rule] {
        let mut json = old.clone();
        json["relays_user"] = serde_json::json!(true);
        json["user_intent"] = serde_json::json!(intent);
        let restored: SemanticMutation = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&restored).unwrap(), json);
        let Command::SendMessage(request) = restored
            .to_command(
                crate::protocol::ids::OperationId::new("op-intent"),
                Some(claim()),
            )
            .unwrap()
        else {
            panic!("wrong command")
        };
        assert_eq!(request.user_intent, Some(intent));
    }
}

// Exercises the CLI-to-journal seam and retries the durable frozen claim after
// reopening, so a dropped classification or refreshed claim fails the test.
#[test]
fn user_intent_send_retry_preserves_recorded_claim() {
    use crate::cli::commands::{CliAction, parse_argv};
    use crate::protocol::summary::UserIntent;
    for (spelling, intent) in [
        ("query", UserIntent::Query),
        ("request", UserIntent::Request),
        ("rule", UserIntent::Rule),
    ] {
        let parsed = parse_argv([
            "herdr-threads",
            "send",
            "t1",
            "--body",
            "user input",
            "--nudge",
            "--relays-user",
            "--user-intent",
            spelling,
        ])
        .unwrap();
        let CliAction::Mutation(spec) = parsed.action else {
            panic!("not a mutation")
        };
        let semantic = crate::cli::cooperative_semantic(spec).unwrap();
        let frozen = SemanticMutation::freeze(semantic, claim()).unwrap();
        let selected_scope = IntentScope::Cooperative {
            instance: "i".into(),
            seat: claim().seat,
        };
        let dir = temp();
        let journal = Journal::open(&dir).unwrap();
        let reference = journal
            .record(selected_scope.clone(), frozen.clone(), 123)
            .unwrap();
        drop(journal);
        let journal = Journal::open(&dir).unwrap();
        assert_eq!(journal.load(&reference).unwrap().semantic, frozen);
        for _ in 0..2 {
            // Retain the intent after response output loss, then retry it again.
            let result = run_retry(
                &journal,
                &reference,
                &selected_scope,
                || panic!("frozen claim must not be refreshed"),
                |command| {
                    let Command::SendMessage(sent) = command else {
                        panic!("wrong command")
                    };
                    assert_eq!(sent.user_intent, Some(intent));
                    assert!(sent.relays_user);
                    assert_eq!(sent.claim, claim());
                    assert_eq!(sent.operation, reference.operation);
                    Ok(CommandResult::MessageSent(MessageId::new("published")))
                },
                |_| Err(io::Error::new(io::ErrorKind::BrokenPipe, "lost output")),
            );
            assert!(result.is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn retry_preflight_read_only_absent_journal() {
    let dir = temp();
    assert_eq!(
        Journal::read_only(dir.join("intents"))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert!(!dir.exists());
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state");
    let host = dir.join("host.sock");
    let mut output = Vec::new();
    let result = crate::cli::run_in_pane(
        [
            "ht".to_owned(),
            "--state-dir".into(),
            state.to_string_lossy().into_owned(),
            "--host-endpoint".into(),
            host.to_string_lossy().into_owned(),
            "retry".into(),
            "local:1".into(),
        ],
        None,
        &mut output,
    );
    assert!(result.is_err());
    assert!(output.is_empty());
    assert!(
        !state.exists(),
        "missing retry must not create instance or journal state"
    );
    if dir.exists() {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn retry_preflight_retained_human_root_refuses_before_connect() {
    let dir = std::env::temp_dir().join(format!("ht-retry-' space-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state");
    let host = dir.join("host.sock");
    let context =
        crate::daemon::paths::RuntimeContext::explicit(state.clone(), host.clone(), None).unwrap();
    let paths = crate::daemon::paths::InstancePaths::resolve_read_only(&context).unwrap();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    let mut human = claim();
    human.harness = Harness::Human;
    let frozen = SemanticMutation::freeze(send(), human.clone()).unwrap();
    let reference = journal
        .record(
            IntentScope::Cooperative {
                instance: human.instance,
                seat: human.seat,
            },
            frozen,
            1,
        )
        .unwrap();
    let before = std::fs::read(journal.path(&reference)).unwrap();
    std::fs::remove_file(journal.root().join("allocator.lock")).unwrap();
    let mut output = Vec::new();
    let result = crate::cli::run_in_pane(
        [
            "ht".to_owned(),
            "--state-dir".into(),
            state.to_string_lossy().into_owned(),
            "--host-endpoint".into(),
            host.to_string_lossy().into_owned(),
            "retry".into(),
            reference.recovery_ref(),
        ],
        None,
        &mut output,
    );
    assert!(
        format!("{result:?}").contains("person/operator retry requires immediate human namespace; use herdr-threads human --state-dir"),
        "{result:?}"
    );
    let detail = result.as_ref().unwrap_err().to_string();
    let replacement = detail.split_once("; use ").unwrap().1;
    let replacement = shlex::split(replacement).unwrap();
    assert_eq!(replacement[1], "human");
    let parsed = crate::cli::commands::parse_argv(replacement).unwrap();
    assert_eq!(
        parsed.actor,
        crate::cli::actor_route::InvocationActor::Human
    );
    assert_eq!(
        parsed.output.context.state_dir.as_deref(),
        Some(context.state_dir.to_str().unwrap())
    );
    assert_eq!(
        parsed.output.context.host.as_deref(),
        Some(context.host_endpoint.to_str().unwrap())
    );
    assert!(output.is_empty());
    assert_eq!(std::fs::read(journal.path(&reference)).unwrap(), before);
    assert!(!journal.root().join("allocator.lock").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn original_actor_frozen_agent_and_human() {
    for harness in [Harness::Codex, Harness::Claude, Harness::Human] {
        let mut original = claim();
        original.harness = harness;
        let scope = IntentScope::Cooperative {
            instance: original.instance.clone(),
            seat: original.seat.clone(),
        };
        let semantic = SemanticMutation::freeze(send(), original).unwrap();
        let bytes = serde_json::to_vec(&semantic).unwrap();
        let expected = if harness == Harness::Human {
            OriginalActor::HumanOrOperator
        } else {
            OriginalActor::Agent
        };
        assert_eq!(
            classify_original_actor(&scope, &semantic).unwrap(),
            expected
        );
        let legacy: SemanticMutation = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(classify_original_actor(&scope, &legacy).unwrap(), expected);
        assert_eq!(serde_json::to_vec(&legacy).unwrap(), bytes);
    }
}

#[test]
fn original_actor_operator_and_native_lifecycle() {
    let target = HostTargetId::new("pane");
    let operator = IntentScope::Operator {
        instance: "i".into(),
        local_user_uid: 7,
    };
    let mutations = [
        SemanticMutation::OperatorRebind {
            seat: SeatId::new("s"),
            target: target.clone(),
        },
        SemanticMutation::OperatorFreshSeat {
            target: target.clone(),
        },
        SemanticMutation::OperatorRetire {
            seat: SeatId::new("s"),
        },
        SemanticMutation::OperatorReplace {
            seat: SeatId::new("s"),
            target: target.clone(),
            replace: SeatId::new("other"),
        },
        SemanticMutation::OperatorOrphanInvite {
            thread: ThreadId::new("t"),
            seat: SeatId::new("s"),
            deadline_millis: None,
        },
    ];
    for semantic in mutations {
        assert_eq!(
            classify_original_actor(&operator, &semantic).unwrap(),
            OriginalActor::HumanOrOperator
        );
    }
    assert_eq!(
        classify_original_actor(&scope(), &SemanticMutation::CheckIn).unwrap(),
        OriginalActor::Agent
    );
    assert_eq!(
        classify_original_actor(
            &IntentScope::ServiceAllocation {
                instance: "i".into(),
                target: target.clone()
            },
            &SemanticMutation::ResolveSeat {
                target: target.clone()
            }
        )
        .unwrap(),
        OriginalActor::Agent
    );
    for harness in [Harness::Claude, Harness::Codex] {
        let continuity = SemanticMutation::ContinuityCheckIn {
            target: target.clone(),
            harness,
            native_session: NativeSessionId::new("session"),
            source: "resume".into(),
            event_id: "event".into(),
            execution: ExecutionId::new("exec"),
        };
        assert_eq!(
            classify_original_actor(
                &IntentScope::Continuity {
                    instance: "i".into(),
                    target: target.clone()
                },
                &continuity
            )
            .unwrap(),
            OriginalActor::Agent
        );
    }
    for operator in [false, true] {
        let mut human = claim();
        human.harness = Harness::Human;
        let cooperative = IntentScope::Cooperative {
            instance: human.instance.clone(),
            seat: human.seat.clone(),
        };
        let lifecycle = SemanticMutation::CooperativeCheckIn {
            claim: human,
            mode: CheckInMode::Lifecycle {
                expected_binding_generation: 0,
            },
            event_id: "event".into(),
            operator,
        };
        assert_eq!(
            classify_original_actor(&cooperative, &lifecycle).unwrap(),
            OriginalActor::HumanOrOperator
        );
    }
}

#[test]
fn original_actor_rejects_scope_claim_contradictions() {
    let original = claim();
    let frozen = SemanticMutation::freeze(send(), original.clone()).unwrap();
    assert!(classify_original_actor(&scope(), &frozen).is_err());
    let wrong_seat = IntentScope::Cooperative {
        instance: original.instance.clone(),
        seat: SeatId::new("other"),
    };
    assert!(classify_original_actor(&wrong_seat, &frozen).is_err());
    let wrong_instance = IntentScope::Cooperative {
        instance: "other".into(),
        seat: original.seat.clone(),
    };
    assert!(classify_original_actor(&wrong_instance, &frozen).is_err());
    let operator = IntentScope::Operator {
        instance: "i".into(),
        local_user_uid: 1,
    };
    assert!(classify_original_actor(&operator, &send()).is_err());
    let mut child = original.clone();
    child.role = crate::protocol::authority::CallerRole::Subagent;
    let scope = IntentScope::Cooperative {
        instance: original.instance,
        seat: original.seat,
    };
    let malformed = SemanticMutation::Frozen {
        claim: child,
        mutation: Box::new(send()),
    };
    assert!(classify_original_actor(&scope, &malformed).is_err());
    let continuity = SemanticMutation::ContinuityCheckIn {
        target: HostTargetId::new("pane"),
        harness: Harness::Human,
        native_session: NativeSessionId::new("session"),
        source: "resume".into(),
        event_id: "event".into(),
        execution: ExecutionId::new("exec"),
    };
    assert!(
        classify_original_actor(
            &IntentScope::Continuity {
                instance: "i".into(),
                target: HostTargetId::new("pane")
            },
            &continuity
        )
        .is_err()
    );
}

#[test]
fn retry_preflight_read_only_existing_journal_preserves_metadata() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    let reference = journal
        .record(scope(), SemanticMutation::CheckIn, 1)
        .unwrap();
    std::fs::remove_file(dir.join("allocator.lock")).unwrap();
    let before = std::fs::read(journal.path(&reference)).unwrap();
    let marker = std::fs::read(dir.join("journal-format")).unwrap();
    let ordinal = std::fs::read(dir.join("next-ordinal")).unwrap();
    assert_eq!(
        crate::cli::retry::preflight_original_actor(
            &dir,
            &reference.recovery_ref(),
            crate::cli::actor_route::InvocationActor::Agent,
            &crate::protocol::output::ContinuationContext::default()
        )
        .unwrap(),
        OriginalActor::Agent
    );
    assert_eq!(std::fs::read(journal.path(&reference)).unwrap(), before);
    assert_eq!(std::fs::read(dir.join("journal-format")).unwrap(), marker);
    assert_eq!(std::fs::read(dir.join("next-ordinal")).unwrap(), ordinal);
    assert!(!dir.join("allocator.lock").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn retry_preflight_legacy_operator_preserves_replay_keys() {
    let dir = temp();
    let journal = Journal::open(&dir).unwrap();
    // SAFETY: querying the current effective UID has no side effects.
    let uid = unsafe { libc::geteuid() };
    let authority = IntentScope::Operator {
        instance: "i".into(),
        local_user_uid: uid,
    };
    let reference = journal
        .record(
            authority.clone(),
            SemanticMutation::OperatorFreshSeat {
                target: HostTargetId::new("pane"),
            },
            1,
        )
        .unwrap();
    let before = std::fs::read(journal.path(&reference)).unwrap();
    std::fs::remove_file(dir.join("allocator.lock")).unwrap();
    let root = crate::cli::actor_route::InvocationActor::Agent;
    let human = crate::cli::actor_route::InvocationActor::Human;
    let refused = crate::cli::retry::preflight_original_actor(
        &dir,
        &reference.recovery_ref(),
        root,
        &crate::protocol::output::ContinuationContext::default(),
    )
    .unwrap_err();
    assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
    assert!(!dir.join("allocator.lock").exists());
    assert_eq!(std::fs::read(journal.path(&reference)).unwrap(), before);
    assert_eq!(
        crate::cli::retry::preflight_original_actor(
            &dir,
            &reference.recovery_ref(),
            human,
            &crate::protocol::output::ContinuationContext::default()
        )
        .unwrap(),
        OriginalActor::HumanOrOperator
    );
    let command = Command::OperatorFreshSeat(crate::protocol::commands::OperatorFreshSeat {
        target: HostTargetId::new("pane"),
        operation: reference.operation.clone(),
    });
    let replay = run_retry(
        &journal,
        &reference,
        &authority,
        || panic!("operator retry must not synthesize a caller"),
        |actual| {
            assert_eq!(actual, command);
            Ok(CommandResult::OperatorFreshSeat(SeatId::new("new-seat")))
        },
        |_| Err(io::Error::other("injected output failure")),
    );
    assert!(replay.is_err());
    assert_eq!(std::fs::read(journal.path(&reference)).unwrap(), before);
    run_retry(
        &journal,
        &reference,
        &authority,
        || panic!("operator retry must not synthesize a caller"),
        |actual| {
            assert_eq!(actual, command);
            Ok(CommandResult::OperatorFreshSeat(SeatId::new("new-seat")))
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(!journal.path(&reference).exists());
    std::fs::remove_dir_all(dir).unwrap();
}
