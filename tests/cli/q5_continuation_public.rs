//! Public parsed CLI controls: all selected rows and opaque cursors come from SQLite.
//! Mounted as a child of tests/cli/lazy_markers.rs; no production helper is needed.
use super::*;
use herdr_threads::{
    cli::{RunError, actor_route::InvocationActor, commands::CliAction},
    ports::{ReadContext, StorePort},
    protocol::{output::with_render_now, results::ErrorCode, time::MonoInstant},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::sync::Arc;

struct RecordedStore {
    inner: StoreClient,
    selected: Mutex<Vec<(Command, OutputSpec, CommandResult)>>,
    advertised: bool,
}
impl LocalClient for RecordedStore {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        self.advertised && name == MESSAGE_DELIVERY_MODES
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
        let result = self
            .inner
            .call_with_output(command.clone(), output, budget)?;
        if matches!(command, Command::History(_) | Command::Search(_)) {
            self.selected
                .lock()
                .unwrap()
                .push((command, output.clone(), result.clone()));
        }
        Ok(result)
    }
}
struct Fixture {
    _iso: TestIsolation,
    db: rusqlite::Connection,
    client: RecordedStore,
}
impl Fixture {
    fn new() -> Self {
        Self::with_lazy_only(None)
    }
    fn with_lazy_only(lazy_only: Option<&str>) -> Self {
        let iso = TestIsolation::new("q5-public-continuations");
        let context =
            StoreContext::new(iso.path("db"), Arc::new(SystemClock::new())).with_test_host();
        let db = context.open_writer().unwrap();
        db.execute_batch("UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=10000;
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane-s',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,41);").unwrap();
        // Historical ordinary-content rows with canonical mixed delivery modes.
        // Read/Search do not require a membership/read cursor; none is manufactured.
        for n in 1..=40_i64 {
            let id = format!("q5-msg-{n}");
            let is_lazy = match lazy_only {
                Some(wanted) => wanted == id,
                None => n % 2 == 0,
            };
            db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at,delivery_mode,author_role) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary','s',?3,?2,0,?4,'agent')",
                rusqlite::params![id, n, format!("needle value π {}", "x".repeat(180)), if is_lazy { "lazy" } else { "ordinary" }]).unwrap();
        }
        let client = RecordedStore {
            inner: StoreClient {
                store: SqliteStore::new(
                    context,
                    "34750f98bd59fcfc946da45aaabe933b",
                    StoreSettings::default(),
                )
                .unwrap(),
                calls: Mutex::new(vec![]),
            },
            selected: Mutex::new(vec![]),
            advertised: true,
        };
        Self {
            _iso: iso,
            db,
            client,
        }
    }
    fn raw(&self, command: &Command, spec: &OutputSpec) -> Result<CommandResult, ApiError> {
        StorePort::query(
            &self.client.inner.store,
            command,
            &ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: spec.clone(),
                operation_scope: None,
            },
            &CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
        )
    }
    fn lazy_ids(&self, result: &CommandResult) -> Vec<MessageId> {
        ids(result)
            .into_iter()
            .filter(|id| {
                self.db
                    .query_row(
                        "SELECT delivery_mode='lazy' FROM messages WHERE id=?1",
                        [id.as_str()],
                        |r| r.get::<_, bool>(0),
                    )
                    .unwrap()
            })
            .collect()
    }
}
fn ids(result: &CommandResult) -> Vec<MessageId> {
    match result {
        CommandResult::History(page) => page.items.iter().map(|m| m.message.clone()).collect(),
        CommandResult::Search(search) => search
            .matches
            .items
            .iter()
            .filter_map(|hit| {
                if let SearchHit::Body(m) = hit {
                    Some(m.message.clone())
                } else {
                    None
                }
            })
            .collect(),
        _ => panic!("unexpected selected result"),
    }
}
fn next(result: &CommandResult) -> (&Option<String>, &Option<Vec<String>>) {
    match result {
        CommandResult::History(page) => (&page.next_cursor, &page.next_argv),
        CommandResult::Search(search) => (&search.matches.next_cursor, &search.matches.next_argv),
        _ => panic!("unexpected selected result"),
    }
}
// Decode the actual stable machine row shape, independently of the backend
// result count. The simple fixture body has no controls requiring escaping.
fn rendered_rows(raw: &CommandResult, text: &str) -> Vec<MessageId> {
    match raw {
        CommandResult::History(page) => {
            let rows: Vec<_> = text.lines().filter(|line| line.starts_with('#')).collect();
            assert_eq!(
                rows.len(),
                page.items.len(),
                "printed History row multiplicity changed"
            );
            rows.iter()
                .zip(&page.items)
                .map(|(row, m)| {
                    let (header, body) = row.split_once(": ").expect("history body separator");
                    assert_eq!(
                        body, m.preview_data,
                        "full selected History body was truncated/changed"
                    );
                    MessageId::new(header.split_whitespace().nth(1).unwrap())
                })
                .collect()
        }
        CommandResult::Search(search) => {
            let rows: Vec<_> = text
                .lines()
                .filter_map(|line| line.strip_prefix("matches: "))
                .collect();
            assert_eq!(
                rows.len(),
                1,
                "actual Search matches payload missing/duplicated"
            );
            let data: serde_json::Value = serde_json::from_str(rows[0]).unwrap();
            let items = data["items"].as_array().unwrap();
            assert_eq!(
                items.len(),
                search.matches.items.len(),
                "printed Search row multiplicity changed"
            );
            items
                .iter()
                .zip(&search.matches.items)
                .map(|(row, hit)| {
                    let SearchHit::Body(m) = hit else {
                        panic!("fixture must match only body rows")
                    };
                    assert_eq!(row["kind"], "body");
                    assert_eq!(
                        row["data"]["preview_data"], m.preview_data,
                        "full selected Search body was truncated/changed"
                    );
                    MessageId::new(row["data"]["message"].as_str().unwrap())
                })
                .collect()
        }
        _ => panic!(),
    }
}
fn page_of(command: &Command) -> &herdr_threads::protocol::pagination::PageRequest {
    match command {
        Command::History(q) => &q.page,
        Command::Search(q) => &q.page,
        _ => panic!(),
    }
}
fn wire(argv: &[String]) -> (Command, OutputSpec, InvocationActor) {
    let parsed = parse_argv(argv.iter().map(String::as_str)).unwrap();
    let CliAction::Wire(command) = parsed.action else {
        panic!("read must use its public wire route")
    };
    (command, parsed.output, parsed.actor)
}
fn argv(search: bool, max: u32, pad: usize, human: bool) -> Vec<String> {
    let mut args = vec!["herdr-threads".into()];
    if human {
        args.push("human".into());
        args.push("--machine".into());
    }
    args.extend([
        "--state-dir".into(),
        "/q5 state with spaces".into(),
        "--host-endpoint".into(),
        format!("/q5 host with spaces/{}", "h".repeat(pad)),
    ]);
    if search {
        args.extend([
            "search".into(),
            "needle value".into(),
            "--thread".into(),
            "t".into(),
        ]);
    } else {
        args.extend(["read".into(), "t".into(), "--after".into(), "0".into()]);
    }
    args.extend([
        "--limit".into(),
        "7".into(),
        "--max-bytes".into(),
        max.to_string(),
    ]);
    args
}
fn execute_owned(argv: &[String], client: &impl LocalClient) -> (Result<(), RunError>, Vec<u8>) {
    let iso = TestIsolation::new("q5-cli-invocation");
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
    let parsed = parse_argv(argv.iter().map(String::as_str)).unwrap();
    let _guard = PresentationGuard::enter(parsed.presentation, &parsed.output);
    let before = std::fs::read_dir(iso.path("intents")).unwrap().count();
    let mut out = Vec::new();
    let result = run_cooperative(
        parsed,
        &journal,
        &contexts,
        None,
        Role::TopLevel,
        client,
        &SystemClock::new(),
        &mut out,
    );
    assert_eq!(
        std::fs::read_dir(iso.path("intents")).unwrap().count(),
        before,
        "reads must not journal a mutation"
    );
    (result, out)
}
// Calibrate only route padding to an actual serializer boundary. The success oracle
// below is a parsed public invocation and its recorded canonical read requests.
fn boundary(
    f: &Fixture,
    search: bool,
    max: u32,
    human: bool,
    lose_lazy: bool,
) -> (Vec<String>, Option<String>) {
    for pad in 0..=512 {
        let args = argv(search, max, pad, human);
        let (command, spec, actor) = wire(&args);
        let _namespace = herdr_threads::protocol::output::CommandNamespaceGuard::enter(
            actor == InvocationActor::Human,
        );
        let raw = match f.raw(&command, &spec) {
            Ok(raw) => raw,
            Err(error) if error.code == ErrorCode::InvalidBudget => continue,
            Err(error) => panic!("canonical calibration failed before budget boundary: {error:?}"),
        };
        let selected = ids(&raw);
        if selected.len() < 2 || next(&raw).1.is_none() {
            continue;
        }
        let lazy = if lose_lazy {
            vec![selected.last().unwrap().clone()]
        } else {
            f.lazy_ids(&raw)
        };
        if lazy.is_empty() || lazy.len() == selected.len() {
            continue;
        }
        let bytes = encode_selected(&raw, &spec).unwrap().len();
        let markers: usize = lazy
            .iter()
            .map(|id| format!("[lazy] {}\n", id.as_str()).len())
            .sum();
        if bytes <= max as usize && bytes + markers > max as usize {
            return (args, lose_lazy.then(|| lazy[0].as_str().to_owned()));
        }
    }
    panic!(
        "could not reach canonical {max}-byte serializer boundary: search={search}, human={human}, lose_lazy={lose_lazy}"
    );
}
fn without_budget(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--max-bytes" {
            assert!(it.next().is_some());
        } else {
            result.push(arg.clone());
        }
    }
    result
}
fn follow(search: bool, max: u32, human: bool, lose_lazy: bool) {
    with_render_now(UtcMillis(0), || {
        let f = Fixture::new();
        let (mut args, lazy_only) = boundary(&f, search, max, human, lose_lazy);
        // Delivery mode is immutable. Seed a fresh canonical fixture with the
        // chosen final-row Lazy mode, rather than modifying existing messages.
        let f = match lazy_only {
            Some(id) => Fixture::with_lazy_only(Some(&id)),
            None => f,
        };
        let before = snapshot(&f.db);
        let (_, original_spec, original_actor) = wire(&args);
        let mut seen = std::collections::BTreeSet::new();
        let mut refits = 0;
        for step in 0..=64 {
            assert!(step < 64, "continuation did not terminate");
            f.client.selected.lock().unwrap().clear();
            let (original_command, spec, actor) = wire(&args);
            assert_eq!(spec, original_spec);
            assert_eq!(actor, original_actor);
            assert_eq!(
                page_of(&original_command).max_bytes,
                max,
                "step={step} argv={args:?}"
            );
            let (result, out) = execute_owned(&args, &f.client);
            result.unwrap_or_else(|e| panic!("step={step} argv={args:?} failed: {e:?}"));
            assert!(
                out.len() <= max as usize,
                "actual final bytes {} exceed M={max}",
                out.len()
            );
            let reads = f.client.selected.lock().unwrap();
            assert!(!reads.is_empty());
            for (command, actual_spec, _) in reads.iter() {
                assert_eq!(actual_spec, &spec);
                let mut expected = original_command.clone();
                match &mut expected {
                    Command::History(q) => q.page.max_bytes = page_of(command).max_bytes,
                    Command::Search(q) => q.page.max_bytes = page_of(command).max_bytes,
                    _ => panic!(),
                }
                assert_eq!(
                    command, &expected,
                    "refit changed original cursor/filter/row limit/candidate bound"
                );
            }
            if reads.len() > 1 {
                refits += 1;
            }
            if step == 0 {
                assert!(
                    reads.len() > 1,
                    "fixture must cross a real annotation refit"
                );
                let initial_ids = ids(&reads[0].2);
                assert!(!f.lazy_ids(&reads[0].2).is_empty());
                assert!(
                    f.lazy_ids(&reads[0].2).len() < initial_ids.len(),
                    "initial page must mix ordinary/Lazy"
                );
                if lose_lazy {
                    assert!(
                        f.lazy_ids(&reads.last().unwrap().2).is_empty(),
                        "reselected page must have no Lazy rows"
                    );
                    assert!(!String::from_utf8_lossy(&out).contains("[lazy]"));
                }
            }
            let (_, _, raw) = reads.last().unwrap();
            let text = std::str::from_utf8(&out).unwrap();
            let rendered_ids = rendered_rows(raw, text);
            assert_eq!(
                rendered_ids,
                ids(raw),
                "actual printed row IDs/order differ from the last canonical selected page"
            );
            for id in rendered_ids {
                assert!(seen.insert(id), "printed continuation replayed a row");
            }
            // Complete row bytes and framing must survive the CLI refit. Only
            // the declared service continuation's budget value may be restored.
            let mut expected_result = raw.clone();
            let expected_argv = match &mut expected_result {
                CommandResult::History(p) => &mut p.next_argv,
                CommandResult::Search(s) => &mut s.matches.next_argv,
                _ => panic!(),
            };
            if let Some(argv) = expected_argv {
                assert_eq!(argv[argv.len() - 2], "--max-bytes");
                let last = argv.len() - 1;
                argv[last] = max.to_string();
            }
            let mut expected_bytes = Vec::new();
            for id in f.lazy_ids(raw) {
                expected_bytes.extend_from_slice(format!("[lazy] {}\n", id.as_str()).as_bytes());
            }
            {
                let _namespace = herdr_threads::protocol::output::CommandNamespaceGuard::enter(
                    original_actor == InvocationActor::Human,
                );
                expected_bytes.extend(encode_selected(&expected_result, &spec).unwrap());
            }
            assert_eq!(
                out, expected_bytes,
                "actual rows/body/framing or original continuation bound changed"
            );
            let printed: Vec<_> = text
                .lines()
                .filter_map(|line| {
                    line.strip_prefix(if search { "matches.next: " } else { "next: " })
                })
                .collect();
            let (cursor, raw_argv) = next(raw);
            let Some(raw_argv) = raw_argv else {
                assert!(printed.is_empty());
                assert!(cursor.is_none());
                assert_eq!(
                    seen.len(),
                    40,
                    "all matching rows must be reached without trimming"
                );
                let expected_ids: std::collections::BTreeSet<_> = (1..=40)
                    .map(|n| MessageId::new(format!("q5-msg-{n}")))
                    .collect();
                assert_eq!(
                    seen, expected_ids,
                    "every exact fixture message must be printed once"
                );
                assert!(refits > 0);
                assert_eq!(
                    snapshot(&f.db),
                    before,
                    "History/Search changed durable rows"
                );
                assert!(
                    f.client
                        .inner
                        .calls
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|c| matches!(
                            c,
                            Command::History(_)
                                | Command::Search(_)
                                | Command::MessageDeliveryModes(_)
                        )),
                    "read made a display mutation/ACK"
                );
                return;
            };
            assert_eq!(printed.len(), 1);
            let emitted = shlex::split(printed[0]).unwrap();
            let (next_command, next_spec, next_actor) = wire(&emitted);
            assert_eq!(
                page_of(&next_command).max_bytes,
                max,
                "internal backend bound escaped in continuation at step {step}: {emitted:?}"
            );
            assert_eq!(
                page_of(&next_command).limit,
                page_of(&original_command).limit
            );
            assert_eq!(
                &page_of(&next_command).cursor,
                cursor,
                "opaque continuation changed"
            );
            assert_eq!(next_spec, original_spec);
            assert_eq!(next_actor, original_actor);
            // SQLite None operation_scope returns agent-spelled argv; the CLI
            // renderer adds the invocation's human namespace on a rendering copy.
            let mut rendered_raw_argv = raw_argv.clone();
            {
                let _namespace = herdr_threads::protocol::output::CommandNamespaceGuard::enter(
                    original_actor == InvocationActor::Human,
                );
                herdr_threads::protocol::output::namespace_argv(&mut rendered_raw_argv);
            }
            assert_eq!(
                without_budget(&emitted),
                without_budget(&rendered_raw_argv),
                "non-budget argv tokens changed"
            );
            match (&original_command, &next_command) {
                (Command::History(a), Command::History(b)) => {
                    assert_eq!(a.thread, b.thread);
                    assert_eq!(a.full_bodies, b.full_bodies);
                }
                (Command::Search(a), Command::Search(b)) => {
                    assert_eq!(a.literal, b.literal);
                    assert_eq!(a.thread, b.thread);
                    assert_eq!(a.max_candidates, b.max_candidates);
                }
                _ => panic!("continuation changed read action"),
            }
            drop(reads);
            args = emitted;
        }
        unreachable!();
    });
}
#[test]
fn q5_history_public_refits_preserve_1400_byte_and_row_bounds_through_completion() {
    follow(false, 1400, false, false);
}
#[test]
fn q5_search_public_refits_preserve_2048_byte_filter_route_actor_and_cursor() {
    follow(true, 2048, true, false);
}
#[test]
fn q5_history_public_reselected_no_lazy_page_still_restores_original_bound() {
    follow(false, 1400, true, true);
}
#[test]
fn q5_search_public_reselected_no_lazy_page_still_restores_original_bound() {
    follow(true, 2048, false, true);
}
#[test]
fn q5_public_json_and_unadvertised_modes_keep_initial_serialization_unchanged() {
    with_render_now(UtcMillis(0), || {
        for search in [false, true] {
            for json in [false, true] {
                let mut f = Fixture::new();
                f.client.advertised = json;
                let mut args = argv(search, 2048, 0, false);
                if json {
                    args.insert(1, "--json".into());
                }
                let (command, spec, _) = wire(&args);
                let expected = encode_selected(&f.raw(&command, &spec).unwrap(), &spec).unwrap();
                let before = snapshot(&f.db);
                let (result, actual) = execute_owned(&args, &f.client);
                result.unwrap();
                assert_eq!(actual, expected);
                assert_eq!(f.client.selected.lock().unwrap().len(), 1);
                assert_eq!(snapshot(&f.db), before);
                assert!(
                    f.client
                        .inner
                        .calls
                        .lock()
                        .unwrap()
                        .iter()
                        .all(|c| matches!(c, Command::History(_) | Command::Search(_)))
                );
            }
        }
    });
}
#[test]
fn q5_public_insufficient_original_bound_writes_nothing_and_performs_no_display_effect() {
    let f = Fixture::new();
    let args = argv(true, 256, 512, false);
    let before = snapshot(&f.db);
    let (result, out) = execute_owned(&args, &f.client);
    assert!(matches!(result,Err(RunError::Api(e)) if e.code==ErrorCode::InvalidBudget));
    assert!(out.is_empty());
    assert_eq!(snapshot(&f.db), before);
    assert!(
        f.client
            .inner
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|c| matches!(c, Command::Search(_) | Command::MessageDeliveryModes(_)))
    );
}
