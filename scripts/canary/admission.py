#!/usr/bin/env python3
"""t0.admission evaluation (nested spec §D3), unit-tested by test_admission.py.

`evaluate(doctor_json, expected, schema, harness)` compares `doctor.hooks.<harness>.installed.admission`
(`listed | schema-matched, live-unverified | optimistic | refused | not_found`) with the probe's
`expected_admission` (the closed set of canary-probe.json: listed | optimistic | schema-matched-or-optimistic |
refused | unasserted). For Codex above verified_max the expectation is finalized by `schema`, the `match |
drift | unextractable` result the gated Rust test wrote into canary-rust.json: `match` requires
`schema-matched, live-unverified`, `drift`/`unextractable` require `optimistic` (coverage r2).

Returns (status, detail): pass | fail | skip (unasserted: recorded, not asserted) | infra (`not_found`: the
harness binary was not even found, never a version break).

CLI: `admission.py evaluate --harness H --expected E [--schema S] DOCTOR_JSON_FILE` prints
`status<TAB>detail<TAB>admission`. Pure stdlib."""
import argparse, json, sys

MATCHED = "schema-matched, live-unverified"
OPTIMISTIC = "optimistic"
DEFERRED = "schema-matched-or-optimistic"
SCHEMA_RESULTS = ("match", "drift", "unextractable")


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


def evaluate(doctor_json, expected, schema=None, harness="claude"):
    observed = admission_of(doctor_json, harness)
    if observed is None:
        return "fail", f"doctor JSON has no doctor.hooks.{harness}.installed.admission string"
    if observed == "not_found":
        return "infra", f"doctor found no {harness} binary (admission not_found); expected {expected}"
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
    ev = sub.add_parser("evaluate")
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
    status, detail = evaluate(doc, args.expected, args.schema or None, harness=args.harness)
    print(status, detail.replace("\t", " ").replace("\n", " "), admission_of(doc, args.harness) or "", sep="\t")
    return 0


if __name__ == "__main__":
    sys.exit(main())
