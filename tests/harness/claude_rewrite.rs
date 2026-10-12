use super::rewrite_command;
use crate::protocol::ids::AgentId;

fn rw(line: &str) -> Option<String> {
    rewrite_command(line, &AgentId::new("a1"))
}

fn some(line: &str, expected: &str) {
    assert_eq!(rw(line).as_deref(), Some(expected), "{line}");
}

fn none(line: &str) {
    assert_eq!(rw(line), None, "{line}");
}

#[test]
fn rewrites_a_plain_command_word() {
    some("ht inbox", "ht --agent-id=a1 inbox");
    some(
        "herdr-threads send t \"x y\"",
        "herdr-threads --agent-id=a1 send t \"x y\"",
    );
    some("ht", "ht --agent-id=a1");
}

#[test]
fn rewrites_every_simple_command_of_a_compound_line() {
    some("cd x && ht inbox", "cd x && ht --agent-id=a1 inbox");
    some("git log; ht inbox", "git log; ht --agent-id=a1 inbox");
    some("ht a | ht b", "ht --agent-id=a1 a | ht --agent-id=a1 b");
    some("false || ht x", "false || ht --agent-id=a1 x");
    some("sleep 1 & ht x", "sleep 1 & ht --agent-id=a1 x");
    some("echo hi\nht x", "echo hi\nht --agent-id=a1 x");
}

#[test]
fn skips_assignments_and_wrappers() {
    some(
        "FOO=1 timeout 5 ht send t m",
        "FOO=1 timeout 5 ht --agent-id=a1 send t m",
    );
    some(
        "nice -n 5 env A=1 nohup ht x",
        "nice -n 5 env A=1 nohup ht --agent-id=a1 x",
    );
    some("command ht x", "command ht --agent-id=a1 x");
    some("time -p ht x", "time -p ht --agent-id=a1 x");
    some("env -i -u HOME ht x", "env -i -u HOME ht --agent-id=a1 x");
    some(
        "timeout --signal=KILL 5s ht x",
        "timeout --signal=KILL 5s ht --agent-id=a1 x",
    );
    some(
        "timeout -s KILL -k 1 5 ht x",
        "timeout -s KILL -k 1 5 ht --agent-id=a1 x",
    );
    some("nice -5 ht x", "nice -5 ht --agent-id=a1 x");
    some("A=\"b c\" ht x", "A=\"b c\" ht --agent-id=a1 x");
}

#[test]
fn command_word_in_quoted_arguments_is_not_a_command() {
    some(
        "echo 'a; ht b' && ht c",
        "echo 'a; ht b' && ht --agent-id=a1 c",
    );
    some(
        "echo \"x && ht y\"; ht z",
        "echo \"x && ht y\"; ht --agent-id=a1 z",
    );
    none("echo 'a; ht b'");
}

#[test]
fn unrecognized_shapes_stay_untouched() {
    for line in [
        "command -v ht",
        "eval 'ht x'",
        "echo ht",
        "\"ht\" inbox",
        "/usr/local/bin/ht inbox",
        "bash -c 'ht x'",
        "x=$(ht inbox)",
        "echo `ht inbox`",
        "ht --agent-id=a1 inbox",
        "ht --agent-id a1 inbox",
        "echo 'unbalanced",
        "echo \"unbalanced",
        "echo $(unbalanced",
        "cat <<EOF\nht x\nEOF",
        "# ht x",
        "",
        "   ",
    ] {
        none(line);
    }
}

#[test]
fn already_rewritten_commands_do_not_double_up() {
    none("ht --agent-id=a1 a | ht --agent-id=a1 b");
    some(
        "ht --agent-id=a1 a | ht b",
        "ht --agent-id=a1 a | ht --agent-id=a1 b",
    );
}

#[test]
fn agent_id_is_shell_quoted_when_needed() {
    let id = AgentId::parse("a'c").unwrap();
    assert_eq!(
        rewrite_command("ht x", &id).as_deref(),
        Some("ht --agent-id='a'\\''c' x")
    );
    let id = AgentId::parse("a b").ok();
    assert!(id.is_none(), "space is not ASCII-graphic");
    let id = AgentId::parse("a$b;c").unwrap();
    assert_eq!(
        rewrite_command("ht x", &id).as_deref(),
        Some("ht --agent-id='a$b;c' x")
    );
    let id = AgentId::parse("agent-7_x.y:z@w+1").unwrap();
    assert_eq!(
        rewrite_command("ht x", &id).as_deref(),
        Some("ht --agent-id=agent-7_x.y:z@w+1 x")
    );
}

#[test]
fn non_ascii_bytes_elsewhere_survive() {
    some("echo 雪 && ht 雪", "echo 雪 && ht --agent-id=a1 雪");
}
