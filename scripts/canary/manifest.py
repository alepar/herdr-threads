#!/usr/bin/env python3
"""Harness manifest writer for the canary (docs/design/herdr-threads/2026-10-02-harness-version-evidence-design.md,
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
issue-only. A probe whose last attempt also failed a tier-0 check other than `t0.payload-parse` ran in a broken
setup: its payload violation is issue-only too. Rows with `source: "manual"` are never modified by the canary.
Legacy release violations use that same source probe's final attempt, joined by normalized harness/version.
Indexed release replay additionally requires the exact source identity/attempt/stage before any output. Without
one unambiguous source and its attempt, a release violation contributes no broken observation: the writer
cannot establish setup eligibility. This may withhold a real violation until reliable source results exist;
existing rows and release verified observations keep their usual behavior.

The release results document has three forms: {"tag", "supported": true, "contract_id", "probes"}; {"tag",
"supported": false} (the release cannot report a contract id); and {"tag", "error": "<reason>"} (an infrastructure
failure: the release says nothing about its contract, so the baseline's rows of every contract id are kept).
"""
import sys
# scripts/canary/bisect.py would shadow the stdlib `bisect` (needed by random/tempfile) when this directory is
# sys.path[0]; drop it while the imports run and restore it after (this module is also imported by tests).
_here = __import__("os").path.dirname(__import__("os").path.abspath(__file__))
_saved_path = list(sys.path)
sys.path[:] = [p for p in sys.path if p not in ("", _here)]

import argparse, copy, datetime, importlib.util, json, os, pathlib, re, subprocess

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
PAYLOAD_CHECK = "t0.payload-parse"


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
    try:
        validate_runtime(doc)
    except (ValueError, KeyError, TypeError) as e:
        errs.append(f"runtime collections: {e}")
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


# Runtime history uses the discovery projection, not a fabricated contract hash.
RUNTIME_KEYS = set("harness identity domain origin contract_id status evidence_stage source required_milestones successful_milestones broken_event broken_field supported_since issue_url last_seen_at".split())


def runtime_key(row):
    return (row["harness"], row["identity"]["key"], row["domain"], row["origin"], row["contract_id"])


def validate_runtime(doc):
    runner = _load_sibling("run")
    contracts = doc.get("runtime_contracts", {})
    rows = doc.get("runtime_rows", [])
    if not isinstance(contracts, dict) or len(contracts) > 64 or not isinstance(rows, list):
        raise ValueError("invalid collections")
    descriptors = {}
    for h, domains in contracts.items():
        if not isinstance(domains, list):
            raise ValueError("invalid descriptors")
        for c in domains:
            runner.validate_discovery({"schema_version": 1, "adapters": [{"id": h, "display_name": h,
                "host_kinds": [], "setup_scopes": [], "legacy_contract_id": None,
                "canary_strategy": None, "contracts": [c]}]})
            key = (h, c["domain"], c["origin"], c["id"])
            if key in descriptors or not c["required_milestones"]:
                raise ValueError("duplicate or empty descriptor")
            descriptors[key] = c
    schema = json.loads((pathlib.Path(_here) / "companion_schema.json").read_text())
    seen = set()
    for r in rows:
        if not isinstance(r, dict) or set(r) != RUNTIME_KEYS:
            raise ValueError("invalid row fields")
        runner._schema(r["identity"], schema["properties"]["identity"])
        if r["identity"] is None:
            raise ValueError("missing identity")
        runner.validate_identity(r["identity"])
        c = descriptors.get((r["harness"], r["domain"], r["origin"], r["contract_id"]))
        if c is None or sorted(r["required_milestones"]) != sorted(c["required_milestones"]):
            raise ValueError("row has no exact descriptor")
        for field in ("required_milestones", "successful_milestones"):
            runner._unique(r[field], 8)
        milestones = {e["milestone"] for e in c["events"] if e["milestone"] is not None}
        if set(r["successful_milestones"]) - milestones:
            raise ValueError("undeclared milestone")
        if r["source"] not in SOURCES or r["status"] not in STATUSES or r["evidence_stage"] not in ("source_captured", "no_model", "live"):
            raise ValueError("invalid status/source/stage")
        if type(r["last_seen_at"]) is not int or not 0 <= r["last_seen_at"] <= 2**63 - 1:
            raise ValueError("last_seen_at must be nonnegative u64 milliseconds in the store range")
        for field, limit in (("supported_since", 128), ("issue_url", 512)):
            if r[field] is not None:
                runner._text(r[field], r"[^\x00-\x1f\x7f]+", limit)
        if r["status"] == "verified":
            if (r["evidence_stage"] == "source_captured" or r["broken_event"] is not None
                    or r["broken_field"] is not None or not set(c["required_milestones"]) <= set(r["successful_milestones"])):
                raise ValueError("unqualified verified row")
        else:
            if r["broken_event"] not in {e["event"] for e in c["events"]}:
                raise ValueError("undeclared violation event")
            runner._text(r["broken_field"], r"[A-Za-z_][A-Za-z0-9_.]*", 64)
        key = runtime_key(r)
        if key in seen:
            raise ValueError("duplicate runtime row")
        seen.add(key)


