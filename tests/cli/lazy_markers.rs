use herdr_threads::{
    app::SystemClock,
    cli::{commands::parse_argv, journal::Journal, output::PresentationGuard, run_cooperative},
    harness::context::{ContextJournal, Role},
    ports::LocalClient,
    protocol::{
        capabilities::MESSAGE_DELIVERY_MODES,
        commands::{Command, DeliveryMode},
        ids::{MessageId, ThreadId},
        output::{OutputSpec, encode_selected},
        pagination::{Consistency, Page, StopReason},
        priority::DeliveryPriority,
        results::{
            ApiError, CommandResult, MessageContent, MessageDeliveryMode, MessageDetails,
            MessageKind, MessageSummary, SearchHit, SearchPage,
        },
        time::{CallBudget, UtcMillis},
    },
    test_support::isolation::TestIsolation,
};
use std::{sync::Mutex, time::Duration};

fn summary(n: u64) -> MessageSummary {
    MessageSummary {
        message: MessageId::new(format!("msg-{n}")),
        thread: ThreadId::new("t"),
        author: None,
        event_author: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        kind: MessageKind::Ordinary,
        sequence: n,
        created_at: UtcMillis(0),
        actor_label: Some("actor".into()),
        preview_data: if n == 1 {
            "[lazy] misleading peer text".into()
        } else {
            "canonical passive body".into()
        },
        preview_omitted: false,
        preview_detail_argv: None,
    }
}
fn page<T>(items: Vec<T>) -> Page<T> {
    Page {
        items,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 999,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}
struct Client {
    advertised: bool,
    count: u64,
    calls: Mutex<Vec<Command>>,
}
impl Client {
    fn new(advertised: bool, count: u64) -> Self {
        Self {
            advertised,
            count,
            calls: Mutex::new(vec![]),
        }
    }
    fn result(&self, command: &Command) -> CommandResult {
        match command {
            Command::History(_) => {
                CommandResult::History(page((1..=self.count).map(summary).collect()))
            }
            Command::Message(q) => {
                let n = q
                    .message
                    .as_str()
                    .strip_prefix("msg-")
                    .unwrap()
                    .parse()
                    .unwrap();
                let s = summary(n);
                CommandResult::Message(MessageDetails {
                    summary: s.clone(),
                    content: MessageContent::Ordinary {
                        body_total_bytes: s.preview_data.len() as u64,
                        body_data: s.preview_data,
                        body_offset: 0,
                        body_complete: true,
                        body_next_cursor: None,
                        body_next_argv: None,
                    },
                })
            }
            Command::Search(_) => CommandResult::Search(SearchPage {
                matches: page(
                    (1..=self.count)
                        .map(|n| SearchHit::Body(summary(n)))
                        .collect(),
                ),
                examined_candidates: self.count as u16,
                examined_utf8_bytes: 100,
            }),
            _ => panic!("unexpected read {command:?}"),
        }
    }
}
impl LocalClient for Client {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        self.advertised && name == MESSAGE_DELIVERY_MODES
    }
    fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        if let Command::MessageDeliveryModes(q) = command {
            assert!(self.advertised, "old daemon received new command");
            assert!(
                q.with_priority,
                "every current client asks for the priority"
            );
            assert!(!q.messages.is_empty() && q.messages.len() <= 100);
            return Ok(CommandResult::MessageDeliveryModes(
                q.messages
                    .into_iter()
                    .map(|message| {
                        let lazy = !matches!(message.as_str(), "msg-1" | "msg-4");
                        // msg-1 is p1 (silent), msg-2 p0, msg-3 p5, msg-4 p0 recorded
                        // ordinary (pre-upgrade or explicit ACK: no marker), others p1.
                        // `delivery_mode` stays the legacy column (msg-2 is recorded lazy).
                        let priority = Some({
                            let n = match message.as_str() {
                                "msg-2" | "msg-4" => 0,
                                "msg-3" => 5,
                                _ => 1,
                            };
                            DeliveryPriority::new(n).unwrap()
                        });
                        MessageDeliveryMode {
                            priority,
                            message,
                            delivery_mode: if lazy {
                                DeliveryMode::Lazy
                            } else {
                                DeliveryMode::Ordinary
                            },
                        }
                    })
                    .collect(),
            ));
        }
        Ok(self.result(&command))
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
}
fn execute<C: LocalClient>(
    argv: &[&str],
    client: &C,
) -> Result<(String, OutputSpec), herdr_threads::cli::RunError> {
    let iso = TestIsolation::new("lazy-markers");
    let journal = Journal::open(iso.path("intents")).unwrap();
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(iso.path("contexts"))
        .unwrap();
    let contexts = ContextJournal::open(
        &iso.path("contexts").canonicalize().unwrap(),
        herdr_threads::node::NodeUuid::from_u128(1),
        "seat-1",
        Duration::from_secs(1),
    )
    .map(|journal| journal.with_host(herdr_threads::test_support::test_host_locator()))
    .unwrap();
    let parsed = parse_argv(std::iter::once("herdr-threads").chain(argv.iter().copied())).unwrap();
    let spec = parsed.output.clone();
    let _guard = PresentationGuard::enter(parsed.presentation, &spec);
    let before = std::fs::read_dir(iso.path("intents")).unwrap().count();
    let mut out = vec![];
    run_cooperative(
        parsed,
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        client,
        &SystemClock::new(),
        &mut out,
    )?;
    assert_eq!(
        std::fs::read_dir(iso.path("intents")).unwrap().count(),
        before
    );
    Ok((String::from_utf8(out).unwrap(), spec))
}
fn run<C: LocalClient>(argv: &[&str], client: &C) -> (String, OutputSpec) {
    execute(argv, client).unwrap()
}
#[test]
fn lazy_markers_history_body_search_use_canonical_metadata() {
    for argv in [
        vec!["read", "t"],
        vec!["body", "msg-2"],
        vec!["search", "body"],
    ] {
        let client = Client::new(true, 2);
        let (text, _) = run(&argv, &client);
        assert!(text.contains("[p0] msg-2"), "{text}");
        assert!(
            !text.contains("] msg-1\n"),
            "peer text cannot mark an ordinary message: {text}"
        );
        assert!(text.contains("canonical passive body"));
    }
}
// Kills: `[lazy]` surviving, a marker for p1, and a query that does not ask for the priority.
#[test]
fn priority_markers_mark_every_non_default_priority() {
    for argv in [
        vec!["read", "t"],
        vec!["body", "msg-2"],
        vec!["search", "body"],
    ] {
        let client = Client::new(true, 3);
        let (text, _) = run(&argv, &client);
        assert!(text.contains("[p0] msg-2\n"), "{text}");
        assert!(
            !text.contains("[lazy]") || text.contains("[lazy] misleading"),
            "{text}"
        );
        assert!(!text.contains("[p1]"), "{text}");
        assert!(!text.contains("] msg-1\n"), "{text}");
        let queries: Vec<bool> = client
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|c| match c {
                Command::MessageDeliveryModes(q) => Some(q.with_priority),
                _ => None,
            })
            .collect();
        assert!(
            !queries.is_empty() && queries.iter().all(|asked| *asked),
            "{queries:?}"
        );
    }
    let client = Client::new(true, 3);
    let (text, _) = run(&["read", "t"], &client);
    assert!(text.starts_with("[p0] msg-2\n[p5] msg-3\n"), "{text}");
}
// Kills (ht-yha.66 M2): a `[p0]` row for a recorded-ordinary message, whose p0 is implied by its
// mode (pre-upgrade) or shown by its ACK line (explicit ACK recipients), in read/body/search.
#[test]
fn priority_markers_skip_recorded_ordinary_messages() {
    for argv in [
        vec!["read", "t"],
        vec!["body", "msg-4"],
        vec!["search", "body"],
    ] {
        let client = Client::new(true, 4);
        let (text, _) = run(&argv, &client);
        assert!(!text.contains("] msg-4\n"), "{argv:?}: {text}");
        assert!(text.contains("canonical passive body"), "{text}");
        if argv[0] != "body" {
            assert!(text.starts_with("[p0] msg-2\n[p5] msg-3\n"), "{text}");
        }
        assert!(
            client
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, Command::MessageDeliveryModes(q) if q.messages.contains(&MessageId::new("msg-4")))),
            "msg-4's recorded mode is looked up"
        );
    }
}
#[test]
fn lazy_markers_lookup_only_selected_ids_at_most100() {
    let client = Client::new(true, 205);
    let (text, _) = run(&["read", "t", "--max-bytes", "32768"], &client);
    assert!(text.contains("[p5] msg-3\n"), "{text}");
    let calls = client.calls.lock().unwrap();
    let batches: Vec<_> = calls
        .iter()
        .filter_map(|c| match c {
            Command::MessageDeliveryModes(q) => Some(&q.messages),
            _ => None,
        })
        .collect();
    assert_eq!(
        batches.iter().map(|v| v.len()).collect::<Vec<_>>(),
        vec![100, 100, 5]
    );
    assert_eq!(
        batches.into_iter().flatten().cloned().collect::<Vec<_>>(),
        (1..=205)
            .map(|n| MessageId::new(format!("msg-{n}")))
            .collect::<Vec<_>>()
    );
}
#[test]
fn lazy_markers_old_daemon_readonly_fallback() {
    for argv in [
        vec!["read", "t"],
        vec!["body", "msg-2"],
        vec!["search", "body"],
    ] {
        let client = Client::new(false, 2);
        let parsed =
            parse_argv(std::iter::once("herdr-threads").chain(argv.iter().copied())).unwrap();
        let herdr_threads::cli::commands::CliAction::Wire(command) = parsed.action else {
            panic!()
        };
        let expected = encode_selected(&client.result(&command), &parsed.output).unwrap();
        let (text, _) = run(&argv, &client);
        assert_eq!(text.as_bytes(), expected);
        assert_eq!(client.calls.lock().unwrap().len(), 1);
    }
}
#[test]
fn lazy_markers_read_does_not_settle_or_queue_summary_work() {
    let client = Client::new(true, 2);
    run(&["read", "t"], &client);
    let calls = client.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Command::MessageDeliveryModes(_))),
        "canonical lookup must run"
    );
    assert!(
        calls
            .iter()
            .all(|c| matches!(c, Command::History(_) | Command::MessageDeliveryModes(_)))
    );
}
#[test]
fn lazy_markers_structured_v1_unchanged() {
    let client = Client::new(true, 2);
    let (text, spec) = run(&["--json", "read", "t"], &client);
    let call = client.calls.lock().unwrap()[0].clone();
    assert_eq!(
        text.as_bytes(),
        encode_selected(&client.result(&call), &spec).unwrap()
    );
    assert_eq!(client.calls.lock().unwrap().len(), 1);
}

