#!/usr/bin/env python3
"""Stub probe for the canary self-test: `stub_probe.py <case.json> <version>`.

The case file's "results" maps a version to a sequence of planted outcomes, consumed one per call
(the last one repeats; "default" covers unlisted versions): "pass", "fail" (tier 0), "fail:t1", "infra".
Call counts live in $HT_SELFTEST_STATE; every raw stdout is also appended to $HT_SELFTEST_STATE/calls.jsonl.
Prints a probeResult (scripts/canary/probe_schema.json); exit 0 pass / 1 fail / 2 infra."""
import json, os, pathlib, sys

case_path, version = sys.argv[1], sys.argv[2]
case = json.loads(pathlib.Path(case_path).read_text())
state = pathlib.Path(os.environ["HT_SELFTEST_STATE"])
state.mkdir(parents=True, exist_ok=True)
counter = state / f"count-{version}"
n = int(counter.read_text()) if counter.exists() else 0
counter.write_text(str(n + 1))
seq = case["results"].get(version, case["results"]["default"])
if isinstance(seq, str):
    seq = [seq]
outcome = seq[min(n, len(seq) - 1)]

if outcome == "pass":
    doc, code = {"result": "pass", "failed_tier": None,
                 "checks": [{"id": "t0.version", "status": "pass", "detail": version},
                            {"id": "t0.schema", "status": "pass"}]}, 0
elif outcome == "fail":
    doc, code = {"result": "fail", "failed_tier": 0,
                 "checks": [{"id": "t0.version", "status": "pass", "detail": version},
                            {"id": "t0.schema", "status": "fail", "detail": f"planted failure at {version}"}]}, 1
elif outcome == "fail:t1":
    doc, code = {"result": "fail", "failed_tier": 1,
                 "checks": [{"id": "t0.version", "status": "pass", "detail": version},
                            {"id": "t1.payload-parse", "status": "fail", "detail": f"planted tier-1 failure at {version}"}]}, 1
elif outcome == "infra":
    doc, code = {"result": "infra", "failed_tier": None, "checks": []}, 2
else:
    raise SystemExit(f"unknown planted outcome {outcome!r}")
text = json.dumps(doc)
with open(state / "calls.jsonl", "a") as f:
    f.write(json.dumps({"version": version, "stdout": text}) + "\n")
print(text)
sys.exit(code)
