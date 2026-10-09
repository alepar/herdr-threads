use herdr_threads::{
    cli::{
        journal::Journal,
        lazy_display::{DisplaySelection, write_page},
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        ids::*,
        output::OutputSpec,
        pagination::{Consistency, Page, StopReason},
        results::{CommandResult, InboxBatchV2Item},
    },
};
use std::io::{self, Write};
struct Fixture {
    root: std::path::PathBuf,
    journal: Journal,
    claim: CallerClaim,
}
impl Fixture {
    fn new() -> Self {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp/ht-big.11/lazy-display-scratch")
            .join(uuid::Uuid::new_v4().to_string());
        let journal = Journal::open(&root).unwrap();
        Self {
            root,
            journal,
            claim: CallerClaim {
                instance: "instance".into(),
                seat: SeatId::new("seat"),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("native"),
                execution: ExecutionId::new("exec"),
                target: HostTargetId::new("pane"),
            },
        }
    }
    fn write(&self, start: u64, end: u64) -> Vec<MessageId> {
        write_page(
            &page(start, end),
            &text_spec(),
            10000,
            &mut Vec::new(),
            &self.journal,
            &self.claim,
            DisplaySelection::OwnDefaultText,
        )
        .unwrap()
        .lazy
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn page(start: u64, end: u64) -> CommandResult {
    CommandResult::InboxBatchV2(Page {
        items: vec![InboxBatchV2Item::LazyMessage {
            thread: ThreadId::new("thread"),
            topic_data: "topic".into(),
            message: MessageId::new("lazy"),
            sequence: 1,
            sender: None,
            author_role: None,
            relays_user: false,
            user_intent: None,
            author_role_backfilled: false,
            body: "abcdefghij"[start as usize..end as usize].into(),
            body_start: start,
            body_end: end,
            body_len: 10,
        }],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 1,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    })
}
#[test]
fn lazy_display_full_flush_contiguous_chain() {
    let f = Fixture::new();
    assert!(f.write(0, 4).is_empty());
    assert!(f.write(0, 4).is_empty());
    assert!(f.write(4, 7).is_empty());
    assert_eq!(f.write(7, 10), vec![MessageId::new("lazy")]);
    assert_eq!(f.write(7, 10), vec![MessageId::new("lazy")]);
}
struct FailWriter {
    flush: bool,
    wrote_prefix: bool,
}
impl Write for FailWriter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        if self.flush {
            Ok(b.len())
        } else if !self.wrote_prefix {
            self.wrote_prefix = true;
            Ok(b.len().min(3))
        } else {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "cancelled"))
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("flush failed"))
    }
}
#[test]
fn lazy_display_partial_write_or_flush_failure_pending() {
    for flush in [false, true] {
        let f = Fixture::new();
        assert!(
            write_page(
                &page(0, 4),
                &text_spec(),
                10000,
                &mut FailWriter {
                    flush,
                    wrote_prefix: false
                },
                &f.journal,
                &f.claim,
                DisplaySelection::OwnDefaultText
            )
            .is_err()
        );
        assert!(f.write(4, 10).is_empty());
        assert_eq!(f.write(0, 10), vec![MessageId::new("lazy")]);
    }
}
#[test]
fn lazy_display_skipped_chunk_restart_binding_change() {
    let mut f = Fixture::new();
    assert!(f.write(4, 7).is_empty());
    assert!(f.write(0, 4).is_empty());
    f.journal = Journal::open(&f.root).unwrap();
    assert!(f.write(7, 10).is_empty());
    let original = f.claim.clone();
    for replacement in 0..5 {
        f.claim = original.clone();
        match replacement {
            0 => f.claim.native_session = NativeSessionId::new("changed"),
            1 => f.claim.binding_generation += 1,
            2 => f.claim.execution = ExecutionId::new("changed"),
            3 => f.claim.harness = Harness::Human,
            _ => f.claim.target = HostTargetId::new("changed"),
        }
        assert!(f.write(4, 10).is_empty());
    }
    f.claim = original;
    assert_eq!(f.write(4, 10), vec![MessageId::new("lazy")]);
}
#[test]
fn lazy_display_human_full_body_durable() {
    let mut f = Fixture::new();
    f.claim.harness = Harness::Human;
    assert_eq!(f.write(0, 10), vec![MessageId::new("lazy")]);
    assert!(std::fs::read_dir(&f.root).unwrap().any(|x| {
        x.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("display-lazy-")
    }));
}

