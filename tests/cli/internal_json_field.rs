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
