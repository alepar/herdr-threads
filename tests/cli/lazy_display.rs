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
            .join(".tmp/ht-big.5.1")
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
            Command::InboxBatchV2(_) if self.v2 => Ok(self.result.clone()),
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
    use herdr_threads::harness::context::{
        ContextJournal, Harness, OccupantContext, Role, SessionReference,
    };
    let context = OccupantContext {
        format_version: 1,
        instance: uuid::Uuid::from_u128(1),
        seat: "seat".into(),
        target: "pane".into(),
        harness: Harness::Codex,
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
    let mut out = vec![];
    herdr_threads::cli::run_cooperative(
        herdr_threads::cli::commands::parse_argv(argv.iter().copied()).unwrap(),
        &f.journal,
        &contexts,
        None,
        role,
        client,
        &herdr_threads::app::SystemClock::new(),
        &mut out,
    )
    .unwrap();
    out
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
    }
    let f = Fixture::new();
    let c = Client {
        calls: std::sync::Mutex::new(vec![]),
        v2: true,
        result: page(0, 10),
    };
    run(&f, &["ht", "inbox"], &c, Role::Subagent);
    assert!(!has_lazy_progress(&f));
}
#[test]
fn lazy_display_default_text_routes_v2_leaves_pending() {
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
    assert!(has_lazy_progress(&f));
    assert_eq!(
        c.calls.lock().unwrap().len(),
        2,
        "capabilities and read only; no lazy ACK or completion"
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
    p.next_cursor = Some("bounded".into());
    p.next_argv = Some(vec![
        "ht".into(),
        "inbox".into(),
        "--cursor".into(),
        "bounded".into(),
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
