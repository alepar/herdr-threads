#!/usr/bin/env python3
"""Harness manifest writer for the canary (docs/superpowers/specs/2026-10-02-harness-version-evidence-design.md,
"Manifest" and "Canary and publishing"). Pure stdlib.

  manifest.py write --baseline PATH --report canary-report.json --binary HT_BIN [--release-results JSON]
                    [--latest-release TAG] [--issue-urls JSON] [--generated-at ISO] --out PATH
  manifest.py validate PATH
  manifest.py set --file PATH --harness H --version V --status verified|known_broken --binary HT_BIN [...]
  manifest.py check-ruleset --rulesets-json PATH --default-branch main
  manifest.py embed --branch-file PATH --out PATH
  manifest.py contract --binary HT_BIN

The canary writes schema-2 rows to the `harness-manifest` branch. Only payload-contract violations become
`known_broken`; every other failing outcome (tier 0, fingerprint drift, infra, inconclusive, flaky) stays
issue-only. Rows with `source: "manual"` are never modified by the canary.
"""
import sys
# scripts/canary/bisect.py would shadow the stdlib `bisect` (needed by random/tempfile) when this directory is
# sys.path[0]; drop it while the imports run and restore it after (this module is also imported by tests).
_here = __import__("os").path.dirname(__import__("os").path.abspath(__file__))
_saved_path = list(sys.path)
sys.path[:] = [p for p in sys.path if p not in ("", _here)]

import argparse, datetime, importlib.util, json, os, re, subprocess

sys.path[:] = _saved_path

MAX_MANIFEST_BYTES = 262_144
CAP_BYTES = MAX_MANIFEST_BYTES * 8 // 10  # the writer fails above 80% of the reader's cap (209715)
RETAIN_PER_HARNESS = 50
HARNESSES = ("claude", "codex")
TOP_KEYS = ("schema_version", "generated_from", "generated_at", "latest_release", "contracts", "rows")
ROW_KEYS = ("harness", "version", "status", "evidence", "contract_id", "source", "supported_since",
            "broken_event", "broken_field", "last_working", "issue_url", "recipe", "known_broken")
STATUSES = ("verified", "known_broken")
EVIDENCE = ("live", "no_model", "schema", "none")
SOURCES = ("canary", "manual")
X_Y_Z = re.compile(r"^(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})\.(0|[1-9][0-9]{0,5})$")
CONTRACT_ID = re.compile(r"^[0-9a-f]{16}$")
TOOL_EVENT = "PreToolUse"