struct Client {
    calls: std::sync::Mutex<Vec<herdr_threads::protocol::commands::Command>>,
    v2: bool,
    result: CommandResult,
}
impl herdr_threads::ports::LocalClient for Client {
    fn supports_capability(
        &self,
        name: &str,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> bool {
        self.v2 && name == herdr_threads::protocol::capabilities::INBOX_BATCH_V2
    }
    fn call(
        &self,
        c: herdr_threads::protocol::commands::Command,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        use herdr_threads::protocol::{commands::Command, results::CapabilityList};
        self.calls.lock().unwrap().push(c.clone());
        match c {
            Command::Capabilities => Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: if self.v2 {
                    vec![herdr_threads::protocol::capabilities::INBOX_BATCH_V2.into()]
                } else {
                    vec![]
                },
            })),
            Command::InboxBatchV2(query) if self.v2 => {
                let mut result = self.result.clone();
                if let CommandResult::InboxBatchV2(page) = &mut result
                    && page.items.is_empty()
                    && page.stop_reason == StopReason::Work
                    && page.has_more
                {
                    // Advance this canned walk like the real bounded producer.
                    let after = query.page.cursor.as_ref().map_or(0, |cursor| {
                        herdr_threads::protocol::pagination::InboxBatchV2CursorState::decode(cursor)
                            .unwrap()
                            .lazy_after_ordinal
                    });
                    let next = test_cursor_after(after + 1);
                    page.next_cursor = Some(next.clone());
                    page.next_argv = Some(vec![
                        "herdr-threads".into(),
                        "inbox".into(),
                        "--cursor".into(),
                        next,
                    ]);
                }
                Ok(result)
            }
            Command::Inbox(_) if !self.v2 => Ok(CommandResult::Inbox(Page {
                items: vec![],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: StopReason::Complete,
                consistency: Consistency::BoundedLive,
            })),
            Command::CompleteInboxDelivery(completion) if self.v2 => {
                Ok(CommandResult::InboxDeliveryCompleted(completion.messages))
            }
            _ => panic!("unexpected mutation/read: {c:?}"),
        }
    }
    fn call_with_output(
        &self,
        c: herdr_threads::protocol::commands::Command,
        _: &OutputSpec,
        b: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        self.call(c, b)
    }
}
fn run(
    f: &Fixture,
    argv: &[&str],
    client: &Client,
    role: herdr_threads::harness::context::Role,
) -> Vec<u8> {
    let mut out = Vec::new();
    run_to_writer(f, argv, client, role, &mut out).unwrap();
    out
}
fn run_to_writer<C: herdr_threads::ports::LocalClient, W: Write>(
    f: &Fixture,
    argv: &[&str],
    client: &C,
    role: herdr_threads::harness::context::Role,
    writer: &mut W,
) -> Result<(), herdr_threads::cli::RunError> {
    use herdr_threads::harness::context::{
        ContextJournal, Harness, OccupantContext, Role, SessionReference,
    };
    let parsed = herdr_threads::cli::commands::parse_argv(argv.iter().copied()).unwrap();
    let context = OccupantContext {
        format_version: 1,
        instance: uuid::Uuid::from_u128(1),
        seat: "seat".into(),
        target: "pane".into(),
        harness: if parsed.actor == herdr_threads::cli::actor_route::InvocationActor::Human {
            Harness::Human
        } else {
            Harness::Codex
        },
        binding_generation: 1,
        execution: uuid::Uuid::from_u128(2),
        session: SessionReference::Native("native".into()),
        role: Role::TopLevel,
    };
    let dir = f.root.join("contexts");
    if !dir.exists() {
        std::fs::create_dir(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let contexts = ContextJournal::open(
        &dir,
        context.instance,
        &context.seat,
        std::time::Duration::from_secs(1),
    )
    .unwrap();
    contexts.install_reattached(context).unwrap();
    herdr_threads::cli::run_cooperative(
        parsed,
        &f.journal,
        &contexts,
        None,
        role,
        client,
        &herdr_threads::app::SystemClock::new(),
        writer,
    )
}
fn has_lazy_progress(f: &Fixture) -> bool {
    std::fs::read_dir(&f.root).unwrap().any(|x| {
        x.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("display-lazy-")
    })
}
#[test]
fn lazy_display_json_machine_explicit_selector_readonly() {
    use herdr_threads::{harness::context::Role, protocol::commands::Command};
    for argv in [
        vec!["ht", "inbox", "--json"],
        vec!["ht", "inbox", "--machine"],
        vec!["ht", "inbox", "--seat", "seat"],
    ] {
        let f = Fixture::new();
        let c = Client {
            calls: std::sync::Mutex::new(vec![]),
            v2: true,
            result: page(0, 10),
        };
        let out = run(&f, &argv, &c, Role::TopLevel);
        assert!(!out.is_empty());
        assert!(
            c.calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, Command::InboxBatchV2(_)))
        );
        assert!(!has_lazy_progress(&f));
        assert!(
            !c.calls.lock().unwrap().iter().any(|command| matches!(
                command,
                Command::CompleteInboxDelivery(_) | Command::Ack(_) | Command::AckDisplayed(_)
            )),
            "readonly discovery cannot settle and then clear its proof"
        );
    }
    let f = Fixture::new();
    let c = Client {
        calls: std::sync::Mutex::new(vec![]),
        v2: true,
        result: page(0, 10),
    };
    run(&f, &["ht", "inbox"], &c, Role::Subagent);
    assert!(!has_lazy_progress(&f));
    assert!(
        !c.calls.lock().unwrap().iter().any(|command| matches!(
            command,
            Command::CompleteInboxDelivery(_) | Command::Ack(_) | Command::AckDisplayed(_)
        )),
        "subagent discovery cannot settle and then clear its proof"
    );
}
#[test]
fn lazy_display_default_text_routes_v2_settles_complete_body() {
    let f = Fixture::new();
    let c = Client {
        calls: std::sync::Mutex::new(vec![]),
        v2: true,
        result: page(0, 10),
    };
    let out = run(
        &f,
        &["ht", "inbox"],
        &c,
        herdr_threads::harness::context::Role::TopLevel,
    );
    assert!(String::from_utf8(out).unwrap().contains("[lazy]"));
    assert!(
        !has_lazy_progress(&f),
        "successful settlement clears its proof"
    );
    assert!(
        f.journal
            .page(&Default::default())
            .unwrap()
            .items
            .is_empty()
    );
    let calls = c.calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        3,
        "capabilities, v2 read and independent completion"
    );
    use herdr_threads::protocol::commands::Command;
    assert!(matches!(calls[0], Command::Capabilities));
    assert!(matches!(calls[1], Command::InboxBatchV2(_)));
    let Command::CompleteInboxDelivery(completion) = &calls[2] else {
        panic!(
            "lazy completion cannot become an ordinary ACK: {:?}",
            calls[2]
        );
    };
    assert_eq!(completion.messages, vec![MessageId::new("lazy")]);
    assert_eq!(
        completion.claim,
        CallerClaim {
            instance: uuid::Uuid::from_u128(1).to_string(),
            seat: SeatId::new("seat"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("native"),
            execution: ExecutionId::new(uuid::Uuid::from_u128(2).to_string()),
            target: HostTargetId::new("pane"),
        }
    );
}
#[test]
fn lazy_display_empty_continuation_not_empty_inbox() {
    let f = Fixture::new();
    let CommandResult::InboxBatchV2(mut p) = page(0, 10) else {
        unreachable!()
    };
    p.items.clear();
    p.has_more = true;
    p.stop_reason = StopReason::Work;
    p.next_cursor = Some(test_cursor());
    p.next_argv = Some(vec![
        "ht".into(),
        "inbox".into(),
        "--cursor".into(),
        test_cursor(),
    ]);
    let c = Client {
        calls: std::sync::Mutex::new(vec![]),
        v2: true,
        result: CommandResult::InboxBatchV2(p),
    };
    let out = String::from_utf8(run(
        &f,
        &["ht", "inbox"],
        &c,
        herdr_threads::harness::context::Role::TopLevel,
    ))
    .unwrap();
    assert!(out.contains("next:"));
    let next = shlex::split(
        out.lines()
            .find_map(|line| line.strip_prefix("next: "))
            .unwrap(),
    )
    .unwrap();
    let cursor = next.iter().position(|arg| arg == "--cursor").unwrap();
    assert_eq!(next[cursor + 1], test_cursor_after(8));
    assert_eq!(
        c.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(
                call,
                herdr_threads::protocol::commands::Command::InboxBatchV2(_)
            ))
            .count(),
        8
    );
    assert!(!out.contains("empty"));
    assert!(!has_lazy_progress(&f));
}
#[test]
fn lazy_display_legacy_readonly_fallback() {
    let f = Fixture::new();
    let c = Client {
        calls: std::sync::Mutex::new(vec![]),
        v2: false,
        result: page(0, 10),
    };
    let out = run(
        &f,
        &["ht", "inbox", "--machine"],
        &c,
        herdr_threads::harness::context::Role::TopLevel,
    );
    assert!(!out.is_empty());
    assert!(matches!(
        c.calls.lock().unwrap().as_slice(),
        [herdr_threads::protocol::commands::Command::Inbox(_)]
    ));
    assert!(!has_lazy_progress(&f));
}
#[test]
fn lazy_display_changed_length_budget_and_readonly_selection() {
    let f = Fixture::new();
    assert!(f.write(0, 4).is_empty());
    assert!(
        f.journal
            .record_lazy_displayed_chunk(&f.claim, &MessageId::new("lazy"), 4, 10, 11)
            .is_err()
    );
    assert!(
        write_page(
            &page(4, 10),
            &text_spec(),
            1,
            &mut Vec::new(),
            &f.journal,
            &f.claim,
            DisplaySelection::OwnDefaultText
        )
        .is_err()
    );
    assert!(
        write_page(
            &page(4, 10),
            &text_spec(),
            10000,
            &mut Vec::new(),
            &f.journal,
            &f.claim,
            DisplaySelection::ReadOnly
        )
        .unwrap()
        .lazy
        .is_empty()
    );
    assert_eq!(f.write(4, 10), vec![MessageId::new("lazy")]);
}

