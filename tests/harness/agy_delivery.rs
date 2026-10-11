//! agy hook-pull delivery (ht-poh.5): the bounded transcript confirm scan, byte-locked output
//! shapes, the per-conversation state file, the downgrade record and its recovery paths, the
//! executable identity, the doctor lines and a CI-run daemon round trip (no agy binary).
//! Included from `src/harness/agy/delivery.rs`; fixtures are `tests/fixtures/agy/delivery/*`.
use super::*;
use crate::{
    harness::{
        adapter::{
            DecodedEvent, DoctorTextForm, HarnessAdapter, HookInput, ResolvedSetupScope,
            SetupEnvironment, SetupRequest,
        },
        agy::{AgyAdapter, AgyAdmission, AgyRoots, decode::decode, witness::tests::Temp},
        registry::builtins,
    },
    protocol::{
        authority::{CallerRole, Harness},
        delivery::{DeliveryConfirmItem, DeliveryConfirmReport, DeliveryPage},
        ids::{ExecutionId, HostTargetId, NativeSessionId, SeatId},
        results::{ApiError, ErrorCode},
        time::{CallBudget, MonoInstant},
    },
};
use std::{cell::RefCell, collections::VecDeque, time::Duration};

const CONV: &str = "5299b3e7-20e3-4970-aa8d-477d46a420c6";
const HOUR: u64 = 60 * 60 * 1000;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/agy/delivery")
        .join(name)
}
fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap()
}
fn wanted<'a>(ids: &[&'a str]) -> BTreeSet<&'a str> {
    ids.iter().copied().collect()
}
fn ident(name: &str) -> Identity {
    Identity {
        path: format!("/opt/{name}/agy"),
        inode: 7,
        size: 100,
        mtime_ns: 5,
    }
}
fn runtime(now: u64, identity: &Identity) -> Runtime {
    let identity = identity.clone();
    Runtime::new(now, Box::new(move || identity.clone()))
}
fn claim(generation: u64) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness: Harness::Agent(builtins().agent("agy").unwrap()),
        native_session: NativeSessionId::new(CONV),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}

// --- the transcript scan ----------------------------------------------------------------------

#[test]
fn scan_finds_the_frame_ids_after_the_offset() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::copy(fixture("transcript-frame.jsonl"), &path).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(&["msg-a1", "msg-lz1", "msg-absent"]));
    assert!(scan.readable && scan.complete);
    assert_eq!(
        scan.found,
        ["msg-a1", "msg-lz1"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
}

#[test]
fn scan_ignores_frames_before_the_recorded_offset() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::copy(fixture("transcript-frame.jsonl"), &path).unwrap();
    let len = std::fs::metadata(&path).unwrap().len();
    let scan = scan_transcript(&path, len, &wanted(&["msg-a1"]));
    assert!(scan.readable && scan.complete);
    assert!(
        scan.found.is_empty(),
        "a frame before the offset is not this injection's"
    );
}

#[test]
fn a_shorter_transcript_than_the_offset_is_scanned_from_its_start() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::copy(fixture("transcript-frame.jsonl"), &path).unwrap();
    let scan = scan_transcript(&path, 10_000_000, &wanted(&["msg-a1"]));
    assert!(scan.found.contains("msg-a1"));
}

#[test]
fn a_frame_pushed_past_a_fixed_tail_by_tool_output_is_still_confirmed() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    let mut bytes = fixture_bytes("transcript-frame.jsonl");
    let line = fixture_bytes("tool-output-line.jsonl");
    // 4 MiB of tool output after the frame: far past any fixed tail, inside the window.
    while bytes.len() < 4 * 1024 * 1024 {
        bytes.extend_from_slice(&line);
    }
    std::fs::write(&path, &bytes).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(&["msg-a1"]));
    assert!(scan.readable && scan.complete);
    assert!(scan.found.contains("msg-a1"));
}

#[test]
fn the_scan_is_bounded_at_8_mib_and_past_the_bound_is_non_evidence() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    let line = fixture_bytes("tool-output-line.jsonl");
    let filler = line.repeat((TRANSCRIPT_WINDOW as usize / line.len()) + 8);
    assert!(filler.len() as u64 > TRANSCRIPT_WINDOW);
    // The frame sits beyond the window: not found, and the scan says it proved nothing.
    let mut beyond = filler.clone();
    beyond.extend_from_slice(&fixture_bytes("transcript-frame.jsonl"));
    std::fs::write(&path, &beyond).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(&["msg-a1"]));
    assert!(scan.readable);
    assert!(!scan.complete, "more than 8 MiB follows the offset");
    assert!(scan.found.is_empty());
    // A frame inside the window is still found in an over-long transcript.
    let mut inside = fixture_bytes("transcript-frame.jsonl");
    inside.extend_from_slice(&filler);
    std::fs::write(&path, &inside).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(&["msg-a1"]));
    assert!(!scan.complete);
    assert!(scan.found.contains("msg-a1"));
}

#[test]
fn a_header_quoted_inside_a_body_is_not_a_frame_and_an_id_prefix_is_not_the_id() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::copy(fixture("transcript-forged-body.jsonl"), &path).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(&["msg-a1", "msg-forged", "msg-a"]));
    assert_eq!(scan.found, ["msg-a1".to_owned()].into_iter().collect());
}

/// Scan a fixture from offset 0 for `ids`; the scan must have read all of it.
fn scan_fixture(name: &str, ids: &[&str]) -> BTreeSet<String> {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::copy(fixture(name), &path).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(ids));
    assert!(scan.readable && scan.complete, "{name}");
    scan.found
}
fn ids_of(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn a_forged_header_after_a_literal_backslash_n_in_a_body_does_not_confirm() {
    let found = scan_fixture(
        "transcript-forged-escaped-newline.jsonl",
        &["msg-a1", "msg-forged"],
    );
    assert_eq!(found, ids_of(&["msg-a1"]));
}

#[test]
fn a_header_in_a_tool_result_does_not_confirm() {
    let found = scan_fixture(
        "transcript-header-in-tool-output.jsonl",
        &["msg-a1", "msg-tool"],
    );
    assert_eq!(found, ids_of(&["msg-a1"]));
}

#[test]
fn a_header_in_model_output_does_not_confirm() {
    let found = scan_fixture(
        "transcript-header-in-model-output.jsonl",
        &["msg-a1", "msg-model"],
    );
    assert_eq!(found, ids_of(&["msg-a1"]));
}

#[test]
fn a_header_in_a_step_with_a_user_source_does_not_confirm() {
    let found = scan_fixture("transcript-header-in-user-input.jsonl", &["msg-user"]);
    assert!(found.is_empty(), "{found:?}");
}

/// The frame ids of a genuine capture, read independently of the scan: the `SYSTEM_SDK` record's
/// decoded text, split on real newlines, lines that start at column 0 with a frame header.
fn genuine_ids(name: &str) -> BTreeSet<String> {
    let text = String::from_utf8(fixture_bytes(name)).unwrap();
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        if record["source"] != "SYSTEM_SDK" {
            continue;
        }
        for row in record["content"].as_str().unwrap().split('\n') {
            if let Some(rest) = row
                .strip_prefix("[herdr-threads] message ")
                .or_else(|| row.strip_prefix("[herdr-threads] lazy "))
            {
                ids.insert(rest.split(' ').next().unwrap().to_owned());
            }
        }
    }
    ids
}

