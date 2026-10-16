//! Unit child of src/cli/mod.rs. Positive read records are selected by SQLite;
//! only declared post-refit argv is malformed for corruption controls.
use super::{RunError, normalize_read_continuation_budget, output::ReadModes, run_wire_result};
use crate::{
    ports::{LocalClient, ReadContext, StorePort},
    protocol::{
        capabilities::MESSAGE_DELIVERY_MODES,
        commands::{Command, HistoryQuery, HistoryRange, MAX_SEARCH_CANDIDATES, SearchQuery},
        ids::ThreadId,
        output::{ContinuationContext, OutputFormat, OutputSpec, encode_selected, with_render_now},
        pagination::{DEFAULT_PAGE_BYTES, PageRequest},
        results::{ApiError, CommandResult, ErrorCode, SearchHit},
        time::{CallBudget, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use std::sync::{Arc, Mutex};

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}
fn request(search: bool, max: u32) -> Command {
    let page = PageRequest {
        cursor: None,
        limit: 7,
        max_bytes: max,
    };
    if search {
        Command::Search(SearchQuery {
            literal: "needle value".into(),
            thread: Some(ThreadId::new("t")),
            page,
            max_candidates: MAX_SEARCH_CANDIDATES,
        })
    } else {
        Command::History(HistoryQuery {
            thread: ThreadId::new("t"),
            page,
            initial: Some(HistoryRange::After { sequence: 0 }),
            full_bodies: false,
        })
    }
}
fn bound(command: &Command) -> u32 {
    match command {
        Command::History(q) => q.page.max_bytes,
        Command::Search(q) => q.page.max_bytes,
        _ => panic!(),
    }
}
fn argv_mut(result: &mut CommandResult) -> &mut Option<Vec<String>> {
    match result {
        CommandResult::History(p) => &mut p.next_argv,
        CommandResult::Search(s) => &mut s.matches.next_argv,
        CommandResult::Message(m) => match &mut m.content {
            crate::protocol::results::MessageContent::Ordinary { body_next_argv, .. } => {
                body_next_argv
            }
            _ => panic!(),
        },
        _ => panic!(),
    }
}
fn selected_ids(result: &CommandResult) -> Vec<String> {
    match result {
        CommandResult::History(p) => p
            .items
            .iter()
            .map(|m| m.message.as_str().to_owned())
            .collect(),
        CommandResult::Search(s) => s
            .matches
            .items
            .iter()
            .filter_map(|h| {
                if let SearchHit::Body(m) = h {
                    Some(m.message.as_str().to_owned())
                } else {
                    None
                }
            })
            .collect(),
        _ => panic!(),
    }
}
#[derive(Clone, Copy, Debug)]
enum Malformation {
    DuplicateBudget,
    DanglingBudget,
    NonNumericBudget,
    InconsistentBudget,
    MissingBudget,
    ConflictingState,
    ConflictingHost,
    EmptyState,
    LongHost,
    NulState,
    InvalidThreadValue,
    IncompleteCooperative,
    UnknownCooperativeHarness,
}
fn corrupt(argv: &mut Vec<String>, case: Malformation) {
    let value = argv
        .iter()
        .position(|a| a == "--max-bytes")
        .map(|i| i + 1)
        .expect("explicit refitted bound");
    match case {
        Malformation::DuplicateBudget => argv.extend(["--max-bytes".into(), argv[value].clone()]),
        Malformation::DanglingBudget => {
            argv.remove(value);
        }
        Malformation::NonNumericBudget => argv[value] = "unknown".into(),
        Malformation::InconsistentBudget => {
            argv[value] = (argv[value].parse::<u32>().unwrap() + 1).to_string()
        }
        Malformation::MissingBudget => {
            argv.drain(value - 1..=value);
        }
        Malformation::ConflictingState => {
            argv.splice(
                1..1,
                [
                    "--state-dir".into(),
                    "one".into(),
                    "--state-dir".into(),
                    "two".into(),
                ],
            );
        }
        Malformation::ConflictingHost => {
            argv.splice(
                1..1,
                ["--host-endpoint=one".into(), "--host-endpoint=two".into()],
            );
        }
        Malformation::EmptyState => {
            argv.splice(1..1, ["--state-dir=".into()]);
        }
        Malformation::LongHost => {
            let route_value = argv.iter().position(|a| a == "--host-endpoint").unwrap() + 1;
            argv[route_value] = "x".repeat(1025);
        }
        Malformation::NulState => {
            argv.splice(1..1, ["--state-dir=a\0b".into()]);
        }
        Malformation::InvalidThreadValue => {
            let thread_value = argv.iter().position(|a| a == "--thread").unwrap() + 1;
            argv[thread_value] = "--max-bytes".into();
        }
        Malformation::IncompleteCooperative => {
            argv.splice(1..1, ["--cooperative-seat".into(), "s".into()]);
        }
        Malformation::UnknownCooperativeHarness => {
            argv.splice(
                1..1,
                [
                    "--cooperative-seat".into(),
                    "s".into(),
                    "--cooperative-target".into(),
                    "pane-s".into(),
                    "--cooperative-harness".into(),
                    "unregistered-q5".into(),
                    "--cooperative-role".into(),
                    "top-level".into(),
                ],
            );
        }
    }
}
struct Fixture {
    _iso: TestIsolation,
    store: SqliteStore,
    calls: Mutex<Vec<Command>>,
    malformed: Mutex<Option<(u32, Malformation)>>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_long_body(false)
    }
    fn with_long_body(long: bool) -> Self {
        let iso = TestIsolation::new("q5-normalizer-real-results");
        // The calibration's subject is byte budgets, not search's 100 ms
        // wall budget (store::queries search_clipped_budget_* covers it).
        let context = StoreContext::new(
            iso.path("db"),
            Arc::new(crate::test_support::slow_clock::SlowMonotonicClock::new()),
        )
        .with_test_host();
        let db = context.open_writer().unwrap();
        db.execute_batch("UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=10000;
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','pane-s',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence,journaled) VALUES('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,41,0);").unwrap();
        for n in 1..=40_i64 {
            let body = if long && n == 1 {
                format!("{}tail", "π💡".repeat(10922))
            } else {
                format!("needle value π {}", "x".repeat(180))
            };
            db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at,delivery_mode,author_role) VALUES(?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'ordinary','s',?3,?2,0,'lazy','agent')",
                rusqlite::params![format!("q5-msg-{n}"),n,body]).unwrap();
        }
        drop(db);
        Self {
            _iso: iso,
            store: SqliteStore::new(
                context,
                "34750f98bd59fcfc946da45aaabe933b",
                StoreSettings::default(),
            )
            .unwrap(),
            calls: Mutex::new(vec![]),
            malformed: Mutex::new(None),
        }
    }
    fn raw(&self, command: &Command, spec: &OutputSpec) -> Result<CommandResult, ApiError> {
        StorePort::query(
            &self.store,
            command,
            &ReadContext {
                instance: "34750f98bd59fcfc946da45aaabe933b".into(),
                output: spec.clone(),
                operation_scope: None,
            },
            &budget(),
        )
    }
    fn boundary(&self, search: bool, max: u32) -> OutputSpec {
        for pad in 0..=512 {
            let spec = OutputSpec {
                format: OutputFormat::Text,
                context: ContinuationContext {
                    state_dir: None,
                    host: Some(format!("/q5 route/{}", "h".repeat(pad))),
                },
            };
            let raw = match self.raw(&request(search, max), &spec) {
                Ok(raw) => raw,
                Err(error) if error.code == ErrorCode::InvalidBudget => continue,
                Err(error) => {
                    panic!("canonical calibration failed before budget boundary: {error:?}")
                }
            };
            let markers: usize = selected_ids(&raw)
                .iter()
                .map(|id| format!("[p0] {id}\n").len())
                .sum();
            let len = encode_selected(&raw, &spec).unwrap().len();
            if markers > 0 && len <= max as usize && len + markers > max as usize {
                return spec;
            }
        }
        panic!("could not force a real annotation refit");
    }
}
impl LocalClient for Fixture {
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
        let mut result = self.raw(&command, spec)?;
        if matches!(command, Command::History(_) | Command::Search(_))
            && let Some((original, case)) = *self.malformed.lock().unwrap()
            && bound(&command) < original
        {
            corrupt(
                argv_mut(&mut result)
                    .as_mut()
                    .expect("continued post-refit real result"),
                case,
            );
        }
        Ok(result)
    }
}
fn stripped(mut result: CommandResult) -> CommandResult {
    *argv_mut(&mut result) = None;
    result
}
#[test]
fn q5_normalizer_changes_only_budget_value_and_preserves_flag_like_raw_data() {
    let f = Fixture::new();
    let command = request(true, 512);
    let mut result = f
        .raw(
            &request(true, 2048),
            &OutputSpec {
                format: OutputFormat::Text,
                ..OutputSpec::default()
            },
        )
        .unwrap();
    let original_opaque = match &result {
        CommandResult::Search(s) => s.matches.next_cursor.clone().unwrap(),
        _ => panic!(),
    };
    let mut args = vec![
        "herdr-threads".into(),
        "human".into(),
        "--state-dir".into(),
        "--".into(),
        "--state-dir=--".into(),
        "--host-endpoint=--max-bytes".into(),
        "--machine".into(),
        "search".into(),
        "--thread".into(),
        "t".into(),
        "--cursor".into(),
        original_opaque,
        "--limit".into(),
        "7".into(),
        "--max-bytes".into(),
        "512".into(),
        "--".into(),
        "--max-bytes".into(),
    ];
    *argv_mut(&mut result) = Some(args.clone());
    let before = result.clone();
    // The request itself describes the exact literal in the declared argv.
    let Command::Search(mut query) = command else {
        panic!()
    };
    query.literal = "--max-bytes".into();
    normalize_read_continuation_budget(&Command::Search(query), &mut result, 2048).unwrap();
    args[15] = "2048".into();
    assert_eq!(
        argv_mut(&mut result).as_ref().unwrap(),
        &args,
        "only the option VALUE slot may change; route '--' is data, trailing '--max-bytes' is literal"
    );
    assert_eq!(
        serde_json::to_value(stripped(result)).unwrap(),
        serde_json::to_value(stripped(before)).unwrap(),
        "rows, typed cursor, filters, counters and high waters changed"
    );
}
#[test]
fn q5_normalizer_default_history_omission_adds_only_original_byte_bound() {
    let f = Fixture::new();
    let command = request(false, DEFAULT_PAGE_BYTES);
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..OutputSpec::default()
    };
    let mut result = f.raw(&command, &spec).unwrap();
    let before = result.clone();
    let mut expected = argv_mut(&mut result).clone().unwrap();
    assert!(!expected.iter().any(|a| a == "--max-bytes"));
    normalize_read_continuation_budget(&command, &mut result, 65536).unwrap();
    expected.extend(["--max-bytes".into(), "65536".into()]);
    assert_eq!(argv_mut(&mut result).as_ref().unwrap(), &expected);
    assert_eq!(
        serde_json::to_value(stripped(result)).unwrap(),
        serde_json::to_value(stripped(before)).unwrap()
    );
}
#[test]
fn q5_normalizer_terminal_and_never_refit_results_remain_exactly_unchanged() {
    let f = Fixture::new();
    let spec = OutputSpec {
        format: OutputFormat::Json,
        ..OutputSpec::default()
    };
    for search in [false, true] {
        let mut command = request(search, DEFAULT_PAGE_BYTES);
        match &mut command {
            Command::History(q) => q.page.limit = 100,
            Command::Search(q) => q.page.limit = 100,
            _ => panic!(),
        }
        let terminal_spec = OutputSpec {
            format: OutputFormat::Text,
            ..OutputSpec::default()
        };
        let mut reached = std::collections::BTreeSet::new();
        let mut result = None;
        for _ in 0..64 {
            assert_eq!(
                bound(&command),
                DEFAULT_PAGE_BYTES,
                "terminal fixture must retain its fixed real bound"
            );
            let selected = f.raw(&command, &terminal_spec).unwrap();
            for id in selected_ids(&selected) {
                assert!(reached.insert(id), "real terminal walk replayed a row");
            }
            let cursor = match &selected {
                CommandResult::History(p) => p.next_cursor.clone(),
                CommandResult::Search(s) => s.matches.next_cursor.clone(),
                _ => panic!(),
            };
            if let Some(cursor) = cursor {
                match &mut command {
                    Command::History(q) => {
                        q.page.cursor = Some(cursor);
                        q.initial = None;
                    }
                    Command::Search(q) => q.page.cursor = Some(cursor),
                    _ => panic!(),
                }
            } else {
                result = Some(selected);
                break;
            }
        }
        let expected: std::collections::BTreeSet<_> =
            (1..=40).map(|n| format!("q5-msg-{n}")).collect();
        assert_eq!(
            reached, expected,
            "terminal fixture must reach all exact real messages"
        );
        let mut result = result.expect("real fixed-bound pagination must reach a terminal result");
        assert!(argv_mut(&mut result).is_none());
        let before = serde_json::to_value(&result).unwrap();
        normalize_read_continuation_budget(&command, &mut result, 65536).unwrap();
        assert_eq!(serde_json::to_value(&result).unwrap(), before);
        let command = request(search, 2048);
        let mut result = f.raw(&command, &spec).unwrap();
        corrupt(
            argv_mut(&mut result).as_mut().unwrap(),
            Malformation::InconsistentBudget,
        );
        let before = serde_json::to_value(&result).unwrap();
        normalize_read_continuation_budget(&command, &mut result, 2048).unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            before,
            "E==M must not add new rejection behavior"
        );
    }
}
#[test]
fn q5_malformed_post_refit_continuations_stop_actual_writer_before_output_or_mutation() {
    with_render_now(UtcMillis(0), || {
        for search in [false, true] {
            let f = Fixture::new();
            let max = 2048;
            let command = request(search, max);
            let spec = f.boundary(search, max);
            for case in [
                Malformation::DuplicateBudget,
                Malformation::DanglingBudget,
                Malformation::NonNumericBudget,
                Malformation::InconsistentBudget,
                Malformation::MissingBudget,
                Malformation::ConflictingState,
                Malformation::ConflictingHost,
                Malformation::EmptyState,
                Malformation::LongHost,
                Malformation::NulState,
                Malformation::InvalidThreadValue,
                Malformation::IncompleteCooperative,
                Malformation::UnknownCooperativeHarness,
            ] {
                if !search && matches!(case, Malformation::InvalidThreadValue) {
                    continue;
                }
                f.calls.lock().unwrap().clear();
                *f.malformed.lock().unwrap() = Some((max, case));
                let mut out = vec![];
                let result = run_wire_result(command.clone(), &spec, &f, &budget, &mut out);
                assert!(
                    matches!(result,Err(RunError::Api(ref e)) if e.code==ErrorCode::StoreCorrupt),
                    "search={search} corruption={case:?} result={result:?}"
                );
                assert!(out.is_empty(), "corrupt refitted argv produced output");
                let calls = f.calls.lock().unwrap();
                assert!(
                    calls
                        .iter()
                        .any(|c| matches!(c, Command::History(_) | Command::Search(_))
                            && bound(c) < max),
                    "corruption never reached reduced request"
                );
                assert!(
                    calls.iter().all(|c| matches!(
                        c,
                        Command::History(_) | Command::Search(_) | Command::MessageDeliveryModes(_)
                    )),
                    "malformed response made a display mutation/Advance/ACK"
                );
            }
        }
    });
}
#[test]
fn q5_normalizer_route_thresholds_and_identical_duplicate_values_preserve_existing_grammar() {
    let f = Fixture::new();
    let command = request(false, 2048);
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..OutputSpec::default()
    };
    for value in [
        "route".to_owned(),
        "--".to_owned(),
        "--max-bytes".to_owned(),
        "x".repeat(1024),
    ] {
        let mut result = f.raw(&command, &spec).unwrap();
        let args = argv_mut(&mut result).as_mut().unwrap();
        args.splice(
            1..1,
            [
                "--state-dir".into(),
                value.clone(),
                format!("--state-dir={value}"),
                "--host-endpoint".into(),
                value.clone(),
                format!("--host-endpoint={value}"),
            ],
        );
        let mut expected = args.clone();
        let budget_value = expected.iter().position(|a| a == "--cursor").unwrap() + 1;
        // Locate by the known generated suffix, avoiding flag-like route data.
        let bound_value = expected.len() - 1;
        assert_eq!(expected[bound_value - 1], "--max-bytes");
        expected[bound_value] = "4096".into();
        let opaque = expected[budget_value].clone();
        normalize_read_continuation_budget(&command, &mut result, 4096).unwrap();
        assert_eq!(argv_mut(&mut result).as_ref().unwrap(), &expected);
        assert_eq!(
            argv_mut(&mut result).as_ref().unwrap()[budget_value],
            opaque
        );
    }
}
#[test]
fn q5_post_refit_empty_lazy_modes_still_guard_actual_final_bytes() {
    let mut modes = ReadModes::default();
    assert!(modes.is_empty());
    modes.require_final_bound();
    let mut out = vec![];
    let error = modes.write(vec![b'x'; 257], 256, &mut out);
    assert!(
        matches!(error,Err(super::output::OutputError::Api(e)) if e.code==ErrorCode::InvalidBudget)
    );
    assert!(out.is_empty(), "final guard ran after output");
    let mut exact = vec![];
    modes.write(vec![b'x'; 256], 256, &mut exact).unwrap();
    assert_eq!(exact, vec![b'x'; 256]);
    let mut legacy = vec![];
    ReadModes::default()
        .write(vec![b'x'; 257], 256, &mut legacy)
        .unwrap();
    assert_eq!(
        legacy.len(),
        257,
        "never-refit/no-mode legacy output policy changed"
    );
}