fn text_spec() -> OutputSpec {
    OutputSpec {
        format: herdr_threads::protocol::output::OutputFormat::Text,
        ..OutputSpec::default()
    }
}

#[test]
fn lazy_display_mixed_candidates_and_human_policy() {
    let mut f = Fixture::new();
    let CommandResult::InboxBatchV2(mut p) = page(0, 10) else {
        unreachable!()
    };
    p.items.push(InboxBatchV2Item::Message {
        thread: ThreadId::new("thread"),
        topic_data: "topic".into(),
        message: MessageId::new("ordinary"),
        sequence: 2,
        sender: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: "body".into(),
        body_start: 0,
        body_end: 4,
        body_len: 4,
        ack_candidate: Some(MessageId::new("ordinary")),
    });
    let result = CommandResult::InboxBatchV2(p);
    let c = write_page(
        &result,
        &text_spec(),
        10000,
        &mut Vec::new(),
        &f.journal,
        &f.claim,
        DisplaySelection::OwnDefaultText,
    )
    .unwrap();
    assert_eq!(c.lazy, vec![MessageId::new("lazy")]);
    assert_eq!(c.ordinary, vec![MessageId::new("ordinary")]);
    assert_eq!(c.claim, f.claim);
    f.claim.harness = Harness::Human;
    let c = write_page(
        &result,
        &text_spec(),
        10000,
        &mut Vec::new(),
        &f.journal,
        &f.claim,
        DisplaySelection::OwnDefaultText,
    )
    .unwrap();
    assert_eq!(c.lazy, vec![MessageId::new("lazy")]);
    assert!(c.ordinary.is_empty());
    f.journal
        .clear_lazy_displayed_chunk(&f.claim, &MessageId::new("lazy"))
        .unwrap();
    assert!(f.write(4, 10).is_empty());
    f.claim.harness = Harness::Codex;
    assert_eq!(f.write(4, 10), vec![MessageId::new("lazy")]);
}
#[test]
fn lazy_display_empty_body_and_invalid_bounds() {
    let f = Fixture::new();
    assert!(
        f.journal
            .record_lazy_displayed_chunk(&f.claim, &MessageId::new("empty"), 0, 0, 0)
            .unwrap()
    );
    let CommandResult::InboxBatchV2(mut p) = page(0, 10) else {
        unreachable!()
    };
    if let InboxBatchV2Item::LazyMessage { body, .. } = &mut p.items[0] {
        *body = "short".into();
    }
    assert!(
        write_page(
            &CommandResult::InboxBatchV2(p),
            &text_spec(),
            10000,
            &mut Vec::new(),
            &f.journal,
            &f.claim,
            DisplaySelection::OwnDefaultText
        )
        .is_err()
    );
    assert!(f.write(4, 10).is_empty());
}

