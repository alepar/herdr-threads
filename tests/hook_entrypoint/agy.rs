//! agy hooks through the built binary against the in-process daemon `Fixture` (agy-adapter D5):
//! the registered `hook agy --event E` command runs with `HOME` set to a temp home holding one of
//! `tests/fixtures/agy/layouts/*.txt`, so the presence lock and subagent listings decide the role.
//! The seeded seat `seat` on pane `w9:p1` is the pane under test.
use super::*;
use herdr_threads::harness::registry::{OccupantHarness, builtins};
use serde_json::{Value, json};

const TOP: &str = "534e47d6-64b3-4fd6-bd7f-446ceec4d86c";
const CLEAR: &str = "e806a19d-b316-4599-b4c5-a56db668fb4a";
const RESUME: &str = "ccac2ead-d847-45fb-aa4b-9fc7126acdc5";
const PANE: &str = "w9:p1";
const LIFECYCLE_PREFIX: &str = "herdr-threads: this conversation is the pane's top-level session; any earlier herdr-threads subagent notice no longer applies.\n";

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agy")
}
fn payload(name: &str) -> Vec<u8> {
    fs::read(fixtures().join("payloads").join(format!("{name}.json"))).unwrap()
}

/// The layout manifest's files, empty, under `<home>/.gemini/antigravity-cli/`.
fn home_with(fx: &Fixture, manifest: &str) -> PathBuf {
    let home = fx.root.join("agy-home");
    let data = home.join(".gemini/antigravity-cli");
    let text =
        fs::read_to_string(fixtures().join("layouts").join(format!("{manifest}.txt"))).unwrap();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = data.join(line);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap();
    }
    home
}

/// The registered command for `event`, built from the installed argv contract.
fn command(fx: &Fixture, event: &str) -> String {
    let harness: Harness = OccupantHarness::Agent(builtins().agent("agy").unwrap()).into();
    let argv = installed_argv(BIN, Some(fx.state.to_str().unwrap()), None, harness);
    let base = herdr_threads::harness::setup::shell_command(&argv).unwrap();
    format!("{base} '--event' '{event}'")
}

/// One agy hook as the harness runs it: `sh -c <command>`, the payload on stdin, the pane
/// identity and `HOME` in the environment.
fn run(fx: &Fixture, home: &Path, event: &str, name: &str) -> Hook {
    run_payload(fx, home, event, &payload(name))
}
fn run_payload(fx: &Fixture, home: &Path, event: &str, bytes: &[u8]) -> Hook {
    run_payload_with(fx, home, event, bytes, true)
}
/// `scaled: false` runs the hook with the production wall-clock budgets (no
/// `HT_TEST_TIMEOUT_SCALE`), for the test that pins the lifecycle bound itself.
fn run_payload_with(fx: &Fixture, home: &Path, event: &str, bytes: &[u8], scaled: bool) -> Hook {
    let started = Instant::now();
    // A native hook runs under its harness, where it takes its liveness fingerprint from
    // (session-keyed bindings): one long-lived stand-in plays the pane's agy process, so every
    // conversation of the pane shares it, as a `/clear` or resume inside one agy process does.
    let mut line = scrubbed_command("/bin/sh");
    herdr_threads::test_support::isolation::as_agent(&mut line, "agy", "agy-process");
    line.arg("-c")
        .arg(command(fx, event))
        .env("HERDR_ENV", "1")
        .env("HERDR_PANE_ID", PANE)
        .env("HERDR_SOCKET_PATH", &fx.host)
        .env("PATH", harness_path(&fx.host))
        .env("HOME", home)
        .env_remove("ANTIGRAVITY_AGENT")
        .env_remove("HERDR_PLUGIN_STATE_DIR")
        .env_remove("HERDR_BIN_PATH")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !scaled {
        line.env_remove(herdr_threads::protocol::time::TEST_TIMEOUT_SCALE_ENV);
    }
    herdr_threads::test_support::spawn::tag(&mut line);
    let mut child = line.spawn_owned().unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    Hook {
        code: output.status.code(),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        elapsed: started.elapsed(),
        timeout_scale: None,
    }
}

/// The text of `{"injectSteps":[{"systemMessage":{"systemMessage":T}}]}`; any other shape fails.
fn system_message(hook: &Hook) -> String {
    assert_eq!(hook.code, Some(0), "{}", hook.stderr);
    let value = hook.context();
    let object = value.as_object().unwrap();
    assert_eq!(object.len(), 1, "{value}");
    let steps = object["injectSteps"].as_array().unwrap();
    assert_eq!(steps.len(), 1, "{value}");
    let step = steps[0].as_object().unwrap();
    assert_eq!(step.len(), 1, "{value}");
    let message = step["systemMessage"].as_object().unwrap();
    assert_eq!(message.len(), 1, "{value}");
    message["systemMessage"].as_str().unwrap().to_owned()
}

fn generation(fx: &Fixture) -> i64 {
    fx.count("SELECT generation FROM seats WHERE id='seat'")
}
/// The seat's live (open and connected) agy binding for `session`, enrolled as a cooperative
/// top-level claim.
fn live_binding(fx: &Fixture, session: &str) -> i64 {
    fx.count(&format!(
        "SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND ended_at IS NULL \
         AND liveness='connected' AND harness='agy' AND native_session='{session}' \
         AND observation_provenance='cooperative_top_level'"
    ))
}
/// Open, connected bindings of the seat (session-keyed bindings: a superseded conversation's
/// binding stays open, disconnected).
fn live_bindings(fx: &Fixture) -> i64 {
    fx.count("SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND ended_at IS NULL AND liveness='connected'")
}

#[test]
fn top_level_session_start_enrolls_and_prints_the_startup_check_in() {
    let fx = Fixture::start();
    let home = home_with(&fx, "top-level");
    let before = generation(&fx);
    let hook = run(&fx, &home, "SessionStart", "session-start-top");
    let text = system_message(&hook);
    assert!(
        text.starts_with("Use inbox; follow its next: commands"),
        "{text}"
    );
    assert!(text.contains(herdr_threads::harness::agy::AGY_COMMAND_GUIDANCE.trim_end()));
    assert!(!text.contains("unavailable"), "{text}");
    assert!(
        text.len() <= 4096,
        "the startup context stays within the encode bound"
    );
    assert_eq!(live_binding(&fx, TOP), 1);
    assert_eq!(live_bindings(&fx), 1);
    // Session-keyed bindings: enrollment adds a binding; the seat's generation does not move.
    assert_eq!(generation(&fx), before);
}

#[test]
fn a_child_session_start_in_an_enrolled_pane_leaves_the_binding_alone() {
    let fx = Fixture::start();
    let home = home_with(&fx, "child-under-bound");
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    let (generation_before, check_ins) = (generation(&fx), fx.check_ins.load(Ordering::SeqCst));
    assert_eq!(live_binding(&fx, TOP), 1);
    let child = run(&fx, &home, "SessionStart", "session-start-child");
    let text = system_message(&child);
    assert!(text.to_lowercase().contains("subagent"), "{text}");
    assert!(!text.starts_with("Use inbox"), "{text}");
    assert_eq!(generation(&fx), generation_before);
    assert_eq!(live_binding(&fx, TOP), 1, "the top-level binding stays");
    assert_eq!(live_bindings(&fx), 1);
    assert_eq!(
        fx.check_ins.load(Ordering::SeqCst),
        check_ins,
        "a child is answered locally and never reaches the daemon"
    );
}

#[test]
fn clear_rotates_the_generation_and_binds_the_new_conversation() {
    let fx = Fixture::start();
    let home = home_with(&fx, "clear");
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    let enrolled = generation(&fx);
    let text = system_message(&run(
        &fx,
        &home,
        "SessionStart",
        "session-start-after-clear",
    ));
    assert!(text.starts_with("Use inbox"), "{text}");
    // Session-keyed bindings: the new conversation is a new binding at the same seat generation;
    // the earlier one, of the same agy process, is superseded (disconnected, never ended).
    assert_eq!(generation(&fx), enrolled);
    assert_eq!(live_binding(&fx, CLEAR), 1);
    assert_eq!(live_bindings(&fx), 1);
    assert_eq!(
        fx.count(&format!(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND native_session='{TOP}' AND liveness='disconnected' AND disconnect_reason='superseded'"
        )),
        1,
        "the earlier conversation's binding was superseded"
    );
}

#[test]
fn inv0_resume_reattaches_the_seat_by_c1_from_the_first_turn_start() {
    let fx = Fixture::start();
    // The pane ran one conversation earlier; `agy --conversation` then resumes another one,
    // which fires no SessionStart: its first turn start carries the lifecycle.
    home_with(&fx, "top-level");
    let home = home_with(&fx, "resume-stale-lock");
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    let enrolled = generation(&fx);
    assert_eq!(live_binding(&fx, TOP), 1);
    let text = system_message(&run(
        &fx,
        &home,
        "PreInvocation",
        "pre-invocation-inv0-resume",
    ));
    assert!(
        text.starts_with(LIFECYCLE_PREFIX),
        "a lifecycle from a turn start carries the supersede line: {text}"
    );
    assert!(
        text[LIFECYCLE_PREFIX.len()..].starts_with("Use inbox"),
        "{text}"
    );
    assert_eq!(
        generation(&fx),
        enrolled,
        "session-keyed: the seat generation stays"
    );
    assert_eq!(live_binding(&fx, RESUME), 1);
    assert_eq!(live_bindings(&fx), 1);
    assert_eq!(
        fx.count("SELECT count(*) FROM seats"),
        3,
        "the same seat is reused; no new seat is allocated"
    );
    // The resumed conversation is now the pane's: its next turn is routine and quiet.
    let routine = run(&fx, &home, "PreInvocation", "pre-invocation-inv0-resume");
    assert_eq!(routine.code, Some(0), "{}", routine.stderr);
    assert!(routine.stdout.is_empty(), "{:?}", routine.stdout);
}

/// Every file named `name` under `dir` whose parent directory is `adapter-state/agy/conversations`.
fn conversation_state_files(dir: &Path, name: &std::ffi::OsStr, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            conversation_state_files(&path, name, found);
        } else if path.file_name() == Some(name)
            && path
                .parent()
                .is_some_and(|parent| parent.ends_with("adapter-state/agy/conversations"))
        {
            found.push(path);
        }
    }
}

/// Every `adapter-state/agy` directory under `dir`.
fn adapter_state_dirs(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.ends_with("adapter-state/agy") {
                found.push(path);
            } else {
                adapter_state_dirs(&path, found);
            }
        }
    }
}

