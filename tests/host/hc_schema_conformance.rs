//! Leaf (a) of the real-core smoke: every `fake_hc` reply conforms to the
//! result schema of its method in the vendored `hc-0.1.15.openrpc.json`
//! (required fields present, types match, extra fields tolerated), and every
//! event notification it emits conforms to the shapes in herdr-core
//! `schema/README.md` (encoded below as tables). This always ships; it is
//! also the fallback when the real-core smoke cannot. Evidence, not a version
//! gate: extra fields and unknown event types pass.

use herdr_threads::test_support::fake_hc::{FakeHcState, FakeTerminal, SERVED_METHODS};
use serde_json::{Value, json};
use std::collections::BTreeSet;

// ---- a small validator over the JSON Schema subset the vendored schema uses

fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => {
            let path = r.strip_prefix("#/").unwrap_or_else(|| panic!("ref {r}"));
            let target = path
                .split('/')
                .fold(root, |v, k| v.get(k).unwrap_or_else(|| panic!("ref {r}")));
            resolve(root, target)
        }
        None => schema,
    }
}

fn type_matches(name: &str, value: &Value) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        other => panic!("unsupported schema type {other}"),
    }
}

/// Validates `value` against `schema` (with `$ref`s into `root`); every
/// violation is returned with its path.
fn validate(root: &Value, schema: &Value, value: &Value, path: &str) -> Vec<String> {
    if schema.as_bool() == Some(true) {
        return Vec::new();
    }
    let schema = resolve(root, schema);
    let mut errors = Vec::new();
    let branches = |key: &str| -> Option<Vec<bool>> {
        schema.get(key).and_then(Value::as_array).map(|list| {
            list.iter()
                .map(|s| validate(root, s, value, path).is_empty())
                .collect()
        })
    };
    if let Some(ok) = branches("anyOf")
        && !ok.iter().any(|b| *b)
    {
        errors.push(format!("{path}: matches no anyOf branch"));
    }
    if let Some(ok) = branches("oneOf")
        && ok.iter().filter(|b| **b).count() != 1
    {
        errors.push(format!("{path}: does not match exactly one oneOf branch"));
    }
    if let Some(ty) = schema.get("type") {
        let names: Vec<&str> = match ty {
            Value::String(s) => vec![s.as_str()],
            Value::Array(list) => list.iter().filter_map(Value::as_str).collect(),
            other => panic!("type {other}"),
        };
        if !names.iter().any(|n| type_matches(n, value)) {
            errors.push(format!("{path}: expected {names:?}, got {value}"));
            return errors;
        }
    }
    if let Some(variants) = schema.get("enum").and_then(Value::as_array)
        && !variants.contains(value)
    {
        errors.push(format!("{path}: {value} not in enum"));
    }
    if let Some(n) = value.as_i64() {
        if let Some(min) = schema.get("minimum").and_then(Value::as_i64)
            && n < min
        {
            errors.push(format!("{path}: {n} below minimum {min}"));
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_i64)
            && n > max
        {
            errors.push(format!("{path}: {n} above maximum {max}"));
        }
    }
    if let Some(map) = value.as_object() {
        for key in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !map.contains_key(key) {
                errors.push(format!("{path}: missing required field {key}"));
            }
        }
        let props = schema.get("properties").and_then(Value::as_object);
        for (key, v) in map {
            let child = format!("{path}.{key}");
            if let Some(sub) = props.and_then(|p| p.get(key)) {
                errors.extend(validate(root, sub, v, &child));
            } else if let Some(extra) = schema.get("additionalProperties")
                && extra.is_object()
            {
                errors.extend(validate(root, extra, v, &child));
            } // other unknown fields are tolerated
        }
    }
    if let (Some(items), Some(list)) = (schema.get("items"), value.as_array()) {
        for (i, v) in list.iter().enumerate() {
            errors.extend(validate(root, items, v, &format!("{path}[{i}]")));
        }
    }
    errors
}

fn openrpc() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/hc/hc-0.1.15.openrpc.json"
    ))
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn method_of<'a>(root: &'a Value, name: &str) -> &'a Value {
    root["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == name)
        .unwrap_or_else(|| panic!("{name} is not in the vendored schema"))
}