#[test]
fn lazy_display_progress_does_not_veto_archival_coverage() {
    use herdr_threads::{
        archival_legacy::Source,
        daemon::paths::{InstancePaths, RuntimeContext},
        protocol::output::ContinuationContext,
    };
    let f = Fixture::new();
    let runtime = RuntimeContext::explicit(f.root.clone(), f.root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve_read_only(&runtime).unwrap();
    std::fs::create_dir_all(&paths.instance_dir).unwrap();
    let journal = Journal::open(paths.instance_dir.join("intents")).unwrap();
    assert!(
        journal
            .record_lazy_displayed_chunk(&f.claim, &MessageId::new("lazy"), 0, 10, 10)
            .unwrap()
    );
    let mut source = Source::new(&paths, "instance".into(), ContinuationContext::default());
    let mut covered = false;
    for _ in 0..100 {
        let scan = source.scan(|| false).unwrap();
        if !scan.pending {
            covered = scan.coverage.is_some();
            break;
        }
    }
    assert!(
        covered,
        "lazy display bookkeeping must not veto quiet archival coverage"
    );
}

// Dropping selectors on an empty work page must fail before a later chunk can settle.
#[test]
fn continuation_empty_work_page_preserves_invocation() {
    use herdr_threads::harness::context::Role;
    for flags in [
        vec!["--machine"],
        vec!["--json"],
        vec!["--seat", "foreign"],
        vec!["human"],
    ] {
        let f = Fixture::new();
        let CommandResult::InboxBatchV2(mut p) = page(0, 4) else {
            unreachable!()
        };
        p.items.clear();
        p.has_more = true;
        p.stop_reason = StopReason::Work;
        let cursor = test_cursor();
        p.next_cursor = Some(cursor.clone());
        p.next_argv = Some(vec![
            "herdr-threads".into(),
            "inbox".into(),
            "--seat".into(),
            "seat".into(),
            "--cursor".into(),
            cursor,
        ]);
        let c = Client {
            calls: Default::default(),
            v2: true,
            result: CommandResult::InboxBatchV2(p),
        };
        let mut argv = vec!["ht"];
        if flags == ["human"] {
            argv.push("human");
        }
        argv.extend([
            "--state-dir",
            "/private/tmp/state ' $",
            "--host-endpoint",
            "/private/tmp/host \" socket",
            "inbox",
        ]);
        if flags != ["human"] {
            argv.extend(&flags);
        }
        let out = String::from_utf8(run(&f, &argv, &c, Role::TopLevel)).unwrap();
        assert!(
            !out.contains("empty"),
            "work-limited page falsely exhausted: {out}"
        );
        let next: Vec<String> = if flags == ["--json"] {
            serde_json::from_str::<serde_json::Value>(&out).unwrap()["result"]["data"]["next_argv"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect()
        } else {
            shlex::split(out.lines().find_map(|l| l.strip_prefix("next: ")).unwrap()).unwrap()
        };
        assert_eq!(
            next.get(1).map(String::as_str) == Some("human"),
            flags == ["human"]
        );
        for flag in ["--state-dir", "--host-endpoint"] {
            assert!(next.iter().any(|s| s == flag), "{next:?}");
        }
        for flag in flags.iter().filter(|s| **s != "human") {
            assert!(next.iter().any(|s| s == flag), "{next:?}");
        }
        assert!(!has_lazy_progress(&f));
        let c = Client {
            calls: Default::default(),
            v2: true,
            result: page(0, 4),
        };
        let refs: Vec<_> = next.iter().map(String::as_str).collect();
        let follow = run(&f, &refs, &c, Role::TopLevel);
        assert!(!follow.is_empty());
        if flags != ["human"] {
            assert!(!has_lazy_progress(&f));
        }
        assert!(!c.calls.lock().unwrap().iter().any(|c| matches!(
            c,
            herdr_threads::protocol::commands::Command::CompleteInboxDelivery(_)
        )));
    }
}

fn test_cursor() -> String {
    test_cursor_after(0)
}

fn test_cursor_after(after: u64) -> String {
    use herdr_threads::protocol::pagination::{
        InboxBatchV2CursorState, InboxBatchV2Source, ReceiptAttentionCursorState,
        SeatAttentionCursorState,
    };
    InboxBatchV2CursorState {
        seat: SeatId::new("seat"),
        binding_generation: Some(1),
        execution: Some(ExecutionId::new(uuid::Uuid::from_u128(2).to_string())),
        source: InboxBatchV2Source::Lazy,
        lazy_after_ordinal: after,
        lazy_high_water_ordinal: 100,
        publication_decision_high_water: 4,
        body: None,
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
    .encode(&uuid::Uuid::from_u128(1).to_string())
    .unwrap()
}

#[test]
fn continuation_mixed_sources_preserve_invocation() {
    use herdr_threads::harness::context::Role;
    let f = Fixture::new();
    let CommandResult::InboxBatchV2(mut p) = page(0, 4) else {
        unreachable!()
    };
    p.items.extend([
        InboxBatchV2Item::Invitation {
            thread: ThreadId::new("invite-thread"),
            topic_data: "invite".into(),
            invitation: InvitationId::new("invitation"),
            required_service: None,
        },
        InboxBatchV2Item::Warning {
            thread: ThreadId::new("warning-thread"),
            topic_data: "warn".into(),
            warning: MessageId::new("warning"),
            sequence: 2,
            informational: false,
        },
        InboxBatchV2Item::Message {
            thread: ThreadId::new("thread"),
            topic_data: "topic".into(),
            message: MessageId::new("ordinary"),
            sequence: 3,
            sender: None,
            author_role: None,
            relays_user: false,
            user_intent: None,
            author_role_backfilled: false,
            body: "ordinary".into(),
            body_start: 0,
            body_end: 8,
            body_len: 8,
            ack_candidate: Some(MessageId::new("ordinary")),
        },
    ]);
    p.has_more = true;
    p.stop_reason = StopReason::Bytes;
    let cursor = test_cursor();
    p.next_cursor = Some(cursor.clone());
    p.next_argv = Some(vec![
        "herdr-threads".into(),
        "inbox".into(),
        "--seat".into(),
        "seat".into(),
        "--cursor".into(),
        cursor,
    ]);
    let c = Client {
        calls: Default::default(),
        v2: true,
        result: CommandResult::InboxBatchV2(p),
    };
    let out = String::from_utf8(run(
        &f,
        &["ht", "inbox", "--machine", "--seat", "foreign"],
        &c,
        Role::TopLevel,
    ))
    .unwrap();
    assert!(
        out.contains("invite") && out.contains("warn") && out.contains("ordinary"),
        "{out}"
    );
    let next = shlex::split(out.lines().find_map(|l| l.strip_prefix("next: ")).unwrap()).unwrap();
    assert!(next.iter().any(|s| s == "--machine"));
    let at = next.iter().position(|s| s == "--seat").unwrap();
    assert_eq!(next[at + 1], "foreign");
    let c = Client {
        calls: Default::default(),
        v2: true,
        result: page(4, 10),
    };
    let refs: Vec<_> = next.iter().map(String::as_str).collect();
    let out = String::from_utf8(run(&f, &refs, &c, Role::TopLevel)).unwrap();
    assert!(out.contains("efghij"));
    assert!(!has_lazy_progress(&f));
    assert!(!c.calls.lock().unwrap().iter().any(|c| matches!(
        c,
        herdr_threads::protocol::commands::Command::CompleteInboxDelivery(_)
            | herdr_threads::protocol::commands::Command::AckDisplayed(_)
    )));
}

struct RefitClient {
    calls: std::sync::Mutex<Vec<herdr_threads::protocol::commands::Command>>,
}
impl herdr_threads::ports::LocalClient for RefitClient {
    fn supports_capability(
        &self,
        name: &str,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> bool {
        name == herdr_threads::protocol::capabilities::INBOX_BATCH_V2
    }
    fn call(
        &self,
        command: herdr_threads::protocol::commands::Command,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        use herdr_threads::protocol::{commands::Command, results::CapabilityList};
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::Capabilities => Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: vec![herdr_threads::protocol::capabilities::INBOX_BATCH_V2.into()],
            })),
            _ => panic!("unexpected accountable call {command:?}"),
        }
    }
    fn call_with_output(
        &self,
        command: herdr_threads::protocol::commands::Command,
        spec: &OutputSpec,
        _: &herdr_threads::protocol::time::CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        use herdr_threads::protocol::{
            commands::Command,
            output::{OutputFormat, encode_selected},
            results::ApiError,
        };
        self.calls.lock().unwrap().push(command.clone());
        let Command::InboxBatchV2(q) = command else {
            panic!("unexpected call")
        };
        let CommandResult::InboxBatchV2(mut p) = page(0, 4) else {
            unreachable!()
        };
        let cursor = test_cursor();
        let mut argv = vec!["herdr-threads".into()];
        if let Some(state) = &spec.context.state_dir {
            argv.extend(["--state-dir".into(), state.clone()]);
        }
        if let Some(host) = &spec.context.host {
            argv.extend(["--host-endpoint".into(), host.clone()]);
        }
        if spec.format == OutputFormat::Json {
            argv.push("--json".into());
        }
        argv.extend([
            "inbox".into(),
            "--seat".into(),
            "seat".into(),
            "--cursor".into(),
            cursor.clone(),
            "--max-bytes".into(),
            q.page.max_bytes.to_string(),
        ]);
        p.next_cursor = Some(cursor);
        p.next_argv = Some(argv);
        p.has_more = true;
        p.stop_reason = StopReason::Bytes;
        for n in (0..=1000).rev() {
            if let InboxBatchV2Item::LazyMessage {
                body,
                body_end,
                body_len,
                ..
            } = &mut p.items[0]
            {
                *body = "界".repeat(n);
                *body_end = (n * 3) as u64;
                *body_len = 3003;
            }
            let result = CommandResult::InboxBatchV2(p.clone());
            if encode_selected(&result, spec)?.len() <= q.page.max_bytes as usize {
                return Ok(result);
            }
        }
        Err(ApiError::invalid_budget("server envelope cannot fit"))
    }
}