def discovery(binary):
    runner = _load_sibling("run")
    rc, stdout, _ = runner.bounded_capture([str(binary), "adapters", "--json"], timeout=60)
    if rc != 0:
        return None  # Explicit legacy fallback; never invent rich descriptors.
    runner = _load_sibling("run")
    doc = runner._json(stdout, 65536)
    runner.validate_discovery(doc)
    return doc


def indexed_results(index_path, registry):
    runner = _load_sibling("run")
    root = pathlib.Path(index_path).parent
    raw = runner._read_bounded(pathlib.Path(index_path), 262144)
    index = runner.validate_index(raw, root, registry["adapters"])
    adapters = {a["id"]: a for a in registry["adapters"]}
    return [(entry, runner.validate_result(runner._read_bounded(root / entry["result_path"], 65536),
                 adapters[entry["harness"]], entry["attempt"], entry["evidence_stage"])) for entry in index["attempts"]]


def qualified_results(results, root, report=None):
    """Retain artifacts, but suppress observations from flaky/failed legacy setup."""
    runner = _load_sibling("run")
    out = []
    for entry, result in results:
        excluded = False
        work = pathlib.Path(root) / "work" / (entry["harness"] + "-" + entry["attempt"])
        legacy = work / "legacy-probe.json"
        if legacy.exists():
            checked = runner._relative(pathlib.Path(root), str(legacy.relative_to(root)))
            probe = runner._json(runner._read_bounded(checked, 65536), 65536)
            excluded = probe.get("result") == "infra" or _unrelated_tier0_failure(probe.get("checks"))
        identity = result["identity"]
        if report is not None and identity is not None and identity["key"].startswith("release:"):
            probes = [(b, p) for b in report.get("harnesses", []) if b.get("harness") == entry["harness"]
                      for p in b.get("probes", []) if p.get("version") == identity["release_version"]]
            if len(probes) > 1:
                excluded = True
            for block, probe in probes:
                attempts = probe.get("attempts") or []
                excluded |= (probe.get("flaky", False) or probe.get("result") == "infra"
                             or block.get("status") in ("infra_error", "inconclusive")
                             or (bool(attempts) and _unrelated_tier0_failure(attempts[-1].get("checks"))))
        out.append((entry, dict(result, outcome="inconclusive") if excluded else result))
    return out


