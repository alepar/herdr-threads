//! agy hooks through the built binary against the in-process daemon `Fixture` (agy-adapter D5):
//! the registered `hook agy --event E` command runs with `HOME` set to a temp home holding one of
//! `tests/fixtures/agy/layouts/*.txt`, so the presence lock and subagent listings decide the role.
//! The seeded seat `seat` on pane `w9:p1` is the pane under test.
use super::*;
use herdr_threads::harness::registry::{OccupantHarness, builtins};

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
    herdr_threads::test_support::spawn::tag(&mut line);
    let mut child = line.spawn_owned().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&payload(name))
        .unwrap();
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