#[derive(Default)]
struct CountingWriter {
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
}
impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.extend(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn continuation_final_argv_budget_blocks_proof_and_mutation() {
    use herdr_threads::{harness::context::Role, protocol::commands::Command};
    // The server page itself fits; only the final invocation decoration is impossible.
    let f = Fixture::new();
    let CommandResult::InboxBatchV2(mut p) = page(0, 10) else {
        unreachable!()
    };
    p.has_more = true;
    p.next_cursor = Some(test_cursor());
    p.next_argv = Some(vec![
        "herdr-threads".into(),
        "inbox".into(),
        "--cursor".into(),
        test_cursor(),
    ]);
    let c = Client {
        calls: Default::default(),
        v2: true,
        result: CommandResult::InboxBatchV2(p),
    };
    let state = format!("/private/tmp/{} ' $", "s".repeat(900));
    let mut writer = CountingWriter::default();
    let result = run_to_writer(
        &f,
        &["ht", "--state-dir", &state, "inbox", "--max-bytes", "1024"],
        &c,
        Role::TopLevel,
        &mut writer,
    );
    assert!(
        matches!(result, Err(herdr_threads::cli::RunError::Api(e)) if e.code == herdr_threads::protocol::results::ErrorCode::InvalidBudget)
    );
    assert_eq!(writer.writes, 0);
    assert_eq!(writer.flushes, 0);
    assert!(!has_lazy_progress(&f));
    assert!(!c.calls.lock().unwrap().iter().any(|c| matches!(
        c,
        Command::CompleteInboxDelivery(_) | Command::AckDisplayed(_)
    )));
    for flags in [
        vec!["--machine"],
        vec!["--json"],
        vec!["--seat", "foreign"],
        vec!["human"],
    ] {
        for impossible in [false, true] {
            let f = Fixture::new();
            let client = RefitClient {
                calls: Default::default(),
            };
            let state = format!(
                "/private/tmp/state '{}' $ {}",
                "s".repeat(if impossible { 900 } else { 20 }),
                "界".repeat(4)
            );
            let host = "/private/tmp/host ' $ socket";
            let mut argv = vec!["ht"];
            if flags == ["human"] {
                argv.push("human");
            }
            argv.extend([
                "--state-dir",
                &state,
                "--host-endpoint",
                host,
                "inbox",
                "--max-bytes",
                "1024",
                "--limit",
                "7",
            ]);
            if flags != ["human"] {
                argv.extend(&flags);
            }
            let mut writer = CountingWriter::default();
            let result = run_to_writer(&f, &argv, &client, Role::TopLevel, &mut writer);
            if impossible {
                assert!(
                    matches!(result, Err(herdr_threads::cli::RunError::Api(e)) if e.code == herdr_threads::protocol::results::ErrorCode::InvalidBudget)
                );
                assert!(writer.bytes.is_empty());
                assert_eq!(writer.writes, 0);
                assert_eq!(writer.flushes, 0);
                assert!(!has_lazy_progress(&f));
            } else {
                result.unwrap();
                assert!(writer.bytes.len() <= 1024);
                assert_eq!(writer.flushes, 1);
                let out = String::from_utf8(writer.bytes).unwrap();
                let next = if flags == ["--json"] {
                    serde_json::from_str::<serde_json::Value>(&out).unwrap()["result"]["data"]["next_argv"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_owned()).collect::<Vec<_>>()
                } else {
                    shlex::split(out.lines().find_map(|l| l.strip_prefix("next: ")).unwrap())
                        .unwrap()
                };
                for (flag, value) in [
                    ("--limit", "7"),
                    ("--max-bytes", "1024"),
                    ("--state-dir", &state),
                    ("--host-endpoint", host),
                ] {
                    let at = next.iter().position(|s| s == flag).unwrap();
                    assert_eq!(next[at + 1], value);
                }
                assert!(
                    client
                        .calls
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|c| matches!(c, Command::InboxBatchV2(_)))
                        .count()
                        > 1,
                    "effective body budget was never reduced"
                );
                if flags == ["human"] {
                    assert_eq!(next[1], "human");
                } else {
                    assert!(next.iter().any(|s| s == flags[0]));
                }
                if flags != ["human"] {
                    assert!(!has_lazy_progress(&f));
                }
            }
            let calls = client.calls.lock().unwrap();
            assert!(
                calls.iter().any(|c| matches!(c, Command::InboxBatchV2(_))),
                "read control never ran"
            );
            assert!(!calls.iter().any(|c| matches!(
                c,
                Command::CompleteInboxDelivery(_) | Command::AckDisplayed(_) | Command::Ack(_)
            )));
        }
    }
}