fn genuine_capture_confirms(name: &str) {
    let expected = genuine_ids(name);
    assert!(!expected.is_empty(), "{name} holds no frame");
    let wanted_ids: Vec<&str> = expected.iter().map(String::as_str).collect();
    assert_eq!(scan_fixture(name, &wanted_ids), expected);
}

#[test]
fn a_genuine_system_message_frame_confirms() {
    genuine_capture_confirms("transcript-genuine-system-message.jsonl");
}

#[test]
fn a_genuine_user_message_frame_confirms() {
    genuine_capture_confirms("transcript-genuine-user-message.jsonl");
}

#[test]
fn a_genuine_post_tool_call_frame_confirms() {
    genuine_capture_confirms("transcript-genuine-post-tool-call.jsonl");
}

#[test]
fn malformed_and_truncated_lines_are_skipped() {
    let found = scan_fixture("transcript-malformed-lines.jsonl", &["msg-a1", "msg-trunc"]);
    assert_eq!(found, ids_of(&["msg-a1"]));
}

#[test]
fn a_window_that_starts_inside_a_record_skips_that_record() {
    let first = fixture_bytes("transcript-frame.jsonl");
    let second = String::from_utf8(first.clone())
        .unwrap()
        .replace("msg-a1", "msg-b1")
        .replace("msg-lz1", "msg-lz2")
        .into_bytes();
    let mut both = first.clone();
    both.extend_from_slice(&second);
    // The offset of the second copy's injected record: the start of its second line.
    let second_start = first.len();
    let injected_start = first.iter().position(|b| *b == b'\n').unwrap() + 1;
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::write(&path, &both).unwrap();
    let all = wanted(&["msg-a1", "msg-lz1", "msg-b1", "msg-lz2"]);
    let from = |offset: usize| scan_transcript(&path, offset as u64, &all).found;
    // Offset 0 reads everything.
    assert_eq!(from(0), ids_of(&["msg-a1", "msg-lz1", "msg-b1", "msg-lz2"]));
    // One byte into the first copy's injected record: that record is partial and skipped.
    assert_eq!(from(injected_start + 1), ids_of(&["msg-b1", "msg-lz2"]));
    // Exactly at the start of the second copy.
    assert_eq!(from(second_start), ids_of(&["msg-b1", "msg-lz2"]));
    // Exactly at the start of the first copy's injected record: a whole record.
    assert_eq!(
        from(injected_start),
        ids_of(&["msg-a1", "msg-lz1", "msg-b1", "msg-lz2"])
    );
}

#[test]
fn unparseable_missing_and_non_file_transcripts_find_nothing() {
    let temp = Temp::new();
    let path = temp.0.join("t.jsonl");
    std::fs::copy(fixture("transcript-truncated.jsonl"), &path).unwrap();
    let scan = scan_transcript(&path, 0, &wanted(&["msg-a1"]));
    assert!(scan.readable && scan.found.is_empty());
    let missing = scan_transcript(&temp.0.join("none.jsonl"), 0, &wanted(&["msg-a1"]));
    assert!(!missing.readable && !missing.complete);
    let directory = scan_transcript(&temp.0, 0, &wanted(&["msg-a1"]));
    assert!(!directory.readable);
}

// --- a rig that runs the step against a scripted daemon ----------------------------------------

#[derive(Default)]
struct Fake {
    replies: RefCell<VecDeque<Result<CommandResult, ApiError>>>,
    calls: RefCell<Vec<Command>>,
}
impl Fake {
    fn with(replies: Vec<Result<CommandResult, ApiError>>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            calls: RefCell::default(),
        }
    }
    fn calls(&self) -> Vec<Command> {
        self.calls.borrow().clone()
    }
    fn confirmed(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Command::DeliveryConfirm(c) => {
                    Some(c.ids.iter().map(|id| id.as_str().to_owned()).collect())
                }
                _ => None,
            })
            .collect()
    }
}
impl HookStepDaemon for Fake {
    fn call(&self, command: Command) -> Result<CommandResult, ApiError> {
        self.calls.borrow_mut().push(command);
        self.replies.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(ApiError::new(
                ErrorCode::InvalidRequest,
                "no scripted reply",
            ))
        })
    }
}
fn page(ids: &[&str], text: &str, version: u64) -> Result<CommandResult, ApiError> {
    Ok(CommandResult::DeliveryPulled(DeliveryPullReply::Page(
        DeliveryPage {
            attention_version: version,
            text: text.to_owned(),
            offered: ids.iter().map(|id| MessageId::new(*id)).collect(),
            truncated: vec![],
            more: false,
        },
    )))
}
fn pulled(reply: DeliveryPullReply) -> Result<CommandResult, ApiError> {
    Ok(CommandResult::DeliveryPulled(reply))
}
fn confirmed(ids: &[&str]) -> Result<CommandResult, ApiError> {
    Ok(CommandResult::DeliveryConfirmed(DeliveryConfirmReport {
        results: ids
            .iter()
            .map(|id| DeliveryConfirmItem {
                id: MessageId::new(*id),
                result: DeliveryConfirmOutcome::Settled,
                reason: None,
            })
            .collect(),
    }))
}

struct Rig {
    _temp: Temp,
    state: PathBuf,
    transcript: PathBuf,
}
impl Rig {
    fn new() -> Self {
        let temp = Temp::new();
        let state = temp.0.join("state");
        let transcript = temp.0.join("transcript.jsonl");
        std::fs::copy(fixture("transcript-no-frame.jsonl"), &transcript).unwrap();
        Self {
            _temp: temp,
            state,
            transcript,
        }
    }
    fn state_file(&self) -> PathBuf {
        paths::conversation_state(&self.state, CONV)
    }
    fn conversation(&self) -> ConversationState {
        serde_json::from_slice(&std::fs::read(self.state_file()).unwrap()).unwrap()
    }
    fn record(&self) -> Option<DowngradeRecord> {
        load_downgrade(&self.state)
    }
    fn append(&self, bytes: &[u8]) {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
    fn run(&self, event: &str, payload: Value, fake: Option<&Fake>, rt: &Runtime) -> Vec<u8> {
        let mut payload = payload;
        payload["conversationId"] = json!(CONV);
        payload["transcriptPath"] = json!(self.transcript.to_str().unwrap());
        let input = HookInput {
            bytes: serde_json::to_vec(&payload).unwrap(),
            registered_event: Some(event.to_owned()),
        };
        let decoded: DecodedEvent = decode(
            &AgyAdmission {
                step: InjectionStep::SystemMessage,
            },
            &input,
        )
        .unwrap();
        run_step(
            &HookStepContext {
                input: &input,
                decoded: &decoded,
                claim: Some(claim(0)),
                daemon: fake.map(|fake| fake as &dyn HookStepDaemon),
                state_dir: Some(self.state.clone()),
                command_prefix: vec!["herdr-threads".into()],
                deadline: Instant::now() + Duration::from_secs(30),
            },
            rt,
        )
    }
    fn pre(&self, invocation: u64, fake: &Fake, rt: &Runtime) -> Vec<u8> {
        self.run(
            "PreInvocation",
            json!({"invocationNum": invocation, "initialNumSteps": 1}),
            Some(fake),
            rt,
        )
    }
    fn stop(&self, payload_fixture: &str, fake: &Fake, rt: &Runtime) -> Vec<u8> {
        let mut payload: Value = serde_json::from_slice(&fixture_bytes(payload_fixture)).unwrap();
        payload.as_object_mut().unwrap().remove("transcriptPath");
        self.run("Stop", payload, Some(fake), rt)
    }
}
const TEXT: &str = "[herdr-threads] Messages follow.\n\n[herdr-threads] message msg-a1 in t1 from s2:\n  hello \"q\"";
/// A fake with one scripted page for `msg-a1` (the first pull of a turn).
fn offering() -> Fake {
    Fake::with(vec![page(&["msg-a1"], TEXT, 11)])
}
fn frame_line() -> Vec<u8> {
    fixture_bytes("transcript-frame.jsonl")
}

// --- output shapes -----------------------------------------------------------------------------

#[test]
fn output_shapes_are_byte_locked() {
    let text = std::fs::read_to_string(fixture("output-text.txt")).unwrap();
    assert_eq!(text, TEXT);
    let render = |step: InjectionStep| serde_json::to_vec(&step.render(&text)).unwrap();
    assert_eq!(
        render(InjectionStep::SystemMessage),
        fixture_bytes("output-system-message.json")
    );
    assert_eq!(
        render(InjectionStep::UserMessage),
        fixture_bytes("output-user-message.json")
    );
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    let out = rig.pre(0, &offering(), &rt);
    assert_eq!(out, fixture_bytes("output-system-message.json"));
}

#[test]
fn nothing_is_printed_for_unchanged_refusals_and_daemon_errors() {
    let rt = runtime(1000, &ident("a"));
    for reply in [
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 3,
        }),
        pulled(DeliveryPullReply::Refused {
            reason: DeliveryRefusalReason::NoBinding,
        }),
        pulled(DeliveryPullReply::Refused {
            reason: DeliveryRefusalReason::Held,
        }),
        Err(ApiError::new(ErrorCode::InvalidRequest, "down")),
        Ok(CommandResult::DeliveryConfirmed(DeliveryConfirmReport {
            results: vec![],
        })),
    ] {
        let rig = Rig::new();
        let out = rig.pre(0, &Fake::with(vec![reply]), &rt);
        assert!(out.is_empty());
    }
    // No daemon, no state dir, an undecodable payload: also nothing.
    let rig = Rig::new();
    assert!(
        rig.run("PreInvocation", json!({"invocationNum": 0}), None, &rt)
            .is_empty()
    );
}

