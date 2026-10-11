//! ht-poh.11: the agy harness adapter's cross-leaf paths, end to end and without a model.
//!
//! This module is a child of `mod_delivery` so it reuses that sweep's rig as is: the elected
//! daemon with an injected clock, the stand-in Herdr that records every `agent.prompt`, the
//! installed Claude hooks, and the REAL `watch` CLI. On top of it one agy pane (`PANE_G`, agent
//! kind `agy`) is enrolled through the REAL `hook agy` command (the installed argv contract,
//! `HOME` holding a recorded presence layout) and plays: pull (`PreInvocation`) -> inject ->
//! transcript confirm (`Stop`) -> settle. The agy transcript is a scripted file; nothing here needs
//! `agy`.
//!
//! Cases no leaf covers: (a) an agy seat and a Claude seat on one daemon (pull and confirm leave
//! the Claude `watch` path alone); (b) `wake_on` across a Native `HookPull` seat and a Claude
//! seat with a live `Channel` registration; (c) a `/clear` rotation discards the pull ledger.

use super::*;
use herdr_threads::{
    harness::registry::{OccupantHarness, builtins},
    notification::policy::WAKE_UP,
};

const PANE_G: &str = "w1:p3";
const TOP: &str = "534e47d6-64b3-4fd6-bd7f-446ceec4d86c";
const CLEAR: &str = "e806a19d-b316-4599-b4c5-a56db668fb4a";

fn agy_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agy")
}

fn agy_pane_value() -> Value {
    agent_pane(PANE_G, "term-g", "agy", None)
}

/// The agy seat `g` of a rig, joined to the rig's thread.
struct Agy {
    seat: String,
    home: PathBuf,
    transcript: PathBuf,
}