def _load_sibling(name):
    mod = sys.modules.get(f"canary_{name}")
    if mod is None:
        spec = importlib.util.spec_from_file_location(f"canary_{name}", os.path.join(_here, f"{name}.py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules[f"canary_{name}"] = mod
        spec.loader.exec_module(mod)
    return mod


versions = _load_sibling("versions")


def warn(msg):
    print(f"manifest.py: {msg}", file=sys.stderr)


# ---------------------------------------------------------------- binary

def contract_ids(binary):
    """`<binary> contract-id --json` -> the whole document ({claude, codex, normalize})."""
    cp = subprocess.run([binary, "contract-id", "--json"], capture_output=True, text=True, timeout=60)
    if cp.returncode != 0:
        raise ValueError(f"{binary} contract-id --json exited {cp.returncode}: {cp.stderr.strip()}")
    try:
        doc = json.loads(cp.stdout)
    except json.JSONDecodeError as e:
        raise ValueError(f"{binary} contract-id --json printed no JSON: {e}") from e
    for h in HARNESSES:
        if not isinstance(doc.get(h), str) or not CONTRACT_ID.match(doc[h]):
            raise ValueError(f"{binary} contract-id --json has no valid id for {h}")
    return doc


class Normalizer:
    """Version keys come only from `<binary> harness-version normalize <harness> <raw>` (one call per distinct
    raw version); a non-zero exit yields None and a warning."""

    def __init__(self, binary):
        self.binary = binary
        self.cache = {}

    def __call__(self, harness, raw):
        key = (harness, raw)
        if key not in self.cache:
            cp = subprocess.run([self.binary, "harness-version", "normalize", harness, raw],
                                capture_output=True, text=True, timeout=60)
            out = cp.stdout.strip()
            if cp.returncode != 0 or not X_Y_Z.match(out):
                warn(f"cannot normalize {harness} version {raw!r} (exit {cp.returncode}); skipping it")
                out = None
            self.cache[key] = out
        return self.cache[key]


# ---------------------------------------------------------------- document model

def version_key(v):
    return tuple(int(p) for p in v.split("."))


def row_key(row):
    return (row["harness"], row["version"], row.get("contract_id"))


def sort_key(row):
    return (row["harness"], version_key(row["version"]), row.get("contract_id") or "")


def normalize_row(row):
    """A row in the schema-2 key order; unknown keys follow the known ones."""
    out = {k: row.get(k) for k in ROW_KEYS}
    if out["known_broken"] is None:
        out["known_broken"] = []
    for k, v in row.items():
        if k not in out:
            out[k] = v
    return out


def render(doc):
    """The one serialization: 2-space indent, keys in the documented order, trailing newline."""
    return json.dumps(doc, indent=2, ensure_ascii=False) + "\n"


def build_doc(generated_from, generated_at, latest_release, contracts, rows):
    return {"schema_version": 2, "generated_from": generated_from, "generated_at": generated_at,
            "latest_release": latest_release, "contracts": contracts,
            "rows": [normalize_row(r) for r in sorted(rows, key=sort_key)]}


def validate_doc(doc):
    """One string per violation; empty when the document is a valid schema-2 manifest."""
    errs = []
    if not isinstance(doc, dict):
        return ["document must be a JSON object"]
    if doc.get("schema_version") != 2:
        errs.append(f"schema_version must be 2, got {doc.get('schema_version')!r}")
    rows = doc.get("rows")
    if not isinstance(rows, list):
        errs.append("rows must be a list")
        return errs
    seen = {}
    for i, row in enumerate(rows):
        where = f"rows[{i}]"
        if not isinstance(row, dict):
            errs.append(f"{where}: must be an object")
            continue
        h, v = row.get("harness"), row.get("version")
        if h not in HARNESSES:
            errs.append(f"{where}: harness must be one of {list(HARNESSES)}, got {h!r}")
        if not isinstance(v, str) or not X_Y_Z.match(v):
            errs.append(f"{where}: version must be a string X.Y.Z, got {v!r}")
        if row.get("status") not in STATUSES:
            errs.append(f"{where}: status must be one of {list(STATUSES)}, got {row.get('status')!r}")
        if row.get("evidence") is not None and row.get("evidence") not in EVIDENCE:
            errs.append(f"{where}: evidence must be one of {list(EVIDENCE)} or null, got {row.get('evidence')!r}")
        if row.get("source") not in SOURCES:
            errs.append(f"{where}: source must be one of {list(SOURCES)}, got {row.get('source')!r}")
        cid = row.get("contract_id")
        if cid is not None and not (isinstance(cid, str) and CONTRACT_ID.match(cid)):
            errs.append(f"{where}: contract_id must be 16 lowercase hex characters or null, got {cid!r}")
        if row.get("status") == "known_broken":
            for k in ("broken_event", "broken_field"):
                if row.get(k) is None:
                    errs.append(f"{where}: a known_broken row needs a non-null {k}")
        key = (h, v, cid)
        if isinstance(h, str) and isinstance(v, str) and key in seen:
            errs.append(f"{where}: duplicate key {key} (first at rows[{seen[key]}])")
        seen.setdefault(key, i)
    size = len(render(doc).encode("utf-8"))
    if size > CAP_BYTES:
        errs.append(f"manifest too large: {size} bytes > {CAP_BYTES}")
    return errs


def load_document(path):
    """versions.load_versions_json, but with the module's ValueError for a missing/unreadable file."""
    try:
        return versions.load_versions_json(path)
    except (OSError, json.JSONDecodeError) as e:
        raise ValueError(f"{path}: {e}") from e


def write_atomic(path, text):
    tmp = f"{path}.tmp{os.getpid()}"
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, path)


# ---------------------------------------------------------------- write

def _outcome(payloads, passed):
    """('broken', event, field) | ('verified', evidence) | None for one probe's payload classifications.
    Any violation breaks it; otherwise it verifies only when the probe passed and every payload is ok."""
    for p in payloads:
        if p.get("kind") == "violation":
            return ("broken", p.get("event"), p.get("field"))
    if passed and payloads and all(p.get("kind") == "ok" for p in payloads):
        live = any(p.get("event") == TOOL_EVENT for p in payloads)
        return ("verified", "live" if live else "no_model")
    return None


def main_observations(report, normalize):
    """(harness, version, outcome, first_bad) for each usable probe of the main contract."""
    out = []
    for block in report.get("harnesses", []):
        h = block.get("harness")
        if h not in HARNESSES or block.get("status") in ("inconclusive", "infra_error"):
            continue
        for probe in block.get("probes", []):
            attempts = probe.get("attempts") or []
            if probe.get("flaky") or probe.get("result") == "infra" or not attempts:
                continue
            v = normalize(h, probe.get("version", ""))
            if v is None:
                continue
            payloads = (attempts[-1].get("contract") or {}).get("payloads") or []
            oc = _outcome(payloads, probe.get("result") == "pass")
            if oc:
                out.append((h, v, oc, block.get("first_bad")))
    return out


def release_observations(release, normalize):
    """(harness, version, outcome) per probe the latest release's contract ran over; none when unsupported."""
    out = []
    if not isinstance(release, dict) or not release.get("supported"):
        return out
    for probe in release.get("probes", []):
        h = probe.get("harness")
        if h not in HARNESSES:
            continue
        v = normalize(h, probe.get("version", ""))
        if v is None:
            continue
        oc = _outcome(probe.get("payloads") or [], True)
        if oc:
            out.append((h, v, oc))
    return out


def release_ids(release):
    """{harness: contract id} of a supported release results document."""
    cid = release.get("contract_id")
    if isinstance(cid, dict):
        return {h: cid.get(h) for h in HARNESSES}
    return {h: cid for h in HARNESSES}


def build(baseline, report, binary, release=None, latest_release=None, issue_urls=None, generated_at=None):
    """The next manifest document (a dict) from the baseline and one canary run."""
    ids = contract_ids(binary)
    main_id = {h: ids[h] for h in HARNESSES}
    normalize = Normalizer(binary)
    rel_ok = isinstance(release, dict) and release.get("supported")
    rel_id = release_ids(release) if rel_ok else {}
    live = {h: {main_id[h]} | ({rel_id[h]} if rel_ok and rel_id.get(h) else set()) for h in HARNESSES}
    tag = (latest_release or baseline.get("latest_release") or None)
    tag = tag[1:] if isinstance(tag, str) and tag.startswith("v") else tag
    issue_urls = issue_urls or {}

    rows = {}
    for r in baseline.get("rows", []):
        r = normalize_row(r)
        rows[row_key(r)] = r

    obs = []  # (harness, version, contract_id, outcome, block first_bad or None)
    for h, v, oc, first_bad in main_observations(report, normalize):
        obs.append((h, v, main_id[h], oc, first_bad))
    rel_obs = release_observations(release, normalize) if rel_ok else []
    if rel_ok:
        for h, v, oc in rel_obs:
            if rel_id.get(h) != main_id[h]:  # the same contract folds into the main probes' rows
                obs.append((h, v, rel_id[h], oc, None))

    baseline_max = {h: versions.verified_max(baseline, h) for h in HARNESSES}

    def upsert(h, v, cid, fields):
        key = (h, v, cid)
        old = rows.get(key)
        if old is not None and old.get("source") == "manual":
            print(f"manifest.py: kept manual row {h} {v} {cid}", file=sys.stderr)
            return None
        base = dict(old) if old else {k: None for k in ROW_KEYS}
        base.update({"harness": h, "version": v, "contract_id": cid, "source": "canary", "recipe": None,
                     "known_broken": []})
        base.update(fields)
        rows[key] = normalize_row(base)
        return rows[key]

    for h, v, cid, oc, _ in obs:  # verified first, so a broken row's last_working sees this run's passes
        if oc[0] == "verified":
            upsert(h, v, cid, {"status": "verified", "evidence": oc[1], "broken_event": None,
                               "broken_field": None, "last_working": None, "issue_url": None})
    for h, v, cid, oc, first_bad in obs:
        if oc[0] != "broken":
            continue
        below = [r["version"] for r in rows.values()
                 if r["harness"] == h and r["contract_id"] == cid and r["status"] == "verified"
                 and version_key(r["version"]) < version_key(v)]
        last_working = max(below, key=version_key) if below else baseline_max[h]
        old = rows.get((h, v, cid)) or {}
        url = issue_urls.get(f"{h} {first_bad}") if first_bad else None
        upsert(h, v, cid, {"status": "known_broken", "evidence": "schema", "broken_event": oc[1],
                           "broken_field": oc[2], "last_working": last_working,
                           "issue_url": url or old.get("issue_url")})

    # rows of contracts that are neither main's nor the release's are dropped (manual rows stay)
    rows = {k: r for k, r in rows.items() if r["source"] == "manual" or r["contract_id"] in live[r["harness"]]}

    # supported_since: the release's contract passed this version, under either id
    if rel_ok and tag:
        passed = {(h, v) for h, v, oc in rel_obs if oc[0] == "verified"}
        # a release whose contract equals main's for a harness passes exactly what the main probes passed
        passed |= {(h, v) for h, v, cid, oc, _ in obs if oc[0] == "verified" and rel_id.get(h) == main_id[h]}
        for r in rows.values():
            if r["source"] == "canary" and r["status"] == "verified" and (r["harness"], r["version"]) in passed \
                    and not r.get("supported_since"):
                r["supported_since"] = tag

    # retention: the newest 50 distinct versions per harness, plus known_broken, recipe-listed and manual rows
    keep_versions = {}
    for h in HARNESSES:
        vs = sorted({r["version"] for r in rows.values() if r["harness"] == h}, key=version_key, reverse=True)
        keep_versions[h] = set(vs[:RETAIN_PER_HARNESS])
    kept = [r for r in rows.values()
            if r["version"] in keep_versions.get(r["harness"], ()) or r["status"] == "known_broken"
            or r.get("recipe") is not None or r["source"] == "manual"]

    now = generated_at or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return build_doc(f"harness-canary {report.get('canary_commit', 'unknown')}", now, tag, main_id, kept)


def cmd_write(a):
    baseline = load_document(a.baseline)
    with open(a.report, encoding="utf-8") as f:
        report = json.load(f)
    release = None
    if a.release_results:
        with open(a.release_results, encoding="utf-8") as f:
            release = json.load(f)
    urls = None
    if a.issue_urls:
        with open(a.issue_urls, encoding="utf-8") as f:
            urls = json.load(f)
    doc = build(baseline, report, a.binary, release, a.latest_release, urls, a.generated_at)
    text = render(doc)
    size = len(text.encode("utf-8"))
    if size > CAP_BYTES:
        print(f"manifest too large: {size} bytes > {CAP_BYTES}")
        return 1
    errs = validate_doc(doc)
    if errs:
        print("\n".join(errs))
        return 1
    write_atomic(a.out, text)
    print(f"wrote {a.out}: {len(doc['rows'])} rows, {size} bytes")
    return 0


# ---------------------------------------------------------------- validate / set

def cmd_validate(a):
    try:
        with open(a.path, encoding="utf-8") as f:
            doc = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        print(f"{a.path}: {e}")
        return 1
    errs = validate_doc(doc)
    for e in errs:
        print(e)
    return 1 if errs else 0


def cmd_set(a):
    ids = contract_ids(a.binary)
    v = Normalizer(a.binary)(a.harness, a.version)
    if v is None:
        return 1
    if os.path.exists(a.file):
        doc = load_document(a.file)
    else:
        doc = {"schema_version": 2, "generated_from": "manifest.py set", "generated_at": None,
               "latest_release": None, "contracts": {}, "rows": []}
    cid = a.contract_id if a.contract_id else ids[a.harness]
    rows = {row_key(r): normalize_row(r) for r in doc["rows"]}
    row = rows.get((a.harness, v, cid)) or {k: None for k in ROW_KEYS}
    row.update({"harness": a.harness, "version": v, "status": a.status, "contract_id": cid, "source": "manual",
                "recipe": row.get("recipe"), "known_broken": row.get("known_broken") or []})
    for key, val in (("evidence", a.evidence), ("broken_event", a.broken_event), ("broken_field", a.broken_field),
                     ("last_working", a.last_working), ("issue_url", a.issue_url),
                     ("supported_since", a.supported_since)):
        if val is not None:
            row[key] = val
    rows[row_key(row)] = normalize_row(row)
    out = build_doc(doc.get("generated_from"), doc.get("generated_at"), doc.get("latest_release"),
                    doc.get("contracts") or {}, list(rows.values()))
    errs = validate_doc(out)
    if errs:
        print("\n".join(errs))
        return 1
    write_atomic(a.file, render(out))
    print(f"set {a.harness} {v} {a.status} (source manual) in {a.file}")
    return 0


# ---------------------------------------------------------------- check-ruleset

def _entries(doc):
    for item in doc if isinstance(doc, list) else [doc]:
        if isinstance(item, list):
            yield from (i for i in item if isinstance(i, dict))
        elif isinstance(item, dict):
            yield item


def _covers(ruleset, patterns, target):
    if ruleset.get("enforcement") != "active" or ruleset.get("target") not in (None, target):
        return set()
    ref = (ruleset.get("conditions") or {}).get("ref_name") or {}
    include, exclude = ref.get("include") or [], ref.get("exclude") or []
    if not any(p in include for p in patterns) or any(p in exclude for p in patterns):
        return set()
    return {r.get("type") for r in ruleset.get("rules") or [] if isinstance(r, dict)}


def ruleset_problems(doc, default_branch):
    branch_patterns = ("~DEFAULT_BRANCH", f"refs/heads/{default_branch}")
    have_branch, have_tag = set(), set()
    for rs in _entries(doc):
        have_branch |= _covers(rs, branch_patterns, "branch")
        have_tag |= _covers(rs, ("refs/tags/v*",), "tag")
    problems = []
    for rule in ("deletion", "non_fast_forward"):
        if rule not in have_branch:
            problems.append(f"no active ruleset on the default branch ({default_branch}) with the {rule} rule")
    for rule in ("deletion", "update"):
        if rule not in have_tag:
            problems.append(f"no active ruleset on refs/tags/v* with the {rule} rule")
    return problems


def cmd_check_ruleset(a):
    try:
        with open(a.rulesets_json, encoding="utf-8") as f:
            doc = json.load(f)
    except (OSError, json.JSONDecodeError) as e:
        print(f"cannot read the rulesets: {e}")
        return 1
    problems = ruleset_problems(doc, a.default_branch)
    for p in problems:
        print(p)
    return 1 if problems else 0


# ---------------------------------------------------------------- embed / contract

def cmd_embed(a):
    why = None
    try:
        size = os.path.getsize(a.branch_file)
        if size > MAX_MANIFEST_BYTES:
            why = f"larger than {MAX_MANIFEST_BYTES} bytes ({size})"
        else:
            with open(a.branch_file, encoding="utf-8") as f:
                doc = json.load(f)
            errs = validate_doc(doc)
            if errs:
                why = "; ".join(errs[:3])
    except (OSError, json.JSONDecodeError) as e:
        why = f"unreadable: {e}"
    if why:
        print(f"::notice::harness-manifest branch file not embedded ({why}); keeping the in-repo {a.out}")
        return 0
    with open(a.branch_file, "rb") as f:
        data = f.read()
    write_atomic(a.out, data.decode("utf-8"))
    print("embedded harness-manifest branch file")
    return 0


def cmd_contract(a):
    print(json.dumps(contract_ids(a.binary)))
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    w = sub.add_parser("write")
    w.add_argument("--baseline", required=True)
    w.add_argument("--report", required=True)
    w.add_argument("--binary", required=True)
    w.add_argument("--release-results")
    w.add_argument("--latest-release")
    w.add_argument("--issue-urls")
    w.add_argument("--generated-at")
    w.add_argument("--out", required=True)
    v = sub.add_parser("validate")
    v.add_argument("path")
    s = sub.add_parser("set")
    s.add_argument("--file", required=True)
    s.add_argument("--harness", required=True, choices=HARNESSES)
    s.add_argument("--version", required=True)
    s.add_argument("--status", required=True, choices=STATUSES)
    s.add_argument("--contract-id")
    s.add_argument("--evidence", choices=EVIDENCE)
    s.add_argument("--broken-event")
    s.add_argument("--broken-field")
    s.add_argument("--last-working")
    s.add_argument("--issue-url")
    s.add_argument("--supported-since")
    s.add_argument("--binary", required=True)
    c = sub.add_parser("check-ruleset")
    c.add_argument("--rulesets-json", required=True)
    c.add_argument("--default-branch", default="main")
    e = sub.add_parser("embed")
    e.add_argument("--branch-file", required=True)
    e.add_argument("--out", required=True)
    k = sub.add_parser("contract")
    k.add_argument("--binary", required=True)
    a = ap.parse_args(argv)
    handler = {"write": cmd_write, "validate": cmd_validate, "set": cmd_set, "check-ruleset": cmd_check_ruleset,
               "embed": cmd_embed, "contract": cmd_contract}[a.cmd]
    try:
        return handler(a)
    except (ValueError, OSError, subprocess.SubprocessError, KeyError) as e:
        print(f"manifest.py: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
