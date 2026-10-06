#!/usr/bin/env python3
"""Core t0.admission and separate historical diagnostics, unit-tested by test_admission.py.

`evaluate_core(doctor_json, harness)` checks ordinary doctor's `contract_declared` only. A pass proves
no exact runtime or native capability. Optional metadata is reported honestly, never qualification.

Historical `evaluate(doctor_json, expected, schema, harness)` compares an explicit diagnostic's admission
(`listed | schema-matched, live-unverified | optimistic | refused | not_found`) with the probe's
`expected_admission` (the closed set of canary-probe.json: listed | optimistic | schema-matched-or-optimistic |
refused | unasserted). For Codex above verified_max the expectation is finalized by `schema`, the `match |
drift | unextractable` result the gated Rust test wrote into canary-rust.json: `match` requires
`schema-matched, live-unverified`, `drift`/`unextractable` require `optimistic` (coverage r2).

Returns (status, detail): pass | fail | skip (unasserted: recorded, not asserted) | infra (`not_found`: the
harness binary was not even found, never a version break).

CLI: `admission.py evaluate-core --harness H DOCTOR_JSON_FILE` or
`admission.py evaluate-historical --harness H --expected E [--schema S] DOCTOR_JSON_FILE` prints
`status<TAB>detail<TAB>admission`. Pure stdlib."""
import argparse, json, sys

MATCHED = "schema-matched, live-unverified"
OPTIMISTIC = "optimistic"
DEFERRED = "schema-matched-or-optimistic"
SCHEMA_RESULTS = ("match", "drift", "unextractable")
DECLARED = "contract_declared"


def admission_of(doctor_json, harness):
    """The observed admission string, or None when doctor's JSON does not carry one."""
    doc = doctor_json
    if isinstance(doc, (str, bytes)):
        try:
            doc = json.loads(doc)
        except ValueError:
            return None
    try:
        value = doc["doctor"]["hooks"][harness]["installed"]["admission"]
    except (KeyError, TypeError):
        return None
    return value if isinstance(value, str) else None


def evaluate_core(doctor_json, harness="claude"):
    observed = admission_of(doctor_json, harness)
    if observed is None:
        return "fail", f"doctor JSON has no doctor.hooks.{harness}.installed.admission string"
    if observed == "not_found":
        return "infra", f"doctor found no {harness} binary (admission not_found)"
    if observed != DECLARED:
        return "fail", f"core admission {observed}, expected {DECLARED}; historical qualification is separate"
    doc = json.loads(doctor_json) if isinstance(doctor_json, (str, bytes)) else doctor_json
    version = doc["doctor"]["hooks"][harness]["installed"].get("version")
    metadata = "runtime metadata unavailable" if version is None else "optional runtime metadata present (diagnostic only)"
    return "pass", f"core contract declared; {metadata}; no exact-runtime/native proof"


def evaluate(doctor_json, expected, schema=None, harness="claude"):
    """Explicit historical admission diagnostics; never ordinary core/runtime qualification."""
    observed = admission_of(doctor_json, harness)
    if observed is None:
        return "fail", f"doctor JSON has no doctor.hooks.{harness}.installed.admission string"
    if observed == "not_found":
        return "infra", f"doctor found no {harness} binary (admission not_found); expected {expected}"
    if observed == DECLARED:
        return "fail", "core contract declaration is not historical exact-runtime/native qualification"
    if expected == "unasserted":
        return "skip", f"admission {observed} recorded, not asserted (version is neither listed, known broken nor above verified_max)"
    if expected == DEFERRED:
        if schema not in SCHEMA_RESULTS:
            return "fail", f"admission {observed}: cannot finalize {DEFERRED}, t0.schema result unavailable ({schema!r})"
        want = MATCHED if schema == "match" else OPTIMISTIC
        if observed == want:
            return "pass", f"admission {observed} (schema {schema})"
        return "fail", f"admission {observed}, expected {want} because schema is {schema}"
    if observed == expected:
        return "pass", f"admission {observed}"
    return "fail", f"admission {observed}, expected {expected}"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    core = sub.add_parser("evaluate-core", help="ordinary declared core admission only")
    core.add_argument("--harness", required=True)
    core.add_argument("doctor_json_file")
    ev = sub.add_parser("evaluate-historical", aliases=["evaluate"], help="explicit historical ladder/schema diagnostics")
    ev.add_argument("--harness", required=True)
    ev.add_argument("--expected", required=True)
    ev.add_argument("--schema", default=None)
    ev.add_argument("doctor_json_file")
    args = ap.parse_args(argv)
    try:
        with open(args.doctor_json_file, encoding="utf-8") as f:
            doc = f.read()
    except OSError as e:
        print("fail", f"cannot read doctor JSON: {e}", "", sep="\t")
        return 0
    if args.cmd == "evaluate-core":
        status, detail = evaluate_core(doc, harness=args.harness)
    else:
        status, detail = evaluate(doc, args.expected, args.schema or None, harness=args.harness)
    print(status, detail.replace("\t", " ").replace("\n", " "), admission_of(doc, args.harness) or "", sep="\t")
    return 0


if __name__ == "__main__":
    sys.exit(main())