def add_runtime(doc, baseline, registry, results, now, release=None):
    contracts = copy.deepcopy(baseline.get("runtime_contracts", {}))
    rows = {runtime_key(r): copy.deepcopy(r) for r in baseline.get("runtime_rows", [])}
    # Only the final indexed attempt of each exact identity contributes observations.
    final = {}
    for entry, result in results:
        if result["identity"] is not None:
            final[(entry["harness"], entry["identity_key"])] = (entry, result)
    has_complete = any(r["outcome"] == "complete" for _, r in final.values())
    if registry is not None and has_complete:
        for a in registry["adapters"]:
            for c in a["contracts"]:
                if not c["required_milestones"]:
                    continue
                existing = contracts.setdefault(a["id"], [])
                key = (c["domain"], c["origin"], c["id"])
                old = next((d for d in existing if (d["domain"], d["origin"], d["id"]) == key), None)
                if old is not None and old != c:
                    raise ValueError("conflicting exact descriptor")
                if old is None:
                    existing.append(copy.deepcopy(c))
    instant = datetime.datetime.fromisoformat(now.replace("Z", "+00:00"))
    if instant.tzinfo is None:
        raise ValueError("generated_at requires an RFC3339 UTC offset")
    timestamp = int(instant.timestamp() * 1000)
    adapters = {a["id"]: a for a in (registry or {}).get("adapters", [])}
    for entry, result in final.values():
        if result["outcome"] != "complete" or result["identity"] is None:
            continue
        descriptors = {c["domain"]: c for c in adapters[entry["harness"]]["contracts"]}
        for d in result["domains"]:
            c = descriptors[d["domain"]]
            broken = d["outcome"] == "contract_violation" and d["violations"]
            verified = (d["outcome"] == "compatible" and result["evidence_stage"] != "source_captured"
                        and bool(c["required_milestones"]) and set(c["required_milestones"]) <= set(d["successful_milestones"]))
            if not broken and not verified or not c["required_milestones"]:
                continue
            r = dict(harness=entry["harness"], identity=result["identity"], domain=d["domain"], origin=d["origin"],
                     contract_id=d["contract_id"], status="known_broken" if broken else "verified",
                     evidence_stage=result["evidence_stage"], source="canary", required_milestones=c["required_milestones"],
                     successful_milestones=d["successful_milestones"], broken_event=d["violations"][0]["event"] if broken else None,
                     broken_field=d["violations"][0]["field"] if broken else None, supported_since=None, issue_url=None, last_seen_at=timestamp)
            key = runtime_key(r)
            old = rows.get(key)
            if old is not None and old["source"] == "manual":
                continue
            if old:
                r.update(supported_since=old["supported_since"], issue_url=old["issue_url"])
            if release and release.get("tag") and verified and not r["supported_since"]:
                r["supported_since"] = release["tag"].removeprefix("v")
            rows[key] = r
    # Missing/infrastructure runs retain every baseline collection, without pruning.
    if has_complete:
        newest = {}
        for r in rows.values():
            newest.setdefault(r["harness"], {})[r["identity"]["key"]] = max(r["last_seen_at"], newest.get(r["harness"], {}).get(r["identity"]["key"], 0))
        keep = {h: {key for key, _ in sorted(vs.items(), key=lambda v: (-v[1], v[0]))[:50]} for h, vs in newest.items()}
        rows = {k: r for k, r in rows.items() if r["identity"]["key"] in keep[r["harness"]]
                or r["source"] == "manual" or r["status"] == "known_broken"
                or any(legacy.get("harness") == r["harness"] and legacy.get("recipe") is not None
                       and r["identity"]["key"] == "release:" + legacy["version"]
                       for legacy in baseline.get("rows", []))}
    if "runtime_rows" in baseline or registry is not None:
        doc["runtime_contracts"] = contracts
        doc["runtime_rows"] = sorted(rows.values(), key=runtime_key) if has_complete else copy.deepcopy(baseline.get("runtime_rows", []))
    return doc


