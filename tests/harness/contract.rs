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

/// The registered operational parsers consume core captures independently of
/// diagnostic runtime/version witnesses. Optional Claude compact is excluded
/// from accepted_fixtures and remains tested under its captured qualification.
fn operational_parse_accepts(harness: &str, bytes: &[u8]) -> bool {
    use crate::harness::operational::{ClaudeContract, CodexContract};
    match harness {
        "claude" => {
            claude::parse_event_for_contract(bytes, "e", &ClaudeContract::registered()).is_ok()
        }
        _ => codex::parse_event_for_contract(bytes, "e", &CodexContract::registered()).is_ok(),
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
        let stripped = serde_json::to_vec(&strip_to_declared(&fx)).unwrap();
        for (mode, parse) in [
            ("diagnostic", parse_accepts as fn(&str, &[u8]) -> bool),
            ("operational", operational_parse_accepts),
        ] {
            assert!(
                parse(fx.harness, &fx.bytes),
                "{} is not accepted by its {mode} parser",
                fx.name
            );
            assert!(
                parse(fx.harness, &stripped),
                "{}: {mode} parser requires a field the contract does not declare (stripped payload: {})",
                fx.name,
                String::from_utf8_lossy(&stripped)
            );
        }
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
            for (mode, parse) in [
                ("diagnostic", parse_accepts as fn(&str, &[u8]) -> bool),
                ("operational", operational_parse_accepts),
            ] {
                assert!(
                    !parse(fx.harness, &bytes),
                    "{}: {mode} parser accepts a payload without declared required field {}",
                    fx.name,
                    spec.path
                );
            }
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

#[test]
fn runtime_build_key_covers_exact_descriptor_and_domain_hash_preserves_legacy_ids() {
    use crate::harness::{
        adapter::HarnessAdapter,
        runtime::{RuntimeDescriptor, RuntimeIdentity},
    };
    let descriptor = RuntimeDescriptor {
        release_version: None,
        source: "git".into(),
        base_version: Some("0.21.5".into()),
        derived_version: Some("0.21.5+3962.g37daf85".into()),
        commit: Some("37daf85b2ad0ee50ed45d7234dc47b7fa24cec09".into()),
        dirty: Some(false),
        distance: Some(3962),
    };
    let identity = RuntimeIdentity::build(descriptor.clone()).unwrap();
    assert_eq!(
        identity.descriptor.canonical_json(),
        r#"{"base_version":"0.21.5","commit":"37daf85b2ad0ee50ed45d7234dc47b7fa24cec09","derived_version":"0.21.5+3962.g37daf85","dirty":false,"distance":3962,"release_version":null,"schema_version":1,"source":"git"}"#
    );
    assert_eq!(
        identity.key,
        "build:013639f0250186e6516b34213a8fba3d64e5ce73f667e0a4feb471047d5507b0"
    );
    for change in 0..5 {
        let mut d = descriptor.clone();
        match change {
            0 => d.commit = Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            1 => d.dirty = Some(true),
            2 => d.source = "native_runtime".into(),
            3 => d.distance = Some(3963),
            _ => d.derived_version = Some("0.21.5+3963".into()),
        }
        assert_ne!(RuntimeIdentity::build(d).unwrap().key, identity.key);
    }
    let descriptor = crate::harness::claude::ClaudeAdapter.contracts()[0];
    assert_eq!(
        descriptor.legacy_contract_id(),
        contract_id(&crate::harness::claude::CONTRACT)
    );
    let id = descriptor.contract_id_v2().unwrap();
    let mut changed = descriptor;
    changed.domain_id = "different";
    assert_ne!(changed.contract_id_v2().unwrap(), id);
    changed = descriptor;
    changed.origin = crate::harness::evidence::EvidenceOrigin::NativeShapeObservation;
    assert_ne!(changed.contract_id_v2().unwrap(), id);
    changed = descriptor;
    changed.qualifications = &["qualified_runtime"];
    assert_ne!(changed.contract_id_v2().unwrap(), id);
    changed = descriptor;
    changed.required_milestones = &["tool"];
    assert_ne!(changed.contract_id_v2().unwrap(), id);
    assert_eq!(
        changed.legacy_contract_id(),
        descriptor.legacy_contract_id()
    );
}

// New discovery must not change the historical public command's exact bytes.
#[test]
fn adapters_local_dispatch_keeps_legacy_contract_id_bytes() {
    let mut legacy = Vec::new();
    crate::cli::run_in_pane(
        ["herdr-threads", "contract-id", "--json"],
        None,
        &mut legacy,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(legacy).unwrap(),
        "{\"claude\":\"3f860645de4c3363\",\"codex\":\"d3b98d74f26f7e2c\",\"normalize\":{\"claude\":{\"2.1.286 (Claude Code)\":\"2.1.286\"},\"codex\":{\"codex-cli 0.158.0\":\"0.158.0\"}}}\n"
    );
    let mut output = Vec::new();
    crate::cli::run_in_pane(["herdr-threads", "adapters", "--json"], None, &mut output).unwrap();
    let discovery = crate::harness::discovery::Discovery::parse(&output).unwrap();
    let registry = crate::harness::registry::builtins();
    assert_discovery_matches_registry(&discovery, registry);
    #[cfg(not(feature = "test-support"))]
    let actual: std::collections::BTreeSet<_> = discovery
        .adapters
        .iter()
        .map(|row| row.id.as_str())
        .collect();
    // These are controls for shipped declarations, not generic requirements
    // imposed on every adapter added to the registry.
    for id in ["claude", "codex", "hermes"] {
        assert_shipped_discovery_metadata(&discovery, id);
    }
    #[cfg(feature = "test-support")]
    assert_shipped_discovery_metadata(&discovery, "synthetic_fourth");
    let row = |id: &str| discovery.adapters.iter().find(|row| row.id == id).unwrap();
    for (id, pinned) in [
        ("claude", "3f860645de4c3363"),
        ("codex", "d3b98d74f26f7e2c"),
    ] {
        assert_eq!(row(id).legacy_contract_id.as_deref(), Some(pinned));
        assert!(row(id).contracts.iter().all(|domain| domain.id != pinned));
    }
    let hermes = row("hermes");
    assert_eq!(hermes.display_name, "Hermes");
    assert!(hermes.host_kinds.iter().any(|kind| kind == "hermes"));
    assert!(hermes.setup_scopes.iter().any(|kind| kind == "profile"));
    assert_eq!(
        registry.by_host_kind("hermes").unwrap().metadata().id,
        "hermes"
    );
    assert_eq!(registry.agent("hermes").unwrap().as_str(), "hermes");
    assert!(hermes.legacy_contract_id.is_none());
    let native = hermes
        .contracts
        .iter()
        .find(|d| d.domain == "native_callback")
        .unwrap();
    let bridge = hermes
        .contracts
        .iter()
        .find(|d| d.domain == "bridge_envelope")
        .unwrap();
    assert_ne!(native.id, bridge.id);
    assert_ne!(native.origin, bridge.origin);
    #[cfg(feature = "test-support")]
    {
        let fourth = row("synthetic_fourth");
        assert!(fourth.canary_strategy.is_none());
        assert!(fourth.contracts.iter().any(|d| d.domain == "synthetic"));
        assert!(
            fourth
                .host_kinds
                .iter()
                .any(|kind| kind == "synthetic_fourth_alias")
        );
        assert_eq!(
            registry
                .by_host_kind("synthetic_fourth_alias")
                .unwrap()
                .metadata()
                .id,
            fourth.id
        );
    }
    #[cfg(not(feature = "test-support"))]
    {
        assert!(!actual.contains("synthetic_fourth"));
        assert!(registry.agent("synthetic_fourth").is_err());
    }
}

// Shared by the actual CLI and independent test-local adapter producers.
fn assert_discovery_matches_registry(
    discovery: &crate::harness::discovery::Discovery,
    registry: &crate::harness::registry::Registry,
) {
    use crate::harness::adapter::SetupScopeKind;
    use std::collections::BTreeSet;
    let actual: BTreeSet<_> = discovery
        .adapters
        .iter()
        .map(|row| row.id.as_str())
        .collect();
    let registered: BTreeSet<_> = registry
        .registrations()
        .iter()
        .map(|r| r.metadata().id)
        .collect();
    assert_eq!(
        actual.len(),
        discovery.adapters.len(),
        "discovery IDs must be unique"
    );
    assert_eq!(actual, registered);
    for registration in registry.registrations() {
        let metadata = registration.metadata();
        let row = discovery
            .adapters
            .iter()
            .find(|row| row.id == metadata.id)
            .unwrap();
        assert_eq!(row.display_name, metadata.display_label);
        assert!(!row.display_name.is_empty());
        assert_eq!(row.host_kinds, metadata.host_kinds);
        let scopes: Vec<_> = metadata
            .setup_scopes
            .iter()
            .map(|kind| match kind {
                SetupScopeKind::ConfigRoot => "config_root",
                SetupScopeKind::Profile => "profile",
            })
            .collect();
        assert_eq!(row.setup_scopes, scopes);
        assert_eq!(row.legacy_contract_id, registration.legacy_contract_id());
        assert_eq!(
            row.canary_strategy,
            registration.canary_strategy().map(|p| p.descriptor())
        );
        let domains: BTreeSet<_> = row.contracts.iter().map(|d| d.domain.as_str()).collect();
        assert_eq!(domains.len(), row.contracts.len());
        assert_eq!(
            domains,
            registration
                .contracts()
                .iter()
                .map(|d| d.domain_id)
                .collect()
        );
        for descriptor in registration.contracts() {
            let domain = row
                .contracts
                .iter()
                .find(|d| d.domain == descriptor.domain_id)
                .unwrap();
            assert_eq!(domain.origin, descriptor.origin);
            assert_eq!(domain.id, descriptor.contract_id_v2().unwrap());
            assert_eq!(domain.required_milestones, descriptor.required_milestones);
            let events: BTreeSet<_> = domain.events.iter().map(|e| e.event.as_str()).collect();
            assert_eq!(events.len(), domain.events.len());
            assert_eq!(
                events,
                descriptor.events.iter().map(|e| e.native_event).collect()
            );
            for declared in descriptor.events {
                let event = domain
                    .events
                    .iter()
                    .find(|e| e.event == declared.native_event)
                    .unwrap();
                assert_eq!(event.milestone.as_deref(), declared.milestone);
                assert_eq!(event.always_send, declared.always_send);
            }
        }
        eprintln!(
            "discovery producer identity={} scopes={:?} domains={:?} canary={}",
            row.id,
            row.setup_scopes,
            domains,
            row.canary_strategy.is_some()
        );
    }
}

mod sparse_discovery_fixture {
    use crate::harness::adapter::*;
    use crate::protocol::time::CallBudget;

    pub struct Adapter(pub bool);
    static METADATA: AdapterMetadata = AdapterMetadata {
        id: "sparse",
        display_label: "Sparse",
        context_spelling: "Sparse",
        context_aliases: &[],
        executable: ExecutableLookup::Unsupported,
        host_kinds: &[],
        setup_scopes: &[],
        budget: EventBudgetPolicy {
            lifecycle_ms: 1,
            observer_ms: 1,
        },
        runtime_sources: &[],
    };
    static CONTRACT: super::HarnessContract = super::HarnessContract {
        harness: "sparse",
        discriminator: "event",
        events: &[],
    };
    static DOMAINS: &[ContractDescriptor] = &[ContractDescriptor {
        domain_id: "unavailable",
        origin: crate::harness::evidence::EvidenceOrigin::NativePayload,
        events: &[],
        required_milestones: &[],
        qualifications: &[],
        holding: crate::harness::evidence::AttributionHolding::Never,
        resumed_unavailable_reason: None,
        domain: ContractDomain::Native,
        contract: &CONTRACT,
    }];
    impl HarnessAdapter for Adapter {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            &METADATA
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            if self.0 { DOMAINS } else { &[] }
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            panic!("metadata discovery must not call observe_install")
        }
        fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            panic!("metadata discovery must not call admit")
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            panic!("metadata discovery must not call version_ladder")
        }
        fn classify(&self, _: &HookInput) -> ContractObservation {
            panic!("metadata discovery must not call classify")
        }
        fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            panic!("metadata discovery must not call decode")
        }
        fn encode(
            &self,
            _: &(),
            _: &DecodedEvent,
            _: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            panic!("metadata discovery must not call encode")
        }
        fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            panic!("metadata discovery must not call attribute_runtime")
        }
        fn setup(&self, _: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            panic!("metadata discovery must not call setup")
        }
        fn status(&self, _: &StatusRequest, _: &CallBudget) -> SetupStatus {
            panic!("metadata discovery must not call status")
        }
        fn unsetup(
            &self,
            _: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            panic!("metadata discovery must not call unsetup")
        }
    }
    pub fn registry(domain: bool) -> crate::harness::registry::Registry {
        use crate::harness::registry::{Registration, Registry};
        let adapter = Box::leak(Box::new(Adapter(domain)));
        Registry::new(Box::leak(Box::new([Registration::new(adapter)]))).unwrap()
    }
}

