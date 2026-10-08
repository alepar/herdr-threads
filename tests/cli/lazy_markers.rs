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
            assert!(!q.messages.is_empty() && q.messages.len() <= 100);
            return Ok(CommandResult::MessageDeliveryModes(
                q.messages
                    .into_iter()
                    .map(|message| {
                        let lazy = message.as_str() != "msg-1";
                        MessageDeliveryMode {
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
        uuid::Uuid::from_u128(1),
        "seat-1",
        Duration::from_secs(1),
    )
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
        assert!(text.contains("[lazy] msg-2"), "{text}");
        assert!(
            !text.contains("[lazy] msg-1"),
            "peer text cannot mark an ordinary message: {text}"
        );
        assert!(text.contains("canonical passive body"));
    }
}
#[test]
fn lazy_markers_lookup_only_selected_ids_at_most100() {
    let client = Client::new(true, 205);
    let (text, _) = run(&["read", "t", "--max-bytes", "32768"], &client);
    assert!(text.contains("[lazy] msg-205"));
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
                instance: "i".into(),
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
    let context = StoreContext::new(iso.path("db"), Arc::new(SystemClock::new()));
    let mut db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES('i',0,'host',1,10000);
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('s','i','resolved','native','pane-s',1,1,0);
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s',1,1,'pane-s','term-s','inc-s','host',1,'codex','ns','es','cooperative_top_level',0,0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES('t','i','topic','goal',0,0,11);").unwrap();
    let full_body = format!("full lazy source {}", "x".repeat(180));
    for n in 1..=10 {
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at,delivery_mode,author_role) VALUES(?1,'i','t',?2,'ordinary','s',?3,?2,0,'lazy','agent')",rusqlite::params![format!("msg-{n}"),n,full_body]).unwrap();
    }
    db.execute_batch("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES('prep-2','i','scope','op',zeroblob(32),'t',0,0,0,0,0,0,0,'building','lazy');
        INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES('prep-2','msg-2','t','s');
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('prep-2','msg-2','i','t',2,0,2,0,1,0);").unwrap();
    let claim = CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("ns"),
        execution: ExecutionId::new("es"),
        target: HostTargetId::new("pane-s"),
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
        let outcome = summary::summary(&tx, "i", &req, &settings, UtcMillis(1000000)).unwrap();
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
                        "i",
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
                    summary::summary_submit(&tx,"i",&SummarySubmitRequest {job_id:job.job_id,lease_token:job.lease_token,claim:claim.clone(),submission:serde_json::json!({"submission_schema":2,"narrative":"cached narrative","prompt_version":"p1","model":"m1"})},&settings,UtcMillis(1000001)).unwrap();
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
    let before = snapshot(&db);
    let client = StoreClient {
        store: SqliteStore::new(context, "i", StoreSettings::default()).unwrap(),
        calls: Mutex::new(vec![]),
    };
    for argv in [
        vec!["read", "t"],
        vec!["body", "msg-2"],
        vec!["search", "full", "--thread", "t"],
    ] {
        let (text, _) = run(&argv, &client);
        assert!(text.contains("[lazy] msg-2"), "{text}");
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
        assert!(text.starts_with("[lazy] msg-"));
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
    let after = summary::summary(&tx, "i", &req, &settings, UtcMillis(1000001)).unwrap();
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
        assert!(text.starts_with("[lazy] msg-2\n"));
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
#[test]
fn lazy_markers_human_selected_body_and_search_are_canonical() {
    for argv in [
        vec!["--human", "body", "msg-2"],
        vec!["--human", "search", "body"],
    ] {
        let client = Client::new(true, 2);
        let (text, _) = run(&argv, &client);
        assert!(text.starts_with("[lazy] msg-2\n"));
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
    let context = StoreContext::new(iso.path("db"), Arc::new(SystemClock::new()));
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,decision_seq) VALUES('i',0,10);
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('s','i','unresolved','native',1,0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES('t','i','topic','goal',0,0,4);
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('p','i','scope','op',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','warn-id','s',1,1,'{}');
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at,delivery_mode) VALUES('ordinary-id','i','t',1,'ordinary','ordinary control',10,0,'ordinary'),('lazy-id','i','t',3,'ordinary','lazy control',11,0,'lazy');
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('p','ordinary-id','i','t',10,0,1,0,1,1);").unwrap();
    let client = StoreClient {
        store: SqliteStore::new(context, "i", StoreSettings::default()).unwrap(),
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
    assert!(history.contains("[lazy] lazy-id"), "{history}");
    assert!(!history.contains("[lazy] ordinary-id"));
    let (body, _) = body_result.unwrap();
    assert!(body.contains("warn-id"), "{body}");
    assert!(!body.contains("[lazy]"), "{body}");
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