// --- the per-conversation state file ----------------------------------------------------------

#[test]
fn the_state_file_is_created_after_a_pull_and_deleted_on_session_mismatch() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    // A refused pull creates nothing.
    rig.pre(
        0,
        &Fake::with(vec![pulled(DeliveryPullReply::Refused {
            reason: DeliveryRefusalReason::NoBinding,
        })]),
        &rt,
    );
    assert!(!rig.state_file().exists());
    // A page creates it: mode 0600, the recorded shape, the offset taken at injection.
    let initial_len = std::fs::metadata(&rig.transcript).unwrap().len();
    rig.pre(0, &offering(), &rt);
    let mode = std::fs::metadata(rig.state_file()).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let raw: Value = serde_json::from_slice(&std::fs::read(rig.state_file()).unwrap()).unwrap();
    assert_eq!(
        raw,
        json!({"v": 1, "seen_attention_version": 11,
               "injected": [{"id": "msg-a1", "offset": initial_len, "turn": 1}],
               "turn": 1, "injected_this_turn": true, "generation": 0})
    );
    // A non-mismatch refusal leaves it; session_mismatch deletes it.
    rig.pre(
        1,
        &Fake::with(vec![pulled(DeliveryPullReply::Refused {
            reason: DeliveryRefusalReason::Busy,
        })]),
        &rt,
    );
    assert!(rig.state_file().exists());
    rig.pre(
        1,
        &Fake::with(vec![pulled(DeliveryPullReply::Refused {
            reason: DeliveryRefusalReason::SessionMismatch,
        })]),
        &rt,
    );
    assert!(!rig.state_file().exists());
}

#[test]
fn a_no_binding_refusal_keeps_the_state_file() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    rig.pre(0, &offering(), &rt);
    let before = std::fs::read(rig.state_file()).unwrap();
    rig.pre(
        1,
        &Fake::with(vec![pulled(DeliveryPullReply::Refused {
            reason: DeliveryRefusalReason::NoBinding,
        })]),
        &rt,
    );
    assert_eq!(std::fs::read(rig.state_file()).unwrap(), before);
}

#[test]
fn an_unchanged_reply_caches_the_attention_version_and_the_next_pull_sends_it() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    let fake = Fake::with(vec![
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 42,
        }),
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 42,
        }),
    ]);
    assert!(rig.pre(0, &fake, &rt).is_empty());
    assert_eq!(rig.conversation().seen_attention_version, Some(42));
    assert!(rig.pre(1, &fake, &rt).is_empty());
    let seen: Vec<Option<u64>> = fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Command::DeliveryPull(pull) => {
                assert_eq!(pull.marker_prefix, vec!["herdr-threads".to_owned()]);
                Some(pull.seen_attention_version)
            }
            _ => None,
        })
        .collect();
    assert_eq!(seen, vec![None, Some(42)]);
}

#[test]
fn the_state_file_never_exceeds_4_kib() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    let ids: Vec<String> = (0..100)
        .map(|n| format!("msg-{}-{n}", uuid::Uuid::new_v4()))
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    rig.pre(0, &Fake::with(vec![page(&refs, TEXT, 1)]), &rt);
    let len = std::fs::metadata(rig.state_file()).unwrap().len();
    assert!(len as usize <= MAX_STATE_BYTES, "{len}");
    let state = rig.conversation();
    assert_eq!(state.injected.len(), MAX_TRACKED);
    assert_eq!(state.injected.last().unwrap().id, *ids.last().unwrap());
}

// Kills: a tracking bound below one full page, which would show the oldest ids again after 60 s.
#[test]
fn a_full_page_of_offered_ids_is_tracked() {
    use crate::{
        protocol::ids::{generate_public_id, prefix},
        service::delivery_pull::PAGE_MAX_BLOCKS,
    };
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    let ids: Vec<String> = (0..PAGE_MAX_BLOCKS)
        .map(|_| generate_public_id(prefix::MESSAGE))
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    rig.pre(0, &Fake::with(vec![page(&refs, TEXT, 1)]), &rt);
    let state = rig.conversation();
    assert_eq!(state.injected.len(), PAGE_MAX_BLOCKS);
    assert_eq!(state.injected.first().unwrap().id, ids[0]);
    let len = std::fs::metadata(rig.state_file()).unwrap().len();
    assert!(len as usize <= MAX_STATE_BYTES, "{len}");
}

#[test]
fn a_new_binding_generation_forgets_the_previous_generations_ids() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    rig.pre(0, &offering(), &rt);
    assert_eq!(rig.conversation().injected.len(), 1);
    // Same step, a claim of generation 1 (rebuilt context).
    let mut payload = json!({"invocationNum": 1, "initialNumSteps": 1});
    payload["conversationId"] = json!(CONV);
    payload["transcriptPath"] = json!(rig.transcript.to_str().unwrap());
    let input = HookInput {
        bytes: serde_json::to_vec(&payload).unwrap(),
        registered_event: Some("PreInvocation".into()),
    };
    let decoded = decode(
        &AgyAdmission {
            step: InjectionStep::SystemMessage,
        },
        &input,
    )
    .unwrap();
    let fake = Fake::with(vec![pulled(DeliveryPullReply::Unchanged {
        attention_version: 5,
    })]);
    run_step(
        &HookStepContext {
            input: &input,
            decoded: &decoded,
            claim: Some(claim(1)),
            daemon: Some(&fake),
            state_dir: Some(rig.state.clone()),
            command_prefix: vec![],
            deadline: Instant::now() + Duration::from_secs(30),
        },
        &rt,
    );
    let state = rig.conversation();
    assert!(state.injected.is_empty());
    assert_eq!(state.generation, Some(1));
    assert!(fake.confirmed().is_empty());
}

