use crate::{
    cli::journal::Journal,
    harness::{Capability, LifecycleEvent, bridge::*, context::*},
    protocol::{
        authority::CallerRole,
        commands::{CheckInMode as WireMode, Command},
    },
};
use std::{fs, time::Duration};
/// Context-lock wait for these fixtures: a liveness bound only. On macOS a
/// lock holder's `sync_all`s are full-device flushes queued behind every
/// other test's, so a short bound turned concurrent dedup into a
/// `LockTimeout` race (ht-zo4); the timeout itself is pinned by
/// `lock_wait_is_bounded_and_crashed_lock_owner_releases`.
const CONTEXT_LOCK_WAIT: Duration = Duration::from_secs(10);

fn fixture() -> (
    std::path::PathBuf,
    Journal,
    ContextJournal,
    OccupantContext,
    LifecycleEvent,
) {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("bridge-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let j = Journal::open(root.join("intents")).unwrap();
    let cj =
        ContextJournal::open(&root, uuid::Uuid::from_u128(1), "seat", CONTEXT_LOCK_WAIT).unwrap();
    let c = OccupantContext {
        format_version: 1,
        instance: uuid::Uuid::from_u128(1),
        seat: "seat".into(),
        target: "pane".into(),
        harness: Harness::Codex,
        binding_generation: 0,
        execution: uuid::Uuid::from_u128(2),
        session: SessionReference::PluginContext(uuid::Uuid::from_u128(2)),
        role: Role::TopLevel,
    };
    let e = LifecycleEvent {
        harness: Harness::Codex,
        source: "explicit".into(),
        kind: EventKind::Startup,
        native_session: None,
        role: Role::TopLevel,
        event_id: "start-one".into(),
        capability: Capability::SourceSupported,
    };
    (root, j, cj, c, e)
}
#[test]
fn first_lifecycle_freezes_service_resolved_nonzero_generation() {
    let (root, journal, contexts, mut seed, event) = fixture();
    seed.binding_generation = 7;
    let request = prepare_event(&journal, &contexts, &event, Some(&seed), 1)
        .unwrap()
        .unwrap();
    let Command::CheckIn(check_in) = decode_request(&request).unwrap() else {
        panic!("expected CheckIn")
    };
    assert_eq!(
        check_in.mode,
        WireMode::Lifecycle {
            expected_binding_generation: 7
        }
    );
    assert_eq!(check_in.claim.binding_generation, 7);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn lifecycle_preparation_uses_one_cli_key_and_tagged_fallback_across_restart() {
    let (root, j, cj, seed, event) = fixture();
    let first = prepare_event(&j, &cj, &event, Some(&seed), 1)
        .unwrap()
        .unwrap();
    let next = prepare_event(&j, &cj, &event, Some(&seed), 2)
        .unwrap()
        .unwrap();
    assert_eq!(first, next);
    let Command::CheckIn(ci) = decode_request(&first).unwrap() else {
        panic!()
    };
    assert_eq!(ci.operation.as_str(), first.operation_id.to_string());
    assert_eq!(ci.claim.role, CallerRole::TopLevel);
    assert_eq!(
        ci.mode,
        WireMode::Lifecycle {
            expected_binding_generation: 0
        }
    );
    assert_ne!(ci.claim.execution.as_str(), seed.execution.to_string());
    assert!(
        ci.claim
            .native_session
            .as_str()
            .starts_with("plugin_context:")
    );
    drop(cj);
    let cj = ContextJournal::open(&root, seed.instance, "seat", CONTEXT_LOCK_WAIT).unwrap();
    assert_eq!(
        prepare_event(&j, &cj, &event, Some(&seed), 3)
            .unwrap()
            .unwrap(),
        first
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn child_and_missing_current_never_dispatch_or_create_intent() {
    let (root, j, cj, seed, mut event) = fixture();
    event.role = Role::Subagent;
    assert!(
        prepare_event(&j, &cj, &event, Some(&seed), 1)
            .unwrap()
            .is_none()
    );
    assert!(cj.pending().unwrap().is_none());
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    event.role = Role::TopLevel;
    event.kind = EventKind::Tool;
    assert!(prepare_event(&j, &cj, &event, None, 1).is_err());
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    fs::remove_dir_all(root).unwrap();
}

struct Transport {
    calls: std::sync::Mutex<Vec<Command>>,
    lose_first: bool,
    reject: bool,
    historical: bool,
    directory: Option<crate::protocol::results::CommandResult>,
    directory_error: bool,
    offered_through: Option<String>,
}
impl Transport {
    fn new() -> Self {
        Self {
            calls: Default::default(),
            lose_first: false,
            reject: false,
            historical: false,
            offered_through: None,
            directory: None,
            directory_error: false,
        }
    }
}
impl crate::ports::LocalClient for Transport {
    crate::default_output_local_client!();
    fn call(
        &self,
        command: Command,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError> {
        assert!(budget.deadline.0 <= 5100);
        let mut calls = self.calls.lock().unwrap();
        calls.push(command.clone());
        if matches!(command, Command::Directory(_)) {
            if self.directory_error {
                return Err(crate::protocol::results::ApiError::invalid_budget(
                    "overview unavailable",
                )
                .with_required_minimum_bytes(4096));
            }
            return Ok(self
                .directory
                .clone()
                .unwrap_or_else(|| crate::protocol::results::CommandResult::Directory(empty())));
        }
        if self.reject || (self.lose_first && calls.len() == 1) {
            return Err(crate::protocol::results::ApiError::new(
                if self.reject {
                    crate::protocol::results::ErrorCode::Conflict
                } else {
                    crate::protocol::results::ErrorCode::UnknownOutcome
                },
                "response lost or CAS conflict",
            ));
        }
        let Command::CheckIn(ci) = command else {
            panic!("hook may only check in")
        };
        let mut claim = ci.claim;
        if let WireMode::Lifecycle {
            expected_binding_generation,
        } = ci.mode
        {
            claim.binding_generation = expected_binding_generation + 1;
        }
        Ok(crate::protocol::results::CommandResult::CheckedIn(
            crate::protocol::results::CheckInResult {
                context: claim.clone(),
                context_disposition: if self.historical {
                    crate::protocol::results::CheckInContextDisposition::Historical
                } else {
                    crate::protocol::results::CheckInContextDisposition::Current
                },
                seat: claim.seat,
                offered_through: self.offered_through.clone(),
                warning_count: 0,
                warning_count_has_more: false,
                warnings: empty(),
                notices: Default::default(),
                inbox: empty(),
            },
        ))
    }
}
fn empty<T>() -> crate::protocol::pagination::Page<T> {
    crate::protocol::pagination::Page {
        items: vec![],
        has_more: false,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        consistency: crate::protocol::pagination::Consistency::BoundedLive,
        stop_reason: crate::protocol::pagination::StopReason::Complete,
    }
}
struct Clock;
impl crate::protocol::time::Clock for Clock {
    fn utc_now(&self) -> crate::protocol::time::UtcMillis {
        crate::protocol::time::UtcMillis(100)
    }
    fn monotonic_now(&self) -> crate::protocol::time::MonoInstant {
        crate::protocol::time::MonoInstant(100)
    }
}
#[test]
fn response_loss_restart_retries_exact_command_and_current_reuses_execution() {
    let (root, j, cj, seed, mut event) = fixture();
    let mut client = Transport::new();
    client.lose_first = true;
    let mut out = vec![];
    assert!(
        run_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            1,
            &client,
            &Clock,
            &Default::default(),
            &mut out
        )
        .is_err()
    );
    let frozen = cj.pending().unwrap().unwrap();
    let key = frozen.operation_id;
    drop(cj);
    let cj = ContextJournal::open(&root, seed.instance, "seat", CONTEXT_LOCK_WAIT).unwrap();
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls[0], calls[1]);
    drop(calls);
    assert!(!out.is_empty());
    assert_eq!(
        cj.current().unwrap().unwrap().execution,
        frozen.context.execution
    );
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    event.kind = EventKind::Tool;
    event.event_id = "tool-two".into();
    let current = prepare_event(&j, &cj, &event, None, 3).unwrap().unwrap();
    assert_eq!(current.context.execution, frozen.context.execution);
    assert_eq!(current.expected_generation, None);
    assert_ne!(current.operation_id, key);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn flush_loss_reuses_cached_response_without_transport_and_only_then_completes_intent() {
    use std::io::{self, Write};
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush loss"))
        }
    }
    let (root, j, cj, seed, event) = fixture();
    let client = Transport::new();
    assert!(
        run_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            1,
            &client,
            &Clock,
            &Default::default(),
            &mut Broken
        )
        .is_err()
    );
    assert!(cj.current().unwrap().is_some());
    assert!(cj.pending().unwrap().is_none());
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 1);
    let mut out = vec![];
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn failed_cas_is_preserved_and_ended_historical_response_does_not_install() {
    let (root, j, cj, seed, event) = fixture();
    let mut client = Transport::new();
    client.reject = true;
    let mut out = vec![];
    assert!(
        run_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            1,
            &client,
            &Clock,
            &Default::default(),
            &mut out
        )
        .is_err()
    );
    let first = cj.pending().unwrap().unwrap();
    assert!(
        run_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            2,
            &client,
            &Clock,
            &Default::default(),
            &mut out
        )
        .is_err()
    );
    assert_eq!(first, cj.pending().unwrap().unwrap());
    assert!(cj.current().unwrap().is_none());
    {
        let calls = client.calls.lock().unwrap();
        assert_eq!(calls[0], calls[1]);
    }
    client.reject = false;
    client.historical = true;
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        3,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    assert!(cj.current().unwrap().is_none());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn duplicate_concurrent_hook_allocates_one_intent_and_execution() {
    let (root, j, cj, seed, event) = fixture();
    let barrier = std::sync::Barrier::new(2);
    let (a, b) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            prepare_event(&j, &cj, &event, Some(&seed), 1)
                .unwrap()
                .unwrap()
        });
        let second = scope.spawn(|| {
            barrier.wait();
            prepare_event(&j, &cj, &event, Some(&seed), 1)
                .unwrap()
                .unwrap()
        });
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_eq!(a, b);
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 1);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn edited_payload_cannot_dispatch_and_native_prefix_cannot_impersonate_fallback() {
    let (root, j, cj, mut seed, event) = fixture();
    seed.session = SessionReference::Native("plugin_context:spoof".into());
    assert!(caller_claim(&seed).is_err());
    seed.session = SessionReference::Native("native".into());
    let mut req = prepare_event(&j, &cj, &event, Some(&seed), 1)
        .unwrap()
        .unwrap();
    req.expected_generation = Some(99);
    assert!(decode_request(&req).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn hook_emits_compact_instructions_without_ack_and_child_emits_only_summary_guidance() {
    let (root, j, cj, seed, mut event) = fixture();
    let client = Transport::new();
    let mut out = vec![];
    run_hook_event(&j, &cj, &event, Some(&seed), 1, &client, &Clock, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("text inbox ACKs only complete pending agent messages"));
    assert!(text.contains("after output is written and flushed"));
    assert!(text.contains("Accept invitations separately"));
    assert_eq!(client.calls.lock().unwrap().len(), 2);
    event.role = Role::Subagent;
    event.event_id = "child".into();
    let mut out = vec![];
    run_hook_event(&j, &cj, &event, None, 2, &client, &Clock, &mut out).unwrap();
    assert_eq!(client.calls.lock().unwrap().len(), 2);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("never check in for this seat, accept or ACK")
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn lifecycle_overview_preserves_server_page_and_signed_age() {
    use crate::protocol::{
        ids::ThreadId,
        pagination::{Consistency, Page, StopReason},
        results::{CommandResult, ThreadSummary},
        time::UtcMillis,
    };
    let (root, j, cj, seed, event) = fixture();
    let mut client = Transport::new();
    let summary = |id: &str, created_at| ThreadSummary {
        last_activity: None,
        name: None,
        thread: ThreadId::new(id),
        managed_owner: None,
        topic_data: "peer \"topic\"\nnext".into(),
        topic_omitted: false,
        topic_detail_argv: None,
        archived: id == "archived",
        orphaned: false,
        message_count: 7,
        created_at: UtcMillis(created_at),
        ordinary_count: 5,
        system_count: 2,
        joined_count: 3,
    };
    client.directory = Some(CommandResult::Directory(Page {
        items: vec![
            summary("joined", 80),
            summary("invited", 110),
            summary("archived", 90),
        ],
        next_cursor: Some("opaque-server-cursor".into()),
        next_argv: Some(vec![
            "herdr-threads".into(),
            "thread".into(),
            "list".into(),
            "--cursor".into(),
            "opaque-server-cursor".into(),
        ]),
        high_water_ordinal: 9,
        scope_revision: Some(4),
        has_more: true,
        stop_reason: StopReason::Work,
        consistency: Consistency::BoundedLive,
    }));
    let mut output = Vec::new();
    run_hook_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &mut output,
    )
    .unwrap();
    assert!(output.len() <= 65_536);
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Current directory overview at presentation time"));
    assert!(text.contains("Original cached CheckIn offer"));
    assert!(text.contains("opaque-server-cursor"));
    assert!(text.contains("\\\"topic\\\"\\nnext"));
    assert!(text.contains("\"age_millis_signed\":\"-10\""));
    assert!(text.contains("\"timeline_messages\":7"));
    assert!(text.contains("\"joined_nonretired_participants\":3"));
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(matches!(&calls[0], Command::CheckIn(_)));
    assert!(matches!(&calls[1], Command::Directory(q)
        if q.membership.as_ref().is_some_and(|s| s.as_str()=="seat")
            && q.membership_filter == crate::protocol::commands::DirectoryMembership::Default
            && q.topic_contains.is_none() && q.page.limit == 8
            && q.page.max_bytes <= 16_384 && q.page.cursor.is_none()));
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn overview_failure_and_reopen_reuse_cached_check_in_and_pending_intent() {
    let (root, j, cj, seed, event) = fixture();
    let mut client = Transport::new();
    client.directory_error = true;
    let mut diagnostic = Vec::new();
    assert!(
        run_hook_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            1,
            &client,
            &Clock,
            &mut diagnostic
        )
        .is_err()
    );
    let text = String::from_utf8(diagnostic).unwrap();
    assert!(text.contains("overview unavailable"));
    assert!(text.contains("\"thread\",\"list\",\"--seat\",\"seat\""));
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 1);
    assert!(cj.pending().unwrap().is_none());
    drop(cj);
    let cj = ContextJournal::open(&root, seed.instance, "seat", CONTEXT_LOCK_WAIT).unwrap();
    client.directory_error = false;
    let mut output = Vec::new();
    run_hook_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &mut output,
    )
    .unwrap();
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Directory(_)))
            .count(),
        2
    );
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quiet_current_hook_does_not_query_directory() {
    let (root, j, cj, seed, mut event) = fixture();
    let mut client = Transport::new();
    // The store always reports its offer frontier (`Some(seq)`); an empty
    // inbox/warning offer is still routine no-change output.
    client.offered_through = Some("7".into());
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &Default::default(),
        &mut Vec::new(),
    )
    .unwrap();
    event.kind = EventKind::Tool;
    event.event_id = "tool-quiet".into();
    let mut output = Vec::new();
    run_hook_event(&j, &cj, &event, None, 2, &client, &Clock, &mut output).unwrap();
    assert!(output.is_empty());
    let calls = client.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|c| matches!(c, Command::CheckIn(_))));
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_client_rejects_nondefault_selected_output_before_dispatch() {
    use crate::ports::LocalClient;
    use crate::protocol::{
        output::{OutputFormat, OutputSpec},
        results::ErrorCode,
        time::{CallBudget, Cancellation, MonoInstant},
    };
    let client = Transport::new();
    let selected = OutputSpec {
        format: OutputFormat::Text,
        ..OutputSpec::default()
    };
    let error = client
        .call_with_output(
            Command::Health,
            &selected,
            &CallBudget {
                deadline: MonoInstant(200),
                cancellation: Cancellation::default(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(client.calls.lock().unwrap().is_empty());
}

#[test]
fn explicit_recovery_on_current_event_reads_one_directory_page() {
    let (root, j, cj, seed, mut event) = fixture();
    let client = Transport::new();
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &Default::default(),
        &mut Vec::new(),
    )
    .unwrap();
    event.kind = EventKind::Compact;
    event.event_id = "recovery-current".into();
    let mut output = Vec::new();
    run_hook_event_with_reason(
        &j,
        &cj,
        &event,
        None,
        2,
        &client,
        &Clock,
        &Default::default(),
        OverviewReason::Recovery,
        &mut output,
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Current directory overview at presentation time"));
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Directory(_)))
            .count(),
        1
    );
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn final_hook_encoder_fits_exact_byte_limit_and_rejects_one_extra_byte() {
    let output = crate::protocol::output::OutputSpec::default();
    let empty = encode_hook(
        &HookInput {
            instruction: "instructions",
            original: OriginalOffer::Inline(&[]),
            overview: OverviewPresentation::NotRequested,
            output: &output,
            presentation_now_millis: 0,
        },
        65_536,
    )
    .unwrap();
    let payload = vec![b'x'; 65_536 - empty.as_bytes().len()];
    let at_limit = encode_hook(
        &HookInput {
            instruction: "instructions",
            original: OriginalOffer::Inline(&payload),
            overview: OverviewPresentation::NotRequested,
            output: &output,
            presentation_now_millis: 0,
        },
        65_536,
    )
    .unwrap();
    assert_eq!(at_limit.as_bytes().len(), 65_536);
    let over = vec![b'x'; payload.len() + 1];
    assert!(matches!(
        encode_hook(
            &HookInput {
                instruction: "instructions",
                original: OriginalOffer::Inline(&over),
                overview: OverviewPresentation::NotRequested,
                output: &output,
                presentation_now_millis: 0,
            },
            65_536
        ),
        Err(PresentationError::TooLarge)
    ));
}

#[test]
fn partial_hook_write_keeps_original_intent_and_cached_response() {
    use std::io::{self, Write};
    struct Partial {
        wrote: usize,
    }
    impl Write for Partial {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.wrote == 0 {
                self.wrote = bytes.len().min(10);
                Ok(self.wrote)
            } else {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "partial loss"))
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (root, j, cj, seed, event) = fixture();
    let client = Transport::new();
    let mut partial = Partial { wrote: 0 };
    assert!(
        run_hook_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            1,
            &client,
            &Clock,
            &mut partial
        )
        .is_err()
    );
    assert_eq!(partial.wrote, 10);
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 1);
    assert!(cj.pending().unwrap().is_none());
    let mut output = Vec::new();
    run_hook_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &mut output,
    )
    .unwrap();
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 0);
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Directory(_)))
            .count(),
        2
    );
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hook_flush_loss_keeps_intent_until_duplicate_delivery_succeeds() {
    use std::io::{self, Write};
    struct BrokenFlush(Vec<u8>);
    impl Write for BrokenFlush {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush loss"))
        }
    }
    let (root, j, cj, seed, event) = fixture();
    let client = Transport::new();
    let mut broken = BrokenFlush(Vec::new());
    assert!(
        run_hook_event(
            &j,
            &cj,
            &event,
            Some(&seed),
            1,
            &client,
            &Clock,
            &mut broken
        )
        .is_err()
    );
    assert!(!broken.0.is_empty());
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 1);
    let mut output = Vec::new();
    run_hook_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &mut output,
    )
    .unwrap();
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 0);
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Directory(_)))
            .count(),
        2
    );
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_completed_offer_is_referenced_without_replaying_check_in() {
    use crate::protocol::time::{CallBudget, Cancellation, MonoInstant};
    let (root, j, cj, seed, event) = fixture();
    let client = Transport::new();
    let request = prepare_event(&j, &cj, &event, Some(&seed), 1)
        .unwrap()
        .unwrap();
    let result = crate::ports::LocalClient::call(
        &client,
        decode_request(&request).unwrap(),
        &CallBudget {
            deadline: MonoInstant(5100),
            cancellation: Cancellation::default(),
        },
    )
    .unwrap();
    let mut saved_json = serde_json::to_value(&result).unwrap();
    saved_json["data"]["warnings"]["has_more"] = serde_json::json!(true);
    saved_json["data"]["warnings"]["next_cursor"] = serde_json::json!("original-cursor");
    saved_json["data"]["warnings"]["next_argv"] = serde_json::json!(["x".repeat(18_000)]);
    saved_json["data"]["warnings"]["stop_reason"] = serde_json::json!("work");
    let original = serde_json::to_vec(&saved_json).unwrap();
    assert!(original.len() > 16_384 && original.len() < 65_536);
    let mut next = request.context.clone();
    next.binding_generation += 1;
    let cached = cj
        .dispatch(&event.event_id, &mut |_pending: &PendingCheckIn| {
            Ok(CheckInResponse {
                context: next.clone(),
                historical: false,
                output: original.clone(),
            })
        })
        .unwrap();
    assert_eq!(cached.output, original);
    let mut output = Vec::new();
    run_hook_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &mut output,
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("immutable cached pages"));
    let command_line = text
        .split("Read its immutable cached pages with argv (JSON data):\n")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let argv: Vec<String> = serde_json::from_str(command_line).unwrap();
    assert!(crate::cli::commands::parse_argv(argv.clone()).is_ok());
    let token = argv.windows(2).find(|w| w[0] == "--reference").unwrap()[1].clone();
    let reference = crate::harness::cache::CacheRefV1::parse(&token).unwrap();
    assert_eq!(reference.key.event_id, event.event_id);
    assert_eq!(
        cj.dispatch(&event.event_id, &mut |_pending: &PendingCheckIn| panic!(
            "no replay"
        ))
        .unwrap()
        .output,
        original
    );
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Directory(_)))
            .count(),
        1
    );
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_first_check_in_and_changed_selector_duplicate_keep_original_continuation() {
    use crate::protocol::{
        output::{ContinuationContext, OutputFormat, OutputSpec},
        pagination::StopReason,
        results::{ApiError, CommandResult},
        time::CallBudget,
    };
    struct SelectedTransport {
        inner: Transport,
        selected: std::sync::Mutex<Vec<OutputSpec>>,
    }
    impl crate::ports::LocalClient for SelectedTransport {
        fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
            self.inner.call(command, budget)
        }
        fn call_with_output(
            &self,
            command: Command,
            output: &OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.selected.lock().unwrap().push(output.clone());
            let mut result = self.inner.call(command, budget)?;
            if let CommandResult::CheckedIn(check) = &mut result {
                check.warnings.has_more = true;
                check.warnings.next_cursor = Some("original-page".into());
                check.warnings.stop_reason = StopReason::Work;
                check.warnings.next_argv = Some(vec![
                    "herdr-threads".into(),
                    "--state-dir".into(),
                    output.context.state_dir.clone().unwrap(),
                    "--host-endpoint".into(),
                    output.context.host.as_ref().unwrap().as_str().into(),
                    "warnings".into(),
                    "--cursor".into(),
                    "original-page".into(),
                ]);
            }
            if let CommandResult::Directory(page) = &mut result {
                page.has_more = true;
                page.next_cursor = Some("current-directory-page".into());
                page.stop_reason = StopReason::Work;
                page.next_argv = Some(vec![
                    "herdr-threads".into(),
                    "--state-dir".into(),
                    output.context.state_dir.clone().unwrap(),
                    "--host-endpoint".into(),
                    output.context.host.as_ref().unwrap().as_str().into(),
                    "thread".into(),
                    "list".into(),
                    "--cursor".into(),
                    "current-directory-page".into(),
                ]);
            }
            Ok(result)
        }
    }
    let (root, j, cj, seed, event) = fixture();
    let client = SelectedTransport {
        inner: Transport::new(),
        selected: Default::default(),
    };
    let selected = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some("/tmp/first-state".into()),
            host: Some("/tmp/first-host".into()),
        },
    };
    let mut first_output = Vec::new();
    run_hook_event_with_reason(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &selected,
        OverviewReason::Lifecycle,
        &mut first_output,
    )
    .unwrap();
    let first_text = String::from_utf8(first_output).unwrap();
    assert!(first_text.contains("/tmp/first-state"));
    assert!(first_text.contains("original-page"));
    let first_cached = cj
        .dispatch(&event.event_id, &mut |_pending: &PendingCheckIn| {
            panic!("cached")
        })
        .unwrap()
        .output;
    let changed = OutputSpec {
        format: OutputFormat::Json,
        context: ContinuationContext {
            state_dir: Some("/tmp/changed-state".into()),
            host: Some("/tmp/changed-host".into()),
        },
    };
    let mut second_output = Vec::new();
    run_hook_event_with_reason(
        &j,
        &cj,
        &event,
        Some(&seed),
        2,
        &client,
        &Clock,
        &changed,
        OverviewReason::Lifecycle,
        &mut second_output,
    )
    .unwrap();
    let second_text = String::from_utf8(second_output).unwrap();
    assert!(second_text.contains("/tmp/first-state"));
    assert!(second_text.contains("/tmp/changed-state"));
    assert_eq!(
        cj.dispatch(&event.event_id, &mut |_pending: &PendingCheckIn| panic!(
            "cached"
        ))
        .unwrap()
        .output,
        first_cached
    );
    let calls = client.inner.calls.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::Directory(_)))
            .count(),
        2
    );
    assert_eq!(
        client.selected.lock().unwrap().as_slice(),
        &[selected.clone(), selected, changed]
    );
    drop(calls);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_selected_hook_fails_before_first_check_in_dispatch() {
    use crate::protocol::{
        output::{ContinuationContext, OutputFormat, OutputSpec},
        results::ErrorCode,
    };
    let (root, j, cj, seed, event) = fixture();
    let client = Transport::new();
    let selected = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some("/tmp/selected".into()),
            host: None,
        },
    };
    let mut output = Vec::new();
    let error = run_hook_event_with_reason(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &selected,
        OverviewReason::Lifecycle,
        &mut output,
    )
    .unwrap_err();
    assert!(matches!(error, BridgeError::Api(ref api) if api.code == ErrorCode::InvalidRequest));
    assert!(client.calls.lock().unwrap().is_empty());
    assert!(output.is_empty());
    assert!(cj.pending().unwrap().is_some());
    assert_eq!(j.page(&Default::default()).unwrap().items.len(), 1);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn successor_keeps_context_when_completed_predecessor_hook_is_delivered_again() {
    let (root, j, cj, seed, mut event) = fixture();
    event.native_session = Some("same-native-session".into());
    let client = Transport::new();
    let mut out = vec![];
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    let first = cj.current().unwrap().unwrap();
    let old = event.clone();
    event.kind = EventKind::Resume;
    event.event_id = "resume-new-execution".into();
    run_event(
        &j,
        &cj,
        &event,
        None,
        2,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    let successor = cj.current().unwrap().unwrap();
    assert_eq!(successor.binding_generation, 2);
    assert_eq!(successor.session, first.session);
    assert_ne!(successor.execution, first.execution);
    for replacement in [None, Some("changed-native-session".into())] {
        let mut edited = old.clone();
        edited.native_session = replacement;
        assert!(matches!(
            run_event(
                &j,
                &cj,
                &edited,
                None,
                3,
                &client,
                &Clock,
                &Default::default(),
                &mut out,
            ),
            Err(BridgeError::Context(ContextError::Conflict))
        ));
    }
    assert_eq!(cj.current().unwrap().unwrap(), successor);
    assert_eq!(client.calls.lock().unwrap().len(), 2);
    run_event(
        &j,
        &cj,
        &old,
        None,
        3,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    assert_eq!(cj.current().unwrap().unwrap(), successor);
    assert_eq!(client.calls.lock().unwrap().len(), 2);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn explicit_model_accept_and_ack_are_separate_calls_after_hook_registration() {
    use crate::protocol::{
        ids::InvitationId,
        results::{AckResult, CommandResult},
    };
    let (root, j, cj, seed, event) = fixture();
    let check = Transport::new();
    let mut out = vec![];
    run_hook_event(&j, &cj, &event, Some(&seed), 1, &check, &Clock, &mut out).unwrap();
    assert_eq!(check.calls.lock().unwrap().len(), 2);
    struct Model(std::sync::Mutex<Vec<Command>>);
    impl crate::ports::LocalClient for Model {
        crate::default_output_local_client!();
        fn call(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, crate::protocol::results::ApiError> {
            self.0.lock().unwrap().push(command.clone());
            Ok(match command {
                Command::Accept(_) => {
                    CommandResult::Accepted(InvitationId::new("invitation").into())
                }
                Command::Ack(ack) => CommandResult::Acknowledged(AckResult {
                    acknowledged: ack.messages,
                    already_acknowledged: vec![],
                }),
                _ => panic!("only explicit model mutation"),
            })
        }
    }
    let model = Model(Default::default());
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "accept", "thread"]).unwrap();
    crate::cli::run_cooperative(
        parsed,
        &j,
        &cj,
        None,
        Role::TopLevel,
        &model,
        &Clock,
        &mut out,
    )
    .unwrap();
    let parsed = crate::cli::commands::parse_argv(["herdr-threads", "ack", "message"]).unwrap();
    crate::cli::run_cooperative(
        parsed,
        &j,
        &cj,
        None,
        Role::TopLevel,
        &model,
        &Clock,
        &mut out,
    )
    .unwrap();
    let calls = model.0.lock().unwrap();
    assert!(matches!(&calls[0],Command::Accept(a) if a.claim.binding_generation==1));
    assert!(
        matches!(&calls[1],Command::Ack(a) if a.claim.binding_generation==1 && a.messages[0].as_str()=="message")
    );
    fs::remove_dir_all(root).unwrap();
}
/// Every code: the eight deterministic codes discard, every other code keeps;
/// an uncorrelated failure always keeps.
#[test]
fn cooperative_typed_rejection_discards_intent_but_transport_failure_keeps_it() {
    use crate::protocol::results::{ApiError, CommandResult, ErrorCode};
    struct Answer(Result<Result<CommandResult, ApiError>, ApiError>);
    impl crate::ports::LocalClient for Answer {
        crate::default_output_local_client!();
        fn call(
            &self,
            _: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<CommandResult, ApiError> {
            unreachable!("cooperative mutations use the definitive call")
        }
        fn call_definitive(
            &self,
            command: Command,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<Result<CommandResult, ApiError>, ApiError> {
            assert!(matches!(command, Command::SendMessage(_)));
            self.0.clone()
        }
    }
    let error = |code| ApiError::new(code, "answer");
    // The cooperative codes that prove an identical retry is refused
    // identically (src/cli/retry.rs is_deterministic_rejection). Written out
    // here, not imported, so a change to the list must change this test.
    const DISCARDED: &[ErrorCode] = &[
        ErrorCode::InvalidRequest,
        ErrorCode::Unauthorized,
        ErrorCode::Archived,
        ErrorCode::Conflict,
        ErrorCode::OperationPayloadMismatch,
        ErrorCode::MembershipRequired,
        ErrorCode::NotFound,
        ErrorCode::StaleRequirementAcceptance,
    ];
    type Reply = Result<Result<CommandResult, ApiError>, ApiError>;
    let mut answers: Vec<(Reply, usize)> = ErrorCode::ALL
        .iter()
        .map(|code| {
            (
                Ok(Err(error(code.clone()))),
                usize::from(!DISCARDED.contains(code)),
            )
        })
        .collect();
    // An uncorrelated failure (transport error) never discards, even with a
    // deterministic code.
    answers.push((Err(error(ErrorCode::HostUnavailable)), 1));
    answers.push((Err(error(ErrorCode::NotFound)), 1));
    answers.push((Err(error(ErrorCode::InvalidRequest)), 1));
    for (answer, pending) in answers {
        let (root, j, cj, seed, event) = fixture();
        let check = Transport::new();
        let mut out = vec![];
        run_hook_event(&j, &cj, &event, Some(&seed), 1, &check, &Clock, &mut out).unwrap();
        let before = j.page(&Default::default()).unwrap().items.len();
        let parsed =
            crate::cli::commands::parse_argv(["herdr-threads", "send", "t1", "--body", "hi"])
                .unwrap();
        let expected = match &answer {
            Ok(Err(e)) | Err(e) => e.code.clone(),
            Ok(Ok(_)) => unreachable!(),
        };
        let failure = crate::cli::run_cooperative(
            parsed,
            &j,
            &cj,
            None,
            Role::TopLevel,
            &Answer(answer),
            &Clock,
            &mut out,
        )
        .unwrap_err();
        assert!(
            matches!(&failure, crate::cli::RunError::Api(e) if e.code == expected),
            "{failure:?}"
        );
        let after = j.page(&Default::default()).unwrap().items.len();
        assert_eq!(after - before, pending, "{expected:?}");
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn cross_instance_context_diagnostic_precedes_any_new_hook_intent() {
    let (root, j, cj, seed, event) = fixture();
    let client = Transport::new();
    let mut out = vec![];
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &Default::default(),
        &mut out,
    )
    .unwrap();
    drop(cj);
    let other =
        ContextJournal::open(&root, uuid::Uuid::from_u128(999), "seat", CONTEXT_LOCK_WAIT).unwrap();
    assert_eq!(
        prepare_event(&j, &other, &event, Some(&seed), 2),
        Err(ContextError::WrongInstance)
    );
    assert_eq!(client.calls.lock().unwrap().len(), 1);
    assert!(j.page(&Default::default()).unwrap().items.is_empty());
    fs::remove_dir_all(root).unwrap();
}

fn offer(
    claim: crate::protocol::authority::CallerClaim,
    inbox: Vec<crate::protocol::results::InboxItem>,
) -> crate::protocol::results::CheckInResult {
    let mut page = empty();
    page.items = inbox;
    crate::protocol::results::CheckInResult {
        context: claim.clone(),
        context_disposition: crate::protocol::results::CheckInContextDisposition::Current,
        seat: claim.seat,
        offered_through: Some("9".into()),
        warning_count: 0,
        warning_count_has_more: false,
        warnings: empty(),
        notices: Default::default(),
        inbox: page,
    }
}
fn inbox_item(thread: &str, invitations: u64, pending: u64) -> crate::protocol::results::InboxItem {
    crate::protocol::results::InboxItem {
        thread: crate::protocol::ids::ThreadId::new(thread),
        invitations,
        invitations_has_more: false,
        pending_receipts: pending,
        pending_receipts_has_more: false,
        warnings: 0,
        warnings_has_more: false,
        pending_requirement: None,
    }
}

/// Seat mailbox with its own decision sequence. Every arriving message gets a
/// publication key greater than every earlier one, exactly like the store.
/// Scripted changes land at precise points: after the digest read, or after
/// the next CheckIn has been answered, so a read's position relative to the
/// offer decides what it sees.
#[derive(Default)]
struct Mailbox {
    /// Pending messages with their publication sequence.
    pending: std::sync::Mutex<Vec<(&'static str, u64)>>,
    next_seq: std::sync::Mutex<u64>,
    arrive_after_digest: std::sync::Mutex<Vec<&'static str>>,
    arrive_after_check_in: std::sync::Mutex<Vec<&'static str>>,
    ack_after_check_in: std::sync::Mutex<Vec<&'static str>>,
    fail_digest: bool,
    calls: std::sync::Mutex<Vec<&'static str>>,
    /// Extra unavailability episodes of the seat itself (the digest reports
    /// 1 + this).
    own_episodes: std::sync::Mutex<u64>,
    /// Notices become carriable independently of their immutable publication token.
    pending_notices: std::sync::Mutex<usize>,
}
impl Mailbox {
    fn with(pending: &[&'static str], arriving: &[&'static str]) -> Self {
        let mailbox = Self::default();
        *mailbox.next_seq.lock().unwrap() = 10;
        mailbox.arrive(pending);
        *mailbox.arrive_after_check_in.lock().unwrap() = arriving.to_vec();
        mailbox
    }
    fn arrive(&self, ids: &[&'static str]) {
        let mut seq = self.next_seq.lock().unwrap();
        for id in ids {
            *seq += 1;
            self.pending.lock().unwrap().push((id, *seq));
        }
    }
    fn ack(&self, ids: &[&'static str]) {
        self.pending
            .lock()
            .unwrap()
            .retain(|(id, _)| !ids.contains(id));
    }
    fn digest(&self) -> crate::protocol::attention::AttentionDigest {
        use crate::protocol::attention::*;
        let pending = self.pending.lock().unwrap().clone();
        let mut newest = pending.clone();
        newest.sort_by(|a, b| b.1.cmp(&a.1));
        newest.truncate(MAX_DIGEST_IDS);
        AttentionDigest {
            version: DIGEST_VERSION,
            seat: crate::protocol::ids::SeatId::new("seat"),
            token: AttentionToken {
                receipt: pending.iter().map(|(_, seq)| (*seq, 0)).max(),
                unavailability_episode: 1 + *self.own_episodes.lock().unwrap(),
                ..Default::default()
            },
            invitations: AttentionClass::default(),
            receipts: AttentionClass {
                count: pending.len() as u64,
                has_more: pending.len() > newest.len(),
                count_has_more: false,
                items: newest
                    .iter()
                    .map(|(id, _)| AttentionRef {
                        id: (*id).into(),
                        thread: crate::protocol::ids::ThreadId::new("t1"),
                        requirement: None,
                    })
                    .collect(),
            },
            warnings: AttentionClass::default(),
            unavailability_open: false,
        }
    }
    fn items(&self) -> Vec<crate::protocol::results::InboxItem> {
        let n = self.pending.lock().unwrap().len() as u64;
        if n == 0 {
            vec![]
        } else {
            vec![inbox_item("t1", 0, n)]
        }
    }
    fn calls(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}
impl crate::ports::LocalClient for Mailbox {
    crate::default_output_local_client!();
    fn supports_capability(&self, name: &str, _budget: &crate::protocol::time::CallBudget) -> bool {
        name == crate::protocol::capabilities::ATTENTION_NOTICE_DELIVERY
    }
    fn call(
        &self,
        command: Command,
        _budget: &crate::protocol::time::CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError> {
        use crate::protocol::results::CommandResult as R;
        let delivery = matches!(&command, Command::AttentionDigestDelivery(_));
        match command {
            Command::AttentionDigest(query) | Command::AttentionDigestDelivery(query) => {
                self.calls.lock().unwrap().push("digest");
                assert_eq!(query.seat.as_str(), "seat");
                if self.fail_digest {
                    return Err(crate::protocol::results::ApiError::read_budget_exhausted(
                        "bounded read exhausted",
                    ));
                }
                let digest = self.digest();
                let arriving: Vec<_> = self.arrive_after_digest.lock().unwrap().drain(..).collect();
                self.arrive(&arriving);
                if delivery {
                    Ok(R::AttentionDigestDelivery {
                        digest,
                        notices_pending: *self.pending_notices.lock().unwrap() > 0,
                    })
                } else {
                    Ok(R::AttentionDigest(digest))
                }
            }
            Command::Directory(_) => Ok(R::Directory(empty())),
            Command::CheckIn(ci) => {
                self.calls.lock().unwrap().push("check_in");
                let mut claim = ci.claim;
                if let WireMode::Lifecycle {
                    expected_binding_generation,
                } = ci.mode
                {
                    claim.binding_generation = expected_binding_generation + 1;
                }
                let mut check = offer(claim, self.items());
                let mut notices = self.pending_notices.lock().unwrap();
                let carried = (*notices).min(crate::protocol::results::MAX_NOTICE_PAGE_ITEMS);
                *notices -= carried;
                check.notices.items = (0..carried)
                    .map(|n| crate::protocol::results::WarningRef {
                        warning: crate::protocol::ids::MessageId::new(format!(
                            "notice-{}",
                            *notices + n
                        )),
                        thread: crate::protocol::ids::ThreadId::new("t1"),
                        sequence: 1,
                        event_seq: 1,
                    })
                    .collect();
                check.notices.has_more = *notices > 0;
                let acked: Vec<_> = self.ack_after_check_in.lock().unwrap().drain(..).collect();
                self.ack(&acked);
                let arriving: Vec<_> = self
                    .arrive_after_check_in
                    .lock()
                    .unwrap()
                    .drain(..)
                    .collect();
                self.arrive(&arriving);
                Ok(R::CheckedIn(check))
            }
            other => panic!("unexpected hook call {other:?}"),
        }
    }
}
fn tool_event() -> LifecycleEvent {
    LifecycleEvent {
        harness: Harness::Codex,
        source: "PreToolUse".into(),
        kind: EventKind::Tool,
        native_session: None,
        role: Role::TopLevel,
        event_id: "tool".into(),
        capability: Capability::SourceSupported,
    }
}
fn budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(5000),
        cancellation: Default::default(),
    }
}
/// Register the seat's execution through a lifecycle CheckIn, seeding the
/// attention mark the way the hook does. Returns whether a seed was produced.
fn register(
    j: &Journal,
    cj: &ContextJournal,
    event: &LifecycleEvent,
    seed: &OccupantContext,
    client: &Mailbox,
) -> bool {
    let seat = crate::protocol::ids::SeatId::new("seat");
    let mut out = vec![];
    let seeded = seeded_lifecycle(cj, &event.event_id, client, &seat, &budget(), || {
        run_event(
            j,
            cj,
            event,
            Some(seed),
            1,
            client,
            &Clock,
            &Default::default(),
            &mut out,
        )
        .map(|_| ())
    })
    .unwrap();
    match seeded {
        Some((execution, digest)) => {
            cj.set_attention_mark(execution, &digest.token).unwrap();
            true
        }
        None => false,
    }
}
fn boundary(cj: &ContextJournal, client: &Mailbox, coalesce: bool) -> ToolBoundary {
    let boundary = tool_boundary_check_in(
        cj,
        &tool_event(),
        client,
        &Clock,
        &Default::default(),
        &budget(),
        coalesce,
    )
    .unwrap();
    if let Some((execution, token)) = &boundary.mark {
        cj.set_attention_mark(*execution, token).unwrap();
    }
    boundary
}
fn tool_call(cj: &ContextJournal, client: &Mailbox) -> Vec<u8> {
    boundary(cj, client, true).text
}

// Logical attention can be unchanged while attribution finishes or a bounded
// notice page remains. Deliver each page, then coalesce again without token edits.
#[test]
fn late_notice_projection_and_remaining_pages_survive_tool_coalescing() {
    let (_root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&[], &[]);
    register(&j, &cj, &event, &seed, &client);
    assert!(tool_call(&cj, &client).is_empty());
    let execution = cj.current().unwrap().unwrap().execution;
    let mark = cj.attention_mark(execution).unwrap();
    *client.pending_notices.lock().unwrap() = 17;
    let first = boundary(&cj, &client, true);
    assert!(first.summary.unwrap().contains("offered notices:"));
    assert_eq!(*client.pending_notices.lock().unwrap(), 1);
    assert_eq!(cj.attention_mark(execution), Some(mark));
    assert!(
        boundary(&cj, &client, true)
            .summary
            .unwrap()
            .contains("offered notices:")
    );
    assert_eq!(*client.pending_notices.lock().unwrap(), 0);
    assert!(tool_call(&cj, &client).is_empty());
}

// Race, tool-boundary path: message B arrives after the CheckIn answered the
// offer. Behavioural kill for "digest read after the offer" (the mark would
// cover B and the next call be quiet) and for "mark taken from the offer's
// later state": B must be offered on the next call, once.
#[test]
fn message_arriving_after_the_tool_offer_is_offered_on_the_next_call() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&[], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    client.arrive(&["A"]);
    *client.arrive_after_check_in.lock().unwrap() = vec!["B"];
    let first = boundary(&cj, &client, true);
    assert!(!first.text.is_empty(), "A is new");
    assert!(first.summary.unwrap().contains("receipts=1 [A@t1]"));
    let second = boundary(&cj, &client, true);
    assert!(!second.text.is_empty(), "new message B suppressed");
    assert!(second.summary.unwrap().contains("receipts=2 [B@t1, A@t1]"));
    assert!(tool_call(&cj, &client).is_empty(), "then coalesced");
    fs::remove_dir_all(root).unwrap();
}

// Race: B arrives between the digest read and the CheckIn. The offer may
// already show B; the mark must still not cover it (a duplicate, never a
// suppression). Kills: taking the mark from a second digest read after the
// CheckIn (it would cover B and the next call be quiet).
#[test]
fn message_arriving_between_digest_and_offer_is_never_suppressed() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&[], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    client.arrive(&["A"]);
    *client.arrive_after_digest.lock().unwrap() = vec!["B"];
    assert!(!tool_call(&cj, &client).is_empty());
    assert!(!tool_call(&cj, &client).is_empty(), "B hidden by the mark");
    assert!(tool_call(&cj, &client).is_empty());
    fs::remove_dir_all(root).unwrap();
}

// Prior review N1, tool path: A is offered and marked; then A is ACKed and B
// arrives. The current maximum is B's key, above the mark, so B is offered.
// Kills: "digest read after the offer" (the post-offer read sees only B and
// would mark it) and an equality comparison (the ACK alone would emit).
#[test]
fn ack_then_arrival_after_the_tool_offer_is_offered_later() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&[], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    client.arrive(&["A"]);
    *client.ack_after_check_in.lock().unwrap() = vec!["A"];
    *client.arrive_after_check_in.lock().unwrap() = vec!["B"];
    assert!(!tool_call(&cj, &client).is_empty(), "A is new");
    let second = boundary(&cj, &client, true);
    assert!(
        !second.text.is_empty(),
        "B (arrived after the offer) suppressed"
    );
    assert!(second.summary.unwrap().contains("receipts=1 [B@t1]"));
    assert!(tool_call(&cj, &client).is_empty(), "then coalesced");
    // An ACK alone (B leaves, nothing arrives) is never an advance.
    client.ack(&["B"]);
    assert!(tool_call(&cj, &client).is_empty(), "an ACK alone emitted");
    fs::remove_dir_all(root).unwrap();
}

// Race, lifecycle seed path: B arrives after the lifecycle CheckIn answered.
// Kills: "seed digest read after the lifecycle CheckIn" (the seed would cover
// B and the first tool call be quiet).
#[test]
fn message_arriving_after_the_lifecycle_offer_is_offered_on_the_first_tool_call() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&["A"], &["B"]);
    assert!(register(&j, &cj, &event, &seed, &client));
    let first = boundary(&cj, &client, true);
    assert!(!first.text.is_empty(), "B suppressed by the lifecycle seed");
    assert!(first.summary.unwrap().contains("receipts=2 [B@t1, A@t1]"));
    assert!(tool_call(&cj, &client).is_empty(), "then coalesced");
    fs::remove_dir_all(root).unwrap();
}

// Prior review N1, lifecycle seed path: A offered at startup, then A is ACKed
// and B arrives. Kills the same seed-after-offer mutation through ACK plus
// arrival (the post-offer read would see only B).
#[test]
fn ack_then_arrival_after_the_lifecycle_offer_is_offered_on_the_first_tool_call() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&["A"], &["B"]);
    *client.ack_after_check_in.lock().unwrap() = vec!["A"];
    assert!(register(&j, &cj, &event, &seed, &client));
    let first = boundary(&cj, &client, true);
    assert!(!first.text.is_empty(), "B suppressed by the lifecycle seed");
    assert!(first.summary.unwrap().contains("receipts=1 [B@t1]"));
    assert!(tool_call(&cj, &client).is_empty(), "then coalesced");
    fs::remove_dir_all(root).unwrap();
}

// The quiet path is one read-only digest query and nothing else; an advance
// adds exactly one CheckIn. Kills: "always CheckIn" (per-call cost would again
// include the offer) and client-side inbox/receipt paging (any other call
// panics in the mailbox).
#[test]
fn quiet_tool_call_is_one_digest_query_and_no_check_in() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&["A"], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    client.calls();
    for _ in 0..3 {
        assert!(tool_call(&cj, &client).is_empty());
        assert_eq!(client.calls(), ["digest"]);
    }
    client.arrive(&["B"]);
    assert!(!tool_call(&cj, &client).is_empty());
    assert_eq!(client.calls(), ["digest", "check_in"]);
    fs::remove_dir_all(root).unwrap();
}