fn result_schema<'a>(root: &'a Value, method: &str) -> &'a Value {
    &method_of(root, method)["result"]["schema"]
}

// ---- driving the fake

fn request(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string()
}

/// The reply lines of one request, parsed.
fn call(state: &FakeHcState, method: &str, params: Value) -> Vec<Value> {
    state
        .respond(&request(method, params))
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The final response (the line with an `id`) of a call.
fn response(lines: &[Value]) -> &Value {
    lines.iter().find(|l| l.get("id").is_some()).unwrap()
}

/// A world with a live terminal carrying a status and labels, a working one,
/// a finished one, a kept one waiting for a new shell, a doc, and every
/// event type the fake can emit in its log.
fn rich_world(unknown_fields: bool) -> FakeHcState {
    let state = FakeHcState::new();
    state.add_terminal(FakeTerminal::live("t1"));
    state.set_status(
        "t1",
        Some(json!({"state": "working", "app": "claude", "msg": "m", "progress": 40})),
    );
    state.set_turn("t1", 3);
    state.set_labels("t1", &[("session", "main"), ("role", "a")]);
    state.set_pid("t1", Some(7));
    state.add_terminal(FakeTerminal::live("t2"));
    state.finish("t2", "exited", Some(0), None);
    state.add_terminal(FakeTerminal::live("t3"));
    state.set_keep("t3", true);
    state.finish("t3", "holder_lost", None, None);
    state.add_terminal(FakeTerminal::live("t4"));
    state.finish("t4", "killed", None, Some(9));
    state.prune("t4");
    state.add_terminal(FakeTerminal::live("t5"));
    state.set_status("t5", Some(json!({"state": "idle"})));
    state.set_status("t5", None);
    state.set_doc("herdr-app", json!({"names": {"t1": "pane"}}), 5);
    state.set_unknown_fields(unknown_fields);
    state
}

/// One reply per served method (the `events.subscribe` stream included), with
/// the method name, for a world.
fn replies(state: &FakeHcState) -> Vec<(&'static str, Vec<Value>)> {
    vec![
        ("hello", call(state, "hello", json!({}))),
        ("server.status", call(state, "server.status", json!({}))),
        (
            "terminal.get",
            call(state, "terminal.get", json!({"terminal": "t1"})),
        ),
        (
            "terminal.get",
            call(state, "terminal.get", json!({"terminal": "t2"})),
        ),
        (
            "terminal.get",
            call(state, "terminal.get", json!({"terminal": "t3"})),
        ),
        (
            "events.subscribe",
            call(state, "events.subscribe", json!({})),
        ),
        (
            "events.subscribe",
            call(state, "events.subscribe", json!({"labels": {"role": "a"}})),
        ),
        (
            "doc.get",
            call(state, "doc.get", json!({"name": "herdr-app"})),
        ),
        ("doc.get", call(state, "doc.get", json!({"name": "absent"}))),
    ]
}

#[test]
fn validator_semantics() {
    let root = json!({"components": {"schemas": {"T": {
        "type": "object", "required": ["a"],
        "properties": {"a": {"type": "integer", "minimum": 0, "maximum": 10},
                       "b": {"anyOf": [{"type": "string"}, {"type": "null"}]},
                       "c": {"items": {"$ref": "#/components/schemas/T"}, "type": "array"},
                       "m": {"type": "object", "additionalProperties": {"type": "string"}}}}}}});
    let t = json!({"$ref": "#/components/schemas/T"});
    let ok = |v: Value| validate(&root, &t, &v, "$").is_empty();
    assert!(
        ok(json!({"a": 1, "b": null, "extra": [1]})),
        "extra tolerated"
    );
    assert!(ok(json!({"a": 1, "c": [{"a": 2}], "m": {"k": "v"}})));
    assert!(!ok(json!({"b": "x"})), "missing required");
    assert!(!ok(json!({"a": "1"})), "wrong type");
    assert!(!ok(json!({"a": 1.5})), "float is not an integer");
    assert!(!ok(json!({"a": 11})), "above maximum");
    assert!(!ok(json!({"a": -1})), "below minimum");
    assert!(!ok(json!({"a": 1, "b": 3})), "anyOf");
    assert!(!ok(json!({"a": 1, "c": [{"b": "x"}]})), "nested ref");
    assert!(!ok(json!({"a": 1, "m": {"k": 1}})), "additionalProperties");
    assert!(!ok(json!([])), "not an object");
    let one = json!({"oneOf": [{"type": "integer"}, {"type": "number"}]});
    assert!(
        validate(&root, &one, &json!(1), "$").len() == 1,
        "two branches match"
    );
    assert!(validate(&root, &one, &json!(1.5), "$").is_empty());
}

#[test]
fn every_served_method_reply_conforms() {
    let root = openrpc();
    let mut seen = BTreeSet::new();
    for unknown in [false, true] {
        let state = rich_world(unknown);
        for (method, lines) in replies(&state) {
            let reply = response(&lines);
            assert!(reply.get("error").is_none(), "{method}: {reply}");
            let errors = validate(
                &root,
                result_schema(&root, method),
                &reply["result"],
                method,
            );
            assert!(
                errors.is_empty(),
                "{method} (unknown={unknown}): {errors:#?}"
            );
            seen.insert(method);
        }
    }
    let served: BTreeSet<&str> = SERVED_METHODS.iter().copied().collect();
    assert_eq!(seen, served, "a served method was never validated");
}

#[test]
fn replies_conform_in_terminal_state_variants() {
    // Every terminal the rich world holds, by address, in every state.
    let root = openrpc();
    let state = rich_world(false);
    let mut states = BTreeSet::new();
    for address in ["t1", "t2", "t3", "t5"] {
        let lines = call(&state, "terminal.get", json!({"terminal": address}));
        let result = &response(&lines)["result"];
        let errors = validate(&root, result_schema(&root, "terminal.get"), result, address);
        assert!(errors.is_empty(), "{address}: {errors:#?}");
        states.insert((
            result["state"].as_str().unwrap().to_owned(),
            result["status"].is_null(),
        ));
    }
    assert!(states.contains(&("live".into(), false)), "{states:?}");
    assert!(states.contains(&("live".into(), true)), "{states:?}");
    assert!(states.contains(&("finished".into(), true)), "{states:?}");
}

#[test]
fn error_replies_have_the_wire_error_shape_and_declared_kinds() {
    let root = openrpc();
    let state = rich_world(false);
    let lines = call(&state, "terminal.get", json!({"terminal": "t9"}));
    let error = &response(&lines)["error"];
    assert!(response(&lines).get("result").is_none());
    let shape = json!({"type": "object", "required": ["code", "message", "data"],
        "properties": {"code": {"type": "integer"}, "message": {"type": "string"},
                       "data": {"type": "object", "required": ["kind"],
                                "properties": {"kind": {"type": "string"}}}}});
    let errors = validate(&root, &shape, error, "error");
    assert!(errors.is_empty(), "{errors:#?}");
    let declared: Vec<(i64, &str)> = method_of(&root, "terminal.get")["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["code"].as_i64().unwrap(),
                e["data"]["kind"].as_str().unwrap(),
            )
        })
        .collect();
    let got = (
        error["code"].as_i64().unwrap(),
        error["data"]["kind"].as_str().unwrap(),
    );
    assert!(
        declared.contains(&got),
        "{got:?} not declared: {declared:?}"
    );
}

