//! Payload contract declarations: canonical form, pinned ids, drift against
//! the production parsers, the violation classifier and version normalization.
use crate::harness::codex::{self, InstalledVersion};
use crate::harness::contract::{
    Classification, EventClass, EventContract, HarnessContract, JsonType, Malformed,
    canonical_json, classify, contract_for, contract_id, contract_ids_json, field,
    normalize_version,
};
use crate::harness::{claude, context::EventKind};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    harness: &'static str,
    name: String,
    event: String,
    bytes: Vec<u8>,
}

fn fixtures_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(sub)
}

fn accepted_fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    let mut add = |harness: &'static str, sub: &str, keep: &dyn Fn(&str) -> bool| {
        let mut names: Vec<String> = std::fs::read_dir(fixtures_dir(sub))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".json") && keep(n))
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no fixtures matched in {sub}");
        for name in names {
            let mut bytes = std::fs::read(fixtures_dir(sub).join(&name)).unwrap();
            let mut v: Value = serde_json::from_slice(&bytes).unwrap();
            // Live captures wrap the native stdin payload as `payload`.
            if let Some(payload) = v.get("payload").cloned() {
                bytes = serde_json::to_vec(&payload).unwrap();
                v = payload;
            }
            let event = v["hook_event_name"].as_str().unwrap().to_owned();
            out.push(Fixture {
                harness,
                name: format!("{sub}/{name}"),
                event,
                bytes,
            });
        }
    };
    let numbered = |n: &str| matches!(n.as_bytes().first(), Some(b'0')) && n.as_bytes()[1] <= b'4';
    add("claude", "claude-2.1.286", &numbered);
    add("codex", "codex-0.158.0", &numbered);
    add("codex", "codex-0.158.0-live", &|n| {
        n.starts_with("run")
            && (n.contains(".session-start")
                || n.contains(".subagent-start")
                || n.contains(".pre-tool-use"))
    });
    out
}

fn contract(harness: &str) -> &'static HarnessContract {
    contract_for(harness).unwrap()
}

fn parse_accepts(harness: &str, bytes: &[u8]) -> bool {
    match harness {
        "claude" => claude::parse_event("2.1.286", bytes, "e").is_ok(),
        _ => {
            let version = InstalledVersion::pinned_for_test();
            let ok = codex::parse_event_for_version(bytes, "e", &version);
            match ok {
                Ok(event) if event.kind == EventKind::Tool => {
                    codex::parse_tool_invocation(bytes, "e", &version).is_ok()
                }
                other => other.is_ok(),
            }
        }
    }
}

/// Keep only declared paths of the fixture's event (nested: dotted paths).
fn strip_to_declared(fx: &Fixture) -> Value {
    let c = contract(fx.harness);
    let event = c.events.iter().find(|e| e.event == fx.event).unwrap();
    let source: Value = serde_json::from_slice(&fx.bytes).unwrap();
    let mut out = json!({});
    for spec in event.fields {
        let mut cur = &source;
        let mut found = true;
        for key in spec.path.split('.') {
            match cur.get(key) {
                Some(v) => cur = v,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if !found {
            continue;
        }
        let mut target = &mut out;
        let keys: Vec<&str> = spec.path.split('.').collect();
        for key in &keys[..keys.len() - 1] {
            target = target
                .as_object_mut()
                .unwrap()
                .entry((*key).to_owned())
                .or_insert_with(|| json!({}));
        }
        target
            .as_object_mut()
            .unwrap()
            .insert(keys[keys.len() - 1].to_owned(), cur.clone());
    }
    out
}

fn remove_path(value: &mut Value, path: &str) {
    let keys: Vec<&str> = path.split('.').collect();
    let mut cur = value;
    for key in &keys[..keys.len() - 1] {
        cur = cur.get_mut(*key).unwrap();
    }
    cur.as_object_mut().unwrap().remove(keys[keys.len() - 1]);
}

fn claude_start() -> Vec<u8> {
    std::fs::read(fixtures_dir("claude-2.1.286").join("01-sessionstart-startup.json")).unwrap()
}

fn codex_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures_dir("codex-0.158.0").join(name)).unwrap()
}

fn with(bytes: &[u8], edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(bytes).unwrap();
    edit(&mut v);
    serde_json::to_vec(&v).unwrap()
}

#[test]
fn canonical_json_sorts_keys_events_and_fields() {
    static CONTRACT: HarnessContract = HarnessContract {
        harness: "h",
        discriminator: "kind",
        events: &[
            EventContract {
                event: "Zed",
                class: EventClass::Tool,
                fields: &[
                    field("z", JsonType::Object, false),
                    field("a.b", JsonType::StringOrNull, true),
                ],
            },
            EventContract {
                event: "Alpha",
                class: EventClass::Lifecycle,
                fields: &[field("q\"x", JsonType::Bool, true)],
            },
        ],
    };
    assert_eq!(
        canonical_json(&CONTRACT),
        concat!(
            r#"{"discriminator":"kind","events":["#,
            r#"{"class":"lifecycle","event":"Alpha","fields":[{"path":"q\"x","required":true,"type":"bool"}]},"#,
            r#"{"class":"tool","event":"Zed","fields":["#,
            r#"{"path":"a.b","required":true,"type":"string_or_null"},"#,
            r#"{"path":"z","required":false,"type":"object"}]}"#,
            r#"],"harness":"h"}"#
        )
    );
}