// native-claude-demo-1 P3: with nothing new pending, the seat's own
// unavailability episodes (one per host snapshot in the demo) must keep the
// tool boundary quiet and make no CheckIn; a real new message still emits.
// Kills: `advanced_beyond` counting the seat's own `unavailability_episode`
// (the demo re-emitted the full ~2 KB offer after every snapshot).
#[test]
fn own_unavailability_episode_alone_keeps_the_tool_boundary_quiet() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&["A"], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    client.calls();
    for _ in 0..5 {
        *client.own_episodes.lock().unwrap() += 1;
        assert!(tool_call(&cj, &client).is_empty(), "own episode emitted");
        assert_eq!(client.calls(), ["digest"]);
    }
    client.arrive(&["B"]);
    assert!(
        !tool_call(&cj, &client).is_empty(),
        "new message suppressed"
    );
    fs::remove_dir_all(root).unwrap();
}

// Fail open only on a query error: every call emits and the mark is kept.
// Kills: "digest error treated as quiet" and "mark advanced without a digest".
#[test]
fn digest_failure_fails_open_and_keeps_the_mark() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&["A"], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    let execution = cj.current().unwrap().unwrap().execution;
    let mark = cj.attention_mark(execution);
    let failing = Mailbox {
        fail_digest: true,
        ..Mailbox::with(&["A"], &[])
    };
    for _ in 0..2 {
        let result = boundary(&cj, &failing, true);
        assert!(!result.text.is_empty(), "digest failure was quiet");
        assert!(result.summary.is_none() && result.mark.is_none());
    }
    assert_eq!(cj.attention_mark(execution), mark);
    // A failed lifecycle digest read seeds nothing.
    let (root2, j2, cj2, seed2, event2) = fixture();
    let failing = Mailbox {
        fail_digest: true,
        ..Mailbox::with(&["A"], &[])
    };
    assert!(!register(&j2, &cj2, &event2, &seed2, &failing));
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(root2).unwrap();
}