#[test]
fn q5_normalizer_mismatched_unrelated_result_kind_is_unchanged() {
    let f = Fixture::new();
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..OutputSpec::default()
    };
    let mut result = f.raw(&request(true, 2048), &spec).unwrap();
    let before = serde_json::to_value(&result).unwrap();
    normalize_read_continuation_budget(&request(false, 2048), &mut result, 4096).unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        before,
        "do not add a global result-schema rejection"
    );
}
#[test]
fn q5_normalizer_actual_complete_message_has_no_continuation_to_rewrite() {
    use crate::protocol::{
        commands::{BodyReadRequest, MessageQuery},
        ids::MessageId,
        results::MessageContent,
    };
    let f = Fixture::new();
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..OutputSpec::default()
    };
    let command = Command::Message(MessageQuery {
        message: MessageId::new("q5-msg-1"),
        body: BodyReadRequest {
            cursor: None,
            offset: None,
            max_bytes: 2048,
        },
    });
    let mut result = f.raw(&command, &spec).unwrap();
    assert!(
        matches!(&result,CommandResult::Message(detail) if matches!(&detail.content,MessageContent::Ordinary {body_offset:0,body_complete:true,body_next_cursor:None,body_next_argv:None,..}))
    );
    let before = serde_json::to_value(&result).unwrap();
    normalize_read_continuation_budget(&command, &mut result, 4096).unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), before);
}