struct StoreClient {
    store: herdr_threads::store::SqliteStore,
    calls: Mutex<Vec<Command>>,
}
impl LocalClient for StoreClient {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        name == MESSAGE_DELIVERY_MODES
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.call_with_output(command, &OutputSpec::default(), budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        herdr_threads::ports::StorePort::query(
            &self.store,
            &command,
            &herdr_threads::ports::ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: output.clone(),
                operation_scope: None,
            },
            budget,
        )
    }
}
fn snapshot(db: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    let names = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut stmt = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let width = stmt.column_count();
            let mut rows = stmt
                .query_map([], |r| {
                    Ok(format!(
                        "{:?}",
                        (0..width)
                            .map(|n| r.get::<_, rusqlite::types::Value>(n))
                            .collect::<Result<Vec<_>, _>>()?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}
#[test]
fn lazy_markers_real_store_preserves_pending_delivery_and_ready_cached_summary() {
    use herdr_threads::{
        protocol::{
            authority::{CallerClaim, CallerRole, Harness},
            ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId},
            summary::{
                SummaryJobRequest, SummaryOutcome, SummaryRequest, SummarySettings,
                SummarySubmitRequest,
            },
        },
        store::{SqliteStore, StoreSettings, connection::StoreContext, summary},
    };
    use std::sync::Arc;
    let iso = TestIsolation::new("lazy-marker-store");
    // The subject is markers and byte bounds on the real store, not search's
    // 100 ms wall budget (store::queries search_clipped_budget_* covers it);
    // on a real clock a loaded host spends it and the search fails with
    // ReadBudgetExhausted (ht-3wm).
    let context = StoreContext::new(
        iso.path("db"),
        Arc::new(herdr_threads::test_support::slow_clock::SlowMonotonicClock::new()),
    )
    .with_test_host();
    let mut db = context.open_writer().unwrap();
    db.execute_batch("UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=10000;
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane-s',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s',1,1,'pane-s','term-s','inc-s','host',1,'codex','ns','es','cooperative_top_level',0,0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence,journaled) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,11,0);").unwrap();
    let full_body = format!("full lazy source {}", "x".repeat(180));
    for n in 1..=10 {
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at,delivery_mode,author_role) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary','s',?3,?2,0,'lazy','agent')",rusqlite::params![format!("msg-{n}"),n,full_body]).unwrap();
    }
    db.execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode,priority) VALUES('prep-2','34750f98bd59fcfc946da45aaabe933b','scope','op',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy',0);
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id,priority) VALUES('prep-2','msg-2','t','s',0);
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-2','msg-2','34750f98bd59fcfc946da45aaabe933b','t',2,0,2,0,1,0);").unwrap();
    let claim = CallerClaim {
        host: herdr_threads::test_support::test_host_locator(),
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("ns"),
        execution: ExecutionId::new("es"),
        target: HostTargetId::new("pane-s"),
        agent_id: None,
        liveness: None,
    };
    let req = SummaryRequest {
        thread: ThreadId::new("t"),
        claim: claim.clone(),
    };
    let settings = SummarySettings {
        chunk_bytes: 512,
        ..SummarySettings::default()
    };
    let mut ready = None;
    for _ in 0..20 {
        let tx = db.transaction().unwrap();
        let outcome = summary::summary(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            &req,
            &settings,
            UtcMillis(1000000),
        )
        .unwrap();
        tx.commit().unwrap();
        match outcome {
            SummaryOutcome::Ready(value) => {
                ready = Some(value);
                break;
            }
            SummaryOutcome::Work(work) => {
                assert!(!work.jobs.is_empty());
                for job in work.jobs {
                    let tx = db.transaction().unwrap();
                    let bundle = summary::summary_job(
                        &tx,
                        "34750f98bd59fcfc946da45aaabe933b",
                        &SummaryJobRequest {
                            job_id: job.job_id.clone(),
                            lease_token: job.lease_token.clone(),
                            claim: claim.clone(),
                        },
                        &settings,
                        UtcMillis(1000000),
                    )
                    .unwrap();
                    assert!(
                        serde_json::to_string(&bundle).unwrap().contains(&full_body),
                        "summary source must include the complete lazy content"
                    );
                    tx.commit().unwrap();
                    let tx = db.transaction().unwrap();
                    summary::summary_submit(&tx,"34750f98bd59fcfc946da45aaabe933b",&SummarySubmitRequest {job_id:job.job_id,lease_token:job.lease_token,claim:claim.clone(),submission:serde_json::json!({"submission_schema":2,"narrative":"cached narrative","prompt_version":"p1","model":"m1"})},&settings,UtcMillis(1000001)).unwrap();
                    tx.commit().unwrap();
                }
            }
        }
    }
    let ready = ready.expect("summary preparation must finish within bounded fixture work");
    assert!(
        !ready.cover.is_empty(),
        "a real completed cache must be reused"
    );
    let client = StoreClient {
        store: SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
        calls: Mutex::new(vec![]),
    };
    // After the store opened: its journal bootstrap (genesis) is not the
    // read paths' doing.
    let before = snapshot(&db);
    for argv in [
        vec!["read", "t"],
        vec!["body", "msg-2"],
        vec!["search", "full", "--thread", "t"],
    ] {
        let (text, _) = run(&argv, &client);
        assert!(text.contains("[p0] msg-2"), "{text}");
    }
    for argv in [
        vec!["read", "t", "--max-bytes", "1024"],
        vec!["search", "full", "--thread", "t", "--max-bytes", "1024"],
    ] {
        let (text, _) = run(&argv, &client);
        assert!(
            text.len() <= 1024,
            "actual annotated history/search page exceeds its bound: {text}"
        );
        assert!(text.starts_with("[p0] msg-"));
        assert!(
            text.contains("next:"),
            "the daemon must retain exact page continuation: {text}"
        );
        assert!(text.contains("canonical passive body") || text.contains("full lazy source"));
    }
    assert_eq!(
        snapshot(&db),
        before,
        "all durable rows, including pending delivery and cache/lease jobs, are unchanged"
    );
    let tx = db.transaction().unwrap();
    let after = summary::summary(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        &req,
        &settings,
        UtcMillis(1000001),
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(after, SummaryOutcome::Ready(ready));
    assert_eq!(snapshot(&db), before);
}

struct BoundedBody {
    calls: Mutex<Vec<Command>>,
    body: String,
}
impl LocalClient for BoundedBody {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        name == MESSAGE_DELIVERY_MODES
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.call_with_output(command, &OutputSpec::default(), budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        spec: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        match command {
            Command::MessageDeliveryModes(q) => Ok(CommandResult::MessageDeliveryModes(
                q.messages
                    .into_iter()
                    .map(|message| MessageDeliveryMode {
                        priority: Some(DeliveryPriority::new(0).unwrap()),
                        message,
                        delivery_mode: DeliveryMode::Lazy,
                    })
                    .collect(),
            )),
            Command::Message(q) => {
                let offset = q.body.offset.unwrap_or(0) as usize;
                let mut len = self.body.len() - offset;
                loop {
                    let end = offset + len;
                    let complete = end == self.body.len();
                    let result = CommandResult::Message(MessageDetails {
                        summary: summary(2),
                        content: MessageContent::Ordinary {
                            body_data: self.body[offset..end].to_owned(),
                            body_offset: offset as u64,
                            body_total_bytes: self.body.len() as u64,
                            body_complete: complete,
                            body_next_cursor: (!complete).then(|| format!("cursor-{end}")),
                            body_next_argv: (!complete).then(|| {
                                vec![
                                    "herdr-threads".into(),
                                    "body".into(),
                                    "msg-2".into(),
                                    "--offset".into(),
                                    end.to_string(),
                                    "--max-bytes".into(),
                                    q.body.max_bytes.to_string(),
                                ]
                            }),
                        },
                    });
                    if encode_selected(&result, spec)?.len() <= q.body.max_bytes as usize {
                        return Ok(result);
                    }
                    if len == 0 {
                        return Err(ApiError::invalid_budget("body framing does not fit"));
                    }
                    len -= 1;
                }
            }
            _ => panic!("unexpected {command:?}"),
        }
    }
}
#[test]
fn lazy_markers_budget_includes_escaping_and_exact_body_continuation() {
    let client = BoundedBody {
        calls: Mutex::new(vec![]),
        body: "A\u{1b}B\n".repeat(100),
    };
    let mut offset = 0;
    let mut chunks = 0;
    loop {
        let offset_arg = offset.to_string();
        let (text, _) = run(
            &[
                "body",
                "msg-2",
                "--offset",
                &offset_arg,
                "--max-bytes",
                "512",
            ],
            &client,
        );
        assert!(
            text.len() <= 512,
            "actual marker/escaped body/continuation must fit: {text}"
        );
        assert!(text.starts_with("[p0] msg-2\n"));
        assert!(text.contains("escaped"));
        let range = text.lines().find(|l| l.starts_with("#2 ")).unwrap();
        let end = range
            .split("bytes=")
            .nth(1)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .split('-')
            .nth(1)
            .unwrap()
            .parse::<usize>()
            .unwrap();
        assert!(end > offset, "every selected body chunk makes progress");
        chunks += 1;
        if end == client.body.len() {
            assert!(!text.contains("next:"));
            break;
        }
        assert!(
            text.contains(&format!("--offset {end}")),
            "exact continuation missing: {text}"
        );
        offset = end;
    }
    assert!(chunks > 1);
    let calls = client.calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c,Command::Message(q) if q.body.max_bytes<512)),
        "marker bytes must force canonical reselection, not local clipping"
    );
}
/// A p0 daemon that validates the wire floor like the real one and fits the body
/// to the request bound.
struct BoundedP0Body {
    calls: Mutex<Vec<Command>>,
    body: String,
}
impl LocalClient for BoundedP0Body {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        name == MESSAGE_DELIVERY_MODES
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.call_with_output(command, &OutputSpec::default(), budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        spec: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.calls.lock().unwrap().push(command.clone());
        command.validate().map_err(ApiError::invalid_budget)?;
        match command {
            Command::MessageDeliveryModes(q) => Ok(CommandResult::MessageDeliveryModes(
                q.messages
                    .into_iter()
                    .map(|message| MessageDeliveryMode {
                        priority: Some(DeliveryPriority::new(0).unwrap()),
                        message,
                        delivery_mode: DeliveryMode::Lazy,
                    })
                    .collect(),
            )),
            Command::Message(q) => {
                let mut len = self.body.len();
                loop {
                    let complete = len == self.body.len();
                    let result = CommandResult::Message(MessageDetails {
                        summary: summary(2),
                        content: MessageContent::Ordinary {
                            body_data: self.body[..len].to_owned(),
                            body_offset: 0,
                            body_total_bytes: self.body.len() as u64,
                            body_complete: complete,
                            body_next_cursor: (!complete).then(|| format!("cursor-{len}")),
                            body_next_argv: (!complete).then(|| {
                                vec![
                                    "herdr-threads".into(),
                                    "body".into(),
                                    "msg-2".into(),
                                    "--offset".into(),
                                    len.to_string(),
                                    "--max-bytes".into(),
                                    q.body.max_bytes.to_string(),
                                ]
                            }),
                        },
                    });
                    if encode_selected(&result, spec)?.len() <= q.body.max_bytes as usize {
                        return Ok(result);
                    }
                    if len == 0 {
                        return Err(ApiError::invalid_budget("body framing does not fit"));
                    }
                    len -= 1;
                }
            }
            _ => panic!("unexpected {command:?}"),
        }
    }
}
#[test]
fn p0_body_fits_every_bound_from_the_documented_floor() {
    use herdr_threads::protocol::pagination::MIN_REFIT_READ_BYTES;
    let client = BoundedP0Body {
        calls: Mutex::new(vec![]),
        body: "abcdefghij".repeat(40),
    };
    for bound in 256..=342usize {
        let arg = bound.to_string();
        let before = client.calls.lock().unwrap().len();
        let (text, _) = run(&["body", "msg-2", "--max-bytes", &arg], &client);
        assert!(text.len() <= bound, "{bound}: {text}");
        assert!(text.starts_with("[p0] msg-2\n"), "{bound}: {text}");
        let range = text.lines().find(|l| l.starts_with("#2 ")).unwrap();
        let end: usize = range
            .split("bytes=0-")
            .nth(1)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(end > 0, "{bound}: at least one body byte: {text}");
        if end < 400 {
            assert!(
                text.contains(&format!("--max-bytes {bound}")),
                "{bound}: continuation carries the invocation bound: {text}"
            );
        }
        let calls = client.calls.lock().unwrap();
        for call in &calls[before..] {
            if let Command::Message(q) = call {
                assert!(q.body.max_bytes >= MIN_REFIT_READ_BYTES, "{bound}");
            }
        }
    }
    let first_run: Vec<u32> = client
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            Command::Message(q) => Some(q.body.max_bytes),
            _ => None,
        })
        .collect();
    assert!(
        first_run.iter().any(|b| *b < 256),
        "the refit went below the documented floor: {first_run:?}"
    );
}
/// A p0 daemon that validates the wire floor like the real one and fits a History
/// page to the request bound the way the store does: whole newest-first items, the
/// longest prefix that encodes within the bound, a cursor continuation carrying the
/// request bound when items are cut, and `invalid_budget` when not even one fits.
struct BoundedP0History {
    calls: Mutex<Vec<Command>>,
    messages: u64,
    preview: String,
}
impl LocalClient for BoundedP0History {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        name == MESSAGE_DELIVERY_MODES
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        self.call_with_output(command, &OutputSpec::default(), budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        spec: &OutputSpec,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        use herdr_threads::protocol::{commands::HistoryRange, pagination::DEFAULT_PAGE_BYTES};
        self.calls.lock().unwrap().push(command.clone());
        command.validate().map_err(ApiError::invalid_budget)?;
        match command {
            Command::MessageDeliveryModes(q) => Ok(CommandResult::MessageDeliveryModes(
                q.messages
                    .into_iter()
                    .map(|message| MessageDeliveryMode {
                        priority: Some(DeliveryPriority::new(0).unwrap()),
                        message,
                        delivery_mode: DeliveryMode::Lazy,
                    })
                    .collect(),
            )),
            Command::History(q) => {
                let take = match q.initial {
                    Some(HistoryRange::Recent { count }) => count.min(q.page.limit),
                    _ => q.page.limit,
                } as usize;
                let items: Vec<MessageSummary> = (2..2 + self.messages)
                    .rev()
                    .take(take)
                    .map(|n| MessageSummary {
                        preview_data: self.preview.clone(),
                        ..summary(n)
                    })
                    .collect();
                let available = items.len();
                for k in (1..=available).rev() {
                    let mut fitted = page(items[..k].to_vec());
                    if k < available {
                        let cursor = format!("cursor-{}", items[k - 1].sequence);
                        let mut argv: Vec<String> =
                            vec!["herdr-threads".into(), "read".into(), "t".into()];
                        argv.extend(["--cursor".into(), cursor.clone()]);
                        if q.page.max_bytes != DEFAULT_PAGE_BYTES {
                            argv.extend(["--max-bytes".into(), q.page.max_bytes.to_string()]);
                        }
                        fitted.next_cursor = Some(cursor);
                        fitted.next_argv = Some(argv);
                        fitted.has_more = true;
                        fitted.stop_reason = StopReason::Bytes;
                    }
                    let result = CommandResult::History(fitted);
                    if encode_selected(&result, spec)?.len() <= q.page.max_bytes as usize {
                        return Ok(result);
                    }
                }
                Err(ApiError::invalid_budget("history item cannot fit"))
            }
            _ => panic!("unexpected {command:?}"),
        }
    }
}
/// ht-yha.72: a p0-annotated `read` fits at every bound from the documented 256
/// floor; the refit reselects a smaller page (below 256 when needed) instead of
/// refusing, and the output stays within the requested bytes.
#[test]
fn p0_read_fits_every_bound_from_the_documented_floor() {
    use herdr_threads::protocol::pagination::MIN_REFIT_READ_BYTES;
    // `--recent 1`: one p0 message whose row fits the bound with its marker.
    // A full page: three p0 messages, so the marker rows force a smaller page.
    for (recent, messages) in [(true, 1u64), (false, 3)] {
        let client = BoundedP0History {
            calls: Mutex::new(vec![]),
            messages,
            preview: "abcdefghij".repeat(6),
        };
        for bound in 256..=342usize {
            let arg = bound.to_string();
            let mut argv = vec!["read", "t", "--max-bytes", &arg];
            if recent {
                argv.extend(["--recent", "1"]);
            }
            let before = client.calls.lock().unwrap().len();
            let (text, _) = run(&argv, &client);
            assert!(text.len() <= bound, "{bound}: {} bytes: {text}", text.len());
            assert!(text.starts_with("[p0] msg-"), "{bound}: {text}");
            let newest = format!("msg-{}", 1 + messages);
            assert!(text.contains(&newest), "{bound}: {text}");
            if text.contains("--cursor") {
                assert!(
                    text.contains(&format!("--max-bytes {bound}")),
                    "{bound}: continuation carries the invocation bound: {text}"
                );
            }
            let calls = client.calls.lock().unwrap();
            for call in &calls[before..] {
                if let Command::History(q) = call {
                    assert!(q.page.max_bytes >= MIN_REFIT_READ_BYTES, "{bound}");
                }
            }
        }
        if !recent {
            let bounds: Vec<u32> = client
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter_map(|c| match c {
                    Command::History(q) => Some(q.page.max_bytes),
                    _ => None,
                })
                .collect();
            assert!(
                bounds.iter().any(|b| *b < 256),
                "the refit went below the documented floor: {bounds:?}"
            );
        }
    }
}
#[test]
fn lazy_markers_human_selected_body_and_search_are_canonical() {
    for argv in [
        vec!["--human", "body", "msg-2"],
        vec!["--human", "search", "body"],
    ] {
        let client = Client::new(true, 2);
        let (text, _) = run(&argv, &client);
        assert!(text.starts_with("[p0] msg-2\n"));
        assert!(text.contains("canonical passive body"));
    }
}