// Compaction (`coalesce: false`) re-presents pending attention, but never
// lowers the stored mark. Kills: a compaction that replaces the mark with an
// older token (a later call would re-emit), or that stays quiet.
#[test]
fn compaction_re_presents_without_lowering_the_mark() {
    let (root, j, cj, seed, event) = fixture();
    let client = Mailbox::with(&["A", "B"], &[]);
    assert!(register(&j, &cj, &event, &seed, &client));
    let execution = cj.current().unwrap().unwrap().execution;
    let before = cj.attention_mark(execution).unwrap();
    client.ack(&["B"]);
    assert!(
        !boundary(&cj, &client, false).text.is_empty(),
        "A not re-presented"
    );
    assert_eq!(cj.attention_mark(execution), Some(before));
    assert!(
        tool_call(&cj, &client).is_empty(),
        "compaction lowered the mark"
    );
    client.ack(&["A"]);
    assert!(
        boundary(&cj, &client, false).text.is_empty(),
        "nothing pending"
    );
    fs::remove_dir_all(root).unwrap();
}

// ---- recovery hot-thread read (spec §9, ht-1ip.9) ----

struct HotClient {
    reply: std::sync::Mutex<
        Option<Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError>>,
    >,
    seen: std::sync::Mutex<Vec<Command>>,
}
impl HotClient {
    fn new(
        reply: Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError>,
    ) -> Self {
        Self {
            reply: std::sync::Mutex::new(Some(reply)),
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }
}
impl crate::ports::LocalClient for HotClient {
    fn call_with_output(
        &self,
        command: Command,
        _: &crate::protocol::output::OutputSpec,
        budget: &crate::protocol::time::CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError> {
        self.call(command, budget)
    }
    fn call(
        &self,
        command: Command,
        _budget: &crate::protocol::time::CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, crate::protocol::results::ApiError> {
        self.seen.lock().unwrap().push(command);
        self.reply
            .lock()
            .unwrap()
            .take()
            .expect("one scripted reply")
    }
}
fn hot_row(thread: &str, topic: &str) -> crate::protocol::results::HotThread {
    crate::protocol::results::HotThread {
        thread: crate::protocol::ids::ThreadId::new(thread),
        topic_data: topic.into(),
        reason: crate::protocol::results::HotReason::Recent,
        effective_deadline: None,
        last_activity: crate::protocol::time::UtcMillis(1),
    }
}
fn hot_budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(u64::MAX),
        cancellation: crate::protocol::time::Cancellation::default(),
    }
}

// Kills: a read that names another seat or an out-of-range limit, a result
// kind other than HotThreads accepted as one, and an over-bound or oversized
// answer trusted as it came.
#[test]
fn read_hot_threads_asks_for_the_seat_and_validates_the_answer() {
    use crate::protocol::results::{ApiError, CommandResult, ErrorCode, HotThreads};
    let seat = crate::protocol::ids::SeatId::new("seat");
    let good = HotThreads {
        hot: vec![hot_row("t1", "a"), hot_row("t2", "b")],
        overflow: vec![crate::protocol::ids::ThreadId::new("t3")],
    };
    let client = HotClient::new(Ok(CommandResult::HotThreads(good.clone())));
    assert_eq!(
        read_hot_threads(&client, &seat, &hot_budget()).unwrap(),
        good
    );
    assert_eq!(
        *client.seen.lock().unwrap(),
        [Command::HotThreads(
            crate::protocol::commands::HotThreadsQuery {
                seat: seat.clone(),
                limit: 8
            }
        )]
    );
    let too_many = HotThreads {
        hot: (0..9).map(|n| hot_row(&format!("t{n}"), "x")).collect(),
        overflow: vec![],
    };
    for bad in [
        CommandResult::HotThreads(too_many),
        CommandResult::HotThreads(HotThreads {
            hot: vec![hot_row("t1", &"x".repeat(81))],
            overflow: vec![],
        }),
        CommandResult::Left(crate::protocol::ids::ThreadId::new("t")),
    ] {
        let client = HotClient::new(Ok(bad));
        assert_eq!(
            read_hot_threads(&client, &seat, &hot_budget())
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
    }
    let client = HotClient::new(Err(ApiError::new(
        ErrorCode::ReadBudgetExhausted,
        "bounded read exhausted",
    )));
    assert_eq!(
        read_hot_threads(&client, &seat, &hot_budget())
            .unwrap_err()
            .code,
        ErrorCode::ReadBudgetExhausted
    );
}

// Saved selectors may be shortened only with the recipient's flag-free proof
// AND an exact canonical match of both saved selectors to this hook's target.
#[test]
fn cached_pinned_continuations_use_verified_recipient_presentation_without_changing_cache() {
    use crate::{
        cli::instance::InstanceInputs,
        protocol::{
            output::{ContinuationContext, OutputSpec},
            pagination::StopReason,
            results::{ApiError, CommandResult},
            time::CallBudget,
        },
    };
    struct PinnedTransport {
        inner: Transport,
    }
    impl crate::ports::LocalClient for PinnedTransport {
        fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
            self.inner.call(command, budget)
        }
        fn call_with_output(
            &self,
            command: Command,
            spec: &OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            let mut result = self.call(command, budget)?;
            if let CommandResult::CheckedIn(check) = &mut result {
                for (argv, command) in [
                    (&mut check.warnings.next_argv, "warnings"),
                    (&mut check.inbox.next_argv, "inbox"),
                ] {
                    *argv = Some(vec![
                        "herdr-threads".into(),
                        "--state-dir".into(),
                        spec.context.state_dir.clone().unwrap(),
                        "--host-endpoint".into(),
                        spec.context.host.clone().unwrap(),
                        command.into(),
                        "--seat".into(),
                        "seat".into(),
                        "--cursor".into(),
                        "saved-cursor".into(),
                    ]);
                }
                check.warnings.has_more = true;
                check.warnings.next_cursor = Some("saved-cursor".into());
                check.warnings.stop_reason = StopReason::Work;
                check.inbox.has_more = true;
                check.inbox.next_cursor = Some("saved-cursor".into());
                check.inbox.stop_reason = StopReason::Work;
            }
            Ok(result)
        }
    }
    let (root, j, cj, seed, event) = fixture();
    let state = root.join("state");
    fs::create_dir(&state).unwrap();
    let endpoint = root.join("host.sock");
    let target = ContinuationContext {
        state_dir: Some(state.display().to_string()),
        host: Some(endpoint.display().to_string()),
    };
    let client = PinnedTransport {
        inner: Transport::new(),
    };
    run_event(
        &j,
        &cj,
        &event,
        Some(&seed),
        1,
        &client,
        &Clock,
        &OutputSpec {
            context: target.clone(),
            ..OutputSpec::default()
        },
        &mut Vec::new(),
    )
    .unwrap();
    let saved = cj
        .dispatch(&event.event_id, &mut |_: &PendingCheckIn| {
            panic!("saved request must not replay")
        })
        .unwrap()
        .output;
    let pane = InstanceInputs {
        env_state: Some(state.clone()),
        env_host: Some(endpoint.clone()),
        ..InstanceInputs::default()
    };
    let mut missing_host = target.clone();
    missing_host.host = None;
    let mut other_host = target.clone();
    other_host.host = Some(root.join("other.sock").display().to_string());
    let mut other_state = target.clone();
    other_state.state_dir = Some(root.join("other-state").display().to_string());
    let foreign_pane = InstanceInputs {
        env_host: Some(root.join("foreign.sock")),
        ..pane.clone()
    };
    for (current, recipient, concise) in [
        (&target, &pane, true),
        (&target, &foreign_pane, false),
        (&missing_host, &pane, false),
        (&other_host, &pane, false),
        (&other_state, &pane, false),
    ] {
        let mut output = Vec::new();
        run_hook_event_reporting_notices(
            &j,
            &cj,
            &event,
            Some(&seed),
            2,
            &client,
            &Clock,
            &OutputSpec::default(),
            OverviewReason::None,
            &mut output,
            &mut None,
            Some(&RecipientRouting {
                target: current,
                pane: recipient,
            }),
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.matches("saved-cursor").count(), 4);
        if concise {
            assert!(text.contains("[\"herdr-threads\",\"warnings\",\"--seat\",\"seat\",\"--cursor\",\"saved-cursor\"]"), "{text}");
            assert!(text.contains("[\"herdr-threads\",\"inbox\",\"--seat\",\"seat\",\"--cursor\",\"saved-cursor\"]"), "{text}");
        }
        assert_eq!(text.contains("--state-dir"), !concise, "{text}");
        assert_eq!(text.contains("--host-endpoint"), !concise, "{text}");
        assert_eq!(
            cj.dispatch(&event.event_id, &mut |_: &PendingCheckIn| panic!("cached"))
                .unwrap()
                .output,
            saved
        );
    }
    assert_eq!(
        client
            .inner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, Command::CheckIn(_)))
            .count(),
        1
    );
    fs::remove_dir_all(root).unwrap();
}