def replay_results(release, source_results):
    """Release classifications may establish schema compatibility at the source stage."""
    registry = release.get("discovery")
    runner = _load_sibling("run")
    if registry is not None:
        runner.validate_discovery(registry)
    adapters = {a["id"]: a for a in (registry or {}).get("adapters", [])}
    sources = {(e["harness"], e["attempt"]): (e, r) for e, r in source_results}
    final = {}
    for e, r in source_results:
        final[(e["harness"], e["identity_key"])] = e["attempt"]
    results = []
    seen = set()
    for p in release.get("probes", []):
        pair = p.get("harness"), p.get("attempt")
        if pair not in sources or pair in seen:
            raise ValueError("missing/ambiguous indexed release source")
        seen.add(pair)
        entry, source = sources[pair]
        if (source["identity"] != p.get("identity") or source["evidence_stage"] != p.get("evidence_stage")
                or final[(entry["harness"], entry["identity_key"])] != entry["attempt"]):
            raise ValueError("release source identity/attempt/stage mismatch")
        if source["outcome"] != "complete" or source["identity"] is None or pair[0] not in adapters:
            continue
        domains = []
        for c in adapters[pair[0]]["contracts"]:
            if c["origin"] != "native_payload" or not any(
                    (d["domain"], d["origin"]) == (c["domain"], c["origin"]) for d in source["domains"]):
                continue
            payloads = p.get("payloads", [])
            classified = [x for x in payloads if x.get("event") in {e["event"] for e in c["events"]}]
            violations = [{"event": x["event"], "field": x.get("field")} for x in classified if x.get("kind") == "violation"]
            passed = {x["event"] for x in classified if x.get("kind") == "ok"}
            milestones = [e["milestone"] for e in c["events"] if e["event"] in passed and e["milestone"] is not None]
            compatible = payloads and all(x.get("kind") == "ok" for x in payloads)
            domains.append(dict(domain=c["domain"], origin=c["origin"], contract_id=c["id"],
                                successful_milestones=sorted(set(milestones)), violations=violations,
                                outcome="contract_violation" if violations else "compatible" if compatible else "inconclusive"))
        # A release without a replay evaluator for every domain is inconclusive;
        # it never credits native shape or bridge domains from legacy payloads.
        result = dict(schema_version=1, harness=pair[0], attempt=pair[1], identity=source["identity"],
                      evidence_stage=source["evidence_stage"], outcome="complete" if len(domains) == len(adapters[pair[0]]["contracts"])
                      and all(d["outcome"] != "inconclusive" for d in domains) else "inconclusive", reason=None, domains=domains)
        result = runner.validate_result(json.dumps(result).encode(), adapters[pair[0]], pair[1], p["evidence_stage"])
        results.append((entry, result))
    return registry, results