#[test]
fn q5_normalizer_actual_partial_utf8_body_restores_only_continuation_budget() {
    use crate::protocol::{
        commands::{BodyReadRequest, MessageQuery},
        ids::MessageId,
        results::MessageContent,
    };
    let f = Fixture::with_long_body(true);
    let spec = OutputSpec {
        format: OutputFormat::Text,
        ..OutputSpec::default()
    };
    let command = Command::Message(MessageQuery {
        message: MessageId::new("q5-msg-1"),
        body: BodyReadRequest {
            cursor: None,
            offset: None,
            max_bytes: 1536,
        },
    });
    let mut result = f.raw(&command, &spec).unwrap();
    assert!(
        matches!(&result,CommandResult::Message(detail) if matches!(&detail.content,MessageContent::Ordinary {body_total_bytes:65536,body_offset:0,body_complete:false,body_next_cursor:Some(_),body_next_argv:Some(_),..}))
    );
    let before = result.clone();
    let mut expected = argv_mut(&mut result).clone().unwrap();
    let last = expected.len() - 1;
    assert_eq!(expected[last - 1], "--max-bytes");
    assert_eq!(expected[last], "1536");
    expected[last] = "2048".into();
    normalize_read_continuation_budget(&command, &mut result, 2048).unwrap();
    assert_eq!(argv_mut(&mut result).as_ref().unwrap(), &expected);
    assert_eq!(
        serde_json::to_value(stripped(result)).unwrap(),
        serde_json::to_value(stripped(before)).unwrap(),
        "body bytes/offset/completion/opaque cursor changed"
    );
}