// --- confirm at PreInvocation and Stop ---------------------------------------------------------

#[test]
fn the_next_pre_invocation_confirms_the_frame_it_finds_after_the_offset() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    rig.pre(0, &offering(), &rt);
    // agy writes the injected message after the recorded offset.
    rig.append(&frame_line());
    let fake = Fake::with(vec![
        confirmed(&["msg-a1"]),
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 12,
        }),
    ]);
    assert!(rig.pre(1, &fake, &rt).is_empty());
    assert_eq!(fake.confirmed(), vec![vec!["msg-a1".to_owned()]]);
    assert!(rig.conversation().injected.is_empty());
}

#[test]
fn a_retryable_confirm_keeps_the_id_for_the_next_attempt() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    rig.pre(0, &offering(), &rt);
    rig.append(&frame_line());
    let retry = Ok(CommandResult::DeliveryConfirmed(DeliveryConfirmReport {
        results: vec![DeliveryConfirmItem {
            id: MessageId::new("msg-a1"),
            result: DeliveryConfirmOutcome::Retryable,
            reason: None,
        }],
    }));
    let fake = Fake::with(vec![
        retry,
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 12,
        }),
    ]);
    rig.pre(1, &fake, &rt);
    assert_eq!(rig.conversation().injected.len(), 1);
}

#[test]
fn stop_confirms_a_frame_in_the_turns_last_call_and_records_no_miss() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    rig.pre(0, &offering(), &rt);
    rig.append(&frame_line());
    let fake = Fake::with(vec![confirmed(&["msg-a1"])]);
    rig.stop("stop-no-tool-call.json", &fake, &rt);
    assert_eq!(fake.confirmed(), vec![vec!["msg-a1".to_owned()]]);
    assert!(rig.conversation().injected.is_empty());
    assert!(rig.record().is_none());
}

// --- downgrade decisions ----------------------------------------------------------------------

/// One turn that injects `msg-a1` and ends with `stop_fixture` while the frame never reaches
/// the transcript.
fn missing_turn(rig: &Rig, stop_fixture: &str, rt: &Runtime) {
    rig.pre(0, &offering(), rt);
    rig.stop(stop_fixture, &Fake::default(), rt);
}

#[test]
fn a_single_injection_miss_does_not_downgrade() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    missing_turn(&rig, "stop-no-tool-call.json", &rt);
    let record = rig.record().unwrap();
    assert_eq!(record.misses, vec![1]);
    assert_eq!(record.downgraded_at, None);
    assert_eq!(record.step(1000), InjectionStep::SystemMessage);
}

#[test]
fn misses_on_the_same_turn_twice_do_not_downgrade() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    missing_turn(&rig, "stop-no-tool-call.json", &rt);
    rig.stop("stop-model-stop.json", &Fake::default(), &rt);
    let record = rig.record().unwrap();
    assert_eq!(record.misses, vec![1]);
    assert_eq!(record.downgraded_at, None);
}

#[test]
fn misses_on_two_distinct_turns_downgrade_to_user_message() {
    let rig = Rig::new();
    let rt = runtime(1000, &ident("a"));
    missing_turn(&rig, "stop-no-tool-call.json", &rt);
    missing_turn(&rig, "stop-model-stop.json", &rt);
    let record = rig.record().unwrap();
    assert_eq!(record.downgraded_at, Some(1000));
    assert_eq!(record.reason.as_deref(), Some("misses"));
    assert!(!record.probation);
    assert_eq!(record.identity, ident("a"));
    // The next page is a userMessage, and its ids carry no systemMessage evidence.
    let out = rig.pre(0, &offering(), &rt);
    assert_eq!(out, fixture_bytes("output-user-message.json"));
    assert!(rig.conversation().injected.iter().all(|entry| entry.user));
    // A userMessage turn that goes missing changes nothing.
    rig.stop("stop-no-tool-call.json", &Fake::default(), &rt);
    assert_eq!(rig.record().unwrap().downgraded_at, Some(1000));
}

#[test]
fn an_abort_style_stop_error_downgrades_at_once() {
    for stop in [
        "stop-error-failed-to-inject.json",
        "stop-error-unknown-step.json",
    ] {
        let rig = Rig::new();
        let rt = runtime(2000, &ident("a"));
        missing_turn(&rig, stop, &rt);
        let record = rig.record().unwrap();
        assert_eq!(record.reason.as_deref(), Some("abort"), "{stop}");
        assert_eq!(record.downgraded_at, Some(2000));
    }
    // It needs no state file: a SessionStart injection can be the one that aborted.
    let rig = Rig::new();
    rig.stop(
        "stop-error-unknown-step.json",
        &Fake::default(),
        &runtime(5, &ident("a")),
    );
    assert_eq!(rig.record().unwrap().reason.as_deref(), Some("abort"));
}

#[test]
fn unreadable_cancelled_errored_and_oversized_transcripts_are_non_evidence() {
    let rt = runtime(1000, &ident("a"));
    // cancelled turn
    let rig = Rig::new();
    missing_turn(&rig, "stop-cancelled.json", &rt);
    assert!(rig.record().is_none());
    // errored turn that is not an injection abort
    let rig = Rig::new();
    missing_turn(&rig, "stop-error-unrelated.json", &rt);
    assert!(rig.record().is_none());
    // unreadable transcript
    let rig = Rig::new();
    rig.pre(0, &offering(), &rt);
    std::fs::remove_file(&rig.transcript).unwrap();
    rig.stop("stop-no-tool-call.json", &Fake::default(), &rt);
    assert!(rig.record().is_none());
    // more than the window after the offset: the absence proves nothing
    let rig = Rig::new();
    rig.pre(0, &offering(), &rt);
    rig.append(
        &fixture_bytes("tool-output-line.jsonl").repeat(TRANSCRIPT_WINDOW as usize / 70 + 9),
    );
    rig.stop("stop-no-tool-call.json", &Fake::default(), &rt);
    assert!(rig.record().is_none());
    // a turn that injected nothing cannot miss
    let rig = Rig::new();
    rig.pre(
        0,
        &Fake::with(vec![pulled(DeliveryPullReply::Unchanged {
            attention_version: 1,
        })]),
        &rt,
    );
    rig.stop("stop-no-tool-call.json", &Fake::default(), &rt);
    assert!(rig.record().is_none());
}

#[test]
fn a_new_executable_identity_injects_a_system_message_again() {
    let rig = Rig::new();
    missing_turn(
        &rig,
        "stop-error-unknown-step.json",
        &runtime(1000, &ident("a")),
    );
    let out = rig.pre(0, &offering(), &runtime(1000, &ident("a")));
    assert_eq!(out, fixture_bytes("output-user-message.json"));
    let out = rig.pre(0, &offering(), &runtime(1000, &ident("b")));
    assert_eq!(out, fixture_bytes("output-system-message.json"));
    // Its misses start from nothing: identity a's record is not extended.
    rig.stop(
        "stop-no-tool-call.json",
        &Fake::default(),
        &runtime(1000, &ident("b")),
    );
    let record = rig.record().unwrap();
    assert_eq!(record.identity, ident("b"));
    assert_eq!(record.downgraded_at, None);
}

