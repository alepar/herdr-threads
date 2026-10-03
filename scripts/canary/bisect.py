"""Canary bisection engine (nested spec §D7, rule 8). ht-p03.14.8 delivered the probe-result
reader and the retry policy; this module adds the search on top of them.

Probe command contract: `--probe-cmd '<cmd> {version}'`, exit 0 pass / 1 fail / 2 infra, one probeResult
JSON document (scripts/canary/probe_schema.json) on stdout.

Note: this file shadows the stdlib `bisect` when scripts/canary is first on sys.path; it therefore
imports nothing that needs the stdlib module (no `random`/`tempfile`)."""
import importlib.util, json, pathlib, shlex, subprocess, sys, time
SCHEMA = pathlib.Path(__file__).with_name("probe_schema.json")
PROBE_TIMEOUT_S = 1800

_RESULTS = ("pass", "fail", "infra")
_STATUSES = ("pass", "fail", "warn", "skip")


def parse_probe_output(text):
    """Parse one --probe stdout document; ValueError when it is not a valid probeResult."""
    try:
        doc = json.loads(text)
    except json.JSONDecodeError as e:
        raise ValueError(f"probe output is not JSON: {e}") from e
    if not isinstance(doc, dict):
        raise ValueError("probe output must be a JSON object")
    extra = set(doc) - {"result", "checks", "failed_tier", "contract"}
    if extra:
        raise ValueError(f"unexpected keys: {sorted(extra)}")
    for key in ("result", "checks", "failed_tier"):  # `contract` is optional (older probes omit it)
        if key not in doc:
            raise ValueError(f"missing key: {key}")
    if doc["result"] not in _RESULTS:
        raise ValueError(f"bad result: {doc['result']!r}")
    ft = doc["failed_tier"]
    if isinstance(ft, bool) or ft not in (0, 1, None):
        raise ValueError(f"bad failed_tier: {ft!r}")
    if not isinstance(doc["checks"], list):
        raise ValueError("checks must be a list")
    for i, c in enumerate(doc["checks"]):
        if not isinstance(c, dict):
            raise ValueError(f"checks[{i}] must be an object")
        if set(c) - {"id", "status", "detail"}:
            raise ValueError(f"checks[{i}] has unexpected keys")
        if not isinstance(c.get("id"), str):
            raise ValueError(f"checks[{i}].id must be a string")
        if c.get("status") not in _STATUSES:
            raise ValueError(f"checks[{i}].status invalid: {c.get('status')!r}")
        if "detail" in c and not isinstance(c["detail"], str):
            raise ValueError(f"checks[{i}].detail must be a string")
    return doc


def retries_for(probe):
    """Extra attempts after a failing probe: twice for a tier-1 failure, once otherwise
    (an infra result is retried once too); none for a pass."""
    if probe["result"] == "pass":
        return 0
    return 2 if probe.get("failed_tier") == 1 else 1


