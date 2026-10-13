use super::*;

fn field(input: &str, path: &str) -> Result<String, i32> {
    let mut out = Vec::new();
    match json_field(input, path, &mut out) {
        Ok(()) => Ok(String::from_utf8(out).unwrap()),
        Err(RunError::Exit(code)) => Err(code),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

// Kills: printing the JSON-quoted string, or only top-level keys.
#[test]
fn extracts_a_nested_string() {
    let json = r#"{"result":{"log":{"log_id":"plugin-log-7","status":"running"}}}"#;
    assert_eq!(field(json, "result.log.log_id").unwrap(), "plugin-log-7\n");
}

// Kills: array indexes unsupported, or numbers/bools rendered differently.
#[test]
fn extracts_a_number_and_bool() {
    let json = r#"{"result":{"logs":[{"exit_code":0},{"exit_code":3,"ok":true}]}}"#;
    assert_eq!(field(json, "result.logs.1.exit_code").unwrap(), "3\n");
    assert_eq!(field(json, "result.logs.1.ok").unwrap(), "true\n");
    assert_eq!(field(json, "result.logs.0.exit_code").unwrap(), "0\n");
}

// Kills: absent paths printing an empty line with success, or an index past the end succeeding.
#[test]
fn missing_path_exits_1() {
    let json = r#"{"a":{"b":1},"l":[1]}"#;
    assert_eq!(field(json, "a.c"), Err(1));
    assert_eq!(field(json, "a.b.c"), Err(1));
    assert_eq!(field(json, "l.1"), Err(1));
}

// Kills: invalid input conflated with an absent field (exit 1).
#[test]
fn invalid_json_exits_2() {
    assert_eq!(field("{\"a\":", "a"), Err(2));
    assert_eq!(field("", "a"), Err(2));
}

// Kills: the helper appearing in the public help text.
#[test]
fn is_hidden_from_help() {
    let mut out = Vec::new();
    crate::cli::run_in_pane(["herdr-threads", "--help"], None, &mut out).unwrap();
    let help = String::from_utf8(out).unwrap();
    assert!(!help.contains("internal"), "{help}");
    assert!(!help.contains("json-field"), "{help}");
}

fn keys(input: &str, path: &str) -> Result<String, i32> {
    let mut out = Vec::new();
    match json_field_with(input, path, true, &mut out) {
        Ok(()) => Ok(String::from_utf8(out).unwrap()),
        Err(RunError::Exit(code)) => Err(code),
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

// Kills: keys printed JSON-quoted, keys with punctuation/spaces mangled, a key holding a newline
// splitting into two lines, or a non-object value printing anything.
#[test]
fn keys_mode_prints_object_keys_exactly_one_per_line() {
    let json = r#"{"profiles":{"codex":{"team.one":{},"my profile":{},"retired@account":1,"-dash":{},"bad\nkey":{},"tab\tkey":{}},"list":[1]}}"#;
    let printed = keys(json, "profiles.codex").unwrap();
    let mut lines: Vec<&str> = printed.lines().collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        ["-dash", "my profile", "retired@account", "team.one"]
    );
    // Absent (1) is told apart from present but not an object (3) and from invalid JSON (2).
    assert_eq!(keys(json, "profiles.list"), Err(3));
    assert_eq!(keys(r#"{"p":{"codex":null}}"#, "p.codex"), Err(3));
    assert_eq!(keys(r#"{"p":{"codex":"x"}}"#, "p.codex"), Err(3));
    assert_eq!(keys(json, "profiles.nope"), Err(1));
    assert_eq!(keys(r#"{"v":2}"#, "profiles.codex"), Err(1));
    assert_eq!(keys(r#"{"a":{}}"#, "a").unwrap(), "");
    assert_eq!(keys("{", "a"), Err(2));
}