#[test]
fn probation_after_24_hours_is_one_system_message_turn() {
    let rig = Rig::new();
    let a = ident("a");
    missing_turn(&rig, "stop-error-unknown-step.json", &runtime(1000, &a));
    // Before 24 h: still userMessage.
    let early = rig.pre(0, &offering(), &runtime(1000 + 24 * HOUR - 1, &a));
    assert_eq!(early, fixture_bytes("output-user-message.json"));
    assert!(!rig.record().unwrap().probation);
    // At 24 h: one systemMessage retry, marked as probation.
    let late = runtime(1000 + 24 * HOUR, &a);
    let out = rig.pre(0, &offering(), &late);
    assert_eq!(out, fixture_bytes("output-system-message.json"));
    assert!(rig.record().unwrap().probation);
    // A miss re-downgrades from now on.
    rig.stop("stop-no-tool-call.json", &Fake::default(), &late);
    let record = rig.record().unwrap();
    assert_eq!(record.downgraded_at, Some(1000 + 24 * HOUR));
    assert_eq!(record.reason.as_deref(), Some("misses"));
    assert!(!record.probation);
    assert_eq!(
        record.step(1000 + 24 * HOUR + 1),
        InjectionStep::UserMessage
    );
}

#[test]
fn a_confirmed_probation_frame_clears_the_downgrade() {
    let rig = Rig::new();
    let a = ident("a");
    missing_turn(&rig, "stop-error-unknown-step.json", &runtime(1000, &a));
    let late = runtime(1000 + 24 * HOUR, &a);
    rig.pre(0, &offering(), &late);
    assert!(rig.record().unwrap().probation);
    rig.append(&frame_line());
    let fake = Fake::with(vec![confirmed(&["msg-a1"])]);
    rig.stop("stop-no-tool-call.json", &fake, &late);
    assert!(rig.record().is_none(), "the downgrade file is gone");
    assert_eq!(fake.confirmed().len(), 1);
}

/// A downgrade at 1000 and the probation turn opened 24 h later; returns the probation time.
fn open_probation(rig: &Rig) -> u64 {
    let a = ident("a");
    missing_turn(rig, "stop-error-unknown-step.json", &runtime(1000, &a));
    let at = 1000 + 24 * HOUR;
    let out = rig.pre(0, &offering(), &runtime(at, &a));
    assert_eq!(out, fixture_bytes("output-system-message.json"));
    assert!(rig.record().unwrap().probation);
    at
}

#[test]
fn a_probation_confirmed_at_a_later_pre_invocation_clears_the_downgrade() {
    let rig = Rig::new();
    let at = open_probation(&rig);
    rig.append(&frame_line());
    let fake = Fake::with(vec![
        confirmed(&["msg-a1"]),
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 11,
        }),
    ]);
    rig.pre(1, &fake, &runtime(at + 1, &ident("a")));
    assert!(rig.record().is_none(), "the downgrade file is gone");
}

fn an_inconclusive_probation_stop_re_downgrades(stop_fixture: &str) {
    let rig = Rig::new();
    let at = open_probation(&rig);
    let a = ident("a");
    rig.stop(stop_fixture, &Fake::default(), &runtime(at, &a));
    let record = rig.record().unwrap();
    assert_eq!(record.downgraded_at, Some(at), "{stop_fixture}");
    assert!(!record.probation, "{stop_fixture}");
    assert_eq!(record.reason.as_deref(), Some("abort"), "{stop_fixture}");
    assert_eq!(record.step(at + 1), InjectionStep::UserMessage);
    assert_eq!(record.step(at + 24 * HOUR), InjectionStep::SystemMessage);
}

#[test]
fn a_cancelled_probation_turn_re_downgrades_with_a_fresh_timestamp() {
    an_inconclusive_probation_stop_re_downgrades("stop-cancelled.json");
}

#[test]
fn an_errored_probation_turn_re_downgrades_with_a_fresh_timestamp() {
    an_inconclusive_probation_stop_re_downgrades("stop-error-unrelated.json");
}

#[test]
fn an_unreadable_transcript_ends_the_probation_turn() {
    let rig = Rig::new();
    let at = open_probation(&rig);
    std::fs::remove_file(&rig.transcript).unwrap();
    rig.stop(
        "stop-no-tool-call.json",
        &Fake::default(),
        &runtime(at, &ident("a")),
    );
    let record = rig.record().unwrap();
    assert_eq!(record.downgraded_at, Some(at));
    assert!(!record.probation);
}

#[test]
fn a_probation_with_no_stop_re_downgrades_at_the_next_injection_after_24_hours() {
    let rig = Rig::new();
    let at = open_probation(&rig);
    let a = ident("a");
    // Still inside the probation window: the turn continues with systemMessage.
    let out = rig.pre(1, &offering(), &runtime(at + 24 * HOUR - 1, &a));
    assert_eq!(out, fixture_bytes("output-system-message.json"));
    assert_eq!(rig.record().unwrap().downgraded_at, Some(1000));
    // 24 h with no Stop: the probation is over, and the injection is a userMessage.
    let out = rig.pre(1, &offering(), &runtime(at + 24 * HOUR, &a));
    assert_eq!(out, fixture_bytes("output-user-message.json"));
    let record = rig.record().unwrap();
    assert_eq!(record.downgraded_at, Some(at + 24 * HOUR));
    assert!(!record.probation);
}

#[test]
fn a_page_with_no_offered_id_neither_opens_nor_spends_probation() {
    let rig = Rig::new();
    let a = ident("a");
    missing_turn(&rig, "stop-error-unknown-step.json", &runtime(1000, &a));
    let at = 1000 + 24 * HOUR;
    let before = rig.record().unwrap();
    // A notices-only page while the probation is due: userMessage, no probation.
    let notices = Fake::with(vec![page(&[], TEXT, 12)]);
    let out = rig.pre(0, &notices, &runtime(at, &a));
    assert_eq!(out, fixture_bytes("output-user-message.json"));
    let record = rig.record().unwrap();
    assert!(!record.probation, "no probation turn opened");
    assert_eq!(record, before, "the record is unchanged");
    assert!(
        !rig.conversation().injected_this_turn,
        "nothing a confirm could prove was injected"
    );
    // The turn ends: nothing to re-downgrade.
    rig.stop("stop-no-tool-call.json", &Fake::default(), &runtime(at, &a));
    assert_eq!(rig.record().unwrap(), before);
    // A page with an offered id opens probation as before.
    let out = rig.pre(0, &offering(), &runtime(at, &a));
    assert_eq!(out, fixture_bytes("output-system-message.json"));
    assert!(rig.record().unwrap().probation);
}

#[test]
fn a_notices_only_page_renders_a_system_message_when_not_downgraded() {
    let rig = Rig::new();
    let a = ident("a");
    let notices = Fake::with(vec![page(&[], TEXT, 12)]);
    let out = rig.pre(0, &notices, &runtime(1000, &a));
    assert_eq!(out, fixture_bytes("output-system-message.json"));
    assert!(!rig.conversation().injected_this_turn);
}