#[test]
fn a_resume_of_an_earlier_conversation_with_a_state_file_rebinds_by_c1() {
    let fx = Fixture::start();
    home_with(&fx, "clear");
    let home = home_with(&fx, "resume-stale-lock");
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    assert_eq!(live_binding(&fx, TOP), 1);
    let text = system_message(&run(
        &fx,
        &home,
        "PreInvocation",
        "pre-invocation-inv0-resume",
    ));
    assert!(text.starts_with(LIFECYCLE_PREFIX), "{text}");
    assert_eq!(live_binding(&fx, RESUME), 1);
    // A routine turn of RESUME. Under the advertised presentation V2 (7c3edc2d) a quiet
    // pull is `Unchanged` and writes no delivery cache, so the cache file a V1 pull or an
    // injected page leaves for RESUME is put in place here, beside the instance's agy state.
    let routine = run(&fx, &home, "PreInvocation", "pre-invocation-inv0-resume");
    assert_eq!(routine.code, Some(0), "{}", routine.stderr);
    assert!(routine.stdout.is_empty(), "{:?}", routine.stdout);
    let mut agy_state = Vec::new();
    adapter_state_dirs(&fx.state, &mut agy_state);
    assert_eq!(
        agy_state.len(),
        1,
        "the instance's agy state: {agy_state:?}"
    );
    let cache = herdr_threads::harness::agy::paths::conversation_state(&agy_state[0], RESUME);
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(
        &cache,
        serde_json::to_vec(&herdr_threads::harness::agy::delivery::ConversationState {
            turn: 1,
            generation: Some(generation(&fx) as u64),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let file_name = cache.file_name().unwrap().to_owned();
    let mut found = Vec::new();
    conversation_state_files(&fx.state, &file_name, &mut found);
    assert_eq!(found.len(), 1, "RESUME's state file: {found:?}");
    // `/clear` moves the pane to another conversation.
    system_message(&run(
        &fx,
        &home,
        "SessionStart",
        "session-start-after-clear",
    ));
    assert_eq!(live_binding(&fx, CLEAR), 1);
    assert_eq!(live_bindings(&fx), 1);
    let before = generation(&fx);
    // `agy --conversation RESUME` again: the state file must not make it known.
    let text = system_message(&run(
        &fx,
        &home,
        "PreInvocation",
        "pre-invocation-inv0-resume",
    ));
    assert!(text.starts_with(LIFECYCLE_PREFIX), "{text}");
    assert!(
        text[LIFECYCLE_PREFIX.len()..].starts_with("Use inbox"),
        "{text}"
    );
    assert_eq!(
        generation(&fx),
        before,
        "session-keyed: the seat generation stays"
    );
    assert_eq!(live_binding(&fx, RESUME), 1);
    assert_eq!(live_bindings(&fx), 1);
    assert!(found[0].exists(), "the fix does not delete the cache file");
}

#[test]
fn a_routine_turn_of_the_enrolled_conversation_prints_nothing() {
    let fx = Fixture::start();
    let home = home_with(&fx, "top-level");
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    let enrolled = generation(&fx);
    for (event, name) in [
        ("PreInvocation", "pre-invocation-inv1"),
        ("Stop", "stop-no-tool-call"),
    ] {
        let hook = run(&fx, &home, event, name);
        assert_eq!(hook.code, Some(0), "{event}: {}", hook.stderr);
        assert!(hook.stdout.is_empty(), "{event}: {:?}", hook.stdout);
    }
    assert_eq!(generation(&fx), enrolled);
}

#[test]
fn a_session_start_without_a_presence_lock_enrolls_nothing() {
    let fx = Fixture::start();
    let home = home_with(&fx, "missing-lock");
    let before = (generation(&fx), fx.check_ins.load(Ordering::SeqCst));
    let hook = run(&fx, &home, "SessionStart", "session-start-top");
    let text = system_message(&hook);
    assert!(!text.starts_with("Use inbox"), "{text}");
    assert_eq!(live_bindings(&fx), 0);
    assert_eq!(
        (generation(&fx), fx.check_ins.load(Ordering::SeqCst)),
        before
    );
}

/// A turn-start or stop hook never starts a daemon: with none running it prints nothing.
#[test]
fn with_the_daemon_down_a_routine_hook_prints_nothing_and_exits_zero() {
    let mut fx = Fixture::start();
    let home = home_with(&fx, "top-level");
    fx.stop();
    for (event, name) in [
        ("PreInvocation", "pre-invocation-inv1"),
        ("Stop", "stop-no-tool-call"),
    ] {
        let hook = run(&fx, &home, event, name);
        assert_eq!(hook.code, Some(0), "{event}: {}", hook.stderr);
        assert!(
            hook.stdout.is_empty(),
            "{event}: {:?}",
            String::from_utf8_lossy(&hook.stdout)
        );
    }
}

/// With the daemon stopped, each lifecycle case still exits 0 (ht-poh.11) inside its production 5 s
/// lifecycle budget (the hook runs unscaled) and prints, as the Claude and Codex lifecycle hooks do,
/// the shared "check-in unavailable" notice as exactly one valid `systemMessage` step: never the
/// startup context, never a malformed step, and never a binding (ht-poh.47). Unlike the routine
/// `hook_step` events these run the core check-in, which ensures a daemon within that budget; with
/// no Herdr behind it the check-in is refused.
fn daemon_down_prints_one_check_in_unavailable_message(cases: &[(&str, &str, &str)]) {
    for &(layout, event, name) in cases {
        let mut fx = Fixture::start();
        let home = home_with(&fx, layout);
        fx.stop();
        let hook = run_payload_with(&fx, &home, event, &payload(name), false);
        assert_eq!(
            hook.code,
            Some(0),
            "{layout} {event} {name}: {}",
            hook.stderr
        );
        assert!(
            hook.elapsed < Duration::from_secs(5) + Duration::from_secs(5),
            "{layout} {event} {name}: the lifecycle bound is 5 s, took {:?}",
            hook.elapsed
        );
        // Exactly one JSON value, then at most one newline.
        let body = hook.stdout.strip_suffix(b"\n").unwrap_or(&hook.stdout);
        let mut values = serde_json::Deserializer::from_slice(body).into_iter::<Value>();
        assert!(
            values.next().is_some_and(|v| v.is_ok()),
            "{layout} {name}: stdout is not a JSON value: {:?}",
            String::from_utf8_lossy(&hook.stdout)
        );
        assert!(
            values.next().is_none(),
            "{layout} {name}: more than one JSON value: {:?}",
            String::from_utf8_lossy(&hook.stdout)
        );
        let text = system_message(&hook);
        assert!(
            text.contains("herdr-threads: check-in unavailable ("),
            "{layout} {name}: {text}"
        );
        assert!(!text.contains("Use inbox"), "{layout} {name}: {text}");
        assert!(
            text.len()
                <= herdr_threads::cli::hook::MAX_CONTEXT
                    + herdr_threads::harness::agy::decode::PRE_INVOCATION_LIFECYCLE_PREFIX.len(),
            "{layout} {name}: {} bytes",
            text.len()
        );
        assert_eq!(
            live_bindings(&fx),
            0,
            "{layout} {name}: nothing was enrolled"
        );
    }
}

#[test]
fn with_the_daemon_down_session_start_prints_one_check_in_unavailable_system_message() {
    daemon_down_prints_one_check_in_unavailable_message(&[
        ("top-level", "SessionStart", "session-start-top"),
        ("clear", "SessionStart", "session-start-after-clear"),
    ]);
}

#[test]
fn with_the_daemon_down_inv0_prints_one_check_in_unavailable_system_message() {
    daemon_down_prints_one_check_in_unavailable_message(&[
        ("top-level", "PreInvocation", "pre-invocation-inv0-fresh"),
        (
            "resume-stale-lock",
            "PreInvocation",
            "pre-invocation-inv0-resume",
        ),
    ]);
}

/// `herdr-threads --json <command>` as the cooperative stand-in caller `seat` on `pane`, whose
/// binding is `harness` (the shared `cooperative` helper always claims Claude).
fn cooperative_as(fx: &Fixture, harness: &str, seat: &str, pane: &str, command: &[&str]) -> Value {
    let mut args = vec![
        "herdr-threads",
        "--json",
        "--state-dir",
        fx.state.to_str().unwrap(),
        "--host-endpoint",
        fx.host.to_str().unwrap(),
        "--cooperative-seat",
        seat,
        "--cooperative-target",
        pane,
        "--cooperative-harness",
        harness,
        "--cooperative-role",
        "top-level",
    ];
    args.extend_from_slice(command);
    let mut out = Vec::new();
    // The in-process CLI acts as the harness session the hooks registered (decision 15).
    let _agent = super::agent_origin(harness, if harness == "agy" { TOP } else { "peer-sess" });
    herdr_threads::cli::run(args, &mut out).unwrap();
    serde_json::from_slice(&out).unwrap()
}
fn lazy_row_state(fx: &Fixture, message: &str) -> String {
    let db = rusqlite::Connection::open(&fx.db).unwrap();
    db.query_row(
        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='seat'",
        [message],
        |row| row.get(0),
    )
    .unwrap()
}

/// agy declares `HookPull`, so a standard hook never reads an inbox page or completes a lazy row
/// (the `cooperative_hook_context` path of ht-poh.17): a `SessionStart` with a lazy row pending
/// shows no inbox block and leaves it `pending`; the routine turn start pulls it into one
/// `systemMessage` (still `pending`: a pull settles nothing); only the transcript-confirmed
/// `Stop` completes it.
#[test]
fn agy_lazy_row_settles_only_through_the_transcript_confirm_never_hook_context() {
    let fx = Fixture::start();
    let home = home_with(&fx, "clear");
    // The pane's first conversation, then a peer (Claude) that shares a thread with the seat.
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    let peer = fx.hook("w9:p2", &start("peer-sess"));
    assert_eq!(peer.code, Some(0), "{}", peer.stderr);
    let thread = data(cooperative_as(
        &fx,
        "claude",
        "peer",
        "w9:p2",
        &["thread", "create", "--topic", "agy-lazy"],
    ));
    cooperative_as(
        &fx,
        "claude",
        "peer",
        "w9:p2",
        &["invite", &thread, "--seat", "seat"],
    );
    cooperative_as(&fx, "agy", "seat", PANE, &["accept", &thread]);
    let lazy = data(cooperative_as(
        &fx,
        "claude",
        "peer",
        "w9:p2",
        &["send", &thread, "--body", "passive hello"],
    ));
    assert_eq!(lazy_row_state(&fx, &lazy), "pending");

    // `/clear` starts a new conversation while the row is pending: its SessionStart shows no
    // inbox block and completes nothing.
    let text = system_message(&run(
        &fx,
        &home,
        "SessionStart",
        "session-start-after-clear",
    ));
    assert!(text.starts_with("Use inbox"), "{text}");
    assert!(
        !text.contains("passive hello") && !text.contains(INBOX_HEADER),
        "the lifecycle context carries no lazy body or inbox block: {text}"
    );
    assert_eq!(live_binding(&fx, CLEAR), 1);
    assert_eq!(
        lazy_row_state(&fx, &lazy),
        "pending",
        "SessionStart must not settle a hook-pull seat's lazy row"
    );

    // The routine turn start pulls: one systemMessage with the body, nothing settled.
    let transcript = fx.root.join("transcript_full.jsonl");
    let for_clear = |name: &str| {
        let mut value: Value = serde_json::from_slice(&payload(name)).unwrap();
        value["conversationId"] = json!(CLEAR);
        value["transcriptPath"] = json!(transcript.to_str().unwrap());
        serde_json::to_vec(&value).unwrap()
    };
    let pulled = system_message(&run_payload(
        &fx,
        &home,
        "PreInvocation",
        &for_clear("pre-invocation-inv1"),
    ));
    assert!(pulled.contains("passive hello"), "{pulled}");
    assert!(
        !pulled.contains(INBOX_HEADER),
        "a pull is not the text inbox page: {pulled}"
    );
    assert_eq!(
        lazy_row_state(&fx, &lazy),
        "pending",
        "a pull settles nothing"
    );

    // agy writes the injected message into its transcript; the Stop hook finds it and confirms.
    let mut line = serde_json::to_vec(&json!({"step_index": 2, "source": "SYSTEM_SDK",
        "type": "SYSTEM_MESSAGE", "status": "DONE", "content": format!(
        "<SYSTEM_MESSAGE>\nThe following system message was injected by a PreInvocation hook:\n{pulled}\n</SYSTEM_MESSAGE>")}))
    .unwrap();
    line.push(b'\n');
    fs::write(&transcript, &line).unwrap();
    let stop = run_payload(&fx, &home, "Stop", &for_clear("stop-no-tool-call"));
    assert_eq!(stop.code, Some(0), "{}", stop.stderr);
    assert!(stop.stdout.is_empty(), "{:?}", stop.stdout);
    assert_eq!(lazy_row_state(&fx, &lazy), "displayed");
}

// Kills: classifying Lazy invitation as passive but losing its complete
// pending item between canonical inbox paging and the registered turn hook.
// The CLI can no longer create a lazy invitation (an agent `invite` always requires acceptance,
// message-priority D6); a lazy one only survives from before the upgrade. The fixture builds that
// legacy shape by sending the lazy `Invite` straight to the daemon with the peer's canonical claim.
#[test]
fn agy_turn_hook_carries_lazy_invitation_without_accepting_or_confirming_it() {
    use herdr_threads::protocol::{
        authority::{CallerClaim, CallerRole, LivenessFingerprint},
        commands::{Command, InvitationConfirmation, Invite},
        ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId, SeatId},
    };
    let fx = Fixture::start();
    let home = home_with(&fx, "top-level");
    system_message(&run(&fx, &home, "SessionStart", "session-start-top"));
    let peer = fx.hook("w9:p2", &start("peer-sess"));
    assert_eq!(peer.code, Some(0), "{}", peer.stderr);
    // Prime the hook's quiet attention-version cache before the invitation arrives.
    let quiet = run(&fx, &home, "PreInvocation", "pre-invocation-inv1");
    assert_eq!(quiet.code, Some(0), "{}", quiet.stderr);
    assert!(quiet.stdout.is_empty(), "{:?}", quiet.stdout);
    let thread = data(cooperative_as(
        &fx,
        "claude",
        "peer",
        "w9:p2",
        &[
            "thread",
            "create",
            "--topic",
            "passive invitation for review",
            "--goal",
            "review recipient remit before acceptance",
        ],
    ));
    // Legacy shape: a lazy invitation sent by the peer's canonical claim straight to the daemon.
    let claim: CallerClaim = rusqlite::Connection::open(&fx.db)
        .unwrap()
        .query_row(
            "SELECT seat_id,generation,execution_id,liveness_pid,liveness_start FROM occupant_bindings WHERE target_id='w9:p2' AND harness='claude' AND native_session='peer-sess' AND role='top_level' AND agent_id='' AND ended_at IS NULL",
            [],
            |row| {
                let liveness = match (row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?) {
                    (Some(pid), Some(start)) => Some(LivenessFingerprint {
                        pid: u32::try_from(pid).unwrap(),
                        start: u64::try_from(start).unwrap(),
                    }),
                    _ => None,
                };
                Ok(CallerClaim {
                    instance: fx.descriptor.instance_uuid.to_string(),
                    seat: SeatId::new(row.get::<_, String>(0)?),
                    binding_generation: u64::try_from(row.get::<_, i64>(1)?).unwrap(),
                    role: CallerRole::TopLevel,
                    harness: herdr_threads::protocol::authority::Harness::Claude,
                    native_session: NativeSessionId::new("peer-sess"),
                    execution: ExecutionId::new(row.get::<_, String>(2)?),
                    target: HostTargetId::new("w9:p2"),
                    agent_id: None,
                    liveness,
                })
            },
        )
        .unwrap();
    fx.call(Command::Invite(Invite {
        confirmation: Some(InvitationConfirmation::Lazy),
        thread: herdr_threads::protocol::ids::ThreadId::new(thread.clone()),
        seat: SeatId::new("seat"),
        deadline_millis: None,
        operation: OperationId::new("lazy-invite"),
        claim,
    }))
    .unwrap();
    let text = system_message(&run(&fx, &home, "PreInvocation", "pre-invocation-inv1"));
    assert!(text.contains("passive invitation for review"), "{text}");
    assert!(
        text.contains("review recipient remit before acceptance"),
        "{text}"
    );
    assert!(text.contains(&thread), "{text}");
    assert!(text.contains("accept"), "{text}");
    assert!(!text.contains("] attention "), "{text}");
    assert_eq!(fx.count("SELECT count(*) FROM invitations WHERE seat_id='seat' AND state='pending' AND confirmation='lazy'"), 1);
    assert_eq!(
        fx.count("SELECT count(*) FROM memberships WHERE seat_id='seat' AND state='joined'"),
        0
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM receipts WHERE seat_id='seat'"),
        0
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM wake_work WHERE seat_id='seat' AND attention_version>0"),
        0
    );
}

/// Registered bridge activation evidence. The daemon is the elected production
/// composition; only its external Herdr host and the private socket observer are
/// test-owned. A successful hook here proves emission/commit, not native uptake.
mod activation {
    use super::*;
    use herdr_threads::{
        daemon::ownership::EndpointDescriptor,
        harness::agy::delivery::{Identity, resolve_identity},
        protocol::{
            authority::CallerClaim,
            capabilities::DELIVERY_PRESENTATION_V2,
            commands::{
                CreateThread, DeliveryMode, InvitationConfirmation, Invite, Join, SendMessage,
            },
            delivery::{
                DeliveryPageV2, DeliveryPresentationConfirmationV2, DeliveryPullReplyV2,
                PresentationControlStampV2, PresentationItemV2, PresentationRenderSelection,
            },
            ids::OperationId,
            results::{ApiError, ErrorCode},
            wire::{WireRequest, WireResponse, encode_wire_response},
        },
    };
    use std::{
        io::Read,
        os::{
            fd::OwnedFd,
            unix::{
                fs::{MetadataExt, PermissionsExt},
                net::{UnixListener, UnixStream},
            },
        },
        sync::Mutex,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Fault {
        None,
        OmitCapability,
        RefuseCapabilities,
        DropConfirm,
        LoseConfirmReply,
    }

    #[derive(Clone, Debug)]
    struct Exchange {
        command: Command,
        result: Option<Result<CommandResult, ApiError>>,
    }

    /// Forward exact frames to the actual elected daemon. Negative modes only
    /// remove/refuse capability discovery or drop one real confirmation exchange.
    /// They never fabricate a capable reply, page, token or successful completion.
    struct Observer {
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
        exchanges: Arc<Mutex<Vec<Exchange>>>,
        endpoint: PathBuf,
        upstream: PathBuf,
        descriptor_path: PathBuf,
        original_descriptor: Vec<u8>,
    }

    fn frame(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
        let mut prefix = [0; 4];
        stream.read_exact(&mut prefix)?;
        let size = u32::from_be_bytes(prefix) as usize;
        assert!(size <= herdr_threads::protocol::wire::MAX_WIRE_FRAME_BYTES);
        let mut bytes = vec![0; size + 4];
        bytes[..4].copy_from_slice(&prefix);
        stream.read_exact(&mut bytes[4..])?;
        Ok(bytes)
    }

    impl Observer {
        fn start(fx: &continuity::Fixture, fault: Fault) -> Self {
            let descriptor_path = fx.instance_dir.join("endpoint.json");
            let original_descriptor = fs::read(&descriptor_path).unwrap();
            let mut descriptor: EndpointDescriptor =
                serde_json::from_slice(&original_descriptor).unwrap();
            let endpoint = descriptor.endpoint.clone();
            let upstream = fx.host.parent().unwrap().join("elected.sock");
            // Keep the descriptor's validated stable pathname. The real listener
            // remains live at its renamed private pathname throughout the test.
            fs::rename(&endpoint, &upstream).unwrap();
            let listener = UnixListener::bind(&endpoint).unwrap();
            fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
            let metadata = fs::metadata(&endpoint).unwrap();
            descriptor.socket_device = metadata.dev();
            descriptor.socket_inode = metadata.ino();
            fs::write(&descriptor_path, serde_json::to_vec(&descriptor).unwrap()).unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let exchanges = Arc::new(Mutex::new(Vec::new()));
            let worker_stop = stop.clone();
            let observed = exchanges.clone();
            let target = upstream.clone();
            let worker = std::thread::spawn(move || {
                let mut dropped = false;
                while !worker_stop.load(Ordering::SeqCst) {
                    let (mut client, _) = match listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("observer accept: {error}"),
                    };
                    client
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    client
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let Ok(request_bytes) = frame(&mut client) else {
                        continue;
                    };
                    let request = WireRequest::decode(&request_bytes[4..]).unwrap();
                    let confirm =
                        matches!(request.command, Command::DeliveryPresentationConfirmV2(_));
                    if confirm && fault == Fault::DropConfirm && !dropped {
                        dropped = true;
                        observed.lock().unwrap().push(Exchange {
                            command: request.command,
                            result: None,
                        });
                        continue;
                    }
                    let mut server = UnixStream::connect(&target).unwrap();
                    server
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    server
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    server.write_all(&request_bytes).unwrap();
                    let mut response_bytes = frame(&mut server).unwrap();
                    let mut response: WireResponse =
                        serde_json::from_slice(&response_bytes[4..]).unwrap();
                    if matches!(request.command, Command::Capabilities) {
                        match fault {
                            Fault::OmitCapability => {
                                let Ok(CommandResult::Capabilities(capabilities)) =
                                    &mut response.result
                                else {
                                    panic!("real capabilities")
                                };
                                capabilities
                                    .capabilities
                                    .retain(|name| name != DELIVERY_PRESENTATION_V2);
                                response_bytes = encode_wire_response(&response).unwrap();
                            }
                            Fault::RefuseCapabilities => {
                                response.result = Err(ApiError::new(
                                    ErrorCode::Unsupported,
                                    "historical peer refuses capabilities",
                                ));
                                response_bytes = encode_wire_response(&response).unwrap();
                            }
                            _ => {}
                        }
                    }
                    observed.lock().unwrap().push(Exchange {
                        command: request.command,
                        result: Some(response.result),
                    });
                    if confirm && fault == Fault::LoseConfirmReply && !dropped {
                        dropped = true;
                        continue;
                    }
                    // Unmodified response bytes in every positive exchange.
                    let _ = client.write_all(&response_bytes);
                }
            });
            Self {
                stop,
                worker: Some(worker),
                exchanges,
                endpoint,
                upstream,
                descriptor_path,
                original_descriptor,
            }
        }
        fn take(&self) -> Vec<Exchange> {
            std::mem::take(&mut *self.exchanges.lock().unwrap())
        }
    }
    impl Drop for Observer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.worker.take().unwrap().join().unwrap();
            fs::remove_file(&self.endpoint).unwrap();
            fs::rename(&self.upstream, &self.endpoint).unwrap();
            fs::write(&self.descriptor_path, &self.original_descriptor).unwrap();
        }
    }

    /// A model-free harness parent that lives for the elected fixture's whole
    /// session. Every installed hook remains a real child with that ancestor.
    struct HarnessProcess {
        child: std::cell::RefCell<herdr_threads::test_support::spawn::OwnedChild>,
        status: std::cell::RefCell<std::io::BufReader<UnixStream>>,
        directory: PathBuf,
        next: std::cell::Cell<u64>,
    }
    impl HarnessProcess {
        fn start(fx: &continuity::Fixture, home: &Path, harness: Harness, pane: &str) -> Self {
            let directory = home.join(format!("harness-{}", harness.as_str()));
            fs::create_dir_all(&directory).unwrap();
            // The shell dispatches one file per invocation. Hook stdin/stderr
            // and successful stdout use private files; only completion status
            // uses the dedicated socket. No hook output is interpreted as code.
            let driver =
                r#"while IFS= read -r script; do /bin/sh "$script"; printf '%s\n' "$?" >&2; done"#;
            let mut command = herdr_threads::test_support::harness_shim::under_harness(
                harness.as_str(),
                Path::new("/bin/sh"),
                &["-c", driver],
                &[],
            );
            herdr_threads::test_support::isolation::scrub_env(&mut command);
            let (status_reader, status_writer) = UnixStream::pair().unwrap();
            status_reader
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap();
            // Retain the original broken-stdout seam: the hook really writes
            // to an inherited socket whose reader is closed, receiving EPIPE.
            let (closed_reader, broken_writer) = UnixStream::pair().unwrap();
            drop(closed_reader);
            command
                .env("HERDR_ENV", "1")
                .env("HERDR_PANE_ID", pane)
                .env("HERDR_SOCKET_PATH", &fx.host)
                .env("PATH", harness_path(&fx.host))
                .env("HOME", home)
                .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
                .env_remove("ANTIGRAVITY_AGENT")
                .env_remove("HERDR_PLUGIN_STATE_DIR")
                .env_remove("HERDR_BIN_PATH")
                .stdin(Stdio::piped())
                .stdout(Stdio::from(OwnedFd::from(broken_writer)))
                .stderr(Stdio::from(OwnedFd::from(status_writer)));
            Self {
                child: std::cell::RefCell::new(command.spawn_owned().unwrap()),
                status: std::cell::RefCell::new(std::io::BufReader::new(status_reader)),
                directory,
                next: std::cell::Cell::new(0),
            }
        }
        fn run(&self, registered: &str, input: &[u8], broken_stdout: bool) -> Hook {
            use std::io::BufRead;
            let n = self.next.get();
            self.next.set(n + 1);
            let path = |kind| self.directory.join(format!("{n}.{kind}"));
            let input_path = path("input");
            let output_path = path("stdout");
            let error_path = path("stderr");
            let script_path = path("sh");
            let quote = |path: &Path| {
                herdr_threads::harness::setup::shell_command(&[path.to_str().unwrap().to_owned()])
                    .unwrap()
            };
            fs::write(&input_path, input).unwrap();
            let stdout = if broken_stdout {
                String::new()
            } else {
                format!(" > {}", quote(&output_path))
            };
            fs::write(
                &script_path,
                format!(
                    "{registered} < {}{stdout} 2> {}\n",
                    quote(&input_path),
                    quote(&error_path)
                ),
            )
            .unwrap();
            // The fixture creates this path, and the dispatcher accepts one
            // literal path per line. Never send payload/body text on this pipe.
            assert!(!script_path.to_str().unwrap().contains(['\n', '\r']));
            let started = Instant::now();
            {
                let mut child = self.child.borrow_mut();
                let stdin = child.stdin.as_mut().unwrap();
                writeln!(stdin, "{}", script_path.display()).unwrap();
                stdin.flush().unwrap();
            }
            let mut status = String::new();
            assert_ne!(
                self.status.borrow_mut().read_line(&mut status).unwrap(),
                0,
                "harness dispatcher exited"
            );
            Hook {
                code: Some(status.trim().parse().expect("harness child exit status")),
                stdout: if broken_stdout {
                    Vec::new()
                } else {
                    fs::read(output_path).unwrap()
                },
                stderr: String::from_utf8_lossy(&fs::read(error_path).unwrap()).into_owned(),
                elapsed: started.elapsed(),
                timeout_scale: None,
            }
        }
        fn stop(&self) {
            self.child.borrow_mut().stop();
        }
    }

    struct Elected {
        fx: continuity::Fixture,
        home: PathBuf,
        state: PathBuf,
        identity: Identity,
        recipient: CallerClaim,
        agy_parent: HarnessProcess,
        claude_parent: HarnessProcess,
    }
    impl Drop for Elected {
        fn drop(&mut self) {
            // Stop owned process groups before Fixture removes its state root.
            self.agy_parent.stop();
            self.claude_parent.stop();
        }
    }
    impl Elected {
        #[track_caller]
        fn start() -> Self {
            let startup_caller = std::panic::Location::caller();
            let fx = continuity::Fixture::start(
                &[
                    (PANE, continuity::Agent::NoSession),
                    ("w9:p2", continuity::Agent::NoSession),
                ],
                |_, _| {},
            );
            fx.wait_reconciled();
            let root = fx.host.parent().unwrap();
            let home = root.join("agy-home");
            let brain = home.join(".gemini/antigravity-cli/brain").join(TOP);
            fs::create_dir_all(brain.join(".system_generated/logs")).unwrap();
            fs::write(
                brain.join(".system_generated/logs/transcript_full.jsonl"),
                b"",
            )
            .unwrap();
            let presence = home.join(".gemini/antigravity-cli/presence");
            fs::create_dir_all(&presence).unwrap();
            fs::write(presence.join(format!("{TOP}.lock")), b"").unwrap();
            // An isolated executable identity, measured exactly as the child
            // measures it; no native AGY invocation or version gate is involved.
            let agy = root.join("bin/agy");
            fs::write(&agy, b"#!/bin/sh\nexit 1\n").unwrap();
            fs::set_permissions(&agy, fs::Permissions::from_mode(0o700)).unwrap();
            let identity =
                resolve_identity(None, Some(std::ffi::OsStr::new(&harness_path(&fx.host))))
                    .unwrap();
            let state = root.join("state");
            let agy_parent = HarnessProcess::start(
                &fx,
                &home,
                OccupantHarness::Agent(builtins().agent("agy").unwrap()).into(),
                PANE,
            );
            let claude_parent = HarnessProcess::start(&fx, &home, Harness::Claude, "w9:p2");
            let observer = Observer::start(&fx, Fault::None);
            let startup = Self::invoke(&home, &state, &agy_parent, "SessionStart", false);
            assert_eq!(startup.code, Some(0), "{}", startup.stderr);
            assert!(!startup.stdout.is_empty(), "{}", startup.stderr);
            let exchanges = observer.take();
            let recipient = exchanges
                .iter()
                .find_map(|exchange| match &exchange.result {
                    Some(Ok(CommandResult::CheckedIn(checked))) => Some(checked.context.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!(
                        "installed SessionStart reaches canonical CheckIn; {}",
                        startup_observation(&exchanges, &startup, root, startup_caller)
                    )
                });
            drop(observer);
            Self {
                fx,
                home,
                state,
                identity,
                recipient,
                agy_parent,
                claude_parent,
            }
        }
        fn adapter_state(&self) -> PathBuf {
            self.fx.instance_dir.join("adapter-state/agy")
        }
        fn downgrade(&self, age_ms: u64) {
            let state = self.adapter_state();
            for directory in [self.fx.instance_dir.join("adapter-state"), state.clone()] {
                if let Err(error) = fs::DirBuilder::new().mode(0o700).create(&directory) {
                    assert_eq!(
                        error.kind(),
                        std::io::ErrorKind::AlreadyExists,
                        "create private {}: {error}",
                        directory.display()
                    );
                }
                let metadata = fs::symlink_metadata(&directory).unwrap();
                assert!(
                    metadata.file_type().is_dir(),
                    "private state must be a directory: {}",
                    directory.display()
                );
                assert_eq!(
                    metadata.mode() & 0o777,
                    0o700,
                    "private state mode: {}",
                    directory.display()
                );
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let record = herdr_threads::harness::agy::delivery::DowngradeRecord {
                v: 1,
                identity: self.identity.clone(),
                misses: vec![],
                downgraded_at: Some(now.saturating_sub(age_ms)),
                reason: Some("abort".into()),
                probation: false,
                probation_at: None,
            };
            fs::write(
                state.join("downgrade.json"),
                serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
        }
        fn run(&self, event: &str, broken_stdout: bool) -> Hook {
            Self::invoke(
                &self.home,
                &self.state,
                &self.agy_parent,
                event,
                broken_stdout,
            )
        }
        fn invoke(
            home: &Path,
            state: &Path,
            parent: &HarnessProcess,
            event: &str,
            broken_stdout: bool,
        ) -> Hook {
            let harness: Harness = OccupantHarness::Agent(builtins().agent("agy").unwrap()).into();
            let argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, harness);
            let base = herdr_threads::harness::setup::shell_command(&argv).unwrap();
            let brain = home.join(".gemini/antigravity-cli/brain").join(TOP);
            let input = json!({"conversationId":TOP,"artifactDirectoryPath":brain,"transcriptPath":brain.join(".system_generated/logs/transcript_full.jsonl"),"workspacePaths":[home],"invocationNum":0,"initialNumSteps":1});
            parent.run(
                &format!("{base} '--event' '{event}'"),
                &serde_json::to_vec(&input).unwrap(),
                broken_stdout,
            )
        }
        fn invite(&self) {
            // The sender is enrolled through its installed production hook too.
            let argv = installed_argv(
                BIN,
                Some(self.state.to_str().unwrap()),
                None,
                Harness::Claude,
            );
            let registered = herdr_threads::harness::setup::shell_command(&argv).unwrap();
            let startup = self.claude_parent.run(
                &registered,
                &continuity::session_start("claude", "origin-session", "startup"),
                false,
            );
            assert_eq!(startup.code, Some(0), "{}", startup.stderr);
            let db = self.fx.db();
            let origin: CallerClaim = db.query_row("SELECT seat_id,generation,execution_id,liveness_pid,liveness_start FROM occupant_bindings WHERE target_id='w9:p2' AND harness='claude' AND native_session='origin-session' AND role='top_level' AND agent_id='' AND ended_at IS NULL", [], |row| {
                let liveness = match (row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?) {
                    (Some(pid), Some(start)) => Some(herdr_threads::protocol::authority::LivenessFingerprint {
                        pid: u32::try_from(pid).expect("canonical binding fingerprint pid is valid"),
                        start: u64::try_from(start).expect("canonical binding fingerprint start is nonnegative"),
                    }),
                    (None, None) => None,
                    _ => panic!("canonical binding fingerprint is complete or absent"),
                };
                Ok(CallerClaim { instance: self.fx.instance.to_string(), seat: herdr_threads::protocol::ids::SeatId::new(row.get::<_,String>(0)?), binding_generation: u64::try_from(row.get::<_, i64>(1)?).expect("canonical binding generation is nonnegative"),
                    role: herdr_threads::protocol::authority::CallerRole::TopLevel, harness: herdr_threads::protocol::authority::Harness::Claude,
                    native_session: herdr_threads::protocol::ids::NativeSessionId::new("origin-session"), execution: herdr_threads::protocol::ids::ExecutionId::new(row.get::<_,String>(2)?), target: herdr_threads::protocol::ids::HostTargetId::new("w9:p2"), agent_id: None, liveness })
            }).unwrap();
            let CommandResult::ThreadCreated(thread) = self
                .fx
                .call(Command::CreateThread(CreateThread {
                    name: None,
                    topic: "registered hook activation".into(),
                    goal: "exact emitted control".into(),
                    operation: OperationId::new("activation-thread"),
                    claim: origin.clone(),
                }))
                .unwrap()
            else {
                panic!("created thread")
            };
            self.fx
                .call(Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::RequireAck),
                    thread,
                    seat: self.recipient.seat.clone(),
                    deadline_millis: Some(300_000),
                    operation: OperationId::new("activation-invite"),
                    claim: origin.clone(),
                }))
                .unwrap();
            let CommandResult::ThreadCreated(thread) = self
                .fx
                .call(Command::CreateThread(CreateThread {
                    name: None,
                    topic: "ordinary hook output".into(),
                    goal: "pending receipt stays pending".into(),
                    operation: OperationId::new("activation-message-thread"),
                    claim: origin.clone(),
                }))
                .unwrap()
            else {
                panic!("message thread")
            };
            self.fx
                .call(Command::Join(Join {
                    thread: thread.clone(),
                    operation: OperationId::new("activation-recipient-join"),
                    claim: self.recipient.clone(),
                }))
                .unwrap();
            self.fx
                .call(Command::SendMessage(SendMessage {
                    priority: None,
                    delivery_mode: DeliveryMode::Ordinary,
                    user_intent: None,
                    thread,
                    body: "ordinary body needs later receipt ACK".into(),
                    direct_recipient: None,
                    invited_recipients: vec![self.recipient.seat.clone()],
                    deadline_millis: Some(300_000),
                    operation: OperationId::new("activation-ordinary-send"),
                    claim: origin,
                    relays_user: false,
                }))
                .unwrap();
        }
        fn conversation_bytes(&self) -> Option<Vec<u8>> {
            fs::read(herdr_threads::harness::agy::paths::conversation_state(
                &self.adapter_state(),
                TOP,
            ))
            .ok()
        }
        fn assert_passive_side_effects(&self) {
            assert_eq!(
                self.fx
                    .count("SELECT count(*) FROM invitations WHERE state='pending'"),
                1,
                "presentation never accepts the invitation"
            );
            let pending: i64 = self.fx.db().query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id=?1 AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1", [self.recipient.seat.as_str()], |row| row.get(0)).unwrap();
            assert_eq!(
                pending, 1,
                "actual ordinary obligation stays pending; presentation is not ACK"
            );
        }
    }

    /// Isolated SessionStart failure evidence only; never log a delivery hook's
    /// output, claims, request payloads, tokens, environment or full argv.
    fn startup_observation(
        exchanges: &[Exchange],
        hook: &Hook,
        root: &Path,
        caller: &std::panic::Location<'_>,
    ) -> String {
        fn snippet(bytes: &[u8], root: &Path, limit: usize) -> String {
            let text = String::from_utf8_lossy(bytes)
                .replace(root.to_string_lossy().as_ref(), "<fixture>");
            let mut encoded = String::from("\"");
            let mut consumed = 0;
            for character in text.chars() {
                let escaped = serde_json::to_string(&character.to_string()).unwrap();
                let escaped = &escaped[1..escaped.len() - 1];
                if encoded.len() + escaped.len() > limit - 64 {
                    break;
                }
                encoded.push_str(escaped);
                consumed += character.len_utf8();
            }
            encoded.push('"');
            encoded.push_str(&format!(
                " [omitted_redacted_bytes={}]",
                text.len() - consumed
            ));
            encoded
        }
        let command_tag = |command: &Command| match command {
            Command::Health => "Health",
            Command::Capabilities => "Capabilities",
            Command::ResolveSeat(_) => "ResolveSeat",
            Command::ContinuityCheckIn(_) => "ContinuityCheckIn",
            Command::CheckIn(_) => "CheckIn",
            Command::OperatorCheckIn(_) => "OperatorCheckIn",
            Command::SeatInspect(_) => "SeatInspect",
            Command::Seats(_) => "Seats",
            Command::HarnessEvidence(_) => "HarnessEvidence",
            Command::HarnessEvidenceV2(_) => "HarnessEvidenceV2",
            Command::HookParseFailure(_) => "HookParseFailure",
            Command::AttentionDigest(_) => "AttentionDigest",
            Command::AttentionDigestDelivery(_) => "AttentionDigestDelivery",
            Command::HotThreads(_) => "HotThreads",
            Command::DeliveryPull(_) => "DeliveryPull",
            Command::DeliveryPullV2(_) => "DeliveryPullV2",
            Command::DeliveryPresentationConfirmV2(_) => "DeliveryPresentationConfirmV2",
            Command::DeliveryConfirm(_) => "DeliveryConfirm",
            Command::CompleteInboxDelivery(_) => "CompleteInboxDelivery",
            _ => "other",
        };
        let result_tag = |result: &CommandResult| match result {
            CommandResult::Health(_) => "Health",
            CommandResult::Capabilities(_) => "Capabilities",
            CommandResult::SeatResolved(_) => "SeatResolved",
            CommandResult::ContinuityReattached(_) => "ContinuityReattached",
            CommandResult::CheckedIn(_) => "CheckedIn",
            CommandResult::CachedCheckInPage(_) => "CachedCheckInPage",
            CommandResult::SeatInspect(_) => "SeatInspect",
            CommandResult::Seats(_) => "Seats",
            CommandResult::HarnessEvidenceRecorded { .. } => "HarnessEvidenceRecorded",
            CommandResult::HarnessEvidenceV2Recorded(_) => "HarnessEvidenceV2Recorded",
            CommandResult::HookParseFailureRecorded => "HookParseFailureRecorded",
            CommandResult::AttentionDigest(_) => "AttentionDigest",
            CommandResult::AttentionDigestDelivery { .. } => "AttentionDigestDelivery",
            CommandResult::HotThreads(_) => "HotThreads",
            CommandResult::DeliveryPulled(_) => "DeliveryPulled",
            CommandResult::DeliveryPulledV2(_) => "DeliveryPulledV2",
            CommandResult::DeliveryPresentationConfirmedV2(_) => "DeliveryPresentationConfirmedV2",
            CommandResult::DeliveryConfirmed(_) => "DeliveryConfirmed",
            CommandResult::InboxDeliveryCompleted(_) => "InboxDeliveryCompleted",
            _ => "other",
        };
        let indices: Vec<_> = if exchanges.len() <= 12 {
            (0..exchanges.len()).collect()
        } else {
            (0..6).chain(exchanges.len() - 6..exchanges.len()).collect()
        };
        let trace: Vec<_> = indices
            .iter()
            .map(|&index| {
                let exchange = &exchanges[index];
                let result = match &exchange.result {
                    None => "no-response".to_owned(),
                    Some(Err(error)) => format!("Err({:?})", error.code),
                    Some(Ok(result)) => result_tag(result).to_owned(),
                };
                format!("{index}:{}->{result}", command_tag(&exchange.command))
            })
            .collect();
        let thread = std::thread::current();
        let test = snippet(thread.name().unwrap_or("unnamed").as_bytes(), root, 192);
        let mut evidence = format!(
            "stdout={} stderr={}\nexchanges(total={},omitted={},indexed=[{}]); stage=Elected::start/SessionStart startup_fault=None iteration=unrecorded caller={}:{} test={} child(code={:?},elapsed_ms={},stdout_bytes={},stderr_captured_utf8_bytes={})",
            snippet(&hook.stdout, root, 1536),
            snippet(hook.stderr.as_bytes(), root, 1536),
            exchanges.len(),
            exchanges.len() - indices.len(),
            trace.join(", "),
            caller.file(),
            caller.line(),
            test,
            hook.code,
            hook.elapsed.as_millis(),
            hook.stdout.len(),
            hook.stderr.len(),
        );
        // Reserve 128 bytes for the assertion prefix within the 6 KiB record;
        // retain the decisive stage and final exchanges if framing ever grows.
        let limit = 6 * 1024 - 128;
        if evidence.len() > limit {
            let mut start = evidence.len() - (limit - 64);
            while !evidence.is_char_boundary(start) {
                start += 1;
            }
            evidence = format!(
                "[diagnostic_leading_bytes_omitted={start}] {}",
                &evidence[start..]
            );
        }
        evidence
    }

    /// Failure-only evidence, capped at 3 KiB without message bodies or full
    /// requests. This observes the same child's traffic; it cannot supply proof.
    fn observation(exchanges: &[Exchange], hook: &Hook, state: &Path) -> String {
        let mode = |path: &Path| {
            fs::symlink_metadata(path)
                .map(|metadata| format!("{:04o}", metadata.mode() & 0o7777))
                .unwrap_or_else(|error| format!("{:?}", error.kind()))
        };
        let trace: Vec<_> = exchanges
            .iter()
            .take(8)
            .map(|exchange| {
                let command = match &exchange.command {
                    Command::Capabilities => "Capabilities",
                    Command::DeliveryPull(_) => "PullV1",
                    Command::DeliveryPullV2(_) => "PullV2",
                    Command::DeliveryPresentationConfirmV2(_) => "ConfirmV2",
                    Command::CheckIn(_) => "CheckIn",
                    _ => "other",
                };
                let result = match &exchange.result {
                    None => "dropped".to_owned(),
                    Some(Err(error)) => format!("Err({:?})", error.code),
                    Some(Ok(CommandResult::Capabilities(caps))) => format!(
                        "Capabilities(v2={})",
                        caps.capabilities
                            .iter()
                            .any(|name| name == DELIVERY_PRESENTATION_V2)
                    ),
                    Some(Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)))) => {
                        format!(
                            "PageV2(items={},controls={})",
                            page.items.len(),
                            page.emitted_controls.len()
                        )
                    }
                    Some(Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused {
                        reason,
                    }))) => format!("RefusedV2({reason:?})"),
                    Some(Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Unchanged {
                        ..
                    }))) => "UnchangedV2".to_owned(),
                    Some(Ok(CommandResult::DeliveryPulled(_))) => "PullV1Reply".to_owned(),
                    Some(Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))) => format!(
                        "ConfirmV2({:?},controls={})",
                        reply.status,
                        reply.acknowledged_controls.len()
                    ),
                    Some(Ok(_)) => "other".to_owned(),
                };
                format!("{command}->{result}")
            })
            .collect();
        let stderr = hook.stderr.as_bytes();
        let tail = &stderr[stderr.len().saturating_sub(512)..];
        let mut evidence = format!(
            "adapter_modes(parent={},agy={}); child(status={:?},stdout_bytes={},elapsed={:?},stderr_tail={}); exchanges(total={},first8={trace:?})",
            mode(state.parent().unwrap_or(state)),
            mode(state),
            hook.code,
            hook.stdout.len(),
            hook.elapsed,
            String::from_utf8_lossy(tail),
            exchanges.len()
        );
        let mut end = evidence.len().min(3 * 1024);
        while !evidence.is_char_boundary(end) {
            end -= 1;
        }
        evidence.truncate(end);
        evidence
    }

    fn actual_page(exchanges: &[Exchange], hook: &Hook, state: &Path) -> DeliveryPageV2 {
        assert!(exchanges.iter().any(|exchange| matches!(&exchange.result, Some(Ok(CommandResult::Capabilities(caps))) if caps.capabilities.iter().any(|name| name == DELIVERY_PRESENTATION_V2))), "elected daemon must actually advertise V2; {}", observation(exchanges, hook, state));
        exchanges
            .iter()
            .find_map(|exchange| match &exchange.result {
                Some(Ok(CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)))) => {
                    Some(page.clone())
                }
                _ => None,
            })
            .expect("installed registered hook selects real V2 Pull")
    }

    fn confirm(exchanges: &[Exchange]) -> &Exchange {
        exchanges
            .iter()
            .find(|exchange| matches!(exchange.command, Command::DeliveryPresentationConfirmV2(_)))
            .expect("production HookStepCommit sends explicit Confirm")
    }

    fn round_trip(user: bool) {
        let elected = Elected::start();
        if user {
            elected.downgrade(0);
        }
        elected.invite();
        let observer = Observer::start(&elected.fx, Fault::None);
        let hook = elected.run("PreInvocation", false);
        assert_eq!(hook.code, Some(0), "{}", hook.stderr);
        let exchanges = observer.take();
        let page = actual_page(&exchanges, &hook, &elected.adapter_state());
        assert_eq!(
            page.render,
            PresentationRenderSelection::Hook {
                selector: u8::from(user)
            }
        );
        assert_eq!(page.output_bytes as usize, hook.stdout.len());
        assert!(!hook.stdout.is_empty() && hook.stdout.len() <= 65_536);
        let native: Value = serde_json::from_slice(&hook.stdout).unwrap();
        let expected = if user {
            json!({"injectSteps":[{"userMessage":page.text}]})
        } else {
            json!({"injectSteps":[{"systemMessage":{"systemMessage":page.text}}]})
        };
        assert_eq!(native, expected, "registered native codec shape");
        assert_eq!(
            hook.stdout,
            serde_json::to_vec(&expected).unwrap(),
            "exact registered native encoding, including escaping and framing"
        );
        assert!(
            page.items
                .iter()
                .any(|item| matches!(item, PresentationItemV2::Invitation { .. })),
            "canonical invitation is physically emitted"
        );
        assert!(page.items.iter().any(|item| matches!(item, PresentationItemV2::Message { lazy: false, content, .. } if content.body == "ordinary body needs later receipt ACK" && content.ack_required)), "ordinary nudge-class body also emits without receipt ACK");
        assert!(
            page.emitted_controls
                .iter()
                .any(|stamp| matches!(stamp, PresentationControlStampV2::Attention { .. })),
            "exercise a nonempty exact control confirmation"
        );
        let completed = confirm(&exchanges);
        let Command::DeliveryPresentationConfirmV2(request) = &completed.command else {
            unreachable!()
        };
        assert_eq!(request.claim, elected.recipient);
        assert_eq!(Some(&request.token), page.presentation_token.as_ref());
        assert_eq!(request.emitted_controls, page.emitted_controls);
        let Some(Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))) = &completed.result
        else {
            panic!("canonical explicit confirmation: {completed:?}")
        };
        assert_eq!(reply.status, DeliveryPresentationConfirmationV2::Confirmed);
        assert_eq!(reply.acknowledged_controls, page.emitted_controls);
        assert_eq!(elected.fx.count("SELECT count(*) FROM invitation_presentation_consumers WHERE consumer='hook' AND outbox_token IS NULL AND last_token IS NOT NULL"), 1);
        let state: herdr_threads::harness::agy::delivery::ConversationState =
            serde_json::from_slice(
                &elected
                    .conversation_bytes()
                    .expect("successful HookStepCommit persists hints"),
            )
            .unwrap();
        let PresentationControlStampV2::Attention { epoch, version } = &page.emitted_controls[0]
        else {
            panic!("captured Attention")
        };
        assert_eq!(state.seen_attention_version, Some(*version));
        assert_eq!(state.seen_presentation_epoch.as_ref(), Some(epoch));
        elected.assert_passive_side_effects();
    }

    // Kills: advertisement omitted; registered bridge remains on V1; native
    // selector drift; or successful emission fails to execute HookStepCommit.
    #[test]
    fn activation_v2_registered_agy_system_hook_round_trip() {
        round_trip(false);
    }
    #[test]
    fn activation_v2_registered_agy_user_hook_round_trip() {
        round_trip(true);
    }

    // Kills: public Ensure loses its Lazy selection, registered Hook falls back,
    // full context disappears, or physical presentation settles participation.
    #[test]
    fn modern_direct_ensure_lazy_pair_reaches_actual_turn_hook_without_settlement() {
        use herdr_threads::protocol::{
            authority::CallerRole,
            delivery::{PresentationConsumer, PresentationRenderRequest},
            results::DirectThreadEnsureOutcome,
        };
        use rusqlite::params;

        let elected = Elected::start();
        let argv = installed_argv(
            BIN,
            Some(elected.state.to_str().unwrap()),
            None,
            Harness::Claude,
        );
        let registered = herdr_threads::harness::setup::shell_command(&argv).unwrap();
        let startup_observer = Observer::start(&elected.fx, Fault::None);
        let startup = elected.claude_parent.run(
            &registered,
            &continuity::session_start("claude", "origin-session", "startup"),
            false,
        );
        assert_eq!(startup.code, Some(0), "{}", startup.stderr);
        assert!(!startup.stdout.is_empty(), "{}", startup.stderr);
        let origin = startup_observer
            .take()
            .into_iter()
            .find_map(|exchange| match exchange.result {
                Some(Ok(CommandResult::CheckedIn(checked))) => Some(checked.context),
                _ => None,
            })
            .expect("installed sender SessionStart reaches canonical CheckIn");
        drop(startup_observer);
        assert_eq!(origin.target.as_str(), "w9:p2");
        assert_eq!(origin.role, CallerRole::TopLevel);
        assert_eq!(origin.harness, OccupantHarness::Claude);
        assert_eq!(origin.native_session.as_str(), "origin-session");
        assert_eq!(origin.instance, elected.recipient.instance);
        assert_eq!(elected.recipient.target.as_str(), PANE);
        assert_eq!(elected.recipient.role, CallerRole::TopLevel);
        assert_eq!(elected.recipient.harness.as_str(), "agy");
        assert_eq!(elected.recipient.native_session.as_str(), TOP);
        assert_ne!(origin.seat, elected.recipient.seat);
        let quiet = elected.run("PreInvocation", false);
        assert_eq!(quiet.code, Some(0), "{}", quiet.stderr);
        assert!(quiet.stdout.is_empty(), "{:?}", quiet.stdout);

        let observer = Observer::start(&elected.fx, Fault::None);
        let body = "direct Ensure body remains pending after Hook presentation";
        let sent: Value = {
            // Preserve the actual registered sender session/fingerprint. The
            // shared helper selects its unrelated claude/sess-1 fixture.
            let _origin = herdr_threads::cli::caller::fix_origin(
                herdr_threads::cli::caller::CallerOrigin::Agent(
                    herdr_threads::harness::caller_env::EnvClaim {
                        harness: origin.harness,
                        native_session: origin.native_session.clone(),
                        agent_id: origin.agent_id.clone(),
                        role: origin.role,
                        fingerprint: origin.liveness.expect("registered sender fingerprint"),
                        provenance:
                            herdr_threads::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE,
                    },
                ),
            );
            let args = [
                "herdr-threads",
                "--json",
                "--state-dir",
                elected.state.to_str().unwrap(),
                "--host-endpoint",
                elected.fx.host.to_str().unwrap(),
                "--cooperative-seat",
                origin.seat.as_str(),
                "--cooperative-target",
                origin.target.as_str(),
                "--cooperative-harness",
                origin.harness.as_str(),
                "--cooperative-role",
                "top-level",
                "send",
                "--seat",
                elected.recipient.seat.as_str(),
                "--body",
                body,
            ];
            let mut output = Vec::new();
            herdr_threads::cli::run(args, &mut output).unwrap();
            serde_json::from_slice(&output).unwrap()
        };
        let producer = observer.take();
        assert_eq!(
            producer
                .iter()
                .filter(|exchange| matches!(exchange.command, Command::EnsureDirectThread(_)))
                .count(),
            1,
            "retain the first public Ensure decision, without a second Ensure"
        );
        let (ensure, actual) = producer
            .iter()
            .find_map(|exchange| match (&exchange.command, &exchange.result) {
                (
                    Command::EnsureDirectThread(request),
                    Some(Ok(CommandResult::DirectThreadEnsured(result))),
                ) => Some((request.clone(), result.clone())),
                _ => None,
            })
            .expect("parsed direct send executes modern Ensure and observes its actual result");
        assert_eq!(ensure.claim, origin);
        assert_eq!(ensure.recipient, elected.recipient.seat);
        let policy = ensure
            .creation_policy
            .as_ref()
            .expect("modern creation policy");
        assert_eq!(policy.confirmation, InvitationConfirmation::Lazy);
        assert_eq!(policy.deadline_millis, None);
        assert_eq!(actual.outcome, DirectThreadEnsureOutcome::Created);
        assert_eq!(actual.recipient, elected.recipient.seat);
        let invitation = actual
            .invitation
            .as_ref()
            .expect("actual selected invitation");
        assert_eq!(invitation.confirmation, InvitationConfirmation::Lazy);
        assert_eq!(invitation.effective_deadline_at, None);
        assert_eq!(invitation.episode, 1);
        assert_eq!(invitation.inviter.as_ref(), Some(&origin.seat));
        assert_eq!(
            sent["version"],
            herdr_threads::protocol::wire::PROTOCOL_VERSION
        );
        let result = sent
            .get("result")
            .filter(|result| result.is_object())
            .expect("versioned direct-send result object");
        assert_eq!(result["kind"], "message_sent");
        assert_eq!(result["thread"], json!(actual.thread));
        assert_eq!(result["selection"]["outcome"], "created");
        assert_eq!(result["selection"]["invitation"], json!(invitation));
        assert_eq!(
            producer
                .iter()
                .filter(|exchange| matches!(exchange.command, Command::SendMessage(_)))
                .count(),
            1
        );
        let (send, message) = producer
            .iter()
            .find_map(|exchange| match (&exchange.command, &exchange.result) {
                (Command::SendMessage(request), Some(Ok(CommandResult::MessageSent(id)))) => {
                    Some((request, id))
                }
                _ => None,
            })
            .expect("actual public SendMessage result");
        assert_eq!(send.claim, origin);
        assert_eq!(send.thread, actual.thread);
        assert_eq!(send.body, body);
        // The daemon advertises send.priority_v1: the default send carries priority 1 (the
        // passive class) and no legacy delivery mode.
        assert_eq!(send.priority.map(|p| p.get()), Some(1));
        assert_eq!(send.delivery_mode, DeliveryMode::Ordinary);
        assert_eq!(send.deadline_millis, None);
        assert_eq!(
            send.direct_recipient.as_ref(),
            Some(&elected.recipient.seat)
        );
        assert_eq!(result["data"], json!(message));
        assert_eq!(
            serde_json::from_value::<CommandResult>(result.clone()).unwrap(),
            CommandResult::MessageSent(message.clone())
        );
        assert!(
            !producer
                .iter()
                .any(|exchange| matches!(exchange.command, Command::Invite(_)))
        );

        let db = elected.fx.db();
        let mut pair = [origin.seat.as_str(), elected.recipient.seat.as_str()];
        pair.sort();
        let pair_rows: i64 = db.query_row(
            "SELECT count(*) FROM direct_threads WHERE instance_id=?1 AND seat_a=?2 AND seat_b=?3 AND thread_id=?4",
            params![origin.instance, pair[0], pair[1], actual.thread.as_str()],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(pair_rows, 1);
        let (topic, goal, topic_revision): (String, String, i64) = db
            .query_row(
                "SELECT topic,goal,topic_revision FROM threads WHERE id=?1",
                [actual.thread.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let literal_context = format!("1:1 {} {}", pair[0], pair[1]);
        assert_eq!(topic, literal_context);
        assert_eq!(goal, literal_context);
        assert!(topic_revision >= 0);
        let invitation_state = || {
            db.query_row(
                "SELECT thread_id,seat_id,episode,state,confirmation,inviter_seat_id,command_context_epoch,deadline_at FROM invitations WHERE id=?1",
                [invitation.id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, i64>(6)?, row.get::<_, i64>(7)?)),
            ).unwrap()
        };
        let before_invitation = invitation_state();
        assert_eq!(before_invitation.0, actual.thread.as_str());
        assert_eq!(before_invitation.1, elected.recipient.seat.as_str());
        assert_eq!(before_invitation.2, 1);
        assert_eq!(before_invitation.3, "pending");
        assert_eq!(before_invitation.4, "lazy");
        assert_eq!(before_invitation.5.as_deref(), Some(origin.seat.as_str()));
        assert!(before_invitation.6 >= 0);
        // Physical legacy timing stays retained; the effective Lazy deadline is absent.
        let command_epoch = before_invitation.6.to_string();
        let topic_revision = topic_revision.to_string();
        let membership_state = || {
            db.query_row(
                "SELECT episode,state,joined_at,left_at,retired_at FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                params![actual.thread.as_str(), elected.recipient.seat.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<i64>>(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?)),
            ).unwrap()
        };
        let before_membership = membership_state();
        assert_eq!(before_membership, (1, "invited".into(), None, None, None));
        let lazy_state = || {
            db.query_row(
                "SELECT ordinal,preparation_id,thread_id,state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                params![message.as_str(), elected.recipient.seat.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            ).unwrap()
        };
        let before_lazy = lazy_state();
        assert_eq!(before_lazy.2, actual.thread.as_str());
        assert_eq!(before_lazy.3, "pending");
        let binding = || {
            db.query_row(
                "SELECT generation,execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
                [elected.recipient.seat.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            ).unwrap()
        };
        let before_binding = binding();
        assert_eq!(
            u64::try_from(before_binding.0).unwrap(),
            elected.recipient.binding_generation
        );
        assert_eq!(before_binding.1, elected.recipient.execution.as_str());
        let assert_pending_without_wake = || {
            let receipts: i64 = db.query_row(
                "SELECT (SELECT count(*) FROM receipts WHERE seat_id=?1)+(SELECT count(*) FROM receipt_state WHERE seat_id=?1)",
                [elected.recipient.seat.as_str()], |row| row.get(0),
            ).unwrap();
            assert_eq!(receipts, 0);
            let logical_receipts: i64 = db.query_row(
                "SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id=?1 AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1",
                [elected.recipient.seat.as_str()], |row| row.get(0),
            ).unwrap();
            assert_eq!(
                logical_receipts, 0,
                "Lazy presentation creates no published logical ACK obligation"
            );
            let intervals: i64 = db
                .query_row(
                    "SELECT count(*) FROM membership_intervals WHERE thread_id=?1 AND seat_id=?2",
                    params![actual.thread.as_str(), elected.recipient.seat.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(intervals, 0);
            let frontier: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(attention_version),0) FROM wake_work WHERE seat_id=?1",
                    [elected.recipient.seat.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(frontier, 0);
            let wake_on: String = db
                .query_row(
                    "SELECT wake_on FROM seats WHERE id=?1",
                    [elected.recipient.seat.as_str()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(wake_on, "nudge", "no Lazy opt-in promotion");
        };
        assert_pending_without_wake();

        // Failure-local observation only. The binding selection uses the exact
        // registered seat/harness/session. Every other relation is looked up by
        // the captured message, thread, invitation, preparation or binding key.
        // Keep this predicate byte-identical to binding_lazy_unread_sql! so the
        // witness separates reader eligibility from publication/continuation.
        let lazy_diagnostic = || {
            let snapshot: String = db.query_row(
                r#"WITH b AS (SELECT * FROM occupant_bindings
                    WHERE seat_id=?2 AND harness=?6 AND native_session=?7
                    AND generation=?8 AND execution_id=?9 AND target_id=?10
                    AND agent_id='' AND ended_at IS NULL)
                SELECT json_object(
                    'binding',(SELECT json_object('id',binding_id,'seat',seat_id,
                        'generation',generation,'execution',execution_id,'session',native_session,
                        'role',role,'liveness',liveness,'pid',liveness_pid,'start',liveness_start,
                        'registered_at',registered_at,'observed_at',observed_at) FROM b),
                    'membership',(SELECT json_object('episode',episode,'state',state,
                        'joined_at',joined_at,'left_at',left_at,'retired_at',retired_at)
                        FROM memberships WHERE thread_id=?3 AND seat_id=?2),
                    'invitation',(SELECT json_object('id',id,'episode',episode,'state',state,
                        'confirmation',confirmation,'created_decision',created_decision_seq)
                        FROM invitations WHERE id=?5 AND thread_id=?3 AND seat_id=?2),
                    'cursor',(SELECT json_object('binding',c.binding_id,'thread',c.thread_id,'seq',c.seq)
                        FROM binding_cursors c JOIN b ON b.binding_id=c.binding_id WHERE c.thread_id=?3),
                    'lazy',(SELECT json_object('ordinal',ordinal,'preparation',preparation_id,
                        'message',message_id,'thread',thread_id,'seat',seat_id,'state',state,
                        'legacy_pending',legacy_pending)
                        FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2),
                    'message',(SELECT json_object('id',id,'instance',instance_id,'thread',thread_id,
                        'kind',kind,'mode',delivery_mode,'sequence',sequence,'decision',decision_seq,
                        'body_bytes',length(CAST(body AS BLOB)),'body_excerpt',substr(body,1,256))
                        FROM messages WHERE id=?1),
                    'manifest',(SELECT json_object('preparation',preparation_id,'message',message_id,
                        'instance',instance_id,'thread',thread_id,'decision',decision_seq,
                        'base_sequence',base_sequence,'recipient_count',recipient_count,
                        'warning_count',warning_count,'interval_high',interval_high_water)
                        FROM send_manifests WHERE message_id=?1),
                    'preparation',(SELECT json_object('id',p.id,'status',p.status,
                        'mode',p.delivery_mode,'recipient_count',p.recipient_count,
                        'recipient_cursor',p.recipient_cursor,'recipient_high',p.recipient_high_water)
                        FROM lazy_recipients r JOIN send_preparations p ON p.id=r.preparation_id
                        WHERE r.message_id=?1 AND r.seat_id=?2),
                    'send_attention',(SELECT json_object('id',j.id,'status',j.status,
                        'position',j.position,'high_water',j.high_water,'last_error',j.last_error)
                        FROM lazy_recipients r JOIN work_jobs j ON j.kind='send_attention'
                        AND j.subject_id=r.preparation_id WHERE r.message_id=?1 AND r.seat_id=?2),
                    'head',(SELECT decision_seq FROM host_instances WHERE id=?4),
                    'reader_gate',(SELECT json_object(
                        'has_cursor',EXISTS(SELECT 1 FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=r.thread_id),
                        'joined',EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=r.thread_id AND mb.seat_id=r.seat_id AND mb.state='joined'),
                        'past_cursor',m.sequence>COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=r.thread_id),0),
                        'legacy_pending',r.legacy_pending,
                        'binding_lazy_unread',((m.sequence>COALESCE((SELECT c.seq FROM binding_cursors c WHERE c.binding_id=b.binding_id AND c.thread_id=r.thread_id),0) AND (EXISTS(SELECT 1 FROM binding_cursors c2 WHERE c2.binding_id=b.binding_id AND c2.thread_id=r.thread_id) OR EXISTS(SELECT 1 FROM memberships mb WHERE mb.thread_id=r.thread_id AND mb.seat_id=r.seat_id AND mb.state='joined'))) OR (r.legacy_pending=1 AND b.role='top_level')))
                        FROM b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN messages m ON m.id=r.message_id
                        WHERE r.message_id=?1 AND r.thread_id=?3),
                    'consumer',(SELECT json_object('binding',c.binding_id,'generation',c.generation,
                        'execution',c.execution,'ordinary_done',c.ordinary_done,
                        'ordinary_version',c.ordinary_version,'ordinary_cursor',c.ordinary_cursor,
                        'ordinary_pending_items',json_array_length(c.ordinary_pending,'$.items'),
                        'ordinary_partial_bytes',length(CAST(c.ordinary_partial AS BLOB)),
                        'ordinary_skip_body',c.ordinary_skip_body,
                        'publication_after',c.ordinary_publication_after,'publication_high',c.ordinary_publication_high,
                        'passive_generation',c.ordinary_passive_generation,'scan_passive_generation',c.ordinary_scan_passive_generation,
                        'outbox_token',c.outbox_token,'last_token',c.last_token,'outbox_more',c.outbox_more,
                        'retired_mask',c.outbox_retired_mask,'confirmed_mask',c.outbox_confirmed_mask)
                        FROM invitation_presentation_consumers c JOIN b ON b.binding_id=c.binding_id
                        WHERE c.consumer='hook'))"#,
                params![message.as_str(), elected.recipient.seat.as_str(), actual.thread.as_str(),
                    elected.recipient.instance, invitation.id.as_str(), elected.recipient.harness.as_str(),
                    elected.recipient.native_session.as_str(), before_binding.0, elected.recipient.execution.as_str(),
                    elected.recipient.target.as_str()],
                |row| row.get(0),
            ).unwrap();
            let canonical: Value = serde_json::from_str(&snapshot).unwrap();
            json!({"message":message,"thread":actual.thread,"recipient":elected.recipient,
                "expected_body":body,"canonical":canonical})
        };
        let lazy_before_hook = lazy_diagnostic();

        let hook = elected.run("PreInvocation", false);
        assert_eq!(hook.code, Some(0), "{}", hook.stderr);
        let consumer = observer.take();
        let page = actual_page(&consumer, &hook, &elected.adapter_state());
        assert!(
            !consumer
                .iter()
                .any(|exchange| matches!(exchange.command, Command::DeliveryPull(_)))
        );
        let pulls: Vec<_> = consumer
            .iter()
            .filter_map(|exchange| match &exchange.command {
                Command::DeliveryPullV2(request) => Some(request),
                _ => None,
            })
            .collect();
        assert_eq!(pulls.len(), 1);
        let pull = pulls[0];
        assert_eq!(pull.claim, elected.recipient);
        assert_eq!(pull.consumer, PresentationConsumer::Hook);
        assert_eq!(pull.presentation_lease, None);
        assert_eq!(
            pull.render,
            PresentationRenderRequest::Hook {
                confirmable_selector: 0,
                passive_selector: 0
            }
        );
        assert_eq!(
            page.render,
            PresentationRenderSelection::Hook { selector: 0 }
        );
        assert!(page.items.len() <= 32);
        assert!(!hook.stdout.is_empty() && hook.stdout.len() <= 65_536);
        assert_eq!(page.output_bytes as usize, hook.stdout.len());
        let expected_native =
            json!({"injectSteps":[{"systemMessage":{"systemMessage":page.text}}]});
        assert_eq!(
            serde_json::from_slice::<Value>(&hook.stdout).unwrap(),
            expected_native
        );
        assert_eq!(hook.stdout, serde_json::to_vec(&expected_native).unwrap());

        let invitations: Vec<_> = page
            .items
            .iter()
            .filter_map(|item| match item {
                PresentationItemV2::Invitation { id, content } => Some((id, content)),
                _ => None,
            })
            .collect();
        assert_eq!(invitations.len(), 1);
        let (id, content) = invitations[0];
        assert_eq!(id, &invitation.id);
        assert_eq!(content.thread, actual.thread);
        assert!(!content.truncated && !content.wakes);
        assert_eq!(
            content.topic_revision.as_deref(),
            Some(topic_revision.as_str())
        );
        assert_eq!(
            content.command_context_epoch.as_deref(),
            Some(command_epoch.as_str())
        );
        let lines: Vec<_> = content.body.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "full context and one usable command, no deadline or continuation"
        );
        assert_eq!(lines[0], topic);
        let goal_line = format!("goal: {goal}");
        assert_eq!(lines[1], goal_line);
        let command = lines[2].strip_prefix("next: ").expect("usable command");
        let argv = shlex::split(command).expect("quoted acceptance argv");
        assert!(
            !pull.marker_prefix.is_empty(),
            "actual registered command prefix"
        );
        let mut expected_argv = pull.marker_prefix.clone();
        expected_argv.extend(["accept".into(), actual.thread.as_str().into()]);
        assert_eq!(argv, expected_argv);
        herdr_threads::cli::commands::parse_argv(argv)
            .expect("parse acceptance without executing it");
        assert!(page.text.contains(&format!(
            "[herdr-threads] invitation {} in {}",
            invitation.id.as_str(),
            actual.thread.as_str()
        )));
        assert!(
            page.text
                .contains(&format!("\n  {topic}\n  {goal_line}\n  next: {command}"))
        );
        let messages: Vec<_> = page
            .items
            .iter()
            .filter_map(|item| match item {
                PresentationItemV2::Message { id, lazy, content } => Some((id, lazy, content)),
                _ => None,
            })
            .collect();
        assert_eq!(
            messages.len(),
            1,
            "exact Lazy body missing; before={lazy_before_hook}; after={}; pull={pull:?}; page={page:?}; native={:?}; confirm={:?}; exchange_count={}",
            lazy_diagnostic(),
            String::from_utf8_lossy(&hook.stdout),
            consumer.iter().find(|exchange| matches!(
                exchange.command,
                Command::DeliveryPresentationConfirmV2(_)
            )),
            consumer.len()
        );
        let (id, lazy, content) = messages[0];
        assert_eq!(id, message);
        assert!(*lazy);
        assert_eq!(content.thread, actual.thread);
        assert_eq!(content.sender.as_ref(), Some(&origin.seat));
        assert_eq!(content.body, body);
        assert_eq!(content.body_len, body.len() as u64);
        assert!(!content.truncated && !content.ack_required);
        assert!(page.text.contains(&format!(
            "[herdr-threads] message {} in {}",
            message.as_str(),
            actual.thread.as_str()
        )));
        assert!(page.text.contains(&format!("\n  {body}")));
        assert!(!page.items.iter().any(|item| matches!(
            item,
            PresentationItemV2::Attention { .. } | PresentationItemV2::AttentionCleared { .. }
        )));
        assert!(page.emitted_controls.is_empty());

        assert_eq!(
            consumer
                .iter()
                .filter(|exchange| matches!(
                    exchange.command,
                    Command::DeliveryPresentationConfirmV2(_)
                ))
                .count(),
            1
        );
        let completed = confirm(&consumer);
        let Command::DeliveryPresentationConfirmV2(request) = &completed.command else {
            unreachable!()
        };
        assert_eq!(request.claim, elected.recipient);
        assert_eq!(request.consumer, PresentationConsumer::Hook);
        assert_eq!(request.presentation_lease, None);
        let token = page
            .presentation_token
            .as_ref()
            .expect("actual physical-output token");
        assert_eq!(&request.token, token);
        assert_eq!(request.emitted_controls, page.emitted_controls);
        let Some(Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))) = &completed.result
        else {
            panic!("canonical explicit confirmation: {completed:?}")
        };
        assert_eq!(reply.status, DeliveryPresentationConfirmationV2::Confirmed);
        assert_eq!(reply.acknowledged_controls, page.emitted_controls);
        let canonical_tokens: (Option<String>, Option<String>) = db.query_row(
            "SELECT outbox_token,last_token FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer='hook' AND generation=?2 AND execution=?3",
            params![elected.recipient.seat.as_str(), before_binding.0, before_binding.1],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(canonical_tokens, (None, Some(token.clone())));
        let complete: i64 = db.query_row(
            "SELECT complete_count FROM invitation_presentation_items WHERE seat_id=?1 AND invitation_id=?2 AND consumer='hook' AND generation=?3 AND execution=?4",
            params![elected.recipient.seat.as_str(), invitation.id.as_str(), before_binding.0, before_binding.1],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(complete, 1);
        assert_eq!(invitation_state(), before_invitation);
        assert_eq!(membership_state(), before_membership);
        assert_eq!(lazy_state(), before_lazy);
        assert_eq!(binding(), before_binding);
        assert_pending_without_wake();
        assert!(!consumer.iter().any(|exchange| matches!(
            exchange.command,
            Command::Accept(_)
                | Command::AcceptRequired(_)
                | Command::Ack(_)
                | Command::AckDisplayed(_)
                | Command::AckModDelivered(_)
                | Command::DeliveryConfirm(_)
                | Command::CompleteInboxDelivery(_)
        )));
        // This host has no safe native wake target: eligibility/work is proved,
        // while native composer uptake and model consumption remain unobserved.
        drop(observer);
    }

    // Kills: failed actual stdout emission still commits the canonical page.
    #[test]
    fn activation_v2_registered_agy_failed_stdout_replays_without_completion() {
        let elected = Elected::start();
        elected.invite();
        elected.downgrade(24 * 60 * 60 * 1000);
        let before = elected.conversation_bytes();
        let downgrade_before = fs::read(elected.adapter_state().join("downgrade.json")).unwrap();
        let observer = Observer::start(&elected.fx, Fault::None);
        let failed_hook = elected.run("PreInvocation", true);
        let failed = observer.take();
        let pending = actual_page(&failed, &failed_hook, &elected.adapter_state());
        // Diagnostic only: snapshot the first allocated Notice (this failure
        // has one) at both boundaries, without changing replay eligibility.
        // Fixed columns and exact keys; no bodies or unbounded row lists.
        let notice = pending.items.iter().find_map(|item| match item {
            PresentationItemV2::Notice {
                warning, thread, ..
            } => Some((warning, thread)),
            _ => None,
        });
        let notice_snapshot = || -> Value {
            let Some((warning, thread)) = notice else {
                return Value::Null;
            };
            let raw: String = elected.fx.db().query_row(
                r#"WITH own AS (
                    SELECT * FROM occupant_bindings WHERE seat_id=?1 AND harness=?2
                    AND native_session=?3 AND agent_id='' AND ended_at IS NULL
                ) SELECT json_object(
                    'binding',(SELECT json_object('id',binding_id,'generation',generation,
                        'execution',execution_id,'liveness',liveness,'reason',disconnect_reason,
                        'pid',liveness_pid,'start',liveness_start) FROM own),
                    'physical',(SELECT json_object('kind',kind,'thread',thread_id,
                        'instance',instance_id,'decision',decision_seq,'offset',event_offset)
                        FROM messages WHERE id=?4),
                    'logical',(SELECT json_object('ordinal',w.ordinal,'preparation',w.preparation_id,
                        'offset',w.warning_offset,'affected',w.affected_seat_id,
                        'episode',w.unavailability_episode,'thread',sm.thread_id,
                        'instance',sm.instance_id,'source',sm.message_id,'decision',sm.decision_seq,
                        'base_sequence',sm.base_sequence,'warning_count',sm.warning_count,
                        'interval_high',sm.interval_high_water)
                        FROM prepared_unavailable_warnings w JOIN send_manifests sm
                        ON sm.preparation_id=w.preparation_id WHERE w.warning_id=?4),
                    'recipient',(SELECT json_object('ordinal',ordinal,'generation',generation)
                        FROM warning_recipients WHERE warning_id=?4 AND seat_id=?1),
                    'digest',(SELECT json_object('ordinal',ordinal,'thread',thread_id,
                        'event_seq',event_seq,'event_offset',event_offset)
                        FROM digest_programmatic_warnings WHERE warning_id=?4 AND seat_id=?1),
                    'frontier',(SELECT offered_ordinal FROM digest_notice_offer
                        WHERE binding_id=(SELECT binding_id FROM own)),
                    'outbox',(SELECT json_object('generation',generation,'execution',execution,
                        'token',outbox_token,'retired_mask',outbox_retired_mask,
                        'confirmed_mask',outbox_confirmed_mask,'last_token',last_token,
                        'last_confirmed_mask',last_confirmed_mask,'codec',outbox_codec_key,
                        'selection',outbox_selection)
                        FROM invitation_presentation_consumers
                        WHERE binding_id=(SELECT binding_id FROM own) AND consumer='hook')
                )"#,
                rusqlite::params![elected.recipient.seat.as_str(), elected.recipient.harness.as_str(),
                    elected.recipient.native_session.as_str(), warning.as_str()],
                |row| row.get(0),
            ).unwrap();
            json!({"notice":warning,"expected_thread":thread,"initial_claim":elected.recipient,
                "allocated_token":pending.presentation_token,"allocated_controls":pending.emitted_controls,
                "canonical":serde_json::from_str::<Value>(&raw).unwrap()})
        };
        let notice_before_retry = notice_snapshot();
        assert_eq!(
            elected.conversation_bytes(),
            before,
            "failed emission never advances seen/injected hints"
        );
        assert_eq!(
            fs::read(elected.adapter_state().join("downgrade.json")).unwrap(),
            downgrade_before,
            "failed emission never opens probation"
        );
        assert!(
            !failed.iter().any(|exchange| matches!(
                exchange.command,
                Command::DeliveryPresentationConfirmV2(_)
            ))
        );
        assert_eq!(elected.fx.count("SELECT count(*) FROM invitation_presentation_consumers WHERE consumer='hook' AND outbox_token IS NOT NULL AND last_token IS NULL"), 1);
        let hook = elected.run("PreInvocation", false);
        assert_eq!(hook.code, Some(0), "{}", hook.stderr);
        let replay = observer.take();
        let page = actual_page(&replay, &hook, &elected.adapter_state());
        assert_eq!(page.presentation_token, pending.presentation_token);
        let nonordinary: Vec<_> = pending
            .items
            .iter()
            .filter(|item| !matches!(item, PresentationItemV2::Message { .. }))
            .cloned()
            .collect();
        assert_eq!(
            page.items,
            nonordinary,
            "the exact durable invitation/control descriptors physically replay; notice_before={}; notice_after={}; retry_checkins={}",
            notice_before_retry,
            notice_snapshot(),
            replay
                .iter()
                .filter(|exchange| matches!(exchange.command, Command::CheckIn(_)))
                .count()
        );
        assert_eq!(page.emitted_controls, pending.emitted_controls);
        assert_eq!(page.output_bytes as usize, hook.stdout.len());
        assert_eq!(system_message(&hook), page.text);
        assert!(
            matches!(&confirm(&replay).result, Some(Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))) if reply.status == DeliveryPresentationConfirmationV2::Confirmed)
        );
        let record: herdr_threads::harness::agy::delivery::DowngradeRecord =
            serde_json::from_slice(
                &fs::read(elected.adapter_state().join("downgrade.json")).unwrap(),
            )
            .unwrap();
        assert!(
            !record.probation,
            "a passive descriptor replay has no offered ordinary frame and cannot spend probation"
        );
        elected.assert_passive_side_effects();
    }

    // Kills: a missing completion retires unseen state, or response loss makes
    // a successfully committed token non-idempotent.
    #[test]
    fn activation_v2_registered_agy_lost_completion_preserves_exact_recovery() {
        for fault in [Fault::DropConfirm, Fault::LoseConfirmReply] {
            let elected = Elected::start();
            elected.invite();
            let before = elected.conversation_bytes();
            let observer = Observer::start(&elected.fx, fault);
            let hook = elected.run("PreInvocation", false);
            assert_eq!(hook.code, Some(0), "{}", hook.stderr);
            let exchanges = observer.take();
            let page = actual_page(&exchanges, &hook, &elected.adapter_state());
            let completed = confirm(&exchanges);
            assert_eq!(
                elected.conversation_bytes(),
                before,
                "missing actual completion response never advances local seen/injected hints"
            );
            if fault == Fault::DropConfirm {
                assert!(completed.result.is_none());
                assert_eq!(elected.fx.count("SELECT count(*) FROM invitation_presentation_consumers WHERE consumer='hook' AND outbox_token IS NOT NULL"), 1);
                let replay = elected.run("PreInvocation", false);
                assert_eq!(replay.code, Some(0), "{}", replay.stderr);
                let replayed = observer.take();
                let replay_page = actual_page(&replayed, &replay, &elected.adapter_state());
                assert_eq!(replay_page.presentation_token, page.presentation_token);
                assert_eq!(replay_page.emitted_controls, page.emitted_controls);
                assert_eq!(system_message(&replay), replay_page.text);
                assert_eq!(replay_page.output_bytes as usize, replay.stdout.len());
                assert!(
                    matches!(&confirm(&replayed).result, Some(Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))) if reply.status == DeliveryPresentationConfirmationV2::Confirmed)
                );
            } else {
                assert!(
                    matches!(&completed.result, Some(Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))) if reply.status == DeliveryPresentationConfirmationV2::Confirmed)
                );
                let CommandResult::DeliveryPresentationConfirmedV2(reply) =
                    elected.fx.call(completed.command.clone()).unwrap()
                else {
                    panic!("actual replay Confirm")
                };
                assert_eq!(
                    reply.status,
                    DeliveryPresentationConfirmationV2::AlreadyConfirmed
                );
                assert_eq!(reply.acknowledged_controls, page.emitted_controls);
            }
            elected.assert_passive_side_effects();
        }
    }

    // Controlled historical peer negatives reach the real registered bridge;
    // they are never used as active capability or completion evidence.
    #[test]
    fn registered_agy_omitted_or_refused_capability_uses_historical_pull() {
        for fault in [Fault::OmitCapability, Fault::RefuseCapabilities] {
            let elected = Elected::start();
            elected.invite();
            let observer = Observer::start(&elected.fx, fault);
            let hook = elected.run("PreInvocation", false);
            assert_eq!(hook.code, Some(0), "{}", hook.stderr);
            let exchanges = observer.take();
            assert!(
                exchanges
                    .iter()
                    .any(|exchange| matches!(exchange.command, Command::Capabilities)),
                "historical capability observation missing; fault={fault:?}; {}",
                observation(&exchanges, &hook, &elected.adapter_state())
            );
            assert!(
                exchanges
                    .iter()
                    .any(|exchange| matches!(exchange.command, Command::DeliveryPull(_))),
                "historical capability discovery selects real V1 bridge"
            );
            assert!(!exchanges.iter().any(|exchange| matches!(
                exchange.command,
                Command::DeliveryPullV2(_) | Command::DeliveryPresentationConfirmV2(_)
            )));
            assert!(system_message(&hook).contains("registered hook activation"));
            elected.assert_passive_side_effects();
        }
    }
}