#[test]
fn adapters_sparse_optional_facilities_match_registered_producers() {
    assert_sparse_discovery(false);
}

#[test]
fn adapters_sparse_empty_domain_remains_unverified() {
    assert_sparse_discovery(true);
}

fn assert_sparse_discovery(domain: bool) {
    use crate::harness::discovery::{self, Discovery};
    let registry = sparse_discovery_fixture::registry(domain);
    let declaration = registry.registrations()[0].contracts();
    for descriptor in declaration {
        descriptor.validate().unwrap();
        assert!(!descriptor.verified(&[], &[]));
        assert!(!descriptor.verified(&["lifecycle", "tool"], &["same_runtime"]));
    }
    let bytes = discovery::render(&registry).unwrap();
    let parsed = Discovery::parse(bytes.as_bytes()).unwrap();
    assert!(parsed.adapters[0].host_kinds.is_empty());
    assert!(parsed.adapters[0].setup_scopes.is_empty());
    assert!(parsed.adapters[0].legacy_contract_id.is_none());
    assert!(parsed.adapters[0].canary_strategy.is_none());
    assert_eq!(parsed.adapters[0].contracts.len(), usize::from(domain));
    eprintln!(
        "legal sparse producer domain={domain} accepted by registry and discovery; verification unavailable"
    );
    assert_discovery_matches_registry(&parsed, &registry);
}