#[test]
fn setup_clears_the_downgrade_record() {
    let temp = Temp::new();
    let home = temp.0.join("home");
    let state = temp.0.join("state");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let env = SetupEnvironment {
        home: Some(home.clone().into()),
        executable: temp.0.join("bin/herdr-threads"),
        state_dir: Some(state.clone()),
        host_endpoint: Some(state.join("host.sock")),
        ..Default::default()
    };
    let dir = setup_state_dir(&env).unwrap();
    let mut record = DowngradeRecord::blank(&ident("a"));
    record.downgrade("abort", 9);
    save_downgrade(&dir, &record);
    assert!(load_downgrade(&dir).is_some());
    AgyAdapter
        .setup(
            &SetupRequest {
                scope: ResolvedSetupScope::ConfigRoot(AgyRoots::from_home(&home).config_root),
                executable: env.executable.clone(),
                environment: env,
                native_binary: None,
                options: [(crate::cli::setup::WITH_PERMISSIONS.to_owned(), true)]
                    .into_iter()
                    .collect(),
            },
            &CallBudget {
                deadline: MonoInstant(u64::MAX / 2),
                cancellation: Default::default(),
            },
        )
        .unwrap();
    assert!(
        load_downgrade(&dir).is_none(),
        "setup agy clears the record"
    );
}

#[test]
fn admission_selects_user_message_only_for_the_current_identity() {
    let temp = Temp::new();
    let dir = temp.0.join("state");
    assert_eq!(
        admission_step_in(Some(&dir), 10),
        InjectionStep::SystemMessage
    );
    assert_eq!(admission_step_in(None, 10), InjectionStep::SystemMessage);
    let mut record = DowngradeRecord::blank(&current_identity());
    record.downgrade("misses", 10);
    save_downgrade(&dir, &record);
    assert_eq!(
        admission_step_in(Some(&dir), 11),
        InjectionStep::UserMessage
    );
    assert_eq!(
        admission_step_in(Some(&dir), 10 + PROBATION_AFTER_MS),
        InjectionStep::SystemMessage
    );
    let mut other = DowngradeRecord::blank(&ident("other"));
    other.downgrade("misses", 10);
    save_downgrade(&dir, &other);
    assert_eq!(
        admission_step_in(Some(&dir), 11),
        InjectionStep::SystemMessage
    );
}

#[test]
fn only_a_hook_command_argv_names_a_state_dir() {
    let args = |list: &[&str]| {
        list.iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
    };
    assert_eq!(argv_state_dir(args(&["cargo-test", "--nocapture"])), None);
    let dir = argv_state_dir(args(&[
        "herdr-threads",
        "--state-dir",
        "/tmp/s",
        "--host-endpoint",
        "/tmp/h.sock",
        "hook",
        "agy",
    ]))
    .unwrap();
    assert_eq!(
        dir,
        adapter_state_dir(Some("/tmp/s".into()), Some("/tmp/h.sock".into())).unwrap()
    );
    assert!(dir.ends_with("adapter-state/agy"));
}

#[test]
fn agy_declares_hook_pull_with_native_idle_wake() {
    let policy = AgyAdapter.delivery_policy().unwrap();
    assert_eq!(policy.mechanism(), DeliveryMechanism::HookPull);
    assert_eq!(policy.provenance(), "cooperative_hook_delivery");
    assert_eq!(policy.idle_wake(), IdleWake::Native);
    assert!(AgyAdapter.hook_step().is_some());
}

// --- executable identity ----------------------------------------------------------------------

fn executable(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
use std::os::unix::fs::PermissionsExt;

#[test]
fn a_symlinked_binary_is_identified_by_its_resolved_target() {
    let temp = Temp::new();
    let root = std::fs::canonicalize(&temp.0).unwrap();
    let real = root.join("versions/1.3.2/agy");
    executable(&real, "#!/bin/sh\n");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let link = root.join("bin/agy");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let identity = resolve_identity(Some(&link), None).unwrap();
    let meta = std::fs::metadata(&real).unwrap();
    assert_eq!(identity.path, real.to_str().unwrap());
    assert_eq!(identity.inode, meta.ino());
    assert_eq!(identity.size, meta.len());
    assert_eq!(
        identity.mtime_ns,
        meta.mtime() * 1_000_000_000 + meta.mtime_nsec()
    );
    // The same identity is found through PATH.
    let by_path = resolve_identity(None, Some(root.join("bin").as_os_str())).unwrap();
    assert_eq!(by_path, identity);
    // A replaced target is a new identity.
    let upgraded = root.join("versions/1.3.3/agy");
    executable(&upgraded, "#!/bin/sh\n# new\n");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&upgraded, &link).unwrap();
    assert_ne!(resolve_identity(Some(&link), None).unwrap(), identity);
}

#[test]
fn a_launcher_parent_falls_back_to_the_path_resolved_agy() {
    let temp = Temp::new();
    let root = std::fs::canonicalize(&temp.0).unwrap();
    let real = root.join("opt/agy");
    executable(&real, "#!/bin/sh\n");
    let launcher = root.join("bin/agy-launcher");
    executable(&launcher, "#!/bin/sh\nexec agy\n");
    std::fs::create_dir_all(root.join("path")).unwrap();
    std::os::unix::fs::symlink(&real, root.join("path/agy")).unwrap();
    let path = root.join("path");
    let from_launcher = resolve_identity(Some(&launcher), Some(path.as_os_str())).unwrap();
    assert_eq!(
        from_launcher.path,
        real.to_str().unwrap(),
        "not the launcher"
    );
    assert_eq!(
        from_launcher,
        resolve_identity(None, Some(path.as_os_str())).unwrap()
    );
    // Neither a parent nor PATH names agy.
    assert_eq!(
        resolve_identity(Some(&launcher), Some(root.join("none").as_os_str())),
        None
    );
    // A parent image that does not exist falls back too.
    assert!(resolve_identity(Some(&root.join("gone/agy")), Some(path.as_os_str())).is_some());
}

// --- doctor -----------------------------------------------------------------------------------

fn doctor_env(temp: &Temp) -> SetupEnvironment {
    let state = temp.0.join("state");
    std::fs::create_dir_all(&state).unwrap();
    SetupEnvironment {
        state_dir: Some(state.clone()),
        host_endpoint: Some(state.join("host.sock")),
        ..Default::default()
    }
}
fn hooks_with(delivery: &Value) -> Value {
    json!({"plugin": {"installed": true, "current": true, "enabled": true, "path": "p", "version": "v"},
           "allow_entry": {"state": "owned", "rule": "r", "file": "f"},
           "delivery": delivery})
}

#[test]
fn doctor_reports_the_downgrade_and_the_never_confirmed_count() {
    let temp = Temp::new();
    let env = doctor_env(&temp);
    assert_eq!(
        doctor_json(&env),
        json!({}),
        "nothing to report on a clean state"
    );
    let dir = setup_state_dir(&env).unwrap();
    let mut record = DowngradeRecord::blank(&ident("a"));
    record.downgrade("abort", 1);
    save_downgrade(&dir, &record);
    // Two ids offered in an earlier turn and never confirmed, one from the running turn.
    let mut state = ConversationState {
        turn: 3,
        generation: Some(4),
        ..Default::default()
    };
    for (id, turn) in [("m1", 1), ("m2", 2), ("m3", 3)] {
        state.injected.push(Injected {
            id: id.into(),
            offset: 0,
            turn,
            user: false,
        });
    }
    state.save(&paths::conversation_state(&dir, CONV));
    let facts = doctor_json(&env);
    assert_eq!(
        facts["never_confirmed"],
        json!({"count": 2, "generation": 4})
    );
    let lines = doctor_lines(&facts);
    assert_eq!(
        lines,
        vec![
            "downgraded to userMessage (abort) for /opt/a/agy".to_owned(),
            "offered, never confirmed (2 in generation 4)".to_owned(),
        ]
    );
    let hooks = hooks_with(&facts);
    let debug = AgyAdapter.doctor_text(&hooks, DoctorTextForm::Debug);
    assert!(
        debug.ends_with(
            "hooks.agy.delivery: downgraded to userMessage (abort) for /opt/a/agy\n\
         hooks.agy.delivery: offered, never confirmed (2 in generation 4)\n"
        ),
        "{debug}"
    );
    let summary = AgyAdapter.doctor_text(&hooks, DoctorTextForm::Summary);
    assert!(
        summary.contains(
            "agy delivery: downgraded to userMessage (abort) for /opt/a/agy\n\
         agy delivery: offered, never confirmed (2 in generation 4)\n"
        ),
        "{summary}"
    );
    // Without delivery facts neither form changes.
    let clean = hooks_with(&Value::Null);
    assert_eq!(
        AgyAdapter.doctor_text(&clean, DoctorTextForm::Summary),
        "agy: plugin installed, allow entry owned\n"
    );
    assert!(
        !AgyAdapter
            .doctor_text(&clean, DoctorTextForm::Debug)
            .contains("delivery")
    );
}