// ---- event notification shapes (herdr-core schema/README.md, "Events")

const TERMINAL: &str = "#/components/schemas/Terminal";

fn string() -> Value {
    json!({"type": "string"})
}

fn int_or_null() -> Value {
    json!({"type": ["integer", "null"]})
}

fn status_or_null() -> Value {
    json!({"anyOf": [{"$ref": "#/components/schemas/Status"}, {"type": "null"}]})
}

fn object_data(fields: &[(&str, Value)]) -> Value {
    let props: serde_json::Map<String, Value> = fields
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect();
    let required: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
    json!({"type": "object", "properties": props, "required": required})
}

fn end_data() -> Value {
    object_data(&[
        ("reason", string()),
        ("exit_code", int_or_null()),
        ("signal", int_or_null()),
        ("finished_ms", int_or_null()),
    ])
}

/// One README event type: the full `data` shape, and the `data` keys this
/// fake emits for it today (`fake_keys`: the fake's events are deliberately
/// thin, so the table pins exactly what it carries and each carried field
/// is held to the README type; a change to the fake must update the pin).
struct Shape {
    kind: &'static str,
    data: Value,
    fake_keys: &'static [&'static str],
}

fn shapes() -> Vec<Shape> {
    let s = |kind, data, fake_keys| Shape {
        kind,
        data,
        fake_keys,
    };
    vec![
        s("created", json!({"$ref": TERMINAL}), &[]),
        s("lost", end_data(), &["reason"]),
        s("restarted", json!({"$ref": TERMINAL}), &[]),
        s(
            "keep",
            object_data(&[("keep", json!({"type": "boolean"}))]),
            &[],
        ),
        s("finished", end_data(), &["reason"]),
        s("removed", json!({"type": "object", "properties": {}}), &[]),
        s(
            "status",
            object_data(&[
                ("status", status_or_null()),
                (
                    "records",
                    json!({"type": "object",
                           "additionalProperties": {"$ref": "#/components/schemas/Status"}}),
                ),
                ("reported", json!({"type": "boolean"})),
            ]),
            &[],
        ),
        s(
            "turn",
            object_data(&[
                ("turn", json!({"type": "integer", "minimum": 0})),
                ("state", string()),
            ]),
            &[],
        ),
        s("title", object_data(&[("title", string())]), &[]),
        s(
            "labels",
            object_data(&[(
                "labels",
                json!({"type": "object", "additionalProperties": {"type": "string"}}),
            )]),
            &[],
        ),
        s(
            "size",
            object_data(&[
                ("cols", json!({"type": "integer", "minimum": 0})),
                ("rows", json!({"type": "integer", "minimum": 0})),
            ]),
            &[],
        ),
        s(
            "foreground",
            json!({"anyOf": [{"$ref": "#/components/schemas/Foreground"}, {"type": "null"}]}),
            &[],
        ),
    ]
}