/// A change to a declared contract changes its id. That must be deliberate: it
/// bumps the contract rows of the evidence manifest. Update the literal only
/// together with them.
#[test]
fn contract_ids_are_pinned() {
    assert_eq!(contract_id(&claude::CONTRACT), "3f860645de4c3363");
    assert_eq!(contract_id(&codex::CONTRACT), "d3b98d74f26f7e2c");
}

#[test]
fn contract_id_is_16_lowercase_hex_and_differs_per_harness() {
    let (a, b) = (
        contract_id(&claude::CONTRACT),
        contract_id(&codex::CONTRACT),
    );
    for id in [&a, &b] {
        assert_eq!(id.len(), 16);
        assert!(
            id.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
    }
    assert_ne!(a, b);
}

#[test]
fn contract_ids_json_has_exact_keys_and_ids() {
    let all = contract_ids_json(None);
    let keys: Vec<&str> = all
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["claude", "codex", "normalize"]);
    assert_eq!(all["claude"], contract_id(&claude::CONTRACT));
    assert_eq!(all["codex"], contract_id(&codex::CONTRACT));
    assert_eq!(
        all["normalize"]["claude"]["2.1.286 (Claude Code)"],
        "2.1.286"
    );
    assert_eq!(all["normalize"]["codex"]["codex-cli 0.158.0"], "0.158.0");
    let one = contract_ids_json(Some("codex"));
    let keys: Vec<&str> = one
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["codex", "normalize"]);
    assert!(one["normalize"].get("claude").is_none());
}

#[test]
fn drift_parsers_require_no_undeclared_field() {
    for fx in accepted_fixtures() {
        assert!(
            parse_accepts(fx.harness, &fx.bytes),
            "{} is not accepted by its parser",
            fx.name
        );
        let stripped = serde_json::to_vec(&strip_to_declared(&fx)).unwrap();
        assert!(
            parse_accepts(fx.harness, &stripped),
            "{}: a parser requires a field the contract does not declare (stripped payload: {})",
            fx.name,
            String::from_utf8_lossy(&stripped)
        );
    }
}

#[test]
fn drift_every_declared_required_field_is_required_by_a_parser() {
    for fx in accepted_fixtures() {
        let c = contract(fx.harness);
        let event = c.events.iter().find(|e| e.event == fx.event).unwrap();
        let source: Value = serde_json::from_slice(&fx.bytes).unwrap();
        for spec in event.fields.iter().filter(|s| s.required) {
            let mut edited = source.clone();
            remove_path(&mut edited, spec.path);
            let bytes = serde_json::to_vec(&edited).unwrap();
            assert!(
                !parse_accepts(fx.harness, &bytes),
                "{}: parser accepts a payload without declared required field {}",
                fx.name,
                spec.path
            );
        }
    }
}

#[test]
fn classify_accepts_every_fixture() {
    for fx in accepted_fixtures() {
        let c = contract(fx.harness);
        for registered in [Some(fx.event.as_str()), None] {
            match classify(c, registered, &fx.bytes) {
                Classification::Ok { event } => assert_eq!(event, fx.event, "{}", fx.name),
                other => panic!("{} registered {registered:?}: {other:?}", fx.name),
            }
        }
    }
}

#[test]
fn classify_missing_required_field_is_violation() {
    let bytes = with(&claude_start(), |v| {
        v.as_object_mut().unwrap().remove("session_id");
    });
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &bytes),
        Classification::Violation {
            event: "SessionStart",
            field: "session_id"
        }
    );
    // A required string that is JSON null counts as missing.
    let bytes = with(&claude_start(), |v| v["source"] = Value::Null);
    assert_eq!(
        classify(&claude::CONTRACT, None, &bytes),
        Classification::Violation {
            event: "SessionStart",
            field: "source"
        }
    );
    // Nested path.
    let bytes = with(&codex_fixture("03-pretooluse-bash-root.json"), |v| {
        v["tool_input"] = json!({});
    });
    assert_eq!(
        classify(&codex::CONTRACT, Some("PreToolUse"), &bytes),
        Classification::Violation {
            event: "PreToolUse",
            field: "tool_input.command"
        }
    );
    // A required string-or-null key must still be present.
    let bytes = with(&codex_fixture("02-subagentstart.json"), |v| {
        v.as_object_mut().unwrap().remove("transcript_path");
    });
    assert_eq!(
        classify(&codex::CONTRACT, Some("SubagentStart"), &bytes),
        Classification::Violation {
            event: "SubagentStart",
            field: "transcript_path"
        }
    );
}