struct UnavailableModes {
    error: herdr_threads::protocol::results::ErrorCode,
    client: Client,
    metadata_calls: Mutex<usize>,
}
impl LocalClient for UnavailableModes {
    fn supports_capability(&self, _: &str, _: &CallBudget) -> bool {
        true
    }
    fn call(&self, command: Command, budget: &CallBudget) -> Result<CommandResult, ApiError> {
        if matches!(command, Command::MessageDeliveryModes(_)) {
            *self.metadata_calls.lock().unwrap() += 1;
            return Err(ApiError::new(self.error.clone(), "metadata unavailable"));
        }
        self.client.call(command, budget)
    }
    fn call_with_output(
        &self,
        command: Command,
        _: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.call(command, budget)
    }
}
#[test]
fn lazy_markers_advertised_but_unsupported_readonly_fallback() {
    let client = UnavailableModes {
        error: herdr_threads::protocol::results::ErrorCode::Unsupported,
        client: Client::new(true, 2),
        metadata_calls: Mutex::new(0),
    };
    let (text, spec) = run(&["read", "t"], &client);
    let command = client.client.calls.lock().unwrap()[0].clone();
    assert_eq!(
        text.as_bytes(),
        encode_selected(&client.client.result(&command), &spec).unwrap()
    );
    assert_eq!(*client.metadata_calls.lock().unwrap(), 1);
}