/// The README `event` params envelope.
fn envelope() -> Value {
    json!({"type": "object", "required": ["seq", "type", "terminal", "data"],
           "properties": {"seq": {"type": "integer", "minimum": 0}, "type": string(),
                          "terminal": string()}})
}

/// Validates one `event` params object against the README tables; unknown
/// event types are tolerated and reported as `Ok(false)`.
fn check_event(root: &Value, params: &Value, full: bool) -> Result<bool, Vec<String>> {
    let mut errors = validate(root, &envelope(), params, "event");
    let Some(kind) = params["type"].as_str() else {
        return Err(errors);
    };
    let all = shapes();
    let Some(shape) = all.iter().find(|s| s.kind == kind) else {
        return if errors.is_empty() {
            Ok(false)
        } else {
            Err(errors)
        };
    };
    let data = &params["data"];
    if full {
        errors.extend(validate(root, &shape.data, data, &format!("{kind}.data")));
    } else if let Some(map) = data.as_object() {
        let keys: BTreeSet<&str> = map.keys().map(String::as_str).collect();
        let pinned: BTreeSet<&str> = shape.fake_keys.iter().copied().collect();
        if keys != pinned {
            errors.push(format!("{kind}.data keys {keys:?}, pinned {pinned:?}"));
        }
        for (key, value) in map {
            match shape.data.get("properties").and_then(|p| p.get(key)) {
                Some(field) => {
                    errors.extend(validate(root, field, value, &format!("{kind}.{key}")))
                }
                None => errors.push(format!("{kind}.{key}: not a README field")),
            }
        }
    } else {
        errors.push(format!("{kind}.data is not an object"));
    }
    if errors.is_empty() {
        Ok(true)
    } else {
        Err(errors)
    }
}