def prepare_replay(root, release_registry, ids):
    """Resolve kept captures solely through validated request/result/index metadata."""
    runner = _load_sibling("run")
    root = pathlib.Path(root)
    index_path = root / "artifact-index.json"
    index = runner._json(runner._read_bounded(index_path, 262144), 262144)
    runner._schema(index, json.loads((pathlib.Path(_here) / "artifact_index_schema.json").read_text()))
    source_adapters = {}
    for entry in index["attempts"]:
        work = root / "work" / (entry["harness"] + "-" + entry["attempt"])
        request_path = runner._relative(work, "request.json")
        request = runner._json(runner._read_bounded(request_path, 65536), 65536)
        a = request["adapter"]
        runner.validate_discovery({"schema_version": 1, "adapters": [a]})
        if a["id"] != entry["harness"] or (a["id"] in source_adapters and source_adapters[a["id"]] != a):
            raise ValueError("ambiguous source adapter")
        source_adapters[a["id"]] = a
    registry = {"schema_version": 1, "adapters": list(source_adapters.values())}
    results = qualified_results(indexed_results(index_path, registry), root)
    release_adapters = {a["id"]: a for a in (release_registry or {}).get("adapters", [])}
    plan = {"contract_id": ids, "discovery": release_registry, "probes": [], "unsupported": []}
    final = {}
    for entry, result in results:
        if result["identity"] is not None:
            final[(entry["harness"], entry["identity_key"])] = (entry, result)
    for entry, result in final.values():
        h = entry["harness"]
        identity = result["identity"]
        declared = release_adapters.get(h, {}).get("contracts", [])
        representable = (h in HARNESSES and identity["key"].startswith("release:") and ids.get(h))
        payload_domains = [d for d in result["domains"] if d["origin"] == "native_payload"]
        for d in result["domains"]:
            if not representable or d not in payload_domains or (release_registry is not None and not any(
                    (c["domain"], c["origin"]) == (d["domain"], d["origin"]) for c in declared)):
                plan["unsupported"].append(dict(harness=h, identity_key=identity["key"], domain=d["domain"],
                                             origin=d["origin"], attempt=entry["attempt"]))
        if not representable or not payload_domains:
            continue
        paths = []
        source_work = root / "work" / (h + "-" + entry["attempt"])
        probe_path = None
        for metadata_path in entry["capture_paths"]:
            metadata = runner._json(runner._read_bounded(root / metadata_path, 65536), 65536)
            if not any((d["domain"], d["origin"]) == (metadata["domain"], metadata["origin"]) for d in payload_domains):
                continue
            target = runner._relative(source_work, metadata["path"])
            tier = "tier1" if target.suffix == ".json" else "tier0"
            if tier == "tier1" and entry["evidence_stage"] != "live":
                continue
            if target.suffix not in (".json", ".stdin", ".argv"):
                continue
            destination = "capture/" + tier + "/" + target.name
            if any(p[1] == destination for p in paths):
                raise ValueError("ambiguous capture destination")
            paths.append((target, destination))
            for parent in target.parents:
                if not parent.is_relative_to(source_work):
                    break
                candidate = parent / "canary-probe.json"
                if candidate.is_file():
                    if probe_path is not None and candidate != probe_path:
                        raise ValueError("ambiguous legacy source probe")
                    probe_path = candidate
                    break
        if not paths:
            continue
        replay_work = runner._relative(root, "release-work/" + h + "-" + entry["attempt"])
        replay_work.mkdir(parents=True, exist_ok=True)
        for target, destination in paths:
            output = replay_work / destination
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(runner._read_bounded(target, 65536))
        if probe_path is not None:
            probe_path = runner._relative(source_work, str(probe_path.relative_to(source_work)))
            probe = runner._json(runner._read_bounded(probe_path, 65536), 65536)
            if (probe.get("harness"), probe.get("version")) != (h, identity["release_version"]):
                raise ValueError("legacy source identity mismatch")
            (replay_work / "canary-probe.json").write_text(json.dumps(probe))
            for help_name in ("codex.txt", "codex-exec.txt"):
                help_path = probe_path.parent / "help" / help_name
                if help_path.is_file():
                    checked = runner._relative(source_work, str(help_path.relative_to(source_work)))
                    (replay_work / "help").mkdir(exist_ok=True)
                    (replay_work / "help" / help_name).write_bytes(runner._read_bounded(checked, 65536))
        plan["probes"].append(dict(harness=h, version=identity["release_version"], identity=identity,
                                   attempt=entry["attempt"], evidence_stage=entry["evidence_stage"],
                                   work_path=str(replay_work.relative_to(root)), source_outcome=result["outcome"]))
    return plan


def collect_replay(root, plan, tag):
    runner = _load_sibling("run")
    probes = []
    for original in plan["probes"]:
        work = pathlib.Path(root) / original["work_path"]
        rust_path = work / "canary-rust.json"
        if not rust_path.is_file():
            continue
        rust = runner._json(runner._read_bounded(rust_path, 262144), 262144)
        payloads = []
        for p in rust.get("payloads", []):
            c = p.get("contract") if isinstance(p, dict) else None
            payloads.append(c if isinstance(c, dict) and c.get("kind") in ("ok", "violation", "malformed")
                            else {"event": None, "kind": "unclassified", "field": None})
        probes.append(dict(original, payloads=payloads))
    return dict(tag=tag, supported=True, contract_id=plan["contract_id"], discovery=plan["discovery"],
                unsupported_domains=plan["unsupported"], probes=probes)


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


def _unrelated_tier0_failure(checks):
    """True when a tier-0 check other than the payload parse failed: the probe ran in a broken setup."""
    return any(c.get("status") == "fail" and c.get("id") != PAYLOAD_CHECK for c in checks or [])


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
            if oc and oc[0] == "broken" and _unrelated_tier0_failure(attempts[-1].get("checks")):
                continue
            if oc:
                out.append((h, v, oc, block.get("first_bad")))
    return out