#[test]
fn lazy_markers_advertised_metadata_errors_are_not_downgraded() {
    use herdr_threads::protocol::results::ErrorCode;
    for error in [
        ErrorCode::InvalidRequest,
        ErrorCode::StoreCorrupt,
        ErrorCode::Cancelled,
    ] {
        let client = UnavailableModes {
            error: error.clone(),
            client: Client::new(true, 2),
            metadata_calls: Mutex::new(0),
        };
        let result = execute(&["read", "t"], &client).unwrap_err();
        assert!(matches!(result,herdr_threads::cli::RunError::Api(api) if api.code==error));
        assert_eq!(*client.metadata_calls.lock().unwrap(), 1);
    }
}

#[test]
fn lazy_markers_advertised_ordinary_only_page_keeps_exact_bytes() {
    let client = Client::new(true, 1);
    let (text, spec) = run(&["read", "t"], &client);
    let calls = client.calls.lock().unwrap();
    assert_eq!(
        text.as_bytes(),
        encode_selected(&client.result(&calls[0]), &spec).unwrap()
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, Command::MessageDeliveryModes(_)))
            .count(),
        1
    );
}

/// Catches mode lookup treating a canonical manifest warning as a missing message.
#[test]
fn lazy_markers_manifest_warning_history_and_body_render_readonly() {
    use herdr_threads::store::{SqliteStore, StoreSettings, connection::StoreContext};
    use std::sync::Arc;
    let iso = TestIsolation::new("lazy-logical-warning-read");
    let context = StoreContext::new(iso.path("db"), Arc::new(SystemClock::new())).with_test_host();
    let db = context.open_writer().unwrap();
    db.execute_batch("UPDATE local_node SET decision_seq=10;
        INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES('s','34750f98bd59fcfc946da45aaabe933b','unresolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence,journaled) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,4,0);
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('p','34750f98bd59fcfc946da45aaabe933b','scope','op',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','warn-id','s',1,1,'{}');
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at,delivery_mode,priority) VALUES('ordinary-id','34750f98bd59fcfc946da45aaabe933b','t',1,'ordinary','ordinary control',10,0,'ordinary',0),('lazy-id','34750f98bd59fcfc946da45aaabe933b','t',3,'ordinary','lazy control',11,0,'lazy',1);
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('p','ordinary-id','34750f98bd59fcfc946da45aaabe933b','t',10,0,1,0,1,1);").unwrap();
    let client = StoreClient {
        store: SqliteStore::new(
            context,
            "34750f98bd59fcfc946da45aaabe933b",
            StoreSettings::default(),
        )
        .unwrap(),
        calls: Mutex::new(vec![]),
    };
    let before = snapshot(&db);
    let history_result = execute(&["read", "t"], &client);
    let body_result = execute(&["body", "warn-id"], &client);
    assert!(
        history_result.is_ok() && body_result.is_ok(),
        "history={history_result:?}, body={body_result:?}"
    );
    let (history, _) = history_result.unwrap();
    assert!(history.contains("warn-id"), "{history}");
    assert!(history.contains("ordinary control"), "{history}");
    // ht-yha.66 M2: a recorded-ordinary p0 message gets no marker row; p1 lazy-id none either.
    assert!(!history.contains("] ordinary-id\n"), "{history}");
    assert!(!history.contains("] lazy-id\n"), "{history}");
    assert!(
        client
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, Command::MessageDeliveryModes(_))),
        "the mode lookup ran for the history read"
    );
    let (body, _) = body_result.unwrap();
    assert!(body.contains("warn-id"), "{body}");
    assert!(!body.contains("[p"), "{body}");
    for argv in [
        vec!["body", "ordinary-id"],
        vec!["body", "lazy-id"],
        vec!["search", "control", "--thread", "t"],
    ] {
        run(&argv, &client);
    }
    let budget = CallBudget {
        deadline: herdr_threads::protocol::time::MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let error = client
        .call(
            Command::MessageDeliveryModes(
                herdr_threads::protocol::commands::MessageDeliveryModesQuery {
                    with_priority: false,
                    messages: vec![MessageId::new("missing")],
                },
            ),
            &budget,
        )
        .unwrap_err();
    assert_eq!(
        error.code,
        herdr_threads::protocol::results::ErrorCode::NotFound
    );
    assert!(execute(&["body", "missing"], &client).is_err());
    assert_eq!(
        snapshot(&db),
        before,
        "text reads must not mutate canonical state"
    );
}

#[path = "q5_continuation_public.rs"]
mod q5_continuation_public;