fn assert_shipped_discovery_metadata(discovery: &crate::harness::discovery::Discovery, id: &str) {
    let row = discovery.adapters.iter().find(|row| row.id == id).unwrap();
    assert!(!row.host_kinds.is_empty(), "{id}");
    assert!(!row.setup_scopes.is_empty(), "{id}");
    assert!(!row.contracts.is_empty(), "{id}");
    for domain in &row.contracts {
        assert!(
            !domain.required_milestones.is_empty(),
            "{id}/{}",
            domain.domain
        );
        assert!(!domain.events.is_empty(), "{id}/{}", domain.domain);
    }
}

#[test]
fn adapters_sparse_projection_rejects_missing_duplicate_invalid_and_producer_drift() {
    use crate::harness::discovery::{self, Discovery};
    let registry = sparse_discovery_fixture::registry(true);
    let bytes = discovery::render(&registry).unwrap();
    let parsed = Discovery::parse(bytes.as_bytes()).unwrap();
    let value: Value = serde_json::from_str(&bytes).unwrap();
    let mut missing = value.clone();
    missing["adapters"][0]
        .as_object_mut()
        .unwrap()
        .remove("canary_strategy");
    assert!(Discovery::parse(missing.to_string().as_bytes()).is_err());
    let mut duplicate = parsed.clone();
    duplicate.adapters.push(duplicate.adapters[0].clone());
    assert!(duplicate.validate().is_err());
    let mut invalid = parsed.clone();
    invalid.adapters[0].contracts[0]
        .required_milestones
        .push("undeclared".into());
    assert!(invalid.validate().is_err());
    let mut invalid = parsed.clone();
    invalid.adapters[0].contracts[0].domain = "Bad".into();
    assert!(invalid.validate().is_err());

    // Valid discovery shapes can still drift from their actual producers;
    // the same generic consumer used by the CLI must reject that drift.
    for change in 0..5 {
        let mut drift = parsed.clone();
        match change {
            0 => drift.adapters.clear(),
            1 => drift.adapters[0].host_kinds.push("invented".into()),
            2 => drift.adapters[0].contracts.clear(),
            3 => drift.adapters[0].contracts[0].id = "0000000000000000".into(),
            _ => {
                drift.adapters[0].contracts[0]
                    .events
                    .push(crate::harness::discovery::DomainEvent {
                        event: "Invented".into(),
                        milestone: None,
                        always_send: false,
                    })
            }
        }
        drift.validate().unwrap();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert_discovery_matches_registry(&drift, &registry);
            }))
            .is_err(),
            "producer drift {change} escaped the shared consumer"
        );
    }
}