#[test]
fn continuation_explicit_pane_stays_readonly_at_direct_entry() {
    use herdr_threads::{harness::context::Role, protocol::commands::Command};
    let f = Fixture::new();
    let c = Client {
        calls: Default::default(),
        v2: true,
        result: page(0, 10),
    };
    let out = run(&f, &["ht", "inbox", "--pane", "pane"], &c, Role::TopLevel);
    assert!(!out.is_empty());
    assert!(!has_lazy_progress(&f));
    let calls = c.calls.lock().unwrap();
    assert!(!calls.iter().any(|c| matches!(
        c,
        Command::CompleteInboxDelivery(_) | Command::AckDisplayed(_)
    )));
    assert!(calls.iter().any(|c| matches!(c, Command::InboxBatchV2(_))));
}

#[test]
fn continuation_server_routing_fallback_stays_passive() {
    use herdr_threads::harness::context::Role;
    let f = Fixture::new();
    let CommandResult::InboxBatchV2(mut p) = page(0, 4) else {
        unreachable!()
    };
    p.has_more = true;
    p.next_cursor = Some(test_cursor());
    p.next_argv = Some(vec![
        "herdr-threads".into(),
        "--state-dir".into(),
        "/server state".into(),
        "--host-endpoint".into(),
        "/server host".into(),
        "inbox".into(),
        "--seat".into(),
        "seat".into(),
        "--cursor".into(),
        test_cursor(),
    ]);
    let c = Client {
        calls: Default::default(),
        v2: true,
        result: CommandResult::InboxBatchV2(p),
    };
    let out =
        String::from_utf8(run(&f, &["ht", "inbox", "--machine"], &c, Role::TopLevel)).unwrap();
    let next = shlex::split(out.lines().find_map(|l| l.strip_prefix("next: ")).unwrap()).unwrap();
    for (flag, value) in [
        ("--state-dir", "/server state"),
        ("--host-endpoint", "/server host"),
        ("--seat", "seat"),
    ] {
        let at = next
            .iter()
            .position(|s| s == flag)
            .expect("daemon continuation routing retained");
        assert_eq!(next[at + 1], value);
        assert_eq!(next.iter().filter(|s| *s == flag).count(), 1);
    }
    assert!(!has_lazy_progress(&f));
}
