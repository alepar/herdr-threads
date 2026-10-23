//! ht-piggyback: a top-level agent without turn delivery (a plain-CLI Codex binding) sees one
//! pending-count line on top of its other text commands; nothing else does, and the line never
//! ACKs or advances anything. Real daemon, isolated Herdr, the real executable.
use super::session_bindings_smoke::{Agent, Smoke, joined_thread};
use serde_json::json;

const HINT: &str = "[herdr-threads] ";

fn hint(stdout: &str) -> Option<&str> {
    stdout.lines().next().filter(|line| line.starts_with(HINT))
}

#[test]
fn hookless_caller_sees_pending_count_on_top_of_text_commands() {
    let Some(mut smoke) = Smoke::new("hookless_caller_sees_pending_count_on_top_of_text_commands")
    else {
        return;
    };
    let start = |session: &str| {
        json!({
            "hook_event_name": "SessionStart", "session_id": session, "source": "startup",
            "turn_id": "turn-1",
        })
    };
    let sender_pane = smoke.pane();
    let sender = Agent::codex("codex-sender", &sender_pane).thread("codex-sender");
    assert_eq!(smoke.hook(&sender, &start("codex-sender")).code, 0);
    // Hook-registered (`cooperative_top_level`): turn hooks deliver its mail.
    let hooked_pane = smoke.pane();
    let hooked_seat = smoke.seat(&hooked_pane);
    let hooked = Agent::codex("codex-hooked", &hooked_pane).thread("codex-hooked");
    assert_eq!(smoke.hook(&hooked, &start("codex-hooked")).code, 0);
    // Plain CLI only (`cooperative_env_session`): no hook ever ran for it.
    let plain_pane = smoke.pane();
    let plain_seat = smoke.seat(&plain_pane);
    let plain = Agent::codex("codex-plain", &plain_pane).thread("codex-plain");

    let quiet = smoke.text(&plain, &["thread", "list"]);
    assert_eq!(quiet.code, 0, "{}", quiet.text());
    assert_eq!(
        hint(&quiet.stdout),
        None,
        "nothing pending: {}",
        quiet.text()
    );

    let thread = joined_thread(&smoke, &sender, &plain, &plain_seat);
    // The accept enrolled it by a plain call.
    let binding = smoke.binding(&plain_seat, "codex", "codex-plain", "");
    assert_eq!(binding.provenance, "cooperative_env_session", "{binding:?}");
    smoke
        .cli(&sender, &["invite", &thread, "--seat", &hooked_seat])
        .data();
    smoke.cli(&hooked, &["accept", &thread]).data();
    smoke
        .cli(&sender, &["send", &thread, "--body", "piggyback probe"])
        .data();

    // On top of an ordinary text command: the count and the caller's own `inbox`, with its
    // routing flags.
    let listed = smoke.text(&plain, &["thread", "list"]);
    assert_eq!(listed.code, 0, "{}", listed.text());
    let mut inbox_argv = vec!["herdr-threads".to_owned()];
    inbox_argv.extend(smoke.routing());
    inbox_argv.push("inbox".into());
    let expected = format!("{HINT}1 pending: {}", inbox_argv.join(" "));
    assert_eq!(
        hint(&listed.stdout),
        Some(expected.as_str()),
        "{}",
        listed.text()
    );
    assert_eq!(listed.stdout.matches(HINT).count(), 1, "{}", listed.text());
    // It is read-only: the next command still sees the same count.
    let again = smoke.text(&plain, &["thread", "show", &thread]);
    assert_eq!(
        hint(&again.stdout),
        Some(expected.as_str()),
        "{}",
        again.text()
    );

    // A seat-acting command (the cooperative path) gets it too, before its own output.
    let sent = smoke.text(&plain, &["send", &thread, "--body", "busy, reading later"]);
    assert_eq!(sent.code, 0, "{}", sent.text());
    assert_eq!(
        hint(&sent.stdout),
        Some(expected.as_str()),
        "{}",
        sent.text()
    );

    // Never on machine output.
    let machine = smoke.cli(&plain, &["thread", "list"]);
    machine.data();
    assert!(!machine.stdout.contains(HINT), "{}", machine.text());
    // Never for a subagent of that binding.
    let child = Agent::codex("codex-plain", &plain_pane).thread("plain-child");
    let read = smoke.text(&child, &["thread", "list"]);
    assert_eq!(read.code, 0, "{}", read.text());
    assert_eq!(hint(&read.stdout), None, "{}", read.text());
    // Never for a binding with turn hooks, though its mail is pending too.
    let served = smoke.text(&hooked, &["thread", "list"]);
    assert_eq!(served.code, 0, "{}", served.text());
    assert_eq!(hint(&served.stdout), None, "{}", served.text());
    assert!(
        smoke
            .text(&hooked, &["inbox"])
            .stdout
            .contains("piggyback probe"),
        "the hooked binding had the message pending"
    );

    // `inbox` shows the rows themselves, with no count line; once shown, the line is gone.
    let inbox = smoke.text(&plain, &["inbox"]);
    assert_eq!(inbox.code, 0, "{}", inbox.text());
    assert_eq!(hint(&inbox.stdout), None, "{}", inbox.text());
    assert!(inbox.stdout.contains("piggyback probe"), "{}", inbox.text());
    let after = smoke.text(&plain, &["thread", "list"]);
    assert_eq!(hint(&after.stdout), None, "{}", after.text());
}