#[test]
fn classify_wrong_type_is_violation() {
    let bytes = with(&claude_start(), |v| v["session_id"] = json!(7));
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &bytes),
        Classification::Violation {
            event: "SessionStart",
            field: "session_id"
        }
    );
    let bytes = with(&claude_start(), |v| v["agent_id"] = json!(7));
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &bytes),
        Classification::Violation {
            event: "SessionStart",
            field: "agent_id"
        }
    );
    let bytes = with(&codex_fixture("02-subagentstart.json"), |v| {
        v["transcript_path"] = json!(["x"]);
    });
    assert_eq!(
        classify(&codex::CONTRACT, None, &bytes),
        Classification::Violation {
            event: "SubagentStart",
            field: "transcript_path"
        }
    );
    // Extra fields and optional null are not violations.
    let bytes = with(&claude_start(), |v| {
        v["agent_id"] = Value::Null;
        v["brand_new_field"] = json!({"x": 1});
    });
    assert!(matches!(
        classify(&claude::CONTRACT, None, &bytes),
        Classification::Ok { .. }
    ));
}

#[test]
fn classify_truncated_stdin_is_malformed() {
    let full = claude_start();
    assert_eq!(
        classify(
            &claude::CONTRACT,
            Some("SessionStart"),
            &full[..full.len() / 2]
        ),
        Classification::Malformed(Malformed::NotJson)
    );
}

#[test]
fn classify_unparseable_json_is_malformed() {
    assert_eq!(
        classify(&claude::CONTRACT, None, b"{not json"),
        Classification::Malformed(Malformed::NotJson)
    );
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), b""),
        Classification::Malformed(Malformed::NotJson)
    );
}

#[test]
fn classify_non_object_is_malformed() {
    for body in [&b"[1,2]"[..], b"\"x\"", b"7", b"null"] {
        assert_eq!(
            classify(&claude::CONTRACT, Some("SessionStart"), body),
            Classification::Malformed(Malformed::NotObject)
        );
    }
}

#[test]
fn classify_oversized_is_malformed() {
    let big = with(&claude_start(), |v| v["pad"] = json!("x".repeat(70_000)));
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &big),
        Classification::Malformed(Malformed::TooLarge)
    );
}

#[test]
fn classify_unknown_event_is_malformed() {
    let stop = with(&claude_start(), |v| v["hook_event_name"] = json!("Stop"));
    assert_eq!(
        classify(&claude::CONTRACT, None, &stop),
        Classification::Malformed(Malformed::UnknownEvent)
    );
    assert_eq!(
        classify(&claude::CONTRACT, Some("Stop"), &claude_start()),
        Classification::Malformed(Malformed::UnknownEvent)
    );
}

#[test]
fn renamed_discriminator_under_registered_session_start_is_violation() {
    let bytes = with(&claude_start(), |v| {
        let o = v.as_object_mut().unwrap();
        let name = o.remove("hook_event_name").unwrap();
        o.insert("hookEventName".into(), name);
    });
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &bytes),
        Classification::Violation {
            event: "SessionStart",
            field: "hook_event_name"
        }
    );
    assert_eq!(
        classify(&claude::CONTRACT, None, &bytes),
        Classification::Malformed(Malformed::UnknownEvent)
    );
    let non_string = with(&claude_start(), |v| v["hook_event_name"] = json!(3));
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &non_string),
        Classification::Violation {
            event: "SessionStart",
            field: "hook_event_name"
        }
    );
}

#[test]
fn registered_event_wins_over_discriminator() {
    let pre =
        std::fs::read(fixtures_dir("claude-2.1.286").join("02-pretooluse-bash-root.json")).unwrap();
    assert_eq!(
        classify(&claude::CONTRACT, Some("SessionStart"), &pre),
        Classification::Violation {
            event: "SessionStart",
            field: "hook_event_name"
        }
    );
}

#[test]
fn codex_fork_session_start_satisfies_the_contract() {
    let bytes = codex_fixture("05-sessionstart-fork.json");
    assert_eq!(
        classify(&codex::CONTRACT, Some("SessionStart"), &bytes),
        Classification::Ok {
            event: "SessionStart"
        }
    );
    assert!(!parse_accepts("codex", &bytes));
}

#[test]
fn normalize_version_table() {
    let cases: [(&str, &str, Option<&str>); 9] = [
        ("claude", "2.1.286 (Claude Code)\n", Some("2.1.286")),
        ("claude", "2.1.286", Some("2.1.286")),
        ("codex", "codex-cli 0.158.0", Some("0.158.0")),
        ("codex", "0.158.0\n", Some("0.158.0")),
        ("codex", "codex-cli 0.160.0-alpha.1", None),
        ("claude", "2.1", None),
        ("claude", "02.1.3", None),
        ("claude", "codex-cli 0.158.0", None),
        ("gemini", "1.0.0", None),
    ];
    for (harness, raw, want) in cases {
        assert_eq!(
            normalize_version(harness, raw).as_deref(),
            want,
            "{harness} {raw:?}"
        );
    }
}
