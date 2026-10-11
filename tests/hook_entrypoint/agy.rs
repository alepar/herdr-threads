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
    let mut line = scrubbed_command("/bin/sh");
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
/// The seat's live agy binding for `session`, enrolled as a cooperative top-level claim.
fn live_binding(fx: &Fixture, session: &str) -> i64 {
    fx.count(&format!(
        "SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND ended_at IS NULL \
         AND harness='agy' AND native_session='{session}' \
         AND observation_provenance='cooperative_top_level'"
    ))
}
fn live_bindings(fx: &Fixture) -> i64 {
    fx.count("SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND ended_at IS NULL")
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
    assert!(generation(&fx) > before);
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
    assert!(generation(&fx) > enrolled);
    assert_eq!(live_binding(&fx, CLEAR), 1);
    assert_eq!(live_bindings(&fx), 1);
    assert_eq!(
        fx.count(&format!(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND native_session='{TOP}' AND ended_at IS NOT NULL"
        )),
        1,
        "the earlier conversation's binding ended"
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
    assert!(generation(&fx) > enrolled);
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
    // A routine turn of RESUME: its pull writes the conversation's delivery-cache file.
    let routine = run(&fx, &home, "PreInvocation", "pre-invocation-inv0-resume");
    assert_eq!(routine.code, Some(0), "{}", routine.stderr);
    assert!(routine.stdout.is_empty(), "{:?}", routine.stdout);
    let file_name = herdr_threads::harness::agy::paths::conversation_state(Path::new(""), RESUME)
        .file_name()
        .unwrap()
        .to_owned();
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
    assert!(generation(&fx) > before);
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
        &["send", &thread, "--lazy", "--body", "passive hello"],
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
#[test]
fn agy_turn_hook_carries_lazy_invitation_without_accepting_or_confirming_it() {
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
    cooperative_as(
        &fx,
        "claude",
        "peer",
        "w9:p2",
        &["invite", &thread, "--seat", "seat"],
    );
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
