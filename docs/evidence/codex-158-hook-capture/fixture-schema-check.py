#!/usr/bin/env python3
"""Check the committed Codex 0.158.0 fixtures against the extracted 0.158.0
input schemas: required keys present, no keys outside `properties`
(additionalProperties is false), `const`/`enum` string values respected, and
declared string/null types respected. Read-only; no jsonschema dependency."""
import json, pathlib, sys

root = pathlib.Path(__file__).resolve().parents[5]
schemas = pathlib.Path(__file__).resolve().parent / "schemas-0.158.0"
fixtures = root / "tests/fixtures/codex-0.158.0"
SCHEMA = {"SessionStart": "session-start", "SubagentStart": "subagent-start", "PreToolUse": "pre-tool-use"}


def resolve(schema, prop):
    if not isinstance(prop, dict):
        return {}  # `true`: any value
    if "$ref" in prop:
        return schema["definitions"][prop["$ref"].rsplit("/", 1)[1]]
    return prop


failures = 0
for path in sorted(fixtures.glob("*.json")):
    payload = json.loads(path.read_text())
    schema = json.loads((schemas / f"{SCHEMA[payload['hook_event_name']]}.command.input.schema.json").read_text())
    problems = [f"missing {k}" for k in schema.get("required", []) if k not in payload]
    if schema.get("additionalProperties") is False:
        problems += [f"extra {k}" for k in payload if k not in schema["properties"]]
    for key, value in payload.items():
        prop = resolve(schema, schema["properties"].get(key, {}))
        if "const" in prop and value != prop["const"]:
            problems.append(f"{key} != const")
        if "enum" in prop and value not in prop["enum"]:
            problems.append(f"{key} not in enum")
        types = prop.get("type")
        types = [types] if isinstance(types, str) else (types or [])
        kind = {str: "string", type(None): "null", dict: "object"}.get(type(value))
        if types and kind not in types:
            problems.append(f"{key} type {kind} not in {types}")
    failures += bool(problems)
    print(path.name, "OK" if not problems else problems)
sys.exit(1 if failures else 0)