impl Agy {
    /// Resolves a seat for `PANE_G`, enrolls it with the REAL `hook agy` `SessionStart` of
    /// conversation `TOP` (layout `clear`, so a later `/clear` finds its own presence lock),
    /// invites it to the rig's thread and accepts as the agy caller.
    fn join(rig: &Rig) -> Self {
        let panes = vec![
            pane(PANE_A, "term-a"),
            claude(PANE_B, "term-b", SESSION_B),
            agy_pane_value(),
        ];
        rig.host.set_panes(panes);
        let seat = rig.seat_resolve(PANE_G);
        let home = rig.root.join("agy-home");
        let layout = fs::read_to_string(agy_fixtures().join("layouts/clear.txt")).unwrap();
        for line in layout.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let path = home.join(".gemini/antigravity-cli").join(line);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::File::create(path).unwrap();
        }
        let agy = Self {
            seat,
            home,
            transcript: rig.root.join("agy-transcript.jsonl"),
        };
        let started = agy.hook(rig, "SessionStart", "session-start-top", TOP);
        assert_eq!(started.code, 0, "{}{}", started.stdout, started.stderr);
        assert!(
            started.stdout.contains("Use inbox"),
            "the top-level startup context: {}",
            started.stdout
        );
        let mark = rig.host.mark();
        rig.cli(
            Some((&rig.a, PANE_A)),
            &["invite", &rig.thread, "--seat", &agy.seat],
        )
        .data("invite agy");
        // The invitation wakes the idle agy pane natively; waiting for that prompt before the
        // accept keeps the setup's wake out of the cases below.
        rig.wait_prompt(mark, PANE_G);
        agy.cli(rig, &["accept", &rig.thread]).data("agy accept");
        let passes = rig.probe.registered_idle_events(Lane::Wakes);
        rig.probe.kick_registered(Lane::Wakes);
        wait_until("the wake lane to finish its pass", STEP, || {
            (rig.probe.registered_idle_events(Lane::Wakes) > passes).then_some(())
        });
        // The native ladder's spacing passes, so the next attention wakes the pane at once.
        rig.clock.advance(31_000);
        agy
    }

    /// `herdr-threads --json <args>` as the agy seat's cooperative stand-in caller.
    fn cli(&self, rig: &Rig, args: &[&str]) -> Out {
        let mut all = vec![
            "--cooperative-seat",
            self.seat.as_str(),
            "--cooperative-target",
            PANE_G,
            "--cooperative-harness",
            "agy",
            "--cooperative-role",
            "top-level",
        ];
        all.extend_from_slice(args);
        collect(
            rig.command(None, &all)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn_owned()
                .unwrap()
                .wait_with_output()
                .unwrap(),
        )
    }

    /// One agy hook as the harness runs it: the installed `hook agy --event E` command through
    /// `sh -c`, the recorded payload (conversation and transcript path rewritten) on stdin.
    fn hook(&self, rig: &Rig, event: &str, fixture: &str, conversation: &str) -> Out {
        let mut payload: Value = serde_json::from_slice(
            &fs::read(agy_fixtures().join(format!("payloads/{fixture}.json"))).unwrap(),
        )
        .unwrap();
        payload["conversationId"] = json!(conversation);
        payload["transcriptPath"] = json!(self.transcript.to_str().unwrap());
        let harness = OccupantHarness::Agent(builtins().agent("agy").unwrap()).into();
        let argv = installed_argv(BIN, Some(rig.state.to_str().unwrap()), None, harness);
        let base = herdr_threads::harness::setup::shell_command(&argv).unwrap();
        let mut child = spawn::command("/bin/sh")
            .arg("-c")
            .arg(format!("{base} '--event' '{event}'"))
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", PANE_G)
            .env("HERDR_SOCKET_PATH", &rig.socket)
            .env("PATH", rig.path())
            .env("HOME", &self.home)
            .env("HERDR_THREADS_OFFLINE", "1")
            .env_remove("ANTIGRAVITY_AGENT")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_BIN_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&payload).unwrap())
            .unwrap();
        collect(child.wait_with_output().unwrap())
    }

    /// The `systemMessage` text of `{"injectSteps":[{"systemMessage":{"systemMessage":T}}]}`;
    /// `None` when the hook printed nothing. Any other shape fails.
    fn injected(out: &Out) -> Option<String> {
        assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
        if out.stdout.is_empty() {
            return None;
        }
        let value: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 1, "{value}");
        let steps = value["injectSteps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "{value}");
        assert_eq!(steps[0].as_object().unwrap().len(), 1, "{value}");
        Some(
            steps[0]["systemMessage"]["systemMessage"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }

    /// agy records the injected message in its transcript the way a live session does.
    fn append_frame(&self, text: &str) {
        let mut line = serde_json::to_vec(&json!({"step_index": 2, "source": "SYSTEM_SDK",
            "type": "SYSTEM_MESSAGE", "status": "DONE", "content": format!(
            "<SYSTEM_MESSAGE>\nThe following system message was injected by a PreInvocation hook:\n{text}\n</SYSTEM_MESSAGE>")}))
        .unwrap();
        line.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.transcript)
            .unwrap();
        file.write_all(&line).unwrap();
    }

    /// The seat's receipt for `message`: its state and the ack observation, when settled.
    fn receipt(&self, rig: &Rig, message: &str) -> (String, Option<Value>) {
        receipt_of(rig, message, &self.seat)
    }

    fn lazy_state(&self, rig: &Rig, message: &str) -> String {
        rig.db()
            .query_row(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                [message, &self.seat],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// A message A sends the agy seat; `ack` makes it an ordinary require-ACK one.
    fn send(&self, rig: &Rig, body: &str, ack: bool) -> String {
        send_to(rig, &self.seat, body, ack)
    }

    /// `wake_on` as the seat itself sets it.
    fn wake_on(&self, rig: &Rig, value: &str) {
        self.cli(rig, &["wake-on", value]).data("wake-on");
    }
}

fn send_to(rig: &Rig, seat: &str, body: &str, ack: bool) -> String {
    let mut args = vec!["send", rig.thread.as_str(), "--body", body];
    if ack {
        args.extend(["--require-ack", seat]);
    }
    rig.cli(Some((&rig.a, PANE_A)), &args).text("send")
}

fn receipt_of(rig: &Rig, message: &str, seat: &str) -> (String, Option<Value>) {
    let db = rig.db();
    for table in ["receipt_state", "receipts"] {
        let row: Option<(String, Option<String>)> = db
            .query_row(
                &format!(
                    "SELECT state, ack_observation FROM {table} WHERE message_id=?1 AND seat_id=?2"
                ),
                [message, seat],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((state, observation)) = row
            && state != "pending"
        {
            return (
                state,
                observation.map(|text| serde_json::from_str(&text).unwrap()),
            );
        }
    }
    ("pending".to_owned(), None)
}

/// The text typed into `pane` by native wakes after `mark`.
fn prompts(rig: &Rig, mark: usize, pane: &str) -> Vec<String> {
    rig.host.calls.lock().unwrap()[mark..]
        .iter()
        .filter(|(method, params)| method == "agent.prompt" && params["target"] == pane)
        .map(|(_, params)| params["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Waits until the wake lane has finished a pass after the current state.
fn settle_wake_lane(rig: &Rig) {
    let passes = rig.probe.registered_idle_events(Lane::Wakes);
    rig.probe.kick_registered(Lane::Wakes);
    wait_until("the wake lane to finish its pass", STEP, || {
        (rig.probe.registered_idle_events(Lane::Wakes) > passes).then_some(())
    });
}

/// (a) One daemon, an agy seat and a Claude seat, a require-ACK message for each. The agy seat
/// pulls (`PreInvocation`), the injected frame lands in its transcript and `Stop` confirms it:
/// the receipt settles with `cooperative_hook_delivery` / `transcript`. The Claude seat's
/// `watch` stream carries only its own message, stays connected, and its receipt settles later
/// by its own mod ack with `cooperative_mod_delivery`, untouched by the agy confirm.
#[test]
fn agy_pull_confirm_and_the_claude_watch_path_coexist_on_one_daemon() {
    let rig = Rig::new();
    let agy = Agy::join(&rig);
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();

    let for_claude = rig.send_ordinary("for the claude seat");
    let for_agy = agy.send(&rig, "for the agy seat", true);
    let line = watch.wait_item(&for_claude);
    assert_eq!(line["ack_required"], true, "{line}");

    // PreInvocation: pull -> one systemMessage page with the thread's pending messages (a message
    // is the thread's, whoever must ACK it).
    let out = agy.hook(&rig, "PreInvocation", "pre-invocation-inv1", TOP);
    let text = Agy::injected(&out).expect("a pull with a pending message injects");
    assert!(text.contains(&format!("message {for_agy} in ")), "{text}");
    assert!(text.contains("for the agy seat"), "{text}");
    assert_eq!(
        agy.receipt(&rig, &for_agy).0,
        "pending",
        "a pull settles nothing"
    );

    // Stop before the frame exists: nothing to confirm yet, nothing printed.
    let early = agy.hook(&rig, "Stop", "stop-no-tool-call", TOP);
    assert_eq!(Agy::injected(&early), None);
    assert_eq!(agy.receipt(&rig, &for_agy).0, "pending");

    // agy writes the frame; Stop finds it and confirms.
    agy.append_frame(&text);
    let stop = agy.hook(&rig, "Stop", "stop-no-tool-call", TOP);
    assert_eq!(Agy::injected(&stop), None);
    let (state, observation) = agy.receipt(&rig, &for_agy);
    assert_eq!(state, "acked");
    let observation = observation.expect("an ack observation");
    assert_eq!(
        observation["action_provenance"],
        "cooperative_hook_delivery"
    );
    assert_eq!(observation["via"], "transcript");

    // The Claude side is unaffected: the agy seat's confirm settled the agy seat's receipt
    // only; the Claude seat's receipt for its own message is still pending, its channel is still
    // live, and it settles later by its own mod ack.
    assert_eq!(rig.receipt_state(&for_claude), "pending");
    assert!(watch.running());
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    let acked = rig.ack_result(SESSION_B, "context", &for_claude);
    assert_eq!(acked["result"], "settled", "{acked}");
    assert_eq!(
        rig.ack_observation(&for_claude)["action_provenance"],
        "cooperative_mod_delivery"
    );
}

/// (b1) A Native `HookPull` seat: a lazy row never wakes a `nudge` seat; `wake_on: all` wakes it
/// once for that row, with the content-free wake text (the first `PreInvocation` of the woken turn
/// pulls the body), and the same frontier is not woken again.
#[test]
fn wake_on_all_wakes_a_native_hook_pull_seat_for_a_lazy_row_and_nudge_never() {
    let rig = Rig::new();
    let agy = Agy::join(&rig);

    // Default `nudge`: a lazy row is passive.
    let mark = rig.host.mark();
    let first = agy.send(&rig, "passive one", false);
    settle_wake_lane(&rig);
    std::thread::sleep(QUIET);
    assert_eq!(prompts(&rig, mark, PANE_G), Vec::<String>::new());
    assert_eq!(agy.lazy_state(&rig, &first), "pending");

    // `wake_on: all` re-judges the pending row: one wake, typed as the minimal wake-up text.
    agy.wake_on(&rig, "all");
    let mark_all = rig.host.mark();
    rig.wait_prompt(mark_all, PANE_G);
    assert_eq!(prompts(&rig, mark_all, PANE_G), vec![WAKE_UP.to_owned()]);
    // The lazy row itself is still delivered by the pull, not by the wake.
    assert_eq!(agy.lazy_state(&rig, &first), "pending");
    let out = agy.hook(&rig, "PreInvocation", "pre-invocation-inv1", TOP);
    let text = Agy::injected(&out).expect("the woken turn's first PreInvocation pulls the body");
    assert!(text.contains("passive one"), "{text}");

    // The same frontier is not woken twice, even after the ladder spacing.
    rig.advance(31_000);
    settle_wake_lane(&rig);
    std::thread::sleep(QUIET);
    assert_eq!(prompts(&rig, mark_all, PANE_G).len(), 1);

    // Back to `nudge`: a new lazy row does not wake.
    agy.wake_on(&rig, "nudge");
    let mark_nudge = rig.host.mark();
    agy.send(&rig, "passive two", false);
    rig.advance(31_000);
    settle_wake_lane(&rig);
    std::thread::sleep(QUIET);
    assert_eq!(prompts(&rig, mark_nudge, PANE_G), Vec::<String>::new());
}

/// (b2) A Claude seat with a live `Channel` registration and `wake_on: all`: a lazy row is
/// promoted on the stream (a `message` line instead of `lazy`, no ACK to run) and no native wake
/// reaches the pane. A `nudge` Claude seat keeps its plain `lazy` line.
#[test]
fn wake_on_all_promotes_a_lazy_row_on_a_live_channel_instead_of_a_native_wake() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();

    // `nudge`: the plain lazy line, no wake.
    let mark = rig.host.mark();
    let quiet = rig.send_lazy("plain lazy");
    let line = watch.wait_item(&quiet);
    assert_eq!(line["kind"], "lazy", "{line}");
    rig.assert_no_prompt(mark, QUIET);

    // `all`: promoted to a message line, still no ACK, still no native wake.
    rig.cli(Some((&rig.b, PANE_B)), &["wake-on", "all"])
        .data("wake-on all");
    let mark_all = rig.host.mark();
    let loud = rig.send_lazy("promoted lazy");
    let line = watch.wait_item(&loud);
    assert_eq!(line["kind"], "message", "{line}");
    assert_eq!(line["ack_required"], false, "{line}");
    assert_eq!(line["body"], "promoted lazy");
    rig.advance(31_000);
    rig.assert_no_prompt(mark_all, QUIET);
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
}

/// (c) A `/clear` rotation discards the pull ledger. The agy seat is offered `m1`; `/clear` binds
/// a new conversation (a new binding generation); the old offer is gone, so the new
/// conversation's first pull offers `m1` again and only its own transcript frame settles it. The
/// previous conversation's frame, still on disk, settles nothing.
#[test]
fn a_clear_rotation_discards_the_pull_ledger_and_reoffers_the_pending_message() {
    let rig = Rig::new();
    let agy = Agy::join(&rig);
    let message = agy.send(&rig, "survives the rotation", true);

    let out = agy.hook(&rig, "PreInvocation", "pre-invocation-inv1", TOP);
    let old_page = Agy::injected(&out).expect("the first pull offers the message");
    assert!(
        old_page.contains(&format!("message {message} in ")),
        "{old_page}"
    );
    // The old conversation's frame is on disk, but `/clear` happens before any Stop confirms it.
    agy.append_frame(&old_page);

    let cleared = agy.hook(&rig, "SessionStart", "session-start-after-clear", CLEAR);
    let startup =
        Agy::injected(&cleared).expect("the /clear lifecycle check-in prints its context");
    assert!(startup.starts_with("Use inbox"), "{startup}");
    assert_eq!(agy.receipt(&rig, &message).0, "pending");

    // The ledger was discarded with the generation: the new conversation is offered `m1` again.
    let again = agy.hook(&rig, "PreInvocation", "pre-invocation-inv1", CLEAR);
    let new_page = Agy::injected(&again).expect("a re-offer after the rotation");
    assert!(
        new_page.contains(&format!("message {message} in ")),
        "{new_page}"
    );

    // Stop of the new conversation: its transcript (the file agy writes for it) has the frame
    // only once the new page was injected; the earlier conversation's frame (same file here, by
    // the scripted transcript) is before the new offset and is not evidence.
    let stop = agy.hook(&rig, "Stop", "stop-no-tool-call", CLEAR);
    assert_eq!(Agy::injected(&stop), None);
    assert_eq!(
        agy.receipt(&rig, &message).0,
        "pending",
        "the previous conversation's frame settles nothing for the new one"
    );
    agy.append_frame(&new_page);
    let stop = agy.hook(&rig, "Stop", "stop-no-tool-call", CLEAR);
    assert_eq!(Agy::injected(&stop), None);
    let (state, observation) = agy.receipt(&rig, &message);
    assert_eq!(state, "acked");
    assert_eq!(
        observation.unwrap()["action_provenance"],
        "cooperative_hook_delivery"
    );
}