#[test]
fn every_event_the_fake_emits_conforms() {
    let root = openrpc();
    let mut kinds = BTreeSet::new();
    for unknown in [false, true] {
        let state = rich_world(unknown);
        let events = state.events();
        assert!(events.len() >= 12, "the world should log many events");
        for event in &events {
            let known = check_event(&root, event, false).unwrap_or_else(|e| panic!("{e:#?}"));
            assert!(known, "the fake logged an event the README does not list");
            kinds.insert(event["type"].as_str().unwrap().to_owned());
        }
    }
    let expected: BTreeSet<String> = [
        "created",
        "lost",
        "restarted",
        "keep",
        "finished",
        "removed",
        "status",
        "turn",
        "labels",
    ]
    .map(String::from)
    .into();
    assert_eq!(
        kinds, expected,
        "the fake's event types changed; extend the world"
    );
}

#[test]
fn stream_notifications_conform() {
    let root = openrpc();
    let state = rich_world(false);
    state.set_stream_before_result(true);
    state.set_unknown_events(true);
    let lines = call(&state, "events.subscribe", json!({}));
    let mut methods = Vec::new();
    for line in lines.iter().filter(|l| l.get("id").is_none()) {
        assert_eq!(line["jsonrpc"], "2.0");
        assert!(line.get("id").is_none(), "a notification has no id");
        let params = &line["params"];
        let method = line["method"].as_str().unwrap();
        methods.push(method.to_owned());
        match method {
            "heartbeat" => {
                let shape = json!({"type": "object", "required": ["seq"],
                                   "properties": {"seq": {"type": "integer", "minimum": 0}}});
                let errors = validate(&root, &shape, params, method);
                assert!(errors.is_empty(), "{errors:#?}");
            }
            "missed" => {
                // README: `missed` carries a fresh snapshot (seq, terminals).
                let errors = validate(
                    &root,
                    result_schema(&root, "events.subscribe"),
                    params,
                    method,
                );
                assert!(errors.is_empty(), "{errors:#?}");
            }
            "event" => {
                // Known types (turn) are held to the table; unknown ones tolerated.
                let known = check_event(&root, params, false).unwrap_or_else(|e| panic!("{e:#?}"));
                assert_eq!(known, params["type"] != "x_future_event");
            }
            other => assert_eq!(other, "x_future_method", "an unlisted notification method"),
        }
    }
    assert_eq!(
        methods,
        ["heartbeat", "event", "missed", "event", "x_future_method"]
    );
}

fn example_for(kind: &str) -> Value {
    let terminal = || {
        let state = FakeHcState::new();
        state.add_terminal(FakeTerminal::live("t7"));
        let lines = call(&state, "terminal.get", json!({"terminal": "t7"}));
        response(&lines)["result"].clone()
    };
    let end = json!({"reason": "exited", "exit_code": 0, "signal": null, "finished_ms": 9});
    let data = match kind {
        "created" | "restarted" => terminal(),
        "lost" | "finished" => end,
        "keep" => json!({"keep": true}),
        "removed" => json!({}),
        "status" => json!({"status": {"state": "idle"}, "records": {"k": {"state": "done"}},
                           "reported": true}),
        "turn" => json!({"turn": 3, "state": "idle"}),
        "title" => json!({"title": "t"}),
        "labels" => json!({"labels": {"a": "b"}, "record": terminal()}),
        "size" => json!({"cols": 80, "rows": 24}),
        "foreground" => json!({"pid": 1, "name": "sh", "argv": ["sh"], "cwd": "/"}),
        other => panic!("{other}"),
    };
    json!({"seq": 42, "type": kind, "terminal": "t7", "data": data})
}

#[test]
fn readme_event_table_is_complete_and_checked_in_full() {
    let root = openrpc();
    let kinds: BTreeSet<&str> = shapes().iter().map(|s| s.kind).collect();
    // The twelve terminal event types of the README table.
    assert_eq!(kinds.len(), 12);
    for kind in kinds {
        let event = example_for(kind);
        assert_eq!(check_event(&root, &event, true), Ok(true), "{kind}");
    }
    // `foreground` may also be null (README: "or null").
    let mut event = example_for("foreground");
    event["data"] = Value::Null;
    assert_eq!(check_event(&root, &event, true), Ok(true));
    // The doc event has no terminal and is not a terminal event type.
    let doc = json!({"seq": 43, "type": "doc", "data": {"name": "herdr-app", "version": 2}});
    assert!(
        check_event(&root, &doc, true).is_err(),
        "terminal is required here"
    );
}