#[test]
fn a_downgrade_of_another_executable_identity_is_not_shown() {
    let facts =
        json!({"downgrade": {"path": "/old/agy", "reason": "misses", "identity_current": false}});
    assert!(doctor_lines(&facts).is_empty());
    let facts =
        json!({"downgrade": {"path": "/old/agy", "reason": "misses", "identity_current": true}});
    assert_eq!(doctor_lines(&facts).len(), 1);
}

// --- the daemon round trip --------------------------------------------------------------------

mod daemon {
    use super::*;
    use crate::{
        ports::{LocalService, StorePort},
        protocol::{
            authority::PeerIdentity,
            commands::{CheckIn, CheckInMode},
            ids::OperationId,
            time::{Clock, UtcMillis},
        },
        service::{dispatch::DomainService, fair_writer::FairWriter},
        store::{SqliteStore, StoreSettings, connection::StoreContext},
    };
    use rusqlite::Connection;
    use std::sync::Arc;

    struct TestClock;
    impl Clock for TestClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1_000_000)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(1000),
            cancellation: Default::default(),
        }
    }
    fn peer() -> PeerIdentity {
        PeerIdentity::from_kernel(501)
    }

    /// Seat `s` (agy) and a thread `t` in which it is joined, with ordinary message `m1` pending.
    struct World {
        service: DomainService,
        db: Connection,
        _temp: Temp,
    }
    fn world() -> World {
        let temp = Temp::new();
        let clock: Arc<dyn Clock> = Arc::new(TestClock);
        let context = StoreContext::new(temp.0.join("store.db"), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,0,0)", []).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-p','inc','coherent_enumeration',1)", []).unwrap();
        db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2)", []).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','joined')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1)", []).unwrap();
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','i','t',1,'ordinary','hello agy',0,3)", []).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m1','t','s','pending',300000)", []).unwrap();
        let store: Arc<dyn StorePort> =
            Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
        let service = DomainService::new("i".into(), store, clock)
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
        World {
            service,
            db,
            _temp: temp,
        }
    }
    impl World {
        fn check_in(&self) -> CallerClaim {
            let claim = claim(1);
            let command = Command::CheckIn(CheckIn {
                mode: CheckInMode::Lifecycle {
                    expected_binding_generation: 1,
                },
                claim,
                operation: OperationId::new("ci-1"),
                lineage_parent: None,
            });
            match self.service.handle(command, peer(), &budget()).unwrap() {
                CommandResult::CheckedIn(result) => result.context,
                other => panic!("unexpected {other:?}"),
            }
        }
        fn receipt(&self) -> (String, String) {
            self.db
                .query_row(
                    "SELECT state, coalesce(ack_observation,'{}') FROM receipts WHERE message_id='m1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap()
        }
    }
    struct Direct<'a>(&'a DomainService, bool);
    impl HookStepDaemon for Direct<'_> {
        fn call(&self, command: Command) -> Result<CommandResult, ApiError> {
            if self.1 {
                return Err(ApiError::new(ErrorCode::InvalidRequest, "daemon down"));
            }
            self.0.handle(command, peer(), &budget())
        }
    }

    #[test]
    fn pull_inject_fixture_transcript_confirm_settles_with_hook_delivery_provenance() {
        let world = world();
        let context = world.check_in();
        let rig = Rig::new();
        let rt = runtime(1000, &ident("a"));
        let run = |event: &str, payload: Value, daemon: &Direct<'_>| {
            let mut payload = payload;
            payload["conversationId"] = json!(CONV);
            payload["transcriptPath"] = json!(rig.transcript.to_str().unwrap());
            let input = HookInput {
                bytes: serde_json::to_vec(&payload).unwrap(),
                registered_event: Some(event.to_owned()),
            };
            let decoded = decode(
                &AgyAdmission {
                    step: InjectionStep::SystemMessage,
                },
                &input,
            )
            .unwrap();
            run_step(
                &HookStepContext {
                    input: &input,
                    decoded: &decoded,
                    claim: Some(context.clone()),
                    daemon: Some(daemon),
                    state_dir: Some(rig.state.clone()),
                    command_prefix: vec!["herdr-threads".into()],
                    deadline: Instant::now() + Duration::from_secs(30),
                },
                &rt,
            )
        };
        let up = Direct(&world.service, false);

        // PreInvocation: pull -> one systemMessage page; the offset is recorded.
        let out = run(
            "PreInvocation",
            json!({"invocationNum": 0, "initialNumSteps": 1}),
            &up,
        );
        let value: Value = serde_json::from_slice(&out).unwrap();
        let text = value["injectSteps"][0]["systemMessage"]["systemMessage"]
            .as_str()
            .unwrap();
        assert!(text.starts_with(crate::delivery::HEADER), "{text}");
        assert!(text.contains("[herdr-threads] message m1 in "), "{text}");
        assert_eq!(value["injectSteps"].as_array().unwrap().len(), 1);
        assert_eq!(rig.conversation().injected[0].id, "m1");
        assert_eq!(world.receipt().0, "pending", "a pull settles nothing");

        // agy writes the injected message into the transcript (fixture shape, real frame text).
        let mut line = serde_json::to_vec(&json!({"step_index": 2, "source": "SYSTEM_SDK", "type": "SYSTEM_MESSAGE",
            "status": "DONE", "content": format!("<SYSTEM_MESSAGE>\nThe following system message was injected by a PreInvocation hook:\n{text}\n</SYSTEM_MESSAGE>")})).unwrap();
        line.push(b'\n');
        rig.append(&line);

        // Stop: confirm -> the receipt is settled by cooperative hook delivery.
        let mut stop: Value =
            serde_json::from_slice(&fixture_bytes("stop-no-tool-call.json")).unwrap();
        stop.as_object_mut().unwrap().remove("transcriptPath");
        assert!(run("Stop", stop, &up).is_empty());
        let (state, observation) = world.receipt();
        assert_eq!(state, "acked");
        let observation: Value = serde_json::from_str(&observation).unwrap();
        assert_eq!(
            observation["action_provenance"],
            "cooperative_hook_delivery"
        );
        assert_eq!(observation["via"], "transcript");
        assert!(rig.conversation().injected.is_empty());
        assert!(rig.record().is_none());

        // The next PreInvocation has no news: one round trip, no output.
        let out = run(
            "PreInvocation",
            json!({"invocationNum": 0, "initialNumSteps": 3}),
            &up,
        );
        assert!(out.is_empty());

        // A daemon error prints nothing and leaves the state alone.
        let before = std::fs::read(rig.state_file()).unwrap();
        let out = run(
            "PreInvocation",
            json!({"invocationNum": 1, "initialNumSteps": 3}),
            &Direct(&world.service, true),
        );
        assert!(out.is_empty());
        assert_eq!(
            rig.conversation().turn,
            serde_json::from_slice::<ConversationState>(&before)
                .unwrap()
                .turn
        );
    }
    /// ht-poh.11 (c): a `/clear` (a new conversation, so a new binding generation) discards the
    /// pull ledger. The offered message is `not_offered` for the new generation, the old claim is
    /// `stale_generation`, nothing settled, and the next pull offers the message afresh.
    #[test]
    fn a_clear_rotation_discards_the_agy_pull_ledger() {
        use crate::protocol::delivery::{
            DeliveryConfirm, DeliveryConfirmOutcome, DeliveryPull, DeliveryPullReply,
        };
        let world = world();
        let first = world.check_in();
        let pull = |claim: &CallerClaim| {
            let command = Command::DeliveryPull(DeliveryPull {
                claim: claim.clone(),
                seen_attention_version: None,
                marker_prefix: vec![],
            });
            match world.service.handle(command, peer(), &budget()).unwrap() {
                CommandResult::DeliveryPulled(reply) => reply,
                other => panic!("unexpected {other:?}"),
            }
        };
        let confirm = |claim: &CallerClaim, operation: &str| {
            let command = Command::DeliveryConfirm(DeliveryConfirm {
                claim: claim.clone(),
                ids: vec![crate::protocol::ids::MessageId::new("m1")],
                operation: crate::protocol::ids::OperationId::new(operation),
            });
            match world.service.handle(command, peer(), &budget()).unwrap() {
                CommandResult::DeliveryConfirmed(report) => {
                    report.results.into_iter().next().unwrap().result
                }
                other => panic!("unexpected {other:?}"),
            }
        };
        let DeliveryPullReply::Page(page) = pull(&first) else {
            panic!("the first pull offers m1");
        };
        assert_eq!(page.offered.len(), 1);

        // `/clear`: the same pane, another conversation.
        let mut next = first.clone();
        next.native_session = NativeSessionId::new("e806a19d-b316-4599-b4c5-a56db668fb4a");
        next.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
        let command = Command::CheckIn(CheckIn {
            mode: CheckInMode::Lifecycle {
                expected_binding_generation: first.binding_generation,
            },
            claim: next,
            operation: crate::protocol::ids::OperationId::new("ci-2"),
            lineage_parent: None,
        });
        let CommandResult::CheckedIn(result) =
            world.service.handle(command, peer(), &budget()).unwrap()
        else {
            panic!("the /clear check-in");
        };
        let second = result.context;
        // Session-keyed bindings: the new conversation is a new binding of the seat; the binding
        // generation is the seat's and does not move.
        assert_ne!(second.native_session, first.native_session);

        assert_eq!(confirm(&second, "op-1"), DeliveryConfirmOutcome::NotOffered);
        assert_eq!(
            confirm(&first, "op-2"),
            DeliveryConfirmOutcome::StaleGeneration
        );
        assert_eq!(world.receipt().0, "pending");
        let DeliveryPullReply::Page(again) = pull(&second) else {
            panic!("the new generation is offered m1 afresh");
        };
        assert_eq!(again.offered, page.offered);
        assert_eq!(confirm(&second, "op-3"), DeliveryConfirmOutcome::Settled);
        assert_eq!(world.receipt().0, "acked");
    }
}