def release_observations(release, report, normalize):
    """(harness, version, outcome) per probe the latest release's contract ran over; none when unsupported."""
    out = []
    if not isinstance(release, dict) or not release.get("supported"):
        return out
    sources = {}
    for block in report.get("harnesses", []):
        h = block.get("harness")
        if h not in HARNESSES:
            continue
        for probe in block.get("probes", []):
            v = normalize(h, probe.get("version", ""))
            if v is not None:
                sources.setdefault((h, v), []).append(probe)
    for probe in release.get("probes", []):
        h = probe.get("harness")
        if h not in HARNESSES:
            continue
        v = normalize(h, probe.get("version", ""))
        if v is None:
            continue
        oc = _outcome(probe.get("payloads") or [], True)
        if "evidence_stage" in probe:
            if probe.get("source_outcome") != "complete" or probe["evidence_stage"] == "source_captured":
                continue
            if oc and oc[0] == "verified":
                oc = ("verified", probe["evidence_stage"])
        if oc and oc[0] == "broken":
            source = sources.get((h, v), [])
            if len(source) != 1 or not source[0].get("attempts"):
                warn(f"release violation for {h} {v} has no unique source attempt; keeping existing rows")
                continue
            if _unrelated_tier0_failure(source[0]["attempts"][-1].get("checks")):
                continue
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
    rel_error = isinstance(release, dict) and bool(release.get("error"))
    tag = (latest_release or baseline.get("latest_release") or None)
    tag = tag[1:] if isinstance(tag, str) and tag.startswith("v") else tag
    issue_urls = issue_urls or {}

    rows = {}
    for r in baseline.get("rows", []):
        r = normalize_row(r)
        rows[row_key(r)] = r
    if rel_error:  # an infrastructure failure says nothing about the release's contract: keep its rows
        warn(f"release check failed ({release['error']}); keeping the baseline's release-contract rows")
        for r in rows.values():
            if r.get("contract_id"):
                live[r["harness"]].add(r["contract_id"])

    obs = []  # (harness, version, contract_id, outcome, block first_bad or None)
    for h, v, oc, first_bad in main_observations(report, normalize):
        obs.append((h, v, main_id[h], oc, first_bad))
    rel_obs = release_observations(release, report, normalize) if rel_ok else []
    if rel_ok:
        for h, v, oc in rel_obs:
            if rel_id.get(h) != main_id[h]:  # the same contract folds into the main probes' rows
                obs.append((h, v, rel_id[h], oc, None))

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
        # the greatest verified version strictly below v that counts under this row's contract (its own rows
        # or a null-contract row); none when there is no such row, so the action falls through to the recipe
        below = [r["version"] for r in rows.values()
                 if r["harness"] == h and versions.applies(r, cid) and r["status"] == "verified"
                 and version_key(r["version"]) < version_key(v)]
        last_working = max(below, key=version_key) if below else None
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
    index_path = a.artifact_index or os.path.join(os.path.dirname(a.report), "artifact-index.json")
    registry = discovery(a.binary) if os.path.exists(index_path) else None
    results = qualified_results(indexed_results(index_path, registry), pathlib.Path(index_path).parent, report) if registry is not None else []
    if os.path.exists(index_path) and registry is None:
        raise ValueError("indexed runtime results require binary discovery")
    doc = add_runtime(doc, baseline, registry, results, doc["generated_at"])
    if release is not None and (release.get("discovery") is not None or any(
            "attempt" in p for p in release.get("probes", []))):
        release_registry, replays = replay_results(release, results)
        if release_registry is not None:
            doc = add_runtime(doc, doc, release_registry, replays, doc["generated_at"], release)
    if (not any(b.get("status") not in ("infra_error", "inconclusive")
                and any(p.get("attempts") and p.get("result") != "infra" and not p.get("flaky")
                        and not _unrelated_tier0_failure(p["attempts"][-1].get("checks"))
                        for p in b.get("probes", [])) for b in report.get("harnesses", [])) and (release is None or release.get("error"))):
        doc["rows"] = copy.deepcopy(baseline["rows"])
        doc["contracts"] = copy.deepcopy(baseline.get("contracts", {}))
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
    for collection in ("runtime_contracts", "runtime_rows"):
        if collection in doc:
            out[collection] = copy.deepcopy(doc[collection])
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
    w.add_argument("--artifact-index")
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