// ---- test of the test: a deliberately malformed reply fails

#[test]
fn malformed_replies_fail_validation() {
    let root = openrpc();
    let state = rich_world(false);
    let mut checked = BTreeSet::new();
    for (method, lines) in replies(&state) {
        let schema = result_schema(&root, method);
        let good = response(&lines)["result"].clone();
        assert!(
            validate(&root, schema, &good, method).is_empty(),
            "{method}"
        );
        // Dropping any one required result field must be caught.
        let required = resolve(&root, schema)["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            !required.is_empty(),
            "{method} has no required fields to drop"
        );
        for field in required {
            let field = field.as_str().unwrap();
            let mut bad = good.clone();
            bad.as_object_mut().unwrap().remove(field);
            let errors = validate(&root, schema, &bad, method);
            assert!(
                errors.iter().any(|e| e.contains(field)),
                "{method}: dropping {field} passed: {errors:?}"
            );
            // And so must a wrong type in its place.
            let mut bad = good.clone();
            bad[field] = json!({"not": ["right"]});
            let still_ok = validate(&root, schema, &bad, method).is_empty();
            let loose = matches!(field, "value");
            assert!(
                loose || !still_ok,
                "{method}: wrong type for {field} passed"
            );
        }
        checked.insert(method);
    }
    let served: BTreeSet<&str> = SERVED_METHODS.iter().copied().collect();
    assert_eq!(checked, served);

    // A nested required field (a terminal inside the snapshot, the core id).
    let lines = call(&state, "events.subscribe", json!({}));
    let mut bad = response(&lines)["result"].clone();
    bad["terminals"][0]
        .as_object_mut()
        .unwrap()
        .remove("address");
    let schema = result_schema(&root, "events.subscribe");
    let errors = validate(&root, schema, &bad, "events.subscribe");
    assert!(
        errors
            .iter()
            .any(|e| e.contains("terminals[0]") && e.contains("address")),
        "{errors:?}"
    );
    let lines = call(&state, "hello", json!({}));
    let mut bad = response(&lines)["result"].clone();
    bad["core"].as_object_mut().unwrap().remove("id");
    let errors = validate(&root, result_schema(&root, "hello"), &bad, "hello");
    assert!(
        errors
            .iter()
            .any(|e| e.contains("core") && e.contains("id")),
        "{errors:?}"
    );
}

#[test]
fn malformed_events_fail_validation() {
    let root = openrpc();
    let good = example_for("finished");
    for field in ["seq", "type", "terminal", "data"] {
        let mut bad = good.clone();
        bad.as_object_mut().unwrap().remove(field);
        assert!(
            check_event(&root, &bad, true).is_err(),
            "dropping {field} passed"
        );
    }
    for field in ["reason", "exit_code", "signal", "finished_ms"] {
        let mut bad = good.clone();
        bad["data"].as_object_mut().unwrap().remove(field);
        assert!(
            check_event(&root, &bad, true).is_err(),
            "dropping data.{field} passed"
        );
    }
    let mut bad = good.clone();
    bad["seq"] = json!("42");
    assert!(check_event(&root, &bad, true).is_err(), "string seq passed");
    let mut bad = example_for("created");
    bad["data"].as_object_mut().unwrap().remove("address");
    assert!(
        check_event(&root, &bad, true).is_err(),
        "created without address passed"
    );
    // The fake-side check also catches a drifted fake: an extra, unpinned key.
    let mut drift = json!({"seq": 1, "type": "turn", "terminal": "t1", "data": {"turn": 1}});
    assert!(
        check_event(&root, &drift, false).is_err(),
        "unpinned key passed"
    );
    // An unknown event type is tolerated, not validated.
    drift["type"] = json!("x_future_event");
    assert_eq!(check_event(&root, &drift, false), Ok(false));
}