// --- stale tracked ids and old state files (ht-poh.36) ---------------------------------------------

/// `msg-old` is injected, then more than one window of frame-less transcript follows, then
/// `msg-a1` is injected and its frame appended: the stale id must not anchor the scan.
fn rig_with_stale_id_and_new_frame(rt: &Runtime) -> Rig {
    let rig = Rig::new();
    rig.pre(0, &Fake::with(vec![page(&["msg-old"], TEXT, 1)]), rt);
    let line = fixture_bytes("tool-output-line.jsonl");
    rig.append(&line.repeat(TRANSCRIPT_WINDOW as usize / line.len() + 8));
    rig.pre(0, &Fake::with(vec![page(&["msg-a1"], TEXT, 2)]), rt);
    rig.append(&frame_line());
    rig
}

#[test]
fn a_stale_never_found_id_does_not_block_confirming_newer_frames() {
    let rt = runtime(1000, &ident("a"));
    let rig = rig_with_stale_id_and_new_frame(&rt);
    let fake = Fake::with(vec![
        confirmed(&["msg-a1"]),
        pulled(DeliveryPullReply::Unchanged {
            attention_version: 3,
        }),
    ]);
    rig.pre(1, &fake, &rt);
    assert_eq!(fake.confirmed(), vec![vec!["msg-a1".to_owned()]]);
    assert!(rig.conversation().injected.is_empty());
}

#[test]
fn stop_confirms_newer_frames_past_a_stale_id_and_records_no_miss_for_it() {
    let rt = runtime(1000, &ident("a"));
    let rig = rig_with_stale_id_and_new_frame(&rt);
    let fake = Fake::with(vec![confirmed(&["msg-a1"])]);
    rig.stop("stop-no-tool-call.json", &fake, &rt);
    assert_eq!(fake.confirmed(), vec![vec!["msg-a1".to_owned()]]);
    assert!(rig.record().is_none(), "no miss for the dropped msg-old");
}

#[test]
fn an_id_within_one_window_of_the_end_stays_tracked() {
    let rt = runtime(1000, &ident("a"));
    let rig = Rig::new();
    rig.pre(0, &offering(), &rt);
    rig.append(&fixture_bytes("tool-output-line.jsonl"));
    rig.pre(
        1,
        &Fake::with(vec![pulled(DeliveryPullReply::Unchanged {
            attention_version: 12,
        })]),
        &rt,
    );
    let ids: Vec<String> = rig
        .conversation()
        .injected
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, vec!["msg-a1".to_owned()]);
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn aged_file(dir: &Path, name: &str, days: u64) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"{}").unwrap();
    let age = Duration::from_secs(days * 24 * 60 * 60);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - age)
        .unwrap();
    path
}

#[test]
fn state_files_untouched_for_30_days_are_pruned_when_a_new_conversation_starts() {
    let rig = Rig::new();
    let dir = rig.state.join("conversations");
    std::fs::create_dir_all(&dir).unwrap();
    let old = aged_file(&dir, "old.json", 31);
    let fresh = aged_file(&dir, "fresh.json", 1);
    let notes = aged_file(&dir, "notes.txt", 31);
    let link = dir.join("link.json");
    std::os::unix::fs::symlink(&old, &link).unwrap();
    let rt = runtime(now_ms(), &ident("a"));
    rig.pre(0, &offering(), &rt);
    assert!(!old.exists(), "a 31-day-old state file is deleted");
    assert!(fresh.exists());
    assert!(notes.exists(), "only .json files are pruned");
    assert!(link.symlink_metadata().is_ok(), "a symlink is skipped");
    assert!(rig.state_file().exists());
}

#[test]
fn an_existing_conversation_does_not_prune() {
    let rig = Rig::new();
    rig.pre(0, &offering(), &runtime(1000, &ident("a")));
    assert!(rig.state_file().exists());
    let dir = rig.state.join("conversations");
    let old = aged_file(&dir, "old.json", 31);
    let rt = runtime(now_ms(), &ident("a"));
    rig.pre(
        1,
        &Fake::with(vec![pulled(DeliveryPullReply::Unchanged {
            attention_version: 12,
        })]),
        &rt,
    );
    assert!(
        old.exists(),
        "pruning runs only when a file is first created"
    );
}