def _sibling(name):
    """Load scripts/canary/<name>.py once, under the name canary_<name> (shared with other loaders)."""
    mod = sys.modules.get(f"canary_{name}")
    if mod is None:
        spec = importlib.util.spec_from_file_location(f"canary_{name}", pathlib.Path(__file__).with_name(f"{name}.py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules[f"canary_{name}"] = mod
        spec.loader.exec_module(mod)
    return mod


def _attempt(probe_cmd, version, tier1):
    """One probe invocation -> (attempt dict, failed_tier)."""
    argv = [a.replace("{version}", version) for a in shlex.split(probe_cmd)]
    start = time.monotonic()
    failed_tier, checks, detail, contract = None, [], None, None
    try:
        cp = subprocess.run(argv, capture_output=True, text=True, timeout=PROBE_TIMEOUT_S)
        result = {0: "pass", 1: "fail"}.get(cp.returncode, "infra")
        try:
            doc = parse_probe_output(cp.stdout)
            checks, failed_tier, contract = doc["checks"], doc["failed_tier"], doc.get("contract")
            if doc["result"] != result:
                result, detail = "infra", f"exit code {cp.returncode} contradicts result {doc['result']!r}"
        except ValueError as e:
            if result != "infra":
                result, detail = "infra", str(e)
    except (OSError, subprocess.TimeoutExpired) as e:
        result, detail = "infra", f"probe command failed to run: {e}"
    if detail is not None:
        checks = checks + [{"id": "t0.probe-output", "status": "fail", "detail": detail}]
    attempt = {"result": result, "tier1": bool(tier1) or failed_tier == 1,
               "duration_ms": int((time.monotonic() - start) * 1000), "checks": checks}
    if contract is not None:
        attempt["contract"] = contract
    return attempt, failed_tier


def run(candidates, baseline, probe_cmd, bisect=True, tier1=False, known_broken=()):
    """Search `candidates` (ascending, §D1) for the first failing version; returns the §D7/§D8 harness
    result: status, first_bad, last_good, probes, excluded, ..."""
    versions = _sibling("versions")
    cands = list(candidates)
    ranges = [tuple(r) for r in known_broken]
    kept, excluded = versions.excluded_by_known_broken(cands, ranges)
    res = {"status": None, "candidates": cands, "baseline": baseline, "first_bad": None, "last_good": None,
           "failing_checks": [], "signals": [], "excluded": excluded, "probes": [],
           "assumes_monotone": True, "crossed_ranges": 0}

    def probe(version, role):
        attempts, budget = [], None
        while True:
            att, ft = _attempt(probe_cmd, version, tier1)
            attempts.append(att)
            if att["result"] == "pass":
                break
            if budget is None:
                budget = retries_for({"result": att["result"], "failed_tier": ft})
            if len(attempts) - 1 >= budget:
                break
        result = attempts[-1]["result"]
        p = {"version": version, "role": role, "attempts": attempts, "result": result,
             "flaky": result == "pass" and len(attempts) > 1}
        res["probes"].append(p)
        return p

    def finish(status):
        res["status"] = status
        return res

    def evidence(p):
        last = p["attempts"][-1]["checks"]
        res["failing_checks"] = [{"id": c["id"], "detail": c.get("detail", "")} for c in last if c["status"] == "fail"]
        res["signals"] = [{"id": c["id"], "detail": c.get("detail", "")} for c in last if c["status"] == "warn"]

    res["crossed_ranges"] = len([r for r in ranges if any(versions.in_range(v, r) for v in excluded)])
    if not kept:
        return finish("no_candidates")

    if not bisect:
        # Every candidate, newest first; no first_bad is claimed.
        for i, v in enumerate(reversed(kept)):
            probe(v, "newest" if i == 0 else "search")
        results = [p["result"] for p in res["probes"]]
        if "infra" in results:
            return finish("infra_error")
        bad = [p for p in res["probes"] if p["result"] == "fail"]
        if bad:
            evidence(bad[0])
            return finish("break")
        return finish("all_pass")

    n = len(kept)
    newest = probe(kept[-1], "newest")
    if newest["result"] == "infra":
        return finish("infra_error")
    if newest["result"] == "pass":
        return finish("all_pass")
    evidence(newest)
    if versions.in_any_range(kept[-1], ranges):
        # Only reachable through an open-ended range: the newest version is probed anyway and fails as declared.
        return finish("known_broken_persists")

    lo = -1
    moved = False
    if excluded:
        top = max(excluded, key=versions.version_key)
        above = [i for i, v in enumerate(kept) if versions.version_key(v) > versions.version_key(top)]
        if above:
            a = above[0]
            if a == n - 1:
                return finish("known_broken_persists")
            pa = probe(kept[a], "baseline-move")
            if pa["result"] == "infra":
                return finish("infra_error")
            if pa["result"] == "fail":
                return finish("known_broken_persists")
            lo, moved = a, True

    hi = n - 1
    while hi - lo > 1:
        mid = (lo + hi) // 2
        p = probe(kept[mid], "search")
        if p["result"] == "infra":
            return finish("infra_error")
        if p["result"] == "pass":
            lo = mid
        else:
            hi = mid
    first_bad = kept[hi]
    last_good = kept[lo] if lo >= 0 else baseline

    pf = probe(first_bad, "confirm")
    if pf["result"] == "infra":
        return finish("infra_error")
    if pf["result"] == "pass":
        return finish("inconclusive")
    if not (moved and lo == above[0]):  # a moved baseline was already probed as baseline-move
        pl = probe(last_good, "confirm")
        if pl["result"] == "infra":
            return finish("infra_error")
        if pl["result"] == "fail":
            return finish("inconclusive")
    evidence(pf)
    res["first_bad"], res["last_good"] = first_bad, last_good
    return finish("break")


def main(argv=None):
    import argparse
    ap = argparse.ArgumentParser(description="Bisect a harness version range with an abstract probe command.")
    ap.add_argument("--probe-cmd", required=True, help="command with a {version} placeholder")
    ap.add_argument("--candidates", required=True, help="comma-separated ascending versions")
    ap.add_argument("--baseline", required=True, help="verified_max, assumed good")
    ap.add_argument("--bisect", action="store_true", help="binary search (default: probe every candidate)")
    ap.add_argument("--tier1", action="store_true", help="the probe command runs the model tier")
    ap.add_argument("--known-broken", default="[]", help='JSON [{"min":"a.b.c"|null,"max":"x.y.z"|null}]')
    args = ap.parse_args(argv)
    ranges = [(r.get("min"), r.get("max")) for r in json.loads(args.known_broken)]
    res = run([c for c in args.candidates.split(",") if c], args.baseline, args.probe_cmd,
              bisect=args.bisect, tier1=args.tier1, known_broken=ranges)
    print(json.dumps(res, indent=2))
    return _sibling("report").exit_code({"harnesses": [res]})


if __name__ == "__main__":
    sys.exit(main())
